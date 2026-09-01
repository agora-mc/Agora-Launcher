use crate::ai_assistant::{self, ChatMessage, ChatResponse};
use crate::auth::{DeviceFlowResponse, GithubProfile};
use crate::crash_diagnostics::{self, CrashReportInfo, CrashTriageResult};
use crate::crash_investigator;
use crate::dependency_ops;
use crate::error::{LauncherError, LauncherResult};
use crate::governance::{
    DiagnosticCheck, GovernanceConfig, GovernanceEvent, GovernanceSummary, ItemVote, ItemVoteState,
};
use crate::instances::{self, CreateInstanceRequest, InstanceDetail, LoaderVersionSummary};
use crate::loader_manifests;
use crate::mcp;
use crate::mod_install::{self, check_not_locked};
use crate::models::{InstanceManifest, InstanceRow, ModVersionCandidate};
use crate::modrinth_raw;
use crate::mojang;
use crate::paths;
use crate::registry::{
    self, AuditLogEntry, CategoryInfo, CuratedAnnotation, ModReview, PackModRow, RegistryItem,
    SortOption, UnderReviewItem,
};
use crate::state::LauncherState;
use crate::version_cache::{self, ModVersionPage, SharedVersionCache};
use agora_core::browse_cache::{self, BrowseFilters, BrowsePage};
use agora_core::installed_content::{InstalledContentMetadata, InstalledContentRow};
use agora_core::modrinth::{ModrinthSearchParams, ModrinthSort};
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};
use tauri::Manager;

const MSA_AUTH_REPLY_HOST: &str = "login.live.com";
const MSA_AUTH_REPLY_PATH: &str = "/oauth20_desktop.srf";

/// Current status of the MCP server.
#[derive(Debug, Clone, serde::Serialize)]
pub struct McpStatus {
    pub running: bool,
    pub url: String,
}

/// Safe account metadata that may cross the Tauri command boundary. OAuth and
/// Minecraft bearer tokens remain backend-only in `MsaCredentials`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MsaAccountStatus {
    pub username: String,
    pub uuid: String,
    pub expires: String,
}

impl From<&agora_core::msa::MsaCredentials> for MsaAccountStatus {
    fn from(credentials: &agora_core::msa::MsaCredentials) -> Self {
        Self {
            username: credentials.username.clone(),
            uuid: credentials.uuid.clone(),
            expires: credentials.expires.to_rfc3339(),
        }
    }
}

/// Global version list cache for paginated mod version resolution.
static VERSION_CACHE: LazyLock<SharedVersionCache> = LazyLock::new(version_cache::new_cache);

/// How many Technic packs to pull per browse query.
///
/// Kept small on purpose: Technic's search returns only names, so each hit costs
/// an extra detail round-trip to get its install/rating counts and tier.
const TECHNIC_BROWSE_LIMIT: u32 = 30;

/// Curated download strategies the user has enabled (Axis A: curated content,
/// opt-out per source). A missing setting defaults to ON so curated content
/// never silently vanishes. Separate from live third-party browsing settings.
fn curated_strategies_from_settings(app: &tauri::AppHandle) -> Vec<String> {
    agora_core::registry::CURATED_DOWNLOAD_STRATEGIES
        .iter()
        .map(|strategy| strategy.to_string())
        .filter(|strategy| {
            crate::core_context(app)
                .map(|ctx| {
                    agora_core::settings::SettingsService::new(ctx)
                        .get_bool_or(&format!("curated_source_{strategy}_enabled"), true)
                        .unwrap_or(true)
                })
                .unwrap_or(true)
        })
        .collect()
}

/// Browse registry items with typed filters (replaces raw-SQL queryRegistry).
///
/// Curated-source visibility is driven by the per-source settings
/// (`curated_source_<strategy>_enabled`, default on); live third-party
/// browsing is gated separately.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn browse_items(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    content_type: Option<String>,
    category: Option<String>,
    sort: Option<SortOption>,
    mc_version: Option<String>,
    loader: Option<String>,
    limit: Option<i64>,
) -> LauncherResult<Vec<RegistryItem>> {
    let curated_strategies = curated_strategies_from_settings(&app);
    tokio::task::spawn_blocking(move || {
        let ctx = crate::core_context(&app)?;
        let svc = agora_core::registry::RegistryService::new(ctx);
        svc.browse_items(
            content_type.as_deref(),
            category.as_deref(),
            &sort.unwrap_or_default(),
            &curated_strategies,
            mc_version.as_deref(),
            loader.as_deref(),
            None,
            limit.unwrap_or(100),
        )
    })
    .await
    .map_err(|_| LauncherError::Generic {
        code: "ERR_REGISTRY_QUERY".to_string(),
        message: "Registry query task failed.".to_string(),
    })?
}

/// "For You" recommendations: boost uninstalled mods whose categories overlap
/// with the user's installed mods (§6.2). Delegates to core
/// [`RegistryService::for_you_items`] for all business logic.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn for_you_items(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    mc_version: Option<String>,
    loader: Option<String>,
    limit: Option<i64>,
    modrinth_categories: Option<Vec<String>>,
    query: Option<String>,
) -> LauncherResult<Vec<RegistryItem>> {
    let limit = limit.unwrap_or(50).clamp(1, 500);
    let curated_strategies = curated_strategies_from_settings(&app);
    tokio::task::spawn_blocking(move || {
        let ctx = crate::core_context(&app)?;
        let svc = agora_core::registry::RegistryService::new(ctx);
        svc.for_you_items(
            &curated_strategies,
            mc_version.as_deref(),
            loader.as_deref(),
            limit,
            modrinth_categories.as_deref(),
            query.as_deref(),
        )
    })
    .await
    .map_err(|_| LauncherError::Generic {
        code: "ERR_REGISTRY_QUERY".to_string(),
        message: "Registry query task failed.".to_string(),
    })?
}

/// Look up a curated annotation for a registry item by its registry id.
#[tauri::command]
pub async fn get_curated_annotation(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    item_id: String,
) -> LauncherResult<Option<CuratedAnnotation>> {
    tokio::task::spawn_blocking(move || {
        let ctx = crate::core_context(&app)?;
        let svc = agora_core::registry::RegistryService::new(ctx);
        svc.get_curated_annotation(&item_id)
    })
    .await
    .map_err(|_| LauncherError::Generic {
        code: "ERR_REGISTRY_QUERY".to_string(),
        message: "Registry query task failed.".to_string(),
    })?
}

/// Fetch a single registry item by ID.
#[tauri::command]
pub async fn get_registry_item(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    item_id: String,
) -> LauncherResult<Option<RegistryItem>> {
    tokio::task::spawn_blocking(move || {
        let ctx = crate::core_context(&app)?;
        let svc = agora_core::registry::RegistryService::new(ctx);
        svc.get_item_by_id(&item_id)
    })
    .await
    .map_err(|_| LauncherError::Generic {
        code: "ERR_REGISTRY_QUERY".to_string(),
        message: "Registry query task failed.".to_string(),
    })?
}

/// List all categories from the registry.
#[tauri::command]
pub async fn list_categories(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
) -> LauncherResult<Vec<CategoryInfo>> {
    tokio::task::spawn_blocking(move || {
        let ctx = crate::core_context(&app)?;
        let svc = agora_core::registry::RegistryService::new(ctx);
        svc.list_categories()
    })
    .await
    .map_err(|_| LauncherError::Generic {
        code: "ERR_REGISTRY_QUERY".to_string(),
        message: "Registry query task failed.".to_string(),
    })?
}

/// List all mods in a pack.
#[tauri::command]
pub async fn list_pack_mods(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    pack_id: String,
) -> LauncherResult<Vec<PackModRow>> {
    tokio::task::spawn_blocking(move || {
        let ctx = crate::core_context(&app)?;
        let svc = agora_core::registry::RegistryService::new(ctx);
        svc.pack_mods_for_pack(&pack_id)
    })
    .await
    .map_err(|_| LauncherError::Generic {
        code: "ERR_REGISTRY_QUERY".to_string(),
        message: "Pack mods query task failed.".to_string(),
    })?
}

/// List audit log entries from the registry DB (§4.6).
#[tauri::command]
pub async fn list_audit_log(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    limit: Option<i64>,
) -> LauncherResult<Vec<AuditLogEntry>> {
    let limit = limit.unwrap_or(200).clamp(1, 1000);
    tokio::task::spawn_blocking(move || {
        let ctx = crate::core_context(&app)?;
        let svc = agora_core::registry::RegistryService::new(ctx);
        svc.list_audit_log(limit)
    })
    .await
    .map_err(|_| LauncherError::Generic {
        code: "ERR_REGISTRY_QUERY".to_string(),
        message: "Audit log query task failed.".to_string(),
    })?
}

/// List all user instances from `local_state.db`.
#[tauri::command]
pub async fn list_instances(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
) -> LauncherResult<Vec<InstanceRow>> {
    tokio::task::spawn_blocking(move || instances::list_instances(&app))
        .await
        .map_err(|_| LauncherError::LocalStateFailed)?
}

/// Fetch a single instance plus its on-disk manifest.
#[tauri::command]
pub async fn get_instance_detail(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
) -> LauncherResult<Option<InstanceDetail>> {
    tokio::task::spawn_blocking(move || instances::get_instance_detail(&app, &instance_id))
        .await
        .map_err(|_| LauncherError::LocalStateFailed)?
}

/// Create a custom instance and inject its modloader.
#[tauri::command]
pub async fn create_instance(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    request: CreateInstanceRequest,
) -> LauncherResult<InstanceRow> {
    instances::create_instance(app, request).await
}

/// Delete an instance, moving its directory to the OS trash.
#[tauri::command]
pub async fn delete_instance(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
) -> LauncherResult<()> {
    tokio::task::spawn_blocking(move || instances::delete_instance(&app, &instance_id))
        .await
        .map_err(|_| LauncherError::LocalStateFailed)?
}

/// Unlock a locked pack instance for manual mod management (Â§6.5).
#[tauri::command]
pub async fn unlock_instance(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
) -> LauncherResult<()> {
    instances::unlock_instance(&app, &instance_id).await
}

/// Lock an unlocked pack instance, discarding the lock snapshot.
#[tauri::command]
pub async fn lock_instance(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
) -> LauncherResult<()> {
    instances::lock_instance(&app, &instance_id).await
}

/// Rename an instance.
#[tauri::command]
pub async fn rename_instance(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
    new_name: String,
) -> LauncherResult<()> {
    instances::rename_instance(&app, &instance_id, &new_name).await
}

/// Revert an unlocked instance to its lock snapshot.
#[tauri::command]
pub async fn revert_instance(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
) -> LauncherResult<()> {
    instances::revert_instance(&app, &instance_id).await
}

/// Launch an instance via the official Mojang launcher delegation. Delegates
/// orchestration to core [`LaunchService`] and runs the outcome monitoring
/// in a background task.  LKG outcome recording and retention happen in core;
/// the desktop adapter only emits Tauri UI events.
fn health_policy_for_approval(
    allow_health_blockers: Option<bool>,
) -> agora_core::launch_service::HealthPolicy {
    if allow_health_blockers.unwrap_or(false) {
        agora_core::launch_service::HealthPolicy::WarnOnly
    } else {
        agora_core::launch_service::HealthPolicy::BlockOnRed
    }
}

#[cfg(test)]
mod health_policy_tests {
    use super::health_policy_for_approval;
    use agora_core::launch_service::HealthPolicy;

    #[test]
    fn only_explicit_approval_downgrades_red_health_to_warning() {
        assert_eq!(health_policy_for_approval(None), HealthPolicy::BlockOnRed);
        assert_eq!(
            health_policy_for_approval(Some(false)),
            HealthPolicy::BlockOnRed
        );
        assert_eq!(
            health_policy_for_approval(Some(true)),
            HealthPolicy::WarnOnly
        );
    }
}

#[tauri::command]
pub async fn launch_instance(
    app: tauri::AppHandle,
    state: tauri::State<'_, LauncherState>,
    instance_id: String,
    allow_health_blockers: Option<bool>,
    health_scan_token: Option<String>,
) -> LauncherResult<()> {
    let sanitized = paths::sanitize_id(&instance_id);
    if sanitized.is_empty() {
        return Err(LauncherError::Generic {
            code: "ERR_INVALID_INSTANCE".into(),
            message: "Instance ID is empty or invalid.".into(),
        });
    }
    {
        let mut shared = state.lock().await;
        // SOL-2 §19.3: a target must never start a launch while its canonical
        // install transaction is applying. Delegated launches have no
        // launch_reservation, so an equivalent atomic start marker is
        // registered here and cleared when the delegated session ends.
        ensure_launch_admitted(&shared, &sanitized)?;
        shared.active_launches.insert(sanitized.clone());
    }

    let ctx = crate::core_context(&app)?;
    let instance_dir =
        paths::instance_dir(&app, &sanitized).map_err(|e| LauncherError::Generic {
            code: "ERR_INSTANCE_PATH".into(),
            message: e.to_string(),
        })?;

    let progress = DelegatedLaunchProgress {
        app: app.clone(),
        instance_id: sanitized.clone(),
    };
    let request = agora_core::launch_service::LaunchRequest {
        instance_id: sanitized.clone(),
        mode: agora_core::launch_service::LaunchMode::Delegated,
        health_policy: health_policy_for_approval(allow_health_blockers),
        health_scan_token,
    };
    let state_for_monitor = state.inner().clone();
    let launch_result = agora_core::launch_service::LaunchService::new(ctx.clone())
        .launch(request, &progress)
        .await;
    if launch_result.is_err() {
        // Normal failure cleanup: release the delegated start marker.
        let mut shared = state.lock().await;
        shared.active_launches.remove(&sanitized);
    }
    let result = launch_result?;

    let launched_at = std::time::SystemTime::now();
    let app_for_monitor = app.clone();
    let id_for_monitor = sanitized.clone();
    let dir_for_monitor = instance_dir;
    let snap_id = result.snapshot_id.clone();
    let session_id = result.session_id;
    // Early release: we cannot reliably know when the Mojang-launched game exits,
    // so free the install/launch block and return the UI to normal shortly after
    // handoff. The official launcher owns duplicate-launch prevention from here.
    let state_for_early = state.inner().clone();
    let app_for_early = app.clone();
    let id_for_early = sanitized.clone();
    let snap_for_early = result.snapshot_id.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        state_for_early
            .lock()
            .await
            .active_launches
            .remove(&id_for_early);
        use tauri::Emitter;
        let _ = app_for_early.emit(
            "game-exited",
            serde_json::json!({
                "instance_id": id_for_early,
                "outcome": "unknown",
                "snapshot_id": snap_for_early,
                "delegated": true,
            }),
        );
    });
    tokio::spawn(async move {
        // Core-owned wait_delegated handles monitoring, LKG recording,
        // and retention. Desktop only emits the Tauri event.
        let outcome = agora_core::launch_service::LaunchService::wait_delegated(
            &ctx,
            &id_for_monitor,
            &dir_for_monitor,
            &snap_id,
            session_id,
            launched_at,
        )
        .await;
        // The delegated session ended: release the launch marker so an install
        // for this instance is permitted again (SOL-2 §19.3).
        state_for_monitor
            .lock()
            .await
            .active_launches
            .remove(&id_for_monitor);

        use tauri::Emitter;
        let _ = app_for_monitor.emit(
            "game-exited",
            serde_json::json!({
                "instance_id": id_for_monitor,
                "outcome": outcome,
                "snapshot_id": snap_id,
                "delegated": true,
            }),
        );
    });

    Ok(())
}

struct DelegatedLaunchProgress {
    app: tauri::AppHandle,
    instance_id: String,
}

impl agora_core::launch_service::LaunchProgress for DelegatedLaunchProgress {
    fn phase(&self, phase: &str, message: &str) {
        use tauri::Emitter;
        let _ = self.app.emit(
            "launch-progress",
            serde_json::json!({
                "instance_id": self.instance_id,
                "phase": phase,
                "message": message,
            }),
        );
    }

    fn phase_completed(&self, phase: &str, duration_ms: u128) {
        use tauri::Emitter;
        let _ = self.app.emit(
            "launch-progress",
            serde_json::json!({
                "instance_id": self.instance_id,
                "phase": format!("{phase}-complete"),
                "message": format!("Completed in {duration_ms} ms"),
                "duration_ms": duration_ms,
            }),
        );
    }

    fn started(&self, started: &agora_core::launch_service::LaunchStarted) {
        use tauri::Emitter;
        let app = self.app.clone();
        let instance_id = self.instance_id.clone();
        let started = started.clone();
        tokio::spawn(async move {
            let _ = app.emit(
                "game-started",
                serde_json::json!({
                    "instance_id": instance_id,
                    "session_id": started.session_id,
                    "delegated": true,
                }),
            );
        });
    }

    fn finished(&self, _result: &agora_core::launch_service::LaunchResult) {
        // Empty for delegated — the background monitoring task emits
        // `game-exited` when the monitored log shows game exit.
    }

    fn handoff(
        &self,
        _identity: &agora_core::launch_planner::LaunchIdentity,
    ) -> LauncherResult<()> {
        instances::launch_instance(&self.app, &self.instance_id)
    }
}

/// Launch an instance with an optional recovery action performed before the
/// launch. The recovery action (provision Java or repair loader) runs in the
/// same backend operation; if it fails the launch is aborted. Uses the same
/// process/session machinery as [`launch_instance_direct`].
///
/// - `action.None` → plain direct launch (same as `launch_instance_direct`)
/// - `action.ProvisionJava { major }` → provision runtime then retry
/// - `action.RepairLoader` → force-reinstall loader then retry
///
/// Always uses Direct mode (recovery actions only make sense for direct
/// launches). Returns the PID on success.
#[tauri::command]
pub async fn launch_instance_with_recovery(
    app: tauri::AppHandle,
    state: tauri::State<'_, LauncherState>,
    instance_id: String,
    action: agora_core::launch_service::LaunchRecoveryAction,
    allow_health_blockers: Option<bool>,
    health_scan_token: Option<String>,
) -> LauncherResult<u32> {
    use tokio::sync::oneshot;

    let sanitized = paths::sanitize_id(&instance_id);
    if sanitized.is_empty() {
        return Err(LauncherError::Generic {
            code: "ERR_INVALID_INSTANCE".into(),
            message: "Instance ID is empty or invalid.".into(),
        });
    }
    {
        let shared = state.lock().await;
        // Multiple instances may run at once, and the same instance may be
        // launched more than once. What must not overlap is two launches of the
        // *same* instance while its directory is still being materialized.
        if shared.launch_reservations.contains(&sanitized) {
            return Err(LauncherError::Generic {
                code: "ERR_ALREADY_RUNNING".into(),
                message: "This instance is already starting.".into(),
            });
        }
    }

    let ctx = crate::core_context(&app)?;
    if !agora_core::instance_service::InstanceService::new(ctx.clone())
        .resolve_direct_launch(&sanitized, true)?
    {
        return Err(LauncherError::Generic {
            code: "ERR_INSTANCE_DELEGATED_ONLY".into(),
            message: "This imported instance requires delegated launch through the official Minecraft Launcher.".into(),
        });
    }
    let (started_tx, started_rx) = oneshot::channel::<LauncherResult<u32>>();
    {
        let mut shared = state.lock().await;
        // SOL-2 §19.3: the reservation is the atomic final transition. Re-check
        // running/reservation AND reject an active install under the same lock
        // (closes the preflight -> reservation race).
        // Multiple instances may run at once, and the same instance may be
        // launched more than once. What must not overlap is two launches of the
        // *same* instance while its directory is still being materialized.
        if shared.launch_reservations.contains(&sanitized) {
            return Err(LauncherError::Generic {
                code: "ERR_ALREADY_RUNNING".into(),
                message: "This instance is already starting.".into(),
            });
        }
        ensure_launch_admitted(&shared, &sanitized)?;
        shared.launch_reservations.insert(sanitized.clone());
    }
    let progress = TauriLaunchProgress::new(
        app.clone(),
        state.inner().clone(),
        sanitized.clone(),
        started_tx,
    );
    let reservation_id = sanitized.clone();
    let request = agora_core::launch_service::LaunchRequest {
        instance_id: sanitized,
        mode: agora_core::launch_service::LaunchMode::Direct,
        health_policy: health_policy_for_approval(allow_health_blockers),
        health_scan_token,
    };
    let task = tokio::spawn(async move {
        agora_core::launch_service::LaunchService::new(ctx)
            .launch_with_recovery(request, action, &progress)
            .await
    });

    let result = match started_rx.await {
        Ok(Ok(pid)) => Ok(pid),
        Ok(Err(error)) => Err(error),
        Err(_) => match task.await {
            Ok(Ok(_)) => Err(LauncherError::Generic {
                code: "ERR_LAUNCH_START_SIGNAL".into(),
                message: "Launch completed without a start signal.".into(),
            }),
            Ok(Err(error)) => Err(error),
            Err(error) => Err(LauncherError::Generic {
                code: "ERR_LAUNCH_TASK".into(),
                message: error.to_string(),
            }),
        },
    };
    if result.is_err() {
        let mut shared = state.lock().await;
        shared.launch_reservations.remove(&reservation_id);
    }
    result
}

/// Direct Java spawn — Agora owns the launch process instead of delegating to Mojang launcher.
/// Core assigns the session ID; desktop mirrors it for presentation only.
#[tauri::command]
pub async fn launch_instance_direct(
    app: tauri::AppHandle,
    state: tauri::State<'_, LauncherState>,
    instance_id: String,
    allow_health_blockers: Option<bool>,
    health_scan_token: Option<String>,
) -> LauncherResult<u32> {
    use tokio::sync::oneshot;

    let sanitized = paths::sanitize_id(&instance_id);
    if sanitized.is_empty() {
        return Err(LauncherError::Generic {
            code: "ERR_INVALID_INSTANCE".into(),
            message: "Instance ID is empty or invalid.".into(),
        });
    }
    {
        let shared = state.lock().await;
        // Multiple instances may run at once, and the same instance may be
        // launched more than once. What must not overlap is two launches of the
        // *same* instance while its directory is still being materialized.
        if shared.launch_reservations.contains(&sanitized) {
            return Err(LauncherError::Generic {
                code: "ERR_ALREADY_RUNNING".into(),
                message: "This instance is already starting.".into(),
            });
        }
    }

    let ctx = crate::core_context(&app)?;
    if !agora_core::instance_service::InstanceService::new(ctx.clone())
        .resolve_direct_launch(&sanitized, true)?
    {
        return Err(LauncherError::Generic {
            code: "ERR_INSTANCE_DELEGATED_ONLY".into(),
            message: "This imported instance requires delegated launch through the official Minecraft Launcher.".into(),
        });
    }
    let (started_tx, started_rx) = oneshot::channel::<LauncherResult<u32>>();
    {
        let mut shared = state.lock().await;
        // SOL-2 §19.3: the reservation is the atomic final transition. Re-check
        // running/reservation AND reject an active install under the same lock
        // (closes the preflight -> reservation race).
        // Multiple instances may run at once, and the same instance may be
        // launched more than once. What must not overlap is two launches of the
        // *same* instance while its directory is still being materialized.
        if shared.launch_reservations.contains(&sanitized) {
            return Err(LauncherError::Generic {
                code: "ERR_ALREADY_RUNNING".into(),
                message: "This instance is already starting.".into(),
            });
        }
        ensure_launch_admitted(&shared, &sanitized)?;
        shared.launch_reservations.insert(sanitized.clone());
    }
    // `sanitized` is moved into the progress reporter; keep a copy so the
    // failure path can release this instance's reservation.
    let reservation_id = sanitized.clone();
    let progress =
        TauriLaunchProgress::new(app.clone(), state.inner().clone(), sanitized, started_tx);
    let request = agora_core::launch_service::LaunchRequest {
        instance_id: progress.instance_id.clone(),
        mode: agora_core::launch_service::LaunchMode::Direct,
        health_policy: health_policy_for_approval(allow_health_blockers),
        health_scan_token,
    };
    let task = tokio::spawn(async move {
        agora_core::launch_service::LaunchService::new(ctx)
            .launch(request, &progress)
            .await
    });

    let result = match started_rx.await {
        Ok(Ok(pid)) => Ok(pid),
        Ok(Err(error)) => Err(error),
        Err(_) => match task.await {
            Ok(Ok(_)) => Err(LauncherError::Generic {
                code: "ERR_LAUNCH_START_SIGNAL".into(),
                message: "Launch completed without a start signal.".into(),
            }),
            Ok(Err(error)) => Err(error),
            Err(error) => Err(LauncherError::Generic {
                code: "ERR_LAUNCH_TASK".into(),
                message: error.to_string(),
            }),
        },
    };
    if result.is_err() {
        let mut shared = state.lock().await;
        shared.launch_reservations.remove(&reservation_id);
    }
    result
}

struct TauriLaunchProgress {
    app: tauri::AppHandle,
    state: LauncherState,
    instance_id: String,
    started: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<LauncherResult<u32>>>>,
    session_id: std::sync::Mutex<Option<u64>>,
    log_sender: std::sync::mpsc::SyncSender<QueuedGameLogLine>,
    dropped_log_lines: Arc<AtomicU64>,
}

#[derive(Debug)]
struct QueuedGameLogLine {
    line: String,
    stream: String,
    session_id: Option<u64>,
}

const GAME_LOG_QUEUE_CAPACITY: usize = 4096;
const GAME_LOG_BATCH_CAPACITY: usize = 256;
const GAME_LOG_FLUSH_INTERVAL: Duration = Duration::from_millis(50);

impl TauriLaunchProgress {
    fn new(
        app: tauri::AppHandle,
        state: LauncherState,
        instance_id: String,
        started: tokio::sync::oneshot::Sender<LauncherResult<u32>>,
    ) -> Self {
        let (log_sender, log_receiver) = std::sync::mpsc::sync_channel(GAME_LOG_QUEUE_CAPACITY);
        let dropped_log_lines = Arc::new(AtomicU64::new(0));
        spawn_game_log_bridge(
            app.clone(),
            instance_id.clone(),
            log_receiver,
            dropped_log_lines.clone(),
        );
        Self {
            app,
            state,
            instance_id,
            started: Mutex::new(Some(started)),
            session_id: Mutex::new(None),
            log_sender,
            dropped_log_lines,
        }
    }
}

