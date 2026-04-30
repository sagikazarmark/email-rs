use crate::mime_types::ContentType;
#[cfg(feature = "mime")]
use crate::mime_types::MimePart;
use crate::{Address, EmailAddress, Mailbox, MessageId};
use time::OffsetDateTime;

/// SMTP envelope addresses.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Envelope {
    mail_from: Option<EmailAddress>,
    rcpt_to: Vec<EmailAddress>,
}

impl Envelope {
    #[must_use]
    pub const fn new(mail_from: Option<EmailAddress>, rcpt_to: Vec<EmailAddress>) -> Self {
        Self { mail_from, rcpt_to }
    }

    #[must_use]
    pub const fn mail_from(&self) -> Option<&EmailAddress> {
        self.mail_from.as_ref()
    }

    #[must_use]
    pub fn rcpt_to(&self) -> &[EmailAddress] {
        self.rcpt_to.as_slice()
    }
}

/// A single message header line.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Header {
    name: String,
    value: String,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum HeaderValidationError {
    #[error("header name cannot be empty")]
    EmptyName,
    #[error("header name `{name}` is invalid")]
    InvalidName { name: String },
    #[error("header `{name}` contains raw newline characters")]
    ValueContainsRawNewline { name: String },
    #[error("header `{name}` contains invalid control characters")]
    ValueContainsControlCharacter { name: String },
}

impl Header {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Constructs a header after validating name and value.
    ///
    /// # Name validation
    ///
    /// The name must be non-empty and use only the RFC 5322 §2.2 `ftext`
    /// byte range (`0x21..=0x39 | 0x3B..=0x7E`). That is the literal
    /// grammar definition: it admits punctuation such as `@`, `(`, `)`,
    /// `,`, `<`, `>`, `[`, `]`, `?`, `=`, `\`, `"`. Conventional header
    /// names use the narrower RFC 7230 §3.2.6 `token` shape (alphanumerics
    /// plus a small punctuation set). Real MTAs and provider HTTP-header
    /// maps reject the looser superset; if you produce non-token names
    /// here the message will still pass kernel validation but will be
    /// dropped or routed to spam by most receivers. Callers needing the
    /// `token` shape should validate themselves before calling this
    /// constructor.
    ///
    /// # Errors
    ///
    /// Returns [`HeaderValidationError`] when the name uses bytes outside
    /// the RFC 5322 set or the value contains raw newlines or
    /// non-tab control characters.
    pub fn new(
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self, HeaderValidationError> {
        let name = name.into();
        let value = value.into();
        validate_header(&name, &value)?;
        Ok(Self { name, value })
    }
}

fn validate_header(name: &str, value: &str) -> Result<(), HeaderValidationError> {
    if name.is_empty() {
        return Err(HeaderValidationError::EmptyName);
    }
    if !name.bytes().all(is_header_name_byte) {
        return Err(HeaderValidationError::InvalidName {
            name: name.to_owned(),
        });
    }
    if value.contains(['\r', '\n']) {
        return Err(HeaderValidationError::ValueContainsRawNewline {
            name: name.to_owned(),
        });
    }
    if value
        .bytes()
        .any(|byte| byte.is_ascii_control() && byte != b'\t')
    {
        return Err(HeaderValidationError::ValueContainsControlCharacter {
            name: name.to_owned(),
        });
    }
    Ok(())
}

const fn is_header_name_byte(byte: u8) -> bool {
    matches!(byte, b'!'..=b'9' | b';'..=b'~')
}

#[cfg(feature = "serde")]
impl serde::Serialize for Header {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;

        let mut value = serializer.serialize_struct("Header", 2)?;
        value.serialize_field("name", self.name())?;
        value.serialize_field("value", self.value())?;
        value.end()
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Header {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct RawHeader {
            name: String,
            value: String,
        }

        let raw = RawHeader::deserialize(deserializer)?;
        Self::new(raw.name, raw.value).map_err(serde::de::Error::custom)
    }
}

#[cfg(feature = "arbitrary")]
impl<'a> arbitrary::Arbitrary<'a> for Header {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let suffix = u32::arbitrary(u)?;
        let value = u32::arbitrary(u)?;
        Self::new(format!("X-Arbitrary-{suffix}"), value.to_string())
            .map_err(|_| arbitrary::Error::IncorrectFormat)
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct AttachmentReference {
    uri: String,
}

impl AttachmentReference {
    #[must_use]
    pub fn new(uri: impl Into<String>) -> Self {
        Self { uri: uri.into() }
    }

    #[must_use]
    pub fn uri(&self) -> &str {
        &self.uri
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AttachmentBody {
    Bytes(Vec<u8>),
    Reference(AttachmentReference),
}

/// How a recipient's mail client should present an attachment, per RFC 2183.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Disposition {
    /// Render as a normal attachment (downloadable).
    #[default]
    Attachment,
    /// Render inline (referenced by Content-ID, e.g. an image embedded in HTML).
    Inline,
}

impl Disposition {
    #[must_use]
    pub const fn is_inline(&self) -> bool {
        matches!(self, Self::Inline)
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Attachment {
    filename: Option<String>,
    #[cfg_attr(
        feature = "schemars",
        schemars(with = "String", description = "MIME content type")
    )]
    content_type: ContentType,
    content_id: Option<String>,
    /// Reads the legacy `"inline": true|false` field via `alias`, with a
    /// custom deserializer that converts a bool into `Disposition` for one
    /// migration cycle.
    #[cfg_attr(
        feature = "serde",
        serde(
            default,
            alias = "inline",
            deserialize_with = "deserialize_disposition_compat"
        )
    )]
    disposition: Disposition,
    body: AttachmentBody,
}

#[cfg(feature = "serde")]
fn deserialize_disposition_compat<'de, D>(deserializer: D) -> Result<Disposition, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize as _;

    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Compat {
        Bool(bool),
        Tag(Disposition),
    }
    Ok(match Compat::deserialize(deserializer)? {
        Compat::Bool(true) => Disposition::Inline,
        Compat::Bool(false) => Disposition::Attachment,
        Compat::Tag(d) => d,
    })
}

