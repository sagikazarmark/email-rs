//! Caller-side transport that invokes the email worker through Restate ingress.

use std::time::Duration;

use email_message::OutboundMessage;
use email_transport::{
    Capabilities, ErrorKind, MaybeSend, SendOptions, SendReport, StructuredSendCapability,
    Transport, TransportError, structured_accepted_for,
};
use restate_email::{InvocationMode, RestateSendOptions, SendResponse, TransportKey};
use serde::{Deserialize, Serialize};

const ERROR_SOURCE_HEADER: &str = "x-restate-error-source";
const SERVICE_PATH: [&str; 2] = ["Email", "send"];

/// Email transport backed by a Restate `Email.send` ingress invocation.
///
/// The transport follows a send as far as its [`InvocationMode`] says: in the
/// default [`InvocationMode::Queued`] mode it returns once Restate has durably
/// accepted the invocation and reports the invocation id; in
/// [`InvocationMode::Sent`] mode it waits for the worker and returns the
/// worker's provider report. A per-send [`RestateSendOptions`] overrides the
/// configured mode and may delay the invocation.
///
/// # Capabilities
///
/// The worker's capabilities cannot be discovered through ingress. The
/// defaults describe the ingress hop itself: structured sends are the only
/// thing `Email.send` accepts, and `SendOptions::idempotency_key` is honored
/// at both hops in both modes: it is sent as Restate's `idempotency-key`
/// header so Restate deduplicates caller retries, and it stays in the queued
/// payload so the worker's transport can deduplicate provider retries.
/// Everything about the worker (attachment references, custom envelopes, and
/// so on) is a deployment assertion made through [`RestateTransportBuilder`].
///
/// # Authentication
///
/// [`RestateTransportBuilder::bearer_token`] attaches an
/// `Authorization: Bearer` header to every ingress request, as required by
/// Restate Cloud. The token is redacted from `Debug` output.
///
/// # Cancellation
///
/// A request may be accepted by Restate even when this side observes an error
/// while reading the response. This is the trait-level cancellation caveat;
/// use `SendOptions::idempotency_key` so that a retry attaches to the same
/// invocation instead of enqueueing a second one.
#[derive(Clone)]
pub struct RestateTransport {
    client: reqwest::Client,
    ingress_url: reqwest::Url,
    call_url: reqwest::Url,
    send_url: reqwest::Url,
    transport: TransportKey,
    invocation_mode: InvocationMode,
    bearer_token: Option<String>,
    capabilities: Capabilities,
}

impl RestateTransport {
    /// `SendReport::provider` reported for sends accepted by Restate in
    /// [`InvocationMode::Queued`] mode.
    pub const PROVIDER: &'static str = "restate";

    /// Create a transport with default settings.
    ///
    /// Equivalent to `RestateTransport::builder(transport, ingress_url).build()`.
    ///
    /// # Panics
    ///
    /// Panics if `ingress_url` cannot carry hierarchical path segments (for
    /// example a `mailto:` or `data:` URL). Such a URL can never address a
    /// Restate ingress, so this is a programming error rather than an input
    /// error.
    #[must_use]
    pub fn new(transport: TransportKey, ingress_url: reqwest::Url) -> Self {
        Self::builder(transport, ingress_url).build()
    }

    /// Start configuring a transport for `transport` behind `ingress_url`.
    ///
    /// Any query and fragment on `ingress_url` are discarded; a path prefix is
    /// preserved so ingress can sit behind a reverse proxy.
    ///
    /// # Panics
    ///
    /// Panics if `ingress_url` cannot carry hierarchical path segments (for
    /// example a `mailto:` or `data:` URL). Such a URL can never address a
    /// Restate ingress, so this is a programming error rather than an input
    /// error.
    #[must_use]
    pub fn builder(transport: TransportKey, ingress_url: reqwest::Url) -> RestateTransportBuilder {
        RestateTransportBuilder::new(transport, ingress_url)
    }

    /// Return the configured transport key.
    #[must_use]
    pub const fn transport_key(&self) -> &TransportKey {
        &self.transport
    }

