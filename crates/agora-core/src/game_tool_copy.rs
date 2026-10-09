//! The copy of the instance's `Data` folder that a swap runs its tool on (MASTER_SPEC §26.9, slice 4e).
//!
//! The copy is a real, writable copy of every file the instance's plan puts under `Data/`. It is planned
//! and written by `game_deploy` (`plan` in copies mode, then the same executor a copies deployment uses),
//! with the `Data/` prefix stripped. Where it lives depends on the volume:
//! - **same volume** as the install: `deployments\<instance>\swap\Data`, kept between runs. Each run first
//!   brings it up to the plan, so only what changed is copied; the first run copies everything.
//! - **another volume**: `<install>\Data.agora-copy-<run>`, made for one run and deleted afterwards. Every
//!   run copies everything in, and the changes come back out.
//!
//! Every copy holds `.agora-swap-copy.json` at its root, which says whose it is. A folder is deleted only
//! when that marker matches the journal (or, for `remove-copy`, the instance). The kept copy's record,
//! `swap-record.json`, sits beside it in the swap folder: what the copy held after the last plan, file by
//! file, with the time of each copy and of each source, so the next run can tell what changed without
//! reading the files.
//!
//! Change detection for the capture is `game_deploy::farm_file_differs`, the one link capture uses.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use agora_game_api::{GameDefinition, RelPath};
use serde::{Deserialize, Serialize};

use crate::content_store::protect;
use crate::ctx::Ctx;
use crate::game_base::get_modified_unix_ms;
use crate::game_deploy::{
    self, DeployMode, DeployedFileRecord, DeploymentRecord, FileSource, Placement, PlannedFile,
};
use crate::game_tool_swap::{self, SpaceNeed, SwapError, SwapJournal, SwapLocation};

/// The marker at the root of every copy. Capture ignores exactly this file.
pub const MARKER_FILE: &str = ".agora-swap-copy.json";
/// The kept copy's record, beside the copy in the swap folder.
pub const RECORD_FILE: &str = "swap-record.json";
/// The copy's name inside the swap folder, and the run name of a kept copy's marker.
const KEPT_DIR: &str = "Data";
const KEPT_RUN: &str = "kept";
const FORMAT: u32 = 1;

/// Whether a copy is kept between runs or made for one run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CopyKind {
    Kept,
    PerRun,
}

impl CopyKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CopyKind::Kept => "kept",
            CopyKind::PerRun => "per_run",
        }
    }

    /// What the marker's `run` names: `kept` for a kept copy, the run id for a per-run copy.
    fn marker_run(self, run: &str) -> &str {
        match self {
            CopyKind::Kept => KEPT_RUN,
            CopyKind::PerRun => run,
        }
    }
}

/// The marker written at the root of a copy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CopyMarker {
    pub format: u32,
    pub instance_id: String,
    pub kind: CopyKind,
    pub run: String,
}

/// What a copy held after its last plan: one entry per planned file, paths relative to `Data`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CopyRecord {
    pub format: u32,
    pub instance_id: String,
    pub kind: CopyKind,
    pub files: Vec<CopyRecordFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CopyRecordFile {
    pub path: RelPath,
    pub source: FileSource,
    pub size: u64,
    /// The copy's own time, as it was after the copy was made.
    pub modified_unix_ms: i64,
    /// The source's time when the copy was made.
    pub source_modified_unix_ms: i64,
}

/// The files and bytes a copy step wrote into the copy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CopyWork {
    pub files: usize,
    pub bytes: u64,
}

/// Where a swap's copy goes and what it needs, decided from trusted inputs and the instance's plan.
#[derive(Debug, Clone)]
pub struct CopyPlan {
    pub location: SwapLocation,
    pub kind: CopyKind,
    pub same_volume: bool,
    pub copy_dir: PathBuf,
    /// The folder whose volume holds the copy: the swap folder when kept, the install when per run.
    pub space_folder: PathBuf,
    /// The planned files under `Data/`, with the prefix stripped.
    pub files: Vec<PlannedFile>,
    pub data_files: usize,
    pub data_bytes: u64,
    /// A kept copy exists in the swap folder (valid or not).
    pub kept_copy_exists: bool,
    /// The bytes and files this run copies in: all of them, or only what changed for a valid kept copy.
    pub to_copy: CopyWork,
}

