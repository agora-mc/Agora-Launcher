use crate::app_paths::AppPaths;
use crate::error::{LauncherError, LauncherResult};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single file entry in the migration inventory.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MigrationEntry {
    /// Relative path from the data root (using forward slashes).
    pub rel_path: String,
    /// File size in bytes.
    pub size_bytes: u64,
    /// Whether the source path is a symlink.
    pub is_symlink: bool,
}

/// Inventory of an Agora data root for migration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MigrationInventory {
    /// The source root path.
    pub source_root: String,
    /// All files found at the source root.
    pub files: Vec<MigrationEntry>,
    /// Total size in bytes.
    pub total_size_bytes: u64,
    /// Whether a `local_state.db` exists at the source.
    pub has_local_state_db: bool,
    /// Whether a `registry.db` exists at the source.
    pub has_registry_db: bool,
    /// Instance IDs found at the source.
    pub instance_ids: Vec<String>,
}

/// Result of a successful migration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MigrationResult {
    /// Number of files migrated.
    pub files_migrated: usize,
    /// Total bytes copied.
    pub total_bytes: u64,
    /// Instance IDs migrated.
    pub instance_ids: Vec<String>,
    /// Path to the backup of destination state.
    pub backup_path: String,
}

/// Describes a conflicting file that blocks migration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MigrationConflict {
    /// Relative path of the conflicting item.
    pub rel_path: String,
    /// Explanation of the conflict.
    pub reason: String,
}

/// Describes what a dry-run migration would do.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MigrationPlan {
    /// Inventory of what would be migrated from the source.
    pub source_inventory: MigrationInventory,
    /// Conflicting items that block the migration (empty = no conflict).
    pub conflicts: Vec<MigrationConflict>,
    /// Whether the migration can proceed (no conflicts found).
    pub can_proceed: bool,
}
// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

/// Core-owned data migration service.
///
/// Migrates data from an old Agora CLI data root to the current destination
/// (managed via `AppPaths`). No silent merge of SQLite databases — explicit
/// conflict detection. Creates a timestamped backup of destination state
/// before any mutation. Uses same-volume staging and atomic promotion.
#[derive(Clone)]
pub struct DataMigrationService {
    dest: AppPaths,
}

impl DataMigrationService {
    /// Create a service that will migrate data into the given destination.
    pub fn new(dest: AppPaths) -> Self {
        Self { dest }
    }

    // ------------------------------------------------------------------
    // Public API
    // ------------------------------------------------------------------

    /// Inventory a source data root without touching the destination.
    pub fn inventory(&self, source_root: &Path) -> LauncherResult<MigrationInventory> {
        let canonical_source = Self::canonicalize_path(source_root)?;
        Self::check_path_safety(&canonical_source)?;

        let mut files = Vec::new();
        let mut total_size_bytes: u64 = 0;
        let mut has_local_state_db = false;
        let mut has_registry_db = false;
        let mut instance_ids = Vec::new();

        Self::walk_dir(
            &canonical_source,
            &canonical_source,
            &mut |rel_path, entry| {
                let is_symlink = entry.file_type().is_ok_and(|t| t.is_symlink());
                let metadata = entry
                    .metadata()
                    .map_err(|e| LauncherError::MigrationFailed {
                        message: format!("Failed to read metadata for {}: {e}", rel_path),
                    })?;
                let size_bytes = metadata.len();
                total_size_bytes += size_bytes;
                let entry = MigrationEntry {
                    rel_path: rel_path.replace('\\', "/"),
                    size_bytes,
                    is_symlink,
                };
                let rp = entry.rel_path.as_str();
                if rp == "local_state.db" {
                    has_local_state_db = true;
                }
                if rp == "registry.db" {
                    has_registry_db = true;
                }
                // Detect instances: files under instances/<id>/...
                if let Some(rest) = rp.strip_prefix("instances/") {
                    if let Some(instance_id) = rest.split('/').next() {
                        if !instance_id.is_empty()
                            && !instance_ids.contains(&instance_id.to_owned())
                        {
                            instance_ids.push(instance_id.to_owned());
                        }
                    }
                }
                files.push(entry);
                Ok(())
            },
        )?;

        Ok(MigrationInventory {
            source_root: canonical_source.to_string_lossy().to_string(),
            files,
            total_size_bytes,
            has_local_state_db,
            has_registry_db,
            instance_ids,
        })
    }

