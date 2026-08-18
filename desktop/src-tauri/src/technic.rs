//! Thin Tauri adapter for `agora_core::technic` + the Technic import paths.
//!
//! Every function extracts the [`agora_core::ctx::Ctx`] from the
//! `tauri::AppHandle` and delegates to core. Consent for Technic browsing and
//! installs is enforced in core (`technic_enabled`, `allow_unverified_packs`,
//! privacy lockdown); the frontend cannot widen it.

use crate::error::LauncherResult;
use agora_core::technic::{self, TechnicPackDetail, TechnicSearchResult};
use tauri::AppHandle;

/// Search Technic, classifying each result into its consent tier.
#[tauri::command]
pub async fn technic_search(
    app: AppHandle,
    query: String,
    limit: Option<u32>,
) -> LauncherResult<Vec<TechnicSearchResult>> {
    let ctx = crate::core_context(&app)?;
    technic::search_technic(&ctx, &query, limit.unwrap_or(20)).await
}

/// Fetch full detail for a Technic pack, with its tier and whether the
/// current consent settings permit installing it.
#[tauri::command]
pub async fn technic_pack_detail(
    app: AppHandle,
    slug: String,
) -> LauncherResult<TechnicPackDetail> {
    let ctx = crate::core_context(&app)?;
    technic::pack_detail(&ctx, &slug).await
}

/// Install a Technic Solder pack (Tier S) as a new instance.
///
/// Resolves the Solder build (recovering the official Forge version from the
/// rehosted `forge` mod entry — never installing Technic's rehosted loader
/// zip), then runs the standard import pipeline (metadata bootstrap, official
/// loader install, DB registration, health scan, snapshot).
#[tauri::command]
pub async fn install_technic_solder_pack(
    app: AppHandle,
    slug: String,
    solder: String,
    build: String,
) -> LauncherResult<agora_core::import::ImportResult> {
    let ctx = crate::core_context(&app)?;
    let pack = technic::resolve_solder_build(&ctx, &solder, &slug, &build).await?;
    technic::install_solder_pack(&ctx, pack).await
}

/// Install a consented Technic zip (Tier Z, or Tier C when `sha256` is pinned)
/// as a new instance.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn install_technic_zip_pack(
    app: AppHandle,
    name: String,
    download_url: String,
    sha256: Option<String>,
    minecraft_version: String,
    loader: String,
    loader_version: String,
) -> LauncherResult<agora_core::import::ImportResult> {
    let ctx = crate::core_context(&app)?;
    let pack = agora_core::import::TechnicZipPack {
        display_name: name,
        download_url,
        sha256: sha256.filter(|value| !value.trim().is_empty()),
        minecraft_version,
        loader,
        loader_version,
    };
    technic::install_zip_pack(&ctx, pack).await
}
