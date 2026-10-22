use crate::auth;
use crate::error::{LauncherError, LauncherResult};
use crate::instances;
use crate::models::{InstalledMod, InstanceManifest, InstanceRow, ModVersionCandidate};
use crate::paths;
use crate::registry;
use std::path::Path;

/// Resolve instance info via core InstanceService.
pub fn load_instance_info(
    app: &tauri::AppHandle,
    instance_id: &str,
) -> LauncherResult<InstanceRow> {
    let ctx = crate::core_context(app)?;
    let svc = agora_core::instance_service::InstanceService::new(ctx);
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
    let svc = agora_core::install_service::InstallService::new(ctx);
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
    let resolver = agora_core::resolver::Resolver::new(ctx);
    resolver
        .list_curated_versions(&item, mc_version, loader)
        .await
}

// The bounded candidate set for update checks is resolved by
// `agora_core::update_cache::check_single_instance_updates_with`, which owns
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
    let resolver = agora_core::resolver::Resolver::new(ctx);
    list_curated_versions_tolerant(
        &resolver,
        &item,
        &instance.minecraft_version,
        &instance.loader,
    )
    .await
}

async fn list_curated_versions_tolerant(
    resolver: &agora_core::resolver::Resolver,
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
) -> agora_core::resolver::Resolver {
    let base = agora_core::resolver::Resolver::new(ctx);
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
    Ok(agora_core::resolver::Resolver::new(ctx).enabled_download_sources(item))
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
pub async fn install_mod_version(
    app: &tauri::AppHandle,
    instance_id: &str,
    item_id: &str,
    candidate: &ModVersionCandidate,
) -> LauncherResult<InstalledMod> {
    let ctx = crate::core_context(app)?;
    let item = load_registry_item(app, item_id)?;
    let content_type = if item.content_type.is_empty() {
        "mod"
    } else {
        &item.content_type
    };
    let exp_sha256 = agora_core::agora_pack_import::expected_sha256_for_candidate(&item, candidate);
    let svc = agora_core::install_service::InstallService::new(ctx);
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
        exp_sha256.as_deref(),
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
        let svc = agora_core::install_service::InstallService::new(ctx);
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

/// Disable artifact via core CrashService.
pub fn disable_instance_mod(
    app: &tauri::AppHandle,
    instance_id: &str,
    filename: &str,
) -> LauncherResult<()> {
    let ctx = crate::core_context(app)?;
    let svc = agora_core::crash_service::CrashService::new(ctx);
    svc.disable_artifact(instance_id, filename)
}

/// Enable artifact via core CrashService.
pub fn enable_instance_mod(
    app: &tauri::AppHandle,
    instance_id: &str,
    filename: &str,
) -> LauncherResult<()> {
    let ctx = crate::core_context(app)?;
    let svc = agora_core::crash_service::CrashService::new(ctx);
    svc.enable_artifact(instance_id, filename)
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
        let svc = agora_core::install_service::InstallService::new(ctx);
        svc.add_manual_artifact(&iid, &sp)
    })
    .await
    .map_err(|_| LauncherError::Generic {
        code: "ERR_MANIFEST_WRITE".into(),
        message: "Manual mod add task failed.".into(),
    })?
}

/// Export pack via core ExportService, to the file the user chose in the save dialog.
pub async fn export_instance_pack(
    app: &tauri::AppHandle,
    instance_id: &str,
    format: &str,
    out_path: &Path,
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
    let manifest: InstanceManifest = {
        let text = std::fs::read_to_string(&manifest_path)
            .map_err(|_| LauncherError::InstanceCreateFailed)?;
        serde_json::from_str(&text).map_err(|_| LauncherError::InstanceCreateFailed)?
    };
    let instance_dir = ctx.paths.instance_dir(instance_id)?;
    agora_core::export_service::export_instance_pack_to(&instance_dir, &manifest, out_path, format)
        .await
}

/// What an import produced: the new instance, and for an Agora pack the
/// per-mod report (installed / failed / needs action) the UI must show.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PackImportOutcome {
    pub instance_id: String,
    pub report: Option<agora_core::agora_pack_import::PackImportReport>,
}

/// Import a pack file.  .mrpack → core ImportService.  .agora-pack.json → local orchestrator.
pub async fn import_instance_pack(
    app: &tauri::AppHandle,
    source_path: &str,
) -> LauncherResult<PackImportOutcome> {
    let lower = source_path.to_ascii_lowercase();
    if lower.ends_with(".mrpack") {
        Ok(PackImportOutcome {
            instance_id: import_mrpack(app, source_path).await?,
            report: None,
        })
    } else if lower.ends_with(".json") || lower.ends_with(".agora-pack.json") {
        import_agora_json(app, source_path).await
    } else {
        Err(LauncherError::Generic {
            code: "ERR_INVALID_FORMAT".into(),
            message: "Unsupported pack file extension. Use .mrpack or .agora-pack.json.".into(),
        })
    }
}

/// Try pack entries again on an existing instance (after the user turned
/// Modrinth on). Entries already installed are not downloaded again.
pub async fn retry_pack_import_entries(
    app: &tauri::AppHandle,
    instance_id: &str,
    entries: Vec<agora_core::agora_pack_import::AgoraPackEntry>,
) -> LauncherResult<agora_core::agora_pack_import::PackImportReport> {
    let ctx = crate::core_context(app)?;
    agora_core::agora_pack_import::retry_agora_pack_entries(&ctx, instance_id, entries).await
}

async fn import_mrpack(app: &tauri::AppHandle, source_path: &str) -> LauncherResult<String> {
    let ctx = crate::core_context(app)?;
    let svc = agora_core::import_service::ImportService::new(ctx);
    let request = agora_core::import_service::ImportRequest {
        source: agora_core::import_service::ImportSource::mrpack(
            Path::new(source_path).to_path_buf(),
        ),
        symlink_saves: false,
    };
    let result = svc.run_import(request).await?;
    Ok(result.instance_id)
}

/// Import an Agora plain-JSON pack (.agora-pack.json).
///
/// Creates the instance, then hands the mods to core, which installs each
/// entry's pinned file, checks it against the pack's hash, and reports every
/// entry's outcome.
async fn import_agora_json(
    app: &tauri::AppHandle,
    source_path: &str,
) -> LauncherResult<PackImportOutcome> {
    let ctx = crate::core_context(app)?;
    let text = std::fs::read_to_string(source_path).map_err(|_| LauncherError::Generic {
        code: "ERR_PACK_READ".into(),
        message: format!("Cannot read pack file: {source_path}"),
    })?;
    let pack = agora_core::agora_pack_import::parse_agora_pack(&text)?;
    let inst = &pack.instance;
    let name = inst
        .name
        .clone()
        .or_else(|| inst.id.clone())
        .unwrap_or_else(|| "imported-pack".to_string());
    let instance_id = paths::sanitize_id(inst.id.as_deref().unwrap_or(&name));
    let instance_id = if instance_id.is_empty() {
        "imported-pack".into()
    } else {
        instance_id
    };
    let req = instances::CreateInstanceRequest {
        name: name.clone(),
        instance_id: instance_id.clone(),
        minecraft_version: inst.minecraft_version.clone(),
        loader: inst.loader.clone(),
        loader_version: inst.loader_version.clone(),
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
    let report =
        agora_core::agora_pack_import::install_agora_pack_mods(&ctx, &instance_id, &name, pack)
            .await;
    Ok(PackImportOutcome {
        instance_id,
        report: Some(report),
    })
}
