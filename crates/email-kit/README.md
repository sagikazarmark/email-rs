# email-kit

[![crates.io](https://img.shields.io/crates/v/email-kit?style=flat-square)](https://crates.io/crates/email-kit)
[![docs.rs](https://img.shields.io/docsrs/email-kit?style=flat-square)](https://docs.rs/email-kit)

**A convenient facade for the email-rs crate family.**

`email-kit` re-exports `email-attachment` as `email_kit::attachment`, `email-message` as `email_kit::message`, and `email-transport` as `email_kit::transport`. Optional features expose the OpenDAL resolver as `email_kit::attachment::opendal`, `email-message-wire` as `email_kit::wire`, the Lettre SMTP adapter as `email_kit::transport::lettre`, and the Resend adapter as `email_kit::transport::resend`.

## Quick Start

```rust
use email_kit::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mailbox: Mailbox = "Mary Smith <mary@example.com>".parse()?;
    assert_eq!(mailbox.email().as_str(), "mary@example.com");

    Ok(())
}
```

Use `email_kit::prelude::*` for common message types and transport traits. When the `wire` feature is enabled, the prelude also includes wire helpers.

## Feature Flags

- `default`: forwards the default features of enabled component crates. For the base facade, `email-transport`'s default enables serde support for both transport and message types; optional wire and provider crates remain disabled.
- `serde`: enables serde support for message and transport types.
- `schemars`: enables JSON Schema support for message and transport types.
- `arbitrary`: enables property-test generation support for message types.
- `attachment-opendal`: exposes OpenDAL attachment resolution through `email_kit::attachment::opendal`.
- `tracing`: enables transport tracing instrumentation.
- `transport-all`: enables every transport currently provided by `email-kit`.
- `transport-lettre`: exposes SMTP support through `email_kit::transport::lettre`.
- `transport-resend`: exposes Resend support through `email_kit::transport::resend`.
- `wire`: exposes RFC822 and MIME parsing and rendering through `email_kit::wire`.

The Lettre adapter, and therefore `transport-all`, does not support `wasm32-unknown-unknown`. Use `transport-resend` when targeting Wasm.

See the [crate documentation](https://docs.rs/email-kit/latest/email_kit/) for API and feature semantics and the [generated feature graph](https://docs.rs/crate/email-kit/latest/features) for activation details.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
