use std::sync::Arc;
use std::time::Duration;

use email_message::{Envelope, OutboundMessage};
use email_message_wire::{MessageRenderError, render_rfc822};
use email_transport::{
    BoxFut, Capabilities, ErrorKind, MaybeSend, RawTransport, SendOptions, SendReport,
    StructuredSendCapability, Transport, TransportError,
};
use lettre::address::Envelope as LettreEnvelope;
use lettre::transport::smtp::{AsyncSmtpTransport, Error as SmtpError};
use lettre::{AsyncTransport as _, Tokio1Executor};

/// SMTP transport backed by Lettre's Tokio client.
///
/// Construct the Lettre client directly when advanced TLS, authentication, or
/// pool configuration is required, then pass it to [`Self::from_client`]. The
/// transport is cheap to clone; clones share both Lettre's connection pool and
/// the private sender adapter. Construct pooled clients and call transport
/// methods from within a Tokio runtime.
#[derive(Clone)]
pub struct LettreTransport {
    sender: Arc<dyn RawSender>,
    in_flight: Arc<tokio::sync::Semaphore>,
}

impl std::fmt::Debug for LettreTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LettreTransport")
            .field("sender", &"<redacted lettre SMTP sender>")
            .finish()
    }
}

impl LettreTransport {
    /// Construct a transport from an initialized Lettre Tokio SMTP client.
    #[must_use]
    pub fn from_client(client: AsyncSmtpTransport<Tokio1Executor>) -> Self {
        Self::from_sender(SmtpSender { client })
    }

    /// Construct a transport from a Lettre SMTP connection URL.
    ///
    /// The URL can include credentials, host, port, EHLO name, and TLS mode as
    /// documented by [`AsyncSmtpTransport::from_url`].
    ///
    /// # Errors
    ///
    /// Returns Lettre's SMTP configuration error when the URL is invalid.
    ///
    /// # Panics
    ///
    /// When the `pool` feature is enabled, call this from within a Tokio
    /// runtime because Lettre starts its pool maintenance task while building
    /// and may panic if no runtime is active.
    #[cfg(any(feature = "native-tls", feature = "rustls-tls"))]
    pub fn from_url(connection_url: &str) -> Result<Self, SmtpError> {
        let client = AsyncSmtpTransport::<Tokio1Executor>::from_url(connection_url)?
            .build::<Tokio1Executor>();
        Ok(Self::from_client(client))
    }

    fn from_sender(sender: impl RawSender + 'static) -> Self {
        Self {
            sender: Arc::new(sender),
            in_flight: Arc::new(tokio::sync::Semaphore::new(MAX_IN_FLIGHT_SENDS)),
        }
    }

    async fn deliver(
        &self,
        envelope: &Envelope,
        rfc822: Vec<u8>,
        timeout: Option<Duration>,
    ) -> Result<SendReport, TransportError> {
        let lettre_envelope = map_envelope(envelope)?;
        let deadline = timeout.map(|timeout| tokio::time::Instant::now() + timeout);
        let permit = if let Some(deadline) = deadline {
            tokio::time::timeout_at(deadline, Arc::clone(&self.in_flight).acquire_owned())
                .await
                .map_err(map_timeout)?
        } else {
            Arc::clone(&self.in_flight).acquire_owned().await
        }
        .map_err(|error| {
            TransportError::new(ErrorKind::Internal, "SMTP send admission failed")
                .with_source(error)
        })?;
        let sender = Arc::clone(&self.sender);

        // Dropping Lettre's send future mid-command can return a desynchronized
        // connection to its pool. Let the SMTP state machine finish in the
        // background when the caller cancels or its waiting budget expires.
        let send = tokio::spawn(async move {
            let _permit = permit;
            sender.send_raw(&lettre_envelope, &rfc822).await
        });
        await_send(send, deadline).await?;

        Ok(SendReport::new("smtp").with_accepted(envelope.rcpt_to().iter().cloned()))
    }
}

