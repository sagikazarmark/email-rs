# email-rs

A provider-agnostic outbound email kernel: typed messages and a transport seam that the same
application code can drive directly or through a durable queue.

## Language

### Delivery

**Transport**:
A destination an outbound message can be handed to for delivery: a provider adapter, a decorator
around one, or a durable queue that eventually reaches one.
_Avoid_: mailer, sender, driver

**Transport key**:
The deployment-chosen name that selects one configured transport for a request, such as
`transactional` or `marketing`. It is deployment topology, not part of the message contract.
_Avoid_: transport name, provider

**Provider key**:
The stable wire identifier for one provider's option shape, such as `resend`. It names a provider's
vocabulary, never a deployment's configuration.
_Avoid_: transport key, adapter name

**Transport option**:
A provider-specific escape hatch attached to a single send, addressed by provider key. Callers may
attach options for several providers at once; the selected transport takes only its own.
_Avoid_: provider option, provider metadata

**Capability**:
An advisory statement about what a transport supports. Capabilities describe intent, they do not
enforce it.
_Avoid_: feature flag, support flag

**Invocation mode**:
How far a Restate-backed transport follows a send before returning: `Queued` (accepted by
Restate, the report carries the invocation id) or `Sent` (handed to the provider, the report is
the worker's). It is a Restate-specific transport option, not a message property.
_Avoid_: sync/async, blocking, fire-and-forget, one-way/request-response

### Attachments

**Attachment reference**:
An opaque, resolver-interpreted string standing in for attachment content that is not yet
materialized. It may be a URI, a plain key, or a provider identifier; the core never parses it.
_Avoid_: URL, attachment URI, link

**Attachment resolver**:
The seam that turns an attachment reference into bytes. Which stores a resolver can reach is fixed
when it is configured, never by the reference itself.
_Avoid_: fetcher, loader, downloader

**Attachment preparation**:
Materializing every reference-backed attachment in a message into bytes before delivery. It is
materialization, not an availability check; the send itself is the availability check.
_Avoid_: resolution pass, hydration
