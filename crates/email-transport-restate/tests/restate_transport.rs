use std::time::Duration;

use email_message::{Address, Body, Envelope, Message, OutboundMessage};
use email_transport::{
    Capabilities, CorrelationId, ErrorKind, IdempotencyKey, SendOptions, StructuredSendCapability,
    Transport, TransportOption, TransportOptions,
};
use email_transport_restate::{InvocationMode, RestateSendOptions, RestateTransport, TransportKey};
use serde::Serialize;
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SEND_PATH: &str = "/restate/send/Email/send";
const CALL_PATH: &str = "/restate/call/Email/send";

#[derive(Debug, Serialize)]
struct ProviderOptions {
    campaign: String,
}

impl TransportOption for ProviderOptions {
    fn provider_key() -> &'static str {
        "provider"
    }
}

fn message() -> OutboundMessage {
    Message::builder(Body::text("hello"))
        .from_mailbox("from@example.com".parse().expect("sender parses"))
        .to(vec![Address::Mailbox(
            "to@example.com".parse().expect("recipient parses"),
        )])
        .subject("Queued message")
        .build()
        .and_then(OutboundMessage::new)
        .expect("message is outbound-valid")
}

fn transport_key() -> TransportKey {
    TransportKey::new("transactional").expect("key is valid")
}

fn ingress_url(server: &MockServer) -> reqwest::Url {
    server.uri().parse().expect("server URL parses")
}

fn queued_transport(server: &MockServer) -> RestateTransport {
    RestateTransport::new(transport_key(), ingress_url(server))
}

fn waiting_transport(server: &MockServer) -> RestateTransport {
    RestateTransport::builder(transport_key(), ingress_url(server))
        .invocation_mode(InvocationMode::Sent)
        .build()
}

fn restate_options(options: RestateSendOptions) -> SendOptions {
    let mut transport_options = TransportOptions::default();
    transport_options.insert(options);
    SendOptions::new().with_transport_options(transport_options)
}

fn accepted_body() -> serde_json::Value {
    serde_json::json!({"invocationId": "inv_1", "status": "Accepted"})
}

fn report_body() -> serde_json::Value {
    serde_json::json!({
        "report": {
            "provider": "resend",
            "provider_message_id": "message-7",
            "accepted": ["to@example.com"]
        }
    })
}

#[tokio::test]
async fn queued_send_posts_wire_options_and_reports_the_invocation() {
    let server = MockServer::start().await;
    let message = message();
    let envelope = Envelope::new(
        Some("bounce@example.com".parse().expect("sender parses")),
        vec!["other@example.com".parse().expect("recipient parses")],
    );
    let mut transport_options = TransportOptions::default();
    transport_options.insert(ProviderOptions {
        campaign: String::from("launch"),
    });
    let options = SendOptions::new()
        .with_envelope(envelope)
        .with_transport_options(transport_options)
        .with_timeout(Duration::new(3, 25))
        .with_idempotency_key(IdempotencyKey::new("enqueue-42").expect("valid key"))
        .with_correlation_id(CorrelationId::new("trace-42").expect("valid id"));

    // The idempotency key travels at both hops: as Restate's header and in the
    // queued options for the worker's provider (ADR 0004).
    Mock::given(method("POST"))
        .and(path(SEND_PATH))
        .and(header("idempotency-key", "enqueue-42"))
        .and(body_json(serde_json::json!({
            "transport": "transactional",
            "message": serde_json::to_value(&message).expect("message serializes"),
            "options": {
                "envelope": {
                    "mail_from": "bounce@example.com",
                    "rcpt_to": ["other@example.com"]
                },
                "transport_options": {
                    "provider": {"campaign": "launch"}
                },
                "timeout": {"secs": 3, "nanos": 25},
                "idempotency_key": "enqueue-42",
                "correlation_id": "trace-42"
            }
        })))
        .respond_with(ResponseTemplate::new(202).set_body_json(accepted_body()))
        .mount(&server)
        .await;

    let transport = queued_transport(&server);
    assert_eq!(transport.invocation_mode(), InvocationMode::Queued);

    let report = transport
        .send(&message, &options)
        .await
        .expect("ingress accepts request");

    assert_eq!(report.provider, RestateTransport::PROVIDER);
    assert_eq!(report.provider_message_id.as_deref(), Some("inv_1"));
    assert_eq!(RestateTransport::invocation_id(&report), Some("inv_1"));
    // The envelope override is not honored unless `custom_envelope` is asserted.
    assert_eq!(report.accepted.len(), 1);
    assert_eq!(report.accepted[0].as_str(), "to@example.com");
}

#[tokio::test]
async fn queued_report_honors_asserted_custom_envelope() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(SEND_PATH))
        .respond_with(ResponseTemplate::new(202).set_body_json(accepted_body()))
        .mount(&server)
        .await;
    let transport = RestateTransport::builder(transport_key(), ingress_url(&server))
        .capabilities(Capabilities::new().with_custom_envelope(true))
        .build();
    let options = SendOptions::new().with_envelope(Envelope::new(
        None,
        vec!["other@example.com".parse().expect("recipient parses")],
    ));

    let report = transport
        .send(&message(), &options)
        .await
        .expect("ingress accepts request");

    assert_eq!(report.accepted.len(), 1);
    assert_eq!(report.accepted[0].as_str(), "other@example.com");
}

