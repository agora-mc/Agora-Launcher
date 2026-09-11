//! Tauri adapter for community plugins.
//!
//! Transport only. Every decision — what may install, what may run, what a
//! plugin may do, what happens when one misbehaves — belongs to
//! `agora_core::plugins::PluginService`, so the CLI gets the same answers this
//! window does. What lives here is exactly three things: holding the single
//! `PluginService` for the process, turning its results into IPC payloads, and
//! forwarding plugin UI intents to the frontend as events.

use agora_core::error::{LauncherError, LauncherResult};
use agora_core::plugins::{
    InstallPreview, LaunchCheckOutcome, PluginService, PluginSummary, PluginUiSink, RepairOutcome,
    UpdateOutcome, UpdateVerdict,
};
use agora_plugin_api::diagnostics::{DiagnosticReport, RepairProposal};
use agora_plugin_api::dto::{Tone, ViewModel};
use agora_plugin_api::manifest::PluginId;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

/// The one plugin service for this process.
///
/// A `OnceLock` rather than eager construction: the service owns running
/// plugin runtimes, so building a second one would mean two sets of live
/// plugins disagreeing about what is enabled.
#[derive(Default)]
pub struct ManagedPlugins {
    service: OnceLock<PluginService>,
    /// Set once the first `activate_all` has run, so a window reload does not
    /// restart every plugin.
    started: Mutex<bool>,
}

/// Forwards a plugin's UI intents to the webview as ordinary Tauri events.
struct TauriUiSink<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> PluginUiSink for TauriUiSink<R> {
    fn refresh(&self, plugin_id: &PluginId, view_id: Option<&str>) {
        let _ = self.app.emit(
            "plugin-refresh",
            serde_json::json!({
                "pluginId": plugin_id.to_string(),
                "viewId": view_id,
            }),
        );
    }

    fn notify(&self, plugin_id: &PluginId, tone: Tone, message: &str) {
        // The plugin id travels with the message so the UI can attribute the
        // toast. A plugin must not be able to raise a notification that looks
        // like it came from Agora itself.
        let _ = self.app.emit(
            "plugin-notify",
            serde_json::json!({
                "pluginId": plugin_id.to_string(),
                "tone": tone,
                "message": message,
            }),
        );
    }
}

/// Get (or build) the process-wide plugin service.
pub fn service<R: Runtime>(app: &AppHandle<R>) -> LauncherResult<PluginService> {
    let managed = app
        .try_state::<ManagedPlugins>()
        .ok_or_else(|| LauncherError::Generic {
            code: "ERR_PLUGINS_UNAVAILABLE".into(),
            message: "The plugin service is not registered.".into(),
        })?;

    if let Some(service) = managed.service.get() {
        return Ok(service.clone());
    }

    let ctx = crate::core_context(app)?;
    let host: Arc<dyn agora_plugin_api::host::ScriptHost> =
        Arc::new(agora_plugin_host::QuickJsHost::new());
    let sink: Arc<dyn PluginUiSink> = Arc::new(TauriUiSink { app: app.clone() });
    let built = PluginService::new(ctx, host, sink);
    // Another thread may have won the race; either way exactly one service
    // ends up in the lock and every caller gets that one.
    let _ = managed.service.set(built);
    managed
        .service
        .get()
        .cloned()
        .ok_or_else(|| LauncherError::Generic {
            code: "ERR_PLUGINS_UNAVAILABLE".into(),
            message: "The plugin service could not be created.".into(),
        })
}

fn parse_id(raw: &str) -> LauncherResult<PluginId> {
    PluginId::parse(raw).map_err(|e| LauncherError::Generic {
        code: "ERR_PLUGIN_ID_INVALID".into(),
        message: e.message,
    })
}

