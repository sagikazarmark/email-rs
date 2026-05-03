#[cfg(feature = "serde")]
use email_message::Message;
use email_message::{
    Address, AddressList, AddressParseError, EmailAddress, Group, GroupParseError, Mailbox,
    MailboxList, MailboxParseError,
};
#[cfg(feature = "schemars")]
use schemars::schema_for;

#[derive(Clone, Copy)]
struct MailboxFixture {
    id: &'static str,
    input: &'static str,
    expected_name: Option<&'static str>,
    expected_email: &'static str,
}

#[derive(Clone, Copy)]
struct GroupFixture {
    id: &'static str,
    input: &'static str,
    expected_name: &'static str,
    expected_members: &'static [(&'static str, Option<&'static str>)],
}

#[derive(Clone, Copy)]
struct TextFixture {
    id: &'static str,
    input: &'static str,
}

// RFC 5322 Appendix A.1.2 (mailbox addresses) + additional addr-spec compatibility vectors.
const RFC_VALID_ADDR_SPEC_FIXTURES: &[MailboxFixture] = &[
    MailboxFixture {
        id: "RFC5322-A.1.2-addr-spec-1",
        input: "jdoe@one.test",
        expected_name: None,
        expected_email: "jdoe@one.test",
    },
    MailboxFixture {
        id: "RFC5322-A.1.2-compat-1",
        input: "simple@example.com",
        expected_name: None,
        expected_email: "simple@example.com",
    },
    MailboxFixture {
        id: "RFC5322-A.1.2-compat-2",
        input: "very.common@example.com",
        expected_name: None,
        expected_email: "very.common@example.com",
    },
    MailboxFixture {
        id: "RFC5322-A.1.2-compat-3",
        input: "disposable.style.email.with+symbol@example.com",
        expected_name: None,
        expected_email: "disposable.style.email.with+symbol@example.com",
    },
    MailboxFixture {
        id: "RFC5322-A.1.2-compat-4",
        input: "other.email-with-hyphen@example.com",
        expected_name: None,
        expected_email: "other.email-with-hyphen@example.com",
    },
    MailboxFixture {
        id: "RFC5322-A.1.2-compat-5",
        input: "fully-qualified-domain@example.com",
        expected_name: None,
        expected_email: "fully-qualified-domain@example.com",
    },
    MailboxFixture {
        id: "RFC5322-A.1.2-compat-6",
        input: "user.name+tag+sorting@example.com",
        expected_name: None,
        expected_email: "user.name+tag+sorting@example.com",
    },
    MailboxFixture {
        id: "RFC5322-A.1.2-compat-7",
        input: "x@example.com",
        expected_name: None,
        expected_email: "x@example.com",
    },
    MailboxFixture {
        id: "RFC5322-A.1.2-compat-8",
        input: "example-indeed@strange-example.com",
        expected_name: None,
        expected_email: "example-indeed@strange-example.com",
    },
    MailboxFixture {
        id: "RFC5322-A.1.2-compat-9",
        input: "admin@mailserver1",
        expected_name: None,
        expected_email: "admin@mailserver1",
    },
    MailboxFixture {
        id: "RFC5322-A.1.2-compat-10",
        input: "example@s.example",
        expected_name: None,
        expected_email: "example@s.example",
    },
    MailboxFixture {
        id: "RFC5322-A.1.2-compat-11",
        input: "\"john..doe\"@example.org",
        expected_name: None,
        expected_email: "\"john..doe\"@example.org",
    },
    MailboxFixture {
        id: "RFC5322-A.1.2-compat-12",
        input: "mailhost!username@example.org",
        expected_name: None,
        expected_email: "mailhost!username@example.org",
    },
    MailboxFixture {
        id: "RFC5322-A.1.2-compat-13",
        input: "user%example.com@example.org",
        expected_name: None,
        expected_email: "user%example.com@example.org",
    },
];

// RFC 5322 Appendix A.1.2 (name-addr forms).
const RFC_VALID_NAME_ADDR_FIXTURES: &[MailboxFixture] = &[
    MailboxFixture {
        id: "RFC5322-A.1.2-name-addr-1",
        input: "\"Joe Q. Public\" <john.q.public@example.com>",
        expected_name: Some("Joe Q. Public"),
        expected_email: "john.q.public@example.com",
    },
    MailboxFixture {
        id: "RFC5322-A.1.2-name-addr-2",
        input: "Mary Smith <mary@x.test>",
        expected_name: Some("Mary Smith"),
        expected_email: "mary@x.test",
    },
    MailboxFixture {
        id: "RFC5322-A.1.2-name-addr-3",
        input: "Who? <one@y.test>",
        expected_name: Some("Who?"),
        expected_email: "one@y.test",
    },
    MailboxFixture {
        id: "RFC5322-A.1.2-name-addr-4",
        input: "\"Giant; \\\"Big\\\" Box\" <sysservices@example.net>",
        expected_name: Some("Giant; \"Big\" Box"),
        expected_email: "sysservices@example.net",
    },
    MailboxFixture {
        id: "RFC5322-A.1.2-name-addr-5",
        input: "<boss@nil.test>",
        expected_name: None,
        expected_email: "boss@nil.test",
    },
];

