#[cfg(feature = "arbitrary")]
#[test]
fn arbitrary_feature_compiles_for_core_types() {
    fn assert_arbitrary<T: arbitrary::Arbitrary<'static>>() {}

    assert_arbitrary::<email_message::EmailAddress>();
    assert_arbitrary::<email_message::Mailbox>();
    assert_arbitrary::<email_message::Address>();
    assert_arbitrary::<email_message::Message>();
}