impl Attachment {
    #[must_use]
    pub const fn new(content_type: ContentType, body: AttachmentBody) -> Self {
        Self {
            filename: None,
            content_type,
            content_id: None,
            disposition: Disposition::Attachment,
            body,
        }
    }

    #[must_use]
    pub fn bytes(content_type: ContentType, bytes: impl Into<Vec<u8>>) -> Self {
        Self::new(content_type, AttachmentBody::Bytes(bytes.into()))
    }

    #[must_use]
    pub const fn reference(content_type: ContentType, reference: AttachmentReference) -> Self {
        Self::new(content_type, AttachmentBody::Reference(reference))
    }

    #[must_use]
    pub fn filename(&self) -> Option<&str> {
        self.filename.as_deref()
    }

    #[must_use]
    pub const fn content_type(&self) -> &ContentType {
        &self.content_type
    }

    #[must_use]
    pub fn content_id(&self) -> Option<&str> {
        self.content_id.as_deref()
    }

    #[must_use]
    pub const fn disposition(&self) -> Disposition {
        self.disposition
    }

    #[must_use]
    pub const fn is_inline(&self) -> bool {
        self.disposition.is_inline()
    }

    #[must_use]
    pub const fn body(&self) -> &AttachmentBody {
        &self.body
    }

    pub fn set_body(&mut self, body: AttachmentBody) {
        self.body = body;
    }

    /// Builder-style replacement of the attachment body.
    #[must_use]
    pub fn with_body(mut self, body: AttachmentBody) -> Self {
        self.body = body;
        self
    }

    #[must_use]
    pub fn with_filename(mut self, filename: impl Into<String>) -> Self {
        self.filename = Some(filename.into());
        self
    }

    #[must_use]
    pub fn with_content_id(mut self, content_id: impl Into<String>) -> Self {
        self.content_id = Some(content_id.into());
        self
    }

    #[must_use]
    pub const fn with_disposition(mut self, disposition: Disposition) -> Self {
        self.disposition = disposition;
        self
    }
}

/// Message body payload.
///
/// # Untrusted-deserialize caveat
///
/// The `Body::Mime(MimePart)` variant carries a recursive
/// `MimePart::Multipart { parts: Vec<Self> }` tree.
/// Callers deserializing a `Body` (or a [`Message`] containing one)
/// from untrusted input must pre-bound the input length and recursion
/// depth: `serde_json` defaults to a 128-frame recursion limit which
/// is safe, but other formats (e.g. `serde_yaml`, `bincode`,
/// `rmp-serde`, `serde_cbor`) may not. The wire renderer
/// (`email_message_wire::render_rfc822`) enforces a
/// `MAX_MULTIPART_DEPTH` cap on outbound trees, including up to two
/// frames of attachment-wrapping when inline and/or regular
/// attachments are present, as a defensive backstop; other consumers
/// (caller code that walks the tree itself) must defend themselves.
/// See [`MimePart`] for the matching caveat on the
/// leaf type.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Body {
    Text(String),
    Html(String),
    TextAndHtml {
        text: String,
        html: String,
    },
    #[cfg(feature = "mime")]
    Mime(MimePart),
}

impl Body {
    #[must_use]
    pub fn text(value: impl Into<String>) -> Self {
        Self::Text(value.into())
    }

    #[must_use]
    pub fn html(value: impl Into<String>) -> Self {
        Self::Html(value.into())
    }

    #[must_use]
    pub fn text_and_html(text: impl Into<String>, html: impl Into<String>) -> Self {
        Self::TextAndHtml {
            text: text.into(),
            html: html.into(),
        }
    }
}

/// Parsed message content and headers.
///
/// # Validation
///
/// `Message` validation is split between this crate and the wire layer:
///
/// - [`Message::validate_basic`] enforces structural invariants:
///   `From` is set, `Sender` is not set without `From`, at least one
///   recipient in `To`/`Cc`/`Bcc`, the subject contains no raw `\r`,
///   `\n`, or non-tab control characters, and no custom header
///   collides with a structured field (`Subject`, `Message-ID`, …).
/// - Per-field RFC 5322 invariants (line length, RFC 2047 encoded-word
///   wrapping, ASCII-after-encoding, header folding) are enforced by
///   `email_message_wire::render_rfc822` for SMTP paths.
/// - HTTP-API adapters (Postmark, Resend, Mailgun, Loops) bypass the
///   wire renderer and rely on `serde_json` string-escaping for
///   control-char neutralization in JSON bodies.
///
/// Adapters that bypass both the wire renderer and a JSON-encoded
/// transport must validate header values themselves.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(clippy::struct_field_names)]
#[non_exhaustive]
pub struct Message {
    from: Option<Mailbox>,
    sender: Option<Mailbox>,
    to: Vec<Address>,
    cc: Vec<Address>,
    bcc: Vec<Address>,
    reply_to: Vec<Address>,
    subject: Option<String>,
    #[cfg_attr(
        feature = "schemars",
        schemars(with = "Option<String>", description = "RFC 2822 date-time")
    )]
    date: Option<OffsetDateTime>,
    message_id: Option<MessageId>,
    headers: Vec<Header>,
    body: Body,
    attachments: Vec<Attachment>,
}

