//! Read-only discovery, planning, and execution for launcher-to-Agora instance imports.
//!
//! ## Public API
//!
//! | Method | Description |
//! |---|---|
//! | [`LauncherImportService::new`] | Create the service from a [`Ctx`]. |
//! | [`LauncherImportService::discover`] | Detect all three launcher types and return an aggregate. |
//! | [`LauncherImportService::plan`] | Build a fingerprinted immutable plan from user selections. |
//! | [`LauncherImportService::execute`] | Execute a plan asynchronously, handling each item independently. |
//!
//! ## Design constraints
//!
//! - Detection is pure adapter-driven and read-only — no network, no source writes.
//! - Planning rejects unsupported / ambiguous loader candidates, collision-resolves
//!   destination IDs, checks provenance by launcher kind + installation key + source
//!   key, and compares the current Agora copy against `instance_import_files` SHA-256
//!   baseline to decide update-vs-copy.
//! - Execution handles each item independently (successes are kept on sibling failure).
//! - All file copies use bounded streaming buffers with concurrent SHA-256 hashing.
//! - Symlinks / reparse points are never followed; known launcher-control files and
//!   `.agora*` internals are excluded from the copy.
//! - Health runs on staging before promotion.
//! - A persistent import job is written before staging and cleaned up after commit.

use crate::ctx::Ctx;
use crate::db::{
    self, find_instance_import_by_source, replace_instance_import, InstanceImportFileRecord,
    InstanceImportJob, InstanceImportRecord,
};
#[cfg(test)]
use crate::download::sha256_hex;
use crate::error::{LauncherError, LauncherResult};
use crate::event_sink::{CancellationToken, ProgressEvent, ProgressPhase};

use crate::health::{self, HealthScore};
use crate::launcher_import::{
    self, CandidateStatus, ImportCandidate, LaunchSettingsPreview, LauncherDiscovery, LauncherKind,
    LoaderTuple,
};
use crate::loader_service::LoaderService;
use crate::models::{InstalledMod, InstanceManifest, InstanceRow};
use crate::network::NetworkPolicy;
use serde::{Deserialize, Serialize};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Public types — discovery aggregate
// ---------------------------------------------------------------------------

/// Full discovery result for all three launcher adapters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LauncherImportDiscovery {
    pub prism: LauncherDiscovery,
    pub curseforge: LauncherDiscovery,
    pub modrinth: LauncherDiscovery,
}

impl LauncherImportDiscovery {
    /// Collect every candidate from every launcher whose status is `Ready`.
    pub fn ready_candidates(&self) -> Vec<&ImportCandidate> {
        let mut out: Vec<&ImportCandidate> = Vec::new();
        for d in [&self.prism, &self.curseforge, &self.modrinth] {
            for c in &d.candidates {
                if matches!(c.status, CandidateStatus::Ready) {
                    out.push(c);
                }
            }
        }
        out
    }

    /// Collect all candidates from every launcher (ready, needs-review, unsupported).
    pub fn all_candidates(&self) -> Vec<&ImportCandidate> {
        let mut out: Vec<&ImportCandidate> = Vec::new();
        for d in [&self.prism, &self.curseforge, &self.modrinth] {
            for c in &d.candidates {
                out.push(c);
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Public types — selection & plan
// ---------------------------------------------------------------------------

/// A user selection referencing one import candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportSelection {
    /// The source key that identifies the candidate (from `ImportCandidate.source_key`).
    pub source_key: String,
    /// The launcher kind (Prism, CurseForge, Modrinth).
    pub launcher_kind: LauncherKind,
    /// The launcher installation key that owns this candidate.
    pub installation_key: String,
    /// Optional custom destination display name. When `None`, the source display name is used.
    pub destination_name: Option<String>,
    /// Preserve detected source memory/JVM/Java settings into the Agora instance.
    #[serde(default = "default_true")]
    pub preserve_settings: bool,
}

fn default_true() -> bool {
    true
}

/// Action to take for one planned item.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemAction {
    /// Import a fresh copy (no prior import exists, or Agora copy was modified).
    New,
    /// Update an existing import whose payload still matches the last-import baseline.
    Update,
    /// Source and Agora copy still match the import baseline; no work needed.
    Unchanged,
}

/// Per-item plan within a [`LauncherImportPlan`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerItemPlan {
    /// Stable plan fingerprint component (SHA-256 of serialized plan fields).
    pub fingerprint: String,
    /// The resolved destination Agora instance ID.
    pub destination_id: String,
    /// The resolved destination display name.
    pub destination_name: String,
    pub action: ItemAction,
    /// Source key from the candidate.
    pub source_key: String,
    pub launcher_kind: LauncherKind,
    pub installation_key: String,
    /// Absolute source payload root path.
    pub source_path: String,
    pub loader_tuple: Option<LoaderTuple>,
    pub total_bytes: u64,
    pub total_files: u64,
    /// Whether settings will be preserved.
    pub preserve_settings: bool,
    pub sanitized_settings: LaunchSettingsPreview,
    /// Existing import record (present only when action is Update).
    pub existing_import: Option<InstanceImportRecord>,
    /// Blockers that prevent this item from being executed.
    pub blockers: Vec<String>,
    /// Non-blocking warnings.
    pub warnings: Vec<String>,
}

/// Immutable, fingerprinted batch plan returned by [`LauncherImportService::plan`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LauncherImportPlan {
    /// Fingerprint of the entire batch (SHA-256 of concatenated per-item fingerprints).
    pub batch_fingerprint: String,
    pub items: Vec<PerItemPlan>,
    /// Total bytes to copy / stage across all items.
    pub peak_bytes: u64,
    /// Total file count across all items.
    pub total_files: u64,
    /// Blockers that apply to the entire batch (e.g. disk space, network policy).
    pub batch_blockers: Vec<String>,
}

// ---------------------------------------------------------------------------
// Public types — outcomes
// ---------------------------------------------------------------------------

/// Outcome of a single planned item.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PerItemOutcome {
    Imported {
        instance_id: String,
        warnings: Vec<String>,
    },
    Updated {
        instance_id: String,
        warnings: Vec<String>,
    },
    Skipped {
        reason: String,
    },
    Failed {
        error: String,
        warnings: Vec<String>,
    },
    Cancelled {
        reason: String,
    },
}

/// Batch execution result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LauncherImportBatchResult {
    pub outcomes: Vec<PerItemOutcome>,
}

// ---------------------------------------------------------------------------
// Blockers & helpers for plan construction
// ---------------------------------------------------------------------------

/// Check that a loader tuple is cataloged. If it is vanilla, forge, fabric,
/// quilt, or neoforge and exists in the loader manifests, return Ok.
/// For unsupported exact loader versions we return a blocker for now.
fn check_loader_supported(tuple: &LoaderTuple) -> (Option<LoaderTuple>, Vec<String>, Vec<String>) {
    let mut blockers = Vec::new();
    let warnings: Vec<String> = Vec::new();

    if tuple.loader == "vanilla" {
        return (Some(tuple.clone()), blockers, warnings);
    }

    let entry = crate::loader_manifests::find_entry(
        &tuple.loader,
        &tuple.minecraft_version,
        &tuple.loader_version,
    );

    match entry {
        Some(_) => (Some(tuple.clone()), blockers, warnings),
        None => {
            blockers.push(format!(
                "Unsupported loader version: {} {} for MC {}",
                tuple.loader, tuple.loader_version, tuple.minecraft_version
            ));
            (None, blockers, warnings)
        }
    }
}

/// Validate a destination ID candidate and produce a safe unique ID.
fn resolve_destination_id(
    candidate: &ImportCandidate,
    custom_name: Option<&str>,
    existing_ids: &[String],
) -> String {
    let base = custom_name.unwrap_or(&candidate.display_name).to_string();
    let sanitized = crate::paths::sanitize_id(&base).to_ascii_lowercase();
    if sanitized.is_empty() {
        // Fall back to source_key if the name sanitizes to empty.
        return crate::paths::sanitize_id(&candidate.source_key);
    }
    if !existing_ids.contains(&sanitized) {
        return sanitized;
    }
    // Collision: append a suffix until unique.
    for i in 1..100 {
        let candidate_id = format!("{sanitized}-{i}");
        if !existing_ids.contains(&candidate_id) {
            return candidate_id;
        }
    }
    format!("{}-{}", sanitized, Uuid::new_v4())
}

// ---------------------------------------------------------------------------
// File-copy helpers
// ---------------------------------------------------------------------------

const COPY_BUFFER_SIZE: usize = 64 * 1024; // 64 KiB
/// Files that are excluded from the copy (launcher metadata, Agora internals).
const EXCLUDED_FILENAMES: &[&str] = &[
    ".curseclient",
    "minecraftinstance.json",
    "profile.json",
    "instance_manifest.json",
    ".agora",
    ".agora_snapshots",
    ".agora-import",
];
/// Subdirectory names whose entire tree is excluded.
const EXCLUDED_DIRNAMES: &[&str] = &[".agora", ".agora_snapshots", ".agora-import"];
/// Stream-copy a regular file from `src` to `dst` while computing SHA-256.
/// Never follows symlinks. The caller must verify `src` is a regular file.
#[cfg(test)]
fn copy_file_with_hash(src: &Path, dst: &Path) -> io::Result<String> {
    copy_file_with_hash_control(src, dst, None, &mut |_| {})
}

fn hash_regular_file(path: &Path) -> io::Result<String> {
    let mut input = std::fs::File::open(path)?;
    let mut hasher = sha2::Sha256::new();
    let mut buffer = [0u8; COPY_BUFFER_SIZE];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn copy_file_with_hash_control(
    src: &Path,
    dst: &Path,
    cancellation: Option<&CancellationToken>,
    on_bytes: &mut dyn FnMut(u64),
) -> io::Result<String> {
    let mut src_file = std::fs::File::open(src)?;
    let mut dst_file = std::fs::File::create(dst)?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; COPY_BUFFER_SIZE];
    loop {
        if cancellation.is_some_and(CancellationToken::is_cancelled) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "import cancelled",
            ));
        }
        let n = src_file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        sha2::Digest::update(&mut hasher, &buf[..n]);
        use std::io::Write;
        dst_file.write_all(&buf[..n])?;
        on_bytes(n as u64);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn is_excluded_source_name(name: &str, is_directory: bool) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with(".agora")
        || EXCLUDED_FILENAMES
            .iter()
            .any(|excluded| lower == excluded.to_ascii_lowercase())
        || (is_directory
            && EXCLUDED_DIRNAMES
                .iter()
                .any(|excluded| lower == excluded.to_ascii_lowercase()))
}

fn unsafe_filesystem_entry(_path: &Path, file_type: &std::fs::FileType) -> bool {
    if file_type.is_symlink() {
        return true;
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if std::fs::symlink_metadata(_path)
            .map(|metadata| metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
            .unwrap_or(true)
        {
            return true;
        }
    }
    false
}

use sha2::Digest;

/// Recursively copy files from `src_root` to `dst_root`, computing SHA-256 for
/// all regular files. Never follows symlinks/reparse points. Excludes known
/// launcher-control files and `.agora*` internals. Returns a file inventory.
#[cfg(test)]
fn copy_payload(src_root: &Path, dst_root: &Path) -> Result<Vec<InstanceImportFileRecord>, String> {
    copy_payload_control(src_root, dst_root, None, &mut |_| {})
}

fn copy_payload_control(
    src_root: &Path,
    dst_root: &Path,
    cancellation: Option<&CancellationToken>,
    on_bytes: &mut dyn FnMut(u64),
) -> Result<Vec<InstanceImportFileRecord>, String> {
    if !src_root.is_dir() {
        return Err(format!("Source is not a directory: {}", src_root.display()));
    }

    let mut inventory: Vec<InstanceImportFileRecord> = Vec::new();
    let mut dirs: Vec<PathBuf> = vec![src_root.to_path_buf()];

    while let Some(src_dir) = dirs.pop() {
        let read_dir = match std::fs::read_dir(&src_dir) {
            Ok(rd) => rd,
            Err(_) => continue,
        };

        for entry in read_dir {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            let file_type = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };

            // Never follow symlinks, junctions, or reparse points.
            if unsafe_filesystem_entry(&entry.path(), &file_type) {
                continue;
            }

            let path = entry.path();
            let fname = entry.file_name().to_string_lossy().into_owned();

            // Exclude known launcher/Agora control files and directories.
            if is_excluded_source_name(&fname, file_type.is_dir()) {
                continue;
            }

            if file_type.is_dir() {
                dirs.push(path);
                continue;
            }

            if !file_type.is_file() {
                continue;
            }

            // Compute relative path from src_root.
            let rel = path
                .strip_prefix(src_root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");

            // Build the dest path mirroring the source structure.
            let dest = dst_root.join(&rel);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Cannot create directory {}: {e}", parent.display()))?;
            }

            let hash = copy_file_with_hash_control(&path, &dest, cancellation, on_bytes)
                .map_err(|e| format!("Cannot copy {}: {e}", path.display()))?;

            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

            // Get source modified time in nanoseconds for baseline tracking.
            let source_modified_ns = std::fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos() as i64);

            inventory.push(InstanceImportFileRecord {
                relative_path: rel,
                sha256: hash,
                size,
                source_modified_ns,
            });
        }
    }

    inventory.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(inventory)
}

