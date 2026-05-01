use core::any::{Any, TypeId};
use core::future::Future;
use core::pin::Pin;
use std::borrow::Cow;
#[cfg(feature = "serde")]
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::time::Duration;

use email_message::{EmailAddress, Envelope, Header, Message, OutboundMessage};
use time::format_description::well_known::Rfc2822;

/// Transport for providers that accept structured [`OutboundMessage`] values.
///
/// # Method discipline
///
/// Only [`send`](Transport::send) is required. Callers without per-send
/// overrides pass `&SendOptions::default()`. Adapters that require
/// [`TransportOptions`] (advertised through
/// `Capabilities::structured_send == StructuredSendCapability::RequiresTransportOptions`)
/// should return `ErrorKind::UnsupportedFeature` when the required typed slot
/// is missing, so callers can distinguish a capability-mismatch error from
/// a message-validation error.
///
/// [`send_owned`](Transport::send_owned) is the owned-input counterpart.
/// It has a default-forward impl that routes through [`send`](Transport::send)
/// via a borrow. Adapters that can avoid an internal clone (e.g. moving the
/// [`OutboundMessage`] body bytes into a request body) should override
/// `send_owned` directly.
///
/// # Cancellation
///
/// `send` and `send_owned` are *not* cancellation-safe. Dropping the
/// returned future before completion does not guarantee the message was
/// not delivered: HTTP-backed providers may have already accepted the
/// request server-side, and SMTP transports may have torn down the
/// connection mid-handshake with indeterminate state. Use
/// [`SendOptions::idempotency_key`] for replay safety when retries cross
/// a cancellation boundary, and [`SendOptions::timeout`] to bound
/// provider-call duration.
pub trait Transport: RuntimeBound {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            structured_send: StructuredSendCapability::Supported,
            ..Capabilities::default()
        }
    }

    /// Send `message` with the supplied per-send `options`.
    ///
    /// Callers without overrides pass `&SendOptions::default()`. See the
    /// trait-level "Cancellation" section.
    fn send<'a>(
        &'a self,
        message: &'a OutboundMessage,
        options: &'a SendOptions,
    ) -> impl Future<Output = Result<SendReport, TransportError>> + MaybeSend + 'a;

    /// Send `message` (owned) with the supplied per-send `options`.
    ///
    /// Default impl forwards to [`send`](Transport::send) via a borrow.
    /// Override when an adapter can move the [`OutboundMessage`] into a
    /// provider-specific request body without cloning.
    fn send_owned<'a>(
        &'a self,
        message: OutboundMessage,
        options: &'a SendOptions,
    ) -> impl Future<Output = Result<SendReport, TransportError>> + MaybeSend + 'a {
        async move { self.send(&message, options).await }
    }
}

