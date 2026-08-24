use email_message::{Address, Mailbox};

use super::encoded_word::encode_rfc2047_phrase;
use super::shared::RFC5322_HARD_LINE_LEN;
use super::{MessageParseError, MessageRenderError};

pub(super) fn render_mailbox_header(mailbox: &Mailbox) -> String {
    mailbox.name().map_or_else(
        || mailbox.email().as_str().to_owned(),
        |name| {
            format!(
                "{} <{}>",
                encode_rfc2047_phrase(name),
                mailbox.email().as_str()
            )
        },
    )
}

fn render_group_header(group: &email_message::Group) -> String {
    let mut out = String::new();
    out.push_str(&encode_rfc2047_phrase(group.name()));
    out.push(':');
    for (idx, member) in group.members().iter().enumerate() {
        if idx > 0 {
            out.push_str(", ");
        }
        out.push_str(&render_mailbox_header(member));
    }
    out.push(';');
    out
}

pub(super) fn render_address_list_header(addresses: &[Address]) -> String {
    let mut out = String::new();
    for (idx, address) in addresses.iter().enumerate() {
        if idx > 0 {
            out.push_str(", ");
        }
        match address {
            Address::Mailbox(mailbox) => out.push_str(&render_mailbox_header(mailbox)),
            Address::Group(group) => out.push_str(&render_group_header(group)),
        }
    }
    out
}

pub(super) fn split_headers_and_body_bytes(input: &[u8]) -> (&[u8], &[u8]) {
    if let Some(rest) = input.strip_prefix(b"\r\n") {
        return (&[], rest);
    }

    if let Some(rest) = input.strip_prefix(b"\n") {
        return (&[], rest);
    }

    if let Some(pos) = input.windows(4).position(|w| w == b"\r\n\r\n") {
        return (&input[..pos], &input[pos + 4..]);
    }

    if let Some(pos) = input.windows(2).position(|w| w == b"\n\n") {
        return (&input[..pos], &input[pos + 2..]);
    }

    (input, &[])
}

pub(super) fn parse_header_lines_bytes(
    raw_headers: &[u8],
) -> Result<Vec<(String, String)>, MessageParseError> {
    let normalized = raw_headers
        .split(|byte| *byte == b'\n')
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line));
    let mut output = Vec::new();
    let mut current: Option<(String, String)> = None;

    for line in normalized {
        if line.is_empty() {
            continue;
        }

        let line_str = std::str::from_utf8(line).map_err(|_| MessageParseError::InvalidUtf8)?;

        if !line_str.is_ascii() {
            return Err(MessageParseError::InvalidHeaderLine {
                line: line_str.to_owned(),
            });
        }

        if line_str
            .chars()
            .any(|ch| ch != '\t' && ch.is_ascii_control())
        {
            return Err(MessageParseError::InvalidHeaderLine {
                line: line_str.to_owned(),
            });
        }

        if line_str.starts_with(' ') || line_str.starts_with('\t') {
            let (_, value) =
                current
                    .as_mut()
                    .ok_or_else(|| MessageParseError::InvalidHeaderLine {
                        line: line_str.to_owned(),
                    })?;
            value.push_str(line_str);
            continue;
        }

        if let Some(entry) = current.take() {
            output.push(entry);
        }

        let Some((name, value)) = line_str.split_once(':') else {
            return Err(MessageParseError::InvalidHeaderLine {
                line: line_str.to_owned(),
            });
        };
        if !is_valid_header_name(name) {
            return Err(MessageParseError::InvalidHeaderLine {
                line: line_str.to_owned(),
            });
        }
        current = Some((name.trim().to_owned(), value.trim_start().to_owned()));
    }

    if let Some(entry) = current.take() {
        output.push(entry);
    }

    Ok(output)
}

/// Headers whose grammar is structured (RFC 5322 §3.6.4 / §3.6.7, RFC
/// 2369, RFC 5321) and must NOT pass through RFC 2047 encoded-word
/// substitution. Generic / custom headers default to unstructured
/// (RFC 5322 §3.6.5) and are encoded by the render loop above.
///
/// The list is intentionally small and covers the structured headers
/// most commonly found in real workflows. Less-common structured
/// headers (e.g. `Disposition-Notification-To`, `MT-Priority`,
/// `Original-Recipient`) are not on the list, if a custom header
/// with such a name carries non-ASCII content the renderer will RFC
/// 2047-encode it, which corrupts the structured grammar. Encode such
/// values ASCII-clean upstream.
pub(super) fn is_structured_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "message-id"
            | "in-reply-to"
            | "references"
            | "received"
            | "return-path"
            | "delivered-to"
            | "envelope-from"
            | "envelope-to"
            | "auto-submitted"
            | "content-id"
            | "content-location"
            | "resent-message-id"
            | "dkim-signature"
            | "arc-seal"
            | "arc-message-signature"
            | "arc-authentication-results"
            | "authentication-results"
    ) || lower.starts_with("list-")
        || lower.starts_with("x-original-")
}

