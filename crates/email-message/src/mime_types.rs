//! MIME content-type, content-disposition, and content-transfer-encoding
//! types.
//!
//! Most of this module, `ContentType`, `MediaType`, `ContentDisposition`,
//! `ContentTransferEncoding`, `ParameterValue`, is **always available**
//! regardless of feature flags. The `mime` Cargo feature gates only
//! [`MimePart`], the multipart/leaf MIME tree used by full-message
//! rendering. A consumer that just wants typed content-type validation
//! can use `email-message` with `default-features = false` and skip the
//! `mime` feature.

use std::fmt::Display;
use std::str::FromStr;

/// MIME content type.
///
/// # Equality and hashing
///
/// `PartialEq` / `Eq` / `Hash` are derived. To make derived equality
/// match RFC 2045 §5.1 semantics (type, subtype, and parameter names
/// are case-insensitive), construction lowercases those tokens. Parameter
/// values are preserved as-is because their case sensitivity depends on
/// the parameter (`boundary` is case-sensitive per RFC 2046 §5.1.1;
/// `charset` is case-insensitive per RFC 2046 §4.1.2 but the kernel
/// leaves the caller's bytes intact for round-trip fidelity).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ContentType(String);

impl ContentType {
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Borrowed type/subtype view, with no parameters.
    ///
    /// Cheap: it slices the stored string; no allocation. Validation guarantees
    /// a well-formed `type/subtype` prefix exists.
    #[must_use]
    pub fn media_type(&self) -> MediaType<'_> {
        let head = self.0.split(';').next().unwrap_or("").trim();
        let (type_, subtype) = head.split_once('/').unwrap_or((head, ""));
        MediaType { type_, subtype }
    }

    /// Iterate `(name, value)` parameter pairs in declaration order.
    ///
    /// Quoted values are returned with surrounding quotes stripped and
    /// backslash escapes resolved.
    pub fn parameters(&self) -> impl Iterator<Item = (&str, ParameterValue<'_>)> {
        let mut segments = split_content_type_segments(self.0.as_str()).into_iter();
        // Skip the type/subtype segment.
        let _ = segments.next();
        segments.filter_map(|segment| {
            let (name, value) = segment.trim().split_once('=')?;
            Some((name.trim(), ParameterValue::from_raw(value.trim())))
        })
    }

    /// Look up a parameter by case-insensitive name.
    #[must_use]
    pub fn parameter(&self, name: &str) -> Option<ParameterValue<'_>> {
        self.parameters()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value)
    }

    /// Convenience accessor for the `boundary` parameter (multipart only).
    #[must_use]
    pub fn boundary(&self) -> Option<ParameterValue<'_>> {
        self.parameter("boundary")
    }

    /// Convenience accessor for the `charset` parameter.
    #[must_use]
    pub fn charset(&self) -> Option<ParameterValue<'_>> {
        self.parameter("charset")
    }
}

/// Borrowed view of a content-type's `type/subtype`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MediaType<'a> {
    type_: &'a str,
    subtype: &'a str,
}

impl<'a> MediaType<'a> {
    #[must_use]
    pub const fn type_(&self) -> &'a str {
        self.type_
    }

    #[must_use]
    pub const fn subtype(&self) -> &'a str {
        self.subtype
    }

    #[must_use]
    pub fn is_text(&self) -> bool {
        self.type_.eq_ignore_ascii_case("text")
    }

    #[must_use]
    pub fn is_multipart(&self) -> bool {
        self.type_.eq_ignore_ascii_case("multipart")
    }

    #[must_use]
    pub fn is_image(&self) -> bool {
        self.type_.eq_ignore_ascii_case("image")
    }

    /// Case-insensitive compare against a `"type/subtype"` literal.
    #[must_use]
    pub fn matches(&self, expected: &str) -> bool {
        let Some((ty, sub)) = expected.split_once('/') else {
            return false;
        };
        self.type_.eq_ignore_ascii_case(ty) && self.subtype.eq_ignore_ascii_case(sub)
    }
}

/// Borrowed parameter value, lazily resolving quoted-string escapes.
#[derive(Clone, Debug)]
pub struct ParameterValue<'a> {
    raw: &'a str,
}

impl<'a> ParameterValue<'a> {
    fn from_raw(raw: &'a str) -> Self {
        Self { raw }
    }

