//! First-party test transports for fixture-friendly tests.
//!
//! Both transports implement the full `Transport` + `RawTransport` contracts
//! so they can be swapped in anywhere a real provider adapter would otherwise
//! go. They avoid any runtime, HTTP, or provider-specific dependencies.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use email_message::{Envelope, Message, OutboundMessage};
use email_message_wire::render_rfc822;

use email_transport::{
    Capabilities, ErrorKind, MaybeSend, RawTransport, SendOptions, SendReport,
    StructuredSendCapability, Transport, TransportError, structured_accepted_for,
};

/// A transport that captures every send in memory without contacting a real
/// provider. Use it in tests to assert what messages would have been sent.
///
/// The transport is cheap to clone: all clones share the same captured-sends
/// log so an application can hand one clone to the production code under test
/// and keep another clone for assertions.
#[derive(Clone, Default)]
pub struct MemoryTransport {
    sends: Arc<Mutex<Vec<CapturedSend>>>,
    provider_message_id: Option<String>,
}

impl MemoryTransport {
    /// Construct an empty in-memory transport.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the `provider_message_id` attached to every returned [`SendReport`].
    #[must_use]
    pub fn with_provider_message_id(mut self, id: impl Into<String>) -> Self {
        self.provider_message_id = Some(id.into());
        self
    }

    /// Returns a snapshot of all captured sends so far.
    #[must_use]
    pub fn captured(&self) -> Vec<CapturedSend> {
        self.lock_sends().clone()
    }

    /// Returns the number of captured sends without cloning the log.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lock_sends().len()
    }

    /// Returns `true` if no sends have been captured yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Clears the captured-sends log.
    pub fn clear(&self) {
        self.lock_sends().clear();
    }

    fn push(&self, captured: CapturedSend) {
        self.lock_sends().push(captured);
    }

    fn lock_sends(&self) -> MutexGuard<'_, Vec<CapturedSend>> {
        self.sends.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn build_report(&self, accepted: Vec<email_message::EmailAddress>) -> SendReport {
        let mut report = SendReport::new("memory").with_accepted(accepted);
        if let Some(id) = &self.provider_message_id {
            report = report.with_provider_message_id(id.clone());
        }
        report
    }
}

impl Transport for MemoryTransport {
    fn capabilities(&self) -> Capabilities {
        Capabilities::new()
            .with_structured_send(StructuredSendCapability::Supported)
            .with_custom_envelope(true)
            .with_custom_headers(true)
            .with_attachments(true)
            .with_inline_attachments(true)
            .with_idempotency_key(true)
            .with_timeout(true)
    }

    fn send<'a>(
        &'a self,
        message: &'a OutboundMessage,
        options: &'a SendOptions,
    ) -> impl core::future::Future<Output = Result<SendReport, TransportError>> + MaybeSend + 'a
    {
        let this = self.clone();
        async move {
            let message = message.as_message().clone();
            let envelope = options.envelope.clone();
            let timeout = options.timeout;
            let idempotency_key = options.idempotency_key.clone();
            let correlation_id = options.correlation_id.clone();
            let accepted =
                structured_accepted_for(&message, options, Transport::capabilities(&this));
            this.push(CapturedSend {
                payload: CapturedPayload::Structured {
                    envelope,
                    message: Box::new(message),
                },
                timeout,
                idempotency_key,
                correlation_id,
            });
            Ok(this.build_report(accepted))
        }
    }
}

impl RawTransport for MemoryTransport {
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
        let this = self.clone();
        async move {
            let envelope = envelope.clone();
            let rfc822 = rfc822.to_vec();
            let timeout = options.timeout;
            let idempotency_key = options.idempotency_key.clone();
            let correlation_id = options.correlation_id.clone();
            let accepted = envelope.rcpt_to().to_vec();
            this.push(CapturedSend {
                payload: CapturedPayload::Raw { envelope, rfc822 },
                timeout,
                idempotency_key,
                correlation_id,
            });
            Ok(this.build_report(accepted))
        }
    }
}

/// A transport that renders every send to disk as an RFC822 `.eml` file.
///
/// Files are written to the configured directory with monotonically
/// increasing names (`message-0001.eml`, `message-0002.eml`, …). Existing
/// paths are skipped rather than overwritten. Intended for integration tests
/// that want to inspect the exact bytes that would have been sent.
pub struct FileTransport {
    dir: PathBuf,
    next_index: Mutex<usize>,
}

