use email_message::{
    Address, Attachment, AttachmentReference, Body, ContentType, Disposition, Mailbox, Message,
};
use email_message_wire::{MessageRenderError, parse_rfc822, render_rfc822};

fn mailbox(input: &str) -> Mailbox {
    input.parse::<Mailbox>().expect("mailbox should parse")
}

fn attachment(
    filename: Option<&str>,
    content_type: &str,
    inline: bool,
    content_id: Option<&str>,
    body: Vec<u8>,
) -> Attachment {
    let attachment = Attachment::bytes(
        ContentType::try_from(content_type).expect("content type should parse"),
        body,
    )
    .with_disposition(if inline {
        Disposition::Inline
    } else {
        Disposition::Attachment
    });
    let attachment = match filename {
        Some(filename) => attachment.with_filename(filename),
        None => attachment,
    };

    match content_id {
        Some(content_id) => attachment.with_content_id(content_id),
        None => attachment,
    }
}

fn message_with(body: Body, attachments: Vec<Attachment>) -> Message {
    Message::builder(body)
        .from_mailbox(mailbox("from@example.com"))
        .to(vec![Address::Mailbox(mailbox("to@example.com"))])
        .subject("Attachment test")
        .attachments(attachments)
        .build()
        .expect("message should validate")
}

#[test]
fn text_with_regular_attachment_renders_multipart_mixed() {
    let message = message_with(
        Body::Text("hello".to_owned()),
        vec![attachment(
            Some("report.pdf"),
            "application/pdf",
            false,
            None,
            b"PDF-BINARY".to_vec(),
        )],
    );

    let rendered = render_rfc822(&message).expect("render should succeed");
    let text = String::from_utf8(rendered).expect("rendered message should be utf8-safe");

    assert!(text.contains("MIME-Version: 1.0\r\n"));
    assert!(
        text.contains("Content-Type: multipart/mixed; boundary=\"=_email_message_boundary_0\"\r\n")
    );
    assert!(text.contains("Content-Type: text/plain\r\n"));
    assert!(text.contains("Content-Type: application/pdf\r\n"));
    assert!(text.contains("Content-Disposition: attachment; filename=\"report.pdf\"\r\n"));
    assert!(text.contains("Content-Transfer-Encoding: base64\r\n"));
}

#[test]
fn html_with_inline_attachment_renders_related_and_content_id() {
    let message = message_with(
        Body::Html("<img src=\"cid:logo@example.com\">".to_owned()),
        vec![attachment(
            Some("logo.png"),
            "image/png",
            true,
            Some("logo@example.com"),
            vec![0, 1, 2, 3, 4, 5],
        )],
    );

    let rendered = render_rfc822(&message).expect("render should succeed");
    let text = String::from_utf8(rendered).expect("rendered message should be utf8-safe");

    assert!(
        text.contains(
            "Content-Type: multipart/related; type=\"text/html\"; boundary=\"=_email_message_boundary_0\"\r\n"
        )
    );
    assert!(text.contains("Content-Type: text/html\r\n"));
    assert!(text.contains("Content-Type: image/png\r\n"));
    assert!(text.contains("Content-ID: <logo@example.com>\r\n"));
    assert!(text.contains("Content-Disposition: inline; filename=\"logo.png\"\r\n"));
}

#[test]
fn text_html_with_inline_and_regular_uses_mixed_related_alternative_nesting() {
    let message = message_with(
        Body::TextAndHtml {
            text: "hello text".to_owned(),
            html: "<p>hello html</p><img src=\"cid:img@example.com\">".to_owned(),
        },
        vec![
            attachment(
                Some("img.png"),
                "image/png",
                true,
                Some("img@example.com"),
                vec![1, 2, 3],
            ),
            attachment(
                Some("readme.txt"),
                "text/plain",
                false,
                None,
                b"attached text".to_vec(),
            ),
        ],
    );

    let rendered = render_rfc822(&message).expect("render should succeed");
    let text = String::from_utf8(rendered).expect("rendered message should be utf8-safe");

    assert!(
        text.contains("Content-Type: multipart/mixed; boundary=\"=_email_message_boundary_0\"\r\n")
    );
    assert!(
        text.contains(
            "Content-Type: multipart/related; type=\"multipart/alternative\"; boundary=\"=_email_message_boundary_1\"\r\n"
        )
    );
    assert!(text.contains(
        "Content-Type: multipart/alternative; boundary=\"=_email_message_boundary_2\"\r\n"
    ));
    assert!(text.contains("Content-Type: text/plain\r\n"));
    assert!(text.contains("Content-Type: text/html\r\n"));
    assert!(text.contains("Content-Disposition: attachment; filename=\"readme.txt\"\r\n"));
    assert!(text.contains("Content-ID: <img@example.com>\r\n"));
}