    /// Return the normalized ingress base URL.
    #[must_use]
    pub const fn ingress_url(&self) -> &reqwest::Url {
        &self.ingress_url
    }

    /// Return the invocation mode used when a send carries no override.
    #[must_use]
    pub const fn invocation_mode(&self) -> InvocationMode {
        self.invocation_mode
    }

    /// Return the HTTP client used for ingress requests.
    #[must_use]
    pub const fn client(&self) -> &reqwest::Client {
        &self.client
    }

    /// Return the Restate invocation id carried by a queued report.
    ///
    /// Reports produced in [`InvocationMode::Queued`] mode name
    /// [`Self::PROVIDER`] as the provider and the invocation id as the
    /// provider message id. The id can be used with Restate's
    /// `/restate/attach/{id}` and `/restate/output/{id}` endpoints. Returns
    /// `None` for reports produced by any other provider.
    #[must_use]
    pub fn invocation_id(report: &SendReport) -> Option<&str> {
        if report.provider != Self::PROVIDER {
            return None;
        }
        report.provider_message_id.as_deref()
    }

    async fn invoke(
        &self,
        message: &OutboundMessage,
        options: &SendOptions,
    ) -> Result<SendReport, TransportError> {
        let restate_options = options.transport_options.get::<RestateSendOptions>();
        let invocation_mode = restate_options
            .and_then(|restate| restate.invocation_mode)
            .unwrap_or(self.invocation_mode);
        let delay = restate_options.and_then(|restate| restate.delay);
        let url = self.endpoint_for(invocation_mode, delay)?;

        let request = IngressSendRequest {
            transport: &self.transport,
            message,
            options,
        };
        let body = serde_json::to_vec(&request).map_err(|error| {
            TransportError::new(
                ErrorKind::Internal,
                "failed to serialize Restate send options",
            )
            .with_source(error)
        })?;
        let mut request_builder = self
            .client
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body);
        if let Some(bearer_token) = self.bearer_token.as_deref() {
            request_builder = request_builder.bearer_auth(bearer_token);
        }
        if let Some(idempotency_key) = options.idempotency_key.as_ref() {
            request_builder = request_builder.header("idempotency-key", idempotency_key.as_str());
        }

        let response = request_builder.send().await.map_err(map_reqwest_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(map_error_response(response).await);
        }

        match invocation_mode {
            InvocationMode::Queued => {
                let accepted = response
                    .json::<IngressSendAccepted>()
                    .await
                    .map_err(map_response_decode_error)?;

                Ok(SendReport::new(Self::PROVIDER)
                    .with_provider_message_id(accepted.invocation_id)
                    .with_accepted(structured_accepted_for(
                        message.as_message(),
                        options,
                        self.capabilities,
                    )))
            }
            InvocationMode::Sent => response
                .json::<SendResponse>()
                .await
                .map(|response| response.report)
                .map_err(map_response_decode_error),
        }
    }

    fn endpoint_for(
        &self,
        invocation_mode: InvocationMode,
        delay: Option<Duration>,
    ) -> Result<reqwest::Url, TransportError> {
        match invocation_mode {
            InvocationMode::Queued => {
                let mut url = self.send_url.clone();
                if let Some(delay) = delay {
                    url.query_pairs_mut()
                        .append_pair("delay", &format_delay(delay));
                }
                Ok(url)
            }
            InvocationMode::Sent => {
                if delay.is_some() {
                    return Err(TransportError::new(
                        ErrorKind::Validation,
                        "Restate delay requires InvocationMode::Queued",
                    ));
                }
                Ok(self.call_url.clone())
            }
        }
    }
}

impl std::fmt::Debug for RestateTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RestateTransport")
            .field("client", &"<reqwest::Client>")
            .field("ingress_url", &redacted_url(&self.ingress_url))
            .field("transport", &self.transport)
            .field("invocation_mode", &self.invocation_mode)
            .field(
                "bearer_token",
                &self.bearer_token.as_ref().map(|_| "<redacted>"),
            )
            .field("capabilities", &self.capabilities)
            .finish_non_exhaustive()
    }
}