fn spawn_game_log_bridge(
    app: tauri::AppHandle,
    instance_id: String,
    receiver: std::sync::mpsc::Receiver<QueuedGameLogLine>,
    dropped_log_lines: Arc<AtomicU64>,
) {
    let _ = std::thread::Builder::new()
        .name("agora-game-log-bridge".into())
        .spawn(move || {
            let mut batch = Vec::with_capacity(GAME_LOG_BATCH_CAPACITY);
            let mut next_flush = Instant::now() + GAME_LOG_FLUSH_INTERVAL;
            loop {
                let wait = next_flush.saturating_duration_since(Instant::now());
                match receiver.recv_timeout(wait) {
                    Ok(message) => batch.push(message),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        emit_game_log_batch(
                            &app,
                            &instance_id,
                            &mut batch,
                            dropped_log_lines.as_ref(),
                        );
                        break;
                    }
                }

                if batch.len() >= GAME_LOG_BATCH_CAPACITY || Instant::now() >= next_flush {
                    emit_game_log_batch(&app, &instance_id, &mut batch, dropped_log_lines.as_ref());
                    next_flush = Instant::now() + GAME_LOG_FLUSH_INTERVAL;
                }
            }
        });
}

fn queue_game_log(
    sender: &std::sync::mpsc::SyncSender<QueuedGameLogLine>,
    dropped_log_lines: &AtomicU64,
    message: QueuedGameLogLine,
) {
    if sender.try_send(message).is_err() {
        dropped_log_lines.fetch_add(1, Ordering::Relaxed);
    }
}

fn emit_game_log_batch(
    app: &tauri::AppHandle,
    instance_id: &str,
    batch: &mut Vec<QueuedGameLogLine>,
    dropped_log_lines: &AtomicU64,
) {
    use tauri::Emitter;

    let dropped_lines = dropped_log_lines.swap(0, Ordering::Relaxed);
    if batch.is_empty() && dropped_lines == 0 {
        return;
    }
    let session_id = batch.last().and_then(|entry| entry.session_id);
    let lines = batch
        .drain(..)
        .map(|entry| {
            serde_json::json!({
                "line": entry.line,
                "stream": entry.stream,
            })
        })
        .collect::<Vec<_>>();
    let _ = app.emit(
        "game-log-batch",
        serde_json::json!({
            "lines": lines,
            "dropped_lines": dropped_lines,
            "instance_id": instance_id,
            "session_id": session_id,
        }),
    );
}

impl agora_core::launch_service::LaunchProgress for TauriLaunchProgress {
    fn phase(&self, phase: &str, message: &str) {
        use tauri::Emitter;
        let _ = self.app.emit(
            "launch-progress",
            serde_json::json!({
                "instance_id": self.instance_id,
                "phase": phase,
                "message": message,
            }),
        );
    }

    fn phase_completed(&self, phase: &str, duration_ms: u128) {
        use tauri::Emitter;
        let _ = self.app.emit(
            "launch-progress",
            serde_json::json!({
                "instance_id": self.instance_id,
                "phase": format!("{phase}-complete"),
                "message": format!("Completed in {duration_ms} ms"),
                "duration_ms": duration_ms,
            }),
        );
    }

    fn started(&self, started: &agora_core::launch_service::LaunchStarted) {
        use tauri::Emitter;
        let sender = self.started.lock().ok().and_then(|mut value| value.take());
        if let Ok(mut session_id) = self.session_id.lock() {
            *session_id = Some(started.session_id);
        }
        let app = self.app.clone();
        let state = self.state.clone();
        let instance_id = self.instance_id.clone();
        let started = started.clone();
        tokio::spawn(async move {
            let mut shared = state.lock().await;
            // Reservation matched by instance_id — core assigns session_id.
            if shared.launch_reservations.remove(instance_id.as_str()) {
                shared.running_processes.insert(
                    started.session_id,
                    agora_core::state::RunningProcess {
                        instance_id: instance_id.clone(),
                        pid: started.pid,
                        session_id: started.session_id,
                    },
                );
            }
            let _ = app.emit(
                "game-started",
                serde_json::json!({
                    "instance_id": instance_id,
                    "pid": started.pid,
                    "session_id": started.session_id,
                }),
            );
            if let Some(sender) = sender {
                let _ = sender.send(Ok(started.pid));
            }
        });
    }

    fn log(&self, stream: &str, line: &str) {
        let session_id = self.session_id.lock().ok().and_then(|value| *value);
        let message = QueuedGameLogLine {
            line: line.to_owned(),
            stream: stream.to_owned(),
            session_id,
        };
        queue_game_log(&self.log_sender, self.dropped_log_lines.as_ref(), message);
    }

    fn finished(&self, result: &agora_core::launch_service::LaunchResult) {
        use tauri::Emitter;
        let app = self.app.clone();
        let state = self.state.clone();
        let instance_id = self.instance_id.clone();
        let result = result.clone();
        tokio::spawn(async move {
            let mut shared = state.lock().await;
            shared.running_processes.remove(&result.session_id);
            let _ = app.emit(
                "game-exited",
                serde_json::json!({
                    "instance_id": instance_id,
                    "pid": result.pid,
                    "session_id": result.session_id,
                    "outcome": result.outcome,
                    "snapshot_id": result.snapshot_id,
                }),
            );
        });
    }
}

/// Returns every currently tracked direct-launch process.
///
/// Several instances may run at once, and the same instance may be launched
/// more than once, so this is a list. The core session manager is
/// authoritative: any session it no longer holds has exited or been
/// terminated, and is pruned from the presentation map here.
#[tauri::command]
pub async fn query_launch_state(
    app: tauri::AppHandle,
    state: tauri::State<'_, LauncherState>,
) -> LauncherResult<Vec<agora_core::state::RunningProcess>> {
    let ctx = crate::core_context(&app)?;

    // Phase 1 — snapshot the presentation map under the AppState lock.
    let tracked: Vec<agora_core::state::RunningProcess> = {
        let s = state.lock().await;
        s.running_processes.values().cloned().collect()
    };

    // Phase 2 — consult the authoritative manager for each session.
    let mut live = Vec::new();
    let mut stale = Vec::new();
    for running in tracked {
        if ctx
            .process_session_manager
            .get(running.session_id)
            .is_some()
        {
            live.push(running);
        } else {
            stale.push(running.session_id);
        }
    }

    if !stale.is_empty() {
        let mut s = state.lock().await;
        for session_id in stale {
            s.running_processes.remove(&session_id);
        }
    }

    // Stable order so the frontend does not reshuffle rows between polls.
    live.sort_by_key(|running| running.session_id);
    Ok(live)
}

/// Kill the backend-owned direct-launch process, if any.
///
/// Delegates verification and signalling to the core-owned
/// [`ProcessSessionManager`] and falls back to AppState for frontend
/// presentation fields.
#[tauri::command]
pub async fn kill_process(
    app: tauri::AppHandle,
    state: tauri::State<'_, LauncherState>,
    pid: u32,
) -> LauncherResult<()> {
    let ctx = crate::core_context(&app)?;

    // Phase 1 — snapshot session_id from AppState.
    let session_id = {
        let s = state.lock().await;
        // A PID is unique among live processes, so this identifies at most one
        // session. The authoritative OS-identity check still happens in
        // ProcessSessionManager::terminate below — this only decides which
        // session to ask about.
        let Some(running) = s.running_processes.values().find(|rp| rp.pid == pid) else {
            return Err(LauncherError::Generic {
                code: "ERR_NOT_OWNED".into(),
                message: format!("PID {pid} is not owned by Agora (no such tracked process)"),
            });
        };
        running.session_id
    };

    // Phase 2 — delegate verify + kill to the authoritative manager.
    match ctx.process_session_manager.terminate(session_id, pid) {
        Ok(()) => {
            // Phase 3 — clean up AppState presentation fields.
            let mut s = state.lock().await;
            s.running_processes.remove(&session_id);
            s.user_cancelled_launches.insert(session_id);
            Ok(())
        }
        Err(agora_core::error::LauncherError::ProcessStale { pid: stale_pid, .. }) => {
            // Stale — clear AppState fields so frontend does not show a
            // zombie process.
            let mut s = state.lock().await;
            s.running_processes.remove(&session_id);
            Err(agora_core::error::LauncherError::ProcessStale {
                pid: stale_pid,
                detail: "Stale process detected during kill".into(),
            })
        }
        Err(other) => Err(other),
    }
}

/// Run the pre-launch health scan on an instance. Returns a [`HealthReport`]
/// with blockers (must resolve before launch) and warnings (may override).
#[tauri::command]
pub async fn check_instance_health(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
) -> LauncherResult<agora_core::health::HealthReport> {
    let ctx = crate::core_context(&app)?;
    ctx.task_scheduler
        .run_blocking(
            agora_core::task_scheduler::BlockingPriority::UserInitiated,
            move || {
                let sanitized = paths::sanitize_id(&instance_id);
                let instance_dir =
                    paths::instance_dir(&app, &sanitized).map_err(|e| LauncherError::Generic {
                        code: "ERR_INSTANCE_PATH".into(),
                        message: e.to_string(),
                    })?;
                let mut manifest = load_manifest(&app, &sanitized)?;

                // Parse jar dependency metadata here, where the user already
                // expects a scan to take a moment, and let it populate the
                // per-instance cache. Two things follow: the health check itself
                // reasons about CURRENT jar declarations rather than whatever the
                // manifest happened to record at install time, and every later
                // dependency read (graph, disable plan, removal plan) hits a warm
                // cache instead of re-opening every jar.
                //
                // Best-effort: a jar that will not parse is a health finding, not
                // a reason to fail the whole check.
                let _ = dependency_ops::refresh_installed_jar_metadata(
                    &app,
                    &sanitized,
                    &mut manifest.mods,
                );

                // Registry DB for curated known_conflicts â€” optional (Phase 3: never required)
                let reg_path = paths::registry_db_path(&app).ok();

                Ok(agora_core::health::cached_health(
                    &instance_dir,
                    &manifest,
                    reg_path.as_deref(),
                    None,
                ))
            },
        )
        .await
        .map_err(|_| LauncherError::LocalStateFailed)?
}

/// Health status for one entry in a background all-instance scan. Individual
/// failures are returned in-band so an unreadable instance never prevents the
/// rest of the library from reporting its health state.
#[derive(Debug, serde::Serialize)]
pub struct InstanceHealthScanResult {
    pub instance_id: String,
    pub report: Option<agora_core::health::HealthReport>,
    pub error: Option<String>,
}

/// Scan every local instance at background priority. The desktop shell calls
/// this periodically to keep instance-card alerts current; pre-launch checks
/// remain user-initiated and authoritative.
#[tauri::command]
pub async fn check_all_instance_health(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
) -> LauncherResult<Vec<InstanceHealthScanResult>> {
    let ctx = crate::core_context(&app)?;
    let worker_ctx = ctx.clone();
    let app_for_scan = app.clone();
    let registry_db_path = paths::registry_db_path(&app).ok();
    ctx.task_scheduler
        .run_blocking(
            agora_core::task_scheduler::BlockingPriority::Background,
            move || {
                let rows = agora_core::instance_service::InstanceService::new(worker_ctx.clone())
                    .list()?;
                let mut results = Vec::with_capacity(rows.len());
                for row in rows {
                    let instance_id = row.instance_id;
                    let result =
                        (|| -> LauncherResult<agora_core::health::HealthReport> {
                            let sanitized = paths::sanitize_id(&instance_id);
                            if sanitized.is_empty() || sanitized != instance_id {
                                return Err(LauncherError::Generic {
                                    code: "ERR_INVALID_INSTANCE".into(),
                                    message: "Stored instance ID is invalid.".into(),
                                });
                            }
                            let instance_dir = paths::instance_dir(&app_for_scan, &sanitized)
                                .map_err(|error| LauncherError::Generic {
                                    code: "ERR_INSTANCE_PATH".into(),
                                    message: error.to_string(),
                                })?;
                            let manifest = load_manifest(&app_for_scan, &sanitized)?;
                            Ok(agora_core::health::cached_health(
                                &instance_dir,
                                &manifest,
                                registry_db_path.as_deref(),
                                None,
                            ))
                        })();
                    match result {
                        Ok(report) => results.push(InstanceHealthScanResult {
                            instance_id,
                            report: Some(report),
                            error: None,
                        }),
                        Err(error) => results.push(InstanceHealthScanResult {
                            instance_id,
                            report: None,
                            error: Some(error.to_string()),
                        }),
                    }
                }
                Ok(results)
            },
        )
        .await
        .map_err(|_| LauncherError::LocalStateFailed)?
}

/// List pinned loader versions for a loader + Minecraft version.
#[tauri::command]
pub async fn list_loader_versions(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    loader: String,
    mc_version: String,
) -> LauncherResult<Vec<LoaderVersionSummary>> {
    let ctx = crate::core_context(&app)?;
    Ok(agora_core::loader_service::LoaderService::new(ctx).list_versions(&loader, &mc_version))
}

/// Plan a loader version switch for an instance without mutating anything.
///
/// Locks the instance for the duration, rejects an active core-managed
/// process, re-inventories the enabled mods against the active signed loader
/// catalog, and returns the current tuple, the proven recommendation (when
/// one exists), and the full compatibility report. Committing a selection is
/// a separate [`change_loader_version`] call.
#[tauri::command]
pub async fn plan_loader_change(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
) -> LauncherResult<agora_core::loader_service::LoaderChangePlan> {
    let ctx = crate::core_context(&app)?;
    let scheduler = ctx.task_scheduler.clone();
    scheduler
        .run_blocking(
            agora_core::task_scheduler::BlockingPriority::UserInitiated,
            move || {
                agora_core::loader_service::LoaderService::new(ctx).plan_loader_change(&instance_id)
            },
        )
        .await
        .map_err(|_| LauncherError::LocalStateFailed)?
}

/// Change an instance's loader version in one transactional operation.
///
/// The target must be an exact pinned signed-catalog tuple for the instance's
/// loader + Minecraft version that satisfies every hard loader requirement of
/// the enabled mods. The target is installed first, then the manifest and DB
/// tuple are committed (with rollback on failure), and a fresh post-switch
/// health report is returned with the result.
#[tauri::command]
pub async fn change_loader_version(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
    target_version: String,
    allow_indeterminate: Option<bool>,
) -> LauncherResult<agora_core::loader_service::LoaderChangeResult> {
    let ctx = crate::core_context(&app)?;
    agora_core::loader_service::LoaderService::new(ctx)
        .change_loader_version_with_confirmation(
            &instance_id,
            &target_version,
            allow_indeterminate.unwrap_or(false),
        )
        .await
}

/// Force-reinstall the loader for an instance (repair command).
///
/// Downloads the curated installer again, backs up the existing profile,
/// runs the installer, validates the result, and generates a fresh receipt.
#[tauri::command]
pub async fn repair_instance_loader(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
) -> LauncherResult<agora_core::installed_profile::InstallReceiptSummary> {
    instances::repair_instance_loader(&app, &instance_id).await
}

/// Distinct loader names present in the embedded loader manifests.
#[tauri::command]
pub async fn list_manifest_loaders(
    _app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
) -> LauncherResult<Vec<String>> {
    Ok(loader_manifests::list_loaders()
        .iter()
        .map(|s| s.to_string())
        .collect())
}

/// Distinct Minecraft versions across all loaders (or one loader when supplied).
#[tauri::command]
pub async fn list_manifest_mc_versions(
    _app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    loader: Option<String>,
) -> LauncherResult<Vec<String>> {
    Ok(loader_manifests::list_mc_versions(loader.as_deref()))
}

/// Read a JSON-encoded setting from `local_state.db`.
#[tauri::command]
pub async fn get_setting(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    key: String,
) -> LauncherResult<Option<serde_json::Value>> {
    tokio::task::spawn_blocking(move || {
        let ctx = crate::core_context(&app)?;
        let svc = agora_core::settings::SettingsService::new(ctx);
        svc.get(&key)
    })
    .await
    .map_err(|_| LauncherError::LocalStateFailed)?
}

/// Upsert a JSON-encoded setting into `local_state.db`.
#[tauri::command]
pub async fn set_setting(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    key: String,
    value: serde_json::Value,
) -> LauncherResult<()> {
    tokio::task::spawn_blocking(move || {
        let ctx = crate::core_context(&app)?;
        let svc = agora_core::settings::SettingsService::new(ctx);
        svc.set(&key, &value)
    })
    .await
    .map_err(|_| LauncherError::LocalStateFailed)?
}

/// Check GitHub Releases for a registry.db update and download + verify it.
#[tauri::command]
pub async fn check_registry_update(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    force: Option<bool>,
) -> LauncherResult<crate::registry_sync::RegistryStatus> {
    crate::registry_sync::check_and_download_update(&app, force.unwrap_or(false)).await
}

/// Return current registry status without network check.
#[tauri::command]
pub async fn get_registry_status(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
) -> LauncherResult<crate::registry_sync::RegistryStatus> {
    Ok(crate::registry_sync::get_status(&app))
}

/// Extract a pack override zip into an instance directory with full sanitization.
///
/// Implements Â§7.2: directory whitelist, zip-bomb limits, banned extensions,
/// and Zip Slip protection.
#[tauri::command]
pub async fn extract_overrides(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    zip_path: String,
    instance_id: String,
) -> LauncherResult<crate::override_sanitizer::ExtractionResult> {
    let zip = std::path::PathBuf::from(zip_path);
    let dest = crate::paths::instance_dir(&app, &instance_id)
        .map_err(|_| LauncherError::InstanceCreateFailed)?;
    tokio::task::spawn_blocking(move || crate::override_sanitizer::extract_overrides(&zip, &dest))
        .await
        .map_err(|_| LauncherError::Generic {
            code: "ERR_OVERRIDE_FAILED".to_string(),
            message: "Extraction task failed.".to_string(),
        })?
}

/// Begin the GitHub OAuth Device Flow and return the code the user must enter.
#[tauri::command]
pub async fn github_login() -> LauncherResult<DeviceFlowResponse> {
    crate::auth::start_device_flow().await
}

