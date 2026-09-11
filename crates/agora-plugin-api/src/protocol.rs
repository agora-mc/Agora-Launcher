//! The wire between a plugin and the host.
//!
//! Deliberately transport-free: a `HostRequest` is the same value whether it
//! crossed an in-process QuickJS bridge, a Tauri IPC hop, or — later — a pipe
//! to a companion process in another language. Nothing here knows which.
//!
//! Every request carries an id and a deadline, and every event carries where
//! it came from. Those two facts are what make the system debuggable: you can
//! always answer "which call is stuck?" and "which plugin caused this?".

use crate::error::PluginError;
use crate::manifest::PluginId;
use serde::{Deserialize, Serialize};

/// A plugin asking the host to do something.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostRequest {
    /// Unique within one plugin session. Echoed on the response so a plugin
    /// with several calls in flight can tell them apart, and so a stuck call
    /// can be named in a log.
    pub request_id: u64,
    /// Dotted method name, e.g. `instance.list`.
    pub method: String,
    #[serde(default)]
    pub args: serde_json::Value,
    /// How long the plugin is willing to wait. The host clamps this to
    /// [`MAX_REQUEST_DEADLINE_MS`]; a plugin cannot opt into waiting forever.
    #[serde(default = "default_deadline_ms")]
    pub deadline_ms: u64,
}

/// Longest any single host call may take before the host gives up on it.
pub const MAX_REQUEST_DEADLINE_MS: u64 = 60_000;

/// What a plugin gets if it does not ask for something else.
pub const DEFAULT_REQUEST_DEADLINE_MS: u64 = 10_000;

fn default_deadline_ms() -> u64 {
    DEFAULT_REQUEST_DEADLINE_MS
}

impl HostRequest {
    pub fn new(request_id: u64, method: impl Into<String>, args: serde_json::Value) -> Self {
        Self {
            request_id,
            method: method.into(),
            args,
            deadline_ms: DEFAULT_REQUEST_DEADLINE_MS,
        }
    }

    /// The deadline the host will actually honour.
    pub fn effective_deadline_ms(&self) -> u64 {
        self.deadline_ms.clamp(1, MAX_REQUEST_DEADLINE_MS)
    }
}

/// The host's answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "status"
)]
pub enum HostResponse {
    Ok {
        request_id: u64,
        value: serde_json::Value,
    },
    Error {
        request_id: u64,
        error: PluginError,
    },
}

impl HostResponse {
    pub fn request_id(&self) -> u64 {
        match self {
            HostResponse::Ok { request_id, .. } | HostResponse::Error { request_id, .. } => {
                *request_id
            }
        }
    }

    pub fn from_result(request_id: u64, result: Result<serde_json::Value, PluginError>) -> Self {
        match result {
            Ok(value) => HostResponse::Ok { request_id, value },
            Err(error) => HostResponse::Error { request_id, error },
        }
    }
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// Something that happened in the launcher, as a plugin sees it.
///
/// A narrow, documented set. `agora-core` emits a much richer `CoreEvent`;
/// this is the subset with a stable meaning that survives core changing its
/// mind about the internals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "event"
)]
pub enum PluginEvent {
    #[serde(rename = "instance.created")]
    InstanceCreated { instance_id: String },
    #[serde(rename = "instance.deleted")]
    InstanceDeleted { instance_id: String },
    #[serde(rename = "instance.renamed")]
    InstanceRenamed { instance_id: String, name: String },
    #[serde(rename = "content.installed")]
    ContentInstalled { instance_id: String, key: String },
    #[serde(rename = "content.removed")]
    ContentRemoved { instance_id: String, key: String },
    #[serde(rename = "content.enabled")]
    ContentEnabled { instance_id: String, key: String },
    #[serde(rename = "content.disabled")]
    ContentDisabled { instance_id: String, key: String },
    #[serde(rename = "launch.started")]
    LaunchStarted { instance_id: String },
    #[serde(rename = "launch.exited")]
    LaunchExited {
        instance_id: String,
        /// `exited`, `crashed`, or `cancelled`.
        outcome: String,
    },
    #[serde(rename = "registry.synced")]
    RegistrySynced {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<String>,
    },
}

