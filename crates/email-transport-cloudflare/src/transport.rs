mod binding;
mod error;
mod payload;

use std::sync::Arc;

use email_message::OutboundMessage;
use email_transport::{
    Capabilities, SendOptions, SendReport, StructuredSendCapability, Transport, TransportError,
    structured_accepted_for,
};
use worker::{Env, SendEmail};

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
/// an [`UnsupportedFeature`](email_transport::ErrorKind::UnsupportedFeature)
/// error instead of reaching wasm-bindgen's panicking extern stubs.
#[derive(Clone)]
pub struct CloudflareTransport {
    /// Shared so cloning never calls wasm-bindgen's object-clone intrinsic,
    /// which is a JS round trip inside a Worker and a panic on native targets.
    binding: Arc<SendEmail>,
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
        Self {
            binding: Arc::new(binding),
        }
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

        let message_id = binding::send(&self.binding, payload).await?;

        Ok(SendReport::new(PROVIDER)
            .with_provider_message_id(message_id)
            .with_accepted(accepted))
    }
}

/// Stable [`SendReport::provider`] identifier for this transport.
pub const PROVIDER: &str = "cloudflare";

/// Message mapping and error classification are pure Rust and tested beside
/// their modules; these tests cover what is left of the transport itself.
#[cfg(test)]
mod tests {
    use email_message::{Address, Body, Mailbox, Message, OutboundMessage};
    use email_transport::{ErrorKind, SendOptions, StructuredSendCapability, Transport};
    use wasm_bindgen::{JsCast as _, JsValue};
    use worker::SendEmail;

    use super::{CloudflareTransport, PROVIDER};

    /// `JsValue::UNDEFINED` is a reserved constant: constructing and dropping
    /// it never calls a wasm-bindgen intrinsic, so it is safe on native
    /// targets.
    fn undefined_binding() -> SendEmail {
        SendEmail::unchecked_from_js(JsValue::UNDEFINED)
    }

    fn mailbox(input: &str) -> Mailbox {
        input.parse().expect("valid mailbox fixture")
    }

    fn minimal_message() -> OutboundMessage {
        Message::builder(Body::text("Body"))
            .from_mailbox(mailbox("sender@example.com"))
            .to(vec![Address::Mailbox(mailbox("recipient@example.com"))])
            .subject("Hello")
            .build_outbound()
            .expect("message should validate")
    }

    #[test]
    fn capabilities_match_binding_behavior() {
        let capabilities = CloudflareTransport::new(undefined_binding()).capabilities();

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
        let rendered = format!("{:?}", CloudflareTransport::new(undefined_binding()));

        assert_eq!(
            rendered,
            "CloudflareTransport { binding: \"<cloudflare send_email binding>\" }"
        );
    }

    #[test]
    fn provider_constant_is_stable() {
        assert_eq!(PROVIDER, "cloudflare");
    }

    #[test]
    fn from_converts_binding() {
        let transport = CloudflareTransport::from(undefined_binding());

        assert!(format!("{transport:?}").contains("<cloudflare send_email binding>"));
    }

    #[tokio::test]
    async fn native_send_against_real_binding_is_unsupported_not_a_panic() {
        let transport = CloudflareTransport::new(undefined_binding());

        let error = transport
            .send(&minimal_message(), &SendOptions::default())
            .await
            .expect_err("binding is unavailable on native targets");

        assert_eq!(error.kind, ErrorKind::UnsupportedFeature);
        assert!(error.message.contains("wasm32-unknown-unknown"));
    }
}
