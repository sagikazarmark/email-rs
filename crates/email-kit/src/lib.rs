//! Convenience facade for the email-rs crates.
//!
//! `email-kit` lets applications depend on one crate while still keeping the
//! lower-level crates available through stable namespaces:
//!
//! - `message` re-exports `email-message`.
//! - `transport` re-exports `email-transport`.
//! - Enable the `transport-resend` feature for `transport::resend`, which
//!   re-exports `email-transport-resend`.
//! - Enable the `wire` feature for `wire`, which re-exports
//!   `email-message-wire`.
//!
//! Use [`prelude`] when you want the common message types, wire helpers, and
//! transport traits in scope:
//!
//! ```rust
//! use email_kit::prelude::*;
//!
//! let mailbox: Mailbox = "Mary Smith <mary@example.com>".parse().unwrap();
//! assert_eq!(mailbox.email().as_str(), "mary@example.com");
//! ```
//!
//! Namespaced access stays available when that is clearer:
//!
//! ```rust
//! let mailbox: email_kit::message::Mailbox = "Mary Smith <mary@example.com>"
//!     .parse()
//!     .unwrap();
//! assert_eq!(mailbox.email().as_str(), "mary@example.com");
//! ```
//!
//! With the `wire` feature enabled, RFC822/MIME helpers are available through
//! `email_kit::wire` and `email_kit::prelude::*`:
//!
//! ```rust
//! # #[cfg(feature = "wire")]
//! # fn wire_example() {
//! let raw = b"From: from@example.com\r\nTo: to@example.com\r\n\r\nHello";
//! let message = email_kit::wire::parse_rfc822(raw).unwrap();
//! let _bytes = email_kit::wire::render_rfc822(&message).unwrap();
//! # }
//! # #[cfg(not(feature = "wire"))]
//! # fn wire_example() {}
//! # wire_example();
//! ```
//!
//! With the `transport-resend` feature enabled, Resend-specific transport
//! types are available through `email_kit::transport::resend`:
//!
//! ```rust
//! # #[cfg(feature = "transport-resend")]
//! # fn resend_example() {
//! use email_kit::transport::resend::ResendTransport;
//!
//! let transport = ResendTransport::new("re_...");
//! # let _ = transport;
//! # }
//! # #[cfg(not(feature = "transport-resend"))]
//! # fn resend_example() {}
//! # resend_example();
//! ```

pub use email_message as message;
#[cfg(feature = "wire")]
pub use email_message_wire as wire;

/// Core transport APIs and optional provider transports.
pub mod transport {
    pub use email_transport::*;

    #[cfg(feature = "transport-resend")]
    pub use email_transport_resend as resend;
}

/// Common imports for applications using the email-rs crate family.
pub mod prelude {
    pub use email_message::*;
    #[cfg(feature = "wire")]
    pub use email_message_wire::*;
    pub use email_transport::*;
}
