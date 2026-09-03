mod binding;
mod error;
mod payload;

use std::sync::Arc;

use email_message::OutboundMessage;
use email_transport::{
    BoxFut, Capabilities, ErrorKind, RuntimeBound, SendOptions, SendReport,
    StructuredSendCapability, Transport, TransportError, structured_accepted_for,
};
use worker::{Env, SendEmail};

use self::payload::EmailPayload;

/// Structured email transport backed by a Cloudflare Workers `send_email`
/// binding.
///
/// The transport maps [`email_message::Message`] values to Cloudflare's
/// structured send API (`EmailMessageBuilder`) and dispatches them through
/// [`worker::SendEmail::send_with_builder`]. It is cheap to clone; clones share
/// the same binding handle.
///
/// The binding only functions on `wasm32-unknown-unknown` inside `workerd`. On
/// other targets the transport still compiles, but [`Transport::send`] returns
/// an [`ErrorKind::UnsupportedFeature`] error instead of reaching
/// wasm-bindgen's panicking extern stubs.
#[derive(Clone)]
pub struct CloudflareTransport {
    binding: Arc<SendEmail>,
    sender: Arc<dyn EmailSender>,
}

/// Hand-written `Debug` so the binding handle never reaches logs. Formatting a
/// `JsValue` calls into the JS runtime, which is unavailable on native targets
/// and uninteresting inside a Worker.
impl std::fmt::Debug for CloudflareTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudflareTransport")
            .field("binding", &"<cloudflare send_email binding>")
            .finish()
    }
}

impl CloudflareTransport {
    /// Construct a transport from an already-obtained `send_email` binding.
    #[must_use]
    pub fn new(binding: SendEmail) -> Self {
        let binding = Arc::new(binding);
        let sender = BindingSender {
            binding: Arc::clone(&binding),
        };
        Self::from_parts(binding, sender)
    }

    /// Construct a transport from the Worker environment and the name of a
    /// `[[send_email]]` binding declared in `wrangler.toml`.
    ///
    /// # Errors
    ///
    /// Returns the `worker` error when no binding with that name exists. A
    /// missing binding is a deployment error, not a send error, so it is not
    /// reported as a [`TransportError`].
    pub fn from_env(env: &Env, binding: &str) -> worker::Result<Self> {
        env.send_email(binding).map(Self::new)
    }

    /// Return the underlying `send_email` binding.
    #[must_use]
    pub fn binding(&self) -> &SendEmail {
        &self.binding
    }

    fn from_parts(binding: Arc<SendEmail>, sender: impl EmailSender + 'static) -> Self {
        Self {
            binding,
            sender: Arc::new(sender),
        }
    }
}

impl From<SendEmail> for CloudflareTransport {
    fn from(binding: SendEmail) -> Self {
        Self::new(binding)
    }
}

impl Transport for CloudflareTransport {
    fn capabilities(&self) -> Capabilities {
        Capabilities::new()
            .with_structured_send(StructuredSendCapability::Supported)
            .with_custom_headers(true)
            .with_attachments(true)
            .with_inline_attachments(true)
    }

    async fn send(
        &self,
        message: &OutboundMessage,
        options: &SendOptions,
    ) -> Result<SendReport, TransportError> {
        let message = message.as_message();
        let payload = payload::build_payload(message)?;
        let accepted = structured_accepted_for(message, options, self.capabilities());

        let message_id = self.sender.send(payload).await.map_err(map_sender_error)?;

        Ok(SendReport::new(PROVIDER)
            .with_provider_message_id(message_id)
            .with_accepted(accepted))
    }
}

/// Stable [`SendReport::provider`] identifier for this transport.
pub const PROVIDER: &str = "cloudflare";

/// Crate-private seam between the transport and the `send_email` binding.
///
/// The production implementation hands the payload to the binding; tests
/// substitute a double that records the payload and returns canned results.
trait EmailSender: RuntimeBound {
    /// Deliver `payload`, returning Cloudflare's `messageId` on success.
    fn send(&self, payload: EmailPayload) -> BoxFut<'_, Result<String, SenderError>>;
}

