//! Glue between the plain-Rust payload and the `send_email` binding.
//!
//! The wasm32 implementation builds Cloudflare's `EmailMessageBuilder` object
//! by hand and decodes the thrown JS error's `code`/`message` properties. It
//! is verified by compilation for `wasm32-unknown-unknown` in CI only. The
//! native implementation never touches wasm-bindgen's extern stubs, which
//! panic off-wasm, and reports the target as unsupported instead.

use worker::SendEmail;

use super::SenderError;
use super::payload::EmailPayload;

#[cfg(target_arch = "wasm32")]
pub(super) async fn send(
    binding: &SendEmail,
    payload: EmailPayload,
) -> Result<String, SenderError> {
    let message = wasm::build_message(&payload)?;
    let result = binding
        .send_with_builder(&message)
        .await
        .map_err(|error| wasm::decode_error(&error))?;
    Ok(result.message_id())
}

#[cfg(not(target_arch = "wasm32"))]
#[allow(
    clippy::unused_async,
    reason = "signature is shared with the wasm32 implementation"
)]
pub(super) async fn send(
    _binding: &SendEmail,
    _payload: EmailPayload,
) -> Result<String, SenderError> {
    Err(SenderError::UnsupportedTarget)
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use js_sys::{Array, Object, Reflect, Uint8Array};
    use wasm_bindgen::{JsCast as _, JsValue};
    use worker::{EmailAddress, EmailAttachment, SendEmailBuilder};

    use super::super::SenderError;
    use super::super::payload::{EmailPayload, PayloadAddress, PayloadAttachment};

    /// Build the `EmailMessageBuilder` object by hand.
    ///
    /// The `worker` crate's generated builder types recipients as bare
    /// strings and requires `to`, whereas the platform accepts
    /// `(string | EmailAddress)[]` and permits cc/bcc-only sends. Named
    /// addresses become `{ name, email }` objects; unnamed ones stay strings.
    pub(super) fn build_message(payload: &EmailPayload) -> Result<SendEmailBuilder, SenderError> {
        let message = Object::new();

        set(&message, "from", &address_value(&payload.from))?;
        // `to` is always present as an array so the field the TypeScript type
        // marks required is never missing; cc/bcc-only sends leave it empty.
        set(&message, "to", &address_array(&payload.to))?;
        if !payload.cc.is_empty() {
            set(&message, "cc", &address_array(&payload.cc))?;
        }
        if !payload.bcc.is_empty() {
            set(&message, "bcc", &address_array(&payload.bcc))?;
        }
        if let Some(reply_to) = &payload.reply_to {
            set(&message, "replyTo", &address_value(reply_to))?;
        }
        set(&message, "subject", &JsValue::from_str(&payload.subject))?;
        if let Some(text) = &payload.text {
            set(&message, "text", &JsValue::from_str(text))?;
        }
        if let Some(html) = &payload.html {
            set(&message, "html", &JsValue::from_str(html))?;
        }
        if !payload.headers.is_empty() {
            let headers = Object::new();
            for (name, value) in &payload.headers {
                set(&headers, name, &JsValue::from_str(value))?;
            }
            set(&message, "headers", &headers.into())?;
        }
        if !payload.attachments.is_empty() {
            let attachments = payload
                .attachments
                .iter()
                .map(attachment_value)
                .collect::<Array>();
            set(&message, "attachments", &attachments.into())?;
        }

        Ok(message.unchecked_into())
    }

    /// Decode a thrown JS error into Cloudflare's `code` and `message`.
    ///
    /// Read directly rather than through `worker::Error`'s variant mapping so
    /// a single code table owns classification. An absent or empty `code`
    /// means the error did not come from the platform's send pipeline. The JS
    /// value itself is not attached as an error source because `JsValue` is
    /// not reliably `Send + Sync` across wasm-bindgen configurations.
    pub(super) fn decode_error(error: &js_sys::Error) -> SenderError {
        let code = Reflect::get(error, &JsValue::from_str("code"))
            .ok()
            .and_then(|value| value.as_string())
            .filter(|code| !code.is_empty());
        let message = String::from(error.message());
        SenderError::Binding { code, message }
    }

    fn address_value(address: &PayloadAddress) -> JsValue {
        match &address.name {
            Some(name) => EmailAddress::new(name, &address.email).into(),
            None => JsValue::from_str(&address.email),
        }
    }

    fn address_array(addresses: &[PayloadAddress]) -> JsValue {
        addresses
            .iter()
            .map(address_value)
            .collect::<Array>()
            .into()
    }

    fn attachment_value(attachment: &PayloadAttachment) -> JsValue {
        // Binary content is passed as a typed array rather than base64 so the
        // payload is not inflated by a third against the 5 MiB limit.
        let content = Uint8Array::from(attachment.content.as_slice());
        let value = if attachment.inline {
            EmailAttachment::new_inline_with_typed_array(
                attachment.content_id.as_deref().unwrap_or_default(),
                &attachment.filename,
                &attachment.content_type,
                &content,
            )
        } else {
            let mut builder = EmailAttachment::builder_attachment_with_typed_array(
                &attachment.filename,
                &attachment.content_type,
                &content,
            );
            if let Some(content_id) = &attachment.content_id {
                builder = builder.content_id(content_id);
            }
            builder.build()
        };
        value.into()
    }

    fn set(target: &Object, key: &str, value: &JsValue) -> Result<(), SenderError> {
        Reflect::set(target, &JsValue::from_str(key), value)
            .map(drop)
            .map_err(|error| SenderError::Binding {
                code: None,
                message: format!(
                    "failed to set `{key}` on the cloudflare email message: {}",
                    describe(&error)
                ),
            })
    }

    fn describe(value: &JsValue) -> String {
        value.dyn_ref::<js_sys::Error>().map_or_else(
            || {
                value
                    .as_string()
                    .unwrap_or_else(|| String::from("unknown JS error"))
            },
            |error| String::from(error.message()),
        )
    }
}
