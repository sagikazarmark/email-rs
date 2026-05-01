use std::collections::{BTreeMap, HashMap};
use std::future::Future;

use email_message::{Address, Attachment, AttachmentBody, Body, Message, OutboundMessage};
use email_transport::{
    BoxFut, Capabilities, ErrorKind, MaybeSend, SendOptions, SendReport, StructuredSendCapability,
    Transport, TransportError, standard_message_headers, structured_accepted_for,
};
use resend_rs::types::{CreateAttachment, CreateEmailBaseOptions, EmailTemplate, Tag};
use resend_rs::{ConfigBuilder, Resend};
use url::Url;

use crate::{ResendSendOptions, ResendTag, ResendTemplate};

#[derive(Clone)]
pub struct ResendTransport {
    client: Resend,
}

/// Hand-written `Debug` so SDK internals never leak through accidental
/// `format!("{:?}", transport)` paths in user code or logs. The base URL is
/// safe to print, while the SDK client is rendered as a redacted placeholder.
impl std::fmt::Debug for ResendTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResendTransport")
            .field("client", &"<redacted resend_rs::Resend>")
            .field("base_url", &self.client.base_url())
            .finish()
    }
}

impl ResendTransport {
    /// Builds a `ResendTransport` against the default Resend base URL using the
    /// official `resend-rs` client.
    #[must_use]
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::builder(api_key).build()
    }

    /// Starts configuring a `ResendTransport` from an API key.
    #[must_use]
    pub fn builder(api_key: impl Into<String>) -> ResendTransportBuilder {
        ResendTransportBuilder::new(api_key)
    }

    /// Constructs a `ResendTransport` from an initialized `resend-rs` client.
    #[must_use]
    pub const fn from_client(client: Resend) -> Self {
        Self { client }
    }

    /// Returns the underlying `resend-rs` client.
    pub const fn client(&self) -> &Resend {
        &self.client
    }

    fn build_email_options(
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

    fn send_payload(
        &self,
        payload: CreateEmailBaseOptions,
        accepted: Vec<email_message::EmailAddress>,
        idempotency_key: Option<email_transport::IdempotencyKey>,
        timeout: Option<std::time::Duration>,
    ) -> BoxFut<'_, Result<SendReport, TransportError>> {
        let client = self.client.clone();
        Box::pin(async move {
            let send = async move {
                let response = if let Some(key) = idempotency_key.as_ref() {
                    client
                        .emails
                        .send(payload.with_idempotency_key(key.as_str()))
                        .await
                } else {
                    client.emails.send(payload).await
                }
                .map_err(map_resend_error)?;

                Ok(SendReport::new("resend")
                    .with_provider_message_id(response.id.to_string())
                    .with_accepted(accepted))
            };

            maybe_timeout(send, timeout).await
        })
    }
}

impl Transport for ResendTransport {
    fn capabilities(&self) -> Capabilities {
        Capabilities::new()
            .with_structured_send(StructuredSendCapability::Supported)
            .with_custom_headers(true)
            .with_attachments(true)
            .with_inline_attachments(true)
            .with_idempotency_key(true)
            .with_timeout(cfg!(not(target_arch = "wasm32")))
    }

    fn send<'a>(
        &'a self,
        message: &'a OutboundMessage,
        options: &'a SendOptions,
    ) -> impl core::future::Future<Output = Result<SendReport, TransportError>> + MaybeSend + 'a
    {
        let message = message.as_message();
        let payload = match Self::build_email_options(message, options) {
            Ok(payload) => payload,
            Err(error) => return failed(error),
        };

        let accepted = structured_accepted_for(message, options, self.capabilities());

        let idempotency_key = options.idempotency_key.clone();

        self.send_payload(payload, accepted, idempotency_key, options.timeout)
    }
}

/// Builder for [`ResendTransport`] with optional Resend SDK configuration.
#[derive(Clone)]
pub struct ResendTransportBuilder {
    api_key: String,
    base_url: Option<Url>,
    client: Option<reqwest::Client>,
}

impl std::fmt::Debug for ResendTransportBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResendTransportBuilder")
            .field("api_key", &"<redacted>")
            .field("base_url", &self.base_url)
            .field("client", &self.client.as_ref().map(|_| "<reqwest::Client>"))
            .finish()
    }
}

