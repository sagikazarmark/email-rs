//! Per-send options and provider-specific transport option storage.

use core::any::{Any, TypeId};
#[cfg(feature = "serde")]
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::time::Duration;

use email_message::Envelope;

#[derive(Default)]
#[non_exhaustive]
pub struct SendOptions {
    /// Optional custom SMTP envelope for structured [`crate::Transport`] sends.
    ///
    /// This is only meaningful for structured transports that advertise
    /// [`crate::Capabilities::custom_envelope`]. Other structured transports may ignore
    /// it. [`crate::RawTransport`] methods take an explicit [`Envelope`] argument, and
    /// that argument is authoritative; raw transports ignore this field.
    pub envelope: Option<Envelope>,
    /// Typed, in-process provider-specific controls.
    ///
    /// With the `serde` feature enabled, [`TransportOptions`] serializes as a
    /// provider-keyed JSON object and can be hydrated through
    /// `TransportOptionRegistry`.
    pub transport_options: TransportOptions,
    /// Upper bound on provider-call duration for this send attempt. Transports
    /// advertising `Capabilities::timeout` must honor it; others should ignore it.
    pub timeout: Option<Duration>,
    /// Provider-level idempotency token for this attempt. Transports advertising
    /// `Capabilities::idempotency_key` must forward it; others should ignore it.
    ///
    /// Validated at construction (rejects empty, NUL, CR/LF, non-tab control
    /// characters, and values longer than 1 KiB), see [`IdempotencyKey`].
    pub idempotency_key: Option<IdempotencyKey>,
    /// Opaque correlation identifier carried end-to-end from queue to transport.
    /// Built-in transports do not automatically expose it to providers; it is
    /// available to adapter-specific typed options when a provider has a natural
    /// slot for it.
    ///
    /// Validated at construction with the same rules as
    /// [`IdempotencyKey`], see [`CorrelationId`].
    pub correlation_id: Option<CorrelationId>,
}

impl SendOptions {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_envelope(mut self, envelope: Envelope) -> Self {
        self.envelope = Some(envelope);
        self
    }

    #[must_use]
    pub fn with_transport_options(mut self, transport_options: TransportOptions) -> Self {
        self.transport_options = transport_options;
        self
    }

    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    #[must_use]
    pub fn with_idempotency_key(mut self, idempotency_key: IdempotencyKey) -> Self {
        self.idempotency_key = Some(idempotency_key);
        self
    }

    #[must_use]
    pub fn with_correlation_id(mut self, correlation_id: CorrelationId) -> Self {
        self.correlation_id = Some(correlation_id);
        self
    }
}

crate::string_newtype! {
    /// Provider idempotency token for safe retries.
    ///
    /// Validated on construction, empty, NUL, CR/LF, non-tab control
    /// characters, and values longer than 1 KiB are rejected. Adapters
    /// that advertise [`crate::Capabilities::idempotency_key`] forward the value
    /// verbatim to the provider's idempotency header; the validation closes
    /// a header-injection seam at the type level.
    ///
    /// `new_unchecked` is available on this type, it is declared via
    /// the `@unchecked` matcher arm of [`crate::string_newtype!`] so
    /// trusted-input construction (internal constants, test fixtures)
    /// stays available. End-user code should reach for
    /// [`Self::new`] / [`std::str::FromStr`] for any value that
    /// originated outside trusted code paths.
    @unchecked IdempotencyKey
}

crate::string_newtype! {
    /// Correlation identifier carried end-to-end from queue to transport.
    ///
    /// Available to adapter-specific typed options when a provider has a
    /// natural slot for it. Validated on construction with the same rules as
    /// [`IdempotencyKey`].
    ///
    /// `new_unchecked` is available, declared via the `@unchecked`
    /// matcher arm. See [`IdempotencyKey`] for guidance.
    @unchecked CorrelationId
}

