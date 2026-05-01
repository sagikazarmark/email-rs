# email-kit

Convenience facade for the email-rs crates.

`email-kit` re-exports:

- `email-message` as `email_kit::message`
- `email-transport` as `email_kit::transport`
- `email-message-wire` as `email_kit::wire` when the `wire` feature is enabled

Use `email_kit::prelude::*` for common message types and transport traits. With the `wire` feature enabled, the prelude also includes wire helpers.

## Feature Flags

- `default`: enables `serde` support.
- `serde`: enables serde support for message and transport types.
- `schemars`: enables JSON Schema support for message and transport types.
- `arbitrary`: enables property-test generation support for message types.
- `tracing`: enables `email-transport` tracing instrumentation.
- `wire`: enables RFC822/MIME parsing and rendering through `email-message-wire`.

## Development

- Run tests: `cargo test -p email-kit --all-features`
- Run clippy: `cargo clippy -p email-kit --all-targets --all-features -- -D warnings`
