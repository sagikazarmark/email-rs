mod attachment;
mod encoded_word;
mod header;
mod shared;
mod transfer_encoding;

use std::str::FromStr;

use email_message::{
    Address, AddressList, Body, ContentDisposition, ContentTransferEncoding, ContentType, Header,
    Mailbox, Message, MessageId, MessageValidationError, MimePart,
};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc2822;

use attachment::{attachment_to_mime_part, partition_attachments};
pub use encoded_word::decode_rfc2047_phrase;
use encoded_word::{
    decode_rfc2047_words, encode_rfc2047_unstructured, escape_encoded_words_inside_quoted_strings,
};
use header::{
    is_structured_header, parse_header_lines_bytes, push_header_line, render_address_list_header,
    render_mailbox_header, split_headers_and_body_bytes,
};
use shared::RFC5322_HARD_LINE_LEN;
pub use shared::{MAX_INPUT_BYTES, MAX_MULTIPART_DEPTH, MAX_MULTIPART_PARTS};
use transfer_encoding::{
    decode_transfer_encoded_body, encode_base64, encode_body_for_transfer_encoding,
    encode_quoted_printable_body, validate_multipart_transfer_encoding,
};

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
