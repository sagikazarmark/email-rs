# email-transport-lettre

[![crates.io](https://img.shields.io/crates/v/email-transport-lettre?style=flat-square)](https://crates.io/crates/email-transport-lettre)
[![docs.rs](https://img.shields.io/docsrs/email-transport-lettre?style=flat-square)](https://docs.rs/email-transport-lettre)

**SMTP delivery for email-rs through Lettre.**

`LettreTransport` implements both `email_transport::Transport` for structured messages and `email_transport::RawTransport` for explicit envelopes and RFC822 bytes. Structured messages are rendered with `email-message-wire` before SMTP submission.

## Quick Start

```rust,no_run
use email_message::{Address, Body, Message};
use email_transport::{SendOptions, Transport};
use email_transport_lettre::LettreTransport;

# async fn send() -> Result<(), Box<dyn std::error::Error>> {
let message = Message::builder(Body::text("Hello"))
    .from_mailbox("sender@example.com".parse()?)
    .to(vec![Address::Mailbox("recipient@example.com".parse()?)])
    .subject("Hello")
    .build_outbound()?;

let transport = LettreTransport::from_url(
    "smtps://username:password@smtp.example.com:465",
)?;
transport.send(&message, &SendOptions::default()).await?;
# Ok(())
# }
```

## Feature Flags

- `default`: enables `pool` and `rustls-tls`.
- `native-tls`: enables Lettre's Tokio native-TLS backend.
- `pool`: enables Lettre's SMTP connection pool.
- `rustls-tls`: enables Lettre's Tokio Rustls backend with WebPKI roots.

Disable default features to build an unencrypted SMTP client for local relays.

## Timeouts and Cancellation

A per-send timeout bounds how long the caller waits; it does not abort an SMTP
transaction already in progress. The transaction continues in the background
so a partially used connection cannot return to Lettre's pool. Delivery may
therefore still succeed after a timeout or caller cancellation. Concurrent
handoffs are bounded so stalled background transactions cannot accumulate
without limit.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
