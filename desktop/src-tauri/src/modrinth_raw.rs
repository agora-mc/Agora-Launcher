//! Thin Tauri adapter for [`agora_game_minecraft::modrinth::ModrinthService`].
//!
//! Every function extracts the [`agora_core::ctx::Ctx`] from the
//! `tauri::AppHandle`, constructs a [`ModrinthService`], and delegates.
//! No Modrinth HTTP request building, gate logic, or manifest-writing
//! logic lives here.

// Re-export all public types from core so existing command signatures
// remain stable.
pub use agora_game_minecraft::modrinth::{
    ModrinthCategoryInfo, ModrinthFileMetadata, ModrinthGameVersionInfo, ModrinthLoaderInfo,
    ModrinthProjectFull, ModrinthSearchPage, ModrinthSearchParams, ModrinthSearchResult,
    ModrinthSort, RawModrinthDependency, RawModrinthVersionCandidate,
};

use crate::error::LauncherResult;
use crate::models::InstalledMod;
use agora_core::http_client::HttpClients;
use agora_core::settings::SettingsService;
use agora_game_minecraft::modrinth::ModrinthService;

/// Read the `modrinth_enabled` boolean setting from `local_state.db` via
/// core-owned [`SettingsService`].
/// Returns `false` on any read failure (security default: off).
pub fn is_modrinth_enabled(app: &tauri::AppHandle) -> bool {
    let ctx = match crate::core_context(app) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let svc = SettingsService::new(ctx);
    svc.get_bool("modrinth_enabled").unwrap_or(false)
}

/// Live Modrinth search (§6.3). Gated by the `modrinth_enabled` setting.
pub async fn search_modrinth(
    _clients: &HttpClients,
    app: &tauri::AppHandle,
    params: &ModrinthSearchParams,
) -> LauncherResult<ModrinthSearchPage> {
    let ctx = crate::core_context(app)?;
    let svc = ModrinthService::new(ctx);
    svc.search_modrinth(params).await
}

/// List Modrinth category tags for the filter UI.
pub async fn list_modrinth_categories(
    _clients: &HttpClients,
    app: &tauri::AppHandle,
) -> LauncherResult<Vec<ModrinthCategoryInfo>> {
    let ctx = crate::core_context(app)?;
    let svc = ModrinthService::new(ctx);
    svc.list_modrinth_categories().await
}

/// List Modrinth loader tags for the filter UI.
pub async fn list_modrinth_loaders(
    _clients: &HttpClients,
    app: &tauri::AppHandle,
) -> LauncherResult<Vec<ModrinthLoaderInfo>> {
    let ctx = crate::core_context(app)?;
    let svc = ModrinthService::new(ctx);
    svc.list_modrinth_loaders().await
}

/// List Modrinth game version tags for the filter UI.
pub async fn list_modrinth_game_versions(
    _clients: &HttpClients,
    app: &tauri::AppHandle,
) -> LauncherResult<Vec<ModrinthGameVersionInfo>> {
    let ctx = crate::core_context(app)?;
    let svc = ModrinthService::new(ctx);
    svc.list_modrinth_game_versions().await
}

/// List raw Modrinth versions for a project, optionally scoped to an
/// instance's Minecraft version and loader.
pub async fn list_raw_modrinth_versions(
    _clients: &HttpClients,
    app: &tauri::AppHandle,
    instance_id: Option<&str>,
    project_id: &str,
    project_type: Option<&str>,
) -> LauncherResult<Vec<RawModrinthVersionCandidate>> {
    let ctx = crate::core_context(app)?;
    let svc = ModrinthService::new(ctx);
    svc.list_raw_modrinth_versions(instance_id, project_id, project_type)
        .await
}

/// Fetch a single Modrinth project's full details including the body (markdown
/// description) via `GET /v2/project/{id}`.
pub async fn fetch_project_full(
    _clients: &HttpClients,
    app: &tauri::AppHandle,
    project_id: &str,
) -> LauncherResult<ModrinthProjectFull> {
    let ctx = crate::core_context(app)?;
    let svc = ModrinthService::new(ctx);
    svc.fetch_project_full(project_id).await
}

/// Resolve Modrinth-published per-file metadata (URL + sha1 + sha512 + size)
/// for a single project by matching the given filename.
pub async fn resolve_modrinth_file_metadata(
    _clients: &HttpClients,
    project_id: &str,
    filename: &str,
) -> Option<ModrinthFileMetadata> {
    agora_game_minecraft::modrinth::resolve_modrinth_file_metadata(project_id, filename).await
}

/// Install a raw (uncurated) Modrinth mod file into an instance.
///
/// Thin adapter: validates Modrinth is enabled and SHA-1 is present, then
/// delegates to core [`ModrinthService::install_raw_modrinth`].
pub async fn install_raw_modrinth(
    app: &tauri::AppHandle,
    instance_id: &str,
    project_id: &str,
    candidate: &RawModrinthVersionCandidate,
    project_type: &str,
) -> LauncherResult<InstalledMod> {
    let ctx = crate::core_context(app)?;
    let svc = ModrinthService::new(ctx);
    svc.install_raw_modrinth(instance_id, project_id, candidate, project_type)
        .await
}
