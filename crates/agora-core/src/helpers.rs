use crate::ctx::Ctx;
use crate::error::{LauncherError, LauncherResult};
use crate::models::{
    heal_pack_managed, InstalledMod, InstanceManifest, PackOrigin, PackPlatform,
    CURRENT_MANIFEST_VERSION,
};
use std::path::Path;

// ---------------------------------------------------------------------------
// Child process helpers — no flashing console windows on Windows
// ---------------------------------------------------------------------------

/// Windows `CREATE_NO_WINDOW` creation flag: the spawned process runs without
/// a console window. The GUI app has no console, so spawning console-subsystem
/// programs (java, taskkill, fsutil) without this flag briefly flashes an
/// empty command prompt window for each invocation.
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Suppress the console window for a synchronous child process spawn.
#[cfg(target_os = "windows")]
pub fn hide_console_window(cmd: &mut std::process::Command) -> &mut std::process::Command {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(CREATE_NO_WINDOW)
}

/// Suppress the console window for an async (`tokio::process`) child spawn.
#[cfg(target_os = "windows")]
pub fn hide_console_window_async(
    cmd: &mut tokio::process::Command,
) -> &mut tokio::process::Command {
    cmd.creation_flags(CREATE_NO_WINDOW)
}

#[cfg(not(target_os = "windows"))]
pub fn hide_console_window(cmd: &mut std::process::Command) -> &mut std::process::Command {
    cmd
}

#[cfg(not(target_os = "windows"))]
pub fn hide_console_window_async(
    cmd: &mut tokio::process::Command,
) -> &mut tokio::process::Command {
    cmd
}

/// Map a content_type string to the instance subdirectory name.
pub fn content_subdir(content_type: &str) -> &str {
    match content_type {
        "resourcepack" => "resourcepacks",
        "shader" => "shaderpacks",
        "datapack" => "datapacks",
        "world" => "saves",
        _ => "mods",
    }
}

const CONTENT_SUBDIRS: &[&str] = &["mods", "resourcepacks", "shaderpacks", "datapacks", "saves"];

/// Push an installed item to the correct array in the manifest.
pub fn push_to_content_array(manifest: &mut InstanceManifest, item: &InstalledMod) {
    match item.content_type.as_str() {
        "resourcepack" => manifest.resourcepacks.push(item.clone()),
        "shader" => manifest.shaders.push(item.clone()),
        "datapack" => manifest.datapacks.push(item.clone()),
        "world" => manifest.worlds.push(item.clone()),
        _ => manifest.mods.push(item.clone()),
    }
}

/// Remove an entry with the given filename from whichever manifest array it
/// resides in. Returns `true` if found and removed.
pub fn remove_from_content_array(manifest: &mut InstanceManifest, filename: &str) -> bool {
    for arr in [
        &mut manifest.mods,
        &mut manifest.resourcepacks,
        &mut manifest.shaders,
        &mut manifest.datapacks,
        &mut manifest.worlds,
    ] {
        let before = arr.len();
        arr.retain(|m| m.filename != filename);
        if arr.len() < before {
            return true;
        }
    }
    false
}

/// Set `enabled` on the manifest entry matching `filename` across all arrays.
pub fn set_enabled_in_all_arrays(
    manifest: &mut InstanceManifest,
    filename: &str,
    enabled: bool,
) -> bool {
    for arr in [
        &mut manifest.mods,
        &mut manifest.resourcepacks,
        &mut manifest.shaders,
        &mut manifest.datapacks,
        &mut manifest.worlds,
    ] {
        if let Some(entry) = arr.iter_mut().find(|m| m.filename == filename) {
            entry.enabled = enabled;
            return true;
        }
    }
    false
}

/// Read the instance manifest and return `Err(InstanceLocked)` if `is_locked` is true.
pub fn check_not_locked(ctx: &Ctx, instance_id: &str) -> LauncherResult<()> {
    let manifest_path = ctx.paths.instance_manifest(instance_id)?;
    if !manifest_path.exists() {
        return Ok(());
    }
    let manifest = read_manifest(&manifest_path)?;
    if manifest.is_read_only() {
        return Err(manifest_read_only_error());
    }
    if manifest.is_locked {
        return Err(LauncherError::InstanceLocked);
    }
    Ok(())
}

