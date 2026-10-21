//! The swap: a tool that works on the real install's `Data` folder runs on a real, writable copy of
//! it (MASTER_SPEC §26.9, slice 4e).
//!
//! Skyrim's tools read the install path from the registry or their own config (the real Nemesis
//! logged `Data Directory: D:\...\data\`), and some open files for writing that a link cannot take.
//! So for the length of one run the real `Data` is renamed aside (`Data.agora-aside-<run>`), the
//! copy is renamed into its place, and the tool runs. When it ends the copy is renamed back and the
//! real `Data` is renamed back. The real `Data` is never written into, only renamed. The copy itself
//! and where it lives are in `game_tool_copy`.
//!
//! The discipline follows `game_user_files`: a journal is written, and made durable, before anything
//! is renamed or copied. A journal left by a crash is put back by the next check, which runs before any
//! launch, deploy, tool run or user-files session for that game, and at CLI start. Recovery discards the
//! run's writes rather than capturing them: the copy is deleted, or moved home and deleted. It never
//! merges folders and never overwrites. It deletes a folder only when that folder's marker matches the
//! journal and its path is one the swap derives from trusted inputs (the instance's base and install,
//! never the journal's own paths).
//!
//! The journal lives at `<data>/tool-swaps/<game>_<store>.json`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use agora_game_api::{BaseReference, GameDefinition};
use serde::{Deserialize, Serialize};

use crate::ctx::Ctx;
use crate::game_base::BaseManifest;
use crate::game_launch::{processes_running_from, RunningGameProcess};
use crate::game_tool_copy::{self, CopyKind};
use crate::lock_manager::LockResource;

/// The name prefix of the folder the real `Data` is renamed to during a run.
pub const ASIDE_PREFIX: &str = "Data.agora-aside-";
/// The name prefix of a per-run copy on the install's volume (`<install>\Data.agora-copy-<run>`).
pub const COPY_PREFIX: &str = "Data.agora-copy-";
/// Journal version 2 (junction swaps were version 1; recovery refuses those).
pub const JOURNAL_VERSION: u32 = 2;
#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;

#[derive(Debug, thiserror::Error)]
pub enum SwapError {
    /// A run was refused before anything changed, or recovery must wait for a run that is still going.
    #[error("{0}")]
    Refused(String),
    /// Recovery found something it will not change, and says exactly what is where.
    #[error("{0}")]
    Stuck(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("lock error: {0}")]
    Lock(#[from] crate::error::LauncherError),
    /// Something the swap needed could not be read or made, with the reason.
    #[error("{0}")]
    Other(String),
}

/// What a swap changed, written before the first change and removed only after the real `Data` is
/// back (version 2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwapJournal {
    pub version: u32,
    pub game: String,
    pub store: String,
    pub instance_id: String,
    pub tool: String,
    pub run: String,
    pub started_unix_ms: i64,
    /// The real install folder, whose `Data` is swapped.
    pub install_dir: PathBuf,
    /// The real `Data` folder.
    pub real_data: PathBuf,
    /// Where the real `Data` is renamed to for the run.
    pub aside: PathBuf,
    /// Whether the copy is kept between runs or made for this run only.
    pub copy_kind: CopyKind,
    /// The copy's folder: `<swap>\Data` when kept, `<install>\Data.agora-copy-<run>` when per run.
    pub copy_dir: PathBuf,
    /// The instance's swap folder under its base's deployments folder.
    pub swap_dir: PathBuf,
}

/// A journal found on disk, for `games tools-swap status`.
#[derive(Debug, Clone)]
pub struct PendingSwap {
    pub game: String,
    pub store: String,
    pub path: PathBuf,
    pub journal: Option<SwapJournal>,
    pub read_error: Option<String>,
    /// A process runs from the install folder: the run is still going.
    pub running: bool,
}

/// What a startup or explicit recovery did for one journal.
#[derive(Debug, Clone)]
pub enum Recovery {
    Restored(Box<SwapJournal>),
    Stuck(String),
}