/// The copy's folder for a kind and a run.
pub fn copy_dir_for(kind: CopyKind, loc: &SwapLocation, run: &str) -> PathBuf {
    match kind {
        CopyKind::Kept => loc.swap_dir.join(KEPT_DIR),
        CopyKind::PerRun => loc
            .install
            .join(format!("{}{run}", game_tool_swap::COPY_PREFIX)),
    }
}

/// The planned files under `Data/` of an instance, with that prefix stripped: what a swap's copy holds.
pub fn data_files(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
) -> Result<Vec<PlannedFile>, SwapError> {
    let plan = game_deploy::plan(ctx, instance_id, definition, DeployMode::Copies)
        .map_err(|e| SwapError::Other(format!("the instance's plan cannot be made: {e}")))?;
    let mut out = Vec::new();
    for file in plan.files {
        let Some((first, rest)) = file.path.as_str().split_once('/') else {
            continue;
        };
        if !first.eq_ignore_ascii_case("Data") || rest.is_empty() {
            continue;
        }
        let path = RelPath::new(rest)
            .map_err(|e| SwapError::Other(format!("invalid path {rest}: {e}")))?;
        out.push(PlannedFile { path, ..file });
    }
    Ok(out)
}

fn key(path: &RelPath) -> String {
    path.as_str().to_ascii_lowercase()
}

fn source_stat(ctx: &Ctx, source: &FileSource) -> Result<(u64, i64), SwapError> {
    let path = game_deploy::source_path(ctx, source);
    let meta = std::fs::metadata(&path).map_err(|e| {
        SwapError::Other(format!(
            "the file {} that the plan needs cannot be read ({e})",
            path.display()
        ))
    })?;
    Ok((meta.len(), get_modified_unix_ms(&meta)))
}

fn copy_stat(path: &Path) -> Option<(u64, i64)> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    Some((meta.len(), get_modified_unix_ms(&meta)))
}

/// Whether a record entry still describes what the plan wants in the copy: the same source, the same
/// size, the source unchanged since the copy was made, and the copy itself unchanged since then.
fn is_current(
    entry: &CopyRecordFile,
    planned: &PlannedFile,
    copy: Option<(u64, i64)>,
    source: (u64, i64),
) -> bool {
    entry.source == planned.source
        && entry.size == planned.size
        && source.0 == planned.size
        && entry.source_modified_unix_ms == source.1
        && copy == Some((entry.size, entry.modified_unix_ms))
}

/// Whether a folder at `copy_dir` is the copy the marker names: `instance_id`, `kind` and the run.
pub fn marker_matches(copy_dir: &Path, instance_id: &str, kind: CopyKind, run: &str) -> bool {
    if game_tool_swap::is_reparse_point(copy_dir) || !copy_dir.is_dir() {
        return false;
    }
    let Ok(text) = std::fs::read_to_string(copy_dir.join(MARKER_FILE)) else {
        return false;
    };
    let Ok(marker) = serde_json::from_str::<CopyMarker>(&text) else {
        return false;
    };
    marker.format == FORMAT
        && marker.instance_id == instance_id
        && marker.kind == kind
        && marker.run == kind.marker_run(run)
}

