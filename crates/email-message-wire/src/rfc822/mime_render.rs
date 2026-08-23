use email_message::{Body, ContentTransferEncoding, Message, MimePart};

use super::MessageRenderError;
use super::attachment::{attachment_to_mime_part, partition_attachments};
use super::content_type::extract_boundary_param;
use super::header::push_header_line;
use super::shared::{MAX_MULTIPART_DEPTH, RFC5322_HARD_LINE_LEN, trim_lwsp_end};
use super::transfer_encoding::{
    encode_base64, encode_body_for_transfer_encoding, encode_quoted_printable_body,
};

type HeaderFields = Vec<(String, String)>;
type RenderedPart = (HeaderFields, Vec<u8>);
type RenderPayload = (HeaderFields, Vec<u8>, bool);

pub(super) enum RenderPart {
    Leaf {
        headers: HeaderFields,
        body: Vec<u8>,
    },
    Multipart {
        content_type: String,
        boundary: Option<String>,
        parts: Vec<Self>,
    },
}

pub(super) fn build_render_payload(
    message: &Message,
    soft_fold_at: Option<usize>,
) -> Result<RenderPayload, MessageRenderError> {
    if message.attachments().is_empty() {
        return match message.body() {
            Body::Text(text) => {
                let canonical_body = canonicalize_text_line_endings(text);
                if text.is_ascii() && !contains_overlong_physical_line(&canonical_body) {
                    Ok((Vec::new(), canonical_body, false))
                } else {
                    let root = renderable_text_leaf("text/plain", text);
                    let mut boundary_counter = 0usize;
                    let (headers, body) =
                        render_part(root, &mut boundary_counter, soft_fold_at, 0)?;
                    Ok((headers, body, true))
                }
            }
            Body::Html(html) => {
                let root = renderable_text_leaf("text/html", html);
                let mut boundary_counter = 0usize;
                let (headers, body) = render_part(root, &mut boundary_counter, soft_fold_at, 0)?;
                Ok((headers, body, true))
            }
            Body::TextAndHtml { .. } | Body::Mime(_) => {
                let root = body_to_root_part(message.body())?;
                let mut boundary_counter = 0usize;
                let (headers, body) = render_part(root, &mut boundary_counter, soft_fold_at, 0)?;
                Ok((headers, body, true))
            }
            _ => Err(MessageRenderError::UnsupportedBody),
        };
    }

    let root_body = body_to_root_part(message.body())?;
    let (inline, regular) = partition_attachments(message.attachments());

    let mut content_root = root_body;

    if !inline.is_empty() {
        let related_type = media_type_of_render_part(&content_root);
        let mut parts = vec![content_root];
        for attachment in inline {
            parts.push(attachment_to_mime_part(attachment)?);
        }

        content_root = RenderPart::Multipart {
            content_type: format!("multipart/related; type=\"{related_type}\""),
            boundary: None,
            parts,
        };
    }

    if !regular.is_empty() {
        let mut parts = vec![content_root];
        for attachment in regular {
            parts.push(attachment_to_mime_part(attachment)?);
        }

        content_root = RenderPart::Multipart {
            content_type: String::from("multipart/mixed"),
            boundary: None,
            parts,
        };
    }

    let mut boundary_counter = 0usize;
    let (headers, body) = render_part(content_root, &mut boundary_counter, soft_fold_at, 0)?;
    Ok((headers, body, true))
}

fn body_to_root_part(body: &Body) -> Result<RenderPart, MessageRenderError> {
    match body {
        Body::Text(text) => Ok(renderable_text_leaf("text/plain", text)),
        Body::Html(html) => Ok(renderable_text_leaf("text/html", html)),
        Body::TextAndHtml { text, html } => Ok(RenderPart::Multipart {
            content_type: String::from("multipart/alternative"),
            boundary: None,
            parts: vec![
                renderable_text_leaf("text/plain", text),
                renderable_text_leaf("text/html", html),
            ],
        }),
        Body::Mime(mime) => mime_to_render_part(mime, 0),
        _ => Err(MessageRenderError::UnsupportedBody),
    }
}

