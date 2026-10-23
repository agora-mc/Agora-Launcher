//! Core-owned ExportService for pack export operations.
//!
//! Owns: instance manifest loading, JSON and mrpack export format generation,
//! file bundling with hash verification, and Modrinth file metadata resolution.

use crate::error::{LauncherError, LauncherResult};
use crate::helpers::{safe_zip_entry_name, stream_jar_into_zip};
use crate::models::InstanceManifest;
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
    let safe_id = crate::paths::sanitize_id(instance_id);
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
    match format {
        "json" | "mrpack" => {}
        other => return Err(invalid_format_error(other)),
    }
    let tmp_path = tmp_sibling_path(out_path);
    let written = if format == "json" {
        write_json_pack(manifest, &tmp_path)
    } else {
        write_mrpack_pack(instance_dir, manifest, &tmp_path).await
    };
    commit_tmp_file(&tmp_path, out_path, written)?;
    Ok(out_path.to_string_lossy().to_string())
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

async fn write_mrpack_pack(
    instance_dir: &Path,
    manifest: &InstanceManifest,
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
        let (p, on_disk_disabled) = crate::source_identify::mod_file_on_disk(&mods_dir, m)
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
            manifest_version: crate::models::CURRENT_MANIFEST_VERSION,
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
            manifest_version: crate::models::CURRENT_MANIFEST_VERSION,
            pack_origin: None,
            instance_id: "test-mrp".into(),
            name: "Test MRP".into(),
            created_from_pack: None,
            minecraft_version: "1.21".into(),
            loader: "fabric".into(),
            loader_version: "0.16".into(),
            is_locked: false,
            mods: vec![crate::models::InstalledMod {
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
            manifest_version: crate::models::CURRENT_MANIFEST_VERSION,
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
            manifest_version: crate::models::CURRENT_MANIFEST_VERSION,
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
        manifest.mods.push(crate::models::InstalledMod {
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

    fn mod_named(filename: &str) -> crate::models::InstalledMod {
        crate::models::InstalledMod {
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

    fn export_fixture(pack_origin: Option<crate::models::PackOrigin>) -> InstanceManifest {
        InstanceManifest {
            manifest_version: crate::models::CURRENT_MANIFEST_VERSION,
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
        let manifest = export_fixture(Some(crate::models::PackOrigin {
            platform: crate::models::PackPlatform::Modrinth,
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
            manifest.mods = vec![crate::models::InstalledMod {
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