    /// Raw textual form as it appears in the header (still quoted/escaped if it
    /// was emitted that way).
    #[must_use]
    pub const fn as_raw(&self) -> &'a str {
        self.raw
    }

    /// Returns the unquoted, unescaped string. For unquoted values this is a
    /// borrow; for quoted values it allocates only to materialize the escapes.
    #[must_use]
    pub fn unquoted(&self) -> std::borrow::Cow<'a, str> {
        let raw = self.raw;
        if !raw.starts_with('"') || !raw.ends_with('"') || raw.len() < 2 {
            return std::borrow::Cow::Borrowed(raw);
        }

        let inner = &raw[1..raw.len() - 1];
        if !inner.contains('\\') {
            return std::borrow::Cow::Borrowed(inner);
        }

        let mut out = String::with_capacity(inner.len());
        let mut escaped = false;
        for ch in inner.chars() {
            if escaped {
                out.push(ch);
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else {
                out.push(ch);
            }
        }
        std::borrow::Cow::Owned(out)
    }
}

impl PartialEq<&str> for ParameterValue<'_> {
    fn eq(&self, other: &&str) -> bool {
        self.unquoted().as_ref() == *other
    }
}

impl Display for ContentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("content type must have a type/subtype form")]
pub struct ContentTypeParseError;

impl FromStr for ContentType {
    type Err = ContentTypeParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        normalize_parameterized_value(s, true)
            .map(Self)
            .ok_or(ContentTypeParseError)
    }
}

fn is_mime_token(value: &str) -> bool {
    value.bytes().all(is_mime_token_byte)
}

fn split_content_type_segments(value: &str) -> Vec<&str> {
    let mut segments = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    let mut escaped = false;

    for (index, ch) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }

        if in_quotes && ch == '\\' {
            escaped = true;
            continue;
        }

        if ch == '"' {
            in_quotes = !in_quotes;
            continue;
        }

        if ch == ';' && !in_quotes {
            segments.push(&value[start..index]);
            start = index + ch.len_utf8();
        }
    }

    segments.push(&value[start..]);
    segments
}

const fn is_mime_token_byte(byte: u8) -> bool {
    matches!(
        byte,
        b'!' | b'#'
            | b'$'
            | b'%'
            | b'&'
            | b'\''
            | b'*'
            | b'+'
            | b'-'
            | b'.'
            | b'^'
            | b'_'
            | b'`'
            | b'|'
            | b'~'
            | b'0'..=b'9'
            | b'A'..=b'Z'
            | b'a'..=b'z'
    )
}

fn is_parameter_value(value: &str) -> bool {
    if value.starts_with('"') {
        return is_quoted_parameter_value(value);
    }

    is_mime_token(value)
}

fn is_quoted_parameter_value(value: &str) -> bool {
    if !(value.ends_with('"') && value.len() >= 2) {
        return false;
    }

    let mut escaped = false;
    for byte in value[1..value.len() - 1].bytes() {
        if escaped {
            if is_forbidden_quoted_parameter_byte(byte) {
                return false;
            }
            escaped = false;
            continue;
        }

        if byte == b'\\' {
            escaped = true;
            continue;
        }

        if byte == b'"' || is_forbidden_quoted_parameter_byte(byte) {
            return false;
        }
    }

    !escaped
}

/// Reject NUL, CR, LF, and any non-tab ASCII control character inside a
/// MIME quoted parameter. Matches the byte-discipline `validate_header`
/// (in `crate::message`) and `push_header_line` (in
/// `email_message_wire::rfc822`) apply to header values, so a parsed
/// `ContentType` cannot carry bytes the wire renderer would later
/// reject (META-001 R3 invariant).
const fn is_forbidden_quoted_parameter_byte(byte: u8) -> bool {
    byte != b'\t' && byte.is_ascii_control()
}

impl TryFrom<&str> for ContentType {
    type Error = ContentTypeParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::from_str(value)
    }
}

impl From<ContentType> for String {
    fn from(value: ContentType) -> Self {
        value.0
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for ContentType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for ContentType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(feature = "schemars")]
impl schemars::JsonSchema for ContentType {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ContentType".into()
    }

    fn schema_id() -> std::borrow::Cow<'static, str> {
        concat!(module_path!(), "::ContentType").into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "description": "MIME Content-Type field value"
        })
    }
}

