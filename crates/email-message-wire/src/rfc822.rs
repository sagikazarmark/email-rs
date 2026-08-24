mod attachment;
mod content_type;
mod encoded_word;
mod header;
mod mime_parse;
mod mime_render;
mod shared;
mod transfer_encoding;

use std::str::FromStr;

use email_message::{
    Address, AddressList, ContentTransferEncoding, Header, Mailbox, Message, MessageId,
    MessageValidationError,
};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc2822;

pub use encoded_word::decode_rfc2047_phrase;
use encoded_word::{
    decode_rfc2047_words, encode_rfc2047_unstructured, escape_encoded_words_inside_quoted_strings,
};
use header::{
    is_structured_header, parse_header_lines_bytes, push_header_line, render_address_list_header,
    render_mailbox_header, split_headers_and_body_bytes,
};
use mime_render::build_render_payload;
pub use shared::{MAX_INPUT_BYTES, MAX_MULTIPART_DEPTH, MAX_MULTIPART_PARTS};

/// Errors returned while parsing RFC 822/MIME bytes.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MessageParseError {
    /// A header line is not valid UTF-8.
    #[error("input is not valid UTF-8")]
    InvalidUtf8,
    /// A header line violates the supported RFC 5322 syntax or byte rules.
    #[error("invalid header line `{line}`")]
    #[non_exhaustive]
    InvalidHeaderLine {
        /// Rejected line or validation details.
        line: String,
    },
    /// A single-mailbox header could not be parsed.
    #[error("failed to parse mailbox from `{header}` header")]
    #[non_exhaustive]
    MailboxHeaderParse {
        /// Header field containing the invalid mailbox.
        header: &'static str,
    },
    /// An address-list header could not be parsed.
    #[error("failed to parse address list from `{header}` header")]
    #[non_exhaustive]
    AddressHeaderParse {
        /// Header field containing the invalid address list.
        header: &'static str,
    },
    /// The `Date` header is not a valid RFC 2822 date-time.
    #[error("failed to parse Date header as RFC 2822 datetime")]
    #[non_exhaustive]
    Date {
        /// Underlying date-time parse error.
        #[source]
        source: time::error::Parse,
    },
    /// The `Message-ID` header is invalid.
    #[error("failed to parse Message-ID header")]
    #[non_exhaustive]
    MessageId {
        /// Underlying message-id parse error.
        #[source]
        source: email_message::MessageIdParseError,
    },
    /// MIME structure, metadata, or transfer-encoded content is invalid.
    #[error("failed to parse MIME body: {details}")]
    #[non_exhaustive]
    MimeBodyParse {
        /// Description of the invalid MIME input.
        details: String,
    },
}

impl PartialEq for MessageParseError {
    /// Pragmatic equality: variants compare by tag, ignoring the
    /// boxed `source` chains on `Date` and `MessageId`. Sufficient
    /// for tests and avoids forcing `Eq` on third-party error types.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::InvalidUtf8, Self::InvalidUtf8)
            | (Self::Date { .. }, Self::Date { .. })
            | (Self::MessageId { .. }, Self::MessageId { .. }) => true,
            (Self::InvalidHeaderLine { line: a }, Self::InvalidHeaderLine { line: b })
            | (Self::MimeBodyParse { details: a }, Self::MimeBodyParse { details: b }) => a == b,
            (Self::MailboxHeaderParse { header: a }, Self::MailboxHeaderParse { header: b })
            | (Self::AddressHeaderParse { header: a }, Self::AddressHeaderParse { header: b }) => {
                a == b
            }
            _ => false,
        }
    }
}

impl Eq for MessageParseError {}