impl PluginEvent {
    /// The wire name, as used in `onEvent:` activation and subscriptions.
    pub fn name(&self) -> &'static str {
        match self {
            PluginEvent::InstanceCreated { .. } => "instance.created",
            PluginEvent::InstanceDeleted { .. } => "instance.deleted",
            PluginEvent::InstanceRenamed { .. } => "instance.renamed",
            PluginEvent::ContentInstalled { .. } => "content.installed",
            PluginEvent::ContentRemoved { .. } => "content.removed",
            PluginEvent::ContentEnabled { .. } => "content.enabled",
            PluginEvent::ContentDisabled { .. } => "content.disabled",
            PluginEvent::LaunchStarted { .. } => "launch.started",
            PluginEvent::LaunchExited { .. } => "launch.exited",
            PluginEvent::RegistrySynced { .. } => "registry.synced",
        }
    }

    /// Every event name this host API version can deliver.
    pub const NAMES: &'static [&'static str] = &[
        "instance.created",
        "instance.deleted",
        "instance.renamed",
        "content.installed",
        "content.removed",
        "content.enabled",
        "content.disabled",
        "launch.started",
        "launch.exited",
        "registry.synced",
    ];

    /// The instance this event is about, when it is about one.
    pub fn instance_id(&self) -> Option<&str> {
        match self {
            PluginEvent::InstanceCreated { instance_id }
            | PluginEvent::InstanceDeleted { instance_id }
            | PluginEvent::InstanceRenamed { instance_id, .. }
            | PluginEvent::ContentInstalled { instance_id, .. }
            | PluginEvent::ContentRemoved { instance_id, .. }
            | PluginEvent::ContentEnabled { instance_id, .. }
            | PluginEvent::ContentDisabled { instance_id, .. }
            | PluginEvent::LaunchStarted { instance_id }
            | PluginEvent::LaunchExited { instance_id, .. } => Some(instance_id),
            PluginEvent::RegistrySynced { .. } => None,
        }
    }

    /// The capability a plugin needs in order to be told about this.
    ///
    /// A subscription is a read. A plugin with no `content:read` does not get
    /// told which mods someone installed just because it asked nicely.
    pub fn required_capability(&self) -> crate::Capability {
        match self {
            PluginEvent::InstanceCreated { .. }
            | PluginEvent::InstanceDeleted { .. }
            | PluginEvent::InstanceRenamed { .. } => crate::Capability::InstanceRead,
            PluginEvent::ContentInstalled { .. }
            | PluginEvent::ContentRemoved { .. }
            | PluginEvent::ContentEnabled { .. }
            | PluginEvent::ContentDisabled { .. } => crate::Capability::ContentRead,
            PluginEvent::LaunchStarted { .. } | PluginEvent::LaunchExited { .. } => {
                crate::Capability::LaunchRead
            }
            PluginEvent::RegistrySynced { .. } => crate::Capability::InstanceRead,
        }
    }
}

/// Who caused an event.
///
/// This is how event loops get broken. An event a plugin caused is not
/// delivered back to that plugin, and a chain of plugin-caused events is cut
/// off at [`MAX_EVENT_DEPTH`]. Without origin the only defence would be
/// hoping plugin authors are careful.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum EventOrigin {
    /// The user did it in the UI or the CLI.
    User,
    /// The launcher did it on its own — a sweep, a recovery, a sync.
    System,
    /// A plugin asked for it.
    Plugin { plugin_id: PluginId },
}

/// How many plugin-caused events may chain before the host stops relaying.
pub const MAX_EVENT_DEPTH: u8 = 4;

/// An event as delivered, with everything needed to reason about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginEventEnvelope {
    /// Monotonic per host session. A plugin that sees a gap knows its queue
    /// was trimmed rather than the launcher going quiet.
    pub sequence: u64,
    #[serde(flatten)]
    pub event: PluginEvent,
    pub origin: EventOrigin,
    /// How many plugin-caused hops led here.
    #[serde(default)]
    pub depth: u8,
    /// Ties an event back to the operation that produced it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
}

impl PluginEventEnvelope {
    /// Whether this envelope should be delivered to `recipient`.
    ///
    /// Two rules, both about not making an infinite loop trivially easy:
    /// a plugin is never told about its own effects, and a chain that has
    /// already bounced between plugins [`MAX_EVENT_DEPTH`] times stops.
    pub fn should_deliver_to(&self, recipient: &PluginId) -> bool {
        if self.depth >= MAX_EVENT_DEPTH {
            return false;
        }
        !matches!(&self.origin, EventOrigin::Plugin { plugin_id } if plugin_id == recipient)
    }

    /// The envelope for an event this event caused, one hop deeper.
    pub fn descend(&self, event: PluginEvent, sequence: u64, cause: PluginId) -> Self {
        Self {
            sequence,
            event,
            origin: EventOrigin::Plugin { plugin_id: cause },
            depth: self.depth.saturating_add(1),
            operation_id: self.operation_id.clone(),
        }
    }
}

