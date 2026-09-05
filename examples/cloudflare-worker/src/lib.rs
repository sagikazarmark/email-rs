//! A minimal Cloudflare Worker that sends one message through
//! `email-transport-cloudflare`.
//!
//! `GET /?to=<address>[&attachment]` sends a text-and-HTML message from the
//! `EMAIL_FROM` variable to `to`, optionally with a small attachment, and
//! reports the outcome as JSON. Failures carry the `TransportError` kind and
//! Cloudflare's error code so the crate's classification table can be checked
//! against the real platform.

use email_message::{
    Address, Attachment, Body, ContentType, Header, Mailbox, Message, OutboundMessage,
};
use email_transport::{ErrorKind, SendOptions, Transport, TransportError};
use email_transport_cloudflare::CloudflareTransport;
use serde::Serialize;
use worker::{Context, Env, Request, Response, Result, Url, event};

/// Name of the `[[send_email]]` binding declared in `wrangler.toml`.
const BINDING: &str = "EMAIL";

#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    console_error_panic_hook::set_once();

    let url = req.url()?;
    let Some(to) = query(&url, "to") else {
        return Response::error("missing `to` query parameter", 400);
    };
    let with_attachment = url.query_pairs().any(|(key, _)| key == "attachment");
    let from = env.var("EMAIL_FROM")?.to_string();

    let message = match build_message(&from, &to, with_attachment) {
        Ok(message) => message,
        Err(error) => return Response::error(format!("invalid message: {error}"), 400),
    };

    let transport = CloudflareTransport::from_env(&env, BINDING)?;
    match transport.send(&message, &SendOptions::default()).await {
        Ok(report) => Response::from_json(&report),
        Err(error) => {
            Ok(Response::from_json(&Failure::from(&error))?.with_status(status_for(&error)))
        }
    }
}

/// JSON body returned when the send fails.
#[derive(Serialize)]
struct Failure {
    kind: String,
    message: String,
    provider_error_code: Option<String>,
    retryable: bool,
}

impl From<&TransportError> for Failure {
    fn from(error: &TransportError) -> Self {
        Self {
            kind: error.kind.to_string(),
            message: error.message.clone(),
            provider_error_code: error.provider_error_code.clone(),
            retryable: error.is_retryable(),
        }
    }
}

fn build_message(
    from: &str,
    to: &str,
    with_attachment: bool,
) -> std::result::Result<OutboundMessage, Box<dyn std::error::Error>> {
    let mut builder = Message::builder(Body::text_and_html(
        "Hello from email-rs on Cloudflare Workers.",
        "<p>Hello from <strong>email-rs</strong> on Cloudflare Workers.</p>",
    ))
    .from_mailbox(from.parse::<Mailbox>()?)
    .to(vec![to.parse::<Address>()?])
    .subject("email-rs Cloudflare example")
    .add_header(Header::new("X-Example", "email-rs")?);

    if with_attachment {
        builder = builder.add_attachment(
            Attachment::bytes(
                ContentType::try_from("text/plain")?,
                b"Hello World\n".to_vec(),
            )
            .with_filename("hello.txt"),
        );
    }

    Ok(builder.build_outbound()?)
}

/// HTTP status mirroring the transport's error classification.
fn status_for(error: &TransportError) -> u16 {
    match error.kind {
        ErrorKind::Validation | ErrorKind::UnsupportedFeature => 400,
        ErrorKind::Authentication => 401,
        ErrorKind::Authorization => 403,
        ErrorKind::RateLimited => 429,
        ErrorKind::Timeout | ErrorKind::TransientNetwork | ErrorKind::TransientProvider => 503,
        _ => 502,
    }
}

fn query(url: &Url, key: &str) -> Option<String> {
    url.query_pairs()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.into_owned())
}