// RFC 5322 Appendix A.1.3 (group syntax).
const RFC_VALID_GROUP_FIXTURES: &[GroupFixture] = &[
    GroupFixture {
        id: "RFC5322-A.1.3-group-1",
        input: "A Group:Ed Jones <c@a.test>,joe@where.test,John <jdoe@one.test>;",
        expected_name: "A Group",
        expected_members: &[
            ("c@a.test", Some("Ed Jones")),
            ("joe@where.test", None),
            ("jdoe@one.test", Some("John")),
        ],
    },
    GroupFixture {
        id: "RFC5322-A.1.3-group-2",
        input: "Undisclosed recipients:;",
        expected_name: "Undisclosed recipients",
        expected_members: &[],
    },
];

const INVALID_MAILBOX_FIXTURES: &[TextFixture] = &[
    TextFixture {
        id: "invalid-mailbox-1",
        input: "plainaddress",
    },
    TextFixture {
        id: "invalid-mailbox-2",
        input: "@missing-local.org",
    },
    TextFixture {
        id: "invalid-mailbox-3",
        input: "A@b@c@example.com",
    },
    TextFixture {
        id: "invalid-mailbox-4",
        input: "john..doe@example.org",
    },
    TextFixture {
        id: "invalid-mailbox-5",
        input: "john.doe@example..org",
    },
    TextFixture {
        id: "invalid-mailbox-6",
        input: "john.doe.@example.org",
    },
    TextFixture {
        id: "invalid-mailbox-7",
        input: ".john.doe@example.org",
    },
];

const INVALID_GROUP_FIXTURES: &[TextFixture] = &[
    TextFixture {
        id: "invalid-group-1",
        input: "A Group",
    },
    TextFixture {
        id: "invalid-group-2",
        input: "A Group Ed Jones <c@a.test>;",
    },
];

const INVALID_ADDRESS_FIXTURES: &[TextFixture] = &[
    TextFixture {
        id: "invalid-address-1",
        input: "",
    },
    TextFixture {
        id: "invalid-address-2",
        input: "A Group",
    },
    TextFixture {
        id: "invalid-address-3",
        input: "john.q.public@example.com, mary@x.test",
    },
];

#[test]
fn mailbox_from_str_accepts_rfc_addr_spec_examples() {
    for fixture in RFC_VALID_ADDR_SPEC_FIXTURES {
        let parsed = fixture.input.parse::<Mailbox>();
        assert!(
            parsed.is_ok(),
            "{} expected valid mailbox: {}",
            fixture.id,
            fixture.input
        );
        let parsed = parsed.expect("validated above");
        assert_eq!(
            parsed.name(),
            fixture.expected_name,
            "{} unexpected display name for {}",
            fixture.id,
            fixture.input
        );
        assert_eq!(
            parsed.email().as_str(),
            fixture.expected_email,
            "{} email mismatch for {}",
            fixture.id,
            fixture.input
        );
    }
}

#[test]
fn mailbox_from_str_accepts_rfc_display_name_examples() {
    for fixture in RFC_VALID_NAME_ADDR_FIXTURES {
        let parsed = fixture.input.parse::<Mailbox>();
        assert!(
            parsed.is_ok(),
            "{} expected valid mailbox: {}",
            fixture.id,
            fixture.input
        );
        let parsed = parsed.expect("validated above");
        assert_eq!(
            parsed.name(),
            fixture.expected_name,
            "{} display name mismatch for {}",
            fixture.id,
            fixture.input
        );
        assert_eq!(
            parsed.email().as_str(),
            fixture.expected_email,
            "{} email mismatch for {}",
            fixture.id,
            fixture.input
        );
    }
}

#[test]
fn mailbox_from_str_rejects_invalid_examples() {
    for fixture in INVALID_MAILBOX_FIXTURES {
        let parsed = fixture.input.parse::<Mailbox>();
        assert!(
            parsed.is_err(),
            "{} expected invalid mailbox: {}",
            fixture.id,
            fixture.input
        );
    }
}

#[test]
fn email_from_str_accepts_rfc_addr_spec_examples() {
    for fixture in RFC_VALID_ADDR_SPEC_FIXTURES {
        let parsed = fixture.input.parse::<EmailAddress>();
        assert!(
            parsed.is_ok(),
            "{} expected valid email: {}",
            fixture.id,
            fixture.input
        );
        let parsed = parsed.expect("validated above");
        assert_eq!(
            parsed.as_str(),
            fixture.expected_email,
            "{} email mismatch for {}",
            fixture.id,
            fixture.input
        );
    }
}

#[test]
fn email_from_str_rejects_invalid_examples() {
    for fixture in INVALID_MAILBOX_FIXTURES {
        let parsed = fixture.input.parse::<EmailAddress>();
        assert!(
            parsed.is_err(),
            "{} expected invalid email: {}",
            fixture.id,
            fixture.input
        );
    }
}

