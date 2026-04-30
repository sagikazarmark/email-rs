use email_message::{Address, Body, ContentType, Header, Mailbox, Message, MimePart};
use email_message_wire::{
    MAX_INPUT_BYTES, MAX_MULTIPART_DEPTH, MAX_MULTIPART_PARTS, MessageParseError,
    MessageRenderError, parse_rfc822, render_rfc822,
};

fn mailbox(input: &str) -> Mailbox {
    input.parse::<Mailbox>().expect("mailbox should parse")
}

#[test]
fn parse_supports_folded_headers_and_recipient_merging() {
    let input = concat!(
        "From: Mary Smith <mary@x.test>\r\n",
        "To: jdoe@one.test\r\n",
        "To: john@two.test\r\n",
        "Subject: Hello\r\n",
        " world\r\n",
        "Date: Fri, 06 Mar 2026 12:00:00 +0000\r\n",
        "Message-ID: <id@example.com>\r\n",
        "\r\n",
        "Hello"
    );

    let parsed = parse_rfc822(input.as_bytes()).expect("message should parse");
    assert_eq!(parsed.subject(), Some("Hello world"));
    assert_eq!(parsed.to().len(), 2);
}

#[test]
fn parse_rejects_header_name_with_space_before_colon() {
    let input = concat!(
        "From: from@example.com\r\n",
        "To: to@example.com\r\n",
        "Subject : hello\r\n",
        "\r\n",
        "Hello"
    );

    assert!(parse_rfc822(input.as_bytes()).is_err());
}

#[test]
fn render_rejects_header_injection_inputs() {
    assert!(Header::new("X-Test", "hello\r\nBcc: victim@example.com").is_err());
}

#[test]
fn mime_multipart_roundtrip_works_through_public_api() {
    let mime = MimePart::Multipart {
        content_type: ContentType::try_from("multipart/alternative")
            .expect("content type should parse"),
        boundary: Some("edge-boundary".to_owned()),
        parts: vec![
            MimePart::Leaf {
                content_type: ContentType::try_from("text/plain")
                    .expect("content type should parse"),
                content_transfer_encoding: None,
                content_disposition: None,
                body: b"hello text".to_vec(),
            },
            MimePart::Leaf {
                content_type: ContentType::try_from("text/html")
                    .expect("content type should parse"),
                content_transfer_encoding: None,
                content_disposition: None,
                body: b"<p>hello html</p>".to_vec(),
            },
        ],
    };

    let message = Message::builder(Body::Mime(mime))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(mailbox("to@example.com"))])
        .build()
        .expect("message should validate");

    let rendered = render_rfc822(&message).expect("mime should render");
    let parsed = parse_rfc822(&rendered).expect("mime should parse");
    assert!(matches!(parsed.body(), Body::Mime(_)));
}

#[test]
fn render_mime_leaf_reencodes_base64_body_payload() {
    let mime = MimePart::Leaf {
        content_type: ContentType::try_from("text/plain;charset=utf-8")
            .expect("content type should parse"),
        content_transfer_encoding: Some(
            "base64"
                .parse()
                .expect("content-transfer-encoding should parse"),
        ),
        content_disposition: None,
        body: "ár".as_bytes().to_vec(),
    };

    let message = Message::builder(Body::Mime(mime))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(mailbox("to@example.com"))])
        .build()
        .expect("message should validate");

    let rendered = render_rfc822(&message).expect("render should succeed");
    let text = String::from_utf8(rendered.clone()).expect("rendered message should be utf8-safe");

    assert!(text.contains("Content-Transfer-Encoding: base64\r\n"));
    assert!(text.contains("\r\n\r\nw6Fy\r\n"));

    let reparsed = parse_rfc822(&rendered).expect("rendered message should parse");
    assert_eq!(reparsed.body(), &Body::Text("ár".to_owned()));
}

#[test]
fn render_mime_leaf_reencodes_quoted_printable_body_payload() {
    let mime = MimePart::Leaf {
        content_type: ContentType::try_from("text/plain;charset=utf-8")
            .expect("content type should parse"),
        content_transfer_encoding: Some(
            "quoted-printable"
                .parse()
                .expect("content-transfer-encoding should parse"),
        ),
        content_disposition: None,
        body: "Olá".as_bytes().to_vec(),
    };

    let message = Message::builder(Body::Mime(mime))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(mailbox("to@example.com"))])
        .build()
        .expect("message should validate");

    let rendered = render_rfc822(&message).expect("render should succeed");
    let text = String::from_utf8(rendered.clone()).expect("rendered message should be utf8-safe");

    assert!(text.contains("Content-Transfer-Encoding: quoted-printable\r\n"));
    assert!(text.contains("\r\n\r\nOl=C3=A1"));

    let reparsed = parse_rfc822(&rendered).expect("rendered message should parse");
    assert_eq!(reparsed.body(), &Body::Text("Olá".to_owned()));
}

#[test]
fn render_mime_leaf_quoted_printable_encodes_lone_cr_and_lf() {
    let raw = b"a\nb\rc\r\nd".to_vec();
    let mime = MimePart::Leaf {
        content_type: ContentType::try_from("application/octet-stream")
            .expect("content type should parse"),
        content_transfer_encoding: Some(
            "quoted-printable"
                .parse()
                .expect("content-transfer-encoding should parse"),
        ),
        content_disposition: None,
        body: raw.clone(),
    };

    let message = Message::builder(Body::Mime(mime))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(mailbox("to@example.com"))])
        .build()
        .expect("message should validate");

    let rendered = render_rfc822(&message).expect("render should succeed");
    let text = String::from_utf8(rendered.clone()).expect("rendered message should be utf8-safe");

    assert!(text.contains("\r\n\r\na=0Ab=0Dc\r\nd"));

    let reparsed = parse_rfc822(&rendered).expect("rendered message should parse");
    let Body::Mime(MimePart::Leaf { body, .. }) = reparsed.body() else {
        panic!("expected MIME leaf body")
    };
    assert_eq!(body, &raw);
}

#[test]
fn strict_date_and_message_id_validation_is_enforced() {
    use email_message_wire::MessageParseError;
    use std::error::Error;

    let invalid_date = concat!(
        "From: from@example.com\r\n",
        "To: to@example.com\r\n",
        "Date: 2026-03-06\r\n",
        "\r\n",
        "Hello"
    );
    let date_err = parse_rfc822(invalid_date.as_bytes()).expect_err("invalid date should error");
    assert!(matches!(date_err, MessageParseError::Date { .. }));
    assert!(
        date_err.source().is_some(),
        "Date variant must expose underlying time::error::Parse via source()"
    );

    let invalid_message_id = concat!(
        "From: from@example.com\r\n",
        "To: to@example.com\r\n",
        "Message-ID: <abc>\r\n",
        "\r\n",
        "Hello"
    );
    let msgid_err =
        parse_rfc822(invalid_message_id.as_bytes()).expect_err("invalid message-id should error");
    assert!(matches!(msgid_err, MessageParseError::MessageId { .. }));
    assert!(
        msgid_err.source().is_some(),
        "MessageId variant must expose underlying MessageIdParseError via source()"
    );
}

