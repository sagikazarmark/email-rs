use core::future::Future;
use std::collections::BTreeMap;

use email_message::AttachmentReference;
use email_transport::{BoxFut, MaybeSend, RuntimeBound};

/// Resolves opaque attachment references into attachment content.
///
/// The resolver owns the interpretation of each reference. References may be
/// URIs, plain keys, provider identifiers, or any other application-defined
/// string.
pub trait AttachmentResolver: RuntimeBound {
    /// Resolve `reference` into attachment bytes.
    fn resolve<'a>(
        &'a self,
        reference: &'a AttachmentReference,
    ) -> impl Future<Output = Result<ResolvedAttachment, AttachmentResolveError>> + MaybeSend + 'a;
}

/// Resolved attachment content.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResolvedAttachment {
    /// Materialized attachment bytes.
    pub bytes: Vec<u8>,
}

impl ResolvedAttachment {
    /// Create resolved attachment content from bytes.
    #[must_use]
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            bytes: bytes.into(),
        }
    }
}

/// Classified attachment resolution failure.
#[derive(Debug)]
pub struct AttachmentResolveError {
    /// Resolution failure category.
    pub kind: ResolveErrorKind,
    message: String,
    source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
}

impl AttachmentResolveError {
    /// Create a resolution error without an underlying source.
    #[must_use]
    pub fn new(kind: ResolveErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            source: None,
        }
    }

    /// Return the human-readable failure description.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Attach the backing store or resolver error that caused this failure.
    #[must_use]
    pub fn with_source(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }
}

impl std::fmt::Display for AttachmentResolveError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.kind, self.message)
    }
}

impl std::error::Error for AttachmentResolveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn std::error::Error + 'static))
    }
}

/// Canonical category for an attachment resolution failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResolveErrorKind {
    /// The resolver does not recognize the reference's addressing or format.
    UnsupportedReference,
    /// The backing store reports that the referenced content does not exist.
    NotFound,
    /// Access to the backing store was denied.
    Denied,
    /// The content exceeds a configured size limit.
    TooLarge,
    /// A transient store or network failure occurred and is safe to retry.
    Transient,
    /// A resolver bug or invariant violation occurred.
    Internal,
}

impl std::fmt::Display for ResolveErrorKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::UnsupportedReference => "unsupported-reference",
            Self::NotFound => "not-found",
            Self::Denied => "denied",
            Self::TooLarge => "too-large",
            Self::Transient => "transient",
            Self::Internal => "internal",
        };
        formatter.write_str(label)
    }
}

/// In-memory resolver for plain-string references and static assets.
#[derive(Clone, Debug, Default)]
pub struct MapResolver {
    entries: BTreeMap<String, Vec<u8>>,
}

impl MapResolver {
    /// Create an empty resolver.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    /// Insert a reference and its bytes.
    pub fn insert(
        &mut self,
        reference: impl Into<String>,
        bytes: impl Into<Vec<u8>>,
    ) -> Option<Vec<u8>> {
        self.entries.insert(reference.into(), bytes.into())
    }

    /// Add a reference and its bytes using builder syntax.
    #[must_use]
    pub fn with_entry(mut self, reference: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        self.insert(reference, bytes);
        self
    }
}

impl AttachmentResolver for MapResolver {
    async fn resolve(
        &self,
        reference: &AttachmentReference,
    ) -> Result<ResolvedAttachment, AttachmentResolveError> {
        self.entries
            .get(reference.uri())
            .cloned()
            .map(ResolvedAttachment::new)
            .ok_or_else(|| {
                AttachmentResolveError::new(
                    ResolveErrorKind::NotFound,
                    format!("attachment reference `{}` was not found", reference.uri()),
                )
            })
    }
}

/// Resolver combinator that routes references by their leading scheme.
///
/// Both `scheme:value` and `scheme://value` select the resolver registered for
/// `scheme`. The selected resolver receives the original, unmodified reference.
#[derive(Default)]
pub struct SchemeRouter {
    resolvers: BTreeMap<String, Box<dyn ErasedAttachmentResolver>>,
}

impl SchemeRouter {
    /// Create a router without registered schemes.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            resolvers: BTreeMap::new(),
        }
    }

    /// Register or replace the resolver for `scheme`.
    pub fn register<R>(&mut self, scheme: impl Into<String>, resolver: R)
    where
        R: AttachmentResolver + 'static,
    {
        self.resolvers
            .insert(normalize_scheme(scheme.into()), Box::new(resolver));
    }

    /// Register a resolver using builder syntax.
    #[must_use]
    pub fn with_resolver<R>(mut self, scheme: impl Into<String>, resolver: R) -> Self
    where
        R: AttachmentResolver + 'static,
    {
        self.register(scheme, resolver);
        self
    }
}

impl AttachmentResolver for SchemeRouter {
    async fn resolve(
        &self,
        reference: &AttachmentReference,
    ) -> Result<ResolvedAttachment, AttachmentResolveError> {
        let scheme = reference.uri().split_once(':').map(|(scheme, _)| scheme);
        let resolver = scheme.and_then(|scheme| self.resolvers.get(scheme));
        match resolver {
            Some(resolver) => resolver.resolve(reference).await,
            None => Err(AttachmentResolveError::new(
                ResolveErrorKind::UnsupportedReference,
                format!(
                    "no attachment resolver is registered for reference `{}`",
                    reference.uri()
                ),
            )),
        }
    }
}

trait ErasedAttachmentResolver: RuntimeBound {
    fn resolve<'a>(
        &'a self,
        reference: &'a AttachmentReference,
    ) -> BoxFut<'a, Result<ResolvedAttachment, AttachmentResolveError>>;
}

impl<R> ErasedAttachmentResolver for R
where
    R: AttachmentResolver + ?Sized,
{
    fn resolve<'a>(
        &'a self,
        reference: &'a AttachmentReference,
    ) -> BoxFut<'a, Result<ResolvedAttachment, AttachmentResolveError>> {
        Box::pin(AttachmentResolver::resolve(self, reference))
    }
}

fn normalize_scheme(scheme: String) -> String {
    scheme.trim_end_matches(&[':', '/'][..]).to_owned()
}
