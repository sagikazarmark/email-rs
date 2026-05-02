//! Restate service adapter for `restate-email`.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use email_kit::transport::transport_option_registry;
use email_message::OutboundMessage;
use email_transport::{
    CorrelationId, ErrorKind, IdempotencyKey, SendOptions, SendReport, TransportError,
    TransportOptionRegistry,
};
use restate_sdk::errors::{HandlerError, TerminalError};
use restate_sdk::prelude::{Context, ContextSideEffects, Endpoint, HandlerResult, Json, RunFuture};
use serde::{Deserialize, Serialize};

use crate::{TransportKey, TransportResolver};

/// Restate service contract for queued email delivery.
///
/// The service is exposed as `Email.send` through Restate ingress. Callers that
/// are not running behind Restate should use [`ServiceImpl::send_request`] to
/// exercise the same dispatch path without the service protocol.
#[restate_sdk::service]
#[name = "Email"]
pub trait Service {
    #[name = "send"]
    async fn send(request: Json<SendRequest>) -> HandlerResult<Json<SendResponse>>;
}

/// Queue payload consumed by `Email.send`.
///
/// The serde shape is part of the crate's cross-process contract: producers can
/// serialize this value into Restate ingress or another queue, and workers can
/// deserialize it later with the configured provider-option registry.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SendRequest {
    /// Configured transport profile to use for this send.
    pub transport: TransportKey,
    /// Validated outbound message payload.
    pub message: OutboundMessage,
    /// Send-time metadata and raw provider-specific transport options.
    #[serde(default)]
    pub options: RawSendOptions,
}

/// Wire-friendly send options whose provider-specific slots are still raw.
///
/// `SendOptions` intentionally needs a [`TransportOptionRegistry`] to hydrate
/// typed provider slots. This staging type lets Restate deserialize the queue
/// payload normally, then uses [`email_transport::TransportOptionsSeed`] to
/// turn the raw provider-keyed map into the typed
/// [`email_transport::TransportOptions`] passed to transports.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct RawSendOptions {
    /// Optional SMTP envelope override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub envelope: Option<email_message::Envelope>,
    /// Provider-keyed raw transport options to hydrate with a
    /// [`TransportOptionRegistry`].
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[cfg_attr(
        feature = "schemars",
        schemars(with = "BTreeMap<String, serde_json::Value>")
    )]
    pub transport_options: BTreeMap<String, serde_value::Value>,
    /// Per-send timeout forwarded to transports that honor timeout metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<Duration>,
    /// Provider-facing idempotency key, when supported by the selected
    /// transport.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<IdempotencyKey>,
    /// Caller-supplied correlation id for tracing provider requests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<CorrelationId>,
}

impl RawSendOptions {
    /// Hydrate raw provider options and assemble typed [`SendOptions`].
    ///
    /// # Errors
    ///
    /// Returns [`serde_value::DeserializerError`] when a provider key is not
    /// registered or a registered provider option has the wrong wire shape.
    pub fn into_send_options(
        self,
        registry: &TransportOptionRegistry,
    ) -> Result<SendOptions, serde_value::DeserializerError> {
        use serde::de::DeserializeSeed as _;

        let mut options = SendOptions::new();

        if let Some(envelope) = self.envelope {
            options = options.with_envelope(envelope);
        }
        if !self.transport_options.is_empty() {
            let transport_options_value = serde_value::Value::Map(
                self.transport_options
                    .into_iter()
                    .map(|(key, value)| (serde_value::Value::String(key), value))
                    .collect(),
            );
            let transport_options = registry
                .transport_options_seed()
                .deserialize(transport_options_value)?;
            options = options.with_transport_options(transport_options);
        }
        if let Some(timeout) = self.timeout {
            options = options.with_timeout(timeout);
        }
        if let Some(idempotency_key) = self.idempotency_key {
            options = options.with_idempotency_key(idempotency_key);
        }
        if let Some(correlation_id) = self.correlation_id {
            options = options.with_correlation_id(correlation_id);
        }

        Ok(options)
    }

