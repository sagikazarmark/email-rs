//! Lettre SMTP transport implementation for structured and raw email delivery.
//!
//! [`LettreTransport`] accepts validated structured messages through
//! [`email_transport::Transport`] and explicit envelopes with pre-rendered RFC
//! 822 bytes through [`email_transport::RawTransport`]. Structured messages are
//! rendered with `email-message-wire` before being submitted through Lettre.
//!
//! # Quick start
//!
//! ```rust,no_run
//! use email_message::{Address, Body, Message};
//! use email_transport::{SendOptions, Transport};
//! use email_transport_lettre::{LettreTransport, lettre};
//!
//! # async fn send() -> Result<(), Box<dyn std::error::Error>> {
//! let message = Message::builder(Body::text("Hello"))
//!     .from_mailbox("sender@example.com".parse()?)
//!     .to(vec![Address::Mailbox("recipient@example.com".parse()?)])
//!     .build_outbound()?;
//! let client = lettre::AsyncSmtpTransport::<lettre::Tokio1Executor>::builder_dangerous(
//!     "localhost",
//! )
//! .port(1025)
//! .build();
//! let transport = LettreTransport::from_client(client);
//!
//! transport.send(&message, &SendOptions::default()).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Features
//!
//! The default feature set enables connection pooling and Tokio-compatible
//! Rustls with WebPKI roots. `native-tls` selects Lettre's native TLS backend,
//! while disabling default features leaves unencrypted SMTP available for local
//! relays and explicitly configured clients.
//!
//! # Platform and runtime
//!
//! The adapter requires a Tokio runtime and does not support
//! `wasm32-unknown-unknown`. Construct pooled clients and call transport methods
//! from within that runtime.
//!
//! # Send options
//!
//! Per-send timeouts bound how long the caller waits. An in-flight SMTP
//! transaction continues in the background after cancellation or timeout so a
//! partially used connection cannot return to Lettre's pool. Its delivery
//! outcome is therefore indeterminate. Concurrent handoffs are bounded to
//! prevent stalled background transactions from accumulating without limit.
//! Structured sends honor envelope overrides. SMTP has no standard idempotency
//! or correlation slot, so those options are ignored.

mod transport;

/// The Lettre version used by this adapter, re-exported for advanced client
/// configuration without a second direct dependency.
pub use lettre;
pub use transport::LettreTransport;