fn mime_to_render_part(part: &MimePart, depth: usize) -> Result<RenderPart, MessageRenderError> {
    if depth > MAX_MULTIPART_DEPTH {
        return Err(MessageRenderError::MimeNestingTooDeep);
    }
    match part {
        MimePart::Leaf {
            content_type,
            content_transfer_encoding,
            content_disposition,
            body,
        } => {
            let mut headers = vec![(
                String::from("Content-Type"),
                content_type.as_str().to_owned(),
            )];
            if let Some(value) = content_transfer_encoding {
                headers.push((
                    String::from("Content-Transfer-Encoding"),
                    value.as_str().to_owned(),
                ));
            }
            if let Some(value) = content_disposition {
                headers.push((
                    String::from("Content-Disposition"),
                    value.as_str().to_owned(),
                ));
            }

            let rendered_body = encode_body_for_transfer_encoding(
                body,
                content_transfer_encoding
                    .as_ref()
                    .map(ContentTransferEncoding::as_str),
            );

            Ok(RenderPart::Leaf {
                headers,
                body: rendered_body,
            })
        }
        MimePart::Multipart {
            content_type,
            boundary,
            parts,
        } => {
            let rendered_parts = parts
                .iter()
                .map(|part| mime_to_render_part(part, depth + 1))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(RenderPart::Multipart {
                content_type: content_type.as_str().to_owned(),
                boundary: boundary.clone(),
                parts: rendered_parts,
            })
        }
    }
}

fn renderable_text_leaf(content_type: &str, value: &str) -> RenderPart {
    let canonical_body = canonicalize_text_line_endings(value);
    let mut content_type_value = String::from(content_type);
    if value.is_ascii() {
        let mut headers = vec![(String::from("Content-Type"), content_type_value)];
        if contains_overlong_physical_line(&canonical_body) {
            headers.push((
                String::from("Content-Transfer-Encoding"),
                String::from("quoted-printable"),
            ));
            return RenderPart::Leaf {
                headers,
                body: encode_quoted_printable_body(&canonical_body),
            };
        }

        return RenderPart::Leaf {
            headers,
            body: canonical_body,
        };
    }

    content_type_value.push_str("; charset=utf-8");
    let mut headers = vec![(String::from("Content-Type"), content_type_value)];

    headers.push((
        String::from("Content-Transfer-Encoding"),
        String::from("base64"),
    ));

    RenderPart::Leaf {
        headers,
        body: encode_base64(&canonical_body),
    }
}

fn canonicalize_text_line_endings(value: &str) -> Vec<u8> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut idx = 0usize;

    while idx < bytes.len() {
        if bytes[idx] == b'\r' {
            out.extend_from_slice(b"\r\n");
            if idx + 1 < bytes.len() && bytes[idx + 1] == b'\n' {
                idx += 2;
            } else {
                idx += 1;
            }
            continue;
        }

        if bytes[idx] == b'\n' {
            out.extend_from_slice(b"\r\n");
            idx += 1;
            continue;
        }

        out.push(bytes[idx]);
        idx += 1;
    }

    out
}

fn contains_overlong_physical_line(body: &[u8]) -> bool {
    body.split(|byte| *byte == b'\n').any(|line| {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        line.len() > RFC5322_HARD_LINE_LEN
    })
}

fn validate_boundary(value: &str) -> Result<(), MessageRenderError> {
    if value.is_empty() {
        return Err(MessageRenderError::EmptyMimeBoundary);
    }

    if value.len() > 70
        || value
            .chars()
            .any(|ch| ch.is_ascii_control() || ch == '\r' || ch == '\n' || !ch.is_ascii())
    {
        return Err(MessageRenderError::InvalidMimeBoundary);
    }

    if value.ends_with(' ') {
        return Err(MessageRenderError::InvalidMimeBoundary);
    }

    if value.chars().any(|ch| {
        !(ch.is_ascii_alphanumeric()
            || matches!(
                ch,
                '\'' | '(' | ')' | '+' | '_' | ',' | '-' | '.' | '/' | ':' | '=' | '?' | ' '
            ))
    }) {
        return Err(MessageRenderError::InvalidMimeBoundary);
    }

    Ok(())
}

fn next_boundary(counter: &mut usize) -> String {
    let value = format!("=_email_message_boundary_{}", *counter);
    *counter += 1;
    value
}

fn contains_boundary_delimiter_line(body: &[u8], boundary: &str) -> bool {
    let mut delimiter = Vec::with_capacity(boundary.len() + 2);
    delimiter.extend_from_slice(b"--");
    delimiter.extend_from_slice(boundary.as_bytes());

    let mut closing = delimiter.clone();
    closing.extend_from_slice(b"--");

    body.split(|byte| *byte == b'\n').any(|raw_line| {
        let line = raw_line.strip_suffix(b"\r").unwrap_or(raw_line);
        let line = trim_lwsp_end(line);
        line == delimiter.as_slice() || line == closing.as_slice()
    })
}

fn multipart_parts_conflict_with_boundary(parts: &[RenderPart], boundary: &str) -> bool {
    parts.iter().any(|part| match part {
        RenderPart::Leaf { body, .. } => contains_boundary_delimiter_line(body, boundary),
        RenderPart::Multipart {
            content_type,
            boundary: nested_boundary,
            parts,
        } => {
            let header_boundary = extract_boundary_param(content_type);
            if nested_boundary.as_deref() == Some(boundary)
                || header_boundary.as_deref() == Some(boundary)
            {
                return true;
            }

            multipart_parts_conflict_with_boundary(parts, boundary)
        }
    })
}