#[test]
fn parse_allows_non_utf8_payload_bytes() {
    let mut input = Vec::from(b"From: from@example.com\r\nTo: to@example.com\r\n\r\n".as_slice());
    input.extend_from_slice(&[0x66, 0x6f, 0x80, 0x6f]);

    let parsed = parse_rfc822(&input).expect("parser should accept non-utf8 body bytes");
    assert!(matches!(parsed.body(), Body::Text(_)));
}

#[test]
fn parse_decodes_rfc2047_subject() {
    let input = concat!(
        "From: from@example.com\r\n",
        "To: to@example.com\r\n",
        "Subject: =?utf-8?B?w6Fy?=\r\n",
        "\r\n",
        "Hello"
    );

    let parsed = parse_rfc822(input.as_bytes()).expect("message should parse");
    assert_eq!(parsed.subject(), Some("ár"));
}

#[test]
fn parse_decodes_rfc2047_display_names_in_addresses() {
    let input = concat!(
        "From: =?utf-8?B?Sm9zw6k=?= <from@example.com>\r\n",
        "To: =?utf-8?B?w6FydsOteg==?= <to@example.com>\r\n",
        "\r\n",
        "Hello"
    );

    let parsed = parse_rfc822(input.as_bytes()).expect("message should parse");
    assert_eq!(parsed.from_mailbox().and_then(Mailbox::name), Some("José"));
    match &parsed.to()[0] {
        Address::Mailbox(mailbox) => assert_eq!(mailbox.name(), Some("árvíz")),
        Address::Group(_) => panic!("expected mailbox"),
    }
}

/// RFC 2047 §5(3): an encoded-word MUST NOT appear within a quoted-string.
/// Implementations MUST treat such occurrences as literal. A display name
/// containing the literal seven-character sequence `=?utf-8?B?Zm9v?=`
/// renders as a quoted-string and must round-trip without decoding.
#[test]
fn parse_treats_encoded_word_inside_quoted_display_name_as_literal() {
    let input = concat!(
        "From: \"=?utf-8?B?Zm9v?=\" <from@example.com>\r\n",
        "To: to@example.com\r\n",
        "\r\n",
        "Hello"
    );

    let parsed = parse_rfc822(input.as_bytes()).expect("message should parse");
    assert_eq!(
        parsed.from_mailbox().and_then(Mailbox::name),
        Some("=?utf-8?B?Zm9v?=")
    );
}

#[test]
fn decode_rfc2047_phrase_decodes_generic_header_value_on_demand() {
    let input = concat!(
        "From: from@example.com\r\n",
        "To: to@example.com\r\n",
        "X-Note: =?utf-8?B?w6Fy?=\r\n",
        "\r\n",
        "Hello"
    );

    let parsed = parse_rfc822(input.as_bytes()).expect("message should parse");
    let x_note = parsed
        .headers()
        .iter()
        .find(|h| h.name().eq_ignore_ascii_case("x-note"))
        .expect("x-note should be preserved");
    // Default value is the raw on-the-wire bytes.
    assert_eq!(x_note.value(), "=?utf-8?B?w6Fy?=");
    // Opt-in decode resolves the encoded-word.
    assert_eq!(
        email_message_wire::decode_rfc2047_phrase(x_note.value()),
        "ár"
    );
}

#[test]
fn parse_preserves_raw_encoded_values_in_generic_headers() {
    let input = concat!(
        "From: from@example.com\r\n",
        "To: to@example.com\r\n",
        "X-Note: =?utf-8?B?w6Fy?=\r\n",
        "\r\n",
        "Hello"
    );

    let parsed = parse_rfc822(input.as_bytes()).expect("message should parse");
    let x_note = parsed
        .headers()
        .iter()
        .find(|h| h.name().eq_ignore_ascii_case("x-note"))
        .expect("x-note should be preserved");
    assert_eq!(x_note.value(), "=?utf-8?B?w6Fy?=");
}

#[test]
fn parse_rejects_non_ascii_header_octets() {
    let mut input =
        Vec::from(b"From: from@example.com\r\nTo: to@example.com\r\nX-Note: ".as_slice());
    input.push(0xE9);
    input.extend_from_slice(b"\r\n\r\nHello");

    assert!(parse_rfc822(&input).is_err());
}

#[test]
fn render_encodes_rfc2047_subject_and_display_name() {
    let message = Message::builder(Body::Text("Hello".to_owned()))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(Mailbox::from((
            Some("árvíz".to_owned()),
            "to@example.com".parse().expect("email"),
        )))])
        .subject("ár")
        .build()
        .expect("message should validate");

    let rendered = render_rfc822(&message).expect("render should succeed");
    let text = String::from_utf8(rendered).expect("rendered message should be utf8-safe");

    assert!(text.contains("Subject: =?utf-8?B?w6Fy?=\r\n"));
    assert!(text.contains("To: =?utf-8?B?w6FydsOteg==?= <to@example.com>\r\n"));
}

#[test]
fn render_splits_long_non_ascii_subject_into_multiple_encoded_words() {
    let subject = "árvíztűrő tükörfúrógép árvíztűrő tükörfúrógép árvíztűrő tükörfúrógép";
    let message = Message::builder(Body::Text("Hello".to_owned()))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(mailbox("to@example.com"))])
        .subject(subject)
        .build()
        .expect("message should validate");

    let rendered = render_rfc822(&message).expect("render should succeed");
    let text = String::from_utf8(rendered).expect("rendered message should be utf8-safe");

    let subject_line = text
        .split("\r\n")
        .find(|line| line.starts_with("Subject: "))
        .expect("subject header line should exist");
    let encoded_words: Vec<&str> = subject_line[9..].split(' ').collect();

    assert!(
        encoded_words.len() > 1,
        "subject should be split into encoded words"
    );
    for word in encoded_words {
        assert!(
            word.len() <= 75,
            "encoded-word exceeded RFC 2047 75-char limit"
        );
    }
}

#[test]
fn render_splits_long_non_ascii_display_name_into_multiple_encoded_words() {
    let long_name = "árvíztűrő tükörfúrógép árvíztűrő tükörfúrógép";
    let message = Message::builder(Body::Text("Hello".to_owned()))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(Mailbox::from((
            Some(long_name.to_owned()),
            "to@example.com".parse().expect("email"),
        )))])
        .build()
        .expect("message should validate");

    let rendered = render_rfc822(&message).expect("render should succeed");
    let text = String::from_utf8(rendered).expect("rendered message should be utf8-safe");

    let to_line = text
        .split("\r\n")
        .find(|line| line.starts_with("To: "))
        .expect("to header line should exist");
    let encoded_section = to_line
        .strip_prefix("To: ")
        .expect("to header prefix")
        .split(" <")
        .next()
        .expect("display-name section");
    let encoded_words: Vec<&str> = encoded_section.split(' ').collect();

    assert!(
        encoded_words.len() > 1,
        "display name should be split into encoded words"
    );
    for word in encoded_words {
        assert!(
            word.len() <= 75,
            "encoded-word exceeded RFC 2047 75-char limit"
        );
    }
}

#[test]
fn render_rejects_unfoldable_subject_exceeding_998_chars() {
    let message = Message::builder(Body::Text("Hello".to_owned()))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(mailbox("to@example.com"))])
        .subject("a".repeat(1200))
        .build()
        .expect("message should validate");

    let err = render_rfc822(&message).expect_err("render should fail");
    assert!(matches!(
        err,
        MessageRenderError::HeaderLineTooLong { name, .. } if name == "Subject"
    ));
}

