use std::collections::HashSet;
use std::fs;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use sha2::Digest;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt;
const RESTORE_MARKER: &str = ".agora_restore_in_progress";
const SNAPSHOT_PENDING_MARKER: &str = ".agora_snapshot_pending";
const SNAPSHOT_FAILED_MARKER: &str = ".agora_snapshot_failed";
const SNAPSHOT_SCHEMA_VERSION: u32 = 3;
const LIVE_METADATA_FINGERPRINT_SCHEMA_VERSION: u32 = 3;
const LIVE_FILE_INDEX_SCHEMA_VERSION: u32 = 1;

const TRACKED_ENTRIES: &[&str] = &[
    "mods",
    "config",
    "resourcepacks",
    "shaderpacks",
    "datapacks",
    "saves",
    "options.txt",
    "instance_manifest.json",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: String,
    pub label: Option<String>,
    pub created_at: String,
    pub file_count: usize,
    pub size_estimate: u64,
}

/// Whether an instance's initial recovery snapshot is usable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SnapshotReadiness {
    Ready,
    Pending,
    Failed,
}

impl SnapshotReadiness {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Pending => "pending",
            Self::Failed => "failed",
        }
    }
}

pub fn snapshot_readiness(instance_dir: &Path) -> SnapshotReadiness {
    if instance_dir.join(SNAPSHOT_PENDING_MARKER).is_file() {
        let owner = fs::read_to_string(instance_dir.join(SNAPSHOT_PENDING_MARKER)).ok();
        let current_process = std::process::id().to_string();
        if owner.as_deref() == Some(current_process.as_str()) {
            SnapshotReadiness::Pending
        } else {
            // A previous process died while the background task was running.
            // Do not leave the instance permanently stuck in a pending state.
            SnapshotReadiness::Failed
        }
    } else if instance_dir.join(SNAPSHOT_FAILED_MARKER).is_file() {
        SnapshotReadiness::Failed
    } else {
        SnapshotReadiness::Ready
    }
}

pub fn snapshot_readiness_error(instance_dir: &Path) -> Option<String> {
    if snapshot_readiness(instance_dir) == SnapshotReadiness::Failed
        && instance_dir.join(SNAPSHOT_FAILED_MARKER).is_file()
    {
        return fs::read_to_string(instance_dir.join(SNAPSHOT_FAILED_MARKER))
            .ok()
            .filter(|message| !message.trim().is_empty());
    }
    if snapshot_readiness(instance_dir) == SnapshotReadiness::Failed
        && instance_dir.join(SNAPSHOT_PENDING_MARKER).is_file()
    {
        return Some("The snapshot worker stopped before the recovery point was finalized.".into());
    }
    None
}

pub fn mark_snapshot_pending(instance_dir: &Path) -> Result<(), String> {
    fs::remove_file(instance_dir.join(SNAPSHOT_FAILED_MARKER)).ok();
    fs::write(
        instance_dir.join(SNAPSHOT_PENDING_MARKER),
        std::process::id().to_string(),
    )
    .map_err(|e| format!("failed to mark snapshot as pending: {e}"))
}

pub fn mark_snapshot_ready(instance_dir: &Path) -> Result<(), String> {
    fs::remove_file(instance_dir.join(SNAPSHOT_PENDING_MARKER)).ok();
    fs::remove_file(instance_dir.join(SNAPSHOT_FAILED_MARKER)).ok();
    Ok(())
}

pub fn mark_snapshot_failed(instance_dir: &Path, error: &str) -> Result<(), String> {
    fs::remove_file(instance_dir.join(SNAPSHOT_PENDING_MARKER)).ok();
    fs::write(instance_dir.join(SNAPSHOT_FAILED_MARKER), error.as_bytes())
        .map_err(|e| format!("failed to record snapshot failure: {e}"))
}

#[derive(Serialize, Deserialize)]
struct SnapshotManifest {
    #[serde(default = "legacy_snapshot_schema_version")]
    schema_version: u32,
    snapshot: Snapshot,
    files: Vec<SnapshotFileEntry>,
}

#[derive(Serialize, Deserialize)]
struct SnapshotFileEntry {
    relative_path: String,
    size: u64,
    /// Snapshots written before schema v2 did not record hashes.  Keep the
    /// field optional so those recovery archives remain listable/restorable;
    /// restore computes the missing hash from the archive before mutation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
    /// Schema v3 stores file contents in the shared content-addressed object
    /// store instead of duplicating them in every snapshot archive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    blob_sha256: Option<String>,
}

fn legacy_snapshot_schema_version() -> u32 {
    1
}

fn snapshots_dir(instance_dir: &Path) -> PathBuf {
    instance_dir.join(".agora_snapshots")
}

fn snapshot_manifest_path(instance_dir: &Path, id: &str) -> PathBuf {
    snapshots_dir(instance_dir).join(format!("{id}.json"))
}

fn snapshot_fingerprint_path(instance_dir: &Path, id: &str) -> PathBuf {
    snapshots_dir(instance_dir).join(format!("{id}.fingerprint"))
}

