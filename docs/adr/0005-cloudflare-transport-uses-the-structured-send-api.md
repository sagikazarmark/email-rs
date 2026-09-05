# The Cloudflare transport uses the structured send API and drops platform-owned headers

`CloudflareTransport` maps an `email_message::Message` to Cloudflare's structured `send()` API
(`EmailMessageBuilder`) and dispatches it through the `worker` crate's `SendEmail` binding. It
builds the JS object by hand from a plain-Rust payload, forwards only the message's custom
headers, exposes no transport option type, and is surfaced through `email-kit` under
`transport-all-wasm` only.

The structured API is the one path that carries what the kernel models: a single call delivers to
every `To`/`Cc`/`Bcc` recipient, display names travel as `{ name, email }` objects, byte
attachments go as typed arrays, and Cloudflare returns exactly one `messageId`, which becomes
`SendReport::provider_message_id`. The legacy raw `EmailMessage` path cannot do this (see below).

The `worker` 0.8 generated builder (`SendEmailBuilder::builder*`) types recipients as bare
`&str`/`&[String]` and requires a `to` value, whereas the runtime's own type definitions
(`workerd/types/defines/email.d.ts`, `EmailDestinations`) make each of `to`, `cc` and `bcc`
optional, accept `(string | EmailAddress)[]` for all three, and require only that at least one is
present; the REST OpenAPI schema says the same (`to`: "Optional if cc or bcc is provided"). The
transport therefore constructs the object with `js_sys::Object`/`Reflect::set` and casts it to
`SendEmailBuilder` for `send_with_builder`, omitting whichever recipient lists are empty. Named
addresses use `worker::EmailAddress`; unnamed ones stay strings.

The same runtime types define attachments as a discriminated union: `disposition: 'inline'`
requires `contentId`, `disposition: 'attachment'` forbids it, and both require `filename`. The
transport rejects an inline attachment without a content id locally (`Validation`), drops a
content id on a regular attachment, and synthesises a missing filename (`attachment-N` from the
1-based position, or the content id for an inline part) instead of rejecting it. Every other
transport in the workspace treats the filename as optional, the RFC 822 renderer simply omits the
parameter, and inline `cid:` images very commonly have none; failing here would make Cloudflare the
one hop where an otherwise portable message breaks. No extension is guessed: the `type` field
carries the MIME type, and a partial MIME-to-extension table would be worse than an honest
placeholder.

The kernel's `standard_message_headers` helper is deliberately not used. Cloudflare rejects `Date`
and `Message-ID` with `E_HEADER_NOT_ALLOWED`, stamps its own `Message-ID`, and runs an allowlist
for everything else. The message's `date`, `message_id` and `sender` are dropped, and the drop is
documented rather than validated: a caller who sets `message_id` on a message destined for
several transports should not have the Cloudflare hop fail because Resend would have accepted it.

There is no `CloudflareSendOptions`. The send API carries nothing per-send beyond what
`email_message::Message` already models, so a transport option type would be an empty struct with
a `serde` feature to forward and a registry entry to maintain. The `"cloudflare"` provider key is
reserved for `SendReport::provider` and any future option type (ADR 0001).

## Considered options

- **The legacy raw `EmailMessage` path** (`new EmailMessage(from, to, raw)`), rendering RFC 822
  with `email-message-wire`. Rejected: it takes a single envelope recipient per call, so a
  multi-recipient message becomes N sends with N message ids and no single
  `provider_message_id`; `Cc`/`Bcc` recipients are not delivered unless each is fanned out; and
  the envelope `from` must equal the header `From`, so `Sender`/`Return-Path` semantics are lost.
  A `RawTransport` over this path remains possible later.
- **The generated typed builder as-is.** Compiles without `js_sys` glue. Rejected: it loses
  display names on every recipient field and forbids cc/bcc-only sends, both of which the
  platform supports and both of which every other structured transport in the workspace honours.
- **Passing `Name <addr>` strings** into the generated builder to keep display names. Rejected:
  the Workers API documents `string` as a bare address and `EmailAddress` as the named form;
  RFC 5322 name-addr strings are undocumented for this API and may be rejected or mangled.
