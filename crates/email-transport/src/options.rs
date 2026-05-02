//! Per-send options and provider-specific transport option storage.

use core::any::{Any, TypeId};
#[cfg(feature = "serde")]
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::time::Duration;

use email_message::Envelope;
#[cfg(feature = "serde")]
use thiserror::Error;

/// Per-send controls shared by structured and raw transport sends.
///
/// With the `serde` feature enabled, this serializes as a sparse object:
/// absent options are omitted, [`TransportOptions`] uses its provider-keyed
/// representation, and `timeout` is encoded as `{ "secs": u64, "nanos": u32 }`.
/// Deserialization is intentionally registry-driven and not implemented on this
/// type because provider-specific options need a [`TransportOptionRegistry`];
/// use [`TransportOptionRegistry::send_options_seed`] (or the convenience
/// [`TransportOptionRegistry::deserialize_send_options`] wrapper) instead.
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct SendOptions {
    /// Optional custom SMTP envelope for structured [`crate::Transport`] sends.
    ///
    /// This is only meaningful for structured transports that advertise
    /// [`crate::Capabilities::custom_envelope`]. Other structured transports may ignore
    /// it. [`crate::RawTransport`] methods take an explicit [`Envelope`] argument, and
    /// that argument is authoritative; raw transports ignore this field.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub envelope: Option<Envelope>,
    /// Typed, in-process provider-specific controls.
    ///
    /// With the `serde` feature enabled, [`TransportOptions`] serializes as a
    /// provider-keyed JSON object and can be hydrated through
    /// `TransportOptionRegistry`.
    #[cfg_attr(
        feature = "serde",
        serde(skip_serializing_if = "TransportOptions::is_empty")
    )]
    #[cfg_attr(
        feature = "schemars",
        schemars(default, skip_serializing_if = "TransportOptions::is_empty")
    )]
    pub transport_options: TransportOptions,
    /// Upper bound on provider-call duration for this send attempt. Transports
    /// advertising `Capabilities::timeout` must honor it; others should ignore it.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub timeout: Option<Duration>,
    /// Provider-level idempotency token for this attempt. Transports advertising
    /// `Capabilities::idempotency_key` must forward it; others should ignore it.
    ///
    /// Validated at construction (rejects empty, NUL, CR/LF, non-tab control
    /// characters, and values longer than 1 KiB), see [`IdempotencyKey`].
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub idempotency_key: Option<IdempotencyKey>,
    /// Opaque correlation identifier carried end-to-end from queue to transport.
    /// Built-in transports do not automatically expose it to providers; it is
    /// available to adapter-specific typed options when a provider has a natural
    /// slot for it.
    ///
    /// Validated at construction with the same rules as
    /// [`IdempotencyKey`], see [`CorrelationId`].
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
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
/// provider-keyed object for queue boundaries through any serde format.
/// Deserialization requires a [`TransportOptionRegistry`] because Rust cannot
/// discover concrete `TransportOption` implementors from a string key alone;
/// drive it through [`TransportOptionsSeed`] / [`SendOptionsSeed`] (or the
/// convenience [`TransportOptionRegistry::deserialize_send_options`]).
#[derive(Default)]
pub struct TransportOptions {
    inner: HashMap<TypeId, TypedSlot>,
}

#[cfg(feature = "serde")]
struct TypedSlot {
    type_name: &'static str,
    provider_key: &'static str,
    value: Box<dyn DynTransportOption>,
}

#[cfg(not(feature = "serde"))]
struct TypedSlot {
    type_name: &'static str,
    value: Box<dyn Any + Send + Sync>,
}

/// Erased-serde-aware view of a `TransportOption` value, plus access back to
/// `Any` for the typed-slot lookup methods.
#[cfg(feature = "serde")]
trait DynTransportOption: erased_serde::Serialize + Send + Sync + 'static {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn into_any(self: Box<Self>) -> Box<dyn Any + Send + Sync>;
}

#[cfg(feature = "serde")]
impl<T> DynTransportOption for T
where
    T: TransportOption + serde::Serialize,
{
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn into_any(self: Box<Self>) -> Box<dyn Any + Send + Sync> {
        self
    }
}

