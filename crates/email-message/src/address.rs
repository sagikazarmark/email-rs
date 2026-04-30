use std::fmt::Display;
use std::str::FromStr;
use std::sync::OnceLock;

use crate::email::{EmailAddress, EmailAddressParseError};

static ADDRESS_PARSER: OnceLock<mail_parser::MessageParser> = OnceLock::new();

/// A mailbox address with optional display name.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Mailbox {
    name: Option<String>,
    email: EmailAddress,
}

impl Mailbox {
    /// Returns the optional display name.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Returns the mailbox email address.
    #[must_use]
    pub const fn email(&self) -> &EmailAddress {
        &self.email
    }
}

impl From<EmailAddress> for Mailbox {
    fn from(email: EmailAddress) -> Self {
        Self { name: None, email }
    }
}

impl From<(String, EmailAddress)> for Mailbox {
    fn from((name, email): (String, EmailAddress)) -> Self {
        Self {
            name: Some(name),
            email,
        }
    }
}

impl From<(Option<String>, EmailAddress)> for Mailbox {
    fn from((name, email): (Option<String>, EmailAddress)) -> Self {
        Self { name, email }
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MailboxParseError {
    #[error("expected a single mailbox, found {found} address item(s)")]
    ExpectedSingleMailbox { found: usize },
    #[error("expected mailbox but found group")]
    UnexpectedAddressKind,
    #[error("mailbox list contains group entries")]
    ContainsGroupEntry,
    #[error("mailbox parse backend failed")]
    Backend {
        #[source]
        source: AddressBackendError,
    },
}

impl FromStr for Mailbox {
    type Err = MailboxParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(email) = EmailAddress::from_str(s) {
            return Ok(Self::from(email));
        }

        let addresses =
            parse_address_items(s).map_err(|source| MailboxParseError::Backend { source })?;
        if addresses.len() != 1 {
            return Err(MailboxParseError::ExpectedSingleMailbox {
                found: addresses.len(),
            });
        }

        match addresses.into_iter().next() {
            Some(Address::Mailbox(mailbox)) => Ok(mailbox),
            _ => Err(MailboxParseError::UnexpectedAddressKind),
        }
    }
}

impl TryFrom<&str> for Mailbox {
    type Error = MailboxParseError;

    /// Parses a single mailbox from a string slice.
    ///
    /// ```rust
    /// use email_message::Mailbox;
    ///
    /// let mailbox = Mailbox::try_from("Mary Smith <mary@x.test>").unwrap();
    /// assert_eq!(mailbox.name(), Some("Mary Smith"));
    /// assert_eq!(mailbox.email().as_str(), "mary@x.test");
    /// ```
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::from_str(value)
    }
}

/// Renders the mailbox as `"display name" <email>` or just `email` if no
/// display name is set.
///
/// The output is **UTF-8-direct**: a non-ASCII display name is emitted
/// verbatim (e.g. `"José" <jose@example.com>`). This is the right shape
/// for HTTP-API consumers (Postmark, Resend, Mailgun, Loops) which
/// JSON-encode UTF-8 strings natively.
///
/// **Do not use the result directly as an RFC 5322 header value.** SMTP
/// headers are 7-bit and require RFC 2047 encoded-word wrapping for
/// non-ASCII display names; the wire renderer
/// (`email_message_wire::render_rfc822`) applies that encoding
/// separately. Routing `Mailbox::to_string()` straight into a `From:`
/// or `To:` header would emit a malformed RFC 5322 line.
impl Display for Mailbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.name() {
            Some(name) => {
                write_quoted(name, f)?;
                f.write_str(" <")?;
                self.email.fmt(f)?;
                f.write_str(">")
            }
            None => self.email.fmt(f),
        }
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for Mailbox {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Mailbox {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(feature = "arbitrary")]
impl<'a> arbitrary::Arbitrary<'a> for Mailbox {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let email = EmailAddress::arbitrary(u)?;
        if bool::arbitrary(u)? {
            let name = format!("User {}", u8::arbitrary(u)?);
            Ok(Self {
                name: Some(name),
                email,
            })
        } else {
            Ok(Self { name: None, email })
        }
    }
}

