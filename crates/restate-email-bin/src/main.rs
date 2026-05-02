mod config;

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use clap::Parser;
use email_kit::transport::resend::ResendTransport;
use figment::Figment;
use figment::providers::{Env, Format, Json, Toml, Yaml};
use restate_email::{ServiceImpl, StaticTransportRegistry};
use restate_sdk::prelude::HttpServer;
use tracing_subscriber::EnvFilter;

use crate::config::{Config, TransportConfig};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let config = cli.load_config()?;
    let registry = create_registry(config.transports)?;
    let service = ServiceImpl::new(registry);
    let bind_addr = format!("0.0.0.0:{}", cli.port);

    tracing::info!(%bind_addr, "starting restate email worker");

    HttpServer::new(service.endpoint())
        .listen_and_serve(bind_addr.parse()?)
        .await;

    Ok(())
}

#[derive(Parser, Debug)]
#[command(version)]
struct Cli {
    /// Path to config file (supports JSON, YAML, or TOML).
    #[arg(long, value_name = "FILE", env = "CONFIG_FILE")]
    config: Option<PathBuf>,

    /// Port to listen on.
    #[arg(long, default_value = "9080", env = "PORT")]
    port: u16,
}

impl Cli {
    fn load_config(&self) -> Result<Config> {
        let mut figment = Figment::new();

        if let Some(path) = self.config.as_deref() {
            if !path.exists() {
                bail!("config file not found: {}", path.display());
            }

            figment = match path.extension().and_then(|extension| extension.to_str()) {
                Some("toml") => figment.merge(Toml::file(path)),
                Some("json") => figment.merge(Json::file(path)),
                Some("yaml" | "yml") => figment.merge(Yaml::file(path)),
                _ => bail!("unsupported config file format; use .toml, .json, .yaml, or .yml"),
            };
        }

        figment = figment.merge(Env::prefixed("RESTATE_EMAIL_").split("__"));

        figment.extract().context("failed to parse configuration")
    }
}

fn create_registry(
    transports: BTreeMap<String, TransportConfig>,
) -> Result<StaticTransportRegistry> {
    if transports.is_empty() {
        bail!("at least one transport must be configured");
    }

    let mut registry = StaticTransportRegistry::new();

    for (key, transport) in transports {
        let provider = transport.provider_name();
        match transport {
            TransportConfig::Resend { api_key, base_url } => {
                let mut builder = ResendTransport::builder(api_key);
                if let Some(base_url) = base_url {
                    builder = builder.base_url(base_url);
                }
                registry.insert(key.clone(), builder.build());
            }
        }

        tracing::info!(transport = %key, provider, "registered email transport");
    }

    Ok(registry)
}
