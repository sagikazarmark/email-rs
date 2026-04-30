//! Property-based coverage for the validators added in MSG-002, MSG-003,
//! and MSG-010. The handwritten unit tests cover the canonical cases;
//! these proptests probe the boundaries between accept and reject.

use email_message::{Address, Body, Header, Mailbox, Message, MessageId, MessageValidationError};
use proptest::prelude::*;

fn mailbox(input: &str) -> Mailbox {
    input.parse::<Mailbox>().expect("test mailbox should parse")
}

fn address(input: &str) -> Address {
    input.parse::<Address>().expect("test address should parse")
}

fn build_with_subject(subject: &str) -> Result<Message, MessageValidationError> {
    Message::builder(Body::text("body"))
        .from_mailbox(mailbox("from@example.com"))
        .add_to(address("to@example.com"))
        .subject(subject.to_owned())
        .build()
}

fn build_with_header_name(name: &str) -> Result<Message, MessageValidationError> {
    // The header name itself is invalid (e.g. empty, contains
    // disallowed bytes); not the case we're stressing here.
    let Ok(header) = Header::new(name, "value") else {
        return Ok(Message::new(
            mailbox("from@example.com"),
            vec![address("to@example.com")],
            Body::text("body"),
        ));
    };
    Message::builder(Body::text("body"))
        .from_mailbox(mailbox("from@example.com"))
        .add_to(address("to@example.com"))
        .add_header(header)
        .build()
}

/// Subjects whose bytes are all printable-ASCII or tab.
fn safe_subject_strategy() -> impl Strategy<Value = String> {
    prop::collection::vec(prop_oneof![Just(b'\t'), 0x20u8..=0x7Eu8], 0..32)
        .prop_map(|bytes| String::from_utf8(bytes).expect("ASCII is utf-8"))
}

/// Subjects that contain at least one CR, LF, or non-tab control char.
fn unsafe_subject_strategy() -> impl Strategy<Value = String> {
    let unsafe_byte = prop_oneof![
        Just(b'\r'),
        Just(b'\n'),
        // 0x00..=0x08, 0x0B, 0x0C, 0x0E..=0x1F
        prop_oneof![0u8..=8u8, Just(0x0Bu8), Just(0x0Cu8), 0x0Eu8..=0x1Fu8],
    ];
    (
        // padding before
        safe_subject_strategy(),
        unsafe_byte,
        // padding after
        safe_subject_strategy(),
    )
        .prop_map(|(prefix, bad, suffix)| {
            let mut bytes = prefix.into_bytes();
            bytes.push(bad);
            bytes.extend(suffix.into_bytes());
            String::from_utf8(bytes).expect("ASCII bytes are utf-8")
        })
}

/// Header names that are syntactically valid but are NOT in the kernel's
/// RFC 5322 §3.6 reserved list.
fn safe_custom_header_name_strategy() -> impl Strategy<Value = String> {
    "X-[A-Za-z]{1,16}"
}

/// Picks a name from the reserved list (case may vary).
fn reserved_header_name_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("From"),
        Just("Sender"),
        Just("Reply-To"),
        Just("To"),
        Just("Cc"),
        Just("Bcc"),
        Just("Date"),
        Just("Subject"),
        Just("Message-ID"),
        Just("from"),
        Just("subject"),
        Just("MESSAGE-ID"),
        Just("Bcc"),
    ]
    .prop_map(str::to_owned)
}

/// Bracketed message-IDs whose local and domain parts are valid dot-atoms.
fn safe_message_id_strategy() -> impl Strategy<Value = String> {
    let atom = "[a-z0-9]{1,12}";
    (atom, atom).prop_map(|(local, domain)| format!("<{local}@{domain}>"))
}

proptest! {
    #[test]
    fn safe_subjects_pass_validation(subject in safe_subject_strategy()) {
        prop_assert!(build_with_subject(&subject).is_ok(), "subject: {subject:?}");
    }

    #[test]
    fn unsafe_subjects_fail_validation(subject in unsafe_subject_strategy()) {
        let error = build_with_subject(&subject).expect_err("unsafe subject must fail");
        prop_assert_eq!(error, MessageValidationError::SubjectContainsInvalidChars);
    }

    #[test]
    fn safe_custom_header_names_pass(name in safe_custom_header_name_strategy()) {
        prop_assert!(build_with_header_name(&name).is_ok(), "name: {name}");
    }

    #[test]
    fn reserved_header_names_fail(name in reserved_header_name_strategy()) {
        let error = build_with_header_name(&name).expect_err("reserved name must fail");
        let is_reserved = matches!(error, MessageValidationError::ReservedHeaderName { .. });
        prop_assert!(is_reserved);
    }

    #[test]
    fn safe_message_ids_parse_and_roundtrip(id in safe_message_id_strategy()) {
        let parsed: MessageId = id.parse().expect("safe message-id must parse");
        prop_assert_eq!(parsed.as_str(), id.as_str());
    }

    #[test]
    fn message_ids_without_brackets_are_rejected(s in "[a-z]{1,8}@[a-z]{1,8}\\.[a-z]{2,4}") {
        let parsed = s.parse::<MessageId>();
        prop_assert!(parsed.is_err(), "input {s} should not parse without brackets");
    }
}