/// A named address group containing mailbox members.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Group {
    name: String,
    members: Vec<Mailbox>,
}

impl Group {
    /// Returns the group display name.
    #[must_use]
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    /// Returns group members.
    #[must_use]
    pub fn members(&self) -> &[Mailbox] {
        self.members.as_slice()
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum GroupParseError {
    #[error("expected a single group, found {found} address item(s)")]
    ExpectedSingleGroup { found: usize },
    #[error("expected group but found mailbox")]
    UnexpectedAddressKind,
    #[error("group parse backend failed")]
    Backend {
        #[source]
        source: AddressBackendError,
    },
}

impl FromStr for Group {
    type Err = GroupParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let addresses =
            parse_address_items(s).map_err(|source| GroupParseError::Backend { source })?;
        if addresses.len() != 1 {
            return Err(GroupParseError::ExpectedSingleGroup {
                found: addresses.len(),
            });
        }

        match addresses.into_iter().next() {
            Some(Address::Group(group)) => Ok(group),
            _ => Err(GroupParseError::UnexpectedAddressKind),
        }
    }
}

impl TryFrom<&str> for Group {
    type Error = GroupParseError;

    /// Parses a single group from a string slice.
    ///
    /// ```rust
    /// use email_message::Group;
    ///
    /// let group = Group::try_from("Undisclosed recipients:;").unwrap();
    /// assert_eq!(group.name(), "Undisclosed recipients");
    /// assert!(group.members().is_empty());
    /// ```
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::from_str(value)
    }
}

/// Renders the group as `"display name": member1, member2, ...;`.
///
/// Same UTF-8-direct caveat as [`Display for Mailbox`]: suitable for
/// HTTP-API consumers, not directly safe as an RFC 5322 header value.
impl Display for Group {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write_quoted(self.name(), f)?;
        f.write_str(":")?;
        for (idx, member) in self.members().iter().enumerate() {
            if idx > 0 {
                f.write_str(", ")?;
            }
            member.fmt(f)?;
        }
        f.write_str(";")
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for Group {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Group {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(feature = "arbitrary")]
impl<'a> arbitrary::Arbitrary<'a> for Group {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let member_count = usize::from(u.int_in_range::<u8>(0..=3)?);
        let mut members = Vec::with_capacity(member_count);
        for _ in 0..member_count {
            members.push(Mailbox::arbitrary(u)?);
        }
        Ok(Self {
            name: format!("Group {}", u8::arbitrary(u)?),
            members,
        })
    }
}

/// A single address item: either a mailbox or a group.
///
/// Deliberately *not* `#[non_exhaustive]`. RFC 5322 §3.4 closes the
/// address grammar to exactly `mailbox / group`; the kernel cannot
/// honestly add a third variant without an RFC update. The
/// derive-required exhaustive `match` lets downstream callers branch
/// on every variant without an `_ =>` arm, useful when an extension
/// crate wants type-safe coverage of the address space.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Address {
    Mailbox(Mailbox),
    Group(Group),
}

impl From<Mailbox> for Address {
    fn from(value: Mailbox) -> Self {
        Self::Mailbox(value)
    }
}

impl From<Group> for Address {
    fn from(value: Group) -> Self {
        Self::Group(value)
    }
}