    /// Hydrate a borrowed raw option set into typed [`SendOptions`].
    ///
    /// # Errors
    ///
    /// Returns [`serde_value::DeserializerError`] when a provider key is not
    /// registered or a registered provider option has the wrong wire shape.
    pub fn to_send_options(
        &self,
        registry: &TransportOptionRegistry,
    ) -> Result<SendOptions, serde_value::DeserializerError> {
        self.clone().into_send_options(registry)
    }
}

/// Wire-stable response shape returned by the Restate `Email.send`
/// handler.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SendResponse {
    /// Provider send report returned by the selected transport.
    pub report: SendReport,
}

impl From<SendReport> for SendResponse {
    fn from(report: SendReport) -> Self {
        Self { report }
    }
}

/// Concrete Restate service implementation over a transport resolver.
///
/// Most applications construct this once at worker startup, register one or
/// more transports in a [`StaticTransportRegistry`](crate::StaticTransportRegistry),
/// and expose [`Self::endpoint`] through `restate_sdk::prelude::HttpServer`.
pub struct ServiceImpl<T> {
    transports: Arc<T>,
    transport_options: Arc<TransportOptionRegistry>,
}

impl<T> Clone for ServiceImpl<T> {
    fn clone(&self) -> Self {
        Self {
            transports: Arc::clone(&self.transports),
            transport_options: Arc::clone(&self.transport_options),
        }
    }
}

impl<T> ServiceImpl<T>
where
    T: TransportResolver + Send + Sync + 'static,
{
    /// Build a service around an owned transport resolver.
    #[must_use]
    pub fn new(transports: T) -> Self {
        Self::from_shared(Arc::new(transports))
    }

    /// Build a service around a shared transport resolver.
    #[must_use]
    pub fn from_shared(transports: Arc<T>) -> Self {
        Self {
            transports,
            transport_options: Arc::new(transport_option_registry()),
        }
    }

    /// Override the provider-option registry used to hydrate queued
    /// `transport_options`.
    ///
    /// The default registry is `email_kit::transport::transport_option_registry()`;
    /// use this method when a worker has additional provider-specific option
    /// types outside `email-kit`.
    #[must_use]
    pub fn with_transport_options(mut self, transport_options: TransportOptionRegistry) -> Self {
        self.transport_options = Arc::new(transport_options);
        self
    }

    /// Build a Restate endpoint that serves this service.
    #[must_use]
    pub fn endpoint(&self) -> Endpoint {
        Endpoint::builder().bind(self.clone().serve()).build()
    }

    /// Send one email request through the configured worker dependencies.
    ///
    /// # Errors
    ///
    /// Returns [`HandlerError`] when validation or sending through the selected
    /// transport fails.
    pub async fn send_request(&self, request: &SendRequest) -> Result<SendResponse, HandlerError> {
        let options = request
            .options
            .to_send_options(self.transport_options.as_ref())
            .map_err(raw_send_options_deserialize_error_to_handler_error)?;
        let transport = self
            .transports
            .resolve(&request.transport)
            .map_err(TerminalError::from)?;

        transport
            .send(&request.message, &options)
            .await
            .map(SendResponse::from)
            .map_err(transport_error_to_handler_error)
    }
}

impl<T> Service for ServiceImpl<T>
where
    T: TransportResolver + Send + Sync + 'static,
{
    async fn send(
        &self,
        ctx: Context<'_>,
        request: Json<SendRequest>,
    ) -> HandlerResult<Json<SendResponse>> {
        let request = request.into_inner();

        Ok(ctx
            .run(|| async move { self.send_request(&request).await.map(Json) })
            .name("send_email")
            .await?)
    }
}

fn raw_send_options_deserialize_error_to_handler_error(
    error: serde_value::DeserializerError,
) -> HandlerError {
    TerminalError::new_with_code(400, error.to_string()).into()
}

fn transport_error_to_handler_error(error: TransportError) -> HandlerError {
    if error.is_retryable() {
        return error.into();
    }
    let code = transport_terminal_code(&error);
    TerminalError::new_with_code(code, error.to_string()).into()
}

const fn transport_terminal_code(error: &TransportError) -> u16 {
    match error.kind {
        ErrorKind::Validation | ErrorKind::UnsupportedFeature => 400,
        ErrorKind::Authentication => 401,
        ErrorKind::Authorization => 403,
        ErrorKind::PermanentProvider => 422,
        _ => 500,
    }
}

