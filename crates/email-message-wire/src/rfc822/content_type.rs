#[derive(Clone, Debug)]
pub(super) struct ContentTypeHeader {
    pub(super) normalized: String,
    pub(super) media_type: String,
    pub(super) boundary: Option<String>,
    pub(super) charset: Option<String>,
}

impl ContentTypeHeader {
    pub(super) fn parse(value: &str) -> Self {
        let trimmed = value.trim();
        let mut parts = split_unquoted_semicolons(trimmed);
        let media_type_segment_raw = parts.next().unwrap_or_default();
        let media_type_segment = media_type_segment_raw.trim();
        let media_type = media_type_segment.to_ascii_lowercase();
        let mut boundary = None;
        let mut charset = None;
        let mut normalized_parts = vec![media_type_segment.to_owned()];

        for param in parts {
            let Some((name, value)) = param.trim().split_once('=') else {
                continue;
            };
            if name.trim().eq_ignore_ascii_case("boundary") {
                let boundary_value = unquote_parameter_value(value.trim());
                if !boundary_value.is_empty() {
                    boundary = Some(boundary_value);
                }
                continue;
            }

            normalized_parts.push(format!("{}={}", name.trim(), value.trim()));

            if name.trim().eq_ignore_ascii_case("charset") {
                let charset_value = unquote_parameter_value(value.trim());
                if !charset_value.is_empty() {
                    charset = Some(charset_value);
                }
            }
        }

        Self {
            normalized: normalized_parts.join(";"),
            media_type,
            boundary,
            charset,
        }
    }
}

pub(super) fn extract_boundary_param(value: &str) -> Option<String> {
    let mut params = split_unquoted_semicolons(value);
    let _ = params.next();

    params.find_map(|param| {
        let (name, _) = param.trim().split_once('=')?;
        if !name.trim().eq_ignore_ascii_case("boundary") {
            return None;
        }

        let (_, value) = param.trim().split_once('=')?;
        let boundary = unquote_parameter_value(value.trim());
        if boundary.is_empty() {
            return None;
        }

        Some(boundary)
    })
}

fn split_unquoted_semicolons(input: &str) -> impl Iterator<Item = &str> {
    let bytes = input.as_bytes();
    let mut start = 0usize;
    let mut idx = 0usize;
    let mut in_quotes = false;
    let mut escape = false;
    let mut done = false;

    std::iter::from_fn(move || {
        if done {
            return None;
        }

        while idx < bytes.len() {
            let ch = bytes[idx];

            if escape {
                escape = false;
                idx += 1;
                continue;
            }

            if in_quotes && ch == b'\\' {
                escape = true;
                idx += 1;
                continue;
            }

            if ch == b'"' {
                in_quotes = !in_quotes;
                idx += 1;
                continue;
            }

            if ch == b';' && !in_quotes {
                let segment = &input[start..idx];
                idx += 1;
                start = idx;
                return Some(segment);
            }

            idx += 1;
        }

        done = true;
        Some(&input[start..])
    })
}

fn unquote_parameter_value(input: &str) -> String {
    let value = input.trim();
    if !(value.starts_with('"') && value.ends_with('"') && value.len() >= 2) {
        return value.to_owned();
    }

    let mut out = String::with_capacity(value.len().saturating_sub(2));
    let mut chars = value[1..value.len() - 1].chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(escaped) = chars.next() {
                out.push(escaped);
            }
            continue;
        }
        out.push(ch);
    }
    out
}