/// Transport for providers that accept a pre-rendered RFC822 message and an
/// explicit envelope (typically SMTP).
///
/// # Envelope source
///
/// The `envelope` argument to [`send_raw`](RawTransport::send_raw) and
/// [`send_raw_owned`](RawTransport::send_raw_owned) is authoritative. Raw
/// transports ignore [`SendOptions::envelope`]; that option exists only for
/// structured [`Transport`] calls where the message is still the primary input
/// and a caller may ask a capable adapter to override its derived envelope.
///
/// # Method discipline
///
/// Only [`send_raw`](RawTransport::send_raw) is required.
/// [`send_raw_owned`](RawTransport::send_raw_owned) carries a default-forward
/// impl that routes through the borrowed method, mirroring [`Transport`].
/// Adapters that can move the envelope and RFC822 bytes into a provider
/// API without cloning (e.g. lettre's SMTP state machine) should override
/// `send_raw_owned` directly.
///
/// # Cancellation
///
/// `send_raw` and `send_raw_owned` are *not* cancellation-safe. Dropping the
/// returned future may leave the underlying connection in indeterminate state
/// (mid-`DATA`, mid-`RCPT`, etc.) and the provider may have already accepted
/// the message. Use [`SendOptions::idempotency_key`] for replay safety where
/// the provider supports it.
pub trait RawTransport: RuntimeBound {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            raw_rfc822: true,
            custom_envelope: true,
            ..Capabilities::default()
        }
    }

    /// Send the pre-rendered RFC822 `rfc822` bytes with the supplied
    /// authoritative `envelope`.
    ///
    /// [`SendOptions::envelope`] is ignored on this path. See the trait-level
    /// "Envelope source" and "Cancellation" sections.
    fn send_raw<'a>(
        &'a self,
        envelope: &'a Envelope,
        rfc822: &'a [u8],
        options: &'a SendOptions,
    ) -> impl Future<Output = Result<SendReport, TransportError>> + MaybeSend + 'a;

    /// Owned variant of [`RawTransport::send_raw`] for callers that already
    /// own the authoritative envelope and bytes and want to hand them over
    /// without an extra borrow.
    ///
    /// Default impl forwards to [`send_raw`](RawTransport::send_raw) via a
    /// borrow. Override when an adapter can move the envelope and bytes into
    /// a provider-specific API without cloning (e.g. lettre's SMTP state
    /// machine).
    fn send_raw_owned<'a>(
        &'a self,
        envelope: Envelope,
        rfc822: Vec<u8>,
        options: &'a SendOptions,
    ) -> impl Future<Output = Result<SendReport, TransportError>> + MaybeSend + 'a {
        async move { self.send_raw(&envelope, &rfc822, options).await }
    }
}

