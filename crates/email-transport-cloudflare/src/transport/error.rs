use email_transport::{ErrorKind, TransportError};

/// Map a failure thrown by the `send_email` binding to a classified
/// [`TransportError`].
///
/// `code` is Cloudflare's machine-readable error code (the JS error's `code`
/// property) and is carried verbatim as
/// [`TransportError::provider_error_code`]. A failure without a code is a
/// programming error at the binding boundary rather than a provider response
/// and is classified as [`ErrorKind::Internal`]. There is no HTTP hop, so
/// [`TransportError::http_status`] is never set.
pub(super) fn map_binding_error(code: Option<&str>, message: String) -> TransportError {
    match code {
        Some(code) => TransportError::new(classify(code), message).with_provider_error_code(code),
        None => TransportError::new(ErrorKind::Internal, message),
    }
}

/// Classify a Cloudflare error code.
///
/// Unrecognised codes fail safe as [`ErrorKind::PermanentProvider`] so a new
/// platform code is never retried forever, mirroring the kernel's HTTP-status
/// default.
fn classify(code: &str) -> ErrorKind {
    match code {
        "E_VALIDATION_ERROR"
        | "E_FIELD_MISSING"
        | "E_TOO_MANY_RECIPIENTS"
        | "E_TOO_MANY_ATTACHMENTS"
        | "E_CONTENT_TOO_LARGE" => ErrorKind::Validation,
        "E_SENDER_NOT_VERIFIED"
        | "E_SENDER_DOMAIN_NOT_AVAILABLE"
        | "E_RECIPIENT_NOT_ALLOWED"
        | "RCPT_NOT_ALLOWED" => ErrorKind::Authorization,
        "E_RECIPIENT_SUPPRESSED" => ErrorKind::PermanentProvider,
        "E_RATE_LIMIT_EXCEEDED" | "E_DAILY_LIMIT_EXCEEDED" => ErrorKind::RateLimited,
        "E_INTERNAL_SERVER_ERROR" | "E_DELIVERY_FAILED" => ErrorKind::TransientProvider,
        // `E_HEADER_*` and `E_HEADERS_*` are all header validation failures.
        _ if code.starts_with("E_HEADER") => ErrorKind::Validation,
        _ => ErrorKind::PermanentProvider,
    }
}