fn write_marker(
    copy_dir: &Path,
    instance_id: &str,
    kind: CopyKind,
    run: &str,
) -> Result<(), SwapError> {
    let marker = CopyMarker {
        format: FORMAT,
        instance_id: instance_id.to_string(),
        kind,
        run: kind.marker_run(run).to_string(),
    };
    let tmp = copy_dir.join(format!("{MARKER_FILE}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec_pretty(&marker)?)?;
    std::fs::rename(&tmp, copy_dir.join(MARKER_FILE))?;
    Ok(())
}

fn record_path(swap_dir: &Path) -> PathBuf {
    swap_dir.join(RECORD_FILE)
}

/// The kept copy's record, when it parses and names this instance and kind. A record that does not is
/// `None`: the copy is then rebuilt, which is always safe.
fn read_record(swap_dir: &Path, instance_id: &str) -> Option<CopyRecord> {
    let text = std::fs::read_to_string(record_path(swap_dir)).ok()?;
    let record: CopyRecord = serde_json::from_str(&text).ok()?;
    (record.format == FORMAT && record.instance_id == instance_id && record.kind == CopyKind::Kept)
        .then_some(record)
}

fn write_record(swap_dir: &Path, record: &CopyRecord) -> Result<(), SwapError> {
    std::fs::create_dir_all(swap_dir)?;
    let tmp = swap_dir.join(format!("{RECORD_FILE}.tmp"));
    {
        let mut file = std::fs::File::create(&tmp)?;
        std::io::Write::write_all(&mut file, &serde_json::to_vec_pretty(record)?)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, record_path(swap_dir))?;
    Ok(())
}

fn remove_record(swap_dir: &Path) -> Result<(), SwapError> {
    let path = record_path(swap_dir);
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

/// Clear the read-only flag of one file, so it can be written or deleted. Agora's copies are made on
/// Windows, where a copied file keeps the read-only flag of its source; the flag is not a Unix permission
/// here, so clearing it is the intent.
#[allow(clippy::permissions_set_readonly_false)]
fn make_writable(path: &Path) -> Result<(), SwapError> {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return Ok(());
    };
    if meta.is_file() {
        let mut perms = meta.permissions();
        if perms.readonly() {
            perms.set_readonly(false);
            std::fs::set_permissions(path, perms)?;
        }
    }
    Ok(())
}

/// Clear read-only flags throughout a folder, then remove it. Only ever called on a marker-verified copy.
fn remove_tree(dir: &Path) -> Result<(), SwapError> {
    fn clear(dir: &Path) -> Result<(), SwapError> {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if game_tool_swap::is_reparse_point(&path) {
                continue;
            }
            if path.is_dir() {
                clear(&path)?;
            } else {
                make_writable(&path)?;
            }
        }
        Ok(())
    }
    clear(dir)?;
    std::fs::remove_dir_all(dir)?;
    Ok(())
}

/// Every file under the copy's folder, `/`-separated and relative to it, except the marker at the root.
fn list_files(root: &Path, rel: &Path, out: &mut Vec<String>) -> Result<(), SwapError> {
    let cur = if rel.as_os_str().is_empty() {
        root.to_path_buf()
    } else {
        root.join(rel)
    };
    for entry in std::fs::read_dir(&cur)? {
        let entry = entry?;
        let name = entry.file_name();
        if rel.as_os_str().is_empty() && name.to_string_lossy() == MARKER_FILE {
            continue;
        }
        let child = rel.join(&name);
        let meta = std::fs::symlink_metadata(entry.path())?;
        if meta.is_dir() && !game_tool_swap::is_reparse_point(&entry.path()) {
            list_files(root, &child, out)?;
        } else {
            out.push(child.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}

/// Remove the empty folders under `dir`, bottom up. `dir` itself stays.
fn prune_empty_dirs(dir: &Path) -> Result<(), SwapError> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() && !game_tool_swap::is_reparse_point(&path) {
            prune_empty_dirs(&path)?;
            if std::fs::read_dir(&path)?.next().is_none() {
                std::fs::remove_dir(&path)?;
            }
        }
    }
    Ok(())
}

/// The swap folder's kept copy, when there is one that is Agora's. `Ok(None)` when there is none or its
/// record is unusable (it will be rebuilt). `Err` when the folder at the copy's path is not Agora's.
fn kept_record(instance_id: &str, swap_dir: &Path) -> Result<Option<CopyRecord>, SwapError> {
    let copy = swap_dir.join(KEPT_DIR);
    if std::fs::symlink_metadata(&copy).is_err() {
        return Ok(None);
    }
    if !marker_matches(&copy, instance_id, CopyKind::Kept, KEPT_RUN) {
        return Err(SwapError::Refused(format!(
            "{} exists but is not Agora's kept copy of Data for instance {instance_id} (its marker does not match), so it was not touched. Nothing was changed. Move it away, then retry.",
            copy.display()
        )));
    }
    Ok(read_record(swap_dir, instance_id))
}

/// The bytes and files a valid kept copy still needs: the planned files whose record, copy or source
/// no longer matches. Reads only metadata.
fn incremental(
    ctx: &Ctx,
    record: &CopyRecord,
    files: &[PlannedFile],
    copy_dir: &Path,
) -> Result<CopyWork, SwapError> {
    let entries: BTreeMap<String, &CopyRecordFile> =
        record.files.iter().map(|e| (key(&e.path), e)).collect();
    let mut work = CopyWork::default();
    for file in files {
        let entry = entries.get(&key(&file.path)).copied();
        let copy = copy_stat(&copy_dir.join(file.path.as_str()));
        let source = source_stat(ctx, &file.source)?;
        if !entry.is_some_and(|e| is_current(e, file, copy, source)) {
            work.files += 1;
            work.bytes += file.size;
        }
    }
    Ok(work)
}

/// The instance's copy as a swap needs it now: where it goes, what it needs, and whether a kept copy is
/// there. Refuses, before anything is created, when the folder at a kept or per-run path is not free or
/// not Agora's. `run` is empty when only the costs are wanted.
pub fn copy_plan(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    run: &str,
) -> Result<CopyPlan, SwapError> {
    let location = game_tool_swap::swap_location(ctx, instance_id)?;
    let same_volume = game_tool_swap::same_volume(&location.install, &location.swap_dir);
    let kind = if same_volume {
        CopyKind::Kept
    } else {
        CopyKind::PerRun
    };
    let copy_dir = copy_dir_for(kind, &location, run);
    let space_folder = match kind {
        CopyKind::Kept => location.swap_dir.clone(),
        CopyKind::PerRun => location.install.clone(),
    };
    let files = data_files(ctx, instance_id, definition)?;
    let (data_files_n, data_bytes) = (files.len(), files.iter().map(|f| f.size).sum::<u64>());

    let (kept_copy_exists, to_copy) = match kind {
        CopyKind::Kept => {
            let record = kept_record(instance_id, &location.swap_dir)?;
            let exists = std::fs::symlink_metadata(&copy_dir).is_ok();
            let to_copy = match record {
                Some(record) => incremental(ctx, &record, &files, &copy_dir)?,
                None => CopyWork {
                    files: data_files_n,
                    bytes: data_bytes,
                },
            };
            (exists, to_copy)
        }
        CopyKind::PerRun => {
            if std::fs::symlink_metadata(&copy_dir).is_ok() {
                return Err(SwapError::Refused(format!(
                    "{} already exists, so the copy of Data cannot be made there. Nothing was changed; move or remove it, then retry.",
                    copy_dir.display()
                )));
            }
            (
                false,
                CopyWork {
                    files: data_files_n,
                    bytes: data_bytes,
                },
            )
        }
    };
    Ok(CopyPlan {
        same_volume,
        kind,
        copy_dir,
        space_folder,
        files,
        data_files: data_files_n,
        data_bytes,
        kept_copy_exists,
        to_copy,
        location,
    })
}

impl CopyPlan {
    /// The space a run needs on the copy's volume.
    pub fn space_need(&self) -> SpaceNeed {
        SpaceNeed {
            folder: self.space_folder.clone(),
            bytes: self.to_copy.bytes,
        }
    }
}

fn record_from_placed(
    ctx: &Ctx,
    kind: CopyKind,
    instance_id: &str,
    placed: Vec<DeployedFileRecord>,
) -> Result<CopyRecord, SwapError> {
    let mut files = Vec::with_capacity(placed.len());
    for rec in placed {
        let source = source_stat(ctx, &rec.source)?;
        files.push(CopyRecordFile {
            path: rec.path,
            source: rec.source,
            size: rec.size,
            modified_unix_ms: rec.modified_unix_ms,
            source_modified_unix_ms: source.1,
        });
    }
    Ok(CopyRecord {
        format: FORMAT,
        instance_id: instance_id.to_string(),
        kind,
        files,
    })
}

/// Write the planned files into a copy folder, with the executor a copies deployment uses, and clear
/// the read-only flag of every file written, so the tool can write it.
fn place(
    ctx: &Ctx,
    files: &[PlannedFile],
    copy_dir: &Path,
) -> Result<(Vec<DeployedFileRecord>, CopyWork), SwapError> {
    let placed = game_deploy::place_files(ctx, files, copy_dir)
        .map_err(|e| SwapError::Other(format!("the copy of Data could not be written: {e}")))?;
    for rec in &placed.recorded {
        make_writable(&copy_dir.join(rec.path.as_str()))?;
    }
    let work = CopyWork {
        files: placed.recorded.len(),
        bytes: files.iter().map(|f| f.size).sum(),
    };
    Ok((placed.recorded, work))
}

/// Make a copy from nothing: its folder, its marker, every planned file. A kept copy's record is written
/// last, so a copy without one is always rebuilt.
fn create_copy(
    ctx: &Ctx,
    kind: CopyKind,
    instance_id: &str,
    run: &str,
    swap_dir: &Path,
    copy_dir: &Path,
    files: &[PlannedFile],
) -> Result<(CopyRecord, CopyWork), SwapError> {
    if kind == CopyKind::Kept {
        std::fs::create_dir_all(swap_dir)?;
    }
    std::fs::create_dir(copy_dir)?;
    protect::grant_delete_child(copy_dir)?;
    write_marker(copy_dir, instance_id, kind, run)?;
    let (placed, mut work) = place(ctx, files, copy_dir)?;
    work.bytes = files.iter().map(|f| f.size).sum();
    work.files = files.len();
    let record = record_from_placed(ctx, kind, instance_id, placed)?;
    if kind == CopyKind::Kept {
        write_record(swap_dir, &record)?;
    }
    Ok((record, work))
}

/// Bring a kept copy up to the plan. Its record is removed first, so an interrupted update is rebuilt
/// next time. Files the plan no longer names are deleted, and folders left empty with them. A file the
/// plan wants that is current is left alone; any other is copied again from its source.
fn update_kept(
    ctx: &Ctx,
    instance_id: &str,
    swap_dir: &Path,
    copy_dir: &Path,
    old: &CopyRecord,
    files: &[PlannedFile],
) -> Result<(CopyRecord, CopyWork), SwapError> {
    remove_record(swap_dir)?;

    let wanted: BTreeSet<String> = files.iter().map(|f| key(&f.path)).collect();
    let mut present = Vec::new();
    list_files(copy_dir, Path::new(""), &mut present)?;
    for rel in present {
        if !wanted.contains(&rel.to_ascii_lowercase()) {
            let path = copy_dir.join(&rel);
            make_writable(&path)?;
            std::fs::remove_file(&path)?;
        }
    }
    prune_empty_dirs(copy_dir)?;

    let entries: BTreeMap<String, &CopyRecordFile> =
        old.files.iter().map(|e| (key(&e.path), e)).collect();
    let mut stale: Vec<(PlannedFile, (u64, i64))> = Vec::new();
    for file in files {
        let source = source_stat(ctx, &file.source)?;
        let copy = copy_stat(&copy_dir.join(file.path.as_str()));
        let current = entries
            .get(&key(&file.path))
            .is_some_and(|e| is_current(e, file, copy, source));
        if !current {
            stale.push((file.clone(), source));
        }
    }
    let stale_files: Vec<PlannedFile> = stale.iter().map(|(f, _)| f.clone()).collect();
    let (placed, mut work) = place(ctx, &stale_files, copy_dir)?;
    work.files = stale_files.len();
    work.bytes = stale_files.iter().map(|f| f.size).sum();

    let placed_by_key: BTreeMap<String, &DeployedFileRecord> =
        placed.iter().map(|r| (key(&r.path), r)).collect();
    let stale_sources: BTreeMap<String, i64> = stale
        .iter()
        .map(|(f, source)| (key(&f.path), source.1))
        .collect();
    let mut out = Vec::with_capacity(files.len());
    for file in files {
        let k = key(&file.path);
        if let (Some(rec), Some(source_time)) = (placed_by_key.get(&k), stale_sources.get(&k)) {
            out.push(CopyRecordFile {
                path: rec.path.clone(),
                source: rec.source.clone(),
                size: rec.size,
                modified_unix_ms: rec.modified_unix_ms,
                source_modified_unix_ms: *source_time,
            });
        } else if let Some(entry) = entries.get(&k) {
            out.push((*entry).clone());
        }
    }
    let record = CopyRecord {
        format: FORMAT,
        instance_id: instance_id.to_string(),
        kind: CopyKind::Kept,
        files: out,
    };
    write_record(swap_dir, &record)?;
    Ok((record, work))
}

/// Step 2 of a run (MASTER_SPEC §26.9, decision 3): bring the copy up to the plan, or make it. The copy
/// is the one the journal names, and the journal is already written. Returns the record of the copy as
/// it now is, which the capture compares with, and the work done.
pub fn prepare(
    ctx: &Ctx,
    journal: &SwapJournal,
    files: &[PlannedFile],
) -> Result<(CopyRecord, CopyWork), SwapError> {
    match journal.copy_kind {
        CopyKind::Kept => match kept_record(&journal.instance_id, &journal.swap_dir)? {
            Some(old) => update_kept(
                ctx,
                &journal.instance_id,
                &journal.swap_dir,
                &journal.copy_dir,
                &old,
                files,
            ),
            None => {
                discard_copy(journal)?;
                create_copy(
                    ctx,
                    CopyKind::Kept,
                    &journal.instance_id,
                    &journal.run,
                    &journal.swap_dir,
                    &journal.copy_dir,
                    files,
                )
            }
        },
        CopyKind::PerRun => create_copy(
            ctx,
            CopyKind::PerRun,
            &journal.instance_id,
            &journal.run,
            &journal.swap_dir,
            &journal.copy_dir,
            files,
        ),
    }
}

/// Step 10 of a run, and the put-back after a failed launch: the kept copy is brought back in line with
/// the plan as it now is, which includes a new generation if the run was promoted. A discarded run's
/// edits do not survive it. A per-run copy is deleted instead.
pub fn finish_copy(
    ctx: &Ctx,
    definition: &GameDefinition,
    journal: &SwapJournal,
) -> Result<CopyWork, SwapError> {
    match journal.copy_kind {
        CopyKind::PerRun => {
            discard_copy(journal)?;
            Ok(CopyWork::default())
        }
        CopyKind::Kept => {
            let files = data_files(ctx, &journal.instance_id, definition)?;
            match kept_record(&journal.instance_id, &journal.swap_dir)? {
                Some(old) => {
                    let (_, work) = update_kept(
                        ctx,
                        &journal.instance_id,
                        &journal.swap_dir,
                        &journal.copy_dir,
                        &old,
                        &files,
                    )?;
                    Ok(work)
                }
                None => {
                    discard_copy(journal)?;
                    let (_, work) = create_copy(
                        ctx,
                        CopyKind::Kept,
                        &journal.instance_id,
                        &journal.run,
                        &journal.swap_dir,
                        &journal.copy_dir,
                        &files,
                    )?;
                    Ok(work)
                }
            }
        }
    }
}

/// Delete a copy the journal names, and the kept copy's record. Only a folder whose marker matches is
/// deleted; a folder that is not Agora's copy stops the call with `Stuck`, and nothing is removed.
pub fn discard_copy(journal: &SwapJournal) -> Result<(), SwapError> {
    let copy = &journal.copy_dir;
    if std::fs::symlink_metadata(copy).is_ok() {
        if !marker_matches(copy, &journal.instance_id, journal.copy_kind, &journal.run) {
            return Err(SwapError::Stuck(format!(
                "{} is not Agora's copy of Data for instance {} (its marker does not match the swap journal), so Agora will not delete it. Nothing was deleted. Check it by hand, then delete the swap journal for {} ({}).",
                copy.display(),
                journal.instance_id,
                journal.game,
                journal.store
            )));
        }
        remove_tree(copy)?;
    }
    if journal.copy_kind == CopyKind::Kept {
        remove_record(&journal.swap_dir)?;
    }
    Ok(())
}

/// The copy's record as a deployment record, so the capture can compare the copy with it through the
/// farm comparison that link capture uses.
pub fn as_deployment_record(record: &CopyRecord) -> DeploymentRecord {
    DeploymentRecord {
        instance_id: record.instance_id.clone(),
        base_id: String::new(),
        mode: DeployMode::Copies,
        fingerprint: String::new(),
        files: record
            .files
            .iter()
            .map(|f| DeployedFileRecord {
                path: f.path.clone(),
                source: f.source.clone(),
                placement: Placement::Copy,
                size: f.size,
                modified_unix_ms: f.modified_unix_ms,
            })
            .collect(),
        deployed_at_unix_ms: 0,
    }
}

/// The swap folder's summary for a costs or status view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwapCosts {
    pub install_dir: PathBuf,
    pub data_files: usize,
    pub data_bytes: u64,
    pub same_volume: bool,
    pub copy_kind: CopyKind,
    pub copy_folder: PathBuf,
    pub kept_copy_exists: bool,
    pub copy_files: usize,
    pub copy_bytes: u64,
    /// `None` when the free space cannot be read.
    pub free_bytes: Option<u64>,
    pub steam_running: bool,
}

/// What a swap of an instance would cost, for the CLI to show before any swap is run. Changes nothing.
pub fn swap_costs(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
) -> Result<SwapCosts, SwapError> {
    let plan = copy_plan(ctx, instance_id, definition, "")?;
    Ok(SwapCosts {
        install_dir: plan.location.install.clone(),
        data_files: plan.data_files,
        data_bytes: plan.data_bytes,
        same_volume: plan.same_volume,
        copy_kind: plan.kind,
        copy_folder: plan.space_folder.clone(),
        kept_copy_exists: plan.kept_copy_exists,
        copy_files: plan.to_copy.files,
        copy_bytes: plan.to_copy.bytes,
        free_bytes: game_tool_swap::available_bytes(&plan.space_folder),
        steam_running: game_tool_swap::steam_running(),
    })
}

/// The result of `remove-copy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoveCopy {
    Removed,
    NothingToRemove,
}

