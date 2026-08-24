use email_attachment::{AttachmentLimits, MapResolver, ResolveErrorKind, prepare_attachments};
use email_message::{
    Address, Attachment, AttachmentBody, AttachmentReference, Body, ContentType, Disposition,
    Message, OutboundMessage,
};

fn message_with(attachments: Vec<Attachment>) -> OutboundMessage {
    Message::builder(Body::text("hello"))
        .from_mailbox("sender@example.com".parse().expect("sender parses"))
        .to(vec![Address::Mailbox(
            "recipient@example.com".parse().expect("recipient parses"),
        )])
        .attachments(attachments)
        .build_outbound()
        .expect("message validates")
}

fn content_type() -> ContentType {
    ContentType::try_from("application/octet-stream").expect("content type parses")
}

#[tokio::test]
async fn preparation_materializes_references_and_preserves_attachment_metadata() {
    let byte_attachment = Attachment::bytes(content_type(), b"existing").with_filename("old.bin");
    let reference_attachment =
        Attachment::reference(content_type(), AttachmentReference::new("document-key"))
            .with_filename("document.bin")
            .with_content_id("document")
            .with_disposition(Disposition::Inline);
    let second_reference = Attachment::reference(
        content_type(),
        AttachmentReference::new("second-document-key"),
    );
    let resolver = MapResolver::new()
        .with_entry("document-key", b"resolved")
        .with_entry("second-document-key", b"also resolved");

    let prepared = prepare_attachments(
        message_with(vec![
            byte_attachment.clone(),
            reference_attachment,
            second_reference,
        ]),
        &resolver,
        &AttachmentLimits::default(),
    )
    .await
    .expect("preparation succeeds");

    let attachments = prepared.as_message().attachments();
    assert_eq!(attachments[0], byte_attachment);
    assert_eq!(attachments[1].filename(), Some("document.bin"));
    assert_eq!(attachments[1].content_id(), Some("document"));
    assert_eq!(attachments[1].disposition(), Disposition::Inline);
    assert_eq!(
        attachments[1].body(),
        &AttachmentBody::Bytes(b"resolved".to_vec())
    );
    assert_eq!(
        attachments[2].body(),
        &AttachmentBody::Bytes(b"also resolved".to_vec())
    );
}

#[tokio::test]
async fn preparation_returns_byte_only_messages_unchanged() {
    let message = message_with(vec![Attachment::bytes(content_type(), b"existing")]);

    let prepared = prepare_attachments(
        message.clone(),
        &MapResolver::new(),
        &AttachmentLimits::default(),
    )
    .await
    .expect("byte-only message passes through");

    assert_eq!(prepared, message);
}

#[tokio::test]
async fn preparation_enforces_limits_on_byte_only_messages() {
    let mut limits = AttachmentLimits::default();
    limits.max_total_bytes = Some(7);

    let error = prepare_attachments(
        message_with(vec![
            Attachment::bytes(content_type(), b"four"),
            Attachment::bytes(content_type(), b"more"),
        ]),
        &MapResolver::new(),
        &limits,
    )
    .await
    .expect_err("a message without references is still held to the limits");

    assert_eq!(error.kind, ResolveErrorKind::TooLarge);
}

#[tokio::test]
async fn preparation_enforces_per_attachment_limit() {
    let mut limits = AttachmentLimits::default();
    limits.max_attachment_bytes = Some(3);

    let error = prepare_attachments(
        message_with(vec![Attachment::reference(
            content_type(),
            AttachmentReference::new("large"),
        )]),
        &MapResolver::new().with_entry("large", b"four"),
        &limits,
    )
    .await
    .expect_err("oversized attachment fails");

    assert_eq!(error.kind, ResolveErrorKind::TooLarge);
}

#[tokio::test]
async fn preparation_enforces_total_limit_across_existing_and_resolved_bytes() {
    let mut limits = AttachmentLimits::default();
    limits.max_total_bytes = Some(7);

    let error = prepare_attachments(
        message_with(vec![
            Attachment::bytes(content_type(), b"four"),
            Attachment::reference(content_type(), AttachmentReference::new("another-four")),
        ]),
        &MapResolver::new().with_entry("another-four", b"more"),
        &limits,
    )
    .await
    .expect_err("oversized total fails");

    assert_eq!(error.kind, ResolveErrorKind::TooLarge);
}
