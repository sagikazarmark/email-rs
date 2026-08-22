use std::collections::{BTreeMap, HashMap};

use email_message::{Address, Attachment, AttachmentBody, Body, Message};
use email_transport::{ErrorKind, SendOptions, TransportError, standard_message_headers};
use resend_rs::types::{CreateAttachment, CreateEmailBaseOptions, EmailTemplate, Tag};

use crate::{ResendSendOptions, ResendTag, ResendTemplate};

pub(super) fn build_email_options(
    message: &Message,
    options: &SendOptions,
) -> Result<CreateEmailBaseOptions, TransportError> {
    let from = message
        .from_mailbox()
        .ok_or_else(|| transport_error(ErrorKind::Validation, "missing From mailbox"))?
        .to_string();

    let to = collect_mailboxes(message.to());
    let cc = collect_mailboxes(message.cc());
    let bcc = collect_mailboxes(message.bcc());

    if to.is_empty() && cc.is_empty() && bcc.is_empty() {
        return Err(transport_error(
            ErrorKind::Validation,
            "at least one recipient is required",
        ));
    }

    let subject = message.subject().map(str::to_owned).unwrap_or_default();
    let (text_body, html_body) = map_body(message.body())?;
    let reply_to = collect_mailboxes(message.reply_to());
    let headers = collect_headers(message)?;
    let attachments = message
        .attachments()
        .iter()
        .map(map_attachment)
        .collect::<Result<Vec<_>, _>>()?;

    let resend_options = options.transport_options.get::<ResendSendOptions>();
    let tags: &[ResendTag] = resend_options.map_or(&[], |options| options.tags.as_slice());
    let template = resend_options.and_then(|options| options.template.as_ref());

    let has_template = template.is_some();
    let has_text_or_html = text_body.is_some() || html_body.is_some();

    if !has_text_or_html && !has_template {
        return Err(transport_error(
            ErrorKind::Validation,
            "resend requires text/html body or template",
        ));
    }

    let mut email = CreateEmailBaseOptions::new(from, to, subject);

    if let Some(text) = text_body {
        email = email.with_text(&text);
    }
    if let Some(html) = html_body {
        email = email.with_html(&html);
    }
    for address in &cc {
        email = email.with_cc(address);
    }
    for address in &bcc {
        email = email.with_bcc(address);
    }
    if !reply_to.is_empty() {
        email = email.with_reply_multiple(&reply_to);
    }
    for (name, value) in &headers {
        email = email.with_header(name, value);
    }
    if !attachments.is_empty() {
        email = email.with_attachments(attachments);
    }
    for tag in tags {
        email = email.with_tag(Tag::new(&tag.name, &tag.value));
    }
    if let Some(template) = template {
        email = email.with_template(map_template(template));
    }

    Ok(email)
}

fn map_body(body: &Body) -> Result<(Option<String>, Option<String>), TransportError> {
    match body {
        Body::Text(text) => Ok((non_empty(text), None)),
        Body::Html(html) => Ok((None, non_empty(html))),
        Body::TextAndHtml { text, html } => Ok((non_empty(text), non_empty(html))),
        #[allow(
            unreachable_patterns,
            reason = "Body is non-exhaustive and future variants must fail explicitly"
        )]
        _ => Err(transport_error(
            ErrorKind::UnsupportedFeature,
            "non-text/html body is not supported by resend structured endpoint",
        )),
    }
}

fn collect_headers(message: &Message) -> Result<BTreeMap<String, String>, TransportError> {
    let mut headers = BTreeMap::new();
    for header in standard_message_headers(message)? {
        headers.insert(header.name().to_owned(), header.value().to_owned());
    }

    for header in message.headers() {
        headers.insert(header.name().to_owned(), header.value().to_owned());
    }

    Ok(headers)
}

