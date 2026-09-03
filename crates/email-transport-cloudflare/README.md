# email-transport-cloudflare

[![crates.io](https://img.shields.io/crates/v/email-transport-cloudflare?style=flat-square)](https://crates.io/crates/email-transport-cloudflare)
[![docs.rs](https://img.shields.io/docsrs/email-transport-cloudflare?style=flat-square)](https://docs.rs/email-transport-cloudflare)

**Send structured `email-message` values through a Cloudflare Workers `send_email` binding.**

## Quick Start

Declare the binding in `wrangler.toml`:

```toml
[[send_email]]
name = "EMAIL"
```

Then construct the transport from the Worker environment and use it like any other `email_transport::Transport`:

```rust
use email_message::{Address, Body, Message};
use email_transport::{SendOptions, Transport};
use email_transport_cloudflare::CloudflareTransport;

async fn send(env: &worker::Env) -> Result<(), Box<dyn std::error::Error>> {
    let message = Message::builder(Body::text("Welcome"))
        .from_mailbox("Sender <sender@yourdomain.com>".parse()?)
        .to(vec![Address::Mailbox("recipient@example.com".parse()?)])
        .subject("Hello")
        .build_outbound()?;

    let report = CloudflareTransport::from_env(env, "EMAIL")?
        .send(&message, &SendOptions::default())
        .await?;
    println!("cloudflare message id: {:?}", report.provider_message_id);
    Ok(())
}
```

`CloudflareTransport::new` accepts an already-obtained `worker::SendEmail` when you want to share one binding handle across several transports or decorators. The transport is `Clone`, and `SendReport::provider` is always `PROVIDER` (`"cloudflare"`).

## Feature Flags

This crate has no Cargo features. Cloudflare's send API carries nothing per-send beyond what `email_message::Message` already models, so there is no provider-specific `TransportOption` type and nothing to serialize.

See the [crate documentation](https://docs.rs/email-transport-cloudflare/latest/email_transport_cloudflare/) for the full mapping and error classification.

## What Reaches Cloudflare

- `From`, `To`, `Cc`, `Bcc` and `Reply-To` keep their display names; address groups are flattened. Cc-only and bcc-only messages are sent: the platform requires at least one of the three recipient lists, not `To` specifically. Cloudflare accepts a single `Reply-To`.
- `Body::Text`, `Body::Html` and `Body::TextAndHtml` map to `text`/`html`. At least one must be non-empty.
- Byte-backed attachments are sent as binary typed arrays with filename, content type and disposition. Cloudflare requires a filename on every attachment, so a regular attachment without one is named `attachment-N` (its 1-based position) and an inline one takes its content id. Inline attachments must carry a content id; a content id on a regular attachment is dropped because the platform accepts none there. Attachment references must be materialised first (for example with `email_attachment::AttachmentResolvingTransport`).
- Custom headers (`X-*`, `List-Unsubscribe`, `In-Reply-To`, ...) are forwarded verbatim; repeated header names collapse to the last value.
- **`date`, `message_id` and `sender` set on the message are dropped.** Cloudflare rejects `Date` and `Message-ID` with `E_HEADER_NOT_ALLOWED` and stamps its own `Message-ID`, which comes back as `SendReport::provider_message_id`.

`SendOptions::idempotency_key` and `SendOptions::timeout` are accepted and ignored: the binding has neither, and `Capabilities` says so.

## Cloudflare Constraints

Limits at the time of writing; none are enforced client-side, the platform's error codes are mapped to `TransportError` instead.

- The sender domain must be verified with Cloudflare Email Service.
- 50 recipients combined across `To`, `Cc` and `Bcc`.
- 32 attachments and 5 MiB total message size.
- Custom headers are allowlisted (`X-*` plus a fixed set such as `In-Reply-To`, `References`, `List-*`); platform-controlled headers are rejected. 16 KB of custom headers in total.
- Sends from the `send_email` binding appear as "dropped" in the Email Routing summary even when delivered.

## Local Development

`wrangler dev` without [remote bindings](https://developers.cloudflare.com/workers/local-development/#remote-bindings) cannot serialize binary attachment content. Use `remote = true` on the binding when testing attachments locally.

## Platform Support

The binding only functions on `wasm32-unknown-unknown` inside `workerd`. The crate compiles on native targets so workspace tests run everywhere, but `Transport::send` returns an `UnsupportedFeature` error there instead of panicking inside wasm-bindgen stubs.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
