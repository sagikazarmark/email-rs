# email-rs

[![ci](https://img.shields.io/github/actions/workflow/status/sagikazarmark/email-rs/ci.yaml?style=flat-square&label=ci)](https://github.com/sagikazarmark/email-rs/actions/workflows/ci.yaml)
[![openssf scorecard](https://api.securityscorecards.dev/projects/github.com/sagikazarmark/email-rs/badge?style=flat-square&label=openssf%20scorecard)](https://securityscorecards.dev/viewer/?uri=github.com/sagikazarmark/email-rs)
[![crates.io](https://img.shields.io/crates/v/email-kit?style=flat-square)](https://crates.io/crates/email-kit)
[![docs.rs](https://img.shields.io/docsrs/email-kit?style=flat-square)](https://docs.rs/email-kit)

**Provider-neutral email modeling and delivery for Rust.**

## Features

- **Typed messages:** model addresses, bodies, attachments, envelopes, and outbound messages independently of any provider.
- **Transport abstraction:** send structured messages or rendered RFC822 through a common transport contract.
- **Wire support:** parse and render RFC822 and MIME messages.
- **Provider adapters:** deliver through SMTP with Lettre or through Resend.
- **Durable workers:** expose email delivery through Restate service contracts and a runnable endpoint.

## Workspace Crates

- [`email-kit`](crates/email-kit): flagship facade for the message, transport, wire, and provider crates.
- [`email-message`](crates/email-message): typed outbound message and address model.
- [`email-message-wire`](crates/email-message-wire): RFC822 and MIME parsing and rendering.
- [`email-transport`](crates/email-transport): provider-neutral transport traits and send-time options.
- [`email-transport-lettre`](crates/email-transport-lettre): Lettre SMTP transport adapter.
- [`email-transport-resend`](crates/email-transport-resend): Resend transport adapter.
- [`email-transport-test`](crates/email-transport-test): unpublished test transports and conformance helpers.
- [`restate-email`](crates/restate-email): Restate worker contracts and service adapter.
- [`restate-email-endpoint`](crates/restate-email-endpoint): standalone endpoint hosting the Restate email service.

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

CI also verifies Rust 1.92, the documented Wasm targets, default and no-default
configurations, and selected named features independently.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
