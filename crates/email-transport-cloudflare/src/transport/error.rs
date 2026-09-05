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
#[cfg_attr(
    not(target_arch = "wasm32"),
    allow(
        dead_code,
        reason = "called only by the wasm32 binding glue; tested on every target"
    )
)]
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
        // `E_DELIVERY_FAILED` is the recipient MTA's verdict on a first
        // attempt; the platform retries soft bounces itself, so what surfaces
        // is a hard bounce, and the binding is a non-atomic multi-recipient
        // call with no idempotency key. See ADR 0005.
        "E_RECIPIENT_SUPPRESSED" | "E_DELIVERY_FAILED" => ErrorKind::PermanentProvider,
        "E_RATE_LIMIT_EXCEEDED" | "E_DAILY_LIMIT_EXCEEDED" => ErrorKind::RateLimited,
        "E_INTERNAL_SERVER_ERROR" => ErrorKind::TransientProvider,
        // `E_HEADER_*` and `E_HEADERS_*` are all header validation failures.
        _ if code.starts_with("E_HEADER") => ErrorKind::Validation,
        _ => ErrorKind::PermanentProvider,
    }
}

#[cfg(test)]
mod tests {
    use email_transport::ErrorKind;

    use super::map_binding_error;

    #[test]
    fn codes_are_classified_by_table() {
        let cases: &[(&str, ErrorKind, bool)] = &[
            ("E_VALIDATION_ERROR", ErrorKind::Validation, false),
            ("E_FIELD_MISSING", ErrorKind::Validation, false),
            ("E_TOO_MANY_RECIPIENTS", ErrorKind::Validation, false),
            ("E_TOO_MANY_ATTACHMENTS", ErrorKind::Validation, false),
            ("E_CONTENT_TOO_LARGE", ErrorKind::Validation, false),
            ("E_HEADER_NOT_ALLOWED", ErrorKind::Validation, false),
            ("E_HEADER_USE_API_FIELD", ErrorKind::Validation, false),
            ("E_HEADER_VALUE_INVALID", ErrorKind::Validation, false),
            ("E_HEADER_VALUE_TOO_LONG", ErrorKind::Validation, false),
            ("E_HEADER_NAME_INVALID", ErrorKind::Validation, false),
            ("E_HEADERS_TOO_LARGE", ErrorKind::Validation, false),
            ("E_HEADERS_TOO_MANY", ErrorKind::Validation, false),
            ("E_SENDER_NOT_VERIFIED", ErrorKind::Authorization, false),
            (
                "E_SENDER_DOMAIN_NOT_AVAILABLE",
                ErrorKind::Authorization,
                false,
            ),
            ("E_RECIPIENT_NOT_ALLOWED", ErrorKind::Authorization, false),
            ("RCPT_NOT_ALLOWED", ErrorKind::Authorization, false),
            (
                "E_RECIPIENT_SUPPRESSED",
                ErrorKind::PermanentProvider,
                false,
            ),
            ("E_RATE_LIMIT_EXCEEDED", ErrorKind::RateLimited, true),
            ("E_DAILY_LIMIT_EXCEEDED", ErrorKind::RateLimited, true),
            (
                "E_INTERNAL_SERVER_ERROR",
                ErrorKind::TransientProvider,
                true,
            ),
            ("E_DELIVERY_FAILED", ErrorKind::PermanentProvider, false),
            ("E_SOMETHING_NEW", ErrorKind::PermanentProvider, false),
        ];

        for (code, kind, retryable) in cases {
            let error = map_binding_error(Some(code), String::from("platform says no"));

            assert_eq!(error.kind, *kind, "{code}: kind");
            assert_eq!(error.is_retryable(), *retryable, "{code}: retryable");
            assert_eq!(
                error.provider_error_code.as_deref(),
                Some(*code),
                "{code}: code"
            );
            assert_eq!(error.message, "platform says no", "{code}: message");
            assert_eq!(error.http_status, None, "{code}: no http hop");
        }
    }

    #[test]
    fn missing_code_is_internal() {
        let error = map_binding_error(None, String::from("TypeError: boom"));

        assert_eq!(error.kind, ErrorKind::Internal);
        assert_eq!(error.provider_error_code, None);
        assert_eq!(error.message, "TypeError: boom");
        assert!(!error.is_retryable());
    }
}
