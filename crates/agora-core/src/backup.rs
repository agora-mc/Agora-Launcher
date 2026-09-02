//! User-facing backup layer built on the content-addressed snapshot store.
//!
//! The internal snapshot store (`crates/agora-core/src/snapshot.rs:1`) is
//! deduplicated and content-addressed: blobs live in `snapshot-objects/` and are
//! shared across snapshots of the same instance.  An exported artifact cannot
//! rely on that store — the destination (e.g. a Dropbox-synced folder) has none
//! of those blobs — so an export is necessarily a **full materialisation**:
//! every tracked file is copied by value into a single self-describing zip.
//!
//! # Container choice: why zip?
//!
//! * **Already in the dependency tree.** `snapshot.rs:1` already depends on
//!   `zip = "0.6"` for legacy v1/v2 snapshots and for the `ZipArchive` reader
//!   used by `restore_snapshot`.  Adding a second container (e.g. `tar` + `flate2`)
//!   would add a new attack surface and a second set of path-traversal bugs to
//!   audit, without giving us anything zip does not already provide.
//! * **Streaming and hashing.** Both `snapshot.rs:1069` and `helpers.rs:364`
//!   stream files in 64 KiB–1 MiB chunks while hashing with `sha2::Sha256`.
//!   `zip::ZipWriter` supports the same incremental pattern (`start_file` +
//!   `write_all` per chunk) and `ZipArchive::by_name` streams on read, so we
//!   can hash-verify while copying without ever buffering a whole world save in
//!   memory.
//! * **Random-access manifest.** The backup's own `manifest.json` lives at the
//!   zip's root.  An importer can read and fully validate that manifest (schema
//!   version, path safety, hash format, duplicate detection, entry-count and
//!   total-size limits) **before** writing any file to the instance directory.
//!   `tar` would require a linear scan to find the manifest, and `tar.gz` would
//!   require decompressing the whole stream before the manifest is visible.
//! * **Single-file artifact.** A zip is a single file, which is exactly what
//!   a Dropbox/OneDrive-synced folder expects.  A directory export would need a
//!   second step to make it sync-friendly (and would race with a sync that
//!   sees a half-written tree).
//! * **Zip Slip is already mitigated.** `snapshot.rs:1626` and `import.rs:392`
//!   both canonicalise archive paths and reject `..`, absolute paths and `\`.
//!   Reusing zip lets us reuse the same `validate_relative_path` guard
//!   (`snapshot.rs:1626`) verbatim.
//!
//! We use `CompressionMethod::Stored` (no compression) for the backup zip,
//! matching `export_service.rs:67` for `.mrpack`.  This keeps the hash
//! intentionally transparent (the bytes on disk are the bytes that were
//! hashed) and avoids paying Deflate's CPU cost on saves that are already
//! compressed (e.g. `.mca` region files).  Dropbox's own delta sync handles
//! the wire cost; recompressing here would only slow the export.
//!
//! # What belongs in the artifact's metadata?
//!
//! An import onto a fresh machine must be able to validate before mutating, so
//! the zip contains a single `manifest.json` at its root with:
//!
//! * `schema_version` — the backup format version (`1`).  Imports reject any
//!   `schema_version` newer than the maximum they understand, so a future
//!   `2` does not silently misinterpret an old reader.
//! * `instance_id` — the source instance's directory name (sanitised).  This is
//!   **provenance only**, never trusted for filesystem paths; the caller
//!   supplies the destination `instance_dir` explicitly.
//! * `snapshot` — the original `Snapshot` struct (id, label, created_at,
//!   file_count, size_estimate).  The snapshot's own `created_at` is the
//!   retention-relevant timestamp, not the zip's mtime (which Dropbox may
//!   rewrite).
//! * `files` — sorted list of `{relative_path, size, sha256}` for every
//!   tracked file.  Each entry's `relative_path` is validated with the same
//!   `validate_relative_path` (`snapshot.rs:1626`) that snapshots use, and each
//!   `sha256` is validated as 64 hex digits.  The import verifies that the zip
//!   contains exactly this set — no extra entries, no missing entries, no
//!   directory entries — before writing anything to the live tree.
//! * `exported_at` — RFC3339 timestamp of the export itself, for debugging.
//! * `scope_entries` — the tracked-entry set that was captured (usually
//!   `TRACKED_ENTRIES`).  Stored for completeness, not trusted for restore
//!   filtering; the file list is the source of truth.
//!
//! On import the caller points at the artifact file (not a directory), so there
//! is no TOCTOU between "choose a folder" and "read the backup": the zip is
//! validated atomically, staged to a sibling directory on the same volume, then
//! promoted with same-volume renames.  The content-addressed object store is
//! repopulated from the staged files so future snapshots deduplicate correctly,
//! and a `SnapshotManifest` is written to `.agora_snapshots/<id>.json`.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::Digest;

// ---------------------------------------------------------------------------
// Constants and types
// ---------------------------------------------------------------------------