impl FileTransport {
    /// Construct a [`FileTransport`] that writes to `dir`. The directory is
    /// created if it does not exist.
    ///
    /// # Errors
    ///
    /// Returns an [`io::Error`] if the directory cannot be created.
    pub fn new(dir: impl Into<PathBuf>) -> io::Result<Self> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            next_index: Mutex::new(1),
        })
    }

    /// Returns the directory this transport writes to.
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.dir
    }

    fn next_path(&self) -> PathBuf {
        let mut index = self
            .next_index
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let path = self.dir.join(format!("message-{:04}.eml", *index));
        *index += 1;
        path
    }

    fn create_next_file(&self) -> Result<(PathBuf, std::fs::File), TransportError> {
        loop {
            let path = self.next_path();
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => return Ok((path, file)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(map_io_error(error)),
            }
        }
    }

    fn write(&self, bytes: &[u8]) -> Result<PathBuf, TransportError> {
        let (path, mut file) = self.create_next_file()?;
        file.write_all(bytes).map_err(map_io_error)?;
        Ok(path)
    }
}

impl Transport for FileTransport {
    fn capabilities(&self) -> Capabilities {
        // `Transport::capabilities` advertises only the flags this
        // trait's methods can support; `raw_rfc822` is meaningful on
        // `RawTransport::capabilities` and is set there separately
        // (see the `RawTransport for FileTransport` impl below).
        Capabilities::new()
            .with_structured_send(StructuredSendCapability::Supported)
            .with_custom_envelope(true)
            .with_custom_headers(true)
            .with_attachments(true)
            .with_inline_attachments(true)
    }

    async fn send(
        &self,
        message: &OutboundMessage,
        options: &SendOptions,
    ) -> Result<SendReport, TransportError> {
        let inner = message.as_message();
        let rfc822 = render_rfc822(inner)
            .map_err(|error| TransportError::new(ErrorKind::Validation, error.to_string()))?;
        let path = self.write(&rfc822)?;
        let accepted = structured_accepted_for(inner, options, Transport::capabilities(self));
        Ok(build_file_report(accepted, &path))
    }
}

impl RawTransport for FileTransport {
    fn capabilities(&self) -> Capabilities {
        Capabilities::new()
            .with_raw_rfc822(true)
            .with_custom_envelope(true)
    }

    async fn send_raw(
        &self,
        envelope: &Envelope,
        rfc822: &[u8],
        _options: &SendOptions,
    ) -> Result<SendReport, TransportError> {
        let path = self.write(rfc822)?;
        let accepted = envelope.rcpt_to().to_vec();
        Ok(build_file_report(accepted, &path))
    }
}

/// One send captured by [`MemoryTransport`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct CapturedSend {
    /// The captured structured or raw payload.
    pub payload: CapturedPayload,
    /// The timeout from the send options, if set.
    pub timeout: Option<std::time::Duration>,
    /// The idempotency key from the send options, if set.
    pub idempotency_key: Option<email_transport::IdempotencyKey>,
    /// The correlation ID from the send options, if set.
    pub correlation_id: Option<email_transport::CorrelationId>,
}

/// Structured or raw payload captured by [`MemoryTransport`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum CapturedPayload {
    /// A structured send through [`Transport::send`].
    Structured {
        /// Optional custom envelope from [`SendOptions::envelope`].
        envelope: Option<Envelope>,
        /// The structured message passed to the transport.
        message: Box<Message>,
    },
    /// A raw RFC 822 send through [`RawTransport::send_raw`].
    Raw {
        /// The envelope passed to the raw transport.
        envelope: Envelope,
        /// The raw RFC 822 bytes passed to the transport.
        rfc822: Vec<u8>,
    },
}

fn map_io_error(error: io::Error) -> TransportError {
    let message = error.to_string();
    TransportError::new(ErrorKind::Internal, message).with_source(error)
}

fn build_file_report(accepted: Vec<email_message::EmailAddress>, path: &Path) -> SendReport {
    SendReport::new("file")
        .with_provider_message_id(path.display().to_string())
        .with_accepted(accepted)
}