impl Address {
    /// Returns the mailbox entries represented by this address item.
    #[must_use]
    pub fn mailboxes(&self) -> impl Iterator<Item = &Mailbox> {
        match self {
            Self::Mailbox(mailbox) => std::slice::from_ref(mailbox).iter(),
            Self::Group(group) => group.members().iter(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AddressParseError {
    #[error("expected a single address, found {found} address item(s)")]
    ExpectedSingleAddress { found: usize },
    #[error("address parse backend failed")]
    Backend {
        #[source]
        source: AddressBackendError,
    },
}

impl FromStr for Address {
    type Err = AddressParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(email) = EmailAddress::from_str(s) {
            return Ok(Self::Mailbox(Mailbox::from(email)));
        }

        let addresses =
            parse_address_items(s).map_err(|source| AddressParseError::Backend { source })?;
        if addresses.len() != 1 {
            return Err(AddressParseError::ExpectedSingleAddress {
                found: addresses.len(),
            });
        }

        addresses
            .into_iter()
            .next()
            .ok_or(AddressParseError::ExpectedSingleAddress { found: 0 })
    }
}

impl TryFrom<&str> for Address {
    type Error = AddressParseError;

    /// Parses a single address (mailbox or group) from a string slice.
    ///
    /// ```rust
    /// use email_message::Address;
    ///
    /// let address = Address::try_from("jdoe@one.test").unwrap();
    /// assert_eq!(address.to_string(), "jdoe@one.test");
    /// ```
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::from_str(value)
    }
}

/// Forwards to the underlying [`Display for Mailbox`] or
/// [`Display for Group`]; same UTF-8-direct caveat applies.
impl Display for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Mailbox(mailbox) => mailbox.fmt(f),
            Self::Group(group) => group.fmt(f),
        }
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for Address {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Address {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(feature = "arbitrary")]
impl<'a> arbitrary::Arbitrary<'a> for Address {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        if bool::arbitrary(u)? {
            Ok(Self::Mailbox(Mailbox::arbitrary(u)?))
        } else {
            Ok(Self::Group(Group::arbitrary(u)?))
        }
    }
}

macro_rules! impl_address_collection {
    ($(#[$meta:meta])* $name:ident, $item:ty, $error:ty, $parse_fn:expr) => {
        $(#[$meta])*
        #[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
        #[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
        pub struct $name {
            items: Vec<$item>,
        }

        impl $name {
            #[must_use]
            pub fn len(&self) -> usize {
                self.items.len()
            }

            #[must_use]
            pub fn is_empty(&self) -> bool {
                self.items.is_empty()
            }

            pub fn iter(&self) -> std::slice::Iter<'_, $item> {
                self.items.iter()
            }

            pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, $item> {
                self.items.iter_mut()
            }

            #[must_use]
            pub fn as_slice(&self) -> &[$item] {
                self.items.as_slice()
            }

            #[must_use]
            pub fn into_vec(self) -> Vec<$item> {
                self.items
            }
        }

        impl From<Vec<$item>> for $name {
            fn from(items: Vec<$item>) -> Self {
                Self { items }
            }
        }

        impl From<$name> for Vec<$item> {
            fn from(value: $name) -> Self {
                value.items
            }
        }

        impl FromStr for $name {
            type Err = $error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let items = ($parse_fn)(s)?;
                Ok(Self { items })
            }
        }

        impl TryFrom<&str> for $name {
            type Error = $error;

            /// Parses a list from a string slice.
            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::from_str(value)
            }
        }

        impl IntoIterator for $name {
            type Item = $item;
            type IntoIter = std::vec::IntoIter<$item>;

            fn into_iter(self) -> Self::IntoIter {
                self.items.into_iter()
            }
        }

        impl<'a> IntoIterator for &'a $name {
            type Item = &'a $item;
            type IntoIter = std::slice::Iter<'a, $item>;

            fn into_iter(self) -> Self::IntoIter {
                self.items.iter()
            }
        }

        impl<'a> IntoIterator for &'a mut $name {
            type Item = &'a mut $item;
            type IntoIter = std::slice::IterMut<'a, $item>;

            fn into_iter(self) -> Self::IntoIter {
                self.items.iter_mut()
            }
        }

        impl AsRef<[$item]> for $name {
            fn as_ref(&self) -> &[$item] {
                self.items.as_slice()
            }
        }

        impl std::iter::FromIterator<$item> for $name {
            fn from_iter<T: IntoIterator<Item = $item>>(iter: T) -> Self {
                Self {
                    items: iter.into_iter().collect(),
                }
            }
        }

        impl Extend<$item> for $name {
            fn extend<T: IntoIterator<Item = $item>>(&mut self, iter: T) {
                self.items.extend(iter);
            }
        }

        impl Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                for (idx, item) in self.items.iter().enumerate() {
                    if idx > 0 {
                        f.write_str(", ")?;
                    }
                    item.fmt(f)?;
                }
                Ok(())
            }
        }

        #[cfg(feature = "serde")]
        impl serde::Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.serialize_str(&self.to_string())
            }
        }

        #[cfg(feature = "serde")]
        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                value.parse().map_err(serde::de::Error::custom)
            }
        }

        #[cfg(feature = "arbitrary")]
        impl<'a> arbitrary::Arbitrary<'a> for $name {
            fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
                let len = usize::from(u.int_in_range::<u8>(0..=4)?);
                let mut items = Vec::with_capacity(len);
                for _ in 0..len {
                    items.push(<$item>::arbitrary(u)?);
                }
                Ok(Self { items })
            }
        }
    };
}

