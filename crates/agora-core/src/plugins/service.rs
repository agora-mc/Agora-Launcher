//! The plugin system, as the rest of Agora sees it.
//!
//! Every adapter — the Tauri commands, the CLI, later the MCP dispatcher —
//! talks to this and nothing below it. That is what keeps plugin *policy*
//! (what may run, what it may do, what happens when it misbehaves) in one
//! place instead of duplicated three times with three different opinions.
//!
//! The load-bearing rule in here: **a plugin proposes, core disposes.** A
//! plugin never performs an operation. It returns a description of one, the
//! user approves it, and this module re-validates the world and then calls the
//! same service the GUI would have called. That is why a stale repair plan
//! fails cleanly instead of doing the wrong thing to the right instance.

use super::dispatch::{self, DispatchContext, NoopUiSink, PluginUiSink};
use super::events::{self, EventSubscriptions, PluginEventBus};
use super::install::{self, InstallPreview};
use super::logs;
use super::registry::{self, NamespacedContribution, PluginStatus, Resolution};
use super::store::{self, PluginRecord, PluginSource, StorageKind};
use super::updates::{self, UpdateVerdict};
use crate::ctx::Ctx;
use crate::error::{LauncherError, LauncherResult};
use agora_plugin_api::capability::CapabilitySet;
use agora_plugin_api::contributions::{
    LaunchCheckFailure, ReplaceableSurface, SettingDefinition, ViewSource, MAX_LAUNCH_CHECK_MS,
};
use agora_plugin_api::diagnostics::{DiagnosticReport, RepairAction, RepairProposal, Severity};
use agora_plugin_api::distribution::{Release, UpdateDocument};
use agora_plugin_api::dto::ViewModel;
use agora_plugin_api::error::{PluginError, PluginErrorCode};
use agora_plugin_api::host::{ActivationRequest, HostBridge, ScriptHost};
use agora_plugin_api::manifest::{ActivationEvent, PluginId, PluginManifest};
use agora_plugin_api::protocol::{HostRequest, HostResponse, LogLevel};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// Setting that turns the whole subsystem on. Off by default.
pub const PLUGINS_ENABLED_SETTING: &str = "plugins_enabled";

/// Which plugin, if any, the user chose to render each replaceable surface.
///
/// One setting holding a map rather than a setting per surface, so adding a
/// surface later does not need a migration and an unknown key from a newer
/// build is ignored rather than read as a selection.
pub const SURFACE_SELECTIONS_SETTING: &str = "plugin_surface_selections";

/// Whether Agora may check publishers for plugin updates on its own.
///
/// Distinct from `network_plugins_enabled`, which is whether a plugin's own
/// code may reach the network. A user can reasonably want either without the
/// other, and conflating them would mean refusing one refuses both.
pub const PLUGIN_UPDATES_ENABLED_SETTING: &str = "plugin_updates_enabled";

/// How long a view render or command may take.
const INVOKE_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a diagnostic may take. Longer than a view: it is doing work the
/// user asked for and is watching a spinner for.
const DIAGNOSTIC_TIMEOUT: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// Public shapes
// ---------------------------------------------------------------------------

/// One plugin, as the manager and the CLI display it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginSummary {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub license: String,
    pub source_url: Option<String>,
    pub enabled: bool,
    pub running: bool,
    pub development: bool,
    pub status: PluginStatus,
    pub status_text: String,
    pub capabilities: Vec<String>,
    pub declared_hosts: Vec<String>,
    pub contributions: Vec<NamespacedContribution>,
    pub definitions: agora_plugin_api::contributions::Contributions,
    pub installed_at: String,
    pub updated_at: String,
    /// Where this plugin checks for updates, and the keys pinned when it was
    /// installed. `None` means it did not come with an update source.
    ///
    /// Read from the trust record rather than the manifest, because the
    /// manifest is whatever the *current* package says and the pin is what the
    /// user actually agreed to.
    pub update_source: Option<install::UpdateSourceSummary>,
    /// What the last update check concluded, and when.
    pub last_update_check: Option<UpdateCheckRecord>,
    /// A saved copy of this plugin's data that could be put back, if one was
    /// taken. Restoring is always the user's decision, never automatic.
    pub restorable_data: Option<store::CheckpointSummary>,
    /// Events this plugin missed because it could not keep up.
    pub dropped_events: u64,
}

/// The outcome of the most recent update check, for display.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckRecord {
    pub at: String,
    pub result: String,
}

/// One plugin's offer to render a built-in surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplacementOffer {
    /// `publisher.plugin/local`.
    pub id: String,
    pub plugin_id: String,
    pub local_id: String,
    pub plugin_name: String,
    pub title: String,
    pub description: Option<String>,
    pub surface: String,
    /// The export to call. Carried so the adapter renders through the same
    /// path a contributed page uses rather than inventing a second one.
    pub export: String,
}

/// Everything the user needs in order to choose who renders one surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceChoice {
    pub surface: String,
    pub title: String,
    /// Every offer from a plugin that is installed, whether or not it can
    /// currently run. An offer from a broken plugin is still shown, because
    /// hiding it would make the user's own selection vanish from the list they
    /// chose it in.
    pub offers: Vec<ReplacementOffer>,
    /// What the user picked, even if it cannot currently render.
    pub selected: Option<String>,
    /// What will actually render. `None` means Agora's own view — either
    /// because nothing was selected, or because what was selected cannot run.
    pub effective: Option<ReplacementOffer>,
    /// Set when a selection exists but is not what will render, with the
    /// reason. This is the difference between "you chose the built-in" and
    /// "your choice is broken and we quietly did something else".
    pub fallback_reason: Option<String>,
}

/// What applying an update did, or what it needs first.
///
/// A separate case rather than an error, because "this version wants more than
/// you granted" is a question to put to the user, not a failure. An error
/// carries a sentence; this carries the same preview the install prompt shows,
/// so the two paths look identical to someone deciding.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "outcome")]
pub enum UpdateOutcome {
    Installed { plugin: Box<PluginSummary> },
    NeedsConsent { preview: Box<InstallPreview> },
}

/// What `apply_repair` actually did.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairOutcome {
    pub applied: Vec<String>,
    pub failed: Vec<RepairFailure>,
    /// Actions skipped because the world had changed since the proposal.
    pub stale: Vec<String>,
}

impl RepairOutcome {
    /// Whether everything the user approved actually happened.
    ///
    /// Used by callers that must not report success optimistically: a partial
    /// repair is reported as partial.
    pub fn is_complete(&self) -> bool {
        self.failed.is_empty() && self.stale.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairFailure {
    pub action: String,
    pub message: String,
    pub retryable: bool,
}

/// A plugin's settings page, as data.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginSettingsView {
    pub definitions: Vec<SettingDefinition>,
    /// Every declared key has a value here, defaults filled in.
    pub values: serde_json::Value,
}

/// Two plugins wanting incompatible things.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairConflict {
    pub left_plugin: String,
    pub left_action: String,
    pub right_plugin: String,
    pub right_action: String,
    pub subject: String,
}

/// One plugin's launch-check result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchCheckResult {
    pub plugin_id: String,
    pub check_id: String,
    pub title: String,
    pub report: DiagnosticReport,
    pub blocking: bool,
}

/// Everything the launch path learned from plugins.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchCheckOutcome {
    pub results: Vec<LaunchCheckResult>,
    /// Checks that failed to run at all. Reported, never silently ignored.
    pub errors: Vec<String>,
}

impl LaunchCheckOutcome {
    /// Whether anything warrants stopping to ask the user.
    ///
    /// Note what this is *not*: a veto. Agora warns and the user decides, and
    /// a plugin does not get to be stricter than the launcher itself.
    pub fn should_prompt(&self) -> bool {
        self.results.iter().any(|result| {
            result.blocking
                && result
                    .report
                    .highest_severity()
                    .is_some_and(|severity| severity >= Severity::Warning)
        })
    }
}

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

struct Inner {
    ctx: Ctx,
    host: Arc<dyn ScriptHost>,
    ui: Arc<dyn PluginUiSink>,
    subscriptions: EventSubscriptions,
    resolution: RwLock<Resolution>,
    logs_root: PathBuf,
}

impl Inner {
    fn record(&self, plugin_id: &PluginId) -> Option<PluginRecord> {
        self.resolution
            .read()
            .ok()?
            .get(plugin_id)
            .map(|resolved| resolved.record.clone())
    }

    fn conn(&self) -> LauncherResult<rusqlite::Connection> {
        crate::db::local_state_connection(&self.ctx.paths.local_state_db()).map_err(|e| {
            LauncherError::Generic {
                code: "ERR_LOCAL_STATE_FAILED".into(),
                message: e.to_string(),
            }
        })
    }
}

/// What the script host calls back into.
///
/// One bridge serves every plugin, and looks the caller's grants up on each
/// call rather than capturing them. That matters: disabling a capability, or
/// the plugin itself, takes effect on the very next call instead of whenever
/// the runtime happens to be recycled.
struct CoreBridge {
    inner: Arc<Inner>,
}