#[cfg(feature = "arbitrary")]
impl<'a> arbitrary::Arbitrary<'a> for ContentType {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let value = match u.int_in_range::<u8>(0..=4)? {
            0 => "text/plain",
            1 => "text/html; charset=utf-8",
            2 => "application/octet-stream",
            3 => "image/png",
            _ => "multipart/mixed; boundary=boundary",
        };
        value.parse().map_err(|_| arbitrary::Error::IncorrectFormat)
    }
}

/// MIME content-transfer-encoding (RFC 2045 §6).
///
/// The five RFC-defined values are explicit variants; any other syntactically
/// valid mime-token (e.g. an `x-` extension) round-trips through `Other`.
///
/// # Casing
///
/// RFC 2045 §6.1 says encoding names are case-insensitive. Both the
/// known-variant parser and the [`Other`] branch normalize to ASCII
/// lowercase on construction, so equality and hashing through the
/// derived impls are case-insensitive automatically: `Other("Base64")`
/// is unreachable (parses to [`Base64`] instead) and `Other("X-MyEnc")`
/// stores `"x-myenc"`.
///
/// [`Base64`]: Self::Base64
/// [`Other`]: Self::Other
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ContentTransferEncoding {
    SevenBit,
    EightBit,
    Binary,
    QuotedPrintable,
    Base64,
    Other(String),
}

impl ContentTransferEncoding {
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::SevenBit => "7bit",
            Self::EightBit => "8bit",
            Self::Binary => "binary",
            Self::QuotedPrintable => "quoted-printable",
            Self::Base64 => "base64",
            Self::Other(value) => value.as_str(),
        }
    }
}

impl Display for ContentTransferEncoding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("content-transfer-encoding cannot be empty")]
pub struct ContentTransferEncodingParseError;

impl FromStr for ContentTransferEncoding {
    type Err = ContentTransferEncodingParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let value = s.trim();
        if value.is_empty() || !is_mime_token(value) {
            return Err(ContentTransferEncodingParseError);
        }
        Ok(if value.eq_ignore_ascii_case("7bit") {
            Self::SevenBit
        } else if value.eq_ignore_ascii_case("8bit") {
            Self::EightBit
        } else if value.eq_ignore_ascii_case("binary") {
            Self::Binary
        } else if value.eq_ignore_ascii_case("quoted-printable") {
            Self::QuotedPrintable
        } else if value.eq_ignore_ascii_case("base64") {
            Self::Base64
        } else {
            Self::Other(value.to_ascii_lowercase())
        })
    }
}

impl TryFrom<&str> for ContentTransferEncoding {
    type Error = ContentTransferEncodingParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::from_str(value)
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for ContentTransferEncoding {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for ContentTransferEncoding {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(feature = "schemars")]
impl schemars::JsonSchema for ContentTransferEncoding {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ContentTransferEncoding".into()
    }

    fn schema_id() -> std::borrow::Cow<'static, str> {
        concat!(module_path!(), "::ContentTransferEncoding").into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "description": "RFC 2045 Content-Transfer-Encoding token"
        })
    }
}

#[cfg(feature = "arbitrary")]
impl<'a> arbitrary::Arbitrary<'a> for ContentTransferEncoding {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(match u.int_in_range::<u8>(0..=5)? {
            0 => Self::SevenBit,
            1 => Self::EightBit,
            2 => Self::Binary,
            3 => Self::QuotedPrintable,
            4 => Self::Base64,
            _ => Self::Other("x-experimental".to_owned()),
        })
    }
}

/// MIME content-disposition token (RFC 2183).
///
/// # Equality and hashing
///
/// Same shape as [`ContentType`]: construction lowercases the disposition
/// kind and parameter names, then `PartialEq` / `Eq` / `Hash` compare that
/// normalized string. RFC 2183 §3 makes the disposition type and parameter
/// names case-insensitive but leaves parameter value case sensitivity
/// dependent on the parameter. The kernel preserves parameter values
/// verbatim; for semantic comparison route through the disposition's
/// accessors rather than comparing raw input strings.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ContentDisposition(String);