crate::string_newtype! {
    /// Provider idempotency token for safe retries.
    ///
    /// Validated on construction, empty, NUL, CR/LF, non-tab control
    /// characters, and values longer than 1 KiB are rejected. Adapters
    /// that advertise [`Capabilities::idempotency_key`] forward the value
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

#[derive(Default)]
#[non_exhaustive]
pub struct SendOptions {
    /// Optional custom SMTP envelope for structured [`Transport`] sends.
    ///
    /// This is only meaningful for structured transports that advertise
    /// [`Capabilities::custom_envelope`]. Other structured transports may ignore
    /// it. [`RawTransport`] methods take an explicit [`Envelope`] argument, and
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

/// Advertised feature set of a [`Transport`] or [`RawTransport`].
///
/// Capabilities are advisory by default: the trait contracts do not
/// auto-validate per-send `SendOptions` against advertised flags. Callers
/// consult `capabilities()` before constructing options and skip features
/// the transport does not support; adapters silently ignore unsupported
/// options unless the flag's tier below says otherwise.
///
/// # Tiers
///
/// **Enforced.** [`StructuredSendCapability::RequiresTransportOptions`] is
/// the one capability the kernel turns into a hard error: adapters that
/// advertise it return [`ErrorKind::UnsupportedFeature`] from
/// [`Transport::send`] when the required typed slot is missing.
///
/// **Honored when present.** `idempotency_key` and `timeout` are read by
/// adapters that advertise them and ignored by adapters that do not. The
/// kernel does not check that an advertising adapter actually applies the
/// option.
///
/// **Hints.** `raw_rfc822`, `custom_envelope`, `custom_headers`,
/// `attachments`, `inline_attachments` are purely declarative. They
/// communicate intent to callers; the kernel neither validates inputs
/// against the flag nor checks that an advertising adapter handles them
/// correctly.
///
/// # Limits
///
/// The struct is a flat set of advisory booleans plus one tri-state
/// (`structured_send`). It does **not** model:
///
/// - **Per-field cardinality.** "Postmark requires at least one `To`
///   recipient", "Loops accepts exactly one recipient", "Mailgun caps
///   `bcc` at N" are not expressible. Such constraints surface as
///   [`ErrorKind::Validation`] from [`Transport::send`] when the
///   adapter rejects the shape.
/// - **Per-provider required fields.** Loops's `transactional_id`
///   requirement is enforced via
///   [`StructuredSendCapability::RequiresTransportOptions`] plus an
///   adapter-side check; the kernel cannot otherwise advertise
///   "field X must be present".
/// - **Body-shape constraints.** Whether an adapter accepts only
///   `Body::Text`, only `Body::Html`, both, or arbitrary `Body::Mime`
///   trees is not advertised.
/// - **Custom-envelope semantics.** The `custom_envelope` flag today
///   says "the adapter has an envelope concept", not "the adapter
///   honors [`SendOptions::envelope`] verbatim". Structured HTTP
///   adapters that build the provider request from
///   `message.to/cc/bcc` may still advertise it. See the
///   [`SendReport::accepted`] caveat for the resulting reporting
///   asymmetry under an envelope override.
///
/// Callers should be prepared for [`ErrorKind::Validation`] (or
/// [`ErrorKind::UnsupportedFeature`] in the
/// `RequiresTransportOptions` case) from [`Transport::send`] even when
/// `capabilities()` looks compatible with their inputs.
///
/// # Example
///
/// The worker layer reads capabilities to decide whether to forward an
/// `idempotency_key` from the queue payload.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[allow(clippy::struct_excessive_bools)]
pub struct Capabilities {
    pub raw_rfc822: bool,
    pub structured_send: StructuredSendCapability,
    pub custom_envelope: bool,
    pub custom_headers: bool,
    pub attachments: bool,
    pub inline_attachments: bool,
    pub idempotency_key: bool,
    pub timeout: bool,
}

impl Capabilities {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub const fn with_raw_rfc822(mut self, value: bool) -> Self {
        self.raw_rfc822 = value;
        self
    }

    #[must_use]
    pub const fn with_structured_send(mut self, value: StructuredSendCapability) -> Self {
        self.structured_send = value;
        self
    }

    #[must_use]
    pub const fn with_custom_envelope(mut self, value: bool) -> Self {
        self.custom_envelope = value;
        self
    }

    #[must_use]
    pub const fn with_custom_headers(mut self, value: bool) -> Self {
        self.custom_headers = value;
        self
    }

    #[must_use]
    pub const fn with_attachments(mut self, value: bool) -> Self {
        self.attachments = value;
        self
    }

    #[must_use]
    pub const fn with_inline_attachments(mut self, value: bool) -> Self {
        self.inline_attachments = value;
        self
    }

    #[must_use]
    pub const fn with_idempotency_key(mut self, value: bool) -> Self {
        self.idempotency_key = value;
        self
    }

    #[must_use]
    pub const fn with_timeout(mut self, value: bool) -> Self {
        self.timeout = value;
        self
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum StructuredSendCapability {
    #[default]
    Unsupported,
    Supported,
    RequiresTransportOptions,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SendReport {
    pub provider: Cow<'static, str>,
    pub provider_message_id: Option<String>,
    /// Recipient list the adapter logically accepted for handoff.
    ///
    /// The default derivation, computed by [`structured_accepted_for`],
    /// returns `options.envelope.rcpt_to` only when the caller supplies an
    /// override **and** the transport advertises
    /// [`Capabilities::custom_envelope`]. Otherwise it reports the
    /// recipients implied by the message itself (`To`, `Cc`, and `Bcc`).
    ///
    /// For [`RawTransport`] adapters (Lettre / SMTP) this matches the
    /// recipient list actually handed to the provider's `RCPT TO` step.
    ///
    /// Single-recipient providers (Loops) populate the single address that
    /// was actually handed to the provider; that adapter's API does not
    /// support multi-recipient delivery, so the list reflects what the
    /// adapter sent rather than what was on the message.
    ///
    /// Adapters do **not** consult provider responses to populate this field;
    /// for provider-confirmed deliveries see
    /// [`SendReport::provider_message_id`] and the provider's webhook events.
    ///
    /// Unsupported [`SendOptions::envelope`] overrides remain advisory and
    /// may be ignored by transports that do not advertise
    /// [`Capabilities::custom_envelope`]; they are not reflected in this
    /// field unless the adapter actually honors them.
    pub accepted: Vec<EmailAddress>,
}

impl SendReport {
    #[must_use]
    pub fn new(provider: impl Into<Cow<'static, str>>) -> Self {
        Self {
            provider: provider.into(),
            provider_message_id: None,
            accepted: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_provider_message_id(mut self, id: impl Into<String>) -> Self {
        self.provider_message_id = Some(id.into());
        self
    }

    #[must_use]
    pub fn with_accepted<I>(mut self, accepted: I) -> Self
    where
        I: IntoIterator<Item = EmailAddress>,
    {
        self.accepted = accepted.into_iter().collect();
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
    Validation,
    Authentication,
    Authorization,
    RateLimited,
    /// In-flight provider/network call timed out. Retryable, a fresh
    /// attempt may complete within the budget.
    Timeout,
    TransientNetwork,
    TransientProvider,
    PermanentProvider,
    UnsupportedFeature,
    Internal,
}

impl ErrorKind {
    /// Map an HTTP status code to a canonical [`ErrorKind`].
    ///
    /// Intended for the **failure path only**, call it on the status of a
    /// non-success response. 1xx, 2xx, and 3xx codes are not failures and
    /// the mapping for them ([`ErrorKind::PermanentProvider`]) is not
    /// meaningful; check `StatusCode::is_success` (or equivalent) before
    /// reaching for this constructor.
    ///
    /// The mapping:
    ///
    /// - `400 | 422` -> [`ErrorKind::Validation`]
    /// - `401` -> [`ErrorKind::Authentication`]
    /// - `403` -> [`ErrorKind::Authorization`]
    /// - `408` -> [`ErrorKind::Timeout`] (RFC 7231 §6.5.7, explicitly retryable)
    /// - `425` -> [`ErrorKind::TransientNetwork`] (RFC 8470, Too Early)
    /// - `429` -> [`ErrorKind::RateLimited`]
    /// - `501 | 505 | 510 | 511` -> [`ErrorKind::PermanentProvider`] (terminal
    ///   server-side errors; retrying produces the same result)
    /// - other `5xx` -> [`ErrorKind::TransientProvider`]
    /// - everything else (including unrecognized `4xx`) ->
    ///   [`ErrorKind::PermanentProvider`]
    ///
    /// Adapters that need provider-specific quirks (e.g. Loops mapping `404`
    /// and `409` to [`ErrorKind::Validation`]) should match those codes
    /// inline before falling through to this constructor.
    #[must_use]
    pub const fn from_http_status(status: u16) -> Self {
        match status {
            400 | 422 => Self::Validation,
            401 => Self::Authentication,
            403 => Self::Authorization,
            408 => Self::Timeout,
            425 => Self::TransientNetwork,
            429 => Self::RateLimited,
            501 | 505 | 510 | 511 => Self::PermanentProvider,
            500..=599 => Self::TransientProvider,
            _ => Self::PermanentProvider,
        }
    }
}

impl std::fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::Validation => "validation",
            Self::Authentication => "authentication",
            Self::Authorization => "authorization",
            Self::RateLimited => "rate-limited",
            Self::Timeout => "timeout",
            Self::TransientNetwork => "transient-network",
            Self::TransientProvider => "transient-provider",
            Self::PermanentProvider => "permanent-provider",
            Self::UnsupportedFeature => "unsupported-feature",
            Self::Internal => "internal",
        };
        f.write_str(label)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{kind}: {message}")]
#[non_exhaustive]
pub struct TransportError {
    pub kind: ErrorKind,
    pub message: String,
    /// HTTP status code from the provider response, when applicable. Use
    /// [`TransportError::with_http_status`] to set. Adapters whose
    /// underlying protocol is not HTTP (e.g. SMTP via Lettre) leave this
    /// `None` and surface protocol-specific reply codes through
    /// [`TransportError::provider_error_code`] instead.
    pub http_status: Option<u16>,
    pub provider_error_code: Option<String>,
    pub request_id: Option<String>,
    pub retry_after: Option<Duration>,
    /// Underlying source error chain. Read through the
    /// [`std::error::Error::source`] impl; the field is private so the
    /// kernel can change the boxing strategy without breaking callers.
    #[source]
    source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
}

impl TransportError {
    #[must_use]
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            http_status: None,
            provider_error_code: None,
            request_id: None,
            retry_after: None,
            source: None,
        }
    }

    /// Records the HTTP status code from the provider response.
    #[must_use]
    pub const fn with_http_status(mut self, status: u16) -> Self {
        self.http_status = Some(status);
        self
    }

    #[must_use]
    pub fn with_provider_error_code(mut self, code: impl Into<String>) -> Self {
        self.provider_error_code = Some(code.into());
        self
    }

    #[must_use]
    pub const fn with_retry_after(mut self, retry_after: Duration) -> Self {
        self.retry_after = Some(retry_after);
        self
    }

    #[must_use]
    pub fn with_source(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        matches!(
            self.kind,
            ErrorKind::RateLimited
                | ErrorKind::Timeout
                | ErrorKind::TransientNetwork
                | ErrorKind::TransientProvider
        )
    }

    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        !self.is_retryable()
    }

    #[must_use]
    pub const fn is_timeout(&self) -> bool {
        matches!(self.kind, ErrorKind::Timeout)
    }
}

/// Marker trait that resolves to `Send + Sync` on native targets and to no
/// bound on `wasm32`.
///
/// Used as the runtime supertrait for [`Transport`] and [`RawTransport`].
/// On native, every receiver must be `Send + Sync` because send futures may be
/// driven on any runtime thread; on `wasm32` the bound is dropped to
/// match the single-threaded browser/worker future model.
///
/// This is the same shape as [`MaybeSend`] but for the receiver instead of
/// the future. Together they let one trait declaration cover both targets.
///
/// You should not implement this trait directly; the blanket impl below
/// covers every type that satisfies the underlying bound.
#[cfg(not(target_arch = "wasm32"))]
pub trait RuntimeBound: Send + Sync {}

#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + Sync + ?Sized> RuntimeBound for T {}

/// Marker trait that resolves to `Send + Sync` on native targets and to no
/// bound on `wasm32`. See the native-target docs for the full rationale.
#[cfg(target_arch = "wasm32")]
pub trait RuntimeBound {}

#[cfg(target_arch = "wasm32")]
impl<T: ?Sized> RuntimeBound for T {}

/// Trait alias that resolves to [`Send`] on native targets and to no bound
/// on `wasm32`.
///
/// Used in the AFIT method return positions of [`Transport`] and
/// [`RawTransport`] so a single trait declaration covers both platforms.
/// On native, every returned future must be `Send` so caller-side
/// orchestrators can `tokio::spawn` them; on `wasm32`, browser-runtime
/// futures hold `!Send` JS handles (`web_sys::JsValue`,
/// `worker::Request`), so requiring `Send` would fail to compile for
/// any wasm transport.
///
/// This is the matrix-rust-sdk `SendOutsideWasm` pattern documented in
/// [matrix-org/matrix-rust-sdk#5082](https://github.com/matrix-org/matrix-rust-sdk/pull/5082).
///
/// # Implementation note
///
/// Do not implement this trait directly. The blanket impl below covers every
/// type that satisfies the underlying bound on each target, and the auto-trait
/// rules of `async fn` propagate `Send`-ness through the marker automatically:
/// an `async` block whose captures are `Send` produces a `Send` future, which
/// then satisfies `MaybeSend` via the blanket.
///
/// # Future
///
/// This marker exists because [return-type notation (RFC 3654)](https://github.com/rust-lang/rust/issues/109417)
/// is not stable. Once it stabilizes, callers will be able to write
/// `where T::send_with(..): Send` at spawn sites and this trait can be
/// deleted. The stabilization push closed unmerged in late 2025
/// ([rust-lang/rust#138424](https://github.com/rust-lang/rust/pull/138424));
/// no near-term replacement is on the roadmap. If the wasm story ever needs
/// to diverge from native independently of RTN, the planned escape hatch is
/// the `async-graphql` `boxed-trait` feature flag pattern.
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSend: Send {}

#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + ?Sized> MaybeSend for T {}

/// Trait alias that resolves to [`Send`] on native targets and to no bound
/// on `wasm32`. See the native-target docs for the full rationale.
#[cfg(target_arch = "wasm32")]
pub trait MaybeSend {}

#[cfg(target_arch = "wasm32")]
impl<T: ?Sized> MaybeSend for T {}

#[cfg(target_arch = "wasm32")]
#[doc(hidden)]
pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

#[cfg(not(target_arch = "wasm32"))]
#[doc(hidden)]
pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

mod sealed {
    pub trait ErasedTransport {}
    pub trait ErasedRawTransport {}
}

/// Object-safe adapter for [`Transport`].
///
/// Sealed: only types that implement [`Transport`] satisfy this trait.
/// Hold trait objects through [`DynTransport`] / [`SharedTransport`]; do not
/// name `ErasedTransport` directly. The exact erasure mechanism (boxed
/// futures today, possibly RTN later) is an implementation detail.
pub trait ErasedTransport: RuntimeBound + sealed::ErasedTransport {
    fn capabilities(&self) -> Capabilities;

    fn send<'a>(
        &'a self,
        message: &'a OutboundMessage,
        options: &'a SendOptions,
    ) -> BoxFut<'a, Result<SendReport, TransportError>>;

    fn send_owned<'a>(
        &'a self,
        message: OutboundMessage,
        options: &'a SendOptions,
    ) -> BoxFut<'a, Result<SendReport, TransportError>>;
}

impl<T: Transport + ?Sized> sealed::ErasedTransport for T {}

impl<T> ErasedTransport for T
where
    T: Transport + ?Sized,
{
    fn capabilities(&self) -> Capabilities {
        Transport::capabilities(self)
    }

    fn send<'a>(
        &'a self,
        message: &'a OutboundMessage,
        options: &'a SendOptions,
    ) -> BoxFut<'a, Result<SendReport, TransportError>> {
        Box::pin(Transport::send(self, message, options))
    }

    fn send_owned<'a>(
        &'a self,
        message: OutboundMessage,
        options: &'a SendOptions,
    ) -> BoxFut<'a, Result<SendReport, TransportError>> {
        Box::pin(Transport::send_owned(self, message, options))
    }
}

