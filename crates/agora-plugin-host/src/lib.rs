//! The QuickJS script host for Agora plugins.
//!
//! `agora-core` decides *whether* a plugin may do something. This crate is
//! what actually runs the plugin's JavaScript, and it deliberately knows
//! nothing about instances, mods, launching or the registry — it moves JSON
//! between a plugin and an [`agora_plugin_api::host::HostBridge`] and enforces
//! the resource limits.
//!
//! # Why QuickJS
//!
//! Measured on the target platform before the choice was made: the engine adds
//! roughly a megabyte to the binary, compiles in seconds on MSVC with no
//! external toolchain, and supports modules, promises and async host calls.
//! A V8-based runtime would have added tens of megabytes to a launcher whose
//! stated position is that it costs nothing to run. Nothing above
//! [`agora_plugin_api::host::ScriptHost`] depends on this choice, so it can be
//! revisited without touching plugin policy or a single plugin.

mod modules;
mod runtime;
mod sdk;

use agora_plugin_api::error::{PluginError, PluginErrorCode, PluginResult};
use agora_plugin_api::host::{ActivationRequest, HostBridge, ScriptHost};
use agora_plugin_api::manifest::PluginId;
use agora_plugin_api::protocol::PluginEventEnvelope;
use runtime::{Control, PluginHandle};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub use runtime::EVENT_QUEUE_CAPACITY;

/// Runs plugins in embedded QuickJS runtimes, one per plugin.
#[derive(Default)]
pub struct QuickJsHost {
    plugins: Mutex<HashMap<PluginId, PluginHandle>>,
}

impl QuickJsHost {
    pub fn new() -> Self {
        Self::default()
    }

    fn with_handle<T>(
        &self,
        plugin_id: &PluginId,
        f: impl FnOnce(&PluginHandle) -> PluginResult<T>,
    ) -> PluginResult<T> {
        let plugins = self.plugins.lock().map_err(poisoned)?;
        let handle = plugins.get(plugin_id).ok_or_else(|| {
            PluginError::new(
                PluginErrorCode::NotActivated,
                format!("`{plugin_id}` is not running"),
            )
        })?;
        f(handle)
    }
}

fn poisoned<T>(_: std::sync::PoisonError<T>) -> PluginError {
    PluginError::internal("the plugin host lock was poisoned")
}

impl ScriptHost for QuickJsHost {
    fn describe(&self) -> String {
        format!("quickjs (rquickjs {})", env!("CARGO_PKG_VERSION"))
    }

    fn activate(
        &self,
        request: ActivationRequest,
        bridge: Arc<dyn HostBridge>,
    ) -> PluginResult<()> {
        let plugin_id = request.plugin_id.clone();
        {
            let plugins = self.plugins.lock().map_err(poisoned)?;
            if plugins.contains_key(&plugin_id) {
                return Err(PluginError::internal(format!(
                    "`{plugin_id}` is already running; deactivate it first"
                )));
            }
        }
        // Spawned outside the lock: activation runs the plugin's own code and
        // can take seconds, and holding the map would stall every other
        // plugin's invoke for that whole time.
        let handle = runtime::spawn(request, bridge)?;
        let mut plugins = self.plugins.lock().map_err(poisoned)?;
        plugins.insert(plugin_id, handle);
        Ok(())
    }

    fn deactivate(&self, plugin_id: &PluginId) -> PluginResult<()> {
        let handle = {
            let mut plugins = self.plugins.lock().map_err(poisoned)?;
            plugins.remove(plugin_id)
        };
        // Deactivating something that is not running is what a recovery path
        // does, so it succeeds rather than erroring.
        let Some(mut handle) = handle else {
            return Ok(());
        };

        let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
        let result = if handle
            .control
            .send(Control::Deactivate { reply: reply_tx })
            .is_err()
        {
            Ok(())
        } else {
            match reply_rx.recv_timeout(Duration::from_secs(5)) {
                Ok(result) => result,
                Err(_) => {
                    // The plugin ignored its chance to clean up. Interrupt it
                    // and let the thread unwind; the alternative is leaving a
                    // runtime alive that the user asked to be rid of.
                    handle.interrupt();
                    Err(PluginError::new(
                        PluginErrorCode::Timeout,
                        format!("`{plugin_id}` did not shut down cleanly and was stopped"),
                    ))
                }
            }
        };

        // Dropping the senders ends the plugin's command loop.
        drop(handle.control);
        drop(handle.events);
        if let Some(thread) = handle.thread.take() {
            let _ = thread.join();
        }
        result
    }

    fn invoke(
        &self,
        plugin_id: &PluginId,
        export: &str,
        args: serde_json::Value,
        timeout: Duration,
    ) -> PluginResult<serde_json::Value> {
        let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
        self.with_handle(plugin_id, |handle| {
            handle.clear_interrupt();
            handle
                .control
                .send(Control::Invoke {
                    export: export.to_string(),
                    args,
                    timeout,
                    reply: reply_tx,
                })
                .map_err(|_| {
                    PluginError::new(
                        PluginErrorCode::NotActivated,
                        format!("`{plugin_id}` stopped running"),
                    )
                })
        })?;

        // Slack past the plugin's own timeout: the runtime thread is expected
        // to answer with a precise Timeout error, and this only catches a
        // thread that has stopped answering entirely.
        match reply_rx.recv_timeout(timeout + Duration::from_secs(2)) {
            Ok(result) => result,
            Err(_) => {
                let _ = self.with_handle(plugin_id, |handle| {
                    handle.interrupt();
                    Ok(())
                });
                Err(PluginError::new(
                    PluginErrorCode::Timeout,
                    format!("`{plugin_id}` stopped responding while running `{export}`"),
                ))
            }
        }
    }

    fn deliver_event(
        &self,
        plugin_id: &PluginId,
        envelope: &PluginEventEnvelope,
    ) -> PluginResult<()> {
        self.with_handle(plugin_id, |handle| {
            // `try_send`, never `send`: the caller is a core operation that
            // just finished doing real work, and it must not be made to wait
            // on a plugin's queue. A full queue drops the event and says so.
            handle
                .events
                .try_send(envelope.clone())
                .map_err(|e| match e {
                    tokio::sync::mpsc::error::TrySendError::Full(_) => PluginError::new(
                        PluginErrorCode::ResourceExhausted,
                        format!("`{plugin_id}` is not keeping up with events; one was dropped"),
                    ),
                    tokio::sync::mpsc::error::TrySendError::Closed(_) => PluginError::new(
                        PluginErrorCode::NotActivated,
                        format!("`{plugin_id}` stopped running"),
                    ),
                })
        })
    }

    fn cancel(&self, plugin_id: &PluginId) {
        let _ = self.with_handle(plugin_id, |handle| {
            handle.interrupt();
            Ok(())
        });
    }

    fn is_active(&self, plugin_id: &PluginId) -> bool {
        self.plugins
            .lock()
            .map(|plugins| plugins.contains_key(plugin_id))
            .unwrap_or(false)
    }

    fn active_plugins(&self) -> Vec<PluginId> {
        self.plugins
            .lock()
            .map(|plugins| plugins.keys().cloned().collect())
            .unwrap_or_default()
    }
}

impl Drop for QuickJsHost {
    fn drop(&mut self) {
        // Interrupt everything before the threads are torn down, so a plugin
        // busy in a loop does not keep the process alive at shutdown.
        if let Ok(plugins) = self.plugins.lock() {
            for handle in plugins.values() {
                handle.interrupt();
            }
        }
    }
}