/// Poll the GitHub token endpoint until the user authorizes the device.
/// Returns true if the token was obtained and stored; false if still pending.
#[tauri::command]
pub async fn github_login_poll(
    app: tauri::AppHandle,
    device_code: String,
    interval: u64,
) -> LauncherResult<bool> {
    crate::auth::log_line(&format!(
        "github_login_poll command ENTERED device_code_len={} interval={}",
        device_code.len(),
        interval
    ));
    let bundle = crate::auth::poll_device_flow(device_code, interval).await?;
    if let Some(b) = bundle {
        crate::auth::store_token_bundle(&app, &b)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Sign out by deleting any stored GitHub token.
#[tauri::command]
pub async fn github_logout(app: tauri::AppHandle) -> Result<(), String> {
    crate::auth::clear_token(&app)
}

/// Whether a GitHub token is currently stored.
#[tauri::command]
pub async fn get_auth_status(app: tauri::AppHandle) -> bool {
    crate::auth::is_authenticated(&app)
}

/// Fetch the authenticated user's GitHub profile, if signed in.
/// Stale tokens are automatically cleared from storage on AuthExpired.
#[tauri::command]
pub async fn get_github_profile(app: tauri::AppHandle) -> LauncherResult<Option<GithubProfile>> {
    match crate::auth::get_validated_github_profile(&app).await {
        Ok(p) => Ok(Some(p)),
        Err(crate::error::LauncherError::AuthExpired) => {
            // Token was cleared in get_validated_github_profile.
            // Propagate so the frontend can show the sign-in prompt.
            Err(crate::error::LauncherError::AuthExpired)
        }
        Err(_) => Ok(None),
    }
}

/// Check whether a fresh crash report appeared after the instance's last launch.
#[tauri::command]
pub async fn check_instance_crash(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
) -> LauncherResult<Option<CrashReportInfo>> {
    tokio::task::spawn_blocking(move || crash_diagnostics::check_for_crash(&app, &instance_id))
        .await
        .map_err(|_| LauncherError::LocalStateFailed)?
}

/// Triage a crash log against curated signatures from the registry.
#[tauri::command]
pub async fn triage_crash_report(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
    filename: String,
) -> LauncherResult<CrashTriageResult> {
    tokio::task::spawn_blocking(move || {
        crash_diagnostics::triage_crash(&app, &instance_id, &filename)
    })
    .await
    .map_err(|_| LauncherError::LocalStateFailed)?
}

/// List all crash report files for an instance.
#[tauri::command]
pub async fn list_crash_reports_cmd(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
) -> LauncherResult<Vec<CrashReportInfo>> {
    tokio::task::spawn_blocking(move || crash_diagnostics::list_crash_reports(&app, &instance_id))
        .await
        .map_err(|_| LauncherError::LocalStateFailed)?
}

/// Read the content of a specific crash report file.
#[tauri::command]
pub async fn read_crash_log_cmd(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
    filename: String,
) -> LauncherResult<String> {
    tokio::task::spawn_blocking(move || {
        crash_diagnostics::read_crash_log(&app, &instance_id, &filename)
    })
    .await
    .map_err(|_| LauncherError::LocalStateFailed)?
}

/// Collect and investigate the most useful recent evidence for an instance.
#[tauri::command]
pub async fn investigate_instance_evidence(
    app: tauri::AppHandle,
    instance_id: String,
) -> LauncherResult<agora_core::crash_service::CrashInvestigation> {
    let ctx = crate::core_context(&app)?;
    tokio::task::spawn_blocking(move || {
        agora_core::crash_service::CrashService::new(ctx).investigate_evidence(&instance_id, &[])
    })
    .await
    .map_err(|_| LauncherError::LocalStateFailed)?
}

/// Pick additional diagnostic files and investigate them without returning
/// their filesystem paths to the webview.
#[tauri::command]
pub async fn pick_and_investigate_crash_evidence(
    app: tauri::AppHandle,
    instance_id: String,
) -> LauncherResult<Option<agora_core::crash_service::CrashInvestigation>> {
    let picked = rfd::AsyncFileDialog::new()
        .set_title("Add crash evidence")
        .add_filter("Diagnostic text", &["log", "txt", "json", "md"])
        .pick_files()
        .await;
    let Some(files) = picked else {
        return Ok(None);
    };
    let paths = files
        .into_iter()
        .map(|file| file.path().to_path_buf())
        .collect::<Vec<_>>();
    let ctx = crate::core_context(&app)?;
    tokio::task::spawn_blocking(move || {
        agora_core::crash_service::CrashService::new(ctx)
            .investigate_evidence(&instance_id, &paths)
            .map(Some)
    })
    .await
    .map_err(|_| LauncherError::LocalStateFailed)?
}

/// List available mod versions for a registry item, resolving live data from
/// the upstream source (GitHub Releases or Modrinth).  Uses a bi-directional
/// initial fetch: page 1 (newest) first, then tail pages (oldest) when the
/// user's MC version isn't found on the first page, so older-version users
/// see compatible versions at the top without scrolling through hundreds of
/// newer releases.
///
/// The result is cached and the first page is returned immediately.  Remaining
/// pages are fetched lazily via `list_mod_versions_load_more`.
#[tauri::command]
pub async fn list_mod_versions(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: Option<String>,
    item_id: String,
) -> LauncherResult<ModVersionPage> {
    // When no instance is provided (e.g. the Versions tab browsing without
    // an instance selected), use empty strings so all releases are fetched
    // without compatibility filtering.
    let (mc_ver, loader) = match &instance_id {
        Some(id) => {
            let inst = mod_install::load_instance_info(&app, id)?;
            (inst.minecraft_version, inst.loader)
        }
        None => (String::new(), String::new()),
    };
    let item = mod_install::load_registry_item(&app, &item_id)?;

    // A GitHub source gets the paginated release browser; everything else
    // resolves through the generic path, which walks the item's sources in
    // order.
    let listing = mod_install::version_listing_source(&app, &item)?;
    let listing_strategy = listing
        .as_ref()
        .map(|source| source.strategy.as_str())
        .unwrap_or("");

    match listing_strategy {
        "github_release" => {
            let repo = listing
                .as_ref()
                .map(|source| source.identifier.clone())
                .unwrap_or_default();
            let (all_versions, total_pages, pages_fetched) =
                mod_install::resolve_github_releases_initial(&app, &repo, &mc_ver, &loader).await?;
            let pages_set: BTreeSet<u32> = pages_fetched.into_iter().collect();
            let total = all_versions.len();
            version_cache::load_versions(
                &VERSION_CACHE,
                &item_id,
                &mc_ver,
                &loader,
                &repo,
                listing_strategy,
                all_versions,
                total_pages,
                pages_set,
            )
            .await;
            let page = version_cache::get_page(&VERSION_CACHE, &item_id, &mc_ver, &loader, 0)
                .await
                .unwrap_or_else(|| ModVersionPage {
                    items: Vec::new(),
                    has_more: false,
                    total,
                });
            Ok(page)
        }
        // Every other strategy returns its whole list in one shot (no
        // pagination). `mc_ver`/`loader` are empty when no instance is
        // selected, which asks each source for its unfiltered version list.
        _ => {
            let all_versions =
                mod_install::list_mod_versions_for(&app, &item_id, &mc_ver, &loader).await?;
            let total = all_versions.len();
            let pages_set: BTreeSet<u32> = [1].into_iter().collect();
            // Record the source that actually produced the list, so a later
            // "load more" targets that source rather than the item's primary.
            let resolved = all_versions
                .first()
                .and_then(|candidate| {
                    Some((
                        candidate.source_identifier.clone()?,
                        candidate.source_strategy.clone()?,
                    ))
                })
                .unwrap_or_else(|| {
                    (
                        listing
                            .as_ref()
                            .map(|source| source.identifier.clone())
                            .unwrap_or_else(|| item.source_identifier.clone()),
                        listing_strategy.to_string(),
                    )
                });
            version_cache::load_versions(
                &VERSION_CACHE,
                &item_id,
                &mc_ver,
                &loader,
                &resolved.0,
                &resolved.1,
                all_versions,
                1,
                pages_set,
            )
            .await;
            let page = version_cache::get_page(&VERSION_CACHE, &item_id, &mc_ver, &loader, 0)
                .await
                .unwrap_or_else(|| ModVersionPage {
                    items: Vec::new(),
                    has_more: false,
                    total,
                });
            Ok(page)
        }
    }
}

/// Load the next page of mod versions from the cache.  If the cache doesn't
/// have enough data yet, it fetches the next batch of GitHub pages lazily
/// and extends the cache before returning.
#[tauri::command]
pub async fn list_mod_versions_load_more(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: Option<String>,
    item_id: String,
    page: usize,
) -> LauncherResult<ModVersionPage> {
    let (mc_ver, loader) = match &instance_id {
        Some(id) => {
            let inst = mod_install::load_instance_info(&app, id)?;
            (inst.minecraft_version, inst.loader)
        }
        None => (String::new(), String::new()),
    };

    // Check if the cache already has enough data for this page.
    if let Some(page_data) =
        version_cache::get_page(&VERSION_CACHE, &item_id, &mc_ver, &loader, page).await
    {
        let need_more = page_data.items.is_empty() && page_data.has_more;
        if !need_more {
            return Ok(page_data);
        }
    }

    // Cache miss or empty page — fetch more pages from whichever source built
    // this entry. The item's primary source is not necessarily that source.
    let entry = version_cache::get_entry(&VERSION_CACHE, &item_id, &mc_ver, &loader).await;
    let (pages_fetched, total_pages, source_identifier) = match &entry {
        Some(e) => (
            e.pages_fetched.clone(),
            e.total_pages,
            e.source_identifier.clone(),
        ),
        None => {
            // Shouldn't happen if list_mod_versions was called first,
            // but guard against it.
            return Err(LauncherError::Generic {
                code: "ERR_VERSION_CACHE_MISS".to_string(),
                message: "Version cache is empty. Call list_mod_versions first.".to_string(),
            });
        }
    };

    // Build the set of unfetched page numbers.
    let to_fetch: Vec<u32> = (2..=total_pages)
        .filter(|p| !pages_fetched.contains(p))
        .collect();

    if to_fetch.is_empty() {
        // All pages already fetched — nothing more to load.
        return version_cache::get_page(&VERSION_CACHE, &item_id, &mc_ver, &loader, page)
            .await
            .ok_or_else(|| LauncherError::Generic {
                code: "ERR_VERSION_CACHE_MISS".to_string(),
                message: "Cache entry vanished.".to_string(),
            });
    }

    // Fetch the next up-to-3 unfetched pages concurrently.
    let batch: Vec<u32> = to_fetch.into_iter().take(3).collect();

    let results = mod_install::fetch_github_versions_batch(
        &app,
        &source_identifier,
        &mc_ver,
        &loader,
        &batch,
    )
    .await?;

    let page_nums: Vec<u32> = results.iter().map(|(p, _)| *p).collect();
    let mut all_more: Vec<ModVersionCandidate> = Vec::new();
    for (_p, cands) in results {
        all_more.extend(cands);
    }

    version_cache::extend_versions(
        &VERSION_CACHE,
        &item_id,
        &mc_ver,
        &loader,
        all_more,
        &page_nums,
    )
    .await;

    // Now try again for the requested page.
    version_cache::get_page(&VERSION_CACHE, &item_id, &mc_ver, &loader, page)
        .await
        .ok_or_else(|| LauncherError::Generic {
            code: "ERR_VERSION_CACHE_MISS".to_string(),
            message: "Cache entry vanished after extend.".to_string(),
        })
}

/// Quick compatibility check: does this mod have at least one release
/// matching the given MC version + loader?  Used by the browse page to
/// show a compatibility indicator without fetching the full version list.
#[tauri::command]
pub async fn check_mod_compat(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
    item_id: String,
) -> LauncherResult<String> {
    mod_install::check_mod_compat(&app, &instance_id, &item_id).await
}

/// Reuse the current LKG archive when tracked content is unchanged. The
/// snapshot layer reuses hashes for files whose metadata is unchanged and only
/// reads changed files on the reconcile path.
#[allow(dead_code)]
fn create_or_reuse_prelaunch_snapshot(instance_dir: &Path, label: &str) -> Result<String, String> {
    let lkg = agora_core::lkg::read_lkg_state(instance_dir)?;
    if let Some(snapshot_id) = lkg.current_lkg_snapshot_id {
        if agora_core::snapshot::snapshot_matches_live_incremental(instance_dir, &snapshot_id)
            .unwrap_or(false)
        {
            return Ok(snapshot_id);
        }
    }
    agora_core::snapshot::create_snapshot(instance_dir, Some(label)).map(|snapshot| snapshot.id)
}

/// Batch compatibility from the signed registry metadata. This avoids one
/// network-backed compatibility request per Browse card while keeping the
/// compatibility decision in Rust rather than duplicating it in React.
#[tauri::command]
pub async fn batch_check_compat(
    app: tauri::AppHandle,
    instance_id: String,
    item_ids: Vec<String>,
) -> LauncherResult<std::collections::BTreeMap<String, String>> {
    let sanitized = paths::sanitize_id(&instance_id);
    if sanitized.is_empty() || sanitized != instance_id {
        return Err(LauncherError::Generic {
            code: "ERR_INVALID_INSTANCE".into(),
            message: "The instance ID is invalid.".into(),
        });
    }
    let manifest_path = paths::instance_manifest_path(&app, &sanitized)
        .map_err(|_| LauncherError::LocalStateFailed)?;
    let manifest = agora_core::helpers::read_manifest(&manifest_path)
        .map_err(|_| LauncherError::LocalStateFailed)?;
    let ctx = crate::core_context(&app)?;
    let svc = agora_core::registry::RegistryService::new(ctx);
    svc.batch_compat_lookup(&item_ids, &manifest.minecraft_version, &manifest.loader)
}

/// Disable a mod by renaming `mods/<filename>` to `mods/<filename>.disabled`.
#[tauri::command]
pub async fn disable_instance_mod(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
    filename: String,
) -> LauncherResult<()> {
    check_not_locked(&app, &instance_id)?;
    tokio::task::spawn_blocking(move || {
        mod_install::disable_instance_mod(&app, &instance_id, &filename)
    })
    .await
    .map_err(|_| LauncherError::LocalStateFailed)?
}

/// Re-enable a disabled mod by renaming `mods/<filename>.disabled` back.
#[tauri::command]
pub async fn enable_instance_mod(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
    filename: String,
) -> LauncherResult<()> {
    check_not_locked(&app, &instance_id)?;
    tokio::task::spawn_blocking(move || {
        mod_install::enable_instance_mod(&app, &instance_id, &filename)
    })
    .await
    .map_err(|_| LauncherError::LocalStateFailed)?
}

/// Open a native file picker and return the chosen file path, or `None` if cancelled.
#[tauri::command]
pub async fn pick_open_file(
    _app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    title: String,
    extensions: Vec<String>,
) -> LauncherResult<Option<String>> {
    let mut dialog = rfd::AsyncFileDialog::new().set_title(&title);
    if !extensions.is_empty() {
        let exts: Vec<&str> = extensions.iter().map(|s| s.as_str()).collect();
        dialog = dialog.add_filter("Allowed", &exts);
    }
    let picked = dialog.pick_file().await;
    Ok(picked.map(|h| h.path().to_string_lossy().to_string()))
}

/// Copy a user-selected image into an unlocked modpack instance.
#[tauri::command]
pub async fn set_custom_instance_icon(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
    source_path: String,
) -> LauncherResult<String> {
    let ctx = crate::core_context(&app)?;
    tokio::task::spawn_blocking(move || {
        agora_core::icon::set_instance_icon(&ctx, &instance_id, std::path::Path::new(&source_path))
    })
    .await
    .map_err(|_| LauncherError::LocalStateFailed)?
}

/// Copy a user-selected image into an unlocked instance's installed mod icons.
#[tauri::command]
pub async fn set_custom_mod_icon(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
    filename: String,
    source_path: String,
) -> LauncherResult<String> {
    let ctx = crate::core_context(&app)?;
    tokio::task::spawn_blocking(move || {
        agora_core::icon::set_mod_icon(
            &ctx,
            &instance_id,
            &filename,
            std::path::Path::new(&source_path),
        )
    })
    .await
    .map_err(|_| LauncherError::LocalStateFailed)?
}

/// Read an Agora-owned custom icon as a data URL for the WebView.
#[tauri::command]
pub async fn get_custom_icon(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
    target: String,
    filename: Option<String>,
) -> LauncherResult<Option<String>> {
    let ctx = crate::core_context(&app)?;
    tokio::task::spawn_blocking(move || {
        agora_core::icon::get_custom_icon(&ctx, &instance_id, &target, filename.as_deref())
    })
    .await
    .map_err(|_| LauncherError::LocalStateFailed)?
}

/// Open a native directory picker for a launcher data root.
#[tauri::command]
pub async fn pick_directory(title: String) -> LauncherResult<Option<String>> {
    let picked = rfd::AsyncFileDialog::new()
        .set_title(&title)
        .pick_folder()
        .await;
    Ok(picked.map(|handle| handle.path().to_string_lossy().to_string()))
}

/// Detect importable Prism, CurseForge, and Modrinth launcher instances.
#[tauri::command]
pub async fn discover_launcher_imports(
    app: tauri::AppHandle,
    custom_root: Option<String>,
) -> LauncherResult<agora_core::launcher_import_service::LauncherImportDiscovery> {
    let ctx = crate::core_context(&app)?;
    tokio::task::spawn_blocking(move || {
        agora_core::launcher_import_service::LauncherImportService::new(ctx)
            .discover(custom_root.map(std::path::PathBuf::from))
    })
    .await
    .map_err(|error| LauncherError::Generic {
        code: "ERR_IMPORT_DISCOVERY".into(),
        message: error.to_string(),
    })
}

/// Build a read-only, provenance-aware launcher import plan.
#[tauri::command]
pub async fn plan_launcher_imports(
    app: tauri::AppHandle,
    selections: Vec<agora_core::launcher_import_service::ImportSelection>,
) -> LauncherResult<agora_core::launcher_import_service::LauncherImportPlan> {
    let ctx = crate::core_context(&app)?;
    tokio::task::spawn_blocking(move || {
        agora_core::launcher_import_service::LauncherImportService::new(ctx).plan(selections)
    })
    .await
    .map_err(|error| LauncherError::Generic {
        code: "ERR_IMPORT_PLAN".into(),
        message: error.to_string(),
    })?
}

/// Execute a reviewed launcher import plan. Items commit independently.
#[tauri::command]
pub async fn execute_launcher_imports(
    app: tauri::AppHandle,
    plan: agora_core::launcher_import_service::LauncherImportPlan,
) -> LauncherResult<agora_core::launcher_import_service::LauncherImportBatchResult> {
    let ctx = crate::core_context(&app)?;
    agora_core::launcher_import_service::LauncherImportService::new(ctx)
        .execute(plan)
        .await
}

/// Export an instance as a shareable pack file (Â§6.5c).
#[tauri::command]
pub async fn export_instance_pack(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
    format: String,
) -> LauncherResult<String> {
    mod_install::export_instance_pack(&app, &instance_id, &format).await
}

/// Import an instance from a pack file (.mrpack or .agora-pack.json).
#[tauri::command]
pub async fn import_instance_pack(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    source_path: String,
) -> LauncherResult<String> {
    mod_install::import_instance_pack(&app, &source_path).await
}

/// Whether the Modrinth integration is currently enabled (Â§6.3 toggle).
#[tauri::command]
pub async fn is_modrinth_enabled(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
) -> LauncherResult<bool> {
    Ok(modrinth_raw::is_modrinth_enabled(&app))
}

/// Live search of all of Modrinth (uncurated, Â§6.3). Gated by the
/// `modrinth_enabled` setting; returns `Err(ModrinthDisabled)` when off.
#[tauri::command]
pub async fn search_modrinth(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    params: modrinth_raw::ModrinthSearchParams,
) -> LauncherResult<modrinth_raw::ModrinthSearchPage> {
    let ctx = crate::core_context(&app)?;
    modrinth_raw::search_modrinth(&ctx.http_clients, &app, &params).await
}

/// List Modrinth category tags for the filter UI.
#[tauri::command]
pub async fn list_modrinth_categories(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
) -> LauncherResult<Vec<modrinth_raw::ModrinthCategoryInfo>> {
    let ctx = crate::core_context(&app)?;
    modrinth_raw::list_modrinth_categories(&ctx.http_clients, &app).await
}

/// List Modrinth loader tags for the filter UI.
#[tauri::command]
pub async fn list_modrinth_loaders(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
) -> LauncherResult<Vec<modrinth_raw::ModrinthLoaderInfo>> {
    let ctx = crate::core_context(&app)?;
    modrinth_raw::list_modrinth_loaders(&ctx.http_clients, &app).await
}

/// List Modrinth game version tags for the filter UI.
#[tauri::command]
pub async fn list_modrinth_game_versions(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
) -> LauncherResult<Vec<modrinth_raw::ModrinthGameVersionInfo>> {
    let ctx = crate::core_context(&app)?;
    modrinth_raw::list_modrinth_game_versions(&ctx.http_clients, &app).await
}

/// List raw Modrinth versions for a project, optionally scoped to an
/// instance's Minecraft version and loader.
#[tauri::command]
pub async fn list_raw_modrinth_versions(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: Option<String>,
    project_id: String,
    project_type: Option<String>,
) -> LauncherResult<Vec<modrinth_raw::RawModrinthVersionCandidate>> {
    let ctx = crate::core_context(&app)?;
    modrinth_raw::list_raw_modrinth_versions(
        &ctx.http_clients,
        &app,
        instance_id.as_deref(),
        &project_id,
        project_type.as_deref(),
    )
    .await
}

/// Fetch a single Modrinth project's full details (including body markdown).
#[tauri::command]
pub async fn fetch_modrinth_project(
    app: tauri::AppHandle,
    project_id: String,
) -> Result<modrinth_raw::ModrinthProjectFull, LauncherError> {
    let ctx = crate::core_context(&app)?;
    modrinth_raw::fetch_project_full(&ctx.http_clients, &app, &project_id).await
}

/// List registry items whose status is `under_review`, ordered by net_score.
#[tauri::command]
pub async fn list_under_review_items(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
) -> LauncherResult<Vec<UnderReviewItem>> {
    tokio::task::spawn_blocking(move || {
        let ctx = crate::core_context(&app)?;
        let svc = agora_core::registry::RegistryService::new(ctx);
        svc.list_under_review_items()
    })
    .await
    .map_err(|_| LauncherError::Generic {
        code: "ERR_REGISTRY_QUERY".to_string(),
        message: "Under-review query task failed.".to_string(),
    })?
}

/// List recent triage resolutions from the audit log.
#[tauri::command]
pub async fn list_recent_resolutions(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    limit: Option<u32>,
) -> LauncherResult<Vec<AuditLogEntry>> {
    let limit = limit.unwrap_or(50);
    tokio::task::spawn_blocking(move || {
        let ctx = crate::core_context(&app)?;
        let svc = agora_core::registry::RegistryService::new(ctx);
        svc.list_recent_resolutions(limit)
    })
    .await
    .map_err(|_| LauncherError::Generic {
        code: "ERR_REGISTRY_QUERY".to_string(),
        message: "Recent resolutions query task failed.".to_string(),
    })?
}

/// Load parsed curator reviews for a single registry item.
#[tauri::command]
pub async fn list_mod_reviews(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    item_id: String,
) -> LauncherResult<Vec<ModReview>> {
    tokio::task::spawn_blocking(move || {
        let ctx = crate::core_context(&app)?;
        let svc = agora_core::registry::RegistryService::new(ctx);
        svc.list_mod_reviews(item_id)
    })
    .await
    .map_err(|_| LauncherError::Generic {
        code: "ERR_REGISTRY_QUERY".to_string(),
        message: "Mod reviews query task failed.".to_string(),
    })?
}

/// Fetch the live triage poll for a mod from GitHub Discussions.
#[tauri::command]
pub async fn fetch_triage_poll(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    mod_id: String,
) -> LauncherResult<crate::governance::TriagePoll> {
    crate::governance::fetch_triage_poll(&app, mod_id).await
}

/// Return the resolved governance configuration.
#[tauri::command]
pub async fn get_governance_config(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
) -> LauncherResult<GovernanceConfig> {
    Ok(crate::governance::get_governance_config(&app))
}

/// Fetch the governance summary for a single registry item.
#[tauri::command]
pub async fn get_governance_summary(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    item_id: String,
) -> LauncherResult<Option<GovernanceSummary>> {
    tokio::task::spawn_blocking(move || crate::governance::get_governance_summary(&app, &item_id))
        .await
        .map_err(|_| LauncherError::Generic {
            code: "ERR_GOVERNANCE_QUERY".to_string(),
            message: "Governance summary query failed.".to_string(),
        })?
}

/// Return the signed-in user's vote on an item's canonical GitHub issue.
#[tauri::command]
pub async fn get_item_vote(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    item_id: String,
) -> LauncherResult<ItemVoteState> {
    crate::governance::get_item_vote(&app, item_id).await
}

/// Set, switch, or retract the signed-in user's canonical item vote.
#[tauri::command]
pub async fn set_item_vote(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    item_id: String,
    vote: Option<ItemVote>,
) -> LauncherResult<ItemVoteState> {
    crate::governance::set_item_vote(&app, item_id, vote).await
}

/// List governance events, optionally filtered by item_id.
#[tauri::command]
pub async fn list_governance_events(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    item_id: Option<String>,
) -> LauncherResult<Vec<GovernanceEvent>> {
    tokio::task::spawn_blocking(move || {
        crate::governance::list_governance_events(&app, item_id.as_deref())
    })
    .await
    .map_err(|_| LauncherError::Generic {
        code: "ERR_GOVERNANCE_QUERY".to_string(),
        message: "Governance events query failed.".to_string(),
    })?
}

/// Run read-only governance diagnostics combining sync + async checks.
///
/// 1. Spawn_blocking for sync checks (oauth, token, repo, schema tables, parses).
/// 2. Await async network checks (repo metadata, issues, discussions, labels).
/// 3. Merge and return all `Vec<DiagnosticCheck>`.
#[tauri::command]
pub async fn run_governance_diagnostics(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
) -> LauncherResult<Vec<DiagnosticCheck>> {
    let sync_app = app.clone();
    let mut sync_checks: Vec<DiagnosticCheck> =
        tokio::task::spawn_blocking(move || crate::governance::run_sync_diagnostics(&sync_app))
            .await
            .map_err(|_| LauncherError::Generic {
                code: "ERR_GOVERNANCE_DIAGNOSTICS".to_string(),
                message: "Governance sync diagnostics task failed.".to_string(),
            })?;

    let net_checks = crate::governance::run_network_diagnostics(&app).await;

    sync_checks.extend(net_checks);
    Ok(sync_checks)
}

/// Return one locally enriched inventory snapshot for an instance.
///
/// Registry data is optional: manifest and filesystem rows remain usable when
/// the cached registry is absent or stale.
#[tauri::command]
pub async fn list_instance_content(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
    content_type: Option<String>,
) -> LauncherResult<Vec<InstalledContentRow>> {
    tokio::task::spawn_blocking(move || {
        let sanitized = paths::sanitize_id(&instance_id);
        let manifest = load_manifest(&app, &sanitized)?;
        let instance_dir = paths::instance_dir(&app, &sanitized)
            .map_err(|_| LauncherError::InstanceCreateFailed)?;
        let ctx = crate::core_context(&app)?;
        let registry = agora_core::registry::RegistryService::new(ctx);
        Ok(agora_core::installed_content::list_installed_content(
            &instance_dir,
            &manifest,
            content_type.as_deref(),
            Some(&registry),
        ))
    })
    .await
    .map_err(|_| LauncherError::LocalStateFailed)?
}

/// Enrich an already-loaded inventory with author metadata. This is a single
/// backend operation so the frontend never performs one Modrinth request per
/// installed row.
#[tauri::command]
pub async fn enrich_instance_content(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
) -> LauncherResult<Vec<InstalledContentMetadata>> {
    let sanitized = paths::sanitize_id(&instance_id);
    let inventory_app = app.clone();
    let rows = tokio::task::spawn_blocking(move || {
        let manifest = load_manifest(&inventory_app, &sanitized)?;
        let instance_dir = paths::instance_dir(&inventory_app, &sanitized)
            .map_err(|_| LauncherError::InstanceCreateFailed)?;
        let ctx = crate::core_context(&inventory_app)?;
        let registry = agora_core::registry::RegistryService::new(ctx);
        Ok::<Vec<InstalledContentRow>, LauncherError>(
            agora_core::installed_content::list_installed_content(
                &instance_dir,
                &manifest,
                None,
                Some(&registry),
            ),
        )
    })
    .await
    .map_err(|_| LauncherError::LocalStateFailed)??;

    let ctx = crate::core_context(&app)?;
    let cache_keys = rows.iter().map(|row| row.key.clone()).collect::<Vec<_>>();
    let cached =
        agora_core::modrinth::load_cached_project_metadata(&ctx, &cache_keys).unwrap_or_default();
    let mut project_ids = std::collections::HashSet::new();
    for row in &rows {
        let Some(project_id) = row.modrinth_id.as_ref() else {
            continue;
        };
        let cache_valid = cached
            .get(&row.key)
            .map(|entry| entry.project_id == *project_id)
            .unwrap_or(false);
        if !cache_valid {
            project_ids.insert(project_id.clone());
        }
    }
    let fetched = agora_core::modrinth::ModrinthService::new(ctx.clone())
        .fetch_project_metadata(&project_ids.into_iter().collect::<Vec<_>>())
        .await
        .unwrap_or_default();
    let cache_entries = rows
        .iter()
        .filter_map(|row| {
            let project_id = row.modrinth_id.as_ref()?;
            let metadata = fetched.get(project_id)?.clone();
            Some((row.key.clone(), project_id.clone(), metadata))
        })
        .collect::<Vec<_>>();
    let _ = agora_core::modrinth::store_cached_project_metadata(&ctx, &cache_entries);

    Ok(rows
        .into_iter()
        .map(|row| {
            let cache_key = row.key.clone();
            InstalledContentMetadata {
                key: cache_key.clone(),
                display_name: row.modrinth_id.as_ref().and_then(|id| {
                    cached
                        .get(&cache_key)
                        .filter(|entry| entry.project_id == *id)
                        .map(|entry| entry.title.clone())
                        .or_else(|| fetched.get(id).map(|item| item.title.clone()))
                }),
                icon_url: row.modrinth_id.as_ref().and_then(|id| {
                    cached
                        .get(&cache_key)
                        .filter(|entry| entry.project_id == *id)
                        .and_then(|entry| entry.icon_url.clone())
                        .or_else(|| fetched.get(id).and_then(|item| item.icon_url.clone()))
                }),
                author: row.author.or_else(|| {
                    row.modrinth_id.as_ref().and_then(|id| {
                        cached
                            .get(&cache_key)
                            .filter(|entry| entry.project_id == *id)
                            .and_then(|entry| entry.author.clone())
                            .or_else(|| fetched.get(id).and_then(|item| item.author.clone()))
                    })
                }),
            }
        })
        .collect())
}

/// Load the instance manifest for the given instance_id.
fn load_manifest<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    instance_id: &str,
) -> LauncherResult<InstanceManifest> {
    let manifest_path = paths::instance_manifest_path(app, instance_id)
        .map_err(|_| LauncherError::InstanceCreateFailed)?;
    let text = std::fs::read_to_string(&manifest_path).map_err(|_| LauncherError::Generic {
        code: "ERR_MANIFEST_MISSING".to_string(),
        message: format!("Instance manifest not found for '{}'.", instance_id),
    })?;
    serde_json::from_str(&text).map_err(|_| LauncherError::Generic {
        code: "ERR_MANIFEST_PARSE".to_string(),
        message: "Failed to parse instance manifest.".to_string(),
    })
}

/// Investigate a crash for an instance using the auto-detected or provided
/// crash log filename. Runs the full guided-isolation pipeline.
#[tauri::command]
pub async fn investigate_crash(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
    filename: Option<String>,
) -> LauncherResult<crash_investigator::InvestigationResult> {
    tokio::task::spawn_blocking(move || {
        // Determine the crash log filename.
        let filename = match filename {
            Some(f) => f,
            None => {
                let report = crash_diagnostics::check_for_crash(&app, &instance_id)
                    .map_err(|_| LauncherError::LocalStateFailed)?;
                report
                    .ok_or_else(|| LauncherError::Generic {
                        code: "ERR_NO_CRASH_LOG".to_string(),
                        message: "No crash log detected for this instance.".to_string(),
                    })?
                    .filename
            }
        };

        // Read the crash log text.
        let crash_text =
            crash_diagnostics::read_crash_log(&app, &instance_id, &filename).map_err(|_| {
                LauncherError::Generic {
                    code: "ERR_CRASH_LOG_READ".to_string(),
                    message: "Could not read the crash log file.".to_string(),
                }
            })?;

        // Load the instance manifest for installed mods.
        let manifest = load_manifest(&app, &instance_id)?;

        // Run the investigation pipeline.
        let fingerprint = match crash_investigator::parse_crash_log(&crash_text) {
            Some(fp) => fp,
            None => {
                // Can't parse â€” return empty investigation.
                return Ok(crash_investigator::InvestigationResult {
                    fingerprint: None,
                    signature_name: None,
                    suspects: Vec::new(),
                    suggested_action: crash_investigator::SuggestedAction::NoSuspects,
                    ruled_out: Vec::new(),
                });
            }
        };

        let ctx = crate::core_context(&app)?;
        let result = crash_investigator::continue_investigation(
            &ctx,
            &fingerprint,
            &manifest.mods,
            &crash_text,
        )?;
        // Per A5 (2026-07-05 audit): feed the investigation result back into the
        // local crash telemetry (local_crash_telemetry) so the Crash Matrix signal
        // B/C data populates for future diagnostics. Skip if no suspects to avoid noise.
        if !result.suspects.is_empty() {
            let mod_ids: Vec<String> = result.suspects.iter().map(|s| s.mod_id.clone()).collect();
            let _ = crash_investigator::record_crash_event(
                &app,
                &instance_id,
                &fingerprint,
                &mod_ids,
                None, // signature_name -- callers pass curated-regex match separately when known
            );
        }
        Ok(result)
    })
    .await
    .map_err(|_| LauncherError::LocalStateFailed)?
}

/// Investigate a crash using a manually-provided crash log text.
#[tauri::command]
pub async fn investigate_manual(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
    log_text: String,
) -> LauncherResult<crash_investigator::InvestigationResult> {
    tokio::task::spawn_blocking(move || {
        let manifest = load_manifest(&app, &instance_id)?;

        let fingerprint = match crash_investigator::parse_crash_log(&log_text) {
            Some(fp) => fp,
            None => {
                return Ok(crash_investigator::InvestigationResult {
                    fingerprint: None,
                    signature_name: None,
                    suspects: Vec::new(),
                    suggested_action: crash_investigator::SuggestedAction::NoSuspects,
                    ruled_out: Vec::new(),
                });
            }
        };

        let ctx = crate::core_context(&app)?;
        crash_investigator::continue_investigation(&ctx, &fingerprint, &manifest.mods, &log_text)
    })
    .await
    .map_err(|_| LauncherError::LocalStateFailed)?
}

/// Temporarily disable a mod by renaming it to `<filename>.disabled`.
#[tauri::command]
pub async fn disable_mod_for_test(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
    filename: String,
) -> LauncherResult<()> {
    let ctx = crate::core_context(&app)?;
    tokio::task::spawn_blocking(move || {
        let svc = agora_core::crash_service::CrashService::new(ctx);
        svc.disable_mod(&instance_id, &filename)
    })
    .await
    .map_err(|_| LauncherError::LocalStateFailed)?
}

/// Re-enable a previously disabled mod (rename back).
#[tauri::command]
pub async fn enable_mod_for_test(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
    filename: String,
) -> LauncherResult<()> {
    let ctx = crate::core_context(&app)?;
    tokio::task::spawn_blocking(move || {
        let svc = agora_core::crash_service::CrashService::new(ctx);
        svc.enable_mod(&instance_id, &filename)
    })
    .await
    .map_err(|_| LauncherError::LocalStateFailed)?
}

/// Confirm that a mod was the cause of a crash (for telemetry).
#[tauri::command]
pub async fn confirm_crash_fix(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    fingerprint: crash_investigator::CrashFingerprint,
    mod_id: String,
) -> LauncherResult<()> {
    tokio::task::spawn_blocking(move || {
        crash_investigator::confirm_attribution(&app, &fingerprint, &mod_id)
    })
    .await
    .map_err(|_| LauncherError::LocalStateFailed)?
}

/// Report that the crash persists after disabling the top suspect.
/// Rules out the mod and re-runs the investigation to find the next suspect.
#[tauri::command]
pub async fn report_still_crashing(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
    fingerprint: crash_investigator::CrashFingerprint,
    ruled_out_mod_id: String,
    crash_log_text: String,
) -> LauncherResult<crash_investigator::InvestigationResult> {
    tokio::task::spawn_blocking(move || {
        // Rule out the mod.
        crash_investigator::rule_out(&app, &fingerprint, &ruled_out_mod_id)
            .map_err(|_| LauncherError::LocalStateFailed)?;

        let ctx = crate::core_context(&app)?;

        // Reload the manifest and re-investigate.
        let manifest = load_manifest(&app, &instance_id)?;

        crash_investigator::continue_investigation(
            &ctx,
            &fingerprint,
            &manifest.mods,
            &crash_log_text,
        )
    })
    .await
    .map_err(|_| LauncherError::LocalStateFailed)?
}

/// Read the whole installed-content dependency graph for an instance.
///
/// Read-only, and one call rather than one per mod — a UI that draws
/// relationships needs every edge at once, and `get_disable_plan` answers only
/// "who needs THIS one". Edges are filename-to-filename because that is the
/// identifier the content list already carries.
#[tauri::command]
pub async fn get_dependency_graph(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
) -> LauncherResult<Vec<dependency_ops::DependencyEdge>> {
    tokio::task::spawn_blocking(move || {
        let mut manifest = load_manifest(&app, &instance_id)?;
        dependency_ops::refresh_installed_jar_metadata(&app, &instance_id, &mut manifest.mods)?;
        Ok(dependency_ops::build_dependency_graph(&manifest.mods))
    })
    .await
    .map_err(|_| LauncherError::LocalStateFailed)?
}

/// Build a disable plan for a mod: which other installed mods would be affected
/// if this mod is disabled (renamed to `.disabled`).
#[tauri::command]
pub async fn get_disable_plan(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
    filename: String,
) -> LauncherResult<dependency_ops::DisablePlan> {
    tokio::task::spawn_blocking(move || {
        let mut manifest = load_manifest(&app, &instance_id)?;
        dependency_ops::refresh_installed_jar_metadata(&app, &instance_id, &mut manifest.mods)?;
        let target = manifest
            .mods
            .iter()
            .find(|m| m.filename == filename)
            .ok_or_else(|| LauncherError::Generic {
                code: "ERR_MOD_NOT_FOUND".to_string(),
                message: format!("Mod '{}' not found in instance manifest.", filename),
            })?
            .clone();
        Ok(dependency_ops::build_disable_plan(&manifest.mods, &target))
    })
    .await
    .map_err(|_| LauncherError::LocalStateFailed)?
}

/// Build a removal plan for a mod: which other installed mods would break if
/// this mod is removed entirely.
#[tauri::command]
pub async fn get_removal_plan(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
    filename: String,
) -> LauncherResult<dependency_ops::RemovalPlan> {
    tokio::task::spawn_blocking(move || {
        let mut manifest = load_manifest(&app, &instance_id)?;
        dependency_ops::refresh_installed_jar_metadata(&app, &instance_id, &mut manifest.mods)?;
        let target = manifest
            .mods
            .iter()
            .chain(manifest.resourcepacks.iter())
            .chain(manifest.shaders.iter())
            .chain(manifest.datapacks.iter())
            .chain(manifest.worlds.iter())
            .find(|m| m.filename == filename)
            .ok_or_else(|| LauncherError::Generic {
                code: "ERR_MOD_NOT_FOUND".to_string(),
                message: format!("'{}' not found in instance manifest.", filename),
            })?
            .clone();
        Ok(dependency_ops::build_removal_plan(&manifest.mods, &target))
    })
    .await
    .map_err(|_| LauncherError::LocalStateFailed)?
}

/// Build an install plan for a target mod: which dependencies are missing,
/// which are optional, and whether there are any conflicts between jar and
/// manifest declarations.
#[tauri::command]
pub async fn get_install_plan(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
    item_id: String,
    jar_path: String,
) -> LauncherResult<dependency_ops::InstallPlan> {
    let ctx = crate::core_context(&app)?;
    let svc = agora_core::registry::RegistryService::new(ctx);

    tokio::task::spawn_blocking(move || {
        // Fetch the target mod's manifest-declared dependencies from the registry.
        let manifest_deps = svc.get_manifest_dependencies(&item_id)?;

        // Load the target instance's installed mods to determine which deps are missing.
        let mut manifest = load_manifest(&app, &instance_id)?;
        // Parse only the metadata for the instance's active loader.
        let jar_metadata = agora_core::jar_metadata::parse_jar_metadata_for_loader(
            std::path::Path::new(&jar_path),
            &manifest.loader,
        );
        dependency_ops::refresh_installed_jar_metadata(&app, &instance_id, &mut manifest.mods)?;

        let aliases = svc.get_all_mod_aliases()?;
        Ok(dependency_ops::build_install_plan_with_aliases(
            manifest_deps,
            &jar_metadata,
            &manifest.mods,
            &dependency_ops::AliasMap::from_pairs(&aliases),
        ))
    })
    .await
    .map_err(|_| LauncherError::LocalStateFailed)?
}

/// Enable a mod by renaming `<filename>.disabled` â†’ `<filename>` and
/// auto-re-enable any previously-disabled required dependencies.
///
/// Returns the list of filenames that were auto-enabled (toast messages).
/// Best-effort: individual enable failures are logged but do not abort the
/// entire operation.
#[tauri::command]
pub async fn enable_mod_with_auto_deps(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
    filename: String,
) -> LauncherResult<Vec<String>> {
    tokio::task::spawn_blocking(move || {
        let mut manifest = load_manifest(&app, &instance_id)?;
        dependency_ops::refresh_installed_jar_metadata(&app, &instance_id, &mut manifest.mods)?;

        let target = manifest
            .mods
            .iter()
            .find(|m| m.filename == filename)
            .ok_or_else(|| LauncherError::Generic {
                code: "ERR_MOD_NOT_FOUND".to_string(),
                message: format!("Mod '{}' not found in instance manifest.", filename),
            })?;

        let mut auto_enabled: Vec<String> = Vec::new();

        // Resolve the target mod's required deps from jar metadata.
        let depends_on = match &target.mod_jar_id {
            Some(_) => &target.depends_on,
            None => &Vec::new(),
        };

        // For each required dep, find the corresponding installed mod and check
        // if it's disabled (`.disabled` file exists). If so, enable it.
        for dep_jar_id in depends_on {
            let dep_lower = dep_jar_id.to_lowercase();

            // Find the physical JAR whose primary or dynamically provided ID
            // matches this dependency.
            let dep_mod = manifest.mods.iter().find(|m| {
                m.mod_jar_id
                    .iter()
                    .chain(m.provided_mod_ids.iter())
                    .any(|jid| jid.to_lowercase() == dep_lower)
            });

            let dep_mod = match dep_mod {
                Some(m) => m,
                None => continue, // Missing entirely â€” skip silently (can't auto-install).
            };

            // Check if the dep's jar file is disabled.
            let mods_dir = paths::instance_dir(&app, &instance_id)
                .map_err(|_| LauncherError::InstanceCreateFailed)?
                .join("mods");
            let disabled_path = mods_dir.join(format!("{}.disabled", dep_mod.filename));

            if !disabled_path.exists() {
                continue; // Already enabled.
            }

            // Best-effort enable: continue past individual failures.
            if let Err(e) = crash_investigator::enable_mod(&app, &instance_id, &dep_mod.filename) {
                crate::auth::log_line(&format!(
                    "enable_mod_with_auto_deps: failed to enable dep '{}': {}",
                    dep_mod.filename, e
                ));
                continue;
            }

            auto_enabled.push(dep_mod.filename.clone());
        }

        // Now enable the target mod itself.
        if let Err(e) = crash_investigator::enable_mod(&app, &instance_id, &filename) {
            crate::auth::log_line(&format!(
                "enable_mod_with_auto_deps: failed to enable target '{}': {}",
                filename, e
            ));
            // Still return the auto-enabled deps we managed; the target failure
            // is surfaced via the Err path below.
            return Err(LauncherError::Generic {
                code: "ERR_ENABLE_FAILED".to_string(),
                message: format!("Failed to enable '{}': {}", filename, e),
            });
        }

        Ok(auto_enabled)
    })
    .await
    .map_err(|_| LauncherError::LocalStateFailed)?
}

/// Start the MCP server if not already running.
/// Checks the `ai_mcp_enabled` setting and delegates lifecycle ownership to
/// the permanent MCP manager state.
/// Returns the server URL.
#[tauri::command]
pub async fn start_mcp_server(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
) -> LauncherResult<McpStatus> {
    let ctx = crate::core_context(&app)?;
    let svc = agora_core::settings::SettingsService::new(ctx);
    let enabled = svc.get_bool("ai_mcp_enabled").unwrap_or(false);
    if !enabled {
        return Ok(McpStatus {
            running: false,
            url: String::new(),
        });
    }

    let manager = app.state::<mcp::McpServerManager>();
    let port = manager
        .start(app.clone())
        .await
        .map_err(|e| LauncherError::Generic {
            code: "ERR_MCP_START_FAILED".to_string(),
            message: format!("Failed to start MCP server: {e}"),
        })?;
    Ok(McpStatus {
        running: true,
        url: format!("http://127.0.0.1:{port}"),
    })
}

/// Stop the MCP server if running.
#[tauri::command]
pub async fn stop_mcp_server(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
) -> LauncherResult<()> {
    app.state::<mcp::McpServerManager>().stop().await;
    Ok(())
}

/// Return the current MCP server status.
#[tauri::command]
pub async fn get_mcp_status(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
) -> LauncherResult<McpStatus> {
    if let Some(port) = app.state::<mcp::McpServerManager>().port().await {
        return Ok(McpStatus {
            running: true,
            url: format!("http://127.0.0.1:{port}"),
        });
    }
    Ok(McpStatus {
        running: false,
        url: String::new(),
    })
}

/// Return the baked-in MCP skill guide content.
#[tauri::command]
pub fn get_mcp_skill_content() -> String {
    crate::mcp::MCP_SKILL_CONTENT.to_string()
}

/// Return the current MCP Bearer token and a ready-to-paste AI client config
/// snippet.  Returns `""` when the MCP server has never been started.
#[tauri::command]
pub async fn get_mcp_token(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
) -> LauncherResult<serde_json::Value> {
    tokio::task::spawn_blocking(move || {
        let ctx = crate::core_context(&app)?;
        let svc = agora_core::settings::SettingsService::new(ctx);
        match svc.get("mcp_bearer_token")? {
            Some(v) => {
                let token = v.as_str().unwrap_or("").to_string();
                Ok(serde_json::json!({
                    "token": token,
                    "config_snippet": format!(
                        r#"{{"mcpServers":{{"agora":{{"url":"http://127.0.0.1:39741/sse","headers":{{"Authorization":"Bearer {}"}}}}}}}}"#,
                        token
                    ),
                }))
            }
            None => Ok(serde_json::json!({"token": "", "config_snippet": ""})),
        }
    }).await.map_err(|_| LauncherError::LocalStateFailed)?
}

/// Generate a fresh MCP Bearer token, persist it, and return it (invalidates
/// any prior token).
#[tauri::command]
pub async fn regenerate_mcp_token(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
) -> LauncherResult<serde_json::Value> {
    tokio::task::spawn_blocking(move || {
        use rand::Rng;
        let bytes: [u8; 32] = rand::thread_rng().gen();
        let token = hex::encode(bytes);
        let ctx = crate::core_context(&app)?;
        let svc = agora_core::settings::SettingsService::new(ctx);
        svc.set("mcp_bearer_token", &serde_json::Value::String(token.clone()))?;
        // Write the token file
        if let Ok(app_data) = paths::app_data_dir(&app) {
            let path = app_data.join("mcp_token");
            if let Ok(mut f) = std::fs::File::create(&path) {
                let _ = std::io::Write::write_all(&mut f, token.as_bytes());
            }
        }
        Ok(serde_json::json!({
            "token": token,
            "config_snippet": format!(
                r#"{{"mcpServers":{{"agora":{{"url":"http://127.0.0.1:39741/sse","headers":{{"Authorization":"Bearer {}"}}}}}}}}"#,
                token
            ),
        }))
    }).await.map_err(|_| LauncherError::LocalStateFailed)?
}

/// Record a user approval grant for an MCP tool + instance pair.
/// `state` is one of: "always_allow", "always_deny", "session".
#[tauri::command]
pub async fn set_mcp_approval(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    tool_name: String,
    instance_id: String,
    state: String,
) -> LauncherResult<()> {
    let ctx = crate::core_context(&app).map_err(|_| LauncherError::LocalStateFailed)?;
    tokio::task::spawn_blocking(move || {
        agora_core::mcp_dispatcher::set_approval_grant(&ctx, &tool_name, &instance_id, &state)
    })
    .await
    .map_err(|_| LauncherError::LocalStateFailed)?
}

/// Start the GitHub Copilot device code flow.
#[tauri::command]
pub async fn copilot_login(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
) -> LauncherResult<ai_assistant::CopilotDeviceFlowResponse> {
    let ctx = crate::core_context(&app)?;
    ai_assistant::start_copilot_flow(&ctx.http_clients).await
}

/// Try to use the existing governance GitHub token for Copilot, skipping the
/// device flow if the token is valid and the user has a Copilot subscription.
/// Returns `Some(CopilotToken)` on success, or `None` if the user needs to
/// go through the device flow instead.
#[tauri::command]
pub async fn copilot_try_governance_token(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
) -> LauncherResult<Option<ai_assistant::CopilotToken>> {
    let ghu_token = match crate::auth::get_valid_access_token(&app).await {
        Some(t) => t,
        None => return Ok(None),
    };
    let ctx = crate::core_context(&app)?;
    match ai_assistant::resolve_copilot_endpoint(&ctx.http_clients, &ghu_token).await {
        Ok(copilot_token) => {
            ai_assistant::store_copilot_token(&copilot_token)?;
            Ok(Some(copilot_token))
        }
        Err(_) => {
            // Token either doesn't have a Copilot subscription or belongs to a
            // different OAuth app — fall through to the device flow.
            Ok(None)
        }
    }
}

/// Poll the Copilot device flow. On success, resolves endpoint + stores token.
#[tauri::command]
pub async fn copilot_login_poll(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    device_code: String,
    interval: u64,
) -> LauncherResult<ai_assistant::CopilotToken> {
    let ctx = crate::core_context(&app)?;
    let ghu_token =
        ai_assistant::poll_copilot_flow(&ctx.http_clients, &device_code, interval).await?;
    let copilot_token =
        ai_assistant::resolve_copilot_endpoint(&ctx.http_clients, &ghu_token).await?;
    ai_assistant::store_copilot_token(&copilot_token)?;
    Ok(copilot_token)
}

/// Check if Copilot is connected and the token is still valid.
#[tauri::command]
pub async fn copilot_status(
    _app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
) -> LauncherResult<Option<ai_assistant::CopilotToken>> {
    ai_assistant::load_copilot_token()
}

/// Sign out of Copilot.
#[tauri::command]
pub async fn copilot_logout(
    _app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
) -> LauncherResult<()> {
    ai_assistant::clear_copilot_token()
}

/// Send a chat message to the AI assistant and return the response.
///
/// If `context` is provided and the messages don't already contain a context
/// message, one is prepended. A system prompt is always inserted as the first
/// message.
#[tauri::command]
pub async fn ai_chat(
    app: tauri::AppHandle,
    messages: Vec<ChatMessage>,
    context: Option<serde_json::Value>,
) -> Result<ChatResponse, LauncherError> {
    // Respect the AI chat setting — when disabled, no AI calls should be made.
    {
        let ctx = crate::core_context(&app)?;
        let enabled = agora_core::settings::SettingsService::new(ctx)
            .get_bool("ai_chat_enabled")
            .unwrap_or(false);
        if !enabled {
            return Err(LauncherError::Generic {
                code: "ERR_AI_DISABLED".into(),
                message: "AI Assistant is disabled in Settings → AI & automation.".into(),
            });
        }
    }
    let token = ai_assistant::load_copilot_token()?
        .ok_or_else(|| LauncherError::Generic {
            code: "ERR_AI_NOT_AUTHENTICATED".to_string(),
            message: "GitHub Copilot is not connected. Click 'Connect with GitHub' in the chat panel to set up free AI diagnostics (50 requests/month).".to_string(),
        })?;

    let mut messages = messages;

    // Build context message if context JSON is provided and not already present.
    if let Some(ctx_val) = &context {
        let has_context = messages.iter().any(|m| {
            m.role == "system"
                || (m.role == "user"
                    && (m.content.contains("## Crash Log")
                        || m.content.contains("## Ranked Suspect Mods")
                        || m.content.contains("## Curated Crash Signatures")))
        });
        if !has_context {
            let instance_id = ctx_val
                .get("instance_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let crash_log = ctx_val
                .get("crash_log")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let crash_signatures = ctx_val
                .get("crash_signatures")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let suspects = ctx_val
                .get("suspects")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let ctx = ai_assistant::AiContext {
                instance_id,
                crash_log,
                crash_signatures,
                suspects,
            };
            let context_text = ai_assistant::build_context_message(&ctx);
            messages.insert(
                0,
                ChatMessage {
                    role: "user".to_string(),
                    content: context_text,
                },
            );
        }
    }

    // Ensure system prompt is first.
    if messages.is_empty() || messages[0].role != "system" {
        messages.insert(
            0,
            ChatMessage {
                role: "system".to_string(),
                content: ai_assistant::build_system_prompt(),
            },
        );
    }

    ai_assistant::chat_completion(messages, &token).await
}

/// Get an AI explanation for a detected crash.
#[tauri::command]
pub async fn explain_crash(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
    crash_log: String,
) -> Result<String, LauncherError> {
    {
        let ctx = crate::core_context(&app)?;
        let enabled = agora_core::settings::SettingsService::new(ctx)
            .get_bool("ai_chat_enabled")
            .unwrap_or(false);
        if !enabled {
            return Err(LauncherError::Generic {
                code: "ERR_AI_DISABLED".into(),
                message: "AI Assistant is disabled in Settings → AI & automation.".into(),
            });
        }
    }
    let token = ai_assistant::load_copilot_token()?.ok_or_else(|| LauncherError::Generic {
        code: "ERR_AI_NOT_AUTHENTICATED".into(),
        message: "GitHub Copilot is not connected. Click 'Connect with GitHub' in the chat panel."
            .into(),
    })?;

    let context = ai_assistant::AiContext {
        instance_id: Some(instance_id),
        crash_log: Some(crash_log),
        crash_signatures: None,
        suspects: None,
    };
    let system = ai_assistant::build_system_prompt();
    let context_msg = ai_assistant::build_context_message(&context);

    let messages = vec![
        ChatMessage {
            role: "system".into(),
            content: system,
        },
        ChatMessage {
            role: "user".into(),
            content: context_msg,
        },
    ];

    let response = ai_assistant::chat_completion(messages, &token).await?;
    Ok(response.content)
}

// ---------------------------------------------------------------------------
// Phase 5: MSA auth + GC architect
// ---------------------------------------------------------------------------

async fn capture_msa_callback(
    app: tauri::AppHandle,
    auth_uri: &str,
) -> LauncherResult<(String, String)> {
    let auth_url: tauri::Url = auth_uri.parse().map_err(|e| LauncherError::Generic {
        code: "ERR_MSA_AUTH_URL".into(),
        message: format!("Microsoft returned an invalid sign-in URL: {e}"),
    })?;

    if let Some(existing) = app.get_webview_window("msa-login") {
        let _ = existing.destroy();
    }

    let (sender, receiver) = tokio::sync::oneshot::channel::<Result<(String, String), String>>();
    let sender = Arc::new(Mutex::new(Some(sender)));
    let navigation_sender = Arc::clone(&sender);
    let close_sender = Arc::clone(&sender);
    let close_app = app.clone();

    let auth_window =
        tauri::WebviewWindowBuilder::new(&app, "msa-login", tauri::WebviewUrl::External(auth_url))
            .title("Sign in to Microsoft")
            .inner_size(520.0, 720.0)
            .center()
            .on_navigation(move |url| {
                let is_callback = url.scheme() == "https"
                    && url.host_str() == Some(MSA_AUTH_REPLY_HOST)
                    && url.path() == MSA_AUTH_REPLY_PATH;
                if !is_callback {
                    return true;
                }

                let query: std::collections::HashMap<_, _> =
                    url.query_pairs().into_owned().collect();
                let result = match (query.get("code").cloned(), query.get("state").cloned()) {
                    (Some(code), Some(state)) => Ok((code, state)),
                    _ => Err(query
                        .get("error_description")
                        .cloned()
                        .or_else(|| query.get("error").cloned())
                        .unwrap_or_else(|| "Microsoft returned no authorization code.".into())),
                };

                if let Ok(mut guard) = navigation_sender.lock() {
                    if let Some(sender) = guard.take() {
                        let _ = sender.send(result);
                    }
                }
                if let Some(window) = close_app.get_webview_window("msa-login") {
                    let _ = window.destroy();
                }
                false
            })
            .build()
            .map_err(|e| LauncherError::Generic {
                code: "ERR_MSA_WINDOW".into(),
                message: format!("Could not open Microsoft sign-in window: {e}"),
            })?;

    auth_window.on_window_event(move |event| {
        if matches!(
            event,
            tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed
        ) {
            if let Ok(mut guard) = close_sender.lock() {
                if let Some(sender) = guard.take() {
                    let _ = sender.send(Err(
                        "The Microsoft sign-in window was closed before authentication completed."
                            .into(),
                    ));
                }
            }
        }
    });

    receiver
        .await
        .map_err(|_| LauncherError::Generic {
            code: "ERR_MSA_WINDOW_CLOSED".into(),
            message: "The Microsoft sign-in window closed unexpectedly.".into(),
        })?
        .map_err(|message| LauncherError::Generic {
            code: "ERR_MSA_LOGIN_CANCELLED".into(),
            message,
        })
}

/// Run the complete Microsoft Account login flow in a dedicated OAuth window.
/// The callback is intercepted before Microsoft sanitizes its query string.
#[tauri::command]
pub async fn msa_login(
    app: tauri::AppHandle,
    state: tauri::State<'_, LauncherState>,
) -> LauncherResult<MsaAccountStatus> {
    let db_path = crate::paths::local_state_db_path(&app).map_err(|e| LauncherError::Generic {
        code: "ERR_DB".into(),
        message: e.to_string(),
    })?;
    let client = { state.lock().await.client.clone() };
    let flow = agora_core::msa::begin_login(&client, &db_path).await?;
    let (code, oauth_state) = capture_msa_callback(app, &flow.auth_uri).await?;
    let creds =
        agora_core::msa::finish_login(&client, &code, &flow, Some(&oauth_state), &db_path).await?;
    Ok(MsaAccountStatus::from(&creds))
}

/// Return the current MSA login status, or None if not authenticated.
#[tauri::command]
pub async fn msa_get_status(
    _app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
) -> LauncherResult<Option<MsaAccountStatus>> {
    Ok(agora_core::msa::load_credentials()?
        .as_ref()
        .map(MsaAccountStatus::from))
}
/// Refresh expired MSA credentials.
#[tauri::command]
pub async fn msa_refresh(
    app: tauri::AppHandle,
    state: tauri::State<'_, LauncherState>,
) -> LauncherResult<MsaAccountStatus> {
    let db_path = crate::paths::local_state_db_path(&app).map_err(|e| LauncherError::Generic {
        code: "ERR_DB".into(),
        message: e.to_string(),
    })?;
    let s = state.lock().await;
    let creds = agora_core::msa::load_credentials()?.ok_or_else(|| LauncherError::Generic {
        code: "ERR_MSA_NOT_AUTHENTICATED".into(),
        message: "Not signed in. Sign in with your Microsoft account first.".into(),
    })?;
    let refreshed = agora_core::msa::refresh_credentials(&s.client, &creds, &db_path).await?;
    Ok(MsaAccountStatus::from(&refreshed))
}

/// Sign out and clear stored MSA credentials.
#[tauri::command]
pub async fn msa_logout(
    _app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
) -> LauncherResult<()> {
    agora_core::msa::clear_credentials()
}

/// Compute the JVM preview using the same GC and pre-touch rules as launch.
#[tauri::command]
pub fn compute_gc_args(
    _state: tauri::State<'_, LauncherState>,
    java_version: u32,
    requested_heap_mb: i64,
    manual_args: String,
    gc_mode: String,
    always_pre_touch: bool,
) -> agora_core::gc::GcResult {
    let override_profile = match gc_mode.trim().to_ascii_lowercase().as_str() {
        "high_efficiency" => Some(agora_core::gc::GcProfile::HighEfficiency),
        "low_latency" => Some(agora_core::gc::GcProfile::LowLatency),
        "manual" => Some(agora_core::gc::GcProfile::Manual),
        _ => None,
    };
    agora_core::gc::compute_gc_with_pre_touch(
        java_version,
        requested_heap_mb,
        &manual_args,
        override_profile,
        Some(always_pre_touch),
    )
}

// ---------------------------------------------------------------------------
// Phase 6: Instance lifecycle — snapshots, loadouts, import, clone, export
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Serialize)]
pub struct SnapshotView {
    #[serde(flatten)]
    pub snapshot: agora_core::snapshot::Snapshot,
    pub is_lkg: bool,
    pub is_current_lkg: bool,
    pub is_pre_restore: bool,
}