/// Errors returned while rendering an RFC 822/MIME message.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MessageRenderError {
    /// A header value contains a raw carriage return or line feed.
    #[error("header `{name}` contains raw newline characters")]
    #[non_exhaustive]
    HeaderContainsRawNewline {
        /// Invalid header name.
        name: String,
    },
    /// A header value contains a forbidden control character.
    #[error("header `{name}` contains invalid control characters")]
    #[non_exhaustive]
    HeaderContainsControlCharacter {
        /// Invalid header name.
        name: String,
    },
    /// A header remains non-ASCII after applicable encoding.
    #[error("header `{name}` contains non-ASCII characters")]
    #[non_exhaustive]
    HeaderContainsNonAscii {
        /// Invalid header name.
        name: String,
    },
    /// A header name violates RFC 5322 field-name syntax.
    #[error("header name `{name}` is invalid")]
    #[non_exhaustive]
    InvalidHeaderName {
        /// Invalid header name.
        name: String,
    },
    /// A header cannot be folded below the RFC 5322 hard line limit.
    #[error("header `{name}` exceeds RFC 5322 hard line length limit")]
    #[non_exhaustive]
    HeaderLineTooLong {
        /// Overlong header name.
        name: String,
    },
    /// The message date could not be formatted as RFC 2822.
    #[error("failed to format Date header as RFC 2822 datetime")]
    DateFormat,
    /// A MIME boundary is empty.
    #[error("MIME boundary cannot be empty")]
    EmptyMimeBoundary,
    /// A MIME boundary contains bytes forbidden by the supported grammar.
    #[error("MIME boundary contains forbidden characters")]
    InvalidMimeBoundary,
    /// The content-type boundary parameter differs from the part boundary.
    #[error("multipart boundary parameter does not match part boundary")]
    MismatchedMimeBoundary,
    /// A multipart node contains no child parts.
    #[error("multipart parts cannot be empty")]
    EmptyMultipartParts,
    /// A MIME tree exceeds [`MAX_MULTIPART_DEPTH`].
    #[error("multipart nesting exceeds maximum depth of {MAX_MULTIPART_DEPTH}")]
    MimeNestingTooDeep,
    /// A multipart node's content type is not `multipart/*`.
    #[error("multipart part must use a multipart content type")]
    InvalidMultipartContentType,
    /// An unresolved attachment reference cannot be rendered.
    #[error("attachment body variant is not supported")]
    UnsupportedAttachmentBody,
    /// An attachment content id is not a valid message-id value.
    #[error("attachment content-id is invalid")]
    InvalidContentId,
    /// The message contains a body variant unsupported by this renderer.
    #[error("message body variant is not supported")]
    UnsupportedBody,
    /// The message failed baseline outbound validation.
    #[error(transparent)]
    MessageValidation(#[from] MessageValidationError),
}

/// Render-time options for [`render_rfc822_with`].
///
/// The struct is `#[non_exhaustive]`; future fields will be additive.
/// Construct via [`Self::new`] or [`Self::default`] and chain
/// `with_*` setters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RenderOptions {
    /// When `true`, the rendered message includes a `Bcc:` header line
    /// listing the message's BCC recipients. Defaults to `false`.
    ///
    /// Most SMTP relays strip `Bcc:` on submission anyway; rendering
    /// the field is occasionally useful for archival, `.eml` fixtures,
    /// or clients that consume the rendered bytes outside the SMTP
    /// path.
    pub include_bcc: bool,
    /// Optional soft-fold target for header lines, in characters.
    ///
    /// `None` (the default) emits header lines at the RFC 5322 §2.1.1
    /// hard limit of 998 characters with no soft folding, long values
    /// flow on a single physical line. `Some(n)` instructs the renderer
    /// to fold longer lines at `n` characters via the standard
    /// folding-whitespace mechanism (CRLF + leading SP/HTAB), targeting
    /// the SHOULD ≤ 78 recommendation when `n == 78`.
    ///
    /// The default is `None` because correct soft folding requires
    /// per-header-grammar awareness (encoded-word boundaries,
    /// address-list comma discipline, structured-header whitespace
    /// rules) that the simple folding helper cannot guarantee in every
    /// case. Callers who want SHOULD-compliant output for archival or
    /// for strict legacy MTAs can opt in via `with_soft_fold(78)`; the
    /// renderer still respects the 998 hard limit regardless.
    pub soft_fold_at: Option<usize>,
}

