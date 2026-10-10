use crate::auth;
use crate::error::{LauncherError, LauncherResult};
use crate::instances;
use crate::models::{InstalledMod, InstanceRow, ModVersionCandidate};
use crate::paths;
use crate::registry;
use std::path::Path;

/// What importing a pack did with the mods it names.
///
/// A mod that could not be installed is never dropped silently: it is listed in
/// `skipped` with the reason, and the frontend shows the whole list after the
/// import. `unverified` lists installed mods whose source published no checksum.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PackImportResult {
    pub instance_id: String,
    pub skipped: Vec<SkippedPackMod>,
    pub unverified: Vec<String>,
}

/// One mod a pack named that the import could not install.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedPackMod {
    /// The file the pack names, or the registry or Modrinth id when it names none.
    pub name: String,
    /// The error code, such as ERR_HASH_CONFIRMATION_REQUIRED.
    pub code: String,
    /// What the user should know: why the mod was not installed.
    pub message: String,
}

impl SkippedPackMod {
    fn from_error(name: &str, error: &LauncherError) -> Self {
        Self {
            name: name.to_string(),
            code: error.code(),
            message: error.to_string(),
        }
    }
}

/// Record what one pack mod's install came to in the import result: a failure
/// is listed with its reason, and an install whose source published no checksum
/// is listed as unverified. Nothing is dropped.
fn record_pack_mod(
    name: &str,
    strategy: Option<&str>,
    outcome: LauncherResult<InstalledMod>,
    skipped: &mut Vec<SkippedPackMod>,
    unverified: &mut Vec<String>,
) {
    match outcome {
        Ok(installed) => {
            if !installed.hash_verified {
                unverified.push(unverified_notice(strategy, &installed.filename));
            }
        }
        Err(error) => skipped.push(SkippedPackMod::from_error(name, &error)),
    }
}

/// The sentence shown when a source published no checksum for a file.
fn unverified_notice(strategy: Option<&str>, file: &str) -> String {
    let source = match strategy {
        Some("github_release") => "GitHub",
        Some("modrinth_id") | Some("modrinth_raw") => "Modrinth",
        _ => "The source",
    };
    format!("{source} published no checksum for {file}, so Agora could not verify it.")
}

/// Resolve instance info via core InstanceService.
pub fn load_instance_info(
    app: &tauri::AppHandle,
    instance_id: &str,
) -> LauncherResult<InstanceRow> {
    let ctx = crate::core_context(app)?;
    let svc = agora_game_minecraft::instance_service::InstanceService::new(ctx);
    svc.get(instance_id)?
        .map(|detail| detail.row)
        .ok_or_else(|| LauncherError::Generic {
            code: "ERR_INSTANCE_NOT_FOUND".into(),
            message: format!("Instance '{instance_id}' not found."),
        })
}

/// Resolve a registry item via core RegistryService.
pub fn load_registry_item(
    app: &tauri::AppHandle,
    item_id: &str,
) -> LauncherResult<registry::RegistryItem> {
    let ctx = crate::core_context(app)?;
    let svc = agora_core::registry::RegistryService::new(ctx);
    svc.get_item_by_id(item_id)?
        .ok_or_else(|| LauncherError::Generic {
            code: "ERR_ITEM_NOT_FOUND".into(),
            message: format!("Registry item '{item_id}' not found."),
        })
}

/// Check instance is not locked via core InstallService.
pub(crate) fn check_not_locked(app: &tauri::AppHandle, instance_id: &str) -> LauncherResult<()> {
    let ctx = crate::core_context(app)?;
    let svc = agora_game_minecraft::install_service::InstallService::new(ctx);
    svc.check_not_locked(instance_id)
}

/// List versions for a curated registry item via core Resolver.
pub async fn list_mod_versions(
    app: &tauri::AppHandle,
    instance_id: &str,
    item_id: &str,
) -> LauncherResult<Vec<ModVersionCandidate>> {
    let instance = load_instance_info(app, instance_id)?;
    list_mod_versions_for(app, item_id, &instance.minecraft_version, &instance.loader).await
}

