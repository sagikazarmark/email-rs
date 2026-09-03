// Mirrors `restate-sdk-shared-core`'s private
// `service_protocol::messages` module at the pinned 7.0 release. Upstream can
// reshape these messages without a semver signal, so keep only fields used by
// these contract tests and verify them when updating the dependency.

use bytes::{Buf, BufMut, Bytes, BytesMut};
use http::Request;
use http_body_util::{BodyExt as _, Full};
use prost::Message as ProstMessage;
use restate_email::{SendRequest, Service};
use restate_sdk::endpoint::{Endpoint, HandleOptions, ProtocolMode};
use restate_sdk::service::IntoServiceDefinition as _;
use restate_sdk_shared_core::Version;

#[derive(Clone, PartialEq, Eq, prost::Message)]
pub(crate) struct RunCommandMessage {
    #[prost(string, tag = "12")]
    pub(crate) name: String,
}

#[derive(Clone, PartialEq, Eq, prost::Message)]
pub(crate) struct Failure {
    #[prost(uint32, tag = "1")]
    pub(crate) code: u32,
    #[prost(string, tag = "2")]
    pub(crate) message: String,
}

#[derive(Clone, PartialEq, Eq, prost::Message)]
struct StartMessage {
    #[prost(bytes = "bytes", tag = "1")]
    id: Bytes,
    #[prost(string, tag = "2")]
    debug_id: String,
    #[prost(uint32, tag = "3")]
    known_entries: u32,
}

#[derive(Clone, PartialEq, Eq, Hash, prost::Message)]
struct Value {
    #[prost(bytes = "bytes", tag = "1")]
    content: Bytes,
}

#[derive(Clone, PartialEq, Eq, prost::Message)]
struct InputCommandMessage {
    #[prost(message, optional, tag = "14")]
    value: Option<Value>,
}

#[derive(Clone, PartialEq, Eq, prost::Message)]
struct OutputCommandMessage {
    #[prost(message, optional, tag = "15")]
    failure: Option<Failure>,
}

const START_MESSAGE_TYPE: u16 = 0x0000;
const INPUT_COMMAND_MESSAGE_TYPE: u16 = 0x0400;
const OUTPUT_COMMAND_MESSAGE_TYPE: u16 = 0x0401;
const RUN_COMMAND_MESSAGE_TYPE: u16 = 0x0411;

pub(crate) fn invoke_protocol_sdk_endpoint(
    service: &Service,
    request: &SendRequest,
) -> http::Response<restate_sdk::endpoint::ResponseBody> {
    invoke_protocol_sdk_endpoint_with_payload(
        service,
        Bytes::from(serde_json::to_vec(request).expect("request should serialize")),
    )
}

pub(crate) fn invoke_protocol_sdk_endpoint_with_payload(
    service: &Service,
    payload: Bytes,
) -> http::Response<restate_sdk::endpoint::ResponseBody> {
    let version = Version::maximum_supported_version();
    let mut body = BytesMut::new();

    body.extend_from_slice(&encode_protocol_message(
        START_MESSAGE_TYPE,
        &StartMessage {
            id: Bytes::from_static(b"123"),
            debug_id: String::from("123"),
            known_entries: 1,
        },
    ));
    body.extend_from_slice(&encode_protocol_message(
        INPUT_COMMAND_MESSAGE_TYPE,
        &InputCommandMessage {
            value: Some(Value { content: payload }),
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

pub(crate) async fn collect_response_body(
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

pub(crate) fn decode_protocol_run_command(body: Bytes) -> RunCommandMessage {
    let mut body = body;
    let (run_ty, run_payload) = decode_protocol_message(&mut body);

    assert_eq!(run_ty, RUN_COMMAND_MESSAGE_TYPE);

    RunCommandMessage::decode(run_payload).expect("message should decode as run command")
}

pub(crate) fn decode_protocol_failure(body: Bytes) -> Failure {
    let mut body = body;
    let (output_ty, output_payload) = decode_protocol_message(&mut body);

    assert_eq!(output_ty, OUTPUT_COMMAND_MESSAGE_TYPE);

    OutputCommandMessage::decode(output_payload)
        .expect("message should decode as output command")
        .failure
        .expect("output should contain a terminal failure")
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
    let message_type = u16::try_from(header >> 48).expect("message type is stored in high 16 bits");
    let message_length =
        usize::try_from(header & 0x0000_FFFF_FFFF_FFFF).expect("message length should fit usize");

    assert!(
        body.remaining() >= message_length,
        "protocol response should include the full payload"
    );

    (message_type, body.copy_to_bytes(message_length))
}
