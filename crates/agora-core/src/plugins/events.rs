//! Telling plugins what happened, without letting them start a stampede.
//!
//! Three separate mechanisms, each solving a different failure:
//!
//! - **Subscriptions** are explicit and capability-checked, so a plugin is
//!   told only about things it was already allowed to read.
//! - **Origin** records who caused an event. A plugin is never told about its
//!   own effects, which removes the most common way an event loop starts.
//! - **Depth** bounds a chain of plugin-caused events, which removes the
//!   second most common way: two plugins reacting to each other.
//!
//! Delivery is best-effort and never blocks the caller. The alternative —
//! making an install wait on a plugin's event handler — trades a launcher bug
//! for a plugin bug, which is the wrong direction.

use agora_plugin_api::host::ScriptHost;
use agora_plugin_api::manifest::PluginId;
use agora_plugin_api::protocol::{EventOrigin, PluginEvent, PluginEventEnvelope};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

/// Who subscribed to what.
#[derive(Clone, Default)]
pub struct EventSubscriptions {
    inner: Arc<RwLock<BTreeMap<PluginId, BTreeSet<String>>>>,
}

impl EventSubscriptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn subscribe(&self, plugin_id: &PluginId, event: &str) {
        if let Ok(mut map) = self.inner.write() {
            map.entry(plugin_id.clone())
                .or_default()
                .insert(event.to_string());
        }
    }

    pub fn unsubscribe(&self, plugin_id: &PluginId, event: &str) {
        if let Ok(mut map) = self.inner.write() {
            if let Some(events) = map.get_mut(plugin_id) {
                events.remove(event);
                if events.is_empty() {
                    map.remove(plugin_id);
                }
            }
        }
    }

    /// Drop every subscription a plugin holds.
    ///
    /// Called on disable and on uninstall. A subscription outliving the plugin
    /// that made it would mean delivering events to a runtime that is gone.
    pub fn clear(&self, plugin_id: &PluginId) {
        if let Ok(mut map) = self.inner.write() {
            map.remove(plugin_id);
        }
    }

    pub fn subscribers(&self, event: &str) -> Vec<PluginId> {
        self.inner
            .read()
            .map(|map| {
                map.iter()
                    .filter(|(_, events)| events.contains(event))
                    .map(|(plugin_id, _)| plugin_id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn events_for(&self, plugin_id: &PluginId) -> Vec<String> {
        self.inner
            .read()
            .map(|map| {
                map.get(plugin_id)
                    .map(|events| events.iter().cloned().collect())
                    .unwrap_or_default()
            })
            .unwrap_or_default()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.read().map(|map| map.is_empty()).unwrap_or(true)
    }
}

// ---------------------------------------------------------------------------
// Origin tracking
// ---------------------------------------------------------------------------

thread_local! {
    /// The plugin, if any, whose request is being served on this thread.
    ///
    /// Core services emit their events synchronously on the calling thread, so
    /// a thread-local is enough to attribute them — and it works without
    /// threading an origin parameter through every service signature, which
    /// would be a large change to code that has nothing to do with plugins.
    static CURRENT_ORIGIN: RefCell<Vec<PluginId>> = const { RefCell::new(Vec::new()) };
}

/// Run `f` with events attributed to `plugin_id`.
pub fn with_origin<T>(plugin_id: &PluginId, f: impl FnOnce() -> T) -> T {
    CURRENT_ORIGIN.with(|stack| stack.borrow_mut().push(plugin_id.clone()));
    let result = f();
    CURRENT_ORIGIN.with(|stack| {
        stack.borrow_mut().pop();
    });
    result
}

/// Who is causing whatever is happening on this thread right now.
pub fn current_origin() -> EventOrigin {
    CURRENT_ORIGIN
        .with(|stack| stack.borrow().last().cloned())
        .map(|plugin_id| EventOrigin::Plugin { plugin_id })
        .unwrap_or(EventOrigin::User)
}

/// How deep into a plugin-caused chain this thread already is.
pub fn current_depth() -> u8 {
    CURRENT_ORIGIN.with(|stack| stack.borrow().len().min(u8::MAX as usize) as u8)
}

// ---------------------------------------------------------------------------
// The bus
// ---------------------------------------------------------------------------

/// Fans launcher events out to subscribed plugins.
#[derive(Clone)]
pub struct PluginEventBus {
    subscriptions: EventSubscriptions,
    host: Arc<dyn ScriptHost>,
    sequence: Arc<AtomicU64>,
    /// Events dropped because a plugin's queue was full. Surfaced in the
    /// plugin manager so "my plugin missed something" has an answer.
    dropped: Arc<RwLock<BTreeMap<PluginId, u64>>>,
}

impl PluginEventBus {
    pub fn new(subscriptions: EventSubscriptions, host: Arc<dyn ScriptHost>) -> Self {
        Self {
            subscriptions,
            host,
            sequence: Arc::new(AtomicU64::new(0)),
            dropped: Arc::new(RwLock::new(BTreeMap::new())),
        }
    }

    pub fn subscriptions(&self) -> &EventSubscriptions {
        &self.subscriptions
    }

    /// How many events a plugin has missed because it could not keep up.
    pub fn dropped_count(&self, plugin_id: &PluginId) -> u64 {
        self.dropped
            .read()
            .map(|map| map.get(plugin_id).copied().unwrap_or(0))
            .unwrap_or(0)
    }

    /// Publish one event to everyone entitled to it.
    ///
    /// Returns how many plugins it actually reached, which is what the tests
    /// assert on and what makes "did anyone hear that?" answerable.
    pub fn publish(&self, event: PluginEvent, operation_id: Option<String>) -> usize {
        let envelope = PluginEventEnvelope {
            sequence: self.sequence.fetch_add(1, Ordering::Relaxed),
            origin: current_origin(),
            depth: current_depth(),
            operation_id,
            event,
        };
        self.publish_envelope(&envelope)
    }

    pub fn publish_envelope(&self, envelope: &PluginEventEnvelope) -> usize {
        let mut delivered = 0;
        for plugin_id in self.subscriptions.subscribers(envelope.event.name()) {
            if !envelope.should_deliver_to(&plugin_id) {
                continue;
            }
            if !self.host.is_active(&plugin_id) {
                continue;
            }
            match self.host.deliver_event(&plugin_id, envelope) {
                Ok(()) => delivered += 1,
                Err(error)
                    if error.code == agora_plugin_api::PluginErrorCode::ResourceExhausted =>
                {
                    if let Ok(mut map) = self.dropped.write() {
                        *map.entry(plugin_id).or_insert(0) += 1;
                    }
                }
                // The plugin stopped between the subscriber lookup and the
                // send. Nothing to report: it is going away anyway.
                Err(_) => {}
            }
        }
        delivered
    }

    /// Translate a launcher event into the plugin-facing one, if there is one.
    ///
    /// Most `CoreEvent` variants have no plugin equivalent, and deliberately
    /// so: the plugin API is a stable contract, not a mirror of core's
    /// internals. Returning `None` here is the normal case.
    pub fn translate(event: &crate::event_sink::CoreEvent) -> Option<PluginEvent> {
        use crate::event_sink::{CoreEvent, EventStatus, ModAction};
        match event {
            CoreEvent::RegistrySync {
                status: EventStatus::Completed,
                new_tag,
                ..
            } => Some(PluginEvent::RegistrySynced {
                tag: new_tag.clone(),
            }),
            CoreEvent::Launch {
                instance_id,
                status: EventStatus::Started,
                ..
            } => Some(PluginEvent::LaunchStarted {
                instance_id: instance_id.clone(),
            }),
            CoreEvent::Launch {
                instance_id,
                status,
                ..
            } if matches!(
                status,
                EventStatus::Completed | EventStatus::Failed | EventStatus::Cancelled
            ) =>
            {
                Some(PluginEvent::LaunchExited {
                    instance_id: instance_id.clone(),
                    outcome: match status {
                        EventStatus::Completed => "exited",
                        EventStatus::Failed => "crashed",
                        _ => "cancelled",
                    }
                    .to_string(),
                })
            }
            CoreEvent::ModOperation {
                instance_id,
                action,
                status: EventStatus::Completed,
                message,
                ..
            } => {
                // `message` carries the filename core reported. A plugin gets
                // the launcher's own identifier for the item rather than a
                // guess reconstructed from the event text.
                let key = message.clone();
                Some(match action {
                    ModAction::Install | ModAction::Update => PluginEvent::ContentInstalled {
                        instance_id: instance_id.clone(),
                        key,
                    },
                    ModAction::Remove => PluginEvent::ContentRemoved {
                        instance_id: instance_id.clone(),
                        key,
                    },
                    ModAction::Enable => PluginEvent::ContentEnabled {
                        instance_id: instance_id.clone(),
                        key,
                    },
                    ModAction::Disable => PluginEvent::ContentDisabled {
                        instance_id: instance_id.clone(),
                        key,
                    },
                })
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agora_plugin_api::error::PluginResult;
    use agora_plugin_api::host::{ActivationRequest, HostBridge};
    use std::sync::Mutex;
    use std::time::Duration;

    fn id(raw: &str) -> PluginId {
        PluginId::parse(raw).unwrap()
    }

    #[derive(Default)]
    struct RecordingHost {
        delivered: Mutex<Vec<(PluginId, String)>>,
        full: Mutex<BTreeSet<PluginId>>,
        active: Mutex<BTreeSet<PluginId>>,
    }

    impl ScriptHost for RecordingHost {
        fn describe(&self) -> String {
            "recording".into()
        }
        fn activate(
            &self,
            _request: ActivationRequest,
            _bridge: Arc<dyn HostBridge>,
        ) -> PluginResult<()> {
            Ok(())
        }
        fn deactivate(&self, _plugin_id: &PluginId) -> PluginResult<()> {
            Ok(())
        }
        fn invoke(
            &self,
            _plugin_id: &PluginId,
            _export: &str,
            _args: serde_json::Value,
            _timeout: Duration,
        ) -> PluginResult<serde_json::Value> {
            Ok(serde_json::Value::Null)
        }
        fn deliver_event(
            &self,
            plugin_id: &PluginId,
            envelope: &PluginEventEnvelope,
        ) -> PluginResult<()> {
            if self.full.lock().unwrap().contains(plugin_id) {
                return Err(agora_plugin_api::PluginError::new(
                    agora_plugin_api::PluginErrorCode::ResourceExhausted,
                    "queue full",
                ));
            }
            self.delivered
                .lock()
                .unwrap()
                .push((plugin_id.clone(), envelope.event.name().to_string()));
            Ok(())
        }
        fn cancel(&self, _plugin_id: &PluginId) {}
        fn is_active(&self, plugin_id: &PluginId) -> bool {
            self.active.lock().unwrap().contains(plugin_id)
        }
        fn active_plugins(&self) -> Vec<PluginId> {
            self.active.lock().unwrap().iter().cloned().collect()
        }
    }

    fn bus_with(active: &[&str]) -> (PluginEventBus, Arc<RecordingHost>) {
        let host = Arc::new(RecordingHost::default());
        for plugin in active {
            host.active.lock().unwrap().insert(id(plugin));
        }
        let bus = PluginEventBus::new(EventSubscriptions::new(), host.clone());
        (bus, host)
    }

    fn launch_started() -> PluginEvent {
        PluginEvent::LaunchStarted {
            instance_id: "inst-a".into(),
        }
    }

    #[test]
    fn only_subscribers_are_told() {
        let (bus, host) = bus_with(&["a.one", "b.two"]);
        bus.subscriptions()
            .subscribe(&id("a.one"), "launch.started");
        assert_eq!(bus.publish(launch_started(), None), 1);
        let delivered = host.delivered.lock().unwrap();
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].0, id("a.one"));
    }

    #[test]
    fn a_plugin_that_is_not_running_is_skipped() {
        let (bus, _host) = bus_with(&[]);
        bus.subscriptions()
            .subscribe(&id("a.one"), "launch.started");
        assert_eq!(bus.publish(launch_started(), None), 0);
    }

    #[test]
    fn clearing_a_plugins_subscriptions_stops_delivery() {
        let (bus, _host) = bus_with(&["a.one"]);
        bus.subscriptions()
            .subscribe(&id("a.one"), "launch.started");
        bus.subscriptions().clear(&id("a.one"));
        assert_eq!(bus.publish(launch_started(), None), 0);
        assert!(bus.subscriptions().is_empty());
    }

    #[test]
    fn a_plugin_does_not_hear_about_what_it_did_itself() {
        let (bus, _host) = bus_with(&["a.one", "b.two"]);
        bus.subscriptions()
            .subscribe(&id("a.one"), "launch.started");
        bus.subscriptions()
            .subscribe(&id("b.two"), "launch.started");
        let delivered = with_origin(&id("a.one"), || bus.publish(launch_started(), None));
        assert_eq!(delivered, 1, "only b.two should have been told");
    }

    #[test]
    fn origin_is_restored_after_a_nested_call_returns() {
        let (bus, _host) = bus_with(&["a.one"]);
        bus.subscriptions()
            .subscribe(&id("a.one"), "launch.started");
        with_origin(&id("b.two"), || {
            with_origin(&id("a.one"), || {
                assert_eq!(bus.publish(launch_started(), None), 0);
            });
            // Back to b.two's frame, so a.one is entitled again.
            assert_eq!(bus.publish(launch_started(), None), 1);
        });
        assert_eq!(current_origin(), EventOrigin::User);
    }

    #[test]
    fn a_chain_that_gets_too_deep_stops_being_delivered() {
        let (bus, _host) = bus_with(&["a.one"]);
        bus.subscriptions()
            .subscribe(&id("a.one"), "launch.started");
        // Four nested plugin frames puts the depth at the limit.
        with_origin(&id("x.one"), || {
            with_origin(&id("x.two"), || {
                with_origin(&id("x.three"), || {
                    with_origin(&id("x.four"), || {
                        assert_eq!(bus.publish(launch_started(), None), 0);
                    })
                })
            })
        });
    }

    #[test]
    fn a_full_queue_is_counted_rather_than_retried() {
        let (bus, host) = bus_with(&["a.one"]);
        host.full.lock().unwrap().insert(id("a.one"));
        bus.subscriptions()
            .subscribe(&id("a.one"), "launch.started");
        assert_eq!(bus.publish(launch_started(), None), 0);
        assert_eq!(bus.dropped_count(&id("a.one")), 1);
    }

    #[test]
    fn sequence_numbers_advance_so_a_plugin_can_see_a_gap() {
        let (bus, host) = bus_with(&["a.one"]);
        bus.subscriptions()
            .subscribe(&id("a.one"), "launch.started");
        bus.publish(launch_started(), None);
        bus.publish(launch_started(), None);
        assert_eq!(host.delivered.lock().unwrap().len(), 2);
    }

    #[test]
    fn core_launch_events_translate_to_plugin_events() {
        use crate::event_sink::{CoreEvent, EventStatus};
        let started = CoreEvent::Launch {
            operation_id: crate::event_sink::OperationId::new("op-1"),
            instance_id: "inst-a".into(),
            status: EventStatus::Started,
            pid: Some(42),
        };
        assert_eq!(
            PluginEventBus::translate(&started).map(|e| e.name()),
            Some("launch.started")
        );

        let failed = CoreEvent::Launch {
            operation_id: crate::event_sink::OperationId::new("op-1"),
            instance_id: "inst-a".into(),
            status: EventStatus::Failed,
            pid: None,
        };
        let Some(PluginEvent::LaunchExited { outcome, .. }) = PluginEventBus::translate(&failed)
        else {
            panic!("a failed launch should translate to launch.exited");
        };
        assert_eq!(outcome, "crashed");
    }

    #[test]
    fn core_events_with_no_plugin_equivalent_translate_to_nothing() {
        use crate::event_sink::CoreEvent;
        let warning = CoreEvent::Warning {
            message: "something".into(),
            details: None,
        };
        assert!(PluginEventBus::translate(&warning).is_none());
    }

    #[test]
    fn an_in_progress_mod_operation_is_not_announced_as_finished() {
        use crate::event_sink::{CoreEvent, EventStatus, ModAction};
        let started = CoreEvent::ModOperation {
            operation_id: crate::event_sink::OperationId::new("op-1"),
            instance_id: "inst-a".into(),
            action: ModAction::Install,
            status: EventStatus::Started,
            message: "sodium.jar".into(),
        };
        assert!(PluginEventBus::translate(&started).is_none());
    }
}