/// List versions for a curated registry item against an explicit
/// `(mc_version, loader)` pair.
///
/// Both may be empty, which asks every source for its complete version list
/// rather than a filtered one. That is what the Versions tab needs when it is
/// opened from Browse with no instance chosen: a curated entry is browsable
/// because one of its sources is enabled, so it must be listable on the same
/// terms — requiring an instance first would make an installable entry look
/// broken.
pub async fn list_mod_versions_for(
    app: &tauri::AppHandle,
    item_id: &str,
    mc_version: &str,
    loader: &str,
) -> LauncherResult<Vec<ModVersionCandidate>> {
    let ctx = crate::core_context(app)?;
    let item = load_registry_item(app, item_id)?;
    let resolver = agora_game_minecraft::resolver::Resolver::new(ctx);
    resolver
        .list_curated_versions(&item, mc_version, loader)
        .await
}

// The bounded candidate set for update checks is resolved by
// `agora_game_minecraft::update_cache::check_single_instance_updates_with`, which owns
// the caching and the matching rules for both the background sweep and the
// `check_instance_updates` command. Adapters must not open a second door to
// `Resolver::list_curated_versions_for_update` — see check 9 in
// `scripts/check_architecture.py`.

/// Quick compatibility badge via core Resolver.
pub async fn check_mod_compat(
    app: &tauri::AppHandle,
    instance_id: &str,
    item_id: &str,
) -> LauncherResult<String> {
    let ctx = crate::core_context(app)?;
    let instance = load_instance_info(app, instance_id)?;
    let item = load_registry_item(app, item_id)?;
    let resolver = agora_game_minecraft::resolver::Resolver::new(ctx);
    list_curated_versions_tolerant(
        &resolver,
        &item,
        &instance.minecraft_version,
        &instance.loader,
    )
    .await
}

async fn list_curated_versions_tolerant(
    resolver: &agora_game_minecraft::resolver::Resolver,
    item: &registry::RegistryItem,
    mc_version: &str,
    loader: &str,
) -> LauncherResult<String> {
    let candidates = resolver
        .list_curated_versions(item, mc_version, loader)
        .await
        .unwrap_or_default();
    Ok(candidates
        .iter()
        .map(|c| c.version_compat.as_str())
        .find(|c| !c.is_empty())
        .unwrap_or("")
        .to_string())
}

async fn make_resolver(
    ctx: agora_core::ctx::Ctx,
    app: &tauri::AppHandle,
) -> agora_game_minecraft::resolver::Resolver {
    let base = agora_game_minecraft::resolver::Resolver::new(ctx);
    match auth::get_valid_access_token(app).await {
        Some(tok) => base.with_stored_github_token(tok),
        None => base,
    }
}

/// Bi-directional initial fetch: page 1 + last 3 pages via core Resolver.
///
/// Takes the repo explicitly rather than reading it off the item: an item's
/// preferred `source_identifier` is only a GitHub repo when its preferred
/// source is `github_release`.
pub async fn resolve_github_releases_initial(
    app: &tauri::AppHandle,
    source_identifier: &str,
    mc_version: &str,
    loader: &str,
) -> LauncherResult<(Vec<ModVersionCandidate>, u32, Vec<u32>)> {
    let ctx = crate::core_context(app)?;
    let resolver = make_resolver(ctx, app).await;
    resolver
        .fetch_github_releases_initial(source_identifier, mc_version, loader)
        .await
}

/// An item's download sources in preference order, minus anything the user
/// turned off in Settings.
pub fn enabled_download_sources(
    app: &tauri::AppHandle,
    item: &registry::RegistryItem,
) -> LauncherResult<Vec<agora_core::registry::DownloadSource>> {
    let ctx = crate::core_context(app)?;
    Ok(agora_game_minecraft::resolver::Resolver::new(ctx).enabled_download_sources(item))
}

/// The source an item lists versions from: the curator's preferred source
/// minus anything the user turned off in Settings.
///
/// Every strategy can list without an instance, so this is the same source the
/// install will use — the Versions tab shows what the user will actually get.
/// Returns `None` only when every source is turned off.
pub fn version_listing_source(
    app: &tauri::AppHandle,
    item: &registry::RegistryItem,
) -> LauncherResult<Option<agora_core::registry::DownloadSource>> {
    Ok(enabled_download_sources(app, item)?.into_iter().next())
}

