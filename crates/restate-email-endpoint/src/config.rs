use std::collections::BTreeMap;

use serde::Deserialize;
use url::Url;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub transports: BTreeMap<String, TransportConfig>,
    pub attachments: Option<AttachmentConfig>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct AttachmentConfig {
    pub max_attachment_bytes: Option<usize>,
    pub max_total_bytes: Option<usize>,
    pub resolvers: BTreeMap<String, ResolverConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ResolverConfig {
    Opendal {
        service: String,
        #[serde(flatten)]
        options: BTreeMap<String, String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "provider", rename_all = "kebab-case")]
pub enum TransportConfig {
    Resend {
        api_key: String,
        #[serde(default)]
        base_url: Option<Url>,
    },
}

impl TransportConfig {
    pub const fn provider_name(&self) -> &'static str {
        match self {
            Self::Resend { .. } => "resend",
        }
    }
}

#[cfg(test)]
mod tests {
    use figment::Figment;
    use figment::providers::{Format, Toml};

    use super::*;

    #[test]
    fn parses_optional_attachment_preparation() {
        let config: Config = Figment::from(Toml::string(
            r#"
            [transports.transactional]
            provider = "resend"
            api_key = "test-key"

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
        let config: Config = Figment::from(Toml::string(
            r#"
            [transports.transactional]
            provider = "resend"
            api_key = "test-key"
            "#,
        ))
        .extract()
        .expect("configuration should parse");

        assert!(config.attachments.is_none());
    }
}