#[test]
fn ascii_filename_with_control_byte_takes_rfc2231_path() {
    // A filename containing TAB is technically ASCII but the legacy
    // `filename="..."` quoted-string is misinterpreted by real MUAs.
    // Force the unambiguous RFC 2231 percent-encoded form.
    let message = message_with(
        Body::Text("hello".to_owned()),
        vec![attachment(
            Some("oh\tno.txt"),
            "text/plain",
            false,
            None,
            b"abc".to_vec(),
        )],
    );

    let rendered = render_rfc822(&message).expect("render should succeed");
    let text = String::from_utf8(rendered).expect("rendered message should be utf8-safe");

    assert!(text.contains("filename*=utf-8''oh%09no.txt"));
    assert!(!text.contains("filename=\"oh\tno.txt\""));
}

#[test]
fn non_ascii_filename_adds_rfc2231_parameter() {
    let message = message_with(
        Body::Text("hello".to_owned()),
        vec![attachment(
            Some("fájl.txt"),
            "text/plain",
            false,
            None,
            b"abc".to_vec(),
        )],
    );

    let rendered = render_rfc822(&message).expect("render should succeed");
    let text = String::from_utf8(rendered).expect("rendered message should be utf8-safe");

    assert!(text.contains("filename*=utf-8''f%C3%A1jl.txt"));
    assert!(!text.contains("filename=\"fájl.txt\""));
}

#[test]
fn non_ascii_text_part_sets_utf8_charset_and_base64() {
    let message = message_with(
        Body::Text("árvíztűrő tükörfúrógép".to_owned()),
        vec![attachment(
            Some("blob.bin"),
            "application/octet-stream",
            false,
            None,
            vec![1, 2, 3],
        )],
    );

    let rendered = render_rfc822(&message).expect("render should succeed");
    let text = String::from_utf8(rendered).expect("rendered message should be utf8-safe");

    assert!(text.contains("Content-Type: text/plain; charset=utf-8\r\n"));
    assert!(text.contains("Content-Transfer-Encoding: base64\r\n"));
}

#[test]
fn invalid_content_id_is_rejected() {
    let message = message_with(
        Body::Html("<img src=\"cid:oops\">".to_owned()),
        vec![attachment(
            Some("logo.png"),
            "image/png",
            true,
            Some("oops > nope"),
            vec![1, 2, 3],
        )],
    );

    assert!(render_rfc822(&message).is_err());
}

#[test]
fn invalid_content_id_addr_spec_is_rejected() {
    for content_id in ["bad@@example.com", ".bad@example.com", "bad.@example.com"] {
        let message = message_with(
            Body::Html(format!("<img src=\"cid:{content_id}\">")),
            vec![attachment(
                Some("logo.png"),
                "image/png",
                true,
                Some(content_id),
                vec![1, 2, 3],
            )],
        );

        let err = render_rfc822(&message).expect_err("invalid content-id should fail");
        assert!(matches!(err, MessageRenderError::InvalidContentId));
    }
}

#[test]
fn unresolved_attachment_reference_is_rejected() {
    let message = message_with(
        Body::Text("hello".to_owned()),
        vec![
            Attachment::reference(
                ContentType::try_from("application/pdf").expect("content type should parse"),
                AttachmentReference::new("s3://attachments/report.pdf"),
            )
            .with_filename("report.pdf"),
        ],
    );

    let err = render_rfc822(&message).expect_err("render should fail");
    assert!(matches!(err, MessageRenderError::UnsupportedAttachmentBody));
}

