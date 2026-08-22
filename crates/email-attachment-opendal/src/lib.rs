//! OpenDAL-backed attachment resolution for email-rs.
//!
//! An [`OpendalResolver`] reads paths from one [`opendal::Operator`] configured
//! by the application at startup. Attachment references never configure a
//! service, endpoint, bucket, or credentials.

use email_attachment::{
    AttachmentResolveError, AttachmentResolver, ResolveErrorKind, ResolvedAttachment,
};
use email_message::AttachmentReference;
use futures_util::TryStreamExt;
use opendal::Operator;

/// Resolves attachment paths through a pre-configured OpenDAL operator.
///
/// Where the service supports `stat`, an oversized object is rejected before
/// any bytes are read. The resolver then retains at most one byte past the
/// limit, so the size policy holds even when a service reports no length or
/// reports one that has gone stale.
#[derive(Clone, Debug)]
pub struct OpendalResolver {
    operator: Operator,
    max_bytes: usize,
}

impl OpendalResolver {
    /// Create a resolver with a maximum size for each resolved attachment.
    #[must_use]
    pub const fn new(operator: Operator, max_bytes: usize) -> Self {
        Self {
            operator,
            max_bytes,
        }
    }

    /// Return the configured per-attachment byte limit.
    #[must_use]
    pub const fn max_bytes(&self) -> usize {
        self.max_bytes
    }
}

impl AttachmentResolver for OpendalResolver {
    async fn resolve(
        &self,
        reference: &AttachmentReference,
    ) -> Result<ResolvedAttachment, AttachmentResolveError> {
        // OpenDAL trims paths before dispatch, so validate and use that same form.
        let path = reference.uri().trim();
        if !is_relative_path_within_root(path) {
            return Err(AttachmentResolveError::new(
                ResolveErrorKind::UnsupportedReference,
                format!("attachment path `{path}` is not relative to the configured store root"),
            ));
        }

        // `stat` is an early rejection only. Its reported length is never used
        // as the read range: it can be stale by the time the read starts, and a
        // service that reports no length would otherwise silently truncate the
        // attachment to nothing.
        if self.operator.info().capability().stat {
            let metadata = self.operator.stat(path).await.map_err(map_error)?;
            if metadata.content_length() > self.max_bytes as u64 {
                return Err(too_large(path, self.max_bytes));
            }
        }

        // OpenDAL's AsyncRead adapter bounds `..` using stat metadata. Stream
        // the unbounded range instead, retaining only enough to enforce the
        // configured limit when that metadata is stale.
        let read_limit = self.max_bytes.saturating_add(1);
        let mut stream = self
            .operator
            .reader(path)
            .await
            .map_err(map_error)?
            .into_stream(..)
            .await
            .map_err(map_error)?;
        let mut bytes = Vec::new();
        while bytes.len() < read_limit {
            let Some(buffer) = stream.try_next().await.map_err(map_error)? else {
                break;
            };
            let remaining = read_limit - bytes.len();
            for chunk in buffer.slice(..buffer.len().min(remaining)) {
                bytes.extend_from_slice(&chunk);
            }
        }

        if bytes.len() > self.max_bytes {
            return Err(too_large(path, self.max_bytes));
        }

        Ok(ResolvedAttachment::new(bytes))
    }
}

fn is_relative_path_within_root(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with(['/', '\\'])
        && !path.contains('\0')
        && path.split(['/', '\\']).all(|segment| segment != "..")
}

fn map_error(error: opendal::Error) -> AttachmentResolveError {
    let kind = classify_error(&error);
    AttachmentResolveError::new(kind, "OpenDAL failed to resolve the attachment").with_source(error)
}

fn classify_error(error: &opendal::Error) -> ResolveErrorKind {
    if error.is_temporary() || error.kind() == opendal::ErrorKind::RateLimited {
        return ResolveErrorKind::Transient;
    }

    match error.kind() {
        opendal::ErrorKind::NotFound => ResolveErrorKind::NotFound,
        opendal::ErrorKind::PermissionDenied => ResolveErrorKind::Denied,
        opendal::ErrorKind::Unsupported
        | opendal::ErrorKind::IsADirectory
        | opendal::ErrorKind::NotADirectory => ResolveErrorKind::UnsupportedReference,
        _ => ResolveErrorKind::Internal,
    }
}

fn too_large(path: &str, max_bytes: usize) -> AttachmentResolveError {
    AttachmentResolveError::new(
        ResolveErrorKind::TooLarge,
        format!("attachment `{path}` exceeds the configured limit of {max_bytes} bytes"),
    )
}

/// The OpenDAL version used by this adapter.
pub use opendal;