/// Batch-fetch specific GitHub pages via core Resolver.
pub async fn fetch_github_versions_batch(
    app: &tauri::AppHandle,
    source: &str,
    mc_version: &str,
    loader: &str,
    pages: &[u32],
) -> LauncherResult<Vec<(u32, Vec<ModVersionCandidate>)>> {
    let ctx = crate::core_context(app)?;
    let resolver = make_resolver(ctx, app).await;
    resolver
        .fetch_github_versions_batch(source, mc_version, loader, pages)
        .await
}

/// Install a mod version via core InstallService.
///
/// `accept_hash_confirmation` is the user's answer to ERR_HASH_CONFIRMATION_REQUIRED:
/// it lets a download through that differs from a curator pin or from the hash
/// recorded on an earlier install. It never lets a mismatch against the hash
/// the source published through.
pub async fn install_mod_version(
    app: &tauri::AppHandle,
    instance_id: &str,
    item_id: &str,
    candidate: &ModVersionCandidate,
    accept_hash_confirmation: bool,
) -> LauncherResult<InstalledMod> {
    let ctx = crate::core_context(app)?;
    let item = load_registry_item(app, item_id)?;
    let content_type = if item.content_type.is_empty() {
        "mod"
    } else {
        &item.content_type
    };
    // Which hash to expect depends on where this candidate actually came from,
    // not on the item's preferred strategy: an item can resolve from a fallback
    // source, and a Modrinth-served file must not be judged against a hash
    // pinned for the GitHub release.
    let candidate_strategy = candidate
        .source_strategy
        .as_deref()
        .map(str::trim)
        .filter(|strategy| !strategy.is_empty())
        .unwrap_or(item.download_strategy.as_str());
    // The manifest hash identifies the one file of a hand-pinned strategy, so
    // only those are checked against it. Every other source is checked against
    // the hash it published for this file, never against the manifest's, which
    // describes at most one historical file.
    let exp_sha256 = match candidate_strategy {
        "direct_hash" | "technic_pack" => item
            .sha256
            .as_deref()
            .map(str::trim)
            .filter(|hash| !hash.is_empty()),
        _ => candidate.sha256.as_deref(),
    };
    let pinned = agora_game_minecraft::resolver::curator_pins_for(&item, candidate)
        .into_iter()
        .next()
        .map(|pin| pin.sha256);
    let svc = agora_game_minecraft::install_service::InstallService::new(ctx);
    svc.install_artifact(
        instance_id,
        &candidate.filename,
        content_type,
        &candidate.download_url,
        Some(item_id),
        item.modrinth_id.as_deref(),
        "registry",
        Some(&candidate.version),
        candidate.sha1.as_deref(),
        exp_sha256,
        pinned.as_deref(),
        accept_hash_confirmation,
    )
    .await
}

/// Remove artifact via core InstallService.
pub async fn remove_mod_from_instance(
    app: &tauri::AppHandle,
    instance_id: &str,
    filename: &str,
) -> LauncherResult<()> {
    let ctx = crate::core_context(app)?;
    let (iid, fn_own) = (instance_id.to_string(), filename.to_string());
    let removed = tokio::task::spawn_blocking(move || {
        let svc = agora_game_minecraft::install_service::InstallService::new(ctx);
        svc.remove_artifact(&iid, &fn_own)
    })
    .await
    .map_err(|_| LauncherError::Generic {
        code: "ERR_REMOVE_FAILED".into(),
        message: "Remove file task failed.".into(),
    })??;
    auth::log_line(&format!(
        "remove_mod_from_instance: file '{filename}' removed={removed}"
    ));
    Ok(())
}

/// Disable an artifact; a data pack's worlds follow at once.
pub fn disable_instance_mod(
    app: &tauri::AppHandle,
    instance_id: &str,
    filename: &str,
) -> LauncherResult<()> {
    let ctx = crate::core_context(app)?;
    agora_game_minecraft::datapack_sync::set_artifact_enabled(&ctx, instance_id, filename, false)
}

