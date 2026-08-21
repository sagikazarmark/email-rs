//! Lettre SMTP transport implementation for structured and raw email delivery.
//!
//! [`LettreTransport`] accepts validated structured messages through
//! [`email_transport::Transport`] and explicit envelopes with pre-rendered RFC
//! 822 bytes through [`email_transport::RawTransport`]. Structured messages are
//! rendered with `email-message-wire` before being submitted through Lettre.
//!
//! # Features
//!
//! The default feature set enables connection pooling and Tokio-compatible
//! Rustls with WebPKI roots. `native-tls` selects Lettre's native TLS backend,
//! while disabling default features leaves unencrypted SMTP available for local
//! relays and explicitly configured clients.
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