impl HostBridge for CoreBridge {
    fn call(&self, plugin_id: &PluginId, request: HostRequest) -> HostResponse {
        let Some(record) = self.inner.record(plugin_id) else {
            return HostResponse::Error {
                request_id: request.request_id,
                error: PluginError::new(
                    PluginErrorCode::NotActivated,
                    format!("`{plugin_id}` is no longer installed"),
                ),
            };
        };

        let cx = DispatchContext {
            ctx: &self.inner.ctx,
            plugin_id,
            granted: &record.granted,
            declared_hosts: &record.manifest.network.hosts,
            settings_schema: &record.manifest.contributions.settings,
            subscriptions: &self.inner.subscriptions,
            ui: self.inner.ui.as_ref(),
        };

        // Everything this call causes is attributed to the plugin, which is
        // what keeps the resulting events from coming back to it.
        let result = events::with_origin(plugin_id, || {
            dispatch::dispatch(&cx, &request.method, &request.args)
        });
        HostResponse::from_result(request.request_id, result)
    }

    fn log(&self, plugin_id: &PluginId, level: LogLevel, message: &str) {
        logs::append(&self.inner.logs_root, plugin_id, level, message);
    }
}

// ---------------------------------------------------------------------------
// The service
// ---------------------------------------------------------------------------

/// Core-owned plugin management and execution.
#[derive(Clone)]
pub struct PluginService {
    inner: Arc<Inner>,
    bus: PluginEventBus,
}

impl PluginService {
    pub fn new(ctx: Ctx, host: Arc<dyn ScriptHost>, ui: Arc<dyn PluginUiSink>) -> Self {
        let subscriptions = EventSubscriptions::new();
        let logs_root = ctx.paths.plugin_logs_root();
        let inner = Arc::new(Inner {
            ctx,
            host: host.clone(),
            ui,
            subscriptions: subscriptions.clone(),
            resolution: RwLock::new(Resolution::default()),
            logs_root,
        });
        Self {
            bus: PluginEventBus::new(subscriptions, host),
            inner,
        }
    }

    /// A service with no UI sink, for the CLI and for tests.
    pub fn headless(ctx: Ctx, host: Arc<dyn ScriptHost>) -> Self {
        Self::new(ctx, host, Arc::new(NoopUiSink))
    }

    pub fn events(&self) -> &PluginEventBus {
        &self.bus
    }

    /// Whether the user has turned the plugin system on at all.
    ///
    /// Checked before anything is loaded. A user who never opts in never has a
    /// plugin runtime in their process.
    pub fn is_enabled(&self) -> bool {
        self.inner
            .conn()
            .ok()
            .and_then(|conn| {
                crate::db::get_setting(&conn, PLUGINS_ENABLED_SETTING)
                    .ok()
                    .flatten()
            })
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
    }

    // -- reading -----------------------------------------------------------

    /// Re-read the install records and work out what can run.
    pub fn reload(&self) -> LauncherResult<()> {
        let conn = self.inner.conn()?;
        let records = store::list(&conn)?;
        let resolution = registry::resolve(records, &agora_plugin_api::HOST_API_VERSION);
        if let Ok(mut slot) = self.inner.resolution.write() {
            *slot = resolution;
        }
        Ok(())
    }

    pub fn list(&self) -> Vec<PluginSummary> {
        let Ok(resolution) = self.inner.resolution.read() else {
            return Vec::new();
        };
        // Read once for the whole list rather than per plugin: this runs on
        // every refresh of the manager, and a query per row would be a query
        // per row for information that changes only on install or check.
        let (trust, checks) = self.trust_for_display();
        let checkpoints = self.checkpoints_for_display();
        resolution
            .plugins
            .iter()
            .map(|resolved| {
                let id = resolved.id().clone();
                let manifest = &resolved.record.manifest;
                PluginSummary {
                    id: id.to_string(),
                    name: manifest.name.clone(),
                    version: manifest.version.to_string(),
                    description: manifest.description.clone(),
                    license: manifest.license.clone(),
                    source_url: manifest.source.clone(),
                    enabled: resolved.record.enabled,
                    running: self.inner.host.is_active(&id),
                    development: resolved.record.source.is_development(),
                    status_text: resolved.status.explain(),
                    status: resolved.status.clone(),
                    capabilities: resolved
                        .record
                        .granted
                        .iter()
                        .map(|cap| cap.as_str().to_string())
                        .collect(),
                    declared_hosts: manifest.network.hosts.clone(),
                    definitions: manifest.contributions.clone(),
                    contributions: resolution
                        .contributions
                        .iter()
                        .filter(|contribution| contribution.plugin_id == id.as_str())
                        .cloned()
                        .collect(),
                    installed_at: resolved.record.installed_at.clone(),
                    updated_at: resolved.record.updated_at.clone(),
                    update_source: trust.get(id.as_str()).cloned(),
                    last_update_check: checks.get(id.as_str()).cloned(),
                    restorable_data: checkpoints.get(id.as_str()).cloned(),
                    dropped_events: self.bus.dropped_count(&id),
                }
            })
            .collect()
    }

    /// Pinned update sources and last check results, keyed by plugin id.
    ///
    /// Failure is silent and empty: a plugin manager that will not render
    /// because the trust table could not be read is worse than one that does
    /// not show where updates come from.
    #[allow(clippy::type_complexity)]
    fn trust_for_display(
        &self,
    ) -> (
        BTreeMap<String, install::UpdateSourceSummary>,
        BTreeMap<String, UpdateCheckRecord>,
    ) {
        let mut sources = BTreeMap::new();
        let mut checks = BTreeMap::new();
        let Ok(conn) = self.inner.conn() else {
            return (sources, checks);
        };
        let Ok(resolution) = self.inner.resolution.read() else {
            return (sources, checks);
        };
        for resolved in &resolution.plugins {
            let id = resolved.id();
            let Ok(Some(record)) = store::get_trust(&conn, id) else {
                continue;
            };
            sources.insert(
                id.to_string(),
                install::UpdateSourceSummary::from_parts(&record.url, &record.keys),
            );
            if let (Some(at), Some(result)) = (record.last_checked_at, record.last_result) {
                checks.insert(id.to_string(), UpdateCheckRecord { at, result });
            }
        }
        (sources, checks)
    }

    // -- replaceable surfaces ----------------------------------------------

    /// Every replaceable surface, who has offered to render it, and what will.
    ///
    /// The whole point of the design is here: a plugin declaring a replacement
    /// does not get the surface. It joins a list, and Agora's own view is what
    /// renders until the user says otherwise. Two plugins offering the same
    /// surface is a list of two rather than a race won by install order.
    pub fn surfaces(&self) -> Vec<SurfaceChoice> {
        let selections = self.surface_selections();
        let offers = self.replacement_offers();
        let runnable: BTreeMap<String, bool> = self
            .list()
            .into_iter()
            .map(|plugin| (plugin.id.clone(), plugin.running || plugin.enabled))
            .collect();

        ReplaceableSurface::ALL
            .iter()
            .map(|surface| {
                let key = surface.as_str().to_string();
                let mine: Vec<ReplacementOffer> = offers
                    .iter()
                    .filter(|offer| offer.surface == key)
                    .cloned()
                    .collect();
                let selected = selections.get(&key).cloned();

                // Resolved rather than assumed. A selection survives the plugin
                // being disabled or uninstalled, so that re-enabling restores
                // the user's choice instead of silently forgetting it — but it
                // never decides what renders.
                let (effective, fallback_reason) = match &selected {
                    None => (None, None),
                    Some(chosen) => match mine.iter().find(|offer| &offer.id == chosen) {
                        None => (
                            None,
                            Some(format!(
                                "`{chosen}` is no longer installed, so Agora's own view is \
                                 being shown."
                            )),
                        ),
                        Some(offer) if runnable.get(&offer.plugin_id).copied().unwrap_or(false) => {
                            (Some(offer.clone()), None)
                        }
                        Some(offer) => (
                            None,
                            Some(format!(
                                "`{}` is not running, so Agora's own view is being shown.",
                                offer.plugin_name
                            )),
                        ),
                    },
                };

                SurfaceChoice {
                    surface: key,
                    title: surface.title().to_string(),
                    offers: mine,
                    selected,
                    effective,
                    fallback_reason,
                }
            })
            .collect()
    }

    /// Choose who renders a surface. `None` restores Agora's own view.
    ///
    /// Accepts only an offer that exists right now. A selection pointing at
    /// something that was never installed is not a preference to be preserved,
    /// it is a typo or a stale client, and storing it would produce a fallback
    /// notice the user can neither act on nor clear.
    pub fn set_surface(
        &self,
        surface: ReplaceableSurface,
        contribution_id: Option<&str>,
    ) -> LauncherResult<()> {
        let mut selections = self.surface_selections();
        match contribution_id {
            None => {
                selections.remove(surface.as_str());
            }
            Some(id) => {
                let known = self
                    .replacement_offers()
                    .into_iter()
                    .any(|offer| offer.id == id && offer.surface == surface.as_str());
                if !known {
                    return Err(LauncherError::Generic {
                        code: "ERR_PLUGIN_SURFACE_UNKNOWN".into(),
                        message: format!("no installed plugin offers `{id}` for {surface}"),
                    });
                }
                selections.insert(surface.as_str().to_string(), id.to_string());
            }
        }
        let conn = self.inner.conn()?;
        crate::db::set_setting(
            &conn,
            SURFACE_SELECTIONS_SETTING,
            &serde_json::to_value(&selections).unwrap_or(serde_json::Value::Null),
        )
        .map_err(|error| LauncherError::Generic {
            code: "ERR_LOCAL_STATE_FAILED".into(),
            message: error.to_string(),
        })
    }