impl RenderOptions {
    /// Creates options with Bcc output and soft folding disabled.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            include_bcc: false,
            soft_fold_at: None,
        }
    }

    /// Chooses whether to emit the `Bcc` header.
    #[must_use]
    pub const fn with_include_bcc(mut self, value: bool) -> Self {
        self.include_bcc = value;
        self
    }

    /// Set the soft-fold target. Pass `78` for the RFC 5322 §2.1.1
    /// SHOULD-compliant recommendation; pass any other positive integer
    /// up to `997` for a custom target.
    #[must_use]
    pub const fn with_soft_fold(mut self, soft_fold_at: usize) -> Self {
        self.soft_fold_at = Some(soft_fold_at);
        self
    }

    /// Disable soft folding. Long header values flow on one physical
    /// line up to the 998-character hard limit.
    #[must_use]
    pub const fn without_soft_fold(mut self) -> Self {
        self.soft_fold_at = None;
        self
    }
}

/// Parse RFC822/MIME bytes into a structured [`Message`].
///
/// # Decoding behavior
///
/// - **Body charset.** Bodies declared `utf-8`, `us-ascii`, `iso-8859-1`,
///   or `latin1` are decoded faithfully. Bodies in other charsets, or
///   bodies declared `utf-8` with invalid UTF-8 byte sequences, are
///   passed through `String::from_utf8_lossy`, invalid bytes become
///   `U+FFFD`. The parser does not error on undecodable bytes; users
///   needing strict decode semantics should pre-validate.
/// - **Encoded words.** RFC 2047 encoded words (`=?charset?Q?…?=` /
///   `=?charset?B?…?=`) are decoded for the same charset allowlist.
///   Encoded words in other charsets (e.g. `windows-1252`, `gbk`,
///   `shift_jis`) pass through as the raw `=?…?=` literal.
/// - **Duplicate headers.** Multiple `To:`, `Cc:`, `Bcc:`, or `Reply-To:`
///   header lines are merged into a single recipient list. RFC 5322 §3.6
///   forbids duplicates, but real MTAs occasionally emit them; the
///   parser is liberal in what it accepts. Outbound rendering emits one
///   line per category.
/// - **RFC 6532 (SMTPUTF8).** Header *lines* must be ASCII-only. Senders
///   that put UTF-8 directly in header bodies (without RFC 2047 encoding)
///   are rejected with [`MessageParseError::InvalidHeaderLine`]. Most
///   senders RFC 2047-encode for compat; this rarely surfaces.
///
/// # Returned message
///
/// The returned [`Message`] has not been promoted through outbound
/// validation. Wrapping it via [`email_message::OutboundMessage::new`]
/// may reject inbound-shaped messages that lack a `From:` header or
/// have no recipients, both legitimate states for an inbound parse.
///
/// # Round-trip caveats
///
/// `parse_rfc822` is a typed-model deserializer, not a byte-faithful
/// re-emitter. A `parse → render_rfc822` round-trip is **not** guaranteed
/// to produce identical bytes:
///
/// - **Header order.** Headers are emitted in a fixed canonical order
///   (`From`, `Sender`, `To`, `Cc`, `Bcc`, `Reply-To`, `Subject`, `Date`,
///   `Message-ID`, generic headers, MIME headers). Trace metadata such
///   as `Received:` is preserved as a generic header but appears below
///   the typed fields rather than at its original parse position.
/// - **Generic-header decoding asymmetry.** RFC 2047 encoded-words are
///   decoded for `Subject` and the address headers (`From`, `Sender`,
///   `To`, `Cc`, `Bcc`, `Reply-To`). For arbitrary other headers, values
///   are preserved literally, a header value emitted as
///   `X-Note: =?utf-8?B?w6Fy?=` round-trips as the literal bytes
///   `=?utf-8?B?w6Fy?=`, *not* the decoded text `ár`. Auto-decoding
///   every unstructured header would be a security regression because
///   opaque-bytes headers (`X-Auth-Token`, `DKIM-Signature`,
///   `Authentication-Results`, `ARC-*`) carry data that must not be
///   silently rewritten. Callers who *know* a header is unstructured-text
///   shaped can opt into decoding via [`decode_rfc2047_phrase`].
///
/// # Resource bounds
///
/// The parser is best-effort and bounded against adversarial input:
///
/// - **Input length.** Inputs larger than [`MAX_INPUT_BYTES`] (16 MiB)
///   are rejected outright with [`MessageParseError::MimeBodyParse`].
/// - **Multipart depth.** Nested `multipart/*` parts are limited to
///   [`MAX_MULTIPART_DEPTH`] (100 levels). Deeper inputs would otherwise
///   stack-overflow on the mutual recursion between the multipart body
///   parser and the part parser.
/// - **Multipart fan-out.** A single multipart body cannot contain more
///   than [`MAX_MULTIPART_PARTS`] (1024) sibling parts.
///
/// These caps cover the recursive *parser* surface. The renderer
/// (`render_rfc822` and `render_rfc822_with`) enforces the symmetric
/// [`MAX_MULTIPART_DEPTH`] cap on outbound trees, including up to two
/// frames of attachment-wrapping added by the renderer itself when
/// inline and/or regular attachments are present (one
/// `multipart/related` frame for inline parts, one `multipart/mixed`
/// frame for regular parts). It returns
/// [`MessageRenderError::MimeNestingTooDeep`] when a `Body::Mime` value
/// plus those wrap frames exceeds the cap. A `Body::Mime` value at
/// exactly [`MAX_MULTIPART_DEPTH`] therefore renders cleanly when no
/// attachments are present but errors when wrapped.
///
/// The kernel does **not** depth-cap `serde::Deserialize<Body>` /
/// `Deserialize<MimePart>` because the recursive
/// `MimePart::Multipart { parts: Vec<Self> }` shape is the data model,
/// not a parser artifact. Callers who deserialize untrusted JSON into
/// [`email_message::Body`] are responsible for pre-bounding the input
/// themselves (e.g. via `serde_json::de::Deserializer::disable_recursion_limit`
/// left at its 128-level default, or a separate length cap). The render
/// path enforces its own cap regardless, so an unbounded deserialize
/// followed by `render_rfc822` errors cleanly rather than overflowing
/// the stack.
///
/// # Errors
///
/// Returns [`MessageParseError`] when headers, mailbox fields, dates,
/// message ids, MIME metadata, or transfer-encoded bodies are malformed.
#[allow(clippy::too_many_lines)]
pub fn parse_rfc822(input: &[u8]) -> Result<Message, MessageParseError> {
    if input.len() > MAX_INPUT_BYTES {
        return Err(MessageParseError::MimeBodyParse {
            details: format!(
                "input is {} bytes, exceeding maximum of {MAX_INPUT_BYTES}",
                input.len()
            ),
        });
    }

    let (raw_headers, raw_body) = split_headers_and_body_bytes(input);
    let parsed_headers = parse_header_lines_bytes(raw_headers)?;

    let mut from: Option<Mailbox> = None;
    let mut sender: Option<Mailbox> = None;
    let mut to: Vec<Address> = Vec::new();
    let mut cc: Vec<Address> = Vec::new();
    let mut bcc: Vec<Address> = Vec::new();
    let mut reply_to: Vec<Address> = Vec::new();
    let mut subject: Option<String> = None;
    let mut date: Option<OffsetDateTime> = None;
    let mut message_id: Option<MessageId> = None;
    let mut root_content_type: Option<String> = None;
    let mut root_content_transfer_encoding: Option<ContentTransferEncoding> = None;
    let mut headers = Vec::new();

    for (header_name, header_value) in parsed_headers {
        let header_name_ref = header_name.as_str();
        let header_value_ref = header_value.as_str();
        let decoded_header_value = decode_rfc2047_words(header_value_ref);

        // Address-typed headers route the *raw* header value to the
        // address parser, after escaping encoded-words inside any
        // quoted-string regions (see
        // `escape_encoded_words_inside_quoted_strings`). The kernel's
        // own `decode_rfc2047_words` pass would unconditionally decode
        // them and the upstream `mail_parser` does the same; the
        // pre-escape is the only place where the RFC 2047 §5(3) rule
        // is enforced.
        let address_value = escape_encoded_words_inside_quoted_strings(header_value_ref);
        if header_name_ref.eq_ignore_ascii_case("from") {
            from = Some(
                address_value
                    .parse::<Mailbox>()
                    .map_err(|_| MessageParseError::MailboxHeaderParse { header: "From" })?,
            );
            continue;
        }

        if header_name_ref.eq_ignore_ascii_case("sender") {
            sender = Some(
                address_value
                    .parse::<Mailbox>()
                    .map_err(|_| MessageParseError::MailboxHeaderParse { header: "Sender" })?,
            );
            continue;
        }

        if header_name_ref.eq_ignore_ascii_case("to") {
            let mut parsed = AddressList::from_str(&address_value)
                .map_err(|_| MessageParseError::AddressHeaderParse { header: "To" })?
                .into_vec();
            to.append(&mut parsed);
            continue;
        }

        if header_name_ref.eq_ignore_ascii_case("cc") {
            let mut parsed = AddressList::from_str(&address_value)
                .map_err(|_| MessageParseError::AddressHeaderParse { header: "Cc" })?
                .into_vec();
            cc.append(&mut parsed);
            continue;
        }

        if header_name_ref.eq_ignore_ascii_case("bcc") {
            let mut parsed = AddressList::from_str(&address_value)
                .map_err(|_| MessageParseError::AddressHeaderParse { header: "Bcc" })?
                .into_vec();
            bcc.append(&mut parsed);
            continue;
        }

        if header_name_ref.eq_ignore_ascii_case("reply-to") {
            let mut parsed = AddressList::from_str(&address_value)
                .map_err(|_| MessageParseError::AddressHeaderParse { header: "Reply-To" })?
                .into_vec();
            reply_to.append(&mut parsed);
            continue;
        }

        if header_name_ref.eq_ignore_ascii_case("subject") {
            subject = Some(decoded_header_value.into_owned());
            continue;
        }

        if header_name_ref.eq_ignore_ascii_case("date") {
            date = Some(
                OffsetDateTime::parse(header_value_ref.trim(), &Rfc2822)
                    .map_err(|source| MessageParseError::Date { source })?,
            );
            continue;
        }

        if header_name_ref.eq_ignore_ascii_case("message-id") {
            message_id = Some(
                MessageId::try_from(header_value_ref.trim())
                    .map_err(|source| MessageParseError::MessageId { source })?,
            );
            continue;
        }

        if header_name_ref.eq_ignore_ascii_case("content-type") {
            root_content_type = Some(header_value);
            continue;
        }

        if header_name_ref.eq_ignore_ascii_case("content-transfer-encoding") {
            root_content_transfer_encoding = Some(
                ContentTransferEncoding::from_str(header_value_ref).map_err(|_| {
                    MessageParseError::MimeBodyParse {
                        details: format!(
                            "invalid top-level content-transfer-encoding `{header_value_ref}`"
                        ),
                    }
                })?,
            );
            continue;
        }

        headers.push(Header::new(header_name, header_value).map_err(|error| {
            MessageParseError::InvalidHeaderLine {
                line: error.to_string(),
            }
        })?);
    }

    let body = mime_parse::parse_body(
        raw_body,
        root_content_type.as_deref(),
        root_content_transfer_encoding,
    )?;

    let mut builder = Message::builder(body)
        .to(to)
        .cc(cc)
        .bcc(bcc)
        .reply_to(reply_to)
        .headers(headers)
        .attachments(Vec::new());

    if let Some(from) = from {
        builder = builder.from_mailbox(from);
    }

    if let Some(sender) = sender {
        builder = builder.sender(sender);
    }

    if let Some(subject) = subject {
        builder = builder.subject(subject);
    }

    if let Some(date) = date {
        builder = builder.date(date);
    }

    if let Some(message_id) = message_id {
        builder = builder.message_id(message_id);
    }

    Ok(builder.build_unchecked())
}

