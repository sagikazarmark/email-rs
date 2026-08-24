//! Attachment materialization and transport decoration for outbound email.
//!
//! Use this crate when messages may contain opaque attachment references that
//! must be resolved before a provider transport or wire renderer receives them.
//!
//! # Quick start
//!
//! ```rust
//! use email_attachment::{AttachmentResolver, MapResolver};
//! use email_message::AttachmentReference;
//!
//! # async fn resolve() -> Result<(), Box<dyn std::error::Error>> {
//! let resolver = MapResolver::new().with_entry("report.txt", b"contents".to_vec());
//! let attachment = resolver
//!     .resolve(&AttachmentReference::new("report.txt"))
//!     .await?;
//!
//! assert_eq!(attachment.bytes, b"contents");
//! # Ok(())
//! # }
//! ```

mod preparation;
mod resolver;
mod transport;

pub use preparation::{AttachmentLimits, prepare_attachments};
pub use resolver::{
    AttachmentResolveError, AttachmentResolver, FallbackResolver, MapResolver, ResolveErrorKind,
    ResolvedAttachment, SchemeDispatch, SchemeRouter,
};
pub use transport::AttachmentResolvingTransport;
