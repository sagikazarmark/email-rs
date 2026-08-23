//! Typed email addresses, message content, and validated outbound messages.
//!
//! Use this crate to construct provider-independent email values. RFC 822/MIME
//! byte parsing and rendering are intentionally handled by
//! `email-message-wire`, while transport crates apply provider-specific limits
//! and delivery policies.
//!
//! # Quick start
//!
//! ```rust
//! use email_message::{Address, Body, Message};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let message = Message::builder(Body::text("Hello"))
//!     .from_mailbox("sender@example.com".parse()?)
//!     .to(vec![Address::Mailbox("recipient@example.com".parse()?)])
//!     .subject("Welcome")
//!     .build_outbound()?;
//!
//! assert_eq!(message.as_message().subject(), Some("Welcome"));
//! # Ok(())
//! # }
//! ```
//!
//! # Cargo features
//!
//! No features are enabled by default.
//!
//! - `mime` exposes [`MimePart`], the low-level MIME tree. The other MIME value
//!   types are always available. The `email-message-wire` crate enables this
//!   feature on its dependency because rendering needs the tree.
//! - `serde` implements serialization and deserialization for public model
//!   types. Binary attachment and MIME bodies use padded base64 strings.
//! - `schemars` implements JSON Schema generation. When combined with `mime`,
//!   schemas include [`MimePart`].
//! - `arbitrary` implements `arbitrary::Arbitrary` for generated test data,
//!   including feature-gated model types that are also enabled.
//! - `rfc5322-string-compat` lets `serde` deserializers and `schemars` schemas
//!   accept RFC 5322 address strings in addition to the typed object shape. It
//!   has no effect unless `serde` or `schemars` is also enabled.
//!
//! All features are additive and may be enabled together.
//!
//! # Platform support
//!
//! This is a `std` crate with no operating-system APIs or target-specific
//! implementation. It supports Rust targets that provide the standard library.

/// RFC 5322 mailboxes, groups, addresses, and address lists.
pub mod address;
/// Validated RFC 5322 `addr-spec` email addresses.
pub mod email;
/// Message bodies, attachments, headers, builders, and outbound validation.
pub mod message;
/// Validated RFC 5322 `Message-ID` values.
pub mod message_id;
pub mod mime_types;

pub use address::{
    Address, AddressBackendError, AddressList, AddressParseError, Group, GroupParseError,
    MAX_ADDRESS_INPUT_BYTES, Mailbox, MailboxList, MailboxParseError,
};
pub use email::{EmailAddress, EmailAddressParseError};
pub use message::{
    Attachment, AttachmentBody, AttachmentReference, Body, Disposition, Envelope, Header,
    HeaderValidationError, Message, MessageBuilder, MessageValidationError, OutboundMessage,
};
pub use message_id::{MessageId, MessageIdParseError};

pub use mime_types::{
    ContentDisposition, ContentDispositionParseError, ContentTransferEncoding,
    ContentTransferEncodingParseError, ContentType, ContentTypeParseError, MediaType,
    ParameterValue,
};

#[cfg(feature = "mime")]
pub use mime_types::MimePart;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
/// Error returned when parsing one of the crate's core string value types.
pub enum ParseError {
    /// An email address failed validation.
    #[error(transparent)]
    EmailAddress(#[from] EmailAddressParseError),
    /// A mailbox failed to parse.
    #[error(transparent)]
    Mailbox(#[from] MailboxParseError),
    /// An address group failed to parse.
    #[error(transparent)]
    Group(#[from] GroupParseError),
    /// A mailbox-or-group address failed to parse.
    #[error(transparent)]
    Address(#[from] AddressParseError),
}