/// Validate a mod filename and return its zip entry name (`mods/<filename>`).
/// Returns `None` for names that could escape the `mods/` directory.
pub fn safe_zip_entry_name(filename: &str) -> Option<String> {
    if filename.is_empty()
        || filename == "."
        || filename == ".."
        || filename.contains('/')
        || filename.contains('\\')
        || filename.contains('\0')
    {
        return None;
    }
    Some(format!("mods/{}", filename))
}

fn manifest_read_only_error() -> LauncherError {
    LauncherError::Generic {
        code: "ERR_MANIFEST_READ_ONLY".into(),
        message: "This instance manifest was written by a newer Agora. Open it with that version to make changes.".into(),
    }
}

fn disk_manifest_version(value: &serde_json::Value) -> u64 {
    value
        .get("manifest_version")
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManifestWriteStage {
    BackupTemporarySynced,
    OriginalBackedUp,
    TemporarySynced,
    Replaced,
}

/// Atomic manifest write helper. Legacy upgrades stay lazy, save the exact
/// original beside the manifest, then sync a temporary file and rename it.
/// Both the supplied object and the current disk version are checked: a stale
/// in-memory v3 value must not overwrite a future manifest written meanwhile.
pub fn atomic_write_manifest(
    manifest_path: &Path,
    manifest: &InstanceManifest,
) -> LauncherResult<()> {
    write_manifest_with_hook(manifest_path, manifest, |_| Ok(()))
}

fn synced_manifest_temp(manifest_path: &Path, bytes: &[u8]) -> LauncherResult<std::path::PathBuf> {
    use std::io::Write;
    let tmp_path = manifest_path.with_extension(format!("json.{}.tmp", uuid::Uuid::new_v4()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp_path)
        .map_err(|_| LauncherError::InstanceCreateFailed)?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| LauncherError::InstanceCreateFailed)?;
    Ok(tmp_path)
}

fn write_manifest_with_hook(
    manifest_path: &Path,
    manifest: &InstanceManifest,
    mut checkpoint: impl FnMut(ManifestWriteStage) -> LauncherResult<()>,
) -> LauncherResult<()> {
    if manifest.is_read_only() {
        return Err(manifest_read_only_error());
    }
    let original = match std::fs::read(manifest_path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err(LauncherError::InstanceCreateFailed),
    };
    let original_value = original
        .as_ref()
        .map(|bytes| serde_json::from_slice::<serde_json::Value>(bytes))
        .transpose()
        .map_err(|_| LauncherError::InstanceCreateFailed)?;
    if original_value
        .as_ref()
        .is_some_and(|v| disk_manifest_version(v) > u64::from(CURRENT_MANIFEST_VERSION))
    {
        return Err(manifest_read_only_error());
    }
    let value = manifest
        .to_disk_value()
        .map_err(|_| LauncherError::InstanceCreateFailed)?;
    // Re-running the upgrade must preserve bytes and timestamps, including
    // an already-current file's formatting, when no actual edit was made.
    if original_value.as_ref() == Some(&value) {
        return Ok(());
    }
    if let (Some(bytes), Some(old_value)) = (&original, &original_value) {
        let old_version = disk_manifest_version(old_value);
        if old_version < u64::from(CURRENT_MANIFEST_VERSION) {
            let backup = manifest_path.with_extension(format!("json.v{old_version}.bak"));
            if backup.exists() {
                if std::fs::read(&backup).map_err(|_| LauncherError::InstanceCreateFailed)?
                    != *bytes
                {
                    return Err(LauncherError::MigrationConflict {
                        message: format!(
                            "Manifest backup '{}' differs from the original; both were kept.",
                            backup.display()
                        ),
                    });
                }
            } else {
                // Publish the backup only after all its bytes are durable. A
                // partial temporary backup after a crash is never treated as it.
                let temp = synced_manifest_temp(manifest_path, bytes)?;
                checkpoint(ManifestWriteStage::BackupTemporarySynced)?;
                std::fs::rename(&temp, &backup).map_err(|_| LauncherError::InstanceCreateFailed)?;
            }
            checkpoint(ManifestWriteStage::OriginalBackedUp)?;
        }
    }
    let text =
        serde_json::to_vec_pretty(&value).map_err(|_| LauncherError::InstanceCreateFailed)?;
    let tmp_path = synced_manifest_temp(manifest_path, &text)?;
    checkpoint(ManifestWriteStage::TemporarySynced)?;
    std::fs::rename(&tmp_path, manifest_path).map_err(|_| LauncherError::InstanceCreateFailed)?;
    checkpoint(ManifestWriteStage::Replaced)?;
    Ok(())
}

/// Read the instance manifest from disk, healing legacy fields lazily.
///
/// After deserializing, runs `heal_pack_managed` and, if `pack_origin` is
/// still `None` but `created_from_pack` is `Some(name)`, synthesizes a
/// name-only `PackOrigin { platform: Unknown, pack_name: name, .. }`.
/// The healed value exists only in memory; the file is rewritten only on
/// the next `atomic_write_manifest`, so there is no startup mass-rewrite.
/// `created_from_pack == None` is treated as UNKNOWN, not "user-created".
pub fn read_manifest(manifest_path: &Path) -> LauncherResult<InstanceManifest> {
    if !manifest_path.exists() {
        return Err(LauncherError::Generic {
            code: "ERR_MANIFEST_MISSING".to_string(),
            message: format!(
                "Instance manifest not found at '{}'. Create the instance first.",
                manifest_path.display()
            ),
        });
    }
    let text =
        std::fs::read_to_string(manifest_path).map_err(|_| LauncherError::InstanceCreateFailed)?;
    let mut manifest: InstanceManifest =
        serde_json::from_str(&text).map_err(|_| LauncherError::InstanceCreateFailed)?;
    if manifest.is_read_only() {
        return Ok(manifest);
    }
    // Lazy backfill: heal pack_managed flags for pre-v2 manifests.
    heal_pack_managed(&mut manifest);
    // Synthesize a name-only PackOrigin for legacy instances that only had
    // `created_from_pack`. Do not invent when it is None — that is UNKNOWN.
    if manifest.pack_origin.is_none() {
        if let Some(name) = manifest.created_from_pack.clone() {
            if !name.trim().is_empty() {
                manifest.pack_origin = Some(PackOrigin {
                    platform: PackPlatform::Unknown,
                    pack_name: name,
                    project_id: None,
                    version_id: None,
                    version_number: None,
                    origin_url: None,
                    pack_content_hash: None,
                    pack_minecraft_version: None,
                    pack_loader: None,
                    pack_loader_version: None,
                    launcher_kind: None,
                    installation_key: None,
                    source_key: None,
                    cloned_from: None,
                    installed_at: chrono::Utc::now().to_rfc3339(),
                });
            }
        }
    }
    manifest.refresh_game_projection();
    Ok(manifest)
}

/// Stream a single file into the zip writer, computing SHA-256 + size as bytes
/// flow through. Peak memory is bounded by `CHUNK` rather than the full file.
pub fn stream_jar_into_zip(
    zip: &mut zip::ZipWriter<std::fs::File>,
    opts: zip::write::FileOptions,
    entry_name: &str,
    path: &std::path::Path,
) -> LauncherResult<(String, u64)> {
    use sha2::Digest;
    use std::io::{Read, Write};
    const CHUNK: usize = 64 * 1024;

    let mut f = std::fs::File::open(path).map_err(|_| LauncherError::InstanceCreateFailed)?;
    zip.start_file(entry_name, opts)
        .map_err(|_| LauncherError::InstanceCreateFailed)?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = [0u8; CHUNK];
    let mut size: u64 = 0;
    loop {
        let n = f
            .read(&mut buf)
            .map_err(|_| LauncherError::InstanceCreateFailed)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        zip.write_all(&buf[..n])
            .map_err(|_| LauncherError::InstanceCreateFailed)?;
        size += n as u64;
    }
    Ok((hex::encode(hasher.finalize()), size))
}

/// Find the content subdirectory containing `filename` (or `filename.disabled`
/// when `enable` is true), rename it to the opposite state, and return
/// `Some(subdir_name)` on success or `None` if no matching file was found.
pub fn rename_in_content_dir(base: &Path, filename: &str, enable: bool) -> Option<String> {
    for sub in CONTENT_SUBDIRS {
        let dir = base.join(sub);
        if enable {
            let source = dir.join(format!("{}.disabled", filename));
            let dest = dir.join(filename);
            if source.exists() && !dest.exists() {
                std::fs::rename(&source, &dest).ok()?;
                return Some(sub.to_string());
            }
        } else {
            let source = dir.join(filename);
            let dest = dir.join(format!("{}.disabled", filename));
            if source.exists() && !dest.exists() {
                std::fs::rename(&source, &dest).ok()?;
                return Some(sub.to_string());
            }
        }
    }
    None
}

/// Find and delete a file in any content subdirectory.
pub fn find_and_delete_file(instance_dir: &Path, filename: &str) -> bool {
    for sub in CONTENT_SUBDIRS {
        let candidate = instance_dir.join(sub).join(filename);
        if candidate.exists() {
            let _ = std::fs::remove_file(&candidate);
            return true;
        }
    }
    false
}

/// Disk space check helpers
const MIN_DISK_SPACE_BYTES: u64 = 500_000_000;

#[cfg(target_os = "windows")]
pub fn available_disk_space_bytes(path: &Path) -> Option<u64> {
    let root = path.ancestors().last()?;
    let mut command = std::process::Command::new("fsutil");
    command.args(["volume", "diskfree"]).arg(root);
    hide_console_window(&mut command);
    let output = command.output().ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("Available free bytes:") {
            return rest.trim().parse::<u64>().ok();
        }
    }
    None
}

