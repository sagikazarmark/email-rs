use std::collections::BTreeMap;

use email_message::{Address, Attachment, AttachmentBody, Body, Header, Mailbox, Message};
use email_transport::{ErrorKind, TransportError};

/// Plain-Rust image of a Cloudflare `EmailMessageBuilder` object.
///
/// Built from an [`email_message::Message`] on every target so the mapping is
/// host-testable; the `wasm32` binding glue turns it into the JS object the
/// `send_email` binding expects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct EmailPayload {
    pub from: PayloadAddress,
    pub to: Vec<PayloadAddress>,
    pub cc: Vec<PayloadAddress>,
    pub bcc: Vec<PayloadAddress>,
    pub reply_to: Option<PayloadAddress>,
    pub subject: String,
    pub text: Option<String>,
    pub html: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub attachments: Vec<PayloadAttachment>,
}

/// A mailbox as Cloudflare's `EmailAddress` (`{ email, name? }`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PayloadAddress {
    pub name: Option<String>,
    pub email: String,
}

impl From<&Mailbox> for PayloadAddress {
    fn from(mailbox: &Mailbox) -> Self {
        Self {
            name: mailbox.name().map(str::to_owned),
            email: mailbox.email().as_str().to_owned(),
        }
    }
}

/// A byte-backed attachment as Cloudflare's `Attachment`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PayloadAttachment {
    pub filename: String,
    pub content_type: String,
    pub disposition: PayloadDisposition,
    pub content: Vec<u8>,
}

/// Cloudflare's attachment disposition union: inline parts carry a content id,
/// regular attachments never do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum PayloadDisposition {
    Attachment,
    Inline { content_id: String },
}

/// Map a message to the payload handed to the `send_email` binding.
///
/// Only the message's custom headers are forwarded. `Date`, `Message-ID` and
/// `Sender` are deliberately dropped: Cloudflare rejects the first two with
/// `E_HEADER_NOT_ALLOWED` and stamps its own `Message-ID`. Cloudflare's
/// `headers` field is a plain object, so repeated header names collapse to the
/// last value, as they do for Resend; see [`collect_headers`] for how names
/// that differ only in case are folded.
///
/// The `to`, `cc` and `bcc` lists may each be empty; the binding glue omits
/// empty ones because the platform requires at least one of the three to be
/// present, not `to` specifically.
pub(super) fn build_payload(message: &Message) -> Result<EmailPayload, TransportError> {
    let from = message
        .from_mailbox()
        .map(PayloadAddress::from)
        .ok_or_else(|| transport_error(ErrorKind::Validation, "missing From mailbox"))?;

    let to = collect_addresses(message.to());
    let cc = collect_addresses(message.cc());
    let bcc = collect_addresses(message.bcc());

    if to.is_empty() && cc.is_empty() && bcc.is_empty() {
        return Err(transport_error(
            ErrorKind::Validation,
            "at least one recipient is required",
        ));
    }

    let reply_to = map_reply_to(message.reply_to())?;
    let subject = message.subject().map(str::to_owned).unwrap_or_default();
    let (text, html) = map_body(message.body())?;

    if text.is_none() && html.is_none() {
        return Err(transport_error(
            ErrorKind::Validation,
            "cloudflare requires a non-empty text or html body",
        ));
    }

    let headers = collect_headers(message.headers());
    let attachments = message
        .attachments()
        .iter()
        .enumerate()
        .map(|(index, attachment)| map_attachment(attachment, index + 1))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(EmailPayload {
        from,
        to,
        cc,
        bcc,
        reply_to,
        subject,
        text,
        html,
        headers,
        attachments,
    })
}

fn map_reply_to(addresses: &[Address]) -> Result<Option<PayloadAddress>, TransportError> {
    let mut reply_to = collect_addresses(addresses);
    match reply_to.len() {
        0 => Ok(None),
        1 => Ok(reply_to.pop()),
        _ => Err(transport_error(
            ErrorKind::UnsupportedFeature,
            "cloudflare accepts a single Reply-To mailbox",
        )),
    }
}

