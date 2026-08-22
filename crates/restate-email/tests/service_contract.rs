#![cfg(feature = "service")]

mod support;

use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use bytes::Bytes;
use email_kit::transport::transport_option_registry;
use email_message::{Address, Attachment, AttachmentBody, Body, Mailbox, Message, OutboundMessage};
use email_message::{ContentType, Envelope};
use email_transport::{SendReport, Transport, TransportError};
use restate_email::{
    CorrelationId, IdempotencyKey, SendOptions, SendRequest, SendRequestSeed, ServiceImpl,
    StaticTransportRegistry, TransportKey, TransportOption, TransportOptionRegistry,
};
use restate_sdk_shared_core::Version;
use serde::de::DeserializeSeed as _;
use serde::{Deserialize, Serialize};
use support::{
    collect_response_body, decode_protocol_failure, decode_protocol_run_command,
    invoke_protocol_sdk_endpoint, invoke_protocol_sdk_endpoint_with_payload,
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct RecordedSend {
    message: Message,
    envelope: Option<Envelope>,
    timeout: Option<Duration>,
    idempotency_key: Option<IdempotencyKey>,
    correlation_id: Option<CorrelationId>,
    custom_label: Option<String>,
}

#[derive(Clone)]
struct RecordingTransport {
    sends: Arc<Mutex<Vec<RecordedSend>>>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct CustomSendOption {
    label: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct PathTestOptions {
    tags: Vec<String>,
}

impl TransportOption for CustomSendOption {
    fn provider_key() -> &'static str {
        "custom"
    }
}

impl TransportOption for PathTestOptions {
    fn provider_key() -> &'static str {
        "path-test"
    }
}

impl RecordingTransport {
    fn take_sends(&self) -> Vec<RecordedSend> {
        std::mem::take(&mut *self.sends.lock().expect("lock should not be poisoned"))
    }
}

impl Default for RecordingTransport {
    fn default() -> Self {
        Self {
            sends: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl Transport for RecordingTransport {
    fn send<'a>(
        &'a self,
        message: &'a email_message::OutboundMessage,
        options: &'a SendOptions,
    ) -> impl core::future::Future<Output = Result<SendReport, TransportError>> + Send + 'a {
        let message = message.as_message().clone();
        let envelope = options.envelope.clone();
        let timeout = options.timeout;
        let idempotency_key = options.idempotency_key.clone();
        let correlation_id = options.correlation_id.clone();
        let custom_label = options
            .transport_options
            .get::<CustomSendOption>()
            .map(|option| option.label.clone());
        let sends = &self.sends;

        Box::pin(async move {
            sends
                .lock()
                .expect("lock should not be poisoned")
                .push(RecordedSend {
                    message,
                    envelope,
                    timeout,
                    idempotency_key,
                    correlation_id,
                    custom_label,
                });
            Ok(SendReport::new("recording")
                .with_provider_message_id("provider-id")
                .with_accepted(vec!["to@example.com".parse().expect("email parses")]))
        })
    }
}

fn mailbox(input: &str) -> Mailbox {
    input.parse::<Mailbox>().expect("mailbox should parse")
}

fn message_with_attachment(bytes: &[u8]) -> OutboundMessage {
    let message = Message::builder(Body::text("hello"))
        .from_mailbox(mailbox("from@example.com"))
        .add_to(Address::Mailbox(mailbox("to@example.com")))
        .add_attachment(
            Attachment::bytes(
                ContentType::try_from("application/pdf").expect("content type should parse"),
                bytes.to_vec(),
            )
            .with_filename("report.pdf"),
        )
        .build()
        .expect("message should validate");
    OutboundMessage::new(message).expect("message should be outbound-valid")
}

fn request_with_attachment() -> SendRequest {
    SendRequest {
        transport: TransportKey::new_unchecked("transactional"),
        message: message_with_attachment(b"attached"),
        options: SendOptions::default(),
    }
}

fn deserialize_send_options(value: serde_json::Value) -> SendOptions {
    transport_option_registry()
        .send_options_seed()
        .ignore_unknown_transport_options()
        .deserialize(value)
        .expect("send options should deserialize")
}

fn deserialize_send_request(
    value: serde_json::Value,
    registry: &TransportOptionRegistry,
) -> SendRequest {
    SendRequestSeed::new(registry)
        .deserialize(value)
        .expect("send email request should deserialize")
}

#[test]
fn send_options_seed_decodes_shared_options() {
    let options = deserialize_send_options(serde_json::json!({
        "envelope": {
            "mail_from": "bounce@example.com",
            "rcpt_to": ["to@example.com"]
        },
        "timeout": {"secs": 1, "nanos": 500_000_000},
        "idempotency_key": "idempotency-key",
        "correlation_id": "corr-123"
    }));

    assert_eq!(options.timeout, Some(Duration::from_millis(1_500)));
    assert_eq!(
        options.idempotency_key.as_ref().map(IdempotencyKey::as_str),
        Some("idempotency-key")
    );
    assert_eq!(
        options.correlation_id.as_ref().map(CorrelationId::as_str),
        Some("corr-123")
    );
    assert_eq!(
        options
            .envelope
            .as_ref()
            .and_then(Envelope::mail_from)
            .map(email_message::EmailAddress::as_str),
        Some("bounce@example.com")
    );
}

#[test]
fn send_options_roundtrip_through_registry() {
    let options = SendOptions::new()
        .with_timeout(Duration::from_secs(5))
        .with_correlation_id(CorrelationId::new_unchecked("corr-456"));

    let json = serde_json::to_string(&options).expect("serialize");
    let back = deserialize_send_options(serde_json::from_str(&json).expect("json value"));
    assert_eq!(back.timeout, Some(Duration::from_secs(5)));
    assert_eq!(
        back.correlation_id.as_ref().map(CorrelationId::as_str),
        Some("corr-456")
    );
    assert!(!json.contains("timeout_ms"));
}

#[test]
#[cfg(feature = "schemars")]
fn send_request_schema_uses_send_options_shape() {
    let schema = schemars::schema_for!(SendRequest);
    let value = schema.as_value();

    assert!(value.pointer("/properties/options").is_some());
    if let Some(required) = value.get("required").and_then(|value| value.as_array()) {
        assert!(
            !required
                .iter()
                .any(|value| value.as_str() == Some("options")),
            "SendRequest options default to empty and should be optional: {value}"
        );
    }
    assert!(
        value.to_string().contains("transport_options"),
        "SendRequest schema should expose SendOptions transport_options"
    );
}

#[tokio::test]
async fn send_dispatches_message_and_options() {
    let mut options = SendOptions::default();
    options.envelope = Some(Envelope::new(
        Some("bounce@example.com".parse().expect("email should parse")),
        vec!["to@example.com".parse().expect("email should parse")],
    ));
    options.timeout = Some(Duration::from_secs(2));
    options.idempotency_key = Some(IdempotencyKey::new_unchecked("send-123"));
    options.correlation_id = Some(CorrelationId::new_unchecked("request-correlation-1"));

    let request = SendRequest {
        transport: TransportKey::new_unchecked("transactional"),
        message: message_with_attachment(b"attached-pdf"),
        options,
    };
    let transport = RecordingTransport::default();
    let mut registry = StaticTransportRegistry::new();
    registry.insert("transactional", transport.clone());
    let service = ServiceImpl::new(registry);

    let response = service
        .send_request(&request)
        .await
        .expect("request should send");

    assert_eq!(response.report.provider, "recording");

    let sends = transport.take_sends();
    assert_eq!(sends.len(), 1);
    assert!(matches!(
        sends[0].message.attachments()[0].body(),
        AttachmentBody::Bytes(bytes) if bytes == b"attached-pdf"
    ));
    assert_eq!(sends[0].timeout, Some(Duration::from_secs(2)));
    assert_eq!(
        sends[0]
            .idempotency_key
            .as_ref()
            .map(IdempotencyKey::as_str),
        Some("send-123")
    );
    assert_eq!(
        sends[0].correlation_id.as_ref().map(CorrelationId::as_str),
        Some("request-correlation-1")
    );
    assert_eq!(
        sends[0]
            .envelope
            .as_ref()
            .and_then(Envelope::mail_from)
            .map(email_message::EmailAddress::as_str),
        Some("bounce@example.com")
    );
}

#[tokio::test]
async fn raw_protocol_endpoint_emits_run_command_for_send_side_effect() {
    let transport = RecordingTransport::default();
    let mut registry = StaticTransportRegistry::new();
    registry.insert("transactional", transport);
    let service = ServiceImpl::new(registry);

    let response = invoke_protocol_sdk_endpoint(&service, &request_with_attachment());
    let (status, headers, body) = collect_response_body(response).await;
    let run = decode_protocol_run_command(body);

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(
        headers
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some(Version::maximum_supported_version().content_type())
    );
    assert_eq!(run.name, "send_email");
}

#[tokio::test]
async fn malformed_options_return_terminal_400_with_json_path() {
    let transport = RecordingTransport::default();
    let mut registry = StaticTransportRegistry::new();
    registry.insert("transactional", transport);
    let mut option_registry = TransportOptionRegistry::new();
    option_registry
        .register::<PathTestOptions>()
        .expect("path test option should register");
    let service = ServiceImpl::new(registry).with_transport_options(option_registry);
    let mut payload =
        serde_json::to_value(request_with_attachment()).expect("request should serialize");
    payload["options"]["transport_options"] = serde_json::json!({
        "path-test": {"tags": "not-a-list"}
    });

    let response = invoke_protocol_sdk_endpoint_with_payload(
        &service,
        Bytes::from(serde_json::to_vec(&payload).expect("payload should serialize")),
    );
    let (status, _, body) = collect_response_body(response).await;
    let error = decode_protocol_failure(body);

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(error.code, 400);
    assert!(
        error
            .message
            .contains("options.transport_options.path-test.tags"),
        "decode error should include the failing JSON path: {}",
        error.message
    );
}

#[cfg(feature = "resend")]
#[test]
fn transport_options_roundtrip_into_typed_slots() {
    use email_kit::transport::resend::ResendSendOptions;

    let mut options = SendOptions::new();
    options
        .transport_options
        .insert(ResendSendOptions::new().with_tag("tag", "welcome"));

    let json = serde_json::to_value(&options).expect("serialize");
    assert_eq!(
        json["transport_options"]["resend"]["tags"][0]["name"],
        "tag"
    );

    let options = deserialize_send_options(json);
    let transport_options = &options.transport_options;

    let resend_options = transport_options
        .get::<ResendSendOptions>()
        .expect("resend options hydrated");
    assert_eq!(resend_options.tags.len(), 1);
    assert_eq!(resend_options.tags[0].name, "tag");
    assert_eq!(resend_options.tags[0].value, "welcome");
}

#[tokio::test]
async fn unknown_transport_option_keys_are_ignored() {
    let request = SendRequest {
        transport: TransportKey::new_unchecked("transactional"),
        message: message_with_attachment(b"attached-pdf"),
        options: SendOptions::default(),
    };
    let mut value = serde_json::to_value(request).expect("request should serialize");
    value["options"]["transport_options"] = serde_json::json!({
        "custom": {"label": "runtime"},
        "acme-unknown": {"foo": "bar"}
    });
    let mut custom_registry = TransportOptionRegistry::new();
    custom_registry
        .register::<CustomSendOption>()
        .expect("custom provider key should be unique");
    let request = deserialize_send_request(value, &custom_registry);
    let transport = RecordingTransport::default();
    let mut registry = StaticTransportRegistry::new();
    registry.insert("transactional", transport.clone());
    let service = ServiceImpl::new(registry).with_transport_options(custom_registry);

    service
        .send_request(&request)
        .await
        .expect("unknown provider options should not prevent delivery");

    let sends = transport.take_sends();
    assert_eq!(sends.len(), 1);
    assert_eq!(sends[0].custom_label.as_deref(), Some("runtime"));
}

#[cfg(feature = "resend")]
#[test]
fn malformed_transport_option_for_known_key_returns_error() {
    // Resend's `tags` field is a sequence; a string value here is a shape
    // mismatch that registry deserialization rejects.
    let result = transport_option_registry()
        .send_options_seed()
        .ignore_unknown_transport_options()
        .deserialize(serde_json::json!({
            "transport_options": {"resend": {"tags": "not-a-list"}}
        }));

    assert!(
        result.is_err(),
        "malformed metadata should surface a deserialization error"
    );
}

#[test]
fn send_request_serde_roundtrip_with_send_options() {
    let mut options = SendOptions::default();
    options.timeout = Some(Duration::from_millis(2_500));
    options.correlation_id = Some(CorrelationId::new_unchecked("corr-request"));

    let request = SendRequest {
        transport: TransportKey::new_unchecked("transactional"),
        message: OutboundMessage::new(
            Message::builder(Body::text("hi"))
                .from_mailbox(mailbox("a@example.com"))
                .add_to(Address::Mailbox(mailbox("b@example.com")))
                .build()
                .expect("message validates"),
        )
        .expect("message should be outbound-valid"),
        options,
    };

    let json = serde_json::to_value(&request).expect("serialize");
    assert_eq!(json["transport"], "transactional");

    let back = deserialize_send_request(json, &TransportOptionRegistry::new());

    assert_eq!(back.options.timeout, Some(Duration::from_millis(2_500)));
    assert_eq!(
        back.options
            .correlation_id
            .as_ref()
            .map(CorrelationId::as_str),
        Some("corr-request")
    );
}

#[test]
fn send_request_deserialization_rejects_invalid_message() {
    let request = SendRequest {
        transport: TransportKey::new_unchecked("transactional"),
        message: message_with_attachment(b"attached-pdf"),
        options: SendOptions::default(),
    };
    let mut value = serde_json::to_value(&request).expect("request should serialize");
    value["message"]
        .as_object_mut()
        .expect("message should serialize as object")
        .remove("from");

    let result = SendRequestSeed::new(&TransportOptionRegistry::new()).deserialize(value);

    assert!(
        result.is_err(),
        "SendRequest should reject messages that are invalid for outbound sending"
    );
}

#[tokio::test]
async fn send_request_deserialization_uses_service_transport_option_registry() {
    let mut custom_registry = TransportOptionRegistry::new();
    custom_registry
        .register::<CustomSendOption>()
        .expect("custom provider key should be unique");
    let transport = RecordingTransport::default();
    let mut registry = StaticTransportRegistry::new();
    registry.insert("transactional", transport.clone());
    let mut send_options = SendOptions::new();
    send_options.transport_options.insert(CustomSendOption {
        label: String::from("runtime"),
    });
    let request = SendRequest {
        transport: TransportKey::new_unchecked("transactional"),
        message: OutboundMessage::new(
            Message::builder(Body::text("hi"))
                .from_mailbox(mailbox("a@example.com"))
                .add_to(Address::Mailbox(mailbox("b@example.com")))
                .build()
                .expect("message validates"),
        )
        .expect("message should be outbound-valid"),
        options: send_options,
    };
    let json = serde_json::to_value(&request).expect("serialize");

    let back = deserialize_send_request(json, &custom_registry);
    let service = ServiceImpl::new(registry).with_transport_options(custom_registry);

    service
        .send_request(&back)
        .await
        .expect("service should send hydrated options");

    let sends = transport.take_sends();
    assert_eq!(sends[0].custom_label.as_deref(), Some("runtime"));
}

#[cfg(feature = "resend")]
#[test]
fn send_request_serde_roundtrip_with_transport_options() {
    use email_kit::transport::resend::ResendSendOptions;

    let mut send_options = SendOptions::new();
    send_options
        .transport_options
        .insert(ResendSendOptions::new().with_tag("t", "v"));
    let request = SendRequest {
        transport: TransportKey::new_unchecked("transactional"),
        message: OutboundMessage::new(
            Message::builder(Body::text("hi"))
                .from_mailbox(mailbox("a@example.com"))
                .add_to(Address::Mailbox(mailbox("b@example.com")))
                .build()
                .expect("message validates"),
        )
        .expect("message should be outbound-valid"),
        options: send_options,
    };

    let json = serde_json::to_string(&request).expect("serialize");
    let back = deserialize_send_request(
        serde_json::from_str(&json).expect("json value"),
        &transport_option_registry(),
    );

    let send_options = back.options;
    let resend_options = send_options
        .transport_options
        .get::<ResendSendOptions>()
        .expect("resend options hydrated");
    assert_eq!(
        resend_options
            .tags
            .first()
            .map(|tag| (tag.name.as_str(), tag.value.as_str())),
        Some(("t", "v"))
    );
}
