use std::fmt::Display;
use std::str::FromStr;

/// A validated RFC 5322 `addr-spec` email address.
///
/// # Domain case-folding
///
/// Per RFC 5321 §2.4 the **domain** part of an address is
/// case-insensitive while the **local part** "MUST BE treated as case
/// sensitive." On construction this type lowercases the domain to
/// ASCII-lowercase and preserves the local-part bytes verbatim, so
/// `"User.Name@Example.COM"` and `"User.Name@example.com"` compare
/// equal via the derived `PartialEq` / `Eq` / `Hash`. IP-literal
/// domains (`[192.0.2.1]`, `[IPv6:::1]`) are not case-folded, RFC 5321
/// §4.1.3 says address literals are case-sensitive, they keep the
/// caller's bytes.
///
/// The case fold is intentional: `HashSet<EmailAddress>` and
/// `Envelope::rcpt_to: Vec<EmailAddress>` dedup paths previously kept
/// differently-cased spellings of the same SMTP mailbox as distinct
/// recipients. Callers who need byte-faithful preservation of the
/// original input should keep the source `String` separately; this
/// type is the SMTP-equivalence value.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EmailAddress {
    value: String,
}

impl EmailAddress {
    /// Returns the normalized address string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.value.as_str()
    }
}

impl Display for EmailAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl AsRef<str> for EmailAddress {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl From<EmailAddress> for String {
    fn from(value: EmailAddress) -> Self {
        value.value
    }
}

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct EmailAddressParseError(#[from] addr_spec::ParseError);

impl FromStr for EmailAddress {
    type Err = EmailAddressParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parsed = addr_spec::AddrSpec::from_str(s)?;
        // RFC 5321 §2.4: domain case-insensitive, local-part case-sensitive.
        // RFC 5321 §4.1.3: literal-form domains keep their bytes.
        // Use `into_serialized_parts` so quoted local parts (e.g.
        // `"john..doe"`) keep their quoting and IP-literal domains keep
        // their `[...]` brackets, `into_parts` would strip both.
        let is_literal = parsed.is_literal();
        let (local, domain) = parsed.into_serialized_parts();
        let value = if is_literal {
            format!("{local}@{domain}")
        } else {
            format!("{local}@{}", domain.to_ascii_lowercase())
        };
        Ok(Self { value })
    }
}

impl TryFrom<&str> for EmailAddress {
    type Error = EmailAddressParseError;

    /// Parses and validates an email from a string slice.
    ///
    /// ```rust
    /// use email_message::EmailAddress;
    ///
    /// let email = EmailAddress::try_from("jdoe@one.test").unwrap();
    /// assert_eq!(email.as_str(), "jdoe@one.test");
    /// ```
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::from_str(value)
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for EmailAddress {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for EmailAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(feature = "schemars")]
impl schemars::JsonSchema for EmailAddress {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> std::borrow::Cow<'static, str> {
        "EmailAddress".into()
    }

    fn schema_id() -> std::borrow::Cow<'static, str> {
        concat!(module_path!(), "::EmailAddress").into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "description": "RFC 5322 addr-spec email address"
        })
    }
}

#[cfg(feature = "arbitrary")]
impl<'a> arbitrary::Arbitrary<'a> for EmailAddress {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let local = u64::arbitrary(u)?;
        let domain = u32::arbitrary(u)?;
        format!("user{local}@domain{domain}.test")
            .parse()
            .map_err(|_| arbitrary::Error::IncorrectFormat)
    }
}

#[cfg(test)]
mod tests {
    use super::EmailAddress;

    const RFC_VALID_EMAILS: &[&str] = &[
        "jdoe@one.test",
        "simple@example.com",
        "very.common@example.com",
        "disposable.style.email.with+symbol@example.com",
        "other.email-with-hyphen@example.com",
        "fully-qualified-domain@example.com",
        "user.name+tag+sorting@example.com",
        "x@example.com",
        "example-indeed@strange-example.com",
        "admin@mailserver1",
        "example@s.example",
        "\"john..doe\"@example.org",
        "mailhost!username@example.org",
        "user%example.com@example.org",
    ];

    const INVALID_EMAILS: &[&str] = &[
        "plainaddress",
        "@missing-local.org",
        "A@b@c@example.com",
        "john..doe@example.org",
        "john.doe@example..org",
        "john.doe.@example.org",
        ".john.doe@example.org",
    ];

    #[test]
    fn email_from_str_accepts_rfc_examples() {
        for input in RFC_VALID_EMAILS {
            let parsed = input.parse::<EmailAddress>();
            assert!(parsed.is_ok(), "expected valid email: {input}");
        }
    }

    #[test]
    fn email_from_str_rejects_invalid_examples() {
        for input in INVALID_EMAILS {
            let parsed = input.parse::<EmailAddress>();
            assert!(parsed.is_err(), "expected invalid email: {input}");
        }
    }

    /// RFC 5321 §4.1.3 / RFC 5322 §3.4.1: `[domain-literal]` IP-literal
    /// domains are valid `addr-spec` forms. Internal SMTP relays often
    /// address recipients via IP literal; rejecting these surprises users
    /// who paste an RFC-valid address into the kernel.
    #[test]
    fn email_from_str_accepts_ipv4_literal_domain() {
        let parsed = "user@[192.168.1.1]".parse::<EmailAddress>();
        assert!(parsed.is_ok(), "expected IPv4 literal to parse: {parsed:?}");
    }

    #[test]
    fn email_from_str_accepts_ipv6_literal_domain() {
        let parsed = "user@[IPv6:fe80::1]".parse::<EmailAddress>();
        assert!(parsed.is_ok(), "expected IPv6 literal to parse: {parsed:?}");
    }

    /// RFC 5321 §2.4, the domain part is case-insensitive. Two
    /// differently-cased spellings of the same mailbox compare equal
    /// and hash to the same value. Local part stays case-sensitive.
    #[test]
    fn email_domain_is_case_folded_for_eq_and_hash() {
        let a: EmailAddress = "User.Name@Example.COM".parse().unwrap();
        let b: EmailAddress = "User.Name@example.com".parse().unwrap();
        assert_eq!(a, b);
        assert_eq!(a.as_str(), "User.Name@example.com");
        assert_eq!(b.as_str(), "User.Name@example.com");

        // HashSet dedup
        use std::collections::HashSet;
        let mut set: HashSet<EmailAddress> = HashSet::new();
        set.insert(a);
        assert!(set.contains(&b));
    }

    /// Local-part case is preserved verbatim per RFC 5321 §2.4.
    #[test]
    fn email_local_part_case_is_preserved() {
        let upper: EmailAddress = "John.Doe@example.com".parse().unwrap();
        let lower: EmailAddress = "john.doe@example.com".parse().unwrap();
        assert_ne!(upper, lower);
        assert_eq!(upper.as_str(), "John.Doe@example.com");
        assert_eq!(lower.as_str(), "john.doe@example.com");
    }

    /// IP-literal domains are case-sensitive per RFC 5321 §4.1.3
    /// they retain the caller's bytes.
    #[test]
    fn email_ipv6_literal_domain_is_not_case_folded() {
        let parsed: EmailAddress = "user@[IPv6:Fe80::1]".parse().unwrap();
        // The literal kept its uppercase letters (specifically `IPv6`
        // is the addr-spec convention; the inner address bytes are
        // also untouched by the kernel, addr-spec may normalize
        // internally but we don't `to_ascii_lowercase` over the
        // bracketed form).
        assert!(parsed.as_str().contains("IPv6"));
    }
}