pub(super) fn push_header_line(
    out: &mut Vec<u8>,
    name: &str,
    value: &str,
    soft_fold_at: Option<usize>,
) -> Result<(), MessageRenderError> {
    validate_header_name(name)?;
    if contains_raw_newlines(value) {
        return Err(MessageRenderError::HeaderContainsRawNewline {
            name: name.to_owned(),
        });
    }
    if contains_invalid_header_control_chars(value) {
        return Err(MessageRenderError::HeaderContainsControlCharacter {
            name: name.to_owned(),
        });
    }
    if !value.is_ascii() {
        return Err(MessageRenderError::HeaderContainsNonAscii {
            name: name.to_owned(),
        });
    }

    let name_len = name.len();
    let first_hard = RFC5322_HARD_LINE_LEN.saturating_sub(name_len + 2);
    let continuation_hard = RFC5322_HARD_LINE_LEN.saturating_sub(1);
    // When soft-folding is enabled, target the caller's preferred width;
    // otherwise pin preferred to the hard limit so the helper emits one
    // line per header up to the RFC 5322 ceiling.
    let first_preferred = soft_fold_at.map_or(first_hard, |target| {
        target.saturating_sub(name_len + 2).min(first_hard)
    });
    let continuation_preferred = soft_fold_at.map_or(continuation_hard, |target| {
        target.saturating_sub(1).min(continuation_hard)
    });

    let lines = split_header_value_for_folding(
        value,
        first_preferred,
        first_hard,
        continuation_preferred,
        continuation_hard,
    )
    .ok_or_else(|| MessageRenderError::HeaderLineTooLong {
        name: name.to_owned(),
    })?;

    for (idx, line) in lines.iter().enumerate() {
        if idx == 0 {
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(b": ");
            out.extend_from_slice(line.as_bytes());
            out.extend_from_slice(b"\r\n");
            continue;
        }

        out.extend_from_slice(b" ");
        out.extend_from_slice(line.as_bytes());
        out.extend_from_slice(b"\r\n");
    }

    Ok(())
}

fn split_header_value_for_folding(
    value: &str,
    first_preferred: usize,
    first_hard: usize,
    continuation_preferred: usize,
    continuation_hard: usize,
) -> Option<Vec<String>> {
    if value.is_empty() {
        return Some(vec![String::new()]);
    }

    let mut remaining = value;
    let mut lines = Vec::new();
    let mut is_first = true;

    while !remaining.is_empty() {
        let preferred = if is_first {
            first_preferred
        } else {
            continuation_preferred
        };
        let hard = if is_first {
            first_hard
        } else {
            continuation_hard
        };
        is_first = false;

        if hard == 0 {
            return None;
        }

        if remaining.len() <= preferred {
            lines.push(remaining.to_owned());
            break;
        }

        let max_preferred = preferred.min(remaining.len());

        if let Some(split_at) = last_lwsp_boundary(remaining, max_preferred) {
            lines.push(remaining[..split_at].to_owned());
            remaining = &remaining[split_at + 1..];
            continue;
        }

        if remaining.len() <= hard {
            lines.push(remaining.to_owned());
            break;
        }

        let max_hard = hard.min(remaining.len());

        if let Some(split_at) = last_lwsp_boundary(remaining, max_hard) {
            lines.push(remaining[..split_at].to_owned());
            remaining = &remaining[split_at + 1..];
            continue;
        }

        return None;
    }

    Some(lines)
}

fn last_lwsp_boundary(value: &str, max_len: usize) -> Option<usize> {
    if max_len == 0 {
        return None;
    }

    let limit = if value.is_char_boundary(max_len) {
        max_len
    } else {
        let mut idx = max_len;
        while idx > 0 && !value.is_char_boundary(idx) {
            idx -= 1;
        }
        idx
    };

    value[..limit].rfind([' ', '\t'])
}

fn validate_header_name(name: &str) -> Result<(), MessageRenderError> {
    if !is_valid_header_name(name) {
        return Err(MessageRenderError::InvalidHeaderName {
            name: name.to_owned(),
        });
    }

    Ok(())
}

fn is_valid_header_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|ch| {
            ch.is_ascii()
                && ch != ':'
                && ch != '\r'
                && ch != '\n'
                && !ch.is_ascii_whitespace()
                && !ch.is_ascii_control()
        })
}

fn contains_raw_newlines(value: &str) -> bool {
    value.contains('\r') || value.contains('\n')
}

fn contains_invalid_header_control_chars(value: &str) -> bool {
    value.chars().any(|ch| {
        matches!(
            ch,
            '\u{0000}'..='\u{0008}'
                | '\u{000B}'
                | '\u{000C}'
                | '\u{000E}'..='\u{001F}'
                | '\u{007F}'
        )
    })
}
