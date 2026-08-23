# email-transport

[![crates.io](https://img.shields.io/crates/v/email-transport?style=flat-square)](https://crates.io/crates/email-transport)
[![docs.rs](https://img.shields.io/docsrs/email-transport?style=flat-square)](https://docs.rs/email-transport)

**Provider-neutral transport contracts for outbound email delivery.**

`Transport` accepts validated structured messages, while `RawTransport` accepts an explicit envelope and pre-rendered RFC822 bytes. `SendOptions` carries per-send metadata such as envelope overrides, timeouts, idempotency keys, correlation IDs, and typed provider options without coupling the core API to a provider.

The crate also defines transport capabilities, send reports, structured error kinds, adapter helpers, and `TracingTransport` when tracing support is enabled.

## Quick Start

```rust
use email_transport::email_message::OutboundMessage;
use email_transport::{SendOptions, SendReport, Transport, TransportError};

async fn deliver<T: Transport>(
    transport: &T,
    message: &OutboundMessage,
) -> Result<SendReport, TransportError> {
    transport.send(message, &SendOptions::default()).await
}
```

## Feature Flags

The `serde` feature is enabled by default.

- `serde`: enables serialization of transport values and provider-option registry support, and forwards serde support to `email-message`.
- `schemars`: enables JSON Schema support and forwards it to `email-message`.
- `tracing`: exposes transport instrumentation; on `wasm32-unknown-unknown`, it uses `web-time` for timing.

See the [crate documentation](https://docs.rs/email-transport/latest/email_transport/) for transport contracts and feature semantics and the [generated feature graph](https://docs.rs/crate/email-transport/latest/features) for activation details.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
