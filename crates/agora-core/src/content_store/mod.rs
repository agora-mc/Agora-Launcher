//! Content store for immutable mod assets (MASTER_SPEC §26.6).
//!
//! A mod's files are stored once, by content hash, protected from modification,
//! and verifiable. Nothing deploys content to a game yet (slice 2).

pub mod protect;

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};

use agora_game_api::RelPath;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::app_paths::AppPaths;
use crate::ctx::Ctx;
use crate::lock_manager::LockResource;

pub use crate::game_base::VerifyDepth;
pub use protect::Protection;

// ---------------------------------------------------------------------------
// Manifest and Outcome Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentFile {
    pub path: RelPath,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ContentSource {
    Archive {
        path: String,
        sha256: String,
        added_at_unix_ms: i64,
    },
    Folder {
        path: String,
        added_at_unix_ms: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentItem {
    pub item_id: String,
    pub name: String,
    pub files: Vec<ContentFile>, // sorted by path
    pub total_size: u64,
    pub sources: Vec<ContentSource>, // oldest first
    pub added_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AddOutcome {
    Added {
        item: ContentItem,
        #[serde(alias = "new")]
        objects_new: usize,
    },
    Existing {
        item: ContentItem,
        #[serde(alias = "new")]
        objects_new: usize,
        #[serde(alias = "restored")]
        objects_restored: usize,
        #[serde(alias = "present")]
        objects_present: usize,
    },
}

impl AddOutcome {
    pub fn item(&self) -> &ContentItem {
        match self {
            AddOutcome::Added { item, .. } => item,
            AddOutcome::Existing { item, .. } => item,
        }
    }

    pub fn objects_new(&self) -> usize {
        match self {
            AddOutcome::Added { objects_new, .. } => *objects_new,
            AddOutcome::Existing { objects_new, .. } => *objects_new,
        }
    }

    pub fn objects_restored(&self) -> usize {
        match self {
            AddOutcome::Added { .. } => 0,
            AddOutcome::Existing {
                objects_restored, ..
            } => *objects_restored,
        }
    }

    pub fn objects_present(&self) -> usize {
        match self {
            AddOutcome::Added { .. } => 0,
            AddOutcome::Existing {
                objects_present, ..
            } => *objects_present,
        }
    }
}

// ---------------------------------------------------------------------------
// Problem and Verification Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProblemKind {
    Missing,
    SizeMismatch { expected: u64, actual: u64 },
    HashMismatch { expected: String, actual: String },
    Unprotected,
    CorruptManifest { error: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentProblem {
    pub item_id: String,
    pub path: String,
    pub kind: ProblemKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentVerification {
    pub item_id: String,
    pub checked: usize,
    pub hashed: usize,
    pub problems: Vec<ContentProblem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentVerificationReport {
    pub checked_items: usize,
    pub checked_files: usize,
    pub hashed_files: usize,
    pub problems: Vec<ContentProblem>,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum ContentError {
    #[error("item not found: {0}")]
    NotFound(String),
    #[error("prefix '{prefix}' matches multiple items: {matches}")]
    AmbiguousPrefix { prefix: String, matches: String },
    #[error("invalid path '{path}': {reason}")]
    InvalidPath { path: String, reason: String },
    #[error("item has no files")]
    EmptyItem,
    #[error("archive entry '{path}' declared size {declared} but yielded more")]
    EntryExceedsDeclaredSize { path: String, declared: u64 },
    #[error("insufficient disk space: need {required} bytes (including 1 GiB headroom), but only {available} bytes available")]
    InsufficientSpace { required: u64, available: u64 },
    #[error("cannot remove item {item_id}: in use by instance(s): {instances}")]
    InUse { item_id: String, instances: String },
    #[error("cannot remove item {item_id}: manifest '{manifest}' is unreadable: {error}")]
    UnreadableManifest {
        item_id: String,
        manifest: String,
        error: String,
    },
    #[error("corrupt archive: {0}")]
    CorruptArchive(String),
    #[error("lock error: {0}")]
    Lock(#[from] crate::error::LauncherError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Other(String),
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const ONE_GIB: u64 = 1024 * 1024 * 1024;
const HASH_BUFFER_SIZE: usize = 128 * 1024;

fn now_unix_ms() -> i64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    }
}

pub fn is_windows_device_name(stem: &str) -> bool {
    let s = stem.to_ascii_uppercase();
    matches!(
        s.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

fn is_reparse_point_or_symlink(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        (metadata.file_attributes() & 0x0000_0400) != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

pub fn validate_entry_path(rel_str: &str) -> Result<RelPath, ContentError> {
    let norm = rel_str.replace('\\', "/");

    // Check empty or '.' components
    for comp in norm.split('/') {
        if comp.is_empty() {
            return Err(ContentError::InvalidPath {
                path: rel_str.to_string(),
                reason: "contains empty component or trailing slash".into(),
            });
        }
        if comp == "." {
            return Err(ContentError::InvalidPath {
                path: rel_str.to_string(),
                reason: "contains '.' component".into(),
            });
        }
        if comp == ".." {
            return Err(ContentError::InvalidPath {
                path: rel_str.to_string(),
                reason: "contains '..' component".into(),
            });
        }
        if comp.ends_with('.') || comp.ends_with(' ') {
            return Err(ContentError::InvalidPath {
                path: rel_str.to_string(),
                reason: "component ends in '.' or space".into(),
            });
        }
        let stem = comp.split('.').next().unwrap_or("");
        if is_windows_device_name(stem) {
            return Err(ContentError::InvalidPath {
                path: rel_str.to_string(),
                reason: format!("component '{comp}' uses Windows device name '{stem}'"),
            });
        }
    }

    RelPath::new(&norm).map_err(|e| ContentError::InvalidPath {
        path: rel_str.to_string(),
        reason: e.to_string(),
    })
}

pub fn validate_path_set(paths: &[RelPath]) -> Result<(), ContentError> {
    if paths.is_empty() {
        return Err(ContentError::EmptyItem);
    }

    let mut seen_lower = HashSet::with_capacity(paths.len());
    for p in paths {
        let lower = p.as_str().to_ascii_lowercase();
        if !seen_lower.insert(lower) {
            return Err(ContentError::InvalidPath {
                path: p.to_string(),
                reason: "equals another file's path case-insensitively".into(),
            });
        }
    }

    for p in paths {
        let parts: Vec<&str> = p.as_str().split('/').collect();
        for i in 1..parts.len() {
            let prefix = parts[..i].join("/").to_ascii_lowercase();
            if seen_lower.contains(&prefix) {
                return Err(ContentError::InvalidPath {
                    path: p.to_string(),
                    reason: format!("prefix folder '{prefix}' matches another file"),
                });
            }
        }
    }

    Ok(())
}

fn disk_free_space(path: &Path) -> Option<u64> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let mut cur = if path.exists() {
            path.to_path_buf()
        } else {
            path.parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| path.to_path_buf())
        };
        while !cur.exists() {
            if let Some(parent) = cur.parent() {
                cur = parent.to_path_buf();
            } else {
                break;
            }
        }
        let wide: Vec<u16> = cur
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut free_bytes_available: u64 = 0;
        let mut total_bytes: u64 = 0;
        let mut total_free_bytes: u64 = 0;
        let ok = unsafe {
            windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut free_bytes_available,
                &mut total_bytes,
                &mut total_free_bytes,
            )
        };
        if ok != 0 {
            return Some(free_bytes_available);
        }
    }
    crate::helpers::available_disk_space_bytes(path)
}

/// Whether an existing object can be kept as is. A protected object of the right size is trusted
/// (the quick check, as `VerifyDepth::Quick`); one that was unprotected or has the wrong size may
/// have been changed, so its bytes are re-hashed.
fn object_is_trustworthy(path: &Path, size: u64, sha256: &str) -> Result<bool, ContentError> {
    if std::fs::metadata(path)?.len() != size {
        return Ok(false);
    }
    if protect::protection(path)? == Protection::Protected {
        return Ok(true);
    }
    Ok(hash_file_path(path)? == sha256)
}

struct StagingGuard<'a> {
    path: &'a Path,
    active: bool,
}

impl<'a> StagingGuard<'a> {
    fn new(path: &'a Path) -> Self {
        Self { path, active: true }
    }
}

impl<'a> Drop for StagingGuard<'a> {
    fn drop(&mut self) {
        if self.active && self.path.exists() {
            let _ = std::fs::remove_dir_all(self.path);
        }
    }
}

fn clean_staging_dir(paths: &AppPaths) {
    let staging_root = paths.content_staging_dir();
    if staging_root.exists() {
        if let Ok(entries) = std::fs::read_dir(&staging_root) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    let _ = std::fs::remove_dir_all(&p);
                } else {
                    let _ = std::fs::remove_file(&p);
                }
            }
        }
    }
}

fn hash_file_path(path: &Path) -> Result<String, std::io::Error> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; HASH_BUFFER_SIZE];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

// ---------------------------------------------------------------------------
// Item Id and Manifest Operations
// ---------------------------------------------------------------------------

pub fn compute_canonical_item_id(files: &[ContentFile]) -> String {
    let mut sorted = files.to_vec();
    sorted.sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));

    let mut canonical = String::new();
    for f in sorted {
        use std::fmt::Write;
        let _ = writeln!(canonical, "{}\t{}\t{}", f.path.as_str(), f.size, f.sha256);
    }
    format!("{:x}", Sha256::digest(canonical.as_bytes()))
}

