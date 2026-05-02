//! Restate-backed worker contracts for outbound email delivery.
//!
//! This crate exposes the serializable email worker contract, transport
//! resolution abstractions, and a Restate service adapter.
//! Provider-specific send options cross the queue boundary through
//! [`RawSendOptions::transport_options`], using the provider-keyed wire
//! representation owned by `email-transport`.
//!
//! The Restate service adapter is available as [`ServiceImpl`].

mod service;
pub mod transport;

// `IdempotencyKey` and `CorrelationId` live in `email-transport` because they
// flow through `SendOptions` directly.
pub use email_transport::{
    CorrelationId, IdempotencyKey, STRING_NEWTYPE_MAX_BYTES, SendOptions, StringNewtypeError,
    TransportOption, TransportOptionRegistry, TransportOptionRegistryError, TransportOptions,
    TransportOptionsSeed,
};
pub use service::{RawSendOptions, SendRequest, SendResponse, Service, ServiceImpl};
pub use transport::{
    CatchAllTransportResolver, RuntimeBound, StaticTransportRegistry, TransportKey,
    TransportLookupError, TransportResolver,
};
