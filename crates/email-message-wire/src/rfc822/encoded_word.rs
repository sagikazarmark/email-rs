use std::borrow::Cow;

use base64::Engine;

use super::shared::hex_val;

/// RFC 2047 §5(3): an encoded-word MUST NOT appear within a `quoted-string`
///, implementations MUST treat such occurrences as literal. The address
/// parser the kernel delegates to (`mail_parser` 0.11.2) decodes
/// encoded-word tokens unconditionally, including inside quoted-strings,
/// which silently rewrites a display name shaped like `"=?utf-8?B?Zm9v?="`
/// into its decoded form. Until the upstream parser grows a quoted-string
/// guard, the kernel pre-processes address-typed header values to escape
/// the encoded-word lead-in (`=?`) inside quoted regions. The escape is
/// the RFC 5322 §3.2.4 quoted-pair `\=` form, which the address parser
/// strips on unquote, so the literal text reaches the caller intact.
pub(super) fn escape_encoded_words_inside_quoted_strings(input: &str) -> Cow<'_, str> {
    let bytes = input.as_bytes();
    let mut needs_escape = false;
    let mut i = 0;
    let mut in_quotes = false;
    let mut escaped_pair = false;
    while i < bytes.len() {
        let byte = bytes[i];
        if escaped_pair {
            escaped_pair = false;
            i += 1;
            continue;
        }
        match byte {
            b'\\' if in_quotes => {
                escaped_pair = true;
            }
            b'"' => {
                in_quotes = !in_quotes;
            }
            b'=' if in_quotes && i + 1 < bytes.len() && bytes[i + 1] == b'?' => {
                needs_escape = true;
                break;
            }
            _ => {}
        }
        i += 1;
    }

    if !needs_escape {
        return Cow::Borrowed(input);
    }

    let mut out = String::with_capacity(input.len() + 4);
    in_quotes = false;
    escaped_pair = false;
    for (idx, byte) in bytes.iter().copied().enumerate() {
        if escaped_pair {
            escaped_pair = false;
            out.push(byte as char);
            continue;
        }
        if in_quotes && byte == b'=' && idx + 1 < bytes.len() && bytes[idx + 1] == b'?' {
            out.push('\\');
            out.push('=');
            continue;
        }
        match byte {
            b'\\' if in_quotes => {
                escaped_pair = true;
                out.push(byte as char);
            }
            b'"' => {
                in_quotes = !in_quotes;
                out.push(byte as char);
            }
            _ => out.push(byte as char),
        }
    }
    Cow::Owned(out)
}

/// Opt-in RFC 2047 decoder for header values that the parser preserved as
/// raw `=?charset?encoding?text?=` tokens.
///
/// [`super::parse_rfc822`] decodes encoded-words for `Subject` and the address
/// headers (`From`, `Sender`, `To`, `Cc`, `Bcc`, `Reply-To`) but deliberately
/// leaves arbitrary other headers untouched, because silently rewriting
/// `=?…?=`-shaped content in opaque-bytes headers such as `X-Auth-Token`,
/// `DKIM-Signature`, `Authentication-Results`, or `ARC-*` would be a security
/// regression. Callers who *know* a header is unstructured-text-shaped and
/// want round-trip semantic equality across `parse → render` cycles can opt
/// into decoding by calling this function on the header value.
///
/// ```rust
/// use email_message_wire::{decode_rfc2047_phrase, parse_rfc822};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let bytes = b"From: from@example.com\r\nTo: to@example.com\r\nX-Note: =?utf-8?B?w6Fy?=\r\n\r\n";
/// let message = parse_rfc822(bytes)?;
/// let header = message
///     .headers()
///     .iter()
///     .find(|h| h.name().eq_ignore_ascii_case("x-note"))
///     .ok_or_else(|| std::io::Error::other("missing X-Note header"))?;
/// assert_eq!(header.value(), "=?utf-8?B?w6Fy?=");
/// assert_eq!(decode_rfc2047_phrase(header.value()), "ár");
/// # Ok(())
/// # }
/// ```
#[must_use]
pub fn decode_rfc2047_phrase(input: &str) -> Cow<'_, str> {
    decode_rfc2047_words(input)
}

