use std::time::Duration;

use email_kit::transport::transport_option_registry;
use email_message::{Address, Attachment, Body, ContentType, EmailAddress, Envelope, Message};
use restate_email::{CorrelationId, IdempotencyKey, SendOptions, SendRequest, TransportKey};

fn fixture_message() -> Result<email_message::OutboundMessage, Box<dyn std::error::Error>> {
    let message = Message::builder(Body::text("hello"))
        .from_mailbox("from@example.com".parse()?)
        .add_to(Address::Mailbox("to@example.com".parse()?))
        .subject("Release fixture")
        .add_attachment(
            Attachment::bytes(ContentType::try_from("text/plain")?, b"report".to_vec())
                .with_filename("report.txt"),
        )
        .build()?;

    Ok(email_message::OutboundMessage::new(message)?)
}

fn fixture_envelope() -> Result<Envelope, Box<dyn std::error::Error>> {
    Ok(Envelope::new(
        Some("bounce@example.com".parse::<EmailAddress>()?),
        vec!["to@example.com".parse::<EmailAddress>()?],
    ))
}

fn base_fixture_request() -> Result<SendRequest, Box<dyn std::error::Error>> {
    let send_options = SendOptions::new()
        .with_envelope(fixture_envelope()?)
        .with_timeout(Duration::from_millis(2_500))
        .with_idempotency_key(IdempotencyKey::new("release-fixture-1")?)
        .with_correlation_id(CorrelationId::new("corr-fixture-1")?);

    Ok(SendRequest {
        transport: TransportKey::new("transactional")?,
        message: fixture_message()?,
        options: serde_json::from_value(serde_json::to_value(send_options)?)?,
    })
}

fn assert_matches_fixture(
    request: &SendRequest,
    fixture: &str,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let expected: serde_json::Value = serde_json::from_str(fixture)?;
    let actual = serde_json::to_value(request)?;

    assert_eq!(actual, expected);

    let decoded: SendRequest = serde_json::from_value(expected.clone())?;
    assert_eq!(serde_json::to_value(decoded)?, expected);

    Ok(expected)
}

#[test]
fn send_request_wire_fixture_matches_base_payload() -> Result<(), Box<dyn std::error::Error>> {
    let request = base_fixture_request()?;
    let expected =
        assert_matches_fixture(&request, include_str!("fixtures/send_request_base.json"))?;
    let decoded: SendRequest = serde_json::from_value(expected)?;
    let options = decoded
        .options
        .into_send_options(&transport_option_registry())?;

    assert_eq!(options.timeout, Some(Duration::from_millis(2_500)));
    assert_eq!(
        options.idempotency_key.as_ref().map(IdempotencyKey::as_str),
        Some("release-fixture-1")
    );

    Ok(())
}

#[cfg(feature = "resend")]
fn resend_fixture_request() -> Result<SendRequest, Box<dyn std::error::Error>> {
    use email_kit::transport::resend::ResendSendOptions;

    let mut send_options = SendOptions::new()
        .with_envelope(fixture_envelope()?)
        .with_timeout(Duration::from_millis(2_500))
        .with_idempotency_key(IdempotencyKey::new("release-fixture-1")?)
        .with_correlation_id(CorrelationId::new("corr-fixture-1")?);
    send_options
        .transport_options
        .insert(ResendSendOptions::new().with_tag("tenant", "blue"));

    Ok(SendRequest {
        transport: TransportKey::new("transactional")?,
        message: fixture_message()?,
        options: serde_json::from_value(serde_json::to_value(send_options)?)?,
    })
}

#[cfg(feature = "resend")]
#[test]
fn send_request_wire_fixture_matches_resend_payload() -> Result<(), Box<dyn std::error::Error>> {
    use email_kit::transport::resend::ResendSendOptions;

    let request = resend_fixture_request()?;
    let expected =
        assert_matches_fixture(&request, include_str!("fixtures/send_request_resend.json"))?;
    let decoded: SendRequest = serde_json::from_value(expected)?;
    let options = decoded
        .options
        .into_send_options(&transport_option_registry())?;
    let resend_options = options
        .transport_options
        .get::<ResendSendOptions>()
        .expect("resend options should hydrate");

    assert_eq!(resend_options.tags.len(), 1);
    assert_eq!(resend_options.tags[0].name, "tenant");
    assert_eq!(resend_options.tags[0].value, "blue");

    Ok(())
}