fn write_manifest_atomic(paths: &AppPaths, item: &ContentItem) -> Result<(), ContentError> {
    let items_dir = paths.content_items_dir();
    std::fs::create_dir_all(&items_dir)?;

    let target_path = paths.content_item_path(&item.item_id);
    let unique = format!("{}-{}", std::process::id(), uuid::Uuid::new_v4().simple());
    let tmp_path = items_dir.join(format!("{}.json.tmp-{}", item.item_id, unique));

    let json_bytes = serde_json::to_string_pretty(item)?;
    std::fs::write(&tmp_path, json_bytes)?;
    std::fs::rename(&tmp_path, &target_path)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Adding: Archives and Folders
// ---------------------------------------------------------------------------

struct StagedEntry {
    rel_path: RelPath,
    staged_path: PathBuf,
    size: u64,
    sha256: String,
}

pub fn add_archive(
    ctx: &Ctx,
    archive_path: &Path,
    name: Option<&str>,
) -> Result<AddOutcome, ContentError> {
    let _lock = ctx
        .lock_manager
        .acquire(LockResource::ContentStore, "content-add")?;

    if !archive_path.exists() {
        return Err(ContentError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("archive not found: {}", archive_path.display()),
        )));
    }

    clean_staging_dir(&ctx.paths);

    let archive_sha256 = hash_file_path(archive_path)?;
    let archive_display_path = archive_path.to_string_lossy().to_string();

    let file = std::fs::File::open(archive_path)?;
    let mut zip =
        zip::ZipArchive::new(file).map_err(|e| ContentError::CorruptArchive(format!("{e}")))?;

    // 1. First pass: validate entry names, types, and collect declared sizes
    let mut declared_total_size: u64 = 0;
    let mut validated_paths = Vec::new();
    let mut file_indices = Vec::new();

    for i in 0..zip.len() {
        let entry = zip
            .by_index(i)
            .map_err(|e| ContentError::CorruptArchive(format!("{e}")))?;

        // Folder entries are ignored; only files count
        if entry.is_dir() || entry.name().ends_with('/') {
            continue;
        }

        // Refuse zip symlink entries (unix mode S_IFLNK)
        if let Some(mode) = entry.unix_mode() {
            if (mode & 0o170000) == 0o120000 {
                return Err(ContentError::InvalidPath {
                    path: entry.name().to_string(),
                    reason: "zip symlink entry is not allowed".into(),
                });
            }
        }

        let rel_path = validate_entry_path(entry.name())?;
        declared_total_size = declared_total_size.saturating_add(entry.size());
        validated_paths.push(rel_path);
        file_indices.push(i);
    }

    // Refuse empty items (no files) and collisions across paths
    validate_path_set(&validated_paths)?;

    // 2. Untrusted sizes: check free disk space
    let content_root = ctx.paths.content_root();
    std::fs::create_dir_all(&content_root)?;
    let required = declared_total_size.saturating_add(ONE_GIB);
    if let Some(available) = disk_free_space(&content_root) {
        if available < required {
            return Err(ContentError::InsufficientSpace {
                required,
                available,
            });
        }
    }

    // 3. Staging directory setup
    let staging_root = ctx.paths.content_staging_dir();
    std::fs::create_dir_all(&staging_root)?;
    let unique = format!("{}-{}", std::process::id(), uuid::Uuid::new_v4().simple());
    let staging_dir = staging_root.join(&unique);
    std::fs::create_dir_all(&staging_dir)?;
    let mut guard = StagingGuard::new(&staging_dir);

    // 4. Extract and stream-hash each file
    let mut staged_entries = Vec::with_capacity(file_indices.len());
    for (idx, &file_idx) in file_indices.iter().enumerate() {
        let mut entry = zip
            .by_index(file_idx)
            .map_err(|e| ContentError::CorruptArchive(format!("{e}")))?;
        let rel_path = validated_paths[idx].clone();
        let declared_size = entry.size();

        let staged_file_path = staging_dir.join(format!("obj_{idx}"));
        use std::io::Write;
        let mut staged_out = std::fs::File::create(&staged_file_path)?;

        let mut hasher = Sha256::new();
        let mut buf = [0u8; HASH_BUFFER_SIZE];
        let mut bytes_written = 0u64;
        let mut limited = (&mut entry).take(declared_size + 1);

        loop {
            let n = limited.read(&mut buf).map_err(|e| {
                ContentError::CorruptArchive(format!("failed reading entry '{}': {e}", rel_path))
            })?;
            if n == 0 {
                break;
            }
            bytes_written += n as u64;
            if bytes_written > declared_size {
                return Err(ContentError::EntryExceedsDeclaredSize {
                    path: rel_path.to_string(),
                    declared: declared_size,
                });
            }
            staged_out.write_all(&buf[..n])?;
            hasher.update(&buf[..n]);
        }

        let sha256 = format!("{:x}", hasher.finalize());
        staged_entries.push(StagedEntry {
            rel_path,
            staged_path: staged_file_path,
            size: bytes_written,
            sha256,
        });
    }

    // 5. Store objects, record counts, compute item ID
    let item_name = name
        .map(|s| s.to_string())
        .or_else(|| {
            archive_path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "content".into());

    let source = ContentSource::Archive {
        path: archive_display_path,
        sha256: archive_sha256,
        added_at_unix_ms: now_unix_ms(),
    };

    let outcome = finish_adding(ctx, &item_name, staged_entries, source)?;
    guard.active = false;
    let _ = std::fs::remove_dir_all(&staging_dir);
    Ok(outcome)
}