pub(super) fn decode_rfc2047_words(input: &str) -> Cow<'_, str> {
    // Fast path: no encoded-word marker anywhere → return the input borrowed.
    if !input.contains("=?") {
        return Cow::Borrowed(input);
    }

    let mut out: Option<String> = None;
    let mut idx = 0usize;
    let mut prev_was_encoded_word = false;

    while idx < input.len() {
        let rest = &input[idx..];
        let Some(start_rel) = rest.find("=?") else {
            if let Some(buffer) = out.as_mut() {
                buffer.push_str(rest);
            }
            break;
        };

        let plain = &rest[..start_rel];
        let candidate = &rest[start_rel..];

        if prev_was_encoded_word
            && !plain.is_empty()
            && plain.bytes().all(|byte| byte == b' ' || byte == b'\t')
            && try_decode_rfc2047_word(candidate).is_some()
        {
            idx += start_rel;
            continue;
        }

        let buffer = out.get_or_insert_with(|| String::with_capacity(input.len()));
        // Keep the buffer in sync with everything we've consumed up to this point.
        if buffer.is_empty() && idx > 0 {
            buffer.push_str(&input[..idx]);
        }
        buffer.push_str(plain);

        if let Some((decoded, consumed)) = try_decode_rfc2047_word(candidate) {
            buffer.push_str(&decoded);
            idx += start_rel + consumed;
            prev_was_encoded_word = true;
        } else {
            buffer.push_str("=?");
            idx += start_rel + 2;
            prev_was_encoded_word = false;
        }
    }

    match out {
        Some(buffer) => Cow::Owned(buffer),
        None => Cow::Borrowed(input),
    }
}

fn try_decode_rfc2047_word(input: &str) -> Option<(String, usize)> {
    let end_rel = input.find("?=")?;
    let consumed = end_rel + 2;
    let word = &input[..consumed];
    Some((decode_rfc2047_word(word)?, consumed))
}

fn decode_rfc2047_word(word: &str) -> Option<String> {
    if !word.starts_with("=?") || !word.ends_with("?=") {
        return None;
    }

    let inner = &word[2..word.len() - 2];
    let mut parts = inner.splitn(3, '?');
    let charset = parts.next()?;
    let encoding = parts.next()?;
    let encoded = parts.next()?;

    let bytes = if encoding.eq_ignore_ascii_case("B") {
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .ok()?
    } else if encoding.eq_ignore_ascii_case("Q") {
        decode_rfc2047_q(encoded)?
    } else {
        return None;
    };

    if charset.eq_ignore_ascii_case("utf-8") || charset.eq_ignore_ascii_case("us-ascii") {
        return String::from_utf8(bytes).ok();
    }

    if charset.eq_ignore_ascii_case("iso-8859-1") || charset.eq_ignore_ascii_case("latin1") {
        return Some(bytes.into_iter().map(char::from).collect());
    }

    None
}

fn decode_rfc2047_q(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut idx = 0usize;

    while idx < bytes.len() {
        let byte = bytes[idx];
        if byte == b'_' {
            out.push(b' ');
            idx += 1;
            continue;
        }

        if byte == b'=' {
            if idx + 2 >= bytes.len() {
                return None;
            }
            let hi = hex_val(bytes[idx + 1])?;
            let lo = hex_val(bytes[idx + 2])?;
            out.push((hi << 4) | lo);
            idx += 3;
            continue;
        }

        out.push(byte);
        idx += 1;
    }

    Some(out)
}

pub(super) fn encode_rfc2047_unstructured(input: &str) -> String {
    if input.is_ascii() {
        return input.to_owned();
    }

    encode_rfc2047_utf8_base64_words(input)
}

pub(super) fn encode_rfc2047_phrase(input: &str) -> String {
    if input.is_ascii() {
        return quote_phrase(input);
    }

    encode_rfc2047_utf8_base64_words(input)
}

fn encode_rfc2047_utf8_base64_words(input: &str) -> String {
    const ENCODED_WORD_OVERHEAD: usize = 12; // =?utf-8?B? + ?=
    const MAX_ENCODED_WORD_LEN: usize = 75;
    const MAX_BASE64_LEN: usize = MAX_ENCODED_WORD_LEN - ENCODED_WORD_OVERHEAD;
    const MAX_CHUNK_BYTES: usize = (MAX_BASE64_LEN / 4) * 3;

    let bytes = input.as_bytes();
    let mut idx = 0usize;
    let mut words = Vec::new();

    while idx < bytes.len() {
        let mut end = (idx + MAX_CHUNK_BYTES).min(bytes.len());
        while end > idx && !input.is_char_boundary(end) {
            end -= 1;
        }

        if end == idx {
            end = bytes.len();
            while end > idx && !input.is_char_boundary(end) {
                end -= 1;
            }
        }

        let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes[idx..end]);
        words.push(format!("=?utf-8?B?{encoded}?="));
        idx = end;
    }

    words.join(" ")
}

fn quote_phrase(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 2);
    out.push('"');
    for ch in input.chars() {
        if ch == '\\' || ch == '"' {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    out
}