    /// The stored selections, ignoring anything that is not a string pair.
    fn surface_selections(&self) -> BTreeMap<String, String> {
        self.inner
            .conn()
            .ok()
            .and_then(|conn| {
                crate::db::get_setting(&conn, SURFACE_SELECTIONS_SETTING)
                    .ok()
                    .flatten()
            })
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default()
    }

    /// Every offer from every installed plugin, runnable or not.
    fn replacement_offers(&self) -> Vec<ReplacementOffer> {
        let Ok(resolution) = self.inner.resolution.read() else {
            return Vec::new();
        };
        let mut offers = Vec::new();
        for resolved in &resolution.plugins {
            let manifest = &resolved.record.manifest;
            for replacement in &manifest.contributions.replacements {
                // Only the host-rendered form reaches here; the manifest
                // validator refuses anything else, so this is a `let else`
                // rather than a branch with a user-facing message.
                let ViewSource::Host { export } = &replacement.view else {
                    continue;
                };
                offers.push(ReplacementOffer {
                    id: manifest.id.qualify(&replacement.id),
                    plugin_id: manifest.id.to_string(),
                    local_id: replacement.id.clone(),
                    plugin_name: manifest.name.clone(),
                    title: replacement.title.clone(),
                    description: replacement.description.clone(),
                    surface: replacement.surface.as_str().to_string(),
                    export: export.clone(),
                });
            }
        }
        // Deterministic, so the picker does not reorder itself between reads.
        offers.sort_by(|a, b| a.id.cmp(&b.id));
        offers
    }

    /// Restorable data copies, keyed by plugin id.
    ///
    /// Silent on failure for the same reason as the trust read: a manager that
    /// will not render is worse than one missing a recovery affordance.
    fn checkpoints_for_display(&self) -> BTreeMap<String, store::CheckpointSummary> {
        let mut out = BTreeMap::new();
        let Ok(conn) = self.inner.conn() else {
            return out;
        };
        let Ok(resolution) = self.inner.resolution.read() else {
            return out;
        };
        for resolved in &resolution.plugins {
            if let Ok(Some(summary)) = store::latest_checkpoint(&conn, resolved.id()) {
                out.insert(resolved.id().to_string(), summary);
            }
        }
        out
    }

    /// Contributions from every plugin that is currently runnable.
    pub fn contributions(&self) -> Vec<NamespacedContribution> {
        if !self.is_enabled() {
            return Vec::new();
        }
        self.inner
            .resolution
            .read()
            .map(|resolution| resolution.contributions.clone())
            .unwrap_or_default()
    }

    pub fn logs(&self, plugin_id: &PluginId, lines: usize) -> Vec<String> {
        logs::tail(&self.inner.logs_root, plugin_id, lines)
    }

    // -- installing --------------------------------------------------------

    pub fn preview_package(&self, archive: &Path) -> LauncherResult<InstallPreview> {
        let existing = self.existing_install(archive_id(archive).as_ref());
        install::preview_package(
            archive,
            existing.as_ref().map(ExistingRecord::as_ref).as_ref(),
        )
    }

    pub fn preview_folder(&self, folder: &Path) -> LauncherResult<InstallPreview> {
        let manifest = install::read_folder_manifest(folder)?;
        let existing = self.existing_install(Some(&manifest.id));
        install::preview_folder(
            folder,
            existing.as_ref().map(ExistingRecord::as_ref).as_ref(),
        )
    }

    /// The live install under this id, or `None` if there is not one.
    ///
    /// A tombstone — uninstalled with its data kept — is deliberately `None`.
    /// Keeping someone's settings across an uninstall is a convenience; it is
    /// not a standing capability grant, so reinstalling asks again.
    fn existing_install(&self, plugin_id: Option<&PluginId>) -> Option<ExistingRecord> {
        let plugin_id = plugin_id?;
        let conn = self.inner.conn().ok()?;
        match store::get(&conn, plugin_id) {
            Ok(Some(record)) if !store::is_tombstone(&record) => Some(ExistingRecord {
                version: record.manifest.version.clone(),
                data_version: record.data_version,
                hosts: record.manifest.network.hosts.clone(),
                granted: record.granted,
            }),
            _ => None,
        }
    }

    /// Install a package, replacing an existing version if there is one.
    ///
    /// `accept_capabilities` is the user's consent. It is a parameter rather
    /// than something this function prompts for, because the prompt belongs to
    /// whichever adapter is in front of the user — and because a caller that
    /// forgets to ask gets a compile error rather than a silent grant.
    pub fn install_package(
        &self,
        archive: &Path,
        accept_capabilities: bool,
    ) -> LauncherResult<PluginSummary> {
        let contents = install::read_package_manifest(archive)?;
        let manifest = contents.manifest;

        let destination =
            install::package_dir(&self.inner.ctx.paths.plugin_packages_root(), &manifest.id);
        let rollback = self
            .inner
            .ctx
            .paths
            .plugin_rollback_root()
            .join(manifest.id.as_str());

        let conn = self.inner.conn()?;
        let previous = store::get(&conn, &manifest.id)?;
        let live = self.existing_install(Some(&manifest.id));
        self.guard_consent(&manifest, live.as_ref(), accept_capabilities)?;

        // Resolved before anything moves. `grant_for` rejects a manifest that
        // requires a capability this build cannot provide, and discovering
        // that *after* the swap would leave the new files in place with the
        // old ones stranded in the rollback directory.
        let granted = install::grant_for(&manifest)?;

        // Anything currently running must stop before its files move.
        let _ = self.deactivate(&manifest.id);

        if let Some(previous) = &previous {
            if previous.data_version != manifest.data_version && !store::is_tombstone(previous) {
                store::capture_checkpoint(
                    &conn,
                    &manifest.id,
                    &previous.manifest.version.to_string(),
                    previous.data_version,
                    &now(&self.inner.ctx),
                )?;
            }
        }

        let had_files = destination.exists();
        if had_files {
            install::stash_for_rollback(&destination, &rollback)?;
        }

        if let Err(error) = install::extract_package(archive, &destination) {
            if had_files {
                // Best effort: if this also fails the user is told, rather
                // than being left believing the old version is back.
                install::restore_rollback(&rollback, &destination)?;
            }
            return Err(error);
        }

        // Updating something the user had switched off must not switch it back
        // on. Only a first install defaults to enabled; a replacement inherits
        // the state the user chose, and a reinstall over a tombstone is a
        // first install again.
        let enabled = match &previous {
            Some(record) if !store::is_tombstone(record) => record.enabled,
            _ => true,
        };

        if let Err(error) = store::upsert(
            &conn,
            &manifest,
            &granted,
            &PluginSource::Package,
            &destination,
            enabled,
            &now(&self.inner.ctx),
        ) {
            // The files are already swapped. Leaving them there would put the
            // new version on disk under the old version's record — the install
            // would look like it never happened while the code on disk had in
            // fact changed.
            if had_files {
                let _ = std::fs::remove_dir_all(&destination);
                install::restore_rollback(&rollback, &destination)?;
            } else {
                let _ = std::fs::remove_dir_all(&destination);
            }
            return Err(error);
        }
        let _ = std::fs::remove_dir_all(&rollback);

        // Pinned at the moment consent was given, and only then. A package
        // that declares no update source clears any previous pin rather than
        // inheriting it, so a plugin cannot acquire an update channel by
        // shipping a version that simply omits the file.
        match &contents.update_source {
            Some(source) => store::put_trust(&conn, &manifest.id, &source.url, &source.keys)?,
            None => store::clear_trust(&conn, &manifest.id)?,
        }

        self.reload()?;
        self.summary(&manifest.id)
    }

    // -- updating ----------------------------------------------------------

    /// Fetch this plugin's update document, verify it, and say what it means.
    ///
    /// Does not install anything and does not download a package. The only
    /// bytes fetched are the small signed JSON document, so a check is cheap,
    /// and a check that fails leaves nothing behind but a recorded reason.
    pub fn check_update(&self, plugin_id: &PluginId) -> LauncherResult<UpdateVerdict> {
        let (record, trust) = self.update_subject(plugin_id)?;

        let body = self.fetch_update_document(&trust.url)?;
        let text = String::from_utf8(body).map_err(|_| LauncherError::Generic {
            code: "ERR_PLUGIN_UPDATE_REFUSED".into(),
            message: "the update document is not text".into(),
        })?;
        let document = UpdateDocument::parse(&text).map_err(|error| LauncherError::Generic {
            code: "ERR_PLUGIN_UPDATE_REFUSED".into(),
            message: error.message,
        })?;

        let pinned = updates::PinnedTrust {
            keys: trust.keys.clone(),
            highest_sequence: trust.highest_sequence,
        };

        // Verification first, always. Nothing below this line may run against a
        // document that has not been vouched for by a pinned key, which is why
        // `decide` is a separate function taking an already-checked one.
        let conn = self.inner.conn()?;
        let verified = updates::verify_document(&pinned, plugin_id, &document);
        if let Err(error) = &verified {
            // Recorded but the sequence is left where it was: a document that
            // did not verify must not be able to raise the replay floor and
            // lock the user out of the real one.
            store::record_check(
                &conn,
                plugin_id,
                trust.highest_sequence,
                &now(&self.inner.ctx),
                &error.to_string(),
            )?;
        }
        verified?;

        let verdict = updates::decide(
            &record.manifest.version,
            &agora_plugin_api::HOST_API_VERSION,
            &document,
        );
        store::record_check(
            &conn,
            plugin_id,
            document.sequence,
            &now(&self.inner.ctx),
            &verdict_summary(&verdict),
        )?;
        Ok(verdict)
    }

