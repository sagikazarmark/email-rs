use std::time::Duration;

use email_message::ContentType;
use email_message::{Address, Attachment, Body, EmailAddress, Envelope, Message, OutboundMessage};
use restate_email::{
    CorrelationId, IdempotencyKey, RawSendOptions, SendRequest, SendResponse, TransportKey,
};

fn sample_request() -> Result<SendRequest, Box<dyn std::error::Error>> {
    let message = Message::builder(Body::html(String::from(
        "<p>Hello from the local invocation example.</p>",
    )))
    .from_mailbox("sender@example.com".parse()?)
    .to(vec![Address::Mailbox("recipient@example.com".parse()?)])
    .subject("Local invocation example")
    .add_attachment(
        Attachment::bytes(
            ContentType::try_from("text/plain")?,
            b"Hello from the sample attachment.\n".to_vec(),
        )
        .with_filename("report.txt"),
    )
    .build()?;

    let mut options = RawSendOptions::default();
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
    let request_url = format!("{}/Email/send", base_url.trim_end_matches('/'));
    let request = sample_request()?;

    println!("POST {request_url}");
    println!("This client targets Restate ingress, not the raw SDK endpoint.");
    println!("Request body:\n{}", serde_json::to_string_pretty(&request)?);

    let response = reqwest::Client::new()
        .post(&request_url)
        .json(&request)
        .send()
        .await?;
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
