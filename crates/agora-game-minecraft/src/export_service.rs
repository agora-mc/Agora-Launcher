//! Core-owned ExportService for pack export operations.
//!
//! Owns: instance manifest loading, JSON and mrpack export format generation,
//! file bundling with hash verification, and Modrinth file metadata resolution.

use agora_core::ctx::Ctx;
use agora_core::error::{LauncherError, LauncherResult};
use agora_core::helpers::{safe_zip_entry_name, stream_jar_into_zip};
use agora_core::models::InstanceManifest;
use agora_core::source_identify;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The `modrinth.index.json` `dependencies` key for a loader, or `None` when
/// the format has no key for it (vanilla, or an unrecognised loader).
fn mrpack_dependency_key(loader: &str) -> Option<&'static str> {
    match loader.trim().to_ascii_lowercase().as_str() {
        "fabric" => Some("fabric-loader"),
        "quilt" => Some("quilt-loader"),
        "forge" => Some("forge"),
        "neoforge" => Some("neoforge"),
        _ => None,
    }
}

/// Streamed SHA-1 and SHA-512 of a file, or `None` if it cannot be read.
fn sha1_and_sha512(path: &Path) -> Option<(String, String)> {
    use sha2::Digest;
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut sha1 = sha1::Sha1::new();
    let mut sha512 = sha2::Sha512::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        sha1.update(&buf[..n]);
        sha512.update(&buf[..n]);
    }
    Some((hex::encode(sha1.finalize()), hex::encode(sha512.finalize())))
}

/// The file name an export of `instance_id` gets by default, for `format`.
///
/// Unknown formats are rejected here, so the desktop can refuse before it asks
/// the user where to save.
pub fn default_export_file_name(instance_id: &str, format: &str) -> LauncherResult<String> {
    let safe_id = agora_core::paths::sanitize_id(instance_id);
    if safe_id.is_empty() {
        return Err(LauncherError::Generic {
            code: "ERR_INVALID_INSTANCE".into(),
            message: "The instance ID is invalid.".into(),
        });
    }
    match format {
        "json" => Ok(format!("{}.agora-pack.json", safe_id)),
        "mrpack" => Ok(format!("{}.mrpack", safe_id)),
        other => Err(invalid_format_error(other)),
    }
}

fn invalid_format_error(format: &str) -> LauncherError {
    LauncherError::Generic {
        code: "ERR_INVALID_FORMAT".into(),
        message: format!(
            "Unknown export format '{}'. Use 'json' or 'mrpack'.",
            format
        ),
    }
}

/// Export an instance into `exports_dir` under its default file name.
///
/// Returns the absolute path to the written export file.
pub async fn export_instance_pack(
    instance_dir: &Path,
    manifest: &InstanceManifest,
    exports_dir: &Path,
    format: &str,
) -> LauncherResult<String> {
    std::fs::create_dir_all(exports_dir).map_err(|_| LauncherError::InstanceCreateFailed)?;
    let file_name = default_export_file_name(&manifest.instance_id, format)?;
    export_instance_pack_to(instance_dir, manifest, &exports_dir.join(file_name), format).await
}

/// Export an instance to `out_path`, the file the user chose to save to.
///
/// - `format == "json"`: a custom `.agora-pack.json` manifest.
/// - `format == "mrpack"`: a `.mrpack` (zip) containing `modrinth.index.json`
///   plus bundled jar files.
///
/// The bytes go to a temp file beside `out_path`, which is then renamed into
/// place. On any failure the temp file is removed and `out_path` is untouched.
/// A mod that is neither a Modrinth download nor a readable jar stops the
/// export: the pack could not install it, so it is not written with a gap.
///
/// Returns the absolute path to the written export file.
pub async fn export_instance_pack_to(
    instance_dir: &Path,
    manifest: &InstanceManifest,
    out_path: &Path,
    format: &str,
) -> LauncherResult<String> {
    export_instance_pack_with_hosted(
        instance_dir,
        manifest,
        out_path,
        format,
        &std::collections::HashMap::new(),
    )
    .await
}

/// [`export_instance_pack_to`] where `hosted` names the resource packs and
/// shader packs Modrinth hosts (by SHA-1 of the local file). A `.mrpack` links
/// those; every other pack file is bundled inside it.
pub async fn export_instance_pack_with_hosted(
    instance_dir: &Path,
    manifest: &InstanceManifest,
    out_path: &Path,
    format: &str,
    hosted: &std::collections::HashMap<String, agora_core::source_identify::HostedFile>,
) -> LauncherResult<String> {
    match format {
        "json" | "mrpack" => {}
        other => return Err(invalid_format_error(other)),
    }
    let tmp_path = tmp_sibling_path(out_path);
    let written = if format == "json" {
        write_json_pack(manifest, &tmp_path)
    } else {
        write_mrpack_pack(instance_dir, manifest, hosted, &tmp_path).await
    };
    commit_tmp_file(&tmp_path, out_path, written)?;
    Ok(out_path.to_string_lossy().to_string())
}

/// The manifest an export of `instance_id` is built from, and its instance
/// folder.
///
/// Carries the name the user sees (older renames only reached the database),
/// and, where the user allows Modrinth, a download source for mods that were
/// copied in from another launcher without one. The ids found are written back
/// to the manifest, so the next export does not ask again for them.
pub async fn manifest_for_export(
    ctx: &Ctx,
    instance_id: &str,
) -> LauncherResult<(InstanceManifest, PathBuf)> {
    let manifest_path = ctx.paths.instance_manifest(instance_id)?;
    if !manifest_path.exists() {
        return Err(LauncherError::Generic {
            code: "ERR_MANIFEST_MISSING".into(),
            message: "Instance manifest not found.".into(),
        });
    }
    let mut manifest = agora_core::helpers::read_manifest(&manifest_path)?;
    // Instances renamed before the manifest followed renames still carry the
    // old name there; the exported pack must carry the one the user sees.
    if let Ok(conn) = agora_core::db::local_state_connection(&ctx.paths.local_state_db()) {
        if let Ok(Some(row)) = agora_core::db::get_instance(&conn, instance_id) {
            manifest.name = row.name;
        }
    }
    let instance_dir = ctx.paths.instance_dir(instance_id)?;
    if crate::modrinth::ModrinthService::new(ctx.clone())
        .check_enabled()
        .is_ok()
    {
        let sourceless_before: Vec<bool> = manifest
            .mods
            .iter()
            .map(|item| !source_identify::has_source(item))
            .collect();
        source_identify::identify_mods_on_modrinth(&instance_dir, &mut manifest.mods).await;
        let found: Vec<agora_core::models::InstalledMod> = manifest
            .mods
            .iter()
            .zip(sourceless_before)
            .filter(|(item, was_sourceless)| *was_sourceless && item.modrinth_id.is_some())
            .map(|(item, _)| item.clone())
            .collect();
        if !found.is_empty() {
            // The write is a cache. If it fails, the next export asks again and
            // this export is unaffected, since it uses the ids found just now.
            let (write_ctx, write_id) = (ctx.clone(), instance_id.to_string());
            let _ = tokio::task::spawn_blocking(move || {
                source_identify::record_identified_mods(&write_ctx, &write_id, &found)
            })
            .await;
        }
    }
    Ok((manifest, instance_dir))
}