#[cfg(feature = "serde")]
erased_serde::serialize_trait_object!(DynTransportOption);

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
        let slot = self.inner.get(&TypeId::of::<T>())?;
        slot.value_any().downcast_ref::<T>()
    }

    pub fn get_mut<T: TransportOption>(&mut self) -> Option<&mut T> {
        let slot = self.inner.get_mut(&TypeId::of::<T>())?;
        slot.value_any_mut().downcast_mut::<T>()
    }

    pub fn remove<T: TransportOption>(&mut self) -> Option<T> {
        let slot = self.inner.remove(&TypeId::of::<T>())?;
        slot.into_any().downcast::<T>().ok().map(|v| *v)
    }

    /// `true` when no typed slots are present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

#[cfg(feature = "serde")]
impl TypedSlot {
    fn value_any(&self) -> &dyn Any {
        self.value.as_any()
    }

    fn value_any_mut(&mut self) -> &mut dyn Any {
        self.value.as_any_mut()
    }

    fn into_any(self) -> Box<dyn Any + Send + Sync> {
        self.value.into_any()
    }
}

#[cfg(not(feature = "serde"))]
impl TypedSlot {
    fn value_any(&self) -> &dyn Any {
        self.value.as_ref()
    }

    fn value_any_mut(&mut self) -> &mut dyn Any {
        self.value.as_mut()
    }

    fn into_any(self) -> Box<dyn Any + Send + Sync> {
        self.value
    }
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
            map.serialize_entry(slot.provider_key, &*slot.value)?;
        }
        map.end()
    }
}

#[cfg(feature = "schemars")]
impl schemars::JsonSchema for TransportOptions {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "TransportOptions".into()
    }

    fn schema_id() -> std::borrow::Cow<'static, str> {
        concat!(module_path!(), "::TransportOptions").into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "object",
            "additionalProperties": true,
        })
    }
}

/// Registry of queue/wire codecs for concrete [`TransportOption`] types.
///
/// Serialization does not need a registry because every typed slot stores its
/// provider key when inserted into [`TransportOptions`]. Deserialization does
/// need a registry so a stable provider key can be mapped back to the concrete
/// Rust type that owns that wire shape; that mapping is exposed through
/// [`TransportOptionsSeed`] and [`SendOptionsSeed`], which implement
/// [`serde::de::DeserializeSeed`] so the registry can drive any serde
/// deserializer (JSON, CBOR, MessagePack, postcard, ...) directly into typed
/// slots without an intermediate `serde_json::Value`.
#[cfg(feature = "serde")]
#[derive(Default)]
pub struct TransportOptionRegistry {
    decoders: HashMap<&'static str, TransportOptionDecoder>,
}

#[cfg(feature = "serde")]
struct TransportOptionDecoder {
    type_name: &'static str,
    decode: for<'de> fn(
        &mut dyn erased_serde::Deserializer<'de>,
        &mut TransportOptions,
    ) -> Result<(), erased_serde::Error>,
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

    /// Build a [`DeserializeSeed`](serde::de::DeserializeSeed) that hydrates a
    /// [`TransportOptions`] map from any serde deserializer.
    ///
    /// Unknown provider keys are rejected by default. Use
    /// [`TransportOptionsSeed::ignore_unknown_transport_options`] to skip them.
    #[must_use]
    pub fn transport_options_seed(&self) -> TransportOptionsSeed<'_> {
        TransportOptionsSeed {
            registry: self,
            ignore_unknown: false,
        }
    }

    /// Build a [`DeserializeSeed`](serde::de::DeserializeSeed) that hydrates a
    /// [`SendOptions`] from any serde deserializer.
    ///
    /// Unknown top-level fields are ignored for forward compatibility. Unknown
    /// provider keys inside `transport_options` are rejected by default; use
    /// [`SendOptionsSeed::ignore_unknown_transport_options`] to skip them.
    #[must_use]
    pub fn send_options_seed(&self) -> SendOptionsSeed<'_> {
        SendOptionsSeed {
            registry: self,
            ignore_unknown: false,
        }
    }

    /// Deserialize [`SendOptions`] from any serde deserializer.
    ///
    /// Convenience wrapper around [`Self::send_options_seed`] for callers that
    /// only need the default strict behavior.
    ///
    /// # Errors
    ///
    /// Returns the deserializer's native error when the payload shape is
    /// malformed, a registered provider option fails to deserialize, or
    /// `transport_options` contains an unregistered provider key.
    pub fn deserialize_send_options<'de, D>(&self, deserializer: D) -> Result<SendOptions, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::DeserializeSeed as _;
        self.send_options_seed().deserialize(deserializer)
    }

    /// Deserialize a single provider option for `provider_key` and overwrite the
    /// matching typed slot in `options` when the provider key is registered.
    ///
    /// Returns `Ok(true)` when a registered option type consumed the value and
    /// `Ok(false)` for unknown provider keys. Unknown keys are intentionally not
    /// errors so queue payloads can be forwarded across workers with different
    /// provider feature sets.
    ///
    /// # Errors
    ///
    /// Returns the deserializer's native error if `provider_key` is registered
    /// but the value does not match that option type's serde shape.
    pub fn hydrate_into<'de, D>(
        &self,
        provider_key: &str,
        deserializer: D,
        options: &mut TransportOptions,
    ) -> Result<bool, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let Some(decoder) = self.decoders.get(provider_key) else {
            return Ok(false);
        };

        let mut erased = <dyn erased_serde::Deserializer<'de>>::erase(deserializer);
        (decoder.decode)(&mut erased, options).map_err(serde::de::Error::custom)?;
        Ok(true)
    }
}