fn map_attachment(attachment: &Attachment) -> Result<CreateAttachment, TransportError> {
    let AttachmentBody::Bytes(content) = attachment.body() else {
        return Err(transport_error(
            ErrorKind::UnsupportedFeature,
            "AttachmentBody variant not supported by structured Resend endpoint; \
             wrap the transport in `email_attachment::ResolvingTransport` or call \
             `email_attachment::prepare_attachments` before send",
        ));
    };

    let content_type = attachment.content_type().to_string();
    let mut resend_attachment =
        CreateAttachment::from_content(content.clone()).with_content_type(&content_type);
    if let Some(filename) = attachment.filename() {
        resend_attachment = resend_attachment.with_filename(filename);
    }
    if let Some(content_id) = attachment.content_id() {
        resend_attachment = resend_attachment.with_content_id(content_id);
    }

    Ok(resend_attachment)
}

fn map_template(template: &ResendTemplate) -> EmailTemplate {
    let mut email_template = EmailTemplate::new(&template.id);
    if let Some(variables) = &template.variables {
        email_template =
            email_template.with_variables(variables.clone().into_iter().collect::<HashMap<_, _>>());
    }
    email_template
}

fn collect_mailboxes(addresses: &[Address]) -> Vec<String> {
    addresses
        .iter()
        .flat_map(Address::mailboxes)
        .map(ToString::to_string)
        .collect()
}

fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

fn transport_error(kind: ErrorKind, message: impl Into<String>) -> TransportError {
    TransportError::new(kind, message)
}

#[cfg(test)]
mod tests {
    use email_message::{
        Address, Attachment, AttachmentReference, Body, ContentType, Mailbox, Message,
        OutboundMessage,
    };
    use email_transport::{ErrorKind, SendOptions, TransportOptions};
    use serde_json::{Value, json};
    use time::OffsetDateTime;

    use super::build_email_options;
    use crate::{ResendSendOptions, ResendTemplate};

    fn mailbox(input: &str) -> Mailbox {
        input.parse().expect("valid mailbox fixture")
    }

    fn message_with(body: Body) -> OutboundMessage {
        Message::builder(body)
            .from_mailbox(mailbox("sender@example.com"))
            .to(vec![Address::Mailbox(mailbox("recipient@example.com"))])
            .subject("Hello")
            .build_outbound()
            .expect("message should validate")
    }

    fn message_with_attachment(body: Body, attachment: Attachment) -> OutboundMessage {
        Message::builder(body)
            .from_mailbox(mailbox("sender@example.com"))
            .to(vec![Address::Mailbox(mailbox("recipient@example.com"))])
            .subject("Hello")
            .add_attachment(attachment)
            .build_outbound()
            .expect("message should validate")
    }

    #[test]
    fn build_payload_maps_text_body() {
        let message = message_with(Body::Text(String::from("Plain body")));

        let built = build_email_options(message.as_message(), &SendOptions::default())
            .expect("text payload should build");
        let json = serde_json::to_value(&built).expect("serialize request");

        assert_eq!(json["text"], "Plain body");
        assert_eq!(json.get("html"), None);
    }

    #[test]
    fn build_payload_maps_html_body() {
        let message = message_with(Body::Html(String::from("<p>HTML body</p>")));

        let built = build_email_options(message.as_message(), &SendOptions::default())
            .expect("HTML payload should build");
        let json = serde_json::to_value(&built).expect("serialize request");

        assert_eq!(json["html"], "<p>HTML body</p>");
        assert_eq!(json.get("text"), None);
    }

    #[test]
    fn build_payload_maps_byte_attachment() {
        let content = b"hello\xff\0world";
        let message = message_with_attachment(
            Body::Text(String::from("Body")),
            Attachment::bytes(
                ContentType::try_from("application/octet-stream").expect("content type parses"),
                content.to_vec(),
            )
            .with_filename("report.bin")
            .with_content_id("report"),
        );

        let built = build_email_options(message.as_message(), &SendOptions::default())
            .expect("attachment payload should build");
        let json = serde_json::to_value(&built).expect("serialize request");

        assert_eq!(
            json["attachments"][0],
            json!({
                "content": content,
                "content_id": "report",
                "contentType": "application/octet-stream",
                "filename": "report.bin",
            })
        );
    }