fn map_body(body: &Body) -> Result<(Option<String>, Option<String>), TransportError> {
    match body {
        Body::Text(text) => Ok((non_empty(text), None)),
        Body::Html(html) => Ok((None, non_empty(html))),
        Body::TextAndHtml { text, html } => Ok((non_empty(text), non_empty(html))),
        // `Body::Mime` and any future variant fail explicitly.
        _ => Err(transport_error(
            ErrorKind::UnsupportedFeature,
            "non-text/html body is not supported by the cloudflare structured send API",
        )),
    }
}

/// Map one attachment. `position` is its 1-based index in the message and
/// names a regular attachment that has no filename.
///
/// Cloudflare requires a filename on every attachment and a content id on
/// every inline one, and accepts no content id on a regular attachment. A
/// missing filename is synthesised (`attachment-N`, or the content id for an
/// inline part) rather than rejected, because every other transport in the
/// workspace treats the filename as optional. A content id on a regular
/// attachment is dropped; the recipient's client renders it identically.
fn map_attachment(
    attachment: &Attachment,
    position: usize,
) -> Result<PayloadAttachment, TransportError> {
    let AttachmentBody::Bytes(content) = attachment.body() else {
        return Err(transport_error(
            ErrorKind::UnsupportedFeature,
            "AttachmentBody variant not supported by the cloudflare send_email binding; \
             wrap the transport in `email_attachment::AttachmentResolvingTransport` or call \
             `email_attachment::prepare_attachments` before send",
        ));
    };

    let disposition = if attachment.is_inline() {
        let content_id = attachment.content_id().ok_or_else(|| {
            transport_error(
                ErrorKind::Validation,
                "cloudflare requires a content id on every inline attachment",
            )
        })?;
        PayloadDisposition::Inline {
            content_id: content_id.to_owned(),
        }
    } else {
        PayloadDisposition::Attachment
    };

    let filename = match (attachment.filename(), &disposition) {
        (Some(filename), _) => filename.to_owned(),
        (None, PayloadDisposition::Inline { content_id }) => content_id.clone(),
        (None, PayloadDisposition::Attachment) => format!("attachment-{position}"),
    };

    Ok(PayloadAttachment {
        filename,
        content_type: attachment.content_type().to_string(),
        disposition,
        content: content.clone(),
    })
}

fn collect_addresses(addresses: &[Address]) -> Vec<PayloadAddress> {
    addresses
        .iter()
        .flat_map(Address::mailboxes)
        .map(PayloadAddress::from)
        .collect()
}

/// Collapse headers into Cloudflare's single-occurrence map.
///
/// Header names are case-insensitive (RFC 5322 §2.2) and Cloudflare matches
/// them that way, but its `headers` field is a plain object keyed by the exact
/// string, so two spellings of one name would reach the platform as two
/// properties with undefined precedence. Names are folded here instead: the
/// first spelling seen is kept and the last value wins, which is the same
/// last-wins rule an exact duplicate already gets.
fn collect_headers(headers: &[Header]) -> BTreeMap<String, String> {
    let mut collected: BTreeMap<String, String> = BTreeMap::new();
    for header in headers {
        let name = collected
            .keys()
            .find(|existing| existing.eq_ignore_ascii_case(header.name()))
            .cloned()
            .unwrap_or_else(|| header.name().to_owned());
        collected.insert(name, header.value().to_owned());
    }
    collected
}

fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

fn transport_error(kind: ErrorKind, message: impl Into<String>) -> TransportError {
    TransportError::new(kind, message)
}

/// Fixtures are built with `build_outbound()` so every case is a message that
/// would genuinely reach the transport; the one `build_unchecked()` case is the
/// missing-From backstop that `OutboundMessage`'s typestate makes unreachable.
#[cfg(test)]
mod tests {
    use email_message::{
        Address, Attachment, AttachmentReference, Body, ContentType, Disposition, Header, Mailbox,
        Message, MimePart, OutboundMessage,
    };
    use email_transport::{ErrorKind, TransportError};
    use email_transport_test::conformance::conformance_message;

    use super::{EmailPayload, PayloadAddress, PayloadDisposition, build_payload};

