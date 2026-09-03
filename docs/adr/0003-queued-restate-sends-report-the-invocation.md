# Queued Restate sends report the invocation, not a delivery state

`RestateTransport` defaults to returning as soon as Restate has durably accepted the invocation
(`POST /restate/send/Email/send`, HTTP 202). At that point no provider has seen the message, so
the transport reports Restate itself as the handoff: `SendReport::provider` is `"restate"` and
`provider_message_id` is the Restate invocation id. Waiting for the worker
(`InvocationMode::Sent`, `POST /restate/call/Email/send`) passes the worker's provider report
through unchanged. The mode is a transport default overridable per send through the `"restate"`
provider slice (`RestateSendOptions`).

The kernel's `Transport::send` contract is a *handoff* contract, not a delivery contract: a Resend
`200` and an SMTP `250` are also acknowledgements that someone else now holds the message. A queued
Restate send is the same shape one hop earlier. Encoding that hop in `provider` keeps the result
recoverable (`RestateTransport::invocation_id`) without changing any kernel type.

## Considered options

- **A `Delivered | Queued` return enum on `Transport::send`.** Type-level distinction between a
  provider handoff and a queue handoff. Rejected: it changes eight trait signatures in
  `email-transport`, every adapter and decorator, `ErasedTransport`, the conformance suite, and the
  `SendResponse` wire contract, to express a bit that the `provider` field already carries;
  the distinction it draws ("delivered") overclaims what any transport knows.
- **A `handoff` field on `SendReport`.** Cheap because the struct is `#[non_exhaustive]` and the
  wire form can `serde(default)` it. Deferred: today it would be redundant with `provider`. It is
  the recorded escape hatch if a second queue-backed transport appears and callers need to branch
  on the hop without matching provider strings.
- **Reporting the eventual provider from the queued path.** Impossible; the worker has not run.

## Consequences

- `report.provider` is mode-dependent for the same transport. Callers that need the provider's
  own id must opt into `InvocationMode::Sent` or attach to the invocation later.
- The `"restate"` provider slice is forwarded in the queued payload like every other slice and
  ignored by workers without a Restate-backed transport in their registry. A worker that does
  chain into another `RestateTransport` applies the same mode and delay on the second hop.
- `delay` lives in the provider slice as scheduling behaviour, per ADR 0001: dropping it delivers
  now rather than later, which relaxes nothing that constrains delivery. If scheduling becomes
  cross-provider it moves to core `SendOptions` then.
- `Capabilities` are mode-independent: `idempotency_key` is honored at both hops in both modes
  (see ADR 0004), and `timeout` stays unadvertised because bounding the caller's wait would not
  bound the invocation Restate keeps running.
