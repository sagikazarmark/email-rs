use email_message::{AddressList, MailboxList};

#[test]
fn mailbox_list_try_from_vec_str() {
    let list = MailboxList::try_from(vec!["jdoe@one.test", "mary@x.test"])
        .expect("mailbox list should parse");
    assert_eq!(list.len(), 2);
}

#[test]
fn address_list_try_from_slice_str() {
    let src = ["jdoe@one.test", "Undisclosed recipients:;"];
    let list = AddressList::try_from(src.as_slice()).expect("address list should parse");
    assert_eq!(list.len(), 2);
}