    fn mailbox(input: &str) -> Mailbox {
        input.parse().expect("valid mailbox fixture")
    }

    fn address(name: Option<&str>, email: &str) -> PayloadAddress {
        PayloadAddress {
            name: name.map(str::to_owned),
            email: email.to_owned(),
        }
    }

    fn message_with(body: Body) -> OutboundMessage {
        Message::builder(body)
            .from_mailbox(mailbox("sender@example.com"))
            .to(vec![Address::Mailbox(mailbox("recipient@example.com"))])
            .subject("Hello")
            .build_outbound()
            .expect("message should validate")
    }

    fn minimal_message() -> OutboundMessage {
        message_with(Body::text("Body"))
    }

    fn message_with_attachment(attachment: Attachment) -> OutboundMessage {
        Message::builder(Body::text("Body"))
            .from_mailbox(mailbox("sender@example.com"))
            .to(vec![Address::Mailbox(mailbox("recipient@example.com"))])
            .subject("Hello")
            .add_attachment(attachment)
            .build_outbound()
            .expect("message should validate")
    }

    fn payload_for(message: &OutboundMessage) -> EmailPayload {
        build_payload(message.as_message()).expect("message should map to a payload")
    }

    fn error_for(message: &OutboundMessage) -> TransportError {
        build_payload(message.as_message()).expect_err("message should be rejected")
    }

    // --- conformance ------------------------------------------------------

    #[test]
    fn conforms_to_shared_semantics_and_drops_platform_headers() {
        let payload = payload_for(&conformance_message());

        assert_eq!(payload.from, address(None, "sender@example.com"));
        assert_eq!(
            payload.to,
            vec![
                address(None, "a@example.com"),
                address(None, "b@example.com")
            ]
        );
        assert_eq!(payload.cc, vec![address(None, "cc@example.com")]);
        assert_eq!(payload.bcc, vec![address(None, "hidden@example.com")]);
        assert_eq!(payload.reply_to, Some(address(None, "reply@example.com")));
        assert_eq!(payload.subject, "Conformance");
        assert_eq!(payload.text.as_deref(), Some("Hello from conformance test"));
        assert_eq!(
            payload.headers.get("X-Test").map(String::as_str),
            Some("demo")
        );
        assert!(!payload.headers.contains_key("Date"));
        assert!(!payload.headers.contains_key("Message-ID"));
        assert!(!payload.headers.contains_key("Sender"));
    }

    // --- addresses --------------------------------------------------------

    #[test]
    fn display_names_are_preserved_on_every_address_field() {
        let message = Message::builder(Body::text("Body"))
            .from_mailbox(mailbox("Sender <sender@example.com>"))
            .to(vec![Address::Mailbox(mailbox("To <to@example.com>"))])
            .cc(vec![Address::Mailbox(mailbox("Cc <cc@example.com>"))])
            .bcc(vec![Address::Mailbox(mailbox("Bcc <bcc@example.com>"))])
            .reply_to(vec![Address::Mailbox(mailbox("Reply <reply@example.com>"))])
            .build_outbound()
            .expect("message should validate");

        let payload = payload_for(&message);

        assert_eq!(payload.from, address(Some("Sender"), "sender@example.com"));
        assert_eq!(payload.to, vec![address(Some("To"), "to@example.com")]);
        assert_eq!(payload.cc, vec![address(Some("Cc"), "cc@example.com")]);
        assert_eq!(payload.bcc, vec![address(Some("Bcc"), "bcc@example.com")]);
        assert_eq!(
            payload.reply_to,
            Some(address(Some("Reply"), "reply@example.com"))
        );
    }

    #[test]
    fn groups_are_flattened_to_member_mailboxes() {
        let message = Message::builder(Body::text("Body"))
            .from_mailbox(mailbox("sender@example.com"))
            .to(vec![
                "Friends: Ada <a@example.com>, b@example.com;"
                    .parse::<Address>()
                    .expect("valid group address"),
            ])
            .build_outbound()
            .expect("message should validate");

        let payload = payload_for(&message);

        assert_eq!(
            payload.to,
            vec![
                address(Some("Ada"), "a@example.com"),
                address(None, "b@example.com")
            ]
        );
    }