/// Hash an existing payload without writing a probe copy.
fn hash_payload(root: &Path) -> Result<Vec<InstanceImportFileRecord>, String> {
    if !root.is_dir() {
        return Err(format!("Payload is not a directory: {}", root.display()));
    }
    let mut inventory = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let entries = std::fs::read_dir(&dir)
            .map_err(|error| format!("Cannot read {}: {error}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| error.to_string())?;
            let file_type = entry.file_type().map_err(|error| error.to_string())?;
            if unsafe_filesystem_entry(&entry.path(), &file_type) {
                continue;
            }
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if is_excluded_source_name(&name, file_type.is_dir()) {
                continue;
            }
            if file_type.is_dir() {
                dirs.push(path);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let relative_path = path
                .strip_prefix(root)
                .map_err(|error| error.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            let metadata = entry.metadata().map_err(|error| error.to_string())?;
            let source_modified_ns = metadata
                .modified()
                .ok()
                .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|value| value.as_nanos() as i64);
            inventory.push(InstanceImportFileRecord {
                relative_path,
                sha256: hash_regular_file(&path)
                    .map_err(|error| format!("Cannot hash {}: {error}", path.display()))?,
                size: metadata.len(),
                source_modified_ns,
            });
        }
    }
    inventory.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(inventory)
}

/// Build canonical `InstanceManifest` arrays for mods, resourcepacks, shaders,
/// datapacks, and worlds from staged content with a given source label.
type ManifestInventories = (
    Vec<InstalledMod>,
    Vec<InstalledMod>,
    Vec<InstalledMod>,
    Vec<InstalledMod>,
    Vec<InstalledMod>,
);

fn build_manifest_arrays(
    instance_dir: &Path,
    source_label: &str,
    loader: &str,
) -> LauncherResult<ManifestInventories> {
    let mods = inventory_directory(instance_dir, "mods", "mod", source_label, loader)?;
    let resourcepacks = inventory_directory(
        instance_dir,
        "resourcepacks",
        "resourcepack",
        source_label,
        loader,
    )?;
    let shaders = inventory_directory(instance_dir, "shaderpacks", "shader", source_label, loader)?;
    let datapacks =
        inventory_directory(instance_dir, "datapacks", "datapack", source_label, loader)?;
    let worlds = inventory_directory(instance_dir, "saves", "world", source_label, loader)?;
    Ok((mods, resourcepacks, shaders, datapacks, worlds))
}

/// Scan one content subdirectory and build `InstalledMod` entries.
fn inventory_directory(
    instance_dir: &Path,
    subdir: &str,
    content_type: &str,
    source: &str,
    loader: &str,
) -> LauncherResult<Vec<InstalledMod>> {
    let root = instance_dir.join(subdir);
    if !root.is_dir() {
        return Ok(Vec::new());
    }

    let mut items: Vec<InstalledMod> = Vec::new();
    let entries = match std::fs::read_dir(&root) {
        Ok(e) => e,
        Err(_) => return Ok(Vec::new()),
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let ft = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };

        if unsafe_filesystem_entry(&entry.path(), &ft) || (!ft.is_file() && !ft.is_dir()) {
            continue;
        }

        let fname = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let sha = match hash_content_entry(&path) {
            Ok(value) => value,
            Err(_) => continue,
        };

        // Track disabled .jar.disabled files as disabled mods.
        let (filename, enabled) = if ft.is_file() {
            if let Some(stripped) = fname.strip_suffix(".disabled") {
                if stripped.ends_with(".jar") {
                    (stripped.to_string(), false)
                } else {
                    (fname.clone(), true)
                }
            } else {
                (fname.clone(), true)
            }
        } else {
            (fname.clone(), true)
        };

        // Only attempt JAR metadata for .jar files.
        let (
            mod_jar_id,
            version,
            java_packages,
            provided_mod_ids,
            depends_on,
            optional_deps,
            incompatible_deps,
        ) = if filename.ends_with(".jar") && ft.is_file() {
            let meta = crate::jar_metadata::parse_jar_metadata_for_loader(&path, loader);
            (
                meta.mod_jar_id.clone(),
                meta.mod_version,
                meta.java_packages,
                meta.provided_mods
                    .into_iter()
                    .map(|provided| provided.mod_id)
                    .collect(),
                meta.depends_on,
                meta.optional_deps,
                meta.incompatible_deps,
            )
        } else {
            (
                None,
                None,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            )
        };

        items.push(InstalledMod {
            filename,
            registry_id: None,
            modrinth_id: None,
            source: format!("imported_{source}"),
            source_url: None,
            version,
            sha256: sha,
            installed_at: chrono::Utc::now().to_rfc3339(),
            java_packages,
            mod_jar_id,
            provided_mod_ids,
            enabled,
            content_type: content_type.to_string(),
            depends_on,
            optional_deps,
            incompatible_deps,
        });
    }

    items.sort_by(|a, b| a.filename.cmp(&b.filename));
    Ok(items)
}

fn hash_content_entry(path: &Path) -> LauncherResult<String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| LauncherError::Generic {
        code: "ERR_IMPORT_HASH".into(),
        message: error.to_string(),
    })?;
    if unsafe_filesystem_entry(path, &metadata.file_type()) {
        return Err(LauncherError::Generic {
            code: "ERR_IMPORT_UNSAFE_PATH".into(),
            message: format!("Refusing to hash symlink {}", path.display()),
        });
    }
    if metadata.is_file() {
        return hash_regular_file(path).map_err(|error| LauncherError::Generic {
            code: "ERR_IMPORT_HASH".into(),
            message: error.to_string(),
        });
    }
    let mut files = Vec::new();
    if metadata.is_dir() {
        let mut dirs = vec![path.to_path_buf()];
        while let Some(directory) = dirs.pop() {
            for entry in std::fs::read_dir(&directory).map_err(|error| LauncherError::Generic {
                code: "ERR_IMPORT_HASH".into(),
                message: error.to_string(),
            })? {
                let entry = entry.map_err(|error| LauncherError::Generic {
                    code: "ERR_IMPORT_HASH".into(),
                    message: error.to_string(),
                })?;
                let file_type = entry.file_type().map_err(|error| LauncherError::Generic {
                    code: "ERR_IMPORT_HASH".into(),
                    message: error.to_string(),
                })?;
                if unsafe_filesystem_entry(&entry.path(), &file_type) {
                    continue;
                }
                if file_type.is_dir() {
                    dirs.push(entry.path());
                } else if file_type.is_file() {
                    files.push(entry.path());
                }
            }
        }
    }
    files.sort();
    let mut hasher = sha2::Sha256::new();
    for file in files {
        let relative = file.strip_prefix(path).unwrap_or(&file).to_string_lossy();
        let file_size = std::fs::metadata(&file)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        hasher.update((relative.len() as u64).to_le_bytes());
        hasher.update(relative.as_bytes());
        hasher.update(file_size.to_le_bytes());
        let mut input = std::fs::File::open(&file).map_err(|error| LauncherError::Generic {
            code: "ERR_IMPORT_HASH".into(),
            message: error.to_string(),
        })?;
        let mut buffer = [0u8; COPY_BUFFER_SIZE];
        loop {
            let read = input
                .read(&mut buffer)
                .map_err(|error| LauncherError::Generic {
                    code: "ERR_IMPORT_HASH".into(),
                    message: error.to_string(),
                })?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Compute a stable plan fingerprint from the serialized PerItemPlan fields.
fn plan_fingerprint(
    source_key: &str,
    launcher_kind: LauncherKind,
    destination_id: &str,
    action: &ItemAction,
    preserve_settings: bool,
) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(source_key.as_bytes());
    hasher.update(b"|");
    hasher.update(
        serde_json::to_string(&launcher_kind)
            .unwrap_or_default()
            .as_bytes(),
    );
    hasher.update(b"|");
    hasher.update(destination_id.as_bytes());
    hasher.update(b"|");
    hasher.update(serde_json::to_string(action).unwrap_or_default().as_bytes());
    hasher.update(b"|");
    hasher.update(if preserve_settings { b"1" } else { b"0" });
    hex::encode(hasher.finalize())
}

fn batch_fingerprint(item_fingerprints: &[String]) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    for fp in item_fingerprints {
        hasher.update(fp.as_bytes());
        hasher.update(b"|");
    }
    hex::encode(hasher.finalize())
}

/// Check whether a file path is in a baseline-ignored directory (logs, crash-reports).
fn is_baseline_ignored(rel: &str) -> bool {
    // Use Path to check prefixes like "logs/", "crash-reports/".
    let p = Path::new(rel);
    for component in p.components() {
        if let std::path::Component::Normal(name) = component {
            let name = name.to_string_lossy();
            if name == "logs" || name == "crash-reports" {
                return true;
            }
        }
    }
    false
}

/// Compare an inventory against the stored baseline and return true if the
/// meaningful (non-ignored, non-Agora-internal) files match.
fn is_baseline_unchanged(
    current: &[InstanceImportFileRecord],
    baseline: &[InstanceImportFileRecord],
) -> bool {
    let meaningful_current: Vec<_> = current
        .iter()
        .filter(|f| !is_baseline_ignored(&f.relative_path))
        .collect();
    let meaningful_baseline: Vec<_> = baseline
        .iter()
        .filter(|f| !is_baseline_ignored(&f.relative_path))
        .collect();

    if meaningful_current.len() != meaningful_baseline.len() {
        return false;
    }

    // Both are sorted by relative_path.
    for (a, b) in meaningful_current.iter().zip(meaningful_baseline.iter()) {
        if a.relative_path != b.relative_path || a.sha256 != b.sha256 {
            return false;
        }
    }
    true
}

fn inventories_equal(
    left: &[InstanceImportFileRecord],
    right: &[InstanceImportFileRecord],
) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.relative_path == right.relative_path
                && left.sha256 == right.sha256
                && left.size == right.size
        })
}

fn imported_metadata_unchanged(conn: &rusqlite::Connection, import: &InstanceImportRecord) -> bool {
    let Some(row) = crate::db::get_instance(conn, &import.instance_id)
        .ok()
        .flatten()
    else {
        return false;
    };
    let expected_tuple: Option<LoaderTuple> = serde_json::from_str(&import.source_metadata_json)
        .ok()
        .flatten();
    if let Some(tuple) = expected_tuple {
        if row.minecraft_version != tuple.minecraft_version
            || row.loader != tuple.loader
            || row.loader_version != tuple.loader_version
        {
            return false;
        }
    }
    let expected_settings: LaunchSettingsPreview =
        serde_json::from_str(&import.imported_settings_json).unwrap_or(LaunchSettingsPreview {
            memory_mb: None,
            java_path: None,
            jvm_args: Vec::new(),
        });
    row.jvm_memory_mb == expected_settings.memory_mb.unwrap_or(4096)
        && row.java_path == expected_settings.java_path
        && row.jvm_custom_args == expected_settings.jvm_args.join(" ")
}

fn source_metadata_matches_baseline(
    tuple: Option<&LoaderTuple>,
    settings: &LaunchSettingsPreview,
    preserve_settings: bool,
    import: &InstanceImportRecord,
) -> bool {
    let baseline_tuple: Option<LoaderTuple> = serde_json::from_str(&import.source_metadata_json)
        .ok()
        .flatten();
    if tuple != baseline_tuple.as_ref() {
        return false;
    }
    if !preserve_settings {
        return true;
    }
    let baseline_settings: LaunchSettingsPreview =
        serde_json::from_str(&import.imported_settings_json).unwrap_or(LaunchSettingsPreview {
            memory_mb: None,
            java_path: None,
            jvm_args: Vec::new(),
        });
    settings.memory_mb.unwrap_or(4096) == baseline_settings.memory_mb.unwrap_or(4096)
        && settings.java_path == baseline_settings.java_path
        && settings.jvm_args == baseline_settings.jvm_args
}

/// Sanitize launch settings from a candidate into the subset we preserve.
fn sanitize_settings(candidate: &ImportCandidate, preserve: bool) -> LaunchSettingsPreview {
    if !preserve {
        return LaunchSettingsPreview {
            memory_mb: None,
            java_path: None,
            jvm_args: Vec::new(),
        };
    }

    // Keep memory, filter java_path (reject launcher-owned paths), keep safe JVM args.
    let java_path = candidate
        .settings_preview
        .java_path
        .as_ref()
        .filter(|jp| {
            // Reject paths that look launcher-owned (within the launcher's directories).
            !jp.contains("PrismLauncher")
                && !jp.contains("CurseForge")
                && !jp.contains("ModrinthApp")
                && !jp.contains("com.modrinth.theseus")
                && !jp.contains("com.modrinth.app")
        })
        .cloned();

    LaunchSettingsPreview {
        memory_mb: candidate
            .settings_preview
            .memory_mb
            .map(|memory| memory.clamp(2048, 32768)),
        java_path,
        jvm_args: candidate
            .settings_preview
            .jvm_args
            .iter()
            .filter(|argument| safe_import_jvm_argument(argument))
            .cloned()
            .collect(),
    }
}

fn safe_import_jvm_argument(argument: &str) -> bool {
    let lower = argument.to_ascii_lowercase();
    !argument.starts_with("-Xmx")
        && !argument.starts_with("-Xms")
        && !argument.starts_with("-cp")
        && !argument.starts_with("-classpath")
        && !argument.starts_with("-agentpath")
        && !argument.starts_with("-agentlib")
        && !argument.starts_with("-javaagent")
        && !argument.starts_with("-Djava.library.path")
        && !lower.contains("token")
        && !lower.contains("password")
        && !lower.contains("credential")
}