- **Forwarding `Date`/`Message-ID`/`Sender` and letting the platform reject them.** Rejected: the
  kernel emits them for every message that sets the fields, so the default outcome would be
  `E_HEADER_NOT_ALLOWED` on messages that succeed everywhere else.
- **Rejecting a filename-less attachment locally.** Explicit, but stricter than SMTP and Resend
  for a field MIME itself treats as optional. Rejected in favour of a positional placeholder.
- **Rejecting a content id on a regular attachment.** Would mirror the multi-`Reply-To` rule,
  but the recipient's client renders the part identically with or without the id, so there is
  nothing lost to warn about. Rejected in favour of dropping it.
- **Including the transport in `email-kit/transport-all`.** Rejected: the transport is
  runtime-bound to Cloudflare Workers, and `transport-all` is the default feature of
  `restate-email-endpoint`; including it would compile `worker`, `wasm-bindgen`, `js-sys` and
  `web-sys` into every native consumer for a transport that can never send there.
- **A `transport-cloudflare` passthrough on `restate-email` and a `transport-all-wasm` feature
  on `restate-email-endpoint`.** Both existed briefly. Rejected: `restate-email`'s `Service` and
  the endpoint binary depend on `restate-sdk`, which does not build for `wasm32`, so neither can
  ever run inside a Worker. The features only pulled the `worker` dependency tree into native
  builds where `send` returns `UnsupportedFeature`, and there was no transport option type for
  them to register. `email-kit/transport-all-wasm` remains the aggregate for Worker-hosted code.

## Consequences

- `Capabilities` advertises structured send, custom headers, attachments and inline attachments;
  `idempotency_key` and `timeout` are false and the options are accepted and ignored per the
  capability contract (ADR 0004). `custom_envelope` is false because the platform controls
  `Return-Path`.
- The crate compiles on native targets so `cargo test --workspace` stays green; `send` returns
  `ErrorKind::UnsupportedFeature` there instead of reaching wasm-bindgen's panicking extern
  stubs. The native stub is load-bearing rather than cosmetic: `worker` and the glue would compile
  off-wasm, but `send_with_builder` awaits a `!Send` `js_sys::JsFuture`, and `Transport::send`
  requires a `Send` future on native, so without the stub the `Transport` impl could not exist
  there. Message mapping and error classification are pure functions unit-tested on the host.
  `Transport::send` is a thin composition of those around the binding call and deliberately has
  no seam for a test double: the machinery to inject one (a sender trait, an intermediate error
  type, a duplicated binding handle) outweighed the value of unit-testing the wrapper. `send`
  itself, the wasm-bindgen glue and the `Env` lookup behind `from_env` are type-checked for
  `wasm32-unknown-unknown` in CI only; `examples/cloudflare-worker` is the deployable smoke test
  that exercises them against the real platform.
- The transport exposes no accessor for the underlying `SendEmail`; callers that need the
  binding elsewhere keep their own handle before constructing the transport.
- Error classification is owned by a single code table keyed on the JS error's `code` property,
  read directly rather than through `worker::Error`, so the upstream `RCPT_NOT_ALLOWED` vs
  `E_RECIPIENT_NOT_ALLOWED` spelling is handled in one place. Unknown codes are
  `PermanentProvider`; a code-less JS error is `Internal`; `http_status` is never set.
- No JS value is attached as a `TransportError` source; code and message are carried instead,
  because `JsValue` is not reliably `Send + Sync` across wasm-bindgen configurations.
- Repeated custom header names collapse to the last value because Cloudflare's `headers` field is
  a plain object, the same last-wins rule Resend applies today. Going one step further than Resend,
  names are compared case-insensitively (RFC 5322 §2.2) and the first spelling is kept, so `X-Foo`
  and `x-foo` never reach the platform as two properties with undefined precedence.
- Neither `restate-email` nor `restate-email-endpoint` exposes a Cloudflare feature; the Restate
  worker is native-only and cannot host the binding.