    #[test]
    fn build_payload_rejects_unresolved_attachment_reference() {
        let message = message_with_attachment(
            Body::Text(String::from("Body")),
            Attachment::reference(
                ContentType::try_from("application/pdf").expect("content type parses"),
                AttachmentReference::new("s3://bucket/key"),
            ),
        );

        let error = build_email_options(message.as_message(), &SendOptions::default())
            .expect_err("unresolved reference should be rejected");

        assert_eq!(error.kind, ErrorKind::UnsupportedFeature);
        assert!(
            error
                .message
                .contains("email_attachment::ResolvingTransport")
                && error
                    .message
                    .contains("email_attachment::prepare_attachments"),
            "error should point at attachment preparation: {error}"
        );
    }

    #[test]
    fn build_payload_flattens_groups() {
        let message = Message::builder(Body::Text(String::from("Body")))
            .from_mailbox(mailbox("sender@example.com"))
            .to(vec![
                "Friends: a@example.com;"
                    .parse::<Address>()
                    .expect("valid group address"),
            ])
            .subject("Hello")
            .build_outbound()
            .expect("message should validate");

        let built = build_email_options(message.as_message(), &SendOptions::default())
            .expect("group recipients should flatten");
        let json = serde_json::to_value(&built).expect("serialize request");

        assert_eq!(json["to"][0], "a@example.com");
    }

    #[test]
    fn build_payload_rejects_missing_recipients() {
        let message = Message::builder(Body::Text(String::from("Body")))
            .from_mailbox(mailbox("sender@example.com"))
            .subject("Hello")
            .build_unchecked();

        let error = build_email_options(&message, &SendOptions::default())
            .expect_err("missing recipients should be rejected");

        assert_eq!(error.kind, ErrorKind::Validation);
    }

    #[test]
    fn build_payload_accepts_template_without_body() {
        let message = message_with(Body::Html(String::new()));

        let mut transport_options = TransportOptions::default();
        transport_options
            .insert(ResendSendOptions::new().with_template(ResendTemplate::new("tmpl_123")));

        let options = SendOptions::new().with_transport_options(transport_options);

        let built = build_email_options(message.as_message(), &options);
        assert!(built.is_ok(), "template-only payload should be accepted");
    }

    #[test]
    fn build_payload_maps_typed_options() {
        let message = message_with(Body::Text(String::from("Body")));

        let mut transport_options = TransportOptions::default();
        transport_options.insert(
            ResendSendOptions::new()
                .with_tag("env", "test")
                .with_template(
                    ResendTemplate::new("tmpl_123")
                        .with_variables([("name", Value::String(String::from("Mark")))]),
                ),
        );

        let options = SendOptions::new().with_transport_options(transport_options);

        let built =
            build_email_options(message.as_message(), &options).expect("payload should build");
        let json = serde_json::to_value(&built).expect("serialize request");

        assert_eq!(json["tags"][0]["name"], "env");
        assert_eq!(json["template"]["id"], "tmpl_123");
        assert_eq!(json["template"]["variables"]["name"], "Mark");
    }

    #[test]
    fn build_payload_includes_typed_standard_headers() {
        let message_id = "<resend@example.com>"
            .parse()
            .expect("message id should parse");

        let message = Message::builder(Body::Text(String::from("Body")))
            .from_mailbox(mailbox("sender@example.com"))
            .sender(mailbox("bounce@example.com"))
            .to(vec![Address::Mailbox(mailbox("recipient@example.com"))])
            .date(OffsetDateTime::UNIX_EPOCH)
            .message_id(message_id)
            .subject("Hello")
            .build_outbound()
            .expect("message should validate");

        let built = build_email_options(message.as_message(), &SendOptions::default())
            .expect("payload should build");
        let json = serde_json::to_value(&built).expect("serialize request");

        let headers = json["headers"]
            .as_object()
            .expect("headers should be an object");

        assert_eq!(
            headers.get("Sender"),
            Some(&Value::String(String::from("bounce@example.com")))
        );
        assert!(headers.contains_key("Date"));
        assert_eq!(
            headers.get("Message-ID"),
            Some(&Value::String(String::from("<resend@example.com>")))
        );
    }
}
