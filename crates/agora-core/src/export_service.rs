//! Core-owned ExportService for pack export operations.
//!
//! Owns: instance manifest loading, JSON and mrpack export format generation,
//! file bundling with hash verification, and Modrinth file metadata resolution.

use crate::error::{LauncherError, LauncherResult};
use crate::helpers::{safe_zip_entry_name, stream_jar_into_zip};
use crate::models::InstanceManifest;
use std::io::Write;
use std::path::Path;

/// Export an instance as a shareable pack file.
///
/// - `format == "json"`: a custom `.agora-pack.json` manifest.
/// - `format == "mrpack"`: a `.mrpack` (zip) containing `modrinth.index.json`
///   plus bundled jar files.
///
/// Returns the absolute path to the written export file.
pub async fn export_instance_pack(
    instance_dir: &Path,
    manifest: &InstanceManifest,
    exports_dir: &Path,
    format: &str,
) -> LauncherResult<String> {
    std::fs::create_dir_all(exports_dir).map_err(|_| LauncherError::InstanceCreateFailed)?;
    let safe_id = crate::paths::sanitize_id(&manifest.instance_id);

    match format {
        "json" => {
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
                })).collect::<Vec<_>>(),
            });
            let out_path = exports_dir.join(format!("{}.agora-pack.json", safe_id));
            let tmp_path = out_path.with_extension("json.tmp");
            let text = serde_json::to_string_pretty(&pack)
                .map_err(|_| LauncherError::InstanceCreateFailed)?;
            std::fs::write(&tmp_path, text).map_err(|_| LauncherError::InstanceCreateFailed)?;
            std::fs::rename(&tmp_path, &out_path)
                .map_err(|_| LauncherError::InstanceCreateFailed)?;
            Ok(out_path.to_string_lossy().to_string())
        }
        "mrpack" => {
            let mods_dir = instance_dir.join("mods");
            let out_path = exports_dir.join(format!("{}.mrpack", safe_id));
            let tmp_path = out_path.with_extension("mrpack.tmp");

            {
                let file = std::fs::File::create(&tmp_path)
                    .map_err(|_| LauncherError::InstanceCreateFailed)?;
                let mut zip = zip::ZipWriter::new(file);
                let opts: zip::write::FileOptions = zip::write::FileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored);

                let mut files_meta: Vec<serde_json::Value> = Vec::new();

                for m in &manifest.mods {
                    if let Some(mid) = m.modrinth_id.as_deref().filter(|s| !s.trim().is_empty()) {
                        if let Some(meta) =
                            crate::modrinth::resolve_modrinth_file_metadata(mid, &m.filename).await
                        {
                            files_meta.push(serde_json::json!({
                                "path": format!("mods/{}", m.filename),
                                "hashes": { "sha1": meta.sha1, "sha512": meta.sha512 },
                                "downloads": [meta.url],
                                "fileSize": meta.size,
                            }));
                            continue;
                        }
                    }
                    let entry_name = match safe_zip_entry_name(&m.filename) {
                        Some(n) => n,
                        None => {
                            files_meta.push(serde_json::json!({
                                "path": format!("mods/{}", m.filename),
                                "hashes": { "sha256": m.sha256 },
                                "downloads": [],
                                "fileSize": 0u64,
                            }));
                            continue;
                        }
                    };
                    let p = mods_dir.join(&m.filename);

                    let is_symlink = std::fs::symlink_metadata(&p)
                        .map(|md| md.file_type().is_symlink())
                        .unwrap_or(false);
                    if is_symlink {
                        files_meta.push(serde_json::json!({
                            "path": entry_name,
                            "hashes": { "sha256": m.sha256 },
                            "downloads": [],
                            "fileSize": 0u64,
                        }));
                        continue;
                    }

                    match stream_jar_into_zip(&mut zip, opts, &entry_name, &p) {
                        Ok((sha, size)) => {
                            files_meta.push(serde_json::json!({
                                "path": entry_name,
                                "hashes": { "sha256": sha },
                                "downloads": [],
                                "fileSize": size,
                            }));
                        }
                        Err(_) => {
                            files_meta.push(serde_json::json!({
                                "path": entry_name,
                                "hashes": { "sha256": m.sha256 },
                                "downloads": [],
                                "fileSize": 0u64,
                            }));
                        }
                    }
                }

                let mut deps = serde_json::Map::new();
                deps.insert(
                    "minecraft".to_string(),
                    serde_json::Value::String(manifest.minecraft_version.clone()),
                );
                deps.insert(
                    manifest.loader.clone(),
                    serde_json::Value::String(manifest.loader_version.clone()),
                );
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
                let index_text = serde_json::to_string_pretty(&index)
                    .map_err(|_| LauncherError::InstanceCreateFailed)?;

                zip.start_file("modrinth.index.json", opts)
                    .map_err(|_| LauncherError::InstanceCreateFailed)?;
                zip.write_all(index_text.as_bytes())
                    .map_err(|_| LauncherError::InstanceCreateFailed)?;
                zip.finish()
                    .map_err(|_| LauncherError::InstanceCreateFailed)?;
            }

            std::fs::rename(&tmp_path, &out_path)
                .map_err(|_| LauncherError::InstanceCreateFailed)?;
            Ok(out_path.to_string_lossy().to_string())
        }
        other => Err(LauncherError::Generic {
            code: "ERR_INVALID_FORMAT".into(),
            message: format!("Unknown export format '{}'. Use 'json' or 'mrpack'.", other),
        }),
    }
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
}
