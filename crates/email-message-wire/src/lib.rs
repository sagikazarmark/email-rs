//! RFC822/MIME wire parsing and rendering for `email-message`.
//!
//! This crate focuses on wire format concerns and keeps `email-message` focused on
//! outbound message representation.
//!
//! ```rust
//! use email_message_wire::{parse_rfc822, render_rfc822};
//!
//! let raw = b"From: from@example.com\r\nTo: to@example.com\r\n\r\nHello";
//! let message = parse_rfc822(raw).unwrap();
//! let _bytes = render_rfc822(&message).unwrap();
//! ```
//!
//! Current rendering support:
//! - Structured body rendering for text, html, text+html, and MIME trees.
//! - `Message::attachments` rendered as MIME parts with multipart nesting, base64 transfer
//!   encoding, Content-Disposition, and optional Content-ID.
//! - RFC2231 `filename*=` parameter emitted for non-ASCII attachment filenames.
//! - Attachment references are model-level values and must be resolved to bytes before
//!   rendering.
//!
//! # Feature interaction with `email-message`
//!
//! Depending on `email-message-wire` enables `email-message`'s `mime`
//! feature for the calling crate as a side effect, `MimePart` becomes
//! visible in `email_message`. The wire crate genuinely needs `mime`
//! for full MIME-tree rendering, so this is unavoidable; downstream
//! crates that want `MimePart` access have an alternative path through
//! the wire dependency.
//!
//! # Parser semantics
//!
//! See [`parse_rfc822`] for the full decoding contract. Highlights:
//! - Body charsets outside `utf-8`/`us-ascii`/`iso-8859-1`/`latin1` are
//!   decoded with `String::from_utf8_lossy`, invalid bytes become
//!   `U+FFFD` rather than producing an error.
//! - Encoded words in unsupported charsets pass through as the raw
//!   `=?…?=` literal.
//! - Duplicate `To:`/`Cc:`/`Bcc:`/`Reply-To:` lines are merged.
//! - RFC 6532 (SMTPUTF8) inbound is not supported; non-ASCII header
//!   lines fail.
//! - The returned `Message` has not been validated for outbound
//!   delivery, wrap via `OutboundMessage::new` if you intend to send
//!   it through a `Transport`.

mod rfc822;

pub use rfc822::{
    MAX_INPUT_BYTES, MAX_MULTIPART_DEPTH, MAX_MULTIPART_PARTS, MessageParseError,
    MessageRenderError, RenderOptions, decode_rfc2047_phrase, parse_rfc822, render_rfc822,
    render_rfc822_with,
};