#[tauri::command]
pub async fn list_snapshots(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
) -> LauncherResult<Vec<SnapshotView>> {
    let sanitized = paths::sanitize_id(&instance_id);
    let instance_dir =
        paths::instance_dir(&app, &sanitized).map_err(|e| LauncherError::Generic {
            code: "ERR_PATH".into(),
            message: e.to_string(),
        })?;
    tokio::task::spawn_blocking(move || {
        let snapshots = agora_core::snapshot::list_snapshots(&instance_dir)?;
        let lkg = agora_core::lkg::read_lkg_state(&instance_dir)?;
        Ok::<_, String>(
            snapshots
                .into_iter()
                .map(|snapshot| {
                    let is_current_lkg = lkg.current_lkg_snapshot_id.as_ref() == Some(&snapshot.id);
                    let is_lkg = is_current_lkg || lkg.promoted_snapshot_ids.contains(&snapshot.id);
                    let is_pre_restore = snapshot
                        .label
                        .as_deref()
                        .is_some_and(|label| label.starts_with("pre-restore-"));
                    SnapshotView {
                        snapshot,
                        is_lkg,
                        is_current_lkg,
                        is_pre_restore,
                    }
                })
                .collect(),
        )
    })
    .await
    .map_err(|e| LauncherError::Generic {
        code: "ERR_SNAPSHOT_TASK".into(),
        message: format!("Snapshot listing task failed: {e}"),
    })?
    .map_err(|e| LauncherError::Generic {
        code: "ERR_SNAPSHOT".into(),
        message: e,
    })
}

