//! SDK-independent request and response contract for the email service.

use email_message::OutboundMessage;
use email_transport::{SendOptions, SendReport, TransportOptionRegistry, string_newtype};
use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, Visitor};
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
/// Provider-specific [`SendOptions::transport_options`] are a best-effort union.
/// Callers may include options for every provider they support; the worker
/// hydrates registered provider slices and ignores unrecognized providers.
/// Switching the selected transport may therefore drop provider-specific
/// behavior.
///
/// Deserialization requires a [`TransportOptionRegistry`] and is intentionally
/// available only through [`SendRequestSeed`].
#[derive(Debug, Serialize)]
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
    pub options: SendOptions,
}

/// Registry-driven deserializer for [`SendRequest`].
///
/// Unknown provider option keys are ignored so a queued payload can carry
/// options for transports that are not installed in a particular worker.
pub struct SendRequestSeed<'a> {
    registry: &'a TransportOptionRegistry,
}

impl<'a> SendRequestSeed<'a> {
    /// Create a request seed backed by `registry`.
    #[must_use]
    pub const fn new(registry: &'a TransportOptionRegistry) -> Self {
        Self { registry }
    }
}

impl<'de> DeserializeSeed<'de> for SendRequestSeed<'_> {
    type Value = SendRequest;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(SendRequestVisitor {
            registry: self.registry,
        })
    }
}

struct SendRequestVisitor<'a> {
    registry: &'a TransportOptionRegistry,
}

#[derive(Deserialize)]
#[serde(field_identifier, rename_all = "snake_case")]
enum SendRequestField {
    Transport,
    Message,
    Options,
    #[serde(other)]
    Other,
}

impl<'de> Visitor<'de> for SendRequestVisitor<'_> {
    type Value = SendRequest;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("an email send request")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut transport = None;
        let mut message = None;
        let mut options = None;

        while let Some(field) = map.next_key::<SendRequestField>()? {
            match field {
                SendRequestField::Transport => {
                    if transport.is_some() {
                        return Err(serde::de::Error::duplicate_field("transport"));
                    }
                    transport = Some(map.next_value()?);
                }
                SendRequestField::Message => {
                    if message.is_some() {
                        return Err(serde::de::Error::duplicate_field("message"));
                    }
                    message = Some(map.next_value()?);
                }
                SendRequestField::Options => {
                    if options.is_some() {
                        return Err(serde::de::Error::duplicate_field("options"));
                    }
                    options = Some(
                        map.next_value_seed(
                            self.registry
                                .send_options_seed()
                                .ignore_unknown_transport_options(),
                        )?,
                    );
                }
                SendRequestField::Other => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }

        Ok(SendRequest {
            transport: transport.ok_or_else(|| serde::de::Error::missing_field("transport"))?,
            message: message.ok_or_else(|| serde::de::Error::missing_field("message"))?,
            options: options.unwrap_or_default(),
        })
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

#[cfg(all(test, feature = "schemars"))]
mod tests {
    use email_transport::TransportOptionRegistry;
    use serde::de::DeserializeSeed as _;

    #[cfg(feature = "schemars")]
    #[test]
    fn schema_example_is_a_valid_send_request() {
        let value = super::example_send_request();

        super::SendRequestSeed::new(&TransportOptionRegistry::new())
            .deserialize(value)
            .expect("schema example should deserialize");
    }
}
