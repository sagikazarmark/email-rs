//! Property-based coverage for the `IdempotencyKey` / `CorrelationId`
//! string-newtype validator.
//!
//! The validator's inputs come from queue payloads, HTTP headers, and worker
//! options that may originate outside the binary. The unit tests in
//! `transport.rs` covers the documented rejection cases by example;
//! these properties make sure no surprise input slips through in either
//! direction.

use email_transport::{
    CorrelationId, IdempotencyKey, STRING_NEWTYPE_MAX_BYTES, StringNewtypeError,
};
use proptest::prelude::*;

/// Strategy: ASCII printable strings (0x20..=0x7E) of length 1..=64. Every
/// byte in this range is unambiguously *valid* per the validator.
fn valid_printable() -> impl Strategy<Value = String> {
    proptest::collection::vec(0x20u8..=0x7e, 1..=64)
        .prop_map(|bytes| String::from_utf8(bytes).expect("ASCII printable is valid UTF-8"))
}

proptest! {
    /// Any printable ASCII string of length 1..=64 must accept and round-trip.
    #[test]
    fn idempotency_key_accepts_printable_ascii(value in valid_printable()) {
        let key = IdempotencyKey::new(value.clone()).expect("printable ASCII must be valid");
        prop_assert_eq!(key.as_str(), value.as_str());
    }

    /// Any input that starts with a CR or LF byte must be rejected with the
    /// `Newline` variant, closes the header-injection seam.
    #[test]
    fn idempotency_key_rejects_any_crlf(
        prefix in proptest::collection::vec(0x20u8..=0x7e, 0..=16),
        injection in prop_oneof![Just(b"\r\n".to_vec()), Just(b"\n".to_vec()), Just(b"\r".to_vec())],
        suffix in proptest::collection::vec(0x20u8..=0x7e, 0..=16),
    ) {
        let mut bytes = prefix;
        bytes.extend_from_slice(&injection);
        bytes.extend(suffix);
        let value = String::from_utf8(bytes).expect("ASCII printable + CRLF is valid UTF-8");
        let err = IdempotencyKey::new(value).unwrap_err();
        prop_assert_eq!(err, StringNewtypeError::Newline);
    }

    /// Any input containing a NUL byte must be rejected with `Nul`.
    #[test]
    fn idempotency_key_rejects_any_nul(
        prefix in proptest::collection::vec(0x20u8..=0x7e, 0..=16),
        suffix in proptest::collection::vec(0x20u8..=0x7e, 0..=16),
    ) {
        let mut bytes = prefix;
        bytes.push(0);
        bytes.extend(suffix);
        let value = String::from_utf8(bytes).expect("ASCII printable + NUL is valid UTF-8");
        let err = IdempotencyKey::new(value).unwrap_err();
        prop_assert_eq!(err, StringNewtypeError::Nul);
    }

    /// Any non-tab control byte (0x01..=0x1F minus tab/CR/LF, plus 0x7F) is
    /// rejected with `Control`. Tab is explicitly allowed.
    #[test]
    fn idempotency_key_rejects_non_tab_controls(
        ctrl in prop_oneof![
            (0x01u8..=0x08).prop_map(|b| b),
            (0x0bu8..=0x0c).prop_map(|b| b),
            (0x0eu8..=0x1f).prop_map(|b| b),
            Just(0x7fu8),
        ],
    ) {
        let bytes = vec![b'a', ctrl, b'b'];
        let value = String::from_utf8(bytes).expect("ASCII control is valid UTF-8");
        let err = IdempotencyKey::new(value).unwrap_err();
        prop_assert_eq!(err, StringNewtypeError::Control);
    }

    /// Tab survives, it's the single allowed control byte.
    #[test]
    fn idempotency_key_accepts_tab(prefix in "[a-z]{1,8}", suffix in "[a-z]{1,8}") {
        let value = format!("{prefix}\t{suffix}");
        let key = IdempotencyKey::new(value.clone()).expect("tab must be allowed");
        prop_assert_eq!(key.as_str(), value.as_str());
    }

    /// Anything strictly longer than `STRING_NEWTYPE_MAX_BYTES` is rejected
    /// with `TooLong { len, max }` carrying the actual byte length.
    #[test]
    fn idempotency_key_rejects_oversize(extra in 1usize..=64) {
        let len = STRING_NEWTYPE_MAX_BYTES + extra;
        let value = "x".repeat(len);
        let err = IdempotencyKey::new(value).unwrap_err();
        match err {
            StringNewtypeError::TooLong { len: got_len, max, .. } => {
                prop_assert_eq!(got_len, len);
                prop_assert_eq!(max, STRING_NEWTYPE_MAX_BYTES);
            }
            other => prop_assert!(false, "expected TooLong, got {:?}", other),
        }
    }

    /// The `STRING_NEWTYPE_MAX_BYTES`-byte boundary is *inclusive*.
    #[test]
    fn idempotency_key_accepts_exact_max(_unit in 0u8..1) {
        let value = "x".repeat(STRING_NEWTYPE_MAX_BYTES);
        let key = IdempotencyKey::new(value).expect("max-length input must be accepted");
        prop_assert_eq!(key.as_str().len(), STRING_NEWTYPE_MAX_BYTES);
    }

    /// CorrelationId shares the validator with IdempotencyKey, so the same
    /// invariants must hold across the seam.
    #[test]
    fn correlation_id_rejects_crlf(prefix in "[a-z]{1,16}", suffix in "[a-z]{1,16}") {
        let value = format!("{prefix}\r\n{suffix}");
        let err = CorrelationId::new(value).unwrap_err();
        prop_assert_eq!(err, StringNewtypeError::Newline);
    }

    /// Serde round-trips for any printable-ASCII value: `serialize` then
    /// `deserialize` returns the same `CorrelationId`.
    #[test]
    #[cfg(feature = "serde")]
    fn correlation_id_serde_roundtrip(value in valid_printable()) {
        let id = CorrelationId::new(value).expect("printable ASCII is valid");
        let json = serde_json::to_string(&id).expect("serialize");
        let back: CorrelationId = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(back, id);
    }
}