/// Where a swap of one instance goes. Built from trusted inputs only: the install folder and the
/// deployments folder come from the instance's pinned base manifest, never from a journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwapLocation {
    pub install: PathBuf,
    /// `<bases>\deployments\<instance>\swap`.
    pub swap_dir: PathBuf,
    pub base_id: String,
    pub store: String,
}

/// A test seam: overrides the volume check and the free space for every swap in this process, until
/// it is set back to `None`. Production never sets it.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VolumeSeam {
    pub same_volume: Option<bool>,
    pub free_bytes: Option<u64>,
}

static SEAM: Mutex<Option<VolumeSeam>> = Mutex::new(None);

/// Set or clear the volume seam (see [`VolumeSeam`]). Tests that set it must run one at a time.
#[doc(hidden)]
pub fn set_volume_seam(seam: Option<VolumeSeam>) {
    if let Ok(mut guard) = SEAM.lock() {
        *guard = seam;
    }
}

fn seam() -> VolumeSeam {
    SEAM.lock()
        .ok()
        .and_then(|guard| *guard)
        .unwrap_or_default()
}

/// The journal file of one game and store.
pub fn journal_path(ctx: &Ctx, game: &str, store: &str) -> PathBuf {
    ctx.paths.tool_swap_journal_path(game, store)
}

/// Write a journal durably: the bytes are on disk (`sync_all`) before this returns, and the file
/// appears under its final name only once they are.
pub fn write_journal(ctx: &Ctx, journal: &SwapJournal) -> Result<(), SwapError> {
    let path = journal_path(ctx, &journal.game, &journal.store);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let data = serde_json::to_vec_pretty(journal)?;
    {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(&data)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Remove a journal, the last step of a swap.
pub fn remove_journal(ctx: &Ctx, journal: &SwapJournal) -> Result<(), SwapError> {
    let path = journal_path(ctx, &journal.game, &journal.store);
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

/// Read a journal. A version 1 journal (a junction swap, which this Agora no longer makes or reads) is
/// refused by name, and nothing is changed.
pub fn read_journal(path: &Path) -> Result<SwapJournal, SwapError> {
    let text = std::fs::read_to_string(path)?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
        SwapError::Stuck(format!(
            "the swap journal {} is unreadable ({e}), so Agora cannot tell what it changed. Nothing was changed. Find the real install's Data folder and any Data.agora-aside-* or Data.agora-copy-* folder next to it, and put them back by hand, then delete the journal.",
            path.display()
        ))
    })?;
    match value.get("version").and_then(serde_json::Value::as_u64) {
        Some(1) => {
            let field = |name: &str| {
                value
                    .get(name)
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("(unknown)")
                    .to_string()
            };
            Err(SwapError::Stuck(format!(
                "the swap journal {} is version 1: a junction swap, which this Agora no longer makes or reads. Nothing was changed. If {} is a link, remove the link by hand; if {} is the original folder, rename it back to {}, and then delete the journal.",
                path.display(),
                field("real_data"),
                field("aside"),
                field("real_data")
            )))
        }
        Some(v) if v == u64::from(JOURNAL_VERSION) => {
            serde_json::from_value(value).map_err(|e| {
                SwapError::Stuck(format!(
                    "the swap journal {} is unreadable ({e}). Nothing was changed.",
                    path.display()
                ))
            })
        }
        other => Err(SwapError::Stuck(format!(
            "the swap journal {} has version {}, which this Agora does not read. Nothing was changed.",
            path.display(),
            other.map(|v| v.to_string()).unwrap_or_else(|| "none".into())
        ))),
    }
}

/// Whether a path is a junction, symbolic link or other reparse point. A missing path is not one.
pub fn is_reparse_point(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => is_reparse_meta(&meta),
        Err(_) => false,
    }
}

