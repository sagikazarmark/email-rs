//! Core attachment APIs and optional resolver adapters.

pub use email_attachment::*;

#[cfg(feature = "attachment-opendal")]
pub use email_attachment_opendal as opendal;