#[cfg(feature = "serde")]
fn decode_transport_option<'de, T>(
    deserializer: &mut dyn erased_serde::Deserializer<'de>,
    options: &mut TransportOptions,
) -> Result<(), erased_serde::Error>
where
    T: TransportOption + serde::Serialize + serde::de::DeserializeOwned,
{
    let value: T = erased_serde::deserialize(deserializer)?;
    options.insert(value);
    Ok(())
}

/// [`DeserializeSeed`](serde::de::DeserializeSeed) for [`TransportOptions`].
///
/// Built through [`TransportOptionRegistry::transport_options_seed`].
#[cfg(feature = "serde")]
pub struct TransportOptionsSeed<'a> {
    registry: &'a TransportOptionRegistry,
    ignore_unknown: bool,
}

#[cfg(feature = "serde")]
impl<'a> TransportOptionsSeed<'a> {
    /// Skip provider keys that the registry has no decoder for instead of
    /// erroring, so payloads can flow across workers compiled with different
    /// adapter feature sets.
    #[must_use]
    pub fn ignore_unknown_transport_options(mut self) -> Self {
        self.ignore_unknown = true;
        self
    }
}

#[cfg(feature = "serde")]
impl<'de, 'a> serde::de::DeserializeSeed<'de> for TransportOptionsSeed<'a> {
    type Value = TransportOptions;

    fn deserialize<D>(self, deserializer: D) -> Result<TransportOptions, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(TransportOptionsVisitor {
            registry: self.registry,
            ignore_unknown: self.ignore_unknown,
        })
    }
}

#[cfg(feature = "serde")]
struct TransportOptionsVisitor<'a> {
    registry: &'a TransportOptionRegistry,
    ignore_unknown: bool,
}

#[cfg(feature = "serde")]
impl<'de, 'a> serde::de::Visitor<'de> for TransportOptionsVisitor<'a> {
    type Value = TransportOptions;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a provider-keyed map of TransportOption values")
    }

    fn visit_map<A>(self, mut map: A) -> Result<TransportOptions, A::Error>
    where
        A: serde::de::MapAccess<'de>,
    {
        let mut options = TransportOptions::default();
        while let Some(key) = map.next_key::<String>()? {
            if let Some(decoder) = self.registry.decoders.get(key.as_str()) {
                map.next_value_seed(TransportOptionDecoderSeed {
                    decode: decoder.decode,
                    options: &mut options,
                })?;
            } else if self.ignore_unknown {
                map.next_value::<serde::de::IgnoredAny>()?;
            } else {
                return Err(serde::de::Error::custom(format_args!(
                    "unknown TransportOption provider key `{key}`"
                )));
            }
        }
        Ok(options)
    }
}

#[cfg(feature = "serde")]
struct TransportOptionDecoderSeed<'a> {
    decode: for<'de> fn(
        &mut dyn erased_serde::Deserializer<'de>,
        &mut TransportOptions,
    ) -> Result<(), erased_serde::Error>,
    options: &'a mut TransportOptions,
}