#[tokio::test]
async fn waiting_send_posts_to_the_call_path_and_returns_the_worker_report() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(CALL_PATH))
        .and(header("idempotency-key", "enqueue-42"))
        .respond_with(ResponseTemplate::new(200).set_body_json(report_body()))
        .mount(&server)
        .await;
    let options = SendOptions::new()
        .with_idempotency_key(IdempotencyKey::new("enqueue-42").expect("valid key"));

    let report = waiting_transport(&server)
        .send(&message(), &options)
        .await
        .expect("ingress accepts request");

    assert_eq!(report.provider, "resend");
    assert_eq!(report.provider_message_id.as_deref(), Some("message-7"));
    assert_eq!(report.accepted[0].as_str(), "to@example.com");
    assert_eq!(RestateTransport::invocation_id(&report), None);
}

#[tokio::test]
async fn bearer_token_is_sent_with_queued_and_waiting_requests() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(SEND_PATH))
        .and(header("authorization", "Bearer ingress-api-key"))
        .respond_with(ResponseTemplate::new(202).set_body_json(accepted_body()))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(CALL_PATH))
        .and(header("authorization", "Bearer ingress-api-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(report_body()))
        .expect(1)
        .mount(&server)
        .await;
    let builder = RestateTransport::builder(transport_key(), ingress_url(&server))
        .bearer_token("ingress-api-key");

    builder
        .clone()
        .build()
        .send(&message(), &SendOptions::default())
        .await
        .expect("authenticated queued send succeeds");
    builder
        .invocation_mode(InvocationMode::Sent)
        .build()
        .send(&message(), &SendOptions::default())
        .await
        .expect("authenticated waiting send succeeds");
}

#[tokio::test]
async fn requests_without_bearer_token_omit_the_authorization_header() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(SEND_PATH))
        .respond_with(ResponseTemplate::new(202).set_body_json(accepted_body()))
        .mount(&server)
        .await;

    queued_transport(&server)
        .send(&message(), &SendOptions::default())
        .await
        .expect("unauthenticated send succeeds");

    let requests = server
        .received_requests()
        .await
        .expect("requests are recorded");
    assert!(
        requests
            .iter()
            .all(|request| !request.headers.contains_key("authorization")),
        "no request should carry an Authorization header"
    );
}

#[tokio::test]
async fn per_send_option_overrides_queued_default_with_sent() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(CALL_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(report_body()))
        .mount(&server)
        .await;
    let options =
        restate_options(RestateSendOptions::new().with_invocation_mode(InvocationMode::Sent));

    let report = queued_transport(&server)
        .send(&message(), &options)
        .await
        .expect("ingress accepts request");

    assert_eq!(report.provider, "resend");
}

#[tokio::test]
async fn per_send_option_overrides_sent_default_with_queued() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(SEND_PATH))
        .respond_with(ResponseTemplate::new(202).set_body_json(accepted_body()))
        .mount(&server)
        .await;
    let options =
        restate_options(RestateSendOptions::new().with_invocation_mode(InvocationMode::Queued));

    let report = waiting_transport(&server)
        .send(&message(), &options)
        .await
        .expect("ingress accepts request");

    assert_eq!(report.provider, RestateTransport::PROVIDER);
}

#[tokio::test]
async fn queued_delay_is_sent_as_rounded_up_milliseconds() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(SEND_PATH))
        .and(query_param("delay", "1001ms"))
        .respond_with(ResponseTemplate::new(202).set_body_json(accepted_body()))
        .mount(&server)
        .await;
    let options = restate_options(RestateSendOptions::new().with_delay(Duration::new(1, 1)));

    queued_transport(&server)
        .send(&message(), &options)
        .await
        .expect("ingress accepts delayed request");
}

#[tokio::test]
async fn delay_with_sent_mode_fails_validation_before_any_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(report_body()))
        .expect(0)
        .mount(&server)
        .await;
    let options = restate_options(
        RestateSendOptions::new()
            .with_invocation_mode(InvocationMode::Sent)
            .with_delay(Duration::from_secs(1)),
    );

    let error = queued_transport(&server)
        .send(&message(), &options)
        .await
        .expect_err("delay requires queued mode");

    assert_eq!(error.kind, ErrorKind::Validation);
    assert!(error.is_terminal());
}

#[tokio::test]
async fn queued_send_tolerates_replayed_status_and_extra_fields() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(SEND_PATH))
        .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({
            "invocationId": "inv_replay",
            "status": "PreviouslyAccepted",
            "executionTime": "2026-08-24T00:00:00Z"
        })))
        .mount(&server)
        .await;

    let report = queued_transport(&server)
        .send(&message(), &SendOptions::default())
        .await
        .expect("replayed acceptance still succeeds");

    assert_eq!(report.provider_message_id.as_deref(), Some("inv_replay"));
}

