use email_message::{Address, Body, Message};
use email_transport::{SendOptions, Transport};
use email_transport_lettre::LettreTransport;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let smtp_url = std::env::var("SMTP_URL")?;
    let to = std::env::var("SMTP_TO")?;

    let message = Message::builder(Body::text("Hello from email-rs"))
        .from_mailbox("sender@example.com".parse()?)
        .to(vec![Address::Mailbox(to.parse()?)])
        .subject("Lettre transport example")
        .build_outbound()?;

    let report = LettreTransport::from_url(&smtp_url)?
        .send(&message, &SendOptions::default())
        .await?;

    println!(
        "provider={} accepted={:?}",
        report.provider, report.accepted
    );
    Ok(())
}
