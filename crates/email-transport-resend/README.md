# email-transport-resend

Resend transport implementation for `email-transport`.

## Scope contract

- Sends structured `email_message::Message` values through Resend's JSON API.
- Supports text/html bodies, custom headers, attachments, inline attachments, and idempotency keys.
- Exposes Resend-specific per-send options through `ResendSendOptions`.

## Send options

- `ResendSendOptions`
- `ResendTag`
- `ResendTemplate`

Insert `ResendSendOptions` into `SendOptions.transport_options` to set Resend-only tags or template data for one send attempt. The same struct is serde-compatible for queued `transport_options.resend` payloads.

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

Run the example with:

```text
RESEND_API_KEY=... RESEND_TO=you@example.com cargo run -p email-transport-resend --example resend_send
```

## Development

- Run tests: `cargo test -p email-transport-resend`
- Check wasm compatibility: `cargo check -p email-transport-resend --target wasm32-unknown-unknown`
