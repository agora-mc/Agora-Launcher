//! Thin compat shim: preserves the original `&tauri::AppHandle` signatures
//! so no caller across the desktop crate needs to change.
//!
//! Internally this module resolves the app data directory from the handle
//! once, then delegates to `agora_core::paths` or `agora_core::app_paths::AppPaths`.

use std::path::PathBuf;
use tauri::Manager;

// Re-export pure (non-AppHandle) helpers directly from core.
pub use agora_core::paths::{launcher_profiles_path, minecraft_dir, sanitize_id};

/// Resolve the official app data directory from the Tauri `AppHandle`.
///
/// This is the only Tauri-specific path resolution — everything else
/// delegates to `agora_core` once the base is known.
pub fn app_data_dir<R: tauri::Runtime>(_app: &tauri::AppHandle<R>) -> anyhow::Result<PathBuf> {
    // Keep desktop and CLI on the same core-owned root. Tauri remains an
    // adapter for lifecycle and IPC, not a second data-directory authority.
    let dir = agora_core::app_paths::AppPaths::platform_default()
        .root()
        .to_path_buf();
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Move data from the pre-CoreContext Tauri app-data root when the new shared
/// root has not been created yet. This is intentionally a rename, not a merge:
/// partial copies could mix databases and registry state from two roots.
pub fn migrate_legacy_data_dir<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> anyhow::Result<bool> {
    // A portable copy or explicit data root must never move an installed
    // copy's profile into its own folder on first launch.
    if crate::commands::is_portable_mode() || std::env::var_os("AGORA_DATA_DIR").is_some() {
        return Ok(false);
    }
    let target = agora_core::app_paths::AppPaths::platform_default()
        .root()
        .to_path_buf();
    let legacy = app
        .path()
        .app_data_dir()
        .map_err(|e| anyhow::anyhow!("Failed to resolve legacy app data dir: {}", e))?;

    if legacy == target || !legacy.exists() || target.exists() {
        return Ok(false);
    }

    let parent = target
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Shared app data root has no parent"))?;
    std::fs::create_dir_all(parent)?;
    std::fs::rename(&legacy, &target).map_err(|error| {
        anyhow::anyhow!(
            "Failed to migrate Agora data from {} to {}: {}",
            legacy.display(),
            target.display(),
            error
        )
    })?;
    Ok(true)
}

/// Build an `agora_core::app_paths::AppPaths` from the Tauri handle.
pub fn app_paths<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> anyhow::Result<agora_core::app_paths::AppPaths> {
    let root = app_data_dir(app)?;
    Ok(agora_core::app_paths::AppPaths::from_root(root))
}

/// The root directory holding all user instances.
pub fn instances_dir<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> anyhow::Result<PathBuf> {
    let base = app_data_dir(app)?;
    agora_core::paths::instances_dir(&base)
}

/// Directory for a single instance (e.g. `instances/<instance_id>`).
pub fn instance_dir<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    instance_id: &str,
) -> anyhow::Result<PathBuf> {
    let base = app_data_dir(app)?;
    agora_core::paths::instance_dir(&base, instance_id)
}

/// Path to an instance's `instance_manifest.json`.
pub fn instance_manifest_path<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    instance_id: &str,
) -> anyhow::Result<PathBuf> {
    let base = app_data_dir(app)?;
    agora_core::paths::instance_manifest_path(&base, instance_id)
}

/// Path to the cached read-only registry database.
pub fn registry_db_path<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> anyhow::Result<PathBuf> {
    let base = app_data_dir(app)?;
    agora_core::paths::registry_db_path(&base)
}

/// Path to the cached registry.db Ed25519 signature file.
pub fn registry_sig_path<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> anyhow::Result<PathBuf> {
    let base = app_data_dir(app)?;
    agora_core::paths::registry_sig_path(&base)
}

/// Path to the mutable local state database.
pub fn local_state_db_path<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> anyhow::Result<PathBuf> {
    let base = app_data_dir(app)?;
    agora_core::paths::local_state_db_path(&base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_id_preserves_alphanumeric() {
        let result = sanitize_id("my-instance-1");
        assert!(result.contains("my-instance-1"));
    }

    #[test]
    fn test_sanitize_id_removes_path_separators() {
        assert!(!sanitize_id("foo/bar").contains('/'));
        assert!(!sanitize_id("foo\\bar").contains('\\'));
    }

    #[test]
    fn test_sanitize_id_removes_dot_dot() {
        let result = sanitize_id("..");
        assert!(!result.contains(".."));
    }

    #[test]
    fn test_sanitize_id_removes_dot_dot_slash() {
        let result = sanitize_id("../etc/passwd");
        assert!(!result.contains(".."));
        assert!(!result.contains('/'));
    }

    #[test]
    fn test_sanitize_id_removes_special_chars() {
        let result = sanitize_id("foo!@#bar");
        assert!(result.contains("foo"));
        assert!(result.contains("bar"));
        assert!(!result.contains(|c: char| ['!', '@', '#'].contains(&c)));
    }

    #[test]
    fn test_sanitize_id_empty_string() {
        let result = sanitize_id("");
        assert_eq!(result, "");
    }

    #[test]
    fn test_sanitize_id_unicode_preserved() {
        let result = sanitize_id("café");
        assert!(!result.is_empty());
    }

    #[test]
    fn test_sanitize_id_null_bytes_removed() {
        let result = sanitize_id("foo\0bar");
        assert!(!result.contains('\0'));
    }
}
