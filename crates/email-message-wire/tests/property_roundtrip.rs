//! Property-based coverage for `email-message-wire` round-trip
//! invariants. The handwritten edge-case suite covers canonical
//! shapes; these proptests stress the boundaries of header folding,
//! RFC 2047 encoded-word handling, and base64 transfer-encoding.

use email_message::{Address, Body, ContentType, Mailbox, Message, MimePart};
use email_message_wire::{parse_rfc822, render_rfc822};
use proptest::prelude::*;

fn mailbox(input: &str) -> Mailbox {
    input.parse::<Mailbox>().expect("test mailbox should parse")
}

/// Generate addr-spec-shaped strings.
fn email_strategy() -> impl Strategy<Value = String> {
    ("[a-z]{1,8}", "[a-z]{1,8}", "[a-z]{2,4}")
        .prop_map(|(local, d1, d2)| format!("{local}@{d1}.{d2}"))
}

/// Printable-ASCII subject strings with at least one non-whitespace
/// character at start and end. Header rendering normalizes
/// surrounding whitespace, so all-whitespace subjects don't round-trip
/// byte-for-byte.
fn ascii_subject_strategy() -> impl Strategy<Value = String> {
    "[A-Za-z0-9?@!_,.+\\-][A-Za-z0-9 ?@!_,.+\\-]{0,150}[A-Za-z0-9?@!_,.+\\-]"
}

/// Arbitrary UTF-8 subject strings, guaranteed to contain at least one
/// non-ASCII char so the RFC 2047 encoded-word path fires during
/// render. No leading/trailing whitespace (header normalization
/// trims it).
fn utf8_subject_strategy() -> impl Strategy<Value = String> {
    "[A-Za-z0-9_]?[\\u{00C0}-\\u{00FF}\\u{4E00}-\\u{4E0F}\\u{0905}-\\u{091F}][\\u{00C0}-\\u{00FF}\\u{4E00}-\\u{4E0F}\\u{0905}-\\u{091F}A-Za-z]{0,40}"
}

/// Arbitrary bytes for an attachment payload, forces base64 encode/decode.
fn attachment_bytes_strategy() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(any::<u8>(), 0..512)
}

fn message_with_subject(from: &str, to: &str, subject: &str) -> Message {
    Message::builder(Body::text("body"))
        .from_mailbox(mailbox(from))
        .add_to(Address::Mailbox(mailbox(to)))
        .subject(subject.to_owned())
        .build()
        .expect("subject must validate")
}

proptest! {
    /// Pure ASCII subjects round-trip exactly: render then parse, and
    /// the parser sees the same subject string.
    #[test]
    fn ascii_subject_render_parse_roundtrip(
        from in email_strategy(),
        to in email_strategy(),
        subject in ascii_subject_strategy(),
    ) {
        // Subject from the strategy is guaranteed CRLF-free; if
        // validate_basic rejects (subject ends with `.` etc.) skip.
        let Ok(message) = Message::builder(Body::text("body"))
            .from_mailbox(mailbox(&from))
            .add_to(Address::Mailbox(mailbox(&to)))
            .subject(subject.clone())
            .build() else { return Ok(()); };

        let rendered = render_rfc822(&message).expect("render must succeed");
        let parsed = parse_rfc822(&rendered).expect("rendered bytes must parse");
        prop_assert_eq!(parsed.subject(), Some(subject.as_str()));
    }

    /// UTF-8 subjects survive the RFC 2047 encode → decode path.
    #[test]
    fn utf8_subject_encoded_word_roundtrip(
        from in email_strategy(),
        to in email_strategy(),
        subject in utf8_subject_strategy(),
    ) {
        let message = message_with_subject(&from, &to, &subject);
        let rendered = render_rfc822(&message).expect("render must succeed");
        let rendered_text = String::from_utf8(rendered.clone())
            .expect("rendered headers are ASCII");
        // RFC 2047 encoding must have fired for non-ASCII subjects.
        prop_assert!(
            rendered_text.contains("=?utf-8?B?"),
            "non-ASCII subject must encode as RFC 2047 base64 word: {rendered_text}",
        );

        let parsed = parse_rfc822(&rendered).expect("rendered bytes must parse");
        prop_assert_eq!(parsed.subject(), Some(subject.as_str()));
    }

    /// UTF-8 display names survive RFC 2047 phrase encoding.
    #[test]
    fn utf8_display_name_roundtrip(
        local in "[a-z]{1,8}",
        domain in "[a-z]{1,8}\\.[a-z]{2,4}",
        display in utf8_subject_strategy(),
    ) {
        let from_str = format!("\"{display}\" <{local}@{domain}>");
        let Ok(from_mb) = from_str.parse::<Mailbox>() else { return Ok(()); };
        let message = Message::builder(Body::text("body"))
            .from_mailbox(from_mb)
            .add_to(Address::Mailbox(mailbox("to@x.test")))
            .build()
            .expect("message must validate");

        let rendered = render_rfc822(&message).expect("render must succeed");
        let parsed = parse_rfc822(&rendered).expect("rendered bytes must parse");
        let parsed_from = parsed.from_mailbox().expect("From must round-trip");
        prop_assert_eq!(parsed_from.email().as_str(), format!("{local}@{domain}"));
        prop_assert_eq!(parsed_from.name(), Some(display.as_str()));
    }

    /// Arbitrary attachment payloads survive base64 round-trip through
    /// the multipart MIME body path.
    #[test]
    fn attachment_bytes_base64_roundtrip(
        from in email_strategy(),
        to in email_strategy(),
        payload in attachment_bytes_strategy(),
    ) {
        use email_message::{Attachment, AttachmentBody};

        let attachment = Attachment::new(
            ContentType::try_from("application/octet-stream")
                .expect("content type parses"),
            AttachmentBody::Bytes(payload.clone()),
        )
        .with_filename("blob.bin");

        let message = Message::builder(Body::text("body"))
            .from_mailbox(mailbox(&from))
            .add_to(Address::Mailbox(mailbox(&to)))
            .add_attachment(attachment)
            .build()
            .expect("message must validate");

        let rendered = render_rfc822(&message).expect("render must succeed");
        let parsed = parse_rfc822(&rendered).expect("rendered bytes must parse");

        // The attachment is now nested inside a Body::Mime tree; walk
        // and assert at least one Leaf carries the original payload.
        let Body::Mime(part) = parsed.body() else {
            prop_assert!(false, "expected Body::Mime after attachment round-trip");
            unreachable!()
        };

        prop_assert!(
            mime_contains_bytes(part, &payload),
            "attachment payload must round-trip through base64",
        );
    }
}

fn mime_contains_bytes(part: &MimePart, needle: &[u8]) -> bool {
    match part {
        MimePart::Leaf { body, .. } => body == needle,
        MimePart::Multipart { parts, .. } => parts.iter().any(|p| mime_contains_bytes(p, needle)),
    }
}
