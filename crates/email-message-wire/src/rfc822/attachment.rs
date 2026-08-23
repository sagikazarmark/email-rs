use email_message::{Attachment, AttachmentBody, MessageId};

use super::MessageRenderError;
use super::transfer_encoding::encode_base64;

struct EncodedFilenameParameter {
    legacy: Option<String>,
    extended: Option<String>,
}

pub(super) fn partition_attachments(
    attachments: &[Attachment],
) -> (Vec<&Attachment>, Vec<&Attachment>) {
    let mut inline = Vec::new();
    let mut regular = Vec::new();

    for attachment in attachments {
        if attachment.is_inline() || attachment.content_id().is_some() {
            inline.push(attachment);
        } else {
            regular.push(attachment);
        }
    }

    (inline, regular)
}

pub(super) fn attachment_to_mime_part(
    attachment: &Attachment,
) -> Result<super::RenderPart, MessageRenderError> {
    let AttachmentBody::Bytes(raw) = attachment.body() else {
        return Err(MessageRenderError::UnsupportedAttachmentBody);
    };

    let mut disposition = if attachment.is_inline() || attachment.content_id().is_some() {
        String::from("inline")
    } else {
        String::from("attachment")
    };

    if let Some(filename) = attachment.filename() {
        let encoded = encode_filename_parameter(filename);
        if let Some(legacy) = encoded.legacy {
            disposition.push_str("; ");
            disposition.push_str(&legacy);
        }
        if let Some(extended) = encoded.extended {
            disposition.push_str("; ");
            disposition.push_str(&extended);
        }
    }

    let mut headers = vec![(
        String::from("Content-Type"),
        attachment.content_type().to_string(),
    )];
    headers.push((
        String::from("Content-Transfer-Encoding"),
        String::from("base64"),
    ));
    headers.push((String::from("Content-Disposition"), disposition));

    if let Some(content_id) = attachment.content_id() {
        headers.push((
            String::from("Content-ID"),
            normalize_content_id(content_id)?,
        ));
    }

    Ok(super::RenderPart::Leaf {
        headers,
        body: encode_base64(raw),
    })
}

fn encode_filename_parameter(filename: &str) -> EncodedFilenameParameter {
    let escaped = filename.replace('\\', "\\\\").replace('"', "\\\"");
    let plain_ascii = filename
        .bytes()
        .all(|b| b.is_ascii() && !b.is_ascii_control());
    if plain_ascii {
        return EncodedFilenameParameter {
            legacy: Some(format!("filename=\"{escaped}\"")),
            extended: None,
        };
    }

    // Filenames containing control bytes (including TAB, CR, LF) take the
    // RFC 2231 percent-encoded path even when the bytes are otherwise ASCII.
    // RFC 6266 §4.1 nominally permits TAB inside a quoted-string, but real
    // MUAs misinterpret tabs in `filename=` parameters; force the
    // unambiguous encoding.
    let mut extended = String::from("filename*=utf-8''");
    // Writing into a String is infallible.
    let _ = write_percent_encoded(filename.as_bytes(), &mut extended);
    EncodedFilenameParameter {
        legacy: None,
        extended: Some(extended),
    }
}

fn write_percent_encoded<W: std::fmt::Write>(input: &[u8], out: &mut W) -> std::fmt::Result {
    for byte in input {
        let ch = *byte as char;
        if ch.is_ascii_alphanumeric()
            || matches!(
                ch,
                '!' | '#' | '$' | '&' | '+' | '-' | '.' | '^' | '_' | '`' | '|' | '~'
            )
        {
            out.write_char(ch)?;
        } else {
            write!(out, "%{byte:02X}")?;
        }
    }
    Ok(())
}

fn normalize_content_id(content_id: &str) -> Result<String, MessageRenderError> {
    let value = content_id.trim();
    if value.is_empty()
        || value
            .chars()
            .any(|ch| ch.is_ascii_control() || ch.is_ascii_whitespace())
    {
        return Err(MessageRenderError::InvalidContentId);
    }

    let left = value.matches('<').count();
    let right = value.matches('>').count();
    if left > 1 || right > 1 {
        return Err(MessageRenderError::InvalidContentId);
    }
    if (left == 1 || right == 1) && !(value.starts_with('<') && value.ends_with('>')) {
        return Err(MessageRenderError::InvalidContentId);
    }

    let addr_spec = if value.starts_with('<') && value.ends_with('>') {
        &value[1..value.len() - 1]
    } else {
        value
    };

    if addr_spec.is_empty()
        || addr_spec
            .chars()
            .any(|ch| ch.is_ascii_control() || ch.is_ascii_whitespace() || ch == '<' || ch == '>')
    {
        return Err(MessageRenderError::InvalidContentId);
    }

    let rendered = if value.starts_with('<') && value.ends_with('>') {
        value.to_owned()
    } else {
        format!("<{value}>")
    };

    rendered
        .parse::<MessageId>()
        .map_err(|_| MessageRenderError::InvalidContentId)?;

    Ok(rendered)
}