#[test]
fn render_rejects_unfoldable_header_value_exceeding_998_chars() {
    let message = Message::builder(Body::Text("Hello".to_owned()))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(mailbox("to@example.com"))])
        .headers(vec![
            Header::new("X-Token", "a".repeat(1200)).expect("header should validate"),
        ])
        .build()
        .expect("message should validate");

    let err = render_rfc822(&message).expect_err("render should fail");
    assert!(matches!(
        err,
        MessageRenderError::HeaderLineTooLong { name, .. } if name == "X-Token"
    ));
}

#[test]
fn render_with_soft_fold_emits_continuation_lines_for_long_subject() {
    use email_message_wire::{RenderOptions, render_rfc822_with};

    // 200-char subject + Subject: prefix easily exceeds 78 chars but
    // stays well below 998. With soft-fold disabled (default) it lands
    // on one physical line; with soft_fold_at(78) it must wrap.
    let long_subject: String = "subject-word ".repeat(20);
    let message = Message::builder(Body::Text("Hello".to_owned()))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(mailbox("to@example.com"))])
        .subject(long_subject.trim())
        .build()
        .expect("message should validate");

    // Default, no soft fold; the Subject line stays one line.
    let default_render = render_rfc822(&message).expect("render should succeed");
    let default_text = String::from_utf8(default_render).expect("rendered should be utf-8");
    assert!(
        default_text.contains("Subject: subject-word subject-word"),
        "subject should appear on a single line by default"
    );
    let single_subject_line = default_text
        .lines()
        .find(|line| line.starts_with("Subject:"))
        .expect("subject line present");
    assert!(
        single_subject_line.len() > 100,
        "default subject line should be long, got {}",
        single_subject_line.len()
    );

    // With soft-fold target 78, the subject value continues on a folded
    // line (CRLF + leading whitespace).
    let folded_render = render_rfc822_with(&message, &RenderOptions::default().with_soft_fold(78))
        .expect("render with soft fold should succeed");
    let folded_text = String::from_utf8(folded_render).expect("rendered should be utf-8");

    let mut subject_block = String::new();
    let mut in_subject = false;
    for line in folded_text.split("\r\n") {
        if line.starts_with("Subject:") {
            in_subject = true;
            subject_block.push_str(line);
            continue;
        }
        if in_subject {
            if line.starts_with(' ') || line.starts_with('\t') {
                subject_block.push_str(line);
                continue;
            }
            break;
        }
    }
    // The subject value, reassembled, still contains the original words.
    assert!(
        subject_block.contains("subject-word subject-word"),
        "soft-folded subject should reassemble to the original value"
    );
    // Each physical Subject line is at most 78 chars (soft target).
    for line in folded_text.split("\r\n") {
        if line.starts_with("Subject:") || line.starts_with(' ') || line.starts_with('\t') {
            assert!(
                line.len() <= 78,
                "physical line longer than soft-fold target: {} chars: {line:?}",
                line.len()
            );
        }
    }
}

#[test]
fn render_non_ascii_text_without_attachments_sets_mime_headers_and_base64() {
    let body = "árvíztűrő tükörfúrógép";
    let message = Message::builder(Body::Text("árvíztűrő tükörfúrógép".to_owned()))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(mailbox("to@example.com"))])
        .build()
        .expect("message should validate");

    let rendered = render_rfc822(&message).expect("render should succeed");
    let text = String::from_utf8(rendered).expect("rendered message should be utf8-safe");

    assert!(text.contains("MIME-Version: 1.0\r\n"));
    assert!(text.contains("Content-Type: text/plain; charset=utf-8\r\n"));
    assert!(text.contains("Content-Transfer-Encoding: base64\r\n"));

    let reparsed = parse_rfc822(text.as_bytes()).expect("rendered message should parse");
    assert_eq!(reparsed.body(), &Body::Text(body.to_owned()));
}

#[test]
fn render_non_ascii_html_without_attachments_sets_mime_headers_and_base64() {
    let body = "<p>Olá</p>";
    let message = Message::builder(Body::Html("<p>Olá</p>".to_owned()))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(mailbox("to@example.com"))])
        .build()
        .expect("message should validate");

    let rendered = render_rfc822(&message).expect("render should succeed");
    let text = String::from_utf8(rendered).expect("rendered message should be utf8-safe");

    assert!(text.contains("MIME-Version: 1.0\r\n"));
    assert!(text.contains("Content-Type: text/html; charset=utf-8\r\n"));
    assert!(text.contains("Content-Transfer-Encoding: base64\r\n"));

    let reparsed = parse_rfc822(text.as_bytes()).expect("rendered message should parse");
    assert_eq!(reparsed.body(), &Body::Html(body.to_owned()));
}

#[test]
fn render_ascii_html_without_attachments_sets_mime_content_type() {
    let message = Message::builder(Body::Html("<p>Hello</p>".to_owned()))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(mailbox("to@example.com"))])
        .build()
        .expect("message should validate");

    let rendered = render_rfc822(&message).expect("render should succeed");
    let text = String::from_utf8(rendered).expect("rendered message should be utf8-safe");

    assert!(text.contains("MIME-Version: 1.0\r\n"));
    assert!(text.contains("Content-Type: text/html\r\n"));
}

#[test]
fn render_overlong_ascii_text_body_uses_quoted_printable() {
    let body = "a".repeat(1_200);
    let message = Message::builder(Body::Text(body.clone()))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(mailbox("to@example.com"))])
        .build()
        .expect("message should validate");

    let rendered = render_rfc822(&message).expect("render should succeed");
    let text = String::from_utf8(rendered.clone()).expect("rendered message should be utf8-safe");

    assert!(text.contains("MIME-Version: 1.0\r\n"));
    assert!(text.contains("Content-Type: text/plain\r\n"));
    assert!(text.contains("Content-Transfer-Encoding: quoted-printable\r\n"));
    for line in text.split("\r\n") {
        assert!(line.len() <= 998, "line exceeded RFC 5322 hard limit");
    }

    let parsed = parse_rfc822(&rendered).expect("rendered message should parse");
    assert_eq!(parsed.body(), &Body::Text(body));
}

#[test]
fn render_overlong_ascii_html_body_uses_quoted_printable() {
    let body = format!("<p>{}</p>", "a".repeat(1_200));
    let message = Message::builder(Body::Html(body.clone()))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(mailbox("to@example.com"))])
        .build()
        .expect("message should validate");

    let rendered = render_rfc822(&message).expect("render should succeed");
    let text = String::from_utf8(rendered.clone()).expect("rendered message should be utf8-safe");

    assert!(text.contains("Content-Type: text/html\r\n"));
    assert!(text.contains("Content-Transfer-Encoding: quoted-printable\r\n"));
    for line in text.split("\r\n") {
        assert!(line.len() <= 998, "line exceeded RFC 5322 hard limit");
    }

    let parsed = parse_rfc822(&rendered).expect("rendered message should parse");
    assert_eq!(parsed.body(), &Body::Html(body));
}

