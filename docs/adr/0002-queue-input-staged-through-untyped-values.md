---
status: superseded
---

# Queue handler input is staged through an untyped value tree

`SendOptions` intentionally has no `Deserialize` implementation: provider-specific options live in
a `TypeId`-keyed map, so decoding them requires a `TransportOptionRegistry` and must go through
`SendOptionsSeed`. Restate's handler input decoding is an associated function —
`Deserialize::deserialize(bytes: &mut Bytes)`, no `&self` — so the generated handler cannot reach
`ServiceImpl::transport_options` to drive that seed. `RawSendOptions` bridges the two: it decodes
eagerly into `BTreeMap<String, serde_value::Value>` at the handler boundary, and the seed runs
afterwards inside `ServiceImpl::send_request`.

This is a workaround for an upstream limitation, not a design goal. It is recorded here because a
reader who knows `email-transport` ships a perfectly good `DeserializeSeed` will otherwise wonder
why the queue does not simply use it. Upstream discussion is
[restatedev/sdk-rust#109](https://github.com/restatedev/sdk-rust/issues/109), where the maintainers
suggested building this over the public `Service + Discoverable` traits rather than changing the
SDK.

## Consequences

- `RawSendOptions` mirrors `SendOptions` field by field and is kept in sync by hand. There is no
  drift guard, so a new `SendOptions` field is silently dropped in both directions on the queue
  path — unlike `SendOptionsSeed`, which is guarded by an exhaustive destructure in its round-trip
  test.
- Payloads are decoded twice, and `serde-value` appears in `restate-email`'s public API, making a
  bump there a breaking change for this crate.
- Seed errors are produced against a value tree, so they carry no JSON line, column, or path when
  they surface as a terminal 400.

## Supersession

Both `restate_sdk::serde::Deserialize` and `PayloadMetadata` are static-function traits, so a
userland `SeededJson<T>` wrapper now captures the request bytes, delegates discovery metadata to
`Json<T>`, and decodes through `SendRequestSeed` inside the handler where the registry is in scope.
This removed the staging type, the double decode, and the `serde-value` dependency without an SDK
change.
