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

- `client`: enables `RestateTransport`, a caller-side `email_transport::Transport` that invokes Restate ingress (`/restate/send/Email/send` and `/restate/call/Email/send`) without depending on `restate-sdk`.
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
- `RestateTransport`: caller-side transport that submits the same contract through Restate ingress.
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

`RestateTransport` follows a send as far as its `InvocationMode` says:

- `InvocationMode::Queued` (default) posts to `/restate/send/Email/send` and
  returns once Restate has durably accepted the invocation. The report names
  `restate` as the provider and carries the Restate invocation id as the
  message id; `RestateTransport::invocation_id` reads it back for
  `/restate/attach/{id}` or `/restate/output/{id}`.
- `InvocationMode::Sent` posts to `/restate/call/Email/send`, waits for the
  worker, and returns the worker's provider report unchanged.

The mode is configured on the builder and overridden per send through
`RestateSendOptions` (provider key `restate`), which can also delay a queued
invocation:

```rust
use std::time::Duration;

use email_transport::{SendOptions, TransportOptions};
use restate_email::{InvocationMode, RestateSendOptions, RestateTransport, TransportKey};

let transport = RestateTransport::builder(
    TransportKey::new("transactional")?,
    "http://127.0.0.1:8080".parse()?,
)
.invocation_mode(InvocationMode::Sent)
.attachment_references(true)
.build();

let mut transport_options = TransportOptions::default();
transport_options.insert(
    RestateSendOptions::new()
        .with_invocation_mode(InvocationMode::Queued)
        .with_delay(Duration::from_secs(60)),
);
let options = SendOptions::new().with_transport_options(transport_options);
```

The `restate` slice is forwarded in the queued payload like every other
provider slice; workers ignore it unless their registry chains into another
Restate-backed transport.

`RestateTransport` consumes `SendOptions::idempotency_key` as Restate's
`idempotency-key` ingress header in both modes. The key is omitted from the
queued `SendOptions`, so it is not forwarded to the provider. This makes
replaying the enqueue safe without accidentally reusing one key across Restate
and provider idempotency domains.

The ingress client cannot inspect worker capabilities. Its defaults describe the
ingress hop (structured sends and ingress idempotency); everything about the
worker is a deployment assertion made on the builder. Unresolved attachment
references remain disabled by default and can be asserted with
`.attachment_references(true)` when the worker has a resolver configured.

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

## Retry Behavior

Retryable transport failures remain retryable Restate handler failures. Unknown transport keys, validation failures, and other permanent errors become terminal Restate errors.

## Examples

- The [basic worker](examples/restate_email_worker.rs) starts an SDK endpoint with a resolver-decorated example transport: `cargo run -p restate-email --example restate_email_worker`.
- The [local ingress client](examples/invoke_local_worker.rs) sends its reference-backed attachment to `http://127.0.0.1:8080/restate/call/Email/send` by default: `cargo run -p restate-email --features client --example invoke_local_worker`. Set `RESTATE_INGRESS_URL` to override the ingress URL.
- The [interchangeable transport example](examples/direct_or_restate.rs) sends through the same application function with either a direct Resend transport or `RestateTransport`: `cargo run -p restate-email --features client --example direct_or_restate`. Set `EMAIL_TO` in both modes, `RESTATE_INGRESS_URL` to select Restate (add `RESTATE_WAIT=1` to wait for the worker's report), or `RESEND_API_KEY` for direct delivery.
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