/// Object-safe raw transport adapter for [`RawTransport`].
///
/// Sealed: only types that implement [`RawTransport`] satisfy this trait.
/// Hold trait objects through [`DynRawTransport`] / [`SharedRawTransport`].
pub trait ErasedRawTransport: RuntimeBound + sealed::ErasedRawTransport {
    fn capabilities(&self) -> Capabilities;

    fn send_raw<'a>(
        &'a self,
        envelope: &'a Envelope,
        rfc822: &'a [u8],
        options: &'a SendOptions,
    ) -> BoxFut<'a, Result<SendReport, TransportError>>;

    fn send_raw_owned<'a>(
        &'a self,
        envelope: Envelope,
        rfc822: Vec<u8>,
        options: &'a SendOptions,
    ) -> BoxFut<'a, Result<SendReport, TransportError>>;
}

impl<T: RawTransport + ?Sized> sealed::ErasedRawTransport for T {}

impl<T> ErasedRawTransport for T
where
    T: RawTransport + ?Sized,
{
    fn capabilities(&self) -> Capabilities {
        RawTransport::capabilities(self)
    }

    fn send_raw<'a>(
        &'a self,
        envelope: &'a Envelope,
        rfc822: &'a [u8],
        options: &'a SendOptions,
    ) -> BoxFut<'a, Result<SendReport, TransportError>> {
        Box::pin(RawTransport::send_raw(self, envelope, rfc822, options))
    }

    fn send_raw_owned<'a>(
        &'a self,
        envelope: Envelope,
        rfc822: Vec<u8>,
        options: &'a SendOptions,
    ) -> BoxFut<'a, Result<SendReport, TransportError>> {
        Box::pin(RawTransport::send_raw_owned(
            self, envelope, rfc822, options,
        ))
    }
}