#[test]
fn group_from_str_accepts_rfc_examples() {
    for fixture in RFC_VALID_GROUP_FIXTURES {
        let parsed = fixture.input.parse::<Group>();
        assert!(
            parsed.is_ok(),
            "{} expected valid group: {}",
            fixture.id,
            fixture.input
        );
        let parsed = parsed.expect("validated above");
        assert_eq!(
            parsed.name(),
            fixture.expected_name,
            "{} group name mismatch for {}",
            fixture.id,
            fixture.input
        );
        assert_eq!(
            parsed.members().len(),
            fixture.expected_members.len(),
            "{} group member count mismatch for {}",
            fixture.id,
            fixture.input
        );
        for (idx, (expected_email, expected_name)) in fixture.expected_members.iter().enumerate() {
            let member = &parsed.members()[idx];
            assert_eq!(
                member.email().as_str(),
                *expected_email,
                "{} group member email mismatch for {}",
                fixture.id,
                fixture.input
            );
            assert_eq!(
                member.name(),
                *expected_name,
                "{} group member name mismatch for {}",
                fixture.id,
                fixture.input
            );
        }
    }
}

#[test]
fn address_list_from_str_accepts_multiple_items() {
    let parsed = "Mary Smith <mary@x.test>, jdoe@one.test".parse::<AddressList>();
    assert!(parsed.is_ok(), "expected valid address list");
    let parsed = parsed.expect("validated above");
    assert_eq!(parsed.as_slice().len(), 2, "unexpected address count");
    assert_eq!(parsed.len(), 2, "unexpected address count via len()");
    assert!(!parsed.is_empty(), "address list should not be empty");

    assert_eq!(
        parsed.iter().count(),
        2,
        "unexpected iterated address count"
    );

    let as_vec: Vec<Address> = parsed.into();
    assert_eq!(as_vec.len(), 2, "unexpected converted address count");
}

#[test]
fn mailbox_list_from_str_accepts_multiple_mailboxes() {
    let parsed = "Mary Smith <mary@x.test>, jdoe@one.test".parse::<MailboxList>();
    assert!(parsed.is_ok(), "expected valid mailbox list");
    let parsed = parsed.expect("validated above");
    assert_eq!(parsed.as_slice().len(), 2, "unexpected mailbox count");
    assert_eq!(parsed.len(), 2, "unexpected mailbox count via len()");
    assert!(!parsed.is_empty(), "mailbox list should not be empty");

    assert_eq!(
        parsed.iter().count(),
        2,
        "unexpected iterated mailbox count"
    );

    let as_vec: Vec<Mailbox> = parsed.into();
    assert_eq!(as_vec.len(), 2, "unexpected converted mailbox count");
}

#[test]
fn mailbox_list_from_str_rejects_groups() {
    let parsed = "Undisclosed recipients:;".parse::<MailboxList>();
    match parsed {
        Err(MailboxParseError::ContainsGroupEntry) => {}
        other => panic!("expected ContainsGroupEntry error, got {other:?}"),
    }
}

#[test]
fn mailbox_from_str_reports_expected_single_mailbox_for_group_input() {
    let parsed = "Undisclosed recipients:;".parse::<Mailbox>();
    match parsed {
        Err(MailboxParseError::UnexpectedAddressKind) => {}
        other => panic!("expected UnexpectedAddressKind error, got {other:?}"),
    }
}

#[test]
fn group_from_str_reports_expected_single_group_for_mailbox_input() {
    let parsed = "jdoe@one.test".parse::<Group>();
    match parsed {
        Err(GroupParseError::UnexpectedAddressKind) => {}
        other => panic!("expected UnexpectedAddressKind error, got {other:?}"),
    }
}

#[test]
fn address_from_str_reports_expected_single_address_for_multiple_items() {
    let parsed = "jdoe@one.test, mary@x.test".parse::<Address>();
    match parsed {
        Err(AddressParseError::ExpectedSingleAddress { found: 2 }) => {}
        other => panic!("expected ExpectedSingleAddress {{ found: 2 }}, got {other:?}"),
    }
}

#[test]
fn group_from_str_rejects_invalid_examples() {
    for fixture in INVALID_GROUP_FIXTURES {
        let parsed = fixture.input.parse::<Group>();
        assert!(
            parsed.is_err(),
            "{} expected invalid group: {}",
            fixture.id,
            fixture.input
        );
    }
}

#[test]
fn address_from_str_accepts_mailbox_and_group_examples() {
    for fixture in RFC_VALID_ADDR_SPEC_FIXTURES {
        let parsed = fixture.input.parse::<Address>();
        assert!(
            parsed.is_ok(),
            "{} expected valid address: {}",
            fixture.id,
            fixture.input
        );
    }

    for fixture in RFC_VALID_NAME_ADDR_FIXTURES {
        let parsed = fixture.input.parse::<Address>();
        assert!(
            parsed.is_ok(),
            "{} expected valid address: {}",
            fixture.id,
            fixture.input
        );
    }

    for fixture in RFC_VALID_GROUP_FIXTURES {
        let parsed = fixture.input.parse::<Address>();
        assert!(
            parsed.is_ok(),
            "{} expected valid address: {}",
            fixture.id,
            fixture.input
        );
    }
}

