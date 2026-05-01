use std::time::Duration;

use email_message::{Address, Body, EmailAddress, Envelope, Message, OutboundMessage};
use email_transport::{CorrelationId, IdempotencyKey, RawTransport, SendOptions, Transport};
use email_transport_test::{CapturedPayload, FileTransport, MemoryTransport};

fn sample_message() -> OutboundMessage {
    Message::builder(Body::text("Hello"))
        .from_mailbox("sender@example.com".parse().expect("from parses"))
        .to(vec![Address::Mailbox(
            "recipient@example.com".parse().expect("to parses"),
        )])
        .subject("Hi")
        .build_outbound()
        .expect("message should validate")
}

#[tokio::test]
async fn memory_transport_captures_structured_send() {
    let transport = MemoryTransport::new().with_provider_message_id("msg-1");

    let report = transport
        .send(&sample_message(), &SendOptions::default())
        .await
        .expect("send should succeed");

    assert_eq!(report.provider, "memory");
    assert_eq!(report.provider_message_id.as_deref(), Some("msg-1"));
    let accepted_strs: Vec<&str> = report
        .accepted
        .iter()
        .map(email_message::EmailAddress::as_str)
        .collect();
    assert_eq!(accepted_strs, vec!["recipient@example.com"]);

    let captured = transport.captured();
    assert_eq!(captured.len(), 1);
    let CapturedPayload::Structured { envelope, message } = &captured[0].payload else {
        panic!("expected structured capture");
    };
    assert!(envelope.is_none());
    assert_eq!(message.subject(), Some("Hi"));
}

#[tokio::test]
async fn memory_transport_captures_send_with_metadata() {
    let transport = MemoryTransport::new();

    let envelope = Envelope::new(
        Some(
            "bounce@example.com"
                .parse::<EmailAddress>()
                .expect("from parses"),
        ),
        vec![
            "override@example.com"
                .parse::<EmailAddress>()
                .expect("rcpt parses"),
        ],
    );
    let options = SendOptions::new()
        .with_envelope(envelope)
        .with_timeout(Duration::from_millis(1_500))
        .with_idempotency_key(IdempotencyKey::new_unchecked("key-1"))
        .with_correlation_id(CorrelationId::new_unchecked("corr-1"));

    let report = transport
        .send(&sample_message(), &options)
        .await
        .expect("send should succeed");
    let accepted: Vec<&str> = report
        .accepted
        .iter()
        .map(email_message::EmailAddress::as_str)
        .collect();
    assert_eq!(accepted, vec!["override@example.com"]);

    let captured = transport.captured();
    assert_eq!(captured.len(), 1);
    let recorded = &captured[0];
    assert_eq!(recorded.timeout, Some(Duration::from_millis(1_500)));
    assert_eq!(
        recorded
            .idempotency_key
            .as_ref()
            .map(IdempotencyKey::as_str),
        Some("key-1")
    );
    assert_eq!(
        recorded.correlation_id.as_ref().map(CorrelationId::as_str),
        Some("corr-1")
    );
    let CapturedPayload::Structured { envelope, .. } = &recorded.payload else {
        panic!("expected structured capture");
    };
    assert!(envelope.is_some());
}

#[tokio::test]
async fn memory_transport_clone_shares_captured_log() {
    let transport = MemoryTransport::new();
    let clone = transport.clone();

    transport
        .send(&sample_message(), &SendOptions::default())
        .await
        .expect("send");
    assert_eq!(clone.len(), 1);

    clone.clear();
    assert!(transport.is_empty());
}

#[tokio::test]
async fn memory_transport_raw_send_captures_bytes() {
    let transport = MemoryTransport::new();
    let envelope = Envelope::new(
        Some(
            "from@example.com"
                .parse::<EmailAddress>()
                .expect("from parses"),
        ),
        vec!["to@example.com".parse::<EmailAddress>().expect("to parses")],
    );
    let rfc822 = b"Subject: Raw\r\n\r\nBody\r\n";

    transport
        .send_raw(&envelope, rfc822, &SendOptions::new())
        .await
        .expect("raw send should succeed");

    let captured = transport.captured();
    let CapturedPayload::Raw {
        envelope: recorded_envelope,
        rfc822: recorded_rfc822,
    } = &captured[0].payload
    else {
        panic!("expected raw capture");
    };
    assert_eq!(recorded_envelope, &envelope);
    assert_eq!(recorded_rfc822.as_slice(), rfc822);
}

#[tokio::test]
async fn file_transport_writes_structured_send_to_disk() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let transport = FileTransport::new(tmp.path()).expect("transport constructs");

    let report = transport
        .send(&sample_message(), &SendOptions::default())
        .await
        .expect("send should succeed");

    assert_eq!(report.provider, "file");
    let path = report
        .provider_message_id
        .expect("file path should be set as provider message id");
    let written = std::fs::read(&path).expect("file should be readable");
    assert!(!written.is_empty());
    assert!(written.starts_with(b"From: "));
}

#[tokio::test]
async fn file_transport_raw_send_preserves_bytes_exactly() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let transport = FileTransport::new(tmp.path()).expect("transport constructs");
    let envelope = Envelope::new(
        Some(
            "from@example.com"
                .parse::<EmailAddress>()
                .expect("from parses"),
        ),
        vec!["to@example.com".parse::<EmailAddress>().expect("to parses")],
    );
    let rfc822 = b"Subject: Raw\r\n\r\nBody\r\n";

    let report = transport
        .send_raw(&envelope, rfc822, &SendOptions::new())
        .await
        .expect("raw send should succeed");

    let path = report.provider_message_id.expect("provider message id");
    let written = std::fs::read(&path).expect("file should be readable");
    assert_eq!(written, rfc822.to_vec());
}

#[tokio::test]
async fn file_transport_does_not_overwrite_existing_files() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let existing = tmp.path().join("message-0001.eml");
    std::fs::write(&existing, b"existing").expect("seed file should write");
    let transport = FileTransport::new(tmp.path()).expect("transport constructs");

    let report = transport
        .send(&sample_message(), &SendOptions::default())
        .await
        .expect("send should succeed");

    assert_eq!(
        std::fs::read(&existing).expect("existing file reads"),
        b"existing"
    );
    assert!(
        report
            .provider_message_id
            .expect("provider message id")
            .ends_with("message-0002.eml")
    );
}