fn live_file_index_cache_path(instance_dir: &Path) -> PathBuf {
    snapshots_dir(instance_dir).join("live-file-index.json")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LiveFileIndexCache {
    schema_version: u32,
    entries: Vec<LiveFileIndexEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LiveFileIndexEntry {
    relative_path: String,
    size: u64,
    modified_ns: u128,
    file_identity: Option<String>,
    sha256: String,
}

/// The object store is shared by instances belonging to the same app data
/// root. Tests and callers that pass a standalone directory still get a
/// sibling object store, while normal instances resolve to `<app>/snapshot-objects`.
fn snapshot_objects_dir(instance_dir: &Path) -> PathBuf {
    let instances_root = instance_dir.parent().unwrap_or(instance_dir);
    let app_root = if instances_root
        .file_name()
        .is_some_and(|name| name == "instances")
    {
        instances_root.parent().unwrap_or(instances_root)
    } else {
        instances_root
    };
    app_root.join("snapshot-objects")
}

fn snapshot_blob_path(instance_dir: &Path, hash: &str) -> PathBuf {
    snapshot_objects_dir(instance_dir)
        .join(&hash[..2.min(hash.len())])
        .join(hash)
}

/// Legacy v1/v2 snapshots are retained as ZIPs and remain readable.
fn snapshot_zip_path(instance_dir: &Path, id: &str) -> PathBuf {
    snapshots_dir(instance_dir).join(format!("{id}.zip"))
}

fn pre_restore_dir(instance_dir: &Path) -> PathBuf {
    instance_dir.join(".agora_pre_restore")
}

/// Return the file index stored in a snapshot. Modern immutable manifests use
/// their recorded hashes without rereading every object blob; restore still
/// verifies object bytes before mutation. Legacy entries with no recorded hash
/// are hashed from their archive bytes. This is also the
/// canonical input for LKG/drift comparisons, ensuring both sides use the same
/// `mods/foo.jar` path format and the same tracked-entry set.
pub fn snapshot_file_index(
    instance_dir: &Path,
    snapshot_id: &str,
) -> Result<Vec<crate::lkg::FileEntry>, String> {
    validate_snapshot_id(snapshot_id)?;
    let manifest_path = snapshot_manifest_path(instance_dir, snapshot_id);
    if manifest_path.is_file() {
        let manifest = read_manifest_file(&manifest_path, snapshot_id)?;
        let mut result = Vec::with_capacity(manifest.files.len());
        let mut seen = HashSet::new();
        for indexed in &manifest.files {
            validate_relative_path(&indexed.relative_path)?;
            if !seen.insert(indexed.relative_path.clone()) {
                return Err(format!(
                    "snapshot manifest contains duplicate path {}",
                    indexed.relative_path
                ));
            }
            let blob_hash = indexed
                .blob_sha256
                .as_deref()
                .or(indexed.sha256.as_deref())
                .ok_or_else(|| {
                    format!(
                        "snapshot manifest has no content hash for {}",
                        indexed.relative_path
                    )
                })?;
            validate_blob_hash(blob_hash)?;
            result.push(crate::lkg::FileEntry {
                path: indexed.relative_path.clone(),
                sha256: indexed
                    .sha256
                    .clone()
                    .unwrap_or_else(|| blob_hash.to_string()),
                size: indexed.size,
            });
        }
        result.sort_by(|a, b| a.path.cmp(&b.path));
        return Ok(result);
    }

    let zip_path = snapshot_zip_path(instance_dir, snapshot_id);
    let file = fs::File::open(&zip_path)
        .map_err(|e| format!("failed to open snapshot {snapshot_id}: {e}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("failed to read snapshot zip: {e}"))?;
    let manifest = read_manifest(&mut archive, snapshot_id)?;

    let mut result = Vec::with_capacity(manifest.files.len());
    let mut seen = HashSet::new();
    for indexed in &manifest.files {
        validate_relative_path(&indexed.relative_path)?;
        if !seen.insert(indexed.relative_path.clone()) {
            return Err(format!(
                "snapshot manifest contains duplicate path {}",
                indexed.relative_path
            ));
        }
        let mut entry = archive.by_name(&indexed.relative_path).map_err(|e| {
            format!(
                "snapshot file {} is missing from archive: {e}",
                indexed.relative_path
            )
        })?;
        let mut contents = Vec::new();
        entry
            .read_to_end(&mut contents)
            .map_err(|e| format!("failed to read {}: {e}", indexed.relative_path))?;
        if contents.len() as u64 != indexed.size {
            return Err(format!(
                "snapshot size mismatch for {}",
                indexed.relative_path
            ));
        }
        let actual = sha256_hex(&contents);
        if let Some(expected) = &indexed.sha256 {
            if !actual.eq_ignore_ascii_case(expected) {
                return Err(format!(
                    "snapshot hash mismatch for {}",
                    indexed.relative_path
                ));
            }
        }
        result.push(crate::lkg::FileEntry {
            path: indexed.relative_path.clone(),
            sha256: actual,
            size: indexed.size,
        });
    }
    result.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(result)
}

/// Scan the current live state using the exact same tracked paths as snapshot
/// creation.  Paths always use forward slashes, even on Windows.
pub fn live_file_index(instance_dir: &Path) -> Result<Vec<crate::lkg::FileEntry>, String> {
    let mut result = Vec::new();
    for entry_name in TRACKED_ENTRIES {
        let path = instance_dir.join(entry_name);
        if path.is_file() {
            let contents =
                fs::read(&path).map_err(|e| format!("failed to read live {entry_name}: {e}"))?;
            result.push(crate::lkg::FileEntry {
                path: (*entry_name).to_string(),
                sha256: sha256_hex(&contents),
                size: contents.len() as u64,
            });
        } else if path.is_dir() {
            walk_and_hash(&path, entry_name, &mut result)?;
        }
    }
    result.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(result)
}

/// Build a metadata-only identity for the tracked live state.  This is used
/// only to decide whether the last exact snapshot can be reused; whenever the
/// receipt is absent or differs, callers fall back to [`live_file_index`].
pub fn live_metadata_fingerprint(instance_dir: &Path) -> Result<String, String> {
    let mut hasher = sha2::Sha256::new();
    hasher.update(b"agora-snapshot-metadata-v1");
    for entry_name in TRACKED_ENTRIES {
        let path = instance_dir.join(entry_name);
        if path.is_file() {
            append_metadata_record(&mut hasher, entry_name, &path)?;
        } else if path.is_dir() {
            append_metadata_record(&mut hasher, entry_name, &path)?;
            walk_metadata(&mut hasher, &path, entry_name)?;
        } else {
            hasher.update(format!("missing:{entry_name}").as_bytes());
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Compare the current live state with a snapshot while reusing the durable
/// per-file index. Only files whose size, modification time, or platform file
/// identity changed are read and hashed. This is the launch-side comparison;
/// callers that explicitly request a full drift audit should continue using
/// live_file_index.
pub fn snapshot_matches_live_incremental(
    instance_dir: &Path,
    snapshot_id: &str,
) -> Result<bool, String> {
    validate_snapshot_id(snapshot_id)?;
    let reference = snapshot_file_index(instance_dir, snapshot_id)?;
    let reference = reference
        .into_iter()
        .map(|entry| (entry.path, (entry.sha256, entry.size)))
        .collect::<std::collections::BTreeMap<_, _>>();
    let previous = read_live_file_index_cache(instance_dir).map(|entries| {
        entries
            .into_iter()
            .map(|entry| (entry.relative_path.clone(), entry))
            .collect::<std::collections::HashMap<_, _>>()
    });
    let current = collect_live_file_metadata(instance_dir)?;
    let mut refreshed = Vec::with_capacity(current.len());
    let mut matches = current.len() == reference.len();

    for mut entry in current {
        let cached = previous.as_ref().and_then(|cache| {
            cache.get(&entry.relative_path).filter(|cached| {
                cached.size == entry.size
                    && cached.modified_ns == entry.modified_ns
                    && cached.file_identity == entry.file_identity
            })
        });
        let (hash, blob_valid) = if let Some(cached) = cached {
            (
                cached.sha256.clone(),
                snapshot_blob_is_available(instance_dir, &cached.sha256, cached.size),
            )
        } else {
            (hash_live_file(instance_dir, &entry.relative_path)?, true)
        };
        entry.sha256 = hash.clone();
        if !blob_valid {
            matches = false;
        }
        if !reference
            .get(&entry.relative_path)
            .is_some_and(|(expected_hash, expected_size)| {
                *expected_size == entry.size
                    && expected_hash.eq_ignore_ascii_case(&hash)
                    && snapshot_blob_is_available(instance_dir, expected_hash, *expected_size)
            })
        {
            matches = false;
        }
        refreshed.push(entry);
    }

    if matches {
        let refreshed_by_path = refreshed
            .iter()
            .map(|entry| {
                (
                    entry.relative_path.as_str(),
                    (entry.size, entry.sha256.as_str()),
                )
            })
            .collect::<std::collections::HashMap<_, _>>();
        for (path, (hash, size)) in &reference {
            let Some((entry_size, entry_hash)) = refreshed_by_path.get(path.as_str()) else {
                matches = false;
                break;
            };
            if *entry_size != *size || !entry_hash.eq_ignore_ascii_case(hash) {
                matches = false;
                break;
            }
        }
    }
    let _ = write_live_file_index_cache(instance_dir, refreshed);
    Ok(matches)
}

fn read_live_file_index_cache(instance_dir: &Path) -> Option<Vec<LiveFileIndexEntry>> {
    let bytes = fs::read(live_file_index_cache_path(instance_dir)).ok()?;
    let cache = serde_json::from_slice::<LiveFileIndexCache>(&bytes).ok()?;
    (cache.schema_version == LIVE_FILE_INDEX_SCHEMA_VERSION).then_some(cache.entries)
}

fn write_live_file_index_cache(
    instance_dir: &Path,
    entries: Vec<LiveFileIndexEntry>,
) -> Result<(), String> {
    let cache = LiveFileIndexCache {
        schema_version: LIVE_FILE_INDEX_SCHEMA_VERSION,
        entries,
    };
    let bytes = serde_json::to_vec(&cache)
        .map_err(|error| format!("failed to serialize live file index: {error}"))?;
    atomic_write(&live_file_index_cache_path(instance_dir), &bytes)
}

fn collect_live_file_metadata(instance_dir: &Path) -> Result<Vec<LiveFileIndexEntry>, String> {
    let mut result = Vec::new();
    for entry_name in TRACKED_ENTRIES {
        let path = instance_dir.join(entry_name);
        if path.is_file() {
            result.push(live_file_metadata_entry(entry_name, &path)?);
        } else if path.is_dir() {
            walk_file_metadata(&path, entry_name, &mut result)?;
        }
    }
    result.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(result)
}

fn walk_file_metadata(
    directory: &Path,
    prefix: &str,
    result: &mut Vec<LiveFileIndexEntry>,
) -> Result<(), String> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| format!("failed to scan {}: {error}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to scan {}: {error}", directory.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let relative = format!("{prefix}/{}", entry.file_name().to_string_lossy());
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
        if file_type.is_dir() {
            walk_file_metadata(&path, &relative, result)?;
        } else if file_type.is_file() {
            result.push(live_file_metadata_entry(&relative, &path)?);
        }
    }
    Ok(())
}

fn live_file_metadata_entry(
    relative_path: &str,
    path: &Path,
) -> Result<LiveFileIndexEntry, String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("failed to inspect live {}: {error}", path.display()))?;
    let modified_ns = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    Ok(LiveFileIndexEntry {
        relative_path: relative_path.to_string(),
        size: metadata.len(),
        modified_ns,
        file_identity: metadata_file_identity(&metadata),
        sha256: String::new(),
    })
}

#[cfg(unix)]
fn metadata_file_identity(metadata: &fs::Metadata) -> Option<String> {
    Some(format!("unix:{}:{}", metadata.dev(), metadata.ino()))
}

#[cfg(windows)]
fn metadata_file_identity(metadata: &fs::Metadata) -> Option<String> {
    Some(format!(
        "windows:creation-time:{}",
        metadata.creation_time()
    ))
}

#[cfg(not(any(unix, windows)))]
fn metadata_file_identity(_metadata: &fs::Metadata) -> Option<String> {
    None
}

fn hash_live_file(instance_dir: &Path, relative_path: &str) -> Result<String, String> {
    let path = instance_dir.join(Path::new(relative_path));
    let contents =
        fs::read(&path).map_err(|error| format!("failed to read live {relative_path}: {error}"))?;
    Ok(sha256_hex(&contents))
}

fn snapshot_blob_is_available(instance_dir: &Path, hash: &str, size: u64) -> bool {
    if validate_blob_hash(hash).is_err() {
        return false;
    }
    let path = snapshot_blob_path(instance_dir, hash);
    if crate::artifact_receipt::is_verified(&path, "sha256", hash, i64::try_from(size).ok()) {
        return true;
    }
    if !blob_file_matches(&path, hash, size) {
        return false;
    }
    let _ =
        crate::artifact_receipt::record_verified(&path, "sha256", hash, i64::try_from(size).ok());
    true
}

/// Return the metadata receipt associated with a snapshot, if it is valid.
pub fn read_snapshot_metadata_fingerprint(
    instance_dir: &Path,
    snapshot_id: &str,
) -> Option<String> {
    if validate_snapshot_id(snapshot_id).is_err() {
        return None;
    }
    let bytes = fs::read(snapshot_fingerprint_path(instance_dir, snapshot_id)).ok()?;
    let receipt: SnapshotMetadataFingerprint = serde_json::from_slice(&bytes).ok()?;
    let manifest_bytes = fs::read(snapshot_manifest_path(instance_dir, snapshot_id)).ok()?;
    let manifest_sha256 = sha256_hex(&manifest_bytes);
    if receipt.schema_version == LIVE_METADATA_FINGERPRINT_SCHEMA_VERSION
        && receipt.snapshot_id == snapshot_id
        && receipt.snapshot_manifest_sha256 == manifest_sha256
    {
        Some(receipt.fingerprint)
    } else {
        None
    }
}

/// Persist the metadata identity for a snapshot.  Failure is reported to the
/// caller, but launch code may safely treat it as a cache miss on the next run.
pub fn write_snapshot_metadata_fingerprint(
    instance_dir: &Path,
    snapshot_id: &str,
    fingerprint: &str,
) -> Result<(), String> {
    validate_snapshot_id(snapshot_id)?;
    let manifest_bytes = fs::read(snapshot_manifest_path(instance_dir, snapshot_id))
        .map_err(|error| format!("failed to read snapshot manifest for fingerprint: {error}"))?;
    let receipt = SnapshotMetadataFingerprint {
        schema_version: LIVE_METADATA_FINGERPRINT_SCHEMA_VERSION,
        snapshot_id: snapshot_id.to_string(),
        fingerprint: fingerprint.to_string(),
        snapshot_manifest_sha256: sha256_hex(&manifest_bytes),
    };
    let bytes = serde_json::to_vec(&receipt)
        .map_err(|error| format!("failed to serialize snapshot fingerprint: {error}"))?;
    atomic_write(
        &snapshot_fingerprint_path(instance_dir, snapshot_id),
        &bytes,
    )
}

#[derive(Debug, Serialize, Deserialize)]
struct SnapshotMetadataFingerprint {
    schema_version: u32,
    snapshot_id: String,
    fingerprint: String,
    snapshot_manifest_sha256: String,
}

fn walk_metadata(hasher: &mut sha2::Sha256, directory: &Path, prefix: &str) -> Result<(), String> {
    let mut entries = fs::read_dir(directory)
        .map_err(|e| format!("failed to scan {}: {e}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("failed to scan {}: {e}", directory.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let relative = format!("{prefix}/{}", entry.file_name().to_string_lossy());
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|e| format!("failed to inspect {}: {e}", path.display()))?;
        if file_type.is_dir() {
            append_metadata_record(hasher, &relative, &path)?;
            walk_metadata(hasher, &path, &relative)?;
        } else if file_type.is_file() {
            append_metadata_record(hasher, &relative, &path)?;
        }
    }
    Ok(())
}

fn append_metadata_record(
    hasher: &mut sha2::Sha256,
    relative: &str,
    path: &Path,
) -> Result<(), String> {
    let metadata = fs::metadata(path)
        .map_err(|e| format!("failed to inspect live metadata {}: {e}", path.display()))?;
    let modified_ns = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    hasher.update((relative.len() as u64).to_le_bytes());
    hasher.update(relative.as_bytes());
    hasher.update([if metadata.is_dir() { 1 } else { 2 }]);
    hasher.update(metadata.len().to_le_bytes());
    hasher.update(modified_ns.to_le_bytes());
    #[cfg(unix)]
    {
        hasher.update(metadata.dev().to_le_bytes());
        hasher.update(metadata.ino().to_le_bytes());
    }
    #[cfg(windows)]
    hasher.update(metadata.creation_time().to_le_bytes());
    Ok(())
}

fn walk_and_hash(
    directory: &Path,
    prefix: &str,
    result: &mut Vec<crate::lkg::FileEntry>,
) -> Result<(), String> {
    let mut entries = fs::read_dir(directory)
        .map_err(|e| format!("failed to scan {}: {e}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("failed to scan {}: {e}", directory.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().to_string();
        let relative = format!("{prefix}/{name}");
        let file_type = entry
            .file_type()
            .map_err(|e| format!("failed to inspect {}: {e}", entry.path().display()))?;
        if file_type.is_dir() {
            walk_and_hash(&entry.path(), &relative, result)?;
        } else if file_type.is_file() {
            let contents = fs::read(entry.path())
                .map_err(|e| format!("failed to read live {relative}: {e}"))?;
            result.push(crate::lkg::FileEntry {
                path: relative,
                sha256: sha256_hex(&contents),
                size: contents.len() as u64,
            });
        }
    }
    Ok(())
}

/// Create a snapshot of an instance directory.
///
/// New snapshots consist of a small immutable manifest under
/// `<instance_dir>/.agora_snapshots/<id>.json`. File contents are streamed once
/// into SHA-256 named objects in the shared snapshot object store. This avoids
/// Deflate CPU cost and lets unchanged files be reused by later snapshots.
/// Legacy ZIP snapshots are still understood by restore and listing.
pub fn create_snapshot(instance_dir: &Path, label: Option<&str>) -> Result<Snapshot, String> {
    let id = uuid::Uuid::new_v4().to_string();

    fs::create_dir_all(snapshots_dir(instance_dir))
        .map_err(|e| format!("failed to create snapshots dir: {e}"))?;
    fs::create_dir_all(snapshot_objects_dir(instance_dir))
        .map_err(|e| format!("failed to create snapshot object store: {e}"))?;

    let previous = read_live_file_index_cache(instance_dir)
        .unwrap_or_default()
        .into_iter()
        .map(|entry| (entry.relative_path.clone(), entry))
        .collect::<std::collections::BTreeMap<_, _>>();
    let current = collect_live_file_metadata(instance_dir)?;
    let mut files: Vec<SnapshotFileEntry> = Vec::with_capacity(current.len());
    let mut live_index = Vec::with_capacity(current.len());
    let mut total_size: u64 = 0;

    for mut entry in current {
        let cached = previous.get(&entry.relative_path);
        let reused = cached
            .filter(|cached| {
                cached.size == entry.size
                    && cached.modified_ns == entry.modified_ns
                    && cached.file_identity == entry.file_identity
                    && snapshot_blob_is_available(instance_dir, &cached.sha256, cached.size)
            })
            .map(|cached| (cached.sha256.clone(), cached.size));
        let (sha256, size) = if let Some(reused) = reused {
            reused
        } else {
            let source = instance_dir.join(Path::new(&entry.relative_path));
            store_snapshot_object(instance_dir, &source)?
        };
        entry.sha256 = sha256.clone();
        files.push(SnapshotFileEntry {
            relative_path: entry.relative_path.clone(),
            size,
            sha256: Some(sha256.clone()),
            blob_sha256: Some(sha256),
        });
        live_index.push(entry);
        total_size += size;
    }

    let snapshot = Snapshot {
        id: id.clone(),
        label: label.map(String::from),
        created_at: chrono::Utc::now().to_rfc3339(),
        file_count: files.len(),
        size_estimate: total_size,
    };

    let manifest = SnapshotManifest {
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        snapshot: snapshot.clone(),
        files,
    };

    let manifest_json = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| format!("failed to serialize manifest: {e}"))?;
    atomic_write(&snapshot_manifest_path(instance_dir, &id), &manifest_json)?;
    let _ = write_live_file_index_cache(instance_dir, live_index);
    mark_snapshot_ready(instance_dir)?;
    Ok(snapshot)
}

fn store_snapshot_object(instance_dir: &Path, source: &Path) -> Result<(String, u64), String> {
    let objects_dir = snapshot_objects_dir(instance_dir);
    fs::create_dir_all(&objects_dir)
        .map_err(|e| format!("failed to create snapshot object store: {e}"))?;
    let temp_path = objects_dir.join(format!(
        ".{}.{}.tmp",
        uuid::Uuid::new_v4(),
        std::process::id()
    ));
    let source_file =
        fs::File::open(source).map_err(|e| format!("failed to read {}: {e}", source.display()))?;
    let mut reader = BufReader::new(source_file);
    let temp_file = fs::File::create(&temp_path)
        .map_err(|e| format!("failed to create snapshot object: {e}"))?;
    let mut writer = BufWriter::new(temp_file);
    let mut hasher = sha2::Sha256::new();
    let mut size = 0u64;
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|e| format!("failed to read {}: {e}", source.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        writer
            .write_all(&buffer[..read])
            .map_err(|e| format!("failed to write snapshot object: {e}"))?;
        size += read as u64;
    }
    let hash = format!("{:x}", hasher.finalize());
    writer
        .flush()
        .map_err(|e| format!("failed to flush snapshot object: {e}"))?;
    writer
        .get_ref()
        .sync_all()
        .map_err(|e| format!("failed to sync snapshot object: {e}"))?;

    let destination = snapshot_blob_path(instance_dir, &hash);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create snapshot object prefix: {e}"))?;
    }
    if destination.exists() {
        let already_verified = crate::artifact_receipt::is_verified(
            &destination,
            "sha256",
            &hash,
            i64::try_from(size).ok(),
        );
        if already_verified || blob_file_matches(&destination, &hash, size) {
            fs::remove_file(&temp_path)
                .map_err(|e| format!("failed to discard duplicate snapshot object: {e}"))?;
            if !already_verified {
                let _ = crate::artifact_receipt::record_verified(
                    &destination,
                    "sha256",
                    &hash,
                    i64::try_from(size).ok(),
                );
            }
        } else {
            fs::remove_file(&destination)
                .map_err(|e| format!("failed to replace corrupt snapshot object: {e}"))?;
            fs::rename(&temp_path, &destination)
                .map_err(|e| format!("failed to replace snapshot object: {e}"))?;
        }
    } else if let Err(error) = fs::rename(&temp_path, &destination) {
        if destination.exists() {
            fs::remove_file(&temp_path).map_err(|cleanup| {
                format!("failed to discard duplicate snapshot object: {cleanup}")
            })?;
        } else {
            return Err(format!("failed to commit snapshot object: {error}"));
        }
    }
    let _ = crate::artifact_receipt::record_verified(
        &destination,
        "sha256",
        &hash,
        i64::try_from(size).ok(),
    );
    Ok((hash, size))
}

fn blob_file_matches(path: &Path, expected_hash: &str, expected_size: u64) -> bool {
    let Ok(file) = fs::File::open(path) else {
        return false;
    };
    let Ok(metadata) = file.metadata() else {
        return false;
    };
    if metadata.len() != expected_size {
        return false;
    }
    let mut reader = BufReader::new(file);
    let mut hasher = sha2::Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let Ok(read) = reader.read(&mut buffer) else {
            return false;
        };
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    format!("{:x}", hasher.finalize()).eq_ignore_ascii_case(expected_hash)
}

fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), String> {
    let temp = path.with_extension(format!(
        "{}.tmp",
        path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("snapshot")
    ));
    let mut file =
        fs::File::create(&temp).map_err(|e| format!("failed to create snapshot manifest: {e}"))?;
    file.write_all(contents)
        .map_err(|e| format!("failed to write snapshot manifest: {e}"))?;
    file.sync_all()
        .map_err(|e| format!("failed to sync snapshot manifest: {e}"))?;
    fs::rename(&temp, path).map_err(|e| format!("failed to commit snapshot manifest: {e}"))
}

/// Restore an instance to a snapshot.
///
/// The archive is completely extracted and hash-verified before any live file
/// is moved.  Tracked top-level entries are then exchanged with same-volume
/// renames.  If any exchange fails, every partially promoted snapshot entry is
/// displaced before the pre-restore entries are moved back, so rollback never
/// depends on renaming over a non-empty destination.
pub fn restore_snapshot(instance_dir: &Path, snapshot_id: &str) -> Result<(), String> {
    restore_snapshot_impl(instance_dir, snapshot_id, None)
}

fn restore_snapshot_impl(
    instance_dir: &Path,
    snapshot_id: &str,
    fail_after_promotions: Option<usize>,
) -> Result<(), String> {
    validate_snapshot_id(snapshot_id)?;
    recover_interrupted_restore(instance_dir)?;
    let manifest_path = snapshot_manifest_path(instance_dir, snapshot_id);
    let zip_path = snapshot_zip_path(instance_dir, snapshot_id);
    if !manifest_path.exists() && !zip_path.exists() {
        return Err(format!("snapshot {snapshot_id} not found"));
    }

    let restore_id = uuid::Uuid::new_v4().to_string();
    let extract_dir = instance_dir.join(format!(".agora_restore_extract_{restore_id}"));
    fs::create_dir_all(&extract_dir).map_err(|e| format!("failed to create extract dir: {e}"))?;

    let manifest = match extract_and_verify(instance_dir, snapshot_id, &extract_dir) {
        Ok(manifest) => manifest,
        Err(error) => {
            let _ = fs::remove_dir_all(&extract_dir);
            return Err(error);
        }
    };

    let pre_dir = pre_restore_dir(instance_dir);
    if pre_dir.exists() {
        fs::remove_dir_all(&pre_dir)
            .map_err(|e| format!("failed to remove pre-restore dir: {e}"))?;
    }
    fs::create_dir_all(&pre_dir).map_err(|e| format!("failed to create pre-restore dir: {e}"))?;

    let marker_path = instance_dir.join(RESTORE_MARKER);
    fs::write(&marker_path, b"restore in progress")
        .map_err(|e| format!("failed to write restore marker: {e}"))?;

    let mut moved_current = Vec::new();
    for entry_name in TRACKED_ENTRIES {
        let src = instance_dir.join(entry_name);
        if src.exists() {
            let dst = pre_dir.join(entry_name);
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("failed to create parent: {e}"))?;
            }
            if let Err(error) = fs::rename(&src, &dst) {
                let rollback =
                    rollback_restore(instance_dir, &pre_dir, &[], &moved_current, &restore_id);
                let _ = fs::remove_dir_all(&extract_dir);
                return Err(combine_restore_error(
                    format!("failed to move current {entry_name} into backup: {error}"),
                    rollback,
                ));
            }
            moved_current.push((*entry_name).to_string());
        }
    }

    let staged_roots = snapshot_roots(&manifest);
    let mut promoted = Vec::new();
    for entry_name in TRACKED_ENTRIES {
        if !staged_roots.contains(*entry_name) {
            continue;
        }

        let src = extract_dir.join(entry_name);
        let dst = instance_dir.join(entry_name);
        let promote_result = if fail_after_promotions == Some(promoted.len()) {
            Err("injected restore promotion failure".to_string())
        } else {
            fs::rename(&src, &dst).map_err(|e| e.to_string())
        };
        if let Err(error) = promote_result {
            let rollback = rollback_restore(
                instance_dir,
                &pre_dir,
                &promoted,
                &moved_current,
                &restore_id,
            );
            let _ = fs::remove_dir_all(&extract_dir);
            return Err(combine_restore_error(
                format!("failed to promote restored {entry_name}: {error}"),
                rollback,
            ));
        }
        promoted.push((*entry_name).to_string());
    }

    if marker_path.exists() {
        fs::remove_file(&marker_path)
            .map_err(|e| format!("failed to remove restore marker: {e}"))?;
    }

    if pre_dir.exists() {
        fs::remove_dir_all(&pre_dir)
            .map_err(|e| format!("restore succeeded but backup cleanup failed: {e}"))?;
    }

    let _ = fs::remove_dir_all(&extract_dir);

    Ok(())
}