    // -- data recovery -----------------------------------------------------

    /// The data copy that could be put back, if there is one.
    ///
    /// A checkpoint is taken automatically before an update that changes a
    /// plugin's `dataVersion`, because that is the moment a plugin is about to
    /// migrate its own stored settings and possibly get it wrong.
    pub fn restorable_data(&self, plugin_id: &PluginId) -> Option<store::CheckpointSummary> {
        let conn = self.inner.conn().ok()?;
        store::latest_checkpoint(&conn, plugin_id).ok().flatten()
    }

    /// Put a plugin's stored data back to its most recent checkpoint.
    ///
    /// Never automatic, and deliberately so. Restoring after a failed
    /// migration sounds obviously right until you notice it throws away
    /// everything the plugin wrote *since* the checkpoint — which, if the
    /// migration half-succeeded, is real data the user would rather keep. The
    /// launcher cannot tell those apart, so it offers and the user decides.
    ///
    /// Returns how many stored entries were put back.
    pub fn restore_data(&self, plugin_id: &PluginId) -> LauncherResult<usize> {
        let conn = self.inner.conn()?;
        if store::get(&conn, plugin_id)?.is_none() {
            return Err(LauncherError::Generic {
                code: "ERR_PLUGIN_NOT_FOUND".into(),
                message: format!("`{plugin_id}` is not installed"),
            });
        }
        // Stopped first. A running plugin holds no lock on its storage, so a
        // restore underneath one would leave it acting on data that no longer
        // exists — and the next thing it writes would overwrite the restore.
        let _ = self.deactivate(plugin_id);

        let restored = store::restore_latest_checkpoint(&conn, plugin_id)?.ok_or_else(|| {
            LauncherError::Generic {
                code: "ERR_PLUGIN_NO_CHECKPOINT".into(),
                message: format!(
                    "there is no saved copy of `{plugin_id}` data to go back to. A copy is kept \\
                     only when an update changes how the plugin stores its data."
                ),
            }
        })?;
        self.reload()?;
        Ok(restored)
    }

    /// Whether the user asked for update checks to happen on their own.
    ///
    /// Separate from [`PLUGINS_ENABLED_SETTING`] because they are different
    /// questions: one is whether plugins run at all, the other is whether
    /// Agora contacts a publisher without being asked. Off by default, like
    /// everything else here that reaches the network.
    pub fn automatic_updates_enabled(&self) -> bool {
        self.inner
            .conn()
            .ok()
            .and_then(|conn| {
                crate::db::get_setting(&conn, PLUGIN_UPDATES_ENABLED_SETTING)
                    .ok()
                    .flatten()
            })
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
    }

    /// Check every plugin that has somewhere to check, if that was asked for.
    ///
    /// Returns an empty list — and contacts nothing — when the setting is off,
    /// so the caller does not have to know the rule. That is deliberate: a
    /// policy question answered in one place cannot be answered differently by
    /// the GUI and the CLI.
    ///
    /// Best effort by design. A publisher being unreachable is not a launcher
    /// failure, so the reason is recorded on the trust record for the manager
    /// to show and nothing is raised at the user.
    pub fn check_all_updates(&self) -> Vec<(String, Result<UpdateVerdict, String>)> {
        if !self.automatic_updates_enabled() || !self.is_enabled() {
            return Vec::new();
        }
        let Ok(conn) = self.inner.conn() else {
            return Vec::new();
        };
        let candidates: Vec<PluginId> = self
            .list()
            .into_iter()
            .filter(|plugin| !plugin.development && plugin.update_source.is_some())
            .filter_map(|plugin| PluginId::parse(&plugin.id).ok())
            .collect();
        drop(conn);

        candidates
            .into_iter()
            .map(|plugin_id| {
                let outcome = self
                    .check_update(&plugin_id)
                    .map_err(|error| error.to_string());
                (plugin_id.to_string(), outcome)
            })
            .collect()
    }

    /// Download and install the release [`Self::check_update`] offered.
    ///
    /// Re-checks rather than trusting a verdict handed back in. Between the
    /// check and the click the publisher may have published again, and the
    /// decision about which bytes to fetch has to come from a document
    /// verified during this call.
    pub fn apply_update(
        &self,
        plugin_id: &PluginId,
        accept_capabilities: bool,
    ) -> LauncherResult<UpdateOutcome> {
        let verdict = self.check_update(plugin_id)?;
        let UpdateVerdict::Available {
            to,
            url,
            sha256,
            size,
            ..
        } = &verdict
        else {
            return Err(LauncherError::Generic {
                code: "ERR_PLUGIN_UPDATE_NONE".into(),
                message: format!(
                    "`{plugin_id}` has no update to apply ({})",
                    verdict_summary(&verdict)
                ),
            });
        };

        let bytes = self.fetch_update_package(url, *size)?;
        self.install_downloaded_update(plugin_id, to, sha256, *size, &bytes, accept_capabilities)
    }

    /// Check downloaded bytes against the signed release and install them.
    ///
    /// Split from [`Self::apply_update`] so that everything after the fetch —
    /// which is all of the logic worth being sure about — can be tested
    /// against real bytes without a server. That is not merely convenient:
    /// the network gate rejects loopback addresses by design, so a test that
    /// stood up a local HTTP server would be testing the gate refusing it.
    ///
    /// Public because it is also the honest shape of an offline update: bytes
    /// obtained some other way, checked against the hash and size a signed
    /// release declared. It grants nothing on its own — the capability
    /// comparison still happens inside `install_package`.
    pub fn install_downloaded_update(
        &self,
        plugin_id: &PluginId,
        version: &str,
        sha256: &str,
        size: u64,
        bytes: &[u8],
        accept_capabilities: bool,
    ) -> LauncherResult<UpdateOutcome> {
        // The signed document said what these bytes must be. The host that
        // served them is trusted for nothing else: it may refuse, and it may
        // serve something different, but something different fails here.
        let release = Release {
            version: version.parse().map_err(|_| LauncherError::Generic {
                code: "ERR_PLUGIN_UPDATE_REFUSED".into(),
                message: format!("`{version}` is not a version"),
            })?,
            url: String::new(),
            sha256: sha256.to_string(),
            size,
            // Compatibility was already decided by `decide`, which only offers
            // a release whose range matches this host.
            api_range: semver::VersionReq::STAR,
            notes: None,
            published: None,
        };
        updates::verify_package_bytes(&release, bytes)?;

        // Staged inside Agora's own directory rather than a system temp dir,
        // so a crash leaves the debris somewhere the launcher already owns.
        let staging = self
            .inner
            .ctx
            .paths
            .plugin_rollback_root()
            .join("downloads");
        std::fs::create_dir_all(&staging).map_err(|e| LauncherError::Generic {
            code: "ERR_PLUGIN_IO".into(),
            message: format!("could not prepare {}: {e}", staging.display()),
        })?;
        let archive = staging.join(format!("{plugin_id}-{version}.zip"));
        std::fs::write(&archive, bytes).map_err(|e| LauncherError::Generic {
            code: "ERR_PLUGIN_IO".into(),
            message: format!("could not write the downloaded package: {e}"),
        })?;

        // Asked before installing, so that a release wanting more permission
        // can be *described* rather than merely refused. The adapter needs the
        // same preview the install prompt uses; discovering the refusal from
        // an error string would leave it with nothing to show.
        let preview = self.preview_package(&archive);
        let needs_consent = preview
            .as_ref()
            .map(|preview| preview.requires_capability_consent())
            .unwrap_or(false);
        if needs_consent && !accept_capabilities {
            let _ = std::fs::remove_dir_all(&staging);
            return Ok(UpdateOutcome::NeedsConsent {
                preview: Box::new(preview.expect("checked above")),
            });
        }

        // Deliberately the same call a manual install makes. An update is not
        // a privileged path: it gets the same capability comparison, the same
        // data checkpoint, the same staging and rollback, and the same refusal
        // when it asks for more than was granted.
        let result = self.install_package(&archive, accept_capabilities);
        let _ = std::fs::remove_file(&archive);

        // The downloaded bytes are not kept. Retaining them would be a second
        // copy of something already on disk, with no way to re-verify it later
        // once the signed document that described it has moved on.
        let _ = std::fs::remove_dir_all(&staging);
        result.map(|summary| UpdateOutcome::Installed {
            plugin: Box::new(summary),
        })
    }

