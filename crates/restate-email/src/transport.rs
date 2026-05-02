//! Transport resolution helpers for `restate-email` workers.

use std::collections::HashMap;

use restate_sdk::errors::TerminalError;

use email_transport::{DynTransport, Transport, string_newtype};

/// Re-export of [`email_transport::RuntimeBound`].
///
/// `restate-email` traits use the same runtime marker as `email-transport`
/// so the same blanket impl satisfies both layers.
pub use email_transport::RuntimeBound;

/// Resolves queued transport keys to configured transport instances.
///
/// Implement this trait when a worker needs dynamic routing that is not covered
/// by [`StaticTransportRegistry`] or [`CatchAllTransportResolver`]. The returned
/// transport is borrowed from `self`, so resolver implementations usually own
/// boxed transports or share a stable registry behind an `Arc`.
pub trait TransportResolver: RuntimeBound {
    /// Resolve a configured transport key.
    ///
    /// # Errors
    ///
    /// Returns [`TransportLookupError`] when the requested key is not
    /// registered.
    fn resolve(&self, transport: &TransportKey) -> Result<&DynTransport, TransportLookupError>;
}

string_newtype! {
    /// Configured transport key (e.g. `"primary"`, `"fallback"`).
    ///
    /// `new_unchecked` is available, declared via the `@unchecked`
    /// matcher arm of [`email_transport::string_newtype!`] so trusted-
    /// input construction (test fixtures, internal constants) stays
    /// available. End-user code should reach for [`Self::new`] /
    /// [`std::str::FromStr`] for any value that originated outside
    /// trusted code paths.
    @unchecked TransportKey
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum TransportLookupError {
    #[error("transport key `{key}` is not configured")]
    UnknownKey { key: String },
}

impl From<TransportLookupError> for TerminalError {
    fn from(error: TransportLookupError) -> Self {
        Self::new_with_code(404, error.to_string())
    }
}

/// Fixed-key transport registry for common worker setups.
///
/// The registry is intentionally small: populate it during startup with one or
/// more transport profiles, then pass it to [`crate::ServiceImpl`]. It is not a
/// hot-reload registry; build your own [`TransportResolver`] if routes need to
/// change while the process is running.
#[derive(Default)]
pub struct StaticTransportRegistry {
    transports: HashMap<String, Box<DynTransport>>,
}

impl StaticTransportRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace a transport under a string key.
    ///
    /// Returns the previous transport registered for the same key, if any.
    pub fn insert<T>(&mut self, key: impl Into<String>, transport: T) -> Option<Box<DynTransport>>
    where
        T: Transport + 'static,
    {
        self.transports.insert(key.into(), Box::new(transport))
    }
}

impl TransportResolver for StaticTransportRegistry {
    fn resolve(&self, transport: &TransportKey) -> Result<&DynTransport, TransportLookupError> {
        self.transports
            .get(transport.as_str())
            .map(Box::as_ref)
            .ok_or_else(|| TransportLookupError::UnknownKey {
                key: transport.as_str().to_owned(),
            })
    }
}

/// Resolver that ignores the requested key and always returns one transport.
///
/// This is useful for workers that expose the queue contract but only have one
/// delivery profile.
pub struct CatchAllTransportResolver {
    transport: Box<DynTransport>,
}

impl CatchAllTransportResolver {
    /// Create a resolver over a single transport.
    #[must_use]
    pub fn new<T>(transport: T) -> Self
    where
        T: Transport + 'static,
    {
        Self {
            transport: Box::new(transport),
        }
    }
}

impl TransportResolver for CatchAllTransportResolver {
    fn resolve(&self, _transport: &TransportKey) -> Result<&DynTransport, TransportLookupError> {
        Ok(self.transport.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestTransport;

    impl Transport for TestTransport {
        fn send<'a>(
            &'a self,
            _message: &'a email_message::OutboundMessage,
            _options: &'a email_transport::SendOptions,
        ) -> impl core::future::Future<
            Output = Result<email_transport::SendReport, email_transport::TransportError>,
        > + Send
        + 'a {
            Box::pin(async { Ok(email_transport::SendReport::new("test")) })
        }
    }

    #[test]
    fn static_transport_registry_returns_unknown_key_for_missing_entry() {
        let registry = StaticTransportRegistry::new();
        let Err(error) = registry.resolve(&TransportKey::new_unchecked("missing")) else {
            panic!("missing key should fail");
        };

        assert_eq!(
            error,
            TransportLookupError::UnknownKey {
                key: String::from("missing")
            }
        );
    }

    #[test]
    fn catch_all_transport_resolver_ignores_requested_key() {
        let resolver = CatchAllTransportResolver::new(TestTransport);

        let first = resolver
            .resolve(&TransportKey::new_unchecked("primary"))
            .expect("transport should resolve");
        let second = resolver
            .resolve(&TransportKey::new_unchecked("missing"))
            .expect("transport should resolve");

        assert!(std::ptr::addr_eq(
            first as *const DynTransport,
            second as *const DynTransport
        ));
    }
}
