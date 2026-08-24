# email-transport-resend

[![crates.io](https://img.shields.io/crates/v/email-transport-resend?style=flat-square)](https://crates.io/crates/email-transport-resend)
[![docs.rs](https://img.shields.io/docsrs/email-transport-resend?style=flat-square)](https://docs.rs/email-transport-resend)

**Send structured `email-message` values through Resend.**

## Quick Start

```rust
use email_message::{Address, Body, Message};
use email_transport::{SendOptions, Transport};
use email_transport_resend::ResendTransport;

async fn send() -> Result<(), Box<dyn std::error::Error>> {
    let message = Message::builder(Body::text("Welcome"))
        .from_mailbox("sender@example.com".parse()?)
        .to(vec![Address::Mailbox("recipient@example.com".parse()?)])
        .subject("Hello")
        .build_outbound()?;

    ResendTransport::new("re_...")
        .send(&message, &SendOptions::default())
        .await?;
    Ok(())
}
```

## Feature Flags

Default features enable reqwest's default HTTP/TLS stack and serde support.

- `serde`: enables queue/wire deserialization for `ResendSendOptions` and forwards serde support to `email-transport`. Resend option types always implement `Serialize` so feature unification remains additive.

See the [crate documentation](https://docs.rs/email-transport-resend/latest/email_transport_resend/) for API and feature semantics and the [generated feature graph](https://docs.rs/crate/email-transport-resend/latest/features) for activation details.

## Send Options

Use `ResendSendOptions`, `ResendTag`, and `ResendTemplate` for per-send tags and template data. Insert `ResendSendOptions` into `SendOptions::transport_options`; with serde enabled, the same type can cross a queued `transport_options.resend` boundary.

```rust
use email_transport::TransportOptions;
use email_transport_resend::{ResendSendOptions, ResendTemplate};

let mut transport_options = TransportOptions::default();
transport_options.insert(
    ResendSendOptions::new()
        .with_tags([("env", "prod"), ("tenant", "blue")])
        .with_template(
            ResendTemplate::new("tmpl_welcome")
                .with_variables([("name", serde_json::json!("Ada"))]),
        ),
);
```

## Example

The canonical [`resend_send` example](examples/resend_send.rs) demonstrates credentials, idempotency, provider options, and the resulting send report:

```sh
RESEND_API_KEY=... RESEND_TO=you@example.com cargo run -p email-transport-resend --example resend_send
```

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