impl ResendTransportBuilder {
    /// Starts configuring a `ResendTransport` from an API key.
    #[must_use]
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: None,
            client: None,
        }
    }

    /// Sets a custom Resend SDK base URL.
    ///
    /// This is intended for test servers, proxies, and Resend-compatible
    /// endpoints. `resend-rs` joins endpoint paths from the URL origin; pass an
    /// origin-style URL such as `http://127.0.0.1:8080`.
    #[must_use]
    pub fn base_url(mut self, base_url: Url) -> Self {
        self.base_url = Some(base_url);
        self
    }

    /// Sets the reqwest HTTP client used by the underlying `resend-rs` client.
    #[must_use]
    pub fn client(mut self, client: reqwest::Client) -> Self {
        self.client = Some(client);
        self
    }

    /// Builds the configured `ResendTransport`.
    #[must_use]
    pub fn build(self) -> ResendTransport {
        let mut config = ConfigBuilder::new(self.api_key);

        if let Some(base_url) = self.base_url {
            config = config.base_url(base_url);
        }

        if let Some(client) = self.client {
            config = config.client(client);
        }

        ResendTransport::from_client(Resend::with_config(config.build()))
    }
}

fn failed<'a>(error: TransportError) -> BoxFut<'a, Result<SendReport, TransportError>> {
    Box::pin(async move { Err(error) })
}