/// Enable an artifact; a data pack's worlds follow at once.
pub fn enable_instance_mod(
    app: &tauri::AppHandle,
    instance_id: &str,
    filename: &str,
) -> LauncherResult<()> {
    let ctx = crate::core_context(app)?;
    agora_game_minecraft::datapack_sync::set_artifact_enabled(&ctx, instance_id, filename, true)
}

/// Add manual .jar via core InstallService.
pub async fn add_manual_mod(
    app: &tauri::AppHandle,
    instance_id: &str,
    source_path: &str,
) -> LauncherResult<InstalledMod> {
    let ctx = crate::core_context(app)?;
    let (iid, sp) = (instance_id.to_string(), source_path.to_string());
    tokio::task::spawn_blocking(move || {
        let svc = agora_game_minecraft::install_service::InstallService::new(ctx);
        svc.add_manual_artifact(&iid, &sp)
    })
    .await
    .map_err(|_| LauncherError::Generic {
        code: "ERR_MANIFEST_WRITE".into(),
        message: "Manual mod add task failed.".into(),
    })?
}

/// Export pack via core ExportService.
pub async fn export_instance_pack(
    app: &tauri::AppHandle,
    instance_id: &str,
    format: &str,
) -> LauncherResult<String> {
    let ctx = crate::core_context(app)?;
    let manifest_path = paths::instance_manifest_path(app, instance_id)
        .map_err(|_| LauncherError::InstanceCreateFailed)?;
    if !manifest_path.exists() {
        return Err(LauncherError::Generic {
            code: "ERR_MANIFEST_MISSING".into(),
            message: "Instance manifest not found.".into(),
        });
    }
    let manifest = agora_core::helpers::read_manifest(&manifest_path)?;
    let instance_dir = ctx.paths.instance_dir(instance_id)?;
    let exports_dir = ctx.paths.root().join("exports");
    agora_game_minecraft::export_service::export_instance_pack(
        &instance_dir,
        &manifest,
        &exports_dir,
        format,
    )
    .await
}

/// Import a pack file.  .mrpack → core ImportService.  .agora-pack.json → local orchestrator.
pub async fn import_instance_pack(
    app: &tauri::AppHandle,
    source_path: &str,
) -> LauncherResult<PackImportResult> {
    let lower = source_path.to_ascii_lowercase();
    if lower.ends_with(".mrpack") {
        import_mrpack(app, source_path).await
    } else if lower.ends_with(".json") || lower.ends_with(".agora-pack.json") {
        import_agora_json(app, source_path).await
    } else {
        Err(LauncherError::Generic {
            code: "ERR_INVALID_FORMAT".into(),
            message: "Unsupported pack file extension. Use .mrpack or .agora-pack.json.".into(),
        })
    }
}

async fn import_mrpack(
    app: &tauri::AppHandle,
    source_path: &str,
) -> LauncherResult<PackImportResult> {
    let ctx = crate::core_context(app)?;
    let svc = agora_game_minecraft::import_service::ImportService::new(ctx);
    let request = agora_game_minecraft::import_service::ImportRequest {
        source: agora_game_minecraft::import_service::ImportSource::mrpack(
            Path::new(source_path).to_path_buf(),
        ),
        symlink_saves: false,
    };
    let result = svc.run_import(request).await?;
    Ok(PackImportResult {
        instance_id: result.instance_id,
        ..Default::default()
    })
}