#[test]
fn content_id_without_inline_flag_still_renders_inline_disposition() {
    let message = message_with(
        Body::Html("<img src=\"cid:logo@example.com\">".to_owned()),
        vec![attachment(
            Some("logo.png"),
            "image/png",
            false,
            Some("logo@example.com"),
            vec![1, 2, 3],
        )],
    );

    let rendered = render_rfc822(&message).expect("render should succeed");
    let text = String::from_utf8(rendered).expect("rendered message should be utf8-safe");

    assert!(text.contains("Content-Disposition: inline; filename=\"logo.png\"\r\n"));
}

#[test]
fn base64_attachment_lines_are_wrapped_to_76_chars() {
    let message = message_with(
        Body::Text("hello".to_owned()),
        vec![attachment(
            Some("blob.bin"),
            "application/octet-stream",
            false,
            None,
            vec![42; 512],
        )],
    );

    let rendered = render_rfc822(&message).expect("render should succeed");
    let text = String::from_utf8(rendered).expect("rendered message should be utf8-safe");

    let mut in_base64 = false;
    for line in text.split("\r\n") {
        if line == "Content-Transfer-Encoding: base64" {
            in_base64 = true;
            continue;
        }

        if in_base64 && line.starts_with("--=_email_message_boundary") {
            in_base64 = false;
            continue;
        }

        if in_base64 {
            if line.is_empty() || line.starts_with("Content-") {
                continue;
            }
            assert!(line.len() <= 76, "base64 line exceeded 76 chars");
        }
    }
}

#[test]
fn parses_rfc2046_style_multipart_fixture() {
    let fixture = include_bytes!("fixtures/rfc/rfc2046_multipart_mixed.eml");
    let parsed = parse_rfc822(fixture).expect("fixture should parse");

    assert!(matches!(parsed.body(), Body::Mime(_)));
}

#[test]
fn parses_multipart_boundary_with_trailing_whitespace() {
    let input = concat!(
        "From: sender@example.com\r\n",
        "To: recipient@example.com\r\n",
        "MIME-Version: 1.0\r\n",
        "Content-Type: multipart/mixed; boundary=\"b\"\r\n",
        "\r\n",
        "--b \t\r\n",
        "Content-Type: text/plain\r\n",
        "\r\n",
        "part one\r\n",
        "--b-- \t\r\n"
    );

    let parsed = parse_rfc822(input.as_bytes()).expect("multipart with LWSP should parse");
    assert!(matches!(parsed.body(), Body::Mime(_)));
}

#[test]
fn parses_multipart_with_semicolon_in_quoted_boundary_parameter() {
    let input = concat!(
        "From: sender@example.com\r\n",
        "To: recipient@example.com\r\n",
        "MIME-Version: 1.0\r\n",
        "Content-Type: multipart/mixed; boundary=\"b;1\"\r\n",
        "\r\n",
        "--b;1\r\n",
        "Content-Type: text/plain\r\n",
        "\r\n",
        "part one\r\n",
        "--b;1--\r\n"
    );

    let parsed = parse_rfc822(input.as_bytes())
        .expect("multipart with quoted semicolon boundary should parse");
    assert!(matches!(parsed.body(), Body::Mime(_)));
}

#[test]
fn rendered_multipart_ends_with_closing_boundary_crlf() {
    let message = message_with(
        Body::Text("hello".to_owned()),
        vec![attachment(
            Some("report.pdf"),
            "application/pdf",
            false,
            None,
            b"PDF-BINARY".to_vec(),
        )],
    );

    let rendered = render_rfc822(&message).expect("render should succeed");
    assert!(
        rendered.ends_with(b"--=_email_message_boundary_0--\r\n"),
        "rendered multipart must end with CRLF after closing boundary"
    );
}

#[test]
fn rendered_nested_multipart_does_not_insert_extra_blank_line_before_boundary() {
    let message = message_with(
        Body::Text("hello".to_owned()),
        vec![attachment(
            Some("report.pdf"),
            "application/pdf",
            false,
            None,
            b"PDF-BINARY".to_vec(),
        )],
    );

    let rendered = render_rfc822(&message).expect("render should succeed");
    let text = String::from_utf8(rendered).expect("rendered message should be utf8-safe");

    assert!(
        !text.contains("hello\r\n\r\n--=_email_message_boundary_0"),
        "multipart boundary must follow part body after exactly one CRLF"
    );
}
