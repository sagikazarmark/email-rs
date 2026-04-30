//! Core email address and outbound message model primitives.
//!
//! Quick examples:
//!
//! ```rust
//! use email_message::{Mailbox, MailboxList};
//!
//! let mailbox: Mailbox = "Mary Smith <mary@x.test>".parse().unwrap();
//! assert_eq!(mailbox.name(), Some("Mary Smith"));
//! assert_eq!(mailbox.email().as_str(), "mary@x.test");
//!
//! let list: MailboxList = "mary@x.test, jdoe@one.test".parse().unwrap();
//! assert_eq!(list.len(), 2);
//! ```
//!
//! Scope contract:
//! - This crate models outbound email content and addresses.
//! - RFC822/MIME wire parsing and rendering live in `email-message-wire`.
//! - Provider-specific limits and operational policies belong to transport crates.

pub mod address;
pub mod email;
pub mod message;
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
pub enum ParseError {
    #[error(transparent)]
    EmailAddress(#[from] EmailAddressParseError),
    #[error(transparent)]
    Mailbox(#[from] MailboxParseError),
    #[error(transparent)]
    Group(#[from] GroupParseError),
    #[error(transparent)]
    Address(#[from] AddressParseError),
}
