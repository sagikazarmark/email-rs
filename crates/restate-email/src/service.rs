//! Restate service adapter for `restate-email`.

use std::convert::Infallible;
use std::marker::PhantomData;
use std::sync::Arc;

use bytes::Bytes;
use email_kit::transport::transport_option_registry;
use email_transport::{ErrorKind, TransportError, TransportOptionRegistry};
use restate_sdk::errors::{HandlerError, TerminalError};
use restate_sdk::prelude::{Context, ContextSideEffects, HandlerResult, Json, RunFuture};
use restate_sdk::serde::{
    Deserialize as RestateDeserialize, InputMetadata, OutputMetadata, PayloadMetadata,
    Serialize as RestateSerialize,
};
use serde::de::DeserializeSeed as _;

use crate::{SendRequest, SendRequestSeed, SendResponse, TransportLookupError, TransportResolver};

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
    /// Returns [`HandlerError`] when the requested transport key is unknown or
    /// sending fails. Unknown transport keys and non-retryable transport
    /// failures become Restate terminal errors; retryable transport failures
    /// remain retryable handler errors.
    pub async fn send_request(&self, request: &SendRequest) -> Result<SendResponse, HandlerError> {
        let transport = self
            .transports
            .resolve(&request.transport)
            .map_err(TerminalError::from)?;

        transport
            .send(&request.message, &request.options)
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
    /// Returns [`HandlerError`] when the request cannot be decoded, the
    /// transport key cannot be resolved, or the selected transport fails.
    /// Unregistered provider option keys are ignored during request decoding.
    #[handler]
    async fn send(
        &self,
        ctx: Context<'_>,
        request: SeededJson<SendRequest>,
    ) -> HandlerResult<Json<SendResponse>> {
        let request = request
            .deserialize(self.transport_options.as_ref())
            .map_err(send_request_deserialize_error_to_handler_error)?;

        Ok(ctx
            .run(|| async move { self.send_request(&request).await.map(Json) })
            .name("send_email")
            .await?)
    }
}

struct SeededJson<T> {
    bytes: Bytes,
    marker: PhantomData<fn() -> T>,
}

impl SeededJson<SendRequest> {
    fn deserialize(
        self,
        registry: &TransportOptionRegistry,
    ) -> Result<SendRequest, SendRequestDeserializeError> {
        let mut deserializer = serde_json::Deserializer::from_slice(&self.bytes);
        let mut track = serde_path_to_error::Track::new();
        let path_deserializer =
            serde_path_to_error::Deserializer::new(&mut deserializer, &mut track);
        let request = SendRequestSeed::new(registry)
            .deserialize(path_deserializer)
            .map_err(|source| SendRequestDeserializeError {
                path: track.path().to_string(),
                source,
            })?;
        deserializer
            .end()
            .map_err(|source| SendRequestDeserializeError {
                path: String::from("."),
                source,
            })?;

        Ok(request)
    }
}

impl<T> RestateDeserialize for SeededJson<T> {
    type Error = Infallible;

    fn deserialize(bytes: &mut Bytes) -> Result<Self, Self::Error> {
        Ok(Self {
            bytes: bytes.clone(),
            marker: PhantomData,
        })
    }
}

impl<T> RestateSerialize for SeededJson<T> {
    type Error = Infallible;

    fn serialize(&self) -> Result<Bytes, Self::Error> {
        Ok(self.bytes.clone())
    }
}

impl<T> PayloadMetadata for SeededJson<T>
where
    Json<T>: PayloadMetadata,
{
    fn json_schema() -> Option<serde_json::Value> {
        <Json<T> as PayloadMetadata>::json_schema()
    }

    fn input_metadata() -> InputMetadata {
        <Json<T> as PayloadMetadata>::input_metadata()
    }

    fn output_metadata() -> OutputMetadata {
        <Json<T> as PayloadMetadata>::output_metadata()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{path}: {source}")]
struct SendRequestDeserializeError {
    path: String,
    source: serde_json::Error,
}

impl From<TransportLookupError> for TerminalError {
    fn from(error: TransportLookupError) -> Self {
        Self::new_with_code(404, error.to_string())
    }
}

#[allow(clippy::needless_pass_by_value)]
fn send_request_deserialize_error_to_handler_error(
    error: SendRequestDeserializeError,
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
    use email_message::ContentType;
    use email_message::{Address, Attachment, Body, Mailbox, Message, OutboundMessage};
    use email_transport::{SendOptions, SendReport, TransportError};
    use restate_sdk::discovery::ServiceType as RestateServiceType;
    use restate_sdk::endpoint::Endpoint;
    use restate_sdk::service::{Discoverable, IntoServiceDefinition};

    use crate::{TransportKey, TransportLookupError};

    use super::*;

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
            options: SendOptions::default(),
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
        let input = discovery.handlers[0]
            .input
            .as_ref()
            .expect("send should have input metadata");
        assert_eq!(
            input.json_schema,
            <Json<SendRequest> as PayloadMetadata>::json_schema()
        );
        assert_eq!(
            input.content_type.as_deref(),
            Some(<Json<SendRequest> as PayloadMetadata>::input_metadata().accept_content_type)
        );

        let _endpoint = Endpoint::builder()
            .bind(service.into_service_definition())
            .build();
    }
}