#[test]
fn address_from_str_rejects_invalid_examples() {
    for fixture in INVALID_ADDRESS_FIXTURES {
        let parsed = fixture.input.parse::<Address>();
        assert!(
            parsed.is_err(),
            "{} expected invalid address: {}",
            fixture.id,
            fixture.input
        );
    }
}

#[test]
fn parsing_rejects_input_exceeding_max_address_input_bytes() {
    use email_message::{AddressBackendError, AddressParseError, MAX_ADDRESS_INPUT_BYTES};

    let oversized = "x".repeat(MAX_ADDRESS_INPUT_BYTES + 1);
    let parsed = oversized.parse::<Address>();
    assert!(
        matches!(
            parsed,
            Err(AddressParseError::Backend {
                source: AddressBackendError::InputTooLong { .. },
            })
        ),
        "expected InputTooLong, got {parsed:?}"
    );
}

#[test]
fn parsing_rejects_raw_newline_injection_inputs() {
    let newline_mailbox = "Mary Smith <mary@x.test>\nBcc: victim@example.com";
    let parsed_mailbox = newline_mailbox.parse::<Mailbox>();
    assert!(
        matches!(
            parsed_mailbox,
            Err(MailboxParseError::Backend {
                source: email_message::AddressBackendError::InputContainsRawNewlines,
            })
        ),
        "expected InputContainsRawNewlines for mailbox newline injection"
    );

    let newline_address = "A Group:Ed Jones <c@a.test>;\r\nCc: victim@example.com";
    let parsed_address = newline_address.parse::<Address>();
    assert!(
        matches!(
            parsed_address,
            Err(AddressParseError::Backend {
                source: email_message::AddressBackendError::InputContainsRawNewlines,
            })
        ),
        "expected InputContainsRawNewlines for address newline injection"
    );
}

#[test]
fn group_parse_reports_member_index_for_invalid_member_addr_spec() {
    let parsed = "Bad Group:good@example.com, john..doe@example.org;".parse::<Group>();
    match parsed {
        Err(GroupParseError::Backend {
            source:
                email_message::AddressBackendError::InvalidGroupMemberAddrSpec { index, input, .. },
        }) => {
            assert_eq!(index, 1, "expected invalid second member index");
            assert_eq!(
                input, "john..doe@example.org",
                "unexpected captured member input"
            );
        }
        other => panic!("expected InvalidGroupMemberAddrSpec backend error, got {other:?}"),
    }
}

#[test]
fn mailbox_display_roundtrips_for_rfc_examples() {
    for fixture in RFC_VALID_ADDR_SPEC_FIXTURES {
        let mailbox = fixture
            .input
            .parse::<Mailbox>()
            .expect("fixture should parse as mailbox");
        let rendered = mailbox.to_string();
        let reparsed = rendered
            .parse::<Mailbox>()
            .expect("rendered mailbox should parse");
        assert_eq!(
            reparsed.email().as_str(),
            fixture.expected_email,
            "{} email mismatch after roundtrip",
            fixture.id
        );
        assert_eq!(
            reparsed.name(),
            mailbox.name(),
            "{} name mismatch after roundtrip",
            fixture.id
        );
    }

    for fixture in RFC_VALID_NAME_ADDR_FIXTURES {
        let mailbox = fixture
            .input
            .parse::<Mailbox>()
            .expect("fixture should parse as mailbox");
        let rendered = mailbox.to_string();
        let reparsed = rendered
            .parse::<Mailbox>()
            .expect("rendered mailbox should parse");
        assert_eq!(
            reparsed.email().as_str(),
            fixture.expected_email,
            "{} email mismatch after roundtrip",
            fixture.id
        );
        assert_eq!(
            reparsed.name(),
            mailbox.name(),
            "{} name mismatch after roundtrip",
            fixture.id
        );
    }
}

#[test]
fn group_display_roundtrips_for_rfc_examples() {
    for fixture in RFC_VALID_GROUP_FIXTURES {
        let group = fixture
            .input
            .parse::<Group>()
            .expect("fixture should parse as group");
        let rendered = group.to_string();
        let reparsed = rendered
            .parse::<Group>()
            .expect("rendered group should parse");

        assert_eq!(
            reparsed.name(),
            group.name(),
            "{} group name mismatch after roundtrip",
            fixture.id
        );
        assert_eq!(
            reparsed.members().len(),
            group.members().len(),
            "{} group member count mismatch after roundtrip",
            fixture.id
        );
    }
}