pub fn add_folder(
    ctx: &Ctx,
    folder_path: &Path,
    name: Option<&str>,
) -> Result<AddOutcome, ContentError> {
    let _lock = ctx
        .lock_manager
        .acquire(LockResource::ContentStore, "content-add")?;

    if !folder_path.exists() {
        return Err(ContentError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("folder not found: {}", folder_path.display()),
        )));
    }
    if !folder_path.is_dir() {
        return Err(ContentError::Other(format!(
            "path is not a directory: {}",
            folder_path.display()
        )));
    }

    clean_staging_dir(&ctx.paths);

    let folder_display_path = folder_path.to_string_lossy().to_string();

    // 1. Walk folder collecting entries
    struct SourceFile {
        rel_path: RelPath,
        abs_path: PathBuf,
    }
    let mut collected = Vec::new();
    let mut validated_paths = Vec::new();
    let mut declared_total_size: u64 = 0;

    fn walk_dir(
        root: &Path,
        rel: &Path,
        collected: &mut Vec<SourceFile>,
        validated_paths: &mut Vec<RelPath>,
        declared_total_size: &mut u64,
    ) -> Result<(), ContentError> {
        let cur = if rel.as_os_str().is_empty() {
            root.to_path_buf()
        } else {
            root.join(rel)
        };
        for entry in std::fs::read_dir(&cur)? {
            let entry = entry?;
            let file_name = entry.file_name();
            let entry_rel = rel.join(&file_name);
            let abs_path = entry.path();
            let meta = std::fs::symlink_metadata(&abs_path)?;

            let rel_str = entry_rel.to_string_lossy().replace('\\', "/");

            if is_reparse_point_or_symlink(&meta) {
                return Err(ContentError::InvalidPath {
                    path: rel_str,
                    reason: "symlinks or reparse points are not allowed".into(),
                });
            }

            if meta.is_dir() {
                walk_dir(
                    root,
                    &entry_rel,
                    collected,
                    validated_paths,
                    declared_total_size,
                )?;
            } else if meta.is_file() {
                let rel_path = validate_entry_path(&rel_str)?;
                let size = meta.len();
                *declared_total_size = declared_total_size.saturating_add(size);
                validated_paths.push(rel_path.clone());
                collected.push(SourceFile { rel_path, abs_path });
            }
        }
        Ok(())
    }

    walk_dir(
        folder_path,
        Path::new(""),
        &mut collected,
        &mut validated_paths,
        &mut declared_total_size,
    )?;

    validate_path_set(&validated_paths)?;

    // 2. Untrusted sizes: check free disk space
    let content_root = ctx.paths.content_root();
    std::fs::create_dir_all(&content_root)?;
    let required = declared_total_size.saturating_add(ONE_GIB);
    if let Some(available) = disk_free_space(&content_root) {
        if available < required {
            return Err(ContentError::InsufficientSpace {
                required,
                available,
            });
        }
    }

    // 3. Staging directory setup
    let staging_root = ctx.paths.content_staging_dir();
    std::fs::create_dir_all(&staging_root)?;
    let unique = format!("{}-{}", std::process::id(), uuid::Uuid::new_v4().simple());
    let staging_dir = staging_root.join(&unique);
    std::fs::create_dir_all(&staging_dir)?;
    let mut guard = StagingGuard::new(&staging_dir);

    // 4. Copy each file to staging while hashing
    let mut staged_entries = Vec::with_capacity(collected.len());
    for (idx, src) in collected.into_iter().enumerate() {
        let staged_file_path = staging_dir.join(format!("obj_{idx}"));
        use std::io::Write;
        let mut file_in = std::fs::File::open(&src.abs_path)?;
        let mut staged_out = std::fs::File::create(&staged_file_path)?;

        let mut hasher = Sha256::new();
        let mut buf = [0u8; HASH_BUFFER_SIZE];
        let mut bytes_written = 0u64;

        loop {
            let n = file_in.read(&mut buf)?;
            if n == 0 {
                break;
            }
            bytes_written += n as u64;
            staged_out.write_all(&buf[..n])?;
            hasher.update(&buf[..n]);
        }

        let sha256 = format!("{:x}", hasher.finalize());
        staged_entries.push(StagedEntry {
            rel_path: src.rel_path,
            staged_path: staged_file_path,
            size: bytes_written,
            sha256,
        });
    }

    let item_name = name
        .map(|s| s.to_string())
        .or_else(|| {
            folder_path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "content".into());

    let source = ContentSource::Folder {
        path: folder_display_path,
        added_at_unix_ms: now_unix_ms(),
    };

    let outcome = finish_adding(ctx, &item_name, staged_entries, source)?;
    guard.active = false;
    let _ = std::fs::remove_dir_all(&staging_dir);
    Ok(outcome)
}

