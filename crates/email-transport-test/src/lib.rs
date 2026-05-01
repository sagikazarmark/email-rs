//! First-party test transports and conformance helpers for `email-transport`.
//!
//! `MemoryTransport` and `FileTransport` implement the full `Transport` and
//! `RawTransport` contracts so they can stand in for a real provider in
//! application tests. Enable the `conformance` feature to use the shared
//! message factory and expected-recipient list that the provider crates use
//! to assert that every adapter agrees on the cross-provider semantics.

#![deny(missing_docs)]

mod transport;

pub use transport::{CapturedPayload, CapturedSend, FileTransport, MemoryTransport};

#[cfg(feature = "conformance")]
pub mod conformance;