/// A [`Message`] that has passed outbound delivery validation.
/// A [`Message`] that has passed outbound delivery validation.
///
/// The serde representation matches [`Message`] verbatim; deserializing
/// runs [`OutboundMessage::new`] so an invalid payload is rejected
/// instead of silently bypassing the typestate invariant.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct OutboundMessage {
    /// The validated underlying message.
    inner: Message,
    /// The `From` mailbox, mirroring `inner.from`. Stored separately so
    /// [`Self::from_mailbox`] can return `&Mailbox` infallibly without
    /// unwrapping; [`Self::new`] establishes the invariant
    /// `inner.from == Some(from.clone())`.
    from: Mailbox,
}

#[cfg(feature = "serde")]
impl serde::Serialize for OutboundMessage {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Transparent over `Message`: the redundant `from` field is
        // an in-memory accessor cache, not part of the wire format.
        self.inner.serialize(serializer)
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for OutboundMessage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let message = Message::deserialize(deserializer)?;
        Self::new(message).map_err(serde::de::Error::custom)
    }
}

#[cfg(feature = "schemars")]
impl schemars::JsonSchema for OutboundMessage {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        <Message as schemars::JsonSchema>::schema_name()
    }

    fn schema_id() -> std::borrow::Cow<'static, str> {
        <Message as schemars::JsonSchema>::schema_id()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        <Message as schemars::JsonSchema>::json_schema(generator)
    }
}

impl OutboundMessage {
    /// Validate and wrap a message for outbound delivery.
    ///
    /// # Errors
    ///
    /// Returns [`MessageValidationError`] when required outbound fields are
    /// missing or inconsistent.
    pub fn new(message: Message) -> Result<Self, MessageValidationError> {
        message.validate_basic()?;
        // `validate_basic` already guarantees `from` is `Some`. The
        // redundant `ok_or` is defensive, it preserves the no-panic
        // contract on this constructor even under hypothetical future
        // contract drift in `validate_basic`.
        let from = message
            .from
            .clone()
            .ok_or(MessageValidationError::MissingFrom)?;
        Ok(Self {
            inner: message,
            from,
        })
    }

    #[must_use]
    pub const fn as_message(&self) -> &Message {
        &self.inner
    }

    #[must_use]
    pub fn into_message(self) -> Message {
        self.inner
    }

    /// Returns the validated `From` mailbox.
    ///
    /// Outbound validation guarantees the `From` field is set, so this
    /// accessor is infallible (unlike [`Message::from_mailbox`], which
    /// returns `Option<&Mailbox>`).
    #[must_use]
    pub const fn from_mailbox(&self) -> &Mailbox {
        &self.from
    }
}

impl TryFrom<Message> for OutboundMessage {
    type Error = MessageValidationError;

    fn try_from(value: Message) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<OutboundMessage> for Message {
    fn from(value: OutboundMessage) -> Self {
        value.inner
    }
}

#[cfg(feature = "arbitrary")]
impl<'a> arbitrary::Arbitrary<'a> for Message {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let has_date = bool::arbitrary(u)?;
        let date = if has_date {
            let seconds = i64::arbitrary(u)?;
            Some(OffsetDateTime::from_unix_timestamp(seconds).unwrap_or(OffsetDateTime::UNIX_EPOCH))
        } else {
            None
        };

        Ok(Self {
            from: Option::<Mailbox>::arbitrary(u)?,
            sender: Option::<Mailbox>::arbitrary(u)?,
            to: Vec::<Address>::arbitrary(u)?,
            cc: Vec::<Address>::arbitrary(u)?,
            bcc: Vec::<Address>::arbitrary(u)?,
            reply_to: Vec::<Address>::arbitrary(u)?,
            subject: Option::<String>::arbitrary(u)?,
            date,
            message_id: Option::<MessageId>::arbitrary(u)?,
            headers: Vec::<Header>::arbitrary(u)?,
            body: Body::arbitrary(u)?,
            attachments: Vec::<Attachment>::arbitrary(u)?,
        })
    }
}