/// Marker for typed provider-specific send options.
///
/// Adapters define their own per-provider option structs and implement this
/// trait so the typed slot in [`TransportOptions`] can store them keyed by
/// [`TypeId`] for in-process lookup.
///
/// Options also provide [`Self::provider_key`], a stable JSON key for this
/// option's provider such as `"resend"` or `"postmark"`. The key is
/// intentionally explicit rather than derived from [`TypeId`] or `type_name`,
/// both of which are implementation identities rather than wire-format
/// identifiers.
///
/// With the `serde` feature enabled, option types must also implement
/// `serde::Serialize` for insertion and `serde::Deserialize` for registry
/// hydration.
///
/// # Send + Sync
///
/// The bound is `Send + Sync` on every target, including `wasm32`, even
/// though most other kernel async surfaces drop `Send` on `wasm32` to
/// accommodate `!Send` JS handles. `TransportOption` values flow through
/// the typed slot map and may be inspected from any thread; in practice
/// adapters store plain newtypes (`Vec<String>`, primitives) that satisfy
/// the bound trivially. A wasm adapter that wants to stash a raw
/// `web_sys::JsValue` inside an option would need its own thread-safe
/// wrapper; the current design assumes that is rare enough not to warrant
/// a cfg gate.
pub trait TransportOption: Any + Send + Sync {
    /// Stable provider key used when serializing this option at queue/wire
    /// boundaries.
    ///
    /// The `Self: Sized` bound keeps [`TransportOption`] dyn-compatible;
    /// callers that only have a `dyn TransportOption` already receive the
    /// provider key from the typed slot metadata captured at insertion time.
    fn provider_key() -> &'static str
    where
        Self: Sized;
}

/// Per-send transport options.
///
/// A typed, in-process map keyed by `TypeId` carrying provider-specific strongly
/// typed values (e.g. `PostmarkTag`, `ResendTags`). Cheap, zero-copy at the
/// adapter boundary.
///
/// Values also carry a stable [`TransportOption::provider_key`]. With the
/// `serde` feature enabled, this allows the map to serialize into a
/// provider-keyed JSON object for queue boundaries. Deserialization requires a
/// `TransportOptionRegistry` because Rust cannot discover concrete
/// `TransportOption` implementors from JSON keys alone.
#[derive(Default)]
pub struct TransportOptions {
    inner: HashMap<TypeId, TypedSlot>,
}

struct TypedSlot {
    type_name: &'static str,
    #[cfg(feature = "serde")]
    provider_key: &'static str,
    #[cfg(feature = "serde")]
    serialize_json: fn(&(dyn Any + Send + Sync)) -> Result<serde_json::Value, serde_json::Error>,
    value: Box<dyn Any + Send + Sync>,
}

impl TransportOptions {
    #[cfg(feature = "serde")]
    pub fn insert<T>(&mut self, value: T)
    where
        T: TransportOption + serde::Serialize,
    {
        self.inner.insert(
            TypeId::of::<T>(),
            TypedSlot {
                type_name: std::any::type_name::<T>(),
                provider_key: T::provider_key(),
                serialize_json: serialize_transport_option::<T>,
                value: Box::new(value),
            },
        );
    }

    #[cfg(not(feature = "serde"))]
    pub fn insert<T: TransportOption>(&mut self, value: T) {
        self.inner.insert(
            TypeId::of::<T>(),
            TypedSlot {
                type_name: std::any::type_name::<T>(),
                value: Box::new(value),
            },
        );
    }

    #[must_use]
    pub fn get<T: TransportOption>(&self) -> Option<&T> {
        self.inner
            .get(&TypeId::of::<T>())?
            .value
            .downcast_ref::<T>()
    }

    pub fn get_mut<T: TransportOption>(&mut self) -> Option<&mut T> {
        self.inner
            .get_mut(&TypeId::of::<T>())?
            .value
            .downcast_mut::<T>()
    }

    pub fn remove<T: TransportOption>(&mut self) -> Option<T> {
        self.inner
            .remove(&TypeId::of::<T>())
            .and_then(|slot| slot.value.downcast::<T>().ok())
            .map(|v| *v)
    }

