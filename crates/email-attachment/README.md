# email-attachment

[![crates.io](https://img.shields.io/crates/v/email-attachment?style=flat-square)](https://crates.io/crates/email-attachment)
[![docs.rs](https://img.shields.io/docsrs/email-attachment?style=flat-square)](https://docs.rs/email-attachment)

**Attachment preparation for provider-neutral outbound email.**

## Quick Start

```rust
use email_attachment::{AttachmentResolver, MapResolver};
use email_message::AttachmentReference;

async fn resolve() -> Result<(), Box<dyn std::error::Error>> {
    let resolver = MapResolver::new().with_entry("report.txt", b"contents".to_vec());
    let attachment = resolver
        .resolve(&AttachmentReference::new("report.txt"))
        .await?;

    assert_eq!(attachment.bytes, b"contents");
    Ok(())
}
```

`AttachmentResolver` materializes opaque `AttachmentReference` values into bytes. `MapResolver` supports plain in-memory keys. `SchemeRouter` dispatches references by their leading scheme (`scheme:value`, with an optional cosmetic `//`) and passes the selected resolver either the stripped value (the default) or the full reference. `FallbackResolver` consults a second resolver only when the first reports an unsupported reference, so unrouted or scheme-less keys can find a home without masking authoritative failures such as not-found. `prepare_attachments` applies shared per-attachment and total size limits.

`AttachmentResolvingTransport` composes preparation onto any `email-transport` implementation. Byte-only messages use the inner transport's borrowed send path unchanged; reference-backed messages are materialized and delegated as owned messages.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
