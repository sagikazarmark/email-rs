# restate-email

Restate email worker contracts and service adapter.

## Scope contract

- Defines a serializable send-email request payload around `email_message::Message`.
- Dispatches messages through keyed `email_transport::Transport` registrations.
- Exposes the payload as a Restate `Email.send` service handler.
- Maps worker and transport failures into Restate retry semantics.

## Key types

- `SendRequest`: transport reference plus message payload and raw send-time options.
- `RawSendOptions`: wire-safe send-time metadata, including envelope overrides, timeout, idempotency key, correlation id, and provider-specific `transport_options` staged as `serde-value` values.
- `TransportResolver`: resolves a transport key into a configured `Transport` instance.
- `StaticTransportRegistry`: small owned registry for common fixed-key worker setups.
- `ServiceImpl`: concrete Restate service wrapper over a `TransportResolver` and provider-option registry. The handler hydrates `RawSendOptions` into `email_transport::SendOptions` and dispatches inside a named Restate `ctx.run` journaled action.
- `SendResponse`: serializable response wrapper containing the transport `SendReport`.

## Features

- `schemars`: derives JSON Schema for public queue payload types such as `SendRequest`, reusing the schema from `email-message` and `email-transport`.
- `resend`: enables email-kit's Resend provider-option deserialization for queued `transport_options` entries.

## Retry behavior

- Retryable transport failures remain retryable Restate handler failures.
- Unknown transport keys, validation errors, and other permanent failures are mapped to terminal Restate errors.

## Example

- Run the local example server: `cargo run -p restate-email --example restate_email_worker`
- The example starts `HttpServer::new(service.endpoint())` on `127.0.0.1:9080`.
- It prints a sample `SendRequest` JSON payload and the Restate ingress handler path `POST /Email/send`.
- The raw SDK endpoint on `127.0.0.1:9080` is meant to be registered behind Restate; it is not a plain JSON handler by itself.
- Invoke the local example server: `cargo run -p restate-email --example invoke_local_worker`
- The client example posts a sample `SendRequest` to `http://127.0.0.1:8080/Email/send` by default, assuming a local Restate ingress.
- Override the target with `RESTATE_INGRESS_URL=http://host:port` if needed.
- Run the Resend-backed example server: `cargo run -p restate-email --example restate_resend_worker`
- The Resend example expects `RESEND_API_KEY`, `RESEND_FROM`, and `RESEND_TO` and starts on `127.0.0.1:9081`.

## Development

- Run tests: `cargo test -p restate-email`
- Run the Dagger test wrapper: `dagger call test`
- Run the Dagger Restate ingress e2e: `dagger call restate-ingress`
- The Rust test suite covers raw SDK protocol handling without starting Docker. Restate ingress e2e coverage runs through the Dagger Restate service module so `cargo test` remains deterministic.