/// Complete rollback from a process interruption before starting any new
/// restore. Only roots with an actual backup are displaced, so roots that had
/// not yet moved when the process stopped remain untouched.
fn recover_interrupted_restore(instance_dir: &Path) -> Result<(), String> {
    let marker = instance_dir.join(RESTORE_MARKER);
    let pre_dir = pre_restore_dir(instance_dir);
    if !marker.exists() {
        if pre_dir.exists() {
            fs::remove_dir_all(&pre_dir)
                .map_err(|e| format!("failed to remove stale restore backup: {e}"))?;
        }
        return Ok(());
    }
    if !pre_dir.is_dir() {
        return Err(
            "Previous restore was interrupted without a recovery backup; live state was left untouched."
                .into(),
        );
    }
    let backed_up = TRACKED_ENTRIES
        .iter()
        .filter(|entry| pre_dir.join(entry).exists())
        .map(|entry| (*entry).to_string())
        .collect::<Vec<_>>();
    if backed_up.is_empty() {
        fs::remove_file(&marker)
            .map_err(|e| format!("failed to clear empty restore marker: {e}"))?;
        fs::remove_dir_all(&pre_dir)
            .map_err(|e| format!("failed to clear empty restore backup: {e}"))?;
        return Ok(());
    }
    rollback_restore(
        instance_dir,
        &pre_dir,
        &backed_up,
        &backed_up,
        &format!("interrupted-{}", uuid::Uuid::new_v4()),
    )?;
    if pre_dir.exists() {
        fs::remove_dir_all(&pre_dir)
            .map_err(|e| format!("failed to clean recovered restore backup: {e}"))?;
    }
    Ok(())
}