/// Import an Agora plain-JSON pack (.agora-pack.json).
async fn import_agora_json(
    app: &tauri::AppHandle,
    source_path: &str,
) -> LauncherResult<PackImportResult> {
    let ctx = crate::core_context(app)?;
    let text = std::fs::read_to_string(source_path).map_err(|_| LauncherError::Generic {
        code: "ERR_PACK_READ".into(),
        message: format!("Cannot read pack file: {source_path}"),
    })?;
    let pack: serde_json::Value =
        serde_json::from_str(&text).map_err(|_| LauncherError::Generic {
            code: "ERR_PACK_PARSE".into(),
            message: "Failed to parse agora-pack JSON.".into(),
        })?;
    let inst = pack.get("instance").ok_or_else(|| LauncherError::Generic {
        code: "ERR_PACK_PARSE".into(),
        message: "agora-pack missing 'instance' object.".into(),
    })?;
    let mc_version = inst
        .get("minecraft_version")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| LauncherError::Generic {
            code: "ERR_PACK_PARSE".into(),
            message: "agora-pack missing minecraft_version.".into(),
        })?;
    let loader = inst
        .get("loader")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| LauncherError::Generic {
            code: "ERR_PACK_PARSE".into(),
            message: "agora-pack missing loader.".into(),
        })?;
    let loader_version = inst
        .get("loader_version")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| LauncherError::Generic {
            code: "ERR_PACK_PARSE".into(),
            message: "agora-pack missing loader_version.".into(),
        })?;
    let name = inst
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| {
            inst.get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "imported-pack".to_string());
    let instance_id = paths::sanitize_id(inst.get("id").and_then(|v| v.as_str()).unwrap_or(&name));
    let instance_id = if instance_id.is_empty() {
        "imported-pack".into()
    } else {
        instance_id
    };
    let req = instances::CreateInstanceRequest {
        name: name.clone(),
        instance_id: instance_id.clone(),
        minecraft_version: mc_version.to_string(),
        loader: loader.to_string(),
        loader_version: loader_version.to_string(),
        jvm_memory_mb: Some(4096),
        jvm_memory_mode: Some("auto".into()),
        jvm_gc: None,
        jvm_custom_args: None,
        jvm_always_pre_touch: None,
        is_modpack: None,
        pack_icon_url: None,
        template_id: None,
    };
    instances::create_instance(app.clone(), req).await?;
    // Every mod the pack names ends up either installed, or listed in the
    // result with its reason. Nothing is dropped silently.
    let mut skipped: Vec<SkippedPackMod> = Vec::new();
    let mut unverified: Vec<String> = Vec::new();
    if let Some(mods_arr) = pack.get("mods").and_then(|m| m.as_array()) {
        for entry in mods_arr {
            if let Some(rid) = entry
                .get("registry_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                let filename = entry.get("filename").and_then(|v| v.as_str()).unwrap_or("");
                let label = if filename.is_empty() { rid } else { filename };
                let candidates = match list_mod_versions(app, &instance_id, rid).await {
                    Ok(candidates) => candidates,
                    Err(error) => {
                        skipped.push(SkippedPackMod::from_error(label, &error));
                        continue;
                    }
                };
                let candidate = candidates
                    .iter()
                    .find(|c| c.filename == filename)
                    .or_else(|| {
                        entry
                            .get("version")
                            .and_then(|v| v.as_str())
                            .and_then(|v| candidates.iter().find(|c| c.version == v))
                    });
                let Some(c) = candidate else {
                    skipped.push(SkippedPackMod {
                        name: label.to_string(),
                        code: "ERR_VERSION_NOT_FOUND".into(),
                        message: format!("No version of {rid} matches {label} in the registry."),
                    });
                    continue;
                };
                let outcome = install_mod_version(app, &instance_id, rid, c, false).await;
                record_pack_mod(
                    label,
                    c.source_strategy.as_deref(),
                    outcome,
                    &mut skipped,
                    &mut unverified,
                );
            } else if let Some(mid) = entry
                .get("modrinth_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
            {
                let candidates = match crate::modrinth_raw::list_raw_modrinth_versions(
                    &ctx.http_clients,
                    app,
                    Some(&instance_id),
                    mid,
                    Some("mod"),
                )
                .await
                {
                    Ok(candidates) => candidates,
                    Err(error) => {
                        skipped.push(SkippedPackMod::from_error(mid, &error));
                        continue;
                    }
                };
                let candidate = candidates
                    .iter()
                    .find(|c| c.primary)
                    .or_else(|| candidates.first());
                let Some(c) = candidate else {
                    skipped.push(SkippedPackMod {
                        name: mid.to_string(),
                        code: "ERR_VERSION_NOT_FOUND".into(),
                        message: format!("Modrinth project {mid} has no version to install."),
                    });
                    continue;
                };
                let outcome =
                    crate::modrinth_raw::install_raw_modrinth(app, &instance_id, mid, c, "mod")
                        .await;
                record_pack_mod(
                    mid,
                    Some("modrinth_id"),
                    outcome,
                    &mut skipped,
                    &mut unverified,
                );
            }
        }
    }
    // Stamp PackOrigin for LocalFile pack and persist inventory.
    // Honest identity is display name only; every id stays None so
    // pack-update can distinguish "unknown" from "known".
    let instance_dir = ctx.paths.instance_dir(&instance_id)?;
    let pack_files = agora_game_minecraft::pack_inventory::collect_pack_inventory(&instance_dir)
        .unwrap_or_default();
    let pack_hash = if pack_files.is_empty() {
        None
    } else {
        Some(agora_game_minecraft::pack_inventory::pack_content_hash(
            &pack_files,
        ))
    };
    if let Ok(conn) = agora_core::db::local_state_connection(&ctx.paths.local_state_db()) {
        let _ = agora_core::db::replace_instance_pack_files(&conn, &instance_id, &pack_files);
    }
    let manifest_path = ctx.paths.instance_manifest(&instance_id)?;
    if let Ok(mut manifest) = agora_core::helpers::read_manifest(&manifest_path) {
        let pack_origin = agora_core::models::PackOrigin {
            platform: agora_core::models::PackPlatform::LocalFile,
            pack_name: name.clone(),
            project_id: None,
            version_id: None,
            version_number: None,
            origin_url: None,
            pack_content_hash: pack_hash,
            pack_minecraft_version: Some(mc_version.to_string()),
            pack_loader: Some(loader.to_string()),
            pack_loader_version: Some(loader_version.to_string()),
            launcher_kind: None,
            installation_key: None,
            source_key: None,
            cloned_from: None,
            installed_at: chrono::Utc::now().to_rfc3339(),
        };
        manifest.pack_origin = Some(pack_origin);
        manifest.manifest_version = agora_core::models::CURRENT_MANIFEST_VERSION;
        agora_core::helpers::atomic_write_manifest(&manifest_path, &manifest)?;
    }
    Ok(PackImportResult {
        instance_id,
        skipped,
        unverified,
    })
}

