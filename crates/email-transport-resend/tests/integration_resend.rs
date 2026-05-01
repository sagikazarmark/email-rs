//! Recorded-fixture integration tests for the Resend adapter.

use email_message::{Address, Body, Message, OutboundMessage};
#[cfg(feature = "serde")]
use email_transport::TransportOptionRegistry;
use email_transport::{ErrorKind, IdempotencyKey, SendOptions, Transport};
#[cfg(feature = "serde")]
use email_transport_resend::ResendSendOptions;
use email_transport_resend::ResendTransport;
use url::Url;
use wiremock::matchers::{header, header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn sample_message() -> OutboundMessage {
    Message::builder(Body::text("Body"))
        .from_mailbox("sender@example.com".parse().expect("sender parses"))
        .to(vec![Address::Mailbox(
            "recipient@example.com".parse().expect("to parses"),
        )])
        .subject("Hello")
        .build_outbound()
        .expect("message validates")
}

fn transport_for(server: &MockServer) -> ResendTransport {
    let base_url = Url::parse(&format!("{}/", server.uri())).expect("base_url parses");
    ResendTransport::builder("test-key")
        .base_url(base_url)
        .build()
}

#[tokio::test]
async fn success_returns_provider_message_id() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/emails"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": "resend-abc-123"})),
        )
        .mount(&server)
        .await;

    let report = transport_for(&server)
        .send(&sample_message(), &SendOptions::default())
        .await
        .expect("send succeeds");

    assert_eq!(report.provider, "resend");
    assert_eq!(
        report.provider_message_id.as_deref(),
        Some("resend-abc-123")
    );
}

#[tokio::test]
async fn bad_request_400_maps_to_validation_not_retryable() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/emails"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "statusCode": 400,
            "name": "validation_error",
            "message": "invalid from address"
        })))
        .mount(&server)
        .await;

    let error = transport_for(&server)
        .send(&sample_message(), &SendOptions::default())
        .await
        .expect_err("send should fail");

    assert_eq!(error.kind, ErrorKind::Validation);
    assert!(!error.is_retryable());
    assert_eq!(error.http_status, Some(400));
    assert_eq!(
        error.provider_error_code.as_deref(),
        Some("validation_error")
    );
}

#[tokio::test]
async fn non_json_error_body_maps_to_provider_parse_failure() {
    // Defense against a Resend contract violation: HTTP 502 with an
    // HTML error page body. `resend-rs` error response parsing
    // fails, the adapter receives the SDK's parse error. Asserts the
    // fallback's behavior: body bytes preserved as the error message, no
    // `provider_error_code` (no JSON to extract `name` from).
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/emails"))
        .respond_with(
            ResponseTemplate::new(502)
                .set_body_string("<html><body><h1>502 Bad Gateway</h1></body></html>"),
        )
        .mount(&server)
        .await;

    let error = transport_for(&server)
        .send(&sample_message(), &SendOptions::default())
        .await
        .expect_err("non-JSON 502 body must surface as a provider parse failure");
    assert_eq!(error.kind, ErrorKind::TransientProvider);
    assert_eq!(error.http_status, None);
    assert!(error.is_retryable());
    assert!(error.provider_error_code.is_none());
    assert!(
        error.message.contains("502") || error.message.contains("Bad Gateway"),
        "fallback message should surface the body bytes: {}",
        error.message
    );
}

#[tokio::test]
async fn http_500_with_2xx_json_status_code_does_not_surface_as_success() {
    // NEW-D: a misbehaving Resend response of HTTP 500 with
    // `{"statusCode": 200}` must not cause `with_http_status(200)`,
    // implying the send succeeded. `resend-rs` exposes only the JSON status
    // here, so the adapter drops success-class statuses from the typed HTTP
    // slot and treats the error as transient provider misbehavior.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/emails"))
        .respond_with(ResponseTemplate::new(500).set_body_json(serde_json::json!({
            "statusCode": 200,
            "name": "internal_error",
            "message": "provider misbehavior"
        })))
        .mount(&server)
        .await;

    let error = transport_for(&server)
        .send(&sample_message(), &SendOptions::default())
        .await
        .expect_err("misbehaving 500 must still surface as a failure");
    assert_eq!(error.http_status, None);
    assert_eq!(error.kind, ErrorKind::TransientProvider);
    assert!(error.is_retryable());
}

