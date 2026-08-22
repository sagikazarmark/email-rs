//! Caller-side transport that invokes the email worker through Restate ingress.

use email_message::OutboundMessage;
use email_transport::{
    Capabilities, ErrorKind, MaybeSend, SendOptions, SendReport, StructuredSendCapability,
    Transport, TransportError,
};
use serde::Deserialize;

use crate::{RawSendOptions, SendRequest, SendResponse, TransportKey};

const ERROR_SOURCE_HEADER: &str = "x-restate-error-source";

/// Email transport backed by a Restate `Email.send` ingress invocation.
///
/// The worker's capabilities cannot be discovered through ingress. The
/// capability setters are therefore deployment assertions made by the caller.
/// Defaults are conservative except for ingress idempotency, which this adapter
/// implements directly.
#[derive(Clone, Debug)]
pub struct RestateTransport {
    client: reqwest::Client,
    endpoint: reqwest::Url,
    transport: TransportKey,
    capabilities: Capabilities,
}

impl RestateTransport {
    /// Create a transport targeting `Email.send` at the ingress base URL.
    ///
    /// `SendOptions::idempotency_key` is consumed as Restate's
    /// `idempotency-key` request header. It is deliberately omitted from the
    /// queued [`RawSendOptions`], so the provider does not receive the same key.
    #[must_use]
    pub fn new(
        ingress_base_url: reqwest::Url,
        transport: TransportKey,
        client: reqwest::Client,
    ) -> Self {
        let endpoint = format!(
            "{}/Email/send",
            ingress_base_url.as_str().trim_end_matches('/')
        )
        .parse()
        .expect("appending the static Email/send path preserves a valid URL");

        Self {
            client,
            endpoint,
            transport,
            capabilities: Capabilities::new().with_idempotency_key(true),
        }
    }

    /// Replace the advertised capabilities with deployment-known worker
    /// capabilities.
    #[must_use]
    pub const fn with_capabilities(mut self, capabilities: Capabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    /// Assert the deployed worker's structured-send support level.
    #[must_use]
    pub const fn with_structured_send(mut self, value: StructuredSendCapability) -> Self {
        self.capabilities.structured_send = value;
        self
    }

    /// Assert whether the deployed worker resolves attachment references.
    #[must_use]
    pub const fn with_attachment_references(mut self, value: bool) -> Self {
        self.capabilities.attachment_references = value;
        self
    }

    async fn invoke(
        &self,
        message: &OutboundMessage,
        options: &SendOptions,
    ) -> Result<SendReport, TransportError> {
        let mut raw_options = RawSendOptions::from_send_options(options).map_err(|error| {
            TransportError::new(
                ErrorKind::Internal,
                "failed to serialize Restate send options",
            )
            .with_source(error)
        })?;
        raw_options.idempotency_key = None;

        let request = SendRequest {
            transport: self.transport.clone(),
            message: message.clone(),
            options: raw_options,
        };
        let mut request_builder = self.client.post(self.endpoint.clone()).json(&request);
        if let Some(idempotency_key) = options.idempotency_key.as_ref() {
            request_builder = request_builder.header("idempotency-key", idempotency_key.as_str());
        }

        let response = request_builder.send().await.map_err(map_reqwest_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(map_error_response(response).await);
        }

        response
            .json::<SendResponse>()
            .await
            .map(|response| response.report)
            .map_err(map_response_decode_error)
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

#[derive(Deserialize)]
struct RestateErrorResponse {
    code: Option<u16>,
    message: Option<String>,
    source: Option<String>,
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