#[tauri::command]
pub async fn create_snapshot(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
    label: Option<String>,
) -> LauncherResult<agora_core::snapshot::Snapshot> {
    let sanitized = paths::sanitize_id(&instance_id);
    let instance_dir =
        paths::instance_dir(&app, &sanitized).map_err(|e| LauncherError::Generic {
            code: "ERR_PATH".into(),
            message: e.to_string(),
        })?;

    let ctx = crate::core_context(&app)?;
    ctx.task_scheduler
        .run_blocking(
            agora_core::task_scheduler::BlockingPriority::UserInitiated,
            move || {
                let result =
                    agora_core::snapshot::create_snapshot(&instance_dir, label.as_deref())?;
                agora_core::lkg::run_retention(&instance_dir)?;
                Ok::<_, String>(result)
            },
        )
        .await
        .map_err(|e| LauncherError::Generic {
            code: "ERR_SNAPSHOT_TASK".into(),
            message: format!("Snapshot creation task failed: {e}"),
        })?
        .map_err(|e| LauncherError::Generic {
            code: "ERR_SNAPSHOT".into(),
            message: e,
        })
}

#[tauri::command]
pub async fn restore_snapshot(
    app: tauri::AppHandle,
    state: tauri::State<'_, LauncherState>,
    instance_id: String,
    snapshot_id: String,
) -> LauncherResult<()> {
    let sanitized = paths::sanitize_id(&instance_id);
    let instance_dir =
        paths::instance_dir(&app, &sanitized).map_err(|e| LauncherError::Generic {
            code: "ERR_PATH".into(),
            message: e.to_string(),
        })?;

    {
        let shared = state.lock().await;
        // Any session of this instance blocks a restore, not just the first.
        let direct_active = shared
            .running_processes
            .values()
            .any(|process| process.instance_id == sanitized);
        let launch_active = shared.launch_reservations.contains(&sanitized);
        if direct_active || launch_active {
            return Err(LauncherError::Generic {
                code: "ERR_INSTANCE_RUNNING".into(),
                message: "Stop the running game before restoring this instance.".into(),
            });
        }
    }

    tokio::task::spawn_blocking(move || {
        let pre_label = format!("pre-restore-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S"));
        agora_core::snapshot::create_snapshot(&instance_dir, Some(&pre_label))
            .map_err(|e| format!("Could not create undo snapshot: {e}"))?;
        agora_core::snapshot::restore_snapshot(&instance_dir, &snapshot_id)?;
        agora_core::lkg::run_retention(&instance_dir)?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| LauncherError::Generic {
        code: "ERR_RESTORE_TASK".into(),
        message: format!("Restore task failed: {e}"),
    })?
    .map_err(|e| LauncherError::Generic {
        code: "ERR_RESTORE".into(),
        message: e,
    })
}

#[tauri::command]
pub async fn delete_snapshot(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
    snapshot_id: String,
) -> LauncherResult<()> {
    let sanitized = paths::sanitize_id(&instance_id);
    let instance_dir =
        paths::instance_dir(&app, &sanitized).map_err(|e| LauncherError::Generic {
            code: "ERR_PATH".into(),
            message: e.to_string(),
        })?;
    tokio::task::spawn_blocking(move || {
        agora_core::snapshot::delete_snapshot(&instance_dir, &snapshot_id)?;
        agora_core::lkg::run_retention(&instance_dir)
    })
    .await
    .map_err(|e| LauncherError::Generic {
        code: "ERR_SNAPSHOT_TASK".into(),
        message: format!("Snapshot deletion task failed: {e}"),
    })?
    .map_err(|e| LauncherError::Generic {
        code: "ERR_SNAPSHOT".into(),
        message: e,
    })
}

#[tauri::command]
pub async fn list_loadout_profiles(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
) -> LauncherResult<Vec<agora_core::loadout::LoadoutProfile>> {
    let sanitized = paths::sanitize_id(&instance_id);
    let instance_dir =
        paths::instance_dir(&app, &sanitized).map_err(|e| LauncherError::Generic {
            code: "ERR_PATH".into(),
            message: e.to_string(),
        })?;
    agora_core::loadout::list_profiles(&instance_dir).map_err(|e| LauncherError::Generic {
        code: "ERR_LOADOUT".into(),
        message: e,
    })
}

#[tauri::command]
pub async fn create_loadout_profile(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
    name: String,
) -> LauncherResult<agora_core::loadout::LoadoutProfile> {
    let sanitized = paths::sanitize_id(&instance_id);
    let instance_dir =
        paths::instance_dir(&app, &sanitized).map_err(|e| LauncherError::Generic {
            code: "ERR_PATH".into(),
            message: e.to_string(),
        })?;
    agora_core::loadout::create_profile(&instance_dir, &name).map_err(|e| LauncherError::Generic {
        code: "ERR_LOADOUT".into(),
        message: e,
    })
}

#[tauri::command]
pub async fn apply_loadout_profile(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
    profile_name: String,
) -> LauncherResult<()> {
    check_not_locked(&app, &instance_id)?;
    let sanitized = paths::sanitize_id(&instance_id);
    let instance_dir =
        paths::instance_dir(&app, &sanitized).map_err(|e| LauncherError::Generic {
            code: "ERR_PATH".into(),
            message: e.to_string(),
        })?;
    tokio::task::spawn_blocking(move || {
        let snapshot = agora_core::snapshot::create_snapshot(&instance_dir, Some("before-loadout"))
            .map_err(|error| LauncherError::Generic {
                code: "ERR_SNAPSHOT_REQUIRED".into(),
                message: format!("Could not create the required recovery snapshot: {error}"),
            })?;
        if let Err(error) = agora_core::loadout::apply_profile(&instance_dir, &profile_name) {
            let restored = agora_core::snapshot::restore_snapshot(&instance_dir, &snapshot.id);
            return Err(LauncherError::Generic {
                code: "ERR_LOADOUT".into(),
                message: match restored {
                    Ok(()) => format!("Loadout application failed and was rolled back: {error}"),
                    Err(restore_error) => format!(
                        "Loadout application failed and rollback also failed: {error}; {restore_error}"
                    ),
                },
            });
        }
        agora_core::lkg::run_retention(&instance_dir).map_err(|error| LauncherError::Generic {
            code: "ERR_RETENTION".into(),
            message: error,
        })
    })
    .await
    .map_err(|_| LauncherError::LocalStateFailed)?
}

#[tauri::command]
pub async fn delete_loadout_profile(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
    profile_name: String,
) -> LauncherResult<()> {
    let sanitized = paths::sanitize_id(&instance_id);
    let instance_dir =
        paths::instance_dir(&app, &sanitized).map_err(|e| LauncherError::Generic {
            code: "ERR_PATH".into(),
            message: e.to_string(),
        })?;
    agora_core::loadout::delete_profile(&instance_dir, &profile_name).map_err(|e| {
        LauncherError::Generic {
            code: "ERR_LOADOUT".into(),
            message: e,
        }
    })
}

#[tauri::command]
pub async fn import_instance(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    source_path: String,
    symlink_saves: bool,
) -> LauncherResult<agora_core::import::ImportResult> {
    let ctx = crate::core_context(&app)?;
    let source = std::path::PathBuf::from(&source_path);
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase);
    let import_source = match extension.as_deref() {
        Some("mrpack") => agora_core::import_service::ImportSource::mrpack(source),
        Some("zip") => agora_core::import_service::ImportSource::PrismZip(source),
        _ => agora_core::import_service::ImportSource::Directory(source),
    };
    let request = agora_core::import_service::ImportRequest {
        source: import_source,
        symlink_saves,
    };
    let svc = agora_core::import_service::ImportService::new(ctx);
    let sink = std::sync::Arc::new(TauriCoreProgressSink {
        app,
        event_name: "operation-progress",
    });
    svc.run_import_with_sink(
        request,
        sink,
        agora_core::event_sink::CancellationToken::new(),
    )
    .await
}

struct TauriCoreProgressSink {
    app: tauri::AppHandle,
    event_name: &'static str,
}

impl agora_core::event_sink::ProgressSink for TauriCoreProgressSink {
    fn report(&self, event: agora_core::event_sink::ProgressEvent) {
        use tauri::Emitter;
        let _ = self.app.emit(self.event_name, event);
    }
}

#[tauri::command]
pub fn cancel_operation(app: tauri::AppHandle, operation_id: String) -> LauncherResult<bool> {
    let ctx = crate::core_context(&app)?;
    Ok(ctx
        .operation_manager
        .cancel(&agora_core::event_sink::OperationId::new(operation_id)))
}

#[tauri::command]
pub fn detect_launchers(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
) -> LauncherResult<Vec<agora_core::import::DetectedLauncher>> {
    let ctx = crate::core_context(&app)?;
    Ok(agora_core::import_service::ImportService::new(ctx).auto_detect_launchers())
}

#[tauri::command]
pub async fn clone_instance_cmd(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
    new_name: String,
    prefs: agora_core::clone::ClonePrefs,
) -> LauncherResult<String> {
    let request = agora_core::instance_service::CloneRequest {
        source_instance_id: instance_id,
        new_name,
        prefs,
    };
    let ctx = crate::core_context(&app)?;
    let service = agora_core::instance_service::InstanceService::new(ctx);
    let row = service.clone(request).await?;
    Ok(row.instance_id)
}

/// Export an instance as a server environment — filters client-only mods,
/// downloads server loader, writes start scripts.
#[tauri::command]
pub async fn export_server_environment(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
    dest_path: String,
) -> LauncherResult<agora_core::server_export::ExportResult> {
    let sanitized = paths::sanitize_id(&instance_id);
    let instance_dir =
        paths::instance_dir(&app, &sanitized).map_err(|e| LauncherError::Generic {
            code: "ERR_PATH".into(),
            message: e.to_string(),
        })?;
    let manifest = load_manifest(&app, &sanitized)?;
    let dest = std::path::PathBuf::from(&dest_path);
    std::fs::create_dir_all(&dest).ok();
    agora_core::server_export::export_server_environment(
        &instance_dir,
        &dest,
        &manifest.loader,
        &manifest.minecraft_version,
    )
    .map_err(|e| LauncherError::Generic {
        code: "ERR_EXPORT".into(),
        message: e.to_string(),
    })
}

/// Install a pack (Tier 1 or Tier 2) from a JSON manifest.
///
/// Delegates to core-owned [`agora_core::import_service::ImportService`].
#[tauri::command]
pub async fn install_pack(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    manifest_json: String,
    instance_id: String,
) -> LauncherResult<agora_core::pack_install::PackInstallResult> {
    let ctx = crate::core_context(&app)?;
    let import_source = agora_core::import_service::ImportSource::PackManifest {
        manifest_json,
        target_instance_id: instance_id,
    };
    let request = agora_core::import_service::ImportRequest {
        source: import_source,
        symlink_saves: false,
    };
    let svc = agora_core::import_service::ImportService::new(ctx);
    svc.install_pack(request).await
}

/// Download a Modrinth .mrpack from a URL and import it as a new locked instance.
/// Returns the new instance ID.
#[tauri::command]
pub async fn import_modrinth_pack_by_url(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    download_url: String,
    pack_icon_url: Option<String>,
) -> LauncherResult<String> {
    let ctx = crate::core_context(&app)?;
    let sink = std::sync::Arc::new(TauriCoreProgressSink {
        app: app.clone(),
        event_name: "pack-install-progress",
    });
    let instance_id = agora_core::import_service::ImportService::new(ctx.clone())
        .run_mrpack_url_with_sink(
            &download_url,
            sink,
            agora_core::event_sink::CancellationToken::new(),
        )
        .await?
        .instance_id;
    if pack_icon_url
        .as_deref()
        .is_some_and(|url| url.starts_with("https://"))
    {
        agora_core::instance_service::InstanceService::new(ctx.clone())
            .set_pack_icon_url(&instance_id, pack_icon_url.as_deref())?;
    }
    // Keep downloaded packs protected by the normal lock transition. The
    // lifecycle service synchronizes the DB row and manifest together.
    instances::lock_instance(&app, &instance_id).await?;
    Ok(instance_id)
}

/// Read the Windows personalization accent color. Returns HSL string or null.
fn windows_accent_abgr_to_hsl(val: u32) -> String {
    // DWM stores AccentColor as ABGR, despite reg.exe displaying the value as
    // one hexadecimal DWORD.
    let r = (val & 0xFF) as f64;
    let g = ((val >> 8) & 0xFF) as f64;
    let b = ((val >> 16) & 0xFF) as f64;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 510.0;
    let s = if max == min {
        0.0
    } else {
        (max - min)
            / if l > 0.5 {
                510.0 - max - min
            } else {
                max + min
            }
    };
    let h = if max == min {
        0.0
    } else if max == r {
        60.0 * ((g - b) / (max - min))
    } else if max == g {
        60.0 * (2.0 + (b - r) / (max - min))
    } else {
        60.0 * (4.0 + (r - g) / (max - min))
    };
    let normalized_h = if h < 0.0 { h + 360.0 } else { h };
    format!(
        "hsl({:.0} {:.0}% {:.0}%)",
        normalized_h,
        s * 100.0,
        l * 100.0
    )
}

#[tauri::command]
pub fn get_windows_accent_color() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        use std::process::Command;
        let output = Command::new("reg")
            .args([
                "query",
                r"HKCU\Software\Microsoft\Windows\DWM",
                "/v",
                "AccentColor",
            ])
            .output()
            .ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        if let Some(line) = stdout.lines().find(|l| l.contains("AccentColor")) {
            if let Some(val_str) = line.split_whitespace().last() {
                if let Ok(val) = u32::from_str_radix(val_str.trim_start_matches("0x"), 16) {
                    return Some(windows_accent_abgr_to_hsl(val));
                }
            }
        }
        None
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

#[cfg(test)]
mod windows_accent_tests {
    use super::windows_accent_abgr_to_hsl;

    #[test]
    fn decodes_dwm_abgr_without_swapping_red_and_blue() {
        assert_eq!(windows_accent_abgr_to_hsl(0xff1a24e2), "hsl(3 79% 49%)");
        assert_eq!(windows_accent_abgr_to_hsl(0xffff0000), "hsl(240 100% 50%)");
    }
}

// ---------------------------------------------------------------------------
// Phase: Rust-backed browse cache (Modrinth + registry, paginated)
// ---------------------------------------------------------------------------

fn modrinth_project_type(content_type: &str) -> &str {
    match content_type {
        "pack" => "modpack",
        "server" => "minecraft_java_server",
        other => other,
    }
}

/// Search browse items — fetches registry + first Modrinth page, merges, caches in Rust, returns first page.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn browse_search(
    app: tauri::AppHandle,
    state: tauri::State<'_, LauncherState>,
    query_key: String,
    query: Option<String>,
    content_type: Option<String>,
    category: Option<String>,
    sort: Option<String>,
    mc_version: Option<String>,
    loader: Option<String>,
) -> LauncherResult<BrowsePage> {
    let s = state.lock().await;
    let (
        modrinth_api_allowed,
        technic_allowed,
        allow_unverified_packs,
        mean_approval,
        registry_items,
    ) = {
        let ctx = crate::core_context(&app)?;
        let svc = agora_core::settings::SettingsService::new(ctx.clone());
        // Live third-party browsing is opt-in per source and off by default, so
        // each source's own toggle is the whole story: curated-only *is* the
        // default state, not a mode to switch into.
        let me = svc.get_bool("modrinth_enabled").unwrap_or(false);
        let net_mr = svc
            .get("network_modrinth_enabled")
            .ok()
            .flatten()
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let api_ok = me && net_mr;
        // Tier Z packs stay hidden until unverified packs are allowed too.
        let technic_ok = svc.get_bool("technic_enabled").unwrap_or(false);
        let allow_unverified = svc.get_bool("allow_unverified_packs").unwrap_or(false);
        let curated_strategies = curated_strategies_from_settings(&app);
        let svc = agora_core::registry::RegistryService::new(ctx);
        let mean_approval = svc.mean_approval();
        let sort_enum = to_sort_option(sort.as_deref().unwrap_or("net_score"));
        let items = svc
            .browse_items(
                content_type.as_deref(),
                category.as_deref(),
                &sort_enum,
                &curated_strategies,
                mc_version.as_deref(),
                loader.as_deref(),
                query.as_deref(),
                100,
            )
            .map_err(|e| LauncherError::Generic {
                code: "ERR_REGISTRY".into(),
                message: e.to_string(),
            })?;
        (api_ok, technic_ok, allow_unverified, mean_approval, items)
    };

    let (modrinth_results, total_hits) = if modrinth_api_allowed {
        let modrinth_pt = content_type
            .as_deref()
            .map(modrinth_project_type)
            .map(str::to_string);
        let params = ModrinthSearchParams {
            query: query.clone(),
            categories: category.clone().map(|c| vec![c]),
            loaders: loader.clone().map(|l| vec![l]),
            game_versions: mc_version.clone().map(|v| vec![v]),
            sort: Some(to_modrinth_sort(sort.as_deref().unwrap_or("net_score"))),
            limit: Some(browse_cache::PAGE_SIZE as u32),
            offset: Some(0),
            project_type: modrinth_pt,
        };
        // Connection only needed for sync DB check — drop before async HTTP
        match agora_core::modrinth::search_modrinth_http(&params).await {
            Ok(page) => (page.results, page.total_hits as usize),
            Err(e) => return Err(e),
        }
    } else {
        (vec![], 0usize)
    };

    // Technic only distributes modpacks, so it is skipped unless the user is
    // looking at packs. Its search API ignores `offset`, so this is the only
    // fetch for the whole query — the results drain through the buffer.
    let wants_packs = content_type
        .as_deref()
        .map(|ct| ct == "pack")
        .unwrap_or(true);
    let technic_results = if technic_allowed && wants_packs {
        let ctx = crate::core_context(&app)?;
        match agora_core::technic::search_technic_http(
            &ctx.http_clients,
            query.as_deref().unwrap_or(""),
            TECHNIC_BROWSE_LIMIT,
        )
        .await
        {
            Ok(results) => results
                .into_iter()
                // Tier Z has no integrity information at all; it stays hidden
                // until the user explicitly accepts unverified packs.
                .filter(|r| {
                    allow_unverified_packs || r.tier == agora_core::technic::TechnicTier::Solder
                })
                .collect(),
            // Technic being down must not break the whole browse list.
            Err(_) => Vec::new(),
        }
    } else {
        Vec::new()
    };

    let offset = browse_cache::PAGE_SIZE;
    let has_more_modrinth = total_hits > offset;

    browse_cache::load_initial(
        &s.browse_cache,
        query_key,
        registry_items,
        modrinth_results,
        technic_results,
        BrowseFilters {
            query: query.unwrap_or_default(),
            content_type,
            category,
            sort: sort.unwrap_or_else(|| "net_score".to_string()),
            mc_version,
            loader,
            modrinth_enabled: modrinth_api_allowed,
        },
        offset,
        has_more_modrinth, // stored separately for load-more use
        mean_approval,
    )
    .await;

    let mut result = browse_cache::get_page(&s.browse_cache, 0).await;
    // has_more is true when there are more cached items than one page
    // OR more Modrinth results to fetch.
    let more_cached = result.has_more;
    let more_modrinth = has_more_modrinth;
    result.has_more = more_cached || more_modrinth;

    Ok(result)
}

/// Load a specific page from the browse cache, fetching additional Modrinth
/// data when the requested page is not yet cached.
#[tauri::command]
pub async fn browse_load_more(
    app: tauri::AppHandle,
    state: tauri::State<'_, LauncherState>,
    query_key: String,
    // The 0-indexed page the frontend wants to display next.
    page_index: usize,
) -> LauncherResult<BrowsePage> {
    let s = state.lock().await;
    let required_end = (page_index + 1) * browse_cache::PAGE_SIZE;
    let mean_approval = {
        let ctx = crate::core_context(&app)?;
        agora_core::registry::RegistryService::new(ctx).mean_approval()
    };

    // Fill the requested page. A fetched Modrinth page can contain duplicates,
    // so continue until the cache contains a full requested page or the remote
    // source is exhausted.
    loop {
        let (filters, modrinth_offset, should_fetch) = {
            let cache = s.browse_cache.read().await;
            if cache.query_key != query_key {
                return Err(LauncherError::Generic {
                    code: "ERR_BROWSE_STALE".into(),
                    message: "Browse query changed before pagination completed.".into(),
                });
            }
            // The carry-forward buffer may already cover the requested page —
            // curated is fetched in full up front, and Technic arrives in one
            // shot, so both can fill several pages with no further network use.
            let should_fetch = cache.items.len() + cache.buffer.len() < required_end
                && cache.has_more_modrinth
                && cache.filters.modrinth_enabled;
            (cache.filters.clone(), cache.modrinth_offset, should_fetch)
        };

        if !should_fetch {
            break;
        }

        let modrinth_pt = filters
            .content_type
            .as_deref()
            .map(modrinth_project_type)
            .map(str::to_string);
        let params = ModrinthSearchParams {
            query: Some(filters.query.clone()),
            categories: filters.category.clone().map(|c| vec![c]),
            loaders: filters.loader.clone().map(|l| vec![l]),
            game_versions: filters.mc_version.clone().map(|v| vec![v]),
            sort: Some(to_modrinth_sort(&filters.sort)),
            limit: Some(browse_cache::PAGE_SIZE as u32),
            offset: Some(modrinth_offset as u32),
            project_type: modrinth_pt,
        };

        let modrinth_page = agora_core::modrinth::search_modrinth_http(&params)
            .await
            .map_err(|e| LauncherError::Generic {
                code: "ERR_MODRINTH".into(),
                message: e.to_string(),
            })?;
        let new_offset = modrinth_offset + browse_cache::PAGE_SIZE;
        let has_more_modrinth = (modrinth_page.total_hits as usize) > new_offset;
        let new_items: Vec<browse_cache::BrowseItem> = modrinth_page
            .results
            .into_iter()
            .map(browse_cache::item_from_modrinth)
            .collect();

        if !browse_cache::append_items(
            &s.browse_cache,
            &query_key,
            new_items,
            new_offset,
            has_more_modrinth,
            mean_approval,
        )
        .await
        {
            return Err(LauncherError::Generic {
                code: "ERR_BROWSE_STALE".into(),
                message: "Browse query changed before pagination completed.".into(),
            });
        }
    }

    // Promote buffered items into the displayed list before slicing the page.
    if !browse_cache::drain_buffer(&s.browse_cache, &query_key, required_end).await {
        return Err(LauncherError::Generic {
            code: "ERR_BROWSE_STALE".into(),
            message: "Browse query changed before pagination completed.".into(),
        });
    }

    let mut page = browse_cache::get_page(&s.browse_cache, page_index).await;
    let cache = s.browse_cache.read().await;
    if cache.query_key != query_key {
        return Err(LauncherError::Generic {
            code: "ERR_BROWSE_STALE".into(),
            message: "Browse query changed before pagination completed.".into(),
        });
    }
    // `get_page` already accounts for the carry-forward buffer; ORing the
    // upstream flag on top only adds the "more to fetch" case. Recomputing the
    // cached half from `items.len()` alone would ignore buffered items and
    // strand them — curated and Technic are one-shot fetches, so once Modrinth
    // is exhausted (or disabled) the buffer is the only thing left to serve.
    page.has_more = page.has_more || (cache.has_more_modrinth && cache.filters.modrinth_enabled);
    Ok(page)
}

/// Get a specific page from the browse cache.
#[tauri::command]
pub async fn browse_page(
    state: tauri::State<'_, LauncherState>,
    query_key: String,
    page: usize,
) -> LauncherResult<BrowsePage> {
    let s = state.lock().await;
    if s.browse_cache.read().await.query_key != query_key {
        return Err(LauncherError::Generic {
            code: "ERR_BROWSE_STALE".into(),
            message: "Browse query changed before pagination completed.".into(),
        });
    }
    Ok(browse_cache::get_page(&s.browse_cache, page).await)
}

// ---------------------------------------------------------------------------
// C1-C4: canonical install pipeline facade commands
// ---------------------------------------------------------------------------

struct InstallProgressEmitter {
    app: tauri::AppHandle,
}

impl agora_core::install_pipeline::ProgressReporter for InstallProgressEmitter {
    fn report(&self, event: agora_core::install_pipeline::ProgressEvent) {
        use tauri::Emitter;
        let _ = self.app.emit("install:progress", event);
    }
}

/// Drop guard that removes install-activity markers even on panic.
/// Uses `try_lock()` on the tokio mutex so it is safe to drop during unwind.
struct InstallActivityGuard {
    state: std::sync::Arc<tokio::sync::Mutex<crate::state::AppState>>,
    ctx: agora_core::ctx::Ctx,
    instance_id: String,
    plan_id: String,
}

impl InstallActivityGuard {
    fn disarm(self) {
        // Arm is consumed — drop runs empty.
        std::mem::forget(self);
    }
}

impl Drop for InstallActivityGuard {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.state.try_lock() {
            guard.active_install_instances.remove(&self.instance_id);
        }
        self.ctx.operation_manager.remove_plan(&self.plan_id);
    }
}