fn finish_adding(
    ctx: &Ctx,
    item_name: &str,
    staged_entries: Vec<StagedEntry>,
    source: ContentSource,
) -> Result<AddOutcome, ContentError> {
    let mut files = Vec::with_capacity(staged_entries.len());
    let mut objects_new = 0usize;
    let mut objects_restored = 0usize;
    let mut objects_present = 0usize;
    let mut total_size = 0u64;

    for entry in staged_entries {
        total_size += entry.size;
        let obj_path = ctx.paths.content_object_path(&entry.sha256);
        if let Some(parent) = obj_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        if obj_path.exists() && object_is_trustworthy(&obj_path, entry.size, &entry.sha256)? {
            let _ = std::fs::remove_file(&entry.staged_path);
            if protect::protection(&obj_path)? != Protection::Protected {
                protect::protect(&obj_path)?;
            }
            objects_present += 1;
        } else if obj_path.exists() {
            // The object no longer holds what its name says (changed while unprotected, or
            // truncated): the staged bytes are the real content, so they replace it. Protecting
            // the old bytes as they are would turn a corruption into the recorded truth.
            protect::unprotect(&obj_path)?;
            std::fs::remove_file(&obj_path)?;
            std::fs::rename(&entry.staged_path, &obj_path)?;
            protect::protect(&obj_path)?;
            objects_new += 1;
        } else {
            std::fs::rename(&entry.staged_path, &obj_path)?;
            protect::protect(&obj_path)?;
            objects_new += 1;
        }

        files.push(ContentFile {
            path: entry.rel_path,
            size: entry.size,
            sha256: entry.sha256,
        });
    }

    files.sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));
    let item_id = compute_canonical_item_id(&files);
    let manifest_path = ctx.paths.content_item_path(&item_id);

    if manifest_path.exists() {
        let content = std::fs::read_to_string(&manifest_path)?;
        let mut existing_item: ContentItem = serde_json::from_str(&content)?;

        // If objects were created on disk for an existing manifest, they were restored
        if objects_new > 0 {
            objects_restored = objects_new;
            objects_new = 0;
        }

        // Append source if it is new
        let is_new_source = match &source {
            ContentSource::Archive { path, sha256, .. } => !existing_item.sources.iter().any(|s| {
                matches!(
                    s,
                    ContentSource::Archive {
                        path: p,
                        sha256: h,
                        ..
                    } if p == path && h == sha256
                )
            }),
            ContentSource::Folder { path, .. } => !existing_item.sources.iter().any(|s| {
                matches!(
                    s,
                    ContentSource::Folder { path: p, .. } if p == path
                )
            }),
        };

        if is_new_source {
            existing_item.sources.push(source);
            write_manifest_atomic(&ctx.paths, &existing_item)?;
        }

        Ok(AddOutcome::Existing {
            item: existing_item,
            objects_new,
            objects_restored,
            objects_present,
        })
    } else {
        let item = ContentItem {
            item_id,
            name: item_name.to_string(),
            files,
            total_size,
            sources: vec![source],
            added_at_unix_ms: now_unix_ms(),
        };
        write_manifest_atomic(&ctx.paths, &item)?;
        Ok(AddOutcome::Added { item, objects_new })
    }
}