#[test]
fn parse_decodes_base64_top_level_text_body() {
    let input = concat!(
        "From: from@example.com\r\n",
        "To: to@example.com\r\n",
        "Content-Type: text/plain; charset=utf-8\r\n",
        "Content-Transfer-Encoding: base64\r\n",
        "\r\n",
        "w6Fy\r\n"
    );

    let parsed = parse_rfc822(input.as_bytes()).expect("message should parse");
    assert_eq!(parsed.body(), &Body::Text("ár".to_owned()));
}

#[test]
fn parse_preserves_top_level_non_text_body_as_mime_leaf() {
    let input = concat!(
        "From: from@example.com\r\n",
        "To: to@example.com\r\n",
        "Content-Type: application/json\r\n",
        "Content-Transfer-Encoding: base64\r\n",
        "\r\n",
        "eyJvayI6dHJ1ZX0=\r\n"
    );

    let parsed = parse_rfc822(input.as_bytes()).expect("message should parse");
    let Body::Mime(MimePart::Leaf {
        content_type,
        content_transfer_encoding,
        body,
        ..
    }) = parsed.body()
    else {
        panic!("expected MIME leaf body")
    };

    assert_eq!(content_type.as_str(), "application/json");
    assert_eq!(
        content_transfer_encoding
            .as_ref()
            .map(email_message::ContentTransferEncoding::as_str),
        Some("base64")
    );
    assert_eq!(body, br#"{"ok":true}"#);
}

#[test]
fn parse_ignores_non_alphabet_characters_in_base64_body() {
    let input = concat!(
        "From: from@example.com\r\n",
        "To: to@example.com\r\n",
        "Content-Type: text/plain; charset=utf-8\r\n",
        "Content-Transfer-Encoding: base64\r\n",
        "\r\n",
        "w6Fy!!@@\r\n"
    );

    let parsed = parse_rfc822(input.as_bytes()).expect("message should parse");
    assert_eq!(parsed.body(), &Body::Text("ár".to_owned()));
}

#[test]
fn parse_accepts_whitespace_in_base64_body() {
    let input = concat!(
        "From: from@example.com\r\n",
        "To: to@example.com\r\n",
        "Content-Type: text/plain; charset=utf-8\r\n",
        "Content-Transfer-Encoding: base64\r\n",
        "\r\n",
        "w6Fy \t\r\n"
    );

    let parsed = parse_rfc822(input.as_bytes()).expect("message should parse");
    assert_eq!(parsed.body(), &Body::Text("ár".to_owned()));
}

#[test]
fn parse_rejects_header_with_control_character() {
    let mut input =
        Vec::from(b"From: from@example.com\r\nTo: to@example.com\r\nSubject: Hello".as_slice());
    input.push(0x01);
    input.extend_from_slice(b"World\r\n\r\nHello");

    assert!(parse_rfc822(&input).is_err());
}

#[test]
fn render_rejects_control_characters_in_header_values() {
    assert!(Header::new("X-Test", "hello\u{0001}world").is_err());
}

#[test]
fn header_new_accepts_non_ascii_values() {
    // The model layer no longer rejects non-ASCII header values; encoding to
    // RFC 2047 (or rejection of structured headers) is a wire-layer concern.
    assert!(Header::new("X-Test", "Olá").is_ok());
}

#[test]
fn render_encodes_non_ascii_custom_header_value() {
    let message = Message::builder(Body::Text("Body".to_owned()))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(mailbox("to@example.com"))])
        .add_header(Header::new("X-Greeting", "Olá").expect("header should validate"))
        .build()
        .expect("message should validate");

    let rendered = render_rfc822(&message).expect("render should succeed");
    let text = String::from_utf8(rendered).expect("rendered message should be utf8-safe");

    assert!(
        text.contains("X-Greeting: =?utf-8?B?T2zDoQ==?="),
        "expected RFC 2047 encoded value, got: {text}"
    );
}

#[test]
fn render_does_not_encode_structured_message_id_header() {
    // Structured headers must round-trip raw; encoding them as RFC 2047
    // would corrupt the addr-spec grammar inside <...>.
    let raw = "<custom-id@example.com>";
    let message = Message::builder(Body::Text("Body".to_owned()))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(mailbox("to@example.com"))])
        .add_header(Header::new("X-Custom-MID", raw).expect("header should validate"))
        // Use a non-typed header name in the structured allowlist so the
        // typed Message-ID path doesn't shadow this.
        .add_header(
            Header::new("In-Reply-To", "<other-id@example.com>").expect("header should validate"),
        )
        .build()
        .expect("message should validate");

    let rendered = render_rfc822(&message).expect("render should succeed");
    let text = String::from_utf8(rendered).expect("rendered message should be utf8-safe");

    assert!(text.contains("In-Reply-To: <other-id@example.com>"));
    assert!(
        !text.contains("=?utf-8?"),
        "no encoded-words expected: {text}"
    );
}

#[test]
fn render_text_body_normalizes_lf_to_crlf() {
    let message = Message::builder(Body::Text("line1\nline2".to_owned()))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(mailbox("to@example.com"))])
        .build()
        .expect("message should validate");

    let rendered = render_rfc822(&message).expect("render should succeed");
    let text = String::from_utf8(rendered).expect("rendered message should be utf8-safe");

    assert!(text.ends_with("\r\n\r\nline1\r\nline2"));
}

#[test]
fn render_non_ascii_text_body_normalizes_lf_to_crlf_before_base64() {
    let message = Message::builder(Body::Text("ár\nvíz".to_owned()))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(mailbox("to@example.com"))])
        .build()
        .expect("message should validate");

    let rendered = render_rfc822(&message).expect("render should succeed");
    let parsed = parse_rfc822(&rendered).expect("rendered message should parse");

    assert_eq!(parsed.body(), &Body::Text("ár\r\nvíz".to_owned()));
}

#[test]
fn render_mime_does_not_duplicate_mime_control_headers_from_message_headers() {
    let message = Message::builder(Body::Html("<p>Hello</p>".to_owned()))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(mailbox("to@example.com"))])
        .headers(vec![
            Header::new("MIME-Version", "1.0").expect("header should validate"),
            Header::new("Content-Type", "text/plain").expect("header should validate"),
            Header::new("Content-Transfer-Encoding", "quoted-printable")
                .expect("header should validate"),
        ])
        .build()
        .expect("message should validate");

    let rendered = render_rfc822(&message).expect("render should succeed");
    let text = String::from_utf8(rendered).expect("rendered message should be utf8-safe");

    assert_eq!(text.matches("MIME-Version:").count(), 1);
    assert_eq!(text.matches("Content-Type:").count(), 1);
    assert_eq!(text.matches("Content-Transfer-Encoding:").count(), 0);
    assert!(text.contains("Content-Type: text/html\r\n"));
}

#[test]
fn parse_decodes_quoted_printable_top_level_text_body() {
    let input = concat!(
        "From: from@example.com\r\n",
        "To: to@example.com\r\n",
        "Content-Type: text/plain; charset=utf-8\r\n",
        "Content-Transfer-Encoding: quoted-printable\r\n",
        "\r\n",
        "Ol=C3=A1"
    );

    let parsed = parse_rfc822(input.as_bytes()).expect("message should parse");
    assert_eq!(parsed.body(), &Body::Text("Olá".to_owned()));
}

