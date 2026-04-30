use std::fmt::Display;
use std::str::FromStr;

use crate::email::EmailAddressParseError;

/// A validated RFC 5322 `Message-ID` field value.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MessageId(String);

impl MessageId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl Display for MessageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Reasons a string cannot be parsed as an RFC 5322 `Message-ID`.
///
/// ```rust
/// use email_message::{MessageId, MessageIdParseError};
///
/// // Brackets are mandatory.
/// assert_eq!(
///     "abc@example.com".parse::<MessageId>().unwrap_err(),
///     MessageIdParseError::MissingBrackets,
/// );
///
/// // Local part validates against the addr-spec dot-atom grammar:
/// // a leading dot is illegal.
/// assert!(matches!(
///     "<.bad@example.com>".parse::<MessageId>().unwrap_err(),
///     MessageIdParseError::InvalidContent { .. },
/// ));
///
/// // A well-formed Message-ID round-trips its bracketed form.
/// let parsed = "<good@example.com>".parse::<MessageId>().unwrap();
/// assert_eq!(parsed.as_str(), "<good@example.com>");
/// ```
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MessageIdParseError {
    #[error("Message-ID must be enclosed in angle brackets")]
    MissingBrackets,
    #[error("Message-ID contains whitespace")]
    ContainsWhitespace,
    #[error("Message-ID is missing the local part")]
    MissingLocal,
    #[error("Message-ID is missing the domain part")]
    MissingDomain,
    #[error("Message-ID local-part or domain is malformed")]
    #[non_exhaustive]
    InvalidContent {
        #[source]
        source: EmailAddressParseError,
    },
    #[error(
        "Message-ID `id-left` uses the obsolete quoted-string form; the kernel commits to RFC 5322 dot-atom-text only"
    )]
    ObsoleteIdLeftForm,
}

impl PartialEq for MessageIdParseError {
    fn eq(&self, other: &Self) -> bool {
        // Pragmatic equality: variants compare by tag, ignoring the
        // boxed `source` chain on `InvalidContent`. Sufficient for tests
        // and avoids forcing `Eq` on the `addr_spec::ParseError` we
        // transitively carry.
        matches!(
            (self, other),
            (Self::MissingBrackets, Self::MissingBrackets)
                | (Self::ContainsWhitespace, Self::ContainsWhitespace)
                | (Self::MissingLocal, Self::MissingLocal)
                | (Self::MissingDomain, Self::MissingDomain)
                | (Self::InvalidContent { .. }, Self::InvalidContent { .. })
                | (Self::ObsoleteIdLeftForm, Self::ObsoleteIdLeftForm)
        )
    }
}

impl Eq for MessageIdParseError {}

impl FromStr for MessageId {
    type Err = MessageIdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let value = s.trim();
        if !(value.starts_with('<') && value.ends_with('>') && value.len() >= 2) {
            return Err(MessageIdParseError::MissingBrackets);
        }

        if value.chars().any(char::is_whitespace) {
            return Err(MessageIdParseError::ContainsWhitespace);
        }

        let inner = &value[1..value.len() - 1];

        // RFC 5322 §3.6.4 `id-left = dot-atom-text / obs-id-left`. The
        // kernel commits to dot-atom-text; `obs-id-left` (which permits
        // `quoted-string`) is the obsolete branch we deliberately reject
        // so equality between canonical and quoted-string spellings
        // doesn't drift (the type derives `Eq`/`Hash` over the stored
        // bytes).
        if inner.starts_with('"') {
            return Err(MessageIdParseError::ObsoleteIdLeftForm);
        }

        // Empty local / empty domain are caught by addr-spec's normalize
        // (it rejects `@example.com`, `abc@`, and `abc` for missing-`@`).
        // We still distinguish the missing-local / missing-domain /
        // no-`@` cases for ergonomic error messages: addr-spec returns a
        // generic parse error for all three, but the kernel can be more
        // specific on the obvious shape problems.
        if let Some((local, domain)) = inner.split_once('@') {
            if local.is_empty() {
                return Err(MessageIdParseError::MissingLocal);
            }
            if domain.is_empty() {
                return Err(MessageIdParseError::MissingDomain);
            }
        } else {
            return Err(MessageIdParseError::MissingDomain);
        }

        // RFC 5321 §2.4: domain case-insensitive, local-part case-sensitive.
        // RFC 5321 §4.1.3: literal-form domains keep their bytes. Mirrors the
        // case-folding `EmailAddress::from_str` performs so two MessageIds that are
        // RFC 5321-equivalent compare equal under derived `Eq`/`Hash`.
        let parsed = addr_spec::AddrSpec::from_str(inner).map_err(|error| {
            MessageIdParseError::InvalidContent {
                source: EmailAddressParseError::from(error),
            }
        })?;
        let is_literal = parsed.is_literal();
        let (local, domain) = parsed.into_serialized_parts();
        let normalized = if is_literal {
            format!("<{local}@{domain}>")
        } else {
            format!("<{local}@{}>", domain.to_ascii_lowercase())
        };