fn from_plugin_error(error: agora_plugin_api::PluginError) -> LauncherError {
    LauncherError::Generic {
        code: format!("ERR_PLUGIN_{:?}", error.code).to_uppercase(),
        message: error.message,
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Whether the user has turned the plugin system on.
#[tauri::command]
pub async fn plugins_enabled(app: AppHandle) -> LauncherResult<bool> {
    Ok(service(&app)?.is_enabled())
}

/// Start the plugins that asked to run at startup. Idempotent across reloads.
///
/// Not every installed plugin: one contributing only a page or a command is
/// left for [`render_plugin_view`] / [`run_plugin_command`] to start on demand.
///
/// Returns the plugins that failed to start, so the UI can say so once rather
/// than leaving the user to notice that something is missing.
#[tauri::command]
pub async fn start_plugins(
    app: AppHandle,
    state: State<'_, ManagedPlugins>,
) -> LauncherResult<Vec<PluginStartFailure>> {
    let service = service(&app)?;
    {
        let mut started = state.started.lock().map_err(|_| LauncherError::Generic {
            code: "ERR_PLUGINS_LOCK".into(),
            message: "The plugin service is busy.".into(),
        })?;
        if *started {
            // A reload re-runs the frontend, not the plugins.
            service.reload()?;
            return Ok(Vec::new());
        }
        *started = true;
    }
    let failures = service.activate_all()?;

    // Off the startup path entirely. Checking is network work whose result
    // nobody is waiting for, and the launcher must open at the same speed
    // whether or not a publisher's host is reachable. Core decides whether it
    // happens at all; this only decides where it runs.
    let background = service.clone();
    std::thread::spawn(move || {
        for (plugin_id, outcome) in background.check_all_updates() {
            if let Err(reason) = outcome {
                // Recorded on the trust record by core already; this is the
                // developer-facing trail for a check nobody asked to see.
                eprintln!("[plugin-update] {plugin_id}: {reason}");
            }
        }
    });

    Ok(failures
        .into_iter()
        .map(|(id, error)| PluginStartFailure {
            plugin_id: id.to_string(),
            message: error.message,
        })
        .collect())
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginStartFailure {
    pub plugin_id: String,
    pub message: String,
}

#[tauri::command]
pub async fn list_plugins(app: AppHandle) -> LauncherResult<Vec<PluginSummary>> {
    let service = service(&app)?;
    service.reload()?;
    Ok(service.list())
}

/// Contributions from every runnable plugin, for the sidebar, palette and
/// settings page to render.
#[tauri::command]
pub async fn list_plugin_contributions(
    app: AppHandle,
) -> LauncherResult<Vec<agora_core::plugins::NamespacedContribution>> {
    Ok(service(&app)?.contributions())
}

/// Describe what installing a package would do, without installing it.
#[tauri::command]
pub async fn preview_plugin_package(
    app: AppHandle,
    path: String,
) -> LauncherResult<InstallPreview> {
    service(&app)?.preview_package(&PathBuf::from(path))
}

#[tauri::command]
pub async fn preview_plugin_folder(app: AppHandle, path: String) -> LauncherResult<InstallPreview> {
    service(&app)?.preview_folder(&PathBuf::from(path))
}

/// Install a package. `accept_capabilities` is the user's answer to the
/// prompt the preview produced; without it the core service refuses.
#[tauri::command]
pub async fn install_plugin_package(
    app: AppHandle,
    path: String,
    accept_capabilities: bool,
) -> LauncherResult<PluginSummary> {
    service(&app)?.install_package(&PathBuf::from(path), accept_capabilities)
}

#[tauri::command]
pub async fn plugin_check_update(
    app: AppHandle,
    plugin_id: String,
) -> LauncherResult<UpdateVerdict> {
    service(&app)?.check_update(&parse_id(&plugin_id)?)
}

#[tauri::command]
pub async fn plugin_apply_update(
    app: AppHandle,
    plugin_id: String,
    accept_capabilities: bool,
) -> LauncherResult<UpdateOutcome> {
    service(&app)?.apply_update(&parse_id(&plugin_id)?, accept_capabilities)
}

#[tauri::command]
pub async fn add_plugin_development_folder(
    app: AppHandle,
    path: String,
    accept_capabilities: bool,
) -> LauncherResult<PluginSummary> {
    service(&app)?.add_development_folder(&PathBuf::from(path), accept_capabilities)
}

#[tauri::command]
pub async fn set_plugin_enabled(
    app: AppHandle,
    plugin_id: String,
    enabled: bool,
) -> LauncherResult<()> {
    service(&app)?.set_enabled(&parse_id(&plugin_id)?, enabled)
}

/// Remove a plugin. Discarding its data is a separate decision.
#[tauri::command]
pub async fn uninstall_plugin(
    app: AppHandle,
    plugin_id: String,
    purge_data: bool,
) -> LauncherResult<()> {
    service(&app)?.uninstall(&parse_id(&plugin_id)?, purge_data)
}

/// Turn everything off. The recovery path, reachable from Settings.
#[tauri::command]
pub async fn disable_all_plugins(app: AppHandle) -> LauncherResult<usize> {
    service(&app)?.disable_all()
}

#[tauri::command]
pub async fn render_plugin_view(
    app: AppHandle,
    plugin_id: String,
    export: String,
    args: Option<serde_json::Value>,
) -> LauncherResult<ViewModel> {
    service(&app)?
        .render_view(
            &parse_id(&plugin_id)?,
            &export,
            args.unwrap_or(serde_json::Value::Null),
        )
        .map_err(from_plugin_error)
}

#[tauri::command]
pub async fn run_plugin_command(
    app: AppHandle,
    plugin_id: String,
    export: String,
    args: Option<serde_json::Value>,
) -> LauncherResult<serde_json::Value> {
    service(&app)?
        .run_command(
            &parse_id(&plugin_id)?,
            &export,
            args.unwrap_or(serde_json::Value::Null),
        )
        .map_err(from_plugin_error)
}

#[tauri::command]
pub async fn run_plugin_diagnostic(
    app: AppHandle,
    plugin_id: String,
    export: String,
    instance_id: String,
) -> LauncherResult<DiagnosticReport> {
    service(&app)?
        .run_diagnostic(
            &parse_id(&plugin_id)?,
            &export,
            serde_json::json!({ "instanceId": instance_id }),
        )
        .map_err(from_plugin_error)
}

/// Apply a repair the user approved. Core re-validates before acting.
#[tauri::command]
pub async fn apply_plugin_repair(
    app: AppHandle,
    plugin_id: String,
    proposal: RepairProposal,
) -> LauncherResult<RepairOutcome> {
    service(&app)?.apply_repair(&parse_id(&plugin_id)?, &proposal)
}

/// Run every contributed pre-launch check for an instance.
#[tauri::command]
pub async fn run_plugin_launch_checks(
    app: AppHandle,
    instance_id: String,
) -> LauncherResult<LaunchCheckOutcome> {
    Ok(service(&app)?.run_launch_checks(&instance_id))
}

/// Read the HTML of a manifest-declared custom view, from inside the package.
///
/// Core resolves `local_id` against the plugin's declared contributions, so a
/// path that was not declared — or that points outside the package — is
/// refused rather than read.
#[tauri::command]
pub async fn read_plugin_custom_view(
    app: AppHandle,
    plugin_id: String,
    local_id: String,
) -> LauncherResult<String> {
    let service = service(&app)?;
    let id = parse_id(&plugin_id)?;
    tauri::async_runtime::spawn_blocking(move || service.custom_view_html(&id, &local_id))
        .await
        .map_err(|error| LauncherError::Generic {
            code: "ERR_PLUGIN_VIEW".into(),
            message: error.to_string(),
        })?
}

/// Forward core events without constructing or starting the plugin service.
struct PluginEventSink<R: Runtime> {
    app: AppHandle<R>,
    next: Arc<dyn agora_core::event_sink::EventSink>,
}

impl<R: Runtime> agora_core::event_sink::EventSink for PluginEventSink<R> {
    fn emit(&self, event: agora_core::event_sink::CoreEvent) {
        self.next.emit(event.clone());
        if let Some(managed) = self.app.try_state::<ManagedPlugins>() {
            if let Some(service) = managed.service.get() {
                if let Some(translated) =
                    agora_core::plugins::events::PluginEventBus::translate(&event)
                {
                    // Through the service, not straight to the bus: a plugin whose
                    // only activation event is this one has not run yet, so it has
                    // not subscribed to anything.
                    service.publish_event(translated, None);
                }
            }
        }
    }
}

pub fn with_events<R: Runtime>(
    app: &AppHandle<R>,
    mut ctx: agora_core::ctx::Ctx,
) -> agora_core::ctx::Ctx {
    ctx.event_sink = Arc::new(PluginEventSink {
        app: app.clone(),
        next: ctx.event_sink,
    });
    ctx
}

/// Tell plugins the user opened an instance, starting any that asked for it.
///
/// Contributed panels already start their plugin when they render; this covers
/// a plugin that wants to react without putting anything on screen.
#[tauri::command]
pub async fn plugin_instance_opened(app: AppHandle) -> LauncherResult<usize> {
    let service = service(&app)?;
    Ok(
        tauri::async_runtime::spawn_blocking(move || service.notify_instance_opened())
            .await
            .unwrap_or(0),
    )
}

/// A plugin's declared settings and their current values.
#[tauri::command]
pub async fn get_plugin_settings(
    app: AppHandle,
    plugin_id: String,
) -> LauncherResult<agora_core::plugins::PluginSettingsView> {
    service(&app)?.settings(&parse_id(&plugin_id)?)
}

#[tauri::command]
pub async fn set_plugin_setting(
    app: AppHandle,
    plugin_id: String,
    key: String,
    value: serde_json::Value,
) -> LauncherResult<()> {
    service(&app)?.set_setting(&parse_id(&plugin_id)?, &key, &value)
}

#[tauri::command]
pub async fn read_plugin_log(
    app: AppHandle,
    plugin_id: String,
    lines: Option<usize>,
) -> LauncherResult<Vec<String>> {
    Ok(service(&app)?.logs(&parse_id(&plugin_id)?, lines.unwrap_or(200).clamp(1, 2_000)))
}

/// Every replaceable surface, who offered to render it, and what will.
#[tauri::command]
pub async fn plugin_surfaces(
    app: AppHandle,
) -> LauncherResult<Vec<agora_core::plugins::SurfaceChoice>> {
    Ok(service(&app)?.surfaces())
}

/// Choose who renders a surface. A null `contributionId` restores Agora's own.
#[tauri::command]
pub async fn plugin_set_surface(
    app: AppHandle,
    surface: String,
    contribution_id: Option<String>,
) -> LauncherResult<()> {
    let surface: agora_plugin_api::contributions::ReplaceableSurface =
        surface.parse().map_err(|_| LauncherError::Generic {
            code: "ERR_PLUGIN_SURFACE_UNKNOWN".into(),
            message: format!("`{surface}` is not a surface this version of Agora can hand over"),
        })?;
    service(&app)?.set_surface(surface, contribution_id.as_deref())
}