#[test]
fn address_and_list_display_roundtrip() {
    let address = "Mary Smith <mary@x.test>"
        .parse::<Address>()
        .expect("address should parse");
    let rendered = address.to_string();
    let reparsed = rendered
        .parse::<Address>()
        .expect("rendered address should parse");
    assert_eq!(address, reparsed, "address roundtrip mismatch");

    let mailbox_list = "Mary Smith <mary@x.test>, jdoe@one.test"
        .parse::<MailboxList>()
        .expect("mailbox list should parse");
    let mailbox_list_rendered = mailbox_list.to_string();
    let mailbox_list_reparsed = mailbox_list_rendered
        .parse::<MailboxList>()
        .expect("rendered mailbox list should parse");
    assert_eq!(
        mailbox_list_reparsed.as_slice(),
        mailbox_list.as_slice(),
        "mailbox list roundtrip mismatch"
    );

    let address_list = "Mary Smith <mary@x.test>, jdoe@one.test"
        .parse::<AddressList>()
        .expect("address list should parse");
    let address_list_rendered = address_list.to_string();
    let address_list_reparsed = address_list_rendered
        .parse::<AddressList>()
        .expect("rendered address list should parse");
    assert_eq!(
        address_list_reparsed.as_slice(),
        address_list.as_slice(),
        "address list roundtrip mismatch"
    );
}

#[cfg(feature = "serde")]
#[test]
fn serde_roundtrip_email_and_message() {
    let email = "john.q.public@example.com"
        .parse::<EmailAddress>()
        .expect("email should parse");
    let encoded = serde_json::to_string(&email).expect("email should serialize");
    let decoded: EmailAddress = serde_json::from_str(&encoded).expect("email should deserialize");
    assert_eq!(decoded, email, "email serde roundtrip mismatch");

    let from = "Mary Smith <mary@x.test>"
        .parse::<Mailbox>()
        .expect("mailbox should parse");
    let to = vec![
        "jdoe@one.test"
            .parse::<Address>()
            .expect("address should parse"),
    ];

    let message = Message::new(from, to, email_message::Body::Text("Hello".to_string()));
    let encoded = serde_json::to_string(&message).expect("message should serialize");
    let decoded: Message = serde_json::from_str(&encoded).expect("message should deserialize");
    assert_eq!(decoded, message, "message serde roundtrip mismatch");

    let attachment_message = Message::builder(email_message::Body::Text("Hello".to_string()))
        .from_mailbox(
            "Mary Smith <mary@x.test>"
                .parse::<Mailbox>()
                .expect("mailbox should parse"),
        )
        .add_to(
            "jdoe@one.test"
                .parse::<Address>()
                .expect("address should parse"),
        )
        .add_attachment(
            email_message::Attachment::reference(
                email_message::ContentType::try_from("application/pdf")
                    .expect("content type should parse"),
                email_message::AttachmentReference::new("s3://attachments/report.pdf"),
            )
            .with_filename("report.pdf"),
        )
        .build()
        .expect("message should validate");
    let encoded =
        serde_json::to_string(&attachment_message).expect("attachment message should serialize");
    let decoded: Message =
        serde_json::from_str(&encoded).expect("attachment message should deserialize");
    assert_eq!(
        decoded, attachment_message,
        "attachment reference serde roundtrip mismatch"
    );
}

#[cfg(feature = "schemars")]
#[test]
fn schemars_generates_message_schema() {
    let schema = schema_for!(email_message::Message);
    let schema_json = serde_json::to_string(&schema).expect("schema should serialize");

    assert!(
        schema_json.contains("\"type\":\"object\""),
        "schema root should be an object: {schema_json}"
    );
    assert!(
        schema_json.contains("\"Message\""),
        "schema should include title"
    );
    assert!(
        schema_json.contains("\"properties\""),
        "schema should include properties"
    );
    assert!(
        schema_json.contains("\"subject\""),
        "schema should include subject field"
    );
}

#[cfg(all(feature = "schemars", not(feature = "rfc5322-string-compat")))]
fn assert_root_type<T: schemars::JsonSchema>(expected_type: &str) {
    let schema = schema_for!(T);
    let value = schema.as_value();

    assert_eq!(
        value.get("type").and_then(|value| value.as_str()),
        Some(expected_type),
        "schema should have expected root type: {value}"
    );
}

#[cfg(all(feature = "schemars", not(feature = "rfc5322-string-compat")))]
#[test]
fn schemars_value_objects_match_serde_shape() {
    assert_root_type::<email_message::EmailAddress>("string");
    assert_root_type::<email_message::Mailbox>("object");
    assert_root_type::<email_message::Group>("object");
    assert_root_type::<email_message::AddressList>("array");
    assert_root_type::<email_message::MailboxList>("array");
    assert_root_type::<email_message::MessageId>("string");
    assert_root_type::<email_message::ContentType>("string");
    assert_root_type::<email_message::ContentTransferEncoding>("string");
    assert_root_type::<email_message::ContentDisposition>("string");

    let address_schema = schema_for!(email_message::Address);
    let address_value = address_schema.as_value().to_string();
    assert!(address_value.contains("mailbox"));
    assert!(address_value.contains("group"));

    let body_schema = schema_for!(email_message::Body);
    let body_value = body_schema.as_value().to_string();
    assert!(body_value.contains("text_and_html"));
}