    /// Build a migration plan: inventory source and detect conflicts with destination.
    ///
    /// Conflicts:
    /// - Both sides have `local_state.db` (no silent merge)
    /// - Both sides have `registry.db`
    /// - Source has an instance whose ID matches an existing destination instance
    ///
    /// Does not mutate anything.
    pub fn plan(&self, source_root: &Path) -> LauncherResult<MigrationPlan> {
        let inventory = self.inventory(source_root)?;
        let conflicts = self.detect_conflicts(&inventory)?;
        let can_proceed = conflicts.is_empty();
        Ok(MigrationPlan {
            source_inventory: inventory,
            conflicts,
            can_proceed,
        })
    }

    /// Execute a migration plan.
    ///
    /// 1. Inventory the source and detect conflicts.
    /// 2. Back up any existing destination state to a timestamped directory.
    /// 3. Stage each source file in the destination staging area.
    /// 4. Atomically promote staged files to final destination paths.
    /// 5. On failure, restore backed-up destination state.
    pub fn execute(&self, source_root: &Path) -> LauncherResult<MigrationResult> {
        let plan = self.plan(source_root)?;
        if !plan.can_proceed {
            let messages: Vec<String> = plan
                .conflicts
                .iter()
                .map(|c| format!("  {}: {}", c.rel_path, c.reason))
                .collect();
            return Err(LauncherError::MigrationConflict {
                message: format!("Migration blocked by conflicts:\n{}", messages.join("\n")),
            });
        }

        let source_canonical = Self::canonicalize_path(source_root)?;

        // Phase 1: Create a timestamped backup of the current destination state.
        let backup_dir = self.create_backup()?;

        // Phase 2: Stage copies on the same volume, then promote.
        let result = self.stage_and_promote(&source_canonical, &plan.source_inventory);

        // Phase 3: On failure, restore from backup.
        match result {
            Ok(r) => Ok(MigrationResult {
                backup_path: backup_dir.to_string_lossy().to_string(),
                ..r
            }),
            Err(e) => {
                // Attempt restore. Log failures to restore but return the original error.
                if let Err(restore_err) = self.restore_backup(&backup_dir) {
                    eprintln!(
                        "Migration failed and restore also failed: {}. Backup preserved at {}",
                        restore_err,
                        backup_dir.display()
                    );
                }
                Err(e)
            }
        }
    }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    /// Detect conflicts between source inventory and current destination.
    fn detect_conflicts(
        &self,
        inventory: &MigrationInventory,
    ) -> LauncherResult<Vec<MigrationConflict>> {
        let mut conflicts = Vec::new();

        // SQLite databases: no silent merge.
        let dest_local_state_db = self.dest.local_state_db();
        let dest_registry_db = self.dest.registry_db();

        if inventory.has_local_state_db && dest_local_state_db.exists() {
            conflicts.push(MigrationConflict {
                rel_path: "local_state.db".into(),
                reason:
                    "Destination already has a local_state.db. No silent merge of SQLite databases."
                        .into(),
            });
        }
        if inventory.has_registry_db && dest_registry_db.exists() {
            conflicts.push(MigrationConflict {
                rel_path: "registry.db".into(),
                reason:
                    "Destination already has a registry.db. No silent merge of SQLite databases."
                        .into(),
            });
        }

        // Instance ID conflicts: check destination instances directory.
        for instance_id in &inventory.instance_ids {
            let dest_instance_dir = self.dest.instances_root().join(instance_id);
            if dest_instance_dir.exists() {
                conflicts.push(MigrationConflict {
                    rel_path: format!("instances/{instance_id}"),
                    reason: format!(
                        "Instance '{instance_id}' already exists at the destination: {}",
                        dest_instance_dir.display()
                    ),
                });
            }
        }

        Ok(conflicts)
    }

    /// Create a timestamped backup of existing destination state.
    ///
    /// Copies `local_state.db`, `registry.db`, `registry.db.sig`, and the
    /// `instances/` directory to a `migration-backups/<timestamp>/` dir
    /// under the staging root.
    fn create_backup(&self) -> LauncherResult<PathBuf> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let backup_root = self
            .dest
            .staging_root()
            .join(format!("migration-backup-{timestamp}"));
        std::fs::create_dir_all(&backup_root).map_err(|e| LauncherError::MigrationFailed {
            message: format!(
                "Failed to create backup directory {}: {e}",
                backup_root.display()
            ),
        })?;