#[test]
fn parse_rejects_invalid_unencoded_control_octet_in_quoted_printable_body() {
    let mut input = Vec::from(
        b"From: from@example.com\r\nTo: to@example.com\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nHello"
            .as_slice(),
    );
    input.push(0x01);
    input.extend_from_slice(b"World\r\n");

    assert!(parse_rfc822(&input).is_err());
}

#[test]
fn parse_decodes_iso_8859_1_top_level_text_body() {
    let mut input = Vec::from(
        b"From: from@example.com\r\nTo: to@example.com\r\nContent-Type: text/plain; charset=iso-8859-1\r\n\r\n"
            .as_slice(),
    );
    input.extend_from_slice(&[0x4f, 0x6c, 0xe1]);

    let parsed = parse_rfc822(&input).expect("message should parse");
    assert_eq!(parsed.body(), &Body::Text("Olá".to_owned()));
}

#[test]
fn parse_decodes_quoted_printable_trailing_whitespace_is_removed() {
    let input = concat!(
        "From: from@example.com\r\n",
        "To: to@example.com\r\n",
        "Content-Type: text/plain; charset=utf-8\r\n",
        "Content-Transfer-Encoding: quoted-printable\r\n",
        "\r\n",
        "hello=20=20\t\r\n",
        "world\r\n"
    );

    let parsed = parse_rfc822(input.as_bytes()).expect("message should parse");
    assert_eq!(
        parsed.body(),
        &Body::Text("hello  \r\nworld\r\n".to_owned())
    );
}

#[test]
fn parse_decodes_quoted_printable_soft_line_break_with_trailing_whitespace() {
    let input = concat!(
        "From: from@example.com\r\n",
        "To: to@example.com\r\n",
        "Content-Type: text/plain; charset=utf-8\r\n",
        "Content-Transfer-Encoding: quoted-printable\r\n",
        "\r\n",
        "Ol=C3= \t\r\n",
        "=A1\r\n"
    );

    let parsed = parse_rfc822(input.as_bytes()).expect("message should parse");
    assert_eq!(parsed.body(), &Body::Text("Olá\r\n".to_owned()));
}

#[test]
fn parse_decodes_base64_in_multipart_leaf_parts() {
    let input = concat!(
        "From: from@example.com\r\n",
        "To: to@example.com\r\n",
        "MIME-Version: 1.0\r\n",
        "Content-Type: multipart/alternative; boundary=\"b\"\r\n",
        "\r\n",
        "--b\r\n",
        "Content-Type: text/plain; charset=utf-8\r\n",
        "Content-Transfer-Encoding: base64\r\n",
        "\r\n",
        "w6Fy\r\n",
        "--b--\r\n"
    );

    let parsed = parse_rfc822(input.as_bytes()).expect("multipart should parse");

    let Body::Mime(MimePart::Multipart { parts, .. }) = parsed.body() else {
        panic!("expected multipart body");
    };

    let MimePart::Leaf { body, .. } = &parts[0] else {
        panic!("expected leaf part");
    };

    assert_eq!(body, "ár".as_bytes());
}

#[test]
fn parse_ignores_non_alphabet_characters_in_base64_multipart_leaf() {
    let input = concat!(
        "From: from@example.com\r\n",
        "To: to@example.com\r\n",
        "MIME-Version: 1.0\r\n",
        "Content-Type: multipart/alternative; boundary=\"b\"\r\n",
        "\r\n",
        "--b\r\n",
        "Content-Type: text/plain; charset=utf-8\r\n",
        "Content-Transfer-Encoding: base64\r\n",
        "\r\n",
        "w6Fy!!@@\r\n",
        "--b--\r\n"
    );

    let parsed = parse_rfc822(input.as_bytes()).expect("multipart should parse");

    let Body::Mime(MimePart::Multipart { parts, .. }) = parsed.body() else {
        panic!("expected multipart body");
    };

    let MimePart::Leaf { body, .. } = &parts[0] else {
        panic!("expected leaf part");
    };

    assert_eq!(body, "ár".as_bytes());
}

#[test]
fn parse_preserves_exact_fws_when_unfolding_headers() {
    let input = concat!(
        "From: from@example.com\r\n",
        "To: to@example.com\r\n",
        "Subject: one\r\n",
        "\t two\r\n",
        "  three\r\n",
        "\r\n",
        "Hello"
    );

    let parsed = parse_rfc822(input.as_bytes()).expect("message should parse");
    assert_eq!(parsed.subject(), Some("one\t two  three"));
}

#[test]
fn render_preserves_single_wsp_after_header_fold() {
    let message = Message::builder(Body::Text("Hello".to_owned()))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(mailbox("to@example.com"))])
        .headers(vec![
            Header::new(
                "X-Trace",
                format!("{} {}", "a".repeat(900), "b".repeat(200)),
            )
            .expect("header should validate"),
        ])
        .build()
        .expect("message should validate");

    let rendered = render_rfc822(&message).expect("render should succeed");
    let parsed = parse_rfc822(&rendered).expect("rendered message should parse");

    let header = parsed
        .headers()
        .iter()
        .find(|h| h.name().eq_ignore_ascii_case("x-trace"))
        .expect("x-trace should exist");

    assert_eq!(
        header.value(),
        format!("{} {}", "a".repeat(900), "b".repeat(200))
    );
}

#[test]
fn parse_decodes_quoted_printable_in_multipart_leaf_parts() {
    let input = concat!(
        "From: from@example.com\r\n",
        "To: to@example.com\r\n",
        "MIME-Version: 1.0\r\n",
        "Content-Type: multipart/alternative; boundary=\"b\"\r\n",
        "\r\n",
        "--b\r\n",
        "Content-Type: text/plain; charset=utf-8\r\n",
        "Content-Transfer-Encoding: quoted-printable\r\n",
        "\r\n",
        "Ol=C3=A1\r\n",
        "--b--\r\n"
    );

    let parsed = parse_rfc822(input.as_bytes()).expect("multipart should parse");

    let Body::Mime(MimePart::Multipart { parts, .. }) = parsed.body() else {
        panic!("expected multipart body");
    };

    let MimePart::Leaf { body, .. } = &parts[0] else {
        panic!("expected leaf part");
    };

    assert_eq!(body, "Olá".as_bytes());
}

#[test]
fn parse_preserves_non_boundary_content_type_params_on_multipart() {
    let input = concat!(
        "From: from@example.com\r\n",
        "To: to@example.com\r\n",
        "MIME-Version: 1.0\r\n",
        "Content-Type: multipart/related; type=\"text/html\"; boundary=\"b\"\r\n",
        "\r\n",
        "--b\r\n",
        "Content-Type: text/html\r\n",
        "\r\n",
        "<p>hello</p>\r\n",
        "--b--\r\n"
    );

    let parsed = parse_rfc822(input.as_bytes()).expect("multipart should parse");
    let Body::Mime(MimePart::Multipart { content_type, .. }) = parsed.body() else {
        panic!("expected multipart body");
    };

    assert_eq!(
        content_type.as_str(),
        "multipart/related; type=\"text/html\""
    );
}

#[test]
fn parse_decodes_adjacent_rfc2047_words_without_injected_space() {
    let input = concat!(
        "From: from@example.com\r\n",
        "To: to@example.com\r\n",
        "Subject: =?UTF-8?B?U2Fn?= =?UTF-8?B?aS1LYXphcm0=?=\r\n",
        "\r\n",
        "Hello"
    );

    let parsed = parse_rfc822(input.as_bytes()).expect("message should parse");
    assert_eq!(parsed.subject(), Some("Sagi-Kazarm"));
}

