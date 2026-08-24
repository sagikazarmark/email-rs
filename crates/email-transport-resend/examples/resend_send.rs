use email_message::{Address, Body, Message};
use email_transport::{IdempotencyKey, SendOptions, Transport};
use email_transport_resend::{ResendSendOptions, ResendTransport};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_key = std::env::var("RESEND_API_KEY")?;
    let from = "onboarding@resend.dev";
    let to = std::env::var("RESEND_TO")?;

    let message = Message::builder(Body::Html(String::from(
        "<p>Congrats on sending your <strong>first email</strong>!</p>",
    )))
    .from_mailbox(from.parse()?)
    .to(vec![Address::Mailbox(to.parse()?)])
    .subject("Hello World")
    .build_outbound()?;

    let options = SendOptions::new()
        .with_idempotency_key(IdempotencyKey::new("example-idempotency-key")?)
        .with_transport_option(ResendSendOptions::new().with_tag("env", "local"));
    // Templates go through the same option type:
    // .with_transport_option(
    //     ResendSendOptions::new().with_template(
    //         email_transport_resend::ResendTemplate::new("tmpl_123")
    //             .with_variables([("name", serde_json::json!("Mark"))]),
    //     ),
    // );

    let transport = ResendTransport::new(api_key);
    let report = transport.send(&message, &options).await?;

    println!("provider: {}", report.provider);
    println!("message id: {:?}", report.provider_message_id);
    println!("accepted: {:?}", report.accepted);

    Ok(())
}
