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
/// owns that JSON shape. Workers that hydrate [`SendOptions`] from queue or
/// wire payloads typically want this exact set of registrations.
///
/// # Examples
///
/// ```rust
/// # #[cfg(feature = "serde")]
/// # fn example() {
/// let registry = email_kit::transport::transport_option_registry();
/// # let _ = registry;
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
#[cfg(feature = "serde")]
pub fn register_transport_options(registry: &mut TransportOptionRegistry) {
    #[cfg(not(feature = "transport-resend"))]
    let _ = registry;

    #[cfg(feature = "transport-resend")]
    registry
        .register::<email_transport_resend::ResendSendOptions>()
        .expect("resend provider key should be unique");
}

/// Process-wide [`TransportOptionRegistry`] populated by
/// [`register_transport_options`], lazily initialized on first access.
///
/// Workers that build one registry at startup and reuse it across sends should
/// prefer this over calling [`transport_option_registry`] each time.
#[cfg(feature = "serde")]
#[must_use]
pub fn default_transport_option_registry() -> &'static TransportOptionRegistry {
    static REGISTRY: std::sync::OnceLock<TransportOptionRegistry> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(transport_option_registry)
}

#[cfg(all(test, feature = "serde"))]
mod tests {
    use super::{
        TransportOptionRegistry, default_transport_option_registry, register_transport_options,
        transport_option_registry,
    };

    #[test]
    fn registry_helpers_are_idempotent() {
        let mut registry = TransportOptionRegistry::new();
        register_transport_options(&mut registry);
        register_transport_options(&mut registry);
    }

    #[test]
    fn default_registry_returns_stable_reference() {
        let first = default_transport_option_registry();
        let second = default_transport_option_registry();
        assert!(std::ptr::eq(first, second));
    }

    #[test]
    fn fresh_registry_can_be_built() {
        let _ = transport_option_registry();
    }
}