fn map_body(body: &Body) -> Result<(Option<String>, Option<String>), TransportError> {
    match body {
        Body::Text(text) => Ok((non_empty(text), None)),
        Body::Html(html) => Ok((None, non_empty(html))),
        Body::TextAndHtml { text, html } => Ok((non_empty(text), non_empty(html))),
        #[allow(unreachable_patterns)]
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
             resolve references via `email_attachment::resolve_message_attachments` before send",
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

fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

#[cfg(not(target_arch = "wasm32"))]
async fn maybe_timeout<F, T>(
    future: F,
    timeout: Option<std::time::Duration>,
) -> Result<T, TransportError>
where
    F: Future<Output = Result<T, TransportError>> + Send,
    T: Send,
{
    if let Some(timeout) = timeout {
        tokio::time::timeout(timeout, future)
            .await
            .map_err(|error| {
                transport_error(ErrorKind::Timeout, "resend request timed out").with_source(error)
            })?
    } else {
        future.await
    }
}

#[cfg(target_arch = "wasm32")]
async fn maybe_timeout<F, T>(
    future: F,
    _timeout: Option<std::time::Duration>,
) -> Result<T, TransportError>
where
    F: Future<Output = Result<T, TransportError>>,
{
    future.await
}

fn map_resend_error(error: resend_rs::Error) -> TransportError {
    let mapped = match &error {
        resend_rs::Error::Http(error) => {
            let kind = if error.is_timeout() {
                ErrorKind::Timeout
            } else if error.is_builder() || error.is_request() {
                ErrorKind::Validation
            } else {
                ErrorKind::TransientNetwork
            };
            transport_error(kind, error.to_string())
        }
        resend_rs::Error::Resend(response) => {
            // `resend-rs` exposes the status from Resend's JSON body, not
            // the wire status. Do not surface impossible success-class
            // statuses as `TransportError::http_status`.
            let mut provider_error = if response.status_code >= 400 {
                transport_error(
                    ErrorKind::from_http_status(response.status_code),
                    response.message.clone(),
                )
                .with_http_status(response.status_code)
            } else {
                transport_error(ErrorKind::TransientProvider, response.message.clone())
            };
            provider_error = provider_error.with_provider_error_code(response.name.clone());
            provider_error
        }
        resend_rs::Error::Parse { message, .. } => {
            transport_error(ErrorKind::TransientProvider, message.clone())
        }
        resend_rs::Error::Other(message) => transport_error(ErrorKind::Internal, message.clone()),
        resend_rs::Error::RateLimit {
            ratelimit_reset, ..
        } => {
            let mut error = transport_error(ErrorKind::RateLimited, "resend rate limit exceeded")
                .with_http_status(429)
                .with_provider_error_code("rate_limit_exceeded");
            if let Some(reset) = ratelimit_reset {
                error = error.with_retry_after(std::time::Duration::from_secs(*reset));
            }
            error
        }
    };

    mapped.with_source(error)
}

fn collect_mailboxes(addresses: &[Address]) -> Vec<String> {
    addresses
        .iter()
        .flat_map(Address::mailboxes)
        .map(ToString::to_string)
        .collect()
}

fn transport_error(kind: ErrorKind, message: impl Into<String>) -> TransportError {
    TransportError::new(kind, message)
}

#[cfg(test)]
mod tests {
    use email_message::{Address, Body, Mailbox, Message, OutboundMessage};
    use email_transport::{ErrorKind, SendOptions, Transport, TransportOptions};
    use serde_json::Value;
    use time::OffsetDateTime;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::ResendTransport;
    use crate::{ResendSendOptions, ResendTemplate};

    fn mailbox(input: &str) -> Mailbox {
        input.parse().expect("valid mailbox fixture")
    }

    fn minimal_message() -> OutboundMessage {
        Message::builder(Body::Text(String::from("Body")))
            .from_mailbox(mailbox("sender@example.com"))
            .to(vec![Address::Mailbox(mailbox("recipient@example.com"))])
            .subject("Hello")
            .build_outbound()
            .expect("minimal message should validate")
    }

    #[test]
    fn debug_redacts_client_internals() {
        let url = url::Url::parse("https://api.test/v1/").expect("base url parses");
        let transport = ResendTransport::builder("super-secret-key")
            .base_url(url)
            .build();
        let rendered = format!("{transport:?}");
        assert!(
            !rendered.contains("super-secret-key"),
            "api_key leaked: {rendered}"
        );
        assert!(
            rendered.contains("<redacted resend_rs::Resend>"),
            "redaction marker missing: {rendered}"
        );
        assert!(
            !rendered.contains("api_key"),
            "unexpected field: {rendered}"
        );
        assert!(rendered.contains("api.test"));
    }

    #[test]
    fn builder_accepts_origin_without_trailing_slash() {
        let url = url::Url::parse("https://api.test/v1").expect("base url parses");
        let transport = ResendTransport::builder("k").base_url(url).build();
        assert_eq!(transport.client().base_url(), "https://api.test/v1");
    }

    #[test]
    fn builder_accepts_path_with_trailing_slash() {
        let url = url::Url::parse("https://api.test/v1/").expect("base url parses");
        let transport = ResendTransport::builder("k").base_url(url).build();
        assert_eq!(transport.client().base_url(), "https://api.test/v1/");
    }

    #[test]
    fn builder_accepts_reqwest_client() {
        let client = reqwest::Client::builder()
            .build()
            .expect("reqwest client should build");
        let transport = ResendTransport::builder("k").client(client).build();

        assert_eq!(transport.client().base_url(), "https://api.resend.com/");
    }

    #[test]
    fn from_client_accepts_initialized_resend_client() {
        let base_url = url::Url::parse("https://api.test/").expect("base url parses");
        let config = resend_rs::ConfigBuilder::new("k")
            .base_url(base_url)
            .build();
        let client = resend_rs::Resend::with_config(config);
        let transport = ResendTransport::from_client(client);

        assert_eq!(transport.client().base_url(), "https://api.test/");
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

        let built =
            ResendTransport::build_email_options(message.as_message(), &SendOptions::default())
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

        let error = ResendTransport::build_email_options(&message, &SendOptions::default())
            .expect_err("missing recipients should be rejected");

        assert_eq!(error.kind, ErrorKind::Validation);
    }

    #[test]
    fn build_payload_accepts_template_without_body() {
        let message = Message::builder(Body::Html(String::new()))
            .from_mailbox(mailbox("sender@example.com"))
            .to(vec![Address::Mailbox(mailbox("recipient@example.com"))])
            .subject("Hello")
            .build_outbound()
            .expect("message should validate");

        let mut transport_options = TransportOptions::default();
        transport_options
            .insert(ResendSendOptions::new().with_template(ResendTemplate::new("tmpl_123")));

        let options = SendOptions::new().with_transport_options(transport_options);

        let built = ResendTransport::build_email_options(message.as_message(), &options);
        assert!(built.is_ok(), "template-only payload should be accepted");
    }

    #[test]
    fn build_payload_maps_typed_options() {
        let message = minimal_message();

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

        let built = ResendTransport::build_email_options(message.as_message(), &options)
            .expect("payload should build");
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

        let built =
            ResendTransport::build_email_options(message.as_message(), &SendOptions::default())
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

    #[tokio::test]
    async fn send_without_base_url_or_key_will_fail_network() {
        let transport = ResendTransport::builder("invalid")
            .base_url(
                "http://127.0.0.1:9/"
                    .parse()
                    .expect("base URL should parse"),
            )
            .build();
        let result = transport
            .send(&minimal_message(), &SendOptions::default())
            .await;

        assert!(result.is_err(), "invalid key call should fail in test env");
    }

    #[tokio::test]
    async fn send_with_base_url_reports_provider_message_id() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/emails"))
            .and(header("authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "re_123"
            })))
            .mount(&server)
            .await;

        let transport = ResendTransport::builder("test-key")
            .base_url(
                format!("{}/", server.uri())
                    .parse()
                    .expect("base URL should parse"),
            )
            .build();

        let report = transport
            .send(&minimal_message(), &SendOptions::default())
            .await
            .expect("send should succeed");

        assert_eq!(report.provider, "resend");
        assert_eq!(report.provider_message_id.as_deref(), Some("re_123"));
        let accepted_strs: Vec<&str> = report
            .accepted
            .iter()
            .map(email_message::EmailAddress::as_str)
            .collect();
        assert_eq!(accepted_strs, vec!["recipient@example.com"]);
    }

    #[tokio::test]
    async fn send_maps_authentication_status() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/emails"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
                "statusCode": 401,
                "name": "missing_api_key",
                "message": "Missing API key"
            })))
            .mount(&server)
            .await;

        let transport = ResendTransport::builder("test-key")
            .base_url(
                format!("{}/", server.uri())
                    .parse()
                    .expect("base URL should parse"),
            )
            .build();

        let error = transport
            .send(&minimal_message(), &SendOptions::default())
            .await
            .expect_err("authentication error should bubble up");

        assert_eq!(error.kind, ErrorKind::Authentication);
    }

    #[tokio::test]
    async fn send_with_timeout_maps_to_timeout() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/emails"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(std::time::Duration::from_millis(200))
                    .set_body_json(serde_json::json!({
                        "id": "re_123"
                    })),
            )
            .mount(&server)
            .await;

        let transport = ResendTransport::builder("test-key")
            .base_url(
                format!("{}/", server.uri())
                    .parse()
                    .expect("base URL should parse"),
            )
            .build();

        let error = transport
            .send(
                &minimal_message(),
                &SendOptions::new().with_timeout(std::time::Duration::from_millis(50)),
            )
            .await
            .expect_err("timeout should bubble up");

        assert_eq!(error.kind, ErrorKind::Timeout);
    }

    #[test]
    fn map_resend_error_classifies_rate_limit() {
        let error = super::map_resend_error(resend_rs::Error::RateLimit {
            ratelimit_limit: Some(10),
            ratelimit_remaining: Some(0),
            ratelimit_reset: Some(42),
        });

        assert_eq!(error.kind, ErrorKind::RateLimited);
        assert_eq!(error.http_status, Some(429));
        assert_eq!(error.retry_after, Some(std::time::Duration::from_secs(42)));
        assert_eq!(
            error.provider_error_code.as_deref(),
            Some("rate_limit_exceeded")
        );
    }

    #[test]
    fn map_resend_error_does_not_surface_success_status_from_error_body() {
        let error =
            super::map_resend_error(resend_rs::Error::Resend(resend_rs::types::ErrorResponse {
                status_code: 200,
                name: String::from("internal_error"),
                message: String::from("provider misbehavior"),
            }));

        assert_eq!(error.kind, ErrorKind::TransientProvider);
        assert_eq!(error.http_status, None);
        assert_eq!(error.provider_error_code.as_deref(), Some("internal_error"));
    }
}
