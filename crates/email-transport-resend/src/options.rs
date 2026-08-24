use std::collections::BTreeMap;

use email_transport::TransportOption;
use serde_json::Value;

/// Resend-specific options for a single send attempt.
///
/// This is the one typed value inserted into [`email_transport::SendOptions::transport_options`]
/// for Resend. The type always implements [`serde::Serialize`] so enabling
/// `email-transport`'s `serde` feature elsewhere does not invalidate typed
/// insertion. Resend's `serde` feature additionally enables deserialization
/// for provider-keyed queue payloads.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[non_exhaustive]
pub struct ResendSendOptions {
    /// Resend dashboard tags attached to the email.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<ResendTag>,
    /// Optional Resend template render settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<ResendTemplate>,
}

impl TransportOption for ResendSendOptions {
    fn provider_key() -> &'static str {
        "resend"
    }
}

impl ResendSendOptions {
    /// Create empty Resend-specific send options.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append one dashboard tag.
    #[must_use]
    pub fn with_tag(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.tags.push(ResendTag::new(name, value));
        self
    }

    /// Append dashboard tags in iteration order.
    #[must_use]
    pub fn with_tags<I, T>(mut self, tags: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<ResendTag>,
    {
        self.tags.extend(tags.into_iter().map(Into::into));
        self
    }

    /// Set the Resend template render settings.
    #[must_use]
    pub fn with_template(mut self, template: ResendTemplate) -> Self {
        self.template = Some(template);
        self
    }

    /// Return whether no tags or template settings are configured.
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
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
pub struct ResendTag {
    /// Tag name used for filtering in Resend.
    pub name: String,
    /// Tag value used for filtering in Resend.
    pub value: String,
}

impl ResendTag {
    /// Create a tag without performing provider-specific validation.
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
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
pub struct ResendTemplate {
    /// Resend template identifier, for example `tmpl_...`.
    pub id: String,
    /// Template variables serialized as a JSON object.
    ///
    /// Stored in a [`BTreeMap`] so queued/provider-option serialization stays
    /// deterministic across processes. Resend's API receives the variables as
    /// a JSON object regardless of map type, so duplicate keys remain
    /// non-representable on the wire by RFC 8259.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variables: Option<BTreeMap<String, Value>>,
}

impl ResendTemplate {
    /// Create template settings without variables.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            variables: None,
        }
    }

    /// Insert or replace one template variable.
    #[must_use]
    pub fn with_variable(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.variables
            .get_or_insert_with(BTreeMap::new)
            .insert(key.into(), value.into());
        self
    }

    /// Insert or replace template variables from an iterator.
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{ResendSendOptions, ResendTemplate};

    #[test]
    fn serialization_omits_empty_optional_values() {
        assert_eq!(
            serde_json::to_value(ResendSendOptions::default()).expect("options should serialize"),
            json!({})
        );
        assert_eq!(
            serde_json::to_value(ResendTemplate::new("tmpl_123"))
                .expect("template should serialize"),
            json!({ "id": "tmpl_123" })
        );
    }
}
