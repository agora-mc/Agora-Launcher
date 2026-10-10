//! The swap: a tool that works on the real install's `Data` folder runs while that folder is a link
//! to the instance's farm (MASTER_SPEC §26.9, slice 4c).
//!
//! Skyrim's tools read the install path from the registry or their own config (the real Nemesis
//! logged `Data Directory: D:\...\data\`), so they must see the mods at the real path. For the length
//! of one run the real `Data` is renamed aside (`Data.agora-aside-<run>`) and replaced by a directory
//! junction to the farm's `Data`. When the run ends the junction is removed and the aside folder is
//! renamed back. Nothing is copied and nothing is deleted recursively.
//!
//! The discipline follows `game_user_files`: a journal is written, and made durable, before the real
//! folder is touched. A journal left by a crash is put back by the next check, which runs before any
//! launch, deploy, tool run or user-files session for that game, and at CLI start. Recovery never
//! merges folders and never overwrites: when it cannot tell what is where, it stops and says so.
//!
//! The journal lives at `<data>/tool-swaps/<game>_<store>.json`.

use std::io::Write;
use std::path::{Path, PathBuf};

use agora_game_api::GameDefinition;
use serde::{Deserialize, Serialize};

use crate::ctx::Ctx;
use crate::game_launch::{processes_running_from, RunningGameProcess};
use crate::lock_manager::LockResource;

/// The name prefix of the folder the real `Data` is renamed to during a run.
pub const ASIDE_PREFIX: &str = "Data.agora-aside-";
pub const JOURNAL_VERSION: u32 = 1;
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
}

/// What a swap changed, written before the change and removed only after the real folder is back.
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
    /// The real `Data` folder, which is a junction for the length of the run.
    pub real_data: PathBuf,
    /// Where the real `Data` was renamed to for the run.
    pub aside: PathBuf,
    /// The farm's `Data` folder that the junction points at.
    pub junction_target: PathBuf,
    /// The farm's game folder: a process running from here is a run that is still going.
    pub deployment_game_dir: PathBuf,
}

/// A journal found on disk, for `games tools-swap status`.
#[derive(Debug, Clone)]
pub struct PendingSwap {
    pub game: String,
    pub store: String,
    pub path: PathBuf,
    pub journal: Option<SwapJournal>,
    pub read_error: Option<String>,
    /// A process runs from the farm: the run is still going.
    pub running: bool,
}

/// What a startup or explicit recovery did for one journal.
#[derive(Debug, Clone)]
pub enum Recovery {
    Restored(Box<SwapJournal>),
    Stuck(String),
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

pub fn read_journal(path: &Path) -> Result<SwapJournal, SwapError> {
    let text = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&text)?)
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

#[cfg(windows)]
fn make_link(target: &Path, link: &Path) -> std::io::Result<()> {
    junction::create(target, link)
}