impl Transport for LettreTransport {
    fn capabilities(&self) -> Capabilities {
        Capabilities::new()
            .with_structured_send(StructuredSendCapability::Supported)
            .with_custom_envelope(true)
            .with_custom_headers(true)
            .with_attachments(true)
            .with_inline_attachments(true)
            .with_timeout(true)
    }

    async fn send(
        &self,
        message: &OutboundMessage,
        options: &SendOptions,
    ) -> Result<SendReport, TransportError> {
        let message = message.as_message();
        let derived_envelope;
        let envelope = if let Some(envelope) = options.envelope.as_ref() {
            envelope
        } else {
            derived_envelope = message.derive_envelope().map_err(|error| {
                TransportError::new(ErrorKind::Validation, error.to_string()).with_source(error)
            })?;
            &derived_envelope
        };
        let rfc822 = render_rfc822(message).map_err(map_render_error)?;

        self.deliver(envelope, rfc822, options.timeout).await
    }
}

impl RawTransport for LettreTransport {
    fn capabilities(&self) -> Capabilities {
        Capabilities::new()
            .with_raw_rfc822(true)
            .with_custom_envelope(true)
            .with_timeout(true)
    }

    fn send_raw<'a>(
        &'a self,
        envelope: &'a Envelope,
        rfc822: &'a [u8],
        options: &'a SendOptions,
    ) -> impl core::future::Future<Output = Result<SendReport, TransportError>> + MaybeSend + 'a
    {
        self.deliver(envelope, rfc822.to_vec(), options.timeout)
    }
}

const MAX_IN_FLIGHT_SENDS: usize = 64;

trait RawSender: Send + Sync {
    fn send_raw<'a>(
        &'a self,
        envelope: &'a LettreEnvelope,
        rfc822: &'a [u8],
    ) -> BoxFut<'a, Result<(), TransportError>>;
}

struct SmtpSender {
    client: AsyncSmtpTransport<Tokio1Executor>,
}

impl RawSender for SmtpSender {
    fn send_raw<'a>(
        &'a self,
        envelope: &'a LettreEnvelope,
        rfc822: &'a [u8],
    ) -> BoxFut<'a, Result<(), TransportError>> {
        Box::pin(async move {
            self.client
                .send_raw(envelope, rfc822)
                .await
                .map(|_| ())
                .map_err(map_smtp_error)
        })
    }
}

fn map_envelope(envelope: &Envelope) -> Result<LettreEnvelope, TransportError> {
    let mail_from = envelope
        .mail_from()
        .map(|address| address.as_str().parse())
        .transpose()
        .map_err(map_address_error)?;
    let rcpt_to = envelope
        .rcpt_to()
        .iter()
        .map(|address| address.as_str().parse())
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_address_error)?;

    LettreEnvelope::new(mail_from, rcpt_to).map_err(|error| {
        TransportError::new(ErrorKind::Validation, error.to_string()).with_source(error)
    })
}

fn map_address_error(error: lettre::address::AddressError) -> TransportError {
    TransportError::new(ErrorKind::Validation, error.to_string()).with_source(error)
}

fn map_render_error(error: MessageRenderError) -> TransportError {
    let kind = match error {
        MessageRenderError::UnsupportedAttachmentBody | MessageRenderError::UnsupportedBody => {
            ErrorKind::UnsupportedFeature
        }
        _ => ErrorKind::Validation,
    };
    let message = error.to_string();
    TransportError::new(kind, message).with_source(error)
}

fn map_smtp_error(error: SmtpError) -> TransportError {
    let status = error.status().map(u16::from);
    let kind = classify_smtp_error(&error, status);
    let message = error.to_string();
    let mut mapped = TransportError::new(kind, message);
    if let Some(status) = status {
        mapped = mapped.with_provider_error_code(status.to_string());
    }
    mapped.with_source(error)
}

fn classify_smtp_error(error: &SmtpError, status: Option<u16>) -> ErrorKind {
    classify_smtp_facts(SmtpFailureFacts {
        timeout: error.is_timeout(),
        transient: error.is_transient(),
        permanent: error.is_permanent(),
        response: error.is_response(),
        client: error.is_client(),
        shutdown: error.is_transport_shutdown(),
        tls: is_tls_error(error),
        status,
    })
}