#[cfg(test)]
mod tests {
    use bytes::{Buf, BufMut, Bytes, BytesMut};
    use email_message::ContentType;
    use email_message::{Address, Attachment, Body, Mailbox, Message, OutboundMessage};
    use email_transport::{SendOptions, Transport, TransportError};
    use http::Request;
    use http_body_util::{BodyExt, Full};
    use prost::Message as ProstMessage;
    use restate_sdk::endpoint::{HandleOptions, ProtocolMode};
    use restate_sdk_shared_core::Version;

    use crate::{StaticTransportRegistry, TransportKey, TransportLookupError};

    use super::*;

    const START_MESSAGE_TYPE: u16 = 0x0000;
    const INPUT_COMMAND_MESSAGE_TYPE: u16 = 0x0400;
    const RUN_COMMAND_MESSAGE_TYPE: u16 = 0x0411;

    mod protocol {
        #[derive(Clone, PartialEq, Eq, ::prost::Message)]
        pub struct StartMessage {
            #[prost(bytes = "bytes", tag = "1")]
            pub id: ::prost::bytes::Bytes,
            #[prost(string, tag = "2")]
            pub debug_id: ::prost::alloc::string::String,
            #[prost(uint32, tag = "3")]
            pub known_entries: u32,
            #[prost(message, repeated, tag = "4")]
            pub state_map: ::prost::alloc::vec::Vec<start_message::StateEntry>,
            #[prost(bool, tag = "5")]
            pub partial_state: bool,
            #[prost(string, tag = "6")]
            pub key: ::prost::alloc::string::String,
            #[prost(uint32, tag = "7")]
            pub retry_count_since_last_stored_entry: u32,
            #[prost(uint64, tag = "8")]
            pub duration_since_last_stored_entry: u64,
            #[prost(uint64, tag = "9")]
            pub random_seed: u64,
        }

        pub mod start_message {
            #[derive(Clone, PartialEq, Eq, Hash, ::prost::Message)]
            pub struct StateEntry {
                #[prost(bytes = "bytes", tag = "1")]
                pub key: ::prost::bytes::Bytes,
                #[prost(bytes = "bytes", tag = "2")]
                pub value: ::prost::bytes::Bytes,
            }
        }

        #[derive(Clone, PartialEq, Eq, Hash, ::prost::Message)]
        pub struct Value {
            #[prost(bytes = "bytes", tag = "1")]
            pub content: ::prost::bytes::Bytes,
        }

        #[derive(Clone, PartialEq, Eq, Hash, ::prost::Message)]
        pub struct Header {
            #[prost(string, tag = "1")]
            pub key: ::prost::alloc::string::String,
            #[prost(string, tag = "2")]
            pub value: ::prost::alloc::string::String,
        }

        #[derive(Clone, PartialEq, Eq, ::prost::Message)]
        pub struct InputCommandMessage {
            #[prost(message, repeated, tag = "1")]
            pub headers: ::prost::alloc::vec::Vec<Header>,
            #[prost(message, optional, tag = "14")]
            pub value: ::core::option::Option<Value>,
            #[prost(string, tag = "12")]
            pub name: ::prost::alloc::string::String,
        }

        #[derive(Clone, PartialEq, Eq, ::prost::Message)]
        pub struct RunCommandMessage {
            #[prost(uint32, tag = "11")]
            pub result_completion_id: u32,
            #[prost(string, tag = "12")]
            pub name: ::prost::alloc::string::String,
        }
    }

    use protocol::{InputCommandMessage, RunCommandMessage, StartMessage};

    fn mailbox(input: &str) -> Mailbox {
        input.parse::<Mailbox>().expect("mailbox should parse")
    }

    fn request_with_attachment() -> SendRequest {
        let message = Message::builder(Body::text("hello"))
            .from_mailbox(mailbox("from@example.com"))
            .add_to(Address::Mailbox(mailbox("to@example.com")))
            .add_attachment(
                Attachment::bytes(
                    ContentType::try_from("application/pdf").expect("content type should parse"),
                    b"attached".to_vec(),
                )
                .with_filename("report.pdf"),
            )
            .build()
            .expect("message should validate");

        SendRequest {
            transport: TransportKey::new_unchecked("transactional"),
            message: OutboundMessage::new(message).expect("message should be outbound-valid"),
            options: RawSendOptions::default(),
        }
    }

