# restate-email

[![crates.io](https://img.shields.io/crates/v/restate-email?style=flat-square)](https://crates.io/crates/restate-email)
[![docs.rs](https://img.shields.io/docsrs/restate-email?style=flat-square)](https://docs.rs/restate-email)

**Durable Restate service contracts for outbound email delivery.**

## Scope Contract

- Defines a serializable `SendRequest` around a validated `email_message::OutboundMessage`.
- Resolves named `email_transport::Transport` registrations and dispatches messages through them.
- Exposes delivery as the Restate `Email.send` service handler.
- Maps worker and transport failures into Restate retry semantics.

## Feature Flags

Default features enable reqwest's default client stack for the local ingress example.

- `resend`: enables Resend provider-option deserialization through `email-kit`.
- `schemars`: derives JSON Schema for public queue payload types and forwards schema support to the message and transport crates.
- `rfc5322-string-compat`: accepts RFC 5322 string addresses in queue payloads and generated schemas in addition to typed address objects.

See the [crate documentation](https://docs.rs/restate-email/latest/restate_email/) for API and feature semantics and the [generated feature graph](https://docs.rs/crate/restate-email/latest/features) for activation details.

## Key Types

- `SendRequest`: transport reference, validated outbound message, and raw send-time options.
- `RawSendOptions`: wire-safe envelope overrides, timeout, idempotency key, correlation ID, and provider-specific transport options.
- `TransportResolver`: resolves a transport key to a configured `Transport`.
- `StaticTransportRegistry`: owned registry for fixed-key worker setups.
- `ServiceImpl`: Restate service wrapper that hydrates provider options and dispatches inside a named, journaled `ctx.run` action.
- `SendResponse`: serializable response containing the transport `SendReport`.

Attachment preparation composes at registry construction rather than in `ServiceImpl`. Wrap a provider transport in `email_kit::attachment::ResolvingTransport` to resolve reference-backed attachments inside the existing `send_email` action before provider delivery:

```rust
use email_kit::attachment::{MapResolver, ResolvingTransport, SchemeRouter};
use restate_email::StaticTransportRegistry;

let resolver = SchemeRouter::new().with_resolver(
    "docs",
    MapResolver::new().with_entry("report.txt", b"report contents".to_vec()),
);
let mut registry = StaticTransportRegistry::new();
registry.insert(
    "transactional",
    ResolvingTransport::new(provider_transport, resolver),
);
```

A queued attachment reference such as `docs:report.txt` is then materialized at delivery time. Resolved bytes are not journaled, so retries may observe changed content; use immutable or versioned references when retry attempts must deliver identical bytes.

Bind the service using Restate's service definition API:

```rust
use restate_email::{ServiceImpl, StaticTransportRegistry};
use restate_sdk::{endpoint::Endpoint, service::IntoServiceDefinition};

let registry = StaticTransportRegistry::new();
let service = ServiceImpl::new(registry).into_service_definition();
let endpoint = Endpoint::builder().bind(service).build();
```

## Retry Behavior

Retryable transport failures remain retryable Restate handler failures. Unknown transport keys, validation failures, and other permanent errors become terminal Restate errors.

## Examples

- The [basic worker](examples/restate_email_worker.rs) starts an SDK endpoint with a resolver-decorated example transport: `cargo run -p restate-email --example restate_email_worker`.
- The [local ingress client](examples/invoke_local_worker.rs) sends its reference-backed attachment to `http://127.0.0.1:8080/Email/send` by default: `cargo run -p restate-email --example invoke_local_worker`. Set `RESTATE_INGRESS_URL` to override the ingress URL.
- The [Resend-backed worker](examples/restate_resend_worker.rs) requires `RESEND_API_KEY`, `RESEND_FROM`, and `RESEND_TO`: `cargo run -p restate-email --features resend --example restate_resend_worker`.

The worker examples expose raw Restate SDK endpoints for registration with Restate; they are not plain JSON HTTP handlers. Invoke `Email.send` through Restate ingress.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
