use crate::ctx::Ctx;
use crate::error::{LauncherError, LauncherResult};
use crate::models::{InstalledMod, InstanceManifest};
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
    let text =
        std::fs::read_to_string(&manifest_path).map_err(|_| LauncherError::InstanceCreateFailed)?;
    let manifest: InstanceManifest =
        serde_json::from_str(&text).map_err(|_| LauncherError::InstanceCreateFailed)?;
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

/// Atomic manifest write helper.
pub fn atomic_write_manifest(
    manifest_path: &Path,
    manifest: &InstanceManifest,
) -> LauncherResult<()> {
    let tmp_path = manifest_path.with_extension("json.tmp");
    let text =
        serde_json::to_string_pretty(manifest).map_err(|_| LauncherError::InstanceCreateFailed)?;
    std::fs::write(&tmp_path, text).map_err(|_| LauncherError::InstanceCreateFailed)?;
    std::fs::rename(&tmp_path, manifest_path).map_err(|_| LauncherError::InstanceCreateFailed)?;
    Ok(())
}

/// Read the instance manifest from disk.
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
    serde_json::from_str(&text).map_err(|_| LauncherError::InstanceCreateFailed)
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
            pack_managed: false,
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
}