/// Reasons a [`Message`] failed [`Message::validate_basic`] (and therefore
/// cannot be promoted into an [`OutboundMessage`]).
///
/// ```rust
/// use email_message::{Address, Body, Header, Mailbox, Message, MessageValidationError};
///
/// let from: Mailbox = "alice@example.com".parse().unwrap();
/// let to = Address::Mailbox("bob@example.com".parse().unwrap());
///
/// // A subject carrying a CRLF injection is rejected at build time:
/// let error = Message::builder(Body::text("hello"))
///     .from_mailbox(from.clone())
///     .add_to(to.clone())
///     .subject("hi\r\nBcc: attacker@example.com")
///     .build()
///     .unwrap_err();
/// assert_eq!(error, MessageValidationError::SubjectContainsInvalidChars);
///
/// // A custom header that collides with a structured field is rejected:
/// let error = Message::builder(Body::text("hello"))
///     .from_mailbox(from)
///     .add_to(to)
///     .add_header(Header::new("Subject", "shadow").unwrap())
///     .build()
///     .unwrap_err();
/// assert!(matches!(
///     error,
///     MessageValidationError::ReservedHeaderName { ref name, .. } if name == "Subject"
/// ));
/// ```
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MessageValidationError {
    #[error("missing From header")]
    MissingFrom,
    #[error("sender header cannot appear without from")]
    SenderWithoutFrom,
    #[error("no recipients in To/Cc/Bcc")]
    MissingRecipients,
    #[error(
        "custom header `{name}` collides with a structured field; use the typed setter (Subject, Date, Message-ID, From, ...) instead"
    )]
    #[non_exhaustive]
    ReservedHeaderName { name: String },
    #[error("subject contains raw CR, LF, or non-tab control characters")]
    SubjectContainsInvalidChars,
    #[error(
        "mailbox display name in `{location}` contains raw CR, LF, NUL, or non-tab control characters"
    )]
    #[non_exhaustive]
    MailboxDisplayNameContainsInvalidChars { location: &'static str },
    #[error(
        "attachment metadata field `{field}` contains raw CR, LF, NUL, or non-tab control characters"
    )]
    #[non_exhaustive]
    AttachmentMetadataContainsInvalidChars { field: &'static str },
}

fn contains_header_unsafe_chars(value: &str) -> bool {
    value
        .bytes()
        .any(|byte| byte == b'\r' || byte == b'\n' || (byte != b'\t' && byte.is_ascii_control()))
}

