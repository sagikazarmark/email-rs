use std::str::FromStr;

use email_message::{Body, ContentDisposition, ContentTransferEncoding, ContentType, MimePart};

use super::MessageParseError;
use super::content_type::ContentTypeHeader;
use super::header::{parse_header_lines_bytes, split_headers_and_body_bytes};
use super::shared::{MAX_MULTIPART_DEPTH, MAX_MULTIPART_PARTS, trim_lwsp_end};
use super::transfer_encoding::{
    decode_transfer_encoded_body, validate_multipart_transfer_encoding,
};

pub(super) fn parse_body(
    raw_body: &[u8],
    root_content_type: Option<&str>,
    root_content_transfer_encoding: Option<ContentTransferEncoding>,
) -> Result<Body, MessageParseError> {
    let Some(root_content_type) = root_content_type else {
        let decoded_root_body = decode_transfer_encoded_body(
            raw_body,
            root_content_transfer_encoding
                .as_ref()
                .map(ContentTransferEncoding::as_str),
        )?;
        return Ok(Body::Text(
            String::from_utf8_lossy(&decoded_root_body).into_owned(),
        ));
    };

    let content_type = ContentTypeHeader::parse(root_content_type);
    if content_type.media_type == "text/plain" {
        let decoded_root_body = decode_transfer_encoded_body(
            raw_body,
            root_content_transfer_encoding
                .as_ref()
                .map(ContentTransferEncoding::as_str),
        )?;
        Ok(Body::Text(decode_text_body(
            &decoded_root_body,
            content_type.charset.as_deref(),
        )))
    } else if content_type.media_type == "text/html" {
        let decoded_root_body = decode_transfer_encoded_body(
            raw_body,
            root_content_transfer_encoding
                .as_ref()
                .map(ContentTransferEncoding::as_str),
        )?;
        Ok(Body::Html(decode_text_body(
            &decoded_root_body,
            content_type.charset.as_deref(),
        )))
    } else if content_type.media_type.starts_with("multipart/") {
        validate_multipart_transfer_encoding(root_content_transfer_encoding.as_ref())?;
        let boundary = content_type
            .boundary
            .ok_or_else(|| MessageParseError::MimeBodyParse {
                details: "multipart body is missing boundary parameter".to_owned(),
            })?;
        Ok(Body::Mime(parse_multipart_body(
            raw_body,
            &content_type.normalized,
            Some(boundary),
            0,
        )?))
    } else {
        let decoded_root_body = decode_transfer_encoded_body(
            raw_body,
            root_content_transfer_encoding
                .as_ref()
                .map(ContentTransferEncoding::as_str),
        )?;
        Ok(Body::Mime(MimePart::Leaf {
            content_type: ContentType::from_str(&content_type.normalized).map_err(|_| {
                MessageParseError::MimeBodyParse {
                    details: format!("invalid content type `{}`", content_type.normalized),
                }
            })?,
            content_transfer_encoding: root_content_transfer_encoding,
            content_disposition: None,
            body: decoded_root_body,
        }))
    }
}

fn decode_text_body(body: &[u8], charset: Option<&str>) -> String {
    let Some(charset) = charset else {
        return String::from_utf8_lossy(body).into_owned();
    };

    if charset.eq_ignore_ascii_case("utf-8") || charset.eq_ignore_ascii_case("us-ascii") {
        return String::from_utf8_lossy(body).into_owned();
    }

    if charset.eq_ignore_ascii_case("iso-8859-1") || charset.eq_ignore_ascii_case("latin1") {
        return body.iter().copied().map(char::from).collect();
    }

    String::from_utf8_lossy(body).into_owned()
}

fn parse_multipart_body(
    body: &[u8],
    content_type_value: &str,
    boundary: Option<String>,
    depth: usize,
) -> Result<MimePart, MessageParseError> {
    if depth > MAX_MULTIPART_DEPTH {
        return Err(MessageParseError::MimeBodyParse {
            details: format!("multipart nesting exceeds maximum depth of {MAX_MULTIPART_DEPTH}"),
        });
    }

    let boundary = boundary.ok_or_else(|| MessageParseError::MimeBodyParse {
        details: "multipart part is missing boundary parameter".to_owned(),
    })?;

    let parts = split_multipart_parts(body, &boundary)?;
    let mut parsed_parts = Vec::with_capacity(parts.len());
    for part in parts {
        parsed_parts.push(parse_mime_part(&part, depth + 1)?);
    }

    Ok(MimePart::Multipart {
        content_type: ContentType::from_str(content_type_value).map_err(|_| {
            MessageParseError::MimeBodyParse {
                details: format!("invalid multipart content type `{content_type_value}`"),
            }
        })?,
        boundary: Some(boundary),
        parts: parsed_parts,
    })
}

