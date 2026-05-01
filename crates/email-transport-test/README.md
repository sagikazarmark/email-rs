# email-transport-test

First-party test transports and shared conformance helpers for the
[`email-transport`](../email-transport) contract.

- `MemoryTransport` captures every send in memory for assertion in tests.
- `FileTransport` writes every send to disk as RFC822 `.eml` files.
- `conformance` (feature) exposes the shared message factory used by the
  provider crates to verify they all agree on the cross-provider semantics.

## Memory transport

Use `MemoryTransport` when test code needs to assert the structured message,
raw bytes, envelope override, timeout, idempotency key, or correlation ID that
would have been sent.

```rust
use email_message::{Address, Body, Message};
use email_transport::{SendOptions, Transport};
use email_transport_test::{CapturedPayload, MemoryTransport};

async fn example() -> Result<(), Box<dyn std::error::Error>> {
    let transport = MemoryTransport::new().with_provider_message_id("msg-1");
    let message = Message::builder(Body::text("Hello"))
        .from_mailbox("sender@example.com".parse()?)
        .to(vec![Address::Mailbox("recipient@example.com".parse()?)])
        .subject("Hi")
        .build_outbound()?;

    let report = transport.send(&message, &SendOptions::default()).await?;
    assert_eq!(report.provider_message_id.as_deref(), Some("msg-1"));

    let captured = transport.captured();
    let CapturedPayload::Structured { message, .. } = &captured[0].payload else {
        panic!("expected structured capture");
    };
    assert_eq!(message.subject(), Some("Hi"));

    Ok(())
}
```

## File transport

Use `FileTransport` when a test needs an RFC822 `.eml` artifact. Existing files
are never overwritten; if `message-0001.eml` already exists, the transport skips
to the next available monotonically numbered path.

```rust
use email_message::{Address, Body, Message};
use email_transport::{SendOptions, Transport};
use email_transport_test::FileTransport;

async fn example(dir: std::path::PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let transport = FileTransport::new(dir)?;
    let message = Message::builder(Body::text("Hello"))
        .from_mailbox("sender@example.com".parse()?)
        .to(vec![Address::Mailbox("recipient@example.com".parse()?)])
        .subject("Hi")
        .build_outbound()?;

    let report = transport.send(&message, &SendOptions::default()).await?;
    let path = report.provider_message_id.expect("file path report id");
    let bytes = std::fs::read(path)?;
    assert!(bytes.starts_with(b"From: "));

    Ok(())
}
```