fn rediscover_selection(selection: &ImportSelection) -> Option<ImportCandidate> {
    let (_, root) = selection.installation_key.split_once(':')?;
    let root = Path::new(root);
    let discovery = match selection.launcher_kind {
        LauncherKind::Prism => launcher_import::discover_prism_launcher(Some(root)),
        LauncherKind::CurseForge => launcher_import::discover_curseforge_launcher(Some(root)),
        LauncherKind::Modrinth => launcher_import::discover_modrinth_launcher(Some(root)),
    };
    discovery
        .candidates
        .into_iter()
        .find(|candidate| candidate.source_key == selection.source_key)
}

// ---------------------------------------------------------------------------
// LauncherImportService
// ---------------------------------------------------------------------------

/// Core service for launcher-instance discovery, planning, and execution.
#[derive(Clone)]
pub struct LauncherImportService {
    ctx: Ctx,
}

struct ImportJobGuard {
    staging: PathBuf,
    db_path: PathBuf,
    job_id: String,
    armed: bool,
}

impl ImportJobGuard {
    fn new(staging: PathBuf, db_path: PathBuf, job_id: String) -> Self {
        Self {
            staging,
            db_path,
            job_id,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ImportJobGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let _ = std::fs::remove_dir_all(&self.staging);
        if let Ok(connection) = crate::db::local_state_connection(&self.db_path) {
            let _ = crate::db::delete_instance_import_job(&connection, &self.job_id);
        }
    }
}

impl LauncherImportService {
    /// Create a new service from the canonical context.
    pub fn new(ctx: Ctx) -> Self {
        Self { ctx }
    }

    /// Discover instances from all three source launchers.
    ///
    /// If `custom_root` is provided, it is passed to each adapter as a manual
    /// root hint in addition to any platform-default paths.
    pub fn discover(&self, custom_root: Option<PathBuf>) -> LauncherImportDiscovery {
        let custom = custom_root.as_deref();
        LauncherImportDiscovery {
            prism: launcher_import::discover_prism_launcher(custom),
            curseforge: launcher_import::discover_curseforge_launcher(custom),
            modrinth: launcher_import::discover_modrinth_launcher(custom),
        }
    }

    /// Build a fingerprinted immutable import plan from user selections.
    ///
    /// This step is read-only: it re-reads source metadata, resolves
    /// collisions, checks provenance, and validates loader support. It does
    /// not copy files or modify any state.
    pub fn plan(&self, selections: Vec<ImportSelection>) -> LauncherResult<LauncherImportPlan> {
        if selections.is_empty() {
            return Ok(LauncherImportPlan {
                batch_fingerprint: String::new(),
                items: Vec::new(),
                peak_bytes: 0,
                total_files: 0,
                batch_blockers: Vec::new(),
            });
        }

        // Re-discover candidates to re-validate source metadata.
        let discovery = self.discover(None);
        let all_candidates: Vec<&ImportCandidate> = discovery.all_candidates();

        let mut items: Vec<PerItemPlan> = Vec::new();
        let mut used_ids: Vec<String> = Vec::new();
        let mut peak_bytes: u64 = 0;
        let mut total_files: u64 = 0;

        // Open a single DB connection for provenance checks.
        let conn = crate::db::local_state_connection(&self.ctx.paths.local_state_db()).ok();

        for sel in &selections {
            let rediscovered_custom = rediscover_selection(sel);
            let candidate = all_candidates
                .iter()
                .find(|c| {
                    c.launcher == sel.launcher_kind
                        && c.source_key == sel.source_key
                        && c.launcher_installation_key == sel.installation_key
                })
                .copied()
                .or(rediscovered_custom.as_ref());

            let candidate = match candidate {
                Some(c) => c,
                None => {
                    items.push(PerItemPlan {
                        fingerprint: String::new(),
                        destination_id: String::new(),
                        destination_name: sel.destination_name.clone().unwrap_or_default(),
                        action: ItemAction::New,
                        source_key: sel.source_key.clone(),
                        launcher_kind: sel.launcher_kind,
                        installation_key: sel.installation_key.clone(),
                        source_path: String::new(),
                        loader_tuple: None,
                        total_bytes: 0,
                        total_files: 0,
                        preserve_settings: sel.preserve_settings,
                        sanitized_settings: LaunchSettingsPreview {
                            memory_mb: None,
                            java_path: None,
                            jvm_args: Vec::new(),
                        },
                        existing_import: None,
                        blockers: vec!["Candidate not found in current discovery".to_string()],
                        warnings: Vec::new(),
                    });
                    continue;
                }
            };

            let mut blockers: Vec<String> = Vec::new();
            let mut warnings: Vec<String> = Vec::new();

            // Reject unsupported candidates.
            if let CandidateStatus::Unsupported { reasons } = &candidate.status {
                blockers.extend(reasons.clone());
            }

            // If this is NeedsReview, add a warning but allow through if no blockers.
            if matches!(candidate.status, CandidateStatus::NeedsReview) {
                warnings.extend(candidate.warnings.clone());
                // If no loader tuple, add a blocker.
                if candidate.loader_tuple.is_none() {
                    blockers.push("No loader tuple could be determined".to_string());
                }
            }

            // Validate loader support.
            let (resolved_tuple, loader_blockers, loader_warnings) =
                if let Some(ref tuple) = candidate.loader_tuple {
                    check_loader_supported(tuple)
                } else {
                    (
                        None,
                        vec!["No loader tuple available".to_string()],
                        Vec::new(),
                    )
                };
            blockers.extend(loader_blockers);
            warnings.extend(loader_warnings);

            let requested_name = sel
                .destination_name
                .clone()
                .unwrap_or_else(|| candidate.display_name.clone());
            let sanitized = sanitize_settings(candidate, sel.preserve_settings);

            // Check existing instances in DB for collision.
            let existing_ids: Vec<String> = conn
                .as_ref()
                .and_then(|c| crate::db::list_instances(c).ok())
                .map(|rows| rows.into_iter().map(|r| r.instance_id).collect())
                .unwrap_or_default();

            // Also account for IDs already claimed in this plan batch.
            let all_used: Vec<String> = existing_ids
                .iter()
                .cloned()
                .chain(used_ids.iter().cloned())
                .collect();

            // Check provenance: has this source been imported before?
            let existing_import = conn.as_ref().and_then(|c| {
                find_instance_import_by_source(
                    c,
                    serde_json::to_string(&sel.launcher_kind)
                        .unwrap_or_default()
                        .trim_matches('"'),
                    &sel.installation_key,
                    &sel.source_key,
                )
                .ok()
                .flatten()
            });

            // Determine action and destination. Updates always target the
            // previously imported instance rather than a collision suffix.
            let (action, dest_id, dest_name, planned_existing_import) = if let Some(ref import) =
                existing_import
            {
                // Check if the current Agora destination is unchanged.
                let baseline = conn
                    .as_ref()
                    .and_then(|c| {
                        crate::db::list_instance_import_files(c, &import.instance_id).ok()
                    })
                    .unwrap_or_default();

                let current_inv = self
                    .ctx
                    .paths
                    .instance_dir(&import.instance_id)
                    .ok()
                    .and_then(|path| hash_payload(&path).ok())
                    .unwrap_or_default();

                let metadata_unchanged = conn
                    .as_ref()
                    .is_some_and(|connection| imported_metadata_unchanged(connection, import));
                if !baseline.is_empty()
                    && is_baseline_unchanged(&current_inv, &baseline)
                    && metadata_unchanged
                {
                    let existing_name = conn
                        .as_ref()
                        .and_then(|connection| {
                            crate::db::get_instance(connection, &import.instance_id)
                                .ok()
                                .flatten()
                        })
                        .map(|row| row.name)
                        .unwrap_or_else(|| requested_name.clone());
                    let source_inventory =
                        hash_payload(&candidate.payload_root).unwrap_or_default();
                    let action = if is_baseline_unchanged(&source_inventory, &baseline)
                        && source_metadata_matches_baseline(
                            candidate.loader_tuple.as_ref(),
                            &sanitized,
                            sel.preserve_settings,
                            import,
                        ) {
                        ItemAction::Unchanged
                    } else {
                        ItemAction::Update
                    };
                    (
                        action,
                        import.instance_id.clone(),
                        existing_name,
                        Some(import.clone()),
                    )
                } else {
                    warnings.push(
                        "Agora copy was modified; offering a new copy instead of update"
                            .to_string(),
                    );
                    let id = resolve_destination_id(candidate, Some(&requested_name), &all_used);
                    (ItemAction::New, id, requested_name.clone(), None)
                }
            } else {
                let id = resolve_destination_id(candidate, Some(&requested_name), &all_used);
                (ItemAction::New, id, requested_name.clone(), None)
            };

            if !used_ids.contains(&dest_id) {
                used_ids.push(dest_id.clone());
            }

            // Build source path string for provenance.
            let source_path = candidate.payload_root.to_string_lossy().to_string();

            let per_item_fingerprint = plan_fingerprint(
                &sel.source_key,
                sel.launcher_kind,
                &dest_id,
                &action,
                sel.preserve_settings,
            );

            let total_bytes = candidate.inventory.total_bytes;
            let total_file_count = candidate.inventory.total_files;
            peak_bytes = peak_bytes.saturating_add(total_bytes);
            total_files = total_files.saturating_add(total_file_count);

            items.push(PerItemPlan {
                fingerprint: per_item_fingerprint,
                destination_id: dest_id,
                destination_name: dest_name,
                action,
                source_key: sel.source_key.clone(),
                launcher_kind: sel.launcher_kind,
                installation_key: sel.installation_key.clone(),
                source_path,
                loader_tuple: resolved_tuple,
                total_bytes,
                total_files: total_file_count,
                preserve_settings: sel.preserve_settings,
                sanitized_settings: sanitized,
                existing_import: planned_existing_import,
                blockers,
                warnings,
            });
        }

        let item_fingerprints: Vec<String> = items.iter().map(|i| i.fingerprint.clone()).collect();
        let bfp = batch_fingerprint(&item_fingerprints);

        let mut batch_blockers = Vec::new();
        if crate::helpers::check_disk_space_for(&self.ctx.paths.staging_root(), peak_bytes).is_err()
        {
            batch_blockers.push(format!(
                "Not enough free disk space for {} bytes of instance data plus safety headroom",
                peak_bytes
            ));
        }

        Ok(LauncherImportPlan {
            batch_fingerprint: bfp,
            items,
            peak_bytes,
            total_files,
            batch_blockers,
        })
    }

    /// Execute a previously-built import plan.
    ///
    /// Each item is handled independently. Successes are kept if a sibling
    /// fails. The plan's source paths are re-validated before any mutation.
    pub async fn execute(
        &self,
        plan: LauncherImportPlan,
    ) -> LauncherResult<LauncherImportBatchResult> {
        if !plan.batch_blockers.is_empty() {
            return Err(LauncherError::DiskFull);
        }
        crate::helpers::check_disk_space_for(&self.ctx.paths.staging_root(), plan.peak_bytes)?;
        // Validate network policy once for the batch.
        let policy = NetworkPolicy::from_ctx(&self.ctx)?;
        let operation = self
            .ctx
            .operation_manager
            .register("Import launcher instances");
        let operation_id = operation.id().clone();
        let cancellation = operation.token().clone();

        let mut outcomes: Vec<PerItemOutcome> = Vec::new();

        for (index, item) in plan.items.iter().enumerate() {
            if cancellation.is_cancelled() {
                outcomes.extend(
                    plan.items[index..]
                        .iter()
                        .map(|_| PerItemOutcome::Cancelled {
                            reason: "Import cancelled".into(),
                        }),
                );
                break;
            }
            // Skip items with blockers.
            if !item.blockers.is_empty() {
                outcomes.push(PerItemOutcome::Skipped {
                    reason: item.blockers.join("; "),
                });
                continue;
            }
            if matches!(item.action, ItemAction::Unchanged) {
                outcomes.push(PerItemOutcome::Skipped {
                    reason: "Already imported and unchanged".into(),
                });
                continue;
            }

            self.ctx.progress_sink.report(
                ProgressEvent::new(
                    operation_id.clone(),
                    ProgressPhase::Staging,
                    format!("Importing {}", item.destination_name),
                )
                .with_progress(index as f64 / plan.items.len().max(1) as f64)
                .with_sub_label(item.destination_name.clone()),
            );
            let result = self
                .execute_item(item, &policy, &cancellation, &operation_id)
                .await;

            match result {
                Ok(outcome) => outcomes.push(outcome),
                Err(_error) if cancellation.is_cancelled() => {
                    outcomes.push(PerItemOutcome::Cancelled {
                        reason: "Import cancelled".into(),
                    });
                }
                Err(e) => outcomes.push(PerItemOutcome::Failed {
                    error: e.to_string(),
                    warnings: item.warnings.clone(),
                }),
            }
        }

        self.ctx.progress_sink.report(
            ProgressEvent::new(
                operation_id,
                ProgressPhase::Done,
                "Launcher import complete",
            )
            .with_progress(1.0),
        );
        operation.complete();
        Ok(LauncherImportBatchResult { outcomes })
    }