#[cfg(not(windows))]
fn make_link(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// Remove a junction only. The folder it points at is never touched. `junction::delete` removes the
/// reparse data and leaves an empty folder, which is then removed with the non-recursive
/// `remove_dir`: it refuses a folder that is not empty, so nothing is ever deleted with contents.
#[cfg(windows)]
fn remove_link(link: &Path) -> std::io::Result<()> {
    junction::delete(link)?;
    std::fs::remove_dir(link)
}

#[cfg(not(windows))]
fn remove_link(link: &Path) -> std::io::Result<()> {
    std::fs::remove_file(link)
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

fn processes_list(procs: &[RunningGameProcess]) -> String {
    procs
        .iter()
        .take(5)
        .map(|p| format!("pid {} ({})", p.pid, p.exe.display()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Refuse a swap before anything is touched (MASTER_SPEC §26.9): a process runs from the real install
/// or from the farm, the real `Data` is already a link, or the aside name is taken.
pub fn preflight(
    install_dir: &Path,
    deployment_game_dir: &Path,
    real_data: &Path,
    aside: &Path,
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
    for (folder, what) in [
        (install_dir, "the game's install folder"),
        (deployment_game_dir, "the instance's deployed game folder"),
    ] {
        let procs = processes_running_from(folder);
        if !procs.is_empty() {
            return Err(SwapError::Refused(format!(
                "a program is running from {what} ({}). Close the game, its launcher and Steam, then retry. Nothing was changed.",
                processes_list(&procs)
            )));
        }
    }
    Ok(())
}

/// Move the real `Data` aside and put a junction to the farm's `Data` in its place. The caller has
/// written the journal first, and restores with [`unswap`] if this fails part way.
pub fn engage(journal: &SwapJournal) -> Result<(), SwapError> {
    std::fs::rename(&journal.real_data, &journal.aside)?;
    std::fs::create_dir_all(&journal.junction_target)?;
    make_link(&journal.junction_target, &journal.real_data)?;
    Ok(())
}

/// Put the real `Data` back: remove the junction (only after checking it points at the recorded
/// farm), rename the aside folder back, and check that a real folder is in place. Never merges and
/// never deletes a folder. Does not remove the journal.
pub fn put_back(journal: &SwapJournal) -> Result<(), SwapError> {
    let real = &journal.real_data;
    let aside = &journal.aside;
    if is_reparse_point(real) {
        let ours = link_target(real)
            .map(|target| same_path(&target, &journal.junction_target))
            .unwrap_or(false);
        if !ours {
            return Err(SwapError::Stuck(format!(
                "the real Data folder {} is {}, not the instance's farm {}. Agora did not change it. Check where that link should point, remove it by hand if it is stale, rename {} to {} if the original is still there, then delete {}.",
                real.display(),
                describe_folder(real),
                journal.junction_target.display(),
                aside.display(),
                real.display(),
                journal_path_hint(journal)
            )));
        }
        if !aside.is_dir() {
            return Err(SwapError::Stuck(format!(
                "the real Data folder {} is a link to the farm, but the original Data folder {} is missing. The link was left in place. Nothing was merged or deleted.",
                real.display(),
                aside.display()
            )));
        }
        remove_link(real)?;
    }
    if real.exists() {
        if aside.exists() {
            return Err(SwapError::Stuck(both_present(journal)));
        }
    } else if aside.exists() {
        std::fs::rename(aside, real)?;
    } else {
        return Err(SwapError::Stuck(format!(
            "neither the real Data folder {} nor {} exists. Agora did not create or change anything. Check the install folder by hand, then delete {}.",
            real.display(),
            aside.display(),
            journal_path_hint(journal)
        )));
    }
    if !real.is_dir() || is_reparse_point(real) {
        return Err(SwapError::Stuck(format!(
            "after the run the real Data folder {} is not a normal folder. Agora left it as it is; check it by hand.",
            real.display()
        )));
    }
    Ok(())
}

fn journal_path_hint(journal: &SwapJournal) -> String {
    format!("the swap journal for {} ({})", journal.game, journal.store)
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

/// Put the real `Data` back and then remove the journal, last.
pub fn unswap(ctx: &Ctx, journal: &SwapJournal) -> Result<(), SwapError> {
    put_back(journal)?;
    let path = journal_path(ctx, &journal.game, &journal.store);
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

/// Recovery for one game and store, under the caller's user-files lock: a journal whose run is not
/// alive is put back. Returns the journal that was put back, if there was one.
/// A journal is a record on disk that recovery acts on, so its paths are only obeyed in the one
/// shape a swap produces: `Data` and its aside are siblings inside the install folder, the aside is
/// named for this run, and the junction points at `Data` inside this instance's deployment
/// (`…\deployments\<instance>\game`). A journal that names anything else could otherwise move
/// an arbitrary folder.
pub fn check_journal_shape(journal: &SwapJournal) -> Result<(), String> {
    fn name_of(p: &Path) -> String {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
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
    if !journal
        .real_data
        .parent()
        .is_some_and(|p| same_path(p, &journal.install_dir))
        || !name_of(&journal.real_data).eq_ignore_ascii_case("Data")
    {
        return Err("its Data folder is not the install folder's Data".into());
    }
    if !journal
        .aside
        .parent()
        .is_some_and(|p| same_path(p, &journal.install_dir))
        || !name_of(&journal.aside).eq_ignore_ascii_case(&format!("{ASIDE_PREFIX}{}", journal.run))
    {
        return Err("its aside folder is not Data's sibling named for this run".into());
    }
    let game_dir = &journal.deployment_game_dir;
    let instance_dir = game_dir.parent();
    let deployments = instance_dir.and_then(Path::parent);
    if !name_of(game_dir).eq_ignore_ascii_case("game")
        || !instance_dir.is_some_and(|d| name_of(d) == journal.instance_id)
        || !deployments.is_some_and(|d| name_of(d).eq_ignore_ascii_case("deployments"))
    {
        return Err("its farm is not this instance's deployment folder".into());
    }
    if !same_path(&journal.junction_target, &game_dir.join("Data")) {
        return Err("its junction target is not the farm's Data folder".into());
    }
    Ok(())
}

pub fn recover_store_unlocked(
    ctx: &Ctx,
    game: &str,
    store: &str,
) -> Result<Option<SwapJournal>, SwapError> {
    let path = journal_path(ctx, game, store);
    if !path.exists() {
        return Ok(None);
    }
    let journal = match read_journal(&path) {
        Ok(journal) => journal,
        Err(e) => {
            return Err(SwapError::Stuck(format!(
                "the swap journal {} is unreadable ({e}), so Agora cannot tell what it changed. Nothing was changed. Find the real install's Data folder and any Data.agora-aside-* folder next to it, and put them back by hand, then delete the journal.",
                path.display()
            )));
        }
    };
    if journal.version != JOURNAL_VERSION {
        return Err(SwapError::Stuck(format!(
            "the swap journal {} has version {}, which this Agora does not read. Nothing was changed.",
            path.display(),
            journal.version
        )));
    }
    if let Err(why) = check_journal_shape(&journal) {
        return Err(SwapError::Stuck(format!(
            "the swap journal {} does not describe a swap Agora makes ({why}), so Agora will not act on it. Nothing was changed. Check the install's Data folder and any Data.agora-aside-* folder next to it by hand, then delete the journal.",
            path.display()
        )));
    }
    let procs = processes_running_from(&journal.deployment_game_dir);
    if !procs.is_empty() {
        return Err(SwapError::Refused(format!(
            "a tool run for instance {} is still going ({}). Let it finish or close it, then retry. Nothing was changed.",
            journal.instance_id,
            processes_list(&procs)
        )));
    }
    unswap(ctx, &journal)?;
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
                    let running = !processes_running_from(&journal.deployment_game_dir).is_empty();
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

    // The journal names Windows paths, and the swap it describes only runs on Windows.
    #[cfg(windows)]
    fn good_journal() -> SwapJournal {
        SwapJournal {
            version: JOURNAL_VERSION,
            game: "skyrim-se".into(),
            store: "steam".into(),
            instance_id: "inst-1".into(),
            tool: "nemesis".into(),
            run: "123-abc".into(),
            started_unix_ms: 0,
            install_dir: PathBuf::from(r"D:\Games\Skyrim"),
            real_data: PathBuf::from(r"D:\Games\Skyrim\Data"),
            aside: PathBuf::from(r"D:\Games\Skyrim\Data.agora-aside-123-abc"),
            junction_target: PathBuf::from(r"E:\Bases\deployments\inst-1\game\Data"),
            deployment_game_dir: PathBuf::from(r"E:\Bases\deployments\inst-1\game"),
        }
    }

    #[cfg(windows)]
    type Tamper = Box<dyn Fn(&mut SwapJournal)>;

    #[cfg(windows)]
    #[test]
    fn only_a_journal_in_the_shape_a_swap_makes_is_obeyed() {
        assert_eq!(check_journal_shape(&good_journal()), Ok(()));
        let tampered: Vec<(&str, Tamper)> = vec![
            (
                "aside elsewhere",
                Box::new(|j| j.aside = PathBuf::from(r"C:\Users\me\Documents")),
            ),
            (
                "aside wrong name",
                Box::new(|j| j.aside = PathBuf::from(r"D:\Games\Skyrim\Saves")),
            ),
            (
                "aside other run",
                Box::new(|j| j.aside = PathBuf::from(r"D:\Games\Skyrim\Data.agora-aside-999")),
            ),
            (
                "data elsewhere",
                Box::new(|j| j.real_data = PathBuf::from(r"C:\Windows\Data")),
            ),
            (
                "data not Data",
                Box::new(|j| j.real_data = PathBuf::from(r"D:\Games\Skyrim\Saves")),
            ),
            ("run with a path", Box::new(|j| j.run = r"..\x".into())),
            ("empty run", Box::new(|j| j.run = String::new())),
            (
                "target outside farm",
                Box::new(|j| j.junction_target = PathBuf::from(r"C:\Windows")),
            ),
            (
                "farm not a deployment",
                Box::new(|j| {
                    j.deployment_game_dir = PathBuf::from(r"C:\Windows\inst-1\game");
                    j.junction_target = PathBuf::from(r"C:\Windows\inst-1\game\Data");
                }),
            ),
            (
                "farm of another instance",
                Box::new(|j| {
                    j.deployment_game_dir = PathBuf::from(r"E:\Bases\deployments\other\game");
                    j.junction_target = PathBuf::from(r"E:\Bases\deployments\other\game\Data");
                }),
            ),
        ];
        for (what, change) in tampered {
            let mut j = good_journal();
            change(&mut j);
            assert!(check_journal_shape(&j).is_err(), "{what} must be refused");
        }
    }
}
