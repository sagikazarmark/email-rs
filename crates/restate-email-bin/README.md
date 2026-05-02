# restate-email-bin

Runnable Restate email worker binary for `restate-email`.

## Configuration

The binary reads a JSON, YAML, or TOML config file plus `RESTATE_EMAIL_` environment overrides.

```toml
[transports.transactional]
provider = "resend"
api_key = "re_..."
```

Run locally:

```sh
cargo run -p restate-email-bin -- --config restate-email.toml --port 9080
```

The Restate service name is `Email`; invoke `send` with a `restate_email::SendRequest` payload whose `transport` matches one of the configured transport keys.
