# restate-email-endpoint

[![crates.io](https://img.shields.io/crates/v/restate-email-endpoint?style=flat-square)](https://crates.io/crates/restate-email-endpoint)
[![docs.rs](https://img.shields.io/docsrs/restate-email-endpoint?style=flat-square)](https://docs.rs/restate-email-endpoint)

**Standalone endpoint hosting the email service for [Restate](https://restate.dev/).**

## Install

```sh
cargo install restate-email-endpoint
```

## Configuration

The `restate-email` binary reads a JSON, YAML, or TOML configuration file and applies `RESTATE_EMAIL_` environment overrides. At least one transport must be configured.

```toml
[transports.transactional]
provider = "resend"
api_key = "re_..."
```

```sh
restate-email --config restate-email.toml --port 9080
```

The Restate service name is `Email`. Invoke its `send` handler with a `restate_email::SendRequest` whose transport key matches a configured entry.

## Feature Flags

Default features enable the component defaults, queue-payload schemas, RFC 5322 string-address compatibility, and all available transports.

- `transport-all`: enables every transport exposed by `email-kit`; currently this is Resend. The binary target requires this feature.

See the [generated feature graph](https://docs.rs/crate/restate-email-endpoint/latest/features) for activation details.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
