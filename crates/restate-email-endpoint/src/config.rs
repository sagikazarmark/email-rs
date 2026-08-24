use std::collections::BTreeMap;

use serde::Deserialize;
use url::Url;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub transports: BTreeMap<String, TransportConfig>,
    pub attachments: Option<AttachmentConfig>,
    /// Restate request identity public keys (`publickeyv1_...`).
    ///
    /// With at least one key configured the endpoint rejects unsigned
    /// requests. Listing the old and the new key keeps both valid during
    /// rotation. Accepts a list or a comma/whitespace-delimited string, so
    /// the `RESTATE_EMAIL_IDENTITY_KEYS` environment override stays a plain
    /// string.
    #[serde(deserialize_with = "identity_keys")]
    pub identity_keys: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct AttachmentConfig {
    pub max_attachment_bytes: Option<usize>,
    pub max_total_bytes: Option<usize>,
    // Without `attachment-opendal` no resolver variant exists and the value
    // type is empty; the field stays so configured resolvers fail loudly.
    #[cfg_attr(
        not(feature = "attachment-opendal"),
        allow(clippy::zero_sized_map_values)
    )]
    pub resolvers: BTreeMap<String, ResolverConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ResolverConfig {
    #[cfg(feature = "attachment-opendal")]
    Opendal {
        service: String,
        #[serde(flatten)]
        options: BTreeMap<String, String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "provider", rename_all = "kebab-case")]
pub enum TransportConfig {
    #[cfg(feature = "transport-resend")]
    Resend {
        api_key: String,
        #[serde(default)]
        base_url: Option<Url>,
    },
    /// SMTP delivery through Lettre.
    ///
    /// The URL carries credentials, host, port, and TLS mode as documented by
    /// Lettre's `AsyncSmtpTransport::from_url` (`smtp://` or `smtps://`).
    #[cfg(feature = "transport-lettre")]
    Smtp { url: Url },
}

impl TransportConfig {
    pub const fn provider_name(&self) -> &'static str {
        match self {
            #[cfg(feature = "transport-resend")]
            Self::Resend { .. } => "resend",
            #[cfg(feature = "transport-lettre")]
            Self::Smtp { .. } => "smtp",
        }
    }
}

fn identity_keys<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum IdentityKeys {
        List(Vec<String>),
        Delimited(String),
    }

    Ok(match IdentityKeys::deserialize(deserializer)? {
        IdentityKeys::List(keys) => keys,
        IdentityKeys::Delimited(keys) => keys
            .split([',', ' ', '\t', '\n'])
            .filter(|key| !key.is_empty())
            .map(str::to_owned)
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use figment::Figment;
    use figment::providers::{Format, Toml};

    use super::*;

    #[cfg(feature = "attachment-opendal")]
    #[test]
    fn parses_optional_attachment_preparation() {
        let config: Config = Figment::from(Toml::string(
            r#"
            [attachments]
            max_attachment_bytes = 26214400
            max_total_bytes = 41943040

            [attachments.resolvers.docs]
            type = "opendal"
            service = "s3"
            bucket = "example-docs"
            root = "/outbound"
            "#,
        ))
        .extract()
        .expect("configuration should parse");

        let attachments = config
            .attachments
            .expect("attachment preparation should be configured");
        assert_eq!(attachments.max_attachment_bytes, Some(26_214_400));
        assert_eq!(attachments.max_total_bytes, Some(41_943_040));
        let ResolverConfig::Opendal { service, options } = attachments
            .resolvers
            .get("docs")
            .expect("docs resolver should be registered");
        assert_eq!(service, "s3");
        assert_eq!(
            options.get("bucket").map(String::as_str),
            Some("example-docs")
        );
        assert_eq!(options.get("root").map(String::as_str), Some("/outbound"));
    }

    #[test]
    fn attachment_preparation_is_absent_by_default() {
        let config: Config = Figment::from(Toml::string(""))
            .extract()
            .expect("configuration should parse");

        assert!(config.attachments.is_none());
        assert!(config.identity_keys.is_empty());
    }

    #[cfg(feature = "transport-resend")]
    #[test]
    fn parses_resend_transport() {
        let config: Config = Figment::from(Toml::string(
            r#"
            [transports.transactional]
            provider = "resend"
            api_key = "test-key"
            "#,
        ))
        .extract()
        .expect("configuration should parse");

        let Some(TransportConfig::Resend { api_key, base_url }) =
            config.transports.get("transactional")
        else {
            panic!("transactional transport should be Resend");
        };
        assert_eq!(api_key, "test-key");
        assert!(base_url.is_none());
    }

    #[cfg(feature = "transport-lettre")]
    #[test]
    fn parses_smtp_transport() {
        let config: Config = Figment::from(Toml::string(
            r#"
            [transports.transactional]
            provider = "smtp"
            url = "smtp://mailpit:1025"
            "#,
        ))
        .extract()
        .expect("configuration should parse");

        let Some(TransportConfig::Smtp { url }) = config.transports.get("transactional") else {
            panic!("transactional transport should be SMTP");
        };
        assert_eq!(url.as_str(), "smtp://mailpit:1025");
    }

    #[test]
    fn parses_identity_keys_from_list_and_delimited_string() {
        let list: Config = Figment::from(Toml::string(
            r#"
            identity_keys = ["publickeyv1_old", "publickeyv1_new"]
            "#,
        ))
        .extract()
        .expect("configuration should parse");
        let delimited: Config = Figment::from(Toml::string(
            r#"
            identity_keys = "publickeyv1_old, publickeyv1_new"
            "#,
        ))
        .extract()
        .expect("configuration should parse");

        assert_eq!(list.identity_keys, ["publickeyv1_old", "publickeyv1_new"]);
        assert_eq!(delimited.identity_keys, list.identity_keys);
    }
}