    /// The record and trust for a plugin that can meaningfully be updated.
    fn update_subject(
        &self,
        plugin_id: &PluginId,
    ) -> LauncherResult<(store::PluginRecord, store::TrustRecord)> {
        if !self.is_enabled() {
            return Err(LauncherError::Generic {
                code: "ERR_PLUGINS_DISABLED".into(),
                message: "the plugin system is switched off".into(),
            });
        }
        let conn = self.inner.conn()?;
        let record = store::get(&conn, plugin_id)?
            .filter(|record| !store::is_tombstone(record))
            .ok_or_else(|| LauncherError::Generic {
                code: "ERR_PLUGIN_NOT_FOUND".into(),
                message: format!("`{plugin_id}` is not installed"),
            })?;
        if record.source.is_development() {
            return Err(LauncherError::Generic {
                code: "ERR_PLUGIN_UPDATE_NONE".into(),
                message: format!(
                    "`{plugin_id}` is a development folder you are editing; Agora will not \
                     overwrite your working copy"
                ),
            });
        }
        let trust = store::get_trust(&conn, plugin_id)?.ok_or_else(|| LauncherError::Generic {
            code: "ERR_PLUGIN_UPDATE_NONE".into(),
            message: format!(
                "`{plugin_id}` did not come with an update source, so there is nowhere to check \
                 and no key to check it against"
            ),
        })?;
        Ok((record, trust))
    }

    /// Fetch the signed document. Small, so it is read whole.
    fn fetch_update_document(&self, url: &str) -> LauncherResult<Vec<u8>> {
        let host = [host_of(url)];
        crate::http_client::blocking_checked_get_bytes_with_policy(
            &self.inner.ctx.http_clients,
            crate::http_client::ClientCategory::PluginUpdate,
            url,
            // The host was pinned when the user installed this plugin, so it is
            // authorisation the user actually gave rather than a list this build
            // compiled in. Every other gate — Lockdown, HTTPS, the private and
            // loopback address rejection — still applies on top of it.
            crate::http_client::HostPolicy::PluginDeclared(&host),
        )
    }

    /// Fetch package bytes named by an already-verified document.
    fn fetch_update_package(&self, url: &str, expected: u64) -> LauncherResult<Vec<u8>> {
        if expected > install::MAX_TOTAL_BYTES {
            return Err(LauncherError::Generic {
                code: "ERR_PLUGIN_UPDATE_REFUSED".into(),
                message: format!(
                    "the signed release declares {expected} bytes, past the {} a plugin \
                     package may be",
                    install::MAX_TOTAL_BYTES
                ),
            });
        }
        let host = [host_of(url)];
        crate::http_client::blocking_checked_get_bytes_with_policy(
            &self.inner.ctx.http_clients,
            crate::http_client::ClientCategory::PluginPackage,
            url,
            // Authorisation here comes from the signature over the document
            // that named this URL *and* its hash, so the host allowlist is a
            // formality by comparison. It is still routed through the gate so
            // Lockdown and the address checks apply.
            crate::http_client::HostPolicy::PluginDeclared(&host),
        )
    }

    /// Load a folder the author is working in, without copying it.
    pub fn add_development_folder(
        &self,
        folder: &Path,
        accept_capabilities: bool,
    ) -> LauncherResult<PluginSummary> {
        let manifest = install::read_folder_manifest(folder)?;

        let conn = self.inner.conn()?;
        let live = self.existing_install(Some(&manifest.id));
        self.guard_consent(&manifest, live.as_ref(), accept_capabilities)?;

        if let Some(existing) = store::get(&conn, &manifest.id)? {
            if !store::is_tombstone(&existing) && !existing.source.is_development() {
                return Err(LauncherError::Generic {
                    code: "ERR_PLUGIN_ID_TAKEN".into(),
                    message: format!(
                        "`{}` is already installed as a package. Remove it before loading a \
                         development copy, so it is never ambiguous which one is running.",
                        manifest.id
                    ),
                });
            }
        }

        let _ = self.deactivate(&manifest.id);
        let granted = install::grant_for(&manifest)?;
        store::upsert(
            &conn,
            &manifest,
            &granted,
            &PluginSource::Development {
                path: folder.to_path_buf(),
            },
            folder,
            true,
            &now(&self.inner.ctx),
        )?;
        self.reload()?;
        self.summary(&manifest.id)
    }

    /// Refuse an install that would grant more than the user has agreed to.
    ///
    /// `previous` is the grant already on record. Passing it is what makes a
    /// replacement different from a first install: reinstalling the same
    /// plugin with the same capabilities is not a new decision and should not
    /// be re-asked, while an update that wants something further must be.
    fn guard_consent(
        &self,
        manifest: &agora_plugin_api::PluginManifest,
        previous: Option<&ExistingRecord>,
        accepted: bool,
    ) -> LauncherResult<()> {
        let added = install::added_capabilities(previous.map(|p| &p.granted), manifest);
        let hosts = install::added_hosts(previous.map(|p| p.hosts.as_slice()), manifest);
        if (added.is_empty() && hosts.is_empty()) || accepted {
            return Ok(());
        }
        let mut wants = Vec::new();
        if !added.is_empty() {
            wants.push(added.join(", "));
        }
        if !hosts.is_empty() {
            wants.push(format!("network access to {}", hosts.join(", ")));
        }
        let wants = wants.join("; and ");
        let message = if previous.is_some() {
            format!(
                "`{}` now asks for {wants}, which was not granted before; installing it is a                  new decision",
                manifest.id
            )
        } else {
            format!(
                "`{}` asks for {wants}, which has not been accepted",
                manifest.id
            )
        };
        Err(LauncherError::Generic {
            code: "ERR_PLUGIN_CONSENT_REQUIRED".into(),
            message,
        })
    }

    pub fn set_enabled(&self, plugin_id: &PluginId, enabled: bool) -> LauncherResult<()> {
        let conn = self.inner.conn()?;
        if !store::set_enabled(&conn, plugin_id, enabled, &now(&self.inner.ctx))? {
            return Err(LauncherError::Generic {
                code: "ERR_PLUGIN_NOT_FOUND".into(),
                message: format!("`{plugin_id}` is not installed"),
            });
        }
        // Clearing the recorded failure is what makes "turn it off and on
        // again" a real recovery rather than a no-op.
        if enabled {
            store::set_last_error(&conn, plugin_id, None)?;
        } else {
            self.deactivate(plugin_id)?;
        }
        self.reload()
    }

    /// Remove a plugin. `purge_data` is a separate decision from removing it.
    pub fn uninstall(&self, plugin_id: &PluginId, purge_data: bool) -> LauncherResult<()> {
        self.deactivate(plugin_id)?;
        let conn = self.inner.conn()?;
        let Some(record) = store::get(&conn, plugin_id)? else {
            return Err(LauncherError::Generic {
                code: "ERR_PLUGIN_NOT_FOUND".into(),
                message: format!("`{plugin_id}` is not installed"),
            });
        };
        install::remove_package_files(&record.source, &record.install_dir)?;
        logs::remove(&self.inner.logs_root, plugin_id);
        store::remove(&conn, plugin_id, purge_data)?;
        self.reload()
    }

    // -- running -----------------------------------------------------------

