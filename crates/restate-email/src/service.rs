//! Restate service adapter for `restate-email`.

use std::sync::Arc;

use email_kit::transport::transport_option_registry;
use email_transport::{ErrorKind, TransportError, TransportOptionRegistry};
use restate_sdk::errors::{HandlerError, TerminalError};
use restate_sdk::prelude::{Context, ContextSideEffects, HandlerResult, Json, RunFuture};

use crate::{SendRequest, SendResponse, TransportLookupError, TransportResolver};

/// Concrete Restate service implementation over a transport resolver.
///
/// Most applications construct this once at worker startup, register one or
/// more transports in a [`StaticTransportRegistry`](crate::StaticTransportRegistry),
/// and bind it to a Restate endpoint through
/// [`IntoServiceDefinition`](restate_sdk::service::IntoServiceDefinition).
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

    /// Send one email request through the configured worker dependencies.
    ///
    /// # Errors
    ///
    /// Returns [`HandlerError`] when provider options cannot be hydrated, the
    /// requested transport key is unknown, or sending fails. Unknown keys and
    /// non-retryable transport failures become Restate terminal errors;
    /// retryable transport failures remain retryable handler errors.
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

/// Restate service for queued email delivery.
///
/// The service is exposed as `Email.send` through Restate ingress. Callers that
/// are not running behind Restate should use [`ServiceImpl::send_request`] to
/// exercise the same dispatch path without the service protocol.
#[restate_sdk::service(name = "Email")]
impl<T> ServiceImpl<T>
where
    T: TransportResolver + Send + Sync + 'static,
{
    /// Dispatch one queued email request through its selected transport.
    ///
    /// # Errors
    ///
    /// Returns [`HandlerError`] when provider options cannot be hydrated, the
    /// transport key cannot be resolved, or the selected transport fails.
    #[handler]
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

impl From<TransportLookupError> for TerminalError {
    fn from(error: TransportLookupError) -> Self {
        Self::new_with_code(404, error.to_string())
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
    use email_transport::{SendOptions, SendReport, Transport, TransportError};
    use http::Request;
    use http_body_util::{BodyExt, Full};
    use prost::Message as ProstMessage;
    use restate_sdk::discovery::ServiceType as RestateServiceType;
    use restate_sdk::endpoint::{Endpoint, HandleOptions, ProtocolMode};
    use restate_sdk::service::{Discoverable, IntoServiceDefinition};
    use restate_sdk_shared_core::Version;

    use crate::{RawSendOptions, StaticTransportRegistry, TransportKey, TransportLookupError};

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

        Endpoint::builder()
            .bind(service.clone().into_service_definition())
            .build()
            .handle_with_options(
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

        Endpoint::builder()
            .bind(service.clone().into_service_definition())
            .build()
            .handle_with_options(
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
    fn service_discovers_and_binds() {
        let service = ServiceImpl::new(StubRegistry {
            error: Some(TransportLookupError::UnknownKey {
                key: "transactional".to_owned(),
            }),
        });

        let discovery = <ServiceImpl<StubRegistry> as Discoverable>::discover();
        assert_eq!(discovery.name.as_str(), "Email");
        assert_eq!(discovery.ty, RestateServiceType::Service);
        assert_eq!(discovery.handlers.len(), 1);
        assert_eq!(discovery.handlers[0].name.as_str(), "send");

        let _endpoint = Endpoint::builder()
            .bind(service.into_service_definition())
            .build();
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
