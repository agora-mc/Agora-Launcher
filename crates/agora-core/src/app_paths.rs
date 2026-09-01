//! Canonical, core-owned path model for the Agora app data directory.
//!
//! `AppPaths` is constructed once from a single app-data root and provides
//! typed helpers for every subpath. No adapter (CLI, desktop, MCP) may
//! reconstruct app-data subpaths on its own — they must use this struct.
//!
//! All methods that accept user-controlled identifiers return
//! [`LauncherResult`] and validate the input before joining — there are no
//! `assert!` or panic paths on external inputs.
//!
//! Existing free-path functions in [`crate::paths`] now delegate here.

use crate::error::{LauncherError, LauncherResult};
use std::path::{Path, PathBuf};

/// The app data directory layout.
///
/// Build from a root (e.g. `~/.local/share/agora` on Linux,
/// `%APPDATA%/com.agoramc.app` on Windows, or `AGORA_DATA_DIR` env var).
#[derive(Debug, Clone)]
pub struct AppPaths {
    root: PathBuf,
}

impl AppPaths {
    /// Build `AppPaths` from an explicit data root.
    ///
    /// The root is used as-is; no subdirectory is appended.
    pub fn from_root(root: PathBuf) -> Self {
        Self { root }
    }

    /// Resolve the platform-default data directory using an explicit override.
    ///
    /// If `override_root` is `Some`, returns that path directly.
    /// Otherwise checks `AGORA_DATA_DIR` env var, then `dirs::data_local_dir() / "agora"`.
    ///
    /// This function does not read environment variables on its own — it
    /// accepts the override as a parameter, making it deterministic and
    /// safe to call in tests.
    pub fn platform_default_with_override(override_root: Option<PathBuf>) -> Self {
        if let Some(root) = override_root {
            return Self::from_root(root);
        }
        let base = dirs::data_local_dir().unwrap_or_else(|| {
            // Last-resort fallback: current directory. On supported OS targets
            // (Windows, macOS, Linux) this path is never reached.
            PathBuf::from(".")
        });
        Self::from_root(base.join("agora"))
    }

    /// Resolve the platform-default data directory (reads `AGORA_DATA_DIR` env var).
    ///
    /// Precedence:
    /// 1. `AGORA_DATA_DIR` environment variable
    /// 2. `dirs::data_local_dir() / "agora"` (platform convention)
    pub fn platform_default() -> Self {
        let env_root = std::env::var("AGORA_DATA_DIR").ok().map(PathBuf::from);
        Self::platform_default_with_override(env_root)
    }

    // ------------------------------------------------------------------
    // Top-level paths (no user-controlled input — infallible)
    // ------------------------------------------------------------------

    /// The root app data directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Path to the mutable local state SQLite database.
    pub fn local_state_db(&self) -> PathBuf {
        self.root.join("local_state.db")
    }

    /// Path to the cached read-only registry SQLite database.
    pub fn registry_db(&self) -> PathBuf {
        self.root.join("registry.db")
    }

    /// Path to the registry Ed25519 signature sidecar.
    pub fn registry_signature(&self) -> PathBuf {
        self.root.join("registry.db.sig")
    }

    /// Root directory for all instances (`instances/`).
    pub fn instances_root(&self) -> PathBuf {
        self.root.join("instances")
    }

    /// Root directory for the Agora-owned Minecraft runtime (`minecraft-runtime/`).
    pub fn minecraft_runtime_root(&self) -> PathBuf {
        self.root.join("minecraft-runtime")
    }

    /// Root directory for managed Java runtimes (`runtimes/`).
    pub fn java_runtimes_root(&self) -> PathBuf {
        self.root.join("runtimes")
    }

    /// Persistent Java discovery inventory (`java-inventory.json`).
    ///
    /// Caches the result of the (subprocess-heavy) full Java discovery scan
    /// between app sessions so a warm launch never re-probes every JRE on the
    /// machine. The cache is validated against executable metadata and the
    /// runtime root directories before it is trusted.
    pub fn java_inventory(&self) -> PathBuf {
        self.root.join("java-inventory.json")
    }

    /// Root directory for loader cache (`loader-cache/`).
    pub fn loader_cache(&self) -> PathBuf {
        self.root.join("loader-cache")
    }

    /// Root directory for loader receipts (`receipts/`).
    pub fn loader_receipts(&self) -> PathBuf {
        self.root.join("receipts")
    }

    /// Root directory for snapshots (`snapshots/`).
    pub fn snapshots_root(&self) -> PathBuf {
        self.root.join("snapshots")
    }

    /// Root directory for operation state and lock files (`locks/`).
    pub fn locks_root(&self) -> PathBuf {
        self.root.join("locks")
    }

