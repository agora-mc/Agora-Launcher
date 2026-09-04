use crate::crash_investigator;
use crate::error::{LauncherError, LauncherResult};
use crate::models::InstanceRow;
use crate::mojang;
use crate::paths;
use std::path::Path;

pub use agora_core::instance_service::{CreateInstanceRequest, InstanceDetail};
pub use agora_core::loader_service::LoaderVersionSummary;

/// Create an isolated instance directory, persist metadata, and ensure loader install.
///
/// Ordering and rollback:
/// 1. Create dirs + manifest (blocking).
/// 2. Ensure runtime layout + bootstrap base Mojang version metadata (async).
/// 3. Ensure loader installed (async) — skipped for vanilla/empty loader.
///    On failure, clean up the instance dir.
/// 4. Persist DB row + launcher profile (blocking). On failure, clean up the
///    instance dir only (do NOT remove globally shared loader files).
pub async fn create_instance<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    req: CreateInstanceRequest,
) -> LauncherResult<InstanceRow> {
    let ctx = crate::core_context(&app)?;
    agora_core::instance_service::InstanceService::new(ctx)
        .create(req)
        .await
}

// ---------------------------------------------------------------------------
// ensure_loader_installed — reusable, shared install-once loader service
// ---------------------------------------------------------------------------

/// Ensure a modloader is installed in the Agora-owned runtime root (not the
/// official `.minecraft`). Returns a summary indicating whether a cached valid
/// install was used or a fresh install ran.
///
/// All loader artifacts (version JSONs, profiles, libraries) are written
/// under `minecraft_root`.  Only receipts and the download cache live under
/// `app_data`.
///
/// # Install-once semantics
///
/// - **Forge/NeoForge**: If the profile exists and a valid receipt adoption
///   succeeds, returns immediately without download/installer execution.
/// - **Fabric/Quilt**: If the profile exists and is valid, returns immediately.
/// - **Cache**: Verified installer/profile bytes are cached under
///   `app_data/loader_cache/<loader>/<mc>/<version>/<file>` with SHA-256
///   verification. Network only on cache miss or hash mismatch.
/// - **Network policy**: `NetworkPolicy::from_db` with `LoaderMetadataAndContent`
///   check before any download or installer execution.
/// - **Concurrency**: The core `LoaderService` serializes Forge/NeoForge
///   installer execution via a process-wide mutex.
pub async fn ensure_loader_installed<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    _instance_id: &str,
    loader: &str,
    mc_version: &str,
    loader_version: &str,
    force_reinstall: bool,
    _minecraft_root: &Path,
) -> LauncherResult<agora_core::installed_profile::InstallReceiptSummary> {
    let ctx = crate::core_context(app)?;
    agora_core::loader_service::LoaderService::new(ctx)
        .ensure_installed(loader, mc_version, loader_version, force_reinstall)
        .await
}

// ---------------------------------------------------------------------------
// repair_instance_loader — force reinstall for a specific instance
// ---------------------------------------------------------------------------

/// Repair the loader for an instance by force-reinstalling it. Returns the
/// install receipt summary.
pub async fn repair_instance_loader<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    instance_id: &str,
) -> LauncherResult<agora_core::installed_profile::InstallReceiptSummary> {
    let ctx = crate::core_context(app)?;
    agora_core::loader_service::LoaderService::new(ctx)
        .repair(instance_id)
        .await
}

// ---------------------------------------------------------------------------
// Blocking helpers
// ---------------------------------------------------------------------------

/// List all user instances from `local_state.db`.
pub fn list_instances<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> LauncherResult<Vec<InstanceRow>> {
    let ctx = crate::core_context(app)?;
    agora_core::instance_service::InstanceService::new(ctx).list()
}

/// Fetch a single instance and its on-disk manifest.
pub fn get_instance_detail<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    instance_id: &str,
) -> LauncherResult<Option<InstanceDetail>> {
    let ctx = crate::core_context(app)?;
    agora_core::instance_service::InstanceService::new(ctx).get(instance_id)
}