/// Resolve an InstallIntent into a ResolvedInstallPlan (read-only, no mutation).
///
/// Uses the core-owned InstallService which handles curated, Modrinth,
/// manual, batch, and remove planning through the Resolver.
#[tauri::command]
pub async fn resolve_install_plan(
    app: tauri::AppHandle,
    intent: agora_core::install_pipeline::InstallIntent,
) -> LauncherResult<agora_core::install_pipeline::ResolvedInstallPlan> {
    let ctx = crate::core_context(&app)?;
    let service = agora_core::install_service::InstallService::new(ctx.clone());
    let reporter = InstallProgressEmitter { app };
    let target_instance = intent.target_instance.clone();
    let action_debug = format!("{:?}", intent.action);
    let plan = match service.resolve(intent, &reporter).await {
        Ok(plan) => plan,
        Err(error) => {
            crate::auth::log_line(&format!(
                "[install-resolve] target={target_instance} action={action_debug} error={error}"
            ));
            return Err(error);
        }
    };
    ctx.operation_manager
        .insert_plan(plan.fingerprint.clone(), plan.clone());
    Ok(plan)
}

/// Atomic launch/process exclusion for install apply (SOL-2 §18.6 /
/// MASTER_SPEC §19.15 / SAFETY_BOUNDARIES §6).
///
/// MUST be called while holding the application state lock so a launch cannot
/// race between this check and install registration. Rejects an install while
/// the TARGET instance is running or a launch is being prepared, and enforces
/// the one-active-install invariant.
fn ensure_install_apply_allowed(
    shared: &crate::state::AppState,
    instance_id: &str,
) -> LauncherResult<()> {
    if shared
        .running_processes
        .values()
        .any(|running| running.instance_id == instance_id)
    {
        return Err(LauncherError::Generic {
            code: "ERR_INSTALL_PROCESS_ACTIVE".into(),
            message: "This instance is running — stop it before installing.".into(),
        });
    }
    {
        if shared.launch_reservations.contains(instance_id) {
            return Err(LauncherError::Generic {
                code: "ERR_INSTALL_LAUNCH_RESERVED".into(),
                message:
                    "This instance is starting — wait for the launch to finish before installing."
                        .into(),
            });
        }
    }
    // Delegated launches have no reservation; the target-aware marker covers
    // the whole delegated session (SOL-2 §19.3).
    if shared.active_launches.contains(instance_id) {
        return Err(LauncherError::Generic {
            code: "ERR_INSTALL_LAUNCH_ACTIVE".into(),
            message:
                "This instance is starting a launch — wait for it to finish before installing."
                    .into(),
        });
    }
    if shared.active_install_instances.contains(instance_id) {
        return Err(LauncherError::Generic {
            code: "ERR_INSTALL_ACTIVE".into(),
            message: "Another install transaction is already active for this instance.".into(),
        });
    }
    Ok(())
}

/// Target-aware launch admission (SOL-2 §19.3): a launch must never start
/// while the target instance has an active install transaction. Evaluated
/// under the same state lock that reserves/begins the launch, so an install
/// cannot slip in between a preflight read and the reservation/start marker.
fn ensure_launch_admitted(
    shared: &crate::state::AppState,
    instance_id: &str,
) -> LauncherResult<()> {
    if shared.active_install_instances.contains(instance_id) {
        return Err(LauncherError::Generic {
            code: "ERR_LAUNCH_INSTALL_ACTIVE".into(),
            message: "This instance is being installed — wait for the install to finish before launching.".into(),
        });
    }
    Ok(())
}

/// Apply a fully-resolved install plan (staged, atomic, verified).
#[tauri::command]
pub async fn apply_install_plan(
    app: tauri::AppHandle,
    state: tauri::State<'_, LauncherState>,
    plan_id: String,
) -> LauncherResult<agora_core::install_pipeline::InstallOutcome> {
    let ctx = crate::core_context(&app)?;

    // Retrieve the plan from the core operation manager.
    let plan = ctx
        .operation_manager
        .get_plan(&plan_id)
        .ok_or_else(|| LauncherError::Generic {
            code: "ERR_PLAN_NOT_FOUND".into(),
            message: "This install plan is no longer available. Resolve it again.".into(),
        })?;
    let instance_id = paths::sanitize_id(&plan.intent.target_instance);
    if instance_id != plan.intent.target_instance || instance_id.is_empty() {
        return Err(LauncherError::Generic {
            code: "ERR_INVALID_INSTANCE".into(),
            message: "The plan targets an invalid instance ID.".into(),
        });
    }
    // Get the shared cancellation token from the stored plan.
    let cancel = ctx
        .operation_manager
        .token_for_plan(&plan_id)
        .ok_or_else(|| LauncherError::Generic {
            code: "ERR_PLAN_NOT_FOUND".into(),
            message: "This install plan is no longer available. Resolve it again.".into(),
        })?;
    let service = agora_core::install_service::InstallService::new(ctx.clone());
    let instance_dir = service.load_instance(&instance_id)?.instance_dir;
    {
        let mut shared = state.lock().await;
        // Atomic exclusion (SOL-2 §18.6): reject an install while the target
        // instance is running or a launch is being prepared, and enforce
        // one-active-install. All checks share the state lock with the
        // registration below, so a launch cannot race between them.
        ensure_install_apply_allowed(&shared, &instance_id)?;
        shared.active_install_instances.insert(instance_id.clone());
    }

    let reporter = InstallProgressEmitter { app: app.clone() };
    // Register a panic-safe guard that always cleans up install markers.
    let guard = InstallActivityGuard {
        state: Arc::clone(&state),
        ctx,
        instance_id: instance_id.clone(),
        plan_id: plan_id.clone(),
    };
    let outcome = service.execute(&plan, &reporter, &cancel).await;
    // Manual cleanup on the normal path — the guard handles the panic path.
    {
        let mut shared = state.lock().await;
        shared.active_install_instances.remove(&instance_id);
    }
    guard.disarm();
    if let Err(error) = agora_core::lkg::run_retention(&instance_dir) {
        crate::auth::log_line(&format!("snapshot retention after install failed: {error}"));
    }
    Ok(outcome)
}

/// Read the LKG marker for an instance, if any.
#[tauri::command]
pub async fn get_lkg_marker(
    app: tauri::AppHandle,
    instance_id: String,
) -> LauncherResult<Option<serde_json::Value>> {
    let sanitized = crate::paths::sanitize_id(&instance_id);
    let instance_dir = crate::paths::instance_dir(&app, &sanitized)
        .map_err(|_| LauncherError::LocalStateFailed)?;
    let lkg_path = instance_dir.join("lkg.json");
    if !lkg_path.is_file() {
        return Ok(None);
    }
    match std::fs::read_to_string(&lkg_path) {
        Ok(text) => {
            let value: serde_json::Value =
                serde_json::from_str(&text).map_err(|_| LauncherError::LocalStateFailed)?;
            Ok(Some(value))
        }
        Err(_) => Ok(None),
    }
}

/// Detect drift between a snapshot's file index and the current instance state.
#[tauri::command]
pub async fn detect_drift(
    app: tauri::AppHandle,
    instance_id: String,
    snapshot_id: String,
) -> LauncherResult<serde_json::Value> {
    let sanitized = crate::paths::sanitize_id(&instance_id);
    let instance_dir = crate::paths::instance_dir(&app, &sanitized)
        .map_err(|_| LauncherError::LocalStateFailed)?;
    let snapshot_id_for_task = snapshot_id.clone();
    let (ref_files, current_files) = tokio::task::spawn_blocking(move || {
        let reference =
            agora_core::snapshot::snapshot_file_index(&instance_dir, &snapshot_id_for_task)?;
        let current = agora_core::snapshot::live_file_index(&instance_dir)?;
        Ok::<_, String>((reference, current))
    })
    .await
    .map_err(|e| LauncherError::Generic {
        code: "ERR_DRIFT_TASK".into(),
        message: format!("Drift scan task failed: {e}"),
    })?
    .map_err(|e| LauncherError::Generic {
        code: "ERR_DRIFT".into(),
        message: e,
    })?;

    let diff = agora_core::lkg::compute_diff(&ref_files, &current_files, Some(snapshot_id), None);
    Ok(serde_json::to_value(&diff).unwrap_or_default())
}

/// Compare a live instance with a canonical lockfile without changing either.
#[tauri::command]
pub async fn verify_lockfile(
    app: tauri::AppHandle,
    instance_id: String,
    lockfile_json: String,
) -> LauncherResult<agora_core::lockfile::DriftReport> {
    use sha2::{Digest, Sha256};
    use std::collections::BTreeMap;

    let sanitized = crate::paths::sanitize_id(&instance_id);
    if sanitized.is_empty() || sanitized != instance_id {
        return Err(lockfile_error(
            "ERR_INVALID_INSTANCE",
            "The instance ID is invalid.",
        ));
    }
    let lockfile = agora_core::lockfile::InstanceLockfile::parse_and_validate(&lockfile_json)
        .map_err(|error| lockfile_error("ERR_LOCKFILE_INVALID", error))?;
    let instance_dir = crate::paths::instance_dir(&app, &sanitized)
        .map_err(|error| lockfile_error("ERR_INSTANCE_PATH", error.to_string()))?;
    tokio::task::spawn_blocking(move || {
        let index = agora_core::snapshot::live_file_index(&instance_dir)
            .map_err(|error| lockfile_error("ERR_DRIFT", error))?;
        let live_files = index
            .iter()
            .filter(|entry| {
                [
                    "mods/",
                    "resourcepacks/",
                    "shaderpacks/",
                    "datapacks/",
                    "saves/",
                ]
                .iter()
                .any(|prefix| entry.path.starts_with(prefix))
            })
            .map(|entry| (entry.path.clone(), entry.sha256.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut config = index
            .iter()
            .filter(|entry| entry.path.starts_with("config/"))
            .map(|entry| (entry.path.clone(), entry.sha256.clone()))
            .collect::<Vec<_>>();
        config.sort();
        let config_hash = if config.is_empty() {
            None
        } else {
            let bytes = serde_json::to_vec(&config)
                .map_err(|error| lockfile_error("ERR_CONFIG_HASH", error.to_string()))?;
            Some(hex::encode(Sha256::digest(bytes)))
        };
        Ok(agora_core::lockfile::detect_drift(
            &lockfile,
            &live_files,
            config_hash.as_deref(),
        ))
    })
    .await
    .map_err(|error| lockfile_error("ERR_DRIFT_TASK", error.to_string()))?
}

/// Repair artifact drift against a validated lockfile through one recovery
/// snapshot and one canonical transaction. Locked instances remain blocked.
#[tauri::command]
pub async fn repair_lockfile(
    app: tauri::AppHandle,
    state: tauri::State<'_, LauncherState>,
    instance_id: String,
    lockfile_json: String,
) -> LauncherResult<agora_core::install_pipeline::InstallOutcome> {
    use agora_core::install_pipeline::{
        InstallAction, InstallIntent, OptionalDepsPolicy, PlanOverrides, PreparedPlan,
        RequestSource, ResolvedOperation,
    };
    use std::collections::BTreeSet;

    let sanitized = paths::sanitize_id(&instance_id);
    if sanitized.is_empty() || sanitized != instance_id {
        return Err(lockfile_error(
            "ERR_INVALID_INSTANCE",
            "The instance ID is invalid.",
        ));
    }
    let lockfile = agora_core::lockfile::InstanceLockfile::parse_and_validate(&lockfile_json)
        .map_err(|error| lockfile_error("ERR_LOCKFILE_INVALID", error))?;
    let instance_dir = paths::instance_dir(&app, &sanitized)
        .map_err(|error| lockfile_error("ERR_INSTANCE_PATH", error.to_string()))?;

    let repair_dir = instance_dir.clone();
    let repair_lockfile = lockfile.clone();
    let (_manifest, _live_index, operations) = tokio::task::spawn_blocking(move || {
        let manifest_text = std::fs::read_to_string(repair_dir.join("instance_manifest.json"))
            .map_err(|error| lockfile_error("ERR_MANIFEST_READ", error.to_string()))?;
        let manifest: agora_core::models::InstanceManifest = serde_json::from_str(&manifest_text) // allow-raw-instance-manifest
            .map_err(|error| lockfile_error("ERR_MANIFEST_PARSE", error.to_string()))?;
        if manifest.minecraft_version != repair_lockfile.instance.minecraft_version
            || manifest.loader != repair_lockfile.instance.loader
            || manifest.loader_version != repair_lockfile.instance.loader_version
        {
            return Err(lockfile_error(
                "ERR_LOCKFILE_INSTANCE_MISMATCH",
                "Minecraft or loader versions differ; clone the lockfile instead of substituting versions.",
            ));
        }

        let live_index = agora_core::snapshot::live_file_index(&repair_dir)
            .map_err(|error| lockfile_error("ERR_DRIFT", error))?;
        let live_hashes = live_index
            .iter()
            .map(|entry| (entry.path.clone(), entry.sha256.clone()))
            .collect::<std::collections::BTreeMap<_, _>>();
        let installed = manifest
            .mods
            .iter()
            .chain(manifest.resourcepacks.iter())
            .chain(manifest.shaders.iter())
            .chain(manifest.datapacks.iter())
            .chain(manifest.worlds.iter())
            .collect::<Vec<_>>();
        let mut operations = Vec::new();
        for artifact in &repair_lockfile.artifacts {
            if artifact.unresolved_reason.is_some() || artifact.source_url.is_none() {
                return Err(lockfile_error(
                    "ERR_LOCKFILE_UNRESOLVED",
                    format!("{} has no reproducible verified source.", artifact.filename),
                ));
            }
            let expected_path = agora_core::lockfile::artifact_path(artifact);
            let in_sync = live_hashes
                .get(&expected_path)
                .map(|hash| hash.eq_ignore_ascii_case(&artifact.sha256))
                == Some(true);
            if in_sync {
                continue;
            }
            let resolved = resolved_lockfile_artifact(artifact)?;
            let existing = installed.iter().find(|entry| {
                artifact
                    .registry_id
                    .as_ref()
                    .zip(entry.registry_id.as_ref())
                    .map(|(left, right)| left.eq_ignore_ascii_case(right))
                    .unwrap_or(false)
                    || artifact
                        .modrinth_id
                        .as_ref()
                        .zip(entry.modrinth_id.as_ref())
                        .map(|(left, right)| left.eq_ignore_ascii_case(right))
                        .unwrap_or(false)
                    || (entry.filename == artifact.filename
                        && normalize_lock_content_type(&entry.content_type)
                            == normalize_lock_content_type(&artifact.content_type))
            });
            operations.push(if let Some(existing) = existing {
                ResolvedOperation::Update {
                    old_version_id: existing.version.clone().unwrap_or_else(|| "unknown".into()),
                    new_artifact: resolved,
                }
            } else {
                ResolvedOperation::Install { artifact: resolved }
            });
        }

        let expected_identities = repair_lockfile
            .artifacts
            .iter()
            .map(lockfile_identity)
            .collect::<BTreeSet<_>>();
        for entry in &installed {
            let identity = installed_lockfile_identity(entry);
            if !expected_identities.contains(&identity) {
                operations.push(ResolvedOperation::Remove {
                    target_filename: entry.filename.clone(),
                    reverse_dependents: Vec::new(),
                    content_type: Some(entry.content_type.clone()),
                });
            }
        }
        let expected_paths = repair_lockfile
            .artifacts
            .iter()
            .map(agora_core::lockfile::artifact_path)
            .collect::<BTreeSet<_>>();
        for entry in &live_index {
            for (prefix, content_type) in &[
                ("mods/", "mod"),
                ("resourcepacks/", "resourcepack"),
                ("shaderpacks/", "shader"),
                ("datapacks/", "datapack"),
                ("saves/", "world"),
            ] {
                if let Some(filename) = entry.path.strip_prefix(prefix) {
                    if !filename.contains('/') && !expected_paths.contains(&entry.path) {
                        operations.push(ResolvedOperation::Remove {
                            target_filename: filename.to_string(),
                            reverse_dependents: Vec::new(),
                            content_type: Some((*content_type).into()),
                        });
                    }
                }
            }
        }
        Ok((manifest, live_index, operations))
    })
    .await
    .map_err(|e| lockfile_error("ERR_LOCKFILE_TASK", e.to_string()))??;

    let intent = InstallIntent {
        action: InstallAction::RepairLockfile {
            content_hash: lockfile.content_hash.clone(),
        },
        target_instance: sanitized.clone(),
        optional_deps: OptionalDepsPolicy::ExcludeAll,
        requested_by: RequestSource::Interactive,
        overrides: PlanOverrides::default(),
    };
    let ctx = crate::core_context(&app)?;
    let service = agora_core::install_service::InstallService::new(ctx.clone());
    let reporter = InstallProgressEmitter { app: app.clone() };
    let plan = service
        .resolve_prepared(
            intent,
            PreparedPlan {
                operation: ResolvedOperation::Reconcile { operations },
                dependencies: Vec::new(),
                conflicts: Vec::new(),
                registry_revision: String::new(),
            },
            &reporter,
        )
        .await
        .map_err(|error| lockfile_error("ERR_LOCKFILE_PLAN", error.to_string()))?;

    // Store plan in core operation manager for cancellation tracking.
    let cancellation = ctx
        .operation_manager
        .insert_plan(plan.fingerprint.clone(), plan.clone());
    {
        let mut shared = state.lock().await;
        if !shared.active_install_instances.insert(sanitized.clone()) {
            return Err(lockfile_error(
                "ERR_INSTALL_ACTIVE",
                "Another install transaction is already active for this instance.",
            ));
        }
    }
    let guard = InstallActivityGuard {
        state: Arc::clone(&state),
        ctx,
        instance_id: sanitized.clone(),
        plan_id: plan.fingerprint.clone(),
    };
    let outcome = service.execute(&plan, &reporter, &cancellation).await;
    {
        let mut shared = state.lock().await;
        shared.active_install_instances.remove(&sanitized);
    }
    guard.disarm();
    if let agora_core::install_pipeline::InstallOutcome::Success { snapshot_id, .. } = &outcome {
        let post_snapshot_id = snapshot_id.clone();
        let post_dir = instance_dir.clone();
        let post_lockfile = lockfile.clone();
        // Resolved out here: the health scan runs on a blocking worker that
        // cannot borrow the AppHandle, and it must use the SAME registry path
        // every other cached_health caller uses (see lockfile_health_report).
        let post_registry_db = paths::registry_db_path(&app).ok();
        let post_result = tokio::task::spawn_blocking(move || {
            if let Err(error) = apply_lockfile_metadata(&post_dir, &post_lockfile) {
                let restored = agora_core::snapshot::restore_snapshot(&post_dir, &post_snapshot_id);
                return Err(lockfile_error(
                    "ERR_LOCKFILE_FINALIZE",
                    format!(
                        "Lockfile metadata repair failed ({error}); restore result: {restored:?}"
                    ),
                ));
            }
            let report = lockfile_health_report(&post_dir, post_registry_db.as_deref())?;
            if !report.blockers.is_empty() {
                let restored = agora_core::snapshot::restore_snapshot(&post_dir, &post_snapshot_id);
                return match restored {
                    Ok(()) => Ok(
                        agora_core::install_pipeline::InstallOutcome::HealthRollback {
                            health_report: report,
                            snapshot_id: post_snapshot_id.clone(),
                            warnings: Vec::new(),
                        },
                    ),
                    Err(error) => Err(lockfile_error(
                        "ERR_LOCKFILE_HEALTH_ROLLBACK",
                        format!("Repaired state has health blockers and rollback failed: {error}"),
                    )),
                };
            }
            let _ = agora_core::lkg::run_retention(&post_dir);
            Ok(outcome.clone())
        })
        .await
        .map_err(|e| lockfile_error("ERR_LOCKFILE_TASK", e.to_string()))??;
        return Ok(post_result);
    }
    Ok(outcome)
}

/// Export a canonical, content-addressed lockfile for an instance.
#[tauri::command]
pub async fn export_lockfile(
    app: tauri::AppHandle,
    instance_id: String,
) -> LauncherResult<serde_json::Value> {
    let sanitized = crate::paths::sanitize_id(&instance_id);
    if sanitized.is_empty() || sanitized != instance_id {
        return Err(lockfile_error(
            "ERR_INVALID_INSTANCE",
            "The instance ID is invalid.",
        ));
    }
    tokio::task::spawn_blocking(move || export_lockfile_sync(&app, &sanitized))
        .await
        .map_err(|error| lockfile_error("ERR_LOCKFILE_TASK", error.to_string()))?
}

fn export_lockfile_sync(
    app: &tauri::AppHandle,
    instance_id: &str,
) -> LauncherResult<serde_json::Value> {
    use agora_core::lockfile::{InstanceLockfile, LockedArtifact, LockedInstance, LockedLoader};
    use sha2::{Digest, Sha256};

    let instance_dir = crate::paths::instance_dir(app, instance_id)
        .map_err(|error| lockfile_error("ERR_INSTANCE_PATH", error.to_string()))?;
    let manifest_path = crate::paths::instance_manifest_path(app, instance_id)
        .map_err(|error| lockfile_error("ERR_INSTANCE_PATH", error.to_string()))?;
    let manifest_bytes = std::fs::read(&manifest_path)
        .map_err(|error| lockfile_error("ERR_MANIFEST_READ", error.to_string()))?;
    let manifest: agora_core::models::InstanceManifest =
        serde_json::from_slice(&manifest_bytes) // allow-raw-instance-manifest
            .map_err(|error| lockfile_error("ERR_MANIFEST_PARSE", error.to_string()))?;
    let manifest_sha256 = hex::encode(Sha256::digest(&manifest_bytes));

    let loader = crate::loader_manifests::find_entry(
        &manifest.loader,
        &manifest.minecraft_version,
        &manifest.loader_version,
    );
    let locked_loader = LockedLoader {
        source_url: loader.as_ref().map(|entry| entry.source_url.clone()),
        sha256: loader.as_ref().map(|entry| {
            crate::loader_manifests::strip_sha_prefix(&entry.sha256).to_ascii_lowercase()
        }),
    };

    let mut artifacts = Vec::new();
    for installed in manifest
        .mods
        .iter()
        .chain(manifest.resourcepacks.iter())
        .chain(manifest.shaders.iter())
        .chain(manifest.datapacks.iter())
        .chain(manifest.worlds.iter())
    {
        let content_type = normalize_lock_content_type(&installed.content_type).to_string();
        let probe = LockedArtifact {
            filename: installed.filename.clone(),
            content_type: content_type.clone(),
            registry_id: installed.registry_id.clone(),
            modrinth_id: installed.modrinth_id.clone(),
            source: installed.source.clone(),
            source_url: installed.source_url.clone(),
            version: installed.version.clone(),
            sha256: installed.sha256.clone(),
            enabled: installed.enabled,
            unresolved_reason: None,
        };
        let live_path = instance_dir.join(agora_core::lockfile::artifact_path(&probe));
        let (sha256, missing) = match hash_file_sha256(&live_path) {
            Ok(hash) => (hash, None),
            Err(error) => {
                let fallback = if valid_sha256(&installed.sha256) {
                    installed.sha256.to_ascii_lowercase()
                } else {
                    "0".repeat(64)
                };
                (
                    fallback,
                    Some(format!("Live artifact could not be read: {error}")),
                )
            }
        };
        let unresolved_reason = match (missing, installed.source_url.as_deref()) {
            (Some(reason), _) => Some(reason),
            (None, None) => {
                Some("No reproducible source URL is recorded for this artifact.".into())
            }
            (None, Some(_)) => None,
        };
        artifacts.push(LockedArtifact {
            sha256,
            unresolved_reason,
            ..probe
        });
    }

    let mut config_index = agora_core::snapshot::live_file_index(&instance_dir)
        .map_err(|error| lockfile_error("ERR_CONFIG_HASH", error))?
        .into_iter()
        .filter(|entry| entry.path.starts_with("config/"))
        .map(|entry| (entry.path, entry.sha256))
        .collect::<Vec<_>>();
    config_index.sort();
    let config_hash = if config_index.is_empty() {
        None
    } else {
        let bytes = serde_json::to_vec(&config_index)
            .map_err(|error| lockfile_error("ERR_CONFIG_HASH", error.to_string()))?;
        Some(hex::encode(Sha256::digest(bytes)))
    };

    let lockfile = InstanceLockfile::new(
        LockedInstance {
            name: manifest.name,
            minecraft_version: manifest.minecraft_version,
            loader: manifest.loader,
            loader_version: manifest.loader_version,
            is_locked: manifest.is_locked,
            user_preferences: manifest.user_preferences,
        },
        artifacts,
        locked_loader,
        manifest_sha256,
        config_hash,
    )
    .map_err(|error| lockfile_error("ERR_LOCKFILE_EXPORT", error))?;
    serde_json::to_value(lockfile)
        .map_err(|error| lockfile_error("ERR_LOCKFILE_EXPORT", error.to_string()))
}

/// Import a lockfile by creating a fresh instance and applying every artifact
/// through the canonical verified transaction. Any failure removes the partial
/// clone and reports the exact unavailable or invalid artifact.
#[tauri::command]
pub async fn import_lockfile(
    app: tauri::AppHandle,
    lockfile_json: String,
) -> LauncherResult<String> {
    use agora_core::install_pipeline::{
        ArtifactMetadata, ArtifactSource, BatchInstallItem, CancellationToken, HashAlgorithm,
        HashSpec, HashedValue, InstallAction, InstallIntent, OptionalDepsPolicy, PlanOverrides,
        PreparedPlan, RequestSource, ResolvedArtifact, ResolvedDownload, ResolvedOperation,
        SourceType,
    };
    use agora_core::lockfile::InstanceLockfile;

    let lockfile = InstanceLockfile::parse_and_validate(&lockfile_json)
        .map_err(|error| lockfile_error("ERR_LOCKFILE_INVALID", error))?;
    let unresolved = lockfile
        .artifacts
        .iter()
        .filter_map(|artifact| {
            artifact
                .unresolved_reason
                .as_ref()
                .map(|reason| format!("{}: {}", artifact.filename, reason))
                .or_else(|| {
                    artifact
                        .source_url
                        .is_none()
                        .then(|| format!("{}: source URL is unavailable", artifact.filename))
                })
        })
        .collect::<Vec<_>>();
    if !unresolved.is_empty() {
        return Err(lockfile_error(
            "ERR_LOCKFILE_UNRESOLVED",
            format!(
                "The lockfile cannot be reproduced without substitution:\n{}",
                unresolved.join("\n")
            ),
        ));
    }

    if let Some(expected) = lockfile.loader.sha256.as_deref() {
        let loader = crate::loader_manifests::find_entry(
            &lockfile.instance.loader,
            &lockfile.instance.minecraft_version,
            &lockfile.instance.loader_version,
        )
        .ok_or_else(|| {
            lockfile_error(
                "ERR_LOADER_UNAVAILABLE",
                "The exact pinned loader is not available in this Agora build.",
            )
        })?;
        let actual = crate::loader_manifests::strip_sha_prefix(&loader.sha256);
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(lockfile_error(
                "ERR_LOADER_HASH",
                "The pinned loader hash does not match the lockfile.",
            ));
        }
    }

    let base = crate::paths::sanitize_id(&lockfile.instance.name);
    let base = if base.is_empty() {
        "imported-instance"
    } else {
        base.as_str()
    };
    let mut instance_id = None;
    for _ in 0..32 {
        let candidate = format!("{}-{:08x}", base.trim_matches('-'), rand::random::<u32>());
        let candidate_dir = crate::paths::instance_dir(&app, &candidate)
            .map_err(|error| lockfile_error("ERR_INSTANCE_PATH", error.to_string()))?;
        if !candidate_dir.exists() {
            instance_id = Some(candidate);
            break;
        }
    }
    let instance_id = instance_id.ok_or_else(|| {
        lockfile_error(
            "ERR_INSTANCE_COLLISION",
            "Could not allocate a unique instance ID for the lockfile clone.",
        )
    })?;
    let explicit_memory = lockfile
        .instance
        .user_preferences
        .get("memoryMb")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| i64::try_from(value).ok());
    let memory = explicit_memory.unwrap_or(4096);
    let request = CreateInstanceRequest {
        name: lockfile.instance.name.clone(),
        instance_id: instance_id.clone(),
        minecraft_version: lockfile.instance.minecraft_version.clone(),
        loader: lockfile.instance.loader.clone(),
        loader_version: lockfile.instance.loader_version.clone(),
        jvm_memory_mb: Some(memory),
        jvm_memory_mode: Some(
            if explicit_memory.is_some() {
                "manual"
            } else {
                "auto"
            }
            .into(),
        ),
        jvm_gc: None,
        jvm_custom_args: None,
        jvm_always_pre_touch: None,
        is_modpack: None,
        pack_icon_url: None,
    };
    crate::instances::create_instance(app.clone(), request).await?;

    let mut operations = Vec::new();
    let mut request_items = Vec::new();
    for artifact in &lockfile.artifacts {
        let source_type = if artifact.registry_id.is_some() {
            SourceType::Curated
        } else if artifact.modrinth_id.is_some() {
            SourceType::Modrinth
        } else {
            SourceType::Manual
        };
        let item_id = artifact
            .registry_id
            .clone()
            .or_else(|| artifact.modrinth_id.clone())
            .unwrap_or_else(|| artifact.filename.clone());
        let source_url = artifact.source_url.clone().expect("validated above");
        let resolved = ResolvedArtifact::Download(ResolvedDownload {
            item_id: item_id.clone(),
            version_id: artifact
                .version
                .clone()
                .unwrap_or_else(|| artifact.sha256.clone()),
            source: ArtifactSource::Download { url: source_url },
            hashes: HashSpec {
                values: vec![HashedValue {
                    algorithm: HashAlgorithm::Sha256,
                    value: artifact.sha256.clone(),
                }],
            },
            size: 0,
            filename: artifact.filename.clone(),
            metadata: ArtifactMetadata {
                source_type: source_type.clone(),
                registry_id: artifact.registry_id.clone(),
                modrinth_id: artifact.modrinth_id.clone(),
                content_type: artifact.content_type.clone(),
                version: artifact.version.clone(),
                download_strategy: None,
                pinned_host: None,
            },
        });
        operations.push(ResolvedOperation::Install { artifact: resolved });
        request_items.push(BatchInstallItem {
            source_type,
            item_id,
            candidate_version: artifact.version.clone(),
        });
    }
    let intent = InstallIntent {
        action: InstallAction::BatchInstall {
            items: request_items,
        },
        target_instance: instance_id.clone(),
        optional_deps: OptionalDepsPolicy::ExcludeAll,
        requested_by: RequestSource::Interactive,
        overrides: PlanOverrides::default(),
    };
    let ctx = crate::core_context(&app)?;
    let service = agora_core::install_service::InstallService::new(ctx);
    let instance_dir = crate::paths::instance_dir(&app, &instance_id)
        .map_err(|error| lockfile_error("ERR_INSTANCE_PATH", error.to_string()))?;
    let reporter = InstallProgressEmitter { app: app.clone() };
    let plan = match service
        .resolve_prepared(
            intent,
            PreparedPlan {
                operation: ResolvedOperation::BatchInstall { operations },
                dependencies: Vec::new(),
                conflicts: Vec::new(),
                registry_revision: String::new(),
            },
            &reporter,
        )
        .await
    {
        Ok(plan) => plan,
        Err(error) => {
            let _ = crate::instances::delete_instance(&app, &instance_id);
            return Err(lockfile_error("ERR_LOCKFILE_PLAN", error.to_string()));
        }
    };
    let outcome = service
        .execute(&plan, &reporter, &CancellationToken::new())
        .await;
    let snapshot_id = match outcome {
        agora_core::install_pipeline::InstallOutcome::Success { snapshot_id, .. } => snapshot_id,
        other => {
            let _ = crate::instances::delete_instance(&app, &instance_id);
            return Err(lockfile_error(
                "ERR_LOCKFILE_INSTALL",
                format!("Lockfile transaction did not complete: {other:?}"),
            ));
        }
    };

    let post_import_dir = instance_dir.clone();
    let post_import_lockfile = lockfile.clone();
    let metadata_result = tokio::task::spawn_blocking(move || {
        apply_lockfile_metadata(&post_import_dir, &post_import_lockfile)
    })
    .await
    .map_err(|e| lockfile_error("ERR_LOCKFILE_TASK", e.to_string()))?;

    if let Err(error) = metadata_result {
        let restore_dir = instance_dir.clone();
        let restore_snap = snapshot_id.clone();
        let restore_result = tokio::task::spawn_blocking(move || {
            agora_core::snapshot::restore_snapshot(&restore_dir, &restore_snap)
        })
        .await
        .map_err(|e| lockfile_error("ERR_LOCKFILE_RESTORE", e.to_string()));
        let _ = crate::instances::delete_instance(&app, &instance_id);
        let message = match restore_result {
            Ok(Ok(())) => format!("Could not finalize lockfile metadata; the clone was rolled back: {error}"),
            Ok(Err(restore_error)) => format!(
                "Could not finalize lockfile metadata and rollback failed: {error}; {restore_error}"
            ),
            Err(join_error) => format!(
                "Could not finalize lockfile metadata and restore task failed: {error}; {join_error}"
            ),
        };
        return Err(lockfile_error("ERR_LOCKFILE_FINALIZE", message));
    }

    let health_dir = instance_dir.clone();
    // Same registry path as every other cached_health caller — see
    // lockfile_health_report for why a None here poisoned the durable cache.
    let health_registry_db = paths::registry_db_path(&app).ok();
    let health = tokio::task::spawn_blocking(move || {
        lockfile_health_report(&health_dir, health_registry_db.as_deref())
    })
    .await
    .map_err(|e| lockfile_error("ERR_LOCKFILE_HEALTH_TASK", e.to_string()))??;
    if !health.blockers.is_empty() {
        let restore_dir = instance_dir.clone();
        let restore_snap = snapshot_id.clone();
        let restore_result = tokio::task::spawn_blocking(move || {
            agora_core::snapshot::restore_snapshot(&restore_dir, &restore_snap)
        })
        .await
        .map_err(|e| lockfile_error("ERR_LOCKFILE_RESTORE", e.to_string()));
        let _ = crate::instances::delete_instance(&app, &instance_id);
        return Err(lockfile_error(
            "ERR_LOCKFILE_HEALTH",
            format!(
                "The reproduced state has health blockers and was discarded; restore result: {restore_result:?}"
            ),
        ));
    }
    if lockfile.instance.is_locked {
        if let Err(error) = crate::instances::lock_instance(&app, &instance_id).await {
            let restore_dir = instance_dir.clone();
            let restore_snap = snapshot_id.clone();
            let _ = tokio::task::spawn_blocking(move || {
                agora_core::snapshot::restore_snapshot(&restore_dir, &restore_snap)
            })
            .await;
            let _ = crate::instances::delete_instance(&app, &instance_id);
            return Err(error);
        }
    }
    {
        let retention_dir = instance_dir.clone();
        let _ = tokio::task::spawn_blocking(move || agora_core::lkg::run_retention(&retention_dir))
            .await;
    }
    Ok(instance_id)
}