    /// Execute a single planned item.
    async fn execute_item(
        &self,
        item: &PerItemPlan,
        policy: &NetworkPolicy,
        cancellation: &CancellationToken,
        operation_id: &crate::event_sink::OperationId,
    ) -> LauncherResult<PerItemOutcome> {
        check_cancelled(cancellation)?;
        let source_path = PathBuf::from(&item.source_path);
        let source_path = source_path
            .canonicalize()
            .map_err(|error| LauncherError::Generic {
                code: "ERR_IMPORT_SOURCE_MISSING".into(),
                message: format!("Cannot resolve source directory: {error}"),
            })?;
        if !source_path.is_dir() {
            return Err(LauncherError::Generic {
                code: "ERR_IMPORT_SOURCE_MISSING".into(),
                message: format!(
                    "Source directory no longer exists: {}",
                    source_path.display()
                ),
            });
        }
        let app_root = self
            .ctx
            .paths
            .root()
            .canonicalize()
            .unwrap_or_else(|_| self.ctx.paths.root().to_path_buf());
        if source_path.starts_with(&app_root) {
            return Err(LauncherError::Generic {
                code: "ERR_IMPORT_RECURSIVE_SOURCE".into(),
                message: "Agora's own data directory cannot be used as an import source.".into(),
            });
        }

        let dest_id = &item.destination_id;
        let dest_name = &item.destination_name;
        let previous_row = if matches!(item.action, ItemAction::Update) {
            let connection = crate::db::local_state_connection(&self.ctx.paths.local_state_db())
                .map_err(|error| LauncherError::Generic {
                    code: "ERR_LOCAL_STATE_FAILED".into(),
                    message: error.to_string(),
                })?;
            crate::db::get_instance(&connection, dest_id).map_err(|error| {
                LauncherError::Generic {
                    code: "ERR_LOCAL_STATE_FAILED".into(),
                    message: error.to_string(),
                }
            })?
        } else {
            None
        };

        // ── Acquire destination lock ─────────────────────────────────────
        let lock = self
            .ctx
            .lock_manager
            .acquire(
                crate::lock_manager::LockResource::Instance(dest_id.clone()),
                "launcher-import-execute",
            )
            .map_err(|e| LauncherError::Generic {
                code: "ERR_LOCK_ACQUIRE".into(),
                message: format!("Cannot lock instance '{}': {e}", dest_id),
            })?;

        // ── Create import job ────────────────────────────────────────────
        let job_id = format!("import-job-{}", Uuid::new_v4());
        let staging_path = self
            .ctx
            .paths
            .staging_root()
            .join(format!(".agora-import-{dest_id}-{}", Uuid::new_v4()));
        let final_path = self.ctx.paths.instances_root().join(dest_id);
        let quarantine_path = matches!(item.action, ItemAction::Update).then(|| {
            self.ctx
                .paths
                .staging_root()
                .join(format!(".agora-quarantine-{dest_id}-{}", Uuid::new_v4()))
        });
        std::fs::create_dir_all(self.ctx.paths.instances_root()).map_err(|error| {
            LauncherError::Generic {
                code: "ERR_IMPORT_MKDIR".into(),
                message: format!("Cannot create instances directory: {error}"),
            }
        })?;

        // Register staging path just for job tracking.
        let staging = staging_path.clone();
        let final_dir = final_path.clone();

        let job = InstanceImportJob {
            job_id: job_id.clone(),
            instance_id: dest_id.clone(),
            launcher_kind: serde_json::to_string(&item.launcher_kind)
                .unwrap_or_default()
                .trim_matches('"')
                .to_string(),
            source_instance_key: item.source_key.clone(),
            staging_path: staging.to_string_lossy().to_string(),
            final_path: final_dir.to_string_lossy().to_string(),
            quarantine_path: quarantine_path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
            state: "staging".to_string(),
            plan_fingerprint: item.fingerprint.clone(),
            error: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        };

        // The journal is required: without it a crash between rename and DB
        // registration can leave an unowned live directory.
        let db_path = self.ctx.paths.local_state_db();
        {
            let connection = crate::db::local_state_connection(&db_path).map_err(|error| {
                LauncherError::Generic {
                    code: "ERR_LOCAL_STATE_FAILED".into(),
                    message: format!("Cannot open import journal: {error}"),
                }
            })?;
            crate::db::upsert_instance_import_job(&connection, &job).map_err(|error| {
                LauncherError::Generic {
                    code: "ERR_IMPORT_JOURNAL".into(),
                    message: format!("Cannot persist import journal: {error}"),
                }
            })?;
        }
        let mut job_guard = ImportJobGuard::new(staging.clone(), db_path.clone(), job_id.clone());

        // ── Create staging directory ─────────────────────────────────────
        std::fs::create_dir_all(&staging).map_err(|e| LauncherError::Generic {
            code: "ERR_IMPORT_MKDIR".into(),
            message: format!("Cannot create staging directory: {e}"),
        })?;

        let rollback_staging = || -> LauncherResult<()> {
            let _ = std::fs::remove_dir_all(&staging);
            if let Ok(connection) = crate::db::local_state_connection(&db_path) {
                let _ = db::delete_instance_import_job(&connection, &job_id);
            }
            Ok(())
        };

        // ── Stage: copy payload with hashing ─────────────────────────────
        let mut copied_bytes = 0u64;
        let mut last_reported = 0u64;
        let mut report_bytes = |delta: u64| {
            copied_bytes = copied_bytes.saturating_add(delta);
            if copied_bytes.saturating_sub(last_reported) >= 4 * 1024 * 1024
                || copied_bytes >= item.total_bytes
            {
                last_reported = copied_bytes;
                self.ctx.progress_sink.report(
                    ProgressEvent::new(
                        operation_id.clone(),
                        ProgressPhase::Staging,
                        format!("Copying {}", item.destination_name),
                    )
                    .with_sub_label(item.destination_name.clone())
                    .with_bytes(copied_bytes, item.total_bytes.max(1)),
                );
            }
        };
        let file_inventory = match copy_payload_control(
            &source_path,
            &staging,
            Some(cancellation),
            &mut report_bytes,
        ) {
            Ok(inv) => inv,
            Err(e) => {
                let _ = rollback_staging();
                return Err(LauncherError::Generic {
                    code: "ERR_IMPORT_COPY".into(),
                    message: e,
                });
            }
        };
        if let Err(error) = check_cancelled(cancellation) {
            let _ = rollback_staging();
            return Err(error);
        }

        // Re-hash the source after copying so additions, removals, and same-size
        // mutations cannot produce a torn instance.
        if ensure_source_stable(&source_path, &file_inventory).is_err() {
            let _ = rollback_staging();
            return Err(LauncherError::Generic {
                code: "ERR_IMPORT_SOURCE_CHANGED".into(),
                message: "Source files changed while Agora was copying them.".into(),
            });
        }

        // ── Build manifest arrays ────────────────────────────────────────
        let source_label = match item.launcher_kind {
            LauncherKind::Prism => "prism",
            LauncherKind::CurseForge => "curseforge",
            LauncherKind::Modrinth => "modrinth",
        };

        let manifest_loader = item
            .loader_tuple
            .as_ref()
            .map(|tuple| tuple.loader.as_str())
            .unwrap_or("vanilla");
        let (mods, resourcepacks, shaders, datapacks, worlds) =
            build_manifest_arrays(&staging, source_label, manifest_loader)?;

        let (loader_name, loader_version, mc_version) = item
            .loader_tuple
            .as_ref()
            .map(|t| {
                (
                    t.loader.clone(),
                    t.loader_version.clone(),
                    t.minecraft_version.clone(),
                )
            })
            .unwrap_or_else(|| (String::new(), String::new(), String::new()));

        let manifest = InstanceManifest {
            instance_id: dest_id.clone(),
            name: dest_name.clone(),
            minecraft_version: mc_version.clone(),
            loader: loader_name.clone(),
            loader_version: loader_version.clone(),
            is_locked: previous_row.as_ref().is_some_and(|row| row.is_locked),
            created_from_pack: None,
            mods,
            resourcepacks,
            shaders,
            datapacks,
            worlds,
            user_preferences: serde_json::json!({}),
        };

        let manifest_json =
            serde_json::to_string_pretty(&manifest).map_err(|e| LauncherError::Generic {
                code: "ERR_IMPORT_SERIALIZE".into(),
                message: format!("Cannot serialize manifest: {e}"),
            })?;
        std::fs::write(staging.join("instance_manifest.json"), &manifest_json).map_err(|e| {
            LauncherError::Generic {
                code: "ERR_IMPORT_WRITE".into(),
                message: format!("Cannot write manifest: {e}"),
            }
        })?;

        // Update job state.
        {
            let connection = crate::db::local_state_connection(&db_path).map_err(|error| {
                let _ = rollback_staging();
                LauncherError::Generic {
                    code: "ERR_IMPORT_JOURNAL".into(),
                    message: format!("Cannot reopen import journal: {error}"),
                }
            })?;
            let mut j = job.clone();
            j.state = match item.action {
                ItemAction::New => "prepared_new",
                ItemAction::Update => "prepared_update",
                ItemAction::Unchanged => "prepared_new",
            }
            .to_string();
            j.updated_at = chrono::Utc::now().to_rfc3339();
            db::upsert_instance_import_job(&connection, &j).map_err(|error| {
                LauncherError::Generic {
                    code: "ERR_IMPORT_JOURNAL".into(),
                    message: format!("Cannot update import journal: {error}"),
                }
            })?;
        }

        // ── Health scan ──────────────────────────────────────────────────
        let health_report =
            health::health(&staging, &manifest, Some(&self.ctx.paths.registry_db()));
        self.ctx.progress_sink.report(
            ProgressEvent::new(
                operation_id.clone(),
                ProgressPhase::HealthScan,
                format!("Checked {}", item.destination_name),
            )
            .with_sub_label(item.destination_name.clone()),
        );
        let mut health_warnings: Vec<String> = Vec::new();
        if health_report.score == HealthScore::Red {
            for b in &health_report.blockers {
                health_warnings.push(format!("Health blocker: {}", b.message));
            }
        }

        // ── Handle new vs update ─────────────────────────────────────────
        match item.action {
            ItemAction::New => {
                // ── Bootstrap Mojang metadata ────────────────────────────
                if !mc_version.is_empty() {
                    let minecraft_root = self.ctx.paths.minecraft_runtime_root();
                    if let Err(e) = crate::minecraft_metadata::ensure_base_version_metadata(
                        &minecraft_root,
                        &mc_version,
                        policy,
                    )
                    .await
                    {
                        let _ = rollback_staging();
                        return Err(e);
                    }
                }

                // ── Install loader ───────────────────────────────────────
                if !loader_name.is_empty() && loader_name != "vanilla" {
                    if let Err(e) = LoaderService::new(self.ctx.clone())
                        .ensure_installed(&loader_name, &mc_version, &loader_version, false)
                        .await
                    {
                        let _ = rollback_staging();
                        return Err(e);
                    }
                }
                if let Err(error) = check_cancelled(cancellation) {
                    let _ = rollback_staging();
                    return Err(error);
                }
                if let Err(error) = ensure_source_stable(&source_path, &file_inventory) {
                    let _ = rollback_staging();
                    return Err(error);
                }

                // ── Atomic rename ────────────────────────────────────────
                if let Err(error) = persist_import_job_state(&db_path, &job, "promoting_new") {
                    let _ = rollback_staging();
                    return Err(error);
                }
                if final_dir.exists() {
                    let _ = rollback_staging();
                    return Err(LauncherError::Generic {
                        code: "ERR_INSTANCE_EXISTS".into(),
                        message: format!("Instance '{dest_id}' already exists on disk"),
                    });
                }

                std::fs::rename(&staging, &final_dir).map_err(|e| LauncherError::Generic {
                    code: "ERR_IMPORT_FINALIZE".into(),
                    message: format!("Cannot finalize import: {e}"),
                })?;
                if let Err(error) = persist_import_job_state(&db_path, &job, "promoted_new") {
                    let _ = std::fs::rename(&final_dir, &staging);
                    let _ = rollback_staging();
                    return Err(error);
                }

                // ── DB registration ──────────────────────────────────────
                let row = InstanceRow {
                    instance_id: dest_id.clone(),
                    name: dest_name.clone(),
                    minecraft_version: mc_version.clone(),
                    loader: loader_name.clone(),
                    loader_version: loader_version.clone(),
                    is_modpack: false,
                    is_locked: false,
                    last_launched_at: None,
                    jvm_memory_mb: item.sanitized_settings.memory_mb.unwrap_or(4096),
                    jvm_memory_mode: if item.sanitized_settings.memory_mb.is_some() {
                        "manual".into()
                    } else {
                        "auto".into()
                    },
                    jvm_gc: "auto".into(),
                    jvm_custom_args: item.sanitized_settings.jvm_args.join(" "),
                    jvm_always_pre_touch: crate::models::recommended_java_version_for_minecraft(
                        &mc_version,
                    ) < 21,
                    created_at: chrono::Utc::now().to_rfc3339(),
                    java_path: item.sanitized_settings.java_path.clone(),
                    java_incompatible_override: false,
                    icon_path: None,
                    launch_mode_override: match item
                        .loader_tuple
                        .as_ref()
                        .map(|t| t.loader.as_str())
                    {
                        Some("vanilla") | None => "auto".to_string(),
                        _ => "auto".to_string(),
                    },
                    import_source: Some(source_label.to_string()),
                };

                let conn = match crate::db::local_state_connection(&self.ctx.paths.local_state_db())
                {
                    Ok(c) => c,
                    Err(e) => {
                        // Rollback: move final dir back to staging for cleanup.
                        let _ = std::fs::rename(&final_dir, &staging);
                        let _ = rollback_staging();
                        return Err(LauncherError::Generic {
                            code: "ERR_LOCAL_STATE_FAILED".into(),
                            message: format!("Failed to open local state DB: {e}"),
                        });
                    }
                };

                if let Err(e) = crate::db::upsert_instance(&conn, &row) {
                    let _ = std::fs::rename(&final_dir, &staging);
                    let _ = rollback_staging();
                    return Err(LauncherError::Generic {
                        code: "ERR_LOCAL_STATE_FAILED".into(),
                        message: format!("Failed to register imported instance: {e}"),
                    });
                }

                // ── Persist import provenance ────────────────────────────
                let import_record = InstanceImportRecord {
                    instance_id: dest_id.clone(),
                    launcher_kind: serde_json::to_string(&item.launcher_kind)
                        .unwrap_or_default()
                        .trim_matches('"')
                        .to_string(),
                    installation_key: item.installation_key.clone(),
                    source_instance_key: item.source_key.clone(),
                    source_path: source_path.to_string_lossy().to_string(),
                    source_metadata_json: serde_json::to_string(&item.loader_tuple)
                        .unwrap_or_default(),
                    imported_settings_json: serde_json::to_string(&LaunchSettingsPreview {
                        memory_mb: Some(row.jvm_memory_mb),
                        java_path: row.java_path.clone(),
                        jvm_args: row
                            .jvm_custom_args
                            .split_whitespace()
                            .map(str::to_owned)
                            .collect(),
                    })
                    .unwrap_or_default(),
                    source_fingerprint: item.fingerprint.clone(),
                    imported_at: chrono::Utc::now().to_rfc3339(),
                    updated_at: chrono::Utc::now().to_rfc3339(),
                    last_result: "imported".to_string(),
                };

                if let Err(e) = replace_instance_import(&conn, &import_record, &file_inventory) {
                    // Non-fatal: instance was registered but provenance write failed.
                    let msg = format!("Import succeeded but provenance record failed: {e}");
                    let _ = rollback_staging();
                    // Clean up the instance to avoid unregistered orphan.
                    let _ = std::fs::remove_dir_all(&final_dir);
                    let _ = db::delete_instance(&conn, dest_id);
                    return Err(LauncherError::Generic {
                        code: "ERR_LOCAL_STATE_FAILED".into(),
                        message: msg,
                    });
                }

                // ── Clean up job ─────────────────────────────────────────
                let _ = db::delete_instance_import_job(&conn, &job_id);
                job_guard.disarm();

                drop(lock);
                // Release the lock before returning.

                // Run health check as warning emission (non-fatal).
                let final_health =
                    health::health(&final_dir, &manifest, Some(&self.ctx.paths.registry_db()));
                if final_health.score == HealthScore::Red {
                    let details: Vec<String> = final_health
                        .blockers
                        .iter()
                        .map(|b| b.message.clone())
                        .collect();
                    health_warnings.extend(details);
                }

                Ok(PerItemOutcome::Imported {
                    instance_id: dest_id.clone(),
                    warnings: health_warnings,
                })
            }

            ItemAction::Update => {
                let existing = match &item.existing_import {
                    Some(e) => e,
                    None => {
                        let _ = rollback_staging();
                        return Err(LauncherError::Generic {
                            code: "ERR_IMPORT_NO_EXISTING".into(),
                            message: "Update requested but no existing import record found"
                                .to_string(),
                        });
                    }
                };

                if final_dir.exists() {
                    crate::snapshot::create_snapshot(
                        &final_dir,
                        Some("Before launcher import update"),
                    )
                    .map_err(|error| {
                        let _ = rollback_staging();
                        LauncherError::Generic {
                            code: "ERR_SNAPSHOT_FAILED".into(),
                            message: format!("Cannot create pre-update recovery snapshot: {error}"),
                        }
                    })?;
                }

                // ── Bootstrap Mojang metadata (same as New) ──────────────
                if !mc_version.is_empty() {
                    let minecraft_root = self.ctx.paths.minecraft_runtime_root();
                    if let Err(e) = crate::minecraft_metadata::ensure_base_version_metadata(
                        &minecraft_root,
                        &mc_version,
                        policy,
                    )
                    .await
                    {
                        let _ = rollback_staging();
                        return Err(e);
                    }
                }

                // ── Install loader ───────────────────────────────────────
                if !loader_name.is_empty() && loader_name != "vanilla" {
                    if let Err(e) = LoaderService::new(self.ctx.clone())
                        .ensure_installed(&loader_name, &mc_version, &loader_version, false)
                        .await
                    {
                        let _ = rollback_staging();
                        return Err(e);
                    }
                }
                if let Err(error) = check_cancelled(cancellation) {
                    let _ = rollback_staging();
                    return Err(error);
                }

                // ── Preserve .agora_snapshots if target exists ───────────
                let target_agora_snapshots = final_dir.join(".agora_snapshots");
                let staging_agora_snapshots = staging.join(".agora_snapshots");
                if target_agora_snapshots.exists() {
                    let _ = std::fs::remove_dir_all(&staging_agora_snapshots);
                    if let Err(message) =
                        copy_dir_recursive(&target_agora_snapshots, &staging_agora_snapshots)
                    {
                        let _ = rollback_staging();
                        return Err(LauncherError::Generic {
                            code: "ERR_SNAPSHOT_FAILED".into(),
                            message,
                        });
                    }
                }
                if let Err(error) = ensure_source_stable(&source_path, &file_inventory) {
                    let _ = rollback_staging();
                    return Err(error);
                }

                // ── Atomic swap ──────────────────────────────────────────
                // Quarantine old target, rename staging to final, restore on failure.
                let quarantine =
                    quarantine_path
                        .as_ref()
                        .ok_or_else(|| LauncherError::Generic {
                            code: "ERR_IMPORT_JOURNAL".into(),
                            message: "Update quarantine path was not journaled.".into(),
                        })?;

                if let Err(error) = persist_import_job_state(&db_path, &job, "swapping_update") {
                    let _ = rollback_staging();
                    return Err(error);
                }
                if final_dir.exists() {
                    std::fs::rename(&final_dir, quarantine).map_err(|e| {
                        LauncherError::Generic {
                            code: "ERR_IMPORT_QUARANTINE".into(),
                            message: format!("Cannot quarantine old instance: {e}"),
                        }
                    })?;
                }

                if let Err(e) = std::fs::rename(&staging, &final_dir) {
                    // Restore from quarantine.
                    if final_dir.exists() {
                        let _ = std::fs::remove_dir_all(&final_dir);
                    }
                    if quarantine.exists() {
                        let _ = std::fs::rename(quarantine, &final_dir);
                    }
                    let _ = rollback_staging();
                    return Err(LauncherError::Generic {
                        code: "ERR_IMPORT_FINALIZE".into(),
                        message: format!("Cannot finalize update: {e}"),
                    });
                }
                if let Err(error) = persist_import_job_state(&db_path, &job, "promoted_update") {
                    let _ = std::fs::rename(&final_dir, &staging);
                    if quarantine.exists() {
                        let _ = std::fs::rename(quarantine, &final_dir);
                    }
                    let _ = rollback_staging();
                    return Err(error);
                }

                // ── Update DB ────────────────────────────────────────────
                let conn = match crate::db::local_state_connection(&self.ctx.paths.local_state_db())
                {
                    Ok(c) => c,
                    Err(e) => {
                        // Restore from quarantine.
                        let _ = std::fs::rename(&final_dir, &staging);
                        if quarantine.exists() {
                            let _ = std::fs::rename(quarantine, &final_dir);
                        }
                        let _ = rollback_staging();
                        return Err(LauncherError::Generic {
                            code: "ERR_LOCAL_STATE_FAILED".into(),
                            message: format!("Failed to open local state DB: {e}"),
                        });
                    }
                };

                let row = InstanceRow {
                    instance_id: dest_id.clone(),
                    name: dest_name.clone(),
                    minecraft_version: mc_version.clone(),
                    loader: loader_name.clone(),
                    loader_version: loader_version.clone(),
                    is_modpack: previous_row.as_ref().is_some_and(|row| row.is_modpack),
                    is_locked: previous_row.as_ref().is_some_and(|row| row.is_locked),
                    last_launched_at: previous_row
                        .as_ref()
                        .and_then(|row| row.last_launched_at.clone()),
                    jvm_memory_mb: if item.preserve_settings {
                        item.sanitized_settings.memory_mb.unwrap_or(4096)
                    } else {
                        previous_row
                            .as_ref()
                            .map(|row| row.jvm_memory_mb)
                            .unwrap_or(4096)
                    },
                    jvm_memory_mode: if item.preserve_settings
                        && item.sanitized_settings.memory_mb.is_some()
                    {
                        "manual".into()
                    } else {
                        previous_row
                            .as_ref()
                            .map(|row| row.jvm_memory_mode.clone())
                            .unwrap_or_else(|| "auto".into())
                    },
                    jvm_gc: previous_row
                        .as_ref()
                        .map(|row| row.jvm_gc.clone())
                        .unwrap_or_else(|| "auto".into()),
                    jvm_custom_args: if item.preserve_settings {
                        item.sanitized_settings.jvm_args.join(" ")
                    } else {
                        previous_row
                            .as_ref()
                            .map(|row| row.jvm_custom_args.clone())
                            .unwrap_or_default()
                    },
                    jvm_always_pre_touch: previous_row
                        .as_ref()
                        .map(|row| row.jvm_always_pre_touch)
                        .unwrap_or_else(|| {
                            crate::models::recommended_java_version_for_minecraft(&mc_version) < 21
                        }),
                    created_at: previous_row
                        .as_ref()
                        .map(|row| row.created_at.clone())
                        .unwrap_or_else(|| existing.imported_at.clone()),
                    java_path: if item.preserve_settings {
                        item.sanitized_settings.java_path.clone()
                    } else {
                        previous_row.as_ref().and_then(|row| row.java_path.clone())
                    },
                    java_incompatible_override: previous_row
                        .as_ref()
                        .is_some_and(|row| row.java_incompatible_override),
                    icon_path: previous_row.as_ref().and_then(|row| row.icon_path.clone()),
                    launch_mode_override: previous_row
                        .as_ref()
                        .map(|row| row.launch_mode_override.clone())
                        .unwrap_or_else(|| "auto".into()),
                    import_source: Some(source_label.to_string()),
                };

                if let Err(e) = crate::db::upsert_instance(&conn, &row) {
                    // Restore from quarantine, then remove failed staging.
                    let _ = std::fs::rename(&final_dir, &staging);
                    if quarantine.exists() {
                        let _ = std::fs::rename(quarantine, &final_dir);
                    }
                    let _ = rollback_staging();
                    return Err(LauncherError::Generic {
                        code: "ERR_LOCAL_STATE_FAILED".into(),
                        message: format!("Failed to update instance: {e}"),
                    });
                }

                // ── Update provenance ────────────────────────────────────
                let import_record = InstanceImportRecord {
                    instance_id: dest_id.clone(),
                    launcher_kind: existing.launcher_kind.clone(),
                    installation_key: item.installation_key.clone(),
                    source_instance_key: item.source_key.clone(),
                    source_path: source_path.to_string_lossy().to_string(),
                    source_metadata_json: serde_json::to_string(&item.loader_tuple)
                        .unwrap_or_default(),
                    imported_settings_json: serde_json::to_string(&LaunchSettingsPreview {
                        memory_mb: Some(row.jvm_memory_mb),
                        java_path: row.java_path.clone(),
                        jvm_args: row
                            .jvm_custom_args
                            .split_whitespace()
                            .map(str::to_owned)
                            .collect(),
                    })
                    .unwrap_or_default(),
                    source_fingerprint: item.fingerprint.clone(),
                    imported_at: existing.imported_at.clone(),
                    updated_at: chrono::Utc::now().to_rfc3339(),
                    last_result: "updated".to_string(),
                };

                if let Err(e) = replace_instance_import(&conn, &import_record, &file_inventory) {
                    if let Some(previous_row) = &previous_row {
                        let _ = crate::db::upsert_instance(&conn, previous_row);
                    }
                    let _ = std::fs::rename(&final_dir, &staging);
                    if quarantine.exists() {
                        let _ = std::fs::rename(quarantine, &final_dir);
                    }
                    let _ = rollback_staging();
                    return Err(LauncherError::Generic {
                        code: "ERR_LOCAL_STATE_FAILED".into(),
                        message: format!("Failed to commit import provenance: {e}"),
                    });
                }

                // ── Clean up quarantine ──────────────────────────────────
                let _ = std::fs::remove_dir_all(quarantine);
                // ── Clean up job ─────────────────────────────────────────
                let _ = db::delete_instance_import_job(&conn, &job_id);
                job_guard.disarm();

                drop(lock);

                // Non-fatal health check.
                let final_health =
                    health::health(&final_dir, &manifest, Some(&self.ctx.paths.registry_db()));
                if final_health.score == HealthScore::Red {
                    let details: Vec<String> = final_health
                        .blockers
                        .iter()
                        .map(|b| b.message.clone())
                        .collect();
                    health_warnings.extend(details);
                }

                Ok(PerItemOutcome::Updated {
                    instance_id: dest_id.clone(),
                    warnings: health_warnings,
                })
            }
            ItemAction::Unchanged => unreachable!("unchanged imports are skipped before execution"),
        }
    }
}

fn check_cancelled(cancellation: &CancellationToken) -> LauncherResult<()> {
    if cancellation.is_cancelled() {
        Err(LauncherError::Generic {
            code: "ERR_OPERATION_CANCELLED".into(),
            message: "Launcher import was cancelled.".into(),
        })
    } else {
        Ok(())
    }
}

fn persist_import_job_state(
    db_path: &Path,
    job: &InstanceImportJob,
    state: &str,
) -> LauncherResult<()> {
    let connection =
        crate::db::local_state_connection(db_path).map_err(|error| LauncherError::Generic {
            code: "ERR_IMPORT_JOURNAL".into(),
            message: format!("Cannot open import journal: {error}"),
        })?;
    let mut updated = job.clone();
    updated.state = state.to_string();
    updated.updated_at = chrono::Utc::now().to_rfc3339();
    crate::db::upsert_instance_import_job(&connection, &updated).map_err(|error| {
        LauncherError::Generic {
            code: "ERR_IMPORT_JOURNAL".into(),
            message: format!("Cannot update import journal: {error}"),
        }
    })
}

/// Recover interrupted imports before normal app operations begin.
pub fn recover_interrupted_jobs(paths: &crate::app_paths::AppPaths) -> Vec<String> {
    let mut warnings = Vec::new();
    let Ok(connection) = crate::db::local_state_connection(&paths.local_state_db()) else {
        return warnings;
    };
    let Ok(jobs) = crate::db::list_instance_import_jobs(&connection) else {
        return warnings;
    };
    for job in jobs {
        let staging = PathBuf::from(&job.staging_path);
        let final_dir = PathBuf::from(&job.final_path);
        if !staging.starts_with(paths.staging_root())
            || !final_dir.starts_with(paths.instances_root())
        {
            warnings.push(format!(
                "Ignored unsafe interrupted import paths for {}",
                job.instance_id
            ));
            let _ = crate::db::delete_instance_import_job(&connection, &job.job_id);
            continue;
        }

        let committed = crate::db::get_instance_import(&connection, &job.instance_id)
            .ok()
            .flatten()
            .is_some_and(|record| record.source_fingerprint == job.plan_fingerprint)
            && crate::db::get_instance(&connection, &job.instance_id)
                .ok()
                .flatten()
                .is_some();

        let quarantine = job
            .quarantine_path
            .as_ref()
            .map(PathBuf::from)
            .filter(|path| path.starts_with(paths.staging_root()) && path.exists())
            .or_else(|| find_import_quarantine(paths, &job.instance_id));
        if committed {
            let _ = std::fs::remove_dir_all(&staging);
            if let Some(quarantine) = quarantine {
                let _ = std::fs::remove_dir_all(quarantine);
            }
            warnings.push(format!(
                "Finished cleanup for imported instance {} after an interrupted shutdown",
                job.instance_id
            ));
        } else if job.state.contains("update") {
            if let Some(quarantine) = quarantine {
                if final_dir.exists() {
                    let _ = std::fs::remove_dir_all(&final_dir);
                }
                if std::fs::rename(&quarantine, &final_dir).is_err() {
                    warnings.push(format!(
                        "Could not restore interrupted update for {} from {}",
                        job.instance_id,
                        quarantine.display()
                    ));
                    continue;
                }
            }
            let _ = std::fs::remove_dir_all(&staging);
            warnings.push(format!(
                "Rolled back interrupted update for {}",
                job.instance_id
            ));
        } else {
            let _ = std::fs::remove_dir_all(&staging);
            if final_dir.exists()
                && crate::db::get_instance(&connection, &job.instance_id)
                    .ok()
                    .flatten()
                    .is_none()
            {
                let _ = std::fs::remove_dir_all(&final_dir);
            }
            warnings.push(format!(
                "Rolled back interrupted import for {}",
                job.instance_id
            ));
        }
        let _ = crate::db::delete_instance_import_job(&connection, &job.job_id);
    }
    warnings
}

fn find_import_quarantine(
    paths: &crate::app_paths::AppPaths,
    instance_id: &str,
) -> Option<PathBuf> {
    let prefix = format!(".agora-quarantine-{instance_id}-");
    std::fs::read_dir(paths.staging_root())
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(&prefix))
        })
}