#[test]
fn render_rejects_invalid_boundary_chars_in_mime_body() {
    let mime = MimePart::Multipart {
        content_type: ContentType::try_from("multipart/alternative")
            .expect("content type should parse"),
        boundary: Some("bad;boundary".to_owned()),
        parts: vec![MimePart::Leaf {
            content_type: ContentType::try_from("text/plain").expect("content type should parse"),
            content_transfer_encoding: None,
            content_disposition: None,
            body: b"hello".to_vec(),
        }],
    };

    let message = Message::builder(Body::Mime(mime))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(mailbox("to@example.com"))])
        .build()
        .expect("message should validate");

    assert!(render_rfc822(&message).is_err());
}

#[test]
fn render_adds_boundary_when_other_param_contains_boundary_substring() {
    let mime = MimePart::Multipart {
        content_type: ContentType::try_from("multipart/related;type=\"xboundary=y\"")
            .expect("content type should parse"),
        boundary: Some("actual-boundary".to_owned()),
        parts: vec![MimePart::Leaf {
            content_type: ContentType::try_from("text/plain").expect("content type should parse"),
            content_transfer_encoding: None,
            content_disposition: None,
            body: b"hello".to_vec(),
        }],
    };

    let message = Message::builder(Body::Mime(mime))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(mailbox("to@example.com"))])
        .build()
        .expect("message should validate");

    let rendered = render_rfc822(&message).expect("render should succeed");
    let text = String::from_utf8(rendered).expect("rendered message should be utf8-safe");

    assert!(text.contains(
        "Content-Type: multipart/related; type=\"xboundary=y\"; boundary=\"actual-boundary\"\r\n"
    ));
}

#[test]
fn render_reuses_boundary_from_content_type_for_delimiters() {
    let mime = MimePart::Multipart {
        content_type: ContentType::try_from("multipart/mixed;boundary=\"header-b\"")
            .expect("content type should parse"),
        boundary: None,
        parts: vec![MimePart::Leaf {
            content_type: ContentType::try_from("text/plain").expect("content type should parse"),
            content_transfer_encoding: None,
            content_disposition: None,
            body: b"hello".to_vec(),
        }],
    };

    let message = Message::builder(Body::Mime(mime))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(mailbox("to@example.com"))])
        .build()
        .expect("message should validate");

    let rendered = render_rfc822(&message).expect("render should succeed");
    let text = String::from_utf8(rendered).expect("rendered message should be utf8-safe");

    assert!(text.contains("Content-Type: multipart/mixed; boundary=\"header-b\"\r\n"));
    assert!(text.contains("--header-b\r\n"));
    assert!(text.contains("--header-b--"));
}

#[test]
fn render_rejects_mismatched_header_and_part_boundaries() {
    let mime = MimePart::Multipart {
        content_type: ContentType::try_from("multipart/mixed;boundary=\"header-b\"")
            .expect("content type should parse"),
        boundary: Some("part-b".to_owned()),
        parts: vec![MimePart::Leaf {
            content_type: ContentType::try_from("text/plain").expect("content type should parse"),
            content_transfer_encoding: None,
            content_disposition: None,
            body: b"hello".to_vec(),
        }],
    };

    let message = Message::builder(Body::Mime(mime))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(mailbox("to@example.com"))])
        .build()
        .expect("message should validate");

    assert!(render_rfc822(&message).is_err());
}

#[test]
fn render_auto_generated_boundary_avoids_collision_with_part_body() {
    let with_attachment = Message::builder(Body::Text(
        "--=_email_message_boundary_0\r\nhello".to_owned(),
    ))
    .from_mailbox(mailbox("from@example.com"))
    .to(vec![Address::Mailbox(mailbox("to@example.com"))])
    .attachments(vec![
        email_message::Attachment::bytes(
            ContentType::try_from("text/plain").expect("content type should parse"),
            b"x".to_vec(),
        )
        .with_filename("a.txt"),
    ])
    .build()
    .expect("message should validate");

    let rendered = render_rfc822(&with_attachment).expect("render should succeed");
    let text = String::from_utf8(rendered).expect("rendered message should be utf8-safe");

    assert!(text.contains("boundary=\"=_email_message_boundary_1\""));
    assert!(!text.contains("boundary=\"=_email_message_boundary_0\""));
}

#[test]
fn render_rejects_explicit_boundary_that_collides_with_part_body() {
    let mime = MimePart::Multipart {
        content_type: ContentType::try_from("multipart/mixed").expect("content type should parse"),
        boundary: Some("collision".to_owned()),
        parts: vec![MimePart::Leaf {
            content_type: ContentType::try_from("text/plain").expect("content type should parse"),
            content_transfer_encoding: None,
            content_disposition: None,
            body: b"--collision\r\ninside".to_vec(),
        }],
    };

    let message = Message::builder(Body::Mime(mime))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(mailbox("to@example.com"))])
        .build()
        .expect("message should validate");

    let err = render_rfc822(&message).expect_err("render should fail");
    assert!(matches!(err, MessageRenderError::InvalidMimeBoundary));
}

#[test]
fn parse_to_header_with_mixed_mailbox_and_group_preserves_all_items() {
    // mail_parser flips the whole header to a `Group`-shaped result as soon
    // as any group syntax appears, and wraps flat mailboxes that appear
    // before/between/after named groups into anonymous `Group{name:None}`
    // entries. The parser must flatten those back to plain Mailbox items
    // and preserve the original ordering rather than failing the whole
    // header.
    let input = concat!(
        "From: from@example.com\r\n",
        "To: alice@example.com, Team: bob@team.com;, dave@example.com\r\n",
        "\r\n",
        "Hello"
    );

    let parsed = parse_rfc822(input.as_bytes()).expect("mixed To header should parse");
    let to = parsed.to();

    assert_eq!(to.len(), 3, "expected three To items, got {to:?}");

    match &to[0] {
        Address::Mailbox(mailbox) => assert_eq!(mailbox.email().as_str(), "alice@example.com"),
        other => panic!("expected first item to be a Mailbox, got {other:?}"),
    }
    match &to[1] {
        Address::Group(group) => {
            assert_eq!(group.name(), "Team");
            assert_eq!(group.members().len(), 1);
            assert_eq!(group.members()[0].email().as_str(), "bob@team.com");
        }
        other => panic!("expected second item to be a Group, got {other:?}"),
    }
    match &to[2] {
        Address::Mailbox(mailbox) => assert_eq!(mailbox.email().as_str(), "dave@example.com"),
        other => panic!("expected third item to be a Mailbox, got {other:?}"),
    }
}

