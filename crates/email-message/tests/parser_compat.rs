use email_message::Mailbox;

#[derive(Clone, Copy)]
struct CompatFixture {
    id: &'static str,
    input: &'static str,
    expected_email: &'static str,
}

// Parser interoperability/normalization quirks.
// These are syntactically valid but may have implementation-specific display-name rendering.
const COMPAT_MAILBOX_FIXTURES: &[CompatFixture] = &[CompatFixture {
    id: "RFC5322-A.1.2-name-addr-comment-heavy-1",
    input: "Pete(A wonderful \\\\) chap) <pete(his account)@silly.test(his host)>",
    expected_email: "pete@silly.test",
}];

#[test]
fn mailbox_parses_complex_comment_forms_for_email_value() {
    for fixture in COMPAT_MAILBOX_FIXTURES {
        let parsed = fixture.input.parse::<Mailbox>();
        assert!(
            parsed.is_ok(),
            "{} expected valid mailbox: {}",
            fixture.id,
            fixture.input
        );
        let parsed = parsed.expect("validated above");
        assert_eq!(
            parsed.email().as_str(),
            fixture.expected_email,
            "{} email mismatch for {}",
            fixture.id,
            fixture.input
        );
    }
}