fn extract_and_verify(
    instance_dir: &Path,
    snapshot_id: &str,
    extract_dir: &Path,
) -> Result<SnapshotManifest, String> {
    let manifest_path = snapshot_manifest_path(instance_dir, snapshot_id);
    if manifest_path.is_file() {
        let manifest = read_manifest_file(&manifest_path, snapshot_id)?;
        let mut seen = HashSet::new();
        for file_entry in &manifest.files {
            validate_relative_path(&file_entry.relative_path)?;
            if !seen.insert(file_entry.relative_path.clone()) {
                return Err(format!(
                    "snapshot manifest contains duplicate path {}",
                    file_entry.relative_path
                ));
            }
            let blob_hash = file_entry
                .blob_sha256
                .as_deref()
                .or(file_entry.sha256.as_deref())
                .ok_or_else(|| {
                    format!(
                        "snapshot manifest has no content hash for {}",
                        file_entry.relative_path
                    )
                })?;
            validate_blob_hash(blob_hash)?;
            let blob_path = snapshot_blob_path(instance_dir, blob_hash);
            let input = fs::File::open(&blob_path).map_err(|e| {
                format!(
                    "failed to open snapshot object for {}: {e}",
                    file_entry.relative_path
                )
            })?;
            let mut reader = BufReader::new(input);
            let output = extract_dir.join(Path::new(&file_entry.relative_path));
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("failed to create restore staging directory: {e}"))?;
            }
            let output_file = fs::File::create(&output)
                .map_err(|e| format!("failed to stage {}: {e}", file_entry.relative_path))?;
            let mut writer = BufWriter::new(output_file);
            let mut hasher = sha2::Sha256::new();
            let mut size = 0u64;
            let mut buffer = vec![0u8; 1024 * 1024];
            loop {
                let read = reader
                    .read(&mut buffer)
                    .map_err(|e| format!("failed to read snapshot object: {e}"))?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
                writer
                    .write_all(&buffer[..read])
                    .map_err(|e| format!("failed to stage {}: {e}", file_entry.relative_path))?;
                size += read as u64;
            }
            writer
                .flush()
                .map_err(|e| format!("failed to flush {}: {e}", file_entry.relative_path))?;
            writer
                .get_ref()
                .sync_all()
                .map_err(|e| format!("failed to sync {}: {e}", file_entry.relative_path))?;
            let actual_hash = format!("{:x}", hasher.finalize());
            if size != file_entry.size {
                return Err(format!(
                    "snapshot size mismatch for {}",
                    file_entry.relative_path
                ));
            }
            if !actual_hash.eq_ignore_ascii_case(blob_hash) {
                return Err(format!(
                    "snapshot hash mismatch for {}: object is corrupted or modified",
                    file_entry.relative_path
                ));
            }
            if let Some(expected) = &file_entry.sha256 {
                if !actual_hash.eq_ignore_ascii_case(expected) {
                    return Err(format!(
                        "snapshot hash mismatch for {}: object is corrupted or modified",
                        file_entry.relative_path
                    ));
                }
            }
        }
        return Ok(manifest);
    }

    let zip_path = snapshot_zip_path(instance_dir, snapshot_id);
    let file = fs::File::open(zip_path).map_err(|e| format!("failed to open snapshot zip: {e}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("failed to read snapshot zip: {e}"))?;

    let manifest = read_manifest(&mut archive, snapshot_id)?;

    let mut seen = HashSet::new();
    for file_entry in &manifest.files {
        validate_relative_path(&file_entry.relative_path)?;
        if !seen.insert(file_entry.relative_path.clone()) {
            return Err(format!(
                "snapshot manifest contains duplicate path {}",
                file_entry.relative_path
            ));
        }

        let mut entry = archive.by_name(&file_entry.relative_path).map_err(|e| {
            format!(
                "snapshot file {} is missing from archive: {e}",
                file_entry.relative_path
            )
        })?;
        if entry.is_dir() {
            return Err(format!(
                "snapshot path {} is a directory, expected a file",
                file_entry.relative_path
            ));
        }

        let mut contents = Vec::new();
        entry
            .read_to_end(&mut contents)
            .map_err(|e| format!("failed to read {}: {e}", file_entry.relative_path))?;
        if contents.len() as u64 != file_entry.size {
            return Err(format!(
                "snapshot size mismatch for {}: expected {}, got {}",
                file_entry.relative_path,
                file_entry.size,
                contents.len()
            ));
        }

        let actual_hash = sha256_hex(&contents);
        if let Some(expected_hash) = &file_entry.sha256 {
            if !actual_hash.eq_ignore_ascii_case(expected_hash) {
                return Err(format!(
                    "snapshot hash mismatch for {}: archive is corrupted or modified",
                    file_entry.relative_path
                ));
            }
        }

        let output = extract_dir.join(Path::new(&file_entry.relative_path));
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create restore staging directory: {e}"))?;
        }
        let mut output_file = fs::File::create(&output)
            .map_err(|e| format!("failed to stage {}: {e}", file_entry.relative_path))?;
        output_file
            .write_all(&contents)
            .map_err(|e| format!("failed to stage {}: {e}", file_entry.relative_path))?;
        output_file
            .sync_all()
            .map_err(|e| format!("failed to sync {}: {e}", file_entry.relative_path))?;
    }

    Ok(manifest)
}