impl ContentDisposition {
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Borrowed disposition kind (`"inline"`, `"attachment"`, or an
    /// `x-` extension), with no parameters.
    ///
    /// Cheap: it slices the stored string; no allocation. Validation
    /// guarantees a well-formed disposition token prefix exists.
    #[must_use]
    pub fn kind(&self) -> &str {
        self.0.split(';').next().unwrap_or("").trim()
    }

    /// Iterate `(name, value)` parameter pairs in declaration order.
    ///
    /// Quoted values are returned with surrounding quotes stripped and
    /// backslash escapes resolved, mirroring [`ContentType::parameters`].
    pub fn parameters(&self) -> impl Iterator<Item = (&str, ParameterValue<'_>)> {
        let mut segments = split_content_type_segments(self.0.as_str()).into_iter();
        // Skip the disposition-kind segment.
        let _ = segments.next();
        segments.filter_map(|segment| {
            let (name, value) = segment.trim().split_once('=')?;
            Some((name.trim(), ParameterValue::from_raw(value.trim())))
        })
    }

    /// Look up a parameter by case-insensitive name.
    #[must_use]
    pub fn parameter(&self, name: &str) -> Option<ParameterValue<'_>> {
        self.parameters()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value)
    }

    /// Convenience accessor for the `filename` parameter.
    ///
    /// RFC 2183 §2.3 defines this as the suggested filename a recipient's
    /// mail client should use when saving the attachment to disk. For
    /// non-ASCII filenames the kernel emits `filename*` (RFC 2231
    /// charset/language extension); this accessor returns `filename` when
    /// present and otherwise falls back to `filename*`.
    #[must_use]
    pub fn filename(&self) -> Option<ParameterValue<'_>> {
        self.parameter("filename")
            .or_else(|| self.parameter("filename*"))
    }

    /// Returns `true` if the disposition kind is `inline`
    /// (case-insensitive).
    #[must_use]
    pub fn is_inline(&self) -> bool {
        self.kind().eq_ignore_ascii_case("inline")
    }

    /// Returns `true` if the disposition kind is `attachment`
    /// (case-insensitive).
    #[must_use]
    pub fn is_attachment(&self) -> bool {
        self.kind().eq_ignore_ascii_case("attachment")
    }
}

impl Display for ContentDisposition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("content-disposition cannot be empty")]
pub struct ContentDispositionParseError;

impl FromStr for ContentDisposition {
    type Err = ContentDispositionParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        normalize_parameterized_value(s, false)
            .map(Self)
            .ok_or(ContentDispositionParseError)
    }
}

impl TryFrom<&str> for ContentDisposition {
    type Error = ContentDispositionParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::from_str(value)
    }
}

/// Validate and normalize a parameterized header value (`Content-Type` shape
/// or `Content-Disposition` shape). When `with_subtype` is true, the head
/// must be `type/subtype`; otherwise it must be a single MIME token.
///
/// Lowercases the type/subtype tokens and parameter names so derived
/// equality matches RFC 2045 §5.1 semantics. Parameter values are
/// preserved verbatim. Returns `None` if the input fails any validation
/// rule the previous bool-returning checks enforced.
fn normalize_parameterized_value(value: &str, with_subtype: bool) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    let segments = split_content_type_segments(value);
    let mut parts = segments.into_iter();
    let head = parts.next()?.trim();

    let canonical_head = if with_subtype {
        let (ty, subtype) = head.split_once('/')?;
        if ty.is_empty()
            || subtype.is_empty()
            || subtype.contains('/')
            || !is_mime_token(ty)
            || !is_mime_token(subtype)
        {
            return None;
        }
        format!(
            "{}/{}",
            ty.to_ascii_lowercase(),
            subtype.to_ascii_lowercase()
        )
    } else {
        if head.is_empty() || !is_mime_token(head) {
            return None;
        }
        head.to_ascii_lowercase()
    };

    let mut canonical = canonical_head;
    for parameter in parts {
        let parameter = parameter.trim();
        let (name, raw_value) = parameter.split_once('=')?;
        let name = name.trim();
        let raw_value = raw_value.trim();
        if name.is_empty()
            || raw_value.is_empty()
            || !is_mime_token(name)
            || !is_parameter_value(raw_value)
        {
            return None;
        }
        canonical.push_str("; ");
        canonical.push_str(&name.to_ascii_lowercase());
        canonical.push('=');
        canonical.push_str(raw_value);
    }

    Some(canonical)
}