// ---------------------------------------------------------------------------
// Prefix Resolution & Queries
// ---------------------------------------------------------------------------

pub fn resolve_item_id(ctx: &Ctx, prefix: &str) -> Result<String, ContentError> {
    // An empty prefix would match every item, so `remove ""` could delete the only one; ids are
    // lowercase hex, so anything else cannot name an item (and can never become a path).
    if prefix.is_empty()
        || !prefix
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(ContentError::NotFound(prefix.to_string()));
    }
    let items_dir = ctx.paths.content_items_dir();
    if !items_dir.exists() {
        return Err(ContentError::NotFound(prefix.to_string()));
    }
    let mut matches = Vec::new();
    for entry in std::fs::read_dir(items_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().map(|e| e == "json").unwrap_or(false) {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                if stem.starts_with(prefix) {
                    matches.push(stem.to_string());
                }
            }
        }
    }
    match matches.len() {
        0 => Err(ContentError::NotFound(prefix.to_string())),
        1 => Ok(matches.remove(0)),
        _ => {
            matches.sort();
            Err(ContentError::AmbiguousPrefix {
                prefix: prefix.to_string(),
                matches: matches.join(", "),
            })
        }
    }
}

pub fn get_item(ctx: &Ctx, item_id_or_prefix: &str) -> Result<ContentItem, ContentError> {
    let item_id = resolve_item_id(ctx, item_id_or_prefix)?;
    let path = ctx.paths.content_item_path(&item_id);
    let content = std::fs::read_to_string(&path)?;
    let item: ContentItem = serde_json::from_str(&content)?;
    Ok(item)
}

