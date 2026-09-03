use std::collections::BTreeMap;

use email_message::{Address, Attachment, AttachmentBody, Body, Mailbox, Message};
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
    pub inline: bool,
    pub content_id: Option<String>,
    pub content: Vec<u8>,
}

/// Map a message to the payload handed to the `send_email` binding.
///
/// Only the message's custom headers are forwarded. `Date`, `Message-ID` and
/// `Sender` are deliberately dropped: Cloudflare rejects the first two with
/// `E_HEADER_NOT_ALLOWED` and stamps its own `Message-ID`. Cloudflare's
/// `headers` field is a plain object, so repeated header names collapse to the
/// last value, as they do for Resend.
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

    let headers = message
        .headers()
        .iter()
        .map(|header| (header.name().to_owned(), header.value().to_owned()))
        .collect();
    let attachments = message
        .attachments()
        .iter()
        .map(map_attachment)
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
        #[allow(
            unreachable_patterns,
            reason = "Body is non-exhaustive and future variants must fail explicitly"
        )]
        _ => Err(transport_error(
            ErrorKind::UnsupportedFeature,
            "non-text/html body is not supported by the cloudflare structured send API",
        )),
    }
}

fn map_attachment(attachment: &Attachment) -> Result<PayloadAttachment, TransportError> {
    let AttachmentBody::Bytes(content) = attachment.body() else {
        return Err(transport_error(
            ErrorKind::UnsupportedFeature,
            "AttachmentBody variant not supported by the cloudflare send_email binding; \
             wrap the transport in `email_attachment::AttachmentResolvingTransport` or call \
             `email_attachment::prepare_attachments` before send",
        ));
    };

    let filename = attachment.filename().map(str::to_owned).ok_or_else(|| {
        transport_error(
            ErrorKind::Validation,
            "cloudflare requires a filename on every attachment",
        )
    })?;

    Ok(PayloadAttachment {
        filename,
        content_type: attachment.content_type().to_string(),
        inline: attachment.is_inline(),
        content_id: attachment.content_id().map(str::to_owned),
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

fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

fn transport_error(kind: ErrorKind, message: impl Into<String>) -> TransportError {
    TransportError::new(kind, message)
}

/// Backstops that `OutboundMessage`'s typestate makes unreachable through
/// `Transport::send`; every other mapping rule is asserted at the public seam
/// in `transport::tests`.
#[cfg(test)]
mod tests {
    use email_message::{Address, Body, Mailbox, Message};
    use email_transport::ErrorKind;

    use super::build_payload;

    fn mailbox(input: &str) -> Mailbox {
        input.parse().expect("valid mailbox fixture")
    }

    #[test]
    fn missing_recipients_fail_validation() {
        let message = Message::builder(Body::text("Body"))
            .from_mailbox(mailbox("sender@example.com"))
            .build_unchecked();

        let error = build_payload(&message).expect_err("missing recipients should be rejected");

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
}
