//! SDK-independent request and response contract for the email service.

use std::collections::BTreeMap;
use std::time::Duration;

use email_message::OutboundMessage;
use email_transport::{
    CorrelationId, IdempotencyKey, SendOptions, SendReport, TransportOptionRegistry, string_newtype,
};
use serde::{Deserialize, Serialize};

string_newtype! {
    /// Configured transport key (e.g. `"primary"`, `"fallback"`).
    ///
    /// End-user code should use [`Self::new`] or [`std::str::FromStr`] for
    /// values originating outside trusted code paths.
    @unchecked TransportKey
}

/// Queue payload consumed by `Email.send`.
///
/// Provider-specific [`RawSendOptions::transport_options`] are a best-effort
/// union. Callers may include options for every provider they support; the
/// selected transport consumes its own registered provider slice, while
/// unrecognized providers are ignored. Switching the selected transport may
/// therefore drop provider-specific behavior.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schemars", schemars(example = example_send_request()))]
pub struct SendRequest {
    /// Configured transport profile to use for this send.
    pub transport: TransportKey,
    /// Validated outbound message payload.
    pub message: OutboundMessage,
    /// Send-time metadata and best-effort provider-specific transport options.
    #[serde(default)]
    #[cfg_attr(feature = "schemars", schemars(default))]
    pub options: RawSendOptions,
}

/// Wire-friendly send options whose provider-specific slots are still raw.
///
/// Unknown fields are rejected so additions to [`SendOptions`] cannot be
/// silently discarded before this staging type is updated to match.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct RawSendOptions {
    /// Optional envelope override for structured transports that support it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub envelope: Option<email_message::Envelope>,
    /// Provider-keyed raw transport options to hydrate with a registry.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[cfg_attr(
        feature = "schemars",
        schemars(default, with = "BTreeMap<String, serde_json::Value>")
    )]
    pub transport_options: BTreeMap<String, serde_value::Value>,
    /// Per-send timeout forwarded to transports that honor timeout metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<Duration>,
    /// Provider-facing idempotency key, when supported by the selected transport.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<IdempotencyKey>,
    /// Caller-supplied correlation id for tracing provider requests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<CorrelationId>,
}

impl RawSendOptions {
    /// Convert typed send options to their provider-keyed wire representation.
    ///
    /// # Errors
    ///
    /// Returns a serialization error when a typed provider option cannot be
    /// represented by the JSON wire contract or the serialized fields no
    /// longer match this staging type.
    pub fn from_send_options(options: &SendOptions) -> Result<Self, serde_json::Error> {
        serde_json::from_value(serde_json::to_value(options)?)
    }

    /// Hydrate raw provider options and assemble typed [`SendOptions`].
    ///
    /// # Errors
    ///
    /// Returns [`serde_value::DeserializerError`] when an option for a
    /// registered provider cannot be hydrated. Unregistered provider keys are
    /// ignored.
    pub fn into_send_options(
        self,
        registry: &TransportOptionRegistry,
    ) -> Result<SendOptions, serde_value::DeserializerError> {
        use serde::de::DeserializeSeed as _;

        let mut options = SendOptions::new();

        if let Some(envelope) = self.envelope {
            options = options.with_envelope(envelope);
        }
        if !self.transport_options.is_empty() {
            let transport_options_value = serde_value::Value::Map(
                self.transport_options
                    .into_iter()
                    .map(|(key, value)| (serde_value::Value::String(key), value))
                    .collect(),
            );
            let transport_options = registry
                .transport_options_seed()
                .ignore_unknown_provider_keys()
                .deserialize(transport_options_value)?;
            options = options.with_transport_options(transport_options);
        }
        if let Some(timeout) = self.timeout {
            options = options.with_timeout(timeout);
        }
        if let Some(idempotency_key) = self.idempotency_key {
            options = options.with_idempotency_key(idempotency_key);
        }
        if let Some(correlation_id) = self.correlation_id {
            options = options.with_correlation_id(correlation_id);
        }

        Ok(options)
    }

    /// Hydrate a borrowed raw option set into typed [`SendOptions`].
    ///
    /// # Errors
    ///
    /// Returns [`serde_value::DeserializerError`] when an option for a
    /// registered provider cannot be hydrated. Unregistered provider keys are
    /// ignored.
    pub fn to_send_options(
        &self,
        registry: &TransportOptionRegistry,
    ) -> Result<SendOptions, serde_value::DeserializerError> {
        self.clone().into_send_options(registry)
    }
}

/// Wire-stable response shape returned by the Restate `Email.send` handler.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schemars", schemars(example = example_send_response()))]
#[non_exhaustive]
pub struct SendResponse {
    /// Provider send report returned by the selected transport.
    pub report: SendReport,
}

impl From<SendReport> for SendResponse {
    fn from(report: SendReport) -> Self {
        Self { report }
    }
}

#[cfg(feature = "schemars")]
fn example_send_response() -> serde_json::Value {
    serde_json::json!({
        "report": {
            "provider": "your-provider",
            "provider_message_id": "184fa9a3-f967-4a98-9d8f-57152e7cbe64",
            "accepted": ["alice@example.com", "bob@example.com"],
        },
    })
}