#[tokio::test]
async fn rate_limit_429_maps_to_retryable_rate_limited() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/emails"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("ratelimit-reset", "30")
                .set_body_json(serde_json::json!({
                    "statusCode": 429,
                    "name": "rate_limit_exceeded",
                    "message": "too many requests"
                })),
        )
        .mount(&server)
        .await;

    let error = transport_for(&server)
        .send(&sample_message(), &SendOptions::default())
        .await
        .expect_err("send should fail");

    assert_eq!(error.kind, ErrorKind::RateLimited);
    assert!(error.is_retryable());
    assert_eq!(
        error.provider_error_code.as_deref(),
        Some("rate_limit_exceeded")
    );
    assert_eq!(error.retry_after, Some(std::time::Duration::from_secs(30)));
}

#[tokio::test]
async fn server_error_500_maps_to_retryable_transient_provider() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/emails"))
        .respond_with(ResponseTemplate::new(503).set_body_json(serde_json::json!({
            "statusCode": 503,
            "name": "internal_server_error",
            "message": "temporary"
        })))
        .mount(&server)
        .await;

    let error = transport_for(&server)
        .send(&sample_message(), &SendOptions::default())
        .await
        .expect_err("send should fail");

    assert_eq!(error.kind, ErrorKind::TransientProvider);
    assert!(error.is_retryable());
}

#[tokio::test]
async fn idempotency_key_is_forwarded_on_the_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/emails"))
        .and(header_exists("idempotency-key"))
        .and(header("idempotency-key", "idem-42"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": "ok-idem"})),
        )
        .mount(&server)
        .await;

    let options = SendOptions::new().with_idempotency_key(IdempotencyKey::new_unchecked("idem-42"));
    let report = transport_for(&server)
        .send(&sample_message(), &options)
        .await
        .expect("send succeeds with idempotency key");

    assert_eq!(report.provider_message_id.as_deref(), Some("ok-idem"));
}

#[tokio::test]
#[cfg(feature = "serde")]
async fn transport_options_wire_hydrates_tags() {
    use serde_json::json;
    use wiremock::matchers::body_partial_json;

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/emails"))
        .and(body_partial_json(json!({
            "tags": [{"name": "tenant", "value": "blue"}]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "ok-tags"})))
        .mount(&server)
        .await;

    let value = json!({
        "tags": [{"name": "tenant", "value": "blue"}]
    });
    let mut options = SendOptions::new();
    let mut registry = TransportOptionRegistry::new();
    registry
        .register::<ResendSendOptions>()
        .expect("register resend options");
    registry
        .hydrate_into("resend", &value, &mut options.transport_options)
        .expect("hydrate_into succeeds");
    let report = transport_for(&server)
        .send(&sample_message(), &options)
        .await
        .expect("send with transport_options tags succeeds");

    assert_eq!(report.provider_message_id.as_deref(), Some("ok-tags"));
}

#[tokio::test]
async fn attachment_encoding_round_trip_through_request_body() {
    use email_message::Attachment;
    use email_message::ContentType;
    use serde_json::json;
    use wiremock::matchers::body_partial_json;

    let server = MockServer::start().await;
    let payload: &[u8] = b"hello\xff\x00world";

    Mock::given(method("POST"))
        .and(path("/emails"))
        .and(body_partial_json(json!({
            "attachments": [{
                "filename": "report.bin",
                "content": payload,
                "contentType": "application/octet-stream",
            }]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "ok-att"})))
        .mount(&server)
        .await;

    let message = Message::builder(Body::text("body"))
        .from_mailbox("sender@example.com".parse().expect("from parses"))
        .to(vec![Address::Mailbox(
            "recipient@example.com".parse().expect("to parses"),
        )])
        .subject("with attachment")
        .add_attachment(
            Attachment::bytes(
                ContentType::try_from("application/octet-stream").expect("ct parses"),
                payload.to_vec(),
            )
            .with_filename("report.bin"),
        )
        .build_outbound()
        .expect("message validates");

    let report = transport_for(&server)
        .send(&message, &SendOptions::default())
        .await
        .expect("send with attachment succeeds");
    assert_eq!(report.provider_message_id.as_deref(), Some("ok-att"));
}

