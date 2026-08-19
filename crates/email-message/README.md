# email-message

[![crates.io](https://img.shields.io/crates/v/email-message?style=flat-square)](https://crates.io/crates/email-message)
[![docs.rs](https://img.shields.io/docsrs/email-message?style=flat-square)](https://docs.rs/email-message)

**Typed, provider-neutral outbound email messages and addresses.**

## Quick Start

```rust
use email_message::{Address, Body, Message};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let message = Message::builder(Body::text("Hello"))
        .from_mailbox("sender@example.com".parse()?)
        .to(vec![Address::Mailbox("recipient@example.com".parse()?)])
        .subject("Welcome")
        .build_outbound()?;

    assert_eq!(message.as_message().subject(), Some("Welcome"));
    Ok(())
}
```

## Feature Flags

No features are enabled by default.

- `mime`: exposes `Body::Mime` and `MimePart`; the MIME value types remain available without this feature.
- `serde`: enables serialization for public model types and byte-backed attachments.
- `schemars`: enables JSON Schema support for public model types.
- `arbitrary`: enables `arbitrary::Arbitrary` for fuzzing and property generation.
- `rfc5322-string-compat`: accepts RFC 5322 string address forms in serde and JSON Schema in addition to typed address objects.

See the [crate documentation](https://docs.rs/email-message/latest/email_message/) for API and feature semantics and the [generated feature graph](https://docs.rs/crate/email-message/latest/features) for activation details.

## Scope Contract

- This crate models outbound email content and addresses.
- RFC822 and MIME wire parsing and rendering are provided by [`email-message-wire`](../email-message-wire).
- Provider-specific limits and operational policies belong to transport crates.
- This crate currently requires `std`; its parsing backends and owned parser components are not `no_std`-ready.

## Stability Contract

- `EmailAddress` values are normalized through `addr-spec` during parsing.
- Address display-name formatting may be canonicalized by `Display` and is not guaranteed to preserve source bytes.
- Address and message parse-render round trips preserve semantic values, not raw wire formatting.
- Public enums marked `#[non_exhaustive]` may gain variants in minor releases.

## Metadata Policy

Put provider-agnostic message semantics and outbound headers in `Message`. Provider-specific controls belong in transport crates as typed `TransportOptions`, and new `Message` fields should have stable meaning across structured and raw delivery.

With `serde` enabled, `Message` can be used in queued worker payloads. `AttachmentBody::Reference` represents large attachments that a worker must resolve to bytes before transport delivery; reference resolution intentionally remains outside this crate.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