#[derive(Clone, Copy, Debug, Default)]
struct SmtpFailureFacts {
    timeout: bool,
    transient: bool,
    permanent: bool,
    response: bool,
    client: bool,
    shutdown: bool,
    tls: bool,
    status: Option<u16>,
}

const fn classify_smtp_facts(facts: SmtpFailureFacts) -> ErrorKind {
    if facts.timeout {
        ErrorKind::Timeout
    } else if matches!(facts.status, Some(535)) {
        ErrorKind::Authentication
    } else if matches!(facts.status, Some(534 | 538)) {
        ErrorKind::UnsupportedFeature
    } else if facts.transient {
        ErrorKind::TransientProvider
    } else if facts.permanent || facts.tls {
        ErrorKind::PermanentProvider
    } else if facts.response {
        ErrorKind::TransientProvider
    } else if facts.client {
        ErrorKind::UnsupportedFeature
    } else if facts.shutdown {
        ErrorKind::Internal
    } else {
        ErrorKind::TransientNetwork
    }
}

#[cfg(any(feature = "native-tls", feature = "rustls-tls"))]
fn is_tls_error(error: &SmtpError) -> bool {
    error.is_tls()
}

#[cfg(not(any(feature = "native-tls", feature = "rustls-tls")))]
const fn is_tls_error(_error: &SmtpError) -> bool {
    false
}

async fn await_send(
    task: tokio::task::JoinHandle<Result<(), TransportError>>,
    deadline: Option<tokio::time::Instant>,
) -> Result<(), TransportError> {
    let result = if let Some(deadline) = deadline {
        tokio::time::timeout_at(deadline, task)
            .await
            .map_err(map_timeout)?
    } else {
        task.await
    };

    result.map_err(|error| {
        TransportError::new(ErrorKind::Internal, "SMTP send task failed").with_source(error)
    })?
}

