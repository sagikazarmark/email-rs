//! Restate-specific per-send options carried through `SendOptions::transport_options`.

use std::time::Duration;

use email_transport::TransportOption;

/// Restate-specific options for a single send attempt.
///
/// This is the one typed value inserted into
/// [`email_transport::SendOptions::transport_options`] for Restate. It travels
/// under the provider key `"restate"` and overrides the defaults configured on
/// the caller-side `RestateTransport` for one send.
///
/// The slot is forwarded to the worker in the queued payload like every other
/// provider slice. Workers ignore it unless their transport registry itself
/// contains a Restate-backed transport, in which case the same mode and delay
/// apply to the second hop as well.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct RestateSendOptions {
    /// Override the transport's configured [`InvocationMode`] for this send.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation_mode: Option<InvocationMode>,
    /// Delay before Restate starts the worker invocation.
    ///
    /// Sent as the ingress `delay` query parameter, encoded as whole
    /// milliseconds rounded up. Only valid with [`InvocationMode::Queued`];
    /// combining it with [`InvocationMode::Sent`] fails the send with
    /// `ErrorKind::Validation` before any request is made.
    ///
    /// This is provider scheduling behavior, not a delivery constraint: a
    /// transport that does not recognize the `restate` slot delivers now
    /// rather than later.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay: Option<Duration>,
}

impl TransportOption for RestateSendOptions {
    fn provider_key() -> &'static str {
        "restate"
    }
}

impl RestateSendOptions {
    /// Create empty Restate-specific send options.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the invocation mode for this send.
    #[must_use]
    pub const fn with_invocation_mode(mut self, invocation_mode: InvocationMode) -> Self {
        self.invocation_mode = Some(invocation_mode);
        self
    }

    /// Delay the worker invocation by `delay`.
    #[must_use]
    pub const fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = Some(delay);
        self
    }

    /// Return whether no override is configured.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.invocation_mode.is_none() && self.delay.is_none()
    }
}

/// How far a Restate-backed send is followed before it returns.
///
/// Both modes invoke the same `Email.send` handler; they differ in the state
/// the send has reached when the future resolves and in what the resulting
/// `SendReport` describes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum InvocationMode {
    /// Return once Restate has durably accepted the invocation.
    ///
    /// The report carries `"restate"` as the provider and the Restate
    /// invocation id as the message id; the worker has not yet run.
    #[default]
    Queued,
    /// Return once the worker has handed the message to its provider.
    ///
    /// The report is the worker's own provider report.
    Sent,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn default_options_serialize_to_empty_object() {
        let options = RestateSendOptions::default();

        assert!(options.is_empty());
        assert_eq!(
            serde_json::to_value(&options).expect("options serialize"),
            json!({})
        );
    }

    #[test]
    fn options_round_trip_through_json() {
        let options = RestateSendOptions::new()
            .with_invocation_mode(InvocationMode::Sent)
            .with_delay(Duration::new(1, 500));

        let value = serde_json::to_value(&options).expect("options serialize");
        assert_eq!(
            value,
            json!({
                "invocation_mode": "sent",
                "delay": {"secs": 1, "nanos": 500}
            })
        );
        assert_eq!(
            serde_json::from_value::<RestateSendOptions>(value).expect("options deserialize"),
            options
        );
    }

    #[test]
    fn invocation_mode_defaults_to_queued() {
        assert_eq!(InvocationMode::default(), InvocationMode::Queued);
        assert_eq!(
            serde_json::to_value(InvocationMode::Queued).expect("mode serializes"),
            json!("queued")
        );
    }
}
