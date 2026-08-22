mod config;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use clap::Parser;
use email_kit::attachment::opendal::OpendalResolver;
use email_kit::attachment::{
    AttachmentResolveError, AttachmentResolver, PreparationLimits, ResolvedAttachment,
    ResolvingTransport, SchemeRouter,
};
use email_kit::message::AttachmentReference;
use email_kit::transport::resend::ResendTransport;
use figment::Figment;
use figment::providers::{Env, Format, Json, Toml, Yaml};
use restate_email::{ServiceImpl, StaticTransportRegistry};
use restate_sdk::{endpoint::Endpoint, http_server::HttpServer, service::IntoServiceDefinition};
use tracing_subscriber::EnvFilter;

use crate::config::{AttachmentConfig, Config, ResolverConfig, TransportConfig};

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

#[derive(Clone)]
struct SharedAttachmentResolver(Arc<SchemeRouter>);

impl AttachmentResolver for SharedAttachmentResolver {
    async fn resolve(
        &self,
        reference: &AttachmentReference,
    ) -> Result<ResolvedAttachment, AttachmentResolveError> {
        self.0.resolve(reference).await
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let config = cli.load_config()?;
    let registry = create_registry(config)?;
    let service = ServiceImpl::new(registry).into_service_definition();
    let endpoint = Endpoint::builder().bind(service);
    let bind_addr = format!("0.0.0.0:{}", cli.port);

    tracing::info!(%bind_addr, "starting Restate email endpoint");

    HttpServer::new(endpoint.build())
        .listen_and_serve(bind_addr.parse()?)
        .await;

    Ok(())
}

fn create_registry(config: Config) -> Result<StaticTransportRegistry> {
    let Config {
        transports,
        attachments,
    } = config;
    if transports.is_empty() {
        bail!("at least one transport must be configured");
    }

    let attachment_preparation = attachments.map(create_attachment_preparation).transpose()?;
    let mut registry = StaticTransportRegistry::new();

    for (key, transport) in transports {
        let provider = transport.provider_name();
        match transport {
            TransportConfig::Resend { api_key, base_url } => {
                let mut builder = ResendTransport::builder(api_key);
                if let Some(base_url) = base_url {
                    builder = builder.base_url(base_url);
                }
                let transport = builder.build();
                if let Some((resolver, limits)) = &attachment_preparation {
                    registry.insert(
                        key.clone(),
                        ResolvingTransport::new(
                            transport,
                            SharedAttachmentResolver(Arc::clone(resolver)),
                        )
                        .with_limits(*limits),
                    );
                } else {
                    registry.insert(key.clone(), transport);
                }
            }
        }

        tracing::info!(transport = %key, provider, "registered email transport");
    }

    Ok(registry)
}

fn create_attachment_preparation(
    config: AttachmentConfig,
) -> Result<(Arc<SchemeRouter>, PreparationLimits)> {
    let limits = PreparationLimits::new()
        .with_max_attachment_bytes(config.max_attachment_bytes)
        .with_max_total_bytes(config.max_total_bytes);
    let resolver_max_bytes = config.max_attachment_bytes.unwrap_or(usize::MAX);
    let mut router = SchemeRouter::new();

    for (scheme, resolver) in config.resolvers {
        match resolver {
            ResolverConfig::Opendal { service, options } => {
                let operator = opendal::Operator::via_iter(&service, options).with_context(|| {
                    format!(
                        "failed to configure attachment resolver `{scheme}` with OpenDAL service `{service}`"
                    )
                })?;
                router.register(&scheme, OpendalResolver::new(operator, resolver_max_bytes));
                tracing::info!(resolver = %scheme, service, "registered attachment resolver");
            }
        }
    }

    Ok((Arc::new(router), limits))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use email_kit::attachment::ResolveErrorKind;
    use email_kit::message::{
        Address, Attachment, AttachmentReference, Body, ContentType, Message,
    };
    use restate_email::{SendOptions, SendRequest, TransportKey};
    use serde_json::json;
    use tempfile::tempdir;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::config::{AttachmentConfig, ResolverConfig};

    fn reference_request(reference: &str) -> SendRequest {
        let message = Message::builder(Body::text("body"))
            .from_mailbox("sender@example.com".parse().expect("sender should parse"))
            .to(vec![Address::Mailbox(
                "recipient@example.com"
                    .parse()
                    .expect("recipient should parse"),
            )])
            .subject("Reference attachment")
            .add_attachment(
                Attachment::reference(
                    ContentType::try_from("application/octet-stream")
                        .expect("content type should parse"),
                    AttachmentReference::new(reference),
                )
                .with_filename("report.bin"),
            )
            .build_outbound()
            .expect("message should validate");

        SendRequest {
            transport: TransportKey::new_unchecked("transactional"),
            message,
            options: SendOptions::default(),
        }
    }

    fn byte_request(bytes: &[u8]) -> SendRequest {
        let message = Message::builder(Body::text("body"))
            .from_mailbox("sender@example.com".parse().expect("sender should parse"))
            .to(vec![Address::Mailbox(
                "recipient@example.com"
                    .parse()
                    .expect("recipient should parse"),
            )])
            .subject("Byte attachment")
            .add_attachment(
                Attachment::bytes(
                    ContentType::try_from("application/octet-stream")
                        .expect("content type should parse"),
                    bytes.to_vec(),
                )
                .with_filename("report.bin"),
            )
            .build_outbound()
            .expect("message should validate");

        SendRequest {
            transport: TransportKey::new_unchecked("transactional"),
            message,
            options: SendOptions::default(),
        }
    }

    fn resend_transport_config(server: &MockServer) -> BTreeMap<String, TransportConfig> {
        BTreeMap::from([(
            String::from("transactional"),
            TransportConfig::Resend {
                api_key: String::from("test-key"),
                base_url: Some(
                    format!("{}/", server.uri())
                        .parse()
                        .expect("mock URL should parse"),
                ),
            },
        )])
    }

    struct TransientResolver;

    impl AttachmentResolver for TransientResolver {
        async fn resolve(
            &self,
            _reference: &AttachmentReference,
        ) -> Result<ResolvedAttachment, AttachmentResolveError> {
            Err(AttachmentResolveError::new(
                ResolveErrorKind::Transient,
                "attachment store is temporarily unavailable",
            ))
        }
    }

    #[tokio::test]
    async fn configured_resolver_delivers_reference_as_bytes() {
        let server = MockServer::start().await;
        let payload: &[u8] = b"resolved attachment";
        Mock::given(method("POST"))
            .and(path("/emails"))
            .and(body_partial_json(json!({
                "attachments": [{
                    "filename": "report.bin",
                    "content": payload,
                    "contentType": "application/octet-stream",
                }]
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "resolved-1"})))
            .mount(&server)
            .await;

        let directory = tempdir().expect("temporary directory should be created");
        std::fs::write(directory.path().join("report.bin"), payload)
            .expect("attachment fixture should be written");
        let mut resolvers = BTreeMap::new();
        resolvers.insert(
            String::from("docs"),
            ResolverConfig::Opendal {
                service: String::from("fs"),
                options: BTreeMap::from([(
                    String::from("root"),
                    directory.path().display().to_string(),
                )]),
            },
        );
        let config = Config {
            transports: resend_transport_config(&server),
            attachments: Some(AttachmentConfig {
                max_attachment_bytes: Some(1024),
                max_total_bytes: Some(2048),
                resolvers,
            }),
        };
        let service = ServiceImpl::new(create_registry(config).expect("registry should build"));

        let response = service
            .send_request(&reference_request("docs:report.bin"))
            .await
            .expect("reference-backed request should send");

        assert_eq!(
            response.report.provider_message_id.as_deref(),
            Some("resolved-1")
        );
    }

    #[tokio::test]
    async fn absent_preparation_preserves_bytes_and_rejects_references_terminally() {
        let server = MockServer::start().await;
        let payload: &[u8] = b"inline attachment";
        Mock::given(method("POST"))
            .and(path("/emails"))
            .and(body_partial_json(json!({
                "attachments": [{
                    "filename": "report.bin",
                    "content": payload,
                    "contentType": "application/octet-stream",
                }]
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "bytes-1"})))
            .expect(1)
            .mount(&server)
            .await;
        let service = ServiceImpl::new(
            create_registry(Config {
                transports: resend_transport_config(&server),
                attachments: None,
            })
            .expect("registry should build"),
        );

        let response = service
            .send_request(&byte_request(payload))
            .await
            .expect("byte-backed request should send");
        assert_eq!(
            response.report.provider_message_id.as_deref(),
            Some("bytes-1")
        );

        let error = service
            .send_request(&reference_request("docs:report.bin"))
            .await
            .expect_err("unresolved reference should fail");
        let source: &(dyn std::error::Error + Send + Sync + 'static) = error.as_ref();
        assert!(
            source.to_string().starts_with("Terminal error [400]"),
            "unsupported references should be terminal: {source}"
        );
    }

    #[tokio::test]
    async fn resolver_failures_keep_the_existing_handler_dispositions() {
        let server = MockServer::start().await;
        let directory = tempdir().expect("temporary directory should be created");
        std::fs::write(directory.path().join("large.bin"), b"five!")
            .expect("attachment fixture should be written");
        let resolvers = BTreeMap::from([(
            String::from("docs"),
            ResolverConfig::Opendal {
                service: String::from("fs"),
                options: BTreeMap::from([(
                    String::from("root"),
                    directory.path().display().to_string(),
                )]),
            },
        )]);
        let service = ServiceImpl::new(
            create_registry(Config {
                transports: resend_transport_config(&server),
                attachments: Some(AttachmentConfig {
                    max_attachment_bytes: Some(4),
                    max_total_bytes: Some(8),
                    resolvers,
                }),
            })
            .expect("registry should build"),
        );

        for reference in ["docs:missing.bin", "unknown:large.bin", "docs:large.bin"] {
            let error = service
                .send_request(&reference_request(reference))
                .await
                .expect_err("resolver failure should fail the send");
            let source: &(dyn std::error::Error + Send + Sync + 'static) = error.as_ref();
            assert!(
                source.to_string().starts_with("Terminal error"),
                "{reference} should fail terminally: {source}"
            );
        }

        let mut registry = StaticTransportRegistry::new();
        registry.insert(
            "transactional",
            ResolvingTransport::new(
                ResendTransport::builder("test-key").build(),
                TransientResolver,
            ),
        );
        let transient_service = ServiceImpl::new(registry);
        let error = transient_service
            .send_request(&reference_request("docs:report.bin"))
            .await
            .expect_err("transient resolver failure should fail the send");
        let source: &(dyn std::error::Error + Send + Sync + 'static) = error.as_ref();
        assert!(
            source.to_string().starts_with("Retryable error"),
            "transient resolver failures should remain retryable: {source}"
        );
    }
}