    /// `true` when no typed slots are present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

#[cfg(feature = "serde")]
fn serialize_transport_option<T>(
    value: &(dyn Any + Send + Sync),
) -> Result<serde_json::Value, serde_json::Error>
where
    T: TransportOption + serde::Serialize,
{
    let value = value.downcast_ref::<T>().expect(
        "TransportOptions typed slot serializer should match the value inserted for this TypeId",
    );
    serde_json::to_value(value)
}

#[cfg(feature = "serde")]
impl serde::Serialize for TransportOptions {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap as _;

        let mut slots: Vec<&TypedSlot> = self.inner.values().collect();
        slots.sort_unstable_by_key(|slot| slot.provider_key);

        let mut seen = BTreeSet::new();
        let mut map = serializer.serialize_map(Some(slots.len()))?;
        for slot in slots {
            if !seen.insert(slot.provider_key) {
                return Err(serde::ser::Error::custom(format_args!(
                    "duplicate TransportOption provider key `{}`",
                    slot.provider_key
                )));
            }

            let value = (slot.serialize_json)(slot.value.as_ref())
                .map_err(|error| serde::ser::Error::custom(error.to_string()))?;
            map.serialize_entry(slot.provider_key, &value)?;
        }
        map.end()
    }
}

/// Registry of queue/wire codecs for concrete [`TransportOption`] types.
///
/// Serialization does not need a registry because every typed slot stores its
/// provider key and serializer when inserted into [`TransportOptions`].
/// Deserialization does need a registry so a stable provider key can be mapped
/// back to the concrete Rust type that owns that JSON shape.
#[cfg(feature = "serde")]
#[derive(Default)]
pub struct TransportOptionRegistry {
    decoders: HashMap<&'static str, TransportOptionDecoder>,
}

#[cfg(feature = "serde")]
struct TransportOptionDecoder {
    type_name: &'static str,
    decode: fn(&serde_json::Value, &mut TransportOptions) -> Result<(), serde_json::Error>,
}

#[cfg(feature = "serde")]
impl TransportOptionRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a concrete provider option type by its
    /// [`TransportOption::provider_key`].
    ///
    /// # Errors
    ///
    /// Returns [`TransportOptionRegistryError::DuplicateProviderKey`] if a
    /// different option type has already claimed the same provider key.
    pub fn register<T>(&mut self) -> Result<(), TransportOptionRegistryError>
    where
        T: TransportOption + serde::Serialize + serde::de::DeserializeOwned,
    {
        let provider_key = T::provider_key();
        if let Some(existing) = self.decoders.get(provider_key) {
            if existing.type_name == std::any::type_name::<T>() {
                return Ok(());
            }

            return Err(TransportOptionRegistryError::DuplicateProviderKey {
                provider_key,
                existing_type: existing.type_name,
                new_type: std::any::type_name::<T>(),
            });
        }

        self.decoders.insert(
            provider_key,
            TransportOptionDecoder {
                type_name: std::any::type_name::<T>(),
                decode: decode_transport_option::<T>,
            },
        );
        Ok(())
    }

    /// Deserialize `value` for `provider_key` and overwrite the matching typed
    /// slot in `options` when the provider key is registered.
    ///
    /// Returns `Ok(true)` when a registered option type consumed the value and
    /// `Ok(false)` for unknown provider keys. Unknown keys are intentionally not
    /// errors so queue payloads can be forwarded across workers with different
    /// provider feature sets.
    ///
    /// # Errors
    ///
    /// Returns [`serde_json::Error`] if `provider_key` is registered but `value`
    /// does not match that option type's serde shape.
    pub fn hydrate_into(
        &self,
        provider_key: &str,
        value: &serde_json::Value,
        options: &mut TransportOptions,
    ) -> Result<bool, serde_json::Error> {
        let Some(decoder) = self.decoders.get(provider_key) else {
            return Ok(false);
        };

        (decoder.decode)(value, options)?;
        Ok(true)
    }
}

