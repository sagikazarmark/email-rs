# email-rs

[![ci](https://img.shields.io/github/actions/workflow/status/sagikazarmark/email-rs/ci.yaml?style=flat-square&label=ci)](https://github.com/sagikazarmark/email-rs/actions/workflows/ci.yaml)
[![openssf scorecard](https://api.securityscorecards.dev/projects/github.com/sagikazarmark/email-rs/badge?style=flat-square&label=openssf%20scorecard)](https://securityscorecards.dev/viewer/?uri=github.com/sagikazarmark/email-rs)
[![crates.io](https://img.shields.io/crates/v/email-kit?style=flat-square)](https://crates.io/crates/email-kit)
[![docs.rs](https://img.shields.io/docsrs/email-kit?style=flat-square)](https://docs.rs/email-kit)

**Provider-neutral email modeling and delivery for Rust.**

## Features

- **Typed messages:** model addresses, bodies, attachments, envelopes, and outbound messages independently of any provider.
- **Attachment adapters:** resolve attachment paths from any OpenDAL-backed storage service.
- **Transport abstraction:** send structured messages or rendered RFC822 through a common transport contract.
- **Wire support:** parse and render RFC822 and MIME messages.
- **Provider adapters:** deliver through SMTP with Lettre, through Resend, or through a Cloudflare Workers `send_email` binding.
- **Durable workers:** expose email delivery through Restate service contracts and a runnable endpoint.

## Workspace Crates

- [`email-attachment`](crates/email-attachment): attachment resolution, preparation, and transport decoration.
- [`email-attachment-opendal`](crates/email-attachment-opendal): attachment resolution through configured OpenDAL storage operators.
- [`email-kit`](crates/email-kit): flagship facade for the message, transport, wire, and provider crates.
- [`email-message`](crates/email-message): typed outbound message and address model.
- [`email-message-wire`](crates/email-message-wire): RFC822 and MIME parsing and rendering.
- [`email-transport`](crates/email-transport): provider-neutral transport traits and send-time options.
- [`email-transport-cloudflare`](crates/email-transport-cloudflare): Cloudflare Workers `send_email` transport adapter.
- [`email-transport-lettre`](crates/email-transport-lettre): Lettre SMTP transport adapter.
- [`email-transport-resend`](crates/email-transport-resend): Resend transport adapter.
- [`email-transport-restate`](crates/email-transport-restate): Restate ingress transport adapter.
- [`email-transport-test`](crates/email-transport-test): unpublished test transports and conformance helpers.
- [`restate-email`](crates/restate-email): Restate worker contracts and service adapter.
- [`restate-email-endpoint`](crates/restate-email-endpoint): standalone endpoint hosting the Restate email service.

## Examples

- [`examples/restate-endpoint`](examples/restate-endpoint): Docker Compose stack running the Restate email endpoint against Restate, a Mailpit mock SMTP server, and RustFS-backed attachment resolution.
- [`examples/cloudflare-worker`](examples/cloudflare-worker): Cloudflare Worker sending through the `send_email` binding with `email-transport-cloudflare`; the runtime smoke test for the crate's `wasm32` glue.

## Development

CI runs the following workspace checks:

```sh
cargo build --workspace --all-targets --all-features --locked
cargo test --workspace --all-features --locked
cargo bench --workspace --all-features --locked
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo audit
cargo deny check
```

The configured Dagger module exposes its complete local check set through:

```bash
dagger check
```

This includes an end-to-end test of the Restate email endpoint ([`.dagger/modules/end-to-end`](.dagger/modules/end-to-end)):
it builds the `restate-email` binary, runs it alongside a Restate server and a [Mailpit](https://mailpit.axllent.org/) SMTP
sink as Dagger services, registers the deployment, then sends an email through the Restate ingress and asserts that Mailpit
received it (a [Hurl](https://hurl.dev/) scenario). Run it on its own with:

```bash
dagger check end-to-end:send
```

The `restate` and `hurl` Dagger modules under [`.dagger/modules`](.dagger/modules) are vendored copies from
[daggerverse-beta](https://github.com/sagikazarmark/daggerverse-beta) carrying fixes that are yet to be upstreamed; see the
header comment in each module.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
