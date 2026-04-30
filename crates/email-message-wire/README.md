# email-message-wire

RFC822 and MIME parsing/rendering for `email-message`.

## Scope contract

- Parses RFC822 messages into the typed `email_message::Message` model.
- Renders typed messages back into RFC822/MIME wire format.
- Handles MIME structure, attachment encoding, RFC2047 encoded words, and related transport-safe formatting rules.

## Key behaviors

- `render_rfc822` strips `Bcc` by default so structured messages can be safely rendered for SMTP or raw delivery.
- `render_rfc822_with` and `RenderOptions` are available when a caller intentionally needs a non-default wire rendering policy, such as including `Bcc`.
- MIME attachments are base64-encoded and wrapped to RFC-compliant line lengths.
- Typed `Date` and `Message-ID` values round-trip through the public `Message` API.
- Attachment references must be resolved to bytes before rendering.

## Development

- Run tests: `cargo test -p email-message-wire`
- Check wasm compatibility: `cargo check -p email-message-wire --target wasm32-unknown-unknown`
