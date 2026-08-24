use email_message::{Address, Body, Message, OutboundMessage};
use email_transport::{SendOptions, SendReport, Transport, TransportError};
use email_transport_resend::ResendTransport;
use email_transport_restate::{InvocationMode, RestateTransport, TransportKey};

async fn send_application_email<T: Transport>(
    transport: &T,
    message: &OutboundMessage,
) -> Result<SendReport, TransportError> {
    transport.send(message, &SendOptions::default()).await
}

fn message(recipient: &str) -> Result<OutboundMessage, Box<dyn std::error::Error>> {
    let message = Message::builder(Body::text("The same call works with either transport."))
        .from_mailbox("onboarding@resend.dev".parse()?)
        .to(vec![Address::Mailbox(recipient.parse()?)])
        .subject("Direct or durable")
        .build()?;

    Ok(OutboundMessage::new(message)?)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let recipient = std::env::var("EMAIL_TO")?;
    let message = message(&recipient)?;

    if let Ok(ingress_url) = std::env::var("RESTATE_INGRESS_URL") {
        // Set RESTATE_WAIT=1 to wait for the worker's provider report instead
        // of returning as soon as Restate has queued the invocation.
        let invocation_mode = if std::env::var_os("RESTATE_WAIT").is_some() {
            InvocationMode::Sent
        } else {
            InvocationMode::Queued
        };
        let mut builder =
            RestateTransport::builder(TransportKey::new("transactional")?, ingress_url.parse()?)
                .invocation_mode(invocation_mode);
        // Restate Cloud ingress requires an API key as a bearer token.
        if let Ok(auth_token) = std::env::var("RESTATE_AUTH_TOKEN") {
            builder = builder.bearer_token(auth_token);
        }
        let transport = builder.build();
        let report = send_application_email(&transport, &message).await?;
        match RestateTransport::invocation_id(&report) {
            Some(invocation_id) => println!("queued as Restate invocation {invocation_id}"),
            None => println!("sent durably through {}", report.provider),
        }
    } else {
        let transport = ResendTransport::new(std::env::var("RESEND_API_KEY")?);
        let report = send_application_email(&transport, &message).await?;
        println!("sent directly through {}", report.provider);
    }

    Ok(())
}
