# Cloudflare Worker example

A minimal [Cloudflare Worker](https://developers.cloudflare.com/workers/) that sends one message
through [`email-transport-cloudflare`](../../crates/email-transport-cloudflare). It exists for two
reasons:

- as a copy-paste starting point for Worker-hosted code that uses `email_transport::Transport`, and
- as the smoke test for the crate's `wasm32` glue, which CI can only type-check: the `send_email`
  binding does not run outside `workerd`, so the object construction, the `Uint8Array` attachment
  path and the error-code decoding are exercised here against the real platform.

`GET /?to=<address>[&attachment]` sends a text-and-HTML message from the `EMAIL_FROM` variable to
`to`, with a custom `X-Example` header and, when `attachment` is present, a small `hello.txt`. The
response is the `SendReport` on success or the classified `TransportError` (kind, message,
Cloudflare error code, retryable) with a matching HTTP status on failure.

## Prerequisites

- A Cloudflare account on the Workers Paid plan with a domain
  [onboarded to Email Service](https://developers.cloudflare.com/email-service/get-started/send-emails/),
  or a [verified destination address](https://developers.cloudflare.com/email-service/configuration/email-routing-addresses/#destination-addresses)
  to send to before onboarding one.
- [`wrangler`](https://developers.cloudflare.com/workers/wrangler/install-and-update/) and the
  `wasm32-unknown-unknown` target (`rustup target add wasm32-unknown-unknown`). The `[build]`
  command in `wrangler.toml` installs `worker-build` on first use.

## Usage

1. Set `EMAIL_FROM` in [`wrangler.toml`](wrangler.toml) to an address on your onboarded domain.

2. Run locally against the real binding. Remote bindings are needed for the `?attachment` path;
   without them the local simulator cannot serialise binary attachment content. Note that a remote
   binding is the production Email Service: every send below is a real delivery that counts
   against your quota, so use recipients you control.

   ```sh
   wrangler dev --remote
   ```

   or deploy:

   ```sh
   wrangler deploy
   ```

3. Send a message:

   ```sh
   curl 'http://localhost:8787/?to=you@example.com'
   curl 'http://localhost:8787/?to=you@example.com&attachment'
   ```

   A successful send returns the report, for example
   `{"provider":"cloudflare","provider_message_id":"...","accepted":["you@example.com"]}`.

4. Provoke a few failures to see the classification. Sending to an address that is not a verified
   destination before the domain is onboarded fails with `E_RECIPIENT_NOT_ALLOWED`
   (`kind: "authorization"`, HTTP 403); a `from` on a domain Cloudflare does not know returns
   `E_SENDER_NOT_VERIFIED` or `E_SENDER_DOMAIN_NOT_AVAILABLE` (also `authorization`).

## What to look for

If you are changing the transport, the platform-only behaviours worth confirming here are:

- cc-only and bcc-only sends succeed (the transport omits empty recipient lists rather than sending
  `[]`; edit `build_message` to move the recipient to `cc` or `bcc`),
- an attachment sent as a typed array arrives intact,
- the `code` property is present on rejected sends and lands in `provider_error_code`, and
- whether a message without a subject is accepted: the transport sends `""` when none is set, and
  the platform documents `subject` as required but does not say how it treats an empty one (remove
  the `.subject(...)` call in `build_message` to test).

## Type-checking without an account

The crate compiles for the target without any Cloudflare credentials, which is what CI runs:

```sh
cargo check --target wasm32-unknown-unknown --locked
```
