//! Resend transport implementation for structured outbound email delivery.
//!
//! This crate maps [`email_message::Message`] values to Resend's official
//! `resend-rs` SDK and exposes Resend-specific typed send options through
//! [`email_transport::TransportOptions`].
//!
//! # Quick start
//!
//! ```rust,no_run
//! use email_message::{Address, Body, Message};
//! use email_transport::{SendOptions, Transport};
//! use email_transport_resend::ResendTransport;
//!
//! # async fn send() -> Result<(), Box<dyn std::error::Error>> {
//! let message = Message::builder(Body::text("Welcome"))
//!     .from_mailbox("sender@example.com".parse()?)
//!     .to(vec![Address::Mailbox("recipient@example.com".parse()?)])
//!     .build_outbound()?;
//!
//! ResendTransport::new("re_...")
//!     .send(&message, &SendOptions::default())
//!     .await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Features
//!
//! The default feature set enables the default `reqwest` client stack and
//! `serde`. Disable default features to choose `reqwest`'s transport/TLS
//! features explicitly. Resend option types always implement
//! [`serde::Serialize`] so feature unification remains additive. The `serde`
//! feature enables their queue/wire deserialization through `email-transport`.
//!
//! # Platform support
//!
//! The adapter can be compiled for native and `wasm32` targets supported by
//! the selected `reqwest` backend. [`email_transport::SendOptions::timeout`] is
//! enforced and advertised only on non-`wasm32` targets; it is ignored on
//! `wasm32`.
//!
//! # Example program
//!
//! The canonical [`resend_send` example](https://github.com/sagikazarmark/email-rs/blob/main/crates/email-transport-resend/examples/resend_send.rs)
//! constructs a validated message, applies idempotency and provider options,
//! sends it with [`ResendTransport`], and prints the resulting report. It reads
//! credentials and the recipient from `RESEND_API_KEY` and `RESEND_TO`.
//!
mod options;
mod transport;

pub use options::{ResendSendOptions, ResendTag, ResendTemplate};
pub use transport::{ResendTransport, ResendTransportBuilder};