/// Verify that source files are still present and have not changed in size
/// since the copy was made (basic stability check).
fn verify_source_stable(
    source_root: &Path,
    inventory: &[InstanceImportFileRecord],
) -> Result<(), String> {
    for file in inventory {
        let src_path = source_root.join(&file.relative_path);
        let meta = match std::fs::metadata(&src_path) {
            Ok(m) => m,
            Err(_) => {
                return Err(format!(
                    "Source file vanished during copy: {}",
                    file.relative_path
                ))
            }
        };
        if meta.len() != file.size {
            return Err(format!(
                "Source file size changed during copy: {} (was {}, now {})",
                file.relative_path,
                file.size,
                meta.len()
            ));
        }
    }
    Ok(())
}

fn ensure_source_stable(
    source_root: &Path,
    copied_inventory: &[InstanceImportFileRecord],
) -> LauncherResult<()> {
    let source_inventory = hash_payload(source_root).map_err(|message| LauncherError::Generic {
        code: "ERR_IMPORT_SOURCE_CHANGED".into(),
        message,
    })?;
    if !inventories_equal(&source_inventory, copied_inventory)
        || verify_source_stable(source_root, copied_inventory).is_err()
    {
        return Err(LauncherError::Generic {
            code: "ERR_IMPORT_SOURCE_CHANGED".into(),
            message: "Source files changed while Agora was preparing the import.".into(),
        });
    }
    Ok(())
}