    #[test]
    fn cc_only_message_is_accepted() {
        let message = Message::builder(Body::text("Body"))
            .from_mailbox(mailbox("sender@example.com"))
            .cc(vec![Address::Mailbox(mailbox("cc@example.com"))])
            .build_outbound()
            .expect("message should validate");

        let payload = payload_for(&message);

        assert!(payload.to.is_empty());
        assert_eq!(payload.cc, vec![address(None, "cc@example.com")]);
    }

    #[test]
    fn bcc_only_message_is_accepted() {
        let message = Message::builder(Body::text("Body"))
            .from_mailbox(mailbox("sender@example.com"))
            .bcc(vec![Address::Mailbox(mailbox("hidden@example.com"))])
            .build_outbound()
            .expect("message should validate");

        let payload = payload_for(&message);

        assert!(payload.to.is_empty());
        assert_eq!(payload.bcc, vec![address(None, "hidden@example.com")]);
    }

    #[test]
    fn empty_group_with_no_other_recipients_fails_validation() {
        // An empty group satisfies `OutboundMessage`'s recipient check (the
        // `to` list is non-empty) but flattens to zero mailboxes, so this is
        // the one way a message with no recipients reaches the transport.
        let message = Message::builder(Body::text("Body"))
            .from_mailbox(mailbox("sender@example.com"))
            .to(vec![
                "Undisclosed recipients:;"
                    .parse::<Address>()
                    .expect("valid empty group address"),
            ])
            .build_outbound()
            .expect("an empty group passes message validation");

        let error = error_for(&message);

        assert_eq!(error.kind, ErrorKind::Validation);
        assert!(error.message.contains("recipient"));
    }

    #[test]
    fn missing_from_fails_validation() {
        let message = Message::builder(Body::text("Body"))
            .to(vec![Address::Mailbox(mailbox("recipient@example.com"))])
            .build_unchecked();

        let error = build_payload(&message).expect_err("missing From should be rejected");

        assert_eq!(error.kind, ErrorKind::Validation);
        assert!(error.message.contains("From"));
    }

    #[test]
    fn multiple_reply_to_mailboxes_are_unsupported() {
        let message = Message::builder(Body::text("Body"))
            .from_mailbox(mailbox("sender@example.com"))
            .to(vec![Address::Mailbox(mailbox("recipient@example.com"))])
            .reply_to(vec![
                Address::Mailbox(mailbox("a@example.com")),
                Address::Mailbox(mailbox("b@example.com")),
            ])
            .build_outbound()
            .expect("message should validate");

        let error = error_for(&message);

        assert_eq!(error.kind, ErrorKind::UnsupportedFeature);
        assert!(error.message.contains("Reply-To"));
    }

    #[test]
    fn absent_reply_to_is_omitted() {
        assert_eq!(payload_for(&minimal_message()).reply_to, None);
    }

    // --- subject and body -------------------------------------------------

    #[test]
    fn missing_subject_is_sent_as_empty_string() {
        let message = Message::builder(Body::text("Body"))
            .from_mailbox(mailbox("sender@example.com"))
            .to(vec![Address::Mailbox(mailbox("recipient@example.com"))])
            .build_outbound()
            .expect("message should validate");

        assert_eq!(payload_for(&message).subject, "");
    }

    #[test]
    fn text_body_maps_to_text() {
        let payload = payload_for(&message_with(Body::text("Plain")));

        assert_eq!(payload.text.as_deref(), Some("Plain"));
        assert_eq!(payload.html, None);
    }

    #[test]
    fn html_body_maps_to_html() {
        let payload = payload_for(&message_with(Body::html("<p>Rich</p>")));

        assert_eq!(payload.text, None);
        assert_eq!(payload.html.as_deref(), Some("<p>Rich</p>"));
    }

    #[test]
    fn text_and_html_body_maps_to_both() {
        let payload = payload_for(&message_with(Body::text_and_html("Plain", "<p>Rich</p>")));

        assert_eq!(payload.text.as_deref(), Some("Plain"));
        assert_eq!(payload.html.as_deref(), Some("<p>Rich</p>"));
    }

