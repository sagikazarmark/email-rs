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
    ///
    /// # Errors
    ///
    /// Returns a resolver-defined [`AttachmentResolveError`] classified by
    /// [`ResolveErrorKind`] when the reference is unsupported, unavailable,
    /// denied, too large, transiently inaccessible, or cannot be processed.
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
            .get(reference.as_str())
            .cloned()
            .map(ResolvedAttachment::new)
            .ok_or_else(|| {
                AttachmentResolveError::new(
                    ResolveErrorKind::NotFound,
                    format!(
                        "attachment reference `{}` was not found",
                        reference.as_str()
                    ),
                )
            })
    }
}

/// Resolver combinator that routes references by their leading scheme.
///
/// A reference is treated as `scheme:value`; the router is not a URI parser.
/// It splits on the first `:`, matches the scheme case-insensitively, and
/// performs no authority parsing, percent-decoding, or path normalization on
/// the value: `scheme://path/to/thing` routes `path/to/thing` as one opaque
/// value, with the `//` treated as cosmetic.
///
/// By default the selected resolver receives only the value, with `scheme:`
/// and an optional `//` stripped. Register with
/// [`SchemeDispatch::FullReference`] to pass the original reference through
/// unchanged, for resolvers that need the whole reference, such as an HTTP
/// fetcher registered for `https`.
///
/// A reference without a registered, valid scheme prefix
/// fails with [`ResolveErrorKind::UnsupportedReference`]; wrap the router in a
/// [`FallbackResolver`] to give plain, scheme-less keys a home.
#[derive(Default)]
pub struct SchemeRouter {
    routes: BTreeMap<String, Route>,
}

impl SchemeRouter {
    /// Create a router without registered schemes.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            routes: BTreeMap::new(),
        }
    }

    /// Register or replace the resolver for `scheme`.
    ///
    /// The resolver receives the reference value with `scheme:` and an
    /// optional `//` stripped; use [`SchemeRouter::register_with`] to choose a
    /// different dispatch mode.
    ///
    /// # Panics
    ///
    /// Panics when `scheme` is not a valid scheme token: a letter followed by
    /// letters, digits, `+`, `-`, or `.`.
    pub fn register<R>(&mut self, scheme: impl Into<String>, resolver: R)
    where
        R: AttachmentResolver + 'static,
    {
        self.register_with(scheme, resolver, SchemeDispatch::StrippedValue);
    }

    /// Register or replace the resolver for `scheme` with an explicit dispatch
    /// mode.
    ///
    /// # Panics
    ///
    /// Panics when `scheme` is not a valid scheme token: a letter followed by
    /// letters, digits, `+`, `-`, or `.`.
    pub fn register_with<R>(
        &mut self,
        scheme: impl Into<String>,
        resolver: R,
        dispatch: SchemeDispatch,
    ) where
        R: AttachmentResolver + 'static,
    {
        let scheme = normalize_scheme(&scheme.into());
        assert!(
            is_valid_scheme(&scheme),
            "`{scheme}` is not a valid attachment reference scheme"
        );

        self.routes.insert(
            scheme,
            Route {
                resolver: Box::new(resolver),
                dispatch,
            },
        );
    }

    /// Register a resolver using builder syntax.
    ///
    /// # Panics
    ///
    /// Panics when `scheme` is not a valid scheme token: a letter followed by
    /// letters, digits, `+`, `-`, or `.`.
    #[must_use]
    pub fn with_resolver<R>(mut self, scheme: impl Into<String>, resolver: R) -> Self
    where
        R: AttachmentResolver + 'static,
    {
        self.register(scheme, resolver);
        self
    }

    /// Register a resolver with an explicit dispatch mode using builder
    /// syntax.
    ///
    /// # Panics
    ///
    /// Panics when `scheme` is not a valid scheme token: a letter followed by
    /// letters, digits, `+`, `-`, or `.`.
    #[must_use]
    pub fn with_resolver_using<R>(
        mut self,
        scheme: impl Into<String>,
        resolver: R,
        dispatch: SchemeDispatch,
    ) -> Self
    where
        R: AttachmentResolver + 'static,
    {
        self.register_with(scheme, resolver, dispatch);
        self
    }
}

impl AttachmentResolver for SchemeRouter {
    async fn resolve(
        &self,
        reference: &AttachmentReference,
    ) -> Result<ResolvedAttachment, AttachmentResolveError> {
        let route = reference
            .as_str()
            .split_once(':')
            .filter(|(scheme, _)| is_valid_scheme(scheme))
            .and_then(|(scheme, value)| {
                self.routes
                    .get(&scheme.to_ascii_lowercase())
                    .map(|route| (route, value))
            });
        let Some((route, value)) = route else {
            return Err(AttachmentResolveError::new(
                ResolveErrorKind::UnsupportedReference,
                format!(
                    "no attachment resolver is registered for reference `{}`",
                    reference.as_str()
                ),
            ));
        };

        match route.dispatch {
            SchemeDispatch::StrippedValue => {
                let value = value.strip_prefix("//").unwrap_or(value);
                route
                    .resolver
                    .resolve(&AttachmentReference::new(value))
                    .await
            }
            SchemeDispatch::FullReference => route.resolver.resolve(reference).await,
        }
    }
}

/// How a [`SchemeRouter`] presents a routed reference to the selected
/// resolver.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum SchemeDispatch {
    /// Pass only the value after `scheme:`, with an optional `//` stripped.
    #[default]
    StrippedValue,
    /// Pass the original reference through unchanged, scheme included.
    FullReference,
}

/// Resolver combinator that consults a fallback for unsupported references.
///
/// The fallback runs only when the primary fails with
/// [`ResolveErrorKind::UnsupportedReference`]: the primary could not interpret
/// the reference at all, so another resolver may claim it. Every other
/// failure, including [`ResolveErrorKind::NotFound`], is authoritative and
/// propagates unchanged, so missing content, denied access, and transient
/// faults are never masked by the fallback.
///
/// Combines naturally with [`SchemeRouter`], which reports unrouted
/// references as unsupported: wrapping a router gives plain, scheme-less keys
/// a home.
#[derive(Clone, Copy, Debug, Default)]
pub struct FallbackResolver<P, F> {
    primary: P,
    fallback: F,
}

impl<P, F> FallbackResolver<P, F> {
    /// Combine `primary` with `fallback`.
    #[must_use]
    pub const fn new(primary: P, fallback: F) -> Self {
        Self { primary, fallback }
    }

    /// Return the primary resolver.
    #[must_use]
    pub const fn primary(&self) -> &P {
        &self.primary
    }

    /// Return the fallback resolver.
    #[must_use]
    pub const fn fallback(&self) -> &F {
        &self.fallback
    }
}

impl<P, F> AttachmentResolver for FallbackResolver<P, F>
where
    P: AttachmentResolver,
    F: AttachmentResolver,
{
    async fn resolve(
        &self,
        reference: &AttachmentReference,
    ) -> Result<ResolvedAttachment, AttachmentResolveError> {
        match self.primary.resolve(reference).await {
            Err(error) if error.kind == ResolveErrorKind::UnsupportedReference => {
                self.fallback.resolve(reference).await
            }
            result => result,
        }
    }
}

struct Route {
    resolver: Box<dyn ErasedAttachmentResolver>,
    dispatch: SchemeDispatch,
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

fn normalize_scheme(scheme: &str) -> String {
    scheme
        .trim_end_matches(&[':', '/'][..])
        .to_ascii_lowercase()
}

fn is_valid_scheme(scheme: &str) -> bool {
    let mut characters = scheme.chars();

    characters
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic())
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        })
}