/// Remove an instance's kept copy (`agora games tools-swap remove-copy`), guarded by its marker. An
/// interrupted swap is put back first, and the instance's lock is held, so no run is going.
pub fn remove_kept_copy(
    ctx: &Ctx,
    definition: &GameDefinition,
    instance_id: &str,
) -> Result<RemoveCopy, SwapError> {
    let _lock = ctx.lock_manager.acquire(
        game_tool_swap::instance_lock(instance_id),
        "tools-swap-remove-copy",
    )?;
    game_tool_swap::recover_game(ctx, definition)?;
    let loc = game_tool_swap::swap_location(ctx, instance_id)?;
    let copy = loc.swap_dir.join(KEPT_DIR);
    let copy_exists = std::fs::symlink_metadata(&copy).is_ok();
    let record_exists = record_path(&loc.swap_dir).exists();
    if !copy_exists && !record_exists {
        return Ok(RemoveCopy::NothingToRemove);
    }
    if copy_exists {
        if !marker_matches(&copy, instance_id, CopyKind::Kept, KEPT_RUN) {
            return Err(SwapError::Refused(format!(
                "{} is not Agora's kept copy of Data for instance {instance_id} (its marker does not match), so nothing was deleted.",
                copy.display()
            )));
        }
        remove_tree(&copy)?;
    }
    remove_record(&loc.swap_dir)?;
    let _ = std::fs::remove_dir(&loc.swap_dir);
    Ok(RemoveCopy::Removed)
}