#[tokio::test]
async fn custom_header_is_forwarded_in_headers_object() {
    use email_message::Header;
    use serde_json::json;
    use wiremock::matchers::body_partial_json;

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/emails"))
        .and(body_partial_json(json!({
            "headers": {"X-Tenant": "blue"}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "ok-hdr"})))
        .mount(&server)
        .await;

    let message = Message::builder(Body::text("body"))
        .from_mailbox("sender@example.com".parse().expect("from parses"))
        .to(vec![Address::Mailbox(
            "recipient@example.com".parse().expect("to parses"),
        )])
        .subject("custom header")
        .add_header(Header::new("X-Tenant", "blue").expect("header validates"))
        .build_outbound()
        .expect("message validates");

    let report = transport_for(&server)
        .send(&message, &SendOptions::default())
        .await
        .expect("send with custom header succeeds");
    assert_eq!(report.provider_message_id.as_deref(), Some("ok-hdr"));
}

#[tokio::test]
async fn template_send_without_text_or_html_body_uses_template_branch() {
    use email_transport::TransportOptions;
    use email_transport_resend::{ResendSendOptions, ResendTemplate};
    use serde_json::json;
    use wiremock::matchers::body_partial_json;

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/emails"))
        .and(body_partial_json(json!({
            "template": {"id": "tmpl-welcome"}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "ok-tmpl"})))
        .mount(&server)
        .await;

    // Empty Html body, only the typed template carries content.
    let message = Message::builder(Body::Html(String::new()))
        .from_mailbox("sender@example.com".parse().expect("from parses"))
        .to(vec![Address::Mailbox(
            "recipient@example.com".parse().expect("to parses"),
        )])
        .subject("Welcome")
        .build_outbound()
        .expect("message validates");

    let mut transport_options = TransportOptions::default();
    transport_options
        .insert(ResendSendOptions::new().with_template(ResendTemplate::new("tmpl-welcome")));
    let options = SendOptions::new().with_transport_options(transport_options);

    let report = transport_for(&server)
        .send(&message, &options)
        .await
        .expect("template-only send succeeds");
    assert_eq!(report.provider_message_id.as_deref(), Some("ok-tmpl"));
}

#[tokio::test]
async fn unresolved_attachment_reference_returns_unsupported_feature_with_resolver_hint() {
    use email_message::ContentType;
    use email_message::{Attachment, AttachmentReference};

    let server = MockServer::start().await;
    // The mock server will not actually receive a request, the
    // adapter rejects on the unresolved Reference before any send.
    Mock::given(method("POST"))
        .and(path("/emails"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id":"unused"})))
        .expect(0)
        .mount(&server)
        .await;

    let message = Message::builder(Body::text("body"))
        .from_mailbox("sender@example.com".parse().expect("from parses"))
        .to(vec![Address::Mailbox(
            "recipient@example.com".parse().expect("to parses"),
        )])
        .subject("ref attachment")
        .add_attachment(Attachment::reference(
            ContentType::try_from("application/pdf").expect("ct parses"),
            AttachmentReference::new("s3://bucket/key"),
        ))
        .build_outbound()
        .expect("message validates");

    let error = transport_for(&server)
        .send(&message, &SendOptions::default())
        .await
        .expect_err("unresolved reference must be rejected");
    assert_eq!(error.kind, ErrorKind::UnsupportedFeature);
    // Wording check, the error must point the caller at the resolver
    // path so they know how to materialize the Reference.
    let msg = error.to_string();
    assert!(
        msg.contains("email_attachment::resolve_message_attachments"),
        "error should point at the resolver: {msg}"
    );
}

#[tokio::test]
async fn typed_options_round_trip_into_request_payload() {
    use email_transport::TransportOptions;
    use email_transport_resend::ResendSendOptions;
    use serde_json::json;
    use wiremock::matchers::body_partial_json;

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/emails"))
        .and(body_partial_json(json!({
            "tags": [
                {"name": "env", "value": "prod"},
                {"name": "tenant", "value": "blue"}
            ]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "ok-typed"})))
        .mount(&server)
        .await;

    let mut transport_options = TransportOptions::default();
    transport_options.insert(
        ResendSendOptions::new()
            .with_tag("env", "prod")
            .with_tag("tenant", "blue"),
    );

    let options = SendOptions::new().with_transport_options(transport_options);
    let report = transport_for(&server)
        .send(&sample_message(), &options)
        .await
        .expect("typed-options send succeeds");
    assert_eq!(report.provider_message_id.as_deref(), Some("ok-typed"));
}