fn apply_lockfile_metadata(
    instance_dir: &std::path::Path,
    lockfile: &agora_core::lockfile::InstanceLockfile,
) -> Result<(), String> {
    use std::io::Write;

    for artifact in lockfile
        .artifacts
        .iter()
        .filter(|artifact| !artifact.enabled)
    {
        let mut enabled = artifact.clone();
        enabled.enabled = true;
        let source = instance_dir.join(agora_core::lockfile::artifact_path(&enabled));
        let target = instance_dir.join(agora_core::lockfile::artifact_path(artifact));
        if target.is_file() && !source.exists() {
            continue;
        }
        if !source.is_file() {
            return Err(format!(
                "Expected imported artifact is missing: {}",
                source.display()
            ));
        }
        if target.exists() {
            return Err(format!(
                "Disabled target already exists: {}",
                target.display()
            ));
        }
        std::fs::rename(&source, &target)
            .map_err(|error| format!("Could not disable {}: {error}", artifact.filename))?;
    }

    let manifest_path = instance_dir.join("instance_manifest.json");
    let mut manifest = agora_core::helpers::read_manifest(&manifest_path)
        .map_err(|error| format!("Could not parse imported manifest: {error}"))?;
    manifest.is_locked = lockfile.instance.is_locked;
    manifest.user_preferences = lockfile.instance.user_preferences.clone();
    for entry in manifest
        .mods
        .iter_mut()
        .chain(manifest.resourcepacks.iter_mut())
        .chain(manifest.shaders.iter_mut())
        .chain(manifest.datapacks.iter_mut())
        .chain(manifest.worlds.iter_mut())
    {
        if let Some(locked) = lockfile.artifacts.iter().find(|artifact| {
            artifact.filename == entry.filename
                && normalize_lock_content_type(&artifact.content_type)
                    == normalize_lock_content_type(&entry.content_type)
        }) {
            entry.registry_id = locked.registry_id.clone();
            entry.modrinth_id = locked.modrinth_id.clone();
            entry.source = locked.source.clone();
            entry.source_url = locked.source_url.clone();
            entry.version = locked.version.clone();
            entry.sha256 = locked.sha256.clone();
            entry.enabled = locked.enabled;
        }
    }
    let bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| format!("Could not serialize imported manifest: {error}"))?;
    let temporary = manifest_path.with_extension("json.tmp");
    let mut output = std::fs::File::create(&temporary)
        .map_err(|error| format!("Could not create imported manifest: {error}"))?;
    output
        .write_all(&bytes)
        .map_err(|error| format!("Could not write imported manifest: {error}"))?;
    output
        .sync_all()
        .map_err(|error| format!("Could not sync imported manifest: {error}"))?;
    std::fs::rename(&temporary, &manifest_path)
        .map_err(|error| format!("Could not commit imported manifest: {error}"))
}

/// Health for a lockfile finalize/rollback decision.
///
/// `registry_db_path` MUST be the same value the other `cached_health` callers
/// pass (`paths::registry_db_path(&app).ok()`). Two reasons, one of which is a
/// bug this signature exists to prevent:
///
///  - Correctness: the registry supplies the curated `known_conflicts` a scan
///    reasons about. Passing `None` silently graded a lockfile install against
///    a smaller rule set than a normal health check.
///  - Cache identity: `cached_health` derives its fingerprint from the registry
///    DB's metadata when present and from the literal `<no-registry>` when not,
///    so a `None` caller computes a DIFFERENT token for the same instance. The
///    in-memory cache is keyed by `(instance_dir, registry_db_path)` and hides
///    that, but the durable cache is one file per instance with no key — so the
///    odd caller out overwrote `.agora/health-report.json` with a token nobody
///    else could match, and the next app start paid a full re-scan of every
///    installed jar. Measured on a 136-mod pack: a fresh process went from
///    "durable report cache hit" to "cache miss, running full scan" purely
///    because a lockfile operation had run beforehand.
fn lockfile_health_report(
    instance_dir: &std::path::Path,
    registry_db_path: Option<&std::path::Path>,
) -> LauncherResult<agora_core::health::HealthReport> {
    let manifest_path = instance_dir.join("instance_manifest.json");
    let manifest = agora_core::helpers::read_manifest(&manifest_path)
        .map_err(|error| lockfile_error("ERR_MANIFEST_PARSE", error.to_string()))?;
    Ok(agora_core::health::cached_health(
        instance_dir,
        &manifest,
        registry_db_path,
        None,
    ))
}

fn resolved_lockfile_artifact(
    artifact: &agora_core::lockfile::LockedArtifact,
) -> LauncherResult<agora_core::install_pipeline::ResolvedArtifact> {
    use agora_core::install_pipeline::{
        ArtifactMetadata, ArtifactSource, HashAlgorithm, HashSpec, HashedValue, ResolvedArtifact,
        ResolvedDownload, SourceType,
    };

    let source_type = if artifact.registry_id.is_some() {
        SourceType::Curated
    } else if artifact.modrinth_id.is_some() {
        SourceType::Modrinth
    } else {
        SourceType::Manual
    };
    let item_id = artifact
        .registry_id
        .clone()
        .or_else(|| artifact.modrinth_id.clone())
        .unwrap_or_else(|| artifact.filename.clone());
    let source_url = artifact.source_url.clone().ok_or_else(|| {
        lockfile_error(
            "ERR_LOCKFILE_UNRESOLVED",
            format!("{} has no reproducible source URL.", artifact.filename),
        )
    })?;
    Ok(ResolvedArtifact::Download(ResolvedDownload {
        item_id,
        version_id: artifact
            .version
            .clone()
            .unwrap_or_else(|| artifact.sha256.clone()),
        source: ArtifactSource::Download { url: source_url },
        hashes: HashSpec {
            values: vec![HashedValue {
                algorithm: HashAlgorithm::Sha256,
                value: artifact.sha256.clone(),
            }],
        },
        size: 0,
        filename: artifact.filename.clone(),
        metadata: ArtifactMetadata {
            source_type,
            registry_id: artifact.registry_id.clone(),
            modrinth_id: artifact.modrinth_id.clone(),
            content_type: artifact.content_type.clone(),
            version: artifact.version.clone(),
            download_strategy: None,
            pinned_host: None,
        },
    }))
}

fn lockfile_identity(artifact: &agora_core::lockfile::LockedArtifact) -> String {
    artifact
        .registry_id
        .as_ref()
        .map(|id| format!("registry:{}", id.to_ascii_lowercase()))
        .or_else(|| {
            artifact
                .modrinth_id
                .as_ref()
                .map(|id| format!("modrinth:{}", id.to_ascii_lowercase()))
        })
        .unwrap_or_else(|| {
            format!(
                "file:{}:{}",
                normalize_lock_content_type(&artifact.content_type),
                artifact.filename.to_ascii_lowercase()
            )
        })
}

fn installed_lockfile_identity(artifact: &crate::models::InstalledMod) -> String {
    artifact
        .registry_id
        .as_ref()
        .map(|id| format!("registry:{}", id.to_ascii_lowercase()))
        .or_else(|| {
            artifact
                .modrinth_id
                .as_ref()
                .map(|id| format!("modrinth:{}", id.to_ascii_lowercase()))
        })
        .unwrap_or_else(|| {
            format!(
                "file:{}:{}",
                normalize_lock_content_type(&artifact.content_type),
                artifact.filename.to_ascii_lowercase()
            )
        })
}