impl_address_collection!(
    /// A parsed list of address items.
    ///
    /// This is used instead of `Vec<Address>` for `FromStr`, because Rust's orphan
    /// rules do not allow implementing foreign traits for foreign types.
    AddressList,
    Address,
    AddressParseError,
    |s| parse_address_items(s).map_err(|source| AddressParseError::Backend { source })
);

impl<'a> TryFrom<Vec<&'a str>> for AddressList {
    type Error = AddressParseError;

    fn try_from(value: Vec<&'a str>) -> Result<Self, Self::Error> {
        value
            .into_iter()
            .map(Address::from_str)
            .collect::<Result<Vec<_>, _>>()
            .map(Self::from)
    }
}

impl<'a> TryFrom<&'a [&'a str]> for AddressList {
    type Error = AddressParseError;

    fn try_from(value: &'a [&'a str]) -> Result<Self, Self::Error> {
        value
            .iter()
            .copied()
            .map(Address::from_str)
            .collect::<Result<Vec<_>, _>>()
            .map(Self::from)
    }
}

impl<'a> TryFrom<Vec<&'a str>> for MailboxList {
    type Error = MailboxParseError;

    fn try_from(value: Vec<&'a str>) -> Result<Self, Self::Error> {
        value
            .into_iter()
            .map(Mailbox::from_str)
            .collect::<Result<Vec<_>, _>>()
            .map(Self::from)
    }
}

impl<'a> TryFrom<&'a [&'a str]> for MailboxList {
    type Error = MailboxParseError;

    fn try_from(value: &'a [&'a str]) -> Result<Self, Self::Error> {
        value
            .iter()
            .copied()
            .map(Mailbox::from_str)
            .collect::<Result<Vec<_>, _>>()
            .map(Self::from)
    }
}

impl_address_collection!(
    /// A parsed list of mailbox items.
    ///
    /// Group entries are rejected when parsing into `MailboxList`.
    MailboxList,
    Mailbox,
    MailboxParseError,
    |s| {
        let addresses = parse_address_items(s).map_err(|source| MailboxParseError::Backend { source })?;
        let mut items = Vec::with_capacity(addresses.len());

        for address in addresses {
            match address {
                Address::Mailbox(mailbox) => items.push(mailbox),
                Address::Group(_) => return Err(MailboxParseError::ContainsGroupEntry),
            }
        }

        Ok(items)
    }
);

