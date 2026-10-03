//! Thin Tauri adapter for `agora_core::providers`.
//!
//! Every decision — which providers exist, whether one may be used, whether a
//! plan is safe to install — is core's. This module only finds the process's
//! plugin service, builds the registry, and moves data across IPC.

use crate::error::LauncherResult;
use agora_core::providers::{
    self, ProjectDetail, ProviderDescriptor, ProviderRegistry, VersionsRequest, VersionsResponse,
};
use agora_game_minecraft::providers::install;
use tauri::AppHandle;

/// The plugin service, when the user has plugins switched on. Providers work
/// without it — the official ones need no plugin runtime at all.
fn plugin_service(app: &AppHandle) -> Option<agora_core::plugins::PluginService> {
    let service = crate::plugins::service(app).ok()?;
    service.is_enabled().then_some(service)
}

/// Every provider this session knows about: Agora's own and any a plugin
/// contributes.
pub fn registry(app: &AppHandle) -> LauncherResult<ProviderRegistry> {
    let ctx = crate::core_context(app)?;
    let plugins = plugin_service(app);
    if let Some(service) = &plugins {
        // Picks up plugins installed or toggled since the service was built.
        let _ = service.reload();
    }
    Ok(ProviderRegistry::new(&ctx, plugins.as_ref()))
}

fn project_of(item_id: &str) -> LauncherResult<(String, String)> {
    providers::parse_item_id(item_id)
        .map(|(provider, project)| (provider.to_string(), project.to_string()))
        .ok_or_else(|| crate::error::LauncherError::Generic {
            code: "ERR_PROVIDER_ITEM".into(),
            message: format!("`{item_id}` is not a content-provider item."),
        })
}

#[tauri::command]
pub async fn list_content_providers(app: AppHandle) -> LauncherResult<Vec<ProviderDescriptor>> {
    Ok(registry(&app)?.descriptors())
}

/// Categories every usable provider offers, for Browse's category picker.
#[tauri::command]
pub async fn list_provider_categories(
    app: AppHandle,
) -> LauncherResult<Vec<providers::ProviderCategories>> {
    Ok(providers::categories(&registry(&app)?).await)
}

#[tauri::command]
pub async fn set_content_provider_enabled(
    app: AppHandle,
    provider_id: String,
    enabled: bool,
) -> LauncherResult<Vec<ProviderDescriptor>> {
    let ctx = crate::core_context(&app)?;
    let plugins = plugin_service(&app);
    let registry = registry(&app)?;
    providers::set_enabled(&ctx, &registry, plugins.as_ref(), &provider_id, enabled)?;
    Ok(self::registry(&app)?.descriptors())
}

#[tauri::command]
pub async fn provider_project(app: AppHandle, item_id: String) -> LauncherResult<ProjectDetail> {
    let (provider_id, project_id) = project_of(&item_id)?;
    registry(&app)?
        .usable(&provider_id)?
        .project(&project_id)
        .await
}

#[tauri::command]
pub async fn provider_versions(
    app: AppHandle,
    item_id: String,
    minecraft_version: Option<String>,
    loader: Option<String>,
) -> LauncherResult<VersionsResponse> {
    let (provider_id, project_id) = project_of(&item_id)?;
    registry(&app)?
        .usable(&provider_id)?
        .versions(VersionsRequest {
            project_id,
            minecraft_version,
            loader,
        })
        .await
}

/// What installing would involve, before anything is downloaded.
#[tauri::command]
pub async fn provider_install_preview(
    app: AppHandle,
    item_id: String,
    version_id: Option<String>,
    minecraft_version: Option<String>,
    loader: Option<String>,
) -> LauncherResult<install::PlanPreview> {
    let ctx = crate::core_context(&app)?;
    install::preview(
        &ctx,
        &registry(&app)?,
        &item_id,
        version_id.as_deref(),
        minecraft_version.as_deref().unwrap_or_default(),
        loader.as_deref().unwrap_or_default(),
    )
    .await
}

/// Install a catalog entry that pins one version of a provider's pack.
/// `accept_changed` is the user's answer to `ERR_PROVIDER_PACK_CHANGED`.
#[tauri::command]
pub async fn install_catalog_provider_pack(
    app: AppHandle,
    item_id: String,
    accept_changed: bool,
) -> LauncherResult<agora_game_minecraft::import::ImportResult> {
    let ctx = crate::core_context(&app)?;
    install::install_catalog_pack(&ctx, &registry(&app)?, &item_id, accept_changed).await
}

/// Install a provider's modpack as a new instance.
#[tauri::command]
pub async fn provider_install_pack(
    app: AppHandle,
    item_id: String,
    version_id: Option<String>,
) -> LauncherResult<agora_game_minecraft::import::ImportResult> {
    let ctx = crate::core_context(&app)?;
    install::install_pack(&ctx, &registry(&app)?, &item_id, version_id.as_deref()).await
}