#[cfg(not(target_os = "windows"))]
pub fn available_disk_space_bytes(_path: &Path) -> Option<u64> {
    None
}

pub fn check_disk_space_for(path: &Path, required_bytes: u64) -> LauncherResult<()> {
    let required_with_headroom = required_bytes.saturating_add(MIN_DISK_SPACE_BYTES);
    if available_disk_space_bytes(path).is_some_and(|free| free < required_with_headroom) {
        return Err(LauncherError::DiskFull);
    }
    Ok(())
}

pub fn check_disk_space(instance_dir: &Path) -> LauncherResult<()> {
    if let Some(free) = available_disk_space_bytes(instance_dir) {
        if free < MIN_DISK_SPACE_BYTES {
            return Err(LauncherError::DiskFull);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Filesystem safety helpers — shared between launcher import and pack inventory
// ---------------------------------------------------------------------------

const LAUNCHER_EXCLUDED_FILENAMES: &[&str] = &[
    ".curseclient",
    "minecraftinstance.json",
    "profile.json",
    "instance_manifest.json",
    ".agora",
    ".agora_snapshots",
    ".agora-import",
];
const LAUNCHER_EXCLUDED_DIRNAMES: &[&str] = &[".agora", ".agora_snapshots", ".agora-import"];

pub fn is_excluded_source_name(name: &str, is_directory: bool) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with(".agora")
        || LAUNCHER_EXCLUDED_FILENAMES
            .iter()
            .any(|excluded| lower == excluded.to_ascii_lowercase())
        || (is_directory
            && LAUNCHER_EXCLUDED_DIRNAMES
                .iter()
                .any(|excluded| lower == excluded.to_ascii_lowercase()))
}

pub fn unsafe_filesystem_entry(path: &Path, file_type: &std::fs::FileType) -> bool {
    if file_type.is_symlink() {
        return true;
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if std::fs::symlink_metadata(path)
            .map(|metadata| metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
            .unwrap_or(true)
        {
            return true;
        }
    }
    // Silence unused warning on non-Windows when path is unused.
    let _ = path;
    false
}

pub fn hash_file_sha256(path: &Path) -> std::io::Result<String> {
    use sha2::Digest;
    let mut input = std::fs::File::open(path)?;
    let mut hasher = sha2::Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        use std::io::Read;
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filename_path_traversal_rejected() {
        assert!(safe_zip_entry_name("../../evil.jar").is_none());
        assert!(safe_zip_entry_name("../../../etc/passwd.jar").is_none());
    }

    #[test]
    fn test_safe_zip_entry_name_valid() {
        let result = safe_zip_entry_name("some-mod-1.0.jar");
        assert_eq!(result, Some("mods/some-mod-1.0.jar".to_string()));
    }

    #[test]
    fn test_safe_zip_entry_name_slash_rejected() {
        assert!(safe_zip_entry_name("foo/bar.jar").is_none());
    }

    #[test]
    fn test_safe_zip_entry_name_backslash_rejected() {
        assert!(safe_zip_entry_name("foo\\bar.jar").is_none());
    }

    #[test]
    fn test_safe_zip_entry_name_null_rejected() {
        assert!(safe_zip_entry_name("foo\0bar.jar").is_none());
    }

    #[test]
    fn test_safe_zip_entry_name_dot_rejected() {
        assert!(safe_zip_entry_name(".").is_none());
        assert!(safe_zip_entry_name("..").is_none());
    }

    #[test]
    fn test_safe_zip_entry_name_empty_rejected() {
        assert!(safe_zip_entry_name("").is_none());
    }

    #[test]
    fn test_content_subdir() {
        assert_eq!(content_subdir("mod"), "mods");
        assert_eq!(content_subdir("resourcepack"), "resourcepacks");
        assert_eq!(content_subdir("shader"), "shaderpacks");
        assert_eq!(content_subdir("datapack"), "datapacks");
        assert_eq!(content_subdir("world"), "saves");
    }

    #[test]
    fn test_push_and_remove_content_array() {
        let mut manifest = InstanceManifest {
            manifest_version: crate::models::CURRENT_MANIFEST_VERSION,
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

        let mod_item = InstalledMod {
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
            sha256: "ab".repeat(32),
            installed_at: String::new(),
            java_packages: vec![],
            mod_jar_id: None,
            depends_on: vec![],
            optional_deps: vec![],
            incompatible_deps: vec![],
            provided_mod_ids: vec![],
            enabled: true,
            content_type: "mod".into(),
        };
        push_to_content_array(&mut manifest, &mod_item);
        assert_eq!(manifest.mods.len(), 1);

        assert!(remove_from_content_array(&mut manifest, "test.jar"));
        assert_eq!(manifest.mods.len(), 0);

        assert!(!remove_from_content_array(&mut manifest, "nonexistent.jar"));
    }

    #[test]
    fn test_read_manifest_heals_legacy_and_synthesizes_unknown() {
        let tmp = tempfile::tempdir().unwrap();
        let manifest_path = tmp.path().join("instance_manifest.json");
        // Legacy manifest: no manifest_version, no pack_origin, no pack_managed,
        // but has created_from_pack and a mod with source modrinth-pack.
        let legacy_json = r#"{
            "instance_id": "legacy-test",
            "name": "Legacy",
            "created_from_pack": "Old Pack",
            "minecraft_version": "1.20.1",
            "loader": "fabric",
            "loader_version": "0.15.0",
            "mods": [
                {"filename":"a.jar","source":"modrinth-pack","sha256":"aa","installed_at":"t"},
                {"filename":"b.jar","source":"registry","sha256":"bb","installed_at":"t"}
            ],
            "user_preferences": {}
        }"#;
        std::fs::write(&manifest_path, legacy_json).unwrap();
        let before = std::fs::read_to_string(&manifest_path).unwrap();

        let manifest = read_manifest(&manifest_path).unwrap();
        // Healed in memory
        assert_eq!(manifest.manifest_version, 1); // read does not stamp, just heals
        assert!(manifest.pack_origin.is_some());
        let origin = manifest.pack_origin.as_ref().unwrap();
        assert_eq!(origin.platform, crate::models::PackPlatform::Unknown);
        assert_eq!(origin.pack_name, "Old Pack");
        assert!(origin.project_id.is_none());
        assert!(origin.version_id.is_none());
        // pack_managed healed for modrinth-pack but not registry
        assert!(manifest.mods[0].pack_managed);
        assert!(!manifest.mods[1].pack_managed);
        // File on disk unchanged
        let after = std::fs::read_to_string(&manifest_path).unwrap();
        assert_eq!(before, after);

        // Now write via atomic_write_manifest and verify it stamps to v3
        atomic_write_manifest(&manifest_path, &manifest).unwrap();
        let reread = read_manifest(&manifest_path).unwrap();
        assert_eq!(
            reread.manifest_version,
            crate::models::CURRENT_MANIFEST_VERSION
        );
        // Still has synthesized origin, now persisted
        assert!(reread.pack_origin.is_some());
    }

    #[test]
    fn test_read_manifest_unknown_without_created_from_pack_stays_none() {
        let tmp = tempfile::tempdir().unwrap();
        let manifest_path = tmp.path().join("instance_manifest.json");
        let legacy_json = r#"{
            "instance_id": "plain",
            "name": "Plain",
            "minecraft_version": "1.20.1",
            "loader": "fabric",
            "loader_version": "0.15.0",
            "mods": [],
            "user_preferences": {}
        }"#;
        std::fs::write(&manifest_path, legacy_json).unwrap();
        let manifest = read_manifest(&manifest_path).unwrap();
        // created_from_pack is None => UNKNOWN, not LocalFile, and pack_origin stays None
        assert!(manifest.created_from_pack.is_none());
        assert!(manifest.pack_origin.is_none());
    }

    #[test]
    fn test_atomic_write_stamps_version_even_when_passed_v1() {
        let tmp = tempfile::tempdir().unwrap();
        let manifest_path = tmp.path().join("instance_manifest.json");
        let manifest = InstanceManifest {
            manifest_version: 1,
            game_data: Default::default(),
            pack_origin: None,
            instance_id: "stamp-test".into(),
            name: "Stamp".into(),
            created_from_pack: None,
            minecraft_version: "1.20.1".into(),
            loader: "fabric".into(),
            loader_version: "0.15.0".into(),
            is_locked: false,
            mods: vec![],
            resourcepacks: vec![],
            shaders: vec![],
            datapacks: vec![],
            worlds: vec![],
            user_preferences: serde_json::json!({}),
        };
        atomic_write_manifest(&manifest_path, &manifest).unwrap();
        // Even though we passed v1, the file should be v3
        let raw = std::fs::read_to_string(&manifest_path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v.get("manifest_version").and_then(|v| v.as_u64()), Some(3));
        // Reading back also heals but version is now 3
        let reread = read_manifest(&manifest_path).unwrap();
        assert_eq!(
            reread.manifest_version,
            crate::models::CURRENT_MANIFEST_VERSION
        );
    }

    fn migration_fixture(version: Option<u32>) -> serde_json::Value {
        let mut value = serde_json::json!({
            "instance_id": "migration", "name": "Keep me", "created_from_pack": "Pack",
            "minecraft_version": "24w14a", "loader": "fabric", "loader_version": "0.16.5",
            "mods": [{"filename": "mod.jar", "source": "modrinth-pack", "sha256": "abc",
                "installed_at": "original-time", "pack_managed": false, "enabled": false}],
            "resourcepacks": [], "shaders": [], "datapacks": [], "worlds": [],
            "user_preferences": {"jvm": "keep", "arbitrary": [1, 2]},
            "extension_from_another_feature": {"preserve": true}
        });
        if let Some(version) = version {
            value["manifest_version"] = serde_json::json!(version);
        }
        value
    }

    fn content_bytes(root: &Path) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
        fn walk(
            root: &Path,
            dir: &Path,
            result: &mut std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>,
        ) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(root, &path, result);
                } else if !path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("instance_manifest.json")
                {
                    result.insert(
                        path.strip_prefix(root).unwrap().to_owned(),
                        std::fs::read(path).unwrap(),
                    );
                }
            }
        }
        let mut result = Default::default();
        walk(root, root, &mut result);
        result
    }

    #[test]
    fn manifest_migration_keeps_original_and_all_instance_bytes_for_v1_and_v2() {
        for version in [None, Some(1), Some(2)] {
            let tmp = tempfile::tempdir().unwrap();
            let path = tmp.path().join("instance_manifest.json");
            let original = serde_json::to_vec_pretty(&migration_fixture(version)).unwrap();
            std::fs::write(&path, &original).unwrap();
            for file in [
                "mods/mod.jar",
                "config/settings.ini",
                "saves/world/level.dat",
                "shaderpacks/shader.zip",
            ] {
                let file = tmp.path().join(file);
                std::fs::create_dir_all(file.parent().unwrap()).unwrap();
                std::fs::write(file, [0, 255, 37, 0, 98]).unwrap();
            }
            let before = content_bytes(tmp.path());
            let manifest = read_manifest(&path).unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), original, "read stays lazy");
            assert_eq!(manifest.game_data.game, "minecraft");
            assert_eq!(
                manifest
                    .game_data
                    .runtime_identity
                    .as_ref()
                    .unwrap()
                    .version,
                "24w14a"
            );
            assert_eq!(
                manifest.mods[0].pack_managed,
                version != Some(2),
                "v2 user override survives"
            );
            atomic_write_manifest(&path, &manifest).unwrap();
            assert_eq!(
                std::fs::read(path.with_extension(format!("json.v{}.bak", version.unwrap_or(1))))
                    .unwrap(),
                original
            );
            assert_eq!(content_bytes(tmp.path()), before);
            let value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(value["manifest_version"], 3);
            assert_eq!(value["game"], "minecraft");
            assert_eq!(value["runtime_identity"]["store"], "mojang");
            assert_eq!(value["runtime_identity"]["version"], "24w14a");
            assert!(
                value["base"].is_null(),
                "migration must not invent a pinned base"
            );
            assert_eq!(value["frameworks"][0]["id"], "fabric");
            assert_eq!(value["frameworks"][0]["version"], "0.16.5");
            assert_eq!(value["layers"].as_array().unwrap().len(), 5);
            assert_eq!(value["layers"][0]["source"]["kind"], "instance_content");
            assert!(value.get("minecraft_version").is_none());
            assert_eq!(value["minecraft"]["mods"][0]["enabled"], false);
            assert_eq!(value["minecraft"]["loader"], "fabric");
            assert_eq!(
                value["user_preferences"],
                migration_fixture(version)["user_preferences"]
            );
            assert_eq!(value["extension_from_another_feature"]["preserve"], true);
            let reread = read_manifest(&path).unwrap();
            assert_eq!(reread.minecraft_version, "24w14a");
            assert_eq!(reread.mods[0].sha256, "abc");
            // IPC retains its existing Minecraft fields.
            assert_eq!(
                serde_json::to_value(reread).unwrap()["minecraft_version"],
                "24w14a"
            );
        }
    }

    #[test]
    fn manifest_migration_interruption_at_every_publish_boundary_is_restartable() {
        for stop in [
            ManifestWriteStage::BackupTemporarySynced,
            ManifestWriteStage::OriginalBackedUp,
            ManifestWriteStage::TemporarySynced,
            ManifestWriteStage::Replaced,
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let path = tmp.path().join("instance_manifest.json");
            let original = serde_json::to_vec_pretty(&migration_fixture(Some(2))).unwrap();
            std::fs::write(&path, &original).unwrap();
            // Remnants from a crash during temporary backup/candidate writes are
            // ignored, never promoted on read and never mistaken for a backup.
            std::fs::write(path.with_extension("json.abandoned.tmp"), b"{half-written").unwrap();
            let manifest = read_manifest(&path).unwrap();
            let result = write_manifest_with_hook(&path, &manifest, |stage| {
                if stage == stop {
                    Err(LauncherError::Generic {
                        code: "SIMULATED_CRASH".into(),
                        message: "interrupted".into(),
                    })
                } else {
                    Ok(())
                }
            });
            assert!(result.is_err());
            let backup = path.with_extension("json.v2.bak");
            if stop == ManifestWriteStage::BackupTemporarySynced {
                assert!(
                    !backup.exists(),
                    "an unpublished backup must stay temporary"
                );
            } else {
                assert_eq!(std::fs::read(&backup).unwrap(), original);
            }
            let interrupted = std::fs::read(&path).unwrap();
            if stop != ManifestWriteStage::Replaced {
                assert_eq!(interrupted, original);
            }
            let value: serde_json::Value = serde_json::from_slice(&interrupted).unwrap();
            assert_eq!(
                value["manifest_version"],
                if stop == ManifestWriteStage::Replaced {
                    3
                } else {
                    2
                }
            );
            // Simulate a new process: reread only the live manifest.
            let restarted = read_manifest(&path).unwrap();
            atomic_write_manifest(&path, &restarted).unwrap();
            let current = std::fs::read(&path).unwrap();
            let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
            let file_count = std::fs::read_dir(tmp.path()).unwrap().count();
            atomic_write_manifest(&path, &read_manifest(&path).unwrap()).unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), current);
            assert_eq!(
                std::fs::metadata(&path).unwrap().modified().unwrap(),
                modified
            );
            assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), file_count);
            assert_eq!(
                std::fs::read(path.with_extension("json.v2.bak")).unwrap(),
                original
            );
        }
    }

    #[test]
    fn manifest_migration_preserves_backup_before_first_user_edit() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("instance_manifest.json");
        let original = serde_json::to_vec(&migration_fixture(Some(2))).unwrap();
        std::fs::write(&path, &original).unwrap();
        let mut manifest = read_manifest(&path).unwrap();
        manifest.name = "Edited".into();
        manifest.minecraft_version = "1.21.1".into();
        manifest.loader = "quilt".into();
        manifest.loader_version = "0.26".into();
        atomic_write_manifest(&path, &manifest).unwrap();
        assert_eq!(
            std::fs::read(path.with_extension("json.v2.bak")).unwrap(),
            original
        );
        let manifest = read_manifest(&path).unwrap();
        assert_eq!(manifest.name, "Edited");
        assert_eq!(
            manifest
                .game_data
                .runtime_identity
                .as_ref()
                .unwrap()
                .version,
            "1.21.1"
        );
        assert_eq!(manifest.game_data.frameworks.len(), 1);
        assert_eq!(manifest.game_data.frameworks[0].id, "quilt");
    }

    #[test]
    fn manifest_migration_backup_conflict_keeps_both_files() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("instance_manifest.json");
        let original = serde_json::to_vec(&migration_fixture(Some(2))).unwrap();
        std::fs::write(&path, &original).unwrap();
        let backup = path.with_extension("json.v2.bak");
        std::fs::write(&backup, b"a different original").unwrap();
        let result = atomic_write_manifest(&path, &read_manifest(&path).unwrap());
        assert_eq!(result.unwrap_err().code(), "ERR_MIGRATION_CONFLICT");
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(std::fs::read(backup).unwrap(), b"a different original");
    }

    #[test]
    fn manifest_migration_newer_schema_is_read_only_even_with_unknown_types() {
        for future in [
            {
                let mut value = migration_fixture(Some(4));
                value["future_feature"] = serde_json::json!({"new": true});
                value
            },
            serde_json::json!({"manifest_version": 99, "instance_id": "future", "name": "Future",
                "game": "valheim", "layers": [{"source": {"kind": "future-layer"}}],
                "runtime_identity": {"future-shape": true}}),
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let path = tmp.path().join("instance_manifest.json");
            let original = serde_json::to_vec_pretty(&future).unwrap();
            std::fs::write(&path, &original).unwrap();
            let mut manifest = read_manifest(&path).unwrap();
            assert!(manifest.is_read_only());
            assert!(manifest.to_disk_value().is_err());
            assert_eq!(
                manifest.game_data.read_only_source.as_ref().unwrap(),
                &future
            );
            assert!(
                manifest.pack_origin.is_none(),
                "future manifests must not be healed"
            );
            assert_eq!(
                atomic_write_manifest(&path, &manifest).unwrap_err().code(),
                "ERR_MANIFEST_READ_ONLY"
            );
            manifest.manifest_version = 1; // Lowering just the number must not bypass protection.
            assert_eq!(
                atomic_write_manifest(&path, &manifest).unwrap_err().code(),
                "ERR_MANIFEST_READ_ONLY"
            );
            let stale: InstanceManifest =
                serde_json::from_value(migration_fixture(Some(2))).unwrap();
            assert_eq!(
                atomic_write_manifest(&path, &stale).unwrap_err().code(),
                "ERR_MANIFEST_READ_ONLY"
            );
            assert_eq!(std::fs::read(&path).unwrap(), original);
            assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 1);
        }
    }

    #[test]
    fn manifest_migration_current_schema_is_noop_and_keeps_unknown_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("instance_manifest.json");
        let legacy: InstanceManifest = serde_json::from_value(migration_fixture(Some(2))).unwrap();
        let mut current = legacy.to_disk_value().unwrap();
        current["created_from_pack"] = serde_json::Value::Null;
        current["minecraft"]["future_minecraft_option"] = serde_json::json!({"keep": true});
        let original = serde_json::to_vec(&current).unwrap(); // Intentionally compact formatting.
        std::fs::write(&path, &original).unwrap();
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        let mut manifest = read_manifest(&path).unwrap();
        atomic_write_manifest(&path, &manifest).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            modified
        );
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 1);
        manifest.name = "A real edit".into();
        atomic_write_manifest(&path, &manifest).unwrap();
        let edited: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(edited["minecraft"]["future_minecraft_option"]["keep"], true);
    }
}
