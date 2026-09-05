//! Cloudflare Workers `send_email` transport implementation for structured
//! outbound email delivery.
//!
//! This crate maps [`email_message::Message`] values to Cloudflare's
//! structured send API and dispatches them through the `worker` crate's
//! [`worker::SendEmail`] binding. Application code written against
//! [`email_transport::Transport`] runs unchanged inside a Cloudflare Worker.
//!
//! # Quick start
//!
//! ```rust,no_run
//! use email_message::{Address, Body, Message};
//! use email_transport::{SendOptions, Transport};
//! use email_transport_cloudflare::CloudflareTransport;
//!
//! # async fn send(env: &worker::Env) -> Result<(), Box<dyn std::error::Error>> {
//! let message = Message::builder(Body::text("Welcome"))
//!     .from_mailbox("Sender <sender@yourdomain.com>".parse()?)
//!     .to(vec![Address::Mailbox("recipient@example.com".parse()?)])
//!     .subject("Hello")
//!     .build_outbound()?;
//!
//! let report = CloudflareTransport::from_env(env, "EMAIL")?
//!     .send(&message, &SendOptions::default())
//!     .await?;
//! println!("cloudflare message id: {:?}", report.provider_message_id);
//! # Ok(())
//! # }
//! ```
//!
//! The binding name is the `name` of a `[[send_email]]` entry in
//! `wrangler.toml`. An already-obtained [`worker::SendEmail`] can be passed to
//! [`CloudflareTransport::new`] instead.
//!
//! # Mapping
//!
//! - `From`, `To`, `Cc`, `Bcc` and `Reply-To` keep their display names.
//!   Address groups are flattened to their member mailboxes. At least one
//!   `To`/`Cc`/`Bcc` recipient is required; cc-only and bcc-only messages are
//!   sent (the platform requires at least one of the three lists, not `To`
//!   specifically). Cloudflare accepts a single `Reply-To`; more than one
//!   fails with [`email_transport::ErrorKind::UnsupportedFeature`].
//! - `Body::Text`, `Body::Html` and `Body::TextAndHtml` map to Cloudflare's
//!   `text`/`html` fields. At least one must be non-empty; a hand-built MIME
//!   body is unsupported.
//! - Byte-backed attachments are forwarded with their filename, content type
//!   and disposition, and content is passed as a typed array, not base64.
//!   Cloudflare requires a filename on every attachment: a regular attachment
//!   without one is named `attachment-N` by its 1-based position, an inline
//!   one takes its content id. Inline attachments must carry a content id
//!   (`Validation` otherwise); a content id on a regular attachment is dropped
//!   because the platform accepts none there. Attachment references must be
//!   materialised first, for example with
//!   `email_attachment::AttachmentResolvingTransport`.
//! - Custom headers (`X-*`, `List-Unsubscribe`, `In-Reply-To`, ...) are
//!   forwarded verbatim. Cloudflare's `headers` field is a plain object that
//!   admits each name once, so names that repeat (compared case-insensitively,
//!   per RFC 5322) collapse to the last value under the first spelling seen.
//!   **The message's `date`, `message_id` and `sender` are dropped**: Cloudflare
//!   rejects `Date` and `Message-ID` with `E_HEADER_NOT_ALLOWED` and stamps its
//!   own `Message-ID`, which is returned as
//!   [`email_transport::SendReport::provider_message_id`].
//! - A missing subject is sent as an empty string.
//!
//! [`email_transport::SendReport::provider`] is always [`PROVIDER`]
//! (`"cloudflare"`).
//!
//! # Capabilities
//!
//! Structured send, custom headers, attachments and inline attachments are
//! advertised. Idempotency keys and timeouts are not: the binding has neither,
//! so [`email_transport::SendOptions::idempotency_key`] and
//! [`email_transport::SendOptions::timeout`] are accepted and ignored.
//!
//! # Errors
//!
//! Cloudflare error codes are classified into
//! [`email_transport::ErrorKind`] and carried verbatim as
//! [`email_transport::TransportError::provider_error_code`]:
//!
//! | Cloudflare code | `ErrorKind` |
//! |---|---|
//! | `E_VALIDATION_ERROR`, `E_FIELD_MISSING`, `E_TOO_MANY_RECIPIENTS`, `E_TOO_MANY_ATTACHMENTS`, `E_CONTENT_TOO_LARGE`, `E_HEADER*` | `Validation` |
//! | `E_SENDER_NOT_VERIFIED`, `E_SENDER_DOMAIN_NOT_AVAILABLE`, `E_RECIPIENT_NOT_ALLOWED`, `RCPT_NOT_ALLOWED` | `Authorization` |
//! | `E_RECIPIENT_SUPPRESSED`, any unrecognised code | `PermanentProvider` |
//! | `E_RATE_LIMIT_EXCEEDED`, `E_DAILY_LIMIT_EXCEEDED` | `RateLimited` |
//! | `E_INTERNAL_SERVER_ERROR`, `E_DELIVERY_FAILED` | `TransientProvider` |
//! | JS error without a `code` (or with an empty one) | `Internal` |
//!
//! # Platform constraints
//!
//! The sender domain must be verified with Cloudflare Email Service. At the
//! time of writing the platform allows 50 recipients across `To`/`Cc`/`Bcc`,
//! 32 attachments, 5 MiB total message size, a header allowlist for custom
//! headers and 16 KB of custom headers in total. None of these limits are
//! enforced client-side; the platform's error codes are mapped instead.
//!
//! A binding may also be restricted in `wrangler.toml` with
//! `allowed_sender_addresses`, `allowed_destination_addresses` or a single
//! `destination_address`. Those restrictions are likewise enforced by the
//! platform and surface as `Authorization` errors. The `destination_address`
//! form, which lets JavaScript callers omit `to`, is not reachable here: a
//! message with no recipient mailboxes is rejected locally with `Validation`.
//!
//! # Platform support
//!
//! The binding only functions on `wasm32-unknown-unknown` inside `workerd`.
//! The crate compiles on native targets so workspace tests run everywhere,
//! but [`email_transport::Transport::send`] returns an `UnsupportedFeature`
//! error there instead of reaching wasm-bindgen's panicking extern stubs.

mod transport;

pub use transport::{CloudflareTransport, PROVIDER};
