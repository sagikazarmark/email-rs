use std::time::Duration;

use email_kit::transport::resend::ResendTransport;
use email_message::ContentType;
use email_message::{Address, Attachment, Body, EmailAddress, Envelope, Message, OutboundMessage};
use email_transport::SendOptions;
use restate_email::{
    CorrelationId, IdempotencyKey, SendRequest, Service, StaticTransportRegistry, TransportKey,
};
use restate_sdk::{endpoint::Endpoint, http_server::HttpServer, service::IntoServiceDefinition};

fn sample_request(from: &str, to: &str) -> Result<SendRequest, Box<dyn std::error::Error>> {
    let message = Message::builder(Body::html(String::from(
        "<p>Hello from the Restate Resend worker example.</p>",
    )))
    .from_mailbox(from.parse()?)
    .to(vec![Address::Mailbox(to.parse()?)])
    .subject("Restate Resend worker example")
    .add_attachment(
        Attachment::bytes(
            ContentType::try_from("text/plain")?,
            b"Hello from the sample attachment.\n".to_vec(),
        )
        .with_filename("report.txt"),
    )
    .build()?;

    let mut options = SendOptions::default();
    options.envelope = Some(Envelope::new(
        Some(from.parse::<EmailAddress>()?),
        vec![to.parse::<EmailAddress>()?],
    ));
    options.timeout = Some(Duration::from_secs(5));
    options.idempotency_key = Some(IdempotencyKey::new("example-resend-request-1")?);
    options.correlation_id = Some(CorrelationId::new("example-resend-correlation-1")?);

    Ok(SendRequest {
        transport: TransportKey::new("resend-default")?,
        message: OutboundMessage::new(message)?,
        options,
    })
}

/// Restate request identity public keys from `RESTATE_IDENTITY_KEY`.
///
/// Accepts one key or a comma-separated list. With at least one key the
/// endpoint rejects unsigned requests; listing the old and the new key keeps
/// both valid during rotation.
fn identity_keys() -> Vec<String> {
    std::env::var("RESTATE_IDENTITY_KEY")
        .map(|keys| {
            keys.split([',', ' '])
                .filter(|key| !key.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_key = std::env::var("RESEND_API_KEY")?;
    let from = std::env::var("RESEND_FROM")?;
    let to = std::env::var("RESEND_TO")?;

    let mut registry = StaticTransportRegistry::new();
    registry.insert("resend-default", ResendTransport::new(api_key));

    let service = Service::new(registry).into_service_definition();
    let mut endpoint = Endpoint::builder().bind(service);
    // Verify Restate's request identity; multiple keys allow rotation.
    let identity_keys = identity_keys();
    for identity_key in &identity_keys {
        endpoint = endpoint.identity_key(identity_key)?;
    }
    let address = "127.0.0.1:9081".parse()?;
    let request = sample_request(&from, &to)?;

    println!("Restate Resend worker example listening on http://{address}");
    println!("Register this SDK endpoint with Restate, then invoke through Restate ingress.");
    println!(
        "Restate ingress paths: POST /restate/send/Email/send (queue) or /restate/call/Email/send (wait)"
    );
    println!(
        "Sample request body:\n{}",
        serde_json::to_string_pretty(&request)?
    );
    if !identity_keys.is_empty() {
        println!(
            "Request identity verification enabled with {} key(s).",
            identity_keys.len()
        );
    }

    HttpServer::new(endpoint.build())
        .listen_and_serve(address)
        .await;

    Ok(())
}