/// Maximum byte length accepted by the address-list parser before
/// rejecting outright. 64 KiB is far above any realistic header value
///, RFC 5322 caps physical lines at 998 bytes; even a header folded
/// across hundreds of continuation lines stays well under this. The
/// cap exists to prevent the `format!("To: {input}\r\n\r\n")`
/// allocation amplification on adversarial multi-megabyte input.
pub const MAX_ADDRESS_INPUT_BYTES: usize = 64 * 1024;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AddressBackendError {
    #[error("address input contains raw newline characters")]
    InputContainsRawNewlines,
    #[error("address input is {len} bytes, exceeding maximum of {max}")]
    #[non_exhaustive]
    InputTooLong { len: usize, max: usize },
    #[error("failed to parse address header")]
    HeaderParse,
    #[error("parsed header did not contain address data")]
    MissingAddress,
    #[error("mailbox is missing addr-spec")]
    MissingAddrSpec,
    #[error("invalid addr-spec `{input}`")]
    InvalidAddrSpec {
        input: String,
        #[source]
        source: EmailAddressParseError,
    },
    #[error("group member at index {index} is missing addr-spec")]
    GroupMemberMissingAddrSpec { index: usize },
    #[error("invalid group member addr-spec `{input}` at index {index}")]
    InvalidGroupMemberAddrSpec {
        index: usize,
        input: String,
        #[source]
        source: EmailAddressParseError,
    },
    #[error("group is missing a name")]
    GroupMissingName,
}

fn write_quoted(value: &str, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str("\"")?;
    for ch in value.chars() {
        if ch == '\\' || ch == '"' {
            f.write_str("\\")?;
        }
        f.write_str(ch.encode_utf8(&mut [0; 4]))?;
    }
    f.write_str("\"")
}

/// Parse-side address-list extractor.
///
/// # Byte discipline at the parser
///
/// This parser deliberately accepts more than the message-level gate
/// rejects. Specifically: it rejects raw CR / LF (which would let an
/// attacker inject a new header line at the parser layer) and
/// inputs over [`MAX_ADDRESS_INPUT_BYTES`]; it does **not** reject
/// NUL or other non-tab ASCII control characters in display-name
/// content.
///
/// The asymmetry is intentional. The kernel's stricter byte-
/// discipline lives at [`crate::Message::validate_basic`] and fires
/// when an outbound `OutboundMessage` is built; inbound parsing is
/// best-effort and used in forensic / archival / replay workflows
/// where rejecting BEL / VT / ESC in display names from real-world
/// malformed-but-recoverable mail loses information. A `Mailbox`
/// carrying questionable bytes is fine *as a parsed value*; it
/// cannot reach an outbound wire renderer because the message-level
/// gate catches it first.
///
/// Callers handing a `Mailbox.name()` directly to a logging sink or
/// non-validated downstream consumer are responsible for their own
/// byte-discipline check.
fn parse_address_items(input: &str) -> Result<Vec<Address>, AddressBackendError> {
    if input.len() > MAX_ADDRESS_INPUT_BYTES {
        return Err(AddressBackendError::InputTooLong {
            len: input.len(),
            max: MAX_ADDRESS_INPUT_BYTES,
        });
    }
    if input.contains('\r') || input.contains('\n') {
        return Err(AddressBackendError::InputContainsRawNewlines);
    }

    let raw = format!("To: {input}\r\n\r\n");
    let parser =
        ADDRESS_PARSER.get_or_init(|| mail_parser::MessageParser::new().with_address_headers());
    let message = parser
        .parse_headers(raw.as_bytes())
        .ok_or(AddressBackendError::HeaderParse)?;
    let parsed = message.to().ok_or(AddressBackendError::MissingAddress)?;

    match parsed {
        mail_parser::Address::List(list) => list
            .iter()
            .map(convert_mailbox)
            .map(|result| result.map(Address::Mailbox))
            .collect(),
        // mail_parser switches the whole header to the `Group` shape as soon
        // as any group syntax appears, and wraps flat mailboxes that appear
        // before/between/after named groups into a synthetic
        // `Group { name: None, ... }`. Flatten those back to `Mailbox`
        // entries so a mixed header like
        // `alice@example.com, Team: bob@team.com;, dave@example.com`
        // produces three items in order rather than a parse error.
        mail_parser::Address::Group(groups) => {
            let mut items = Vec::with_capacity(groups.len());
            for group in groups {
                if group.name.is_some() {
                    items.push(Address::Group(convert_group(group)?));
                } else {
                    for addr in group.addresses.iter() {
                        items.push(Address::Mailbox(convert_mailbox(addr)?));
                    }
                }
            }
            Ok(items)
        }
    }
}