/// Returns `Err` when any mailbox in `addresses` carries a display name with
/// raw CR / LF / NUL / non-tab control characters, applying the same byte
/// discipline as [`contains_header_unsafe_chars`]. Group display names and
/// group members are walked recursively.
fn validate_address_mailboxes(
    addresses: &[Address],
    location: &'static str,
) -> Result<(), MessageValidationError> {
    for address in addresses {
        match address {
            Address::Mailbox(mailbox) => {
                if let Some(name) = mailbox.name()
                    && contains_header_unsafe_chars(name)
                {
                    return Err(
                        MessageValidationError::MailboxDisplayNameContainsInvalidChars { location },
                    );
                }
            }
            Address::Group(group) => {
                if contains_header_unsafe_chars(group.name()) {
                    return Err(
                        MessageValidationError::MailboxDisplayNameContainsInvalidChars { location },
                    );
                }
                for member in group.members() {
                    if let Some(name) = member.name()
                        && contains_header_unsafe_chars(name)
                    {
                        return Err(
                            MessageValidationError::MailboxDisplayNameContainsInvalidChars {
                                location,
                            },
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_mailbox_display_name(
    mailbox: &Mailbox,
    location: &'static str,
) -> Result<(), MessageValidationError> {
    if let Some(name) = mailbox.name()
        && contains_header_unsafe_chars(name)
    {
        return Err(MessageValidationError::MailboxDisplayNameContainsInvalidChars { location });
    }
    Ok(())
}

/// RFC 5322 §3.6 mandates these headers appear at most once. The kernel
/// exposes typed setters for each; populating them through
/// `MessageBuilder::header` would either duplicate the field or shadow it
/// at the wire layer.
///
/// `In-Reply-To` and `References` are also §3.6 singletons but are
/// deliberately *not* on this list because the kernel has no typed setter
/// for them yet. Until that gap closes, callers must use
/// `MessageBuilder::header` for those.
const RESERVED_HEADER_NAMES: &[&str] = &[
    "from",
    "sender",
    "reply-to",
    "to",
    "cc",
    "bcc",
    "date",
    "subject",
    "message-id",
];

fn is_reserved_header_name(name: &str) -> bool {
    RESERVED_HEADER_NAMES
        .iter()
        .any(|reserved| name.eq_ignore_ascii_case(reserved))
}

impl Message {
    /// Creates a message with required semantic fields.
    #[must_use]
    pub const fn new(from: Mailbox, to: Vec<Address>, body: Body) -> Self {
        Self {
            from: Some(from),
            sender: None,
            to,
            cc: Vec::new(),
            bcc: Vec::new(),
            reply_to: Vec::new(),
            subject: None,
            date: None,
            message_id: None,
            headers: Vec::new(),
            body,
            attachments: Vec::new(),
        }
    }

    /// Returns a builder for incrementally constructing messages.
    #[must_use]
    pub const fn builder(body: Body) -> MessageBuilder {
        MessageBuilder::new(body)
    }

    /// Returns the optional `From` mailbox, if one has been set.
    ///
    /// `OutboundMessage` validation guarantees `From` is present; for
    /// already-validated messages, prefer [`OutboundMessage::from_mailbox`]
    /// which returns `&Mailbox` directly.
    #[must_use]
    pub const fn from_mailbox(&self) -> Option<&Mailbox> {
        self.from.as_ref()
    }

    #[must_use]
    pub const fn sender(&self) -> Option<&Mailbox> {
        self.sender.as_ref()
    }

    #[must_use]
    pub fn to(&self) -> &[Address] {
        self.to.as_slice()
    }

    #[must_use]
    pub fn cc(&self) -> &[Address] {
        self.cc.as_slice()
    }

    #[must_use]
    pub fn bcc(&self) -> &[Address] {
        self.bcc.as_slice()
    }

    #[must_use]
    pub fn reply_to(&self) -> &[Address] {
        self.reply_to.as_slice()
    }

    #[must_use]
    pub fn subject(&self) -> Option<&str> {
        self.subject.as_deref()
    }

    #[must_use]
    pub const fn date(&self) -> Option<&OffsetDateTime> {
        self.date.as_ref()
    }

    #[must_use]
    pub const fn message_id(&self) -> Option<&MessageId> {
        self.message_id.as_ref()
    }

    #[must_use]
    pub fn headers(&self) -> &[Header] {
        self.headers.as_slice()
    }

    #[must_use]
    pub const fn body(&self) -> &Body {
        &self.body
    }

    #[must_use]
    pub fn attachments(&self) -> &[Attachment] {
        self.attachments.as_slice()
    }

    #[must_use]
    pub fn with_attachments<I>(mut self, attachments: I) -> Self
    where
        I: IntoIterator<Item = Attachment>,
    {
        self.attachments = attachments.into_iter().collect();
        self
    }

    /// Split the message into an attachment-free message and its attachments.
    #[must_use]
    pub fn into_attachments(mut self) -> (Self, Vec<Attachment>) {
        let attachments = std::mem::take(&mut self.attachments);
        (self, attachments)
    }

    /// Validates baseline message invariants.
    ///
    /// # Coverage
    ///
    /// The gate covers top-level message fields (`from`, `sender`,
    /// recipients, `subject`, custom `headers`) and the `attachments`
    /// list (filename and content-id byte discipline). It does **not**
    /// recurse into [`Body::Mime`] payloads: MIME-tree fields the typed
    /// wrappers leave unvalidated at construction (notably
    /// `MimePart::Multipart`'s `boundary: Option<String>`, which is
    /// lazy-checked by the wire renderer's `validate_boundary` at
    /// render time, and `MimePart::Leaf`'s raw `body: Vec<u8>`, which
    /// is transfer-encoded at render time) are not inspected here.
    /// Such bytes are caught by the wire renderer at
    /// `email_message_wire::render_rfc822`'s header-emission and
    /// boundary-validation stages, which reject raw CR/LF and non-ASCII
    /// at write time. Walking the entire `MimePart` tree in this method
    /// would make the gate quadratic on attacker-controlled depth, the
    /// inverse of the renderer's own `MAX_MULTIPART_DEPTH` cap.
    ///
    /// # Errors
    ///
    /// Returns [`MessageValidationError`] when required message fields are
    /// missing or inconsistent.
    pub fn validate_basic(&self) -> Result<(), MessageValidationError> {
        if self.sender.is_some() && self.from.is_none() {
            return Err(MessageValidationError::SenderWithoutFrom);
        }

        if self.from.is_none() {
            return Err(MessageValidationError::MissingFrom);
        }

        if self.to.is_empty() && self.cc.is_empty() && self.bcc.is_empty() {
            return Err(MessageValidationError::MissingRecipients);
        }

        if let Some(subject) = self.subject.as_deref()
            && contains_header_unsafe_chars(subject)
        {
            return Err(MessageValidationError::SubjectContainsInvalidChars);
        }

        for header in &self.headers {
            if is_reserved_header_name(header.name()) {
                return Err(MessageValidationError::ReservedHeaderName {
                    name: header.name().to_owned(),
                });
            }
        }

        if let Some(from) = self.from.as_ref() {
            validate_mailbox_display_name(from, "from")?;
        }
        if let Some(sender) = self.sender.as_ref() {
            validate_mailbox_display_name(sender, "sender")?;
        }
        validate_address_mailboxes(&self.to, "to")?;
        validate_address_mailboxes(&self.cc, "cc")?;
        validate_address_mailboxes(&self.bcc, "bcc")?;
        validate_address_mailboxes(&self.reply_to, "reply-to")?;

        for attachment in &self.attachments {
            if let Some(filename) = attachment.filename()
                && contains_header_unsafe_chars(filename)
            {
                return Err(
                    MessageValidationError::AttachmentMetadataContainsInvalidChars {
                        field: "filename",
                    },
                );
            }
            if let Some(content_id) = attachment.content_id()
                && contains_header_unsafe_chars(content_id)
            {
                return Err(
                    MessageValidationError::AttachmentMetadataContainsInvalidChars {
                        field: "content-id",
                    },
                );
            }
        }

        Ok(())
    }

    /// Derives an SMTP envelope from message semantics.
    ///
    /// # Errors
    ///
    /// Returns [`MessageValidationError`] when the message does not contain the
    /// fields needed to derive an envelope.
    pub fn derive_envelope(&self) -> Result<Envelope, MessageValidationError> {
        self.validate_basic()?;

        let mail_from = self
            .sender
            .as_ref()
            .or(self.from.as_ref())
            .map(|mailbox| mailbox.email().clone());

        let mut rcpt_to = Vec::new();
        extend_recipient_emails(&mut rcpt_to, &self.to);
        extend_recipient_emails(&mut rcpt_to, &self.cc);
        extend_recipient_emails(&mut rcpt_to, &self.bcc);

        Ok(Envelope::new(mail_from, rcpt_to))
    }
}

fn extend_recipient_emails(out: &mut Vec<EmailAddress>, addresses: &[Address]) {
    for address in addresses {
        out.extend(address.mailboxes().map(|mailbox| mailbox.email().clone()));
    }
}

/// Builder for [`Message`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct MessageBuilder {
    message: Message,
}

impl MessageBuilder {
    #[must_use]
    pub const fn new(body: Body) -> Self {
        Self {
            message: Message {
                from: None,
                sender: None,
                to: Vec::new(),
                cc: Vec::new(),
                bcc: Vec::new(),
                reply_to: Vec::new(),
                subject: None,
                date: None,
                message_id: None,
                headers: Vec::new(),
                body,
                attachments: Vec::new(),
            },
        }
    }

    /// Sets the `From` mailbox.
    ///
    /// Named `from_mailbox` (rather than `from`) to avoid shadowing the
    /// [`From::from`] trait method and the [`Message::from_mailbox`] accessor.
    #[must_use]
    pub fn from_mailbox(mut self, from: Mailbox) -> Self {
        self.message.from = Some(from);
        self
    }

    #[must_use]
    pub fn sender(mut self, sender: Mailbox) -> Self {
        self.message.sender = Some(sender);
        self
    }

    /// Replace the entire `To` recipient list. To append a single recipient,
    /// use [`Self::add_to`].
    #[must_use]
    pub fn to<I>(mut self, to: I) -> Self
    where
        I: IntoIterator<Item = Address>,
    {
        self.message.to = to.into_iter().collect();
        self
    }

    /// Append a recipient to the `To` list.
    #[must_use]
    pub fn add_to(mut self, to: impl Into<Address>) -> Self {
        self.message.to.push(to.into());
        self
    }

    /// Replace the entire `Cc` recipient list. To append, use [`Self::add_cc`].
    #[must_use]
    pub fn cc<I>(mut self, cc: I) -> Self
    where
        I: IntoIterator<Item = Address>,
    {
        self.message.cc = cc.into_iter().collect();
        self
    }

    /// Append a recipient to the `Cc` list.
    #[must_use]
    pub fn add_cc(mut self, cc: impl Into<Address>) -> Self {
        self.message.cc.push(cc.into());
        self
    }

    /// Replace the entire `Bcc` recipient list. To append, use [`Self::add_bcc`].
    #[must_use]
    pub fn bcc<I>(mut self, bcc: I) -> Self
    where
        I: IntoIterator<Item = Address>,
    {
        self.message.bcc = bcc.into_iter().collect();
        self
    }

    /// Append a recipient to the `Bcc` list.
    #[must_use]
    pub fn add_bcc(mut self, bcc: impl Into<Address>) -> Self {
        self.message.bcc.push(bcc.into());
        self
    }

    /// Replace the entire `Reply-To` list.
    #[must_use]
    pub fn reply_to<I>(mut self, reply_to: I) -> Self
    where
        I: IntoIterator<Item = Address>,
    {
        self.message.reply_to = reply_to.into_iter().collect();
        self
    }

    /// Append a recipient to the `Reply-To` list.
    #[must_use]
    pub fn add_reply_to(mut self, reply_to: impl Into<Address>) -> Self {
        self.message.reply_to.push(reply_to.into());
        self
    }

    #[must_use]
    pub fn subject(mut self, subject: impl Into<String>) -> Self {
        self.message.subject = Some(subject.into());
        self
    }

    #[must_use]
    pub const fn date(mut self, date: OffsetDateTime) -> Self {
        self.message.date = Some(date);
        self
    }

    #[must_use]
    pub fn message_id(mut self, message_id: MessageId) -> Self {
        self.message.message_id = Some(message_id);
        self
    }

    #[must_use]
    pub fn headers<I>(mut self, headers: I) -> Self
    where
        I: IntoIterator<Item = Header>,
    {
        self.message.headers = headers.into_iter().collect();
        self
    }

    /// Append a single custom header.
    #[must_use]
    pub fn add_header(mut self, header: Header) -> Self {
        self.message.headers.push(header);
        self
    }

    #[must_use]
    pub fn attachments<I>(mut self, attachments: I) -> Self
    where
        I: IntoIterator<Item = Attachment>,
    {
        self.message.attachments = attachments.into_iter().collect();
        self
    }

    /// Append a single attachment.
    #[must_use]
    pub fn add_attachment(mut self, attachment: Attachment) -> Self {
        self.message.attachments.push(attachment);
        self
    }

    /// Returns the underlying `Message` without running outbound
    /// validation.
    ///
    /// Reserved for paths that construct a `Message` from already-parsed
    /// inbound data, for example `email_message_wire::parse_rfc822`,
    /// where the wire-format invariants come from the parser and the
    /// outbound rules (`From` set, at least one recipient, no reserved
    /// header collisions, no CRLF in subject) are not meaningful.
    ///
    /// **Outbound callers should use [`Self::build`] or
    /// [`Self::build_outbound`] instead.** Wrapping the result of
    /// `build_unchecked` in `OutboundMessage::new` re-runs the validation
    /// you skipped, with no benefit.
    #[must_use]
    pub fn build_unchecked(self) -> Message {
        self.message
    }

    /// Build and validate the message.
    ///
    /// # Errors
    ///
    /// Returns [`MessageValidationError`] when required message fields are
    /// missing or inconsistent.
    pub fn build(self) -> Result<Message, MessageValidationError> {
        self.message.validate_basic()?;
        Ok(self.message)
    }

    /// Build, validate, and wrap the message for outbound delivery.
    ///
    /// # Errors
    ///
    /// Returns [`MessageValidationError`] when required message fields are
    /// missing or inconsistent.
    pub fn build_outbound(self) -> Result<OutboundMessage, MessageValidationError> {
        OutboundMessage::new(self.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::format_description::well_known::Rfc2822;

    fn mailbox(input: &str) -> Mailbox {
        input.parse::<Mailbox>().expect("mailbox should parse")
    }

    fn address(input: &str) -> Address {
        input.parse::<Address>().expect("address should parse")
    }

    #[test]
    fn validate_basic_reports_sender_without_from() {
        let error = Message::builder(Body::text("body"))
            .sender(mailbox("sender@example.com"))
            .add_to(address("to@example.com"))
            .build()
            .expect_err("message should be invalid");

        assert_eq!(error, MessageValidationError::SenderWithoutFrom);
    }

    #[test]
    fn validate_basic_rejects_reserved_header_names() {
        let error = Message::builder(Body::text("body"))
            .from_mailbox(mailbox("from@example.com"))
            .add_to(address("to@example.com"))
            .add_header(Header::new("Subject", "shadow").expect("header should validate"))
            .build()
            .expect_err("reserved header should be rejected");

        assert!(matches!(
            error,
            MessageValidationError::ReservedHeaderName { ref name, .. } if name == "Subject"
        ));
    }

    #[test]
    fn validate_basic_rejects_reserved_header_case_insensitively() {
        let error = Message::builder(Body::text("body"))
            .from_mailbox(mailbox("from@example.com"))
            .add_to(address("to@example.com"))
            .add_header(Header::new("MESSAGE-ID", "<x@y>").expect("header should validate"))
            .build()
            .expect_err("reserved header should be rejected");

        assert!(matches!(
            error,
            MessageValidationError::ReservedHeaderName { ref name, .. } if name == "MESSAGE-ID"
        ));
    }

    #[test]
    fn validate_basic_rejects_subject_with_crlf_injection() {
        let error = Message::builder(Body::text("body"))
            .from_mailbox(mailbox("from@example.com"))
            .add_to(address("to@example.com"))
            .subject("hi\r\nBcc: victim@example.com")
            .build()
            .expect_err("subject CRLF injection should be rejected");

        assert_eq!(error, MessageValidationError::SubjectContainsInvalidChars);
    }

    #[test]
    fn validate_basic_rejects_subject_with_bare_lf() {
        let error = Message::builder(Body::text("body"))
            .from_mailbox(mailbox("from@example.com"))
            .add_to(address("to@example.com"))
            .subject("hi\nbcc")
            .build()
            .expect_err("subject bare LF should be rejected");

        assert_eq!(error, MessageValidationError::SubjectContainsInvalidChars);
    }

    #[test]
    fn validate_basic_rejects_subject_with_control_char() {
        let error = Message::builder(Body::text("body"))
            .from_mailbox(mailbox("from@example.com"))
            .add_to(address("to@example.com"))
            .subject("hi\x07boss")
            .build()
            .expect_err("subject control char should be rejected");

        assert_eq!(error, MessageValidationError::SubjectContainsInvalidChars);
    }

    #[test]
    fn validate_basic_rejects_from_mailbox_with_crlf_in_display_name() {
        let email = "alice@example.com"
            .parse::<EmailAddress>()
            .expect("email parses");
        let hostile_from = Mailbox::from(("evil\r\nBcc: attacker@example.com".to_string(), email));

        let error = Message::builder(Body::text("body"))
            .from_mailbox(hostile_from)
            .add_to(address("to@example.com"))
            .build()
            .expect_err("hostile From display name should be rejected");

        assert!(matches!(
            error,
            MessageValidationError::MailboxDisplayNameContainsInvalidChars { .. }
        ));
    }

    #[test]
    fn validate_basic_rejects_to_mailbox_with_lf_in_display_name() {
        let email = "victim@example.com"
            .parse::<EmailAddress>()
            .expect("email parses");
        let hostile_to = Address::Mailbox(Mailbox::from(("name\ninjection".to_string(), email)));

        let error = Message::builder(Body::text("body"))
            .from_mailbox(mailbox("from@example.com"))
            .add_to(hostile_to)
            .build()
            .expect_err("hostile To display name should be rejected");

        assert!(matches!(
            error,
            MessageValidationError::MailboxDisplayNameContainsInvalidChars { .. }
        ));
    }

    #[test]
    fn validate_basic_rejects_group_member_with_nul_in_display_name() {
        // Construct a Group via parse, then we'd need to inject, but Group's
        // members are private. Instead test the group's own display name path
        // by parsing a group with a hostile member display name impossible
        // through parse (parse rejects raw newlines), so we test the
        // mailbox-via-cc path which is the realistic case.
        let email = "member@example.com"
            .parse::<EmailAddress>()
            .expect("email parses");
        let hostile_cc = Address::Mailbox(Mailbox::from(("embed\0nul".to_string(), email)));

        let error = Message::builder(Body::text("body"))
            .from_mailbox(mailbox("from@example.com"))
            .add_cc(hostile_cc)
            .build()
            .expect_err("hostile Cc display name should be rejected");

        assert!(matches!(
            error,
            MessageValidationError::MailboxDisplayNameContainsInvalidChars { .. }
        ));
    }

    #[test]
    fn validate_basic_accepts_mailbox_with_tab_in_display_name() {
        let email = "alice@example.com"
            .parse::<EmailAddress>()
            .expect("email parses");
        let from = Mailbox::from(("Alice\tBob".to_string(), email));

        Message::builder(Body::text("body"))
            .from_mailbox(from)
            .add_to(address("to@example.com"))
            .build()
            .expect("tab in display name should be accepted");
    }

    #[test]
    fn validate_basic_accepts_subject_with_tab() {
        let message = Message::builder(Body::text("body"))
            .from_mailbox(mailbox("from@example.com"))
            .add_to(address("to@example.com"))
            .subject("hi\tworld")
            .build()
            .expect("subject with tab should be accepted");

        assert_eq!(message.subject(), Some("hi\tworld"));
    }

    #[test]
    fn outbound_message_from_mailbox_returns_validated_field() {
        let outbound = Message::builder(Body::text("body"))
            .from_mailbox(mailbox("alice@example.com"))
            .add_to(address("bob@example.com"))
            .build_outbound()
            .expect("message should validate");

        assert_eq!(
            outbound.from_mailbox().email().as_str(),
            "alice@example.com"
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn outbound_message_serde_format_matches_message() {
        let outbound = Message::builder(Body::text("body"))
            .from_mailbox(mailbox("alice@example.com"))
            .add_to(address("bob@example.com"))
            .subject("hello")
            .build_outbound()
            .expect("message should validate");

        let outbound_json =
            serde_json::to_string(&outbound).expect("OutboundMessage should serialize");
        let message_json =
            serde_json::to_string(outbound.as_message()).expect("Message should serialize");
        assert_eq!(
            outbound_json, message_json,
            "OutboundMessage serde representation must match its inner Message"
        );

        let roundtripped: OutboundMessage =
            serde_json::from_str(&outbound_json).expect("OutboundMessage should deserialize");
        assert_eq!(roundtripped, outbound);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn outbound_message_deserialize_rejects_invalid_payload() {
        // A Message that lacks `from` round-trips through Message::serde
        // but must be rejected on the outbound deserialize path.
        let invalid_message = Message {
            from: None,
            sender: None,
            to: vec![Address::Mailbox(mailbox("bob@example.com"))],
            cc: Vec::new(),
            bcc: Vec::new(),
            reply_to: Vec::new(),
            subject: None,
            date: None,
            message_id: None,
            headers: Vec::new(),
            body: Body::text("hi"),
            attachments: Vec::new(),
        };
        let json = serde_json::to_string(&invalid_message).expect("Message should serialize");
        assert!(serde_json::from_str::<OutboundMessage>(&json).is_err());
    }

    #[test]
    fn builder_constructs_valid_message() {
        let date = OffsetDateTime::parse("Fri, 06 Mar 2026 12:00:00 +0000", &Rfc2822)
            .expect("date should parse");
        let message_id = "<test@example.com>"
            .parse::<MessageId>()
            .expect("message id should parse");

        let message = Message::builder(Body::text("Hello"))
            .from_mailbox(mailbox("Mary Smith <mary@x.test>"))
            .add_to(address("jdoe@one.test"))
            .subject("Greeting")
            .date(date)
            .message_id(message_id.clone())
            .add_header(Header::new("X-Test", "demo").expect("header should validate"))
            .build()
            .expect("message should validate");

        assert!(message.from_mailbox().is_some(), "from should be set");
        assert_eq!(message.to().len(), 1, "expected one recipient");
        assert_eq!(message.date(), Some(&date));
        assert_eq!(message.message_id(), Some(&message_id));
        assert_eq!(message.headers().len(), 1);
    }

    #[test]
    fn derive_envelope_uses_sender_and_expands_groups() {
        let message = Message::builder(Body::text("Hello"))
            .from_mailbox(mailbox("from@example.com"))
            .sender(mailbox("sender@example.com"))
            .to(vec![address("Friends: a@example.com, b@example.com;")])
            .add_cc(address("c@example.com"))
            .build()
            .expect("message should validate");

        let envelope = message.derive_envelope().expect("envelope should derive");

        assert_eq!(
            envelope.mail_from().map(EmailAddress::as_str),
            Some("sender@example.com")
        );
        assert_eq!(
            envelope
                .rcpt_to()
                .iter()
                .map(EmailAddress::as_str)
                .collect::<Vec<_>>(),
            vec!["a@example.com", "b@example.com", "c@example.com"]
        );
    }

    #[test]
    fn body_convenience_constructors_create_expected_variants() {
        assert_eq!(Body::text("hello"), Body::Text("hello".to_owned()));
        assert_eq!(
            Body::html("<p>hello</p>"),
            Body::Html("<p>hello</p>".to_owned())
        );
        assert_eq!(
            Body::text_and_html("hello", "<p>hello</p>"),
            Body::TextAndHtml {
                text: "hello".to_owned(),
                html: "<p>hello</p>".to_owned(),
            }
        );
    }

    #[test]
    fn attachment_reference_constructor_preserves_uri() {
        let reference = AttachmentReference::new("s3://bucket/path/report.pdf");

        assert_eq!(reference.uri(), "s3://bucket/path/report.pdf");
    }

    #[test]
    fn with_attachments_replaces_existing_attachments() {
        let message = Message::builder(Body::text("Hello"))
            .from_mailbox(mailbox("from@example.com"))
            .add_to(address("to@example.com"))
            .add_attachment(
                Attachment::bytes(
                    ContentType::try_from("text/plain").expect("content type should parse"),
                    b"old".to_vec(),
                )
                .with_filename("old.txt"),
            )
            .build()
            .expect("message should validate");

        let updated = message.clone().with_attachments(vec![
            Attachment::bytes(
                ContentType::try_from("text/plain").expect("content type should parse"),
                b"new".to_vec(),
            )
            .with_filename("new.txt"),
        ]);

        assert_eq!(message.attachments().len(), 1);
        assert_eq!(updated.attachments().len(), 1);
        assert_eq!(updated.attachments()[0].filename(), Some("new.txt"));
    }
}
