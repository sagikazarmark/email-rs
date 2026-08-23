/// Maximum input byte length accepted by [`super::parse_rfc822`]. 16 MiB is far
/// above any practical RFC 5322 message including base64-inflated
/// attachments; anything larger is treated as adversarial and rejected
/// before allocation.
pub const MAX_INPUT_BYTES: usize = 16 * 1024 * 1024;

/// Maximum nesting depth for `multipart/*` parts during inbound parse.
/// Real-world archive formats nest at most ~10 levels; 100 leaves
/// generous headroom while preventing stack-overflow on adversarial
/// input with deeply-nested multipart parts.
pub const MAX_MULTIPART_DEPTH: usize = 100;

/// Maximum number of sibling parts inside a single multipart body
/// during inbound parse. Adversarial input could otherwise produce
/// millions of empty parts (a "fan-out bomb") at one level deep.
pub const MAX_MULTIPART_PARTS: usize = 1024;

pub(super) const RFC5322_HARD_LINE_LEN: usize = 998;

pub(super) const fn hex_val(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

pub(super) fn trim_lwsp_end(value: &[u8]) -> &[u8] {
    let mut end = value.len();
    while end > 0 && (value[end - 1] == b' ' || value[end - 1] == b'\t') {
        end -= 1;
    }

    &value[..end]
}