fn normalize_lock_content_type(content_type: &str) -> &str {
    match content_type {
        "resourcepack" | "resourcepacks" => "resourcepack",
        "shader" | "shaderpack" | "shaderpacks" => "shader",
        "datapack" | "datapacks" => "datapack",
        "world" | "worlds" => "world",
        _ => "mod",
    }
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn hash_file_sha256(path: &std::path::Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut file =
        std::fs::File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn lockfile_error(code: impl Into<String>, message: impl Into<String>) -> LauncherError {
    LauncherError::Generic {
        code: code.into(),
        message: message.into(),
    }
}

/// Cancel a running install.
#[tauri::command]
pub async fn cancel_install(app: tauri::AppHandle, plan_id: String) -> LauncherResult<()> {
    let ctx = crate::core_context(&app)?;

    if ctx.operation_manager.cancel_plan(&plan_id) {
        return Ok(());
    }
    Err(LauncherError::Generic {
        code: "ERR_INSTALL_NOT_ACTIVE".into(),
        message: "No active or pending install plan matches this identifier.".into(),
    })
}

/// Re-export the core `UpdateInfo` so the IPC shape stays identical to the
/// frontend contract at `desktop/src/lib/tauri.ts:633`. The canonical
/// definition lives in core (`crates/agora-core/src/update_cache.rs`) and is
/// also the row serialized into `instance_update_cache`.
pub use agora_core::update_cache::UpdateInfo;

/// Check for available updates for all tracked content in an instance.
///
/// Thin wrapper over `agora_core::update_cache::check_single_instance_updates_with`,
/// which is the single implementation of the matching rules -- the background
/// sweep (`sweep_all_updates`) drives the instance badge from that same code, so
/// the badge and this panel cannot disagree. All this adds is the process-local
/// candidate cache layered over core's persistent one, and the stricter error
/// policy an explicit user action needs: a failed lookup must surface as an
/// error here rather than read as "up to date".
#[tauri::command]
pub async fn check_instance_updates(
    app: tauri::AppHandle,
    state: tauri::State<'_, LauncherState>,
    instance_id: String,
) -> LauncherResult<Vec<UpdateInfo>> {
    let ctx = crate::core_context(&app)?;
    let sanitized = crate::paths::sanitize_id(&instance_id);
    let shared_state = state.inner().clone();

    let updates = agora_core::update_cache::check_single_instance_updates_with(
        &ctx,
        &sanitized,
        agora_core::update_cache::UpdateCheckOptions {
            memory_cache: Some(&shared_state),
            on_item_error: agora_core::update_cache::ItemErrorPolicy::Fail,
        },
    )
    .await?;

    // Persist so the result survives restart and can be read back without network.
    if let Ok(conn) = agora_core::db::local_state_connection(&ctx.paths.local_state_db()) {
        let _ = agora_core::update_cache::set_cached_instance_updates(&conn, &sanitized, &updates);
    }

    Ok(updates)
}

/// Read cached update results for a single instance without network.
///
/// Instant, offline-safe read from `instance_update_cache` (db.rs:370-v11).
/// Returns `None` when no sweep or explicit check has been cached yet.
#[tauri::command]
pub async fn get_cached_instance_updates(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
) -> LauncherResult<Option<Vec<UpdateInfo>>> {
    let ctx = crate::core_context(&app)?;
    let sanitized = crate::paths::sanitize_id(&instance_id);
    tokio::task::spawn_blocking(move || {
        let conn = agora_core::db::local_state_connection(&ctx.paths.local_state_db())
            .map_err(|_| LauncherError::LocalStateFailed)?;
        let cached = agora_core::update_cache::get_cached_instance_updates(&conn, &sanitized)
            .map_err(|_| LauncherError::LocalStateFailed)?;
        Ok(cached.map(|(updates, _checked_at)| updates))
    })
    .await
    .map_err(|_| LauncherError::LocalStateFailed)?
}

/// Cached update envelope for one instance.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CachedInstanceUpdates {
    pub instance_id: String,
    pub updates: Vec<UpdateInfo>,
    pub checked_at: String,
}

/// Read every cached instance row without network (instant hydration).
///
/// The frontend can render the last sweep's results immediately on mount
/// without waiting for a fresh network check; a background refresh can follow.
#[tauri::command]
pub async fn get_cached_all_updates(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
) -> LauncherResult<Vec<CachedInstanceUpdates>> {
    let ctx = crate::core_context(&app)?;
    tokio::task::spawn_blocking(move || {
        let conn = agora_core::db::local_state_connection(&ctx.paths.local_state_db())
            .map_err(|_| LauncherError::LocalStateFailed)?;
        let rows = agora_core::update_cache::get_all_cached_instance_updates(&conn)
            .map_err(|_| LauncherError::LocalStateFailed)?;
        Ok(rows
            .into_iter()
            .map(|(instance_id, updates, checked_at)| CachedInstanceUpdates {
                instance_id,
                updates,
                checked_at,
            })
            .collect())
    })
    .await
    .map_err(|_| LauncherError::LocalStateFailed)?
}

/// Invalidate the cached update row after a successful install.
///
/// The cache behaves like an invalidated view: the installer never touches it
/// (install_pipeline/install_service remain unaware). The frontend clears
/// optimistically on `InstallFlow` success so the badge does not linger.
#[tauri::command]
pub async fn clear_cached_instance_updates(
    app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    instance_id: String,
) -> LauncherResult<()> {
    let ctx = crate::core_context(&app)?;
    let sanitized = crate::paths::sanitize_id(&instance_id);
    tokio::task::spawn_blocking(move || {
        let conn = agora_core::db::local_state_connection(&ctx.paths.local_state_db())
            .map_err(|_| LauncherError::LocalStateFailed)?;
        agora_core::update_cache::delete_cached_instance_updates(&conn, &sanitized)
            .map_err(|_| LauncherError::LocalStateFailed)
    })
    .await
    .map_err(|_| LauncherError::LocalStateFailed)?
}

// ---------------------------------------------------------------------------
// Launcher path helpers (B3)
// ---------------------------------------------------------------------------

/// Auto-detect the Mojang launcher executable path.
///
/// Calls `mojang::resolve_launcher_path(None)` to discover the launcher
/// via OS-specific heuristics (registry, AppX, default install paths).
/// Returns the detected path or `ERR_MOJANG_NOT_FOUND`.
#[tauri::command]
pub fn detect_mojang_launcher(
    _app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
) -> LauncherResult<String> {
    let path = mojang::resolve_launcher_path(None)?;
    Ok(path.to_string_lossy().to_string())
}

/// Validate that a given launcher path exists and appears to be a valid
/// executable.
///
/// Returns `true` on success, or an error with a descriptive message.
#[tauri::command]
pub fn test_launcher_path(
    _app: tauri::AppHandle,
    _state: tauri::State<'_, LauncherState>,
    path: String,
) -> LauncherResult<bool> {
    let p = std::path::Path::new(&path);
    if !p.exists() {
        return Err(LauncherError::Generic {
            code: "ERR_LAUNCHER_PATH_NOT_FOUND".to_string(),
            message: format!("Path does not exist: {}", path),
        });
    }
    if !p.is_file() {
        return Err(LauncherError::Generic {
            code: "ERR_LAUNCHER_PATH_NOT_FILE".to_string(),
            message: format!("Path is not a file: {}", path),
        });
    }
    #[cfg(target_os = "windows")]
    {
        let ext = p.extension().and_then(|e| e.to_str());
        if !ext.is_some_and(|e| e.eq_ignore_ascii_case("exe")) {
            return Err(LauncherError::Generic {
                code: "ERR_LAUNCHER_PATH_NOT_EXE".to_string(),
                message: "The selected file is not an executable (.exe).".to_string(),
            });
        }
    }
    Ok(true)
}

// ---------------------------------------------------------------------------
// Java runtime management commands (Stage 3)
// ---------------------------------------------------------------------------

/// Process-wide per-major mutex to prevent duplicate runtime downloads for
/// the same Java major version.
static JAVA_RUNTIME_MUTEXES: LazyLock<
    std::sync::Mutex<std::collections::HashMap<u32, std::sync::Arc<tokio::sync::Mutex<()>>>>,
> = LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// Get or create a per-major download mutex.
fn java_runtime_mutex(major: u32) -> std::sync::Arc<tokio::sync::Mutex<()>> {
    let mut map = JAVA_RUNTIME_MUTEXES.lock().unwrap();
    map.entry(major)
        .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

/// Process-wide map of operation ID → cancellation flag for Java runtime provisioning.
/// Operations register an `Arc<AtomicBool>` before starting and remove it on completion.
static JAVA_RUNTIME_CANCELLATIONS: LazyLock<
    std::sync::Mutex<
        std::collections::HashMap<String, std::sync::Arc<std::sync::atomic::AtomicBool>>,
    >,
> = LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// Register a cancellation flag for a java runtime operation.
/// Returns the key and the shared flag.
fn register_java_runtime_cancel(
    operation_id: &str,
) -> (String, std::sync::Arc<std::sync::atomic::AtomicBool>) {
    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut map = JAVA_RUNTIME_CANCELLATIONS.lock().unwrap();
    map.insert(operation_id.to_string(), flag.clone());
    (operation_id.to_string(), flag)
}

/// RAII guard that unregisters a Java runtime cancellation flag on drop.
/// Ensures cleanup on all return paths, panics, and join errors.
struct CancelGuard {
    operation_id: String,
}

impl CancelGuard {
    fn new(operation_id: &str) -> Self {
        // register_java_runtime_cancel is called separately to get the flag;
        // this guard only handles unregistration on drop.
        Self {
            operation_id: operation_id.to_string(),
        }
    }
}

impl Drop for CancelGuard {
    fn drop(&mut self) {
        let mut map = JAVA_RUNTIME_CANCELLATIONS.lock().unwrap();
        map.remove(&self.operation_id);
    }
}

/// Cancel a Java runtime provisioning operation by operation ID.
#[tauri::command]
pub async fn cancel_java_runtime(operation_id: String) -> LauncherResult<()> {
    let map = JAVA_RUNTIME_CANCELLATIONS.lock().unwrap();
    if let Some(flag) = map.get(&operation_id) {
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    } else {
        Err(LauncherError::Generic {
            code: "ERR_CANCEL_NOT_FOUND".into(),
            message: format!("No active java runtime operation with id '{operation_id}'"),
        })
    }
}

/// Summary of a detected or managed Java runtime.
#[derive(Debug, Clone, serde::Serialize)]
pub struct JavaRuntimeSummary {
    pub path: String,
    pub version: u32,
    pub version_string: String,
    pub source: String,
    pub arch: Option<String>,
}

impl From<agora_core::java::JavaInstallation> for JavaRuntimeSummary {
    fn from(j: agora_core::java::JavaInstallation) -> Self {
        Self {
            path: j.path.to_string_lossy().to_string(),
            version: j.version,
            version_string: j.version_string,
            source: format!("{:?}", j.source),
            arch: j.arch,
        }
    }
}

/// List all discovered Java runtimes (managed + Mojang + system).
///
/// Discovery goes through the core `RuntimeService`, which is cache-first:
/// a valid persisted inventory is reused, so a warm call spawns NO
/// `java -XshowSettings:properties -version` probes. Calling
/// `java::detect_java_candidates` directly from here re-probed every JRE on
/// the machine on every call — one subprocess per Mojang-bundled and system
/// runtime — which is why this read dominated the High Interaction load.
#[tauri::command]
pub async fn list_java_runtimes(app: tauri::AppHandle) -> LauncherResult<Vec<JavaRuntimeSummary>> {
    let ctx = crate::core_context(&app)?;

    // Read global java_path setting to prepend as Override source.
    let global_java = agora_core::settings::SettingsService::new(ctx.clone())
        .get_string("java_path")
        .ok()
        .flatten()
        .filter(|s| !s.trim().is_empty());

    let summaries = tokio::task::spawn_blocking(move || {
        let candidates = agora_core::runtime_service::RuntimeService::new(ctx)
            .list_candidates()
            .unwrap_or_default();
        let mut results: Vec<JavaRuntimeSummary> = candidates
            .into_iter()
            .map(JavaRuntimeSummary::from)
            .collect();

        // Prepend global java_path if valid and not a duplicate.
        if let Some(ref java_path) = global_java {
            let java_path = java_path.trim().to_string();
            if !java_path.is_empty()
                && !results.iter().any(|r| r.path == java_path)
                && std::path::Path::new(&java_path).is_file()
            {
                if let Some(inst) = agora_core::java::inspect_java(std::path::Path::new(&java_path))
                {
                    results.insert(
                        0,
                        JavaRuntimeSummary {
                            path: inst.path.to_string_lossy().to_string(),
                            version: inst.version,
                            version_string: inst.version_string,
                            source: "Override".to_string(),
                            arch: inst.arch,
                        },
                    );
                }
            }
        }

        results
    })
    .await
    .map_err(|e| LauncherError::Generic {
        code: "ERR_JAVA_DETECTION".into(),
        message: format!("Java detection task failed: {e}"),
    })?;

    Ok(summaries)
}

/// Ensure a managed Java runtime for the given major version is installed.
/// Uses a per-major mutex to prevent duplicate downloads.
/// Returns a summary of the provisioned runtime.
///
/// Accepts an optional `operation_id` for cancellation; if omitted a stable
/// key `"settings-{major}"` is used.
#[tauri::command]
pub async fn ensure_java_runtime(
    app: tauri::AppHandle,
    major: u32,
    operation_id: Option<String>,
) -> LauncherResult<JavaRuntimeSummary> {
    use tauri::Emitter;

    // Stable operation ID when caller doesn't provide one.
    let op_id = operation_id.unwrap_or_else(|| format!("settings-{major}"));

    let ctx = crate::core_context(&app)?;
    let runtimes_root = ctx.paths.java_runtimes_root();
    let catalog = ctx.runtime_catalog.snapshot();
    let policy = agora_core::network::NetworkPolicy::from_ctx(&ctx)?;

    // Check network policy.
    policy.check(agora_core::network::NetworkCategory::JavaRuntime)?;

    // Acquire per-major mutex to prevent concurrent download of the same version.
    let major_mutex = java_runtime_mutex(major);
    let _major_lock = major_mutex.lock().await;

    // Register cancellation flag and RAII guard for automatic cleanup on return/panic/error.
    let (_op_id, cancel_flag) = register_java_runtime_cancel(&op_id);
    let _cancel_guard = CancelGuard::new(&op_id);

    // Use a channel-based progress bridge so the progress impl can be 'static.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(String, Option<f64>)>();
    let app_clone = app.clone();

    // Spawn a task to forward progress events to Tauri.
    let _progress_task = tokio::spawn(async move {
        while let Some((msg, pct)) = rx.recv().await {
            let stage = if pct.is_some_and(|p| p >= 100.0) {
                "ready"
            } else {
                "downloading"
            };
            let _ = app_clone.emit(
                "java-runtime-progress",
                serde_json::json!({
                    "instance_id": "",
                    "major": major,
                    "stage": stage,
                    "message": msg,
                    "percent": pct.unwrap_or(0.0),
                }),
            );
        }
    });

    let cancel_for_progress = cancel_flag.clone();
    let lock_manager = ctx.lock_manager().clone();
    let ensured = tokio::task::spawn_blocking(move || {
        struct ChannelProgress {
            sender: tokio::sync::mpsc::UnboundedSender<(String, Option<f64>)>,
            cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
        }
        impl agora_core::runtime_manager::RuntimeProgress for ChannelProgress {
            fn on_progress(&self, message: &str, percent: Option<f64>) {
                let _ = self.sender.send((message.to_string(), percent));
            }
            fn is_cancelled(&self) -> bool {
                self.cancel.load(std::sync::atomic::Ordering::SeqCst)
            }
        }
        let progress = ChannelProgress {
            sender: tx,
            cancel: cancel_for_progress,
        };
        agora_core::runtime_manager::ensure_runtime(
            &runtimes_root,
            major,
            &catalog,
            &policy,
            Some(&progress),
            Some(&lock_manager),
        )
    })
    .await
    .map_err(|_| LauncherError::Generic {
        code: "ERR_ENSURE_RUNTIME".into(),
        message: format!("Failed to provision Java {major} runtime."),
    })??;

    Ok(JavaRuntimeSummary::from(ensured))
}

/// Remove unused managed Java runtimes (keep newest per major).
/// Returns the number of runtimes that were removed.
#[tauri::command]
pub async fn remove_unused_java_runtimes(app: tauri::AppHandle) -> LauncherResult<usize> {
    let ctx = crate::core_context(&app)?;
    let runtimes_root = ctx.paths.java_runtimes_root();
    let catalog = ctx.runtime_catalog.snapshot();

    let removed = tokio::task::spawn_blocking(move || {
        agora_core::runtime_manager::remove_unused(&runtimes_root, &catalog, &[])
    })
    .await
    .map_err(|e| LauncherError::Generic {
        code: "ERR_REMOVE_UNUSED".into(),
        message: format!("Remove unused runtimes task failed: {e}"),
    })??;

    Ok(removed)
}

/// Inspect a Java executable at the given path and return its summary.
/// Used for picker validation before the user saves a custom Java path.
#[tauri::command]
pub async fn inspect_java_executable(path: String) -> LauncherResult<JavaRuntimeSummary> {
    let p = std::path::PathBuf::from(&path);
    if !p.is_file() {
        return Err(LauncherError::Generic {
            code: "ERR_JAVA_PATH_NOT_FILE".into(),
            message: format!("Java executable not found at: {path}"),
        });
    }
    let insp = tokio::task::spawn_blocking(move || agora_core::java::inspect_java(&p))
        .await
        .map_err(|_| LauncherError::Generic {
            code: "ERR_JAVA_INSPECT".into(),
            message: format!("Failed to inspect Java at: {path}"),
        })?
        .ok_or_else(|| LauncherError::Generic {
            code: "ERR_JAVA_INSPECT_FAILED".into(),
            message: format!("Could not parse Java version info from: {path}"),
        })?;

    Ok(JavaRuntimeSummary::from(insp))
}

/// Update per-instance Java path and incompatible override setting.
/// Pass `path` as null to clear the per-instance override.
#[tauri::command]
pub async fn update_instance_java(
    app: tauri::AppHandle,
    instance_id: String,
    path: Option<String>,
    allow_incompatible: bool,
    custom_args: Option<String>,
) -> LauncherResult<()> {
    let ctx = crate::core_context(&app)?;
    let service = agora_core::instance_service::InstanceService::new(ctx);
    service.update_java(
        &instance_id,
        path.as_deref(),
        allow_incompatible,
        custom_args.as_deref(),
    )
}

/// Update the structured JVM tuning controls for an instance.
#[tauri::command]
pub async fn update_instance_jvm(
    app: tauri::AppHandle,
    instance_id: String,
    memory_mb: i64,
    gc: String,
    always_pre_touch: bool,
    custom_args: String,
    memory_mode: Option<String>,
) -> LauncherResult<()> {
    let ctx = crate::core_context(&app)?;
    let service = agora_core::instance_service::InstanceService::new(ctx);
    service.update_jvm(
        &instance_id,
        memory_mb,
        &gc,
        always_pre_touch,
        &custom_args,
        memory_mode.as_deref().unwrap_or("manual"),
    )
}

#[tauri::command]
pub async fn recommend_instance_memory(
    app: tauri::AppHandle,
    instance_id: String,
) -> LauncherResult<agora_core::memory_recommendation::MemoryRecommendation> {
    let ctx = crate::core_context(&app)?;
    agora_core::instance_service::InstanceService::new(ctx).memory_recommendation(&instance_id)
}

/// Pick the upstream Modrinth ordering that best matches our blended score.
///
/// Chunks are sorted only within themselves, so the closer Modrinth's order is
/// to ours, the smaller the inversions across a chunk boundary. Measured,
/// `index=follows` returns sodium -> fabric-api -> iris -> modmenu, which
/// tracks our formula far better than `downloads` (which leads with fabric-api,
/// a library we deliberately demote).
fn to_modrinth_sort(sort: &str) -> ModrinthSort {
    match sort {
        "downloads" => ModrinthSort::Downloads,
        "newest" => ModrinthSort::Newest,
        "updated" | "velocity" => ModrinthSort::Updated,
        // Every blended sort wants engagement-led ordering.
        "net_score" | "most_upvoted" | "most_downvoted" | "follows" => ModrinthSort::Follows,
        _ => ModrinthSort::Relevance,
    }
}

fn to_sort_option(sort: &str) -> registry::SortOption {
    match sort {
        "net_score" => registry::SortOption::NetScore,
        "velocity" => registry::SortOption::Velocity,
        "most_downvoted" => registry::SortOption::MostDownvoted,
        "newest" => registry::SortOption::Newest,
        "most_upvoted" => registry::SortOption::MostUpvoted,
        _ => registry::SortOption::NetScore,
    }
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn open_instance_folder(
    app: tauri::AppHandle,
    instance_id: String,
) -> Result<(), String> {
    let path = crate::paths::instance_dir(&app, &instance_id)
        .map_err(|e| format!("Failed to resolve instance path: {e}"))?;
    open_path_in_explorer(&path)
}

#[tauri::command]
pub async fn open_data_folder(app: tauri::AppHandle) -> Result<(), String> {
    let path = crate::paths::app_data_dir(&app)
        .map_err(|e| format!("Failed to resolve application data folder: {e}"))?;
    open_path_in_explorer(&path)
}

/// Relaunch the app.
///
/// Called after the updater has staged a new version. `AppHandle::restart` is
/// part of Tauri core, so this needs no additional plugin — the frontend
/// previously invoked `plugin:process|restart`, which would have failed because
/// `tauri-plugin-process` is not a dependency.
///
/// This never returns: `restart` terminates the current process.
#[tauri::command]
pub async fn restart_app(app: tauri::AppHandle) -> Result<(), String> {
    app.restart()
}

#[tauri::command]
pub async fn reveal_path(path: String) -> Result<(), String> {
    reveal_in_explorer(std::path::Path::new(&path))
}

/// Open an external content URL in the user's real browser.
///
/// A plain `<a target="_blank">` does nothing inside the Tauri webview, which is
/// why "View source" never worked on any card. The URL comes from community
/// content (a Modrinth project, a Technic page, a curated `page_url`), so it is
/// validated before being handed to the OS: https only, no embedded
/// credentials, and a host must be present. Without those checks this command
/// would hand arbitrary strings — `file://`, `javascript:`, UNC paths — to the
/// shell.
#[tauri::command]
pub async fn open_external_url(url: String) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url.trim()).map_err(|_| "Not a valid URL.".to_string())?;
    if parsed.scheme() != "https" {
        return Err("Only https links can be opened.".into());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("Links with embedded credentials are refused.".into());
    }
    if parsed.host_str().is_none() {
        return Err("Link has no host.".into());
    }
    open_url_in_browser(parsed.as_str())
}

fn open_url_in_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(url)
            .spawn()
            .map_err(|e| format!("Failed to open link: {e}"))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map_err(|e| format!("Failed to open link: {e}"))?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(|e| format!("Failed to open link: {e}"))?;
    }
    Ok(())
}

fn open_path_in_explorer(path: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(path)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {e}"))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {e}"))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {e}"))?;
    }
    Ok(())
}

fn reveal_in_explorer(path: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let path_str = path.to_string_lossy().to_string();
        // Explorer parses its command line itself and does not treat the path
        // as a separate argv token. `/select,` and the path must be a single
        // argument, or paths containing spaces fail with "Location is not
        // available" while the file actually exists.
        std::process::Command::new("explorer")
            .arg(format!("/select,{path_str}"))
            .spawn()
            .map_err(|e| format!("Failed to reveal file: {e}"))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn()
            .map_err(|e| format!("Failed to reveal file: {e}"))?;
    }
    #[cfg(target_os = "linux")]
    {
        // xdg-open cannot select files; open the parent directory instead
        if let Some(parent) = path.parent() {
            std::process::Command::new("xdg-open")
                .arg(parent)
                .spawn()
                .map_err(|e| format!("Failed to open folder: {e}"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod command_helper_tests {
    use super::{
        apply_lockfile_metadata, create_or_reuse_prelaunch_snapshot, ensure_install_apply_allowed,
        ensure_launch_admitted, installed_lockfile_identity, lockfile_identity,
        normalize_lock_content_type, queue_game_log, QueuedGameLogLine,
    };
    use crate::state::LauncherState;

    #[test]
    fn install_apply_rejects_running_target_instance() {
        let state = LauncherState::default();
        {
            let mut shared = state.blocking_lock();
            shared.running_processes.insert(
                1,
                crate::state::RunningProcess {
                    instance_id: "inst-a".into(),
                    pid: 42,
                    session_id: 1,
                },
            );
        }
        let shared = state.blocking_lock();
        let err = ensure_install_apply_allowed(&shared, "inst-a").unwrap_err();
        assert_eq!(err.code(), "ERR_INSTALL_PROCESS_ACTIVE");
        // A different instance is not blocked by the target check.
        assert!(ensure_install_apply_allowed(&shared, "inst-b").is_ok());
    }

    #[test]
    fn install_apply_rejects_launch_reservation_for_target() {
        let state = LauncherState::default();
        {
            let mut shared = state.blocking_lock();
            shared.launch_reservations.insert("inst-a".into());
        }
        let shared = state.blocking_lock();
        let err = ensure_install_apply_allowed(&shared, "inst-a").unwrap_err();
        assert_eq!(err.code(), "ERR_INSTALL_LAUNCH_RESERVED");
        assert!(ensure_install_apply_allowed(&shared, "inst-b").is_ok());
    }

    #[test]
    fn install_apply_rejects_competing_install_and_recovers_after_completion() {
        let state = LauncherState::default();
        {
            let mut shared = state.blocking_lock();
            shared.active_install_instances.insert("inst-a".into());
        }
        {
            let shared = state.blocking_lock();
            let err = ensure_install_apply_allowed(&shared, "inst-a").unwrap_err();
            assert_eq!(err.code(), "ERR_INSTALL_ACTIVE");
            // Post-rejection state: the marker is left untouched (the apply
            // never registered the competing transaction).
            assert!(shared.active_install_instances.contains("inst-a"));
        }
        // Cancellation/completion removes the marker; apply is allowed again.
        {
            let mut shared = state.blocking_lock();
            shared.active_install_instances.remove("inst-a");
        }
        let shared = state.blocking_lock();
        assert!(ensure_install_apply_allowed(&shared, "inst-a").is_ok());
        assert!(!shared.active_install_instances.contains("inst-a"));
    }

    #[test]
    fn install_apply_rejects_after_running_process_clears_then_allows() {
        let state = LauncherState::default();
        {
            let mut shared = state.blocking_lock();
            shared.running_processes.insert(
                2,
                crate::state::RunningProcess {
                    instance_id: "inst-a".into(),
                    pid: 7,
                    session_id: 2,
                },
            );
        }
        {
            let shared = state.blocking_lock();
            assert_eq!(
                ensure_install_apply_allowed(&shared, "inst-a")
                    .unwrap_err()
                    .code(),
                "ERR_INSTALL_PROCESS_ACTIVE"
            );
        }
        // Process exits; apply is allowed and registers the marker.
        {
            let mut shared = state.blocking_lock();
            shared.running_processes.clear();
            ensure_install_apply_allowed(&shared, "inst-a").unwrap();
            shared.active_install_instances.insert("inst-a".into());
        }
        let shared = state.blocking_lock();
        assert!(shared.active_install_instances.contains("inst-a"));
    }

    // SOL-2 §19.3: launch admission is the inverse of install admission. A
    // launch must never start while the target has an active install, and an
    // install must never apply while the target has an active/reserved launch.

    #[test]
    fn launch_admission_rejects_active_install_for_target() {
        let state = LauncherState::default();
        {
            let mut shared = state.blocking_lock();
            shared.active_install_instances.insert("inst-a".into());
        }
        let shared = state.blocking_lock();
        let err = ensure_launch_admitted(&shared, "inst-a").unwrap_err();
        assert_eq!(err.code(), "ERR_LAUNCH_INSTALL_ACTIVE");
        // A different instance is not blocked by the target check.
        assert!(ensure_launch_admitted(&shared, "inst-b").is_ok());
    }

    #[test]
    fn launch_admission_catches_install_registered_between_preflight_and_reservation() {
        // Direct/recovery preflight runs, then an install registers, then the
        // reservation-block admission check must reject (the race Sol called out).
        let state = LauncherState::default();
        {
            let mut shared = state.blocking_lock();
            // Preflight sees neither a process nor a reservation...
            assert!(ensure_launch_admitted(&shared, "inst-a").is_ok());
            // ...an install registers in the gap...
            shared.active_install_instances.insert("inst-a".into());
        }
        {
            let shared = state.blocking_lock();
            // ...the reservation-block check (the final transition) rejects.
            let err = ensure_launch_admitted(&shared, "inst-a").unwrap_err();
            assert_eq!(err.code(), "ERR_LAUNCH_INSTALL_ACTIVE");
        }
    }

    #[test]
    fn install_apply_rejects_active_launch_marker_and_recovers_after_cleanup() {
        let state = LauncherState::default();
        {
            let mut shared = state.blocking_lock();
            shared.active_launches.insert("inst-a".into());
        }
        {
            let shared = state.blocking_lock();
            let err = ensure_install_apply_allowed(&shared, "inst-a").unwrap_err();
            assert_eq!(err.code(), "ERR_INSTALL_LAUNCH_ACTIVE");
        }
        // Cancellation/completion removes the delegated launch marker; install
        // apply is permitted again.
        {
            let mut shared = state.blocking_lock();
            shared.active_launches.remove("inst-a");
        }
        let shared = state.blocking_lock();
        assert!(ensure_install_apply_allowed(&shared, "inst-a").is_ok());
    }

    #[test]
    fn mutual_exclusion_both_directions_with_cleanup_then_retry() {
        // Install registers first: launch is rejected, then install cleanup
        // permits a launch.
        let state = LauncherState::default();
        {
            let mut shared = state.blocking_lock();
            shared.active_install_instances.insert("inst-a".into());
        }
        {
            let shared = state.blocking_lock();
            assert_eq!(
                ensure_launch_admitted(&shared, "inst-a")
                    .unwrap_err()
                    .code(),
                "ERR_LAUNCH_INSTALL_ACTIVE"
            );
        }
        {
            let mut shared = state.blocking_lock();
            shared.active_install_instances.remove("inst-a");
            shared.active_launches.insert("inst-a".into());
        }
        {
            let shared = state.blocking_lock();
            // Install while the (delegated) launch marker is set is rejected.
            assert_eq!(
                ensure_install_apply_allowed(&shared, "inst-a")
                    .unwrap_err()
                    .code(),
                "ERR_INSTALL_LAUNCH_ACTIVE"
            );
            // Launch is permitted once the install is gone.
            assert!(ensure_launch_admitted(&shared, "inst-a").is_ok());
        }
        {
            let mut shared = state.blocking_lock();
            shared.active_launches.remove("inst-a");
        }
        let shared = state.blocking_lock();
        assert!(ensure_launch_admitted(&shared, "inst-a").is_ok());
        assert!(ensure_install_apply_allowed(&shared, "inst-a").is_ok());
    }

    #[test]
    fn game_log_queue_drops_overflow_without_blocking() {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let dropped = std::sync::atomic::AtomicU64::new(0);
        queue_game_log(
            &sender,
            &dropped,
            QueuedGameLogLine {
                line: "first".into(),
                stream: "stdout".into(),
                session_id: Some(1),
            },
        );
        queue_game_log(
            &sender,
            &dropped,
            QueuedGameLogLine {
                line: "overflow".into(),
                stream: "stderr".into(),
                session_id: Some(1),
            },
        );

        assert_eq!(
            dropped.load(std::sync::atomic::Ordering::Relaxed),
            1,
            "a full UI queue must drop rather than block the JVM pipe reader"
        );
        assert_eq!(receiver.try_recv().unwrap().line, "first");
    }

    #[test]
    fn unchanged_prelaunch_state_reuses_current_lkg_snapshot() {
        let temp = temp_instance_dir();
        let first = create_or_reuse_prelaunch_snapshot(&temp, "first").unwrap();
        agora_core::lkg::record_launch_outcome(
            &temp,
            Some(&first),
            "launch-1",
            agora_core::lkg::LaunchOutcome::Success,
        )
        .unwrap();

        let reused = create_or_reuse_prelaunch_snapshot(&temp, "second").unwrap();
        assert_eq!(reused, first);
        assert_eq!(
            agora_core::snapshot::list_snapshots(&temp).unwrap().len(),
            1
        );
        std::fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn changed_prelaunch_state_creates_a_new_snapshot() {
        let temp = temp_instance_dir();
        let first = create_or_reuse_prelaunch_snapshot(&temp, "first").unwrap();
        agora_core::lkg::record_launch_outcome(
            &temp,
            Some(&first),
            "launch-1",
            agora_core::lkg::LaunchOutcome::Success,
        )
        .unwrap();
        std::fs::write(temp.join("mods/changed.jar"), b"changed").unwrap();

        let second = create_or_reuse_prelaunch_snapshot(&temp, "second").unwrap();
        assert_ne!(second, first);
        assert_eq!(
            agora_core::snapshot::list_snapshots(&temp).unwrap().len(),
            2
        );
        std::fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn lockfile_content_type_normalization_and_identity_are_stable() {
        assert_eq!(normalize_lock_content_type("resourcepacks"), "resourcepack");
        assert_eq!(normalize_lock_content_type("shaderpack"), "shader");
        assert_eq!(normalize_lock_content_type("worlds"), "world");
        let installed = test_installed_mod("example.jar", true);
        assert_eq!(installed_lockfile_identity(&installed), "registry:example");
        let locked = agora_core::lockfile::LockedArtifact {
            filename: "example.jar".into(),
            content_type: "mod".into(),
            registry_id: Some("example".into()),
            modrinth_id: None,
            source: "registry".into(),
            source_url: Some("https://example.com/example.jar".into()),
            version: Some("1.0".into()),
            sha256: "ab".repeat(32),
            enabled: true,
            unresolved_reason: None,
        };
        assert_eq!(lockfile_identity(&locked), "registry:example");
    }

    #[test]
    fn apply_lockfile_metadata_disables_artifact_and_is_idempotent() {
        let directory = temp_instance_dir();
        std::fs::write(directory.join("mods/example.jar"), b"example").unwrap();
        let mut manifest: agora_core::models::InstanceManifest = serde_json::from_str(
            // allow-raw-instance-manifest
            &std::fs::read_to_string(directory.join("instance_manifest.json")).unwrap_or_default(),
        )
        .unwrap_or_else(|_| test_manifest());
        manifest.mods = vec![test_installed_mod("example.jar", true)];
        std::fs::write(
            directory.join("instance_manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let lockfile = agora_core::lockfile::InstanceLockfile::new(
            agora_core::lockfile::LockedInstance {
                name: "Test".into(),
                minecraft_version: "1.21.1".into(),
                loader: "fabric".into(),
                loader_version: "0.16.0".into(),
                is_locked: false,
                user_preferences: serde_json::json!({}),
            },
            vec![agora_core::lockfile::LockedArtifact {
                filename: "example.jar".into(),
                content_type: "mod".into(),
                registry_id: Some("example".into()),
                modrinth_id: None,
                source: "registry".into(),
                source_url: Some("https://example.com/example.jar".into()),
                version: Some("1.0".into()),
                sha256: agora_core::download::sha256_hex(b"example"),
                enabled: false,
                unresolved_reason: None,
            }],
            agora_core::lockfile::LockedLoader {
                source_url: None,
                sha256: None,
            },
            "cd".repeat(32),
            None,
        )
        .unwrap();

        apply_lockfile_metadata(&directory, &lockfile).unwrap();
        apply_lockfile_metadata(&directory, &lockfile).unwrap();
        assert!(!directory.join("mods/example.jar").exists());
        assert!(directory.join("mods/example.jar.disabled").is_file());
        let updated: agora_core::models::InstanceManifest = serde_json::from_slice(
            // allow-raw-instance-manifest
            &std::fs::read(directory.join("instance_manifest.json")).unwrap(),
        )
        .unwrap();
        assert!(!updated.mods[0].enabled); // allow-raw-instance-manifest
        std::fs::remove_dir_all(directory).unwrap();
    }

    fn temp_instance_dir() -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "agora-command-test-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(directory.join("mods")).unwrap();
        std::fs::write(directory.join("instance_manifest.json"), "{}").unwrap();
        directory
    }

    fn test_manifest() -> agora_core::models::InstanceManifest {
        agora_core::models::InstanceManifest {
            manifest_version: agora_core::models::CURRENT_MANIFEST_VERSION,
            pack_origin: None,
            instance_id: "test".into(),
            name: "Test".into(),
            created_from_pack: None,
            minecraft_version: "1.21.1".into(),
            loader: "fabric".into(),
            loader_version: "0.16.0".into(),
            is_locked: false,
            mods: vec![],
            resourcepacks: vec![],
            shaders: vec![],
            datapacks: vec![],
            worlds: vec![],
            user_preferences: serde_json::json!({}),
        }
    }

    fn test_installed_mod(filename: &str, enabled: bool) -> agora_core::models::InstalledMod {
        agora_core::models::InstalledMod {
            pack_managed: false,
            filename: filename.into(),
            registry_id: Some("example".into()),
            modrinth_id: None,
            source: "registry".into(),
            source_url: Some("https://example.com/example.jar".into()),
            version: Some("1.0".into()),
            sha256: agora_core::download::sha256_hex(b"example"),
            installed_at: "2026-07-12T00:00:00Z".into(),
            java_packages: vec![],
            mod_jar_id: Some("example".into()),
            provided_mod_ids: vec![],
            enabled,
            content_type: "mod".into(),
            depends_on: vec![],
            optional_deps: vec![],
            incompatible_deps: vec![],
        }
    }
}