/// Failure reported by an [`EmailSender`] before classification.
#[derive(Clone, Debug, PartialEq, Eq)]
enum SenderError {
    /// The binding threw a JS error carrying Cloudflare's `code` (when
    /// present) and `message` properties.
    #[cfg_attr(
        not(target_arch = "wasm32"),
        allow(
            dead_code,
            reason = "constructed only by the wasm32 binding glue and by tests"
        )
    )]
    Binding {
        code: Option<String>,
        message: String,
    },
    /// The binding cannot be invoked on this compilation target.
    #[cfg_attr(
        target_arch = "wasm32",
        allow(dead_code, reason = "constructed only on non-wasm32 targets")
    )]
    UnsupportedTarget,
}

struct BindingSender {
    binding: Arc<SendEmail>,
}

impl EmailSender for BindingSender {
    fn send(&self, payload: EmailPayload) -> BoxFut<'_, Result<String, SenderError>> {
        Box::pin(binding::send(&self.binding, payload))
    }
}

fn map_sender_error(error: SenderError) -> TransportError {
    match error {
        SenderError::Binding { code, message } => {
            error::map_binding_error(code.as_deref(), message)
        }
        SenderError::UnsupportedTarget => TransportError::new(
            ErrorKind::UnsupportedFeature,
            "the cloudflare send_email binding is only available on wasm32-unknown-unknown \
             inside Cloudflare Workers",
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use email_message::{
        Address, Attachment, AttachmentReference, Body, ContentType, Disposition, Header, Mailbox,
        Message, MimePart, OutboundMessage,
    };
    use email_transport::{
        BoxFut, ErrorKind, IdempotencyKey, SendOptions, StructuredSendCapability, Transport,
    };
    use email_transport_test::conformance::{EXPECTED_ACCEPTED, conformance_message};
    use wasm_bindgen::{JsCast as _, JsValue};
    use worker::SendEmail;

    use super::payload::{EmailPayload, PayloadAddress, PayloadDisposition};
    use super::{CloudflareTransport, EmailSender, PROVIDER, SenderError};

    /// Records the payload handed to the binding and answers with a canned
    /// result.
    #[derive(Clone)]
    struct RecordingSender {
        payloads: Arc<Mutex<Vec<EmailPayload>>>,
        response: Result<String, SenderError>,
    }

    impl EmailSender for RecordingSender {
        fn send(&self, payload: EmailPayload) -> BoxFut<'_, Result<String, SenderError>> {
            self.payloads
                .lock()
                .expect("payload log should not be poisoned")
                .push(payload);
            let response = self.response.clone();
            Box::pin(async move { response })
        }
    }

    fn placeholder_binding() -> Arc<SendEmail> {
        // `JsValue::UNDEFINED` is a reserved constant: constructing and
        // dropping it never calls a wasm-bindgen intrinsic, so it is safe on
        // native targets.
        Arc::new(SendEmail::unchecked_from_js(JsValue::UNDEFINED))
    }

    fn transport_with(
        response: Result<String, SenderError>,
    ) -> (CloudflareTransport, RecordingSender) {
        let sender = RecordingSender {
            payloads: Arc::new(Mutex::new(Vec::new())),
            response,
        };
        let transport = CloudflareTransport::from_parts(placeholder_binding(), sender.clone());
        (transport, sender)
    }

    fn succeeding_transport() -> (CloudflareTransport, RecordingSender) {
        transport_with(Ok(String::from("<cf-123@example.com>")))
    }

    fn failing_transport(code: Option<&str>, message: &str) -> CloudflareTransport {
        transport_with(Err(SenderError::Binding {
            code: code.map(str::to_owned),
            message: message.to_owned(),
        }))
        .0
    }

    fn recorded(sender: &RecordingSender) -> Vec<EmailPayload> {
        sender
            .payloads
            .lock()
            .expect("payload log should not be poisoned")
            .clone()
    }

    fn only_payload(sender: &RecordingSender) -> EmailPayload {
        let payloads = recorded(sender);
        assert_eq!(payloads.len(), 1, "exactly one payload should be recorded");
        payloads.into_iter().next().expect("payload recorded")
    }

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

    async fn send_ok(message: &OutboundMessage) -> (EmailPayload, email_transport::SendReport) {
        let (transport, sender) = succeeding_transport();
        let report = transport
            .send(message, &SendOptions::default())
            .await
            .expect("send should succeed");
        (only_payload(&sender), report)
    }

    async fn send_err(message: &OutboundMessage) -> email_transport::TransportError {
        let (transport, sender) = succeeding_transport();
        let error = transport
            .send(message, &SendOptions::default())
            .await
            .expect_err("send should fail");
        assert!(
            recorded(&sender).is_empty(),
            "nothing should reach the binding when mapping fails"
        );
        error
    }

    // --- capabilities, debug, construction -------------------------------

    #[test]
    fn capabilities_match_binding_behavior() {
        let (transport, _) = succeeding_transport();
        let capabilities = transport.capabilities();

        assert_eq!(
            capabilities.structured_send,
            StructuredSendCapability::Supported
        );
        assert!(capabilities.custom_headers);
        assert!(capabilities.attachments);
        assert!(capabilities.inline_attachments);
        assert!(!capabilities.idempotency_key);
        assert!(!capabilities.timeout);
        assert!(!capabilities.custom_envelope);
        assert!(!capabilities.raw_rfc822);
        assert!(!capabilities.attachment_references);
    }

    #[test]
    fn debug_hides_binding_internals() {
        let (transport, _) = succeeding_transport();
        let rendered = format!("{transport:?}");

        assert_eq!(
            rendered,
            "CloudflareTransport { binding: \"<cloudflare send_email binding>\" }"
        );
    }

    #[test]
    fn provider_constant_is_stable() {
        assert_eq!(PROVIDER, "cloudflare");
    }

    #[tokio::test]
    async fn clones_share_the_same_sender() {
        let (transport, sender) = succeeding_transport();
        let clone = transport.clone();

        transport
            .send(&minimal_message(), &SendOptions::default())
            .await
            .expect("first send succeeds");
        clone
            .send(&minimal_message(), &SendOptions::default())
            .await
            .expect("second send succeeds");

        assert_eq!(recorded(&sender).len(), 2);
    }

    #[tokio::test]
    async fn native_send_against_real_binding_is_unsupported_not_a_panic() {
        let transport = CloudflareTransport::new(SendEmail::unchecked_from_js(JsValue::UNDEFINED));

        let error = transport
            .send(&minimal_message(), &SendOptions::default())
            .await
            .expect_err("binding is unavailable on native targets");

        assert_eq!(error.kind, ErrorKind::UnsupportedFeature);
        assert!(error.message.contains("wasm32-unknown-unknown"));
    }

    #[test]
    fn from_converts_binding() {
        let transport = CloudflareTransport::from(SendEmail::unchecked_from_js(JsValue::UNDEFINED));

        assert!(format!("{transport:?}").contains("<cloudflare send_email binding>"));
    }

    // --- report -----------------------------------------------------------

    #[tokio::test]
    async fn report_carries_provider_message_id_and_accepted_recipients() {
        let (_, report) = send_ok(&minimal_message()).await;

        assert_eq!(report.provider, PROVIDER);
        assert_eq!(
            report.provider_message_id.as_deref(),
            Some("<cf-123@example.com>")
        );
        assert_eq!(
            report
                .accepted
                .iter()
                .map(email_message::EmailAddress::as_str)
                .collect::<Vec<_>>(),
            vec!["recipient@example.com"]
        );
    }

    #[tokio::test]
    async fn idempotency_key_and_timeout_are_accepted_and_ignored() {
        let (transport, sender) = succeeding_transport();
        let options = SendOptions::new()
            .with_idempotency_key(IdempotencyKey::new("key-1").expect("key should validate"))
            .with_timeout(std::time::Duration::from_secs(5));

        transport
            .send(&minimal_message(), &options)
            .await
            .expect("options must not fail the send");
        transport
            .send(&minimal_message(), &SendOptions::default())
            .await
            .expect("default options succeed");

        let payloads = recorded(&sender);
        assert_eq!(payloads.len(), 2);
        assert_eq!(
            payloads[0], payloads[1],
            "options must not change the payload"
        );
    }

    // --- conformance ------------------------------------------------------

    #[tokio::test]
    async fn conforms_to_shared_semantics_and_drops_platform_headers() {
        let (payload, report) = send_ok(&conformance_message()).await;

        assert_eq!(
            report
                .accepted
                .iter()
                .map(email_message::EmailAddress::as_str)
                .collect::<Vec<_>>(),
            EXPECTED_ACCEPTED
        );
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

    #[tokio::test]
    async fn display_names_are_preserved_on_every_address_field() {
        let message = Message::builder(Body::text("Body"))
            .from_mailbox(mailbox("Sender <sender@example.com>"))
            .to(vec![Address::Mailbox(mailbox("To <to@example.com>"))])
            .cc(vec![Address::Mailbox(mailbox("Cc <cc@example.com>"))])
            .bcc(vec![Address::Mailbox(mailbox("Bcc <bcc@example.com>"))])
            .reply_to(vec![Address::Mailbox(mailbox("Reply <reply@example.com>"))])
            .build_outbound()
            .expect("message should validate");

        let (payload, _) = send_ok(&message).await;

        assert_eq!(payload.from, address(Some("Sender"), "sender@example.com"));
        assert_eq!(payload.to, vec![address(Some("To"), "to@example.com")]);
        assert_eq!(payload.cc, vec![address(Some("Cc"), "cc@example.com")]);
        assert_eq!(payload.bcc, vec![address(Some("Bcc"), "bcc@example.com")]);
        assert_eq!(
            payload.reply_to,
            Some(address(Some("Reply"), "reply@example.com"))
        );
    }

    #[tokio::test]
    async fn groups_are_flattened_to_member_mailboxes() {
        let message = Message::builder(Body::text("Body"))
            .from_mailbox(mailbox("sender@example.com"))
            .to(vec![
                "Friends: Ada <a@example.com>, b@example.com;"
                    .parse::<Address>()
                    .expect("valid group address"),
            ])
            .build_outbound()
            .expect("message should validate");

        let (payload, _) = send_ok(&message).await;

        assert_eq!(
            payload.to,
            vec![
                address(Some("Ada"), "a@example.com"),
                address(None, "b@example.com")
            ]
        );
    }

    #[tokio::test]
    async fn cc_only_message_is_sent() {
        let message = Message::builder(Body::text("Body"))
            .from_mailbox(mailbox("sender@example.com"))
            .cc(vec![Address::Mailbox(mailbox("cc@example.com"))])
            .build_outbound()
            .expect("message should validate");

        let (payload, report) = send_ok(&message).await;

        assert!(payload.to.is_empty());
        assert_eq!(payload.cc, vec![address(None, "cc@example.com")]);
        assert_eq!(report.accepted[0].as_str(), "cc@example.com");
    }

    #[tokio::test]
    async fn bcc_only_message_is_sent() {
        let message = Message::builder(Body::text("Body"))
            .from_mailbox(mailbox("sender@example.com"))
            .bcc(vec![Address::Mailbox(mailbox("hidden@example.com"))])
            .build_outbound()
            .expect("message should validate");

        let (payload, report) = send_ok(&message).await;

        assert!(payload.to.is_empty());
        assert_eq!(payload.bcc, vec![address(None, "hidden@example.com")]);
        assert_eq!(report.accepted[0].as_str(), "hidden@example.com");
    }

    #[tokio::test]
    async fn empty_group_with_no_other_recipients_fails_validation() {
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

        let error = send_err(&message).await;

        assert_eq!(error.kind, ErrorKind::Validation);
        assert!(error.message.contains("recipient"));
    }

    #[tokio::test]
    async fn multiple_reply_to_mailboxes_are_unsupported() {
        let message = Message::builder(Body::text("Body"))
            .from_mailbox(mailbox("sender@example.com"))
            .to(vec![Address::Mailbox(mailbox("recipient@example.com"))])
            .reply_to(vec![
                Address::Mailbox(mailbox("a@example.com")),
                Address::Mailbox(mailbox("b@example.com")),
            ])
            .build_outbound()
            .expect("message should validate");

        let error = send_err(&message).await;

        assert_eq!(error.kind, ErrorKind::UnsupportedFeature);
        assert!(error.message.contains("Reply-To"));
    }

    #[tokio::test]
    async fn absent_reply_to_is_omitted() {
        let (payload, _) = send_ok(&minimal_message()).await;

        assert_eq!(payload.reply_to, None);
    }

    // --- subject and body -------------------------------------------------

    #[tokio::test]
    async fn missing_subject_is_sent_as_empty_string() {
        let message = Message::builder(Body::text("Body"))
            .from_mailbox(mailbox("sender@example.com"))
            .to(vec![Address::Mailbox(mailbox("recipient@example.com"))])
            .build_outbound()
            .expect("message should validate");

        let (payload, _) = send_ok(&message).await;

        assert_eq!(payload.subject, "");
    }

    #[tokio::test]
    async fn text_body_maps_to_text() {
        let (payload, _) = send_ok(&message_with(Body::text("Plain"))).await;

        assert_eq!(payload.text.as_deref(), Some("Plain"));
        assert_eq!(payload.html, None);
    }

    #[tokio::test]
    async fn html_body_maps_to_html() {
        let (payload, _) = send_ok(&message_with(Body::html("<p>Rich</p>"))).await;

        assert_eq!(payload.text, None);
        assert_eq!(payload.html.as_deref(), Some("<p>Rich</p>"));
    }

    #[tokio::test]
    async fn text_and_html_body_maps_to_both() {
        let (payload, _) =
            send_ok(&message_with(Body::text_and_html("Plain", "<p>Rich</p>"))).await;

        assert_eq!(payload.text.as_deref(), Some("Plain"));
        assert_eq!(payload.html.as_deref(), Some("<p>Rich</p>"));
    }

    #[tokio::test]
    async fn empty_text_in_text_and_html_is_dropped() {
        let (payload, _) = send_ok(&message_with(Body::text_and_html("", "<p>Rich</p>"))).await;

        assert_eq!(payload.text, None);
        assert_eq!(payload.html.as_deref(), Some("<p>Rich</p>"));
    }

    #[tokio::test]
    async fn empty_body_fails_validation() {
        let error = send_err(&message_with(Body::text(""))).await;

        assert_eq!(error.kind, ErrorKind::Validation);
        assert!(error.message.contains("body"));
    }

    #[tokio::test]
    async fn mime_body_is_unsupported() {
        let body = Body::Mime(MimePart::Leaf {
            content_type: ContentType::try_from("text/plain").expect("content type parses"),
            content_transfer_encoding: None,
            content_disposition: None,
            body: b"Body".to_vec(),
        });

        let error = send_err(&message_with(body)).await;

        assert_eq!(error.kind, ErrorKind::UnsupportedFeature);
    }

    // --- headers ----------------------------------------------------------

    #[tokio::test]
    async fn custom_headers_are_forwarded() {
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

        let (payload, _) = send_ok(&message).await;

        assert_eq!(payload.headers.len(), 3);
        assert_eq!(payload.headers["X-Campaign"], "spring");
        assert_eq!(
            payload.headers["List-Unsubscribe"],
            "<mailto:unsub@example.com>"
        );
        assert_eq!(payload.headers["In-Reply-To"], "<parent@example.com>");
    }

    #[tokio::test]
    async fn date_message_id_and_sender_are_never_emitted() {
        let message = Message::builder(Body::text("Body"))
            .from_mailbox(mailbox("sender@example.com"))
            .sender(mailbox("bounce@example.com"))
            .to(vec![Address::Mailbox(mailbox("recipient@example.com"))])
            .date(time::OffsetDateTime::UNIX_EPOCH)
            .message_id("<mine@example.com>".parse().expect("message id parses"))
            .build_outbound()
            .expect("message should validate");

        let (payload, _) = send_ok(&message).await;

        assert!(payload.headers.is_empty(), "headers: {:?}", payload.headers);
    }

    // --- attachments ------------------------------------------------------

    #[tokio::test]
    async fn byte_attachment_carries_metadata_and_binary_content() {
        let content = b"hello\xff\0world";
        let message = message_with_attachment(
            Attachment::bytes(
                ContentType::try_from("application/octet-stream").expect("content type parses"),
                content.to_vec(),
            )
            .with_filename("report.bin"),
        );

        let (payload, _) = send_ok(&message).await;

        assert_eq!(payload.attachments.len(), 1);
        let attachment = &payload.attachments[0];
        assert_eq!(attachment.filename, "report.bin");
        assert_eq!(attachment.content_type, "application/octet-stream");
        assert_eq!(attachment.disposition, PayloadDisposition::Attachment);
        assert_eq!(attachment.content, content);
    }

    #[tokio::test]
    async fn inline_attachment_carries_content_id_and_disposition() {
        let message = message_with_attachment(
            Attachment::bytes(
                ContentType::try_from("image/png").expect("content type parses"),
                vec![0x89, b'P', b'N', b'G'],
            )
            .with_filename("logo.png")
            .with_content_id("logo")
            .with_disposition(Disposition::Inline),
        );

        let (payload, _) = send_ok(&message).await;

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

    #[tokio::test]
    async fn regular_attachment_without_filename_is_named_by_position() {
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

        let (payload, _) = send_ok(&message).await;

        let filenames: Vec<&str> = payload
            .attachments
            .iter()
            .map(|attachment| attachment.filename.as_str())
            .collect();
        assert_eq!(filenames, vec!["attachment-1", "named.pdf", "attachment-3"]);
    }

    #[tokio::test]
    async fn inline_attachment_without_filename_is_named_by_content_id() {
        let message = message_with_attachment(
            Attachment::bytes(
                ContentType::try_from("image/png").expect("content type parses"),
                vec![0x89, b'P', b'N', b'G'],
            )
            .with_content_id("logo@example.com")
            .with_disposition(Disposition::Inline),
        );

        let (payload, _) = send_ok(&message).await;

        assert_eq!(payload.attachments[0].filename, "logo@example.com");
    }

    #[tokio::test]
    async fn inline_attachment_without_content_id_fails_validation() {
        let message = message_with_attachment(
            Attachment::bytes(
                ContentType::try_from("image/png").expect("content type parses"),
                vec![0x89, b'P', b'N', b'G'],
            )
            .with_filename("logo.png")
            .with_disposition(Disposition::Inline),
        );

        let error = send_err(&message).await;

        assert_eq!(error.kind, ErrorKind::Validation);
        assert!(error.message.contains("content id"));
    }

    #[tokio::test]
    async fn content_id_on_regular_attachment_is_dropped() {
        let message = message_with_attachment(
            Attachment::bytes(
                ContentType::try_from("application/pdf").expect("content type parses"),
                b"%PDF".to_vec(),
            )
            .with_filename("report.pdf")
            .with_content_id("report"),
        );

        let (payload, _) = send_ok(&message).await;

        assert_eq!(
            payload.attachments[0].disposition,
            PayloadDisposition::Attachment
        );
    }

    #[tokio::test]
    async fn attachment_reference_is_unsupported_with_preparation_hint() {
        let message = message_with_attachment(Attachment::reference(
            ContentType::try_from("application/pdf").expect("content type parses"),
            AttachmentReference::new("s3://bucket/key"),
        ));

        let error = send_err(&message).await;

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

    // --- error classification ----------------------------------------------

    #[tokio::test]
    async fn binding_errors_are_classified_by_code() {
        let cases: &[(&str, ErrorKind, bool)] = &[
            ("E_VALIDATION_ERROR", ErrorKind::Validation, false),
            ("E_FIELD_MISSING", ErrorKind::Validation, false),
            ("E_TOO_MANY_RECIPIENTS", ErrorKind::Validation, false),
            ("E_TOO_MANY_ATTACHMENTS", ErrorKind::Validation, false),
            ("E_CONTENT_TOO_LARGE", ErrorKind::Validation, false),
            ("E_HEADER_NOT_ALLOWED", ErrorKind::Validation, false),
            ("E_HEADER_USE_API_FIELD", ErrorKind::Validation, false),
            ("E_HEADER_VALUE_INVALID", ErrorKind::Validation, false),
            ("E_HEADER_VALUE_TOO_LONG", ErrorKind::Validation, false),
            ("E_HEADER_NAME_INVALID", ErrorKind::Validation, false),
            ("E_HEADERS_TOO_LARGE", ErrorKind::Validation, false),
            ("E_HEADERS_TOO_MANY", ErrorKind::Validation, false),
            ("E_SENDER_NOT_VERIFIED", ErrorKind::Authorization, false),
            (
                "E_SENDER_DOMAIN_NOT_AVAILABLE",
                ErrorKind::Authorization,
                false,
            ),
            ("E_RECIPIENT_NOT_ALLOWED", ErrorKind::Authorization, false),
            ("RCPT_NOT_ALLOWED", ErrorKind::Authorization, false),
            (
                "E_RECIPIENT_SUPPRESSED",
                ErrorKind::PermanentProvider,
                false,
            ),
            ("E_RATE_LIMIT_EXCEEDED", ErrorKind::RateLimited, true),
            ("E_DAILY_LIMIT_EXCEEDED", ErrorKind::RateLimited, true),
            (
                "E_INTERNAL_SERVER_ERROR",
                ErrorKind::TransientProvider,
                true,
            ),
            ("E_DELIVERY_FAILED", ErrorKind::TransientProvider, true),
            ("E_SOMETHING_NEW", ErrorKind::PermanentProvider, false),
        ];

        for (code, kind, retryable) in cases {
            let transport = failing_transport(Some(code), "platform says no");
            let error = transport
                .send(&minimal_message(), &SendOptions::default())
                .await
                .expect_err("binding failure should surface");

            assert_eq!(error.kind, *kind, "{code}: kind");
            assert_eq!(error.is_retryable(), *retryable, "{code}: retryable");
            assert_eq!(
                error.provider_error_code.as_deref(),
                Some(*code),
                "{code}: code"
            );
            assert_eq!(error.message, "platform says no", "{code}: message");
            assert_eq!(error.http_status, None, "{code}: no http hop");
        }
    }

    #[tokio::test]
    async fn binding_error_without_code_is_internal() {
        let transport = failing_transport(None, "TypeError: boom");

        let error = transport
            .send(&minimal_message(), &SendOptions::default())
            .await
            .expect_err("binding failure should surface");

        assert_eq!(error.kind, ErrorKind::Internal);
        assert_eq!(error.provider_error_code, None);
        assert_eq!(error.message, "TypeError: boom");
        assert!(!error.is_retryable());
    }
}