impl Transport for RestateTransport {
    fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    fn send<'a>(
        &'a self,
        message: &'a OutboundMessage,
        options: &'a SendOptions,
    ) -> impl core::future::Future<Output = Result<SendReport, TransportError>> + MaybeSend + 'a
    {
        self.invoke(message, options)
    }
}

/// Builder for [`RestateTransport`].
///
/// Created through [`RestateTransport::builder`]. Every setter has a default,
/// so `build` never fails.
#[derive(Clone)]
pub struct RestateTransportBuilder {
    transport: TransportKey,
    ingress_url: reqwest::Url,
    client: Option<reqwest::Client>,
    invocation_mode: InvocationMode,
    bearer_token: Option<String>,
    capabilities: Capabilities,
}

impl RestateTransportBuilder {
    /// Start configuring a transport for `transport` behind `ingress_url`.
    ///
    /// # Panics
    ///
    /// Panics if `ingress_url` cannot carry hierarchical path segments; see
    /// [`RestateTransport::builder`].
    #[must_use]
    pub fn new(transport: TransportKey, mut ingress_url: reqwest::Url) -> Self {
        assert!(
            !ingress_url.cannot_be_a_base(),
            "Restate ingress URL must support path segments"
        );
        ingress_url.set_query(None);
        ingress_url.set_fragment(None);

        Self {
            transport,
            ingress_url,
            client: None,
            invocation_mode: InvocationMode::Queued,
            bearer_token: None,
            capabilities: Capabilities::new()
                .with_structured_send(StructuredSendCapability::Supported)
                .with_idempotency_key(true),
        }
    }

    /// Set the HTTP client used for ingress requests.
    ///
    /// Defaults to `reqwest::Client::new()`. Request-level limits such as
    /// connect or read timeouts belong on this client;
    /// `SendOptions::timeout` is forwarded to the worker instead.
    #[must_use]
    pub fn client(mut self, client: reqwest::Client) -> Self {
        self.client = Some(client);
        self
    }

    /// Set the invocation mode used when a send carries no override.
    ///
    /// Defaults to [`InvocationMode::Queued`].
    #[must_use]
    pub const fn invocation_mode(mut self, invocation_mode: InvocationMode) -> Self {
        self.invocation_mode = invocation_mode;
        self
    }

    /// Send `Authorization: Bearer <token>` with every ingress request.
    ///
    /// Restate Cloud ingress requires an API key here. Self-hosted Restate
    /// has no built-in ingress authentication, but a fronting reverse proxy
    /// may expect the same header. Defaults to sending no `Authorization`
    /// header. The token is redacted from `Debug` output.
    ///
    /// Other header schemes can be attached through
    /// `reqwest::ClientBuilder::default_headers` on a custom [`Self::client`].
    #[must_use]
    pub fn bearer_token(mut self, bearer_token: impl Into<String>) -> Self {
        self.bearer_token = Some(bearer_token.into());
        self
    }

    /// Replace the advertised capabilities with deployment-known worker
    /// capabilities.
    ///
    /// The default advertises structured sends and ingress idempotency only.
    #[must_use]
    pub const fn capabilities(mut self, capabilities: Capabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    /// Assert the deployed worker's structured-send support level.
    #[must_use]
    pub const fn structured_send(mut self, value: StructuredSendCapability) -> Self {
        self.capabilities.structured_send = value;
        self
    }

    /// Assert whether the deployed worker resolves attachment references.
    #[must_use]
    pub const fn attachment_references(mut self, value: bool) -> Self {
        self.capabilities.attachment_references = value;
        self
    }

    /// Build the configured transport.
    #[must_use]
    pub fn build(self) -> RestateTransport {
        let call_url = endpoint(&self.ingress_url, "call");
        let send_url = endpoint(&self.ingress_url, "send");

        RestateTransport {
            client: self.client.unwrap_or_default(),
            ingress_url: self.ingress_url,
            call_url,
            send_url,
            transport: self.transport,
            invocation_mode: self.invocation_mode,
            bearer_token: self.bearer_token,
            capabilities: self.capabilities,
        }
    }
}

impl std::fmt::Debug for RestateTransportBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RestateTransportBuilder")
            .field("transport", &self.transport)
            .field("ingress_url", &redacted_url(&self.ingress_url))
            .field("client", &self.client.as_ref().map(|_| "<reqwest::Client>"))
            .field("invocation_mode", &self.invocation_mode)
            .field(
                "bearer_token",
                &self.bearer_token.as_ref().map(|_| "<redacted>"),
            )
            .field("capabilities", &self.capabilities)
            .finish()
    }
}

