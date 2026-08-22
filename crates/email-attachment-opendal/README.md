# email-attachment-opendal

[![crates.io](https://img.shields.io/crates/v/email-attachment-opendal?style=flat-square)](https://crates.io/crates/email-attachment-opendal)
[![docs.rs](https://img.shields.io/docsrs/email-attachment-opendal?style=flat-square)](https://docs.rs/email-attachment-opendal)

**OpenDAL attachment resolution for email-rs.**

`OpendalResolver` reads attachment paths through an `opendal::Operator` configured at application startup. References cannot select a service, endpoint, bucket, root, or credentials. Each resolver is limited to its operator's configured store and root.

The resolver requires a per-attachment byte limit. It checks metadata before reading when `stat` is available and otherwise reads through a capped reader.

Register multiple resolvers with `email_attachment::SchemeRouter` to use multiple stores. For a resolver registered as `assets`, both `assets:images/logo.png` and `assets://images/logo.png` are passed to the resolver as `images/logo.png`.

## Feature Flags

The default feature forwards OpenDAL's default feature set. The commonly used OpenDAL features `services-azblob`, `services-fs`, `services-gcs`, `services-http`, `services-memory`, and `services-s3` are available under the same names. Operators for all other OpenDAL services are also supported; enable those services on a direct `opendal` dependency when constructing the operator.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://opensource.org/licenses/Apache-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