#[test]
fn render_caps_auto_boundary_attempts_on_adversarial_body() {
    // A leaf body whose bytes contain `--=_email_message_boundary_N\r\n` lines
    // for every counter value the renderer would try will force the
    // auto-generation loop to exhaust its retry cap. The renderer must return
    // an error rather than spin indefinitely.
    let mut adversarial = Vec::new();
    for n in 0..256usize {
        adversarial.extend_from_slice(b"--=_email_message_boundary_");
        adversarial.extend_from_slice(n.to_string().as_bytes());
        adversarial.extend_from_slice(b"\r\n");
    }

    let outer = MimePart::Multipart {
        content_type: ContentType::try_from("multipart/mixed").expect("content type should parse"),
        boundary: None,
        parts: vec![MimePart::Leaf {
            content_type: ContentType::try_from("text/plain").expect("content type should parse"),
            content_transfer_encoding: None,
            content_disposition: None,
            body: adversarial,
        }],
    };

    let message = Message::builder(Body::Mime(outer))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(mailbox("to@example.com"))])
        .build()
        .expect("message should validate");

    let err = render_rfc822(&message).expect_err("render should fail rather than loop");
    assert!(matches!(err, MessageRenderError::InvalidMimeBoundary));
}

#[test]
fn render_rejects_outer_boundary_inside_nested_leaf_body() {
    // The pre-render walk only inspects content_type/boundary fields on
    // nested multipart nodes; it cannot see leaf bytes that a nested
    // multipart will produce. A leaf inside a nested multipart whose body
    // bytes contain a line matching the outer boundary must still be
    // detected and rejected by the post-render scan.
    let inner_leaf = MimePart::Leaf {
        content_type: ContentType::try_from("text/plain").expect("content type should parse"),
        content_transfer_encoding: None,
        content_disposition: None,
        body: b"prefix\r\n--outer-boundary\r\nsuffix".to_vec(),
    };
    let nested = MimePart::Multipart {
        content_type: ContentType::try_from("multipart/alternative")
            .expect("content type should parse"),
        boundary: Some("inner-b".to_owned()),
        parts: vec![inner_leaf],
    };
    let outer = MimePart::Multipart {
        content_type: ContentType::try_from("multipart/mixed").expect("content type should parse"),
        boundary: Some("outer-boundary".to_owned()),
        parts: vec![nested],
    };

    let message = Message::builder(Body::Mime(outer))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(mailbox("to@example.com"))])
        .build()
        .expect("message should validate");

    let err = render_rfc822(&message).expect_err("render should reject");
    assert!(matches!(err, MessageRenderError::InvalidMimeBoundary));
}

#[test]
fn render_rejects_boundary_reused_by_nested_multipart() {
    let nested = MimePart::Multipart {
        content_type: ContentType::try_from("multipart/alternative;boundary=\"collision\"")
            .expect("content type should parse"),
        boundary: None,
        parts: vec![MimePart::Leaf {
            content_type: ContentType::try_from("text/plain").expect("content type should parse"),
            content_transfer_encoding: None,
            content_disposition: None,
            body: b"hello".to_vec(),
        }],
    };

    let outer = MimePart::Multipart {
        content_type: ContentType::try_from("multipart/mixed;boundary=\"collision\"")
            .expect("content type should parse"),
        boundary: None,
        parts: vec![nested],
    };

    let message = Message::builder(Body::Mime(outer))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(mailbox("to@example.com"))])
        .build()
        .expect("message should validate");

    let err = render_rfc822(&message).expect_err("render should fail");
    assert!(matches!(err, MessageRenderError::InvalidMimeBoundary));
}

#[test]
fn parse_multipart_preserves_trailing_blank_lines_inside_part_body() {
    let input = concat!(
        "From: sender@example.com\r\n",
        "To: recipient@example.com\r\n",
        "MIME-Version: 1.0\r\n",
        "Content-Type: multipart/mixed; boundary=\"b\"\r\n",
        "\r\n",
        "--b\r\n",
        "Content-Type: text/plain\r\n",
        "\r\n",
        "line 1\r\n",
        "\r\n",
        "\r\n",
        "--b--\r\n"
    );

    let parsed = parse_rfc822(input.as_bytes()).expect("multipart should parse");

    let Body::Mime(MimePart::Multipart { parts, .. }) = parsed.body() else {
        panic!("expected multipart body");
    };

    let MimePart::Leaf { body, .. } = &parts[0] else {
        panic!("expected leaf part");
    };

    assert_eq!(body, b"line 1\r\n\r\n");
}

#[test]
fn parse_rejects_base64_content_transfer_encoding_on_multipart_root() {
    let input = concat!(
        "From: from@example.com\r\n",
        "To: to@example.com\r\n",
        "MIME-Version: 1.0\r\n",
        "Content-Type: multipart/mixed; boundary=\"b\"\r\n",
        "Content-Transfer-Encoding: base64\r\n",
        "\r\n",
        "--b\r\n",
        "Content-Type: text/plain\r\n",
        "\r\n",
        "hello\r\n",
        "--b--\r\n"
    );

    assert!(parse_rfc822(input.as_bytes()).is_err());
}

#[test]
fn parse_rejects_quoted_printable_content_transfer_encoding_on_nested_multipart() {
    let input = concat!(
        "From: from@example.com\r\n",
        "To: to@example.com\r\n",
        "MIME-Version: 1.0\r\n",
        "Content-Type: multipart/mixed; boundary=\"outer\"\r\n",
        "\r\n",
        "--outer\r\n",
        "Content-Type: multipart/alternative; boundary=\"inner\"\r\n",
        "Content-Transfer-Encoding: quoted-printable\r\n",
        "\r\n",
        "--inner\r\n",
        "Content-Type: text/plain\r\n",
        "\r\n",
        "hello\r\n",
        "--inner--\r\n",
        "--outer--\r\n"
    );

    assert!(parse_rfc822(input.as_bytes()).is_err());
}

#[test]
fn parse_rejects_multipart_with_only_closing_boundary() {
    let input = concat!(
        "From: from@example.com\r\n",
        "To: to@example.com\r\n",
        "MIME-Version: 1.0\r\n",
        "Content-Type: multipart/mixed; boundary=\"b\"\r\n",
        "\r\n",
        "--b--\r\n"
    );

    assert!(parse_rfc822(input.as_bytes()).is_err());
}

#[test]
fn parse_accepts_multipart_with_empty_part_between_boundaries() {
    let input = concat!(
        "From: from@example.com\r\n",
        "To: to@example.com\r\n",
        "MIME-Version: 1.0\r\n",
        "Content-Type: multipart/mixed; boundary=\"b\"\r\n",
        "\r\n",
        "--b\r\n",
        "\r\n",
        "part one\r\n",
        "--b\r\n",
        "--b--\r\n"
    );

    let parsed = parse_rfc822(input.as_bytes()).expect("multipart should parse");
    let Body::Mime(MimePart::Multipart { parts, .. }) = parsed.body() else {
        panic!("expected multipart body");
    };

    assert_eq!(parts.len(), 2);
    let MimePart::Leaf {
        body: first_body, ..
    } = &parts[0]
    else {
        panic!("expected first leaf part");
    };
    assert_eq!(first_body, b"part one");

    let MimePart::Leaf {
        body: second_body, ..
    } = &parts[1]
    else {
        panic!("expected second leaf part");
    };
    assert!(second_body.is_empty());
}

#[test]
fn parse_accepts_empty_headers_in_multipart_part() {
    let input = concat!(
        "From: from@example.com\r\n",
        "To: to@example.com\r\n",
        "MIME-Version: 1.0\r\n",
        "Content-Type: multipart/mixed; boundary=\"b\"\r\n",
        "\r\n",
        "--b\r\n",
        "\r\n",
        "part body\r\n",
        "--b--\r\n"
    );

    let parsed = parse_rfc822(input.as_bytes()).expect("multipart should parse");

    let Body::Mime(MimePart::Multipart { parts, .. }) = parsed.body() else {
        panic!("expected multipart body");
    };

    let MimePart::Leaf { body, .. } = &parts[0] else {
        panic!("expected leaf part");
    };

    assert_eq!(body, b"part body");
}

