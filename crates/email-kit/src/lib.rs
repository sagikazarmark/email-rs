//! Convenience facade for the email-rs crates.
//!
//! `email-kit` lets applications depend on one crate while still keeping the
//! lower-level crates available through stable namespaces:
//!
//! - `attachment` re-exports `email-attachment`.
//! - Enable `attachment-opendal` for `attachment::opendal`, which re-exports
//!   `email-attachment-opendal`.
//! - `message` re-exports `email-message`.
//! - `transport` re-exports `email-transport`.
//! - Enable `transport-lettre` for `transport::lettre`, which re-exports
//!   `email-transport-lettre`.
//! - Enable `transport-resend` for `transport::resend`, which re-exports
//!   `email-transport-resend`.
//! - Enable the `wire` feature for `wire`, which re-exports
//!   `email-message-wire`.
//!
//! # Quick start
//!
//! Use [`prelude`] when you want the common message types, wire helpers, and
//! transport traits in scope:
//!
//! ```rust
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use email_kit::prelude::*;
//!
//! let mailbox: Mailbox = "Mary Smith <mary@example.com>".parse()?;
//! assert_eq!(mailbox.email().as_str(), "mary@example.com");
//! # Ok(())
//! # }
//! ```
//!
//! # Features
//!
//! The default feature set forwards the default features of the always-present
//! message and transport crates. It does not enable `wire`, an attachment
//! adapter, or a provider transport.
//!
//! - `attachment-opendal`: the OpenDAL resolver at `attachment::opendal`.
//! - `wire`: RFC 822/MIME parsing and rendering through `wire`.
//! - `transport-lettre`: the Lettre SMTP adapter at `transport::lettre`.
//! - `transport-resend`: the Resend adapter at `transport::resend`.
//! - `transport-all`: all transport adapters.
//! - `serde`, `schemars`, and `arbitrary`: forward the corresponding data-model
//!   integrations.
//! - `tracing`: transport instrumentation through
//!   `transport::TracingTransport`.
//!
//! # Platform support
//!
//! The message, wire, and core transport facades support native and
//! `wasm32` targets. The Lettre adapter, and therefore `transport-all`, does not
//! support `wasm32-unknown-unknown`. The Resend adapter supports that target,
//! but advertises and enforces per-send timeouts only on non-`wasm32` targets.
//!
//! Namespaced access stays available when that is clearer:
//!
//! ```rust
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let mailbox: email_kit::message::Mailbox = "Mary Smith <mary@example.com>"
//!     .parse()?;
//! assert_eq!(mailbox.email().as_str(), "mary@example.com");
//! # Ok(())
//! # }
//! ```
//!
//! With the `wire` feature enabled, RFC822/MIME helpers are available through
//! `email_kit::wire` and `email_kit::prelude::*`:
//!
//! ```rust
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # #[cfg(feature = "wire")]
//! # {
//! let raw = b"From: from@example.com\r\nTo: to@example.com\r\n\r\nHello";
//! let message = email_kit::wire::parse_rfc822(raw)?;
//! let _bytes = email_kit::wire::render_rfc822(&message)?;
//! # }
//! # Ok(())
//! # }
//! ```
//!
//! With the `transport-resend` or `transport-all` feature enabled,
//! Resend-specific transport types are available through
//! `email_kit::transport::resend`:
//!
//! ```rust
//! # #[cfg(any(feature = "transport-resend", feature = "transport-all"))]
//! # fn resend_example() {
//! use email_kit::transport::resend::ResendTransport;
//!
//! let transport = ResendTransport::new("re_...");
//! # let _ = transport;
//! # }
//! # #[cfg(not(any(feature = "transport-resend", feature = "transport-all")))]
//! # fn resend_example() {}
//! # resend_example();
//! ```
//!
//! With the `transport-lettre` or `transport-all` feature enabled, the SMTP
//! transport is available through `email_kit::transport::lettre`:
//!
//! ```rust,no_run
//! # #[cfg(any(feature = "transport-lettre", feature = "transport-all"))]
//! # fn lettre_example() -> Result<(), Box<dyn std::error::Error>> {
//! use email_kit::transport::lettre::{LettreTransport, lettre};
//!
//! let client = lettre::AsyncSmtpTransport::<lettre::Tokio1Executor>::builder_dangerous(
//!     "localhost",
//! )
//! .port(1025)
//! .build();
//! let transport = LettreTransport::from_client(client);
//! # let _ = transport;
//! # Ok(())
//! # }
//! # #[cfg(not(any(feature = "transport-lettre", feature = "transport-all")))]
//! # fn lettre_example() -> Result<(), Box<dyn std::error::Error>> { Ok(()) }
//! # lettre_example().expect("Lettre transport example should build");
//! ```

pub mod attachment;
pub use email_message as message;
#[cfg(feature = "wire")]
pub use email_message_wire as wire;

pub mod transport;

/// Common imports for applications using the email-rs crate family.
pub mod prelude {
    pub use email_attachment::*;
    pub use email_message::*;
    #[cfg(feature = "wire")]
    pub use email_message_wire::*;
    pub use email_transport::*;
}
