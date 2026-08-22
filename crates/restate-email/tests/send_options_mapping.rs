use restate_email::RawSendOptions;

#[test]
fn raw_send_options_reject_unmapped_fields() {
    let result = serde_json::from_value::<RawSendOptions>(serde_json::json!({
        "future_option": "must not be dropped"
    }));

    let error = result.expect_err("unmapped send option fields must be rejected");
    assert!(
        error.to_string().contains("future_option"),
        "error should identify the unmapped field: {error}"
    );
}