/// Delete an instance: delegates to core `InstanceService::delete` with the
/// OS-trash adapter so files land in the Recycle Bin (not hard-removed).
pub fn delete_instance<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    instance_id: &str,
) -> LauncherResult<()> {
    let ctx = crate::core_context(app)?;
    let service = agora_core::instance_service::InstanceService::new(ctx);
    service.delete(instance_id, Some(trash_adapter()))
}

/// Build the OS-trash adapter that core calls on the quarantined directory.
fn trash_adapter() -> agora_core::instance_service::TrashFn {
    std::sync::Arc::new(|path| {
        trash::delete(path).map_err(|e| LauncherError::Generic {
            code: "ERR_INSTANCE_DELETE".into(),
            message: format!("Failed to trash directory: {e}"),
        })
    })
}

/// Unlock a locked pack instance for manual mod management (§6.5).
pub async fn unlock_instance<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    instance_id: &str,
) -> LauncherResult<()> {
    let ctx = crate::core_context(app)?;
    agora_core::instance_service::InstanceService::new(ctx).unlock(instance_id)
}

/// Lock an unlocked pack instance, discarding the lock snapshot.
pub async fn lock_instance<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    instance_id: &str,
) -> LauncherResult<()> {
    let ctx = crate::core_context(app)?;
    agora_core::instance_service::InstanceService::new(ctx).lock(instance_id)
}

/// Rename an instance in the local state DB.
pub async fn rename_instance<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    instance_id: &str,
    new_name: &str,
) -> LauncherResult<()> {
    let ctx = crate::core_context(app)?;
    agora_core::instance_service::InstanceService::new(ctx).rename(instance_id, new_name)
}

/// Revert an unlocked instance to its lock snapshot (§6.5).
pub async fn revert_instance<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    instance_id: &str,
) -> LauncherResult<()> {
    lock_instance(app, instance_id).await
}

/// Launch an instance by delegating to the official Mojang launcher.
///
/// The official launcher reads `launcher_profiles.json` and its saved
/// installation selection once, at startup. A handoff into an already-running
/// launcher therefore cannot select the pack — it is single-instance, so the
/// spawn below only focuses the existing window and leaves whatever was
/// selected before in place. Rather than hand off and let the user find the
/// wrong pack loaded, that case is reported as `ERR_MOJANG_LAUNCHER_RUNNING`
/// so the UI can offer to restart it; `restart_launcher` is the confirmed
/// retry.
pub fn launch_instance<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    instance_id: &str,
    restart_launcher: bool,
) -> LauncherResult<()> {
    let ctx = crate::core_context(app)?;
    let service = agora_core::instance_service::InstanceService::new(ctx);
    let mut preparation = service.prepare_delegated_launch(instance_id)?;

    if preparation.launcher_running && restart_launcher {
        let resolved = mojang::resolve_launcher_path(preparation.launcher_path.as_deref()).ok();
        if !agora_core::official_launcher::close_running(
            resolved.as_deref(),
            std::time::Duration::from_secs(15),
        ) {
            return Err(LauncherError::Generic {
                code: "ERR_MOJANG_LAUNCHER_RUNNING".into(),
                message:
                    "The Minecraft Launcher did not close. Close it yourself, then launch again."
                        .into(),
            });
        }
        // Re-prepare against a closed launcher: only now can Agora write the
        // installation selection the launcher will actually read on startup.
        preparation = service.prepare_delegated_launch(instance_id)?;
    }

    if preparation.launcher_running {
        return Err(LauncherError::Generic {
            code: "ERR_MOJANG_LAUNCHER_RUNNING".into(),
            message:
                "The Minecraft Launcher is already open, so it will not switch to this pack. Restart it to continue."
                    .into(),
        });
    }

    let launcher_path = mojang::resolve_launcher_path(preparation.launcher_path.as_deref())?;
    std::process::Command::new(&launcher_path)
        // Ignored by the current launcher, which restores its own saved
        // selection instead (measured on core 3.39.31); kept for the legacy
        // launcher, which does read it.
        .arg("--profile")
        .arg(&preparation.profile_id)
        .spawn()
        .map_err(|_| LauncherError::LaunchFailed)?;

    // Signal D: record which mods survived this launch for survival baseline learning.
    let sanitized = paths::sanitize_id(instance_id);
    let _ = crash_investigator::record_survival(app, &sanitized, &preparation.mod_ids);

    Ok(())
}