fn split_multipart_parts(body: &[u8], boundary: &str) -> Result<Vec<Vec<u8>>, MessageParseError> {
    let delimiter = {
        let mut value = Vec::with_capacity(boundary.len() + 2);
        value.extend_from_slice(b"--");
        value.extend_from_slice(boundary.as_bytes());
        value
    };
    let end_delimiter = {
        let mut value = delimiter.clone();
        value.extend_from_slice(b"--");
        value
    };

    let mut parts = Vec::new();
    let mut current = Vec::new();
    let mut in_part = false;
    let mut found_opening = false;
    let mut found_closing = false;

    for raw_line in body.split(|byte| *byte == b'\n') {
        let line = raw_line.strip_suffix(b"\r").unwrap_or(raw_line);
        let line = trim_lwsp_end(line);

        if line == delimiter.as_slice() {
            if in_part {
                if parts.len() >= MAX_MULTIPART_PARTS {
                    return Err(MessageParseError::MimeBodyParse {
                        details: format!(
                            "multipart body exceeds maximum of {MAX_MULTIPART_PARTS} parts"
                        ),
                    });
                }
                strip_boundary_separator_newline(&mut current);
                parts.push(std::mem::take(&mut current));
            }
            in_part = true;
            found_opening = true;
            continue;
        }

        if line == end_delimiter.as_slice() {
            if in_part {
                if parts.len() >= MAX_MULTIPART_PARTS {
                    return Err(MessageParseError::MimeBodyParse {
                        details: format!(
                            "multipart body exceeds maximum of {MAX_MULTIPART_PARTS} parts"
                        ),
                    });
                }
                strip_boundary_separator_newline(&mut current);
                parts.push(std::mem::take(&mut current));
            }
            found_closing = true;
            break;
        }

        if in_part {
            current.extend_from_slice(raw_line);
            current.push(b'\n');
        }
    }

    if !found_closing {
        return Err(MessageParseError::MimeBodyParse {
            details: "multipart body missing closing boundary".to_owned(),
        });
    }

    if !found_opening {
        return Err(MessageParseError::MimeBodyParse {
            details: "multipart body missing opening boundary".to_owned(),
        });
    }

    Ok(parts)
}

fn parse_mime_part(part: &[u8], depth: usize) -> Result<MimePart, MessageParseError> {
    if depth > MAX_MULTIPART_DEPTH {
        return Err(MessageParseError::MimeBodyParse {
            details: format!("multipart nesting exceeds maximum depth of {MAX_MULTIPART_DEPTH}"),
        });
    }

    let (raw_headers, raw_body) = split_headers_and_body_bytes(part);
    let parsed_headers = parse_header_lines_bytes(raw_headers)?;

    let mut content_type = ContentTypeHeader {
        normalized: "text/plain".to_owned(),
        media_type: "text/plain".to_owned(),
        boundary: None,
        charset: None,
    };
    let mut content_transfer_encoding = None;
    let mut content_disposition = None;

    for (name, value) in parsed_headers {
        if name.eq_ignore_ascii_case("content-type") {
            content_type = ContentTypeHeader::parse(&value);
            continue;
        }
        if name.eq_ignore_ascii_case("content-transfer-encoding") {
            content_transfer_encoding =
                Some(ContentTransferEncoding::from_str(&value).map_err(|_| {
                    MessageParseError::MimeBodyParse {
                        details: format!("invalid content-transfer-encoding `{value}`"),
                    }
                })?);
            continue;
        }
        if name.eq_ignore_ascii_case("content-disposition") {
            content_disposition = Some(ContentDisposition::from_str(&value).map_err(|_| {
                MessageParseError::MimeBodyParse {
                    details: format!("invalid content-disposition `{value}`"),
                }
            })?);
        }
    }

    if content_type.media_type.starts_with("multipart/") {
        validate_multipart_transfer_encoding(content_transfer_encoding.as_ref())?;
        return parse_multipart_body(
            raw_body,
            &content_type.normalized,
            content_type.boundary,
            depth,
        );
    }

    let decoded_body = decode_transfer_encoded_body(
        raw_body,
        content_transfer_encoding
            .as_ref()
            .map(ContentTransferEncoding::as_str),
    )?;

    Ok(MimePart::Leaf {
        content_type: ContentType::from_str(&content_type.normalized).map_err(|_| {
            MessageParseError::MimeBodyParse {
                details: format!("invalid content type `{}`", content_type.normalized),
            }
        })?,
        content_transfer_encoding,
        content_disposition,
        body: decoded_body,
    })
}

fn strip_boundary_separator_newline(value: &mut Vec<u8>) {
    if value.ends_with(b"\r\n") {
        value.truncate(value.len() - 2);
        return;
    }

    if value.ends_with(b"\n") {
        value.truncate(value.len() - 1);
    }
}