    struct StubRegistry {
        error: Option<TransportLookupError>,
    }

    impl TransportResolver for StubRegistry {
        fn resolve(
            &self,
            _transport: &TransportKey,
        ) -> Result<&email_transport::DynTransport, TransportLookupError> {
            Err(self.error.clone().expect("expected lookup error"))
        }
    }

    struct SuccessfulTransport;

    impl Transport for SuccessfulTransport {
        fn send<'a>(
            &'a self,
            _message: &'a email_message::OutboundMessage,
            _options: &'a SendOptions,
        ) -> impl core::future::Future<Output = Result<SendReport, TransportError>> + Send + 'a
        {
            Box::pin(async move {
                Ok(SendReport::new("example")
                    .with_provider_message_id("provider-id")
                    .with_accepted(vec!["to@example.com".parse().expect("email parses")]))
            })
        }
    }

    fn invoke_raw_sdk_endpoint<T>(
        service: &ServiceImpl<T>,
        request: &SendRequest,
    ) -> http::Response<restate_sdk::endpoint::ResponseBody>
    where
        T: TransportResolver + Send + Sync + 'static,
    {
        let request = Request::builder()
            .method("POST")
            .uri("/invoke/Email/send")
            .header("content-type", "application/json")
            .body(Full::from(Bytes::from(
                serde_json::to_vec(request).expect("request should serialize"),
            )))
            .expect("request should build");

        service.endpoint().handle_with_options(
            request,
            HandleOptions {
                protocol_mode: ProtocolMode::RequestResponse,
            },
        )
    }

    fn invoke_protocol_sdk_endpoint<T>(
        service: &ServiceImpl<T>,
        request: &SendRequest,
    ) -> http::Response<restate_sdk::endpoint::ResponseBody>
    where
        T: TransportResolver + Send + Sync + 'static,
    {
        let version = Version::maximum_supported_version();
        let mut body = BytesMut::new();

        body.extend_from_slice(&encode_protocol_message(
            START_MESSAGE_TYPE,
            &StartMessage {
                id: Bytes::from_static(b"123"),
                debug_id: String::from("123"),
                known_entries: 1,
                ..StartMessage::default()
            },
        ));
        body.extend_from_slice(&encode_protocol_message(
            INPUT_COMMAND_MESSAGE_TYPE,
            &InputCommandMessage {
                value: Some(protocol::Value {
                    content: Bytes::from(
                        serde_json::to_vec(request).expect("request should serialize"),
                    ),
                }),
                ..InputCommandMessage::default()
            },
        ));

        let request = Request::builder()
            .method("POST")
            .uri("/invoke/Email/send")
            .header("content-type", version.content_type())
            .body(Full::from(body.freeze()))
            .expect("request should build");

        service.endpoint().handle_with_options(
            request,
            HandleOptions {
                protocol_mode: ProtocolMode::RequestResponse,
            },
        )
    }

    async fn collect_response_body(
        response: http::Response<restate_sdk::endpoint::ResponseBody>,
    ) -> (http::StatusCode, http::HeaderMap, Bytes) {
        let (parts, body) = response.into_parts();
        let bytes = body
            .collect()
            .await
            .expect("body should collect")
            .to_bytes();

        (parts.status, parts.headers, bytes)
    }

    fn decode_protocol_run_command(body: Bytes) -> RunCommandMessage {
        let mut body = body;
        let (run_ty, run_payload) = decode_protocol_message(&mut body);

        assert_eq!(run_ty, RUN_COMMAND_MESSAGE_TYPE);

        RunCommandMessage::decode(run_payload).expect("message should decode as run command")
    }

    fn encode_protocol_message<M: ProstMessage>(message_type: u16, message: &M) -> Bytes {
        let mut buffer = BytesMut::with_capacity(8 + message.encoded_len());
        let header = (u64::from(message_type) << 48) | (message.encoded_len() as u64);
        buffer.put_u64(header);
        message
            .encode(&mut buffer)
            .expect("protocol message should encode");
        buffer.freeze()
    }

    fn decode_protocol_message(body: &mut Bytes) -> (u16, Bytes) {
        assert!(
            body.remaining() >= 8,
            "protocol response should include a header"
        );

        let header = body.get_u64();
        let message_type =
            u16::try_from(header >> 48).expect("message type is stored in high 16 bits");
        let message_length = usize::try_from(header & 0x0000_FFFF_FFFF_FFFF)
            .expect("message length should fit usize");

        assert!(
            body.remaining() >= message_length,
            "protocol response should include the full payload"
        );

        (message_type, body.copy_to_bytes(message_length))
    }

    #[test]
    fn send_email_response_maps_from_send_report() {
        let report = SendReport::new("resend")
            .with_provider_message_id("id-1")
            .with_accepted(vec!["to@example.com".parse().expect("email parses")]);

        let response = SendResponse::from(report);

        assert_eq!(response.report.provider, "resend");
        assert_eq!(response.report.provider_message_id.as_deref(), Some("id-1"));
        assert_eq!(response.report.accepted[0].as_str(), "to@example.com");
    }

    #[test]
    fn transport_error_disposition_maps_all_current_error_kinds() {
        let retryable = [
            ErrorKind::RateLimited,
            ErrorKind::Timeout,
            ErrorKind::TransientNetwork,
            ErrorKind::TransientProvider,
        ];
        for kind in retryable {
            let label = kind.to_string();
            let error = TransportError::new(kind, "retryable");
            assert!(error.is_retryable(), "{label} should remain retryable");
        }

        let terminal = [
            (ErrorKind::Validation, 400),
            (ErrorKind::Authentication, 401),
            (ErrorKind::Authorization, 403),
            (ErrorKind::PermanentProvider, 422),
            (ErrorKind::UnsupportedFeature, 400),
            (ErrorKind::Internal, 500),
        ];
        for (kind, expected_code) in terminal {
            let label = kind.to_string();
            let error = TransportError::new(kind, "terminal");
            assert!(!error.is_retryable(), "{label} should remain terminal");
            assert_eq!(super::transport_terminal_code(&error), expected_code);
        }
    }

    #[tokio::test]
    async fn service_send_maps_lookup_error_to_terminal() {
        let service = ServiceImpl::new(StubRegistry {
            error: Some(TransportLookupError::UnknownKey {
                key: "transactional".to_owned(),
            }),
        });

        let error = service
            .send_request(&request_with_attachment())
            .await
            .expect_err("request should fail");

        let source: &(dyn std::error::Error + Send + Sync + 'static) = error.as_ref();
        assert!(source.to_string().contains("transactional"));
    }

    #[test]
    fn endpoint_builder_binds_service() {
        let service = ServiceImpl::new(StubRegistry {
            error: Some(TransportLookupError::UnknownKey {
                key: "transactional".to_owned(),
            }),
        });

        let _endpoint = service.endpoint();
    }

    #[test]
    fn raw_sdk_endpoint_rejects_plain_json_invocation() {
        let mut registry = StaticTransportRegistry::new();
        registry.insert("transactional", SuccessfulTransport);
        let service = ServiceImpl::new(registry);

        let response = invoke_raw_sdk_endpoint(&service, &request_with_attachment());

        assert_eq!(response.status().as_u16(), 415);
    }

    #[tokio::test]
    async fn raw_protocol_endpoint_emits_run_command_for_send_side_effect() {
        let mut registry = StaticTransportRegistry::new();
        registry.insert("transactional", SuccessfulTransport);
        let service = ServiceImpl::new(registry);

        let response = invoke_protocol_sdk_endpoint(&service, &request_with_attachment());
        let (status, headers, body) = collect_response_body(response).await;
        let run = decode_protocol_run_command(body);

        assert_eq!(status, http::StatusCode::OK);
        assert_eq!(
            headers
                .get("content-type")
                .and_then(|value| value.to_str().ok()),
            Some(Version::maximum_supported_version().content_type())
        );
        assert_eq!(run.name, "send_email");
    }
}