#[cfg(feature = "serde")]
impl serde::Serialize for ContentDisposition {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for ContentDisposition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(feature = "schemars")]
impl schemars::JsonSchema for ContentDisposition {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ContentDisposition".into()
    }

    fn schema_id() -> std::borrow::Cow<'static, str> {
        concat!(module_path!(), "::ContentDisposition").into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "description": "RFC 2183 Content-Disposition field value"
        })
    }
}

#[cfg(feature = "arbitrary")]
impl<'a> arbitrary::Arbitrary<'a> for ContentDisposition {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let value = match u.int_in_range::<u8>(0..=2)? {
            0 => "inline",
            1 => "attachment",
            _ => "attachment; filename=example.txt",
        };
        value.parse().map_err(|_| arbitrary::Error::IncorrectFormat)
    }
}

/// Low-level MIME tree node, gated behind the `mime` Cargo feature.
///
/// `MimePart` is the kernel's escape hatch for callers building exotic
/// MIME structures (custom multipart shapes, hand-rolled
/// transfer-encoding choices, etc.). High-level paths through
/// [`Body::Text`](crate::Body) / `Body::Html` / `Body::TextAndHtml` cover
/// the common cases and apply byte-discipline (auto-promote non-ASCII
/// text to base64, etc.) on the caller's behalf.
///
/// # Body byte-discipline is the caller's responsibility
///
/// Constructing `MimePart::Leaf` directly bypasses the kernel's
/// auto-promotion path. The wire renderer enforces *header* invariants
/// strictly (rejects raw CR / LF / NUL / non-tab control chars in any
/// header value, regardless of `Content-Transfer-Encoding`), but it
/// **trusts the caller's bytes** for body content under any transfer
/// encoding other than `base64` / `quoted-printable`. That includes
/// `7bit`, `8bit`, `binary`, and any `Other(...)` value: the renderer
/// emits the body verbatim. RFC 2045 §6.2 forbids bytes > 127 under
/// `7bit` and forbids bare CR / LF under both `7bit` and `8bit`;
/// callers building `MimePart::Leaf` with a non-base64 / non-QP
/// encoding must satisfy those invariants themselves, or downstream
/// MTAs may reject the message.
///
/// # Variant set
///
/// Deliberately *not* `#[non_exhaustive]`. RFC 2046 closes MIME
/// parts to exactly `discrete` (Leaf) and `composite` (Multipart);
/// the kernel cannot honestly add a third variant without an RFC
/// update. The exhaustive `match` shape lets downstream callers
/// type-cover both arms without an `_ =>` clause.
///
/// # Untrusted-deserialize caveat
///
/// `MimePart::Multipart { parts: Vec<Self> }` is recursive: any
/// caller deserializing a `MimePart` (or a `Body` containing one)
/// from untrusted input must pre-bound the input length and the
/// recursion depth. `serde_json` defaults to a 128-frame recursion
/// limit which is safe; other formats (e.g. `serde_yaml`,
/// `bincode`, `rmp-serde`, `serde_cbor`) may not, and a deeply
/// nested attacker payload yields a `MimePart` value of arbitrary
/// depth. The wire renderer (`email_message_wire::render_rfc822`)
/// enforces a `MAX_MULTIPART_DEPTH` cap on outbound trees, including
/// up to two frames of attachment-wrapping when inline and/or regular
/// attachments are present, but other consumers of a deserialized
/// `MimePart` (e.g. arbitrary caller code that walks the tree) must
/// defend themselves.
#[cfg(feature = "mime")]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MimePart {
    Leaf {
        content_type: ContentType,
        content_transfer_encoding: Option<ContentTransferEncoding>,
        content_disposition: Option<ContentDisposition>,
        body: Vec<u8>,
    },
    Multipart {
        content_type: ContentType,
        boundary: Option<String>,
        parts: Vec<Self>,
    },
}

#[cfg(all(feature = "mime", feature = "serde"))]
impl serde::Serialize for MimePart {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use base64::Engine as _;
        use serde::ser::SerializeStruct as _;

