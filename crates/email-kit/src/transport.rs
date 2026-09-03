//! Core transport APIs and optional provider transports.

pub use email_transport::*;

#[cfg(feature = "transport-cloudflare")]
pub use email_transport_cloudflare as cloudflare;

#[cfg(feature = "transport-lettre")]
pub use email_transport_lettre as lettre;

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
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// # #[cfg(feature = "serde")]
/// # {
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
/// # }
/// # Ok(())
/// # }
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
///
/// Calling this function repeatedly is safe when the registry contains the
/// same concrete option types.
///
/// # Panics
///
/// Panics if a different option type has already registered a provider key
/// owned by an enabled built-in adapter, such as `"resend"`.
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

    #[cfg(feature = "transport-resend")]
    struct ResendKeyCollision;

    #[cfg(feature = "transport-resend")]
    impl email_transport::__macro_serde::Serialize for ResendKeyCollision {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: email_transport::__macro_serde::Serializer,
        {
            serializer.serialize_unit()
        }
    }

    #[cfg(feature = "transport-resend")]
    impl<'de> email_transport::__macro_serde::Deserialize<'de> for ResendKeyCollision {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: email_transport::__macro_serde::Deserializer<'de>,
        {
            <() as email_transport::__macro_serde::Deserialize>::deserialize(deserializer)?;
            Ok(Self)
        }
    }

    #[cfg(feature = "transport-resend")]
    impl email_transport::TransportOption for ResendKeyCollision {
        fn provider_key() -> &'static str {
            "resend"
        }
    }

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

    #[cfg(feature = "transport-resend")]
    #[test]
    #[should_panic(expected = "resend provider key should be unique")]
    fn registration_panics_when_resend_key_is_owned_by_another_type() {
        let mut registry = TransportOptionRegistry::new();
        registry
            .register::<ResendKeyCollision>()
            .expect("collision fixture registers first");

        register_transport_options(&mut registry);
    }
}
