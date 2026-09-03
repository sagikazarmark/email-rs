# Send an email through the Restate ingress and check the delivery report.
POST {{restate_url}}/Email/send
Content-Type: application/json
{
  "transport": "transactional",
  "message": {
    "from": {"type": "mailbox", "email": "sender@example.com"},
    "to": [{"type": "mailbox", "email": "recipient@example.com"}],
    "subject": "e2e {{run_id}}",
    "body": {"type": "text", "text": "Hello from the Restate email endpoint! ({{run_id}})"}
  }
}
HTTP 200
[Asserts]
jsonpath "$.report.provider" == "smtp"
jsonpath "$.report.accepted" count == 1
jsonpath "$.report.accepted[0]" == "recipient@example.com"

# Mailpit stored exactly one message with this run's subject.
GET {{mailpit_url}}/api/v1/search
[Query]
query: subject:"e2e {{run_id}}"
[Options]
retry: 20
retry-interval: 500ms
HTTP 200
[Captures]
message_id: jsonpath "$.messages[0].ID"
[Asserts]
jsonpath "$.messages_count" == 1
jsonpath "$.messages[0].Subject" == "e2e {{run_id}}"
jsonpath "$.messages[0].From.Address" == "sender@example.com"
jsonpath "$.messages[0].To" count == 1
jsonpath "$.messages[0].To[0].Address" == "recipient@example.com"

# The stored message carries the body that was sent.
GET {{mailpit_url}}/api/v1/message/{{message_id}}
HTTP 200
[Asserts]
jsonpath "$.Subject" == "e2e {{run_id}}"
jsonpath "$.Text" contains "Hello from the Restate email endpoint! ({{run_id}})"
jsonpath "$.Attachments" count == 0