/// Object-safe structured transport trait object.
pub type DynTransport = dyn ErasedTransport;

/// Shared structured transport handle.
pub type SharedTransport = std::sync::Arc<DynTransport>;

/// Object-safe raw transport trait object.
pub type DynRawTransport = dyn ErasedRawTransport;

/// Shared raw transport handle.
pub type SharedRawTransport = std::sync::Arc<DynRawTransport>;

/// Returns all envelope recipient [`EmailAddress`]s implied by `To`, `Cc`, and `Bcc`.
#[must_use]
pub fn accepted_recipient_emails(message: &Message) -> Vec<EmailAddress> {
    message
        .to()
        .iter()
        .chain(message.cc())
        .chain(message.bcc())
        .flat_map(email_message::Address::mailboxes)
        .map(|mailbox| mailbox.email().clone())
        .collect()
}

/// Returns the [`SendReport::accepted`] list per the documented spec.
///
/// Yields `options.envelope.rcpt_to().to_vec()` only when the caller supplied
/// an envelope override and `capabilities.custom_envelope` is true; otherwise
/// yields [`accepted_recipient_emails`]. Use this from every structured
/// [`Transport`] adapter whose accepted-recipient semantics match its
/// capabilities so the field reports what the adapter actually attempted to
/// hand to the provider.
#[must_use]
pub fn structured_accepted_for(
    message: &Message,
    options: &SendOptions,
    capabilities: Capabilities,
) -> Vec<EmailAddress> {
    if capabilities.custom_envelope
        && let Some(envelope) = options.envelope.as_ref()
    {
        return envelope.rcpt_to().to_vec();
    }

    accepted_recipient_emails(message)
}

