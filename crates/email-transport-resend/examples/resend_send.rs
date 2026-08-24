use email_message::{Address, Body, Message};
use email_transport::{IdempotencyKey, SendOptions, Transport};
use email_transport_resend::{ResendSendOptions, ResendTemplate, ResendTransport};

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

    // Tags and templates live on one `ResendSendOptions` value; inserting a
    // second value of the same type would replace the first one.
    let mut resend_options = ResendSendOptions::new().with_tag("env", "local");
    if let Ok(template_id) = std::env::var("RESEND_TEMPLATE_ID") {
        resend_options = resend_options
            .with_template(ResendTemplate::new(template_id).with_variables([("name", "Mark")]));
    }

    let options = SendOptions::new()
        .with_idempotency_key(IdempotencyKey::new("example-idempotency-key")?)
        .with_transport_option(resend_options);

    let transport = ResendTransport::new(api_key);
    let report = transport.send(&message, &options).await?;

    println!("provider: {}", report.provider);
    println!("message id: {:?}", report.provider_message_id);
    println!("accepted: {:?}", report.accepted);

    Ok(())
}