#[cfg(feature = "schemars")]
fn example_send_request() -> serde_json::Value {
    serde_json::json!({
        "transport": "your-transport",
        "options": {
            "idempotency_key": "foo",
            "transport_options": {
                "your-transport": {"tags": [{"name": "campaign", "value": "test"}]}
            },
        },
        "message": {
            "from": {"type": "mailbox", "name": "Alice", "email": "alice@example.com"},
            "to": [{"type": "mailbox", "name": "Bob", "email": "bob@example.com"}],
            "subject": "Test email",
            "body": {"type": "text", "text": "Hello everyone! This is a test email."},
        },
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use email_message::{EmailAddress, Envelope};
    use email_transport::{
        CorrelationId, IdempotencyKey, SendOptions, TransportOption, TransportOptionRegistry,
    };
    use serde::{Deserialize, Serialize};

    use super::RawSendOptions;

    #[derive(Debug, Deserialize, Serialize)]
    struct TestTransportOption {
        value: String,
    }

    impl TransportOption for TestTransportOption {
        fn provider_key() -> &'static str {
            "mapping-test"
        }
    }

    /// Drift guard for the [`SendOptions`] <-> [`RawSendOptions`] mapping.
    ///
    /// `SendOptions` is non-exhaustive in a dependency, so its fields cannot be
    /// exhaustively destructured here. The schema test below guards that side
    /// of the mapping. For the local type, the destructure below intentionally
    /// has no `..`, making a new `RawSendOptions` field a compile error until a
    /// non-default assertion is added for both conversion directions.
    #[test]
    fn send_options_mapping_covers_every_field_in_both_directions() {
        let mut original = SendOptions::new()
            .with_envelope(Envelope::new(
                Some("sender@example.com".parse().unwrap()),
                vec!["recipient@example.com".parse().unwrap()],
            ))
            .with_timeout(Duration::new(7, 11))
            .with_idempotency_key(IdempotencyKey::new("idem-99").unwrap())
            .with_correlation_id(CorrelationId::new("corr-77").unwrap());
        original.transport_options.insert(TestTransportOption {
            value: String::from("typed"),
        });

        let raw = RawSendOptions::from_send_options(&original).expect("convert to raw options");

        let RawSendOptions {
            envelope,
            transport_options,
            timeout,
            idempotency_key,
            correlation_id,
        } = &raw;
        assert_eq!(
            envelope
                .as_ref()
                .and_then(Envelope::mail_from)
                .map(EmailAddress::as_str),
            Some("sender@example.com"),
            "from_send_options dropped envelope"
        );
        assert!(
            transport_options.contains_key(TestTransportOption::provider_key()),
            "from_send_options dropped transport_options"
        );
        assert_eq!(
            *timeout,
            Some(Duration::new(7, 11)),
            "from_send_options dropped timeout"
        );
        assert_eq!(
            idempotency_key.as_ref().map(IdempotencyKey::as_str),
            Some("idem-99"),
            "from_send_options dropped idempotency_key"
        );
        assert_eq!(
            correlation_id.as_ref().map(CorrelationId::as_str),
            Some("corr-77"),
            "from_send_options dropped correlation_id"
        );

        let mut registry = TransportOptionRegistry::new();
        registry
            .register::<TestTransportOption>()
            .expect("register test transport option");
        let hydrated = raw
            .into_send_options(&registry)
            .expect("hydrate send options");

        assert_eq!(
            hydrated
                .envelope
                .as_ref()
                .and_then(Envelope::mail_from)
                .map(EmailAddress::as_str),
            Some("sender@example.com"),
            "into_send_options dropped envelope"
        );
        assert_eq!(
            hydrated
                .transport_options
                .get::<TestTransportOption>()
                .map(|option| option.value.as_str()),
            Some("typed"),
            "into_send_options dropped transport_options"
        );
        assert_eq!(
            hydrated.timeout,
            Some(Duration::new(7, 11)),
            "into_send_options dropped timeout"
        );
        assert_eq!(
            hydrated
                .idempotency_key
                .as_ref()
                .map(IdempotencyKey::as_str),
            Some("idem-99"),
            "into_send_options dropped idempotency_key"
        );
        assert_eq!(
            hydrated.correlation_id.as_ref().map(CorrelationId::as_str),
            Some("corr-77"),
            "into_send_options dropped correlation_id"
        );
    }

    #[cfg(feature = "schemars")]
    #[test]
    fn send_options_and_raw_send_options_have_the_same_fields() {
        fn field_names<T: schemars::JsonSchema>() -> std::collections::BTreeSet<String> {
            schemars::schema_for!(T)
                .as_value()
                .get("properties")
                .and_then(serde_json::Value::as_object)
                .expect("send options schema should have object properties")
                .keys()
                .cloned()
                .collect()
        }

        assert_eq!(
            field_names::<SendOptions>(),
            field_names::<RawSendOptions>(),
            "SendOptions/RawSendOptions mapping drifted; update both conversion directions"
        );
    }

    #[cfg(feature = "schemars")]
    #[test]
    fn schema_example_is_a_valid_send_request() {
        let value = super::example_send_request();

        serde_json::from_value::<super::SendRequest>(value)
            .expect("schema example should deserialize");
    }
}