/// A line a plugin wrote to its own log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
    }
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin(id: &str) -> PluginId {
        PluginId::parse(id).unwrap()
    }

    fn envelope(origin: EventOrigin, depth: u8) -> PluginEventEnvelope {
        PluginEventEnvelope {
            sequence: 1,
            event: PluginEvent::ContentEnabled {
                instance_id: "i1".into(),
                key: "sodium".into(),
            },
            origin,
            depth,
            operation_id: None,
        }
    }

    #[test]
    fn a_plugin_is_not_told_about_an_event_it_caused_itself() {
        let env = envelope(
            EventOrigin::Plugin {
                plugin_id: plugin("acme.tidy"),
            },
            0,
        );
        assert!(!env.should_deliver_to(&plugin("acme.tidy")));
        assert!(env.should_deliver_to(&plugin("other.plugin")));
    }

    #[test]
    fn a_user_caused_event_reaches_everyone() {
        let env = envelope(EventOrigin::User, 0);
        assert!(env.should_deliver_to(&plugin("acme.tidy")));
    }

    #[test]
    fn a_chain_of_plugin_events_stops_at_the_depth_limit() {
        let env = envelope(EventOrigin::System, MAX_EVENT_DEPTH);
        assert!(!env.should_deliver_to(&plugin("acme.tidy")));
    }

    #[test]
    fn descending_increments_depth_and_records_the_cause() {
        let env = envelope(EventOrigin::User, 0);
        let next = env.descend(
            PluginEvent::LaunchStarted {
                instance_id: "i1".into(),
            },
            2,
            plugin("acme.tidy"),
        );
        assert_eq!(next.depth, 1);
        assert_eq!(
            next.origin,
            EventOrigin::Plugin {
                plugin_id: plugin("acme.tidy")
            }
        );
    }

    #[test]
    fn a_ping_pong_between_two_plugins_terminates() {
        // Each plugin reacting to the other's effect adds a hop; after
        // MAX_EVENT_DEPTH hops nothing is delivered and the loop dies.
        let mut env = envelope(EventOrigin::User, 0);
        let a = plugin("a.one");
        let b = plugin("b.two");
        let mut hops = 0;
        loop {
            let recipient = if hops % 2 == 0 { &a } else { &b };
            if !env.should_deliver_to(recipient) {
                break;
            }
            let cause = recipient.clone();
            env = env.descend(
                PluginEvent::ContentDisabled {
                    instance_id: "i1".into(),
                    key: "sodium".into(),
                },
                env.sequence + 1,
                cause,
            );
            hops += 1;
            assert!(hops <= MAX_EVENT_DEPTH as usize, "event chain did not stop");
        }
        assert_eq!(hops, MAX_EVENT_DEPTH as usize);
    }

    #[test]
    fn every_event_name_is_listed_in_names() {
        let samples = [
            PluginEvent::InstanceCreated {
                instance_id: "i".into(),
            },
            PluginEvent::InstanceDeleted {
                instance_id: "i".into(),
            },
            PluginEvent::InstanceRenamed {
                instance_id: "i".into(),
                name: "n".into(),
            },
            PluginEvent::ContentInstalled {
                instance_id: "i".into(),
                key: "k".into(),
            },
            PluginEvent::ContentRemoved {
                instance_id: "i".into(),
                key: "k".into(),
            },
            PluginEvent::ContentEnabled {
                instance_id: "i".into(),
                key: "k".into(),
            },
            PluginEvent::ContentDisabled {
                instance_id: "i".into(),
                key: "k".into(),
            },
            PluginEvent::LaunchStarted {
                instance_id: "i".into(),
            },
            PluginEvent::LaunchExited {
                instance_id: "i".into(),
                outcome: "exited".into(),
            },
            PluginEvent::RegistrySynced { tag: None },
        ];
        assert_eq!(samples.len(), PluginEvent::NAMES.len());
        for sample in &samples {
            assert!(
                PluginEvent::NAMES.contains(&sample.name()),
                "`{}` is missing from PluginEvent::NAMES",
                sample.name()
            );
        }
    }

    #[test]
    fn an_event_serialises_with_its_name_as_the_tag() {
        let json = serde_json::to_value(PluginEvent::LaunchStarted {
            instance_id: "i1".into(),
        })
        .unwrap();
        assert_eq!(json["event"], "launch.started");
        assert_eq!(json["instanceId"], "i1");
    }

    #[test]
    fn a_request_cannot_opt_into_waiting_forever() {
        let mut request = HostRequest::new(1, "instance.list", serde_json::json!({}));
        request.deadline_ms = u64::MAX;
        assert_eq!(request.effective_deadline_ms(), MAX_REQUEST_DEADLINE_MS);
        request.deadline_ms = 0;
        assert_eq!(request.effective_deadline_ms(), 1);
    }
}
