//! Parses and renders `email-message` values as RFC 822/MIME bytes.
//!
//! Use this crate at wire-format boundaries such as SMTP submission, `.eml`
//! import and export, and MIME processing. The `email-message` crate remains
//! responsible for the provider-independent model and outbound validation.
//!
//! # Quick start
//!
//! ```rust
//! use email_message_wire::{parse_rfc822, render_rfc822};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let raw = b"From: from@example.com\r\nTo: to@example.com\r\n\r\nHello";
//! let message = parse_rfc822(raw)?;
//! let _bytes = render_rfc822(&message)?;
//! # Ok(())
//! # }
//! ```
//!
//! # Rendering support
//!
//! - Structured body rendering for text, html, text+html, and MIME trees.
//! - `Message::attachments` rendered as MIME parts with multipart nesting, base64 transfer
//!   encoding, Content-Disposition, and optional Content-ID.
//! - RFC2231 `filename*=` parameter emitted for non-ASCII attachment filenames.
//! - Attachment references are model-level values and must be resolved to bytes before
//!   rendering.
//!
//! # Cargo features
//!
//! This crate has no optional features, and its default feature set is empty;
//! default and all-feature builds are therefore equivalent. Its required
//! `email-message` dependency always enables that crate's `mime` feature, so
//! [`email_message::MimePart`] is also available when both crates occur in the
//! same dependency graph. Other `email-message` features remain independent.
//!
//! # Platform support
//!
//! This is a `std` crate with no operating-system APIs or target-specific
//! implementation. Parsing and rendering operate entirely on in-memory bytes
//! and support Rust targets that provide the standard library.
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