fn read_manifest_file(path: &Path, snapshot_id: &str) -> Result<SnapshotManifest, String> {
    let content =
        fs::read_to_string(path).map_err(|e| format!("failed to read snapshot manifest: {e}"))?;
    let manifest: SnapshotManifest = serde_json::from_str(&content)
        .map_err(|e| format!("failed to parse snapshot manifest: {e}"))?;
    validate_manifest(&manifest, snapshot_id)?;
    Ok(manifest)
}

fn validate_manifest(manifest: &SnapshotManifest, snapshot_id: &str) -> Result<(), String> {
    if manifest.schema_version == 0 || manifest.schema_version > SNAPSHOT_SCHEMA_VERSION {
        return Err(format!(
            "unsupported snapshot schema version {} (maximum supported is {})",
            manifest.schema_version, SNAPSHOT_SCHEMA_VERSION
        ));
    }
    if manifest.snapshot.id != snapshot_id {
        return Err(format!(
            "snapshot identity mismatch: requested {snapshot_id}, archive contains {}",
            manifest.snapshot.id
        ));
    }
    Ok(())
}

fn validate_blob_hash(hash: &str) -> Result<(), String> {
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("snapshot contains an invalid content hash".into());
    }
    Ok(())
}

fn read_manifest(
    archive: &mut zip::ZipArchive<fs::File>,
    snapshot_id: &str,
) -> Result<SnapshotManifest, String> {
    let manifest: SnapshotManifest = {
        let mut entry = archive
            .by_name("manifest.json")
            .map_err(|e| format!("snapshot manifest is missing: {e}"))?;
        let mut content = String::new();
        entry
            .read_to_string(&mut content)
            .map_err(|e| format!("failed to read snapshot manifest: {e}"))?;
        serde_json::from_str(&content)
            .map_err(|e| format!("failed to parse snapshot manifest: {e}"))?
    };

    validate_manifest(&manifest, snapshot_id)?;
    Ok(manifest)
}