/// Render a structured [`Message`] as RFC822/MIME bytes.
///
/// # Encoding choices
///
/// Non-ASCII [`Body::Text`](email_message::Body) and `Body::Html` values are
/// always rendered with `Content-Transfer-Encoding: base64`. ASCII text bodies
/// whose physical lines would exceed RFC 5322's 998-octet hard limit are
/// rendered with `Content-Transfer-Encoding: quoted-printable`. A message
/// parsed from quoted-printable bytes through [`parse_rfc822`] and rendered
/// back through this function will therefore round-trip with a different
/// `Content-Transfer-Encoding`. Callers that need quoted-printable for
/// near-ASCII bodies can construct a [`MimePart::Leaf`](email_message::MimePart)
/// with an explicit `content_transfer_encoding` and use
/// [`Body::Mime`](email_message::Body).
///
/// # Errors
///
/// Returns [`MessageRenderError`] when headers or MIME parts cannot be rendered
/// according to this crate's RFC822 constraints.
pub fn render_rfc822(message: &Message) -> Result<Vec<u8>, MessageRenderError> {
    render_rfc822_with(message, &RenderOptions::default())
}

/// Render a structured [`Message`] as RFC822/MIME bytes with custom options.
///
/// See [`render_rfc822`] for the encoding-choice notes; the same trade-offs
/// apply.
///
/// # Errors
///
/// Returns [`MessageRenderError`] when headers or MIME parts cannot be rendered
/// according to this crate's RFC822 constraints.
#[allow(clippy::too_many_lines)]
pub fn render_rfc822_with(
    message: &Message,
    options: &RenderOptions,
) -> Result<Vec<u8>, MessageRenderError> {
    message.validate_basic()?;

    let mut out = Vec::new();

    if let Some(from) = message.from_mailbox() {
        push_header_line(
            &mut out,
            "From",
            &render_mailbox_header(from),
            options.soft_fold_at,
        )?;
    }

    if let Some(sender) = message.sender() {
        push_header_line(
            &mut out,
            "Sender",
            &render_mailbox_header(sender),
            options.soft_fold_at,
        )?;
    }

    if !message.to().is_empty() {
        push_header_line(
            &mut out,
            "To",
            &render_address_list_header(message.to()),
            options.soft_fold_at,
        )?;
    }

    if !message.cc().is_empty() {
        push_header_line(
            &mut out,
            "Cc",
            &render_address_list_header(message.cc()),
            options.soft_fold_at,
        )?;
    }

    if options.include_bcc && !message.bcc().is_empty() {
        push_header_line(
            &mut out,
            "Bcc",
            &render_address_list_header(message.bcc()),
            options.soft_fold_at,
        )?;
    }

    if !message.reply_to().is_empty() {
        push_header_line(
            &mut out,
            "Reply-To",
            &render_address_list_header(message.reply_to()),
            options.soft_fold_at,
        )?;
    }

    if let Some(subject) = message.subject() {
        push_header_line(
            &mut out,
            "Subject",
            &encode_rfc2047_unstructured(subject),
            options.soft_fold_at,
        )?;
    }

    if let Some(date) = message.date() {
        let formatted = date
            .format(&Rfc2822)
            .map_err(|_| MessageRenderError::DateFormat)?;
        push_header_line(&mut out, "Date", &formatted, options.soft_fold_at)?;
    }

    if let Some(message_id) = message.message_id() {
        push_header_line(
            &mut out,
            "Message-ID",
            message_id.as_str(),
            options.soft_fold_at,
        )?;
    }

    let (mime_headers, body_out, is_mime) = build_render_payload(message, options.soft_fold_at)?;

    for header in message.headers() {
        if is_mime
            && (header.name().eq_ignore_ascii_case("content-type")
                || header
                    .name()
                    .eq_ignore_ascii_case("content-transfer-encoding")
                || header.name().eq_ignore_ascii_case("mime-version"))
        {
            continue;
        }
        // RFC 2047 only applies to *unstructured* fields. Structured
        // headers (Message-ID, In-Reply-To, References, List-*, Received,
        // and the standard structured fields) carry their own grammar and
        // would be corrupted by encoded-word substitution. Generic
        // headers default to unstructured; a small allowlist below
        // bypasses the encoder for the structured ones.
        let value_owned;
        let value: &str = if header.value().is_ascii() || is_structured_header(header.name()) {
            header.value()
        } else {
            value_owned = encode_rfc2047_unstructured(header.value());
            &value_owned
        };
        push_header_line(&mut out, header.name(), value, options.soft_fold_at)?;
    }

    if is_mime {
        push_header_line(&mut out, "MIME-Version", "1.0", options.soft_fold_at)?;
        for (name, value) in mime_headers {
            push_header_line(&mut out, &name, &value, options.soft_fold_at)?;
        }
    }

    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(&body_out);

    Ok(out)
}

