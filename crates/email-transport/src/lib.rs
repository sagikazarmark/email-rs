//! Core transport traits and per-send option types.
//!
//! `Transport` models providers that accept structured [`email_message::Message`] values,
//! while `RawTransport` is for providers such as SMTP that send an explicit envelope plus
//! RFC822 bytes. Provider-specific options are carried through [`TransportOptions`] so the
//! core API stays transport-agnostic.
//!
//! # Adapter helpers (public API)
//!
//! Several utility functions are deliberately public so adapter crates can
//! share kernel logic without re-implementing it. They are part of the 1.0
//! semver surface:
//!
//! - [`accepted_recipient_emails`], derive the `To + Cc + Bcc` envelope
//!   recipient list from a [`Message`].
//! - [`structured_accepted_for`], populate [`SendReport::accepted`] in a
//!   structured-`Transport` adapter, honoring a [`SendOptions::envelope`]
//!   override only when the adapter advertises
//!   [`Capabilities::custom_envelope`].
//! - [`standard_message_headers`], render `Sender`, `Date`, and
//!   `Message-ID` as a `Vec<Header>` for adapters whose provider API treats
//!   them as custom headers.
//!
//! [`Message`]: email_message::Message

pub mod options;
pub mod string_newtype;
#[cfg(feature = "tracing")]
pub mod tracing;
pub mod transport;

pub use string_newtype::{STRING_NEWTYPE_MAX_BYTES, StringNewtypeError};
#[cfg(feature = "tracing")]
pub use tracing::TracingTransport;
pub use transport::*;

/// Re-export of [`email_message`] so leaf consumers can reach the typed
/// outbound message and address model through a single dependency on
/// `email-transport`. Provided as a facade, the `email-message` crate
/// remains separately publishable and consumable.
pub use email_message;

/// Re-export of the `serde` crate so the [`string_newtype!`] macro can
/// reach into serde's traits via `$crate::__macro_serde::*` instead of
/// the bare `::serde` path when the `serde` feature is enabled. Downstream
/// users who invoke [`string_newtype!`] therefore do **not** need to declare
/// `serde` as a direct dependency themselves; it flows through
/// `email-transport`'s re-export.
///
/// Not part of the curated rustdoc surface; do not name directly.
#[cfg(feature = "serde")]
#[doc(hidden)]
pub use serde as __macro_serde;

/// Re-export of the `schemars` crate so the [`string_newtype!`] macro can
/// generate [`schemars::JsonSchema`] impls without requiring downstream macro
/// users to declare `schemars` as a direct dependency.
///
/// Not part of the curated rustdoc surface; do not name directly.
#[cfg(feature = "schemars")]
#[doc(hidden)]
pub use schemars as __macro_schemars;