        match self {
            Self::Leaf {
                content_type,
                content_transfer_encoding,
                content_disposition,
                body,
            } => {
                let mut len = 3; // type + content_type + body
                if content_transfer_encoding.is_some() {
                    len += 1;
                }
                if content_disposition.is_some() {
                    len += 1;
                }
                let encoded = base64::engine::general_purpose::STANDARD.encode(body);
                let mut value = serializer.serialize_struct("MimePart", len)?;
                value.serialize_field("type", "leaf")?;
                value.serialize_field("content_type", content_type)?;
                if let Some(cte) = content_transfer_encoding {
                    value.serialize_field("content_transfer_encoding", cte)?;
                }
                if let Some(cd) = content_disposition {
                    value.serialize_field("content_disposition", cd)?;
                }
                value.serialize_field("body", &encoded)?;
                value.end()
            }
            Self::Multipart {
                content_type,
                boundary,
                parts,
            } => {
                let mut len = 3; // type + content_type + parts
                if boundary.is_some() {
                    len += 1;
                }
                let mut value = serializer.serialize_struct("MimePart", len)?;
                value.serialize_field("type", "multipart")?;
                value.serialize_field("content_type", content_type)?;
                if let Some(b) = boundary {
                    value.serialize_field("boundary", b)?;
                }
                value.serialize_field("parts", parts)?;
                value.end()
            }
        }
    }
}

#[cfg(all(feature = "mime", feature = "serde"))]
impl<'de> serde::Deserialize<'de> for MimePart {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use base64::Engine as _;

        #[derive(serde::Deserialize)]
        #[serde(tag = "type", rename_all = "snake_case")]
        enum RawMimePart {
            Leaf {
                content_type: ContentType,
                #[serde(default)]
                content_transfer_encoding: Option<ContentTransferEncoding>,
                #[serde(default)]
                content_disposition: Option<ContentDisposition>,
                body: String,
            },
            Multipart {
                content_type: ContentType,
                #[serde(default)]
                boundary: Option<String>,
                #[serde(default)]
                parts: Vec<MimePart>,
            },
        }

        Ok(match RawMimePart::deserialize(deserializer)? {
            RawMimePart::Leaf {
                content_type,
                content_transfer_encoding,
                content_disposition,
                body,
            } => {
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(body.as_bytes())
                    .map_err(|err| {
                        serde::de::Error::custom(format!("invalid base64 MIME body: {err}"))
                    })?;
                Self::Leaf {
                    content_type,
                    content_transfer_encoding,
                    content_disposition,
                    body: decoded,
                }
            }
            RawMimePart::Multipart {
                content_type,
                boundary,
                parts,
            } => Self::Multipart {
                content_type,
                boundary,
                parts,
            },
        })
    }
}

