//! `tracing` instrumentation for transport sends.
//!
//! [`TracingTransport`] wraps any [`Transport`] or [`RawTransport`] and emits
//! PII-safe spans/events around each send. It records transport metadata such
//! as provider, message ID, correlation ID, error kind, and latency; it does
//! not log message bodies, subject lines, or recipient addresses.

use core::future::Future;
use core::time::Duration;

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
use std::time::Instant;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use web_time::Instant;

use ::tracing::{Instrument, Span, field};
use email_message::{Envelope, OutboundMessage};

use crate::{
    Capabilities, MaybeSend, RawTransport, SendOptions, SendReport, Transport, TransportError,
};

/// Transport wrapper that emits `tracing` spans/events around send attempts.
///
/// The wrapper preserves the wrapped transport's behavior and capabilities. It
/// intentionally logs only transport-level metadata by default; message body,
/// subject, and recipient addresses are omitted to avoid accidental PII leaks.
pub struct TracingTransport<T> {
    inner: T,
}

impl<T> TracingTransport<T> {
    /// Wrap a transport with `tracing` instrumentation.
    #[must_use]
    pub const fn new(inner: T) -> Self {
        Self { inner }
    }

    /// Return the wrapped transport.
    #[must_use]
    pub const fn inner(&self) -> &T {
        &self.inner
    }

    /// Unwrap and return the wrapped transport.
    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T> Clone for TracingTransport<T>
where
    T: Clone,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> Transport for TracingTransport<T>
where
    T: Transport,
{
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }

    fn send<'a>(
        &'a self,
        message: &'a OutboundMessage,
        options: &'a SendOptions,
    ) -> impl Future<Output = Result<SendReport, TransportError>> + MaybeSend + 'a {
        trace_send(SendKind::Structured, options, move || {
            self.inner.send(message, options)
        })
    }

    fn send_owned<'a>(
        &'a self,
        message: OutboundMessage,
        options: &'a SendOptions,
    ) -> impl Future<Output = Result<SendReport, TransportError>> + MaybeSend + 'a {
        trace_send(SendKind::Structured, options, move || {
            self.inner.send_owned(message, options)
        })
    }
}

impl<T> RawTransport for TracingTransport<T>
where
    T: RawTransport,
{
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }

    fn send_raw<'a>(
        &'a self,
        envelope: &'a Envelope,
        rfc822: &'a [u8],
        options: &'a SendOptions,
    ) -> impl Future<Output = Result<SendReport, TransportError>> + MaybeSend + 'a {
        trace_send(SendKind::Raw, options, move || {
            self.inner.send_raw(envelope, rfc822, options)
        })
    }

    fn send_raw_owned<'a>(
        &'a self,
        envelope: Envelope,
        rfc822: Vec<u8>,
        options: &'a SendOptions,
    ) -> impl Future<Output = Result<SendReport, TransportError>> + MaybeSend + 'a {
        trace_send(SendKind::Raw, options, move || {
            self.inner.send_raw_owned(envelope, rfc822, options)
        })
    }
}

fn trace_send<'a, F, Fut>(
    kind: SendKind,
    options: &'a SendOptions,
    call: F,
) -> impl Future<Output = Result<SendReport, TransportError>> + MaybeSend + 'a
where
    F: FnOnce() -> Fut + MaybeSend + 'a,
    Fut: Future<Output = Result<SendReport, TransportError>> + MaybeSend + 'a,
{
    let span = send_span(kind, options);
    async move {
        let started_at = StartTime::now();
        let mut cancel_guard = CancelGuard::new(Span::current(), kind, started_at);
        ::tracing::debug!(kind = kind.as_str(), "email send started");

        match call().await {
            Ok(report) => {
                record_success(kind, started_at, &report);
                cancel_guard.disarm();
                Ok(report)
            }
            Err(error) => {
                record_failure(kind, started_at, &error);
                cancel_guard.disarm();
                Err(error)
            }
        }
    }
    .instrument(span)
}

fn send_span(kind: SendKind, options: &SendOptions) -> Span {
    let span = ::tracing::info_span!(
        "email.send",
        kind = kind.as_str(),
        correlation_id = field::Empty,
        provider = field::Empty,
        provider_message_id = field::Empty,
        error_kind = field::Empty,
        http_status = field::Empty,
        provider_error_code = field::Empty,
        request_id = field::Empty,
        retryable = field::Empty,
    );

    if let Some(correlation_id) = options.correlation_id.as_ref() {
        span.record("correlation_id", correlation_id.as_str());
    }

    span
}

fn record_success(kind: SendKind, started_at: StartTime, report: &SendReport) {
    let latency_ms = latency_ms(started_at.elapsed());
    let span = Span::current();
    span.record("provider", field::display(&report.provider));
    if let Some(provider_message_id) = report.provider_message_id.as_deref() {
        span.record("provider_message_id", provider_message_id);
        ::tracing::info!(
            kind = kind.as_str(),
            provider = %report.provider,
            provider_message_id,
            accepted_count = report.accepted.len() as u64,
            latency_ms,
            "email send succeeded",
        );
    } else {
        ::tracing::info!(
            kind = kind.as_str(),
            provider = %report.provider,
            accepted_count = report.accepted.len() as u64,
            latency_ms,
            "email send succeeded",
        );
    }
}

fn record_failure(kind: SendKind, started_at: StartTime, error: &TransportError) {
    let latency_ms = latency_ms(started_at.elapsed());
    let span = Span::current();
    span.record("error_kind", field::debug(&error.kind));
    span.record("retryable", error.is_retryable());
    if let Some(status) = error.http_status {
        span.record("http_status", status);
    }
    if let Some(provider_error_code) = error.provider_error_code.as_deref() {
        span.record("provider_error_code", provider_error_code);
    }
    if let Some(request_id) = error.request_id.as_deref() {
        span.record("request_id", request_id);
    }

    ::tracing::warn!(
        kind = kind.as_str(),
        error_kind = ?error.kind,
        retryable = error.is_retryable(),
        latency_ms,
        "email send failed",
    );
}

