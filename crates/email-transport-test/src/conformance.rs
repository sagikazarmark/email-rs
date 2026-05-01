//! Shared inputs for cross-provider conformance tests.
//!
//! Each provider crate hosts its own conformance test that runs against a
//! `wiremock::MockServer`. The provider-specific request-body assertions
//! live in those tests because the wire shapes differ. The pieces every
//! provider must agree on (the message under test, the canonical accepted
//! recipient list) live here so they cannot drift apart.

use email_message::{Address, Body, Header, Mailbox, Message, MessageId, OutboundMessage};
use time::OffsetDateTime;

/// The canonical `OutboundMessage` used by every provider's conformance test.
///
/// Hand it to the transport under test, then assert that the transport's
/// request to its mock server matches the provider's wire shape.
///
/// # Panics
///
/// Panics only if this crate's hard-coded fixture literals stop parsing or
/// validating. Callers do not supply input to this function.
#[must_use]
pub fn conformance_message() -> OutboundMessage {
    let message_id = "<conformance@example.com>"
        .parse::<MessageId>()
        .expect("message id should parse");

    Message::builder(Body::Text(String::from("Hello from conformance test")))
        .from_mailbox(mailbox("sender@example.com"))
        .sender(mailbox("bounce@example.com"))
        .to(vec![
            "Friends: a@example.com, b@example.com;"
                .parse::<Address>()
                .expect("group should parse"),
        ])
        .cc(vec![Address::Mailbox(mailbox("cc@example.com"))])
        .bcc(vec![Address::Mailbox(mailbox("hidden@example.com"))])
        .reply_to(vec![Address::Mailbox(mailbox("reply@example.com"))])
        .subject("Conformance")
        .date(OffsetDateTime::UNIX_EPOCH)
        .message_id(message_id)
        .add_header(Header::new("X-Test", "demo").expect("header should validate"))
        .build_outbound()
        .expect("message should validate")
}

/// The recipients every provider must report as accepted for
/// [`conformance_message`]. Order matters: the underlying group expansion
/// places To-group members first, then CC, then BCC.
pub const EXPECTED_ACCEPTED: &[&str] = &[
    "a@example.com",
    "b@example.com",
    "cc@example.com",
    "hidden@example.com",
];

fn mailbox(input: &str) -> Mailbox {
    input.parse().expect("mailbox should parse")
}