#[cfg(all(feature = "mime", feature = "schemars"))]
impl schemars::JsonSchema for MimePart {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "MimePart".into()
    }

    fn schema_id() -> std::borrow::Cow<'static, str> {
        concat!(module_path!(), "::MimePart").into()
    }

    /// MIME parts have no RFC 5322 string form, so this schema is *not*
    /// wrapped in an `rfc5322-string-compat` `oneOf: [object, string]`
    /// the way `Mailbox` / `Group` / `Address` are. The asymmetry is
    /// deliberate: there is no producer-side wire shape for "MIME part
    /// as a header-like string" to migrate from.
    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let recursive = generator.subschema_for::<MimePart>();
        schemars::json_schema!({
            "oneOf": [
                {
                    "type": "object",
                    "properties": {
                        "type": {"const": "leaf"},
                        "content_type": {
                            "type": "string",
                            "description": "MIME Content-Type field value"
                        },
                        "content_transfer_encoding": {
                            "type": "string",
                            "description": "RFC 2045 Content-Transfer-Encoding token"
                        },
                        "content_disposition": {
                            "type": "string",
                            "description": "RFC 2183 Content-Disposition field value"
                        },
                        "body": {
                            "type": "string",
                            "contentEncoding": "base64",
                            "description": "Base64-encoded MIME part body (RFC 4648, with padding)"
                        }
                    },
                    "required": ["type", "content_type", "body"]
                },
                {
                    "type": "object",
                    "properties": {
                        "type": {"const": "multipart"},
                        "content_type": {
                            "type": "string",
                            "description": "MIME Content-Type field value"
                        },
                        "boundary": {"type": "string"},
                        "parts": {"type": "array", "items": recursive}
                    },
                    "required": ["type", "content_type", "parts"]
                }
            ]
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{ContentTransferEncoding, ContentType};

    #[test]
    fn content_type_accepts_valid_media_types_and_parameters() {
        for value in [
            "text/plain",
            "text/plain;charset=utf-8",
            "multipart/related; type=\"text/html\"",
            "application/octet-stream; name=\"a;b.txt\"",
        ] {
            assert!(
                ContentType::try_from(value).is_ok(),
                "expected valid content type: {value}"
            );
        }
    }

    #[test]
    fn content_type_rejects_invalid_media_types() {
        for value in [
            "text/",
            "/plain",
            "text/plain/html",
            "text /plain",
            "text/plain; charset",
            "text/plain; charset=\"unterminated",
        ] {
            assert!(
                ContentType::try_from(value).is_err(),
                "expected invalid content type: {value}"
            );
        }
    }

    #[test]
    fn content_type_rejects_quoted_parameter_with_control_chars() {
        // Direct bytes, NUL, BEL, VT, ESC must be rejected to match the
        // wire renderer's `push_header_line` byte discipline.
        for value in [
            "text/plain; name=\"x\u{0}y\"",
            "text/plain; name=\"x\u{07}y\"",
            "text/plain; name=\"x\u{0B}y\"",
            "text/plain; name=\"x\u{1B}y\"",
        ] {
            assert!(
                ContentType::try_from(value).is_err(),
                "expected control-char rejection: {value:?}"
            );
        }
    }

    #[test]
    fn content_type_rejects_quoted_parameter_with_escaped_control_chars() {
        // Even after a `\` escape, control chars are still rejected.
        for value in [
            "text/plain; name=\"x\\\u{0}y\"",
            "text/plain; name=\"x\\\u{07}y\"",
        ] {
            assert!(
                ContentType::try_from(value).is_err(),
                "expected escaped-control-char rejection: {value:?}"
            );
        }
    }

    #[test]
    fn content_type_accepts_tab_inside_quoted_parameter() {
        // Tab is the documented exception in the byte-discipline rule.
        assert!(ContentType::try_from("text/plain; name=\"a\tb\"").is_ok());
    }

    #[test]
    fn content_type_media_type_view_splits_type_and_subtype() {
        let ct: ContentType = "text/plain; charset=utf-8".parse().unwrap();
        let media = ct.media_type();
        assert_eq!(media.type_(), "text");
        assert_eq!(media.subtype(), "plain");
        assert!(media.is_text());
        assert!(!media.is_multipart());
        assert!(media.matches("text/plain"));
        assert!(media.matches("TEXT/PLAIN"));
    }

    #[test]
    fn content_type_parameter_lookup_is_case_insensitive_and_unquotes() {
        let ct: ContentType = "multipart/mixed; Boundary=\"abc\\\"def\"".parse().unwrap();
        let boundary = ct.boundary().expect("boundary present");
        assert_eq!(boundary.as_raw(), "\"abc\\\"def\"");
        assert_eq!(boundary.unquoted().as_ref(), "abc\"def");
    }

    #[test]
    fn content_type_parameters_iterates_in_declaration_order() {
        let ct: ContentType = "text/html; charset=utf-8; boundary=x".parse().unwrap();
        let pairs: Vec<(String, String)> = ct
            .parameters()
            .map(|(k, v)| (k.to_owned(), v.unquoted().into_owned()))
            .collect();
        assert_eq!(
            pairs,
            vec![
                ("charset".to_owned(), "utf-8".to_owned()),
                ("boundary".to_owned(), "x".to_owned()),
            ]
        );
    }

    #[test]
    fn content_transfer_encoding_canonicalizes_known_tokens() {
        assert_eq!(
            "Base64"
                .parse::<ContentTransferEncoding>()
                .unwrap()
                .as_str(),
            "base64"
        );
        assert_eq!(
            "7BIT".parse::<ContentTransferEncoding>().unwrap().as_str(),
            "7bit"
        );
        assert_eq!(
            "Quoted-Printable"
                .parse::<ContentTransferEncoding>()
                .unwrap(),
            ContentTransferEncoding::QuotedPrintable
        );

        let other: ContentTransferEncoding = "x-my-encoding".parse().unwrap();
        assert_eq!(
            other,
            ContentTransferEncoding::Other("x-my-encoding".to_owned())
        );
        assert_eq!(other.as_str(), "x-my-encoding");
    }

    #[test]
    fn content_disposition_kind_and_parameter_accessors() {
        use super::ContentDisposition;
        let cd: ContentDisposition = "attachment; filename=\"report.pdf\""
            .parse()
            .expect("disposition should parse");
        assert_eq!(cd.kind(), "attachment");
        assert!(cd.is_attachment());
        assert!(!cd.is_inline());
        let filename = cd.filename().expect("filename present");
        assert_eq!(filename.unquoted().as_ref(), "report.pdf");
    }

    #[test]
    fn content_disposition_filename_falls_back_to_extended_parameter() {
        use super::ContentDisposition;
        let cd: ContentDisposition = "attachment; filename*=utf-8''f%C3%A1jl.txt"
            .parse()
            .expect("disposition should parse");

        let filename = cd.filename().expect("filename* present");
        assert_eq!(filename.as_raw(), "utf-8''f%C3%A1jl.txt");
    }

    #[test]
    fn content_disposition_inline_kind_is_case_insensitive() {
        use super::ContentDisposition;
        let cd: ContentDisposition = "INLINE".parse().expect("disposition should parse");
        assert!(cd.is_inline());
        assert!(!cd.is_attachment());
    }

    #[test]
    fn content_disposition_parameters_iterates_in_declaration_order() {
        use super::ContentDisposition;
        let cd: ContentDisposition = "attachment; filename=report.pdf; size=42".parse().unwrap();
        let pairs: Vec<(String, String)> = cd
            .parameters()
            .map(|(k, v)| (k.to_owned(), v.unquoted().into_owned()))
            .collect();
        assert_eq!(
            pairs,
            vec![
                ("filename".to_owned(), "report.pdf".to_owned()),
                ("size".to_owned(), "42".to_owned()),
            ]
        );
    }

    #[test]
    fn content_disposition_parameter_lookup_is_case_insensitive() {
        use super::ContentDisposition;
        let cd: ContentDisposition = "attachment; FileName=\"x.txt\"".parse().unwrap();
        assert_eq!(
            cd.parameter("filename").unwrap().unquoted().as_ref(),
            "x.txt"
        );
        assert_eq!(
            cd.parameter("FILENAME").unwrap().unquoted().as_ref(),
            "x.txt"
        );
    }

    #[test]
    fn content_transfer_encoding_other_is_case_insensitive() {
        // RFC 2045 §6.1, encoding names are case-insensitive. Two
        // differently-cased spellings of the same x-* extension must
        // compare equal and hash to the same value.
        let a: ContentTransferEncoding = "X-MyEnc".parse().unwrap();
        let b: ContentTransferEncoding = "x-myenc".parse().unwrap();
        let c: ContentTransferEncoding = "X-MYENC".parse().unwrap();
        assert_eq!(a, b);
        assert_eq!(a, c);
        assert_eq!(a.as_str(), "x-myenc");
        assert_eq!(c.as_str(), "x-myenc");

        // Same value can be safely used as a HashMap/HashSet key.
        use std::collections::HashSet;
        let mut set: HashSet<ContentTransferEncoding> = HashSet::new();
        set.insert(a);
        assert!(set.contains(&b));
        assert!(set.contains(&c));
    }

    #[test]
    fn content_type_eq_is_case_insensitive_after_normalize() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let upper = ContentType::try_from("TEXT/PLAIN; CHARSET=UTF-8").unwrap();
        let lower = ContentType::try_from("text/plain; charset=UTF-8").unwrap();

        assert_eq!(upper, lower);
        assert_eq!(upper.as_str(), "text/plain; charset=UTF-8");

        let mut h_u = DefaultHasher::new();
        upper.hash(&mut h_u);
        let mut h_l = DefaultHasher::new();
        lower.hash(&mut h_l);
        assert_eq!(h_u.finish(), h_l.finish());

        // Parameter values are preserved as-is (case-sensitive per RFC 2046
        // §5.1.1 for `boundary`).
        let preserved = ContentType::try_from("multipart/mixed; BOUNDARY=\"AbC\"").unwrap();
        assert_eq!(preserved.as_str(), "multipart/mixed; boundary=\"AbC\"");
    }

    #[test]
    fn content_disposition_eq_is_case_insensitive_after_normalize() {
        use super::ContentDisposition;
        let upper = ContentDisposition::try_from("ATTACHMENT; FILENAME=\"x.pdf\"").unwrap();
        let lower = ContentDisposition::try_from("attachment; filename=\"x.pdf\"").unwrap();

        assert_eq!(upper, lower);
        assert_eq!(upper.as_str(), "attachment; filename=\"x.pdf\"");
    }
}
