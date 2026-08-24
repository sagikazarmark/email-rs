//! Restate-backed worker contracts for outbound email delivery.
//!
//! This crate exposes the serializable email worker contract, an optional
//! caller-side ingress transport, and an optional Restate service adapter.
//! Provider-specific send options cross the queue boundary through the
//! registry-driven [`SendRequestSeed`], using the provider-keyed wire
//! representation owned by `email-transport`.
//!
//! The Restate service adapter is available as [`Service`] with the
//! `service` feature. [`RestateTransport`] is available with `client`; it
//! returns as soon as Restate has queued the invocation by default and can be
//! asked to wait for the worker's provider report with [`InvocationMode::Sent`],
//! either as a transport default or per send through [`RestateSendOptions`].
//!
//! # Quick start
//!
//! ```rust
//! # #[cfg(feature = "service")]
//! # {
//! use restate_email::{Service, StaticTransportRegistry};
//! use restate_sdk::{endpoint::Endpoint, service::IntoServiceDefinition};
//!
//! let registry = StaticTransportRegistry::new();
//! let service = Service::new(registry).into_service_definition();
//! let _endpoint = Endpoint::builder().bind(service).build();
//! # }
//! ```
//!
//! # Features
//!
//! The default feature set enables `service` for backwards-compatible worker
//! builds. Disable default features to consume only the SDK-free wire contract.
//!
//! - `client`: caller-side [`email_transport::Transport`] implementation using
//!   Restate ingress (`/restate/send/Email/send` and `/restate/call/Email/send`).
//!   This does not enable `restate-sdk`.
//! - `service`: Restate worker service adapter and transport registry.
//! - `resend`: registers Resend provider options and enables the Resend worker
//!   example.
//! - `schemars`: JSON Schema implementations and examples for queue contracts.
//! - `rfc5322-string-compat`: accepts the legacy RFC 5322 string address shape
//!   supported by `email-message`.
//!
//! # Platform support
//!
//! This crate targets native Restate workers. Transport implementations may
//! support additional platforms, but [`Service`] depends on Restate's
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
//!   invokes `Email.send` through Restate ingress and waits for the response.
//! - [`direct_or_restate`](https://github.com/sagikazarmark/email-rs/blob/main/crates/restate-email/examples/direct_or_restate.rs)
//!   sends through the same application function using either a direct
//!   provider transport or `RestateTransport`.

#[cfg(feature = "client")]
mod client;
mod contract;
mod options;
#[cfg(feature = "service")]
mod service;
#[cfg(feature = "service")]
pub mod transport;

// `IdempotencyKey` and `CorrelationId` live in `email-transport` because they
// flow through `SendOptions` directly.
#[cfg(feature = "client")]
pub use client::{RestateTransport, RestateTransportBuilder};
pub use contract::{SendRequest, SendRequestSeed, SendResponse, TransportKey};
pub use email_transport::{
    CorrelationId, IdempotencyKey, STRING_NEWTYPE_MAX_BYTES, SendOptions, StringNewtypeError,
    TransportOption, TransportOptionRegistry, TransportOptionRegistryError, TransportOptions,
    TransportOptionsSeed,
};
pub use options::{InvocationMode, RestateSendOptions};
#[cfg(feature = "service")]
pub use service::{Service, ServiceClient};
#[cfg(feature = "service")]
pub use transport::{
    CatchAllTransportResolver, RuntimeBound, StaticTransportRegistry, TransportLookupError,
    TransportResolver,
};
