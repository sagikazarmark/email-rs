use std::sync::{Arc, Mutex, PoisonError};

use email_attachment::{
    AttachmentResolveError, MapResolver, PreparationLimits, ResolveErrorKind, ResolvingTransport,
};
use email_message::{
    Address, Attachment, AttachmentBody, AttachmentReference, Body, ContentType, Message,
    OutboundMessage,
};
use email_transport::{
    Capabilities, ErrorKind, SendOptions, SendReport, StructuredSendCapability, Transport,
    TransportError,
};
use email_transport_test::{CapturedPayload, MemoryTransport};

fn message_with(attachment: Attachment) -> OutboundMessage {
    Message::builder(Body::text("hello"))
        .from_mailbox("sender@example.com".parse().expect("sender parses"))
        .to(vec![Address::Mailbox(
            "recipient@example.com".parse().expect("recipient parses"),
        )])
        .add_attachment(attachment)
        .build_outbound()
        .expect("message validates")
}

fn content_type() -> ContentType {
    ContentType::try_from("application/octet-stream").expect("content type parses")
}

#[derive(Clone, Default)]
struct RecordingTransport {
    routes: Arc<Mutex<Vec<&'static str>>>,
}

impl RecordingTransport {
    fn routes(&self) -> Vec<&'static str> {
        self.routes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, route: &'static str) {
        self.routes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(route);
    }
}

impl Transport for RecordingTransport {
    fn capabilities(&self) -> Capabilities {
        Capabilities::new()
            .with_structured_send(StructuredSendCapability::Supported)
            .with_attachments(true)
    }

    async fn send(
        &self,
        _message: &OutboundMessage,
        _options: &SendOptions,
    ) -> Result<SendReport, TransportError> {
        self.record("borrowed");
        Ok(SendReport::new("recording"))
    }

    async fn send_owned(
        &self,
        _message: OutboundMessage,
        _options: &SendOptions,
    ) -> Result<SendReport, TransportError> {
        self.record("owned");
        Ok(SendReport::new("recording"))
    }
}

#[tokio::test]
async fn decorator_borrows_byte_only_messages_without_preparation() {
    let inner = RecordingTransport::default();
    let transport = ResolvingTransport::new(inner.clone(), MapResolver::new());
    let message = message_with(Attachment::bytes(content_type(), b"ready"));

    transport
        .send(&message, &SendOptions::default())
        .await
        .expect("send succeeds");

    assert_eq!(inner.routes(), vec!["borrowed"]);
}

#[tokio::test]
async fn decorator_enforces_limits_on_byte_only_messages() {
    let inner = RecordingTransport::default();
    let transport = ResolvingTransport::new(inner.clone(), MapResolver::new())
        .with_limits(PreparationLimits::new().with_max_attachment_bytes(Some(3)));
    let message = message_with(Attachment::bytes(content_type(), b"ready"));

    let error = transport
        .send(&message, &SendOptions::default())
        .await
        .expect_err("an oversized byte-backed attachment is rejected");

    assert_eq!(error.kind, ErrorKind::Validation);
    assert!(
        inner.routes().is_empty(),
        "the inner transport should not be reached"
    );
}

#[tokio::test]
async fn decorator_resolves_borrowed_references_then_delegates_owned() {
    let inner = RecordingTransport::default();
    let transport = ResolvingTransport::new(
        inner.clone(),
        MapResolver::new().with_entry("asset", b"ready"),
    );
    let message = message_with(Attachment::reference(
        content_type(),
        AttachmentReference::new("asset"),
    ));

    transport
        .send(&message, &SendOptions::default())
        .await
        .expect("send succeeds");

    assert_eq!(inner.routes(), vec!["owned"]);
}

#[test]
fn decorator_preserves_inner_capabilities_and_advertises_references() {
    let transport = ResolvingTransport::new(RecordingTransport::default(), MapResolver::new());

    let capabilities = transport.capabilities();

    assert!(capabilities.attachments);
    assert!(capabilities.attachment_references);
    assert_eq!(
        capabilities.structured_send,
        StructuredSendCapability::Supported
    );
}

#[test]
fn resolution_errors_map_to_transport_retry_classification() {
    let cases = [
        (
            ResolveErrorKind::UnsupportedReference,
            ErrorKind::UnsupportedFeature,
            false,
        ),
        (ResolveErrorKind::NotFound, ErrorKind::Validation, false),
        (ResolveErrorKind::TooLarge, ErrorKind::Validation, false),
        (ResolveErrorKind::Denied, ErrorKind::Internal, false),
        (
            ResolveErrorKind::Transient,
            ErrorKind::TransientProvider,
            true,
        ),
        (ResolveErrorKind::Internal, ErrorKind::Internal, false),
    ];

    for (resolve_kind, transport_kind, retryable) in cases {
        let error = TransportError::from(AttachmentResolveError::new(resolve_kind, "failed"));
        assert_eq!(error.kind, transport_kind);
        assert_eq!(error.is_retryable(), retryable);
        assert_eq!(error.is_terminal(), !retryable);
        assert!(std::error::Error::source(&error).is_some());
    }
}

#[tokio::test]
async fn reference_backed_message_is_delivered_as_bytes_end_to_end() {
    let inner = MemoryTransport::new();
    let transport = ResolvingTransport::new(
        inner.clone(),
        MapResolver::new().with_entry("invoice", b"invoice bytes"),
    );
    let message = message_with(Attachment::reference(
        content_type(),
        AttachmentReference::new("invoice"),
    ));

    transport
        .send_owned(message, &SendOptions::default())
        .await
        .expect("decorated send succeeds");

    let captured = inner.captured();
    let CapturedPayload::Structured { message, .. } = &captured[0].payload else {
        panic!("expected a structured message");
    };
    assert_eq!(
        message.attachments()[0].body(),
        &AttachmentBody::Bytes(b"invoice bytes".to_vec())
    );
}