#[test]
fn render_rejects_multipart_with_no_parts() {
    let mime = MimePart::Multipart {
        content_type: ContentType::try_from("multipart/mixed").expect("content type should parse"),
        boundary: Some("b".to_owned()),
        parts: Vec::new(),
    };

    let message = Message::builder(Body::Mime(mime))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(mailbox("to@example.com"))])
        .build()
        .expect("message should validate");

    let err = render_rfc822(&message).expect_err("render should fail");
    assert!(matches!(err, MessageRenderError::EmptyMultipartParts));
}

#[test]
fn render_rejects_multipart_part_with_non_multipart_content_type() {
    let mime = MimePart::Multipart {
        content_type: ContentType::try_from("text/plain").expect("content type should parse"),
        boundary: Some("b".to_owned()),
        parts: vec![MimePart::Leaf {
            content_type: ContentType::try_from("text/plain").expect("content type should parse"),
            content_transfer_encoding: None,
            content_disposition: None,
            body: b"hello".to_vec(),
        }],
    };

    let message = Message::builder(Body::Mime(mime))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(mailbox("to@example.com"))])
        .build()
        .expect("message should validate");

    let err = render_rfc822(&message).expect_err("render should fail");
    assert!(matches!(
        err,
        MessageRenderError::InvalidMultipartContentType
    ));
}

#[test]
fn rendered_multipart_with_part_body_ending_in_crlf_preserves_trailing_crlf_on_reparse() {
    let mime = MimePart::Multipart {
        content_type: ContentType::try_from("multipart/mixed").expect("content type should parse"),
        boundary: Some("b".to_owned()),
        parts: vec![MimePart::Leaf {
            content_type: ContentType::try_from("text/plain").expect("content type should parse"),
            content_transfer_encoding: None,
            content_disposition: None,
            body: b"line\r\n".to_vec(),
        }],
    };

    let message = Message::builder(Body::Mime(mime))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(mailbox("to@example.com"))])
        .build()
        .expect("message should validate");

    let rendered = render_rfc822(&message).expect("render should succeed");
    let text = String::from_utf8(rendered).expect("rendered message should be utf8-safe");

    assert!(
        text.contains("line\r\n\r\n--b--\r\n"),
        "multipart writer must emit separator CRLF even when part body already ends in CRLF"
    );

    let reparsed = parse_rfc822(text.as_bytes()).expect("rendered multipart should parse");
    let Body::Mime(MimePart::Multipart { parts, .. }) = reparsed.body() else {
        panic!("expected multipart body");
    };
    let MimePart::Leaf { body, .. } = &parts[0] else {
        panic!("expected leaf body");
    };
    assert_eq!(body, b"line\r\n");
}

#[test]
fn parse_rejects_input_exceeding_max_bytes() {
    // Allocate slightly larger than the cap; we don't even need a
    // valid RFC 5322 prefix because the length check fires first.
    let huge = vec![b'a'; MAX_INPUT_BYTES + 1];
    let err = parse_rfc822(&huge).expect_err("oversized input must be rejected");
    assert!(matches!(err, MessageParseError::MimeBodyParse { .. }));
}

#[test]
fn parse_rejects_multipart_nested_beyond_max_depth() {
    // Construct a synthetic message with MAX_MULTIPART_DEPTH + 5 levels
    // of nested multipart/mixed. Distinct boundary per level
    // (b0, b1, …) so the parser can actually walk into each level.
    let depth = MAX_MULTIPART_DEPTH + 5;

    // Innermost: a leaf part inside the deepest boundary.
    let mut body =
        format!("--b{depth}\r\nContent-Type: text/plain\r\n\r\nleaf\r\n--b{depth}--\r\n");

    // Wrap each level outward; each level's boundary opens, then
    // declares a single child part whose Content-Type is the next
    // inner multipart, then closes.
    for level in (0..depth).rev() {
        let inner_b = level + 1;
        body = format!(
            "--b{level}\r\nContent-Type: multipart/mixed; boundary=b{inner_b}\r\n\r\n{body}--b{level}--\r\n"
        );
    }

    let message = format!(
        "From: a@example.com\r\nTo: b@example.com\r\n\
         Subject: deep\r\nMIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=b0\r\n\r\n{body}"
    );

    let err = parse_rfc822(message.as_bytes()).expect_err("excessive nesting must be rejected");
    assert!(
        matches!(
            err,
            MessageParseError::MimeBodyParse { ref details, .. }
            if details.contains("nesting") || details.contains("depth")
        ),
        "expected nesting/depth error, got {err:?}"
    );
}

#[test]
fn parse_rejects_multipart_fan_out_beyond_max_parts() {
    // Construct a single-level multipart with MAX_MULTIPART_PARTS + 1
    // empty parts. Each part has `Content-Type: text/plain`.
    let mut body = String::new();
    for _ in 0..(MAX_MULTIPART_PARTS + 1) {
        body.push_str("--b\r\nContent-Type: text/plain\r\n\r\nx\r\n");
    }
    body.push_str("--b--\r\n");

    let message = format!(
        "From: a@example.com\r\nTo: b@example.com\r\n\
         Subject: fanout\r\nMIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=b\r\n\r\n{body}"
    );

    let err = parse_rfc822(message.as_bytes()).expect_err("excessive fan-out must be rejected");
    assert!(
        matches!(
            err,
            MessageParseError::MimeBodyParse { ref details, .. }
            if details.contains("parts") || details.contains("fan")
        ),
        "expected parts/fan error, got {err:?}"
    );
}

fn nested_mime_multipart(depth: usize) -> MimePart {
    let leaf = MimePart::Leaf {
        content_type: ContentType::try_from("text/plain").expect("content type should parse"),
        content_transfer_encoding: None,
        content_disposition: None,
        body: b"leaf".to_vec(),
    };
    let mut current = leaf;
    for level in 0..depth {
        current = MimePart::Multipart {
            content_type: ContentType::try_from("multipart/mixed")
                .expect("content type should parse"),
            boundary: Some(format!("nest-b{level}")),
            parts: vec![current],
        };
    }
    current
}

#[test]
fn render_rejects_mime_nested_beyond_max_depth() {
    let mime = nested_mime_multipart(MAX_MULTIPART_DEPTH + 5);
    let message = Message::builder(Body::Mime(mime))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(mailbox("to@example.com"))])
        .build()
        .expect("message should validate");

    let err = render_rfc822(&message).expect_err("excessive nesting must be rejected");
    assert!(
        matches!(err, MessageRenderError::MimeNestingTooDeep),
        "expected MimeNestingTooDeep, got {err:?}"
    );
}

#[test]
fn render_accepts_mime_nested_at_max_depth() {
    let mime = nested_mime_multipart(MAX_MULTIPART_DEPTH);
    let message = Message::builder(Body::Mime(mime))
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(mailbox("to@example.com"))])
        .build()
        .expect("message should validate");

    render_rfc822(&message).expect("nesting at the cap should render");
}