/// Names of the mods an export of `instance_id` in `format` would leave a
/// friend unable to install. The Agora pack only lists mods, so a mod with no
/// catalog or Modrinth id has nowhere to come from. A `.mrpack` bundles those
/// files, so it has none.
pub async fn sourceless_mods_of_instance(
    ctx: &Ctx,
    instance_id: &str,
    format: &str,
) -> LauncherResult<Vec<String>> {
    if format != "json" {
        return Ok(Vec::new());
    }
    let (manifest, _) = manifest_for_export(ctx, instance_id).await?;
    Ok(source_identify::sourceless_mod_names(&manifest))
}

/// Export `instance_id` in `format` to `out_path`. A `.mrpack` links the
/// resource, shader and datapacks Modrinth hosts, when the user allows Modrinth
/// lookups, and bundles the rest.
pub async fn export_pack_of_instance(
    ctx: &Ctx,
    instance_id: &str,
    format: &str,
    out_path: &Path,
) -> LauncherResult<String> {
    let (manifest, instance_dir) = manifest_for_export(ctx, instance_id).await?;
    let lookups_allowed = crate::modrinth::ModrinthService::new(ctx.clone())
        .check_enabled()
        .is_ok();
    let hosted = if format == "mrpack" && lookups_allowed {
        source_identify::find_hosted_pack_content(&instance_dir).await
    } else {
        std::collections::HashMap::new()
    };
    export_instance_pack_with_hosted(&instance_dir, &manifest, out_path, format, &hosted).await
}

/// A temp file name beside `out_path` (`<name>.tmp`). Staying in the same
/// folder keeps the final rename on one volume.
pub fn tmp_sibling_path(out_path: &Path) -> PathBuf {
    let mut name = out_path.as_os_str().to_owned();
    name.push(".tmp");
    PathBuf::from(name)
}

/// Moves a finished temp file to `out_path`, or removes it if writing failed.
///
/// Either the new file replaces `out_path` in one step, or `out_path` is left as
/// it was and no temp file stays behind.
pub fn commit_tmp_file(
    tmp_path: &Path,
    out_path: &Path,
    written: LauncherResult<()>,
) -> LauncherResult<()> {
    if let Err(error) = written {
        // The write error is the one worth reporting; a failed cleanup is not.
        let _ = std::fs::remove_file(tmp_path);
        return Err(error);
    }
    if std::fs::rename(tmp_path, out_path).is_err() {
        let _ = std::fs::remove_file(tmp_path);
        return Err(LauncherError::Generic {
            code: "ERR_EXPORT_WRITE".into(),
            message: "Could not save the file. Nothing was changed.".into(),
        });
    }
    Ok(())
}

fn write_json_pack(manifest: &InstanceManifest, tmp_path: &Path) -> LauncherResult<()> {
    let pack = serde_json::json!({
        "format": "agora-pack/v1",
        "instance": {
            "id": manifest.instance_id,
            "name": manifest.name,
            "minecraft_version": manifest.minecraft_version,
            "loader": manifest.loader,
            "loader_version": manifest.loader_version,
        },
        "mods": manifest.mods.iter().map(|m| serde_json::json!({
            "filename": m.filename,
            "registry_id": m.registry_id,
            "modrinth_id": m.modrinth_id,
            "source": m.source,
            "version": m.version,
            "sha256": m.sha256,
            "enabled": m.enabled,
        })).collect::<Vec<_>>(),
    });
    let text =
        serde_json::to_string_pretty(&pack).map_err(|_| LauncherError::InstanceCreateFailed)?;
    std::fs::write(tmp_path, text).map_err(|_| LauncherError::InstanceCreateFailed)
}

/// The error for a mod the pack can neither link to nor carry.
fn unbundleable_mod(filename: &str, reason: &str) -> LauncherError {
    LauncherError::Generic {
        code: "ERR_EXPORT_MOD_NOT_BUNDLED".into(),
        message: format!(
            "Could not add {filename} to the pack: {reason}. Fix it or remove the mod, then export again."
        ),
    }
}

