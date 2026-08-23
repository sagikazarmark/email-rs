use base64::Engine;
use email_message::ContentTransferEncoding;

use super::MessageParseError;
use super::shared::hex_val;

pub(super) fn encode_body_for_transfer_encoding(body: &[u8], encoding: Option<&str>) -> Vec<u8> {
    let Some(encoding) = encoding else {
        return body.to_vec();
    };

    if encoding.eq_ignore_ascii_case("base64") {
        return encode_base64(body);
    }

    if encoding.eq_ignore_ascii_case("quoted-printable") {
        return encode_quoted_printable_body(body);
    }

    body.to_vec()
}

pub(super) fn encode_base64(input: &[u8]) -> Vec<u8> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(input);
    let mut output = Vec::with_capacity(encoded.len() + (encoded.len() / 76 + 2) * 2);

    for chunk in encoded.as_bytes().chunks(76) {
        output.extend_from_slice(chunk);
        output.extend_from_slice(b"\r\n");
    }

    output
}

pub(super) fn decode_transfer_encoded_body(
    body: &[u8],
    encoding: Option<&str>,
) -> Result<Vec<u8>, MessageParseError> {
    let Some(encoding) = encoding else {
        return Ok(body.to_vec());
    };

    if encoding.eq_ignore_ascii_case("base64") {
        return decode_base64_body(body).ok_or_else(|| MessageParseError::MimeBodyParse {
            details: "invalid base64 content-transfer-encoding payload".to_owned(),
        });
    }

    if encoding.eq_ignore_ascii_case("quoted-printable") {
        return decode_quoted_printable_body(body).ok_or_else(|| {
            MessageParseError::MimeBodyParse {
                details: "invalid quoted-printable content-transfer-encoding payload".to_owned(),
            }
        });
    }

    Ok(body.to_vec())
}

pub(super) fn validate_multipart_transfer_encoding(
    encoding: Option<&ContentTransferEncoding>,
) -> Result<(), MessageParseError> {
    let Some(encoding) = encoding else {
        return Ok(());
    };

    let value = encoding.as_str();
    if value.eq_ignore_ascii_case("7bit")
        || value.eq_ignore_ascii_case("8bit")
        || value.eq_ignore_ascii_case("binary")
    {
        return Ok(());
    }

    Err(MessageParseError::MimeBodyParse {
        details: format!("multipart part cannot use content-transfer-encoding `{value}`"),
    })
}

fn decode_base64_body(body: &[u8]) -> Option<Vec<u8>> {
    let mut filtered = Vec::with_capacity(body.len());
    for byte in body.iter().copied() {
        if matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'/' | b'=') {
            filtered.push(byte);
        }
    }

    base64::engine::general_purpose::STANDARD
        .decode(filtered)
        .ok()
}

fn decode_quoted_printable_body(body: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(body.len());
    let mut idx = 0usize;

    while idx < body.len() {
        let line_start = idx;
        while idx < body.len() && body[idx] != b'\r' && body[idx] != b'\n' {
            idx += 1;
        }

        let line = &body[line_start..idx];
        let mut line_end = line.len();
        while line_end > 0 && matches!(line[line_end - 1], b' ' | b'\t') {
            line_end -= 1;
        }
        let line = &line[..line_end];

        let mut newline = &[][..];
        if idx < body.len() {
            if body[idx] == b'\r' {
                if idx + 1 < body.len() && body[idx + 1] == b'\n' {
                    newline = b"\r\n";
                    idx += 2;
                } else {
                    newline = b"\r";
                    idx += 1;
                }
            } else {
                newline = b"\n";
                idx += 1;
            }
        }

        let soft_break = line.ends_with(b"=");
        let encoded = if soft_break {
            &line[..line.len().saturating_sub(1)]
        } else {
            line
        };

        let mut line_idx = 0usize;
        while line_idx < encoded.len() {
            if encoded[line_idx] != b'=' {
                if !is_valid_quoted_printable_literal(encoded[line_idx]) {
                    return None;
                }
                out.push(encoded[line_idx]);
                line_idx += 1;
                continue;
            }

            if line_idx + 2 >= encoded.len() {
                return None;
            }

            let hi = hex_val(encoded[line_idx + 1])?;
            let lo = hex_val(encoded[line_idx + 2])?;
            out.push((hi << 4) | lo);
            line_idx += 3;
        }

        if soft_break {
            if newline.is_empty() {
                return None;
            }
            continue;
        }

        out.extend_from_slice(newline);
    }

    Some(out)
}

const fn is_valid_quoted_printable_literal(byte: u8) -> bool {
    matches!(byte, b'\t' | b' ' | 33..=60 | 62..=126)
}

pub(super) fn encode_quoted_printable_body(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + body.len() / 2);
    let mut idx = 0usize;
    let mut line_len = 0usize;

    while idx < body.len() {
        let byte = body[idx];

        if byte == b'\r' {
            if idx + 1 < body.len() && body[idx + 1] == b'\n' {
                out.extend_from_slice(b"\r\n");
                idx += 2;
                line_len = 0;
                continue;
            }

            let token = quoted_printable_token(byte, false);
            if line_len + token.len() > 76 {
                out.extend_from_slice(b"=\r\n");
                line_len = 0;
            }
            out.extend_from_slice(token.as_bytes());
            line_len += token.len();
            idx += 1;
            continue;
        }

        if byte == b'\n' {
            let token = quoted_printable_token(byte, false);
            if line_len + token.len() > 76 {
                out.extend_from_slice(b"=\r\n");
                line_len = 0;
            }
            out.extend_from_slice(token.as_bytes());
            line_len += token.len();
            idx += 1;
            continue;
        }

        let next_is_newline =
            idx + 1 >= body.len() || body[idx + 1] == b'\r' || body[idx + 1] == b'\n';

        let token = quoted_printable_token(byte, next_is_newline);
        if line_len + token.len() > 76 {
            out.extend_from_slice(b"=\r\n");
            line_len = 0;
        }

        out.extend_from_slice(token.as_bytes());
        line_len += token.len();
        idx += 1;
    }

    out
}

fn quoted_printable_token(byte: u8, at_line_end: bool) -> String {
    if matches!(byte, 33..=60 | 62..=126) {
        return (byte as char).to_string();
    }

    if (byte == b' ' || byte == b'\t') && !at_line_end {
        return (byte as char).to_string();
    }

    format!("={byte:02X}")
}