#[cfg(all(feature = "schemars", feature = "rfc5322-string-compat"))]
fn assert_one_of_has_string<T: schemars::JsonSchema>() {
    let schema = schema_for!(T);
    let value = schema.as_value();
    let one_of = value
        .get("oneOf")
        .and_then(|value| value.as_array())
        .unwrap_or_else(|| panic!("schema should have oneOf: {value}"));

    assert!(
        one_of
            .iter()
            .any(|value| value.get("type").and_then(|value| value.as_str()) == Some("string")),
        "schema should advertise string compatibility: {value}"
    );
}

#[cfg(all(feature = "schemars", feature = "rfc5322-string-compat"))]
#[test]
fn schemars_rfc5322_string_compat_advertises_string_alternatives() {
    assert_one_of_has_string::<email_message::Mailbox>();
    assert_one_of_has_string::<email_message::Group>();
    assert_one_of_has_string::<email_message::Address>();
    assert_one_of_has_string::<email_message::AddressList>();
    assert_one_of_has_string::<email_message::MailboxList>();
}

#[cfg(all(feature = "schemars", feature = "rfc5322-string-compat"))]
#[test]
fn schemars_address_compat_keeps_typed_variants_top_level() {
    let schema = schema_for!(email_message::Address);
    let value = schema.as_value();
    let one_of = value
        .get("oneOf")
        .and_then(|value| value.as_array())
        .unwrap_or_else(|| panic!("Address schema should have oneOf: {value}"));

    assert!(
        one_of.iter().any(|value| value
            .get("properties")
            .and_then(|value| value.get("type"))
            .and_then(|value| value.get("const"))
            .and_then(|value| value.as_str())
            == Some("mailbox")),
        "Address schema should advertise mailbox as a top-level alternative: {value}"
    );
    assert!(
        one_of.iter().any(|value| value
            .get("properties")
            .and_then(|value| value.get("type"))
            .and_then(|value| value.get("const"))
            .and_then(|value| value.as_str())
            == Some("group")),
        "Address schema should advertise group as a top-level alternative: {value}"
    );
    assert!(
        one_of
            .iter()
            .any(|value| value.get("type").and_then(|value| value.as_str()) == Some("string")),
        "Address schema should advertise string compatibility: {value}"
    );
    assert!(
        !one_of.iter().any(|value| value.get("oneOf").is_some()),
        "Address schema should not nest typed alternatives under another oneOf: {value}"
    );
}

#[cfg(feature = "schemars")]
#[test]
fn schemars_body_schema_is_tagged() {
    let body_schema = schema_for!(email_message::Body);
    let body_value = body_schema.as_value().to_string();

    assert!(body_value.contains("text"));
    assert!(body_value.contains("html"));
    assert!(body_value.contains("text_and_html"));
}

#[cfg(feature = "serde")]
#[test]
fn serde_uses_tagged_address_and_body_shapes() {
    let address = "Team: alice@example.com;"
        .parse::<email_message::Address>()
        .expect("group address parses");
    let address_json = serde_json::to_value(&address).expect("address serializes");
    assert_eq!(address_json["type"], "group");
    assert_eq!(address_json["name"], "Team");
    assert_eq!(address_json["members"][0]["type"], "mailbox");
    assert_eq!(address_json["members"][0]["email"], "alice@example.com");

    let decoded: email_message::Address =
        serde_json::from_value(address_json).expect("address deserializes");
    assert_eq!(decoded, address);

    let body = email_message::Body::text_and_html("plain", "<p>html</p>");
    let body_json = serde_json::to_value(&body).expect("body serializes");
    assert_eq!(body_json["type"], "text_and_html");
    assert_eq!(body_json["text"], "plain");
    assert_eq!(body_json["html"], "<p>html</p>");

    let decoded: email_message::Body =
        serde_json::from_value(body_json).expect("body deserializes");
    assert_eq!(decoded, body);
}

#[cfg(all(feature = "serde", not(feature = "rfc5322-string-compat")))]
#[test]
fn serde_rejects_rfc5322_string_addresses_without_compat() {
    assert!(
        serde_json::from_value::<email_message::Mailbox>(serde_json::json!("alice@example.com"))
            .is_err()
    );
    assert!(
        serde_json::from_value::<email_message::Group>(serde_json::json!(
            "Team: alice@example.com;"
        ))
        .is_err()
    );
    assert!(
        serde_json::from_value::<email_message::Address>(serde_json::json!(
            "Team: alice@example.com;"
        ))
        .is_err()
    );
    assert!(
        serde_json::from_value::<email_message::AddressList>(serde_json::json!(
            "alice@example.com, Team: bob@example.com;"
        ))
        .is_err()
    );
    assert!(
        serde_json::from_value::<email_message::MailboxList>(serde_json::json!(
            "alice@example.com, bob@example.com"
        ))
        .is_err()
    );
}