        Ok(Self(normalized))
    }
}

impl TryFrom<&str> for MessageId {
    type Error = MessageIdParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::from_str(value)
    }
}

impl From<MessageId> for String {
    fn from(value: MessageId) -> Self {
        value.0
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for MessageId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for MessageId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(feature = "arbitrary")]
impl<'a> arbitrary::Arbitrary<'a> for MessageId {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let local = u64::arbitrary(u)?;
        let domain = u32::arbitrary(u)?;
        Ok(Self(format!("<{local}@{domain}.test>")))
    }
}

#[cfg(test)]
mod tests {
    use super::{MessageId, MessageIdParseError};

    #[test]
    fn message_id_from_str_accepts_valid_values() {
        let parsed = "<abc@example.com>".parse::<MessageId>();
        assert!(parsed.is_ok(), "expected valid message id");
    }

    #[test]
    fn message_id_from_str_rejects_missing_brackets() {
        let parsed = "abc@example.com".parse::<MessageId>();
        assert_eq!(parsed.unwrap_err(), MessageIdParseError::MissingBrackets);
    }

    #[test]
    fn message_id_from_str_rejects_missing_at() {
        // `<abc>` has no `@`; the `id-right` (domain) portion is
        // structurally absent.
        let parsed = "<abc>".parse::<MessageId>();
        assert_eq!(parsed.unwrap_err(), MessageIdParseError::MissingDomain);
    }

    #[test]
    fn message_id_from_str_rejects_whitespace() {
        let parsed = "<abc @example.com>".parse::<MessageId>();
        assert_eq!(parsed.unwrap_err(), MessageIdParseError::ContainsWhitespace);
    }

    #[test]
    fn message_id_from_str_rejects_empty_local_part() {
        let parsed = "<@example.com>".parse::<MessageId>();
        assert_eq!(parsed.unwrap_err(), MessageIdParseError::MissingLocal);
    }

    #[test]
    fn message_id_from_str_rejects_empty_domain() {
        let parsed = "<abc@>".parse::<MessageId>();
        assert_eq!(parsed.unwrap_err(), MessageIdParseError::MissingDomain);
    }

    #[test]
    fn message_id_from_str_rejects_dot_atom_violations() {
        // Leading dot, double dot, trailing dot in the local-part are
        // dot-atom violations; previously slipped through.
        for input in [
            "<.bad@example.com>",
            "<a..b@example.com>",
            "<a.@example.com>",
        ] {
            let parsed = input.parse::<MessageId>();
            assert!(
                matches!(parsed, Err(MessageIdParseError::InvalidContent { .. })),
                "expected InvalidContent for {input}, got {parsed:?}"
            );
        }
    }

    /// RFC 5322 §3.6.4 `id-left = dot-atom-text / obs-id-left`. The kernel
    /// commits to dot-atom-text only; `obs-id-left` (which permits
    /// `quoted-string`) is the obsolete branch. Accepting quoted-string
    /// here would mean two semantically equal IDs (canonical vs
    /// quoted-string spelling) hash and compare unequal because
    /// `MessageId` derives `Eq`/`Hash` over the stored bytes.
    #[test]
    fn message_id_from_str_rejects_quoted_string_id_left() {
        let parsed = "<\"weird\"@example.com>".parse::<MessageId>();
        assert_eq!(parsed.unwrap_err(), MessageIdParseError::ObsoleteIdLeftForm);
    }

    #[test]
    fn message_id_from_str_rejects_quoted_at_in_local_part() {
        let parsed = "<\"a@b\"@example.com>".parse::<MessageId>();
        assert_eq!(parsed.unwrap_err(), MessageIdParseError::ObsoleteIdLeftForm);
    }

    /// Two RFC 5321-equivalent message ids that differ only in domain casing
    /// must compare equal and hash identically. Mirrors `EmailAddress`'s case-folding
    /// guarantee.
    #[test]
    fn message_id_from_str_case_folds_domain() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let upper = "<foo@Example.COM>"
            .parse::<MessageId>()
            .expect("upper-case domain should parse");
        let lower = "<foo@example.com>"
            .parse::<MessageId>()
            .expect("lower-case domain should parse");

        assert_eq!(upper, lower);
        assert_eq!(upper.as_str(), "<foo@example.com>");

        let mut h_upper = DefaultHasher::new();
        upper.hash(&mut h_upper);
        let mut h_lower = DefaultHasher::new();
        lower.hash(&mut h_lower);
        assert_eq!(h_upper.finish(), h_lower.finish());
    }
}