/// Builds standard structured headers that provider APIs usually model as
/// custom headers rather than first-class request fields.
///
/// # Errors
///
/// Returns [`TransportError`] if the message date cannot be formatted as an
/// RFC 2822 header value.
pub fn standard_message_headers(message: &Message) -> Result<Vec<Header>, TransportError> {
    let mut headers = Vec::new();

    if let Some(sender) = message.sender() {
        headers.push(
            Header::new("Sender", sender.to_string())
                .map_err(|error| TransportError::new(ErrorKind::Validation, error.to_string()))?,
        );
    }

    if let Some(date) = message.date() {
        headers.push(
            Header::new(
                "Date",
                date.format(&Rfc2822).map_err(|error| {
                    TransportError::new(ErrorKind::Validation, error.to_string())
                })?,
            )
            .map_err(|error| TransportError::new(ErrorKind::Validation, error.to_string()))?,
        );
    }

    if let Some(message_id) = message.message_id() {
        headers.push(
            Header::new("Message-ID", message_id.to_string())
                .map_err(|error| TransportError::new(ErrorKind::Validation, error.to_string()))?,
        );
    }

    Ok(headers)
}

#[cfg(test)]
mod tests {
    use email_message::{Address, Body, EmailAddress, Envelope, Message};

    use crate::{STRING_NEWTYPE_MAX_BYTES, StringNewtypeError};