#[cfg(all(feature = "serde", feature = "rfc5322-string-compat"))]
#[test]
fn serde_accepts_rfc5322_string_addresses_with_compat() {
    let mailbox: email_message::Mailbox =
        serde_json::from_value(serde_json::json!("Mary Smith <mary@example.com>"))
            .expect("mailbox string should deserialize with compat");
    assert_eq!(mailbox.name(), Some("Mary Smith"));
    assert_eq!(mailbox.email().as_str(), "mary@example.com");

    let group: email_message::Group =
        serde_json::from_value(serde_json::json!("Team: alice@example.com;"))
            .expect("group string should deserialize with compat");
    assert_eq!(group.name(), "Team");
    assert_eq!(group.members().len(), 1);

    let address: email_message::Address =
        serde_json::from_value(serde_json::json!("Team: alice@example.com;"))
            .expect("address string should deserialize with compat");
    assert!(matches!(address, email_message::Address::Group(_)));

    let address_list: email_message::AddressList = serde_json::from_value(serde_json::json!(
        "alice@example.com, Team: bob@example.com;"
    ))
    .expect("address list string should deserialize with compat");
    assert_eq!(address_list.len(), 2);

    let mailbox_list: email_message::MailboxList =
        serde_json::from_value(serde_json::json!("alice@example.com, bob@example.com"))
            .expect("mailbox list string should deserialize with compat");
    assert_eq!(mailbox_list.len(), 2);
}

#[cfg(feature = "schemars")]
#[test]
fn schemars_message_schema_allows_omitting_defaulted_collections() {
    let schema = schema_for!(email_message::Message);
    let value = schema.as_value();
    let required = value
        .get("required")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();

    for field in ["to", "cc", "bcc", "reply_to", "headers", "attachments"] {
        assert!(
            !required.iter().any(|value| value.as_str() == Some(field)),
            "field `{field}` should not be required: {value}"
        );
    }
}

#[cfg(feature = "schemars")]
#[test]
fn schemars_outbound_message_schema_matches_message_schema() {
    // Regression guard: the manual `Serialize`/`Deserialize` on
    // `OutboundMessage` is transparent over the inner `Message`. The
    // hand-written `JsonSchema` must mirror that, otherwise a
    // downstream consumer validating an actual payload against the
    // schema would reject every valid message.
    let outbound_schema = schema_for!(email_message::OutboundMessage);
    let message_schema = schema_for!(email_message::Message);
    assert_eq!(
        serde_json::to_string(&outbound_schema).expect("OutboundMessage schema serializes"),
        serde_json::to_string(&message_schema).expect("Message schema serializes"),
        "OutboundMessage JsonSchema must match Message's"
    );
}

/// `mail_parser` switches the entire To-header to the group-shape as soon
/// as any group syntax appears, wrapping flat mailboxes that appear
/// before/between/after named groups into synthetic nameless groups.
/// Pin that the parser flattens those back so the documented
/// `address.rs` flatten-path produces the obvious result: an in-order
/// mix of mailboxes and named groups.
#[test]
fn address_list_flattens_mailboxes_around_named_group() {
    let parsed = "alice@example.com, Team: bob@example.com;, dave@example.com"
        .parse::<AddressList>()
        .expect("mixed mailbox + group list should parse");

    assert_eq!(parsed.len(), 3, "expected three address items in order");

    match &parsed.as_slice()[0] {
        Address::Mailbox(mailbox) => assert_eq!(mailbox.email().as_str(), "alice@example.com"),
        other => panic!("index 0: expected Mailbox(alice), got {other:?}"),
    }

    match &parsed.as_slice()[1] {
        Address::Group(group) => {
            assert_eq!(group.name(), "Team");
            assert_eq!(group.members().len(), 1);
            assert_eq!(group.members()[0].email().as_str(), "bob@example.com");
        }
        other => panic!("index 1: expected Group(Team, [bob]), got {other:?}"),
    }

    match &parsed.as_slice()[2] {
        Address::Mailbox(mailbox) => assert_eq!(mailbox.email().as_str(), "dave@example.com"),
        other => panic!("index 2: expected Mailbox(dave), got {other:?}"),
    }
}

#[cfg(all(feature = "serde", feature = "rfc5322-string-compat"))]
#[test]
fn serde_accepts_per_element_string_in_list_under_compat() {
    // Per-element RFC 5322 strings inside a `MailboxList`/`AddressList`
    // array are accepted by design (compat applies to each element
    // independently). Pin the behaviour so it doesn't drift.
    let mailbox_list: email_message::MailboxList = serde_json::from_value(serde_json::json!([
        "alice@example.com",
        {"type": "mailbox", "email": "bob@example.com"},
    ]))
    .expect("mixed string/object mailbox list should deserialize under compat");
    assert_eq!(mailbox_list.len(), 2);
    assert_eq!(
        mailbox_list.as_slice()[0].email().as_str(),
        "alice@example.com"
    );
    assert_eq!(
        mailbox_list.as_slice()[1].email().as_str(),
        "bob@example.com"
    );

    let address_list: email_message::AddressList = serde_json::from_value(serde_json::json!([
        "alice@example.com",
        {"type": "group", "name": "Team", "members": [{"type": "mailbox", "email": "bob@example.com"}]},
    ]))
    .expect("mixed string/object address list should deserialize under compat");
    assert_eq!(address_list.len(), 2);
    assert!(matches!(
        &address_list.as_slice()[0],
        email_message::Address::Mailbox(_)
    ));
    assert!(matches!(
        &address_list.as_slice()[1],
        email_message::Address::Group(_)
    ));
}