pub fn list_items(ctx: &Ctx) -> Result<Vec<ContentItem>, ContentError> {
    let items_dir = ctx.paths.content_items_dir();
    if !items_dir.exists() {
        return Ok(Vec::new());
    }
    let mut items = Vec::new();
    for entry in std::fs::read_dir(items_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().map(|e| e == "json").unwrap_or(false) {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(item) = serde_json::from_str::<ContentItem>(&content) {
                    items.push(item);
                }
            }
        }
    }
    items.sort_by(|a, b| a.item_id.cmp(&b.item_id));
    Ok(items)
}

// ---------------------------------------------------------------------------
// Verification
// ---------------------------------------------------------------------------

pub fn verify_item(
    ctx: &Ctx,
    item_id_or_prefix: &str,
    depth: VerifyDepth,
) -> Result<ContentVerification, ContentError> {
    let item_id = resolve_item_id(ctx, item_id_or_prefix)?;
    let path = ctx.paths.content_item_path(&item_id);

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            return Ok(ContentVerification {
                item_id: item_id.clone(),
                checked: 0,
                hashed: 0,
                problems: vec![ContentProblem {
                    item_id,
                    path: path.display().to_string(),
                    kind: ProblemKind::CorruptManifest {
                        error: e.to_string(),
                    },
                }],
            });
        }
    };

    let item: ContentItem = match serde_json::from_str(&content) {
        Ok(it) => it,
        Err(e) => {
            return Ok(ContentVerification {
                item_id: item_id.clone(),
                checked: 0,
                hashed: 0,
                problems: vec![ContentProblem {
                    item_id,
                    path: path.display().to_string(),
                    kind: ProblemKind::CorruptManifest {
                        error: e.to_string(),
                    },
                }],
            });
        }
    };

    let mut problems = Vec::new();
    let mut checked = 0usize;
    let mut hashed = 0usize;

    for file in &item.files {
        checked += 1;
        let obj_path = ctx.paths.content_object_path(&file.sha256);
        let meta = match std::fs::metadata(&obj_path) {
            Ok(m) => m,
            Err(_) => {
                problems.push(ContentProblem {
                    item_id: item.item_id.clone(),
                    path: file.path.to_string(),
                    kind: ProblemKind::Missing,
                });
                continue;
            }
        };

        if meta.len() != file.size {
            problems.push(ContentProblem {
                item_id: item.item_id.clone(),
                path: file.path.to_string(),
                kind: ProblemKind::SizeMismatch {
                    expected: file.size,
                    actual: meta.len(),
                },
            });
        }

        match protect::protection(&obj_path) {
            Ok(Protection::Unprotected) => {
                problems.push(ContentProblem {
                    item_id: item.item_id.clone(),
                    path: file.path.to_string(),
                    kind: ProblemKind::Unprotected,
                });
            }
            Ok(Protection::Protected) | Ok(Protection::Unsupported) => {}
            Err(e) => {
                problems.push(ContentProblem {
                    item_id: item.item_id.clone(),
                    path: file.path.to_string(),
                    kind: ProblemKind::CorruptManifest {
                        error: format!("protection check failed: {e}"),
                    },
                });
            }
        }

        if depth == VerifyDepth::Full {
            hashed += 1;
            match hash_file_path(&obj_path) {
                Ok(h) => {
                    if h != file.sha256 {
                        problems.push(ContentProblem {
                            item_id: item.item_id.clone(),
                            path: file.path.to_string(),
                            kind: ProblemKind::HashMismatch {
                                expected: file.sha256.clone(),
                                actual: h,
                            },
                        });
                    }
                }
                Err(e) => {
                    problems.push(ContentProblem {
                        item_id: item.item_id.clone(),
                        path: file.path.to_string(),
                        kind: ProblemKind::CorruptManifest {
                            error: format!("hash read error: {e}"),
                        },
                    });
                }
            }
        }
    }

    Ok(ContentVerification {
        item_id: item.item_id,
        checked,
        hashed,
        problems,
    })
}