#[tokio::test]
async fn queued_send_without_invocation_id_is_transient() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(SEND_PATH))
        .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({
            "status": "Accepted"
        })))
        .mount(&server)
        .await;

    let error = queued_transport(&server)
        .send(&message(), &SendOptions::default())
        .await
        .expect_err("missing invocation id is an invalid response");

    assert_eq!(error.kind, ErrorKind::TransientProvider);
}

#[tokio::test]
async fn queued_ingress_rejection_is_validation() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(SEND_PATH))
        .respond_with(
            ResponseTemplate::new(400)
                .insert_header("x-restate-error-source", "ingress")
                .set_body_json(serde_json::json!({
                    "code": 400,
                    "message": "invalid delay",
                    "source": "ingress"
                })),
        )
        .mount(&server)
        .await;

    let error = queued_transport(&server)
        .send(&message(), &SendOptions::default())
        .await
        .expect_err("ingress rejection should propagate");

    assert_eq!(error.kind, ErrorKind::Validation);
    assert_eq!(error.http_status, Some(400));
    assert_eq!(error.message, "invalid delay");
}

#[tokio::test]
async fn unauthenticated_ingress_rejection_is_authentication() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(SEND_PATH))
        .respond_with(
            ResponseTemplate::new(401)
                .insert_header("x-restate-error-source", "ingress")
                .set_body_json(serde_json::json!({
                    "code": 401,
                    "message": "missing or invalid API key",
                    "source": "ingress"
                })),
        )
        .mount(&server)
        .await;

    let error = queued_transport(&server)
        .send(&message(), &SendOptions::default())
        .await
        .expect_err("unauthenticated request should be rejected");

    assert_eq!(error.kind, ErrorKind::Authentication);
    assert!(error.is_terminal());
    assert_eq!(error.http_status, Some(401));
}

#[tokio::test]
async fn send_replaces_ingress_query_and_fragment_with_service_path() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(SEND_PATH))
        .respond_with(ResponseTemplate::new(202).set_body_json(accepted_body()))
        .mount(&server)
        .await;
    let ingress_url = format!("{}?tenant=blue#configuration", server.uri())
        .parse()
        .expect("server URL parses");
    let transport = RestateTransport::new(transport_key(), ingress_url);

    let report = transport
        .send(&message(), &SendOptions::default())
        .await
        .expect("ingress accepts request at the service path");

    assert_eq!(report.provider, RestateTransport::PROVIDER);
}

#[tokio::test]
async fn invocation_error_is_terminal_and_preserves_worker_code() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(CALL_PATH))
        .respond_with(
            ResponseTemplate::new(500)
                .insert_header("x-restate-error-source", "invocation")
                .set_body_json(serde_json::json!({
                    "code": 500,
                    "message": "worker failed terminally",
                    "source": "invocation"
                })),
        )
        .mount(&server)
        .await;

    let error = waiting_transport(&server)
        .send(&message(), &SendOptions::default())
        .await
        .expect_err("worker error should propagate");

    assert_eq!(error.kind, ErrorKind::Internal);
    assert!(error.is_terminal());
    assert_eq!(error.http_status, Some(500));
    assert_eq!(error.provider_error_code.as_deref(), Some("500"));
    assert_eq!(error.message, "worker failed terminally");
}

#[tokio::test]
async fn ingress_service_failure_is_retryable() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(SEND_PATH))
        .respond_with(
            ResponseTemplate::new(501)
                .insert_header("x-restate-error-source", "ingress")
                .set_body_json(serde_json::json!({
                    "code": 501,
                    "message": "routing unavailable",
                    "source": "ingress"
                })),
        )
        .mount(&server)
        .await;

    let error = queued_transport(&server)
        .send(&message(), &SendOptions::default())
        .await
        .expect_err("ingress error should propagate");

    assert_eq!(error.kind, ErrorKind::TransientProvider);
    assert!(error.is_retryable());
    assert_eq!(error.http_status, Some(501));
}

#[tokio::test]
async fn connection_failure_is_retryable() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("port binds");
    let address = listener.local_addr().expect("address available");
    drop(listener);
    let transport = RestateTransport::new(
        transport_key(),
        format!("http://{address}").parse().expect("URL parses"),
    );

    let error = transport
        .send(&message(), &SendOptions::default())
        .await
        .expect_err("connection should fail");

    assert_eq!(error.kind, ErrorKind::TransientNetwork);
    assert!(error.is_retryable());
}

#[test]
fn builder_setters_adjust_capabilities() {
    let url: reqwest::Url = "http://127.0.0.1:8080".parse().expect("URL parses");
    let transport = RestateTransport::builder(transport_key(), url.clone())
        .client(reqwest::Client::new())
        .structured_send(StructuredSendCapability::RequiresTransportOptions)
        .attachment_references(true)
        .build();

    assert_eq!(
        transport.capabilities().structured_send,
        StructuredSendCapability::RequiresTransportOptions
    );
    assert!(transport.capabilities().attachment_references);
    assert!(transport.capabilities().idempotency_key);
    assert_eq!(transport.transport_key().as_str(), "transactional");
    assert_eq!(transport.ingress_url(), &url);
}
