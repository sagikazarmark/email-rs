use email_message::{Address, Body, Mailbox, Message, MessageId};
use email_message_wire::{RenderOptions, parse_rfc822, render_rfc822, render_rfc822_with};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc2822;

#[test]
fn parse_rfc822_extracts_core_headers_and_body() {
    let input = concat!(
        "From: Mary Smith <mary@x.test>\r\n",
        "To: jdoe@one.test\r\n",
        "Subject: Test\r\n",
        "Date: Fri, 06 Mar 2026 12:00:00 +0000\r\n",
        "Message-ID: <test@example.com>\r\n",
        "X-Custom: demo\r\n",
        "\r\n",
        "hello"
    );

    let message = parse_rfc822(input.as_bytes()).expect("message should parse");
    let expected_from = "Mary Smith <mary@x.test>"
        .parse::<Mailbox>()
        .expect("valid mailbox");

    assert_eq!(message.from_mailbox(), Some(&expected_from));
    assert_eq!(
        message.to(),
        &[Address::Mailbox(
            "jdoe@one.test".parse::<Mailbox>().expect("valid mailbox")
        )]
    );
    assert_eq!(message.subject(), Some("Test"));
    assert_eq!(
        message.date(),
        Some(
            &OffsetDateTime::parse("Fri, 06 Mar 2026 12:00:00 +0000", &Rfc2822)
                .expect("date should parse")
        )
    );
    assert_eq!(
        message.message_id(),
        Some(
            &"<test@example.com>"
                .parse::<MessageId>()
                .expect("message id should parse")
        )
    );
    assert_eq!(
        message.headers(),
        &[email_message::Header::new("X-Custom", "demo").expect("header should validate")]
    );
    assert_eq!(message.body(), &Body::Text("hello".to_owned()));
}

#[test]
fn render_rfc822_writes_expected_lines() {
    let date = OffsetDateTime::parse("Fri, 06 Mar 2026 12:00:00 +0000", &Rfc2822)
        .expect("date should parse");
    let message_id = "<test@example.com>"
        .parse::<MessageId>()
        .expect("message id should parse");

    let message = Message::builder(Body::Text("hello".to_owned()))
        .from_mailbox(
            "Mary Smith <mary@x.test>"
                .parse::<Mailbox>()
                .expect("valid mailbox"),
        )
        .to(vec![Address::Mailbox(
            "jdoe@one.test".parse::<Mailbox>().expect("valid mailbox"),
        )])
        .subject("Test")
        .date(date)
        .message_id(message_id)
        .headers(vec![
            email_message::Header::new("X-Custom", "demo").expect("header should validate"),
        ])
        .build()
        .expect("message should validate");

    let rendered = render_rfc822(&message).expect("render should succeed");
    let text = String::from_utf8(rendered).expect("rendered text should be utf8");

    assert!(text.contains("From: \"Mary Smith\" <mary@x.test>\r\n"));
    assert!(text.contains("To: jdoe@one.test\r\n"));
    assert!(text.contains("Subject: Test\r\n"));
    assert!(text.contains("Date: Fri, 06 Mar 2026 12:00:00 +0000\r\n"));
    assert!(text.contains("Message-ID: <test@example.com>\r\n"));
    assert!(text.contains("X-Custom: demo\r\n"));
    assert!(text.ends_with("\r\n\r\nhello"));
}

#[test]
fn render_rfc822_strips_bcc_by_default() {
    let message = Message::builder(Body::Text("hello".to_owned()))
        .from_mailbox(
            "from@example.com"
                .parse::<Mailbox>()
                .expect("valid mailbox"),
        )
        .to(vec![Address::Mailbox(
            "to@example.com".parse::<Mailbox>().expect("valid mailbox"),
        )])
        .add_bcc(Address::Mailbox(
            "hidden@example.com"
                .parse::<Mailbox>()
                .expect("valid mailbox"),
        ))
        .build()
        .expect("message should validate");

    let rendered = render_rfc822(&message).expect("render should succeed");
    let text = String::from_utf8(rendered).expect("rendered text should be utf8");

    assert!(
        !text.contains("Bcc:"),
        "Bcc header must be omitted by default"
    );
}

#[test]
fn render_rfc822_can_include_bcc_when_requested() {
    let message = Message::builder(Body::Text("hello".to_owned()))
        .from_mailbox(
            "from@example.com"
                .parse::<Mailbox>()
                .expect("valid mailbox"),
        )
        .to(vec![Address::Mailbox(
            "to@example.com".parse::<Mailbox>().expect("valid mailbox"),
        )])
        .add_bcc(Address::Mailbox(
            "hidden@example.com"
                .parse::<Mailbox>()
                .expect("valid mailbox"),
        ))
        .build()
        .expect("message should validate");

    let rendered = render_rfc822_with(&message, &RenderOptions::new().with_include_bcc(true))
        .expect("render should succeed");
    let text = String::from_utf8(rendered).expect("rendered text should be utf8");

    assert!(text.contains("Bcc: hidden@example.com\r\n"));
}