pub fn verify_all(
    ctx: &Ctx,
    depth: VerifyDepth,
) -> Result<ContentVerificationReport, ContentError> {
    let items_dir = ctx.paths.content_items_dir();
    if !items_dir.exists() {
        return Ok(ContentVerificationReport {
            checked_items: 0,
            checked_files: 0,
            hashed_files: 0,
            problems: Vec::new(),
        });
    }

    let mut checked_items = 0usize;
    let mut checked_files = 0usize;
    let mut hashed_files = 0usize;
    let mut all_problems = Vec::new();

    for entry in std::fs::read_dir(items_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().map(|e| e == "json").unwrap_or(false) {
            checked_items += 1;
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown");
            let ver = verify_item(ctx, stem, depth)?;
            checked_files += ver.checked;
            hashed_files += ver.hashed;
            all_problems.extend(ver.problems);
        }
    }

    all_problems.sort_by(|a, b| a.item_id.cmp(&b.item_id).then_with(|| a.path.cmp(&b.path)));

    Ok(ContentVerificationReport {
        checked_items,
        checked_files,
        hashed_files,
        problems: all_problems,
    })
}

// ---------------------------------------------------------------------------
// Removal
// ---------------------------------------------------------------------------

/// Remove a content item and any objects that no remaining item references.
///
/// Under the lock, every instance manifest is checked first; if any instance
/// references the item, removal refuses naming the instances. If any instance
/// manifest cannot be read or parsed, removal refuses too (fail closed).
/// Then every other content manifest is read and validated; if any cannot be read
/// or parsed, removal refuses and deletes nothing to prevent sweeping objects that
/// an unreadable item still needs.
pub fn remove_item(ctx: &Ctx, item_id_or_prefix: &str) -> Result<(), ContentError> {
    let _lock = ctx
        .lock_manager
        .acquire(LockResource::ContentStore, "content-remove")?;

    let item_id = resolve_item_id(ctx, item_id_or_prefix)?;
    let target_manifest_path = ctx.paths.content_item_path(&item_id);

    if !target_manifest_path.exists() {
        return Err(ContentError::NotFound(item_id));
    }

    // 0. Check every generic game instance first; refuse if one uses the item, or if any of them
    //    cannot be listed or read (fail closed). Minecraft instances share the instances folder
    //    and the manifest file name in their own format, and never hold content layers, so they
    //    are found through the generic instance table rather than by parsing every folder.
    let mut using_instances = Vec::new();
    let records =
        crate::game_instance::list(ctx).map_err(|e| ContentError::UnreadableManifest {
            item_id: item_id.clone(),
            manifest: "game instance list".into(),
            error: e.to_string(),
        })?;
    for record in records {
        let manifest =
            crate::game_instance::get_manifest(ctx, &record.instance_id).map_err(|e| {
                ContentError::UnreadableManifest {
                    item_id: item_id.clone(),
                    manifest: format!("instance {}", record.instance_id),
                    error: e.to_string(),
                }
            })?;
        let uses_item = manifest.layers.layers().iter().any(|layer| {
            matches!(&layer.source, agora_game_api::LayerSource::Content { content } if content == &item_id)
        });
        if uses_item {
            using_instances.push(manifest.instance_id.clone());
        }
    }

    if !using_instances.is_empty() {
        using_instances.sort();
        return Err(ContentError::InUse {
            item_id,
            instances: using_instances.join(", "),
        });
    }

    // The target's own manifest is not parsed: the sweep below works from the remaining items, so
    // a corrupt item can still be removed.

    // 1. Read every other manifest first; refuse if ANY cannot be read or parsed
    let items_dir = ctx.paths.content_items_dir();
    let mut other_items = Vec::new();

    if items_dir.exists() {
        for entry in std::fs::read_dir(&items_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    if stem == item_id {
                        continue;
                    }
                    let content = match std::fs::read_to_string(&path) {
                        Ok(c) => c,
                        Err(e) => {
                            return Err(ContentError::UnreadableManifest {
                                item_id: item_id.clone(),
                                manifest: path.display().to_string(),
                                error: e.to_string(),
                            });
                        }
                    };
                    let item: ContentItem = match serde_json::from_str(&content) {
                        Ok(it) => it,
                        Err(e) => {
                            return Err(ContentError::UnreadableManifest {
                                item_id: item_id.clone(),
                                manifest: path.display().to_string(),
                                error: e.to_string(),
                            });
                        }
                    };
                    other_items.push(item);
                }
            }
        }
    }

    // 2. Collect referenced hashes from all remaining items
    let mut remaining_hashes = HashSet::new();
    for it in &other_items {
        for f in &it.files {
            remaining_hashes.insert(f.sha256.clone());
        }
    }

    // 3. Delete target manifest first
    std::fs::remove_file(&target_manifest_path)?;

    // 4. Delete every object that no remaining item references (unprotect, then delete): the
    //    removed item's own, and any an interrupted add left behind before writing its manifest.
    //    A failure here only leaves an unreferenced object for the next sweep.
    let objects_dir = ctx.paths.content_objects_dir();
    if objects_dir.exists() {
        for shard in std::fs::read_dir(&objects_dir)?.flatten() {
            if !shard.path().is_dir() {
                continue;
            }
            for object in std::fs::read_dir(shard.path())?.flatten() {
                let obj_path = object.path();
                let name = object.file_name().to_string_lossy().into_owned();
                if obj_path.is_file() && !remaining_hashes.contains(&name) {
                    let _ = protect::unprotect(&obj_path);
                    let _ = std::fs::remove_file(&obj_path);
                }
            }
        }
    }

    // 5. Clean up any staging leftovers
    clean_staging_dir(&ctx.paths);

    Ok(())
}
