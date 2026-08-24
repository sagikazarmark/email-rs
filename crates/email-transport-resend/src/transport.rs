mod payload;

use std::future::Future;

use email_message::OutboundMessage;
use email_transport::{
    BoxFut, Capabilities, ErrorKind, MaybeSend, SendOptions, SendReport, StructuredSendCapability,
    Transport, TransportError, structured_accepted_for,
};
use resend_rs::types::CreateEmailBaseOptions;
use resend_rs::{ConfigBuilder, Resend};
use url::Url;

/// Structured email transport backed by the official `resend-rs` client.
///
/// The transport maps message bodies, recipients, standard/custom headers,
/// byte-backed attachments, tags, and templates to Resend's structured send
/// endpoint. Its [`Debug`](std::fmt::Debug) implementation redacts client
/// internals so API keys are not exposed.
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

    /// Construct a transport from an initialized `resend-rs` client.
    ///
    /// Use this when SDK configuration outside [`ResendTransportBuilder`] is
    /// required.
    #[must_use]
    pub const fn from_client(client: Resend) -> Self {
        Self { client }
    }

    /// Return the underlying `resend-rs` client.
    pub const fn client(&self) -> &Resend {
        &self.client
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

impl From<Resend> for ResendTransport {
    fn from(client: Resend) -> Self {
        Self::from_client(client)
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
        let payload = match payload::build_email_options(message, options) {
            Ok(payload) => payload,
            Err(error) => return failed(error),
        };

        let accepted = structured_accepted_for(message, options, self.capabilities());

        let idempotency_key = options.idempotency_key.clone();

        self.send_payload(payload, accepted, idempotency_key, options.timeout)
    }
}

/// Builder for [`ResendTransport`] with optional Resend SDK configuration.
///
/// API keys are redacted by its [`Debug`](std::fmt::Debug) implementation.
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
            } else if error.is_builder() {
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

fn transport_error(kind: ErrorKind, message: impl Into<String>) -> TransportError {
    TransportError::new(kind, message)
}

#[cfg(test)]
mod tests {
    use email_message::{Address, Body, Mailbox, Message, OutboundMessage};
    use email_transport::{ErrorKind, SendOptions, Transport};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::ResendTransport;

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
    fn from_converts_initialized_resend_client() {
        let base_url = url::Url::parse("https://api.test/").expect("base url parses");
        let config = resend_rs::ConfigBuilder::new("k")
            .base_url(base_url)
            .build();
        let transport = ResendTransport::from(resend_rs::Resend::with_config(config));

        assert_eq!(transport.client().base_url(), "https://api.test/");
    }

    #[tokio::test]
    async fn connection_failure_is_transient() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("port should bind");
        let address = listener.local_addr().expect("address should be available");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("connection should arrive");
            drop(stream);
        });
        let transport = ResendTransport::builder("invalid")
            .base_url(
                format!("http://{address}/")
                    .parse()
                    .expect("base URL should parse"),
            )
            .build();
        let options = SendOptions::new().with_timeout(std::time::Duration::from_secs(1));
        let result = transport
            .send(&minimal_message(), &options)
            .await
            .expect_err("connection should fail");
        server.await.expect("server task should finish");

        assert_eq!(result.kind, ErrorKind::TransientNetwork);
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
