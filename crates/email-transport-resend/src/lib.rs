//! Resend transport implementation for structured outbound email delivery.
//!
//! This crate maps [`email_message::Message`] values to Resend's official
//! `resend-rs` SDK and exposes Resend-specific typed send options through
//! [`email_transport::TransportOptions`].
//!
//! # Example
//!
//! ```no_run
//! use email_message::{Address, Body, Message};
//! use email_transport::{SendOptions, Transport, TransportOptions};
//! use email_transport_resend::{ResendSendOptions, ResendTransport};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let message = Message::builder(Body::text("Welcome"))
//!     .from_mailbox("sender@example.com".parse()?)
//!     .to(vec![Address::Mailbox("recipient@example.com".parse()?)])
//!     .subject("Hello")
//!     .build_outbound()?;
//!
//! let mut transport_options = TransportOptions::default();
//! transport_options.insert(
//!     ResendSendOptions::new()
//!         .with_tags([("env", "prod"), ("tenant", "blue")])
//!         .with_template(
//!             email_transport_resend::ResendTemplate::new("tmpl_welcome")
//!                 .with_variables([("name", serde_json::json!("Ada"))]),
//!         ),
//! );
//!
//! let transport = ResendTransport::new("re_...");
//! let report = transport
//!     .send(
//!         &message,
//!         &SendOptions::new().with_transport_options(transport_options),
//!     )
//!     .await?;
//! # let _ = report;
//! # Ok(())
//! # }
//! ```
//!
mod options;
mod transport;

pub use options::{ResendSendOptions, ResendTag, ResendTemplate};
pub use transport::{ResendTransport, ResendTransportBuilder};
