use std::borrow::Cow;
use std::str::FromStr;

use base64::Engine;
use email_message::{
    Address, AddressList, Attachment, AttachmentBody, Body, ContentDisposition,
    ContentTransferEncoding, ContentType, Header, Mailbox, Message, MessageId,
    MessageValidationError, MimePart,
};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc2822;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MessageParseError {
    #[error("input is not valid UTF-8")]
    InvalidUtf8,
    #[error("invalid header line `{line}`")]
    #[non_exhaustive]
    InvalidHeaderLine { line: String },
    #[error("failed to parse mailbox from `{header}` header")]
    #[non_exhaustive]
    MailboxHeaderParse { header: &'static str },
    #[error("failed to parse address list from `{header}` header")]
    #[non_exhaustive]
    AddressHeaderParse { header: &'static str },
    #[error("failed to parse Date header as RFC 2822 datetime")]
    #[non_exhaustive]
    Date {
        #[source]
        source: time::error::Parse,
    },
    #[error("failed to parse Message-ID header")]
    #[non_exhaustive]
    MessageId {
        #[source]
        source: email_message::MessageIdParseError,
    },
    #[error("failed to parse MIME body: {details}")]
    #[non_exhaustive]
    MimeBodyParse { details: String },
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

/// Maximum input byte length accepted by [`parse_rfc822`]. 16 MiB is far
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

const RFC5322_HARD_LINE_LEN: usize = 998;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MessageRenderError {
    #[error("header `{name}` contains raw newline characters")]
    #[non_exhaustive]
    HeaderContainsRawNewline { name: String },
    #[error("header `{name}` contains invalid control characters")]
    #[non_exhaustive]
    HeaderContainsControlCharacter { name: String },
    #[error("header `{name}` contains non-ASCII characters")]
    #[non_exhaustive]
    HeaderContainsNonAscii { name: String },
    #[error("header name `{name}` is invalid")]
    #[non_exhaustive]
    InvalidHeaderName { name: String },
    #[error("header `{name}` exceeds RFC 5322 hard line length limit")]
    #[non_exhaustive]
    HeaderLineTooLong { name: String },
    #[error("failed to format Date header as RFC 2822 datetime")]
    DateFormat,
    #[error("MIME boundary cannot be empty")]
    EmptyMimeBoundary,
    #[error("MIME boundary contains forbidden characters")]
    InvalidMimeBoundary,
    #[error("multipart boundary parameter does not match part boundary")]
    MismatchedMimeBoundary,
    #[error("multipart parts cannot be empty")]
    EmptyMultipartParts,
    #[error("multipart nesting exceeds maximum depth of {MAX_MULTIPART_DEPTH}")]
    MimeNestingTooDeep,
    #[error("multipart part must use a multipart content type")]
    InvalidMultipartContentType,
    #[error("attachment body variant is not supported")]
    UnsupportedAttachmentBody,
    #[error("attachment content-id is invalid")]
    InvalidContentId,
    #[error("message body variant is not supported")]
    UnsupportedBody,
    #[error(transparent)]
    MessageValidation(#[from] MessageValidationError),
}

type HeaderFields = Vec<(String, String)>;
type RenderedPart = (HeaderFields, Vec<u8>);
type RenderPayload = (HeaderFields, Vec<u8>, bool);

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
    #[must_use]
    pub const fn new() -> Self {
        Self {
            include_bcc: false,
            soft_fold_at: None,
        }
    }

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
    let mut root_content_type: Option<ContentTypeHeader> = None;
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
            root_content_type = Some(ContentTypeHeader::parse(header_value_ref));
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

    let body = if let Some(content_type) = root_content_type {
        if content_type.media_type == "text/plain" {
            let decoded_root_body = decode_transfer_encoded_body(
                raw_body,
                root_content_transfer_encoding
                    .as_ref()
                    .map(ContentTransferEncoding::as_str),
            )?;
            Body::Text(decode_text_body(
                &decoded_root_body,
                content_type.charset.as_deref(),
            ))
        } else if content_type.media_type == "text/html" {
            let decoded_root_body = decode_transfer_encoded_body(
                raw_body,
                root_content_transfer_encoding
                    .as_ref()
                    .map(ContentTransferEncoding::as_str),
            )?;
            Body::Html(decode_text_body(
                &decoded_root_body,
                content_type.charset.as_deref(),
            ))
        } else if content_type.media_type.starts_with("multipart/") {
            validate_multipart_transfer_encoding(root_content_transfer_encoding.as_ref())?;
            let boundary =
                content_type
                    .boundary
                    .ok_or_else(|| MessageParseError::MimeBodyParse {
                        details: "multipart body is missing boundary parameter".to_owned(),
                    })?;
            Body::Mime(parse_multipart_body(
                raw_body,
                &content_type.normalized,
                Some(boundary),
                0,
            )?)
        } else {
            let decoded_root_body = decode_transfer_encoded_body(
                raw_body,
                root_content_transfer_encoding
                    .as_ref()
                    .map(ContentTransferEncoding::as_str),
            )?;
            Body::Mime(MimePart::Leaf {
                content_type: ContentType::from_str(&content_type.normalized).map_err(|_| {
                    MessageParseError::MimeBodyParse {
                        details: format!("invalid content type `{}`", content_type.normalized),
                    }
                })?,
                content_transfer_encoding: root_content_transfer_encoding,
                content_disposition: None,
                body: decoded_root_body,
            })
        }
    } else {
        let decoded_root_body = decode_transfer_encoded_body(
            raw_body,
            root_content_transfer_encoding
                .as_ref()
                .map(ContentTransferEncoding::as_str),
        )?;
        Body::Text(String::from_utf8_lossy(&decoded_root_body).into_owned())
    };

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
/// with an explicit `content_transfer_encoding` and use [`Body::Mime`].
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

fn build_render_payload(
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

enum RenderPart {
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

fn encode_body_for_transfer_encoding(body: &[u8], encoding: Option<&str>) -> Vec<u8> {
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

fn partition_attachments(attachments: &[Attachment]) -> (Vec<&Attachment>, Vec<&Attachment>) {
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

fn attachment_to_mime_part(attachment: &Attachment) -> Result<RenderPart, MessageRenderError> {
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
        if let Some(star) = encoded.extended {
            disposition.push_str("; ");
            disposition.push_str(&star);
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

    Ok(RenderPart::Leaf {
        headers,
        body: encode_base64(raw),
    })
}

struct EncodedFilenameParameter {
    legacy: Option<String>,
    extended: Option<String>,
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

fn encode_base64(input: &[u8]) -> Vec<u8> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(input);
    let mut output = Vec::with_capacity(encoded.len() + (encoded.len() / 76 + 2) * 2);

    for chunk in encoded.as_bytes().chunks(76) {
        output.extend_from_slice(chunk);
        output.extend_from_slice(b"\r\n");
    }

    output
}

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
fn escape_encoded_words_inside_quoted_strings(input: &str) -> Cow<'_, str> {
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
/// [`parse_rfc822`] decodes encoded-words for `Subject` and the address
/// headers (`From`, `Sender`, `To`, `Cc`, `Bcc`, `Reply-To`) but
/// deliberately leaves arbitrary other headers untouched, because
/// silently rewriting `=?…?=`-shaped content in opaque-bytes headers
/// such as `X-Auth-Token`, `DKIM-Signature`, `Authentication-Results`,
/// or `ARC-*` would be a security regression. Callers who *know* a
/// header is unstructured-text-shaped and want round-trip semantic
/// equality across `parse → render` cycles can opt into decoding by
/// calling this function on the header value.
///
/// ```rust
/// use email_message_wire::{decode_rfc2047_phrase, parse_rfc822};
///
/// let bytes = b"From: from@example.com\r\nTo: to@example.com\r\nX-Note: =?utf-8?B?w6Fy?=\r\n\r\n";
/// let message = parse_rfc822(bytes).unwrap();
/// let header = message
///     .headers()
///     .iter()
///     .find(|h| h.name().eq_ignore_ascii_case("x-note"))
///     .unwrap();
/// assert_eq!(header.value(), "=?utf-8?B?w6Fy?=");
/// assert_eq!(decode_rfc2047_phrase(header.value()), "ár");
/// ```
#[must_use]
pub fn decode_rfc2047_phrase(input: &str) -> Cow<'_, str> {
    decode_rfc2047_words(input)
}

fn decode_rfc2047_words(input: &str) -> Cow<'_, str> {
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

const fn hex_val(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn encode_rfc2047_unstructured(input: &str) -> String {
    if input.is_ascii() {
        return input.to_owned();
    }

    encode_rfc2047_utf8_base64_words(input)
}

fn encode_rfc2047_phrase(input: &str) -> String {
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

fn render_mailbox_header(mailbox: &Mailbox) -> String {
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

fn render_address_list_header(addresses: &[Address]) -> String {
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

fn split_headers_and_body_bytes(input: &[u8]) -> (&[u8], &[u8]) {
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

fn parse_header_lines_bytes(
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

#[derive(Clone, Debug)]
struct ContentTypeHeader {
    normalized: String,
    media_type: String,
    boundary: Option<String>,
    charset: Option<String>,
}

impl ContentTypeHeader {
    fn parse(value: &str) -> Self {
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
fn is_structured_header(name: &str) -> bool {
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

fn push_header_line(
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
    let first_preferred = soft_fold_at
        .map(|target| target.saturating_sub(name_len + 2).min(first_hard))
        .unwrap_or(first_hard);
    let continuation_preferred = soft_fold_at
        .map(|target| target.saturating_sub(1).min(continuation_hard))
        .unwrap_or(continuation_hard);

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
    value
        .chars()
        .any(|ch| matches!(ch, '\u{0000}'..='\u{0008}' | '\u{000B}' | '\u{000C}' | '\u{000E}'..='\u{001F}' | '\u{007F}'))
}

fn trim_lwsp_end(value: &[u8]) -> &[u8] {
    let mut end = value.len();
    while end > 0 && (value[end - 1] == b' ' || value[end - 1] == b'\t') {
        end -= 1;
    }

    &value[..end]
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

fn decode_transfer_encoded_body(
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

fn validate_multipart_transfer_encoding(
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

fn encode_quoted_printable_body(body: &[u8]) -> Vec<u8> {
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

fn extract_boundary_param(value: &str) -> Option<String> {
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