#[cfg(windows)]
fn is_reparse_meta(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_meta(meta: &std::fs::Metadata) -> bool {
    meta.file_type().is_symlink()
}

#[cfg(windows)]
fn link_target(link: &Path) -> Option<PathBuf> {
    junction::get_target(link).ok()
}

#[cfg(not(windows))]
fn link_target(link: &Path) -> Option<PathBuf> {
    std::fs::read_link(link).ok()
}

/// Compare two paths the way Windows does: case-insensitively, without the `\\?\` prefix.
pub fn same_path(a: &Path, b: &Path) -> bool {
    fn norm(p: &Path) -> String {
        let s = p.to_string_lossy();
        let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
        s.replace('/', "\\")
            .trim_end_matches('\\')
            .to_ascii_lowercase()
    }
    norm(a) == norm(b)
}

/// What a path is, in words for a message.
pub fn describe_folder(path: &Path) -> String {
    if is_reparse_point(path) {
        match link_target(path) {
            Some(target) => format!("a link to {}", target.display()),
            None => "a link whose target Agora cannot read".to_string(),
        }
    } else if path.is_dir() {
        format!("a folder with {} file(s)", count_files(path))
    } else if path.exists() {
        "a file".to_string()
    } else {
        "missing".to_string()
    }
}

/// The files under a folder, recursively. Only used to describe a folder in a message.
fn count_files(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| {
            let path = entry.path();
            match std::fs::symlink_metadata(&path) {
                Ok(meta) if meta.is_dir() && !is_reparse_meta(&meta) => count_files(&path),
                Ok(_) => 1,
                Err(_) => 0,
            }
        })
        .sum()
}

/// The nearest folder that exists: the path itself, or the first ancestor that does.
fn nearest_existing(path: &Path) -> Option<PathBuf> {
    path.ancestors().find(|p| p.exists()).map(Path::to_path_buf)
}

/// The mount root of the volume a path is on, lower-cased, or `None` when it cannot be read.
fn volume_key(path: &Path) -> Option<String> {
    let probe = nearest_existing(path)?;
    crate::game_discovery::volume::get_volume_mount_root(&probe)
        .map(|root| root.to_string_lossy().to_ascii_lowercase())
}

/// Whether two paths are on the same volume. An unknown volume is treated as a different one, which
/// is the choice that always works (a per-run copy needs no rename across volumes).
pub fn same_volume(a: &Path, b: &Path) -> bool {
    if let Some(same) = seam().same_volume {
        return same;
    }
    match (volume_key(a), volume_key(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// The free bytes available to this user on the volume of `path`, or `None` when they cannot be read.
#[cfg(windows)]
pub fn available_bytes(path: &Path) -> Option<u64> {
    if let Some(free) = seam().free_bytes {
        return Some(free);
    }
    use std::os::windows::ffi::OsStrExt;
    let probe = nearest_existing(path)?;
    let wide: Vec<u16> = probe
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut free_to_caller: u64 = 0;
    let mut total: u64 = 0;
    let mut total_free: u64 = 0;
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free_to_caller,
            &mut total,
            &mut total_free,
        )
    };
    (ok != 0).then_some(free_to_caller)
}

#[cfg(not(windows))]
pub fn available_bytes(path: &Path) -> Option<u64> {
    seam().free_bytes.filter(|_| path.exists())
}

/// Bytes in words for a message: "12.4 GB", "830 MB", "512 bytes".
pub fn human_bytes(bytes: u64) -> String {
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.0} MB", b / MB)
    } else {
        format!("{bytes} bytes")
    }
}

/// Whether Steam's client is running. Steam updating the game during a swap is a warning, not a refusal.
pub fn steam_running() -> bool {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes();
    sys.processes()
        .values()
        .any(|process| process.name().eq_ignore_ascii_case("steam.exe"))
}