#[cfg(all(feature = "serde", feature = "rfc5322-string-compat"))]
#[test]
fn serde_compat_object_error_keeps_field_provenance() {
    // Regression guard for the Visitor-based compat dispatch (vs the
    // older `#[serde(untagged)]` shape, which collapsed all errors to
    // "did not match any variant"). A typed-object input with a missing
    // required field must surface a field-level diagnostic.
    let err = serde_json::from_value::<email_message::Mailbox>(serde_json::json!({
        "name": "Mary"
    }))
    .expect_err("missing email field should fail");
    let msg = err.to_string();
    assert!(
        msg.contains("email") || msg.contains("missing field"),
        "expected field-level error mentioning the missing field, got: {msg}"
    );
}

#[cfg(feature = "serde")]
#[test]
fn serde_omits_null_optional_fields_in_mailbox() {
    let mailbox: email_message::Mailbox = "alice@example.com".parse().expect("mailbox parses");
    let value = serde_json::to_value(&mailbox).expect("mailbox serializes");
    assert!(
        value.get("name").is_none(),
        "Mailbox without a display name should not emit `name`: {value}"
    );

    let mailbox: email_message::Mailbox = "Mary <mary@example.com>".parse().expect("mailbox parses");
    let value = serde_json::to_value(&mailbox).expect("mailbox serializes");
    assert_eq!(value["name"], "Mary");
}

#[cfg(feature = "serde")]
#[test]
fn serde_omits_empty_collections_in_message() {
    let from: Mailbox = "alice@example.com".parse().expect("mailbox parses");
    let to: Address = "bob@example.com".parse().expect("address parses");
    let message = email_message::Message::new(
        from,
        vec![to],
        email_message::Body::Text("hi".to_string()),
    );
    let value = serde_json::to_value(&message).expect("message serializes");

    for absent in ["cc", "bcc", "reply_to", "headers", "attachments", "sender"] {
        assert!(
            value.get(absent).is_none(),
            "field `{absent}` should be omitted when empty/none, got: {value}"
        );
    }
    assert_eq!(value["to"][0]["email"], "bob@example.com");
}

#[cfg(feature = "serde")]
#[test]
fn serde_attachment_bytes_round_trip_as_base64() {
    use email_message::{Attachment, AttachmentBody, ContentType};

    let attachment = Attachment::bytes(
        ContentType::try_from("text/plain").expect("content type parses"),
        b"report".to_vec(),
    );
    let value = serde_json::to_value(&attachment).expect("attachment serializes");
    assert_eq!(value["body"]["type"], "bytes");
    assert_eq!(
        value["body"]["bytes"], "cmVwb3J0",
        "Bytes payload must serialize as RFC 4648 base64 (with padding)"
    );

    let decoded: Attachment =
        serde_json::from_value(value).expect("attachment deserializes from base64 form");
    match decoded.body() {
        AttachmentBody::Bytes(bytes) => assert_eq!(bytes.as_slice(), b"report"),
        other => panic!("expected Bytes after roundtrip, got {other:?}"),
    }
}

#[cfg(feature = "serde")]
#[test]
fn serde_attachment_bytes_rejects_invalid_base64() {
    let err = serde_json::from_value::<email_message::Attachment>(serde_json::json!({
        "content_type": "text/plain",
        "disposition": "attachment",
        "body": {"type": "bytes", "bytes": "!!! not base64 !!!"}
    }))
    .expect_err("garbage base64 should fail to deserialize");
    assert!(
        err.to_string().contains("base64"),
        "expected base64 diagnostic, got: {err}"
    );
}

#[cfg(all(feature = "serde", feature = "mime"))]
#[test]
fn serde_mime_part_uses_internal_tag_with_snake_case() {
    use email_message::{ContentType, MimePart};

    let leaf = MimePart::Leaf {
        content_type: ContentType::try_from("text/plain").expect("ct parses"),
        content_transfer_encoding: None,
        content_disposition: None,
        body: b"hi".to_vec(),
    };
    let value = serde_json::to_value(&leaf).expect("leaf serializes");
    assert_eq!(
        value["type"], "leaf",
        "MimePart should use internally-tagged snake_case discriminator, not the derived externally-tagged PascalCase shape: {value}"
    );
    assert_eq!(value["body"], "aGk=", "MIME body should serialize as base64");
    assert!(
        value.get("content_transfer_encoding").is_none(),
        "absent optional fields should not emit null"
    );

    let multipart = MimePart::Multipart {
        content_type: ContentType::try_from("multipart/mixed; boundary=abc").expect("ct parses"),
        boundary: Some("abc".to_string()),
        parts: vec![leaf.clone()],
    };
    let value = serde_json::to_value(&multipart).expect("multipart serializes");
    assert_eq!(value["type"], "multipart");
    assert_eq!(value["boundary"], "abc");
    assert_eq!(value["parts"][0]["type"], "leaf");

    let decoded: MimePart =
        serde_json::from_value(value).expect("multipart roundtrips back through deserialize");
    assert_eq!(decoded, multipart);
}
