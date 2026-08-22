use email_message::{AttachmentBody, OutboundMessage};

use crate::{AttachmentResolveError, AttachmentResolver, ResolveErrorKind};

/// Size policy applied while materializing attachment references.
///
/// The default is unlimited. Deployments opt into limits according to their
/// storage, memory, and provider constraints.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PreparationLimits {
    /// Maximum bytes allowed for one attachment.
    pub max_attachment_bytes: Option<usize>,
    /// Maximum bytes allowed across all attachments in the message.
    pub max_total_bytes: Option<usize>,
}

impl PreparationLimits {
    /// Create an unlimited preparation policy.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            max_attachment_bytes: None,
            max_total_bytes: None,
        }
    }

    /// Set the per-attachment byte limit.
    #[must_use]
    pub const fn with_max_attachment_bytes(mut self, limit: Option<usize>) -> Self {
        self.max_attachment_bytes = limit;
        self
    }

    /// Set the total attachment byte limit.
    #[must_use]
    pub const fn with_max_total_bytes(mut self, limit: Option<usize>) -> Self {
        self.max_total_bytes = limit;
        self
    }
}

/// Materialize every reference-backed attachment in `message`.
///
/// Byte-backed attachments and all attachment metadata are preserved. If the
/// message contains no references, it is returned unchanged.
///
/// # Errors
///
/// Returns the resolver's classified error, or [`ResolveErrorKind::TooLarge`]
/// when the prepared message exceeds `limits`.
pub async fn prepare_attachments<R: AttachmentResolver>(
    message: OutboundMessage,
    resolver: &R,
    limits: &PreparationLimits,
) -> Result<OutboundMessage, AttachmentResolveError> {
    if !has_attachment_references(&message) {
        return Ok(message);
    }

    let (message, mut attachments) = message.into_message().into_attachments();
    let mut total_bytes = 0usize;

    for attachment in &attachments {
        if let AttachmentBody::Bytes(bytes) = attachment.body() {
            enforce_attachment_limit(bytes.len(), limits)?;
            total_bytes = checked_total(total_bytes, bytes.len(), limits)?;
        }
    }

    for attachment in &mut attachments {
        let reference = match attachment.body() {
            AttachmentBody::Bytes(_) => continue,
            AttachmentBody::Reference(reference) => reference.clone(),
            _ => {
                return Err(AttachmentResolveError::new(
                    ResolveErrorKind::Internal,
                    "unsupported attachment body variant during preparation",
                ));
            }
        };
        let resolved = resolver.resolve(&reference).await?;
        enforce_attachment_limit(resolved.bytes.len(), limits)?;
        total_bytes = checked_total(total_bytes, resolved.bytes.len(), limits)?;
        attachment.set_body(AttachmentBody::Bytes(resolved.bytes));
    }

    OutboundMessage::new(message.with_attachments(attachments)).map_err(|error| {
        let message = format!("prepared outbound message failed validation: {error}");
        AttachmentResolveError::new(ResolveErrorKind::Internal, message).with_source(error)
    })
}

pub(crate) fn has_attachment_references(message: &OutboundMessage) -> bool {
    message
        .as_message()
        .attachments()
        .iter()
        .any(|attachment| matches!(attachment.body(), AttachmentBody::Reference(_)))
}

fn enforce_attachment_limit(
    bytes: usize,
    limits: &PreparationLimits,
) -> Result<(), AttachmentResolveError> {
    if limits
        .max_attachment_bytes
        .is_some_and(|limit| bytes > limit)
    {
        return Err(AttachmentResolveError::new(
            ResolveErrorKind::TooLarge,
            format!("attachment size {bytes} exceeds the configured per-attachment limit"),
        ));
    }
    Ok(())
}

fn checked_total(
    current: usize,
    additional: usize,
    limits: &PreparationLimits,
) -> Result<usize, AttachmentResolveError> {
    let total = current.checked_add(additional).ok_or_else(|| {
        AttachmentResolveError::new(
            ResolveErrorKind::TooLarge,
            "total attachment size overflowed",
        )
    })?;
    if limits.max_total_bytes.is_some_and(|limit| total > limit) {
        return Err(AttachmentResolveError::new(
            ResolveErrorKind::TooLarge,
            format!("total attachment size {total} exceeds the configured limit"),
        ));
    }
    Ok(total)
}