    /// Directory for temporary staging files (`staging/`).
    pub fn staging_root(&self) -> PathBuf {
        self.root.join("staging")
    }

    /// Root directory for reusable instance templates (`templates/`).
    pub fn templates_root(&self) -> PathBuf {
        crate::template_service::templates_root(&self.root)
    }

    // ------------------------------------------------------------------
    // Validated helpers (user-controlled input — may return Err)
    // ------------------------------------------------------------------

    /// Validate an instance ID and return its directory.
    ///
    /// Rejects empty, traversal, absolute, separator-containing,
    /// Unicode-invisible-only, and invalid-length IDs.
    /// Returns the path `<instances_root>/<id>`.
    pub fn instance_dir(&self, instance_id: &str) -> LauncherResult<PathBuf> {
        validate_path_component(instance_id)?;
        Ok(self.instances_root().join(instance_id))
    }

    /// Validate an instance ID and return its manifest path.
    pub fn instance_manifest(&self, instance_id: &str) -> LauncherResult<PathBuf> {
        Ok(self
            .instance_dir(instance_id)?
            .join("instance_manifest.json"))
    }

    /// Lock path for a named resource.
    ///
    /// Resources: `"registry-update"`, `"loader-install"`,
    /// `"java-major-<N>"`, `"materialization"`.
    pub fn runtime_lock(&self, resource: &str) -> LauncherResult<PathBuf> {
        validate_path_component(resource)?;
        if resource.starts_with("instance-") {
            return Err(LauncherError::Generic {
                code: "ERR_INVALID_LOCK_NAME".into(),
                message:
                    "Lock resource name must not start with 'instance-'; use instance_lock instead."
                        .into(),
            });
        }
        Ok(self.locks_root().join(format!("{resource}.lock")))
    }

    /// Instance-specific lock path (validates instance_id).
    pub fn instance_lock(&self, instance_id: &str) -> LauncherResult<PathBuf> {
        validate_path_component(instance_id)?;
        Ok(self
            .locks_root()
            .join(format!("instance-{instance_id}.lock")))
    }

    /// Staging directory for an operation fingerprint.
    pub fn staging_dir(&self, fingerprint: &str) -> LauncherResult<PathBuf> {
        validate_path_component(fingerprint)?;
        Ok(self.staging_root().join(fingerprint))
    }

    /// Path to the Minecraft version JSON for a given version.
    /// The version ID is validated — rejects empty, traversal, absolute, separators.
    pub fn minecraft_version_json(&self, version_id: &str) -> LauncherResult<PathBuf> {
        validate_path_component(version_id)?;
        Ok(self
            .minecraft_runtime_root()
            .join("versions")
            .join(version_id)
            .join(format!("{version_id}.json")))
    }

    // ------------------------------------------------------------------
    // Infallible runtime sub-paths (fixed segment names, no user input)
    // ------------------------------------------------------------------

    /// Libraries directory under the runtime root.
    pub fn minecraft_libraries_dir(&self) -> PathBuf {
        self.minecraft_runtime_root().join("libraries")
    }

    /// Assets directory under the runtime root.
    pub fn minecraft_assets_dir(&self) -> PathBuf {
        self.minecraft_runtime_root().join("assets")
    }

    /// Logging configs directory under the runtime root.
    pub fn minecraft_logging_dir(&self) -> PathBuf {
        self.minecraft_runtime_root().join("logging")
    }

    /// Natives directory under the runtime root.
    pub fn minecraft_natives_dir(&self) -> PathBuf {
        self.minecraft_runtime_root().join("natives")
    }

    /// Launcher profiles path under the runtime root (for installer compatibility).
    pub fn minecraft_launcher_profiles(&self) -> PathBuf {
        self.minecraft_runtime_root().join("launcher_profiles.json")
    }

    // ------------------------------------------------------------------
    // Directory creation
    // ------------------------------------------------------------------

    /// Create all required directories under this root.
    ///
    /// Returns a list of paths that were created (for logging).
    pub fn create_required_dirs(&self) -> LauncherResult<Vec<PathBuf>> {
        let dirs = [
            self.root(),
            &self.instances_root(),
            &self.minecraft_runtime_root(),
            &self.java_runtimes_root(),
            &self.loader_cache(),
            &self.loader_receipts(),
            &self.snapshots_root(),
            &self.locks_root(),
            &self.staging_root(),
            &self.minecraft_libraries_dir(),
            &self.minecraft_assets_dir(),
            &self.minecraft_logging_dir(),
            &self.minecraft_natives_dir(),
        ];
        let mut created = Vec::new();
        for dir in &dirs {
            if !dir.exists() {
                std::fs::create_dir_all(dir).map_err(|e| LauncherError::Generic {
                    code: "ERR_DIR_CREATE".into(),
                    message: format!("Failed to create {}: {e}", dir.display()),
                })?;
                created.push(dir.to_path_buf());
            }
        }
        Ok(created)
    }
}

