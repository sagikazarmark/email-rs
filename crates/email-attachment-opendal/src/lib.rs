//! OpenDAL-backed attachment resolution for email-rs.
//!
//! An [`OpendalResolver`] reads paths from one [`opendal::Operator`] configured
//! by the application at startup. Attachment references never configure a
//! service, endpoint, bucket, or credentials.

use email_attachment::{
    AttachmentResolveError, AttachmentResolver, ResolveErrorKind, ResolvedAttachment,
};
use email_message::AttachmentReference;
use futures_util::io::AsyncReadExt;
use opendal::Operator;

/// Resolves attachment paths through a pre-configured OpenDAL operator.
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

        let bytes = if self.operator.info().capability().stat {
            let metadata = self.operator.stat(path).await.map_err(map_error)?;
            if metadata.content_length() > self.max_bytes as u64 {
                return Err(too_large(path, self.max_bytes));
            }
            self.operator
                .reader(path)
                .await
                .map_err(map_error)?
                .read(0..metadata.content_length())
                .await
                .map_err(map_error)?
                .to_vec()
        } else {
            let read_limit = self.max_bytes.saturating_add(1);
            let reader = self
                .operator
                .reader_with(path)
                .chunk(read_limit.max(1))
                .await
                .map_err(map_error)?
                .into_futures_async_read(..)
                .await
                .map_err(map_error)?;
            let mut reader = reader.take(read_limit as u64);
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).await.map_err(map_io_error)?;
            bytes
        };
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

fn map_io_error(error: std::io::Error) -> AttachmentResolveError {
    let kind = error
        .get_ref()
        .and_then(|source| source.downcast_ref::<opendal::Error>())
        .map(classify_error)
        .unwrap_or_else(|| match error.kind() {
            std::io::ErrorKind::NotFound => ResolveErrorKind::NotFound,
            std::io::ErrorKind::PermissionDenied => ResolveErrorKind::Denied,
            std::io::ErrorKind::InvalidInput | std::io::ErrorKind::Unsupported => {
                ResolveErrorKind::UnsupportedReference
            }
            std::io::ErrorKind::Interrupted
            | std::io::ErrorKind::TimedOut
            | std::io::ErrorKind::WouldBlock => ResolveErrorKind::Transient,
            _ => ResolveErrorKind::Internal,
        });
    AttachmentResolveError::new(kind, "OpenDAL failed while reading the attachment")
        .with_source(error)
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