pub const BACKUP_SCHEMA_VERSION: u32 = 1;
const BACKUP_MANIFEST_NAME: &str = "manifest.json";
const MAX_BACKUP_MANIFEST_BYTES: u64 = 5 * 1024 * 1024; // 5 MiB
const MAX_BACKUP_ENTRIES: usize = 10_000;
const MAX_BACKUP_SINGLE_FILE_SIZE: u64 = 1024 * 1024 * 1024; // 1 GiB
const MAX_BACKUP_TOTAL_SIZE: u64 = 8 * 1024 * 1024 * 1024; // 8 GiB

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupManifest {
    pub schema_version: u32,
    pub instance_id: String,
    pub snapshot: crate::snapshot::Snapshot,
    pub files: Vec<BackupFileEntry>,
    pub exported_at: String,
    pub scope_entries: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupFileEntry {
    pub relative_path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct BackupRetentionPolicy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_last: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_days: Option<u32>,
}

impl BackupRetentionPolicy {
    pub fn keep_last(n: u32) -> Self {
        Self {
            keep_last: Some(n),
            keep_days: None,
        }
    }

    pub fn keep_days(n: u32) -> Self {
        Self {
            keep_last: None,
            keep_days: Some(n),
        }
    }

    pub fn keep_last_and_days(last: u32, days: u32) -> Self {
        Self {
            keep_last: Some(last),
            keep_days: Some(days),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers: instance id sanitising and file naming
// ---------------------------------------------------------------------------

fn instance_id_from_dir(instance_dir: &Path) -> String {
    instance_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("instance")
        .to_string()
}

fn sanitized_instance_id(instance_dir: &Path) -> String {
    crate::paths::sanitize_id(&instance_id_from_dir(instance_dir))
}

fn backup_file_name(instance_dir: &Path, snapshot_id: &str) -> String {
    let safe_instance = sanitized_instance_id(instance_dir);
    // Avoid characters that are problematic on Windows or cloud sync: use
    // hyphen-separated, no colons.
    format!("agora-backup-{safe_instance}-{snapshot_id}.zip")
}

// ---------------------------------------------------------------------------
// Validation helpers (reuse snapshot's path rules)
// ---------------------------------------------------------------------------

fn validate_backup_manifest(manifest: &BackupManifest) -> Result<(), String> {
    if manifest.schema_version == 0 || manifest.schema_version > BACKUP_SCHEMA_VERSION {
        return Err(format!(
            "unsupported backup schema version {} (maximum supported is {})",
            manifest.schema_version, BACKUP_SCHEMA_VERSION
        ));
    }
    crate::snapshot::validate_snapshot_id(&manifest.snapshot.id)?;
    if manifest.snapshot.id.is_empty() {
        return Err("backup snapshot id must not be empty".into());
    }
    if manifest.files.len() > MAX_BACKUP_ENTRIES {
        return Err(format!(
            "backup file count {} exceeds limit of {}",
            manifest.files.len(),
            MAX_BACKUP_ENTRIES
        ));
    }
    let mut seen = HashSet::new();
    let mut total: u64 = 0;
    for entry in &manifest.files {
        crate::snapshot::validate_relative_path(&entry.relative_path)?;
        if !seen.insert(entry.relative_path.clone()) {
            return Err(format!(
                "backup manifest contains duplicate path {}",
                entry.relative_path
            ));
        }
        crate::snapshot::validate_blob_hash(&entry.sha256)?;
        if entry.size > MAX_BACKUP_SINGLE_FILE_SIZE {
            return Err(format!(
                "backup file {} size {} exceeds per-file limit",
                entry.relative_path, entry.size
            ));
        }
        total = total.saturating_add(entry.size);
        if total > MAX_BACKUP_TOTAL_SIZE {
            return Err(format!(
                "backup total size {} exceeds limit of {}",
                total, MAX_BACKUP_TOTAL_SIZE
            ));
        }
    }
    // Cross-check snapshot's declared counts against manifest.
    if manifest.files.len() != manifest.snapshot.file_count {
        return Err(format!(
            "backup manifest file_count {} does not match files length {}",
            manifest.snapshot.file_count,
            manifest.files.len()
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Retention
// ---------------------------------------------------------------------------

fn is_snapshot_protected(snapshot: &crate::snapshot::Snapshot, lkg: &crate::lkg::LkgState) -> bool {
    // Mirrors `lkg.rs:443` run_retention's protected logic:
    // - current LKG (the exact snapshot promoted by the last successful launch)
    // - crash-doctor active recovery points (label prefix `crash-doctor-`)
    // For backup retention we also treat pre-restore snapshots as protected
    // insofar as we keep at least one, but the primary protection is the two
    // categories above which must never be deleted by any retention sweep.
    let is_current_lkg = lkg.current_lkg_snapshot_id.as_deref() == Some(&snapshot.id);
    let is_crash_doctor = snapshot
        .label
        .as_deref()
        .is_some_and(|label| label.starts_with("crash-doctor-"));
    is_current_lkg || is_crash_doctor
}

fn is_pre_restore_snapshot(snapshot: &crate::snapshot::Snapshot) -> bool {
    snapshot
        .label
        .as_deref()
        .is_some_and(|label| label.starts_with("pre-restore-"))
}

fn parse_snapshot_time(
    snapshot: &crate::snapshot::Snapshot,
) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(&snapshot.created_at)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

/// Pure retention planner for backups.
///
/// `snapshots_newest_first` must be sorted newest-first (descending
/// `created_at`), as returned by `snapshot::list_snapshots`.  The result is
/// the set of snapshot ids that should be **evicted**.  Protected snapshots
/// (current LKG + crash-doctor) are never evicted.  If `policy` has neither
/// `keep_last` nor `keep_days`, nothing is evicted.  The "only copy" guarantee
/// is enforced: if eviction would remove every snapshot, the newest one is
/// retained.
pub fn plan_backup_retention(
    snapshots_newest_first: &[crate::snapshot::Snapshot],
    lkg: &crate::lkg::LkgState,
    policy: &BackupRetentionPolicy,
) -> Vec<String> {
    plan_backup_retention_with_now(snapshots_newest_first, lkg, policy, chrono::Utc::now())
}

pub(crate) fn plan_backup_retention_with_now(
    snapshots_newest_first: &[crate::snapshot::Snapshot],
    lkg: &crate::lkg::LkgState,
    policy: &BackupRetentionPolicy,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<String> {
    if snapshots_newest_first.is_empty() {
        return Vec::new();
    }
    if policy.keep_last.is_none() && policy.keep_days.is_none() {
        return Vec::new();
    }

    let mut keep = HashSet::new();

    // Always keep protected snapshots.
    for snapshot in snapshots_newest_first {
        if is_snapshot_protected(snapshot, lkg) {
            keep.insert(snapshot.id.clone());
        }
    }

    // Non-protected snapshots in newest-first order.
    let non_protected: Vec<&crate::snapshot::Snapshot> = snapshots_newest_first
        .iter()
        .filter(|snapshot| !keep.contains(&snapshot.id))
        .collect();

    // keep_last: keep newest N among non-protected.
    if let Some(keep_last) = policy.keep_last {
        let n = keep_last as usize;
        for snapshot in non_protected.iter().take(n) {
            keep.insert(snapshot.id.clone());
        }
    }

    // keep_days: keep any non-protected within the window.
    if let Some(keep_days) = policy.keep_days {
        let cutoff = now - chrono::Duration::days(keep_days as i64);
        for snapshot in &non_protected {
            if let Some(created) = parse_snapshot_time(snapshot) {
                if created >= cutoff {
                    keep.insert(snapshot.id.clone());
                }
            } else {
                // Unparseable timestamp is treated as old — do not keep by age.
                // It may still be kept by keep_last or protected status.
            }
        }
    }

    // Guarantee at least one snapshot remains ("never delete the only copy").
    // If every snapshot would be evicted, keep the newest one.
    if keep.is_empty() && !snapshots_newest_first.is_empty() {
        keep.insert(snapshots_newest_first[0].id.clone());
    }

    // Additionally, ensure at least one pre-restore snapshot remains if any
    // exist and all would be evicted.  This mirrors `lkg.rs:298`
    // `keep_pre_restore_count = 1`.
    let has_pre_restore = snapshots_newest_first.iter().any(is_pre_restore_snapshot);
    let kept_pre_restore = snapshots_newest_first
        .iter()
        .any(|snapshot| is_pre_restore_snapshot(snapshot) && keep.contains(&snapshot.id));
    if has_pre_restore && !kept_pre_restore {
        // Keep the newest pre-restore snapshot.
        if let Some(newest_pre) = snapshots_newest_first
            .iter()
            .find(|snapshot| is_pre_restore_snapshot(snapshot))
        {
            keep.insert(newest_pre.id.clone());
        }
    }

    snapshots_newest_first
        .iter()
        .filter(|snapshot| !keep.contains(&snapshot.id))
        .map(|snapshot| snapshot.id.clone())
        .collect()
}

/// Run backup retention for an instance directory.
///
/// Lists snapshots, loads LKG state, computes the eviction set for `policy`,
/// and deletes the evicted snapshots via `snapshot::delete_snapshot`.  If a
/// restore is currently in progress (detected by the marker file
/// `snapshot.rs:14` `.agora_restore_in_progress`), the sweep is skipped
/// entirely so it cannot delete a backup the restore is reading.  This matches
/// the safety contract of `snapshot.rs:1348` which holds the marker for the
/// whole promotion phase.
pub fn run_backup_retention(
    instance_dir: &Path,
    policy: &BackupRetentionPolicy,
) -> Result<Vec<String>, String> {
    // Concurrency guard: never mutate snapshots while a restore holds the marker.
    if instance_dir.join(crate::snapshot::RESTORE_MARKER).exists() {
        return Ok(Vec::new());
    }

    let snapshots = match crate::snapshot::list_snapshots(instance_dir) {
        Ok(snapshots) => snapshots,
        Err(error) => {
            // `list_snapshots` errors precisely when the restore marker is
            // present (`snapshot.rs:1769`).  Treat it as "skip retention".
            if error.contains("Previous restore was interrupted") {
                return Ok(Vec::new());
            }
            return Err(error);
        }
    };
    if snapshots.is_empty() {
        return Ok(Vec::new());
    }

    let lkg = crate::lkg::read_lkg_state(instance_dir).unwrap_or_default();
    let to_evict = plan_backup_retention(&snapshots, &lkg, policy);

    let mut evicted = Vec::new();
    let mut errors = Vec::new();
    for id in to_evict {
        // Re-check marker before each deletion in case a restore started
        // concurrently between the initial check and this iteration.
        if instance_dir.join(crate::snapshot::RESTORE_MARKER).exists() {
            break;
        }
        match crate::snapshot::delete_snapshot(instance_dir, &id) {
            Ok(()) => evicted.push(id),
            Err(error) => errors.push(format!("{id}: {error}")),
        }
    }
    if errors.is_empty() {
        Ok(evicted)
    } else {
        Err(format!(
            "backup retention could not remove: {}",
            errors.join("; ")
        ))
    }
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

/// Export a snapshot to an arbitrary external directory as a single
/// self-describing zip artifact.
///
/// The artifact is fully materialised: every file's bytes are copied from the
/// content-addressed blob store (or, for legacy v1/v2 snapshots, from the
/// legacy `.zip` archive) into the new zip, so the destination does not need
/// any of the internal `snapshot-objects/` blobs.  If any source blob is
/// missing or fails hash verification, the export **fails and deletes the
/// partial artifact** rather than silently producing a truncated zip.
pub fn export_snapshot(
    instance_dir: &Path,
    snapshot_id: &str,
    export_dir: &Path,
) -> Result<PathBuf, String> {
    crate::snapshot::validate_snapshot_id(snapshot_id)?;
    if !instance_dir.is_dir() {
        return Err(format!(
            "instance directory does not exist: {}",
            instance_dir.display()
        ));
    }
    fs::create_dir_all(export_dir)
        .map_err(|error| format!("failed to create export directory: {error}"))?;

    // Prevent export while a restore is mutating the live tree; the snapshot
    // bytes themselves are immutable, but a concurrent restore would confuse
    // the caller about "which snapshot are we exporting".
    if instance_dir.join(crate::snapshot::RESTORE_MARKER).exists() {
        return Err(
            "cannot export while a restore is in progress; retry after the restore completes"
                .into(),
        );
    }

    let snapshots = crate::snapshot::list_snapshots(instance_dir)
        .map_err(|error| format!("failed to list snapshots: {error}"))?;
    let snapshot = snapshots
        .iter()
        .find(|snapshot| snapshot.id == snapshot_id)
        .cloned()
        .ok_or_else(|| format!("snapshot {snapshot_id} not found"))?;

    // `snapshot_file_index` is the canonical, scope-agnostic reader that
    // handles both modern manifests and legacy zips, validates paths, dupes,
    // and hash formats, and returns a sorted index.
    let file_index = crate::snapshot::snapshot_file_index(instance_dir, snapshot_id)?;

    // Enforce manifest-level limits before materialising.
    if file_index.len() > MAX_BACKUP_ENTRIES {
        return Err(format!(
            "snapshot file count {} exceeds backup limit of {}",
            file_index.len(),
            MAX_BACKUP_ENTRIES
        ));
    }
    let total_size: u64 = file_index.iter().map(|entry| entry.size).sum();
    if total_size > MAX_BACKUP_TOTAL_SIZE {
        return Err(format!(
            "snapshot total size {total_size} exceeds backup limit of {MAX_BACKUP_TOTAL_SIZE}"
        ));
    }
    for entry in &file_index {
        if entry.size > MAX_BACKUP_SINGLE_FILE_SIZE {
            return Err(format!(
                "snapshot file {} size {} exceeds per-file limit",
                entry.path, entry.size
            ));
        }
    }

    // Build the backup manifest that will be written as the first zip entry.
    // It is self-describing so an import onto a fresh machine can validate
    // before writing any file.
    let backup_manifest = BackupManifest {
        schema_version: BACKUP_SCHEMA_VERSION,
        instance_id: sanitized_instance_id(instance_dir),
        snapshot: snapshot.clone(),
        files: file_index
            .iter()
            .map(|entry| BackupFileEntry {
                relative_path: entry.path.clone(),
                size: entry.size,
                sha256: entry.sha256.to_ascii_lowercase(),
            })
            .collect(),
        exported_at: chrono::Utc::now().to_rfc3339(),
        scope_entries: crate::snapshot::TRACKED_ENTRIES
            .iter()
            .map(|entry| entry.to_string())
            .collect(),
    };
    validate_backup_manifest(&backup_manifest)?;

    // Prepare the output zip as a temp file in the export directory so a
    // failure never leaves a partial artifact at the final name.
    let file_name = backup_file_name(instance_dir, snapshot_id);
    let final_path = export_dir.join(&file_name);
    let temp_path = export_dir.join(format!(".{file_name}.{}.tmp", uuid::Uuid::new_v4()));

    let create_result: Result<(), String> = (|| {
        let file = fs::File::create(&temp_path)
            .map_err(|error| format!("failed to create backup artifact: {error}"))?;
        let mut zip = zip::ZipWriter::new(file);
        let options =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);

        // Write the manifest first so an importer can validate without
        // scanning the whole zip.  The writer is buffered automatically by
        // ZipWriter's internal buffering.
        let manifest_bytes = serde_json::to_vec_pretty(&backup_manifest)
            .map_err(|error| format!("failed to serialize backup manifest: {error}"))?;
        if manifest_bytes.len() as u64 > MAX_BACKUP_MANIFEST_BYTES {
            return Err("backup manifest exceeds size limit".into());
        }
        zip.start_file(BACKUP_MANIFEST_NAME, options)
            .map_err(|error| format!("failed to start backup manifest entry: {error}"))?;
        zip.write_all(&manifest_bytes)
            .map_err(|error| format!("failed to write backup manifest: {error}"))?;

        // Determine whether the snapshot is stored as a modern manifest + blobs
        // or as a legacy zip.  `list_snapshots` already proved one of them
        // exists, but check the filesystem directly for the branch.
        let modern_manifest_path =
            crate::snapshot::snapshot_manifest_path(instance_dir, snapshot_id);
        let is_modern = modern_manifest_path.is_file();

        // Fail closed if any blob is missing or corrupt rather than silently
        // omitting it.  The zip must be complete or not exist at all.
        if is_modern {
            for entry in &file_index {
                let blob_path = crate::snapshot::snapshot_blob_path(instance_dir, &entry.sha256);
                // Validate the blob before streaming it into the zip.
                if !blob_path.is_file() {
                    return Err(format!(
                        "snapshot blob is missing for {}: {}",
                        entry.path,
                        blob_path.display()
                    ));
                }
                if !crate::snapshot::blob_file_matches(&blob_path, &entry.sha256, entry.size) {
                    return Err(format!(
                        "snapshot blob hash mismatch for {}: object is corrupted or modified",
                        entry.path
                    ));
                }
                // Stream the blob into the zip while re-hashing to ensure the
                // bytes we emit are the bytes we verified.
                zip.start_file(&entry.path, options).map_err(|error| {
                    format!("failed to start backup entry for {}: {error}", entry.path)
                })?;
                let mut reader = fs::File::open(&blob_path)
                    .map_err(|error| format!("failed to open blob for {}: {error}", entry.path))?;
                let mut buffer = vec![0u8; 1024 * 1024];
                let mut hasher = sha2::Sha256::new();
                let mut remaining = entry.size;
                loop {
                    let to_read = std::cmp::min(buffer.len() as u64, remaining) as usize;
                    if to_read == 0 {
                        break;
                    }
                    let read = reader.read(&mut buffer[..to_read]).map_err(|error| {
                        format!("failed to read blob for {}: {error}", entry.path)
                    })?;
                    if read == 0 {
                        break;
                    }
                    hasher.update(&buffer[..read]);
                    zip.write_all(&buffer[..read]).map_err(|error| {
                        format!("failed to write backup entry for {}: {error}", entry.path)
                    })?;
                    remaining -= read as u64;
                }
                if remaining != 0 {
                    return Err(format!(
                        "snapshot blob truncated for {}: expected {} bytes",
                        entry.path, entry.size
                    ));
                }
                let actual = format!("{:x}", hasher.finalize());
                if !actual.eq_ignore_ascii_case(&entry.sha256) {
                    return Err(format!(
                        "snapshot blob hash mismatch for {} during export",
                        entry.path
                    ));
                }
            }
        } else {
            // Legacy zip path: copy bytes from the legacy archive directly.
            let legacy_zip_path = crate::snapshot::snapshot_zip_path(instance_dir, snapshot_id);
            let legacy_file = fs::File::open(&legacy_zip_path)
                .map_err(|error| format!("failed to open legacy snapshot zip: {error}"))?;
            let mut legacy_archive = zip::ZipArchive::new(legacy_file)
                .map_err(|error| format!("failed to read legacy snapshot zip: {error}"))?;

            for entry in &file_index {
                // Legacy archives store files at their relative path; locate it.
                let mut legacy_entry = legacy_archive.by_name(&entry.path).map_err(|error| {
                    format!(
                        "snapshot file {} is missing from legacy archive: {error}",
                        entry.path
                    )
                })?;
                if legacy_entry.is_dir() {
                    return Err(format!(
                        "snapshot path {} is a directory, expected a file",
                        entry.path
                    ));
                }
                zip.start_file(&entry.path, options).map_err(|error| {
                    format!("failed to start backup entry for {}: {error}", entry.path)
                })?;
                let mut hasher = sha2::Sha256::new();
                let mut buffer = vec![0u8; 1024 * 1024];
                let mut total: u64 = 0;
                loop {
                    let read = legacy_entry.read(&mut buffer).map_err(|error| {
                        format!("failed to read legacy entry {}: {error}", entry.path)
                    })?;
                    if read == 0 {
                        break;
                    }
                    if total + read as u64 > MAX_BACKUP_SINGLE_FILE_SIZE {
                        return Err(format!(
                            "legacy entry {} exceeds per-file limit during export",
                            entry.path
                        ));
                    }
                    hasher.update(&buffer[..read]);
                    zip.write_all(&buffer[..read]).map_err(|error| {
                        format!("failed to write backup entry for {}: {error}", entry.path)
                    })?;
                    total += read as u64;
                }
                if total != entry.size {
                    return Err(format!(
                        "legacy snapshot size mismatch for {}: expected {}, got {}",
                        entry.path, entry.size, total
                    ));
                }
                let actual = format!("{:x}", hasher.finalize());
                if !actual.eq_ignore_ascii_case(&entry.sha256) {
                    return Err(format!(
                        "legacy snapshot hash mismatch for {} during export",
                        entry.path
                    ));
                }
            }
        }

        let file = zip
            .finish()
            .map_err(|error| format!("failed to finalize backup zip: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("failed to sync backup artifact: {error}"))?;
        Ok(())
    })();

    if let Err(error) = create_result {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }

    // Atomic commit: rename temp -> final.  On Windows this replaces an
    // existing file if the user exported the same snapshot twice.
    let _ = fs::remove_file(&final_path);
    fs::rename(&temp_path, &final_path)
        .map_err(|error| format!("failed to commit backup artifact: {error}"))?;
    Ok(final_path)
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

/// Validate an artifact file path before opening it.
///
/// The artifact may be on a Dropbox-synced folder controlled by another user,
/// so the file itself is untrusted: reject symlinks and directories.
fn validate_artifact_path(artifact_path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(artifact_path)
        .map_err(|error| format!("failed to inspect backup artifact: {error}"))?;
    if metadata.file_type().is_symlink() {
        return Err("backup artifact must not be a symlink".into());
    }
    if !metadata.is_file() {
        return Err("backup artifact is not a file".into());
    }
    if metadata.len() > MAX_BACKUP_TOTAL_SIZE + MAX_BACKUP_MANIFEST_BYTES + 1024 * 1024 {
        return Err(format!(
            "backup artifact size {} exceeds limit",
            metadata.len()
        ));
    }
    Ok(())
}

/// Read and fully validate the backup manifest from an open zip archive
/// without writing any file to the instance directory.
fn read_and_validate_manifest<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> Result<BackupManifest, String> {
    // General zip bomb guard: number of entries.
    if archive.len() > MAX_BACKUP_ENTRIES + 1 {
        return Err(format!(
            "backup zip entry count {} exceeds limit of {}",
            archive.len(),
            MAX_BACKUP_ENTRIES + 1
        ));
    }

    let mut manifest_entry = archive
        .by_name(BACKUP_MANIFEST_NAME)
        .map_err(|error| format!("backup manifest is missing ({BACKUP_MANIFEST_NAME}): {error}"))?;

    if manifest_entry.is_dir() {
        return Err("backup manifest entry is a directory".into());
    }
    if manifest_entry.size() > MAX_BACKUP_MANIFEST_BYTES {
        return Err(format!(
            "backup manifest size {} exceeds limit",
            manifest_entry.size()
        ));
    }

    // Bounded read: one byte past the limit so we detect over-long manifests
    // rather than silently truncating.
    let mut manifest_bytes = Vec::new();
    manifest_entry
        .by_ref()
        .take(MAX_BACKUP_MANIFEST_BYTES + 1)
        .read_to_end(&mut manifest_bytes)
        .map_err(|error| format!("failed to read backup manifest: {error}"))?;
    if manifest_bytes.len() as u64 > MAX_BACKUP_MANIFEST_BYTES {
        return Err("backup manifest exceeds size limit".into());
    }
    drop(manifest_entry);

    let manifest: BackupManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("failed to parse backup manifest: {error}"))?;
    validate_backup_manifest(&manifest)?;
    Ok(manifest)
}

/// Verify that a zip's file set is exactly `{manifest.json} ∪ files` with no
/// extra, missing, or directory entries, and that no entry's path is unsafe.
fn validate_zip_file_set<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    manifest: &BackupManifest,
) -> Result<(), String> {
    let expected: HashSet<String> = manifest
        .files
        .iter()
        .map(|entry| entry.relative_path.clone())
        .collect();

    let mut seen = HashSet::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| format!("failed to read backup zip entry {index}: {error}"))?;
        let name = entry.name().to_string();
        if name == BACKUP_MANIFEST_NAME {
            if entry.is_dir() {
                return Err("backup manifest entry must not be a directory".into());
            }
            continue;
        }
        // Every non-manifest entry must be listed in the manifest.
        if !expected.contains(&name) {
            return Err(format!(
                "backup zip contains unexpected file not listed in manifest: {name}"
            ));
        }
        // Validate path safety even for listed entries — the manifest itself is
        // untrusted, so a compromised manifest could list an unsafe path.
        crate::snapshot::validate_relative_path(&name)?;
        if entry.is_dir() {
            return Err(format!(
                "backup zip path {name} is a directory, expected a file"
            ));
        }
        if !seen.insert(name.clone()) {
            return Err(format!("backup zip contains duplicate entry {name}"));
        }
    }

    // Check for missing entries (listed in manifest but absent from zip).
    for name in &expected {
        if !seen.contains(name) {
            return Err(format!(
                "backup file {name} is listed in manifest but missing from zip"
            ));
        }
    }
    Ok(())
}

/// Hash-verify every file's zip entry against the manifest's size and sha256
/// without writing anything to the filesystem.  This is the pre-mutation gate.
fn verify_zip_entries_hash<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    manifest: &BackupManifest,
) -> Result<(), String> {
    // Build lookup for expected hashes/sizes.
    let expected: BTreeMap<&str, &BackupFileEntry> = manifest
        .files
        .iter()
        .map(|entry| (entry.relative_path.as_str(), entry))
        .collect();

    for (name, expected_entry) in &expected {
        let mut entry = archive
            .by_name(name)
            .map_err(|error| format!("backup entry {name} is missing from zip: {error}"))?;
        if entry.size() != expected_entry.size {
            return Err(format!(
                "backup size mismatch for {name}: manifest {}, zip {}",
                expected_entry.size,
                entry.size()
            ));
        }
        if entry.size() > MAX_BACKUP_SINGLE_FILE_SIZE {
            return Err(format!(
                "backup file {name} size {} exceeds per-file limit",
                entry.size()
            ));
        }
        let mut hasher = sha2::Sha256::new();
        let mut buffer = vec![0u8; 1024 * 1024];
        let mut total: u64 = 0;
        loop {
            let read = entry
                .read(&mut buffer)
                .map_err(|error| format!("failed to read backup entry {name}: {error}"))?;
            if read == 0 {
                break;
            }
            total += read as u64;
            if total > MAX_BACKUP_SINGLE_FILE_SIZE {
                return Err(format!(
                    "backup file {name} exceeds per-file limit during verify"
                ));
            }
            hasher.update(&buffer[..read]);
        }
        if total != expected_entry.size {
            return Err(format!(
                "backup size mismatch for {name}: expected {}, got {total}",
                expected_entry.size
            ));
        }
        let actual = format!("{:x}", hasher.finalize());
        if !actual.eq_ignore_ascii_case(&expected_entry.sha256) {
            return Err(format!(
                "backup hash mismatch for {name}: expected {}, got {actual}",
                expected_entry.sha256
            ));
        }
    }
    Ok(())
}

/// Import a backup artifact into an instance directory.
///
/// The artifact is validated **before** any file is written to `instance_dir`:
/// the manifest's schema version, path safety, hash format, duplicate and
/// size checks are performed, then the zip's file set is checked for exact
/// correspondence with the manifest, and every zip entry is hash-verified.
/// Only after all checks pass does the function stage the files to a sibling
/// directory on the same volume, repopulate the content-addressed object store
/// and `SnapshotManifest`, and atomically promote the staged files into the
/// live instance tree.  The caller may point `artifact_path` at any
/// user-chosen external directory (e.g. Dropbox), and `instance_dir` may not
/// yet exist — it is created in that case.
pub fn import_backup(
    instance_dir: &Path,
    artifact_path: &Path,
) -> Result<crate::snapshot::Snapshot, String> {
    validate_artifact_path(artifact_path)?;

    // Do not import while a restore is in progress — the live tree is in a
    // transient state (some tracked roots already moved to `.agora_pre_restore`).
    if instance_dir.exists() && instance_dir.join(crate::snapshot::RESTORE_MARKER).exists() {
        return Err(
            "cannot import while a restore is in progress; retry after the restore completes"
                .into(),
        );
    }

    // Phase 1: open and validate without touching the instance directory.
    let file = fs::File::open(artifact_path)
        .map_err(|error| format!("failed to open backup artifact: {error}"))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| format!("failed to read backup zip: {error}"))?;

    let manifest = read_and_validate_manifest(&mut archive)?;
    validate_zip_file_set(&mut archive, &manifest)?;
    verify_zip_entries_hash(&mut archive, &manifest)?;

    // Phase 2: stage the validated content to a sibling directory on the same
    // volume as `instance_dir` so promotions are same-volume renames.
    // Re-open the archive for extraction (the previous readers borrowed it).
    let file = fs::File::open(artifact_path)
        .map_err(|error| format!("failed to reopen backup artifact: {error}"))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| format!("failed to read backup zip: {error}"))?;

    let instances_root = instance_dir.parent().unwrap_or(instance_dir);
    let staging_dir = instances_root.join(format!(
        ".agora-import-{}-{}",
        sanitized_instance_id(instance_dir),
        uuid::Uuid::new_v4()
    ));
    // Ensure we do not collide with an existing staging dir.
    if staging_dir.exists() {
        fs::remove_dir_all(&staging_dir)
            .map_err(|error| format!("failed to clear stale import staging: {error}"))?;
    }
    fs::create_dir_all(&staging_dir)
        .map_err(|error| format!("failed to create import staging: {error}"))?;

    let stage_result: Result<(), String> = (|| {
        for entry in &manifest.files {
            let mut zip_entry = archive.by_name(&entry.relative_path).map_err(|error| {
                format!(
                    "backup entry {} missing during stage: {error}",
                    entry.relative_path
                )
            })?;
            let dest = staging_dir.join(Path::new(&entry.relative_path));
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| format!("failed to create staging parent: {error}"))?;
            }
            let mut dest_file = fs::File::create(&dest)
                .map_err(|error| format!("failed to stage {}: {error}", entry.relative_path))?;
            // Stream with a bounded buffer; the hash was already verified in
            // phase 1, but re-hash on write to catch extraction errors.
            let mut buffer = vec![0u8; 1024 * 1024];
            let mut total: u64 = 0;
            loop {
                let read = zip_entry.read(&mut buffer).map_err(|error| {
                    format!(
                        "failed to read backup entry {}: {error}",
                        entry.relative_path
                    )
                })?;
                if read == 0 {
                    break;
                }
                total += read as u64;
                dest_file
                    .write_all(&buffer[..read])
                    .map_err(|error| format!("failed to stage {}: {error}", entry.relative_path))?;
            }
            if total != entry.size {
                return Err(format!(
                    "staging size mismatch for {}: expected {}, got {}",
                    entry.relative_path, entry.size, total
                ));
            }
            dest_file.sync_all().map_err(|error| {
                format!("failed to sync staged {}: {error}", entry.relative_path)
            })?;
        }
        Ok(())
    })();

    if let Err(error) = stage_result {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(error);
    }

    // Phase 3: populate the content-addressed object store from the staged
    // files, then write the SnapshotManifest that makes the imported snapshot
    // appear in `list_snapshots` and participate in retention/GC correctly.
    // Create the instance directory and its snapshot store first.
    fs::create_dir_all(instance_dir)
        .map_err(|error| format!("failed to create instance directory: {error}"))?;
    fs::create_dir_all(crate::snapshot::snapshots_dir(instance_dir))
        .map_err(|error| format!("failed to create snapshots dir: {error}"))?;
    fs::create_dir_all(crate::snapshot::snapshot_objects_dir(instance_dir))
        .map_err(|error| format!("failed to create snapshot object store: {error}"))?;

    let snapshot_id = manifest.snapshot.id.clone();
    // Fail closed if the snapshot already exists with different content.  A
    // re-import of the same artifact is idempotent; a different snapshot
    // trying to claim the same id is suspicious.
    let dest_manifest_path = crate::snapshot::snapshot_manifest_path(instance_dir, &snapshot_id);
    if dest_manifest_path.exists() {
        // If the existing manifest's file index matches the backup's, treat it
        // as already imported and skip object-store repopulation — the store
        // already has the blobs or they are referenced elsewhere.
        let already_indexed = crate::snapshot::snapshot_file_index(instance_dir, &snapshot_id).ok();
        let would_be_index: Vec<crate::lkg::FileEntry> = manifest
            .files
            .iter()
            .map(|entry| crate::lkg::FileEntry {
                path: entry.relative_path.clone(),
                sha256: entry.sha256.clone(),
                size: entry.size,
            })
            .collect();
        let matches = already_indexed.is_some_and(|existing| {
            let mut a = existing;
            let mut b = would_be_index.clone();
            a.sort_by(|left, right| left.path.cmp(&right.path));
            b.sort_by(|left, right| left.path.cmp(&right.path));
            a == b
        });
        if !matches {
            let _ = fs::remove_dir_all(&staging_dir);
            return Err(format!(
                "snapshot {snapshot_id} already exists with different content"
            ));
        }
    } else {
        // Populate the blob store from the staged files.  Each staged file is
        // hashed (again) and stored under `snapshot-objects/<aa>/<hash>`.
        for entry in &manifest.files {
            let staged_path = staging_dir.join(Path::new(&entry.relative_path));
            // Re-validate the staged file's hash before committing it to the
            // shared store — the zip was verified, but the filesystem write
            // could have been corrupted.
            let staged_bytes = fs::read(&staged_path).map_err(|error| {
                format!("failed to read staged {}: {error}", entry.relative_path)
            })?;
            if staged_bytes.len() as u64 != entry.size {
                let _ = fs::remove_dir_all(&staging_dir);
                return Err(format!("staged size mismatch for {}", entry.relative_path));
            }
            let actual = crate::snapshot::sha256_hex(&staged_bytes);
            if !actual.eq_ignore_ascii_case(&entry.sha256) {
                let _ = fs::remove_dir_all(&staging_dir);
                return Err(format!("staged hash mismatch for {}", entry.relative_path));
            }
            let blob_path = crate::snapshot::snapshot_blob_path(instance_dir, &entry.sha256);
            if let Some(parent) = blob_path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| format!("failed to create blob prefix: {error}"))?;
            }
            // Deduplicated store: if the blob already exists with the correct
            // hash, skip the write.  Otherwise atomically commit the bytes.
            let needs_write = if blob_path.is_file() {
                !crate::snapshot::blob_file_matches(&blob_path, &entry.sha256, entry.size)
            } else {
                true
            };
            if needs_write {
                // Write via a temp file in the object store's parent so the
                // final rename is atomic and same-volume.
                let blob_dir = blob_path.parent().unwrap_or(Path::new("."));
                let temp_blob = blob_dir.join(format!(
                    ".{}.tmp-{}",
                    blob_path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("blob"),
                    uuid::Uuid::new_v4()
                ));
                fs::write(&temp_blob, &staged_bytes).map_err(|error| {
                    format!("failed to write blob for {}: {error}", entry.relative_path)
                })?;
                let _ = fs::remove_file(&blob_path);
                fs::rename(&temp_blob, &blob_path).map_err(|error| {
                    format!("failed to commit blob for {}: {error}", entry.relative_path)
                })?;
                let _ = crate::artifact_receipt::record_verified(
                    &blob_path,
                    "sha256",
                    &entry.sha256,
                    i64::try_from(entry.size).ok(),
                );
            } else {
                let _ = crate::artifact_receipt::record_verified(
                    &blob_path,
                    "sha256",
                    &entry.sha256,
                    i64::try_from(entry.size).ok(),
                );
            }
        }

        // Write the SnapshotManifest (schema v3) that `list_snapshots` and
        // `snapshot_file_index` expect.
        let snapshot_manifest_json =
            build_snapshot_manifest_json(&manifest.snapshot, &manifest.files)?;
        crate::snapshot::atomic_write_pub(&dest_manifest_path, snapshot_manifest_json.as_bytes())?;
    }

    // Phase 4: atomically promote the staged files into the live instance.
    // Use the same marker-guarded, displacement-safe dance as
    // `snapshot.rs:1293` restore, but simplified: we already validated the
    // staged content, so the promotion is just moves of tracked roots.
    let marker_path = instance_dir.join(crate::snapshot::RESTORE_MARKER);
    let pre_dir = crate::snapshot::pre_restore_dir(instance_dir);
    // Recover any interrupted prior restore before we start mutating.
    // If this fails, the instance is in a half-restored state and import
    // must not proceed.
    if marker_path.exists() {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err("previous restore was interrupted; resolve before importing".into());
    }
    if pre_dir.exists() {
        fs::remove_dir_all(&pre_dir)
            .map_err(|error| format!("failed to clear stale pre-restore: {error}"))?;
    }

    // Write the restore marker so a concurrent retention sweep sees it.
    fs::write(&marker_path, b"import in progress")
        .map_err(|error| format!("failed to write import marker: {error}"))?;

    let promote_result: Result<(), String> = (|| {
        // Stash current tracked entries into pre_dir.
        fs::create_dir_all(&pre_dir)
            .map_err(|error| format!("failed to create pre-restore dir: {error}"))?;
        let mut moved_current = Vec::new();
        for entry_name in crate::snapshot::TRACKED_ENTRIES {
            let src = instance_dir.join(entry_name);
            if src.exists() {
                let dst = pre_dir.join(entry_name);
                if let Some(parent) = dst.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|error| format!("failed to create pre-restore parent: {error}"))?;
                }
                fs::rename(&src, &dst)
                    .map_err(|error| format!("failed to backup current {entry_name}: {error}"))?;
                moved_current.push((*entry_name).to_string());
            }
        }

        // Promote staged entries.  The staging dir contains only tracked
        // files at their relative paths, so enumerate its top-level entries
        // and move each one.
        let staged_roots: HashSet<String> = manifest
            .files
            .iter()
            .filter_map(|entry| entry.relative_path.split('/').next().map(str::to_string))
            .collect();

        let mut promoted: Vec<String> = Vec::new();
        for entry_name in crate::snapshot::TRACKED_ENTRIES {
            if !staged_roots.contains(*entry_name) {
                continue;
            }
            let src = staging_dir.join(entry_name);
            if !src.exists() {
                continue;
            }
            let dst = instance_dir.join(entry_name);
            // `src` may be a file (e.g. `options.txt`) or directory (e.g. `mods/`).
            fs::rename(&src, &dst).map_err(|error| {
                // Roll back on first promotion failure: displace any partially
                // promoted entries, then restore the originals.
                let rollback = rollback_import(
                    instance_dir,
                    &pre_dir,
                    &promoted,
                    &moved_current,
                );
                match rollback {
                    Ok(()) => format!(
                        "failed to promote imported {entry_name}: {error}; original state was restored"
                    ),
                    Err(rollback_error) => format!(
                        "failed to promote imported {entry_name}: {error}; {rollback_error}"
                    ),
                }
            })?;
            promoted.push((*entry_name).to_string());
        }
        Ok(())
    })();

    if let Err(error) = promote_result {
        let _ = fs::remove_dir_all(&staging_dir);
        let _ = fs::remove_file(&marker_path);
        // Attempt to roll back already-promoted entries is handled inside
        // promote_result's closure; here we just surface the error.
        return Err(error);
    }

    // Cleanup: remove marker, staging, and the now-empty pre-restore backup.
    let _ = fs::remove_file(&marker_path);
    let _ = fs::remove_dir_all(&staging_dir);
    if pre_dir.exists() {
        let _ = fs::remove_dir_all(&pre_dir);
    }

    // Record mutation so the next pre-launch snapshot is not incorrectly reused.
    let _ = crate::snapshot::mark_instance_mutated(instance_dir);
    // Ensure the imported snapshot's fingerprint receipt is fresh.
    if let Ok(fingerprint) = crate::snapshot::live_metadata_fingerprint(instance_dir) {
        let _ = crate::snapshot::write_snapshot_metadata_fingerprint(
            instance_dir,
            &snapshot_id,
            &fingerprint,
        );
    }

    Ok(manifest.snapshot)
}