    #[cfg(feature = "serde")]
    use super::CorrelationId;
    use super::{
        Capabilities, ErrorKind, IdempotencyKey, SendOptions, StructuredSendCapability,
        TransportError, TransportOption, TransportOptions, structured_accepted_for,
    };
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

    fn message_with_recipient(recipient: &str) -> Message {
        Message::builder(Body::text("hello"))
            .from_mailbox("sender@example.com".parse().expect("sender parses"))
            .to(vec![Address::Mailbox(
                recipient.parse().expect("recipient parses"),
            )])
            .build()
            .expect("message validates")
    }

    fn options_with_envelope(recipient: &str) -> SendOptions {
        SendOptions::new().with_envelope(Envelope::new(
            Some(
                "bounce@example.com"
                    .parse::<EmailAddress>()
                    .expect("from parses"),
            ),
            vec![recipient.parse::<EmailAddress>().expect("rcpt parses")],
        ))
    }

    fn accepted_strings(accepted: &[EmailAddress]) -> Vec<&str> {
        accepted.iter().map(EmailAddress::as_str).collect()
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

    #[test]
    fn capabilities_default_to_false() {
        assert_eq!(
            Capabilities::default(),
            Capabilities {
                raw_rfc822: false,
                structured_send: StructuredSendCapability::Unsupported,
                custom_envelope: false,
                custom_headers: false,
                attachments: false,
                inline_attachments: false,
                idempotency_key: false,
                timeout: false,
            }
        );
    }

    #[test]
    fn structured_accepted_ignores_envelope_when_capability_is_false() {
        let message = message_with_recipient("message@example.com");
        let options = options_with_envelope("envelope@example.com");

        let accepted = structured_accepted_for(&message, &options, Capabilities::new());

        assert_eq!(accepted_strings(&accepted), vec!["message@example.com"]);
    }

    #[test]
    fn structured_accepted_uses_envelope_when_capability_is_true() {
        let message = message_with_recipient("message@example.com");
        let options = options_with_envelope("envelope@example.com");
        let capabilities = Capabilities::new().with_custom_envelope(true);

        let accepted = structured_accepted_for(&message, &options, capabilities);

        assert_eq!(accepted_strings(&accepted), vec!["envelope@example.com"]);
    }

    #[test]
    fn transport_error_kind_predicates_classify_each_variant() {
        let cases = [
            (ErrorKind::Validation, "validation"),
            (ErrorKind::Authentication, "auth"),
            (ErrorKind::Authorization, "authz"),
            (ErrorKind::RateLimited, "rate"),
            (ErrorKind::Timeout, "timeout"),
            (ErrorKind::TransientNetwork, "net"),
            (ErrorKind::TransientProvider, "transient"),
            (ErrorKind::PermanentProvider, "permanent"),
            (ErrorKind::UnsupportedFeature, "unsupported"),
            (ErrorKind::Internal, "internal"),
        ];

        for (kind, label) in cases {
            let err = TransportError::new(kind.clone(), label);

            let retryable = matches!(
                kind,
                ErrorKind::RateLimited
                    | ErrorKind::Timeout
                    | ErrorKind::TransientNetwork
                    | ErrorKind::TransientProvider
            );
            assert_eq!(err.is_retryable(), retryable, "{label}: is_retryable");
            assert_eq!(err.is_terminal(), !retryable, "{label}: is_terminal");

            assert_eq!(
                err.is_timeout(),
                matches!(kind, ErrorKind::Timeout),
                "{label}: is_timeout"
            );
        }
    }

    #[test]
    fn from_http_status_maps_documented_codes() {
        assert_eq!(ErrorKind::from_http_status(400), ErrorKind::Validation);
        assert_eq!(ErrorKind::from_http_status(422), ErrorKind::Validation);
        assert_eq!(ErrorKind::from_http_status(401), ErrorKind::Authentication);
        assert_eq!(ErrorKind::from_http_status(403), ErrorKind::Authorization);
        assert_eq!(ErrorKind::from_http_status(408), ErrorKind::Timeout);
        assert_eq!(
            ErrorKind::from_http_status(425),
            ErrorKind::TransientNetwork
        );
        assert_eq!(ErrorKind::from_http_status(429), ErrorKind::RateLimited);
        assert_eq!(
            ErrorKind::from_http_status(500),
            ErrorKind::TransientProvider
        );
        assert_eq!(
            ErrorKind::from_http_status(599),
            ErrorKind::TransientProvider
        );
        for code in [501u16, 505, 510, 511] {
            assert_eq!(
                ErrorKind::from_http_status(code),
                ErrorKind::PermanentProvider,
                "code {code}"
            );
        }
        assert_eq!(
            ErrorKind::from_http_status(418),
            ErrorKind::PermanentProvider
        );
    }

    #[test]
    fn from_http_status_408_is_retryable_timeout() {
        let kind = ErrorKind::from_http_status(408);
        assert_eq!(kind, ErrorKind::Timeout);
        let err = TransportError::new(kind, "request timeout");
        assert!(err.is_retryable());
        assert!(err.is_timeout());
    }

    #[test]
    fn from_http_status_terminal_5xx_is_not_retryable() {
        for code in [501u16, 505, 510, 511] {
            let kind = ErrorKind::from_http_status(code);
            let err = TransportError::new(kind, "terminal");
            assert!(!err.is_retryable(), "{code} must not be retryable");
            assert!(err.is_terminal(), "{code} must be terminal");
        }
    }
}