fn validate_relative_path(relative_path: &str) -> Result<(), String> {
    if relative_path.is_empty() || relative_path.contains('\\') {
        return Err(format!("invalid snapshot path {relative_path:?}"));
    }
    let path = Path::new(relative_path);
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(format!("unsafe snapshot path {relative_path:?}"));
    }
    let root = path
        .components()
        .next()
        .and_then(|part| match part {
            Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .ok_or_else(|| format!("invalid snapshot path {relative_path:?}"))?;
    if !TRACKED_ENTRIES.contains(&root) {
        return Err(format!(
            "snapshot path is outside tracked entries: {relative_path}"
        ));
    }
    if TRACKED_ENTRIES
        .iter()
        .any(|file| *file == root && Path::new(file).extension().is_some())
        && path.components().count() != 1
    {
        return Err(format!(
            "snapshot file path cannot contain children: {relative_path}"
        ));
    }
    Ok(())
}

fn validate_snapshot_id(snapshot_id: &str) -> Result<(), String> {
    if snapshot_id.is_empty()
        || snapshot_id == "."
        || snapshot_id == ".."
        || snapshot_id.contains('/')
        || snapshot_id.contains('\\')
    {
        return Err("invalid snapshot id".into());
    }
    Ok(())
}

fn snapshot_roots(manifest: &SnapshotManifest) -> HashSet<&str> {
    manifest
        .files
        .iter()
        .filter_map(|entry| entry.relative_path.split('/').next())
        .collect()
}

fn sha256_hex(contents: &[u8]) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(contents);
    format!("{:x}", hasher.finalize())
}

