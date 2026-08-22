# email-attachment

[![crates.io](https://img.shields.io/crates/v/email-attachment?style=flat-square)](https://crates.io/crates/email-attachment)
[![docs.rs](https://img.shields.io/docsrs/email-attachment?style=flat-square)](https://docs.rs/email-attachment)

**Attachment preparation for provider-neutral outbound email.**

`AttachmentResolver` materializes opaque `AttachmentReference` values into bytes. `MapResolver` supports plain in-memory keys, while `SchemeRouter` dispatches references by prefix and strips `scheme:` plus an optional `//` before calling the selected resolver. `prepare_attachments` applies shared per-attachment and total size limits.

`ResolvingTransport` composes preparation onto any `email-transport` implementation. Byte-only messages use the inner transport's borrowed send path unchanged; reference-backed messages are materialized and delegated as owned messages.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