fn processes_list(procs: &[RunningGameProcess]) -> String {
    procs
        .iter()
        .take(5)
        .map(|p| format!("pid {} ({})", p.pid, p.exe.display()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The swap's location for one instance, from trusted inputs: the instance's pinned base manifest gives
/// the install folder (its source) and the deployments folder (beside the base's own folder).
pub fn swap_location(ctx: &Ctx, instance_id: &str) -> Result<SwapLocation, SwapError> {
    let manifest = crate::game_instance::get_manifest(ctx, instance_id).map_err(|e| {
        SwapError::Other(format!(
            "instance '{instance_id}' cannot be read ({e}), so its swap location is unknown"
        ))
    })?;
    let BaseReference::Pinned { id: base_id, .. } = manifest.base else {
        return Err(SwapError::Refused(format!(
            "instance '{instance_id}' runs from its install folder, so there is no copy of Data to swap."
        )));
    };
    let base_path = ctx.paths.base_manifest_path(&base_id);
    let text = std::fs::read_to_string(&base_path).map_err(|e| {
        SwapError::Other(format!(
            "base '{base_id}' of instance '{instance_id}' cannot be read ({e})"
        ))
    })?;
    let base: BaseManifest = serde_json::from_str(&text)
        .map_err(|e| SwapError::Other(format!("base manifest '{base_id}' is unreadable: {e}")))?;
    let bases_root = base
        .location
        .parent()
        .ok_or_else(|| SwapError::Other(format!("base '{base_id}' has no parent folder")))?;
    Ok(SwapLocation {
        install: base.source_location.clone(),
        swap_dir: bases_root
            .join("deployments")
            .join(instance_id)
            .join("swap"),
        base_id,
        store: base.runtime.store.as_str().to_string(),
    })
}

/// The space a copy needs, and the folder whose volume must hold it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpaceNeed {
    pub folder: PathBuf,
    pub bytes: u64,
}

/// Refuse a swap before anything is created or moved (MASTER_SPEC §26.9): the real `Data` is already a
/// link, is missing, or the aside name is taken; a program runs from the install folder; or the copy's
/// volume lacks the space for what must be copied. Nothing is changed in any case.
pub fn preflight(
    install_dir: &Path,
    real_data: &Path,
    aside: &Path,
    space: &SpaceNeed,
) -> Result<(), SwapError> {
    if is_reparse_point(real_data) {
        return Err(SwapError::Refused(format!(
            "the real Data folder {} is already {}, so it cannot be swapped. Nothing was changed.",
            real_data.display(),
            describe_folder(real_data)
        )));
    }
    if !real_data.is_dir() {
        return Err(SwapError::Refused(format!(
            "the real Data folder {} is missing, so there is nothing to swap. Nothing was changed.",
            real_data.display()
        )));
    }
    if std::fs::symlink_metadata(aside).is_ok() {
        return Err(SwapError::Refused(format!(
            "{} already exists, so the real Data folder cannot be moved aside there. Nothing was changed; move or remove it, then retry.",
            aside.display()
        )));
    }
    let procs = processes_running_from(install_dir);
    if !procs.is_empty() {
        return Err(SwapError::Refused(format!(
            "a program is running from the game's install folder ({}). Close the game, its launcher and Steam, then retry. Nothing was changed.",
            processes_list(&procs)
        )));
    }
    match available_bytes(&space.folder) {
        None => Err(SwapError::Refused(format!(
            "Agora could not read the free space on {}, so it will not copy Data there. Nothing was changed.",
            space.folder.display()
        ))),
        Some(free) if free < space.bytes => Err(SwapError::Refused(format!(
            "the copy of Data needs {} on the volume of {}, but only {} is free. Nothing was changed. Free at least {} and retry.",
            human_bytes(space.bytes),
            space.folder.display(),
            human_bytes(free),
            human_bytes(space.bytes - free)
        ))),
        Some(_) => Ok(()),
    }
}

/// Steps 4 and 5 of a run: the real `Data` is renamed aside, and the copy is renamed into its place.
/// The caller has written the journal, and runs [`restore_and_discard`] if this fails part way.
pub fn engage(journal: &SwapJournal) -> Result<(), SwapError> {
    std::fs::rename(&journal.real_data, &journal.aside)?;
    std::fs::rename(&journal.copy_dir, &journal.real_data)?;
    Ok(())
}

/// Whether the folder at the real `Data` path is the copy this journal made (its marker matches).
pub fn copy_is_in_place(journal: &SwapJournal) -> bool {
    game_tool_copy::marker_matches(
        &journal.real_data,
        &journal.instance_id,
        journal.copy_kind,
        &journal.run,
    )
}

/// Steps 7 and 8 of a run: the copy goes back to where it came from, and the real `Data` comes back
/// from the aside. Never merges and never deletes. Nothing is changed when the folders are not what a
/// swap of this journal left.
pub fn disengage(journal: &SwapJournal) -> Result<(), SwapError> {
    if !copy_is_in_place(journal) {
        return Err(SwapError::Stuck(format!(
            "the real Data folder {} is {}, not the copy this run made, so Agora did not change it. Nothing was merged or deleted. Check it by hand, then delete the swap journal for {} ({}).",
            journal.real_data.display(),
            describe_folder(&journal.real_data),
            journal.game,
            journal.store
        )));
    }
    if std::fs::symlink_metadata(&journal.copy_dir).is_ok() {
        return Err(SwapError::Stuck(format!(
            "both the copy's home {} and the copy in place of Data exist, so Agora cannot move the copy home. Nothing was changed.",
            journal.copy_dir.display()
        )));
    }
    if !journal.aside.is_dir() {
        return Err(SwapError::Stuck(format!(
            "the original Data folder {} is missing, so the real Data cannot come back. The copy was left in place. Nothing was merged or deleted.",
            journal.aside.display()
        )));
    }
    std::fs::rename(&journal.real_data, &journal.copy_dir)?;
    std::fs::rename(&journal.aside, &journal.real_data)?;
    Ok(())
}

fn both_present(journal: &SwapJournal) -> String {
    format!(
        "both {} ({}) and {} ({}) exist, so Agora cannot tell which one is the game's Data folder. Nothing was changed and nothing was merged. {} is the Data folder as it was before the tool run; {} is what is there now. To fix it by hand: check both, move the one you do not want out of the way, rename {} to {}, then delete the swap journal for {} ({}).",
        journal.real_data.display(),
        describe_folder(&journal.real_data),
        journal.aside.display(),
        describe_folder(&journal.aside),
        journal.aside.display(),
        journal.real_data.display(),
        journal.aside.display(),
        journal.real_data.display(),
        journal.game,
        journal.store,
    )
}

/// Put the real `Data` back, whatever state a swap was left in, and discard the copy's writes. The
/// states are the ones a crash can leave (MASTER_SPEC §26.9, decision 5):
/// - the real `Data` is in place and nothing was renamed aside: the copy is discarded;
/// - the copy is in place of `Data` (its marker matches): it is moved home, then the aside comes back,
///   then the copy is discarded;
/// - the aside holds the real `Data` and nothing is in place: the aside comes back, then the copy is
///   discarded.
///
/// Anything else is `Stuck`: nothing is merged, overwritten, or deleted. Does not remove the journal.
pub fn restore_and_discard(journal: &SwapJournal) -> Result<(), SwapError> {
    let real = &journal.real_data;
    let aside = &journal.aside;
    if is_reparse_point(real) {
        return Err(SwapError::Stuck(format!(
            "the real Data folder {} is {}. A swap now makes a copy, not a link, so Agora did not change it. Check it by hand, then delete the swap journal for {} ({}).",
            real.display(),
            describe_folder(real),
            journal.game,
            journal.store
        )));
    }
    // A folder at the copy's home that is not the copy the journal made is refused before anything moves.
    if std::fs::symlink_metadata(&journal.copy_dir).is_ok()
        && !game_tool_copy::marker_matches(
            &journal.copy_dir,
            &journal.instance_id,
            journal.copy_kind,
            &journal.run,
        )
    {
        return Err(SwapError::Stuck(format!(
            "{} is not Agora's copy of Data for instance {} (its marker does not match the swap journal). Nothing was moved or deleted. Check it by hand, then delete the swap journal for {} ({}).",
            journal.copy_dir.display(),
            journal.instance_id,
            journal.game,
            journal.store
        )));
    }
    if real.is_dir() && !copy_is_in_place(journal) {
        if aside.exists() {
            return Err(SwapError::Stuck(both_present(journal)));
        }
        return game_tool_copy::discard_copy(journal);
    }
    if copy_is_in_place(journal) {
        if !aside.exists() {
            return Err(SwapError::Stuck(format!(
                "the copy is in place of the real Data folder {}, but the original {} is missing. Nothing was changed or deleted. Check both by hand, then delete the swap journal for {} ({}).",
                real.display(),
                aside.display(),
                journal.game,
                journal.store
            )));
        }
        if std::fs::symlink_metadata(&journal.copy_dir).is_ok() {
            return Err(SwapError::Stuck(format!(
                "both the copy's home {} and the copy in place of Data exist. Nothing was changed or deleted.",
                journal.copy_dir.display()
            )));
        }
        std::fs::rename(real, &journal.copy_dir)?;
    } else if real.exists() {
        return Err(SwapError::Stuck(format!(
            "the real Data path {} is {}, which Agora does not recognise. Nothing was changed. Check it by hand.",
            real.display(),
            describe_folder(real)
        )));
    }
    if !aside.exists() {
        return Err(SwapError::Stuck(format!(
            "neither the real Data folder {} nor {} exists. Agora did not create or change anything. Check the install folder by hand, then delete the swap journal for {} ({}).",
            real.display(),
            aside.display(),
            journal.game,
            journal.store
        )));
    }
    std::fs::rename(aside, real)?;
    game_tool_copy::discard_copy(journal)
}

/// Whether the journal's paths are the shape a swap of its instance makes, checked against the
/// instance's trusted location. A journal naming anything else is not acted on.
pub fn check_journal_shape(journal: &SwapJournal, loc: &SwapLocation) -> Result<(), String> {
    let run_ok = !journal.run.is_empty()
        && journal
            .run
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if !run_ok {
        return Err(format!(
            "its run id '{}' is not one Agora writes",
            journal.run
        ));
    }
    if !same_path(&journal.install_dir, &loc.install) {
        return Err("its install folder is not the instance's install folder".into());
    }
    if !same_path(&journal.real_data, &loc.install.join("Data")) {
        return Err("its Data folder is not the install folder's Data".into());
    }
    if !same_path(
        &journal.aside,
        &loc.install.join(format!("{ASIDE_PREFIX}{}", journal.run)),
    ) {
        return Err("its aside folder is not Data's sibling named for this run".into());
    }
    if !same_path(&journal.swap_dir, &loc.swap_dir) {
        return Err("its swap folder is not the instance's swap folder".into());
    }
    let expected = game_tool_copy::copy_dir_for(journal.copy_kind, loc, &journal.run);
    if !same_path(&journal.copy_dir, &expected) {
        return Err(format!(
            "its copy folder is not where a {} copy goes",
            journal.copy_kind.as_str()
        ));
    }
    Ok(())
}

/// Recovery for one game and store, under the caller's user-files lock: a journal whose run is not
/// alive is put back. Returns the journal that was put back, if there was one.
pub fn recover_store_unlocked(
    ctx: &Ctx,
    game: &str,
    store: &str,
) -> Result<Option<SwapJournal>, SwapError> {
    let path = journal_path(ctx, game, store);
    if !path.exists() {
        return Ok(None);
    }
    let journal = read_journal(&path)?;
    let loc = swap_location(ctx, &journal.instance_id).map_err(|e| {
        SwapError::Stuck(format!(
            "the swap journal {} names instance {}, whose location Agora cannot read ({e}). Nothing was changed.",
            path.display(),
            journal.instance_id
        ))
    })?;
    if let Err(why) = check_journal_shape(&journal, &loc) {
        return Err(SwapError::Stuck(format!(
            "the swap journal {} does not describe a swap Agora makes ({why}), so Agora will not act on it. Nothing was changed. Check the install's Data folder and any Data.agora-aside-* or Data.agora-copy-* folder next to it by hand, then delete the journal.",
            path.display()
        )));
    }
    let procs = processes_running_from(&journal.install_dir);
    if !procs.is_empty() {
        return Err(SwapError::Refused(format!(
            "a tool run for instance {} is still going ({}). Let it finish or close it, then retry. Nothing was changed.",
            journal.instance_id,
            processes_list(&procs)
        )));
    }
    restore_and_discard(&journal)?;
    remove_journal(ctx, &journal)?;
    Ok(Some(journal))
}

/// The per-game check: for each store of the game with a journal, take the game's user-files lock
/// and put back an interrupted swap. One file existence test per store when nothing is pending.
pub fn recover_game(ctx: &Ctx, definition: &GameDefinition) -> Result<Vec<SwapJournal>, SwapError> {
    let mut restored = Vec::new();
    for entry in &definition.stores {
        let store = entry.store.as_str();
        if !journal_path(ctx, definition.id.as_str(), store).exists() {
            continue;
        }
        let _lock = ctx.lock_manager.acquire(
            LockResource::GameUserFiles(definition.id.clone(), entry.store.clone()),
            "tool-swap-recover",
        )?;
        if let Some(journal) = recover_store_unlocked(ctx, definition.id.as_str(), store)? {
            restored.push(journal);
        }
    }
    Ok(restored)
}

/// The startup check: recover every game the registry knows. Each result is reported, never raised.
pub fn recover_all(ctx: &Ctx) -> Vec<Recovery> {
    let mut out = Vec::new();
    for definition in ctx.games.games() {
        match recover_game(ctx, definition) {
            Ok(restored) => out.extend(
                restored
                    .into_iter()
                    .map(|journal| Recovery::Restored(Box::new(journal))),
            ),
            Err(e) => out.push(Recovery::Stuck(e.to_string())),
        }
    }
    out
}

/// Every journal the registry's games and stores have, for `games tools-swap status`.
pub fn pending(ctx: &Ctx) -> Vec<PendingSwap> {
    let mut out = Vec::new();
    for definition in ctx.games.games() {
        for entry in &definition.stores {
            let store = entry.store.as_str();
            let path = journal_path(ctx, definition.id.as_str(), store);
            if !path.exists() {
                continue;
            }
            match read_journal(&path) {
                Ok(journal) => {
                    let running = !processes_running_from(&journal.install_dir).is_empty();
                    out.push(PendingSwap {
                        game: definition.id.to_string(),
                        store: store.to_string(),
                        path,
                        journal: Some(journal),
                        read_error: None,
                        running,
                    });
                }
                Err(e) => out.push(PendingSwap {
                    game: definition.id.to_string(),
                    store: store.to_string(),
                    path,
                    journal: None,
                    read_error: Some(e.to_string()),
                    running: false,
                }),
            }
        }
    }
    out
}

/// The `LockResource` a swap of an instance holds for its own changes, for callers outside a run.
pub(crate) fn instance_lock(instance_id: &str) -> LockResource {
    LockResource::Instance(instance_id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_compare_without_case_or_the_extended_prefix() {
        assert!(same_path(
            Path::new(r"\\?\C:\Games\Data\"),
            Path::new(r"c:/games/data")
        ));
        assert!(!same_path(
            Path::new(r"C:\Games\Data"),
            Path::new(r"C:\Games\Data2")
        ));
    }

    #[test]
    fn human_bytes_says_gb_mb_or_bytes() {
        assert_eq!(human_bytes(512), "512 bytes");
        assert_eq!(human_bytes(3 * 1024 * 1024), "3 MB");
        assert_eq!(
            human_bytes(5 * 1024 * 1024 * 1024 + 512 * 1024 * 1024),
            "5.5 GB"
        );
    }
}