/// Reverse a failed restore without renaming over live destinations.  Any
/// partially promoted snapshot entries are first moved aside.  If rollback
/// itself fails, both the backup and displaced paths are retained and named in
/// the returned error so recovery never silently loses the protected state.
fn rollback_restore(
    instance_dir: &Path,
    pre_dir: &Path,
    promoted: &[String],
    moved_current: &[String],
    restore_id: &str,
) -> Result<(), String> {
    let failed_dir = instance_dir.join(format!(".agora_failed_restore_{restore_id}"));
    fs::create_dir_all(&failed_dir)
        .map_err(|e| format!("failed to create rollback displacement directory: {e}"))?;

    let mut errors = Vec::new();
    for entry_name in promoted.iter().rev() {
        let live = instance_dir.join(entry_name);
        if live.exists() {
            let displaced = failed_dir.join(entry_name);
            if let Some(parent) = displaced.parent() {
                if let Err(e) = fs::create_dir_all(parent) {
                    errors.push(format!(
                        "could not prepare displacement for {entry_name}: {e}"
                    ));
                    continue;
                }
            }
            if let Err(e) = fs::rename(&live, &displaced) {
                errors.push(format!(
                    "could not displace partial restore {entry_name}: {e}"
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
        if let Err(e) = fs::rename(&backup, &live) {
            errors.push(format!("could not restore original {entry_name}: {e}"));
        }
    }

    if errors.is_empty() {
        let _ = fs::remove_dir_all(&failed_dir);
        let marker = instance_dir.join(RESTORE_MARKER);
        let _ = fs::remove_file(marker);
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

fn combine_restore_error(primary: String, rollback: Result<(), String>) -> String {
    match rollback {
        Ok(()) => format!("{primary}; original instance state was restored"),
        Err(rollback_error) => format!("{primary}; {rollback_error}"),
    }
}

/// List all snapshots for an instance.
pub fn list_snapshots(instance_dir: &Path) -> Result<Vec<Snapshot>, String> {
    let marker = instance_dir.join(RESTORE_MARKER);
    if marker.exists() {
        return Err(
            "Previous restore was interrupted. Check .agora_pre_restore/ for backed-up files."
                .into(),
        );
    }

    let dir = snapshots_dir(instance_dir);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut snapshots = Vec::new();

    let entries = fs::read_dir(&dir).map_err(|e| format!("failed to read snapshots dir: {e}"))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("failed to read entry: {e}"))?;
        let path = entry.path();
        match path.extension().and_then(|e| e.to_str()) {
            Some("json") => {
                let Some(id) = path.file_stem().and_then(|name| name.to_str()) else {
                    continue;
                };
                if let Ok(manifest) = read_manifest_file(&path, id) {
                    snapshots.push(manifest.snapshot);
                }
            }
            Some("zip") => {
                let Some(id) = path.file_stem().and_then(|name| name.to_str()) else {
                    continue;
                };
                let file = match fs::File::open(&path) {
                    Ok(f) => f,
                    Err(_) => continue,
                };
                let mut archive = match zip::ZipArchive::new(file) {
                    Ok(a) => a,
                    Err(_) => continue,
                };
                if let Ok(manifest) = read_manifest(&mut archive, id) {
                    snapshots.push(manifest.snapshot);
                }
            }
            _ => continue,
        }
    }

    snapshots.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(snapshots)
}

/// Delete a snapshot.
pub fn delete_snapshot(instance_dir: &Path, snapshot_id: &str) -> Result<(), String> {
    validate_snapshot_id(snapshot_id)?;
    let manifest_path = snapshot_manifest_path(instance_dir, snapshot_id);
    let zip_path = snapshot_zip_path(instance_dir, snapshot_id);
    if !manifest_path.exists() && !zip_path.exists() {
        return Err(format!("snapshot {snapshot_id} not found"));
    }

    let mut blobs = Vec::new();
    if manifest_path.is_file() {
        if let Ok(manifest) = read_manifest_file(&manifest_path, snapshot_id) {
            blobs = manifest
                .files
                .iter()
                .filter_map(|file| file.blob_sha256.as_deref().or(file.sha256.as_deref()))
                .filter(|hash| validate_blob_hash(hash).is_ok())
                .map(str::to_string)
                .collect();
        }
        fs::remove_file(&manifest_path)
            .map_err(|e| format!("failed to delete snapshot manifest: {e}"))?;
    } else {
        fs::remove_file(&zip_path).map_err(|e| format!("failed to delete snapshot zip: {e}"))?;
    }
    let _ = fs::remove_file(snapshot_fingerprint_path(instance_dir, snapshot_id));

    for hash in blobs {
        if !blob_is_referenced_by_any_snapshot(instance_dir, &hash) {
            let path = snapshot_blob_path(instance_dir, &hash);
            let _ = fs::remove_file(&path);
            if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
                let _ = fs::remove_file(path.with_file_name(format!("{name}.agora-verified.json")));
            }
            if let Some(parent) = path.parent() {
                let _ = fs::remove_dir(parent);
            }
        }
    }

    let dir = snapshots_dir(instance_dir);
    if dir.exists()
        && dir
            .read_dir()
            .map(|mut d| d.next().is_none())
            .unwrap_or(false)
    {
        let _ = fs::remove_dir(&dir);
    }

    Ok(())
}

fn blob_is_referenced_by_any_snapshot(instance_dir: &Path, hash: &str) -> bool {
    if validate_blob_hash(hash).is_err() {
        return false;
    }
    let instances_root = instance_dir.parent().unwrap_or(instance_dir);
    let candidates = fs::read_dir(instances_root)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .map(|entry| entry.path().join(".agora_snapshots"))
        .chain(std::iter::once(snapshots_dir(instance_dir)));

    candidates
        .filter_map(|dir| fs::read_dir(dir).ok())
        .flatten()
        .any(|entry| {
            let Ok(entry) = entry else {
                return false;
            };
            if entry.path().extension().and_then(|e| e.to_str()) != Some("json") {
                return false;
            }
            let path = entry.path();
            let Some(id) = path.file_stem().and_then(|name| name.to_str()) else {
                return false;
            };
            let Ok(manifest) = read_manifest_file(&path, id) else {
                return false;
            };
            manifest
                .files
                .iter()
                .any(|file| file.blob_sha256.as_deref().or(file.sha256.as_deref()) == Some(hash))
        })
}

/// Return the physical storage estimate used by retention for a snapshot.
/// Shared objects may be counted by more than one snapshot, which is
/// intentionally conservative for a size-cap policy.
pub fn snapshot_storage_size(instance_dir: &Path, snapshot_id: &str) -> u64 {
    if validate_snapshot_id(snapshot_id).is_err() {
        return 0;
    }
    let manifest_path = snapshot_manifest_path(instance_dir, snapshot_id);
    if let Ok(manifest) = read_manifest_file(&manifest_path, snapshot_id) {
        return fs::metadata(&manifest_path).map(|m| m.len()).unwrap_or(0)
            + manifest
                .files
                .iter()
                .filter_map(|file| file.blob_sha256.as_deref().or(file.sha256.as_deref()))
                .filter(|hash| validate_blob_hash(hash).is_ok())
                .map(|hash| {
                    fs::metadata(snapshot_blob_path(instance_dir, hash))
                        .map(|m| m.len())
                        .unwrap_or(0)
                })
                .sum::<u64>();
    }
    fs::metadata(snapshot_zip_path(instance_dir, snapshot_id))
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

/// Remove content-addressed objects that are no longer referenced by any
/// instance snapshot manifest. Garbage collection is best-effort at lifecycle
/// boundaries; restore correctness never depends on it.
pub fn prune_unreferenced_objects(instance_dir: &Path) -> Result<(), String> {
    let root = snapshot_objects_dir(instance_dir);
    if !root.is_dir() {
        return Ok(());
    }
    let prefixes =
        fs::read_dir(&root).map_err(|e| format!("failed to scan snapshot object store: {e}"))?;
    for prefix in prefixes {
        let prefix = prefix.map_err(|e| format!("failed to read snapshot object prefix: {e}"))?;
        if !prefix.path().is_dir() {
            continue;
        }
        for entry in fs::read_dir(prefix.path())
            .map_err(|e| format!("failed to scan snapshot objects: {e}"))?
        {
            let entry = entry.map_err(|e| format!("failed to read snapshot object: {e}"))?;
            let Some(hash) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if validate_blob_hash(&hash).is_ok()
                && !blob_is_referenced_by_any_snapshot(instance_dir, &hash)
            {
                let _ = fs::remove_file(entry.path());
            }
        }
        let _ = fs::remove_dir(prefix.path());
    }
    let _ = fs::remove_dir(root);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use zip::write::FileOptions;
    use zip::CompressionMethod;

    fn make_instance(tmp: &TempDir) -> PathBuf {
        let dir = tmp.path().join("instance");
        fs::create_dir_all(dir.join("mods")).unwrap();
        fs::create_dir_all(dir.join("config")).unwrap();
        fs::create_dir_all(dir.join("resourcepacks")).unwrap();
        fs::create_dir_all(dir.join("shaderpacks")).unwrap();
        fs::write(dir.join("mods").join("test.jar"), b"mod content").unwrap();
        fs::write(dir.join("config").join("settings.toml"), b"key=value").unwrap();
        fs::write(dir.join("options.txt"), b"render_distance=12").unwrap();
        dir
    }

    fn count_snapshot_objects(instance_dir: &Path) -> usize {
        fs::read_dir(snapshot_objects_dir(instance_dir))
            .unwrap()
            .filter_map(Result::ok)
            .map(|prefix| fs::read_dir(prefix.path()).unwrap().count())
            .sum()
    }

    #[test]
    fn create_and_list_snapshot() {
        let tmp = TempDir::new().unwrap();
        let inst = make_instance(&tmp);

        let snap = create_snapshot(&inst, Some("before-update")).unwrap();
        assert_eq!(snap.label.as_deref(), Some("before-update"));
        assert!(snap.file_count > 0);
        assert!(snap.size_estimate > 0);

        let snaps = list_snapshots(&inst).unwrap();
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].id, snap.id);
    }

    #[test]
    fn snapshots_reuse_content_addressed_objects() {
        let tmp = TempDir::new().unwrap();
        let inst = make_instance(&tmp);

        let first = create_snapshot(&inst, Some("first")).unwrap();
        let object_count = count_snapshot_objects(&inst);
        let second = create_snapshot(&inst, Some("second")).unwrap();

        assert_ne!(first.id, second.id);
        assert_eq!(count_snapshot_objects(&inst), object_count);
        assert_eq!(
            snapshot_file_index(&inst, &first.id).unwrap(),
            snapshot_file_index(&inst, &second.id).unwrap()
        );
    }

    #[test]
    fn incremental_snapshot_reuses_unchanged_file_hashes() {
        let tmp = TempDir::new().unwrap();
        let inst = make_instance(&tmp);

        let first = create_snapshot(&inst, Some("first")).unwrap();
        let first_index = snapshot_file_index(&inst, &first.id).unwrap();
        fs::write(inst.join("options.txt"), b"render_distance=20").unwrap();

        assert!(!snapshot_matches_live_incremental(&inst, &first.id).unwrap());
        let second = create_snapshot(&inst, Some("second")).unwrap();
        let second_index = snapshot_file_index(&inst, &second.id).unwrap();
        let first_mod = first_index
            .iter()
            .find(|entry| entry.path == "mods/test.jar")
            .unwrap();
        let second_mod = second_index
            .iter()
            .find(|entry| entry.path == "mods/test.jar")
            .unwrap();
        assert_eq!(first_mod, second_mod);
        assert!(snapshot_matches_live_incremental(&inst, &second.id).unwrap());
    }

    #[test]
    fn snapshot_fingerprint_is_bound_to_its_manifest() {
        let tmp = TempDir::new().unwrap();
        let inst = make_instance(&tmp);
        let snapshot = create_snapshot(&inst, Some("bound")).unwrap();
        let fingerprint = live_metadata_fingerprint(&inst).unwrap();
        write_snapshot_metadata_fingerprint(&inst, &snapshot.id, &fingerprint).unwrap();

        assert_eq!(
            read_snapshot_metadata_fingerprint(&inst, &snapshot.id).as_deref(),
            Some(fingerprint.as_str())
        );

        let manifest_path = snapshot_manifest_path(&inst, &snapshot.id);
        let mut manifest = fs::read(&manifest_path).unwrap();
        manifest.push(b'\n');
        fs::write(manifest_path, manifest).unwrap();
        assert!(read_snapshot_metadata_fingerprint(&inst, &snapshot.id).is_none());
    }

    #[test]
    fn snapshot_readiness_markers_are_transitional() {
        let tmp = TempDir::new().unwrap();
        let inst = make_instance(&tmp);

        assert_eq!(snapshot_readiness(&inst), SnapshotReadiness::Ready);
        mark_snapshot_pending(&inst).unwrap();
        assert_eq!(snapshot_readiness(&inst), SnapshotReadiness::Pending);
        mark_snapshot_failed(&inst, "test failure").unwrap();
        assert_eq!(snapshot_readiness(&inst), SnapshotReadiness::Failed);
        assert_eq!(
            snapshot_readiness_error(&inst).as_deref(),
            Some("test failure")
        );
        mark_snapshot_ready(&inst).unwrap();
        assert_eq!(snapshot_readiness(&inst), SnapshotReadiness::Ready);
    }

    #[test]
    fn restore_snapshot_preserves_content() {
        let tmp = TempDir::new().unwrap();
        let inst = make_instance(&tmp);

        let snap = create_snapshot(&inst, None).unwrap();

        fs::write(inst.join("mods").join("test.jar"), b"modified").unwrap();
        fs::write(inst.join("options.txt"), b"modified").unwrap();

        restore_snapshot(&inst, &snap.id).unwrap();

        assert_eq!(
            fs::read(inst.join("mods").join("test.jar")).unwrap(),
            b"mod content"
        );
        assert_eq!(
            fs::read(inst.join("options.txt")).unwrap(),
            b"render_distance=12"
        );

        assert!(!inst.join(".agora_pre_restore").exists());
    }

    #[test]
    fn snapshot_is_immutable() {
        let tmp = TempDir::new().unwrap();
        let inst = make_instance(&tmp);

        let snap = create_snapshot(&inst, None).unwrap();

        fs::write(inst.join("mods").join("test.jar"), b"changed").unwrap();

        let manifest =
            read_manifest_file(&snapshot_manifest_path(&inst, &snap.id), &snap.id).unwrap();
        let hash = manifest
            .files
            .iter()
            .find(|file| file.relative_path == "mods/test.jar")
            .unwrap()
            .blob_sha256
            .as_deref()
            .unwrap();
        let content = fs::read(snapshot_blob_path(&inst, hash)).unwrap();
        assert_eq!(content, b"mod content");
    }

    #[test]
    fn delete_snapshot_removes_zip() {
        let tmp = TempDir::new().unwrap();
        let inst = make_instance(&tmp);

        let snap = create_snapshot(&inst, None).unwrap();
        let manifest_path = snapshot_manifest_path(&inst, &snap.id);
        assert!(manifest_path.exists());

        delete_snapshot(&inst, &snap.id).unwrap();
        assert!(!manifest_path.exists());
        assert_eq!(count_snapshot_objects(&inst), 0);
    }

    #[test]
    fn list_snapshots_empty_when_none() {
        let tmp = TempDir::new().unwrap();
        let snaps = list_snapshots(tmp.path()).unwrap();
        assert!(snaps.is_empty());
    }

    #[test]
    fn legacy_snapshot_without_hashes_remains_listable_and_restorable() {
        let tmp = TempDir::new().unwrap();
        let inst = make_instance(&tmp);
        let id = "legacy-snapshot";
        fs::create_dir_all(snapshots_dir(&inst)).unwrap();

        let contents = b"legacy mod content";
        let snapshot = Snapshot {
            id: id.into(),
            label: Some("from-v1".into()),
            created_at: "2026-01-01T00:00:00Z".into(),
            file_count: 1,
            size_estimate: contents.len() as u64,
        };
        let legacy_manifest = serde_json::json!({
            "snapshot": snapshot,
            "files": [{
                "relative_path": "mods/legacy.jar",
                "size": contents.len()
            }]
        });
        write_test_archive(
            &snapshot_zip_path(&inst, id),
            &[
                ("mods/legacy.jar", contents.as_slice()),
                (
                    "manifest.json",
                    serde_json::to_vec_pretty(&legacy_manifest)
                        .unwrap()
                        .as_slice(),
                ),
            ],
        );

        let listed = list_snapshots(&inst).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, id);

        fs::write(inst.join("mods").join("test.jar"), b"current state").unwrap();
        restore_snapshot(&inst, id).unwrap();
        assert_eq!(
            fs::read(inst.join("mods").join("legacy.jar")).unwrap(),
            contents
        );
        assert!(!inst.join("mods").join("test.jar").exists());
    }

    #[test]
    fn restore_rejects_hash_tampering_before_live_mutation() {
        let tmp = TempDir::new().unwrap();
        let inst = make_instance(&tmp);
        let snapshot = create_snapshot(&inst, None).unwrap();
        fs::write(inst.join("mods").join("test.jar"), b"current safe state").unwrap();

        let manifest =
            read_manifest_file(&snapshot_manifest_path(&inst, &snapshot.id), &snapshot.id).unwrap();
        let hash = manifest
            .files
            .iter()
            .find(|file| file.relative_path == "mods/test.jar")
            .unwrap()
            .blob_sha256
            .as_deref()
            .unwrap();
        fs::write(snapshot_blob_path(&inst, hash), b"bad content").unwrap();

        let error = restore_snapshot(&inst, &snapshot.id).unwrap_err();
        assert!(error.contains("hash mismatch"));
        assert_eq!(
            fs::read(inst.join("mods").join("test.jar")).unwrap(),
            b"current safe state"
        );
        assert!(!inst.join(RESTORE_MARKER).exists());
    }

    #[test]
    fn partial_promotion_failure_restores_the_entire_current_state() {
        let tmp = TempDir::new().unwrap();
        let inst = make_instance(&tmp);
        let snapshot = create_snapshot(&inst, None).unwrap();

        fs::write(inst.join("mods").join("test.jar"), b"new mod state").unwrap();
        fs::write(
            inst.join("config").join("settings.toml"),
            b"new config state",
        )
        .unwrap();
        fs::write(inst.join("options.txt"), b"new options state").unwrap();

        let error = restore_snapshot_impl(&inst, &snapshot.id, Some(1)).unwrap_err();
        assert!(error.contains("original instance state was restored"));
        assert_eq!(
            fs::read(inst.join("mods").join("test.jar")).unwrap(),
            b"new mod state"
        );
        assert_eq!(
            fs::read(inst.join("config").join("settings.toml")).unwrap(),
            b"new config state"
        );
        assert_eq!(
            fs::read(inst.join("options.txt")).unwrap(),
            b"new options state"
        );
        assert!(!inst.join(RESTORE_MARKER).exists());
    }

    #[test]
    fn interrupted_restore_is_recovered_before_a_new_attempt() {
        let tmp = TempDir::new().unwrap();
        let inst = make_instance(&tmp);
        fs::write(
            inst.join("mods").join("test.jar"),
            b"protected current state",
        )
        .unwrap();

        let pre = pre_restore_dir(&inst);
        fs::create_dir_all(&pre).unwrap();
        fs::rename(inst.join("mods"), pre.join("mods")).unwrap();
        fs::create_dir_all(inst.join("mods")).unwrap();
        fs::write(
            inst.join("mods").join("test.jar"),
            b"partial snapshot state",
        )
        .unwrap();
        fs::write(inst.join(RESTORE_MARKER), b"restore in progress").unwrap();

        let error = restore_snapshot(&inst, "missing-snapshot").unwrap_err();
        assert!(error.contains("not found"));
        assert_eq!(
            fs::read(inst.join("mods").join("test.jar")).unwrap(),
            b"protected current state"
        );
        assert!(!inst.join(RESTORE_MARKER).exists());
        assert!(!pre.exists());
    }

    fn write_test_archive(path: &Path, entries: &[(&str, &[u8])]) {
        let file = fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = FileOptions::default().compression_method(CompressionMethod::Deflated);
        for (name, contents) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(contents).unwrap();
        }
        zip.finish().unwrap();
    }
}
