//! Attachment materialization and transport decoration for outbound email.

mod preparation;
mod resolver;
mod transport;

pub use preparation::{PreparationLimits, prepare_attachments};
pub use resolver::{
    AttachmentResolveError, AttachmentResolver, MapResolver, ResolveErrorKind, ResolvedAttachment,
    SchemeRouter,
};
pub use transport::ResolvingTransport;