/// Borrowed `Email.send` request body.
///
/// Serializes exactly like [`restate_email::SendRequest`] without cloning the
/// message. `options.idempotency_key` stays in the body so the worker can
/// forward it to the provider; the same key is also sent as Restate's
/// `idempotency-key` header (see ADR 0004).
#[derive(Serialize)]
#[serde(rename = "SendRequest")]
struct IngressSendRequest<'a> {
    transport: &'a TransportKey,
    message: &'a OutboundMessage,
    options: &'a SendOptions,
}

/// Body of a Restate `202 Accepted` response to a one-way send.
///
/// `status` (`Accepted` or `PreviouslyAccepted` on idempotent replay) and any
/// other field are deliberately ignored.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IngressSendAccepted {
    invocation_id: String,
}

#[derive(Deserialize)]
struct RestateErrorResponse {
    code: Option<u16>,
    message: Option<String>,
    source: Option<String>,
}

/// Build `{base}/restate/{kind}/Email/send` from the normalized ingress base.
fn endpoint(base: &reqwest::Url, kind: &str) -> reqwest::Url {
    let mut url = base.clone();
    url.path_segments_mut()
        .expect("ingress URL is validated by RestateTransportBuilder::new")
        .pop_if_empty()
        .extend(["restate", kind])
        .extend(SERVICE_PATH);
    url
}

/// Encode a delay as whole milliseconds, rounded up, in Restate's humantime
/// form.
fn format_delay(delay: Duration) -> String {
    let millis = delay.as_millis() + u128::from(!delay.subsec_nanos().is_multiple_of(1_000_000));
    format!("{millis}ms")
}

fn redacted_url(url: &reqwest::Url) -> reqwest::Url {
    let mut url = url.clone();
    if url.password().is_some() {
        // Only cannot-be-a-base URLs reject a password, and those never reach here.
        let _ = url.set_password(Some("redacted"));
    }
    url
}

async fn map_error_response(response: reqwest::Response) -> TransportError {
    let status = response.status().as_u16();
    let invocation_header = response
        .headers()
        .get(ERROR_SOURCE_HEADER)
        .and_then(|value| value.to_str().ok())
        == Some("invocation");
    let body = match response.text().await {
        Ok(body) => body,
        Err(error) => return map_reqwest_error(error),
    };
    let payload = serde_json::from_str::<RestateErrorResponse>(&body).ok();
    let invocation_error = invocation_header
        || payload
            .as_ref()
            .and_then(|payload| payload.source.as_deref())
            == Some("invocation");
    let code = payload.as_ref().and_then(|payload| payload.code);
    let message = payload
        .as_ref()
        .and_then(|payload| payload.message.clone())
        .filter(|message| !message.is_empty())
        .unwrap_or_else(|| {
            if body.is_empty() {
                format!("Restate ingress returned HTTP {status}")
            } else {
                body
            }
        });
    let kind = if invocation_error {
        terminal_invocation_kind(code.unwrap_or(status))
    } else {
        ingress_error_kind(status)
    };
    let mut error = TransportError::new(kind, message).with_http_status(status);
    if let Some(code) = code {
        error = error.with_provider_error_code(code.to_string());
    }
    error
}

fn ingress_error_kind(status: u16) -> ErrorKind {
    match status {
        500..=599 => ErrorKind::TransientProvider,
        _ => ErrorKind::from_http_status(status),
    }
}

fn terminal_invocation_kind(code: u16) -> ErrorKind {
    match code {
        400 => ErrorKind::Validation,
        401 => ErrorKind::Authentication,
        403 => ErrorKind::Authorization,
        422 => ErrorKind::PermanentProvider,
        _ => ErrorKind::Internal,
    }
}

