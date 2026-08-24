# email-transport-restate

[![crates.io](https://img.shields.io/crates/v/email-transport-restate?style=flat-square)](https://crates.io/crates/email-transport-restate)
[![docs.rs](https://img.shields.io/docsrs/email-transport-restate?style=flat-square)](https://docs.rs/email-transport-restate)

**Restate ingress transport for outbound email delivery.**

## Quick Start

```rust
use email_transport_restate::{RestateTransport, TransportKey};

let transport = RestateTransport::new(
    TransportKey::new("transactional")?,
    "http://127.0.0.1:8080".parse()?,
);
```

`RestateTransport` implements `email_transport::Transport`, so the same
application code can hand a message to a durable Restate queue or to a direct
provider adapter. The wire contract it submits — and the worker that consumes
it — live in [`restate-email`](https://crates.io/crates/restate-email); this
crate never depends on `restate-sdk`.

## Invocation Modes

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
use email_transport_restate::{
    InvocationMode, RestateSendOptions, RestateTransport, TransportKey,
};

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

## Authentication

Restate Cloud ingress requires an API key as a bearer token:

```rust
use email_transport_restate::{RestateTransport, TransportKey};

let transport = RestateTransport::builder(
    TransportKey::new("transactional")?,
    "https://example.env.us.restate.cloud:8080".parse()?,
)
.bearer_token(std::env::var("RESTATE_AUTH_TOKEN")?)
.build();
```

Self-hosted Restate has no built-in ingress authentication; a fronting reverse
proxy that expects the same `Authorization: Bearer` header is covered by the
same setter. Any other header scheme can be attached to a custom
`reqwest::Client` (via `reqwest::ClientBuilder::default_headers`) passed to
the builder's `client` setter. The token is redacted from `Debug` output, and
ingress `401`/`403` responses map to `ErrorKind::Authentication` /
`ErrorKind::Authorization`.

Authenticating the worker endpoint itself (Restate's request identity) is the
worker's concern; see the `restate-email` documentation.

## Idempotency

`RestateTransport` consumes `SendOptions::idempotency_key` as Restate's
`idempotency-key` ingress header in both modes. The key is omitted from the
queued `SendOptions`, so it is not forwarded to the provider. This makes
replaying the enqueue safe without accidentally reusing one key across Restate
and provider idempotency domains.

## Capabilities

The ingress client cannot inspect worker capabilities. Its defaults describe the
ingress hop (structured sends and ingress idempotency); everything about the
worker is a deployment assertion made on the builder. Unresolved attachment
references remain disabled by default and can be asserted with
`.attachment_references(true)` when the worker has a resolver configured.

## Examples

- The [local ingress client](examples/invoke_local_worker.rs) sends its
  reference-backed attachment to `http://127.0.0.1:8080/restate/call/Email/send`
  by default: `cargo run -p email-transport-restate --example invoke_local_worker`.
  Set `RESTATE_INGRESS_URL` to override the ingress URL and `RESTATE_AUTH_TOKEN`
  to authenticate.
- The [interchangeable transport example](examples/direct_or_restate.rs) sends
  through the same application function with either a direct Resend transport
  or `RestateTransport`: `cargo run -p email-transport-restate --example direct_or_restate`.
  Set `EMAIL_TO` in both modes, `RESTATE_INGRESS_URL` to select Restate (add
  `RESTATE_WAIT=1` to wait for the worker's report, `RESTATE_AUTH_TOKEN` to
  authenticate), or `RESEND_API_KEY` for direct delivery.

## Feature Flags

The default feature set enables the default `reqwest` client stack. Disable
default features to choose `reqwest`'s transport/TLS features explicitly.

- `schemars`: forwards JSON Schema derivation to the re-exported
  `restate-email` queue payload types.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
