use std::time::Duration;

use email_message::ContentType;
use email_message::{
    Address, Attachment, AttachmentReference, Body, EmailAddress, Envelope, Message,
    OutboundMessage,
};
use email_transport::{CorrelationId, IdempotencyKey, SendOptions};
use email_transport_restate::{SendRequest, SendResponse, TransportKey};

fn sample_request() -> Result<SendRequest, Box<dyn std::error::Error>> {
    let message = Message::builder(Body::html(String::from(
        "<p>Hello from the local invocation example.</p>",
    )))
    .from_mailbox("sender@example.com".parse()?)
    .to(vec![Address::Mailbox("recipient@example.com".parse()?)])
    .subject("Local invocation example")
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
    let base_url = std::env::var("RESTATE_INGRESS_URL")
        .unwrap_or_else(|_| String::from("http://127.0.0.1:8080"));
    let request_url = format!("{}/restate/call/Email/send", base_url.trim_end_matches('/'));
    let request = sample_request()?;

    println!("POST {request_url}");
    println!("This client targets Restate ingress, not the raw SDK endpoint.");
    println!("The call path waits for the worker; /restate/send/Email/send would only queue it.");
    println!("Request body:\n{}", serde_json::to_string_pretty(&request)?);

    let mut request_builder = reqwest::Client::new().post(&request_url).json(&request);
    // Restate Cloud ingress requires an API key as a bearer token.
    if let Ok(auth_token) = std::env::var("RESTATE_AUTH_TOKEN") {
        request_builder = request_builder.bearer_auth(auth_token);
    }
    let response = request_builder.send().await?;
    let status = response.status();
    let headers = response.headers().clone();
    let body = response.text().await?;

    println!("status: {status}");
    println!("response headers: {headers:#?}");
    println!("response body:\n{body}");

    if !status.is_success() {
        return Err(format!("Restate ingress returned {status}").into());
    }

    let payload: SendResponse = serde_json::from_str(&body)?;
    if payload.report.provider != "example-worker" {
        return Err(format!(
            "unexpected provider in response: {}",
            payload.report.provider
        )
        .into());
    }
    if payload.report.provider_message_id.as_deref() != Some("example-message-id") {
        return Err(format!(
            "unexpected provider message id in response: {:?}",
            payload.report.provider_message_id
        )
        .into());
    }

    Ok(())
}
