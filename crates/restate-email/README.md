# restate-email

[![crates.io](https://img.shields.io/crates/v/restate-email?style=flat-square)](https://crates.io/crates/restate-email)
[![docs.rs](https://img.shields.io/docsrs/restate-email?style=flat-square)](https://docs.rs/restate-email)

**Durable Restate service contracts for outbound email delivery.**

## Quick Start

Bind a transport registry to a Restate endpoint:

```rust
use restate_email::{Service, StaticTransportRegistry};
use restate_sdk::{endpoint::Endpoint, service::IntoServiceDefinition};

let registry = StaticTransportRegistry::new();
let service = Service::new(registry).into_service_definition();
let endpoint = Endpoint::builder().bind(service).build();
```

## Scope Contract

- Defines a serializable, registry-decoded `SendRequest` around a validated `email_message::OutboundMessage`.
- Resolves named `email_transport::Transport` registrations and dispatches messages through them.
- Exposes delivery as the Restate `Email.send` service handler.
- Maps worker and transport failures into Restate retry semantics.

## Feature Flags

Default features enable the Restate worker service adapter. The SDK-free wire
contract remains available with default features disabled.

- `service`: enables `Service`, the worker registry, and the `restate-sdk` dependency. This is enabled by default.
- `resend`: enables Resend provider-option deserialization through `email-kit`.
- `schemars`: derives JSON Schema for public queue payload types and forwards schema support to the message and transport crates.
- `rfc5322-string-compat`: accepts RFC 5322 string addresses in queue payloads and generated schemas in addition to typed address objects.

See the [crate documentation](https://docs.rs/restate-email/latest/restate_email/) for API and feature semantics and the [generated feature graph](https://docs.rs/crate/restate-email/latest/features) for activation details.

## Key Types

- `SendRequest`: transport reference, validated outbound message, and typed send-time options.
- `SendRequestSeed`: registry-driven deserializer for queued requests and provider-specific options.
- `TransportResolver`: resolves a transport key to a configured `Transport`.
- `StaticTransportRegistry`: owned registry for fixed-key worker setups.
- `Service`: Restate service wrapper that hydrates provider options and dispatches inside a named, journaled `ctx.run` action.
- `SendResponse`: serializable response containing the transport `SendReport`.
- `InvocationMode` / `RestateSendOptions`: how far a Restate-backed send is followed (`Queued` or `Sent`), as a transport default and as a per-send `"restate"` transport option.

Provider-specific `transport_options` use best-effort union semantics. A caller
may include slices for every provider it supports; the selected transport takes
its own registered provider slice, and unrecognized provider keys are ignored.
This preserves deployment-time transport switching, but switching transports
may drop provider-specific behavior such as tags.

Consequently, `transport_options` may only add or relax behavior. Controls that
constrain delivery, such as sandbox mode, suppression-list toggles, or
"never deliver to real recipients", must be core `SendOptions` that every
transport honors or rejects. See the
[`TransportOption` safety boundary](https://docs.rs/email-transport/latest/email_transport/trait.TransportOption.html).

## Caller-Side Transport

The caller-side ingress transport lives in
[`email-transport-restate`](https://crates.io/crates/email-transport-restate).
It submits this crate's `SendRequest` contract through Restate ingress
(`/restate/send/Email/send` and `/restate/call/Email/send`) without depending
on `restate-sdk`, follows a send as far as its `InvocationMode` says, and
authenticates to Restate Cloud ingress with a bearer token.

## Attachment Preparation

Attachment preparation composes at registry construction rather than in `Service`. Wrap a provider transport in `email_kit::attachment::AttachmentResolvingTransport` to resolve reference-backed attachments inside the existing `send_email` action before provider delivery:

```rust
use email_kit::attachment::{MapResolver, AttachmentResolvingTransport, SchemeRouter};
use restate_email::StaticTransportRegistry;

let resolver = SchemeRouter::new().with_resolver(
    "docs",
    MapResolver::new().with_entry("report.txt", b"report contents".to_vec()),
);
let mut registry = StaticTransportRegistry::new();
registry.insert(
    "transactional",
    AttachmentResolvingTransport::new(provider_transport, resolver),
);
```

A queued attachment reference such as `docs:report.txt` is then materialized at delivery time. Resolved bytes are not journaled, so retries may observe changed content; use immutable or versioned references when retry attempts must deliver identical bytes.

## Securing the Worker

Restate signs every request it makes to an SDK endpoint when the runtime is
configured with a request identity key. Register the matching `publickeyv1_...`
public keys on the endpoint builder to reject unsigned requests:

```rust
let endpoint = Endpoint::builder()
    .bind(service)
    .identity_key("publickeyv1_w7YHemBctH5Ck2nQRQ47iBBqhNHy4FV7t2Usbye2A6f")?
    .identity_key("publickeyv1_ChjENKeMvCtRnqG2mrBK1HmPKufgFUc98K8B3ononQvp")?
    .build();
```

Multiple keys stay valid at once, so rotation is a deployment change: register
the old and the new key, switch the runtime to the new private key, then drop
the old one. Identity keys authenticate the Restate runtime to the worker;
callers authenticate to Restate ingress separately (see
`email-transport-restate`).

## Retry Behavior

Retryable transport failures remain retryable Restate handler failures. Unknown transport keys, validation failures, and other permanent errors become terminal Restate errors.

## Examples

- The [basic worker](examples/restate_email_worker.rs) starts an SDK endpoint with a resolver-decorated example transport: `cargo run -p restate-email --example restate_email_worker`.
- The [Resend-backed worker](examples/restate_resend_worker.rs) requires `RESEND_API_KEY`, `RESEND_FROM`, and `RESEND_TO`: `cargo run -p restate-email --features resend --example restate_resend_worker`.

The worker examples expose raw Restate SDK endpoints for registration with Restate; they are not plain JSON HTTP handlers. Invoke `Email.send` through Restate ingress. Set `RESTATE_IDENTITY_KEY` (one key or a comma-separated list) to require signed requests. Caller-side examples live in [`email-transport-restate`](../email-transport-restate).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
