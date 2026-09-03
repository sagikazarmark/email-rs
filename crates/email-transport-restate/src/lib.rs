//! Restate ingress transport for outbound email delivery.
//!
//! This crate implements [`email_transport::Transport`] on top of a Restate
//! `Email.send` ingress invocation, so the same application code can hand a
//! message to a durable queue instead of a provider. The wire contract it
//! submits is owned by [`restate-email`](https://docs.rs/restate-email); the
//! worker side of the contract lives there as well. Depending on this crate
//! never pulls in the Restate SDK.
//!
//! [`RestateTransport`] follows a send as far as its [`InvocationMode`] says:
//! in the default [`InvocationMode::Queued`] mode it returns once Restate has
//! durably accepted the invocation and reports the invocation id; in
//! [`InvocationMode::Sent`] mode it waits for the worker and returns the
//! worker's provider report. A per-send [`RestateSendOptions`] overrides the
//! configured mode and may delay a queued invocation.
//!
//! # Quick start
//!
//! ```rust,no_run
//! use email_message::{Address, Body, Message};
//! use email_transport::{SendOptions, Transport};
//! use email_transport_restate::{RestateTransport, TransportKey};
//!
//! # async fn send() -> Result<(), Box<dyn std::error::Error>> {
//! let message = Message::builder(Body::text("Welcome"))
//!     .from_mailbox("sender@example.com".parse()?)
//!     .to(vec![Address::Mailbox("recipient@example.com".parse()?)])
//!     .build_outbound()?;
//!
//! let transport = RestateTransport::new(
//!     TransportKey::new("transactional")?,
//!     "http://127.0.0.1:8080".parse()?,
//! );
//! transport.send(&message, &SendOptions::default()).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Authentication
//!
//! Restate Cloud ingress requires an API key as a bearer token; configure it
//! with [`RestateTransportBuilder::bearer_token`]. Self-hosted Restate has no
//! built-in ingress authentication; a fronting reverse proxy that expects
//! the same `Authorization: Bearer` header is covered by the same setter,
//! and any other header scheme can be attached to a custom
//! [`reqwest::Client`] passed to [`RestateTransportBuilder::client`].
//!
//! # Features
//!
//! The default feature set enables the default `reqwest` client stack.
//! Disable default features to choose `reqwest`'s transport/TLS features
//! explicitly.
//!
//! - `schemars`: forwards JSON Schema derivation to the re-exported
//!   `restate-email` queue payload types.
//!
//! # Compatibility
//!
//! The transport targets the `/restate/call` and `/restate/send` ingress
//! paths introduced in Restate 1.7. Telling a terminal worker error apart
//! from an ingress-level failure relies on the `x-restate-error-source`
//! header and the `source`/`code` fields of the error body, which Restate
//! emits since 1.7.4; against 1.7.0 through 1.7.3 a terminal worker failure
//! is reported as a retryable [`email_transport::ErrorKind::TransientProvider`].
//!
//! # Platform support
//!
//! The transport can be compiled for native and `wasm32` targets supported by
//! the selected `reqwest` backend.
//!
//! # Example programs
//!
//! - [`invoke_local_worker`](https://github.com/sagikazarmark/email-rs/blob/main/crates/email-transport-restate/examples/invoke_local_worker.rs)
//!   invokes `Email.send` through Restate ingress with a raw HTTP client and
//!   waits for the response.
//! - [`direct_or_restate`](https://github.com/sagikazarmark/email-rs/blob/main/crates/email-transport-restate/examples/direct_or_restate.rs)
//!   sends through the same application function using either a direct
//!   provider transport or [`RestateTransport`].

mod transport;

pub use restate_email::{
    InvocationMode, RestateSendOptions, SendRequest, SendRequestSeed, SendResponse, TransportKey,
};
pub use transport::{RestateTransport, RestateTransportBuilder};