fn media_type_of_render_part(part: &RenderPart) -> String {
    match part {
        RenderPart::Leaf { headers, .. } => headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .map_or_else(
                || String::from("application/octet-stream"),
                |(_, value)| {
                    value
                        .split(';')
                        .next()
                        .unwrap_or("application/octet-stream")
                        .trim()
                        .to_owned()
                },
            ),
        RenderPart::Multipart { content_type, .. } => content_type
            .split(';')
            .next()
            .unwrap_or("multipart/mixed")
            .trim()
            .to_owned(),
    }
}

fn render_part(
    part: RenderPart,
    boundary_counter: &mut usize,
    soft_fold_at: Option<usize>,
    depth: usize,
) -> Result<RenderedPart, MessageRenderError> {
    if depth > MAX_MULTIPART_DEPTH {
        return Err(MessageRenderError::MimeNestingTooDeep);
    }
    match part {
        RenderPart::Leaf { headers, body } => Ok((headers, body)),
        RenderPart::Multipart {
            content_type,
            boundary,
            parts,
        } => {
            let media_type = content_type
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            if !media_type.starts_with("multipart/") {
                return Err(MessageRenderError::InvalidMultipartContentType);
            }

            if parts.is_empty() {
                return Err(MessageRenderError::EmptyMultipartParts);
            }

            let mut content_type_value = content_type;
            let header_boundary = extract_boundary_param(&content_type_value);
            let has_header_boundary = header_boundary.is_some();

            let boundary_value = if let Some(header_boundary_value) = header_boundary {
                validate_boundary(&header_boundary_value)?;
                if let Some(explicit_boundary) = boundary.as_ref() {
                    validate_boundary(explicit_boundary)?;
                    if header_boundary_value != explicit_boundary.as_str() {
                        return Err(MessageRenderError::MismatchedMimeBoundary);
                    }
                }
                header_boundary_value
            } else {
                match boundary {
                    Some(value) => {
                        validate_boundary(&value)?;
                        value
                    }
                    None => {
                        // Cap auto-generation attempts so an adversarial body whose
                        // bytes contain successive `--=_email_message_boundary_N` lines
                        // cannot spin the renderer indefinitely.
                        const MAX_AUTO_BOUNDARY_ATTEMPTS: usize = 128;
                        let mut chosen = None;
                        for _ in 0..MAX_AUTO_BOUNDARY_ATTEMPTS {
                            let candidate = next_boundary(boundary_counter);
                            validate_boundary(&candidate)?;
                            if !multipart_parts_conflict_with_boundary(&parts, &candidate) {
                                chosen = Some(candidate);
                                break;
                            }
                        }
                        match chosen {
                            Some(value) => value,
                            None => return Err(MessageRenderError::InvalidMimeBoundary),
                        }
                    }
                }
            };

            if multipart_parts_conflict_with_boundary(&parts, &boundary_value) {
                return Err(MessageRenderError::InvalidMimeBoundary);
            }

            if !has_header_boundary {
                content_type_value.push_str("; boundary=\"");
                content_type_value.push_str(&boundary_value);
                content_type_value.push('"');
            }
            let headers = vec![(String::from("Content-Type"), content_type_value)];

            let mut body = Vec::new();

            for part in parts {
                body.extend_from_slice(b"--");
                body.extend_from_slice(boundary_value.as_bytes());
                body.extend_from_slice(b"\r\n");
                let (part_headers, part_body) =
                    render_part(part, boundary_counter, soft_fold_at, depth + 1)?;
                // The pre-render `multipart_parts_conflict_with_boundary` walk
                // checks `RenderPart::Multipart` nodes against `boundary_value`
                // by inspecting their declared `content_type` and `boundary`
                // fields, but it cannot see the bytes a nested multipart will
                // produce (those are only known after `render_part` returns).
                // Re-scan the rendered child bytes here so a nested multipart
                // whose own auto-generated or leaf body contains a line
                // matching the outer boundary cannot slip through.
                if contains_boundary_delimiter_line(&part_body, &boundary_value) {
                    return Err(MessageRenderError::InvalidMimeBoundary);
                }
                for (name, value) in part_headers {
                    push_header_line(&mut body, &name, &value, soft_fold_at)?;
                }
                body.extend_from_slice(b"\r\n");
                body.extend_from_slice(&part_body);
                body.extend_from_slice(b"\r\n");
            }

            body.extend_from_slice(b"--");
            body.extend_from_slice(boundary_value.as_bytes());
            body.extend_from_slice(b"--");
            body.extend_from_slice(b"\r\n");

            Ok((headers, body))
        }
    }
}