fn convert_mailbox(value: &mail_parser::Addr<'_>) -> Result<Mailbox, AddressBackendError> {
    let raw_email = value
        .address()
        .ok_or(AddressBackendError::MissingAddrSpec)?;
    let email = EmailAddress::from_str(raw_email).map_err(|source| {
        AddressBackendError::InvalidAddrSpec {
            input: raw_email.to_owned(),
            source,
        }
    })?;

    Ok(match value.name() {
        Some(name) => Mailbox::from((name.to_owned(), email)),
        None => Mailbox::from(email),
    })
}

fn convert_group(value: &mail_parser::Group<'_>) -> Result<Group, AddressBackendError> {
    let name = value
        .name
        .as_deref()
        .ok_or(AddressBackendError::GroupMissingName)?
        .to_owned();
    let mut members = Vec::with_capacity(value.addresses.len());
    for (index, member) in value.addresses.iter().enumerate() {
        let mailbox = convert_mailbox(member).map_err(|error| match error {
            AddressBackendError::MissingAddrSpec => {
                AddressBackendError::GroupMemberMissingAddrSpec { index }
            }
            AddressBackendError::InvalidAddrSpec { input, source } => {
                AddressBackendError::InvalidGroupMemberAddrSpec {
                    index,
                    input,
                    source,
                }
            }
            other => other,
        })?;
        members.push(mailbox);
    }

    Ok(Group { name, members })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mailbox_from_str_accepts_rfc_examples() {
        let parsed = "Mary Smith <mary@x.test>".parse::<Mailbox>();
        assert!(parsed.is_ok(), "expected valid mailbox");

        let parsed = "jdoe@one.test".parse::<Mailbox>();
        assert!(parsed.is_ok(), "expected valid mailbox");
    }

    #[test]
    fn mailbox_from_str_rejects_group() {
        let parsed = "Undisclosed recipients:;".parse::<Mailbox>();
        assert!(matches!(
            parsed,
            Err(MailboxParseError::UnexpectedAddressKind)
        ));
    }

    #[test]
    fn group_from_str_accepts_rfc_examples() {
        let parsed =
            "A Group:Ed Jones <c@a.test>,joe@where.test,John <jdoe@one.test>;".parse::<Group>();
        assert!(parsed.is_ok(), "expected valid group");

        let parsed = "Undisclosed recipients:;".parse::<Group>();
        assert!(parsed.is_ok(), "expected valid group");
    }

    #[test]
    fn address_list_roundtrip() {
        let list = "Mary Smith <mary@x.test>, jdoe@one.test"
            .parse::<AddressList>()
            .expect("address list should parse");
        let rendered = list.to_string();
        let reparsed = rendered
            .parse::<AddressList>()
            .expect("rendered address list should parse");
        assert_eq!(reparsed.as_slice(), list.as_slice());
    }

    #[test]
    fn mailbox_from_str_rejects_input_with_raw_newline() {
        let parsed = "Mary Smith <mary@x.test>\nBcc: victim@example.com".parse::<Mailbox>();
        assert!(matches!(
            parsed,
            Err(MailboxParseError::Backend {
                source: AddressBackendError::InputContainsRawNewlines,
            })
        ));
    }
}