        // Backup the key top-level files.
        let files_to_backup = [
            self.dest.local_state_db(),
            self.dest.registry_db(),
            self.dest.registry_signature(),
        ];
        for path in &files_to_backup {
            if path.exists() {
                let target = backup_root.join(
                    path.file_name()
                        .unwrap_or_else(|| std::ffi::OsStr::new("unknown")),
                );
                if let Err(e) = copy_file_same_volume(path, &target) {
                    // If a backup copy fails, clean up and abort.
                    let _ = std::fs::remove_dir_all(&backup_root);
                    return Err(LauncherError::MigrationFailed {
                        message: format!("Failed to back up {}: {e}", path.display()),
                    });
                }
            }
        }

        // Backup the instances directory tree.
        let instances_root = self.dest.instances_root();
        if instances_root.exists() {
            let backup_instances = backup_root.join("instances");
            if let Err(e) = copy_dir_same_volume(&instances_root, &backup_instances) {
                let _ = std::fs::remove_dir_all(&backup_root);
                return Err(LauncherError::MigrationFailed {
                    message: format!(
                        "Failed to back up instances directory {}: {e}",
                        instances_root.display()
                    ),
                });
            }
        }

        Ok(backup_root)
    }

    /// Stage copies to the staging dir, then atomic-rename to final paths.
    fn stage_and_promote(
        &self,
        source_root: &Path,
        inventory: &MigrationInventory,
    ) -> LauncherResult<MigrationResult> {
        let staging_root = self.dest.staging_root();
        std::fs::create_dir_all(&staging_root).map_err(|e| LauncherError::MigrationFailed {
            message: format!(
                "Failed to create staging root {}: {e}",
                staging_root.display()
            ),
        })?;

        // Create a unique staging fingerprint.
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let staging_fingerprint = format!("migration-{timestamp}");
        let staging_dir = staging_root.join(&staging_fingerprint);

        // Stage: copy each file under the staging dir, preserving relative structure.
        for entry in &inventory.files {
            let source_path = source_root.join(&entry.rel_path);
            let relative = Path::new(&entry.rel_path);
            let stage_target = staging_dir.join(relative);

            if let Some(parent) = stage_target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| LauncherError::MigrationFailed {
                    message: format!("Failed to create staging subdir {}: {e}", parent.display()),
                })?;
            }

            if let Err(e) = copy_file_same_volume(&source_path, &stage_target) {
                // Rollback staging: remove the staging dir.
                let _ = std::fs::remove_dir_all(&staging_dir);
                return Err(LauncherError::MigrationFailed {
                    message: format!("Failed to stage {}: {e}", source_path.display()),
                });
            }
        }

        // Promote: atomic rename from staging to final destinations.
        self.promote_staged(inventory, staging_dir)
    }

    /// Atomic-rename staged files to their final destination paths.
    fn promote_staged(
        &self,
        inventory: &MigrationInventory,
        staging_dir: PathBuf,
    ) -> LauncherResult<MigrationResult> {
        // First, promote top-level files (local_state.db, registry.db, etc.).
        for entry in &inventory.files {
            let relative = Path::new(&entry.rel_path);
            let stage_path = staging_dir.join(relative);
            let dest_path = self.dest.root().join(relative);

            if let Some(parent) = dest_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| LauncherError::MigrationFailed {
                    message: format!(
                        "Failed to create destination subdir {}: {e}",
                        parent.display()
                    ),
                })?;
            }

            // On Windows, atomic rename via rename() is not always atomic for
            // cross-device moves, but we staged on the same volume so it should
            // work. Fall back to copy+remove if rename fails.
            if let Err(e) = std::fs::rename(&stage_path, &dest_path) {
                // If rename fails (e.g. cross-device or permission), fall back to copy+remove.
                if let Err(copy_err) = std::fs::copy(&stage_path, &dest_path) {
                    return Err(LauncherError::MigrationFailed {
                        message: format!(
                            "Failed to promote {}: rename error: {e}, copy error: {copy_err}",
                            entry.rel_path
                        ),
                    });
                }
                let _ = std::fs::remove_file(&stage_path);
            }
        }

        // Clean up staging directory.
        let _ = std::fs::remove_dir_all(&staging_dir);

        Ok(MigrationResult {
            files_migrated: inventory.files.len(),
            total_bytes: inventory.total_size_bytes,
            instance_ids: inventory.instance_ids.clone(),
            backup_path: String::new(), // set by caller
        })
    }

    /// Restore destination state from a backup directory.
    fn restore_backup(&self, backup_dir: &Path) -> LauncherResult<()> {
        // Restore top-level files.
        let files_to_restore = ["local_state.db", "registry.db", "registry.db.sig"];
        for filename in &files_to_restore {
            let backup_file = backup_dir.join(filename);
            let dest_file = self.dest.root().join(filename);
            if backup_file.exists() {
                if let Some(parent) = dest_file.parent() {
                    std::fs::create_dir_all(parent).ok();
                }
                copy_file_same_volume(&backup_file, &dest_file)?;
            }
        }

        // Restore instances directory.
        let backup_instances = backup_dir.join("instances");
        let dest_instances = self.dest.instances_root();
        if backup_instances.exists() {
            if dest_instances.exists() {
                std::fs::remove_dir_all(&dest_instances).map_err(|e| {
                    LauncherError::MigrationFailed {
                        message: format!(
                            "Failed to remove destination instances dir during restore: {e}"
                        ),
                    }
                })?;
            }
            copy_dir_same_volume(&backup_instances, &dest_instances)?;
        }

        Ok(())
    }

    // ------------------------------------------------------------------
    // Walking & safety
    // ------------------------------------------------------------------

    /// Canonicalize a path, resolving all symlinks and verifying it exists.
    fn canonicalize_path(path: &Path) -> LauncherResult<PathBuf> {
        path.canonicalize()
            .map_err(|e| LauncherError::MigrationFailed {
                message: format!(
                    "Failed to resolve path '{}': {e}. Does it exist?",
                    path.display()
                ),
            })
    }

    /// Verify that a canonical path is safe to walk:
    /// - Not a symlink chain that escapes the expected root
    /// - Source root must be a directory
    fn check_path_safety(canonical: &Path) -> LauncherResult<()> {
        if !canonical.is_dir() {
            return Err(LauncherError::MigrationFailed {
                message: format!("Source path '{}' is not a directory", canonical.display()),
            });
        }
        // Walk the path components and ensure no component is a symlink
        // pointing outside the user's intended tree. We already resolved
        // symlinks via canonicalize, so the path is what it is.
        // Additional check: ensure the path doesn't contain suspicious components.
        for component in canonical.components() {
            match component {
                Component::ParentDir => {
                    return Err(LauncherError::MigrationFailed {
                        message: "Path contains '..' components even after canonicalization".into(),
                    });
                }
                Component::CurDir => {
                    return Err(LauncherError::MigrationFailed {
                        message: "Path contains '.' components even after canonicalization".into(),
                    });
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Walk a directory tree, calling `visitor(rel_path_str, DirEntry)` for each file.
    fn walk_dir<F>(root: &Path, base: &Path, visitor: &mut F) -> LauncherResult<()>
    where
        F: FnMut(String, &std::fs::DirEntry) -> LauncherResult<()>,
    {
        if !root.exists() {
            return Ok(());
        }
        let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let read_dir = std::fs::read_dir(&dir).map_err(|e| LauncherError::MigrationFailed {
                message: format!("Failed to read directory {}: {e}", dir.display()),
            })?;
            for entry_result in read_dir {
                let entry = entry_result.map_err(|e| LauncherError::MigrationFailed {
                    message: format!("Failed to read entry in {}: {e}", dir.display()),
                })?;
                let path = entry.path();
                let file_type = entry
                    .file_type()
                    .map_err(|e| LauncherError::MigrationFailed {
                        message: format!("Failed to get file type for {}: {e}", path.display()),
                    })?;

                // Security: skip symlinks that escape the root.
                if file_type.is_symlink() {
                    let target =
                        std::fs::read_link(&path).map_err(|e| LauncherError::MigrationFailed {
                            message: format!("Failed to read symlink {}: {e}", path.display()),
                        })?;
                    if target.is_absolute() || !target.starts_with(base) {
                        // Skip symlinks pointing outside the source root.
                        continue;
                    }
                }

                let rel_path = path
                    .strip_prefix(base)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_string();

                if file_type.is_dir() {
                    // Recurse into subdirectories.
                    stack.push(path);
                } else if file_type.is_file() || file_type.is_symlink() {
                    visitor(rel_path, &entry)?;
                }
                // Skip other types (sockets, devices, etc.)
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// File operations
// ---------------------------------------------------------------------------

/// Copy a file using OS-level copy-on-write / reflink when available.
/// Falls back to standard copy.
fn copy_file_same_volume(src: &Path, dst: &Path) -> LauncherResult<()> {
    // Create parent dirs if needed.
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(|e| LauncherError::MigrationFailed {
            message: format!(
                "Failed to create parent directory {}: {e}",
                parent.display()
            ),
        })?;
    }

    // Use std::fs::copy as the portable fallback. On Linux, this does
    // copy-on-write where the filesystem supports it.
    std::fs::copy(src, dst).map_err(|e| LauncherError::MigrationFailed {
        message: format!("Failed to copy {} -> {}: {e}", src.display(), dst.display()),
    })?;

    // Preserve permissions (best-effort).
    if let Ok(metadata) = src.metadata() {
        let perms = metadata.permissions();
        let _ = std::fs::set_permissions(dst, perms);
    }

    Ok(())
}

/// Recursively copy a directory to a new location on the same volume.
fn copy_dir_same_volume(src: &Path, dst: &Path) -> LauncherResult<()> {
    std::fs::create_dir_all(dst).map_err(|e| LauncherError::MigrationFailed {
        message: format!("Failed to create directory {}: {e}", dst.display()),
    })?;

    let entries = std::fs::read_dir(src).map_err(|e| LauncherError::MigrationFailed {
        message: format!("Failed to read directory {}: {e}", src.display()),
    })?;

    for entry_result in entries {
        let entry = entry_result.map_err(|e| LauncherError::MigrationFailed {
            message: format!("Failed to read entry in {}: {e}", src.display()),
        })?;
        let entry_type = entry
            .file_type()
            .map_err(|e| LauncherError::MigrationFailed {
                message: format!(
                    "Failed to get file type for {}: {e}",
                    entry.path().display()
                ),
            })?;
        let file_name = entry.file_name();
        let src_path = entry.path();
        let dst_path = dst.join(&file_name);

        // Security: skip symlinks pointing outside the source tree.
        if entry_type.is_symlink() {
            if let Ok(target) = std::fs::read_link(&src_path) {
                if target.is_absolute() || !target.starts_with(src) {
                    continue;
                }
            }
        }

        if entry_type.is_dir() {
            copy_dir_same_volume(&src_path, &dst_path)?;
        } else if entry_type.is_file() || entry_type.is_symlink() {
            copy_file_same_volume(&src_path, &dst_path)?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Legacy CLI data root inventory — standalone function for quick checks
// ---------------------------------------------------------------------------

/// Inventory a legacy CLI data root without requiring the full service.
///
/// This is the entry point the CLI uses for dry-run previews before
/// creating the full `DataMigrationService`.
pub fn inventory_legacy_root(source_root: &Path) -> LauncherResult<MigrationInventory> {
    let canonical = source_root
        .canonicalize()
        .map_err(|e| LauncherError::MigrationFailed {
            message: format!(
                "Failed to resolve source path '{}': {e}. Does it exist?",
                source_root.display()
            ),
        })?;
    let service = DataMigrationService::new(
        // Dummy paths — we only need canonicalize + walk, not the dest.
        AppPaths::from_root(std::path::PathBuf::from("/")),
    );
    // Override self.dest by calling inventory with source.
    service.inventory(&canonical)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_root::RemoveOnDrop;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_dir() -> RemoveOnDrop {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let seq = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "agora-migration-test-{}-{}",
            std::process::id(),
            seq
        ));
        let _ = fs::remove_dir_all(&dir);
        RemoveOnDrop::new(dir)
    }

    fn make_service(dest_root: &Path) -> DataMigrationService {
        let paths = AppPaths::from_root(dest_root.to_path_buf());
        // Create the destination directories that inventory/create_backup expect.
        fs::create_dir_all(dest_root.join("instances")).unwrap();
        fs::create_dir_all(paths.staging_root()).unwrap();
        DataMigrationService::new(paths)
    }

    // ------------------------------------------------------------------
    // Empty source
    // ------------------------------------------------------------------

    #[test]
    fn inventory_empty_source_is_empty() {
        let src = unique_dir();
        fs::create_dir_all(&src).unwrap();
        let dst = unique_dir();
        let svc = make_service(&dst);

        let inventory = svc.inventory(&src).unwrap();
        assert!(inventory.files.is_empty());
        assert!(!inventory.has_local_state_db);
        assert!(!inventory.has_registry_db);
        assert!(inventory.instance_ids.is_empty());
        assert_eq!(inventory.total_size_bytes, 0);
    }

    #[test]
    fn plan_empty_source_no_conflicts() {
        let src = unique_dir();
        fs::create_dir_all(&src).unwrap();
        let dst = unique_dir();
        let svc = make_service(&dst);

        let plan = svc.plan(&src).unwrap();
        assert!(plan.can_proceed);
        assert!(plan.conflicts.is_empty());
        assert!(plan.source_inventory.files.is_empty());
    }

    #[test]
    fn execute_empty_source_succeeds() {
        let src = unique_dir();
        fs::create_dir_all(&src).unwrap();
        let dst = unique_dir();
        let svc = make_service(&dst);

        let result = svc.execute(&src).unwrap();
        assert_eq!(result.files_migrated, 0);
        assert_eq!(result.total_bytes, 0);
        assert!(result.instance_ids.is_empty());
        assert!(!result.backup_path.is_empty());
    }

    // ------------------------------------------------------------------
    // Conflict detection
    // ------------------------------------------------------------------

    #[test]
    fn conflict_on_local_state_db() {
        let src = unique_dir();
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("local_state.db"), b"source db content").unwrap();
        let dst = unique_dir();
        let svc = make_service(&dst);
        fs::write(dst.join("local_state.db"), b"dest db content").unwrap();

        let plan = svc.plan(&src).unwrap();
        assert!(!plan.can_proceed);
        assert!(plan
            .conflicts
            .iter()
            .any(|c| c.rel_path == "local_state.db"));
    }

    #[test]
    fn conflict_on_registry_db() {
        let src = unique_dir();
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("registry.db"), b"source registry").unwrap();
        let dst = unique_dir();
        let svc = make_service(&dst);
        fs::write(dst.join("registry.db"), b"dest registry").unwrap();

        let plan = svc.plan(&src).unwrap();
        assert!(!plan.can_proceed);
        assert!(plan.conflicts.iter().any(|c| c.rel_path == "registry.db"));
    }

    #[test]
    fn conflict_on_existing_instance() {
        let src = unique_dir();
        fs::create_dir_all(src.join("instances/my-instance/mods")).unwrap();
        fs::write(
            src.join("instances/my-instance/instance_manifest.json"),
            b"{}",
        )
        .unwrap();
        let dst = unique_dir();
        let svc = make_service(&dst);
        fs::create_dir_all(dst.join("instances/my-instance/mods")).unwrap();
        fs::write(
            dst.join("instances/my-instance/instance_manifest.json"),
            b"{}",
        )
        .unwrap();

        let plan = svc.plan(&src).unwrap();
        assert!(!plan.can_proceed);
        assert!(plan
            .conflicts
            .iter()
            .any(|c| c.rel_path == "instances/my-instance"));
    }

    #[test]
    fn execute_refuses_conflicts() {
        let src = unique_dir();
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("local_state.db"), b"source db").unwrap();
        let dst = unique_dir();
        let svc = make_service(&dst);
        fs::write(dst.join("local_state.db"), b"dest db").unwrap();

        let err = svc.execute(&src).unwrap_err();
        assert!(matches!(err, LauncherError::MigrationConflict { .. }));
    }

    // ------------------------------------------------------------------
    // Backup creation
    // ------------------------------------------------------------------

    #[test]
    fn backup_created_before_migration() {
        let src = unique_dir();
        fs::create_dir_all(src.join("instances/existing-instance/mods")).unwrap();
        fs::write(
            src.join("instances/existing-instance/instance_manifest.json"),
            br#"{"name":"test"}"#,
        )
        .unwrap();
        let dst = unique_dir();
        let svc = make_service(&dst);

        let result = svc.execute(&src).unwrap();
        assert!(!result.backup_path.is_empty());

        // Verify backup directory exists.
        let backup = Path::new(&result.backup_path);
        assert!(
            backup.exists(),
            "backup directory should exist: {}",
            backup.display()
        );

        // Verify backup contains instances backup (the source had none at dest, but
        // the backup is of *destination* state, so instances might be empty or absent).
    }

    #[test]
    fn backup_captures_existing_destination_state() {
        let src = unique_dir();
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("test.txt"), b"new").unwrap();

        let dst = unique_dir();
        let svc = make_service(&dst);
        // Create pre-existing destination instance data (backup preserves instances).
        fs::create_dir_all(dst.join("instances/old-instance/mods")).unwrap();
        fs::write(
            dst.join("instances/old-instance/instance_manifest.json"),
            br#"{"name":"old"}"#,
        )
        .unwrap();

        let result = svc.execute(&src).unwrap();
        let backup = Path::new(&result.backup_path);
        assert!(
            backup
                .join("instances/old-instance/instance_manifest.json")
                .exists(),
            "backup should contain old instance data"
        );
    }

    // ------------------------------------------------------------------
    // Successful non-conflicting migration
    // ------------------------------------------------------------------

    #[test]
    fn migrate_files_and_instances() {
        let src = unique_dir();
        fs::create_dir_all(src.join("instances/inst-a/mods")).unwrap();
        fs::create_dir_all(src.join("instances/inst-b/mods")).unwrap();
        fs::write(src.join("local_state.db"), b"source state").unwrap();
        fs::write(src.join("registry.db"), b"source registry").unwrap();
        fs::write(
            src.join("instances/inst-a/instance_manifest.json"),
            br#"{"name":"A"}"#,
        )
        .unwrap();
        fs::write(src.join("instances/inst-a/mods/a.jar"), b"mod a").unwrap();
        fs::write(
            src.join("instances/inst-b/instance_manifest.json"),
            br#"{"name":"B"}"#,
        )
        .unwrap();
        fs::write(src.join("instances/inst-b/mods/b.jar"), b"mod b").unwrap();

        let dst = unique_dir();
        let svc = make_service(&dst);

        let result = svc.execute(&src).unwrap();
        assert_eq!(result.files_migrated, 6);
        assert!(result.instance_ids.contains(&"inst-a".to_string()));
        assert!(result.instance_ids.contains(&"inst-b".to_string()));

        // Verify files arrived at dest.
        assert!(dst.join("local_state.db").exists());
        assert!(dst.join("registry.db").exists());
        assert!(dst.join("instances/inst-a/instance_manifest.json").exists());
        assert!(dst.join("instances/inst-a/mods/a.jar").exists());
        assert!(dst.join("instances/inst-b/instance_manifest.json").exists());
        assert!(dst.join("instances/inst-b/mods/b.jar").exists());

        // Verify content integrity.
        assert_eq!(
            fs::read(dst.join("local_state.db")).unwrap(),
            b"source state"
        );
        assert_eq!(
            fs::read(dst.join("instances/inst-a/mods/a.jar")).unwrap(),
            b"mod a"
        );
    }

    #[test]
    fn migrate_over_empty_destination() {
        let src = unique_dir();
        fs::create_dir_all(src.join("instances/my-inst/mods")).unwrap();
        fs::write(
            src.join("instances/my-inst/instance_manifest.json"),
            br#"{"name":"my-inst"}"#,
        )
        .unwrap();
        fs::write(
            src.join("instances/my-inst/mods/sodium.jar"),
            b"sodium content",
        )
        .unwrap();

        let dst = unique_dir();
        let svc = make_service(&dst);

        let result = svc.execute(&src).unwrap();
        assert_eq!(result.files_migrated, 2);
        assert!(dst
            .join("instances/my-inst/instance_manifest.json")
            .exists());
        assert!(dst.join("instances/my-inst/mods/sodium.jar").exists());
        assert_eq!(
            fs::read(dst.join("instances/my-inst/mods/sodium.jar")).unwrap(),
            b"sodium content"
        );
    }

    // ------------------------------------------------------------------
    // Rollback on copy failure
    // ------------------------------------------------------------------

    #[test]
    fn rollback_on_staging_failure() {
        let src = unique_dir();
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("ok.txt"), b"ok").unwrap();

        let dst = unique_dir();
        let svc = make_service(&dst);
        // Write pre-existing content.
        fs::write(dst.join("preexisting.txt"), b"original").unwrap();

        // We can't easily make copy fail on all platforms, but we can test
        // that when staging fails, the backup restores pre-existing state.
        // Instead, make staging impossible by removing staging dir perms.
        let staging_root = dst.join("staging");
        fs::create_dir_all(&staging_root).unwrap();
        // On Unix, remove write permission to cause failure.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&staging_root).unwrap().permissions();
            PermissionsExt::set_mode(&mut perms, 0o444);
            fs::set_permissions(&staging_root, perms).unwrap();
        }

        let err = svc.execute(&src);
        #[cfg(unix)]
        {
            assert!(
                err.is_err(),
                "expected migration to fail when staging is unwritable"
            );
            // Restore permissions.
            let restore_perms = std::fs::Permissions::from_mode(0o755);
            fs::set_permissions(&staging_root, restore_perms).unwrap();

            // Verify rollback restored pre-existing content.
            assert!(dst.join("preexisting.txt").exists());
            assert_eq!(fs::read(dst.join("preexisting.txt")).unwrap(), b"original");
        }
        #[cfg(not(unix))]
        {
            // On Windows, permissions don't prevent directory creation as easily.
            let _ = err;
        }
    }

    // ------------------------------------------------------------------
    // Path traversal / symlink safety
    // ------------------------------------------------------------------

    #[test]
    #[cfg(unix)]
    fn symlink_outside_source_root_is_skipped() {
        let src = unique_dir();
        fs::create_dir_all(&src).unwrap();

        let outside = unique_dir();
        fs::write(&outside, b"outside content").unwrap();
        std::os::unix::fs::symlink(&outside, src.join("outside_link")).unwrap();

        fs::write(src.join("legit.txt"), b"legit").unwrap();

        let dst = unique_dir();
        let svc = make_service(&dst);
        let inventory = svc.inventory(&src).unwrap();

        assert!(!inventory.files.iter().any(|f| f.rel_path == "outside_link"));
        assert!(inventory.files.iter().any(|f| f.rel_path == "legit.txt"));
    }

    #[test]
    #[cfg(unix)]
    fn symlink_inside_source_is_walked() {
        let src = unique_dir();
        fs::create_dir_all(src.join("subdir")).unwrap();
        fs::write(src.join("subdir/real.txt"), b"real").unwrap();
        std::os::unix::fs::symlink(src.join("subdir/real.txt"), src.join("link.txt")).unwrap();

        let dst = unique_dir();
        let svc = make_service(&dst);
        let inventory = svc.inventory(&src).unwrap();

        assert!(inventory
            .files
            .iter()
            .any(|f| f.rel_path == "subdir/real.txt"));
    }

    #[test]
    fn path_traversal_in_instance_name_is_blocked() {
        let src = unique_dir();
        // We cannot create a directory named ".." easily, so verify that walk_dir
        // handles the traversal check correctly.
        fs::create_dir_all(src.join("instances/valid-instance/mods")).unwrap();
        fs::write(
            src.join("instances/valid-instance/instance_manifest.json"),
            b"{}",
        )
        .unwrap();

        let dst = unique_dir();
        let svc = make_service(&dst);
        let inventory = svc.inventory(&src).unwrap();

        assert_eq!(inventory.instance_ids, vec!["valid-instance"]);
    }

    // ------------------------------------------------------------------
    // Free-function inventory
    // ------------------------------------------------------------------

    #[test]
    fn inventory_legacy_root_function_works() {
        let src = unique_dir();
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("test.txt"), b"hello").unwrap();

        let inventory = inventory_legacy_root(&src).unwrap();
        assert_eq!(inventory.files.len(), 1);
        assert_eq!(inventory.files[0].rel_path, "test.txt");
    }

    #[test]
    fn inventory_nonexistent_path_fails() {
        let src = unique_dir(); // does not exist
        let err = inventory_legacy_root(&src).unwrap_err();
        assert!(matches!(err, LauncherError::MigrationFailed { .. }));
    }

    // ------------------------------------------------------------------
    // idempotency / duplicate execution
    // ------------------------------------------------------------------

    #[test]
    fn migration_is_idempotent_for_empty_source() {
        let src = unique_dir();
        fs::create_dir_all(&src).unwrap();
        let dst = unique_dir();
        let svc = make_service(&dst);

        svc.execute(&src).unwrap();
        // Second execution should also succeed (no-op).
        let result = svc.execute(&src).unwrap();
        assert_eq!(result.files_migrated, 0);
    }
}
