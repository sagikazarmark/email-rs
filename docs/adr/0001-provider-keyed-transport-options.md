# Transport options cross the queue as a best-effort provider-keyed bag

Transports are selected at delivery time by a runtime key from a queue payload, so the receiving
binary cannot statically type the provider-specific options that travel with a request. We
therefore carry them as a provider-keyed object on the wire — `transport_options: {"resend": {…}}` —
with **best-effort union semantics**: a caller may set options for every provider it cares about,
the selected transport takes its own slice, and everything else is ignored. This keeps the
transport swappable by deployment configuration without the caller rewriting its request.

What is locked is the *wire shape*. The Rust mechanism behind it (the `TypeId`-keyed
`TransportOptions` map, `TransportOptionRegistry`, and the `DeserializeSeed` hydration path) is an
implementation detail and may be replaced. `TransportOption::provider_key` is deliberately explicit
rather than derived from `TypeId` or `type_name`, which is precisely what allows that replacement
without moving the contract.

## Considered options

- **A closed `enum ProviderOptions` in the contract crate.** Fully typed, exact JSON schema, no
  registry. Rejected: it forces every provider into the core crate, makes each new provider a
  breaking change to a central type, and ends support for out-of-tree adapters.
- **An associated `Transport::Options` type.** Fully typed with no dynamism. Rejected: it is
  incompatible with a heterogeneous transport registry resolved by a runtime key, which is the
  whole point of the queue.
- **Raw JSON slots handed to each adapter.** No registry, no `Any`. Rejected: it would force the
  direct in-process path — where callers hold typed option structs and never touch serde — to
  serialize through JSON to set an option.

## Consequences

- **Unregistered provider keys are ignored, not rejected.** A payload may name providers a given
  worker was not compiled with; that is the forwarding case, not an error. A malformed value under
  a *registered* key stays terminal.
- **Switching transports may silently drop behavior.** This is the accepted trade, and it matches
  the advisory posture of `Capabilities`. It also draws a line: an option that *constrains* rather
  than relaxes — a sandbox flag, a suppression-list toggle, a "never deliver to real recipients"
  switch — must not live in this bag. Silently dropping a safety control on a transport switch
  turns it into a no-op while still reporting success. Constraining controls belong in core
  `SendOptions`, where every transport must honor them or fail.
- **The discovery schema for `transport_options` stays open** (`additionalProperties: true`) and is
  not derived from either registry. Deriving it from configured transports would turn deployment
  configuration into wire contract: the same code would advertise different contracts per
  deployment, config-only edits would churn the schema, and generated clients would break when
  operations renames a transport key. Unknown keys are rejected at runtime instead (404 for an
  unknown transport key, 400 for a malformed option).
- **The dynamic step is closed over a startup-registered set.** Wire data selects among decoders
  registered at startup; it can never cause instantiation of a type nobody registered. This is the
  same posture as the OpenDAL attachment resolver, which can only read stores configured at
  startup.
- **A `SendRequest` is a privileged payload.** Anyone who can reach ingress already controls
  sender, recipients, and body, so the option bag adds no meaningful escalation — but ingress must
  be treated as an authenticated internal surface.