fn map_response_decode_error(error: reqwest::Error) -> TransportError {
    if let Some(kind) = network_error_kind(&error) {
        TransportError::new(kind, "failed to read Restate ingress response").with_source(error)
    } else if error.is_body() {
        TransportError::new(
            ErrorKind::TransientNetwork,
            "failed to read Restate ingress response",
        )
        .with_source(error)
    } else {
        TransportError::new(
            ErrorKind::TransientProvider,
            "Restate ingress returned an invalid response",
        )
        .with_source(error)
    }
}

fn map_reqwest_error(error: reqwest::Error) -> TransportError {
    let kind = if let Some(kind) = network_error_kind(&error) {
        kind
    } else if error.is_builder() || error.is_request() {
        ErrorKind::Validation
    } else {
        ErrorKind::TransientNetwork
    };

    TransportError::new(kind, error.to_string()).with_source(error)
}

fn network_error_kind(error: &reqwest::Error) -> Option<ErrorKind> {
    if error.is_timeout() {
        Some(ErrorKind::Timeout)
    } else if error.is_connect() {
        Some(ErrorKind::TransientNetwork)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> TransportKey {
        TransportKey::new_unchecked("transactional")
    }

    fn url(input: &str) -> reqwest::Url {
        input.parse().expect("URL parses")
    }

    #[test]
    fn builder_derives_call_and_send_endpoints_under_a_path_prefix() {
        let transport = RestateTransport::new(key(), url("http://ingress.local/proxy/"));

        assert_eq!(
            transport.call_url.as_str(),
            "http://ingress.local/proxy/restate/call/Email/send"
        );
        assert_eq!(
            transport.send_url.as_str(),
            "http://ingress.local/proxy/restate/send/Email/send"
        );
        assert_eq!(
            transport.ingress_url().as_str(),
            "http://ingress.local/proxy/"
        );
    }

    #[test]
    fn builder_strips_query_and_fragment() {
        let transport =
            RestateTransport::new(key(), url("http://ingress.local?tenant=blue#configuration"));

        assert_eq!(transport.ingress_url().as_str(), "http://ingress.local/");
        assert_eq!(
            transport.send_url.as_str(),
            "http://ingress.local/restate/send/Email/send"
        );
    }

    #[test]
    #[should_panic(expected = "Restate ingress URL must support path segments")]
    fn builder_rejects_urls_that_cannot_be_a_base() {
        let _ = RestateTransport::builder(key(), url("mailto:ops@example.com"));
    }

    #[test]
    fn default_capabilities_describe_the_ingress_hop() {
        let capabilities = RestateTransport::new(key(), url("http://ingress.local")).capabilities;

        assert_eq!(
            capabilities.structured_send,
            StructuredSendCapability::Supported
        );
        assert!(capabilities.idempotency_key);
        assert!(!capabilities.timeout);
        assert!(!capabilities.attachment_references);
    }

    #[test]
    fn format_delay_rounds_up_to_whole_milliseconds() {
        assert_eq!(format_delay(Duration::from_millis(1500)), "1500ms");
        assert_eq!(format_delay(Duration::new(1, 1)), "1001ms");
        assert_eq!(format_delay(Duration::ZERO), "0ms");
    }

    #[test]
    fn invocation_id_is_only_read_from_restate_reports() {
        let queued = SendReport::new(RestateTransport::PROVIDER).with_provider_message_id("inv_1");
        let sent = SendReport::new("resend").with_provider_message_id("msg_1");

        assert_eq!(RestateTransport::invocation_id(&queued), Some("inv_1"));
        assert_eq!(RestateTransport::invocation_id(&sent), None);
    }

    #[test]
    fn debug_redacts_client_ingress_password_and_bearer_token() {
        let builder = RestateTransport::builder(key(), url("http://user:hunter2@ingress.local"))
            .client(reqwest::Client::new())
            .bearer_token("tok_supersecret");
        let rendered = format!("{builder:?} {:?}", builder.clone().build());

        assert!(!rendered.contains("hunter2"));
        assert!(!rendered.contains("tok_supersecret"));
        assert!(rendered.contains("redacted"));
        assert!(rendered.contains("<reqwest::Client>"));
    }
}
