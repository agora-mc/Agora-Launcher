//! Pinned bases for game instances (MASTER_SPEC §26.4).
//!
//! A pinned base is a private copy of one game version that every instance
//! pinned to that version runs from, so a store update cannot break an instance.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

use agora_game_api::{GameDefinition, InstallId, RuntimeIdentity, VolumeInfo};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::app_paths::AppPaths;
use crate::game_discovery::volume::VolumeDetector;
use crate::game_registry::IdentifiedInstall;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BaseMode {
    Linked,
    Copied,
}

impl std::fmt::Display for BaseMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BaseMode::Linked => write!(f, "linked"),
            BaseMode::Copied => write!(f, "copied"),
        }
    }
}

impl std::str::FromStr for BaseMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "linked" => Ok(BaseMode::Linked),
            "copied" => Ok(BaseMode::Copied),
            other => Err(format!(
                "invalid base mode '{other}': expected 'linked' or 'copied'"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FileIdentity {
    pub volume: u64,
    pub index: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseFile {
    pub path: String, // relative, '/'-separated, as RelPath normalises it
    pub size: u64,
    pub sha256: String, // lowercase hex
    pub linked: bool,
    pub identity: Option<FileIdentity>, // of the base file, recorded after it is created
    pub modified_unix_ms: i64,          // of the base file
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseManifest {
    pub manifest_version: u32, // 1
    pub base_id: String,
    pub runtime: RuntimeIdentity,
    pub mode: BaseMode,
    pub source_install: InstallId,
    pub source_location: PathBuf,
    pub location: PathBuf, // the base's own folder
    pub created_unix_ms: i64,
    pub files: Vec<BaseFile>, // sorted by path
    pub skipped: Vec<String>, // source entries not copied (symlinks, junctions), with reasons
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BuildOutcome {
    Built {
        manifest: BaseManifest,
        linked_bytes: u64,
        copied_bytes: u64,
    },
    Existing(BaseManifest),
}

impl BuildOutcome {
    pub fn manifest(&self) -> &BaseManifest {
        match self {
            BuildOutcome::Built { manifest, .. } => manifest,
            BuildOutcome::Existing(manifest) => manifest,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildProgress {
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_hashed: u64,
    pub bytes_total: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifyDepth {
    Quick,
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProblemKind {
    Missing,
    SizeChanged { expected: u64, actual: u64 },
    ContentChanged,
    Unexpected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseProblem {
    pub path: String,
    pub kind: ProblemKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseVerification {
    pub checked: usize,
    pub hashed: usize,
    pub problems: Vec<BaseProblem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaseListing {
    pub manifest: BaseManifest,
    pub present: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum BaseError {
    #[error("install runtime is unknown or unidentified")]
    RuntimeUnknown,
    #[error("hardlinks unavailable: {reason} (copied base requires {copied_bytes} bytes)")]
    LinkUnavailable { reason: String, copied_bytes: u64 },
    #[error("base not found: {0}")]
    NotFound(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Other(String),
}

// ---------------------------------------------------------------------------
// Base ID calculation
// ---------------------------------------------------------------------------

pub fn sanitize_base_id_part(original: &str) -> String {
    let mut kept = String::new();
    for c in original.chars() {
        let lc = c.to_ascii_lowercase();
        if lc.is_ascii_lowercase() || lc.is_ascii_digit() || lc == '.' || lc == '-' {
            kept.push(lc);
        }
    }
    let changed = kept != original;
    let is_empty = kept.is_empty();
    // Windows strips a trailing dot from a folder name, and a leading one hides
    // it on Unix; either would make the recorded location wrong.
    let edge_dot = kept.starts_with('.') || kept.ends_with('.');

    if changed || is_empty || edge_dot {
        let hash = format!("{:08x}", crate::game_registry::fnv1a_32(original));
        let kept = kept.trim_matches('.');
        if kept.is_empty() {
            format!("h{hash}")
        } else {
            format!("{kept}-{hash}")
        }
    } else {
        kept
    }
}

pub fn make_base_id(runtime: &RuntimeIdentity) -> String {
    let game = sanitize_base_id_part(runtime.game.as_str());
    let store = sanitize_base_id_part(runtime.store.as_str());
    let version = sanitize_base_id_part(&runtime.version);
    let build = match runtime.build.as_deref() {
        Some(b) if !b.is_empty() => sanitize_base_id_part(b),
        _ => "nobuild".to_string(),
    };
    format!("{game}_{store}_{version}_{build}")
}

// ---------------------------------------------------------------------------
// Volume and file identity helpers
// ---------------------------------------------------------------------------

pub fn get_file_identity(path: &Path) -> Option<FileIdentity> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        let file = std::fs::File::open(path).ok()?;
        let handle = file.as_raw_handle();
        let mut info: windows_sys::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION =
            unsafe { std::mem::zeroed() };
        let ok = unsafe {
            windows_sys::Win32::Storage::FileSystem::GetFileInformationByHandle(
                handle as _,
                &mut info,
            )
        };
        if ok != 0 {
            let volume = info.dwVolumeSerialNumber as u64;
            let index = ((info.nFileIndexHigh as u64) << 32) | (info.nFileIndexLow as u64);
            Some(FileIdentity { volume, index })
        } else {
            None
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = std::fs::metadata(path).ok()?;
        Some(FileIdentity {
            volume: meta.dev(),
            index: meta.ino(),
        })
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = path;
        None
    }
}

pub fn get_file_identity_from_file(file: &std::fs::File) -> Option<FileIdentity> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        let handle = file.as_raw_handle();
        let mut info: windows_sys::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION =
            unsafe { std::mem::zeroed() };
        let ok = unsafe {
            windows_sys::Win32::Storage::FileSystem::GetFileInformationByHandle(
                handle as _,
                &mut info,
            )
        };
        if ok != 0 {
            let volume = info.dwVolumeSerialNumber as u64;
            let index = ((info.nFileIndexHigh as u64) << 32) | (info.nFileIndexLow as u64);
            Some(FileIdentity { volume, index })
        } else {
            None
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = file.metadata().ok()?;
        Some(FileIdentity {
            volume: meta.dev(),
            index: meta.ino(),
        })
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = file;
        None
    }
}

pub fn get_modified_unix_ms(metadata: &std::fs::Metadata) -> i64 {
    match metadata.modified() {
        Ok(t) => match t.duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => d.as_millis() as i64,
            Err(e) => -(e.duration().as_millis() as i64),
        },
        Err(_) => 0,
    }
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

/// Pure volume decision function for checking whether hardlinking is supported.
pub fn evaluate_link_support(
    base_volume: Option<&VolumeInfo>,
    install_volume: Option<&VolumeInfo>,
) -> Result<(), String> {
    let (Some(bv), Some(iv)) = (base_volume, install_volume) else {
        return Err("volume information unavailable".to_string());
    };
    if !bv.id.eq_ignore_ascii_case(&iv.id) {
        return Err(format!(
            "base location is on volume {}, but game install is on volume {}",
            bv.id, iv.id
        ));
    }
    if !iv.supports_hardlinks || !bv.supports_hardlinks {
        return Err(format!(
            "volume filesystem '{}' does not support hard links",
            iv.filesystem
        ));
    }
    Ok(())
}

pub fn bases_root_for(
    paths: &AppPaths,
    install_location: &Path,
    install_volume_id: Option<&str>,
) -> PathBuf {
    #[cfg(windows)]
    {
        let detector = VolumeDetector::new();
        let data_vol = detector.get_volume_info(&paths.bases_dir());
        let install_vol_id = install_volume_id
            .map(|s| s.to_string())
            .or_else(|| detector.get_volume_info(install_location).map(|v| v.id));

        if let (Some(d), Some(i)) = (data_vol.map(|v| v.id), install_vol_id) {
            if d.eq_ignore_ascii_case(&i) {
                return paths.bases_dir();
            }
        }

        if let Some(mount_root) =
            crate::game_discovery::volume::get_volume_mount_root(install_location)
        {
            return mount_root.join("AgoraBases");
        }
    }
    let _ = install_location;
    let _ = install_volume_id;
    paths.bases_dir()
}

// ---------------------------------------------------------------------------
// Source walking and staging
// ---------------------------------------------------------------------------

struct SourceEntry {
    rel_path: String,
    abs_src_path: PathBuf,
    size: u64,
    is_archive: bool,
}

fn walk_source_dir(
    source_root: &Path,
    rel_dir: &Path,
    definition: &GameDefinition,
    entries: &mut Vec<SourceEntry>,
    skipped: &mut Vec<String>,
) -> Result<(), std::io::Error> {
    let cur_dir = if rel_dir.as_os_str().is_empty() {
        source_root.to_path_buf()
    } else {
        source_root.join(rel_dir)
    };

    let read_dir = std::fs::read_dir(&cur_dir)?;
    for entry in read_dir {
        let entry = entry?;
        let file_name = entry.file_name();
        let rel_path = rel_dir.join(&file_name);
        let abs_path = entry.path();
        let rel_str = rel_path.to_string_lossy().replace('\\', "/");

        let meta = match std::fs::symlink_metadata(&abs_path) {
            Ok(m) => m,
            Err(e) => {
                skipped.push(format!("{rel_str}: failed to read metadata ({e})"));
                continue;
            }
        };

        if is_reparse_point_or_symlink(&meta) {
            skipped.push(format!("{rel_str}: symlink or junction"));
            continue;
        }

        if meta.is_dir() {
            walk_source_dir(source_root, &rel_path, definition, entries, skipped)?;
        } else if meta.is_file() {
            let is_archive = definition.is_linked_archive(&rel_str);
            entries.push(SourceEntry {
                rel_path: rel_str,
                abs_src_path: abs_path,
                size: meta.len(),
                is_archive,
            });
        } else {
            skipped.push(format!("{rel_str}: non-regular file skipped"));
        }
    }
    Ok(())
}

fn hash_file(path: &Path) -> Result<String, std::io::Error> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 128 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

struct StagingGuard<'a> {
    path: &'a Path,
    active: bool,
}

impl<'a> Drop for StagingGuard<'a> {
    fn drop(&mut self) {
        if self.active && self.path.exists() {
            let _ = std::fs::remove_dir_all(self.path);
        }
    }
}

// ---------------------------------------------------------------------------
// Build base
// ---------------------------------------------------------------------------

pub fn build_base(
    paths: &AppPaths,
    install: &IdentifiedInstall,
    definition: &GameDefinition,
    mode: BaseMode,
    root_override: Option<&Path>,
    progress: &(dyn Fn(BuildProgress) + Send + Sync),
) -> Result<BuildOutcome, BaseError> {
    // 1. Runtime must be Identified
    let runtime = match &install.runtime {
        crate::game_registry::RuntimeResolution::Identified { runtime, .. } => runtime,
        crate::game_registry::RuntimeResolution::Unidentified { .. } => {
            return Err(BaseError::RuntimeUnknown);
        }
    };

    let base_id = make_base_id(runtime);
    let manifest_path = paths.base_manifest_path(&base_id);

    // 2. Existing manifest -> return without touching anything
    if manifest_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&manifest_path) {
            if let Ok(manifest) = serde_json::from_str::<BaseManifest>(&content) {
                return Ok(BuildOutcome::Existing(manifest));
            }
        }
    }

    let source_dir = &install.discovered.location;
    if !source_dir.exists() {
        return Err(BaseError::Other(format!(
            "source install location does not exist: {}",
            source_dir.display()
        )));
    }

    // Walk source without following symlinks/junctions
    let mut entries = Vec::new();
    let mut skipped = Vec::new();
    walk_source_dir(
        source_dir,
        Path::new(""),
        definition,
        &mut entries,
        &mut skipped,
    )?;

    let total_source_bytes: u64 = entries.iter().map(|e| e.size).sum();

    // Determine root
    let base_root = root_override.map(|p| p.to_path_buf()).unwrap_or_else(|| {
        bases_root_for(
            paths,
            source_dir,
            install.discovered.volume.as_ref().map(|v| v.id.as_str()),
        )
    });

    // 3. Linked mode checks
    if mode == BaseMode::Linked {
        let detector = VolumeDetector::new();
        let base_volume = detector.get_volume_info(&base_root);
        let install_volume = install.discovered.volume.as_ref();
        if let Err(reason) = evaluate_link_support(base_volume.as_ref(), install_volume) {
            return Err(BaseError::LinkUnavailable {
                reason,
                copied_bytes: total_source_bytes,
            });
        }
    }

    std::fs::create_dir_all(&base_root)?;

    // 4. Staging folder <root>/<base_id>.partial-<unique>
    let unique = format!("{}-{}", std::process::id(), uuid::Uuid::new_v4().simple());
    let staging_dir = base_root.join(format!("{base_id}.partial-{unique}"));
    std::fs::create_dir_all(&staging_dir)?;

    let mut guard = StagingGuard {
        path: &staging_dir,
        active: true,
    };

    let mut linked_bytes = 0u64;
    let mut copied_bytes = 0u64;
    let mut staged_entries = Vec::with_capacity(entries.len());

    for entry in entries {
        let dest = staging_dir.join(&entry.rel_path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let link_it = mode == BaseMode::Linked && entry.is_archive;
        if link_it {
            std::fs::hard_link(&entry.abs_src_path, &dest)?;
            linked_bytes += entry.size;
        } else {
            std::fs::copy(&entry.abs_src_path, &dest)?;
            copied_bytes += entry.size;
        }
        staged_entries.push((entry, link_it));
    }

    // 5. Hash every base file with scoped threads
    let pool_size = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(8);

    let total_files = staged_entries.len() as u64;
    let current_idx = AtomicUsize::new(0);
    let files_done_counter = AtomicU64::new(0);
    let bytes_hashed_counter = AtomicU64::new(0);
    let hashed_files = Mutex::new(Vec::with_capacity(staged_entries.len()));

    let hash_ctx = HashProgressContext {
        bytes_hashed_counter: &bytes_hashed_counter,
        files_done_counter: &files_done_counter,
        total_files,
        total_bytes: total_source_bytes,
        progress,
    };

    std::thread::scope(|s| {
        for _ in 0..pool_size {
            s.spawn(|| loop {
                let idx = current_idx.fetch_add(1, Ordering::Relaxed);
                if idx >= staged_entries.len() {
                    break;
                }
                let (entry, linked) = &staged_entries[idx];
                let dest = staging_dir.join(&entry.rel_path);
                let res = hash_and_record(&dest, &entry.rel_path, *linked, &hash_ctx);
                hashed_files.lock().unwrap().push(res);
            });
        }
    });

    let results = hashed_files.into_inner().unwrap();
    let mut base_files = Vec::with_capacity(results.len());
    for r in results {
        base_files.push(r?);
    }
    base_files.sort_by(|a, b| a.path.cmp(&b.path));
    skipped.sort();

    let final_base_dir = base_root.join(&base_id);

    // A folder under our own name with no readable manifest is what an
    // interrupted build leaves (renamed into place, manifest never written).
    // Move it aside and delete it, or this base id could never be built again.
    if final_base_dir.exists() && !manifest_path.exists() {
        let stale = base_root.join(format!("{base_id}.stale-{unique}"));
        std::fs::rename(&final_base_dir, &stale)?;
        std::fs::remove_dir_all(&stale)?;
    }

    // 6. Rename staging folder to <root>/<base_id>/
    if let Err(e) = std::fs::rename(&staging_dir, &final_base_dir) {
        // If rename failed because another build won the race, delete staging and return Existing
        if final_base_dir.exists() && manifest_path.exists() {
            let _ = std::fs::remove_dir_all(&staging_dir);
            guard.active = false;
            if let Ok(content) = std::fs::read_to_string(&manifest_path) {
                if let Ok(m) = serde_json::from_str::<BaseManifest>(&content) {
                    return Ok(BuildOutcome::Existing(m));
                }
            }
        }
        return Err(BaseError::Io(e));
    }

    guard.active = false;

    let created_unix_ms = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    };

    let manifest = BaseManifest {
        manifest_version: 1,
        base_id: base_id.clone(),
        runtime: runtime.clone(),
        mode,
        source_install: install.install_id.clone(),
        source_location: source_dir.clone(),
        location: final_base_dir,
        created_unix_ms,
        files: base_files,
        skipped,
    };

    // Write manifest (temp file + rename)
    let manifests_dir = paths.base_manifests_dir();
    std::fs::create_dir_all(&manifests_dir)?;
    let tmp_manifest = manifests_dir.join(format!("{base_id}.json.tmp-{unique}"));
    let manifest_bytes = serde_json::to_string_pretty(&manifest)?;
    std::fs::write(&tmp_manifest, manifest_bytes)?;
    std::fs::rename(&tmp_manifest, &manifest_path)?;

    Ok(BuildOutcome::Built {
        manifest,
        linked_bytes,
        copied_bytes,
    })
}

struct HashProgressContext<'a> {
    bytes_hashed_counter: &'a AtomicU64,
    files_done_counter: &'a AtomicU64,
    total_files: u64,
    total_bytes: u64,
    progress: &'a (dyn Fn(BuildProgress) + Send + Sync),
}

fn hash_and_record(
    dest_path: &Path,
    rel_path: &str,
    linked: bool,
    ctx: &HashProgressContext<'_>,
) -> Result<BaseFile, std::io::Error> {
    use std::io::Read;

    let mut file = std::fs::File::open(dest_path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 128 * 1024];
    let mut file_size: u64 = 0;

    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        file_size += n as u64;
        let hashed = ctx
            .bytes_hashed_counter
            .fetch_add(n as u64, Ordering::Relaxed)
            + (n as u64);
        let done = ctx.files_done_counter.load(Ordering::Relaxed);
        (ctx.progress)(BuildProgress {
            files_done: done,
            files_total: ctx.total_files,
            bytes_hashed: hashed,
            bytes_total: ctx.total_bytes,
        });
    }

    let sha256 = format!("{:x}", hasher.finalize());
    let metadata = file.metadata()?;
    let identity = get_file_identity_from_file(&file);
    let modified_unix_ms = get_modified_unix_ms(&metadata);

    let done = ctx.files_done_counter.fetch_add(1, Ordering::Relaxed) + 1;
    let hashed = ctx.bytes_hashed_counter.load(Ordering::Relaxed);
    (ctx.progress)(BuildProgress {
        files_done: done,
        files_total: ctx.total_files,
        bytes_hashed: hashed,
        bytes_total: ctx.total_bytes,
    });

    Ok(BaseFile {
        path: rel_path.to_string(),
        size: file_size,
        sha256,
        linked,
        identity,
        modified_unix_ms,
    })
}

// ---------------------------------------------------------------------------
// Verify base
// ---------------------------------------------------------------------------

fn walk_base_dir(base_root: &Path, rel_dir: &Path, out: &mut Vec<String>) {
    let cur_dir = if rel_dir.as_os_str().is_empty() {
        base_root.to_path_buf()
    } else {
        base_root.join(rel_dir)
    };
    let Ok(read_dir) = std::fs::read_dir(&cur_dir) else {
        return;
    };
    for entry in read_dir.flatten() {
        let file_name = entry.file_name();
        let rel_path = rel_dir.join(&file_name);
        let abs_path = entry.path();
        let rel_str = rel_path.to_string_lossy().replace('\\', "/");

        let Ok(meta) = std::fs::symlink_metadata(&abs_path) else {
            continue;
        };

        if is_reparse_point_or_symlink(&meta) {
            out.push(rel_str);
            continue;
        }

        if meta.is_dir() {
            walk_base_dir(base_root, &rel_path, out);
        } else if meta.is_file() {
            out.push(rel_str);
        }
    }
}

pub fn verify_base(manifest: &BaseManifest, depth: VerifyDepth) -> BaseVerification {
    let mut problems = Vec::new();
    let mut checked = 0;
    let mut hashed = 0;

    if !manifest.location.exists() || !manifest.location.is_dir() {
        for file in &manifest.files {
            problems.push(BaseProblem {
                path: file.path.clone(),
                kind: ProblemKind::Missing,
            });
        }
        return BaseVerification {
            checked: manifest.files.len(),
            hashed: 0,
            problems,
        };
    }

    for file in &manifest.files {
        checked += 1;
        let file_path = manifest.location.join(&file.path);
        let meta = match std::fs::symlink_metadata(&file_path) {
            Ok(m) => m,
            Err(_) => {
                problems.push(BaseProblem {
                    path: file.path.clone(),
                    kind: ProblemKind::Missing,
                });
                continue;
            }
        };

        if !meta.is_file() {
            problems.push(BaseProblem {
                path: file.path.clone(),
                kind: ProblemKind::Missing,
            });
            continue;
        }

        let actual_size = meta.len();
        if actual_size != file.size {
            problems.push(BaseProblem {
                path: file.path.clone(),
                kind: ProblemKind::SizeChanged {
                    expected: file.size,
                    actual: actual_size,
                },
            });
            continue;
        }

        match depth {
            VerifyDepth::Quick => {
                let cur_identity = get_file_identity(&file_path);
                let cur_modified = get_modified_unix_ms(&meta);
                let identity_differs = file.identity.is_some() && cur_identity != file.identity;
                let modified_differs = cur_modified != file.modified_unix_ms;

                if identity_differs || modified_differs {
                    hashed += 1;
                    if let Ok(actual_hash) = hash_file(&file_path) {
                        if actual_hash != file.sha256 {
                            problems.push(BaseProblem {
                                path: file.path.clone(),
                                kind: ProblemKind::ContentChanged,
                            });
                        }
                    } else {
                        problems.push(BaseProblem {
                            path: file.path.clone(),
                            kind: ProblemKind::ContentChanged,
                        });
                    }
                }
            }
            VerifyDepth::Full => {
                hashed += 1;
                if let Ok(actual_hash) = hash_file(&file_path) {
                    if actual_hash != file.sha256 {
                        problems.push(BaseProblem {
                            path: file.path.clone(),
                            kind: ProblemKind::ContentChanged,
                        });
                    }
                } else {
                    problems.push(BaseProblem {
                        path: file.path.clone(),
                        kind: ProblemKind::ContentChanged,
                    });
                }
            }
        }
    }

    let manifest_file_set: HashSet<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();

    let mut actual_files = Vec::new();
    walk_base_dir(&manifest.location, Path::new(""), &mut actual_files);
    for actual_rel in actual_files {
        if !manifest_file_set.contains(actual_rel.as_str()) {
            problems.push(BaseProblem {
                path: actual_rel,
                kind: ProblemKind::Unexpected,
            });
        }
    }

    problems.sort_by(|a, b| a.path.cmp(&b.path));

    BaseVerification {
        checked,
        hashed,
        problems,
    }
}

// ---------------------------------------------------------------------------
// Listing and removing
// ---------------------------------------------------------------------------

pub fn list_bases(paths: &AppPaths) -> Vec<BaseListing> {
    let manifests_dir = paths.base_manifests_dir();
    let Ok(entries) = std::fs::read_dir(manifests_dir) else {
        return Vec::new();
    };
    let mut listings = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().map(|e| e == "json").unwrap_or(false) {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(manifest) = serde_json::from_str::<BaseManifest>(&content) {
                    let present = manifest.location.exists() && manifest.location.is_dir();
                    listings.push(BaseListing { manifest, present });
                }
            }
        }
    }
    listings.sort_by(|a, b| a.manifest.base_id.cmp(&b.manifest.base_id));
    listings
}

/// Whether `location` can only be a base folder Agora made for `base_id`: a
/// folder named after the base, directly inside a bases root, and nowhere near
/// the install it was built from. A manifest is a user-writable file, so
/// removal checks this rather than deleting whatever path it names.
fn is_own_base_folder(paths: &AppPaths, manifest: &BaseManifest, base_id: &str) -> bool {
    let location = &manifest.location;
    if manifest.base_id != base_id || location.file_name() != Some(std::ffi::OsStr::new(base_id)) {
        return false;
    }
    let Some(parent) = location.parent() else {
        return false;
    };
    let in_a_bases_root = parent == paths.bases_dir()
        || parent.file_name() == Some(std::ffi::OsStr::new("AgoraBases"));
    let overlaps_source = location.starts_with(&manifest.source_location)
        || manifest.source_location.starts_with(location);
    in_a_bases_root && !overlaps_source
}

pub fn remove_base(paths: &AppPaths, base_id: &str) -> Result<(), BaseError> {
    let manifest_path = paths.base_manifest_path(base_id);
    if !manifest_path.exists() {
        return Err(BaseError::NotFound(base_id.to_string()));
    }
    let content = std::fs::read_to_string(&manifest_path)?;
    let manifest: BaseManifest = serde_json::from_str(&content)?;
    if !is_own_base_folder(paths, &manifest, base_id) {
        return Err(BaseError::Other(format!(
            "refusing to remove {}: the manifest for base {base_id} does not point at a base folder Agora made",
            manifest.location.display()
        )));
    }
    if manifest.location.exists() {
        std::fs::remove_dir_all(&manifest.location)?;
    }
    let _ = std::fs::remove_file(&manifest_path);
    Ok(())
}