#[cfg(test)]
mod tests {
    use email_message::{Body, Message, MessageId};
    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc2822;

    use super::{parse_rfc822, render_rfc822};

    #[test]
    fn parse_rfc822_extracts_core_headers_and_body() {
        let input = concat!(
            "From: Mary Smith <mary@x.test>\r\n",
            "To: jdoe@one.test\r\n",
            "Subject: Test\r\n",
            "Date: Fri, 06 Mar 2026 12:00:00 +0000\r\n",
            "Message-ID: <test@example.com>\r\n",
            "X-Custom: demo\r\n",
            "\r\n",
            "hello"
        );

        let message = parse_rfc822(input.as_bytes()).expect("message should parse");
        assert_eq!(message.subject(), Some("Test"));
        assert_eq!(message.to().len(), 1);
        assert_eq!(
            message.date(),
            Some(
                &OffsetDateTime::parse("Fri, 06 Mar 2026 12:00:00 +0000", &Rfc2822)
                    .expect("date should parse")
            )
        );
        assert_eq!(
            message.message_id(),
            Some(
                &"<test@example.com>"
                    .parse::<MessageId>()
                    .expect("message id should parse")
            )
        );
        assert_eq!(message.body(), &Body::Text("hello".to_owned()));
    }

    #[test]
    fn render_rfc822_writes_expected_lines() {
        let message = Message::builder(Body::Text("hello".to_owned()))
            .from_mailbox("Mary Smith <mary@x.test>".parse().expect("valid mailbox"))
            .to(vec![email_message::Address::Mailbox(
                "jdoe@one.test".parse().expect("valid mailbox"),
            )])
            .subject("Test")
            .build()
            .expect("message should validate");

        let rendered = render_rfc822(&message).expect("render should succeed");
        let text = String::from_utf8(rendered).expect("rendered text should be utf8");

        assert!(text.contains("From: \"Mary Smith\" <mary@x.test>\r\n"));
        assert!(text.contains("To: jdoe@one.test\r\n"));
        assert!(text.contains("Subject: Test\r\n"));
        assert!(text.ends_with("\r\n\r\nhello"));
    }
}