#[cfg(feature = "serde")]
fn decode_transport_option<T>(
    value: &serde_json::Value,
    options: &mut TransportOptions,
) -> Result<(), serde_json::Error>
where
    T: TransportOption + serde::Serialize + serde::de::DeserializeOwned,
{
    options.insert(serde_json::from_value::<T>(value.clone())?);
    Ok(())
}

#[cfg(feature = "serde")]
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TransportOptionRegistryError {
    #[error(
        "duplicate TransportOption provider key `{provider_key}` for `{new_type}`; already registered by `{existing_type}`"
    )]
    DuplicateProviderKey {
        provider_key: &'static str,
        existing_type: &'static str,
        new_type: &'static str,
    },
}

impl std::fmt::Debug for TransportOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut typed: Vec<&'static str> = self.inner.values().map(|slot| slot.type_name).collect();
        typed.sort_unstable();

        f.debug_struct("TransportOptions")
            .field("typed", &typed)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use crate::{STRING_NEWTYPE_MAX_BYTES, StringNewtypeError};

    #[cfg(feature = "serde")]
    use super::CorrelationId;
    use super::{IdempotencyKey, TransportOption, TransportOptions};
    #[cfg(feature = "serde")]
    use super::{TransportOptionRegistry, TransportOptionRegistryError};

    #[derive(Debug, PartialEq, Eq)]
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    struct TestOption(String);

    impl TransportOption for TestOption {
        fn provider_key() -> &'static str {
            "test"
        }
    }

    #[cfg(feature = "serde")]
    #[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    struct OtherTestOption {
        value: u32,
    }

    #[cfg(feature = "serde")]
    impl TransportOption for OtherTestOption {
        fn provider_key() -> &'static str {
            "other"
        }
    }

    #[cfg(feature = "serde")]
    #[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    struct DuplicateKeyOption(String);

    #[cfg(feature = "serde")]
    impl TransportOption for DuplicateKeyOption {
        fn provider_key() -> &'static str {
            "test"
        }
    }

    #[test]
    fn idempotency_key_rejects_crlf_injection() {
        let err = IdempotencyKey::new("hi\r\nBcc: victim").unwrap_err();
        assert_eq!(err, StringNewtypeError::Newline);
    }

    #[test]
    fn idempotency_key_rejects_empty() {
        let err = IdempotencyKey::new("").unwrap_err();
        assert_eq!(err, StringNewtypeError::Empty);
    }

    #[test]
    fn idempotency_key_rejects_nul() {
        let err = IdempotencyKey::new("hi\0bye").unwrap_err();
        assert_eq!(err, StringNewtypeError::Nul);
    }

    #[test]
    fn idempotency_key_rejects_oversize() {
        let payload = "x".repeat(STRING_NEWTYPE_MAX_BYTES + 1);
        let err = IdempotencyKey::new(payload).unwrap_err();
        assert!(matches!(err, StringNewtypeError::TooLong { .. }));
    }

    #[test]
    fn idempotency_key_accepts_well_formed() {
        let key = IdempotencyKey::new("job-12345").unwrap();
        assert_eq!(key.as_str(), "job-12345");
    }

    #[test]
    fn idempotency_key_rejects_unicode_tag_codepoints() {
        // U+E0041 is the tag-form of ASCII 'A'. Invisible to humans
        // but readable by some downstream tooling, a known ASCII-
        // smuggling vector. The validator must reject it.
        let smuggled = format!("trace-{}", '\u{E0041}');
        let err = IdempotencyKey::new(smuggled).unwrap_err();
        assert_eq!(err, StringNewtypeError::UnicodeTag);
    }

    #[test]
    fn idempotency_key_accepts_legitimate_non_ascii_utf8() {
        // Legitimate UTF-8 (e.g. accented letters) stays accepted
        // the kernel's gate is for injection prevention, not
        // HTTP-header validity. Adapters that forward the value as
        // an HTTP header are responsible for additional validation.
        let key = IdempotencyKey::new("trace-héllo").unwrap();
        assert_eq!(key.as_str(), "trace-héllo");
    }

    #[test]
    #[cfg(feature = "serde")]
    fn correlation_id_round_trips_through_serde() {
        let id = CorrelationId::new("trace-abc").unwrap();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"trace-abc\"");
        let back: CorrelationId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    #[cfg(feature = "serde")]
    fn correlation_id_serde_rejects_invalid() {
        // Deserialize routes through the generated new constructor, so validation runs.
        let result: Result<CorrelationId, _> = serde_json::from_str("\"hi\\r\\nbad\"");
        assert!(result.is_err());
    }

    #[test]
    fn transport_options_store_typed_values() {
        let mut options = TransportOptions::default();
        options.insert(TestOption(String::from("value")));

        assert_eq!(
            options.get::<TestOption>().map(|value| value.0.as_str()),
            Some("value")
        );
        assert_eq!(
            options
                .remove::<TestOption>()
                .as_ref()
                .map(|value| value.0.as_str()),
            Some("value")
        );
        assert!(options.get::<TestOption>().is_none());
    }

    #[test]
    fn transport_options_debug_lists_typed_slot_names() {
        let mut options = TransportOptions::default();
        options.insert(TestOption(String::from("v")));

        let rendered = format!("{options:?}");
        assert!(rendered.contains("TestOption"), "got {rendered}");
    }

    #[test]
    #[cfg(feature = "serde")]
    fn transport_options_serialize_provider_keyed_map() {
        let mut options = TransportOptions::default();
        options.insert(TestOption(String::from("value")));
        options.insert(OtherTestOption { value: 42 });

        let json = serde_json::to_value(&options).expect("transport options serialize");
        assert_eq!(json["test"], serde_json::json!("value"));
        assert_eq!(json["other"], serde_json::json!({"value": 42}));
    }

    #[test]
    #[cfg(feature = "serde")]
    fn transport_option_registry_hydrates_and_overwrites() {
        let mut registry = TransportOptionRegistry::new();
        registry
            .register::<TestOption>()
            .expect("register succeeds");

        let mut options = TransportOptions::default();
        options.insert(TestOption(String::from("typed")));

        let hydrated = registry
            .hydrate_into("test", &serde_json::json!("json"), &mut options)
            .expect("hydration succeeds");

        assert!(hydrated);
        assert_eq!(
            options.get::<TestOption>().map(|value| value.0.as_str()),
            Some("json")
        );
    }

    #[test]
    #[cfg(feature = "serde")]
    fn transport_option_registry_ignores_unknown_keys() {
        let registry = TransportOptionRegistry::new();
        let mut options = TransportOptions::default();

        let hydrated = registry
            .hydrate_into("unknown", &serde_json::json!({"value": 1}), &mut options)
            .expect("unknown keys do not error");

        assert!(!hydrated);
        assert!(options.is_empty());
    }

    #[test]
    #[cfg(feature = "serde")]
    fn transport_option_registry_rejects_malformed_known_values() {
        let mut registry = TransportOptionRegistry::new();
        registry
            .register::<OtherTestOption>()
            .expect("register succeeds");
        let mut options = TransportOptions::default();

        let result =
            registry.hydrate_into("other", &serde_json::json!("not-an-object"), &mut options);

        assert!(result.is_err());
        assert!(options.is_empty());
    }

    #[test]
    #[cfg(feature = "serde")]
    fn transport_option_registry_rejects_duplicate_provider_keys() {
        let mut registry = TransportOptionRegistry::new();
        registry
            .register::<TestOption>()
            .expect("register succeeds");

        let error = registry
            .register::<DuplicateKeyOption>()
            .expect_err("duplicate provider key should fail");

        match error {
            TransportOptionRegistryError::DuplicateProviderKey { provider_key, .. } => {
                assert_eq!(provider_key, "test");
            }
        }
    }
}
