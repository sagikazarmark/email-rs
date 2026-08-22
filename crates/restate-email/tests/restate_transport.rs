#![cfg(feature = "client")]

use std::time::Duration;

use email_message::{Address, Body, Envelope, Message, OutboundMessage};
use email_transport::{
    CorrelationId, ErrorKind, IdempotencyKey, SendOptions, StructuredSendCapability, Transport,
    TransportOption, TransportOptions,
};
use restate_email::{RestateTransport, TransportKey};
use serde::Serialize;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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

#[tokio::test]
async fn send_posts_wire_options_and_uses_idempotency_for_ingress() {
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

    Mock::given(method("POST"))
        .and(path("/Email/send"))
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
                "correlation_id": "trace-42"
            }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "report": {
                "provider": "resend",
                "provider_message_id": "message-7",
                "accepted": ["to@example.com"]
            }
        })))
        .mount(&server)
        .await;

    let transport = RestateTransport::new(
        server.uri().parse().expect("server URL parses"),
        TransportKey::new("transactional").expect("key is valid"),
        reqwest::Client::new(),
    );
    assert!(!transport.capabilities().attachment_references);
    assert_eq!(
        transport.capabilities().structured_send,
        StructuredSendCapability::Unsupported
    );
    let transport = transport
        .with_structured_send(StructuredSendCapability::Supported)
        .with_attachment_references(true);

    let report = transport
        .send(&message, &options)
        .await
        .expect("ingress accepts request");

    assert_eq!(report.provider, "resend");
    assert_eq!(report.provider_message_id.as_deref(), Some("message-7"));
    assert_eq!(report.accepted[0].as_str(), "to@example.com");
    assert!(transport.capabilities().attachment_references);
}

#[tokio::test]
async fn invocation_error_is_terminal_and_preserves_worker_code() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/Email/send"))
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
    let transport = restate_transport(&server);

    let error = transport
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
        .and(path("/Email/send"))
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
    let transport = restate_transport(&server);

    let error = transport
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
        format!("http://{address}").parse().expect("URL parses"),
        TransportKey::new("transactional").expect("key is valid"),
        reqwest::Client::new(),
    );

    let error = transport
        .send(&message(), &SendOptions::default())
        .await
        .expect_err("connection should fail");

    assert_eq!(error.kind, ErrorKind::TransientNetwork);
    assert!(error.is_retryable());
}

fn restate_transport(server: &MockServer) -> RestateTransport {
    RestateTransport::new(
        server.uri().parse().expect("server URL parses"),
        TransportKey::new("transactional").expect("key is valid"),
        reqwest::Client::new(),
    )
}