fn map_timeout(error: tokio::time::error::Elapsed) -> TransportError {
    TransportError::new(ErrorKind::Timeout, "SMTP send timed out").with_source(error)
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;
    use std::sync::Arc;
    use std::time::Duration;

    use email_message::{Address, Attachment, AttachmentReference, Body, ContentType, Envelope};
    use email_transport::{
        ErrorKind, RawTransport, SendOptions, StructuredSendCapability, Transport,
    };
    use email_transport_test::conformance::{EXPECTED_ACCEPTED, conformance_message};
    use lettre::AsyncTransport as _;
    use lettre::transport::stub::AsyncStubTransport;

    #[cfg(any(feature = "native-tls", feature = "rustls-tls"))]
    use super::map_smtp_error;
    use super::{
        LettreEnvelope, LettreTransport, RawSender, SmtpFailureFacts, classify_smtp_facts,
        map_render_error,
    };

    #[derive(Clone)]
    struct StubSender {
        client: AsyncStubTransport,
    }

    impl RawSender for StubSender {
        fn send_raw<'a>(
            &'a self,
            envelope: &'a LettreEnvelope,
            rfc822: &'a [u8],
        ) -> email_transport::BoxFut<'a, Result<(), email_transport::TransportError>> {
            Box::pin(async move {
                self.client
                    .send_raw(envelope, rfc822)
                    .await
                    .map_err(|error| {
                        email_transport::TransportError::new(ErrorKind::Internal, error.to_string())
                            .with_source(error)
                    })
            })
        }
    }

    struct BlockingSender {
        started: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
        completed: Arc<tokio::sync::Notify>,
    }

    impl RawSender for BlockingSender {
        fn send_raw<'a>(
            &'a self,
            _envelope: &'a LettreEnvelope,
            _rfc822: &'a [u8],
        ) -> email_transport::BoxFut<'a, Result<(), email_transport::TransportError>> {
            Box::pin(async move {
                self.started.notify_one();
                self.release.notified().await;
                self.completed.notify_one();
                Ok(())
            })
        }
    }

    fn transport_with_stub(response: bool) -> (LettreTransport, AsyncStubTransport) {
        let client = if response {
            AsyncStubTransport::new_ok()
        } else {
            AsyncStubTransport::new_error()
        };
        let transport = LettreTransport::from_sender(StubSender {
            client: client.clone(),
        });
        (transport, client)
    }

    fn envelope() -> Envelope {
        Envelope::new(
            Some("sender@example.com".parse().expect("sender parses")),
            vec![
                "first@example.com".parse().expect("recipient parses"),
                "second@example.com".parse().expect("recipient parses"),
            ],
        )
    }

    #[test]
    fn capabilities_match_smtp_behavior() {
        let (transport, _) = transport_with_stub(true);

        let structured = Transport::capabilities(&transport);
        assert_eq!(
            structured.structured_send,
            StructuredSendCapability::Supported
        );
        assert!(structured.custom_envelope);
        assert!(structured.custom_headers);
        assert!(structured.attachments);
        assert!(structured.inline_attachments);
        assert!(structured.timeout);
        assert!(!structured.idempotency_key);

        let raw = RawTransport::capabilities(&transport);
        assert!(raw.raw_rfc822);
        assert!(raw.custom_envelope);
        assert!(raw.timeout);
        assert_eq!(raw.structured_send, StructuredSendCapability::Unsupported);
    }

    #[tokio::test]
    async fn raw_send_passes_envelope_and_bytes_to_lettre() {
        let (transport, stub) = transport_with_stub(true);
        let raw = b"Subject: raw\r\n\r\nbody";
        let ignored_envelope = Envelope::new(
            Some("ignored@example.com".parse().expect("sender parses")),
            vec!["ignored@example.com".parse().expect("recipient parses")],
        );
        let options = SendOptions::new().with_envelope(ignored_envelope);

        let report = transport
            .send_raw(&envelope(), raw, &options)
            .await
            .expect("raw send succeeds");

        assert_eq!(report.provider, "smtp");
        assert!(report.provider_message_id.is_none());
        assert_eq!(
            report
                .accepted
                .iter()
                .map(email_message::EmailAddress::as_str)
                .collect::<Vec<_>>(),
            vec!["first@example.com", "second@example.com"]
        );

        let messages = stub.messages().await;
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].0.from().map(ToString::to_string).as_deref(),
            Some("sender@example.com")
        );
        assert_eq!(
            messages[0]
                .0
                .to()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            vec!["first@example.com", "second@example.com"]
        );
        assert_eq!(messages[0].1.as_bytes(), raw);
    }

    #[tokio::test]
    async fn raw_send_supports_null_reverse_path() {
        let (transport, stub) = transport_with_stub(true);
        let envelope = Envelope::new(
            None,
            vec!["recipient@example.com".parse().expect("recipient parses")],
        );

        transport
            .send_raw(&envelope, b"\r\nbody", &SendOptions::default())
            .await
            .expect("raw send succeeds");

        assert!(stub.messages().await[0].0.from().is_none());
    }

    #[tokio::test]
    async fn structured_send_conforms_and_hides_bcc_header() {
        let (transport, stub) = transport_with_stub(true);

        let report = transport
            .send(&conformance_message(), &SendOptions::default())
            .await
            .expect("structured send succeeds");

        assert_eq!(
            report
                .accepted
                .iter()
                .map(email_message::EmailAddress::as_str)
                .collect::<Vec<_>>(),
            EXPECTED_ACCEPTED
        );
        let messages = stub.messages().await;
        let (envelope, rfc822) = &messages[0];
        assert_eq!(
            envelope.from().map(ToString::to_string).as_deref(),
            Some("bounce@example.com")
        );
        assert_eq!(
            envelope
                .to()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            EXPECTED_ACCEPTED
        );
        assert!(rfc822.contains("X-Test: demo\r\n"));
        assert!(rfc822.contains("Message-ID: <conformance@example.com>\r\n"));
        assert!(!rfc822.contains("Bcc:"));
    }

    #[tokio::test]
    async fn structured_send_honors_custom_envelope() {
        let (transport, stub) = transport_with_stub(true);
        let override_envelope = Envelope::new(
            Some("return@example.com".parse().expect("sender parses")),
            vec!["override@example.com".parse().expect("recipient parses")],
        );
        let options = SendOptions::new().with_envelope(override_envelope);

        let report = transport
            .send(&conformance_message(), &options)
            .await
            .expect("structured send succeeds");

        assert_eq!(report.accepted[0].as_str(), "override@example.com");
        let messages = stub.messages().await;
        assert_eq!(
            messages[0].0.from().map(ToString::to_string).as_deref(),
            Some("return@example.com")
        );
        assert_eq!(messages[0].0.to()[0].to_string(), "override@example.com");
        assert!(messages[0].1.contains("a@example.com"));
        assert!(!messages[0].1.contains("override@example.com"));
    }

    #[tokio::test]
    async fn owned_structured_send_matches_borrowed_behavior() {
        let (transport, stub) = transport_with_stub(true);

        let report = transport
            .send_owned(conformance_message(), &SendOptions::default())
            .await
            .expect("owned structured send succeeds");

        assert_eq!(report.accepted.len(), EXPECTED_ACCEPTED.len());
        assert_eq!(stub.messages().await.len(), 1);
    }

    #[tokio::test]
    async fn empty_raw_recipient_list_is_validation_error() {
        let (transport, stub) = transport_with_stub(true);
        let envelope = Envelope::new(None, Vec::new());

        let error = transport
            .send_raw(&envelope, b"body", &SendOptions::default())
            .await
            .expect_err("empty recipient list fails");

        assert_eq!(error.kind, ErrorKind::Validation);
        assert!(stub.messages().await.is_empty());
    }

    #[tokio::test]
    async fn lettre_stub_failure_preserves_source() {
        let (transport, _) = transport_with_stub(false);

        let error = transport
            .send_raw(&envelope(), b"body", &SendOptions::default())
            .await
            .expect_err("stub failure propagates");

        assert_eq!(error.kind, ErrorKind::Internal);
        assert!(error.source().is_some());
    }

    #[cfg(any(feature = "native-tls", feature = "rustls-tls"))]
    #[test]
    fn smtp_configuration_failure_preserves_lettre_source() {
        let smtp_error = lettre::AsyncSmtpTransport::<lettre::Tokio1Executor>::from_url("invalid")
            .expect_err("relative SMTP URL is invalid");

        let error = map_smtp_error(smtp_error);

        assert!(
            error
                .source()
                .and_then(|source| source.downcast_ref::<lettre::transport::smtp::Error>())
                .is_some()
        );
    }

    #[tokio::test]
    async fn per_send_timeout_detaches_in_flight_handoff() {
        let started = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let completed = Arc::new(tokio::sync::Notify::new());
        let transport = LettreTransport::from_sender(BlockingSender {
            started: Arc::clone(&started),
            release: Arc::clone(&release),
            completed: Arc::clone(&completed),
        });
        let options = SendOptions::new().with_timeout(Duration::from_millis(10));

        let error = transport
            .send_raw(&envelope(), b"body", &options)
            .await
            .expect_err("pending send times out");

        assert_eq!(error.kind, ErrorKind::Timeout);
        assert!(error.is_retryable());
        assert!(error.source().is_some());

        started.notified().await;
        release.notify_one();
        tokio::time::timeout(Duration::from_secs(1), completed.notified())
            .await
            .expect("detached handoff completes after timeout");
    }

    #[tokio::test]
    async fn owned_raw_send_matches_borrowed_behavior() {
        let (transport, stub) = transport_with_stub(true);
        let envelope = envelope();
        let raw = b"Subject: owned\r\n\r\nbody".to_vec();

        let report = transport
            .send_raw_owned(envelope, raw.clone(), &SendOptions::default())
            .await
            .expect("owned raw send succeeds");

        assert_eq!(report.accepted.len(), 2);
        assert_eq!(stub.messages().await[0].1.as_bytes(), raw);
    }

    #[tokio::test]
    async fn clones_share_the_same_lettre_stub() {
        let (transport, stub) = transport_with_stub(true);
        let clone = transport.clone();

        transport
            .send_raw(&envelope(), b"first", &SendOptions::default())
            .await
            .expect("first send succeeds");
        clone
            .send_raw(&envelope(), b"second", &SendOptions::default())
            .await
            .expect("second send succeeds");

        assert_eq!(stub.messages().await.len(), 2);
    }

    #[tokio::test]
    async fn debug_redacts_sender_configuration() {
        let client = lettre::AsyncSmtpTransport::<lettre::Tokio1Executor>::builder_dangerous(
            "smtp.secret.example",
        )
        .credentials(lettre::transport::smtp::authentication::Credentials::new(
            String::from("user"),
            String::from("super-secret-password"),
        ))
        .build();
        let transport = LettreTransport::from_client(client);

        let rendered = format!("{transport:?}");
        assert!(rendered.contains("<redacted lettre SMTP sender>"));
        assert!(!rendered.contains("smtp.secret.example"));
        assert!(!rendered.contains("super-secret-password"));
    }

    #[test]
    fn unsupported_render_variants_map_to_unsupported_feature() {
        let message = email_message::Message::builder(Body::text("body"))
            .from_mailbox("sender@example.com".parse().expect("sender parses"))
            .to(vec![Address::Mailbox(
                "recipient@example.com".parse().expect("recipient parses"),
            )])
            .add_attachment(Attachment::reference(
                ContentType::try_from("application/pdf").expect("content type parses"),
                AttachmentReference::new("s3://bucket/key"),
            ))
            .build_outbound()
            .expect("message validates");

        let render_error = email_message_wire::render_rfc822(message.as_message())
            .expect_err("reference cannot render");
        let error = map_render_error(render_error);

        assert_eq!(error.kind, ErrorKind::UnsupportedFeature);
        assert!(error.source().is_some());
    }

    #[test]
    fn smtp_classification_matrix_matches_retry_policy() {
        let cases = [
            (
                classify_smtp_facts(SmtpFailureFacts {
                    timeout: true,
                    transient: true,
                    status: Some(451),
                    ..SmtpFailureFacts::default()
                }),
                ErrorKind::Timeout,
            ),
            (
                classify_smtp_facts(SmtpFailureFacts {
                    permanent: true,
                    status: Some(530),
                    ..SmtpFailureFacts::default()
                }),
                ErrorKind::PermanentProvider,
            ),
            (
                classify_smtp_facts(SmtpFailureFacts {
                    permanent: true,
                    status: Some(535),
                    ..SmtpFailureFacts::default()
                }),
                ErrorKind::Authentication,
            ),
            (
                classify_smtp_facts(SmtpFailureFacts {
                    permanent: true,
                    status: Some(538),
                    ..SmtpFailureFacts::default()
                }),
                ErrorKind::UnsupportedFeature,
            ),
            (
                classify_smtp_facts(SmtpFailureFacts {
                    transient: true,
                    status: Some(451),
                    ..SmtpFailureFacts::default()
                }),
                ErrorKind::TransientProvider,
            ),
            (
                classify_smtp_facts(SmtpFailureFacts {
                    permanent: true,
                    status: Some(550),
                    ..SmtpFailureFacts::default()
                }),
                ErrorKind::PermanentProvider,
            ),
            (
                classify_smtp_facts(SmtpFailureFacts {
                    response: true,
                    ..SmtpFailureFacts::default()
                }),
                ErrorKind::TransientProvider,
            ),
            (
                classify_smtp_facts(SmtpFailureFacts {
                    client: true,
                    ..SmtpFailureFacts::default()
                }),
                ErrorKind::UnsupportedFeature,
            ),
            (
                classify_smtp_facts(SmtpFailureFacts {
                    shutdown: true,
                    ..SmtpFailureFacts::default()
                }),
                ErrorKind::Internal,
            ),
            (
                classify_smtp_facts(SmtpFailureFacts {
                    tls: true,
                    ..SmtpFailureFacts::default()
                }),
                ErrorKind::PermanentProvider,
            ),
            (
                classify_smtp_facts(SmtpFailureFacts::default()),
                ErrorKind::TransientNetwork,
            ),
        ];

        for (actual, expected) in cases {
            assert_eq!(actual, expected);
        }
    }
}