/// Recursively copy a directory (simple non-hashing copy, used for
/// `.agora_snapshots` preservation during updates).
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    if !src.is_dir() {
        return Err(format!("Not a directory: {}", src.display()));
    }
    std::fs::create_dir_all(dst).map_err(|e| format!("Cannot create {dst:?}: {e}"))?;
    for entry in
        std::fs::read_dir(src).map_err(|e| format!("Cannot read {}: {e}", src.display()))?
    {
        let entry = entry.map_err(|e| format!("Entry error: {e}"))?;
        let ft = entry.file_type().map_err(|e| format!("FT error: {e}"))?;
        let fname = entry.file_name();
        let src_child = entry.path();
        let dst_child = dst.join(&fname);

        if unsafe_filesystem_entry(&entry.path(), &ft) {
            continue;
        }
        if ft.is_dir() {
            copy_dir_recursive(&src_child, &dst_child)?;
        } else if ft.is_file() {
            std::fs::copy(&src_child, &dst_child)
                .map_err(|e| format!("Cannot copy {}: {e}", src_child.display()))?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Helper: LauncherKind serialization for DB storage
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctx::CoreContext;
    use crate::launcher_import::{ContentInventory, LaunchStrategy};
    use std::sync::atomic::{AtomicU64, Ordering};

    fn test_tmp(label: &str) -> PathBuf {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "agora-launcher-import-{label}-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn test_ctx() -> Ctx {
        let tmp = test_tmp("ctx");
        let _ = std::fs::create_dir_all(&tmp);
        let ctx = CoreContext::for_testing(tmp);
        let _ = crate::db::init_local_state_db(&ctx.paths.local_state_db());
        ctx
    }

    // --------------------------------------------------------------
    // copy_payload tests
    // --------------------------------------------------------------

    #[test]
    fn test_copy_payload_basic() {
        let src = test_tmp("copy-src");
        let dst = test_tmp("copy-dst");
        std::fs::create_dir_all(src.join("mods")).unwrap();
        std::fs::write(src.join("mods").join("test.jar"), b"hello").unwrap();
        std::fs::create_dir_all(src.join("config")).unwrap();
        std::fs::write(src.join("config").join("options.txt"), b"opt").unwrap();

        let _inv = copy_payload(&src, &dst).unwrap();
        assert_eq!(_inv.len(), 2);

        assert!(dst.join("mods").join("test.jar").exists());
        assert!(dst.join("config").join("options.txt").exists());
    }

    #[test]
    fn test_copy_payload_excludes_control_files() {
        let src = test_tmp("copy-excl");
        let dst = test_tmp("copy-excl-dst");
        std::fs::create_dir_all(src.clone()).unwrap();
        std::fs::create_dir_all(src.join("mods")).unwrap();
        std::fs::write(src.join("mods").join("real-mod.jar"), b"mod").unwrap();
        std::fs::write(src.join("minecraftinstance.json"), b"{}").unwrap();
        std::fs::write(src.join("profile.json"), b"{}").unwrap();
        std::fs::write(src.join("instance_manifest.json"), b"{}").unwrap();
        std::fs::write(src.join(".curseclient"), b"nope").unwrap();

        let inv = copy_payload(&src, &dst).unwrap();
        // Only real-mod.jar should be in inventory.
        assert_eq!(inv.len(), 1);
        assert!(!dst.join("minecraftinstance.json").exists());
        assert!(!dst.join("profile.json").exists());
    }

    #[test]
    fn test_copy_payload_skips_symlinks() {
        let src = test_tmp("copy-sym");
        let dst = test_tmp("copy-sym-dst");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("real.txt"), b"real").unwrap();
        // Create a symlink if supported; otherwise skip this test.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/nonexistent", src.join("link")).unwrap();
        }
        #[cfg(windows)]
        {
            // Try to create a symlink (may fail on some Windows configs).
            let _ = std::os::windows::fs::symlink_file("/nonexistent", src.join("link"));
        }

        let _inv = copy_payload(&src, &dst).unwrap();
        // Only real.txt should be present; symlinks are skipped.
        assert!(dst.join("real.txt").exists());
    }

    #[test]
    fn test_copy_payload_includes_disabled_jars() {
        let src = test_tmp("copy-disabled");
        let dst = test_tmp("copy-disabled-dst");
        std::fs::create_dir_all(src.join("mods")).unwrap();
        std::fs::write(
            src.join("mods").join("disabled-mod.jar.disabled"),
            b"disabled",
        )
        .unwrap();

        let inv = copy_payload(&src, &dst).unwrap();
        assert_eq!(inv.len(), 1);
        assert!(dst.join("mods").join("disabled-mod.jar.disabled").exists());
    }

    // --------------------------------------------------------------
    // plan_fingerprint tests
    // --------------------------------------------------------------

    #[test]
    fn test_plan_fingerprint_is_stable() {
        let fp1 = plan_fingerprint("key1", LauncherKind::Prism, "dest1", &ItemAction::New, true);
        let fp2 = plan_fingerprint("key1", LauncherKind::Prism, "dest1", &ItemAction::New, true);
        assert_eq!(fp1, fp2);
    }

    #[test]
    fn test_plan_fingerprint_differs_on_action() {
        let fp_new = plan_fingerprint("k", LauncherKind::Prism, "d", &ItemAction::New, true);
        let fp_upd = plan_fingerprint("k", LauncherKind::Prism, "d", &ItemAction::Update, true);
        assert_ne!(fp_new, fp_upd);
    }

    #[test]
    fn test_batch_fingerprint_is_stable() {
        let fp = batch_fingerprint(&["a".into(), "b".into()]);
        let fp2 = batch_fingerprint(&["a".into(), "b".into()]);
        assert_eq!(fp, fp2);
    }

    // --------------------------------------------------------------
    // resolve_destination_id tests
    // --------------------------------------------------------------

    fn make_candidate(name: &str) -> ImportCandidate {
        let root = PathBuf::from("/tmp");
        ImportCandidate {
            source_key: name.to_string(),
            launcher: LauncherKind::Prism,
            launcher_installation_key: "test".to_string(),
            display_name: name.to_string(),
            icon_path: None,
            payload_root: root.clone(),
            inventory: ContentInventory {
                payload_root: root,
                total_files: 0,
                total_bytes: 0,
                has_mods: false,
                has_resourcepacks: false,
                has_shaderpacks: false,
                has_datapacks: false,
                has_saves: false,
            },
            loader_tuple: None,
            last_played: None,
            launch_strategy: LaunchStrategy::Normal,
            settings_preview: LaunchSettingsPreview {
                memory_mb: None,
                java_path: None,
                jvm_args: Vec::new(),
            },
            status: CandidateStatus::Ready,
            warnings: Vec::new(),
        }
    }

    #[test]
    fn test_resolve_destination_id_unique() {
        let c = make_candidate("My Instance");
        let id = resolve_destination_id(&c, None, &[]);
        assert_eq!(id, "my-instance");
    }

    #[test]
    fn test_resolve_destination_id_collision() {
        let c = make_candidate("My Instance");
        let id = resolve_destination_id(&c, None, &["my-instance".to_string()]);
        assert_eq!(id, "my-instance-1");
    }

    #[test]
    fn test_resolve_destination_id_custom_name() {
        let c = make_candidate("Original");
        let id = resolve_destination_id(&c, Some("Custom"), &[]);
        assert_eq!(id, "custom");
    }

    // --------------------------------------------------------------
    // is_baseline_unchanged tests
    // --------------------------------------------------------------

    #[test]
    fn test_baseline_matching() {
        let current = vec![
            InstanceImportFileRecord {
                relative_path: "mods/a.jar".into(),
                sha256: "abc".into(),
                size: 100,
                source_modified_ns: None,
            },
            InstanceImportFileRecord {
                relative_path: "logs/latest.log".into(),
                sha256: "log".into(),
                size: 50,
                source_modified_ns: None,
            },
        ];
        let baseline = vec![InstanceImportFileRecord {
            relative_path: "mods/a.jar".into(),
            sha256: "abc".into(),
            size: 100,
            source_modified_ns: None,
        }];
        // logs/ should be ignored, so a.jar matches -> unchanged.
        assert!(is_baseline_unchanged(&current, &baseline));
    }

    #[test]
    fn test_baseline_mismatch() {
        let current = vec![InstanceImportFileRecord {
            relative_path: "mods/a.jar".into(),
            sha256: "abc".into(),
            size: 100,
            source_modified_ns: None,
        }];
        let baseline = vec![InstanceImportFileRecord {
            relative_path: "mods/a.jar".into(),
            sha256: "def".into(),
            size: 100,
            source_modified_ns: None,
        }];
        assert!(!is_baseline_unchanged(&current, &baseline));
    }

    #[test]
    fn test_baseline_new_file() {
        let current = vec![
            InstanceImportFileRecord {
                relative_path: "mods/a.jar".into(),
                sha256: "abc".into(),
                size: 100,
                source_modified_ns: None,
            },
            InstanceImportFileRecord {
                relative_path: "mods/b.jar".into(),
                sha256: "def".into(),
                size: 200,
                source_modified_ns: None,
            },
        ];
        let baseline = vec![InstanceImportFileRecord {
            relative_path: "mods/a.jar".into(),
            sha256: "abc".into(),
            size: 100,
            source_modified_ns: None,
        }];
        // b.jar is new -> not unchanged.
        assert!(!is_baseline_unchanged(&current, &baseline));
    }

    // --------------------------------------------------------------
    // copy_file_with_hash tests
    // --------------------------------------------------------------

    #[test]
    fn test_copy_file_with_hash_matches() {
        let dir = test_tmp("hash-copy");
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("test.bin");
        let dst = dir.join("out.bin");
        let data = b"some test data for hashing";
        std::fs::write(&src, data).unwrap();

        let hash = copy_file_with_hash(&src, &dst).unwrap();
        assert_eq!(hash, sha256_hex(data));
        assert!(dst.exists());
        assert_eq!(std::fs::read(&dst).unwrap(), data);
    }

    // --------------------------------------------------------------
    // sanitize_settings tests
    // --------------------------------------------------------------

    fn make_candidate_with_settings(
        memory_mb: Option<i64>,
        java_path: Option<&str>,
        jvm_args: Vec<&str>,
    ) -> ImportCandidate {
        let mut c = make_candidate("settings-test");
        c.settings_preview = LaunchSettingsPreview {
            memory_mb,
            java_path: java_path.map(|s| s.to_string()),
            jvm_args: jvm_args.into_iter().map(|s| s.to_string()).collect(),
        };
        c
    }

    #[test]
    fn test_sanitize_settings_preserve_memory() {
        let c = make_candidate_with_settings(Some(8192), None, vec![]);
        let s = sanitize_settings(&c, true);
        assert_eq!(s.memory_mb, Some(8192));
    }

    #[test]
    fn test_sanitize_settings_rejects_launcher_java_path() {
        let c = make_candidate_with_settings(
            None,
            Some("C:\\PrismLauncher\\runtime\\java17\\bin\\javaw.exe"),
            vec![],
        );
        let s = sanitize_settings(&c, true);
        assert!(s.java_path.is_none());
    }

    #[test]
    fn test_sanitize_settings_keep_system_java() {
        let c = make_candidate_with_settings(
            None,
            Some("C:\\Program Files\\Java\\jdk-17\\bin\\javaw.exe"),
            vec![],
        );
        let s = sanitize_settings(&c, true);
        assert!(s.java_path.is_some());
    }

    #[test]
    fn test_sanitize_settings_discarded_when_not_preserved() {
        let c = make_candidate_with_settings(Some(4096), Some("/usr/bin/java"), vec![]);
        let s = sanitize_settings(&c, false);
        assert!(s.memory_mb.is_none());
        assert!(s.java_path.is_none());
    }

    #[test]
    fn test_sanitize_settings_allows_jvm_args() {
        let c = make_candidate_with_settings(None, None, vec!["-XX:+UseG1GC", "-Dfoo=bar"]);
        let s = sanitize_settings(&c, true);
        assert!(s.jvm_args.contains(&"-XX:+UseG1GC".to_string()));
        assert!(s.jvm_args.contains(&"-Dfoo=bar".to_string()));
    }

    // --------------------------------------------------------------
    // verify_source_stable tests
    // --------------------------------------------------------------

    #[test]
    fn test_verify_source_stable_ok() {
        let dir = test_tmp("verify-ok");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), b"data").unwrap();
        let inv = vec![InstanceImportFileRecord {
            relative_path: "a.txt".into(),
            sha256: "".into(),
            size: 4,
            source_modified_ns: None,
        }];
        assert!(verify_source_stable(&dir, &inv).is_ok());
    }

    #[test]
    fn test_verify_source_stable_size_changed() {
        let dir = test_tmp("verify-fail");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), b"data").unwrap();
        let inv = vec![InstanceImportFileRecord {
            relative_path: "a.txt".into(),
            sha256: "".into(),
            size: 999,
            source_modified_ns: None,
        }];
        assert!(verify_source_stable(&dir, &inv).is_err());
    }

    #[test]
    fn test_verify_source_stable_vanished() {
        let dir = test_tmp("verify-vanish");
        std::fs::create_dir_all(&dir).unwrap();
        // Don't create the file.
        let inv = vec![InstanceImportFileRecord {
            relative_path: "ghost.txt".into(),
            sha256: "".into(),
            size: 10,
            source_modified_ns: None,
        }];
        assert!(verify_source_stable(&dir, &inv).is_err());
    }

    // --------------------------------------------------------------
    // Full service integration tests (temp fixtures)
    // --------------------------------------------------------------

    #[test]
    fn test_discover_returns_empty_when_no_launcher() {
        let ctx = test_ctx();
        let svc = LauncherImportService::new(ctx);
        let discovery = svc.discover(Some(PathBuf::from("C:\\nonexistent_agora_test_path_xyz")));
        assert!(discovery.prism.launcher.is_none());
        assert!(discovery.curseforge.launcher.is_none());
        assert!(discovery.modrinth.launcher.is_none());
        assert!(discovery.ready_candidates().is_empty());
        assert!(discovery.all_candidates().is_empty());
    }

    #[test]
    fn test_discover_with_prism_fixture() {
        let ctx = test_ctx();
        let svc = LauncherImportService::new(ctx);

        let root = test_tmp("prism-disc");
        let pd = root.join("PrismLauncher");
        std::fs::create_dir_all(&pd).unwrap();
        std::fs::write(
            pd.join("prismlauncher.cfg"),
            "[General]\nInstanceDir=instances\n",
        )
        .unwrap();
        let inst_dir = pd.join("instances");
        std::fs::create_dir_all(&inst_dir).unwrap();
        std::fs::create_dir_all(inst_dir.join("TestInst").join("minecraft").join("mods")).unwrap();
        std::fs::write(
            inst_dir.join("TestInst").join("instance.cfg"),
            "[Minecraft]\nname=Test Instance\n",
        )
        .unwrap();
        std::fs::write(
            inst_dir.join("TestInst").join("mmc-pack.json"),
            r#"{"components":[{"uid":"net.minecraft","version":"1.21"},{"uid":"net.fabricmc.fabric-loader","version":"0.16.0"}]}"#,
        )
        .unwrap();
        std::fs::write(
            inst_dir
                .join("TestInst")
                .join("minecraft")
                .join("mods")
                .join("mod.jar"),
            b"mod content",
        )
        .unwrap();

        let discovery = svc.discover(Some(pd));
        assert!(discovery.prism.launcher.is_some());
        assert!(!discovery.prism.candidates.is_empty());
        let candidates = discovery.ready_candidates();
        assert!(!candidates.is_empty());
        assert_eq!(candidates[0].display_name, "Test Instance");
    }

    #[tokio::test]
    async fn test_plan_with_valid_selection() {
        let ctx = test_ctx();
        let svc = LauncherImportService::new(ctx.clone());

        let root = test_tmp("plan-test");
        let pd = root.join("PrismLauncher");
        std::fs::create_dir_all(&pd).unwrap();
        std::fs::write(
            pd.join("prismlauncher.cfg"),
            "[General]\nInstanceDir=instances\n",
        )
        .unwrap();
        let inst_dir = pd.join("instances");
        std::fs::create_dir_all(&inst_dir).unwrap();
        std::fs::create_dir_all(inst_dir.join("PlanInst").join("minecraft").join("mods")).unwrap();
        std::fs::write(
            inst_dir.join("PlanInst").join("instance.cfg"),
            "[Minecraft]\nname=Plan Instance\n",
        )
        .unwrap();
        std::fs::write(
            inst_dir.join("PlanInst").join("mmc-pack.json"),
            r#"{"components":[{"uid":"net.minecraft","version":"1.21"},{"uid":"net.fabricmc.fabric-loader","version":"0.16.0"}]}"#,
        )
        .unwrap();
        std::fs::write(
            inst_dir
                .join("PlanInst")
                .join("minecraft")
                .join("mods")
                .join("a.jar"),
            b"mod content",
        )
        .unwrap();

        let discovery = svc.discover(Some(pd));
        let prism_candidates = discovery.prism.candidates;
        assert!(!prism_candidates.is_empty());
        let c = &prism_candidates[0];

        let sel = ImportSelection {
            source_key: c.source_key.clone(),
            launcher_kind: c.launcher,
            installation_key: discovery
                .prism
                .launcher
                .as_ref()
                .unwrap()
                .installation_key
                .clone(),
            destination_name: None,
            preserve_settings: true,
        };

        let plan = svc.plan(vec![sel]).unwrap();
        assert_eq!(plan.items.len(), 1);
        let item = &plan.items[0];
        assert_eq!(item.destination_name, "Plan Instance");
        assert!(!item.fingerprint.is_empty());
        assert!(!plan.batch_fingerprint.is_empty());
    }

    #[tokio::test]
    async fn test_plan_rejects_unsupported_candidate() {
        let ctx = test_ctx();
        let svc = LauncherImportService::new(ctx);

        let root = test_tmp("plan-reject");
        let pd = root.join("PrismLauncher");
        std::fs::create_dir_all(&pd).unwrap();
        std::fs::write(
            pd.join("prismlauncher.cfg"),
            "[General]\nInstanceDir=instances\n",
        )
        .unwrap();
        let inst_dir = pd.join("instances");
        std::fs::create_dir_all(&inst_dir).unwrap();
        std::fs::create_dir_all(inst_dir.join("BrokenInst")).unwrap();
        std::fs::write(
            inst_dir.join("BrokenInst").join("instance.cfg"),
            "[Minecraft]\nname=Broken\n",
        )
        .unwrap();
        std::fs::write(
            inst_dir.join("BrokenInst").join("mmc-pack.json"),
            r#"{"components":[{"uid":"net.minecraft","version":"99.0.0"},{"uid":"com.example.unknown","version":"1.0"}]}"#,
        )
        .unwrap();

        let discovery = svc.discover(Some(pd));
        let c = &discovery.prism.candidates[0];
        let sel = ImportSelection {
            source_key: c.source_key.clone(),
            launcher_kind: c.launcher,
            installation_key: discovery
                .prism
                .launcher
                .as_ref()
                .unwrap()
                .installation_key
                .clone(),
            destination_name: None,
            preserve_settings: true,
        };

        let plan = svc.plan(vec![sel]).unwrap();
        assert!(!plan.items[0].blockers.is_empty());
    }

    #[tokio::test]
    async fn test_execute_empty_plan() {
        let ctx = test_ctx();
        let svc = LauncherImportService::new(ctx);

        let plan = LauncherImportPlan {
            batch_fingerprint: String::new(),
            items: Vec::new(),
            peak_bytes: 0,
            total_files: 0,
            batch_blockers: Vec::new(),
        };
        let result = svc.execute(plan).await.unwrap();
        assert!(result.outcomes.is_empty());
    }

    #[tokio::test]
    async fn test_execute_full_import_new_instance() {
        let ctx = test_ctx();
        let svc = LauncherImportService::new(ctx.clone());

        // Create a source instance directory.
        let src_root = test_tmp("exec-src");
        std::fs::create_dir_all(src_root.join("mods")).unwrap();
        std::fs::write(src_root.join("mods").join("sodium.jar"), b"sodium content").unwrap();
        std::fs::write(src_root.join("options.txt"), b"options").unwrap();
        std::fs::create_dir_all(src_root.join("resourcepacks")).unwrap();
        std::fs::write(src_root.join("resourcepacks").join("vanilla.zip"), b"zip").unwrap();

        let item = PerItemPlan {
            fingerprint: plan_fingerprint(
                "test-key",
                LauncherKind::Prism,
                "test-import",
                &ItemAction::New,
                true,
            ),
            destination_id: "test-import".to_string(),
            destination_name: "Test Import".to_string(),
            action: ItemAction::New,
            source_key: "test-key".to_string(),
            launcher_kind: LauncherKind::Prism,
            installation_key: "prism:test".to_string(),
            source_path: src_root.to_string_lossy().to_string(),
            loader_tuple: Some(LoaderTuple {
                loader: "vanilla".to_string(),
                loader_version: String::new(),
                minecraft_version: String::new(),
            }),
            total_bytes: 100,
            total_files: 3,
            preserve_settings: false,
            sanitized_settings: LaunchSettingsPreview {
                memory_mb: None,
                java_path: None,
                jvm_args: Vec::new(),
            },
            existing_import: None,
            blockers: Vec::new(),
            warnings: Vec::new(),
        };

        let plan = LauncherImportPlan {
            batch_fingerprint: batch_fingerprint(std::slice::from_ref(&item.fingerprint)),
            items: vec![item],
            peak_bytes: 100,
            total_files: 3,
            batch_blockers: Vec::new(),
        };

        let result = svc.execute(plan).await.unwrap();
        assert_eq!(result.outcomes.len(), 1);
        match &result.outcomes[0] {
            PerItemOutcome::Imported {
                instance_id,
                warnings,
            } => {
                assert_eq!(instance_id, "test-import");
                assert!(warnings.is_empty());
            }
            other => panic!("Expected Imported, got {other:?}"),
        }

        // Verify instance directory exists.
        let instance_dir = ctx.paths.instances_root().join("test-import");
        assert!(instance_dir.exists());
        assert!(instance_dir.join("instance_manifest.json").exists());
        assert!(instance_dir.join("mods").join("sodium.jar").exists());

        // Verify DB registration.
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        let row = crate::db::get_instance(&conn, "test-import").unwrap();
        assert!(row.is_some());
        assert_eq!(row.unwrap().name, "Test Import");

        // Verify provenance.
        let import =
            find_instance_import_by_source(&conn, "prism", "prism:test", "test-key").unwrap();
        assert!(import.is_some());
    }

    #[tokio::test]
    async fn test_execute_update_unchanged() {
        let ctx = test_ctx();
        let svc = LauncherImportService::new(ctx.clone());

        // Create source.
        let src_root = test_tmp("upd-src");
        std::fs::create_dir_all(src_root.join("mods")).unwrap();
        std::fs::write(src_root.join("mods").join("a.jar"), b"mod a").unwrap();

        // First import to create the baseline.
        let item_new = PerItemPlan {
            fingerprint: plan_fingerprint(
                "upd-key",
                LauncherKind::Prism,
                "upd-inst",
                &ItemAction::New,
                false,
            ),
            destination_id: "upd-inst".to_string(),
            destination_name: "Update Instance".to_string(),
            action: ItemAction::New,
            source_key: "upd-key".to_string(),
            launcher_kind: LauncherKind::Prism,
            installation_key: "prism:upd".to_string(),
            source_path: src_root.to_string_lossy().to_string(),
            loader_tuple: Some(LoaderTuple {
                loader: "vanilla".to_string(),
                loader_version: String::new(),
                minecraft_version: String::new(),
            }),
            total_bytes: 6,
            total_files: 1,
            preserve_settings: false,
            sanitized_settings: LaunchSettingsPreview {
                memory_mb: None,
                java_path: None,
                jvm_args: Vec::new(),
            },
            existing_import: None,
            blockers: Vec::new(),
            warnings: Vec::new(),
        };

        let plan_new = LauncherImportPlan {
            batch_fingerprint: String::new(),
            items: vec![item_new],
            peak_bytes: 6,
            total_files: 1,
            batch_blockers: Vec::new(),
        };
        svc.execute(plan_new).await.unwrap();

        // Now create update plan.
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        let existing = find_instance_import_by_source(&conn, "prism", "prism:upd", "upd-key")
            .unwrap()
            .unwrap();

        let item_upd = PerItemPlan {
            fingerprint: plan_fingerprint(
                "upd-key",
                LauncherKind::Prism,
                "upd-inst",
                &ItemAction::Update,
                false,
            ),
            destination_id: "upd-inst".to_string(),
            destination_name: "Update Instance".to_string(),
            action: ItemAction::Update,
            source_key: "upd-key".to_string(),
            launcher_kind: LauncherKind::Prism,
            installation_key: "prism:upd".to_string(),
            source_path: src_root.to_string_lossy().to_string(),
            loader_tuple: Some(LoaderTuple {
                loader: "vanilla".to_string(),
                loader_version: String::new(),
                minecraft_version: String::new(),
            }),
            total_bytes: 6,
            total_files: 1,
            preserve_settings: false,
            sanitized_settings: LaunchSettingsPreview {
                memory_mb: None,
                java_path: None,
                jvm_args: Vec::new(),
            },
            existing_import: Some(existing),
            blockers: Vec::new(),
            warnings: Vec::new(),
        };

        let plan_upd = LauncherImportPlan {
            batch_fingerprint: String::new(),
            items: vec![item_upd],
            peak_bytes: 6,
            total_files: 1,
            batch_blockers: Vec::new(),
        };
        let result = svc.execute(plan_upd).await.unwrap();
        match &result.outcomes[0] {
            PerItemOutcome::Updated { instance_id, .. } => {
                assert_eq!(instance_id, "upd-inst");
            }
            other => panic!("Expected Updated, got {other:?}"),
        }
    }

    #[test]
    fn test_recover_interrupted_new_import_removes_orphan() {
        let ctx = test_ctx();
        let staging = ctx.paths.staging_root().join("recover-new-staging");
        let final_dir = ctx.paths.instances_root().join("recover-new");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::create_dir_all(&final_dir).unwrap();
        let connection = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        crate::db::upsert_instance_import_job(
            &connection,
            &InstanceImportJob {
                job_id: "recover-new-job".into(),
                instance_id: "recover-new".into(),
                launcher_kind: "prism".into(),
                source_instance_key: "source".into(),
                staging_path: staging.to_string_lossy().into_owned(),
                final_path: final_dir.to_string_lossy().into_owned(),
                quarantine_path: None,
                state: "prepared_new".into(),
                plan_fingerprint: "plan".into(),
                error: None,
                created_at: now.clone(),
                updated_at: now,
            },
        )
        .unwrap();
        drop(connection);

        let warnings = recover_interrupted_jobs(&ctx.paths);
        assert!(!warnings.is_empty());
        assert!(!staging.exists());
        assert!(!final_dir.exists());
        let connection = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        assert!(crate::db::list_instance_import_jobs(&connection)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn test_recover_interrupted_update_restores_quarantine() {
        let ctx = test_ctx();
        let staging = ctx.paths.staging_root().join("recover-update-staging");
        let final_dir = ctx.paths.instances_root().join("recover-update");
        let quarantine = ctx
            .paths
            .staging_root()
            .join(".agora-quarantine-recover-update-test");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::create_dir_all(&final_dir).unwrap();
        std::fs::write(final_dir.join("new.txt"), b"new").unwrap();
        std::fs::create_dir_all(&quarantine).unwrap();
        std::fs::write(quarantine.join("old.txt"), b"old").unwrap();
        let connection = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        crate::db::upsert_instance_import_job(
            &connection,
            &InstanceImportJob {
                job_id: "recover-update-job".into(),
                instance_id: "recover-update".into(),
                launcher_kind: "prism".into(),
                source_instance_key: "source".into(),
                staging_path: staging.to_string_lossy().into_owned(),
                final_path: final_dir.to_string_lossy().into_owned(),
                quarantine_path: Some(quarantine.to_string_lossy().into_owned()),
                state: "prepared_update".into(),
                plan_fingerprint: "plan".into(),
                error: None,
                created_at: now.clone(),
                updated_at: now,
            },
        )
        .unwrap();
        drop(connection);

        recover_interrupted_jobs(&ctx.paths);
        assert!(final_dir.join("old.txt").exists());
        assert!(!final_dir.join("new.txt").exists());
        assert!(!quarantine.exists());
    }

    #[tokio::test]
    async fn test_execute_skips_blocked_items() {
        let ctx = test_ctx();
        let svc = LauncherImportService::new(ctx);

        let item = PerItemPlan {
            fingerprint: String::new(),
            destination_id: "blocked".to_string(),
            destination_name: "Blocked".to_string(),
            action: ItemAction::New,
            source_key: "blocked-key".to_string(),
            launcher_kind: LauncherKind::Prism,
            installation_key: "test".to_string(),
            source_path: "/nonexistent".to_string(),
            loader_tuple: None,
            total_bytes: 0,
            total_files: 0,
            preserve_settings: false,
            sanitized_settings: LaunchSettingsPreview {
                memory_mb: None,
                java_path: None,
                jvm_args: Vec::new(),
            },
            existing_import: None,
            blockers: vec!["Test blocker".to_string()],
            warnings: Vec::new(),
        };

        let plan = LauncherImportPlan {
            batch_fingerprint: String::new(),
            items: vec![item],
            peak_bytes: 0,
            total_files: 0,
            batch_blockers: Vec::new(),
        };
        let result = svc.execute(plan).await.unwrap();
        assert_eq!(result.outcomes.len(), 1);
        assert!(matches!(result.outcomes[0], PerItemOutcome::Skipped { .. }));
    }

    // --------------------------------------------------------------
    // inventory_directory tests
    // --------------------------------------------------------------

    #[test]
    fn test_inventory_directory_disabled_jar() {
        let dir = test_tmp("inv-disabled");
        std::fs::create_dir_all(dir.join("mods")).unwrap();
        std::fs::write(dir.join("mods").join("mymod.jar.disabled"), b"content").unwrap();

        let items = inventory_directory(&dir, "mods", "mod", "prism", "fabric").unwrap();
        assert_eq!(items.len(), 1);
        assert!(!items[0].enabled);
        assert_eq!(items[0].filename, "mymod.jar");
    }

    #[test]
    fn test_inventory_directory_regular() {
        let dir = test_tmp("inv-regular");
        std::fs::create_dir_all(dir.join("mods")).unwrap();
        std::fs::write(dir.join("mods").join("enabled.jar"), b"enabled").unwrap();
        std::fs::write(dir.join("mods").join("also-enabled.jar"), b"also").unwrap();

        let items = inventory_directory(&dir, "mods", "mod", "prism", "fabric").unwrap();
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|m| m.enabled));
        assert!(items.iter().all(|m| m.source == "imported_prism"));
    }

    #[test]
    fn test_inventory_directory_empty() {
        let dir = test_tmp("inv-empty");
        std::fs::create_dir_all(dir.join("mods")).unwrap();
        let items = inventory_directory(&dir, "mods", "mod", "prism", "fabric").unwrap();
        assert!(items.is_empty());
    }

    // --------------------------------------------------------------
    // LauncherImportDiscovery tests
    // --------------------------------------------------------------

    #[test]
    fn test_discovery_aggregate() {
        let empty = LauncherImportDiscovery {
            prism: LauncherDiscovery::empty(),
            curseforge: LauncherDiscovery::empty(),
            modrinth: LauncherDiscovery::empty(),
        };
        assert!(empty.ready_candidates().is_empty());
        assert!(empty.all_candidates().is_empty());
    }

    // --------------------------------------------------------------
    // check_loader_supported tests
    // --------------------------------------------------------------

    #[test]
    fn test_check_loader_supported_vanilla() {
        let tuple = LoaderTuple {
            loader: "vanilla".to_string(),
            loader_version: String::new(),
            minecraft_version: "1.21".to_string(),
        };
        let (resolved, blockers, _) = check_loader_supported(&tuple);
        assert!(resolved.is_some());
        assert!(blockers.is_empty());
    }

    #[test]
    fn test_check_loader_supported_unknown() {
        let tuple = LoaderTuple {
            loader: "fabric".to_string(),
            loader_version: "999.999.999".to_string(),
            minecraft_version: "1.21".to_string(),
        };
        let (resolved, blockers, _) = check_loader_supported(&tuple);
        assert!(resolved.is_none());
        assert!(!blockers.is_empty());
    }

    // --------------------------------------------------------------
    // Serialization roundtrip tests
    // --------------------------------------------------------------

    #[test]
    fn test_types_serialize_roundtrip() {
        let selection = ImportSelection {
            source_key: "key1".into(),
            launcher_kind: LauncherKind::Prism,
            installation_key: "inst-key".into(),
            destination_name: Some("New Name".into()),
            preserve_settings: true,
        };
        let json = serde_json::to_string(&selection).unwrap();
        let back: ImportSelection = serde_json::from_str(&json).unwrap();
        assert_eq!(back.source_key, "key1");
        assert_eq!(back.destination_name.as_deref(), Some("New Name"));

        let plan = LauncherImportPlan {
            batch_fingerprint: "fp".into(),
            items: Vec::new(),
            peak_bytes: 0,
            total_files: 0,
            batch_blockers: Vec::new(),
        };
        let json = serde_json::to_string(&plan).unwrap();
        let back: LauncherImportPlan = serde_json::from_str(&json).unwrap();
        assert_eq!(back.batch_fingerprint, "fp");

        let outcome = PerItemOutcome::Imported {
            instance_id: "id".into(),
            warnings: vec!["warn".into()],
        };
        let json = serde_json::to_string(&outcome).unwrap();
        let back: PerItemOutcome = serde_json::from_str(&json).unwrap();
        match back {
            PerItemOutcome::Imported {
                instance_id,
                warnings,
            } => {
                assert_eq!(instance_id, "id");
                assert_eq!(warnings, vec!["warn"]);
            }
            _ => panic!("wrong variant"),
        }

        let batch_result = LauncherImportBatchResult {
            outcomes: vec![outcome],
        };
        let json = serde_json::to_string(&batch_result).unwrap();
        let back: LauncherImportBatchResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back.outcomes.len(), 1);
    }

    // --------------------------------------------------------------
    // copy_dir_recursive tests (used for .agora_snapshots preservation)
    // --------------------------------------------------------------

    #[test]
    fn test_copy_dir_recursive() {
        let src = test_tmp("copy-dir-src");
        let dst = test_tmp("copy-dir-dst");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("a.txt"), b"a").unwrap();
        std::fs::write(src.join("sub").join("b.txt"), b"b").unwrap();

        copy_dir_recursive(&src, &dst).unwrap();
        assert!(dst.join("a.txt").exists());
        assert!(dst.join("sub").join("b.txt").exists());
        assert_eq!(std::fs::read_to_string(dst.join("a.txt")).unwrap(), "a");
    }

    #[test]
    fn test_copy_dir_recursive_skip_symlinks() {
        let src = test_tmp("copy-dir-sym");
        let dst = test_tmp("copy-dir-sym-dst");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("real.txt"), b"real").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("/nonexistent", src.join("link")).unwrap();

        copy_dir_recursive(&src, &dst).unwrap();
        assert!(dst.join("real.txt").exists());
    }
}
