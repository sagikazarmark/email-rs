use email_message::{AddressList, Mailbox};
use proptest::prelude::*;

fn local_part_strategy() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![
            Just('a'),
            Just('b'),
            Just('c'),
            Just('d'),
            Just('e'),
            Just('f'),
            Just('g'),
            Just('h'),
            Just('i'),
            Just('j'),
            Just('k'),
            Just('l'),
            Just('m'),
            Just('n'),
            Just('o'),
            Just('p'),
            Just('q'),
            Just('r'),
            Just('s'),
            Just('t'),
            Just('u'),
            Just('v'),
            Just('w'),
            Just('x'),
            Just('y'),
            Just('z'),
            Just('0'),
            Just('1'),
            Just('2'),
            Just('3'),
            Just('4'),
            Just('5'),
            Just('6'),
            Just('7'),
            Just('8'),
            Just('9'),
            Just('.'),
            Just('_'),
            Just('+'),
            Just('-')
        ],
        1..20,
    )
    .prop_map(|chars| chars.into_iter().collect::<String>())
    .prop_filter("local-part should not start/end with dot", |s| {
        !s.starts_with('.') && !s.ends_with('.') && !s.contains("..")
    })
}

fn domain_label_strategy() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![
            Just('a'),
            Just('b'),
            Just('c'),
            Just('d'),
            Just('e'),
            Just('f'),
            Just('g'),
            Just('h'),
            Just('i'),
            Just('j'),
            Just('k'),
            Just('l'),
            Just('m'),
            Just('n'),
            Just('o'),
            Just('p'),
            Just('q'),
            Just('r'),
            Just('s'),
            Just('t'),
            Just('u'),
            Just('v'),
            Just('w'),
            Just('x'),
            Just('y'),
            Just('z'),
            Just('0'),
            Just('1'),
            Just('2'),
            Just('3'),
            Just('4'),
            Just('5'),
            Just('6'),
            Just('7'),
            Just('8'),
            Just('9'),
            Just('-')
        ],
        1..12,
    )
    .prop_map(|chars| chars.into_iter().collect::<String>())
    .prop_filter("domain label should not start/end with hyphen", |s| {
        !s.starts_with('-') && !s.ends_with('-')
    })
}

fn email_strategy() -> impl Strategy<Value = String> {
    (
        local_part_strategy(),
        domain_label_strategy(),
        domain_label_strategy(),
    )
        .prop_map(|(local, d1, d2)| format!("{local}@{d1}.{d2}"))
}

proptest! {
    #[test]
    fn mailbox_display_roundtrip_holds(email in email_strategy()) {
        let mailbox: Mailbox = email.parse().expect("generated email should parse");
        let rendered = mailbox.to_string();
        let reparsed: Mailbox = rendered.parse().expect("rendered mailbox should parse");

        prop_assert_eq!(reparsed.email().as_str(), mailbox.email().as_str());
        prop_assert_eq!(reparsed.name(), mailbox.name());
    }

    #[test]
    fn address_list_display_roundtrip_holds(
        emails in prop::collection::vec(email_strategy(), 1..8)
    ) {
        let joined = emails.join(", ");
        let list: AddressList = joined.parse().expect("generated list should parse");
        let rendered = list.to_string();
        let reparsed: AddressList = rendered.parse().expect("rendered list should parse");

        prop_assert_eq!(reparsed.as_slice(), list.as_slice());
    }
}