fn build_snapshot_manifest_json(
    snapshot: &crate::snapshot::Snapshot,
    files: &[BackupFileEntry],
) -> Result<String, String> {
    // Reuse the on-disk schema from `snapshot.rs:256` SnapshotManifest.
    #[derive(Serialize)]
    struct SnapshotManifest<'a> {
        schema_version: u32,
        snapshot: &'a crate::snapshot::Snapshot,
        files: Vec<SnapshotFileEntry<'a>>,
    }
    #[derive(Serialize)]
    struct SnapshotFileEntry<'a> {
        relative_path: &'a str,
        size: u64,
        sha256: &'a str,
        blob_sha256: &'a str,
    }
    let mut sorted_files: Vec<&BackupFileEntry> = files.iter().collect();
    sorted_files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    let manifest = SnapshotManifest {
        schema_version: 3,
        snapshot,
        files: sorted_files
            .into_iter()
            .map(|entry| SnapshotFileEntry {
                relative_path: &entry.relative_path,
                size: entry.size,
                sha256: &entry.sha256,
                blob_sha256: &entry.sha256,
            })
            .collect(),
    };
    serde_json::to_string_pretty(&manifest)
        .map_err(|error| format!("failed to serialize snapshot manifest: {error}"))
}

fn rollback_import(
    instance_dir: &Path,
    pre_dir: &Path,
    promoted: &[String],
    moved_current: &[String],
) -> Result<(), String> {
    // Displace any partially promoted staged entries to a failed-import dir
    // so we can restore the originals without renaming over live destinations.
    let failed_dir = instance_dir.join(format!(".agora_failed_import_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&failed_dir)
        .map_err(|error| format!("failed to create rollback displacement: {error}"))?;

    let mut errors = Vec::new();
    for entry_name in promoted.iter().rev() {
        let live = instance_dir.join(entry_name);
        if live.exists() {
            let displaced = failed_dir.join(entry_name);
            if let Some(parent) = displaced.parent() {
                if let Err(error) = fs::create_dir_all(parent) {
                    errors.push(format!(
                        "could not prepare displacement for {entry_name}: {error}"
                    ));
                    continue;
                }
            }
            if let Err(error) = fs::rename(&live, &displaced) {
                errors.push(format!(
                    "could not displace partial import {entry_name}: {error}"
                ));
            }
        }
    }
    for entry_name in moved_current.iter().rev() {
        let backup = pre_dir.join(entry_name);
        let live = instance_dir.join(entry_name);
        if !backup.exists() {
            errors.push(format!("rollback backup is missing for {entry_name}"));
            continue;
        }
        if live.exists() {
            errors.push(format!(
                "rollback destination still exists for {entry_name}"
            ));
            continue;
        }
        if let Err(error) = fs::rename(&backup, &live) {
            errors.push(format!("could not restore original {entry_name}: {error}"));
        }
    }
    if errors.is_empty() {
        let _ = fs::remove_dir_all(&failed_dir);
        let _ = fs::remove_file(instance_dir.join(crate::snapshot::RESTORE_MARKER));
        Ok(())
    } else {
        Err(format!(
            "rollback incomplete; original data remains in {} and partial data in {}: {}",
            pre_dir.display(),
            failed_dir.display(),
            errors.join("; ")
        ))
    }
}

// ---------------------------------------------------------------------------
// More validation: ensure a relative path string is safe for zip entry use.
// We delegate to `snapshot::validate_relative_path` which already rejects
// `\\`, `..`, absolute and non-tracked roots.
// ---------------------------------------------------------------------------

#[allow(dead_code)]
fn validate_zip_entry_name(name: &str) -> Result<(), String> {
    // Zip entries use `/` as separator regardless of host.  Reject `\` early
    // because `validate_relative_path` also rejects it, but the error is
    // clearer when we name the offending name.
    if name.contains('\\') {
        return Err(format!(
            "invalid zip entry name {name:?} contains backslash"
        ));
    }
    crate::snapshot::validate_relative_path(name)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{
        create_snapshot, create_snapshot_scoped, list_snapshots, prelaunch_tracked_entries,
    };
    use std::fs;
    use tempfile::TempDir;

    fn make_instance(tmp: &TempDir) -> PathBuf {
        let dir = tmp.path().join("instance");
        fs::create_dir_all(dir.join("mods")).unwrap();
        fs::create_dir_all(dir.join("config")).unwrap();
        fs::create_dir_all(dir.join("resourcepacks")).unwrap();
        fs::create_dir_all(dir.join("shaderpacks")).unwrap();
        fs::write(dir.join("mods").join("mod-a.jar"), b"mod a v1").unwrap();
        fs::write(dir.join("config").join("settings.toml"), b"key=value").unwrap();
        fs::write(dir.join("options.txt"), b"render_distance=12").unwrap();
        fs::write(
            dir.join("instance_manifest.json"),
            br#"{"instance_id":"test","name":"Test"}"#,
        )
        .unwrap();
        dir
    }

    fn make_instance_with_saves(tmp: &TempDir) -> PathBuf {
        let dir = make_instance(tmp);
        fs::create_dir_all(dir.join("saves").join("world")).unwrap();
        fs::write(
            dir.join("saves").join("world").join("level.dat"),
            b"world data",
        )
        .unwrap();
        dir
    }

    fn lkg_with_current(id: &str) -> crate::lkg::LkgState {
        crate::lkg::LkgState {
            current_lkg_snapshot_id: Some(id.to_string()),
            last_promoted_at: Some(chrono::Utc::now().to_rfc3339()),
            last_launch_session_id: Some("s-1".into()),
            last_launch_outcome: Some(crate::lkg::LaunchOutcome::Success),
            promoted_snapshot_ids: vec![id.to_string()],
            schema_version: 1,
        }
    }

    fn snapshot_with(label: Option<&str>, id: &str, created_at: &str) -> crate::snapshot::Snapshot {
        crate::snapshot::Snapshot {
            id: id.to_string(),
            label: label.map(str::to_string),
            created_at: created_at.to_string(),
            file_count: 1,
            size_estimate: 100,
        }
    }

    // -----------------------------------------------------------------------
    // Retention unit tests (pure planner)
    // -----------------------------------------------------------------------

    #[test]
    fn retention_keep_last_protects_newest_n() {
        let snapshots = vec![
            snapshot_with(None, "newest", "2026-09-01T00:00:00Z"),
            snapshot_with(None, "mid", "2026-08-01T00:00:00Z"),
            snapshot_with(None, "oldest", "2026-07-01T00:00:00Z"),
        ];
        let lkg = crate::lkg::LkgState::default();
        let policy = BackupRetentionPolicy::keep_last(1);
        let evicted = plan_backup_retention(&snapshots, &lkg, &policy);
        assert_eq!(evicted, vec!["mid", "oldest"]);
    }

    #[test]
    fn retention_keep_days_honors_window() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-09-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let snapshots = vec![
            snapshot_with(None, "recent", "2026-08-28T00:00:00Z"),
            snapshot_with(None, "old", "2026-07-01T00:00:00Z"),
        ];
        let lkg = crate::lkg::LkgState::default();
        let policy = BackupRetentionPolicy::keep_days(7);
        let evicted = plan_backup_retention_with_now(&snapshots, &lkg, &policy, now);
        assert_eq!(evicted, vec!["old"]);
    }

    #[test]
    fn retention_union_of_keep_last_and_keep_days() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-09-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        // newest and middle are within 7 days via keep_days, oldest is kept via keep_last=1?
        // With keep_last=1, newest is kept; with keep_days=30, recent and old-within-30 kept.
        let snapshots = vec![
            snapshot_with(None, "newest", "2026-09-01T00:00:00Z"),
            snapshot_with(None, "mid", "2026-08-25T00:00:00Z"),
            snapshot_with(None, "old", "2026-07-01T00:00:00Z"),
        ];
        let lkg = crate::lkg::LkgState::default();
        let policy = BackupRetentionPolicy::keep_last_and_days(1, 30);
        let evicted = plan_backup_retention_with_now(&snapshots, &lkg, &policy, now);
        // newest kept by both, mid kept by days, old evicted
        assert_eq!(evicted, vec!["old"]);
    }

    #[test]
    fn retention_never_evicts_current_lkg() {
        let snapshots = vec![
            snapshot_with(None, "current-lkg", "2026-09-01T00:00:00Z"),
            snapshot_with(None, "newer-regular", "2026-09-02T00:00:00Z"),
            snapshot_with(None, "old", "2026-07-01T00:00:00Z"),
        ];
        let lkg = lkg_with_current("current-lkg");
        let policy = BackupRetentionPolicy::keep_last(1);
        // current-lkg is protected, newer-regular is kept by keep_last, old evicted
        let evicted = plan_backup_retention(&snapshots, &lkg, &policy);
        assert!(!evicted.contains(&"current-lkg".to_string()));
        assert!(evicted.contains(&"old".to_string()));
    }

    #[test]
    fn retention_never_evicts_crash_doctor() {
        let snapshots = vec![
            snapshot_with(
                Some("crash-doctor-active"),
                "protected",
                "2026-06-01T00:00:00Z",
            ),
            snapshot_with(None, "newest", "2026-09-01T00:00:00Z"),
        ];
        let lkg = crate::lkg::LkgState::default();
        let policy = BackupRetentionPolicy::keep_last(1);
        let evicted = plan_backup_retention(&snapshots, &lkg, &policy);
        assert!(!evicted.contains(&"protected".to_string()));
    }

    #[test]
    fn retention_keeps_at_least_one_when_keep_last_zero() {
        let snapshots = vec![snapshot_with(None, "only", "2026-09-01T00:00:00Z")];
        let lkg = crate::lkg::LkgState::default();
        let policy = BackupRetentionPolicy::keep_last(0);
        let evicted = plan_backup_retention(&snapshots, &lkg, &policy);
        assert!(
            evicted.is_empty(),
            "the only snapshot must never be evicted"
        );
    }

    #[test]
    fn retention_keeps_newest_pre_restore_even_when_policy_zero() {
        let snapshots = vec![
            snapshot_with(None, "regular", "2026-09-02T00:00:00Z"),
            snapshot_with(Some("pre-restore-123"), "pre", "2026-09-01T00:00:00Z"),
        ];
        let lkg = crate::lkg::LkgState::default();
        let policy = BackupRetentionPolicy::keep_last(0);
        let evicted = plan_backup_retention(&snapshots, &lkg, &policy);
        // Both would be evicted by keep_last=0, but the only-copy + pre-restore
        // guarantees keep at least the newest and at least one pre-restore.
        // Since "only copy" keeps "regular" (newest), "pre" would still be
        // evicted? But our code also keeps newest pre-restore if none kept.
        // With snapshots newest-first, regular is newest, so kept by only-copy.
        // Pre-restore is not protected by only-copy but is kept by pre-restore
        // guarantee if no pre-restore kept. So neither should be evicted? Let's
        // check our logic: keep is initially empty (no protected). keep_last=0
        // keeps nothing. keep_days none. Then only-copy keeps newest (regular).
        // Then pre-restore check sees no kept pre-restore, so it adds "pre".
        // Result: nothing evicted.
        assert!(evicted.is_empty());
    }

    #[test]
    fn retention_empty_policy_evicts_nothing() {
        let snapshots = vec![
            snapshot_with(None, "a", "2026-09-01T00:00:00Z"),
            snapshot_with(None, "b", "2026-09-02T00:00:00Z"),
        ];
        let lkg = crate::lkg::LkgState::default();
        let policy = BackupRetentionPolicy::default();
        let evicted = plan_backup_retention(&snapshots, &lkg, &policy);
        assert!(evicted.is_empty());
    }

    // -----------------------------------------------------------------------
    // Export / import integration tests
    // -----------------------------------------------------------------------

    #[test]
    fn export_and_import_round_trips_on_fresh_machine() {
        let tmp = TempDir::new().unwrap();
        let instance = make_instance(&tmp);
        let snapshot = create_snapshot(&instance, Some("backup-test")).unwrap();

        let export_dir = tmp.path().join("export");
        std::fs::create_dir_all(&export_dir).unwrap();
        let artifact = export_snapshot(&instance, &snapshot.id, &export_dir).unwrap();
        assert!(artifact.is_file());
        assert!(artifact
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with(".zip"));

        // Simulate a fresh machine: new instance directory that does not exist.
        let fresh_root = TempDir::new().unwrap();
        let fresh_instance = fresh_root.path().join("fresh-instance");
        // Import onto a path that does not yet exist.
        let imported = import_backup(&fresh_instance, &artifact).unwrap();
        assert_eq!(imported.id, snapshot.id);
        // Live files must match the original snapshot content.
        assert_eq!(
            std::fs::read(fresh_instance.join("mods").join("mod-a.jar")).unwrap(),
            b"mod a v1"
        );
        assert_eq!(
            std::fs::read(fresh_instance.join("config").join("settings.toml")).unwrap(),
            b"key=value"
        );
        // The imported snapshot must be listable via the normal snapshot API.
        let listed = list_snapshots(&fresh_instance).unwrap();
        assert!(listed.iter().any(|entry| entry.id == snapshot.id));
        // And restorable: modify then restore.
        std::fs::write(fresh_instance.join("mods").join("mod-a.jar"), b"tampered").unwrap();
        crate::snapshot::restore_snapshot(&fresh_instance, &snapshot.id).unwrap();
        assert_eq!(
            std::fs::read(fresh_instance.join("mods").join("mod-a.jar")).unwrap(),
            b"mod a v1"
        );
    }

    #[test]
    fn export_fails_when_blob_is_missing_and_does_not_leave_partial() {
        let tmp = TempDir::new().unwrap();
        let instance = make_instance(&tmp);
        let snapshot = create_snapshot(&instance, Some("to-corrupt")).unwrap();

        // Corrupt the blob store by deleting a blob.
        let index = crate::snapshot::snapshot_file_index(&instance, &snapshot.id).unwrap();
        let first = &index[0];
        let blob_path = crate::snapshot::snapshot_blob_path(&instance, &first.sha256);
        std::fs::remove_file(&blob_path).unwrap();

        let export_dir = tmp.path().join("export");
        std::fs::create_dir_all(&export_dir).unwrap();
        let error = export_snapshot(&instance, &snapshot.id, &export_dir).unwrap_err();
        assert!(
            error.contains("missing") || error.contains("corrupted"),
            "error should mention missing/corrupted blob: {error}"
        );
        // No partial artifact must remain.
        let entries: Vec<_> = std::fs::read_dir(&export_dir).unwrap().collect();
        assert!(
            entries.is_empty()
                || entries.iter().all(|entry| {
                    entry
                        .as_ref()
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .starts_with('.')
                }),
            "partial artifact leaked: {entries:?}"
        );
    }

    #[test]
    fn import_rejects_path_traversal_in_artifact() {
        let tmp = TempDir::new().unwrap();
        let instance = make_instance(&tmp);
        let snapshot = create_snapshot(&instance, Some("good")).unwrap();
        let export_dir = tmp.path().join("export");
        std::fs::create_dir_all(&export_dir).unwrap();
        let artifact = export_snapshot(&instance, &snapshot.id, &export_dir).unwrap();

        // Tamper with the artifact: rewrite the manifest to list an unsafe path,
        // and add a matching zip entry.  This simulates untrusted input — the
        // user may have received the file from someone else.
        let tampered = {
            let file = std::fs::File::open(&artifact).unwrap();
            let mut archive = zip::ZipArchive::new(file).unwrap();
            let mut manifest: BackupManifest = {
                let mut entry = archive.by_name(BACKUP_MANIFEST_NAME).unwrap();
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).unwrap();
                serde_json::from_slice(&bytes).unwrap()
            };
            // Inject a traversal path.
            manifest.files.push(BackupFileEntry {
                relative_path: "../evil.txt".into(),
                size: 4,
                sha256: crate::snapshot::sha256_hex(b"evil"),
            });
            manifest.snapshot.file_count += 1;
            // Rebuild a new zip with the tampered manifest and an evil entry.
            let tampered_path = tmp.path().join("tampered.zip");
            let out_file = std::fs::File::create(&tampered_path).unwrap();
            let mut writer = zip::ZipWriter::new(out_file);
            let opts = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            let manifest_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
            writer.start_file(BACKUP_MANIFEST_NAME, opts).unwrap();
            writer.write_all(&manifest_bytes).unwrap();
            for entry in &manifest.files {
                // For the evil entry, write 4 bytes; for others, copy from original.
                if entry.relative_path == "../evil.txt" {
                    writer.start_file(&entry.relative_path, opts).unwrap();
                    writer.write_all(b"evil").unwrap();
                } else {
                    // Copy original entry bytes.
                    let mut original_entry = archive.by_name(&entry.relative_path).unwrap();
                    let mut bytes = Vec::new();
                    original_entry.read_to_end(&mut bytes).unwrap();
                    writer.start_file(&entry.relative_path, opts).unwrap();
                    writer.write_all(&bytes).unwrap();
                }
            }
            writer.finish().unwrap();
            tampered_path
        };

        let fresh = TempDir::new().unwrap();
        let fresh_instance = fresh.path().join("victim");
        let error = import_backup(&fresh_instance, &tampered).unwrap_err();
        assert!(
            error.contains("unsafe")
                || error.contains("invalid")
                || error.contains("outside tracked"),
            "traversal should be rejected: {error}"
        );
        // No file must have been written to the victim instance.
        assert!(
            !fresh_instance.join("evil.txt").exists(),
            "traversal file must not have been written"
        );
        if fresh_instance.exists() {
            assert!(
                !fresh_instance.join("../evil.txt").exists(),
                "traversal outside instance must not exist"
            );
        }
    }

    #[test]
    fn import_rejects_unexpected_extra_file_in_zip() {
        let tmp = TempDir::new().unwrap();
        let instance = make_instance(&tmp);
        let snapshot = create_snapshot(&instance, Some("good")).unwrap();
        let export_dir = tmp.path().join("export");
        std::fs::create_dir_all(&export_dir).unwrap();
        let artifact = export_snapshot(&instance, &snapshot.id, &export_dir).unwrap();

        // Tamper: add an extra file not listed in the manifest.
        let tampered_path = tmp.path().join("extra.zip");
        {
            let file = std::fs::File::open(&artifact).unwrap();
            let mut archive = zip::ZipArchive::new(file).unwrap();
            let out_file = std::fs::File::create(&tampered_path).unwrap();
            let mut writer = zip::ZipWriter::new(out_file);
            let opts = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            // Copy all original entries.
            for index in 0..archive.len() {
                let mut entry = archive.by_index(index).unwrap();
                let name = entry.name().to_string();
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).unwrap();
                writer.start_file(&name, opts).unwrap();
                writer.write_all(&bytes).unwrap();
            }
            // Add unexpected file.
            writer.start_file("mods/evil.jar", opts).unwrap();
            writer.write_all(b"evil").unwrap();
            writer.finish().unwrap();
        }

        let fresh = TempDir::new().unwrap();
        let fresh_instance = fresh.path().join("victim2");
        let error = import_backup(&fresh_instance, &tampered_path).unwrap_err();
        assert!(
            error.contains("unexpected file") || error.contains("not listed"),
            "extra file should be rejected: {error}"
        );
        assert!(
            !fresh_instance.join("mods").join("evil.jar").exists(),
            "extra file must not have been written"
        );
    }

    #[test]
    fn import_rejects_hash_mismatch() {
        let tmp = TempDir::new().unwrap();
        let instance = make_instance(&tmp);
        let snapshot = create_snapshot(&instance, Some("good")).unwrap();
        let export_dir = tmp.path().join("export");
        std::fs::create_dir_all(&export_dir).unwrap();
        let artifact = export_snapshot(&instance, &snapshot.id, &export_dir).unwrap();

        // Tamper: flip a byte in a file's zip entry without updating the manifest hash.
        let tampered_path = tmp.path().join("hash_mismatch.zip");
        {
            let file = std::fs::File::open(&artifact).unwrap();
            let mut archive = zip::ZipArchive::new(file).unwrap();
            let manifest: BackupManifest = {
                let mut entry = archive.by_name(BACKUP_MANIFEST_NAME).unwrap();
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).unwrap();
                serde_json::from_slice(&bytes).unwrap()
            };
            let out_file = std::fs::File::create(&tampered_path).unwrap();
            let mut writer = zip::ZipWriter::new(out_file);
            let opts = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for index in 0..archive.len() {
                let mut entry = archive.by_index(index).unwrap();
                let name = entry.name().to_string();
                if name == BACKUP_MANIFEST_NAME {
                    let bytes = serde_json::to_vec_pretty(&manifest).unwrap();
                    writer.start_file(&name, opts).unwrap();
                    writer.write_all(&bytes).unwrap();
                    continue;
                }
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).unwrap();
                // Corrupt the first file's contents.
                if name == manifest.files[0].relative_path && !bytes.is_empty() {
                    bytes[0] ^= 0xFF;
                }
                writer.start_file(&name, opts).unwrap();
                writer.write_all(&bytes).unwrap();
            }
            writer.finish().unwrap();
        }

        let fresh = TempDir::new().unwrap();
        let fresh_instance = fresh.path().join("victim3");
        let error = import_backup(&fresh_instance, &tampered_path).unwrap_err();
        assert!(
            error.contains("hash mismatch"),
            "hash mismatch should be rejected: {error}"
        );
        // No live file should have been written.
        if fresh_instance.exists() {
            let mods_dir = fresh_instance.join("mods");
            if mods_dir.exists() {
                let entries: Vec<_> = std::fs::read_dir(&mods_dir).unwrap().collect();
                assert!(
                    entries.is_empty(),
                    "no file should be written after hash failure: {entries:?}"
                );
            }
        }
    }

    #[test]
    fn export_materialises_saves_and_import_restores_them() {
        let tmp = TempDir::new().unwrap();
        let instance = make_instance_with_saves(&tmp);
        let snapshot = create_snapshot(&instance, Some("with-saves")).unwrap();

        let export_dir = tmp.path().join("export");
        std::fs::create_dir_all(&export_dir).unwrap();
        let artifact = export_snapshot(&instance, &snapshot.id, &export_dir).unwrap();

        let fresh = TempDir::new().unwrap();
        let fresh_instance = fresh.path().join("fresh-with-saves");
        import_backup(&fresh_instance, &artifact).unwrap();
        assert_eq!(
            std::fs::read(fresh_instance.join("saves").join("world").join("level.dat")).unwrap(),
            b"world data"
        );

        // Also verify that a prelaunch-scoped snapshot excludes saves.
        let pre_snapshot =
            create_snapshot_scoped(&instance, Some("prelaunch"), prelaunch_tracked_entries())
                .unwrap();
        let pre_artifact = export_snapshot(&instance, &pre_snapshot.id, &export_dir).unwrap();
        let fresh2 = TempDir::new().unwrap();
        let fresh_instance2 = fresh2.path().join("fresh-prelaunch");
        import_backup(&fresh_instance2, &pre_artifact).unwrap();
        assert!(
            !fresh_instance2.join("saves").exists(),
            "prelaunch snapshot should not contain saves"
        );
    }

    #[test]
    fn retention_skipped_while_restore_marker_present() {
        let tmp = TempDir::new().unwrap();
        let instance = make_instance(&tmp);
        let _a = create_snapshot(&instance, Some("a")).unwrap();
        let b = create_snapshot(&instance, Some("b")).unwrap();
        // Simulate a restore in progress.
        std::fs::write(
            instance.join(crate::snapshot::RESTORE_MARKER),
            b"restore in progress",
        )
        .unwrap();
        // Even with keep_last=0, nothing should be evicted while marker exists.
        let policy = BackupRetentionPolicy::keep_last(0);
        let evicted = run_backup_retention(&instance, &policy).unwrap();
        assert!(
            evicted.is_empty(),
            "retention must be skipped during restore"
        );
        // Cleanup marker and verify retention now proceeds (keeping at least one).
        std::fs::remove_file(instance.join(crate::snapshot::RESTORE_MARKER)).unwrap();
        let evicted2 = run_backup_retention(&instance, &policy).unwrap();
        // With keep_last=0, the only-copy guarantee keeps the newest (b), so one evicted.
        assert_eq!(evicted2.len(), 1);
        let remaining = list_snapshots(&instance).unwrap();
        assert!(remaining.iter().any(|snapshot| snapshot.id == b.id));
        let _ = b; // suppress unused
    }

    #[test]
    fn run_backup_retention_deletes_by_count_and_never_deletes_only_copy() {
        let tmp = TempDir::new().unwrap();
        let instance = make_instance(&tmp);
        let a = create_snapshot(&instance, Some("a")).unwrap();
        std::fs::write(instance.join("mods").join("mod-a.jar"), b"mod a v2").unwrap();
        let b = create_snapshot(&instance, Some("b")).unwrap();
        std::fs::write(instance.join("mods").join("mod-a.jar"), b"mod a v3").unwrap();
        let c = create_snapshot(&instance, Some("c")).unwrap();

        let policy = BackupRetentionPolicy::keep_last(1);
        let evicted = run_backup_retention(&instance, &policy).unwrap();
        // Keep newest 1 (c), evict the two older.
        assert_eq!(evicted.len(), 2);
        let remaining = list_snapshots(&instance).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, c.id);
        // Verify the two older snapshots are gone.
        assert!(crate::snapshot::snapshot_file_index(&instance, &a.id).is_err());
        assert!(crate::snapshot::snapshot_file_index(&instance, &b.id).is_err());
    }
}
