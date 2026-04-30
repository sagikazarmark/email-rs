# email-message

Core email address and outbound message model primitives.

## Scope contract

- This crate models outbound email content and addresses.
- RFC822/MIME wire parsing and rendering are provided by `email-message-wire`.
- Provider-specific limits and operational policies belong to transport crates.

## Stability Contract

- `EmailAddress` values are normalized via `addr-spec` during parsing.
- Address display-name formatting may be canonicalized during render (`Display`) and is not guaranteed byte-for-byte equivalent with source text.
- Address/message parse-render roundtrips preserve semantic values (mailbox/group membership and header meaning), but not raw wire formatting details.
- Public enums marked `#[non_exhaustive]` may gain variants in minor releases.

## Feature Flags

- `serde`: enables `Serialize`/`Deserialize` derives for public model types.
- `schemars`: enables `JsonSchema` derives for public model types.
- `arbitrary`: enables `arbitrary::Arbitrary` derives to support fuzz/property generation.

## std support

This crate currently requires `std`.

### Why no `no_std` today?

- Parsing backends currently depend on `std`.
- The crate uses owned strings/collections and parser components that are not `no_std`-ready.

## MIME model

- Enable the `mime` feature to use `Body::Mime` and `MimePart`.
- MIME parsing/rendering is provided by `email-message-wire`.

## Wire format support

- RFC822/MIME parsing and rendering are provided by `email-message-wire`.

## Metadata policy

- Put provider-agnostic message semantics in `Message`.
  Examples: `Date`, `Message-ID`, `Sender`, recipients, body, attachments.
- Put arbitrary outbound headers in `Message.headers` when they should survive both SMTP and structured API paths.
- Put provider-specific controls in transport crates via typed `TransportOptions`, not in the message model.
- Avoid adding fields to `Message` unless they have clear cross-provider meaning and stable semantics across structured and raw delivery.

## Worker-friendly payloads

- With the `serde` feature enabled, `Message` is a good serializable content payload inside a queued worker envelope such as `email_worker::EmailJob`.
- `AttachmentBody::Bytes` is still the direct send/render form.
- `AttachmentBody::Reference(AttachmentReference)` is available for large attachments that should be dereferenced by a worker before transport delivery.
- Reference resolution policy belongs outside this crate so the core model stays transport-agnostic.

## Development

- Run tests: `cargo test -p email-message --all-features`
- Run clippy: `cargo clippy -p email-message --all-targets --all-features -- -D warnings`
- Run benches: `cargo bench -p email-message`
