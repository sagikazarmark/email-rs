# email-transport-test

**First-party test transports and conformance helpers for [`email-transport`](../email-transport).**

`MemoryTransport` captures sends in memory for assertions. `FileTransport` writes sends as RFC822 `.eml` files. The optional `conformance` module provides shared message fixtures for verifying cross-provider semantics.

This crate is an unpublished workspace utility.

## Feature Flags

No features are enabled by default.

- `conformance`: exposes shared provider conformance fixtures and enables their time support.

## Memory Transport

Use `MemoryTransport` to assert the structured message, raw bytes, envelope override, timeout, idempotency key, or correlation ID that would have been sent.

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

    transport.send(&message, &SendOptions::default()).await?;
    let captured = transport.captured();
    let CapturedPayload::Structured { message, .. } = &captured[0].payload else {
        panic!("expected structured capture");
    };
    assert_eq!(message.subject(), Some("Hi"));

    Ok(())
}
```

## File Transport

Use `FileTransport` when a test needs an RFC822 `.eml` artifact. Existing files are never overwritten; if `message-0001.eml` exists, the transport advances to the next monotonically numbered path.

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
    let path = report.provider_message_id.ok_or_else(|| {
        std::io::Error::other("file transport did not report the output path")
    })?;
    assert!(std::fs::read(path)?.starts_with(b"From: "));

    Ok(())
}
```

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](../../LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](../../LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
