# The idempotency key is honored at every hop

`SendOptions::idempotency_key` travels with a queued send at both hops. `RestateTransport` sends it
as Restate's `idempotency-key` ingress header *and* leaves it in the queued `SendOptions`; the
worker forwards the queued options to the selected transport unchanged, so a provider that
advertises `Capabilities::idempotency_key` receives the same key it would have received from a
direct send. Restate deduplicates the caller's retries of the ingress call; the provider
deduplicates the worker's retries of the provider call.

The second hop needs its own protection because `ctx.run` is at-least-once: a worker that crashes
after the provider accepted the message but before the run result is journaled re-executes the
provider call on the next attempt. Restate's ingress idempotency cannot see that window. Only a
provider-level key can, and for providers without one (SMTP) the option is ignored by the
`Capabilities` contract, so forwarding costs nothing there.

This supersedes the previous behavior, in which the client stripped the key from the body "so the
provider does not receive the same key". The concern was reusing one key across two idempotency
domains. The domains are independent (Restate scopes keys to a service handler, a provider to an
account), Restate short-circuits any replay before the worker runs, so the provider only ever
sees the key from the worker's own attempts, and a caller that reuses a key across unrelated sends
has the same problem with a direct transport. Nothing new is introduced by carrying the key
through.

## Considered options

- **Restate hop only** (the previous behavior). Rejected: leaves the at-least-once window of
  `ctx.run` uncovered, and made `Capabilities::idempotency_key` false for `RestateTransport`,
  which advertised "forwards to the provider" while consuming the key itself.
- **Provider hop only** (no ingress header). Rejected: a caller retrying a timed-out enqueue
  would create a second invocation; the provider might deduplicate the eventual delivery, but the
  caller would hold two invocation ids and the second invocation would still run.
- **Separate keys per hop** (e.g. a `restate` provider slice carrying an ingress-only key).
  Rejected: doubles the caller's bookkeeping for no isolation benefit, see above.

## Consequences

- `SendRequest` carries `options.idempotency_key` on the wire exactly as `SendOptions` serializes
  it. The client no longer needs a body shape that differs from the contract type, and
  `SendOptions::serializable_without_idempotency_key` is removed from `email-transport`.
- Callers invoking `Email.send` through ingress without `RestateTransport` should set both the
  header and the body field, as the `invoke_local_worker` example does. Setting only the body
  field gives provider deduplication but no ingress deduplication.
- `Capabilities::idempotency_key` on `RestateTransport` keeps its kernel meaning: the key reaches
  the provider, subject to the worker's transport advertising it, which is a deployment assertion
  like every other worker capability.
- A `PreviouslyAccepted` replay returns the original invocation id, so the `provider_message_id`
  reported under ADR 0003 is stable for retries within Restate's idempotency retention. After the
  retention window a retry creates a new invocation; the provider-side key then decides whether a
  second email goes out.