#[cfg(feature = "serde")]
impl<'de, 'a> serde::de::DeserializeSeed<'de> for TransportOptionDecoderSeed<'a> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<(), D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut erased = <dyn erased_serde::Deserializer<'de>>::erase(deserializer);
        (self.decode)(&mut erased, self.options).map_err(serde::de::Error::custom)
    }
}

/// [`DeserializeSeed`](serde::de::DeserializeSeed) for [`SendOptions`].
///
/// Built through [`TransportOptionRegistry::send_options_seed`].
#[cfg(feature = "serde")]
pub struct SendOptionsSeed<'a> {
    registry: &'a TransportOptionRegistry,
    ignore_unknown: bool,
}

#[cfg(feature = "serde")]
impl<'a> SendOptionsSeed<'a> {
    /// Skip unknown provider keys inside `transport_options` instead of
    /// erroring. See [`TransportOptionsSeed::ignore_unknown_transport_options`].
    #[must_use]
    pub fn ignore_unknown_transport_options(mut self) -> Self {
        self.ignore_unknown = true;
        self
    }
}

#[cfg(feature = "serde")]
impl<'de, 'a> serde::de::DeserializeSeed<'de> for SendOptionsSeed<'a> {
    type Value = SendOptions;

    fn deserialize<D>(self, deserializer: D) -> Result<SendOptions, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(SendOptionsVisitor {
            registry: self.registry,
            ignore_unknown: self.ignore_unknown,
        })
    }
}

#[cfg(feature = "serde")]
struct SendOptionsVisitor<'a> {
    registry: &'a TransportOptionRegistry,
    ignore_unknown: bool,
}

/// Compile-time field identifier for [`SendOptions`].
///
/// Adding a field to [`SendOptions`] without adding a variant here means the
/// new field never reaches the typed value through the seed; the round-trip
/// test in this module's `tests` will catch that. Adding a variant here without
/// extending the `match` in [`SendOptionsVisitor::visit_map`] is a compile
/// error.
#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
#[serde(field_identifier, rename_all = "snake_case")]
enum SendOptionsField {
    Envelope,
    TransportOptions,
    Timeout,
    IdempotencyKey,
    CorrelationId,
    #[serde(other)]
    Other,
}

#[cfg(feature = "serde")]
impl<'de, 'a> serde::de::Visitor<'de> for SendOptionsVisitor<'a> {
    type Value = SendOptions;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a SendOptions map")
    }

    fn visit_map<A>(self, mut map: A) -> Result<SendOptions, A::Error>
    where
        A: serde::de::MapAccess<'de>,
    {
        let mut options = SendOptions::default();
        while let Some(field) = map.next_key::<SendOptionsField>()? {
            match field {
                SendOptionsField::Envelope => options.envelope = map.next_value()?,
                SendOptionsField::TransportOptions => {
                    options.transport_options = map.next_value_seed(TransportOptionsSeed {
                        registry: self.registry,
                        ignore_unknown: self.ignore_unknown,
                    })?;
                }
                SendOptionsField::Timeout => options.timeout = map.next_value()?,
                SendOptionsField::IdempotencyKey => {
                    options.idempotency_key = map.next_value()?;
                }
                SendOptionsField::CorrelationId => {
                    options.correlation_id = map.next_value()?;
                }
                SendOptionsField::Other => {
                    map.next_value::<serde::de::IgnoredAny>()?;
                }
            }
        }
        Ok(options)
    }
}