#[cfg(test)]
mod pack_import_tests {
    use super::*;

    fn installed(filename: &str, hash_verified: bool) -> InstalledMod {
        serde_json::from_value(serde_json::json!({
            "filename": filename,
            "source": "registry",
            "sha256": "a".repeat(64),
            "installed_at": "now",
            "hash_verified": hash_verified,
        }))
        .expect("a minimal installed mod record")
    }

    #[test]
    fn a_pack_mod_that_fails_is_reported_with_its_reason_not_dropped() {
        let mut skipped = Vec::new();
        let mut unverified = Vec::new();
        record_pack_mod(
            "xaeros-1.2.0.jar",
            Some("github_release"),
            Err(LauncherError::HashConfirmationRequired(
                agora_core::error::HashConfirmation {
                    file: "xaeros-1.2.0.jar".into(),
                    release: Some("v1.2.0".into()),
                    expected: "c".repeat(64),
                    actual: "d".repeat(64),
                    expected_from: agora_core::artifact_hash::HashOrigin::CuratorPin,
                },
            )),
            &mut skipped,
            &mut unverified,
        );
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].name, "xaeros-1.2.0.jar");
        assert_eq!(skipped[0].code, "ERR_HASH_CONFIRMATION_REQUIRED");
        assert!(skipped[0].message.contains("xaeros-1.2.0.jar"));
        assert!(unverified.is_empty());
    }

    #[test]
    fn an_install_whose_source_published_no_checksum_is_listed_as_unverified() {
        let mut skipped = Vec::new();
        let mut unverified = Vec::new();
        record_pack_mod(
            "old.jar",
            Some("github_release"),
            Ok(installed("old.jar", false)),
            &mut skipped,
            &mut unverified,
        );
        assert!(skipped.is_empty());
        assert_eq!(
            unverified,
            vec![
                "GitHub published no checksum for old.jar, so Agora could not verify it."
                    .to_string()
            ]
        );
    }

    #[test]
    fn a_verified_install_adds_nothing_to_the_report() {
        let mut skipped = Vec::new();
        let mut unverified = Vec::new();
        record_pack_mod(
            "ok.jar",
            Some("modrinth_id"),
            Ok(installed("ok.jar", true)),
            &mut skipped,
            &mut unverified,
        );
        assert!(skipped.is_empty());
        assert!(unverified.is_empty());
    }
}