/// Delete the swap folder of an instance that is being deleted: its kept copy when the marker names this
/// instance, its record, and the folder itself when nothing else is in it. Anything else is left alone.
pub fn remove_deleted_instance_copy(
    ctx: &Ctx,
    instance_id: &str,
    base_id: &str,
) -> Result<(), SwapError> {
    let base_path = ctx.paths.base_manifest_path(base_id);
    let text = std::fs::read_to_string(&base_path)?;
    let base: crate::game_base::BaseManifest = serde_json::from_str(&text)
        .map_err(|e| SwapError::Other(format!("base manifest is unreadable: {e}")))?;
    let Some(bases_root) = base.location.parent() else {
        return Ok(());
    };
    let swap_dir = bases_root
        .join("deployments")
        .join(instance_id)
        .join("swap");
    if !swap_dir.exists() {
        return Ok(());
    }
    let copy = swap_dir.join(KEPT_DIR);
    if std::fs::symlink_metadata(&copy).is_ok() {
        if !marker_matches(&copy, instance_id, CopyKind::Kept, KEPT_RUN) {
            return Err(SwapError::Refused(format!(
                "{} is not Agora's copy of Data for instance {instance_id}, so it was left in place.",
                copy.display()
            )));
        }
        remove_tree(&copy)?;
    }
    remove_record(&swap_dir)?;
    let _ = std::fs::remove_dir(&swap_dir);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kept_copy_is_named_data_under_the_swap_folder() {
        let loc = SwapLocation {
            install: PathBuf::from(r"D:\Games\Skyrim"),
            swap_dir: PathBuf::from(r"D:\Bases\deployments\inst\swap"),
            base_id: "b".into(),
            store: "steam".into(),
        };
        assert_eq!(
            copy_dir_for(CopyKind::Kept, &loc, "r1"),
            PathBuf::from(r"D:\Bases\deployments\inst\swap\Data")
        );
        assert_eq!(
            copy_dir_for(CopyKind::PerRun, &loc, "r1"),
            PathBuf::from(r"D:\Games\Skyrim\Data.agora-copy-r1")
        );
    }
}