#[cfg(feature = "serde")]
#[derive(Debug, Error)]
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
    use email_message::Envelope;

    #[cfg(feature = "serde")]
    use super::CorrelationId;
    #[cfg(any(feature = "serde", feature = "schemars"))]
    use super::SendOptions;
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
    #[cfg(feature = "serde")]
    fn send_options_serialize_empty_as_empty_object() {
        let json = serde_json::to_value(SendOptions::default()).expect("send options serialize");

        assert_eq!(json, serde_json::json!({}));
    }

    #[test]
    #[cfg(feature = "serde")]
    fn send_options_serialize_sparse_provider_keyed_payload() {
        let mut transport_options = TransportOptions::default();
        transport_options.insert(TestOption(String::from("value")));

        let options = SendOptions::new()
            .with_transport_options(transport_options)
            .with_timeout(std::time::Duration::new(3, 25))
            .with_idempotency_key(IdempotencyKey::new("idem-123").unwrap())
            .with_correlation_id(CorrelationId::new("corr-456").unwrap());

        let json = serde_json::to_value(options).expect("send options serialize");

        assert_eq!(
            json,
            serde_json::json!({
                "transport_options": {"test": "value"},
                "timeout": {"secs": 3, "nanos": 25},
                "idempotency_key": "idem-123",
                "correlation_id": "corr-456"
            })
        );
    }

    #[test]
    #[cfg(feature = "serde")]
    fn send_options_serialize_envelope() {
        let envelope = Envelope::new(
            Some("sender@example.com".parse().unwrap()),
            vec!["recipient@example.com".parse().unwrap()],
        );
        let options = SendOptions::new().with_envelope(envelope);

        let json = serde_json::to_value(options).expect("send options serialize");

        assert_eq!(json["envelope"]["mail_from"], "sender@example.com");
        assert_eq!(
            json["envelope"]["rcpt_to"],
            serde_json::json!(["recipient@example.com"])
        );
    }

    #[test]
    #[cfg(feature = "schemars")]
    fn transport_options_schema_is_provider_keyed_object() {
        let schema = schemars::schema_for!(TransportOptions);
        let value = schema.as_value();

        assert_eq!(
            value.get("type").and_then(|value| value.as_str()),
            Some("object")
        );
        assert_eq!(
            value
                .get("additionalProperties")
                .and_then(|value| value.as_bool()),
            Some(true)
        );
    }

    #[test]
    #[cfg(feature = "schemars")]
    fn send_options_schema_allows_omitting_transport_options() {
        let schema = schemars::schema_for!(SendOptions);
        let value = schema.as_value();

        assert!(value.pointer("/properties/transport_options").is_some());
        if let Some(required) = value.get("required").and_then(|value| value.as_array()) {
            assert!(
                !required
                    .iter()
                    .any(|value| value.as_str() == Some("transport_options"))
            );
        }
    }

    #[test]
    #[cfg(feature = "serde")]
    fn send_options_deserialize_hydrates_registered_transport_options() {
        let mut registry = TransportOptionRegistry::new();
        registry
            .register::<TestOption>()
            .expect("register succeeds");

        let options = registry
            .deserialize_send_options(serde_json::json!({
                "envelope": {
                    "mail_from": "sender@example.com",
                    "rcpt_to": ["recipient@example.com"]
                },
                "transport_options": {"test": "value"},
                "timeout": {"secs": 3, "nanos": 25},
                "idempotency_key": "idem-123",
                "correlation_id": "corr-456",
                "future_field": "ignored"
            }))
            .expect("send options deserialize");

        let envelope = options.envelope.as_ref().expect("envelope hydrates");
        assert_eq!(
            envelope
                .mail_from()
                .map(email_message::EmailAddress::as_str),
            Some("sender@example.com")
        );
        assert_eq!(
            envelope
                .rcpt_to()
                .iter()
                .map(email_message::EmailAddress::as_str)
                .collect::<Vec<_>>(),
            vec!["recipient@example.com"]
        );
        assert_eq!(options.timeout, Some(std::time::Duration::new(3, 25)));
        assert_eq!(
            options.idempotency_key.as_ref().map(IdempotencyKey::as_str),
            Some("idem-123")
        );
        assert_eq!(
            options.correlation_id.as_ref().map(CorrelationId::as_str),
            Some("corr-456")
        );
        assert_eq!(
            options
                .transport_options
                .get::<TestOption>()
                .map(|value| value.0.as_str()),
            Some("value")
        );
    }

    #[test]
    #[cfg(feature = "serde")]
    fn send_options_deserialize_rejects_unknown_transport_options_by_default() {
        let registry = TransportOptionRegistry::new();

        let result = registry.deserialize_send_options(serde_json::json!({
            "transport_options": {"unknown": {"value": 1}}
        }));

        let error = result.expect_err("unknown provider key should fail");
        assert!(
            error.to_string().contains("unknown"),
            "error should name the unknown provider key, got `{error}`"
        );
    }

    /// Drift guard for [`SendOptionsField`]/[`SendOptionsVisitor`].
    ///
    /// Every public field on [`SendOptions`] must round-trip through the seed
    /// with a non-default value; if a new field is added to the struct but not
    /// wired through the visitor's `match`, the corresponding assertion below
    /// fails and surfaces the drift.
    #[test]
    #[cfg(feature = "serde")]
    fn send_options_seed_round_trip_covers_every_field() {
        let mut transport_options = TransportOptions::default();
        transport_options.insert(TestOption(String::from("typed")));

        let original = SendOptions::new()
            .with_envelope(Envelope::new(
                Some("sender@example.com".parse().unwrap()),
                vec!["recipient@example.com".parse().unwrap()],
            ))
            .with_transport_options(transport_options)
            .with_timeout(std::time::Duration::new(7, 11))
            .with_idempotency_key(IdempotencyKey::new("idem-99").unwrap())
            .with_correlation_id(CorrelationId::new("corr-77").unwrap());

        let json = serde_json::to_value(&original).expect("serialize");

        let mut registry = TransportOptionRegistry::new();
        registry
            .register::<TestOption>()
            .expect("register succeeds");

        let hydrated = registry
            .deserialize_send_options(json)
            .expect("deserialize");

        assert_eq!(
            hydrated
                .envelope
                .as_ref()
                .and_then(|envelope| envelope.mail_from())
                .map(email_message::EmailAddress::as_str),
            Some("sender@example.com"),
            "envelope did not round-trip — did you add a field to SendOptions \
             without extending SendOptionsField/SendOptionsVisitor?"
        );
        assert_eq!(
            hydrated
                .transport_options
                .get::<TestOption>()
                .map(|value| value.0.as_str()),
            Some("typed"),
            "transport_options did not round-trip"
        );
        assert_eq!(
            hydrated.timeout,
            Some(std::time::Duration::new(7, 11)),
            "timeout did not round-trip"
        );
        assert_eq!(
            hydrated.idempotency_key.as_ref().map(IdempotencyKey::as_str),
            Some("idem-99"),
            "idempotency_key did not round-trip"
        );
        assert_eq!(
            hydrated.correlation_id.as_ref().map(CorrelationId::as_str),
            Some("corr-77"),
            "correlation_id did not round-trip"
        );
    }

    #[test]
    #[cfg(feature = "serde")]
    fn send_options_seed_can_ignore_unknown_transport_options() {
        use serde::de::DeserializeSeed as _;

        let mut registry = TransportOptionRegistry::new();
        registry
            .register::<TestOption>()
            .expect("register succeeds");

        let payload = serde_json::json!({
            "transport_options": {
                "test": "value",
                "unknown": {"value": 1}
            }
        });
        let options = registry
            .send_options_seed()
            .ignore_unknown_transport_options()
            .deserialize(payload)
            .expect("ignore_unknown succeeds");

        assert_eq!(
            options
                .transport_options
                .get::<TestOption>()
                .map(|value| value.0.as_str()),
            Some("value")
        );
    }

    #[test]
    #[cfg(feature = "serde")]
    fn send_options_deserialize_rejects_malformed_known_transport_options() {
        let mut registry = TransportOptionRegistry::new();
        registry
            .register::<OtherTestOption>()
            .expect("register succeeds");

        let result = registry.deserialize_send_options(serde_json::json!({
            "transport_options": {"other": "not-an-object"}
        }));

        assert!(result.is_err());
    }

    #[test]
    #[cfg(feature = "serde")]
    fn transport_options_seed_drives_a_streaming_deserializer() {
        // Format-agnostic seed exercise: drive the seed straight off a
        // streaming `serde_json::Deserializer` (not a `serde_json::Value`),
        // which is the path any non-JSON serde format would also take.
        use serde::de::DeserializeSeed as _;

        let mut registry = TransportOptionRegistry::new();
        registry
            .register::<TestOption>()
            .expect("register succeeds");

        let mut original = TransportOptions::default();
        original.insert(TestOption(String::from("round-trip")));

        let bytes = serde_json::to_vec(&original).expect("serialize");
        let mut deserializer = serde_json::Deserializer::from_slice(&bytes);
        let hydrated = registry
            .transport_options_seed()
            .deserialize(&mut deserializer)
            .expect("hydrate from streaming deserializer");

        assert_eq!(
            hydrated.get::<TestOption>().map(|value| value.0.as_str()),
            Some("round-trip")
        );
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
            .hydrate_into("test", serde_json::json!("json"), &mut options)
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
            .hydrate_into("unknown", serde_json::json!({"value": 1}), &mut options)
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
            registry.hydrate_into("other", serde_json::json!("not-an-object"), &mut options);

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
