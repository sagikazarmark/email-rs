//! Restate-backed worker contracts for outbound email delivery.
//!
//! This crate exposes the serializable email worker contract, transport
//! resolution abstractions, and a Restate service adapter.
//! Provider-specific send options cross the queue boundary through
//! [`RawSendOptions::transport_options`], using the provider-keyed wire
//! representation owned by `email-transport`.
//!
//! The Restate service adapter is available as [`ServiceImpl`].
//!
//! # Features
//!
//! The default feature set enables the default `reqwest` client stack used by
//! the local invocation tooling.
//!
//! - `resend`: registers Resend provider options and enables the Resend worker
//!   example.
//! - `schemars`: JSON Schema implementations and examples for queue contracts.
//! - `rfc5322-string-compat`: accepts the legacy RFC 5322 string address shape
//!   supported by `email-message`.
//!
//! # Platform support
//!
//! This crate targets native Restate workers. Transport implementations may
//! support additional platforms, but [`ServiceImpl`] depends on Restate's
//! native endpoint/runtime integration.
//!
//! # Examples
//!
//! - [`restate_email_worker`](https://github.com/sagikazarmark/email-rs/blob/main/crates/restate-email/examples/restate_email_worker.rs)
//!   builds a worker with a custom transport.
//! - [`restate_resend_worker`](https://github.com/sagikazarmark/email-rs/blob/main/crates/restate-email/examples/restate_resend_worker.rs)
//!   wires [`email_kit::transport::resend::ResendTransport`] into the service;
//!   it requires the `resend` feature.
//! - [`invoke_local_worker`](https://github.com/sagikazarmark/email-rs/blob/main/crates/restate-email/examples/invoke_local_worker.rs)
//!   invokes `Email.send` through Restate ingress and validates the response.

mod service;
pub mod transport;

// `IdempotencyKey` and `CorrelationId` live in `email-transport` because they
// flow through `SendOptions` directly.
pub use email_transport::{
    CorrelationId, IdempotencyKey, STRING_NEWTYPE_MAX_BYTES, SendOptions, StringNewtypeError,
    TransportOption, TransportOptionRegistry, TransportOptionRegistryError, TransportOptions,
    TransportOptionsSeed,
};
pub use service::{RawSendOptions, SendRequest, SendResponse, ServiceImpl, ServiceImplClient};
pub use transport::{
    CatchAllTransportResolver, RuntimeBound, StaticTransportRegistry, TransportKey,
    TransportLookupError, TransportResolver,
};
