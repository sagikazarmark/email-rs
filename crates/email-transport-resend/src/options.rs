use std::collections::BTreeMap;

use email_transport::TransportOption;
use serde_json::Value;

/// Resend-specific options for a single send attempt.
///
/// This is the one typed value inserted into [`email_transport::SendOptions::transport_options`]
/// for Resend. With the `serde` feature enabled, the same shape can also be
/// serialized through `SendOptions`' provider-keyed `transport_options` wire
/// shape under [`TransportOption::provider_key`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct ResendSendOptions {
    /// Resend dashboard tags attached to the email.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub tags: Vec<ResendTag>,
    /// Optional Resend template render settings.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub template: Option<ResendTemplate>,
}

impl TransportOption for ResendSendOptions {
    fn provider_key() -> &'static str {
        "resend"
    }
}

impl ResendSendOptions {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_tag(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.tags.push(ResendTag::new(name, value));
        self
    }

    #[must_use]
    pub fn with_tags<I, T>(mut self, tags: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<ResendTag>,
    {
        self.tags.extend(tags.into_iter().map(Into::into));
        self
    }

    #[must_use]
    pub fn with_template(mut self, template: ResendTemplate) -> Self {
        self.template = Some(template);
        self
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.tags.is_empty() && self.template.is_none()
    }
}

/// A Resend tag attached to an outgoing email for filtering /
/// segmentation in the Resend dashboard.
///
/// # Validation
///
/// The kernel does not validate `name` or `value` byte-for-byte.
/// Resend's API documents constraints on tag tokens (per their
/// docs: ASCII letters, digits, hyphen, underscore; per-field
/// length cap on the order of a few hundred chars), but those
/// rules can shift between API versions and the kernel
/// deliberately delegates the check to the provider's typed 400
/// response. On a violation, the adapter surfaces the provider's
/// `name` field through [`email_transport::TransportError::provider_error_code`]
/// (e.g. `"validation_error"`) and the human-readable message
/// through `error.message`. Callers who need build-time validation
/// against current Resend rules should layer their own newtype on
/// top of this struct.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ResendTag {
    pub name: String,
    pub value: String,
}

impl ResendTag {
    #[must_use]
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

impl<N, V> From<(N, V)> for ResendTag
where
    N: Into<String>,
    V: Into<String>,
{
    fn from((name, value): (N, V)) -> Self {
        Self::new(name, value)
    }
}

/// A Resend template id and the variables used to render it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ResendTemplate {
    /// Resend template identifier, for example `tmpl_...`.
    pub id: String,
    /// Template variables serialized as a JSON object.
    ///
    /// Stored in a [`BTreeMap`] so queued/provider-option serialization stays
    /// deterministic across processes. Resend's API receives the variables as
    /// a JSON object regardless of map type, so duplicate keys remain
    /// non-representable on the wire by RFC 8259.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub variables: Option<BTreeMap<String, Value>>,
}

impl ResendTemplate {
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            variables: None,
        }
    }

    #[must_use]
    pub fn with_variable(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.variables
            .get_or_insert_with(BTreeMap::new)
            .insert(key.into(), value.into());
        self
    }

    #[must_use]
    pub fn with_variables<I, K, V>(mut self, variables: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<Value>,
    {
        self.variables.get_or_insert_with(BTreeMap::new).extend(
            variables
                .into_iter()
                .map(|(key, value)| (key.into(), value.into())),
        );
        self
    }
}
