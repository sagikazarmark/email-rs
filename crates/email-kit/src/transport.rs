//! Core transport APIs and optional provider transports.

pub use email_transport::*;

#[cfg(feature = "transport-resend")]
pub use email_transport_resend as resend;

/// Build a [`TransportOptionRegistry`] preloaded with every provider option
/// type for the adapter features compiled into this crate.
///
/// The registry is what
/// [`TransportOptionRegistry::deserialize_send_options`] consults to map a
/// provider key like `"resend"` back to the concrete Rust option type that
/// owns that wire shape. Workers that hydrate [`SendOptions`] from queue or
/// wire payloads typically want this exact set of registrations.
///
/// # Examples
///
/// ```rust
/// # #[cfg(feature = "serde")]
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// use email_kit::transport::SendOptions;
///
/// let registry = email_kit::transport::transport_option_registry();
///
/// let payload = r#"{
///     "envelope": {
///         "mail_from": "sender@example.com",
///         "rcpt_to": ["recipient@example.com"]
///     },
///     "timeout": {"secs": 5, "nanos": 0}
/// }"#;
/// let mut deserializer = serde_json::Deserializer::from_str(payload);
/// let options: SendOptions = registry.deserialize_send_options(&mut deserializer)?;
///
/// assert!(options.envelope.is_some());
/// assert_eq!(options.timeout, Some(std::time::Duration::from_secs(5)));
/// # Ok(())
/// # }
/// # #[cfg(not(feature = "serde"))]
/// # fn example() -> Result<(), Box<dyn std::error::Error>> { Ok(()) }
/// # example().unwrap();
/// ```
#[cfg(feature = "serde")]
#[must_use]
pub fn transport_option_registry() -> TransportOptionRegistry {
    let mut registry = TransportOptionRegistry::new();
    register_transport_options(&mut registry);
    registry
}

/// Register every provider option type for the adapter features compiled into
/// this crate into `registry`.
///
/// Use this when the host application keeps its own
/// [`TransportOptionRegistry`] (for example, to also register
/// application-specific [`TransportOption`] types) and just wants to layer the
/// email-rs adapters on top.
#[cfg(feature = "serde")]
pub fn register_transport_options(registry: &mut TransportOptionRegistry) {
    #[cfg(not(feature = "transport-resend"))]
    let _ = registry;

    #[cfg(feature = "transport-resend")]
    registry
        .register::<email_transport_resend::ResendSendOptions>()
        .expect("resend provider key should be unique");
}

#[cfg(all(test, feature = "serde"))]
mod tests {
    use super::{TransportOptionRegistry, register_transport_options, transport_option_registry};

    #[test]
    fn registry_helpers_are_idempotent() {
        let mut registry = TransportOptionRegistry::new();
        register_transport_options(&mut registry);
        register_transport_options(&mut registry);
    }

    #[test]
    fn fresh_registry_can_be_built() {
        let _ = transport_option_registry();
    }
}