#[derive(Clone, Copy)]
enum SendKind {
    Structured,
    Raw,
}

impl SendKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Structured => "structured",
            Self::Raw => "raw",
        }
    }
}

struct CancelGuard {
    span: Span,
    kind: SendKind,
    started_at: StartTime,
    disarmed: bool,
}

impl CancelGuard {
    fn new(span: Span, kind: SendKind, started_at: StartTime) -> Self {
        Self {
            span,
            kind,
            started_at,
            disarmed: false,
        }
    }

    fn disarm(&mut self) {
        self.disarmed = true;
    }
}

impl Drop for CancelGuard {
    fn drop(&mut self) {
        if self.disarmed {
            return;
        }

        let _entered = self.span.enter();
        ::tracing::debug!(
            kind = self.kind.as_str(),
            latency_ms = latency_ms(self.started_at.elapsed()),
            "email send cancelled",
        );
    }
}

#[derive(Clone, Copy)]
struct StartTime {
    inner: Instant,
}

impl StartTime {
    fn now() -> Self {
        Self {
            inner: Instant::now(),
        }
    }

    fn elapsed(self) -> Duration {
        self.inner.elapsed()
    }
}

fn latency_ms(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use email_message::{Address, Body, Message};

    use super::*;
    use crate::{ErrorKind, StructuredSendCapability};

    #[derive(Clone)]
    struct FixedTransport;

    impl Transport for FixedTransport {
        fn capabilities(&self) -> Capabilities {
            Capabilities::new()
                .with_structured_send(StructuredSendCapability::Supported)
                .with_timeout(true)
        }

        async fn send(
            &self,
            _message: &OutboundMessage,
            _options: &SendOptions,
        ) -> Result<SendReport, TransportError> {
            Ok(SendReport::new("fixed").with_provider_message_id("structured-id"))
        }

        async fn send_owned(
            &self,
            _message: OutboundMessage,
            _options: &SendOptions,
        ) -> Result<SendReport, TransportError> {
            Ok(SendReport::new("fixed").with_provider_message_id("owned-id"))
        }
    }

    impl RawTransport for FixedTransport {
        fn capabilities(&self) -> Capabilities {
            Capabilities::new()
                .with_raw_rfc822(true)
                .with_custom_envelope(true)
        }

        async fn send_raw(
            &self,
            envelope: &Envelope,
            _rfc822: &[u8],
            _options: &SendOptions,
        ) -> Result<SendReport, TransportError> {
            Ok(SendReport::new("fixed-raw").with_accepted(envelope.rcpt_to().to_vec()))
        }

        async fn send_raw_owned(
            &self,
            envelope: Envelope,
            _rfc822: Vec<u8>,
            _options: &SendOptions,
        ) -> Result<SendReport, TransportError> {
            Ok(SendReport::new("fixed-raw-owned").with_accepted(envelope.rcpt_to().to_vec()))
        }
    }

    struct FailingTransport;

    impl Transport for FailingTransport {
        async fn send(
            &self,
            _message: &OutboundMessage,
            _options: &SendOptions,
        ) -> Result<SendReport, TransportError> {
            Err(TransportError::new(ErrorKind::TransientProvider, "failed"))
        }
    }

    fn sample_message() -> OutboundMessage {
        Message::builder(Body::text("Hello"))
            .from_mailbox("sender@example.com".parse().expect("from parses"))
            .to(vec![Address::Mailbox(
                "recipient@example.com".parse().expect("to parses"),
            )])
            .subject("Hi")
            .build_outbound()
            .expect("message should validate")
    }

    #[tokio::test]
    async fn tracing_transport_forwards_all_success_routes() {
        let transport = TracingTransport::new(FixedTransport);
        assert_eq!(
            Transport::capabilities(&transport).structured_send,
            StructuredSendCapability::Supported,
        );
        assert!(RawTransport::capabilities(&transport).raw_rfc822);

        let report = transport
            .send(&sample_message(), &SendOptions::default())
            .await
            .expect("send succeeds");
        assert_eq!(report.provider, "fixed");
        assert_eq!(report.provider_message_id.as_deref(), Some("structured-id"));

        let report = transport
            .send_owned(sample_message(), &SendOptions::default())
            .await
            .expect("owned send succeeds");
        assert_eq!(report.provider_message_id.as_deref(), Some("owned-id"));

        let envelope = sample_message()
            .as_message()
            .derive_envelope()
            .expect("envelope derives");
        let report = transport
            .send_raw(
                &envelope,
                b"Subject: Hi\r\n\r\nBody",
                &SendOptions::default(),
            )
            .await
            .expect("raw send succeeds");
        assert_eq!(report.provider, "fixed-raw");
        assert_eq!(report.accepted, envelope.rcpt_to().to_vec());

        let report = transport
            .send_raw_owned(
                envelope.clone(),
                b"Subject: Hi\r\n\r\nBody".to_vec(),
                &SendOptions::default(),
            )
            .await
            .expect("owned raw send succeeds");
        assert_eq!(report.provider, "fixed-raw-owned");
        assert_eq!(report.accepted, envelope.rcpt_to().to_vec());
    }

    #[tokio::test]
    async fn tracing_transport_forwards_failures() {
        let transport = TracingTransport::new(FailingTransport);

        let error = transport
            .send(&sample_message(), &SendOptions::default())
            .await
            .expect_err("send fails");

        assert_eq!(error.kind, ErrorKind::TransientProvider);
        assert_eq!(error.message, "failed");
    }
}