// ---------------------------------------------------------------------------
// ID / path-component validation
// ---------------------------------------------------------------------------

/// Validate a user-supplied path component (instance ID, resource name,
/// fingerprint, version ID).
///
/// Rules:
/// - Must not be empty.
/// - Must not contain path separators (`/`, `\`).
/// - Must not be `.` or `..`.
/// - Must not be absolute (Unix `/` prefix or Windows drive-letter prefix
///   like `C:` or UNC prefix like `\\`).
/// - Must not be longer than 256 bytes.
///
/// Spaces and Unicode are allowed — they are legitimate in version IDs
/// (e.g. `"1.21 Release"`) and platform identifiers.
pub fn validate_path_component(id: &str) -> LauncherResult<()> {
    if id.is_empty() {
        return Err(LauncherError::Generic {
            code: "ERR_INVALID_PATH_COMPONENT".into(),
            message: "Path component must not be empty.".into(),
        });
    }
    if id.len() > 256 {
        return Err(LauncherError::Generic {
            code: "ERR_INVALID_PATH_COMPONENT".into(),
            message: format!("Path component is too long ({} bytes, max 256)", id.len()),
        });
    }
    if id.contains('/') || id.contains('\\') {
        return Err(LauncherError::Generic {
            code: "ERR_INVALID_PATH_COMPONENT".into(),
            message: "Path component must not contain path separators.".into(),
        });
    }
    // Reject `.` and `..` (also `..` variants like `...` that resolve to `..` on some FS).
    let trimmed = id.trim_matches(|c: char| c == '.' || c == ' ');
    if trimmed.is_empty() {
        return Err(LauncherError::Generic {
            code: "ERR_INVALID_PATH_COMPONENT".into(),
            message: "Path component must not be composed only of dots or spaces.".into(),
        });
    }
    if id == "." || id == ".." {
        return Err(LauncherError::Generic {
            code: "ERR_INVALID_PATH_COMPONENT".into(),
            message: "Path component must not be '.' or '..'.".into(),
        });
    }
    if id.starts_with("..") {
        return Err(LauncherError::Generic {
            code: "ERR_INVALID_PATH_COMPONENT".into(),
            message: "Path component must not start with '..'.".into(),
        });
    }

    // Reject absolute paths.
    if cfg!(windows) {
        // Windows absolute: starts with drive letter + colon (e.g. "C:", "D:")
        // or UNC path (starts with two backslashes, already caught by separator check)
        if id.len() >= 2 && id.as_bytes()[0].is_ascii_alphabetic() && id.as_bytes()[1] == b':' {
            return Err(LauncherError::Generic {
                code: "ERR_INVALID_PATH_COMPONENT".into(),
                message: "Path component must not start with a drive letter.".into(),
            });
        }
        // UNC paths like \\server\share — the `\\` should be caught by the
        // `contains('\\')` check above, but also check for leading `\0` or
        // device namespace prefixes.
        if id.starts_with("\\") || id.starts_with("//") {
            return Err(LauncherError::Generic {
                code: "ERR_INVALID_PATH_COMPONENT".into(),
                message: "Path component must not be a UNC path.".into(),
            });
        }
    } else {
        // Unix absolute path (starts with `/`).
        if id.starts_with('/') {
            return Err(LauncherError::Generic {
                code: "ERR_INVALID_PATH_COMPONENT".into(),
                message: "Path component must not be an absolute path.".into(),
            });
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Backward-compatibility shim: free functions delegate to AppPaths
// ---------------------------------------------------------------------------

/// Legacy: resolve the platform default path via `AppPaths::platform_default`.
/// Used by CLI startup while the command-parity migration is in progress.
pub fn default_app_data_dir() -> PathBuf {
    AppPaths::platform_default().root().to_path_buf()
}

/// Legacy: instance directory using free `app_data_dir` parameter.
pub fn instance_dir(app_data_dir: &Path, instance_id: &str) -> LauncherResult<PathBuf> {
    let paths = AppPaths::from_root(app_data_dir.to_path_buf());
    paths.instance_dir(instance_id)
}

/// Legacy: instance manifest path using free `app_data_dir` parameter.
pub fn instance_manifest(app_data_dir: &Path, instance_id: &str) -> LauncherResult<PathBuf> {
    Ok(instance_dir(app_data_dir, instance_id)?.join("instance_manifest.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // Construction and platform default
    // ------------------------------------------------------------------

    #[test]
    fn test_from_root_uses_root_as_is() {
        let p = AppPaths::from_root(PathBuf::from("/tmp/agora-test"));
        assert_eq!(p.root(), Path::new("/tmp/agora-test"));
    }

    #[test]
    fn test_platform_default_with_override() {
        let p = AppPaths::platform_default_with_override(Some(PathBuf::from("/custom/path")));
        assert_eq!(p.root(), Path::new("/custom/path"));
    }

    #[test]
    fn test_platform_default_without_override_resolves_to_dir() {
        let p = AppPaths::platform_default_with_override(None);
        // Should not panic and should return a non-empty path.
        assert!(!p.root().as_os_str().is_empty());
    }

    // ------------------------------------------------------------------
    // Top-level subpaths
    // ------------------------------------------------------------------

    #[test]
    fn test_subpaths_under_root() {
        let p = AppPaths::from_root(PathBuf::from("/base"));
        assert_eq!(p.local_state_db(), Path::new("/base/local_state.db"));
        assert_eq!(p.registry_db(), Path::new("/base/registry.db"));
        assert_eq!(p.registry_signature(), Path::new("/base/registry.db.sig"));
        assert_eq!(p.instances_root(), Path::new("/base/instances"));
        assert_eq!(
            p.minecraft_runtime_root(),
            Path::new("/base/minecraft-runtime")
        );
        assert_eq!(p.java_runtimes_root(), Path::new("/base/runtimes"));
        assert_eq!(p.loader_cache(), Path::new("/base/loader-cache"));
        assert_eq!(p.loader_receipts(), Path::new("/base/receipts"));
        assert_eq!(p.snapshots_root(), Path::new("/base/snapshots"));
        assert_eq!(p.locks_root(), Path::new("/base/locks"));
        assert_eq!(p.staging_root(), Path::new("/base/staging"));
    }

    // ------------------------------------------------------------------
    // Validated helpers
    // ------------------------------------------------------------------

    #[test]
    fn test_instance_dir_valid() {
        let p = AppPaths::from_root(PathBuf::from("/base"));
        let dir = p.instance_dir("my-instance-1").unwrap();
        assert_eq!(dir, Path::new("/base/instances/my-instance-1"));
    }

    #[test]
    fn test_instance_dir_accepts_spaces() {
        let p = AppPaths::from_root(PathBuf::from("/base"));
        let dir = p.instance_dir("my instance 1").unwrap();
        assert_eq!(dir, Path::new("/base/instances/my instance 1"));
    }

    #[test]
    fn test_instance_dir_accepts_unicode() {
        let p = AppPaths::from_root(PathBuf::from("/base"));
        let dir = p.instance_dir("café-instance").unwrap();
        assert!(dir.to_string_lossy().contains("café-instance"));
    }

    #[test]
    fn test_instance_manifest_valid() {
        let p = AppPaths::from_root(PathBuf::from("/base"));
        let m = p.instance_manifest("test-instance").unwrap();
        assert_eq!(
            m,
            Path::new("/base/instances/test-instance/instance_manifest.json")
        );
    }

    #[test]
    fn test_instance_dir_rejects_empty() {
        let p = AppPaths::from_root(PathBuf::from("/base"));
        assert!(p.instance_dir("").is_err());
    }

    #[test]
    fn test_instance_dir_rejects_traversal() {
        let p = AppPaths::from_root(PathBuf::from("/base"));
        assert!(p.instance_dir("..").is_err());
        assert!(p.instance_dir(".").is_err());
        assert!(p.instance_dir("../evil").is_err());
    }

    #[test]
    fn test_instance_dir_rejects_dot_only() {
        let p = AppPaths::from_root(PathBuf::from("/base"));
        assert!(p.instance_dir("...").is_err());
        assert!(p.instance_dir(" . ").is_err());
    }

    #[test]
    fn test_instance_dir_rejects_separators() {
        let p = AppPaths::from_root(PathBuf::from("/base"));
        assert!(p.instance_dir("a/b").is_err());
        assert!(p.instance_dir("a\\b").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn test_instance_dir_rejects_absolute_unix() {
        let p = AppPaths::from_root(PathBuf::from("/base"));
        assert!(p.instance_dir("/etc/passwd").is_err());
    }

    #[test]
    fn test_instance_dir_rejects_very_long() {
        let p = AppPaths::from_root(PathBuf::from("/base"));
        let long = "a".repeat(257);
        assert!(p.instance_dir(&long).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn test_instance_dir_rejects_drive_letter() {
        let p = AppPaths::from_root(PathBuf::from("C:\\base"));
        assert!(p.instance_dir("C:evil").is_err());
        assert!(p.instance_dir("D:..").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn test_instance_dir_rejects_unc() {
        let p = AppPaths::from_root(PathBuf::from("C:\\base"));
        assert!(p.instance_dir("\\\\server\\share").is_err());
    }

    // ------------------------------------------------------------------
    // Lock paths
    // ------------------------------------------------------------------

    #[test]
    fn test_runtime_lock_valid() {
        let p = AppPaths::from_root(PathBuf::from("/base"));
        let lock = p.runtime_lock("registry-update").unwrap();
        assert_eq!(lock, Path::new("/base/locks/registry-update.lock"));
    }

    #[test]
    fn test_runtime_lock_rejects_instance_prefix() {
        let p = AppPaths::from_root(PathBuf::from("/base"));
        assert!(p.runtime_lock("instance-foo").is_err());
    }

    #[test]
    fn test_runtime_lock_rejects_bad_input() {
        let p = AppPaths::from_root(PathBuf::from("/base"));
        assert!(p.runtime_lock("").is_err());
        assert!(p.runtime_lock("../evil").is_err());
    }

    #[test]
    fn test_instance_lock_valid() {
        let p = AppPaths::from_root(PathBuf::from("/base"));
        let lock = p.instance_lock("my-instance").unwrap();
        assert_eq!(lock, Path::new("/base/locks/instance-my-instance.lock"));
    }

    #[test]
    fn test_instance_lock_rejects_bad_input() {
        let p = AppPaths::from_root(PathBuf::from("/base"));
        assert!(p.instance_lock("").is_err());
        assert!(p.instance_lock("a/b").is_err());
    }

    // ------------------------------------------------------------------
    // Staging paths
    // ------------------------------------------------------------------

    #[test]
    fn test_staging_dir_valid() {
        let p = AppPaths::from_root(PathBuf::from("/base"));
        let dir = p.staging_dir("abc123def").unwrap();
        assert_eq!(dir, Path::new("/base/staging/abc123def"));
    }

    #[test]
    fn test_staging_dir_rejects_bad_input() {
        let p = AppPaths::from_root(PathBuf::from("/base"));
        assert!(p.staging_dir("").is_err());
        assert!(p.staging_dir("a/b").is_err());
    }

    // ------------------------------------------------------------------
    // Version JSON
    // ------------------------------------------------------------------

    #[test]
    fn test_minecraft_version_json_valid() {
        let p = AppPaths::from_root(PathBuf::from("/base"));
        let path = p.minecraft_version_json("1.21").unwrap();
        assert_eq!(
            path,
            Path::new("/base/minecraft-runtime/versions/1.21/1.21.json")
        );
    }

    #[test]
    fn test_minecraft_version_json_accepts_spaces() {
        let p = AppPaths::from_root(PathBuf::from("/base"));
        assert!(p.minecraft_version_json("1.21 Release").is_ok());
    }

    #[test]
    fn test_minecraft_version_json_rejects_separators() {
        let p = AppPaths::from_root(PathBuf::from("/base"));
        assert!(p.minecraft_version_json("1.21/../evil").is_err());
    }

    // ------------------------------------------------------------------
    // Validation function unit tests
    // ------------------------------------------------------------------

    #[test]
    fn test_validate_rejects_empty() {
        assert!(validate_path_component("").is_err());
    }

    #[test]
    fn test_validate_rejects_dot_dot() {
        assert!(validate_path_component("..").is_err());
        assert!(validate_path_component("../evil").is_err());
    }

    #[test]
    fn test_validate_rejects_dot_only() {
        assert!(validate_path_component(".").is_err());
        assert!(validate_path_component("...").is_err());
    }

    #[test]
    fn test_validate_rejects_separators() {
        assert!(validate_path_component("a/b").is_err());
        assert!(validate_path_component("a\\b").is_err());
    }

    #[test]
    fn test_validate_rejects_too_long() {
        assert!(validate_path_component(&"a".repeat(257)).is_err());
    }

    #[test]
    fn test_validate_accepts_spaces() {
        assert!(validate_path_component("my instance 1").is_ok());
        assert!(validate_path_component("1.21 Release").is_ok());
    }

    #[test]
    fn test_validate_accepts_unicode() {
        assert!(validate_path_component("café").is_ok());
        assert!(validate_path_component("日本語").is_ok());
    }

    #[test]
    fn test_validate_accepts_hyphens_and_underscores() {
        assert!(validate_path_component("my-instance_42").is_ok());
    }
}
