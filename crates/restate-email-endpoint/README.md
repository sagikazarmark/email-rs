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
identity_keys = ["publickeyv1_w7YHemBctH5Ck2nQRQ47iBBqhNHy4FV7t2Usbye2A6f"]

[transports.transactional]
provider = "resend"
api_key = "re_..."

[attachments]
max_attachment_bytes = 26214400
max_total_bytes = 41943040

[attachments.resolvers.docs]
type = "opendal"
service = "s3"
bucket = "example-docs"
root = "/outbound"
```

```sh
restate-email --config restate-email.toml --port 9080
```

The Restate service name is `Email`. Invoke its `send` handler with a `restate_email::SendRequest` whose transport key matches a configured entry. In the example above, an attachment reference such as `docs:reports/quarterly.pdf` is resolved as `reports/quarterly.pdf` within the configured S3 bucket and root before Resend receives the message.

The `[attachments]` section is optional. When present, every configured transport is wrapped with attachment preparation. The resolver key is the reference routing prefix, and all fields other than `type` and `service` are passed to the selected OpenDAL service as operator options. Supported services include `azblob`, `fs`, `gcs`, `http`, and `s3`.

With the `transport-lettre` feature (also enabled by `transport-all`), SMTP transports are configured through a Lettre connection URL:

```toml
[transports.notifications]
provider = "smtp"
url = "smtps://user:password@smtp.example.com:465"
```

The URL carries credentials, host, port, EHLO name, and TLS mode (`smtp://`, `smtps://`, and the `?tls=` query parameter) as documented by Lettre's `AsyncSmtpTransport::from_url`.

Without `[attachments]`, byte-backed messages work unchanged and provider transports reject unresolved references terminally. Missing objects, unsupported references, access failures, and size violations are terminal; transient storage failures are retryable. Resolution occurs during the existing `send_email` Restate action, and resolved bytes are not journaled. Use immutable or versioned references when retries must resolve identical content.

## Request Identity

Restate signs every request it makes to a service endpoint when the runtime is
configured with a request identity key. `identity_keys` lists the matching
`publickeyv1_...` public keys; with at least one key configured the endpoint
rejects unsigned requests. Multiple keys stay valid at once, so rotation is a
config change: add the new key, switch the runtime to the new private key, then
drop the old one. The environment override accepts a comma-separated list:

```sh
RESTATE_EMAIL_IDENTITY_KEYS="publickeyv1_old,publickeyv1_new" restate-email --config restate-email.toml
```

Without `identity_keys` the endpoint accepts unsigned requests. Identity keys
authenticate the Restate runtime to this endpoint; callers authenticate to
Restate ingress separately (see
[`email-transport-restate`](../email-transport-restate)).

## Feature Flags

The default build is batteries-included: `minimal` plus `transport-all`. For a slimmer binary, disable default features and pick `minimal` plus the transports you need.

- `minimal`: the component defaults, queue-payload schemas, RFC 5322 string-address compatibility, and tracing. Every build wants this.
- `attachment-opendal`: enables endpoint attachment preparation and the OpenDAL services listed above. It is enabled by `transport-all`. Without it, `[attachments]` limits still apply to byte-backed attachments, but resolvers cannot be configured.
- `transport-lettre`: enables the SMTP transport and its `provider = "smtp"` endpoint configuration. It is enabled by `transport-all`.
- `transport-resend`: enables the Resend transport and provider-option deserialization. It is enabled by `transport-all`.
- `transport-all`: enables attachment preparation and every transport exposed by `email-kit`.

The binary target requires at least one transport feature and fails the build with a clear error without one:

```sh
cargo install restate-email-endpoint --no-default-features --features minimal,transport-lettre
```

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