    /// Start the plugins that asked to run at startup, in dependency order.
    ///
    /// **Not every runnable plugin.** A plugin that declares `onView:` or
    /// `onCommand:` and nothing else is left alone here and started the moment
    /// something actually needs it, which is what keeps a dozen installed
    /// plugins from becoming a dozen runtimes at boot. Starting everything
    /// eagerly would make the manifest's activation events decorative.
    ///
    /// A plugin that fails to activate is recorded and skipped; the rest still
    /// start. One broken plugin disabling the others would make the system
    /// exactly as fragile as having no isolation at all.
    pub fn activate_all(&self) -> LauncherResult<Vec<(PluginId, PluginError)>> {
        if !self.is_enabled() {
            return Ok(Vec::new());
        }
        self.reload()?;
        let eager: Vec<PluginId> = self
            .inner
            .resolution
            .read()
            .map(|resolution| {
                resolution
                    .activation_order
                    .iter()
                    .filter(|plugin_id| {
                        resolution
                            .get(plugin_id)
                            .is_some_and(|resolved| wants_startup(&resolved.record.manifest))
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();

        let mut failures = Vec::new();
        for plugin_id in eager {
            if let Err(error) = self.activate(&plugin_id) {
                failures.push((plugin_id, error));
            }
        }
        if !failures.is_empty() {
            self.reload()?;
        }
        Ok(failures)
    }

    /// Publish a launcher event to plugins, starting any that were waiting for it.
    ///
    /// Adapters call this rather than [`PluginEventBus::publish`] directly. A
    /// plugin whose only activation event is `onEvent:content.installed` has
    /// not run yet, so it has not subscribed to anything — going straight to
    /// the bus would deliver to nobody and the declaration would never fire.
    pub fn publish_event(
        &self,
        event: agora_plugin_api::protocol::PluginEvent,
        operation_id: Option<String>,
    ) -> usize {
        if !self.is_enabled() {
            return 0;
        }
        self.activate_waiting_for_event(event.name());
        self.bus.publish(event, operation_id)
    }

    /// Start any runnable plugin that declared `onEvent:<name>`.
    fn activate_waiting_for_event(&self, event_name: &str) {
        let waiting: Vec<PluginId> = self
            .inner
            .resolution
            .read()
            .map(|resolution| {
                resolution
                    .runnable()
                    .filter(|resolved| {
                        resolved.record.manifest.activation.iter().any(|activation| {
                            matches!(activation, ActivationEvent::Event(name) if name == event_name)
                        })
                    })
                    .map(|resolved| resolved.id().clone())
                    .collect()
            })
            .unwrap_or_default();
        for plugin_id in waiting {
            if !self.inner.host.is_active(&plugin_id) {
                // A failure here is already recorded against the plugin; the
                // event still goes to everyone else.
                let _ = self.activate(&plugin_id);
            }
        }
    }

    /// Start any plugin that asked to run when the user opens an instance.
    ///
    /// Contributed panels already start their plugin when they render. This
    /// exists for a plugin that wants to react to an instance being opened
    /// without putting anything on screen.
    pub fn notify_instance_opened(&self) -> usize {
        if !self.is_enabled() {
            return 0;
        }
        let waiting: Vec<PluginId> = self
            .inner
            .resolution
            .read()
            .map(|resolution| {
                resolution
                    .runnable()
                    .filter(|resolved| {
                        resolved
                            .record
                            .manifest
                            .activation
                            .iter()
                            .any(|activation| matches!(activation, ActivationEvent::InstanceOpened))
                    })
                    .map(|resolved| resolved.id().clone())
                    .collect()
            })
            .unwrap_or_default();
        let mut started = 0;
        for plugin_id in waiting {
            if !self.inner.host.is_active(&plugin_id) && self.activate(&plugin_id).is_ok() {
                started += 1;
            }
        }
        started
    }

    pub fn activate(&self, plugin_id: &PluginId) -> Result<(), PluginError> {
        let Some(record) = self.inner.record(plugin_id) else {
            return Err(PluginError::new(
                PluginErrorCode::NotActivated,
                format!("`{plugin_id}` is not installed"),
            ));
        };
        if record.manifest.is_declarative_only() {
            // A theme has no script. There is nothing to activate, and
            // pretending otherwise would spawn a runtime to do nothing.
            return Ok(());
        }
        if self.inner.host.is_active(plugin_id) {
            return Ok(());
        }

        let settings = self.settings_snapshot(plugin_id, &record);
        let request = ActivationRequest {
            plugin_id: plugin_id.clone(),
            package_root: record.install_dir.clone(),
            entrypoint: record.manifest.entrypoint.clone().unwrap_or_default(),
            memory_limit_bytes: ActivationRequest::DEFAULT_MEMORY_LIMIT,
            stack_limit_bytes: ActivationRequest::DEFAULT_STACK_LIMIT,
            api_version: agora_plugin_api::host_api_version_string(),
            settings,
        };
        let bridge: Arc<dyn HostBridge> = Arc::new(CoreBridge {
            inner: self.inner.clone(),
        });

        match self.inner.host.activate(request, bridge) {
            Ok(()) => Ok(()),
            Err(error) => {
                // Recorded so the manager can explain the failure without the
                // user reading a log, and so resolution holds it back next
                // time rather than retrying on every startup.
                if let Ok(conn) = self.inner.conn() {
                    let _ = store::set_last_error(&conn, plugin_id, Some(&error.message));
                }
                logs::append(
                    &self.inner.logs_root,
                    plugin_id,
                    LogLevel::Error,
                    &format!("activation failed: {}", error.message),
                );
                Err(error)
            }
        }
    }

    pub fn deactivate(&self, plugin_id: &PluginId) -> LauncherResult<()> {
        self.inner.subscriptions.clear(plugin_id);
        self.inner
            .host
            .deactivate(plugin_id)
            .map_err(|e| LauncherError::Generic {
                code: "ERR_PLUGIN_DEACTIVATE".into(),
                message: e.message,
            })
    }

    /// Stop and disable everything. The recovery path.
    ///
    /// Deliberately does not run any plugin code beyond the `deactivate` each
    /// one is already given, so it works when the thing that is wrong is a
    /// plugin that hangs the moment it is asked to do anything.
    pub fn disable_all(&self) -> LauncherResult<usize> {
        let conn = self.inner.conn()?;
        let records = store::list(&conn)?;
        let mut disabled = 0;
        for record in records {
            if store::is_tombstone(&record) || !record.enabled {
                continue;
            }
            let id = record.id().clone();
            self.inner.host.cancel(&id);
            let _ = self.deactivate(&id);
            store::set_enabled(&conn, &id, false, &now(&self.inner.ctx))?;
            disabled += 1;
        }
        self.reload()?;
        Ok(disabled)
    }

    fn settings_snapshot(&self, plugin_id: &PluginId, record: &PluginRecord) -> serde_json::Value {
        let Ok(conn) = self.inner.conn() else {
            return serde_json::json!({});
        };
        let mut values =
            store::storage_all(&conn, plugin_id, StorageKind::Setting, None).unwrap_or_default();
        for definition in &record.manifest.contributions.settings {
            values
                .entry(definition.key.clone())
                .or_insert_with(|| definition.schema.default_value());
        }
        serde_json::Value::Object(values)
    }

    /// A plugin's declared settings and their current values.
    ///
    /// Returned together because a settings page needs both and reading them
    /// separately invites the two drifting: a value for a setting that no
    /// longer exists, or a definition with no value because the default was
    /// never materialised.
    pub fn settings(&self, plugin_id: &PluginId) -> LauncherResult<PluginSettingsView> {
        let Some(record) = self.inner.record(plugin_id) else {
            return Err(LauncherError::Generic {
                code: "ERR_PLUGIN_NOT_FOUND".into(),
                message: format!("`{plugin_id}` is not installed"),
            });
        };
        let values = self.settings_snapshot(plugin_id, &record);
        Ok(PluginSettingsView {
            definitions: record.manifest.contributions.settings.clone(),
            values,
        })
    }

    /// Write a declared setting, validated against the schema that declared it.
    pub fn set_setting(
        &self,
        plugin_id: &PluginId,
        key: &str,
        value: &serde_json::Value,
    ) -> LauncherResult<()> {
        let Some(record) = self.inner.record(plugin_id) else {
            return Err(LauncherError::Generic {
                code: "ERR_PLUGIN_NOT_FOUND".into(),
                message: format!("`{plugin_id}` is not installed"),
            });
        };
        let Some(definition) = record
            .manifest
            .contributions
            .settings
            .iter()
            .find(|definition| definition.key == key)
        else {
            return Err(LauncherError::Generic {
                code: "ERR_PLUGIN_SETTING_UNKNOWN".into(),
                message: format!("`{plugin_id}` declares no setting named `{key}`"),
            });
        };
        if !definition.schema.accepts(value) {
            return Err(LauncherError::Generic {
                code: "ERR_PLUGIN_SETTING_INVALID".into(),
                message: format!("that value is not valid for `{key}`"),
            });
        }
        let conn = self.inner.conn()?;
        store::storage_set(&conn, plugin_id, StorageKind::Setting, None, key, value)
    }

    // -- calling into plugins ---------------------------------------------

    /// Render a contributed view.
    pub fn render_view(
        &self,
        plugin_id: &PluginId,
        export: &str,
        args: serde_json::Value,
    ) -> Result<ViewModel, PluginError> {
        self.ensure_running(plugin_id)?;
        let value = self
            .inner
            .host
            .invoke(plugin_id, export, args, INVOKE_TIMEOUT)?;
        let model: ViewModel = serde_json::from_value(value).map_err(|e| {
            PluginError::new(
                PluginErrorCode::ScriptError,
                format!("`{export}` did not return a view Agora can render: {e}"),
            )
        })?;
        model.validate()?;
        Ok(model)
    }

    /// Run a contributed command.
    pub fn run_command(
        &self,
        plugin_id: &PluginId,
        export: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, PluginError> {
        self.ensure_running(plugin_id)?;
        self.inner
            .host
            .invoke(plugin_id, export, args, INVOKE_TIMEOUT)
    }

    /// Run a diagnostic and return what it found.
    pub fn run_diagnostic(
        &self,
        plugin_id: &PluginId,
        export: &str,
        args: serde_json::Value,
    ) -> Result<DiagnosticReport, PluginError> {
        self.ensure_running(plugin_id)?;
        let value = self
            .inner
            .host
            .invoke(plugin_id, export, args, DIAGNOSTIC_TIMEOUT)?;
        let report: DiagnosticReport = serde_json::from_value(value).map_err(|e| {
            PluginError::new(
                PluginErrorCode::ScriptError,
                format!("`{export}` did not return a diagnostic report: {e}"),
            )
        })?;
        report.validate()?;
        for finding in &report.findings {
            for repair in &finding.repairs {
                repair.validate()?;
            }
        }
        Ok(report)
    }

    /// Run every contributed launch check for an instance.
    ///
    /// Bounded twice over: each check gets at most its declared timeout,
    /// clamped to [`MAX_LAUNCH_CHECK_MS`], and a check that fails to run is
    /// reported as an error rather than being treated as a pass. Neither a
    /// slow plugin nor a broken one can stop someone launching their game.
    pub fn run_launch_checks(&self, instance_id: &str) -> LaunchCheckOutcome {
        let mut outcome = LaunchCheckOutcome::default();
        if !self.is_enabled() {
            return outcome;
        }
        let Ok(resolution) = self.inner.resolution.read() else {
            return outcome;
        };
        let runnable: Vec<_> = resolution
            .runnable()
            .map(|resolved| resolved.record.clone())
            .collect();
        drop(resolution);

        for record in runnable {
            if !record
                .granted
                .contains(agora_plugin_api::Capability::LaunchPrepare)
            {
                continue;
            }
            for check in &record.manifest.contributions.launch_checks {
                let plugin_id = record.id().clone();
                if let Err(error) = self.ensure_running(&plugin_id) {
                    outcome
                        .errors
                        .push(format!("{} could not start: {}", plugin_id, error.message));
                    continue;
                }
                let timeout = Duration::from_millis(check.timeout_ms.min(MAX_LAUNCH_CHECK_MS));
                let args = serde_json::json!({ "instanceId": instance_id });
                match self
                    .inner
                    .host
                    .invoke(&plugin_id, &check.export, args, timeout)
                {
                    Ok(value) => match serde_json::from_value::<DiagnosticReport>(value) {
                        Ok(report) if report.validate().is_ok() => {
                            outcome.results.push(LaunchCheckResult {
                                plugin_id: plugin_id.to_string(),
                                check_id: record.manifest.id.qualify(&check.id),
                                title: check.title.clone(),
                                report,
                                blocking: matches!(check.on_failure, LaunchCheckFailure::Prompt),
                            });
                        }
                        Ok(_) | Err(_) => outcome.errors.push(format!(
                            "`{}` returned something that is not a diagnostic report",
                            record.manifest.id.qualify(&check.id)
                        )),
                    },
                    Err(error) => outcome.errors.push(format!(
                        "`{}` did not complete: {}",
                        record.manifest.id.qualify(&check.id),
                        error.message
                    )),
                }
            }
        }
        outcome
    }

    fn ensure_running(&self, plugin_id: &PluginId) -> Result<(), PluginError> {
        if !self.is_enabled() {
            return Err(PluginError::new(
                PluginErrorCode::NotActivated,
                "Plugins are turned off",
            ));
        }
        if self.inner.host.is_active(plugin_id) {
            return Ok(());
        }
        // Lazy activation is what makes `onView:` and `onCommand:` cheap: the
        // runtime starts when something actually needs it.
        let runnable = self
            .inner
            .resolution
            .read()
            .ok()
            .and_then(|resolution| {
                resolution
                    .get(plugin_id)
                    .map(|resolved| resolved.status.is_runnable())
            })
            .unwrap_or(false);
        if !runnable {
            return Err(PluginError::new(
                PluginErrorCode::NotActivated,
                format!("`{plugin_id}` is not available"),
            ));
        }
        self.activate(plugin_id)
    }

    // -- repairs -----------------------------------------------------------

    /// Find pairs of proposals that want opposite things.
    ///
    /// Surfaced to the user rather than resolved automatically. Last-writer-
    /// wins would silently undo one plugin's advice with another's, and the
    /// user would have no way to know it happened.
    pub fn find_conflicts(proposals: &[(PluginId, RepairProposal)]) -> Vec<RepairConflict> {
        let mut conflicts = Vec::new();
        for (left_index, (left_plugin, left)) in proposals.iter().enumerate() {
            for (right_plugin, right) in proposals.iter().skip(left_index + 1) {
                for left_action in &left.actions {
                    for right_action in &right.actions {
                        if left_action.disagrees_with(right_action) {
                            conflicts.push(RepairConflict {
                                left_plugin: left_plugin.to_string(),
                                left_action: left_action.describe(),
                                right_plugin: right_plugin.to_string(),
                                right_action: right_action.describe(),
                                subject: left_action.conflict_key(),
                            });
                        }
                    }
                }
            }
        }
        conflicts
    }

    /// Apply a repair the user approved.
    ///
    /// Every action is re-validated against the world as it is *now*, not as
    /// it was when the plugin proposed it. A plugin that suggested disabling a
    /// mod the user has since removed gets told the plan is stale rather than
    /// having its action quietly skipped or, worse, applied to something else.
    pub fn apply_repair(
        &self,
        plugin_id: &PluginId,
        proposal: &RepairProposal,
    ) -> LauncherResult<RepairOutcome> {
        proposal.validate().map_err(|e| LauncherError::Generic {
            code: "ERR_PLUGIN_REPAIR_INVALID".into(),
            message: e.message,
        })?;

        // A proposal that disagrees with itself is a bug in the plugin, and
        // applying half of it would leave the user somewhere neither the
        // plugin nor they intended.
        for (index, action) in proposal.actions.iter().enumerate() {
            for other in proposal.actions.iter().skip(index + 1) {
                if action.disagrees_with(other) {
                    return Err(LauncherError::Generic {
                        code: "ERR_PLUGIN_REPAIR_CONFLICT".into(),
                        message: format!(
                            "this repair asks for two incompatible things: {} and {}",
                            action.describe(),
                            other.describe()
                        ),
                    });
                }
            }
        }

        let record = self
            .inner
            .record(plugin_id)
            .filter(|record| record.enabled)
            .ok_or_else(|| LauncherError::Generic {
                code: "ERR_PLUGIN_DISABLED".into(),
                message: "Plugin is unavailable or disabled".into(),
            })?;
        if !self.is_enabled() {
            return Err(LauncherError::Generic {
                code: "ERR_PLUGIN_DISABLED".into(),
                message: "Plugins are turned off".into(),
            });
        }
        for action in &proposal.actions {
            let required = match action {
                RepairAction::DisableContent { .. }
                | RepairAction::EnableContent { .. }
                | RepairAction::PinContentUpdate { .. }
                | RepairAction::UnpinContentUpdate { .. } => {
                    agora_plugin_api::Capability::ContentWrite
                }
                _ => agora_plugin_api::Capability::InstanceWrite,
            };
            if !record.granted.contains(required) {
                return Err(LauncherError::Generic {
                    code: "ERR_PLUGIN_CAPABILITY_DENIED".into(),
                    message: format!("Repair requires {}", required.as_str()),
                });
            }
        }
        let mut outcome = RepairOutcome::default();
        for action in &proposal.actions {
            if let Err(reason) = self.revalidate(action) {
                outcome
                    .stale
                    .push(format!("{}: {reason}", action.describe()));
                continue;
            }
            // Attributed to the plugin so the resulting events do not come
            // straight back to it.
            let result = events::with_origin(plugin_id, || self.execute(action));
            match result {
                Ok(()) => outcome.applied.push(action.describe()),
                Err(error) => outcome.failed.push(RepairFailure {
                    action: action.describe(),
                    message: error.to_string(),
                    retryable: true,
                }),
            }
        }
        Ok(outcome)
    }

    /// Check that an action still makes sense before performing it.
    fn revalidate(&self, action: &RepairAction) -> Result<(), String> {
        let instance_id = action.instance_id();
        let service = crate::instance_service::InstanceService::new(self.inner.ctx.clone());
        let detail = match service.get(instance_id) {
            Ok(Some(detail)) => detail,
            Ok(None) => return Err(format!("instance `{instance_id}` no longer exists")),
            Err(error) => return Err(error.to_string()),
        };

        match action {
            RepairAction::DisableContent { key, .. }
            | RepairAction::EnableContent { key, .. }
            | RepairAction::PinContentUpdate { key, .. }
            | RepairAction::UnpinContentUpdate { key, .. } => {
                if detail.row.is_locked {
                    return Err("the instance is locked".to_string());
                }
                let Some(manifest) = detail.manifest else {
                    return Err("the instance has no manifest to read".to_string());
                };
                let instance_dir = self
                    .inner
                    .ctx
                    .paths
                    .instance_dir(instance_id)
                    .map_err(|e| e.to_string())?;
                let present = crate::installed_content::list_installed_content(
                    &instance_dir,
                    &manifest,
                    None,
                    None,
                )
                .into_iter()
                .any(|row| &row.key == key);
                if !present {
                    return Err(format!("`{key}` is no longer installed"));
                }
                Ok(())
            }
            RepairAction::SetJvmMemory { .. } | RepairAction::ResetJvmArgs { .. } => Ok(()),
            RepairAction::CreateSnapshot { .. } => Ok(()),
        }
    }

    fn execute(&self, action: &RepairAction) -> LauncherResult<()> {
        let ctx = self.inner.ctx.clone();
        match action {
            RepairAction::DisableContent { instance_id, key }
            | RepairAction::EnableContent { instance_id, key } => {
                let filename = self.filename_for(instance_id, key)?;
                let service = crate::crash_service::CrashService::new(ctx);
                if matches!(action, RepairAction::EnableContent { .. }) {
                    service.enable_mod(instance_id, &filename)
                } else {
                    service.disable_mod(instance_id, &filename)
                }
            }
            RepairAction::PinContentUpdate { instance_id, key }
            | RepairAction::UnpinContentUpdate { instance_id, key } => {
                let filename = self.filename_for(instance_id, key)?;
                let pinned = matches!(action, RepairAction::PinContentUpdate { .. });
                crate::install_service::InstallService::new(ctx)
                    .set_update_pinned(instance_id, &filename, pinned)
                    .map(|_| ())
            }
            RepairAction::SetJvmMemory {
                instance_id,
                memory_mb,
            } => {
                let service = crate::instance_service::InstanceService::new(ctx);
                let Some(detail) = service.get(instance_id)? else {
                    return Err(LauncherError::Generic {
                        code: "ERR_INSTANCE_NOT_FOUND".into(),
                        message: format!("no instance `{instance_id}`"),
                    });
                };
                service.update_jvm(
                    instance_id,
                    *memory_mb,
                    &detail.row.jvm_gc,
                    detail.row.jvm_always_pre_touch,
                    &detail.row.jvm_custom_args,
                    "manual",
                )
            }
            RepairAction::ResetJvmArgs { instance_id } => {
                let service = crate::instance_service::InstanceService::new(ctx);
                let Some(detail) = service.get(instance_id)? else {
                    return Err(LauncherError::Generic {
                        code: "ERR_INSTANCE_NOT_FOUND".into(),
                        message: format!("no instance `{instance_id}`"),
                    });
                };
                service.update_jvm(
                    instance_id,
                    detail.row.jvm_memory_mb,
                    &detail.row.jvm_gc,
                    detail.row.jvm_always_pre_touch,
                    "",
                    &detail.row.jvm_memory_mode,
                )
            }
            RepairAction::CreateSnapshot { instance_id, label } => {
                crate::snapshot_service::SnapshotService::new(ctx)
                    .create(instance_id, label.as_deref())
                    .map(|_| ())
            }
        }
    }

    fn filename_for(&self, instance_id: &str, key: &str) -> LauncherResult<String> {
        let service = crate::instance_service::InstanceService::new(self.inner.ctx.clone());
        let Some(detail) = service.get(instance_id)? else {
            return Err(LauncherError::Generic {
                code: "ERR_INSTANCE_NOT_FOUND".into(),
                message: format!("no instance `{instance_id}`"),
            });
        };
        let Some(manifest) = detail.manifest else {
            return Err(LauncherError::Generic {
                code: "ERR_INSTANCE_MANIFEST_MISSING".into(),
                message: format!("`{instance_id}` has no manifest"),
            });
        };
        let instance_dir = self.inner.ctx.paths.instance_dir(instance_id)?;
        crate::installed_content::list_installed_content(&instance_dir, &manifest, None, None)
            .into_iter()
            .find(|row| row.key == key)
            .map(|row| row.filename)
            .ok_or_else(|| LauncherError::Generic {
                code: "ERR_PLUGIN_REPAIR_STALE".into(),
                message: format!("`{key}` is no longer installed in `{instance_id}`"),
            })
    }

    fn summary(&self, plugin_id: &PluginId) -> LauncherResult<PluginSummary> {
        self.list()
            .into_iter()
            .find(|summary| summary.id == plugin_id.as_str())
            .ok_or_else(|| LauncherError::Generic {
                code: "ERR_PLUGIN_NOT_FOUND".into(),
                message: format!("`{plugin_id}` is not installed"),
            })
    }
}

/// Whether a plugin should be started at launch rather than on demand.
///
/// Two cases qualify. `onStartup` is the explicit request. An **empty**
/// activation list also qualifies, because a plugin that declares no trigger
/// has no lazy path that could ever start it — treating that as "never run"
/// would silently break the simplest possible plugin.
fn wants_startup(manifest: &PluginManifest) -> bool {
    manifest.activation.is_empty()
        || manifest
            .activation
            .iter()
            .any(|activation| matches!(activation, ActivationEvent::Startup))
}

fn now(ctx: &Ctx) -> String {
    chrono::DateTime::<chrono::Utc>::from(ctx.clock.now()).to_rfc3339()
}

/// The plugin id an archive claims, when it can be read cheaply.
fn archive_id(archive: &Path) -> Option<PluginId> {
    install::read_package_manifest(archive)
        .ok()
        .map(|contents| contents.manifest.id)
}

/// Capabilities a stored grant covers, for display.
/// An install read out of the database, owned so the connection can be closed.
struct ExistingRecord {
    version: semver::Version,
    data_version: u32,
    hosts: Vec<String>,
    granted: CapabilitySet,
}

impl ExistingRecord {
    fn as_ref(&self) -> install::ExistingInstall<'_> {
        install::ExistingInstall {
            version: &self.version,
            data_version: self.data_version,
            granted: &self.granted,
            hosts: &self.hosts,
        }
    }
}

/// One line describing a verdict, for the trust record and for an error.
fn verdict_summary(verdict: &UpdateVerdict) -> String {
    match verdict {
        UpdateVerdict::UpToDate => "up to date".into(),
        UpdateVerdict::Available { from, to, .. } => format!("{from} -> {to} available"),
        UpdateVerdict::NeedsNewerHost { latest, requires } => {
            format!("{latest} needs a host matching {requires}")
        }
        UpdateVerdict::InstalledIsNewer { installed, latest } => {
            format!("installed {installed} is newer than the published {latest}")
        }
        UpdateVerdict::NoReleases => "the publisher lists no releases".into(),
    }
}

/// Host part of an https URL, for the network policy.
///
/// String work rather than a URL parser because the value has already been
/// validated as starting with `https://`, and because the authoritative host
/// check happens inside the network layer, which parses it properly.
fn host_of(url: &str) -> String {
    url.strip_prefix("https://")
        .and_then(|rest| rest.split('/').next())
        .map(|host| host.split('@').next_back().unwrap_or(host).to_string())
        .unwrap_or_default()
}

pub fn granted_names(granted: &CapabilitySet) -> Vec<String> {
    granted.iter().map(|cap| cap.as_str().to_string()).collect()
}

/// Group contributions by the surface they belong to, for the UI.
pub fn by_kind(
    contributions: &[NamespacedContribution],
) -> BTreeMap<String, Vec<NamespacedContribution>> {
    let mut grouped: BTreeMap<String, Vec<NamespacedContribution>> = BTreeMap::new();
    for contribution in contributions {
        grouped
            .entry(contribution.kind.as_str().to_string())
            .or_default()
            .push(contribution.clone());
    }
    grouped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(raw: &str) -> PluginId {
        PluginId::parse(raw).unwrap()
    }

    fn disable(key: &str) -> RepairAction {
        RepairAction::DisableContent {
            instance_id: "i1".into(),
            key: key.into(),
        }
    }

    fn enable(key: &str) -> RepairAction {
        RepairAction::EnableContent {
            instance_id: "i1".into(),
            key: key.into(),
        }
    }

    fn proposal(actions: Vec<RepairAction>) -> RepairProposal {
        RepairProposal {
            id: "fix".into(),
            title: "Fix it".into(),
            description: None,
            actions,
        }
    }

    #[test]
    fn two_plugins_wanting_opposite_things_are_reported_as_a_conflict() {
        let conflicts = PluginService::find_conflicts(&[
            (id("a.one"), proposal(vec![disable("sodium")])),
            (id("b.two"), proposal(vec![enable("sodium")])),
        ]);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].left_plugin, "a.one");
        assert_eq!(conflicts[0].right_plugin, "b.two");
    }

    #[test]
    fn two_plugins_proposing_the_same_thing_do_not_conflict() {
        let conflicts = PluginService::find_conflicts(&[
            (id("a.one"), proposal(vec![disable("sodium")])),
            (id("b.two"), proposal(vec![disable("sodium")])),
        ]);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn proposals_about_different_items_do_not_conflict() {
        let conflicts = PluginService::find_conflicts(&[
            (id("a.one"), proposal(vec![disable("sodium")])),
            (id("b.two"), proposal(vec![disable("iris")])),
        ]);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn a_partial_repair_is_not_reported_as_complete() {
        let outcome = RepairOutcome {
            applied: vec!["Disable `a`".into()],
            failed: vec![RepairFailure {
                action: "Disable `b`".into(),
                message: "locked".into(),
                retryable: true,
            }],
            stale: vec![],
        };
        assert!(!outcome.is_complete());
    }

    #[test]
    fn a_repair_with_nothing_left_to_do_is_still_complete() {
        assert!(RepairOutcome::default().is_complete());
    }

    #[test]
    fn only_a_blocking_check_with_a_real_finding_prompts() {
        let warning = DiagnosticReport {
            findings: vec![agora_plugin_api::diagnostics::Finding {
                id: "f".into(),
                title: "Something".into(),
                severity: Severity::Warning,
                summary: None,
                evidence: vec![],
                repairs: vec![],
            }],
            incomplete_reason: None,
        };

        let non_blocking = LaunchCheckOutcome {
            results: vec![LaunchCheckResult {
                plugin_id: "a.one".into(),
                check_id: "a.one/check".into(),
                title: "Check".into(),
                report: warning.clone(),
                blocking: false,
            }],
            errors: vec![],
        };
        assert!(!non_blocking.should_prompt());

        let blocking = LaunchCheckOutcome {
            results: vec![LaunchCheckResult {
                plugin_id: "a.one".into(),
                check_id: "a.one/check".into(),
                title: "Check".into(),
                report: warning,
                blocking: true,
            }],
            errors: vec![],
        };
        assert!(blocking.should_prompt());
    }

    #[test]
    fn a_clean_blocking_check_does_not_prompt() {
        let outcome = LaunchCheckOutcome {
            results: vec![LaunchCheckResult {
                plugin_id: "a.one".into(),
                check_id: "a.one/check".into(),
                title: "Check".into(),
                report: DiagnosticReport::default(),
                blocking: true,
            }],
            errors: vec![],
        };
        assert!(!outcome.should_prompt());
    }

    #[test]
    fn a_check_that_failed_to_run_is_recorded_rather_than_treated_as_a_pass() {
        let outcome = LaunchCheckOutcome {
            results: vec![],
            errors: vec!["a.one/check did not complete: timed out".into()],
        };
        assert!(!outcome.should_prompt());
        assert_eq!(outcome.errors.len(), 1);
    }
}