    #[test]
    fn empty_text_in_text_and_html_is_dropped() {
        let payload = payload_for(&message_with(Body::text_and_html("", "<p>Rich</p>")));

        assert_eq!(payload.text, None);
        assert_eq!(payload.html.as_deref(), Some("<p>Rich</p>"));
    }

    #[test]
    fn empty_body_fails_validation() {
        let error = error_for(&message_with(Body::text("")));

        assert_eq!(error.kind, ErrorKind::Validation);
        assert!(error.message.contains("body"));
    }

    #[test]
    fn mime_body_is_unsupported() {
        let body = Body::Mime(MimePart::Leaf {
            content_type: ContentType::try_from("text/plain").expect("content type parses"),
            content_transfer_encoding: None,
            content_disposition: None,
            body: b"Body".to_vec(),
        });

        let error = error_for(&message_with(body));

        assert_eq!(error.kind, ErrorKind::UnsupportedFeature);
    }

    // --- headers ----------------------------------------------------------

    #[test]
    fn custom_headers_are_forwarded() {
        let message = Message::builder(Body::text("Body"))
            .from_mailbox(mailbox("sender@example.com"))
            .to(vec![Address::Mailbox(mailbox("recipient@example.com"))])
            .add_header(Header::new("X-Campaign", "spring").expect("header validates"))
            .add_header(
                Header::new("List-Unsubscribe", "<mailto:unsub@example.com>")
                    .expect("header validates"),
            )
            .add_header(
                Header::new("In-Reply-To", "<parent@example.com>").expect("header validates"),
            )
            .build_outbound()
            .expect("message should validate");

        let payload = payload_for(&message);

        assert_eq!(payload.headers.len(), 3);
        assert_eq!(payload.headers["X-Campaign"], "spring");
        assert_eq!(
            payload.headers["List-Unsubscribe"],
            "<mailto:unsub@example.com>"
        );
        assert_eq!(payload.headers["In-Reply-To"], "<parent@example.com>");
    }

    #[test]
    fn header_names_fold_case_insensitively_keeping_first_spelling_and_last_value() {
        let message = Message::builder(Body::text("Body"))
            .from_mailbox(mailbox("sender@example.com"))
            .to(vec![Address::Mailbox(mailbox("recipient@example.com"))])
            .add_header(Header::new("X-Campaign", "spring").expect("header validates"))
            .add_header(Header::new("x-campaign", "summer").expect("header validates"))
            .add_header(Header::new("X-CAMPAIGN", "autumn").expect("header validates"))
            .add_header(Header::new("X-Other", "kept").expect("header validates"))
            .build_outbound()
            .expect("message should validate");

        let payload = payload_for(&message);

        assert_eq!(payload.headers.len(), 2, "headers: {:?}", payload.headers);
        assert_eq!(payload.headers["X-Campaign"], "autumn");
        assert_eq!(payload.headers["X-Other"], "kept");
    }

    #[test]
    fn date_message_id_and_sender_are_never_emitted() {
        let message = Message::builder(Body::text("Body"))
            .from_mailbox(mailbox("sender@example.com"))
            .sender(mailbox("bounce@example.com"))
            .to(vec![Address::Mailbox(mailbox("recipient@example.com"))])
            .date(time::OffsetDateTime::UNIX_EPOCH)
            .message_id("<mine@example.com>".parse().expect("message id parses"))
            .build_outbound()
            .expect("message should validate");

        let payload = payload_for(&message);

        assert!(payload.headers.is_empty(), "headers: {:?}", payload.headers);
    }

    // --- attachments ------------------------------------------------------

    #[test]
    fn byte_attachment_carries_metadata_and_binary_content() {
        let content = b"hello\xff\0world";
        let message = message_with_attachment(
            Attachment::bytes(
                ContentType::try_from("application/octet-stream").expect("content type parses"),
                content.to_vec(),
            )
            .with_filename("report.bin"),
        );

        let payload = payload_for(&message);

        assert_eq!(payload.attachments.len(), 1);
        let attachment = &payload.attachments[0];
        assert_eq!(attachment.filename, "report.bin");
        assert_eq!(attachment.content_type, "application/octet-stream");
        assert_eq!(attachment.disposition, PayloadDisposition::Attachment);
        assert_eq!(attachment.content, content);
    }

