//! Restate-backed worker contracts for outbound email delivery.
//!
//! This crate exposes the serializable email worker contract and an optional
//! Restate service adapter. Provider-specific send options cross the queue
//! boundary through the registry-driven [`SendRequestSeed`], using the
//! provider-keyed wire representation owned by `email-transport`.
//!
//! The Restate service adapter is available as [`Service`] with the
//! `service` feature. The caller-side ingress transport lives in the
//! [`email-transport-restate`](https://docs.rs/email-transport-restate)
//! crate, which submits this contract through Restate ingress without
//! depending on `restate-sdk`.
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
//! # Securing the worker
//!
//! Restate signs every request it makes to an SDK endpoint when the runtime
//! is configured with a request identity key. Verify those signatures by
//! registering the corresponding public keys on the endpoint builder; once at
//! least one key is registered, unsigned requests are rejected. Registering
//! several keys keeps both the old and the new key valid during rotation:
//!
//! ```rust
//! # #[cfg(feature = "service")]
//! # fn build() -> Result<(), Box<dyn std::error::Error>> {
//! # use restate_email::{Service, StaticTransportRegistry};
//! # use restate_sdk::{endpoint::Endpoint, service::IntoServiceDefinition};
//! # let service = Service::new(StaticTransportRegistry::new()).into_service_definition();
//! let _endpoint = Endpoint::builder()
//!     .bind(service)
//!     .identity_key("publickeyv1_w7YHemBctH5Ck2nQRQ47iBBqhNHy4FV7t2Usbye2A6f")?
//!     .identity_key("publickeyv1_ChjENKeMvCtRnqG2mrBK1HmPKufgFUc98K8B3ononQvp")?
//!     .build();
//! # Ok(())
//! # }
//! ```
//!
//! Identity keys authenticate the Restate runtime to the worker; callers
//! authenticate to Restate ingress separately (see `email-transport-restate`).
//!
//! # Features
//!
//! The default feature set enables `service` for backwards-compatible worker
//! builds. Disable default features to consume only the SDK-free wire contract.
//!
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
//!   builds a worker with a custom transport and optional identity-key
//!   verification.
//! - [`restate_resend_worker`](https://github.com/sagikazarmark/email-rs/blob/main/crates/restate-email/examples/restate_resend_worker.rs)
//!   wires [`email_kit::transport::resend::ResendTransport`] into the service;
//!   it requires the `resend` feature.

mod contract;
mod options;
#[cfg(feature = "service")]
mod service;
#[cfg(feature = "service")]
pub mod transport;

// `IdempotencyKey` and `CorrelationId` live in `email-transport` because they
// flow through `SendOptions` directly.
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
