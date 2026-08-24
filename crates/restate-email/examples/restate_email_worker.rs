use std::time::Duration;

use email_kit::attachment::{MapResolver, ResolvingTransport, SchemeRouter};
use email_message::ContentType;
use email_message::{
    Address, Attachment, AttachmentBody, AttachmentReference, Body, EmailAddress, Envelope,
    Message, OutboundMessage,
};
use email_transport::{ErrorKind, SendOptions, SendReport, Transport, TransportError};
use restate_email::{
    CorrelationId, IdempotencyKey, SendRequest, Service, StaticTransportRegistry, TransportKey,
};
use restate_sdk::{endpoint::Endpoint, http_server::HttpServer, service::IntoServiceDefinition};

struct ExampleTransport;

impl Transport for ExampleTransport {
    fn send<'a>(
        &'a self,
        message: &'a email_message::OutboundMessage,
        _options: &'a SendOptions,
    ) -> impl core::future::Future<Output = Result<SendReport, TransportError>> + Send + 'a {
        Box::pin(async move { example_send_report(message.as_message()) })
    }
}

fn example_send_report(message: &Message) -> Result<SendReport, TransportError> {
    for attachment in message.attachments() {
        if !matches!(attachment.body(), AttachmentBody::Bytes(_)) {
            return Err(TransportError::new(
                ErrorKind::UnsupportedFeature,
                "example transport only accepts byte-backed attachments",
            ));
        }
    }

    let envelope = message.derive_envelope().map_err(|error| {
        TransportError::new(ErrorKind::Validation, error.to_string()).with_source(error)
    })?;

    Ok(SendReport::new("example-worker")
        .with_provider_message_id("example-message-id")
        .with_accepted(envelope.rcpt_to().to_vec()))
}

fn sample_request() -> Result<SendRequest, Box<dyn std::error::Error>> {
    let message = Message::builder(Body::html(String::from(
        "<p>Hello from the Restate worker example.</p>",
    )))
    .from_mailbox("sender@example.com".parse()?)
    .to(vec![Address::Mailbox("recipient@example.com".parse()?)])
    .subject("Restate worker example")
    .add_attachment(
        Attachment::reference(
            ContentType::try_from("text/plain")?,
            AttachmentReference::new("example:report.txt"),
        )
        .with_filename("report.txt"),
    )
    .build()?;

    let mut options = SendOptions::default();
    options.envelope = Some(Envelope::new(
        Some("bounce@example.com".parse::<EmailAddress>()?),
        vec!["recipient@example.com".parse::<EmailAddress>()?],
    ));
    options.timeout = Some(Duration::from_secs(5));
    options.idempotency_key = Some(IdempotencyKey::new("example-request-1")?);
    options.correlation_id = Some(CorrelationId::new("example-correlation-1")?);

    Ok(SendRequest {
        transport: TransportKey::new("transactional")?,
        message: OutboundMessage::new(message)?,
        options,
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let resolver = SchemeRouter::new().with_resolver(
        "example",
        MapResolver::new().with_entry(
            "report.txt",
            b"Hello from the resolved sample attachment.\n".to_vec(),
        ),
    );
    let mut registry = StaticTransportRegistry::new();
    registry.insert(
        "transactional",
        ResolvingTransport::new(ExampleTransport, resolver),
    );

    let service = Service::new(registry).into_service_definition();
    let endpoint = Endpoint::builder().bind(service);
    let address = std::env::var("RESTATE_EMAIL_WORKER_ADDR")
        .unwrap_or_else(|_| String::from("127.0.0.1:9080"))
        .parse()?;
    let request = sample_request()?;

    println!("Restate worker example listening on http://{address}");
    println!("Register this SDK endpoint with Restate, then invoke through Restate ingress.");
    println!(
        "Restate ingress paths: POST /restate/send/Email/send (queue) or /restate/call/Email/send (wait)"
    );
    println!(
        "Sample request body:\n{}",
        serde_json::to_string_pretty(&request)?
    );

    HttpServer::new(endpoint.build())
        .listen_and_serve(address)
        .await;

    Ok(())
}
