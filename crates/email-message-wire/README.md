# email-message-wire

[![crates.io](https://img.shields.io/crates/v/email-message-wire?style=flat-square)](https://crates.io/crates/email-message-wire)
[![docs.rs](https://img.shields.io/docsrs/email-message-wire?style=flat-square)](https://docs.rs/email-message-wire)

**RFC822 and MIME parsing and rendering for `email-message`.**

This crate has no optional feature flags.

## Quick Start

```rust
use email_message_wire::{parse_rfc822, render_rfc822};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let raw = b"From: from@example.com\r\nTo: to@example.com\r\n\r\nHello";
    let message = parse_rfc822(raw)?;
    let rendered = render_rfc822(&message)?;

    assert!(!rendered.is_empty());
    Ok(())
}
```

## Scope Contract

- Parses RFC822 messages into the typed `email_message::Message` model.
- Renders typed messages as RFC822 and MIME wire data.
- Handles MIME structure, attachment encoding, RFC2047 encoded words, and related transport-safe formatting.
- Enables `email-message`'s `mime` feature because full MIME-tree rendering requires `MimePart`.

## Key Behaviors

- `render_rfc822` strips `Bcc` by default so messages are safe for SMTP or raw delivery.
- `render_rfc822_with` and `RenderOptions` allow intentional non-default policies such as retaining `Bcc`.
- MIME attachments are base64-encoded and wrapped to RFC-compliant line lengths.
- Typed `Date` and `Message-ID` values round trip through the public `Message` API.
- Attachment references must be resolved to bytes before rendering.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