    #[test]
    fn inline_attachment_carries_content_id_and_disposition() {
        let message = message_with_attachment(
            Attachment::bytes(
                ContentType::try_from("image/png").expect("content type parses"),
                vec![0x89, b'P', b'N', b'G'],
            )
            .with_filename("logo.png")
            .with_content_id("logo")
            .with_disposition(Disposition::Inline),
        );

        let payload = payload_for(&message);

        let attachment = &payload.attachments[0];
        assert_eq!(attachment.filename, "logo.png");
        assert_eq!(attachment.content_type, "image/png");
        assert_eq!(
            attachment.disposition,
            PayloadDisposition::Inline {
                content_id: String::from("logo")
            }
        );
    }

    #[test]
    fn regular_attachment_without_filename_is_named_by_position() {
        let pdf = ContentType::try_from("application/pdf").expect("content type parses");
        let message = Message::builder(Body::text("Body"))
            .from_mailbox(mailbox("sender@example.com"))
            .to(vec![Address::Mailbox(mailbox("recipient@example.com"))])
            .add_attachment(Attachment::bytes(pdf.clone(), b"%PDF-1".to_vec()))
            .add_attachment(
                Attachment::bytes(pdf.clone(), b"%PDF-2".to_vec()).with_filename("named.pdf"),
            )
            .add_attachment(Attachment::bytes(pdf, b"%PDF-3".to_vec()))
            .build_outbound()
            .expect("message should validate");

        let payload = payload_for(&message);

        let filenames: Vec<&str> = payload
            .attachments
            .iter()
            .map(|attachment| attachment.filename.as_str())
            .collect();
        assert_eq!(filenames, vec!["attachment-1", "named.pdf", "attachment-3"]);
    }

    #[test]
    fn inline_attachment_without_filename_is_named_by_content_id() {
        let message = message_with_attachment(
            Attachment::bytes(
                ContentType::try_from("image/png").expect("content type parses"),
                vec![0x89, b'P', b'N', b'G'],
            )
            .with_content_id("logo@example.com")
            .with_disposition(Disposition::Inline),
        );

        assert_eq!(
            payload_for(&message).attachments[0].filename,
            "logo@example.com"
        );
    }

    #[test]
    fn inline_attachment_without_content_id_fails_validation() {
        let message = message_with_attachment(
            Attachment::bytes(
                ContentType::try_from("image/png").expect("content type parses"),
                vec![0x89, b'P', b'N', b'G'],
            )
            .with_filename("logo.png")
            .with_disposition(Disposition::Inline),
        );

        let error = error_for(&message);

        assert_eq!(error.kind, ErrorKind::Validation);
        assert!(error.message.contains("content id"));
    }

    #[test]
    fn content_id_on_regular_attachment_is_dropped() {
        let message = message_with_attachment(
            Attachment::bytes(
                ContentType::try_from("application/pdf").expect("content type parses"),
                b"%PDF".to_vec(),
            )
            .with_filename("report.pdf")
            .with_content_id("report"),
        );

        assert_eq!(
            payload_for(&message).attachments[0].disposition,
            PayloadDisposition::Attachment
        );
    }

    #[test]
    fn attachment_reference_is_unsupported_with_preparation_hint() {
        let message = message_with_attachment(Attachment::reference(
            ContentType::try_from("application/pdf").expect("content type parses"),
            AttachmentReference::new("s3://bucket/key"),
        ));

        let error = error_for(&message);

        assert_eq!(error.kind, ErrorKind::UnsupportedFeature);
        assert!(
            error
                .message
                .contains("email_attachment::AttachmentResolvingTransport")
                && error
                    .message
                    .contains("email_attachment::prepare_attachments"),
            "error should point at attachment preparation: {error}"
        );
    }
}
