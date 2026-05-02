use std::collections::BTreeMap;

use serde::Deserialize;
use url::Url;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub transports: BTreeMap<String, TransportConfig>,
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
