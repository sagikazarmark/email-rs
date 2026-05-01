use email_transport::{SendOptions, Transport};
use email_transport_resend::ResendTransport;
use email_transport_test::conformance::{EXPECTED_ACCEPTED, conformance_message};
use serde_json::Value;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn resend_transport_conforms_shared_semantics() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/emails"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "re_123"
        })))
        .mount(&server)
        .await;

    let transport = ResendTransport::builder("test-key")
        .base_url(
            format!("{}/", server.uri())
                .parse()
                .expect("base URL should parse"),
        )
        .build();

    let report = transport
        .send(&conformance_message(), &SendOptions::default())
        .await
        .expect("send should succeed");

    let accepted_strs: Vec<&str> = report
        .accepted
        .iter()
        .map(email_message::EmailAddress::as_str)
        .collect();
    assert_eq!(accepted_strs, EXPECTED_ACCEPTED);

    let requests = server
        .received_requests()
        .await
        .expect("request recording should be enabled");
    let body: Value = requests[0]
        .body_json()
        .expect("request body should be valid json");

    assert_eq!(body["to"][0], "a@example.com");
    assert_eq!(body["to"][1], "b@example.com");
    assert_eq!(body["cc"][0], "cc@example.com");
    assert_eq!(body["bcc"][0], "hidden@example.com");
    assert_eq!(body["reply_to"][0], "reply@example.com");
    assert_eq!(body["headers"]["Sender"], "bounce@example.com");
    assert_eq!(body["headers"]["Message-ID"], "<conformance@example.com>");
    assert_eq!(body["headers"]["X-Test"], "demo");
    assert!(body["headers"]["Date"].is_string());
}
