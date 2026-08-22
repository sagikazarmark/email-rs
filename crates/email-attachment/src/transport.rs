use email_message::OutboundMessage;
use email_transport::{
    Capabilities, ErrorKind, SendOptions, SendReport, Transport, TransportError,
};

use crate::preparation::{enforce_limits, has_attachment_references};
use crate::{
    AttachmentResolveError, AttachmentResolver, PreparationLimits, ResolveErrorKind,
    prepare_attachments,
};

/// Transport decorator that resolves attachment references before delivery.
///
/// The configured [`PreparationLimits`] apply to every send, not only to sends
/// that carry references: a byte-backed message is checked against the same
/// policy before it is passed through untouched.
pub struct ResolvingTransport<T, R> {
    inner: T,
    resolver: R,
    limits: PreparationLimits,
}

impl<T, R> ResolvingTransport<T, R> {
    /// Wrap `inner` with attachment resolution using unlimited size policy.
    #[must_use]
    pub fn new(inner: T, resolver: R) -> Self {
        Self {
            inner,
            resolver,
            limits: PreparationLimits::default(),
        }
    }

    /// Set the attachment preparation size policy.
    #[must_use]
    pub const fn with_limits(mut self, limits: PreparationLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Return the wrapped transport.
    #[must_use]
    pub const fn inner(&self) -> &T {
        &self.inner
    }

    /// Return the configured attachment resolver.
    #[must_use]
    pub const fn resolver(&self) -> &R {
        &self.resolver
    }

    /// Return the attachment preparation size policy.
    #[must_use]
    pub const fn limits(&self) -> &PreparationLimits {
        &self.limits
    }

    /// Unwrap and return the inner transport.
    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T, R> Clone for ResolvingTransport<T, R>
where
    T: Clone,
    R: Clone,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            resolver: self.resolver.clone(),
            limits: self.limits,
        }
    }
}

impl<T, R> Transport for ResolvingTransport<T, R>
where
    T: Transport,
    R: AttachmentResolver,
{
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities().with_attachment_references(true)
    }

    async fn send(
        &self,
        message: &OutboundMessage,
        options: &SendOptions,
    ) -> Result<SendReport, TransportError> {
        if !has_attachment_references(message) {
            // Still a zero-copy path: the size policy is checked against the
            // borrowed message so byte-backed sends are held to the same limits
            // as resolved ones.
            enforce_limits(message, &self.limits).map_err(TransportError::from)?;
            return self.inner.send(message, options).await;
        }

        let prepared = prepare_attachments(message.clone(), &self.resolver, &self.limits)
            .await
            .map_err(TransportError::from)?;
        self.inner.send_owned(prepared, options).await
    }

    async fn send_owned(
        &self,
        message: OutboundMessage,
        options: &SendOptions,
    ) -> Result<SendReport, TransportError> {
        let prepared = prepare_attachments(message, &self.resolver, &self.limits)
            .await
            .map_err(TransportError::from)?;
        self.inner.send_owned(prepared, options).await
    }
}

impl From<AttachmentResolveError> for TransportError {
    fn from(error: AttachmentResolveError) -> Self {
        let kind = match error.kind {
            ResolveErrorKind::UnsupportedReference => ErrorKind::UnsupportedFeature,
            ResolveErrorKind::NotFound | ResolveErrorKind::TooLarge => ErrorKind::Validation,
            ResolveErrorKind::Denied | ResolveErrorKind::Internal => ErrorKind::Internal,
            ResolveErrorKind::Transient => ErrorKind::TransientProvider,
        };
        let message = error.message().to_owned();
        Self::new(kind, message).with_source(error)
    }
}