/// Every regular file under `root`, as (path relative to `root` with `/`
/// separators, path on disk), sorted. A link stops the export: it cannot be
/// bundled, and leaving it out would hand a friend a pack that differs.
fn pack_content_files(root: &Path) -> LauncherResult<Vec<(String, PathBuf)>> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, PathBuf)>) -> LauncherResult<()> {
        let unreadable = |what: &str| unbundleable_mod(what, "it could not be read");
        let read = std::fs::read_dir(dir).map_err(|_| unreadable(prefix))?;
        for entry in read {
            let entry = entry.map_err(|_| unreadable(prefix))?;
            let file_name = entry.file_name();
            let name = file_name
                .to_str()
                .ok_or_else(|| unbundleable_mod(prefix, "a file name cannot be stored in a pack"))?
                .to_string();
            let rel = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            let kind = entry.file_type().map_err(|_| unreadable(&rel))?;
            if kind.is_symlink() {
                return Err(unbundleable_mod(
                    &rel,
                    "it is a link, and links cannot be bundled",
                ));
            }
            if kind.is_dir() {
                walk(&entry.path(), &rel, out)?;
            } else if kind.is_file() {
                out.push((rel, entry.path()));
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    if root.is_dir() {
        walk(root, "", &mut files)?;
    }
    files.sort();
    Ok(files)
}

async fn write_mrpack_pack(
    instance_dir: &Path,
    manifest: &InstanceManifest,
    hosted: &std::collections::HashMap<String, agora_core::source_identify::HostedFile>,
    tmp_path: &Path,
) -> LauncherResult<()> {
    let mods_dir = instance_dir.join("mods");
    let file = std::fs::File::create(tmp_path).map_err(|_| LauncherError::InstanceCreateFailed)?;
    let mut zip = zip::ZipWriter::new(file);
    let opts: zip::write::SimpleFileOptions =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

    let mut files_meta: Vec<serde_json::Value> = Vec::new();

    for m in &manifest.mods {
        // A disabled mod sits on disk as `<name>.disabled`. The pack keeps it
        // disabled the way Modrinth's own app does: the `.disabled` suffix stays
        // on the file's path in the pack, so it installs switched off.
        let disabled_suffix = if m.enabled { "" } else { ".disabled" };
        if let Some(mid) = m.modrinth_id.as_deref().filter(|s| !s.trim().is_empty()) {
            if let Some(meta) =
                crate::modrinth::resolve_modrinth_file_metadata(mid, &m.filename).await
            {
                files_meta.push(serde_json::json!({
                    "path": format!("mods/{}{}", m.filename, disabled_suffix),
                    "hashes": { "sha1": meta.sha1, "sha512": meta.sha512 },
                    "downloads": [meta.url],
                    "fileSize": meta.size,
                }));
                continue;
            }
        }
        let entry_name = safe_zip_entry_name(&m.filename).ok_or_else(|| {
            unbundleable_mod(&m.filename, "its file name cannot be stored in a pack")
        })?;
        let (p, on_disk_disabled) = agora_core::source_identify::mod_file_on_disk(&mods_dir, m)
            .ok_or_else(|| unbundleable_mod(&m.filename, "it is missing from the mods folder"))?;
        let entry_name = if on_disk_disabled {
            format!("{entry_name}.disabled")
        } else {
            entry_name
        };
        let metadata = std::fs::symlink_metadata(&p)
            .map_err(|_| unbundleable_mod(&m.filename, "it is missing from the mods folder"))?;
        if metadata.file_type().is_symlink() {
            return Err(unbundleable_mod(
                &m.filename,
                "it is a link, and links cannot be bundled",
            ));
        }

        let unreadable =
            || unbundleable_mod(&m.filename, "it could not be read from the mods folder");
        let (sha, size) =
            stream_jar_into_zip(&mut zip, opts, &entry_name, &p).map_err(|_| unreadable())?;
        // The Modrinth format (and Agora's own importer) verifies bundled files
        // by sha1/sha512; sha256 alone made the pack impossible to re-import.
        let (sha1, sha512) = sha1_and_sha512(&p).ok_or_else(unreadable)?;
        files_meta.push(serde_json::json!({
            "path": entry_name,
            "hashes": { "sha256": sha, "sha1": sha1, "sha512": sha512 },
            "downloads": [],
            "fileSize": size,
        }));
    }

    // Resource packs, shader packs and instance-level datapacks are content the user chose (unlike
    // configs and settings, which stay out), so they travel with the pack:
    // linked when Modrinth hosts that exact file, otherwise bundled under
    // `overrides/`, where the format keeps loose files.
    for dir in agora_core::source_identify::PACK_CONTENT_DIRS {
        for (rel, path) in pack_content_files(&instance_dir.join(dir))? {
            let pack_path = format!("{dir}/{rel}");
            let unreadable = || unbundleable_mod(&pack_path, "it could not be read");
            let (sha1, sha512) = sha1_and_sha512(&path).ok_or_else(unreadable)?;
            let top_level = !rel.contains('/');
            // Linked only when both hashes agree, so the link is to these bytes.
            let linked = top_level
                .then(|| hosted.get(&sha1).filter(|file| file.sha512 == sha512))
                .flatten();
            if let Some(file) = linked {
                files_meta.push(serde_json::json!({
                    "path": pack_path,
                    "hashes": { "sha1": file.sha1, "sha512": file.sha512 },
                    "downloads": [file.url],
                    "fileSize": file.size,
                }));
            } else {
                stream_jar_into_zip(&mut zip, opts, &format!("overrides/{pack_path}"), &path)
                    .map_err(|_| unreadable())?;
            }
        }
    }

    let mut deps = serde_json::Map::new();
    deps.insert(
        "minecraft".to_string(),
        serde_json::Value::String(manifest.minecraft_version.clone()),
    );
    // The Modrinth spec keys the loader as `fabric-loader` /
    // `quilt-loader` (not the bare loader name) and has no key for
    // vanilla at all. Writing the bare name made a re-import
    // produce an instance with no loader.
    if let Some(key) = mrpack_dependency_key(&manifest.loader) {
        deps.insert(
            key.to_string(),
            serde_json::Value::String(manifest.loader_version.clone()),
        );
    }
    // `versionId` identifies the *pack* version, not the loader's.
    // Writing `loader_version` here mislabels every exported pack
    // for any launcher that reads it, and since import now records
    // this field as provenance, a round-trip through Agora would
    // store the Fabric version as the pack version.
    //
    // Prefer what we recorded at import so identity survives an
    // export/import round-trip; otherwise stamp the export date,
    // which is at least true. An edited instance still claims its
    // source version — mrpack has no way to express "derived from".
    let version_id = manifest
        .pack_origin
        .as_ref()
        .and_then(|origin| {
            origin
                .version_number
                .clone()
                .or_else(|| origin.version_id.clone())
        })
        .unwrap_or_else(|| chrono::Utc::now().format("%Y.%m.%d").to_string());
    let index = serde_json::json!({
        "formatVersion": 1,
        "game": "minecraft",
        "versionId": version_id,
        "name": manifest.name,
        "dependencies": deps,
        "files": files_meta,
    });
    let index_text =
        serde_json::to_string_pretty(&index).map_err(|_| LauncherError::InstanceCreateFailed)?;

    zip.start_file("modrinth.index.json", opts)
        .map_err(|_| LauncherError::InstanceCreateFailed)?;
    zip.write_all(index_text.as_bytes())
        .map_err(|_| LauncherError::InstanceCreateFailed)?;
    zip.finish()
        .map_err(|_| LauncherError::InstanceCreateFailed)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_json_export_creates_file() {
        let tmp = TempDir::new().unwrap();
        let instance_dir = tmp.path().join("instance");
        std::fs::create_dir_all(instance_dir.join("mods")).unwrap();
        let exports_dir = tmp.path().join("exports");

        let manifest = InstanceManifest {
            manifest_version: agora_core::models::CURRENT_MANIFEST_VERSION,
            game_data: Default::default(),
            pack_origin: None,
            instance_id: "test-export".into(),
            name: "Test Export".into(),
            created_from_pack: None,
            minecraft_version: "1.21".into(),
            loader: "fabric".into(),
            loader_version: "0.16".into(),
            is_locked: false,
            mods: vec![],
            resourcepacks: vec![],
            shaders: vec![],
            datapacks: vec![],
            worlds: vec![],
            user_preferences: serde_json::json!({}),
        };

        let result = export_instance_pack(&instance_dir, &manifest, &exports_dir, "json")
            .await
            .unwrap();
        assert!(result.ends_with(".agora-pack.json"));
        assert!(std::path::Path::new(&result).exists());
    }

    #[tokio::test]
    async fn test_mrpack_export_creates_file() {
        let tmp = TempDir::new().unwrap();
        let instance_dir = tmp.path().join("instance");
        std::fs::create_dir_all(instance_dir.join("mods")).unwrap();
        let exports_dir = tmp.path().join("exports");

        std::fs::write(instance_dir.join("mods").join("test.jar"), b"fake jar").unwrap();

        let manifest = InstanceManifest {
            manifest_version: agora_core::models::CURRENT_MANIFEST_VERSION,
            game_data: Default::default(),
            pack_origin: None,
            instance_id: "test-mrp".into(),
            name: "Test MRP".into(),
            created_from_pack: None,
            minecraft_version: "1.21".into(),
            loader: "fabric".into(),
            loader_version: "0.16".into(),
            is_locked: false,
            mods: vec![agora_core::models::InstalledMod {
                provider: None,
                update_pinned: false,
                pack_managed: false,
                installed_as_dependency: false,
                filename: "test.jar".into(),
                registry_id: None,
                modrinth_id: None,
                source: "test".into(),
                source_url: None,
                version: None,
                sha256: "00".repeat(32),
                hash_verified: true,
                installed_at: String::new(),
                java_packages: vec![],
                mod_jar_id: None,
                depends_on: vec![],
                optional_deps: vec![],
                incompatible_deps: vec![],
                provided_mod_ids: vec![],
                enabled: true,
                content_type: "mod".into(),
            }],
            resourcepacks: vec![],
            shaders: vec![],
            datapacks: vec![],
            worlds: vec![],
            user_preferences: serde_json::json!({}),
        };

        let result = export_instance_pack(&instance_dir, &manifest, &exports_dir, "mrpack")
            .await
            .unwrap();
        assert!(result.ends_with(".mrpack"));
        assert!(std::path::Path::new(&result).exists());
    }

    #[tokio::test]
    async fn test_export_rejects_unknown_format() {
        let tmp = TempDir::new().unwrap();
        let instance_dir = tmp.path().join("instance");
        std::fs::create_dir_all(instance_dir.join("mods")).unwrap();
        let manifest = InstanceManifest {
            manifest_version: agora_core::models::CURRENT_MANIFEST_VERSION,
            game_data: Default::default(),
            pack_origin: None,
            instance_id: "test".into(),
            name: "Test".into(),
            created_from_pack: None,
            minecraft_version: "1.21".into(),
            loader: "fabric".into(),
            loader_version: "0.16".into(),
            is_locked: false,
            mods: vec![],
            resourcepacks: vec![],
            shaders: vec![],
            datapacks: vec![],
            worlds: vec![],
            user_preferences: serde_json::json!({}),
        };
        let err = export_instance_pack(&instance_dir, &manifest, tmp.path(), "zip")
            .await
            .unwrap_err();
        assert_eq!(err.code(), "ERR_INVALID_FORMAT");
    }

    fn empty_manifest(instance_id: &str) -> InstanceManifest {
        InstanceManifest {
            manifest_version: agora_core::models::CURRENT_MANIFEST_VERSION,
            pack_origin: None,
            instance_id: instance_id.into(),
            name: "Test".into(),
            created_from_pack: None,
            minecraft_version: "1.21".into(),
            loader: "fabric".into(),
            loader_version: "0.16".into(),
            is_locked: false,
            mods: vec![],
            resourcepacks: vec![],
            shaders: vec![],
            datapacks: vec![],
            worlds: vec![],
            user_preferences: serde_json::json!({}),
            game_data: Default::default(),
        }
    }

    #[tokio::test]
    async fn export_to_chosen_path_writes_exactly_there_and_leaves_no_tmp() {
        let tmp = TempDir::new().unwrap();
        let instance_dir = tmp.path().join("instance");
        std::fs::create_dir_all(instance_dir.join("mods")).unwrap();
        let chosen = tmp.path().join("my loadout.json");

        let result = export_instance_pack_to(
            &instance_dir,
            &empty_manifest("test-chosen"),
            &chosen,
            "json",
        )
        .await
        .unwrap();

        assert_eq!(std::path::Path::new(&result), chosen.as_path());
        assert!(chosen.is_file());
        let leftovers: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "tmp files left behind: {leftovers:?}");
    }

    #[tokio::test]
    async fn export_to_existing_path_replaces_the_old_file() {
        // The save dialog has already asked the user to confirm the overwrite;
        // the export itself must then replace the file, not fail on it.
        let tmp = TempDir::new().unwrap();
        let instance_dir = tmp.path().join("instance");
        std::fs::create_dir_all(instance_dir.join("mods")).unwrap();
        let chosen = tmp.path().join("pack.agora-pack.json");
        std::fs::write(&chosen, b"old contents").unwrap();

        export_instance_pack_to(
            &instance_dir,
            &empty_manifest("test-replace"),
            &chosen,
            "json",
        )
        .await
        .unwrap();

        let text = std::fs::read_to_string(&chosen).unwrap();
        assert!(
            text.contains("agora-pack/v1"),
            "old file was not replaced: {text}"
        );
    }

    #[test]
    fn default_export_file_names_follow_the_format() {
        assert_eq!(
            default_export_file_name("my-pack", "json").unwrap(),
            "my-pack.agora-pack.json"
        );
        assert_eq!(
            default_export_file_name("my-pack", "mrpack").unwrap(),
            "my-pack.mrpack"
        );
        let err = default_export_file_name("my-pack", "zip").unwrap_err();
        assert_eq!(err.code(), "ERR_INVALID_FORMAT");
    }

    #[test]
    fn default_export_file_name_cannot_escape_the_folder() {
        for hostile in ["../../escape", "a/b", "a\\b", "C:evil", "   "] {
            let name = default_export_file_name(hostile, "json").unwrap();
            assert!(
                !name.contains('/') && !name.contains('\\'),
                "{hostile:?} -> {name:?}"
            );
            assert!(
                !name.contains(':') && !name.starts_with('.'),
                "{hostile:?} -> {name:?}"
            );
        }
        // An empty id must fail rather than name a bare ".agora-pack.json".
        let err = default_export_file_name("", "json").unwrap_err();
        assert_eq!(err.code(), "ERR_INVALID_INSTANCE");
    }

    // REVIEW: expected to FAIL on this branch. A mod the manifest lists but whose
    // jar cannot be read is written as a metadata-only entry, and the export
    // still reports success.
    #[tokio::test]
    async fn review_mrpack_export_fails_when_an_installed_jar_is_missing() {
        let tmp = TempDir::new().unwrap();
        let instance_dir = tmp.path().join("instance");
        std::fs::create_dir_all(instance_dir.join("mods")).unwrap();
        let mut manifest = empty_manifest("test-missing-jar");
        manifest.mods.push(agora_core::models::InstalledMod {
            provider: None,
            update_pinned: false,
            pack_managed: false,
            installed_as_dependency: false,
            filename: "gone.jar".into(),
            registry_id: None,
            modrinth_id: None,
            source: "test".into(),
            source_url: None,
            version: None,
            sha256: "00".repeat(32),
            installed_at: String::new(),
            java_packages: vec![],
            mod_jar_id: None,
            depends_on: vec![],
            optional_deps: vec![],
            incompatible_deps: vec![],
            provided_mod_ids: vec![],
            enabled: true,
            content_type: "mod".into(),
            hash_verified: true,
        });
        let out = tmp.path().join("pack.mrpack");

        let result = export_instance_pack_to(&instance_dir, &manifest, &out, "mrpack").await;

        assert!(
            result.is_err(),
            "export reported success although gone.jar was not bundled"
        );
    }

    // REVIEW: expected to FAIL on this branch. When the final rename fails, the
    // sibling .tmp file is left in the folder the user chose.
    #[tokio::test]
    async fn review_failed_export_leaves_no_tmp_file_in_chosen_folder() {
        let tmp = TempDir::new().unwrap();
        let instance_dir = tmp.path().join("instance");
        std::fs::create_dir_all(instance_dir.join("mods")).unwrap();
        // A folder already sits at the chosen name, so renaming the file onto it fails.
        let chosen = tmp.path().join("pack.agora-pack.json");
        std::fs::create_dir_all(&chosen).unwrap();

        let result = export_instance_pack_to(
            &instance_dir,
            &empty_manifest("test-blocked"),
            &chosen,
            "json",
        )
        .await;
        assert!(result.is_err(), "setup: export onto a folder should fail");

        let leftovers: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "failed export left tmp files behind: {leftovers:?}"
        );
    }

    fn mod_named(filename: &str) -> agora_core::models::InstalledMod {
        agora_core::models::InstalledMod {
            provider: None,
            update_pinned: false,
            pack_managed: false,
            installed_as_dependency: false,
            filename: filename.into(),
            registry_id: None,
            modrinth_id: None,
            source: "test".into(),
            source_url: None,
            version: None,
            sha256: "00".repeat(32),
            installed_at: String::new(),
            java_packages: vec![],
            mod_jar_id: None,
            depends_on: vec![],
            optional_deps: vec![],
            incompatible_deps: vec![],
            provided_mod_ids: vec![],
            enabled: true,
            content_type: "mod".into(),
            hash_verified: true,
        }
    }

    fn folder_has_tmp_files(folder: &std::path::Path) -> bool {
        std::fs::read_dir(folder)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .any(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
    }

    #[tokio::test]
    async fn missing_jar_error_names_the_jar_and_writes_nothing() {
        let tmp = TempDir::new().unwrap();
        let instance_dir = tmp.path().join("instance");
        std::fs::create_dir_all(instance_dir.join("mods")).unwrap();
        let mut manifest = empty_manifest("test-missing");
        manifest.mods.push(mod_named("gone.jar"));
        let out = tmp.path().join("pack.mrpack");

        let err = export_instance_pack_to(&instance_dir, &manifest, &out, "mrpack")
            .await
            .unwrap_err();

        assert_eq!(err.code(), "ERR_EXPORT_MOD_NOT_BUNDLED");
        assert!(err.to_string().contains("gone.jar"), "{err}");
        assert!(!out.exists(), "a failed export must not leave a pack");
        assert!(!folder_has_tmp_files(tmp.path()));
    }

    #[tokio::test]
    async fn agora_pack_round_trip_keeps_a_disabled_mod_disabled() {
        let tmp = TempDir::new().unwrap();
        let instance_dir = tmp.path().join("instance");
        std::fs::create_dir_all(instance_dir.join("mods")).unwrap();
        let mut manifest = empty_manifest("test-json-disabled");
        let mut off = mod_named("off.jar");
        off.enabled = false;
        manifest.mods.push(off);
        manifest.mods.push(mod_named("on.jar"));
        let out = tmp.path().join("pack.agora-pack.json");

        export_instance_pack_to(&instance_dir, &manifest, &out, "json")
            .await
            .unwrap();

        let parsed =
            crate::agora_pack_import::parse_agora_pack(&std::fs::read_to_string(&out).unwrap())
                .unwrap();
        let enabled: Vec<bool> = parsed
            .mods
            .iter()
            .map(|m| match m {
                crate::agora_pack_import::PackEntryParse::Entry(e) => e.arrives_enabled(),
                _ => panic!("unreadable entry"),
            })
            .collect();
        assert_eq!(enabled, vec![false, true]);
    }

    #[tokio::test]
    async fn a_disabled_mod_is_exported_and_stays_disabled_in_the_pack() {
        // The manifest names `iris.jar`; the file on disk is `iris.jar.disabled`.
        let tmp = TempDir::new().unwrap();
        let instance_dir = tmp.path().join("instance");
        std::fs::create_dir_all(instance_dir.join("mods")).unwrap();
        std::fs::write(instance_dir.join("mods").join("iris.jar.disabled"), b"jar").unwrap();
        let mut manifest = empty_manifest("test-disabled");
        let mut iris = mod_named("iris.jar");
        iris.enabled = false;
        manifest.mods.push(iris);
        let out = tmp.path().join("pack.mrpack");

        export_instance_pack_to(&instance_dir, &manifest, &out, "mrpack")
            .await
            .unwrap();

        let index = read_exported_index(out.to_str().unwrap());
        assert_eq!(index["files"][0]["path"], "mods/iris.jar.disabled");
        let file = std::fs::File::open(&out).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        assert!(archive.by_name("mods/iris.jar.disabled").is_ok());
    }

    #[tokio::test]
    async fn mod_name_that_cannot_be_stored_stops_the_export() {
        let tmp = TempDir::new().unwrap();
        let instance_dir = tmp.path().join("instance");
        std::fs::create_dir_all(instance_dir.join("mods")).unwrap();
        // Even if this file existed outside mods/, the pack must not reach for it.
        std::fs::write(tmp.path().join("evil.jar"), b"outside the mods folder").unwrap();
        let mut manifest = empty_manifest("test-traversal");
        manifest.mods.push(mod_named("../evil.jar"));
        let out = tmp.path().join("pack.mrpack");

        let err = export_instance_pack_to(&instance_dir, &manifest, &out, "mrpack")
            .await
            .unwrap_err();

        assert_eq!(err.code(), "ERR_EXPORT_MOD_NOT_BUNDLED");
        assert!(!out.exists());
        assert!(!folder_has_tmp_files(tmp.path()));
    }

    #[test]
    fn failed_write_keeps_the_old_file_the_user_agreed_to_replace() {
        let tmp = TempDir::new().unwrap();
        let out = tmp.path().join("lockfile.json");
        std::fs::write(&out, b"the user's old file").unwrap();
        let tmp_file = tmp_sibling_path(&out);
        std::fs::write(&tmp_file, b"partial new bytes").unwrap();

        let result = commit_tmp_file(
            &tmp_file,
            &out,
            Err(LauncherError::Generic {
                code: "ERR_TEST".into(),
                message: "write failed".into(),
            }),
        );

        assert!(result.is_err());
        assert_eq!(std::fs::read(&out).unwrap(), b"the user's old file");
        assert!(!tmp_file.exists(), "temp file must be removed on failure");
    }

    #[test]
    fn tmp_sibling_path_stays_beside_the_target() {
        let out = Path::new("C:/Users/me/Save Here/my pack.mrpack");
        assert_eq!(
            tmp_sibling_path(out),
            PathBuf::from("C:/Users/me/Save Here/my pack.mrpack.tmp")
        );
    }

    /// Read `modrinth.index.json` back out of an exported `.mrpack`.
    fn read_exported_index(path: &str) -> serde_json::Value {
        let file = std::fs::File::open(path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut entry = archive.by_name("modrinth.index.json").unwrap();
        let mut text = String::new();
        std::io::Read::read_to_string(&mut entry, &mut text).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    fn export_fixture(pack_origin: Option<agora_core::models::PackOrigin>) -> InstanceManifest {
        InstanceManifest {
            manifest_version: agora_core::models::CURRENT_MANIFEST_VERSION,
            game_data: Default::default(),
            pack_origin,
            instance_id: "ver-id".into(),
            name: "Version Id".into(),
            created_from_pack: None,
            minecraft_version: "1.21".into(),
            loader: "fabric".into(),
            loader_version: "0.16.5".into(),
            is_locked: false,
            mods: vec![],
            resourcepacks: vec![],
            shaders: vec![],
            datapacks: vec![],
            worlds: vec![],
            user_preferences: serde_json::json!({}),
        }
    }

    /// Regression: `versionId` identifies the pack, never the loader. Writing
    /// `loader_version` here mislabelled every exported pack, and since import
    /// now reads this field as provenance it would round-trip as a lie.
    #[tokio::test]
    async fn mrpack_export_never_writes_the_loader_version_as_version_id() {
        let tmp = TempDir::new().unwrap();
        let instance_dir = tmp.path().join("instance");
        std::fs::create_dir_all(instance_dir.join("mods")).unwrap();
        let manifest = export_fixture(None);

        let result = export_instance_pack(
            &instance_dir,
            &manifest,
            &tmp.path().join("exports"),
            "mrpack",
        )
        .await
        .unwrap();
        let index = read_exported_index(&result);
        let version_id = index["versionId"].as_str().unwrap();
        assert_ne!(
            version_id, "0.16.5",
            "that is the Fabric version, not the pack's"
        );
        assert!(!version_id.is_empty());
    }

    #[tokio::test]
    async fn mrpack_export_round_trips_a_recorded_pack_version() {
        let tmp = TempDir::new().unwrap();
        let instance_dir = tmp.path().join("instance");
        std::fs::create_dir_all(instance_dir.join("mods")).unwrap();
        let manifest = export_fixture(Some(agora_core::models::PackOrigin {
            platform: agora_core::models::PackPlatform::Modrinth,
            pack_name: "Some Pack".into(),
            project_id: None,
            version_id: Some("abcd1234".into()),
            version_number: Some("1.4.2".into()),
            origin_url: None,
            pack_content_hash: None,
            pack_minecraft_version: None,
            pack_loader: None,
            pack_loader_version: None,
            launcher_kind: None,
            installation_key: None,
            source_key: None,
            cloned_from: None,
            installed_at: "2026-08-31T00:00:00Z".into(),
        }));

        let result = export_instance_pack(
            &instance_dir,
            &manifest,
            &tmp.path().join("exports"),
            "mrpack",
        )
        .await
        .unwrap();
        let index = read_exported_index(&result);
        assert_eq!(
            index["versionId"].as_str().unwrap(),
            "1.4.2",
            "the human version is preferred over the opaque id when both are known"
        );
    }

    fn manifest_with_jar(loader: &str, loader_version: &str, with_mod: bool) -> InstanceManifest {
        let mut manifest = export_fixture(None);
        manifest.minecraft_version = "26.1.2".into();
        manifest.loader = loader.into();
        manifest.loader_version = loader_version.into();
        if with_mod {
            manifest.mods = vec![agora_core::models::InstalledMod {
                provider: None,
                update_pinned: false,
                pack_managed: false,
                installed_as_dependency: false,
                filename: "test.jar".into(),
                registry_id: None,
                modrinth_id: None,
                source: "test".into(),
                source_url: None,
                version: None,
                sha256: "00".repeat(32),
                hash_verified: true,
                installed_at: String::new(),
                java_packages: vec![],
                mod_jar_id: None,
                depends_on: vec![],
                optional_deps: vec![],
                incompatible_deps: vec![],
                provided_mod_ids: vec![],
                enabled: true,
                content_type: "mod".into(),
            }];
        }
        manifest
    }

    /// Regression: the exporter keyed the loader by its bare name ("fabric"),
    /// which the importer (and every other Modrinth-format reader) ignores, so
    /// a round-trip dropped the loader and left a vanilla instance full of mods.
    #[tokio::test]
    async fn mrpack_round_trip_keeps_loader_and_version() {
        for (loader, version, spec_key) in [
            ("fabric", "0.19.5", Some("fabric-loader")),
            ("quilt", "0.27.1", Some("quilt-loader")),
            ("forge", "52.0.1", Some("forge")),
            ("neoforge", "21.1.77", Some("neoforge")),
            ("vanilla", "", None),
        ] {
            let tmp = TempDir::new().unwrap();
            let instance_dir = tmp.path().join("instance");
            std::fs::create_dir_all(instance_dir.join("mods")).unwrap();
            std::fs::write(instance_dir.join("mods").join("test.jar"), b"fake jar").unwrap();
            let manifest = manifest_with_jar(loader, version, loader != "vanilla");

            let exported = export_instance_pack(
                &instance_dir,
                &manifest,
                &tmp.path().join("exports"),
                "mrpack",
            )
            .await
            .unwrap();

            let deps = read_exported_index(&exported)["dependencies"].clone();
            assert_eq!(deps["minecraft"], "26.1.2");
            match spec_key {
                Some(key) => assert_eq!(deps[key], version, "{loader}: {deps}"),
                None => assert_eq!(
                    deps.as_object().unwrap().len(),
                    1,
                    "vanilla must not write a loader key: {deps}"
                ),
            }

            let imported =
                crate::import::import_mrpack(Path::new(&exported), &tmp.path().join("in"), false)
                    .unwrap();
            assert_eq!(imported.minecraft_version, "26.1.2");
            assert_eq!(
                imported.loader,
                if loader == "vanilla" { "" } else { loader }
            );
            assert_eq!(imported.loader_version, version, "{loader}");
        }
    }
}

#[cfg(test)]
mod pack_content_tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::TempDir;

    fn manifest() -> InstanceManifest {
        serde_json::from_value(serde_json::json!({
            "instance_id": "spiff", "name": "Spiff", "minecraft_version": "1.20.4",
            "loader": "fabric", "loader_version": "0.15.0", "mods": [],
        }))
        .unwrap()
    }

    fn read_index(path: &Path) -> serde_json::Value {
        let mut archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        let entry = archive.by_name("modrinth.index.json").unwrap();
        serde_json::from_reader(entry).unwrap()
    }

    fn instance_with_packs(tmp: &TempDir) -> PathBuf {
        let dir = tmp.path().join("instance");
        std::fs::create_dir_all(dir.join("shaderpacks")).unwrap();
        std::fs::create_dir_all(dir.join("resourcepacks")).unwrap();
        std::fs::write(
            dir.join("shaderpacks").join("Complementary.zip"),
            b"shader one",
        )
        .unwrap();
        std::fs::write(dir.join("shaderpacks").join("BSL.zip"), b"shader two").unwrap();
        std::fs::write(dir.join("resourcepacks").join("Faithful.zip"), b"textures").unwrap();
        std::fs::create_dir_all(dir.join("datapacks")).unwrap();
        std::fs::write(dir.join("datapacks").join("Tweaks.zip"), b"data").unwrap();
        // A world's own datapacks are part of that world, not instance content.
        std::fs::create_dir_all(dir.join("saves").join("W").join("datapacks")).unwrap();
        std::fs::write(
            dir.join("saves").join("W").join("datapacks").join("w.zip"),
            b"w",
        )
        .unwrap();
        // Settings are not content the user chose; they stay out.
        std::fs::create_dir_all(dir.join("config")).unwrap();
        std::fs::write(dir.join("config").join("sodium.json"), b"{}").unwrap();
        dir
    }

    #[tokio::test]
    async fn shader_and_resource_packs_travel_inside_the_mrpack_and_come_back_on_import() {
        let tmp = TempDir::new().unwrap();
        let dir = instance_with_packs(&tmp);
        let out = tmp.path().join("pack.mrpack");

        export_instance_pack_to(&dir, &manifest(), &out, "mrpack")
            .await
            .unwrap();

        let mut archive = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
        assert!(archive
            .by_name("overrides/shaderpacks/Complementary.zip")
            .is_ok());
        assert!(archive.by_name("overrides/shaderpacks/BSL.zip").is_ok());
        assert!(archive
            .by_name("overrides/resourcepacks/Faithful.zip")
            .is_ok());
        assert!(archive.by_name("overrides/datapacks/Tweaks.zip").is_ok());
        assert!(
            archive
                .by_name("overrides/saves/W/datapacks/w.zip")
                .is_err(),
            "per-world datapacks stay with the world"
        );
        assert!(
            archive.by_name("overrides/config/sodium.json").is_err(),
            "configs are not exported"
        );

        let imported = crate::import::import_mrpack(&out, &tmp.path().join("in"), false).unwrap();
        let root = tmp.path().join("in").join(&imported.instance_id);
        assert!(root.join("shaderpacks").join("Complementary.zip").is_file());
        assert!(root.join("shaderpacks").join("BSL.zip").is_file());
        assert!(root.join("resourcepacks").join("Faithful.zip").is_file());
        assert!(root.join("datapacks").join("Tweaks.zip").is_file());
    }

    #[tokio::test]
    async fn a_hosted_match_on_sha1_alone_is_bundled_when_the_sha512_differs() {
        let tmp = TempDir::new().unwrap();
        let dir = instance_with_packs(&tmp);
        let out = tmp.path().join("pack.mrpack");
        let (sha1, _local_sha512) =
            sha1_and_sha512(&dir.join("shaderpacks").join("BSL.zip")).unwrap();
        // Same SHA-1, different SHA-512: these are not the bytes on disk.
        let mut hosted = HashMap::new();
        hosted.insert(
            sha1.clone(),
            agora_core::source_identify::HostedFile {
                url: "https://cdn.modrinth.com/data/abc/versions/def/BSL.zip".into(),
                sha1: sha1.clone(),
                sha512: "0".repeat(128),
                size: 10,
            },
        );

        export_instance_pack_with_hosted(&dir, &manifest(), &out, "mrpack", &hosted)
            .await
            .unwrap();

        let index = read_index(&out);
        let files = index["files"].as_array().unwrap();
        assert!(
            files.iter().all(|f| f["path"] != "shaderpacks/BSL.zip"),
            "a file whose SHA-512 differs must not be linked: {index}"
        );
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
        assert!(
            archive.by_name("overrides/shaderpacks/BSL.zip").is_ok(),
            "the local bytes are bundled instead"
        );
    }

    #[tokio::test]
    async fn a_pack_modrinth_hosts_is_linked_by_hash_and_the_rest_is_bundled() {
        let tmp = TempDir::new().unwrap();
        let dir = instance_with_packs(&tmp);
        let out = tmp.path().join("pack.mrpack");
        let (sha1, sha512) = sha1_and_sha512(&dir.join("shaderpacks").join("BSL.zip")).unwrap();
        let mut hosted = HashMap::new();
        hosted.insert(
            sha1.clone(),
            agora_core::source_identify::HostedFile {
                url: "https://cdn.modrinth.com/data/abc/versions/def/BSL.zip".into(),
                sha1: sha1.clone(),
                sha512,
                size: 10,
            },
        );

        export_instance_pack_with_hosted(&dir, &manifest(), &out, "mrpack", &hosted)
            .await
            .unwrap();

        let index = read_index(&out);
        let files = index["files"].as_array().unwrap();
        assert_eq!(files.len(), 1, "{index}");
        assert_eq!(files[0]["path"], "shaderpacks/BSL.zip");
        assert_eq!(
            files[0]["downloads"][0],
            "https://cdn.modrinth.com/data/abc/versions/def/BSL.zip"
        );
        assert_eq!(files[0]["hashes"]["sha1"], sha1.as_str());
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
        assert!(
            archive.by_name("overrides/shaderpacks/BSL.zip").is_err(),
            "a linked pack is not also bundled"
        );
        assert!(archive
            .by_name("overrides/shaderpacks/Complementary.zip")
            .is_ok());
        assert!(archive
            .by_name("overrides/resourcepacks/Faithful.zip")
            .is_ok());
    }

    #[tokio::test]
    async fn an_unzipped_shader_folder_is_bundled_whole() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("instance");
        std::fs::create_dir_all(dir.join("shaderpacks").join("MyShader").join("shaders")).unwrap();
        std::fs::write(
            dir.join("shaderpacks")
                .join("MyShader")
                .join("shaders")
                .join("final.fsh"),
            b"void main(){}",
        )
        .unwrap();
        let out = tmp.path().join("pack.mrpack");

        export_instance_pack_to(&dir, &manifest(), &out, "mrpack")
            .await
            .unwrap();

        let mut archive = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
        assert!(archive
            .by_name("overrides/shaderpacks/MyShader/shaders/final.fsh")
            .is_ok());
    }

    #[test]
    fn modrinth_reply_gives_the_hosted_file_behind_each_hash() {
        let reply = serde_json::json!({
            "AAA1": { "project_id": "p", "files": [
                { "hashes": { "sha1": "bbb2", "sha512": "x" }, "url": "https://cdn.modrinth.com/other.zip", "size": 1 },
                { "hashes": { "sha1": "aaa1", "sha512": "S512" }, "url": "https://cdn.modrinth.com/data/p/versions/v/a.zip", "size": 42 }
            ]},
            "ccc3": { "files": [
                { "hashes": { "sha1": "ccc3", "sha512": "s" }, "url": "https://evil.example.com/c.zip", "size": 1 }
            ]}
        });
        let hosted = agora_core::source_identify::parse_hosted_files(&reply);
        assert_eq!(hosted.len(), 1, "{hosted:?}");
        let file = &hosted["aaa1"];
        assert_eq!(file.url, "https://cdn.modrinth.com/data/p/versions/v/a.zip");
        assert_eq!(file.sha512, "s512");
        assert_eq!(file.size, 42);
    }
}

#[cfg(test)]
mod round_trip_tests {
    use super::*;
    use std::io::{Read, Write};
    use tempfile::TempDir;

    fn mod_entry(
        filename: &str,
        modrinth_id: Option<&str>,
        enabled: bool,
    ) -> agora_core::models::InstalledMod {
        serde_json::from_value(serde_json::json!({
            "filename": filename, "registry_id": null, "modrinth_id": modrinth_id,
            "source": "test", "version": null, "sha256": "00", "installed_at": "t",
            "enabled": enabled,
        }))
        .unwrap()
    }

    /// Our own export, planned and imported by our own importer: nothing it
    /// writes may be a hard problem, and everything comes back.
    #[tokio::test]
    async fn an_export_with_bundled_disabled_and_shader_content_plans_clean_and_imports_whole() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("instance");
        std::fs::create_dir_all(dir.join("mods")).unwrap();
        std::fs::create_dir_all(dir.join("shaderpacks")).unwrap();
        // (b) a mod Modrinth cannot identify, bundled; (c) a disabled one.
        std::fs::write(dir.join("mods").join("curseforge-only.jar"), b"cf jar").unwrap();
        std::fs::write(dir.join("mods").join("off.jar.disabled"), b"off jar").unwrap();
        // (d) a shader pack.
        std::fs::write(dir.join("shaderpacks").join("BSL.zip"), b"shader").unwrap();
        let mut manifest: InstanceManifest = serde_json::from_value(serde_json::json!({
            "instance_id": "rt", "name": "RT", "minecraft_version": "1.20.4",
            "loader": "fabric", "loader_version": "0.15.0", "mods": [],
        }))
        .unwrap();
        manifest
            .mods
            .push(mod_entry("curseforge-only.jar", None, true));
        manifest.mods.push(mod_entry("off.jar", None, false));
        let out = tmp.path().join("rt.mrpack");
        export_instance_pack_to(&dir, &manifest, &out, "mrpack")
            .await
            .unwrap();

        // (a) a Modrinth-identified mod: the export links it through a network
        // lookup, so add the entry that lookup would have written.
        let with_modrinth = tmp.path().join("rt-modrinth.mrpack");
        {
            let mut src = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
            let mut writer = zip::ZipWriter::new(std::fs::File::create(&with_modrinth).unwrap());
            let opts = zip::write::SimpleFileOptions::default();
            for i in 0..src.len() {
                let mut entry = src.by_index(i).unwrap();
                let name = entry.name().unwrap().to_string();
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).unwrap();
                if name == "modrinth.index.json" {
                    let mut index: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                    index["files"].as_array_mut().unwrap().push(serde_json::json!({
                        "path": "mods/sodium.jar",
                        "hashes": { "sha1": "da39a3ee5e6b4b0d3255bfef95601890afd80709", "sha512": "x" },
                        "downloads": ["https://cdn.modrinth.com/data/AANobbMI/versions/v/sodium.jar"],
                        "fileSize": 1,
                    }));
                    bytes = serde_json::to_vec(&index).unwrap();
                }
                writer.start_file(name, opts).unwrap();
                writer.write_all(&bytes).unwrap();
            }
            writer.finish().unwrap();
        }

        let ctx = agora_core::ctx::CoreContext::for_testing(tmp.path().join("data"));
        let _ = agora_core::db::init_local_state_db(&ctx.paths.local_state_db());
        let plan = crate::import_service::ImportService::new(ctx)
            .plan_mrpack_import(&with_modrinth)
            .unwrap();
        assert!(plan.problems.is_empty(), "hard: {:?}", plan.problems);
        assert!(
            plan.soft_problems.is_empty(),
            "soft: {:?}",
            plan.soft_problems
        );
        let source_of = |n: &str| {
            plan.entries
                .iter()
                .find(|e| e.name == n)
                .unwrap_or_else(|| panic!("{n} not in {:?}", plan.entries))
                .source
        };
        use crate::agora_pack_import::PlannedSource;
        assert_eq!(source_of("mods/sodium.jar"), PlannedSource::Modrinth);
        assert_eq!(
            source_of("mods/curseforge-only.jar"),
            PlannedSource::Bundled
        );
        assert_eq!(source_of("mods/off.jar.disabled"), PlannedSource::Bundled);
        assert!(plan
            .entries
            .iter()
            .any(|e| e.name == "mods/off.jar.disabled" && e.arrives_disabled));
        assert_eq!(source_of("shaderpacks/BSL.zip"), PlannedSource::Bundled);

        // The import of the file we exported reproduces all of it.
        let imported = crate::import::import_mrpack(&out, &tmp.path().join("in"), false).unwrap();
        let root = tmp.path().join("in").join(&imported.instance_id);
        assert!(root.join("mods").join("curseforge-only.jar").is_file());
        assert!(root.join("mods").join("off.jar.disabled").is_file());
        assert!(
            !root.join("mods").join("off.jar").exists(),
            "disabled stays disabled"
        );
        assert!(root.join("shaderpacks").join("BSL.zip").is_file());

        // The manifest, not just the files: a disabled mod is recorded under its
        // logical name and switched off, so the mod table and the State filter
        // see it as disabled.
        let manifest =
            agora_core::helpers::read_manifest(&root.join("instance_manifest.json")).unwrap();
        let mods: Vec<(&str, bool)> = manifest
            .mods
            .iter()
            .map(|m| (m.filename.as_str(), m.enabled))
            .collect();
        assert_eq!(
            mods,
            vec![("curseforge-only.jar", true), ("off.jar", false)],
            "mods as recorded in the manifest"
        );
        let shaders: Vec<(&str, bool)> = manifest
            .shaders
            .iter()
            .map(|m| (m.filename.as_str(), m.enabled))
            .collect();
        assert_eq!(shaders, vec![("BSL.zip", true)]);
    }
}
