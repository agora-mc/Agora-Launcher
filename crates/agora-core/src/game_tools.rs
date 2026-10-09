//! Tools and the generated layers they write (MASTER_SPEC §26.9).
//!
//! A game's package declares tools: programs that read the installed mods and write files the game
//! needs, such as Nemesis's behaviour files. Core runs a tool under the instance's virtual file
//! system, so everything it writes lands in a staging folder, never in a mod or in the game's own
//! writable layer. A run that exits 0 and passes its output checks is promoted: its folder becomes
//! the tool's next generation, and the instance's generated layer for the tool points at it. Any
//! other run is discarded and the current generation stays in effect.
//!
//! On disk, a tool's output lives in `<instance>/generated/<tool>/`:
//! - `<n>/` is a generation: exactly what one successful run wrote, with its deletions kept as
//!   whiteout markers in the layer's manifest entry;
//! - `staging-<run>/` exists only while a run is going;
//! - `failed-<run>/` is a discarded run, the newest few kept for diagnostics;
//! - `state.json` names the previous generation, so rollback and diff can reach it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

use agora_game_api::{
    glob_match, BaseReference, GameDefinition, GamePath, InputFingerprint, Layer, LayerId,
    LayerSource, LayerStack, RelPath, StoreId, ToolDefinition, ToolId,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ctx::Ctx;
use crate::event_sink::CancellationToken;
use crate::game_base::BaseManifest;
use crate::game_deploy::{
    compare_farm, deploy_locked, deploy_locked_with_copy, discard_deployment, farm_file_differs,
    read_deployment_record, DeployMode, DeploymentRecord, FarmChange, Placement,
};
use crate::game_ini;
use crate::game_instance::GameInstanceManifest;
use crate::game_launch::{
    self, LaunchError, LaunchRoots, LaunchedGame, Launcher, PreparedLaunch, VfsLaunch,
};
use crate::game_load_order;
use crate::game_tool_copy::{self, CopyKind};
use crate::game_tool_swap::{self, SwapError, SwapJournal};
use crate::game_user_files::{self, UserFilesError};
use crate::install_watch::{InstallChanges, InstallWatch};
use crate::lock_manager::LockResource;

/// Failed runs kept in `failed-<run>` folders, the newest first (MASTER_SPEC §26.9).
pub const FAILED_RUNS_KEPT: usize = 3;
/// Written paths a run reports in full; the rest are counted.
pub const REPORTED_PATHS: usize = 20;
/// The largest file a failure marker is searched in: a bigger file is not searched (MASTER_SPEC §26.9).
pub const MARKER_SEARCH_LIMIT: u64 = 16 * 1024 * 1024;

pub(crate) const WHITEOUT_DIR: &str = ".agvfs-wh";
const WHITEOUT_SUFFIX: &str = ".wh";
const STATE_FILE: &str = "state.json";
/// The generation a would-be output is planned under. It names no folder: the staging folder is used.
const WOULD_BE_GENERATION: &str = "would-be";
const VFS_CONFIG: &str = "tool-config.json";
const POLL: Duration = Duration::from_millis(250);
/// How long no process may run from a tool's folder before its run counts as over.
const QUIET: Duration = Duration::from_secs(1);

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("instance '{0}' not found")]
    InstanceNotFound(String),
    #[error("instance '{0}' runs from its install folder, so it has no deployed folder for a tool to run in")]
    Unpinned(String),
    #[error("game '{game}' declares no tool '{tool}'")]
    UnknownTool { game: String, tool: String },
    #[error("tool '{0}' has no output in this instance")]
    NoOutput(String),
    #[error("tool '{0}' has no earlier generation to compare with or roll back to")]
    NoPrevious(String),
    #[error("the game is running from instance '{0}'; close it before running a tool")]
    SessionRunning(String),
    /// The run was asked to capture under the virtual file system (`CaptureMode::Vfs`), and the
    /// virtual file system could not start. Under `Auto` the run captures from links instead.
    #[error(
        "the virtual file system could not start, and this run was asked to use only it: {reason}"
    )]
    VfsUnavailable {
        reason: String,
        next: Option<DeployMode>,
    },
    /// A run captured from links found a linked file changed. A link is the game's copy of a mod's
    /// own file and is never written in place, so the run is discarded and the paths are named.
    #[error("{tool} changed linked file(s) in place: {}. A linked file is never written, so the run was discarded", paths.join(", "))]
    LinkedFileChanged {
        tool: String,
        paths: Vec<String>,
        failed_folder: Option<PathBuf>,
    },
    /// The tool works on the real install's `Data` folder, so under `auto` it cannot run under the
    /// virtual file system or link capture: its writes would miss them. Nothing was deployed. The CLI
    /// asks, and runs it with `--capture swap` (MASTER_SPEC §26.9).
    #[error(
        "{tool} works on the real install's Data folder, so the virtual file system and link capture would miss its writes. It runs by swap: run it with --capture swap"
    )]
    InstallPathNeedsSwap { tool: String, instance_id: String },
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Swap(#[from] SwapError),
    #[error(transparent)]
    Deploy(#[from] crate::game_deploy::DeployError),
    #[error(transparent)]
    Launch(#[from] LaunchError),
    #[error(transparent)]
    UserFiles(#[from] UserFilesError),
    #[error(transparent)]
    Instance(#[from] crate::game_instance::InstanceError),
    #[error(transparent)]
    Layer(#[from] agora_game_api::LayerError),
    #[error(transparent)]
    Id(#[from] agora_game_api::IdError),
    #[error("lock error: {0}")]
    Lock(#[from] crate::error::LauncherError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Other(String),
}

/// Whether a tool's output is what the instance's inputs would build now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputStatus {
    /// The inputs the output was built from are the inputs the instance has now.
    Current,
    /// An input changed since the output was built.
    Stale,
    /// The output was imported, so nobody recorded what it was built from.
    Unknown,
}

impl OutputStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            OutputStatus::Current => "current",
            OutputStatus::Stale => "stale",
            OutputStatus::Unknown => "unknown",
        }
    }
}

/// A warning about a tool's output, shown at launch and by `games instance check`. Never a refusal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputFinding {
    pub tool: String,
    pub generation: String,
    pub status: OutputStatus,
    pub message: String,
}

/// One tool in an instance, as `games instance tools list` shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolState {
    pub tool: String,
    pub name: String,
    /// The game's package still declares the tool. An output whose tool is gone is listed anyway.
    pub declared: bool,
    pub current: Option<String>,
    pub previous: Option<String>,
    /// `None` when the instance has no output for the tool.
    pub status: Option<OutputStatus>,
}

/// How a tool run captures what it writes (MASTER_SPEC §26.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureMode {
    /// Under the virtual file system. A run whose virtual file system cannot start is refused.
    Vfs,
    /// From a linked deployment: the tool runs in the game's folder, and what it changed is
    /// compared with the deployment's record.
    Links,
    /// The virtual file system first, and link capture when it cannot start. A tool that works on the
    /// real install (`uses_install_path`) is refused here: it needs `Swap`.
    Auto,
    /// The real install's `Data` folder is swapped for a link to the farm for the length of the run,
    /// and put back afterwards (MASTER_SPEC §26.9). Asked for by name; never chosen by `Auto`.
    Swap,
}

impl CaptureMode {
    pub fn as_str(self) -> &'static str {
        match self {
            CaptureMode::Vfs => "vfs",
            CaptureMode::Links => "links",
            CaptureMode::Auto => "auto",
            CaptureMode::Swap => "swap",
        }
    }

    /// Parse a user-facing capture name (`vfs`, `links`, `auto`, `swap`).
    pub fn parse(s: &str) -> Option<CaptureMode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "vfs" => Some(CaptureMode::Vfs),
            "links" => Some(CaptureMode::Links),
            "auto" => Some(CaptureMode::Auto),
            "swap" => Some(CaptureMode::Swap),
            _ => None,
        }
    }
}

/// What captured a run's writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureMethod {
    Vfs,
    Links,
    Swap,
}

/// What one run wrote, and what became of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunOutcome {
    pub tool: String,
    pub name: String,
    /// The run's id, which names its `failed-<run>` folder when it was discarded.
    pub run: String,
    /// What captured the run's writes.
    pub capture: CaptureMethod,
    /// Why the run was captured from links: the virtual file system's failure, or a request for
    /// links. `None` for a run captured under the virtual file system.
    pub capture_reason: Option<String>,
    pub promoted: bool,
    /// The generation the output is now: the new one when promoted, else the one that was in effect.
    pub current: Option<String>,
    /// The generation kept for rollback.
    pub previous: Option<String>,
    /// Every file the run wrote, relative to the game folder, `/`-separated.
    pub written: Vec<String>,
    /// Lower files the run deleted, which the output hides.
    pub deleted: Vec<String>,
    pub exit_code: Option<i32>,
    pub cancelled: bool,
    /// Where a discarded run was kept.
    pub failed_folder: Option<PathBuf>,
    /// Why the run was discarded, when it was: the exit code, or the output check it failed and the
    /// file or text that failed it (MASTER_SPEC §26.9). `None` for a promoted run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
    /// The content layer whose files were copied for this run, because it provides the tool's
    /// executable: the tool may then rewrite its own files (MASTER_SPEC §26.9). `None` when no
    /// content layer was copied, as under the virtual file system.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copied_layer: Option<String>,
    /// What changed under the real install folder during the run, outside what Agora captured: a
    /// report, never a refusal (MASTER_SPEC §26.9). Under a swap, `Data` is the link, so its writes
    /// are captured and not listed here.
    #[serde(default)]
    pub install_changes: InstallChanges,
    /// What a swap did with its copy of `Data`, for a run captured by swap (MASTER_SPEC §26.9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swap: Option<SwapRunReport>,
}

/// What a swap did with its copy of the instance's `Data` (MASTER_SPEC §26.9): how much was copied in
/// before the tool ran, what the capture found, and how much was copied again afterwards to match the
/// plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwapRunReport {
    /// Whether the copy is kept between runs (same volume) or made for this run only.
    pub copy: CopyKind,
    /// Files and bytes written into the copy before the tool ran: all of `Data` for a new copy, only
    /// what changed for a kept one.
    pub copied_in_files: usize,
    pub copied_in_bytes: u64,
    /// Changed or new files the capture found in the copy.
    pub captured_files: usize,
    /// Files the tool deleted from the copy, captured as whiteouts.
    pub deleted_files: usize,
    /// Files written into the copy after the run to match the plan again.
    pub refreshed_files: usize,
    pub refreshed_bytes: u64,
}

/// The difference between a tool's previous and current generations, by file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffReport {
    pub tool: String,
    pub previous: String,
    pub current: String,
    pub added: Vec<String>,
    pub changed: Vec<String>,
    pub removed: Vec<String>,
}

/// A generation's record, kept in `state.json` so rollback can bring it back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Generation {
    generation: String,
    inputs: InputFingerprint,
    whiteouts: Vec<RelPath>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct ToolRecord {
    #[serde(default)]
    previous: Option<Generation>,
}

/// Where a tool's generations live under an instance folder.
pub fn tool_dir(instance_dir: &Path, tool: &str) -> PathBuf {
    instance_dir.join("generated").join(tool)
}

/// The folder of one generation. A generation is a number, so nothing else can name a folder
/// outside the tool's own.
pub fn generation_dir(
    instance_dir: &Path,
    tool: &str,
    generation: &str,
) -> Result<PathBuf, String> {
    if generation.is_empty() || !generation.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!(
            "generation '{generation}' of tool '{tool}' is not a number"
        ));
    }
    Ok(tool_dir(instance_dir, tool).join(generation))
}

/// The tool the game's package declares under `tool_id`.
pub fn declared_tool(
    ctx: &Ctx,
    definition: &GameDefinition,
    tool_id: &ToolId,
) -> Result<ToolDefinition, ToolError> {
    ctx.games
        .tool(&definition.id, tool_id)
        .cloned()
        .ok_or_else(|| ToolError::UnknownTool {
            game: definition.id.to_string(),
            tool: tool_id.to_string(),
        })
}

fn load_manifest(ctx: &Ctx, instance_id: &str) -> Result<GameInstanceManifest, ToolError> {
    crate::game_instance::get_manifest(ctx, instance_id).map_err(|e| match e {
        crate::game_instance::InstanceError::NotFound(id) => ToolError::InstanceNotFound(id),
        other => ToolError::Instance(other),
    })
}

fn load_base(ctx: &Ctx, base_id: &str) -> Result<BaseManifest, ToolError> {
    let path = ctx.paths.base_manifest_path(base_id);
    if !path.exists() {
        return Err(ToolError::Other(format!(
            "pinned base '{base_id}' not found"
        )));
    }
    let text = std::fs::read_to_string(&path)?;
    serde_json::from_str(&text)
        .map_err(|e| ToolError::Other(format!("base manifest is unreadable: {e}")))
}

/// The folder the game runs from when an instance is deployed: `<bases>/deployments/<id>/game`.
fn deployment_folder(base: &BaseManifest, instance_id: &str) -> Result<PathBuf, ToolError> {
    let bases_root = base
        .location
        .parent()
        .ok_or_else(|| ToolError::Other("base location has no parent".into()))?;
    Ok(bases_root
        .join("deployments")
        .join(instance_id)
        .join("game"))
}

fn read_record(dir: &Path) -> Result<ToolRecord, ToolError> {
    let path = dir.join(STATE_FILE);
    if !path.exists() {
        return Ok(ToolRecord::default());
    }
    let text = std::fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&text)?)
}

fn write_record(dir: &Path, record: &ToolRecord) -> Result<(), ToolError> {
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("{STATE_FILE}.{}.tmp", new_id_suffix()));
    std::fs::write(&tmp, serde_json::to_vec_pretty(record)?)?;
    std::fs::rename(&tmp, dir.join(STATE_FILE))?;
    Ok(())
}

/// The numbers of the generation folders in a tool's folder.
fn generation_numbers(dir: &Path) -> Result<Vec<u64>, ToolError> {
    let mut out = Vec::new();
    if !dir.is_dir() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            if let Some(n) = entry
                .file_name()
                .to_str()
                .and_then(|s| s.parse::<u64>().ok())
            {
                out.push(n);
            }
        }
    }
    Ok(out)
}

fn new_id_suffix() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..8].to_string()
}

/// A run id that sorts by start time, so the newest failed runs are the last names.
fn new_run_id() -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{ms:013}-{}", new_id_suffix())
}

fn generated_position(layers: &[Layer], tool: &str) -> Option<usize> {
    layers.iter().position(|layer| {
        matches!(&layer.source, LayerSource::Generated { tool: t, .. } if t.as_str() == tool)
    })
}

fn generated_layer(
    tool: &ToolId,
    generation: &str,
    inputs: InputFingerprint,
    whiteouts: Vec<RelPath>,
) -> Result<Layer, ToolError> {
    Ok(Layer {
        id: LayerId::new(format!("generated:{}", tool.as_str()))?,
        enabled: true,
        mount_path: RelPath::default(),
        source_path: RelPath::default(),
        source: LayerSource::Generated {
            tool: tool.clone(),
            generation: generation.to_string(),
            inputs,
        },
        whiteouts,
        own_copy: false,
    })
}

/// Put the generated layers in their order: a tool's output sits above the tools it names in
/// `after_tools`, and otherwise tools follow their ids. Base and content stay below them, the
/// writable layer and any staging above (MASTER_SPEC §26.5, §26.9).
fn arrange(ctx: &Ctx, definition: &GameDefinition, layers: Vec<Layer>) -> Vec<Layer> {
    let (generated, others): (Vec<Layer>, Vec<Layer>) = layers
        .into_iter()
        .partition(|layer| matches!(layer.source, LayerSource::Generated { .. }));

    let mut pending: BTreeMap<String, (Layer, Vec<String>)> = BTreeMap::new();
    for layer in generated {
        let LayerSource::Generated { tool, .. } = &layer.source else {
            continue;
        };
        let after = ctx
            .games
            .tool(&definition.id, tool)
            .map(|t| t.after_tools.iter().map(|a| a.to_string()).collect())
            .unwrap_or_default();
        pending.insert(tool.to_string(), (layer, after));
    }

    let mut placed: Vec<Layer> = Vec::with_capacity(pending.len());
    while !pending.is_empty() {
        // The lowest-named tool whose `after_tools` are all placed. A cycle has none, and then the
        // lowest name goes first, so the order is always defined.
        let ready = pending
            .iter()
            .find(|(name, (_, after))| {
                after
                    .iter()
                    .all(|dep| dep == *name || !pending.contains_key(dep))
            })
            .map(|(name, _)| name.clone());
        let next = ready
            .or_else(|| pending.keys().next().cloned())
            .unwrap_or_default();
        if let Some((layer, _)) = pending.remove(&next) {
            placed.push(layer);
        }
    }

    let mut out = others;
    let at = out
        .iter()
        .position(|layer| layer.source.rank() > 3)
        .unwrap_or(out.len());
    let tail = out.split_off(at);
    out.extend(placed);
    out.extend(tail);
    out
}

fn save_layers(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    manifest: &mut GameInstanceManifest,
    layers: Vec<Layer>,
) -> Result<(), ToolError> {
    manifest.layers = LayerStack::new(arrange(ctx, definition, layers))?;
    crate::game_deploy::write_instance_manifest_atomic(ctx, instance_id, manifest)?;
    Ok(())
}

/// Make files an import found in a Mod Organizer 2 `overwrite` folder into a tool's output
/// (MASTER_SPEC §26.10). They become generation 1 of the tool, copied whole into its folder, and
/// the layer's inputs are unknown: nothing records what they were built from, so the launch check
/// offers a rebuild rather than calling them current. `files` pairs each game-relative path
/// (`Data/...`) with the file it is copied from. Returns how many files were copied.
pub fn import_output(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    tool_id: &ToolId,
    files: &[(String, PathBuf)],
) -> Result<usize, ToolError> {
    declared_tool(ctx, definition, tool_id)?;
    if files.is_empty() {
        return Err(ToolError::Invalid(format!(
            "no files to import as the output of '{tool_id}'"
        )));
    }
    let _lock = ctx.lock_manager.acquire(
        LockResource::Instance(instance_id.to_string()),
        "tool-import",
    )?;
    let mut manifest = load_manifest(ctx, instance_id)?;
    if generated_position(manifest.layers.layers(), tool_id.as_str()).is_some() {
        return Err(ToolError::Invalid(format!(
            "tool '{tool_id}' already has output in this instance"
        )));
    }
    let instance_dir = ctx
        .paths
        .instance_dir(instance_id)
        .map_err(|e| ToolError::Other(e.to_string()))?;
    let folder =
        generation_dir(&instance_dir, tool_id.as_str(), "1").map_err(ToolError::Invalid)?;
    if folder.exists() {
        return Err(ToolError::Invalid(format!(
            "a folder for the first output of '{tool_id}' already exists; remove it first"
        )));
    }
    for (rel, source) in files {
        let dest = crate::game_ini::join_rel(&folder, rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(source, &dest)?;
    }
    let layer = generated_layer(tool_id, "1", InputFingerprint::Unknown, Vec::new())?;
    let mut layers = manifest.layers.layers().to_vec();
    layers.push(layer);
    save_layers(ctx, instance_id, definition, &mut manifest, layers)?;
    Ok(files.len())
}

/// Split `file:section:key` into its three parts.
fn split_setting(setting: &str) -> Result<(&str, &str, &str), ToolError> {
    match setting.split(':').collect::<Vec<_>>().as_slice() {
        [file, section, key] if !file.is_empty() && !section.is_empty() && !key.is_empty() => {
            Ok((file, section, key))
        }
        _ => Err(ToolError::Invalid(format!(
            "relevant setting '{setting}' must be file:section:key"
        ))),
    }
}

/// The digest of everything a tool's output depends on: the enabled content layers in order, the
/// active plugin load order, and the values of the tool's relevant settings (MASTER_SPEC §26.9).
pub fn input_fingerprint(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    tool: &ToolDefinition,
) -> Result<String, ToolError> {
    let manifest = load_manifest(ctx, instance_id)?;
    let mut hasher = Sha256::new();
    for layer in manifest.layers.layers() {
        if let (true, LayerSource::Content { content }) = (layer.enabled, &layer.source) {
            hasher.update(
                format!(
                    "content\t{content}\t{}\t{}\n",
                    layer.mount_path.as_str(),
                    layer.source_path.as_str()
                )
                .as_bytes(),
            );
        }
    }
    if definition.plugin_list.is_some() {
        let order = game_load_order::order(ctx, instance_id, definition)
            .map_err(|e| ToolError::Other(format!("cannot read the load order: {e}")))?;
        for entry in order.entries.iter().filter(|entry| entry.active) {
            hasher.update(format!("plugin\t{}\n", entry.name).as_bytes());
        }
    }
    for setting in &tool.relevant_settings {
        let (file, section, key) = split_setting(setting)?;
        let read = game_ini::read(ctx, instance_id, definition, file)
            .map_err(|e| ToolError::Other(format!("cannot read '{setting}': {e}")))?;
        let value = match read.document.get(section, key) {
            Some(value) => format!("set\t{value}"),
            None => "unset".to_string(),
        };
        hasher.update(format!("setting\t{setting}\t{value}\n").as_bytes());
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Whether a generated layer's output matches the inputs now. Any failure to read the inputs is
/// `Unknown`, never `Current`.
fn layer_status(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    tool: &ToolId,
    inputs: &InputFingerprint,
) -> OutputStatus {
    let InputFingerprint::Known(recorded) = inputs else {
        return OutputStatus::Unknown;
    };
    let Some(declared) = ctx.games.tool(&definition.id, tool).cloned() else {
        return OutputStatus::Unknown;
    };
    match input_fingerprint(ctx, instance_id, definition, &declared) {
        Ok(now) if now == *recorded => OutputStatus::Current,
        Ok(_) => OutputStatus::Stale,
        Err(_) => OutputStatus::Unknown,
    }
}

fn rebuild_command(instance_id: &str, tool: &str) -> String {
    format!("agora games instance tools run {instance_id} {tool}")
}

/// The tools of an instance: every tool the game declares, and every tool with output here.
pub fn list(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
) -> Result<Vec<ToolState>, ToolError> {
    let manifest = load_manifest(ctx, instance_id)?;
    let instance_dir = ctx
        .paths
        .instance_dir(instance_id)
        .map_err(|e| ToolError::Other(e.to_string()))?;
    let mut states: BTreeMap<String, ToolState> = BTreeMap::new();
    for tool_id in &definition.tool_ids {
        let name = ctx
            .games
            .tool(&definition.id, tool_id)
            .map(|t| t.name.clone())
            .unwrap_or_else(|| tool_id.to_string());
        states.insert(
            tool_id.to_string(),
            ToolState {
                tool: tool_id.to_string(),
                name,
                declared: true,
                current: None,
                previous: None,
                status: None,
            },
        );
    }
    for layer in manifest.layers.layers() {
        let LayerSource::Generated {
            tool,
            generation,
            inputs,
        } = &layer.source
        else {
            continue;
        };
        let declared = ctx.games.tool(&definition.id, tool);
        let name = declared
            .map(|t| t.name.clone())
            .unwrap_or_else(|| tool.to_string());
        let previous = read_record(&tool_dir(&instance_dir, tool.as_str()))?
            .previous
            .map(|g| g.generation);
        states.insert(
            tool.to_string(),
            ToolState {
                tool: tool.to_string(),
                name,
                declared: declared.is_some(),
                current: Some(generation.clone()),
                previous,
                status: Some(layer_status(ctx, instance_id, definition, tool, inputs)),
            },
        );
    }
    Ok(states.into_values().collect())
}

/// The warnings a launch or check carries about the instance's enabled generated layers: each one
/// that is stale or unknown. A missing folder is not a warning; it refuses the deployment.
pub fn output_findings(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
) -> Result<Vec<OutputFinding>, ToolError> {
    let manifest = load_manifest(ctx, instance_id)?;
    let mut findings = Vec::new();
    for layer in manifest.layers.layers() {
        let LayerSource::Generated {
            tool,
            generation,
            inputs,
        } = &layer.source
        else {
            continue;
        };
        if !layer.enabled {
            continue;
        }
        let status = layer_status(ctx, instance_id, definition, tool, inputs);
        if status == OutputStatus::Current {
            continue;
        }
        let name = ctx
            .games
            .tool(&definition.id, tool)
            .map(|t| t.name.clone())
            .unwrap_or_else(|| tool.to_string());
        let command = rebuild_command(instance_id, tool.as_str());
        let message = match status {
            OutputStatus::Stale => format!(
                "{name} output is out of date: rebuild with `{command}`"
            ),
            _ => format!(
                "{name} output has unknown inputs (it was imported, so Agora cannot tell whether it is current): rebuild with `{command}`"
            ),
        };
        findings.push(OutputFinding {
            tool: tool.to_string(),
            generation: generation.clone(),
            status,
            message,
        });
    }
    Ok(findings)
}

/// Whether a user-files session is running for this game and store. A tool does not run then.
fn refuse_during_session(
    ctx: &Ctx,
    definition: &GameDefinition,
    store: &StoreId,
) -> Result<(), ToolError> {
    match game_user_files::status(ctx, &definition.id, store) {
        Some(session) if session.running => Err(ToolError::SessionRunning(session.instance)),
        _ => Ok(()),
    }
}

/// Restore the game's user files once a run is over, if a session was swapped in for it.
fn release_session(
    ctx: &Ctx,
    definition: &GameDefinition,
    store: &StoreId,
) -> Result<(), ToolError> {
    if game_user_files::status(ctx, &definition.id, store).is_none() {
        return Ok(());
    }
    match game_user_files::restore(ctx, definition, store) {
        Ok(_) | Err(UserFilesError::NoSession { .. }) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

struct TreeExit {
    code: Option<i32>,
    cancelled: bool,
}

/// Wait for the tool and every process it started from `dir` to end. A cancel ends them all.
fn wait_for_tree(dir: &Path, launched: &mut LaunchedGame, cancel: &CancellationToken) -> TreeExit {
    let mut code = None;
    let mut child_done = false;
    let mut cancelled = false;
    let mut quiet_since = Instant::now();
    loop {
        if !cancelled && cancel.is_cancelled() {
            cancelled = true;
            let _ = launched.child.kill();
            kill_from(dir);
        }
        if !child_done {
            if let Ok(Some(status)) = launched.child.try_wait() {
                child_done = true;
                code = status.code();
            }
        }
        let running = game_launch::processes_running_from(dir);
        if !running.is_empty() || !child_done {
            quiet_since = Instant::now();
        }
        if child_done && running.is_empty() && quiet_since.elapsed() >= QUIET {
            break;
        }
        std::thread::sleep(POLL);
    }
    TreeExit { code, cancelled }
}

fn kill_from(dir: &Path) {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes();
    for process in sys.processes().values() {
        if let Some(exe) = process.exe() {
            if game_launch::is_subpath(exe, dir) {
                process.kill();
            }
        }
    }
}

/// Every file under `root`, `/`-separated and relative to it, skipping the VFS's markers.
fn written_files(root: &Path) -> Result<Vec<String>, ToolError> {
    fn walk(root: &Path, rel: &Path, out: &mut Vec<String>) -> Result<(), ToolError> {
        let cur = if rel.as_os_str().is_empty() {
            root.to_path_buf()
        } else {
            root.join(rel)
        };
        for entry in std::fs::read_dir(&cur)? {
            let entry = entry?;
            let name = entry.file_name();
            if rel.as_os_str().is_empty() && name.to_string_lossy() == WHITEOUT_DIR {
                continue;
            }
            let child = rel.join(&name);
            let meta = std::fs::symlink_metadata(entry.path())?;
            if meta.is_dir() {
                walk(root, &child, out)?;
            } else if meta.is_file() {
                out.push(child.to_string_lossy().replace('\\', "/"));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    if root.is_dir() {
        walk(root, Path::new(""), &mut out)?;
    }
    out.sort();
    Ok(out)
}

/// The paths a run deleted, from the VFS's markers `<staging>\.agvfs-wh\<path>.wh`.
fn read_markers(staging: &Path) -> Result<Vec<RelPath>, ToolError> {
    fn walk(dir: &Path, rel: &str, out: &mut Vec<String>) -> Result<(), ToolError> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            let child = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            let meta = std::fs::symlink_metadata(entry.path())?;
            if meta.is_dir() {
                walk(&entry.path(), &child, out)?;
            } else if meta.is_file() {
                if let Some(path) = child.strip_suffix(WHITEOUT_SUFFIX) {
                    if !path.is_empty() {
                        out.push(path.to_string());
                    }
                }
            }
        }
        Ok(())
    }
    let dir = staging.join(WHITEOUT_DIR);
    let mut names = Vec::new();
    if dir.is_dir() {
        walk(&dir, "", &mut names)?;
    }
    names.sort();
    names
        .into_iter()
        .map(|n| RelPath::new(n).map_err(|e| ToolError::Other(format!("invalid deletion: {e}"))))
        .collect()
}

/// Make a finished run's folder the tool's next generation. The previous current generation is
/// kept for rollback, and the one before it is deleted.
fn promote(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    tool: &ToolDefinition,
    dir: &Path,
    staging: &Path,
    fingerprint: &str,
) -> Result<(String, Option<String>, Vec<RelPath>), ToolError> {
    let deletions = read_markers(staging)?;
    let markers = staging.join(WHITEOUT_DIR);
    if markers.exists() {
        std::fs::remove_dir_all(&markers)?;
    }

    let mut manifest = load_manifest(ctx, instance_id)?;
    let mut layers = manifest.layers.layers().to_vec();
    let position = generated_position(&layers, tool.id.as_str());
    let current = position.and_then(|i| match &layers[i].source {
        LayerSource::Generated {
            generation, inputs, ..
        } => Some(Generation {
            generation: generation.clone(),
            inputs: inputs.clone(),
            whiteouts: layers[i].whiteouts.clone(),
        }),
        _ => None,
    });

    let mut record = read_record(dir)?;
    let mut numbers = generation_numbers(dir)?;
    numbers.extend(
        current
            .iter()
            .filter_map(|g| g.generation.parse::<u64>().ok()),
    );
    numbers.extend(
        record
            .previous
            .iter()
            .filter_map(|g| g.generation.parse::<u64>().ok()),
    );
    let next = numbers.into_iter().max().unwrap_or(0) + 1;
    let next_name = next.to_string();

    std::fs::rename(staging, dir.join(&next_name))?;

    record.previous = current.clone();
    write_record(dir, &record)?;

    let inputs = InputFingerprint::Known(fingerprint.to_string());
    match position {
        Some(i) => {
            layers[i].source = LayerSource::Generated {
                tool: tool.id.clone(),
                generation: next_name.clone(),
                inputs,
            };
            layers[i].whiteouts = deletions.clone();
        }
        None => layers.push(generated_layer(
            &tool.id,
            &next_name,
            inputs,
            deletions.clone(),
        )?),
    }
    save_layers(ctx, instance_id, definition, &mut manifest, layers)?;

    // Only the new current and the previous generation stay on disk.
    let keep: BTreeSet<String> = std::iter::once(next_name.clone())
        .chain(current.as_ref().map(|g| g.generation.clone()))
        .collect();
    for n in generation_numbers(dir)? {
        if !keep.contains(&n.to_string()) {
            std::fs::remove_dir_all(dir.join(n.to_string()))?;
        }
    }

    Ok((next_name, current.map(|g| g.generation), deletions))
}

/// Keep a discarded run's folder as `failed-<run>`, and the newest few of those.
fn discard(dir: &Path, staging: &Path, run: &str) -> Result<PathBuf, ToolError> {
    let failed = dir.join(format!("failed-{run}"));
    std::fs::rename(staging, &failed)?;
    let mut names: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if let Some(name) = entry.file_name().to_str() {
            if name.starts_with("failed-") && entry.file_type()?.is_dir() {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    let excess = names.len().saturating_sub(FAILED_RUNS_KEPT);
    for name in names.iter().take(excess) {
        std::fs::remove_dir_all(dir.join(name))?;
    }
    Ok(failed)
}

/// Run a tool in an instance and capture what it writes (MASTER_SPEC §26.9): under the virtual file
/// system, or from a linked deployment, as `capture` asks. Then promote the output if the tool exited
/// 0 and discard it otherwise.
///
/// Under `Auto` a run whose virtual file system cannot start is captured from links instead, and the
/// outcome says why. Under `Vfs` that start failure is returned as [`ToolError::VfsUnavailable`].
///
/// The instance lock is held for the whole run, and the game's user files are swapped in for it, as
/// for a launch, so the tool and the game never run at once.
pub fn run(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    tool_id: &ToolId,
    capture: CaptureMode,
    launcher: &dyn Launcher,
    cancel: &CancellationToken,
) -> Result<RunOutcome, ToolError> {
    let tool = declared_tool(ctx, definition, tool_id)?;
    let record = crate::game_instance::get(ctx, instance_id)?
        .ok_or_else(|| ToolError::InstanceNotFound(instance_id.to_string()))?;
    let base_id = match &record.base {
        BaseReference::Pinned { id, .. } => id.clone(),
        BaseReference::Unpinned { .. } => {
            return Err(ToolError::Unpinned(instance_id.to_string()));
        }
    };
    let base = load_base(ctx, &base_id)?;
    let store = base.runtime.store.clone();
    refuse_during_session(ctx, definition, &store)?;

    // A tool that works on the real install would miss its writes under either other capture, so
    // `auto` refuses it before anything is deployed; the CLI asks, and then runs it by swap.
    if capture == CaptureMode::Auto && tool.uses_install_path {
        return Err(ToolError::InstallPathNeedsSwap {
            tool: tool_id.to_string(),
            instance_id: instance_id.to_string(),
        });
    }

    // Decided before anything is deployed: a run that must use the VFS cannot start without it.
    let route = match capture {
        CaptureMode::Links => Route::Links(Some("linked capture was asked for".to_string())),
        CaptureMode::Swap => Route::Swap(Some(
            "the tool works on the real install's Data folder, and swap was asked for".to_string(),
        )),
        CaptureMode::Vfs | CaptureMode::Auto => match launcher.locate_vfs_dll() {
            Ok(dll) => Route::Vfs(dll),
            Err(reason) if capture == CaptureMode::Vfs => {
                return Err(ToolError::VfsUnavailable { reason, next: None });
            }
            Err(reason) => Route::Links(Some(vfs_fallback_reason(&reason))),
        },
    };

    let _lock = ctx
        .lock_manager
        .acquire(LockResource::Instance(instance_id.to_string()), "tool-run")?;

    // An interrupted swap for this game is put back before anything else runs (MASTER_SPEC §26.9).
    game_tool_swap::recover_game(ctx, definition)?;

    match route {
        Route::Swap(reason) => run_by_swap(
            ctx,
            instance_id,
            definition,
            &tool,
            tool_id,
            &base,
            launcher,
            cancel,
            reason,
        ),
        Route::Vfs(dll) => match run_under_vfs(
            ctx,
            instance_id,
            definition,
            &tool,
            tool_id,
            &base,
            dll,
            launcher,
            cancel,
        ) {
            Ok(outcome) => Ok(outcome),
            Err(ToolError::VfsUnavailable { reason, .. }) if capture == CaptureMode::Auto => {
                run_from_links(
                    ctx,
                    instance_id,
                    definition,
                    &tool,
                    tool_id,
                    &base,
                    launcher,
                    cancel,
                    Some(vfs_fallback_reason(&reason)),
                )
            }
            Err(e) => Err(e),
        },
        Route::Links(reason) => run_from_links(
            ctx,
            instance_id,
            definition,
            &tool,
            tool_id,
            &base,
            launcher,
            cancel,
            reason,
        ),
    }
}

/// Where a tool run goes: under the virtual file system (with its DLL), or from links with the reason.
enum Route {
    Vfs(PathBuf),
    Links(Option<String>),
    Swap(Option<String>),
}

/// Why a run was captured from links instead of the virtual file system, in the words an outcome
/// shows: "because <this>".
fn vfs_fallback_reason(reason: &str) -> String {
    format!("the virtual file system could not start: {reason}")
}

/// A run under the virtual file system: the instance is deployed in the VFS, the tool runs with its
/// staging folder as the writable layer, and its output is settled. The caller holds the lock.
#[allow(clippy::too_many_arguments)]
fn run_under_vfs(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    tool: &ToolDefinition,
    tool_id: &ToolId,
    base: &BaseManifest,
    dll: PathBuf,
    launcher: &dyn Launcher,
    cancel: &CancellationToken,
) -> Result<RunOutcome, ToolError> {
    let store = base.runtime.store.clone();
    let instance_dir = ctx
        .paths
        .instance_dir(instance_id)
        .map_err(|e| ToolError::Other(e.to_string()))?;
    let game_dir = deployment_folder(base, instance_id)?;
    let writable = instance_dir.join("writable");

    deploy_locked(ctx, instance_id, definition, DeployMode::Virtual)?;
    let fingerprint = input_fingerprint(ctx, instance_id, definition, tool)?;

    let roots = LaunchRoots {
        runtime: game_dir.clone(),
        install: Some(base.source_location.clone()),
        base: Some(base.location.clone()),
    };
    let resolved = game_launch::resolve_recipe(&tool.launch, &roots)?;

    let dir = tool_dir(&instance_dir, tool_id.as_str());
    let run = new_run_id();
    let staging = dir.join(format!("staging-{run}"));
    std::fs::create_dir_all(&staging)?;

    let mut prepared = PreparedLaunch::undeployed(resolved, Vec::new(), None);
    prepared.deployment = Some(DeployMode::Virtual);
    // The game folder is mounted; its lowers are the instance's writable layer over the farm, so
    // the tool sees what the game wrote but every write lands in this run's staging folder.
    prepared.vfs = Some(VfsLaunch {
        dll,
        mount: game_dir.clone(),
        upper: staging.clone(),
        lowers: vec![writable, game_dir.clone()],
        config_path: instance_dir.join("vfs").join(VFS_CONFIG),
        log: instance_dir.join("logs").join("vfs.log"),
    });

    if let Err(e) = game_user_files::swap_in(ctx, instance_id, definition, &store, &game_dir) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e.into());
    }

    // The real install is watched from just before the tool starts until it exits (MASTER_SPEC §26.9).
    let watch = InstallWatch::start(&base.source_location);
    let mut launched = match launcher.launch(&prepared) {
        Ok(launched) => launched,
        Err(e) => {
            release_session(ctx, definition, &store)?;
            let _ = std::fs::remove_dir_all(&staging);
            return Err(match e {
                LaunchError::VfsUnavailable { reason } => {
                    ToolError::VfsUnavailable { reason, next: None }
                }
                other => ToolError::Launch(other),
            });
        }
    };
    let _ = game_user_files::record_process(ctx, &definition.id, &store, launched.identity.clone());

    let exit = wait_for_tree(&game_dir, &mut launched, cancel);
    let installed = watch.finish();
    let settled = settle_run(
        ctx,
        instance_id,
        definition,
        tool,
        &dir,
        &staging,
        &run,
        &fingerprint,
        &exit,
    );
    // The game's files go back whatever became of the output, and a failure to settle is reported
    // only after that.
    let released = release_session(ctx, definition, &store);
    let settled = settled?;
    released?;

    let mut out = outcome(tool_id, tool, run, &exit, settled, CaptureMethod::Vfs, None);
    out.install_changes = installed;
    Ok(out)
}

/// The content layer that provides a tool's executable, when one does: the layer whose file wins the
/// executable's path among the layers that hold it (MASTER_SPEC §26.9). That layer is copied for the
/// tool's run, so the tool can rewrite its own files. `None` when the executable is not a content
/// layer's file (a base file, or the instance's own output) or the recipe names no runtime path.
fn executable_layer(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    tool: &ToolDefinition,
) -> Result<Option<String>, ToolError> {
    let GamePath::Runtime { path } = &tool.launch.executable else {
        return Ok(None);
    };
    let planned = crate::game_deploy::plan(ctx, instance_id, definition, DeployMode::Links)?;
    let winner = planned
        .files
        .iter()
        .find(|file| file.path.as_str().eq_ignore_ascii_case(path.as_str()));
    Ok(winner.and_then(|file| match file.source {
        crate::game_deploy::FileSource::Content { .. } => Some(file.layer_id.clone()),
        _ => None,
    }))
}

/// A run captured from a linked deployment (MASTER_SPEC §26.9). The instance is deployed in Links
/// mode, which records each file's placement, size and source: that record is the "before". The tool
/// runs in the game's folder, and what it changed is compared with the record. New files and changed
/// copies become the output; deletions become whiteouts; a changed link is an error and the run is
/// discarded. Then the output settles as a VFS run's does, and the farm is taken away, so the tool's
/// writes never become the game's writable layer. The caller holds the lock.
#[allow(clippy::too_many_arguments)]
fn run_from_links(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    tool: &ToolDefinition,
    tool_id: &ToolId,
    base: &BaseManifest,
    launcher: &dyn Launcher,
    cancel: &CancellationToken,
    reason: Option<String>,
) -> Result<RunOutcome, ToolError> {
    let store = base.runtime.store.clone();
    let instance_dir = ctx
        .paths
        .instance_dir(instance_id)
        .map_err(|e| ToolError::Other(e.to_string()))?;
    let game_dir = deployment_folder(base, instance_id)?;
    let deployment_dir = game_dir
        .parent()
        .ok_or_else(|| ToolError::Other("the deployment has no folder".into()))?
        .to_path_buf();

    let copied = executable_layer(ctx, instance_id, definition, tool)?;
    deploy_locked_with_copy(
        ctx,
        instance_id,
        definition,
        DeployMode::Links,
        copied.as_deref(),
    )?;
    let before = read_deployment_record(&deployment_dir)?;
    let fingerprint = input_fingerprint(ctx, instance_id, definition, tool)?;

    let roots = LaunchRoots {
        runtime: game_dir.clone(),
        install: Some(base.source_location.clone()),
        base: Some(base.location.clone()),
    };
    let resolved = game_launch::resolve_recipe(&tool.launch, &roots)?;

    let dir = tool_dir(&instance_dir, tool_id.as_str());
    let run = new_run_id();
    let staging = dir.join(format!("staging-{run}"));

    // No VFS: the tool's working folder is the game's folder, so its writes land in the farm.
    let mut prepared = PreparedLaunch::undeployed(resolved, Vec::new(), None);
    prepared.deployment = Some(DeployMode::Links);

    game_user_files::swap_in(ctx, instance_id, definition, &store, &game_dir)?;

    // The real install is watched from just before the tool starts until it exits (MASTER_SPEC §26.9).
    let watch = InstallWatch::start(&base.source_location);
    let mut launched = match launcher.launch(&prepared) {
        Ok(launched) => launched,
        Err(e) => {
            release_session(ctx, definition, &store)?;
            return Err(ToolError::Launch(e));
        }
    };
    let _ = game_user_files::record_process(ctx, &definition.id, &store, launched.identity.clone());

    let exit = wait_for_tree(&game_dir, &mut launched, cancel);
    let installed = watch.finish();
    let settled = capture_from_links(
        ctx,
        instance_id,
        definition,
        tool,
        &dir,
        &staging,
        &run,
        &fingerprint,
        &game_dir,
        &before,
        &exit,
    );
    // The farm holds whatever the tool wrote. It goes whatever became of the output, and without a
    // harvest: the writes were captured above, and the next launch deploys the promoted output.
    let forgotten = discard_deployment(&deployment_dir, instance_id);
    let released = release_session(ctx, definition, &store);
    let settled = settled?;
    forgotten?;
    released?;

    let mut out = outcome(
        tool_id,
        tool,
        run,
        &exit,
        settled,
        CaptureMethod::Links,
        reason,
    );
    out.copied_layer = copied;
    out.install_changes = installed;
    Ok(out)
}

/// A run that works on the real install's `Data` folder (MASTER_SPEC §26.9, slice 4e). The instance's
/// `Data` is copied as real, writable files (the copy is kept between runs on the same volume, or made
/// for this run on the install's volume). The real `Data` is renamed aside, the copy is renamed into its
/// place, and the tool runs from the install folder, so it sees the real path. When it exits the copy
/// is renamed back and the real `Data` with it. The copy's changes are captured from the copy against
/// its record, the way link capture does, then the copy is brought back in line with the plan. The
/// instance's normal farm is not touched. The caller holds the lock.
#[allow(clippy::too_many_arguments)]
fn run_by_swap(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    tool: &ToolDefinition,
    tool_id: &ToolId,
    base: &BaseManifest,
    launcher: &dyn Launcher,
    cancel: &CancellationToken,
    reason: Option<String>,
) -> Result<RunOutcome, ToolError> {
    let store = base.runtime.store.clone();
    let instance_dir = ctx
        .paths
        .instance_dir(instance_id)
        .map_err(|e| ToolError::Other(e.to_string()))?;
    let run = new_run_id();

    // Decided from trusted inputs before anything is created or moved.
    let copy = game_tool_copy::copy_plan(ctx, instance_id, definition, &run)?;
    let install = copy.location.install.clone();
    let real_data = install.join("Data");
    let aside = install.join(format!("{}{run}", game_tool_swap::ASIDE_PREFIX));
    let dir = tool_dir(&instance_dir, tool_id.as_str());
    let staging = dir.join(format!("staging-{run}"));

    game_tool_swap::preflight(&install, &real_data, &aside, &copy.space_need())?;
    let fingerprint = input_fingerprint(ctx, instance_id, definition, tool)?;

    // The user files go in before the real Data moves, so the swap journal is the only swap in progress
    // for this game when the recovery check runs.
    game_user_files::swap_in(ctx, instance_id, definition, &store, &install)?;

    let journal = SwapJournal {
        version: game_tool_swap::JOURNAL_VERSION,
        game: definition.id.to_string(),
        store: store.as_str().to_string(),
        instance_id: instance_id.to_string(),
        tool: tool_id.to_string(),
        run: run.clone(),
        started_unix_ms: now_unix_ms(),
        install_dir: install.clone(),
        real_data: real_data.clone(),
        aside: aside.clone(),
        copy_kind: copy.kind,
        copy_dir: copy.copy_dir.clone(),
        swap_dir: copy.location.swap_dir.clone(),
    };
    // The journal is written, and made durable, before the copy or the real Data is touched.
    if let Err(e) = game_tool_swap::write_journal(ctx, &journal) {
        let _ = release_session(ctx, definition, &store);
        return Err(e.into());
    }

    let (before, prepared) = match game_tool_copy::prepare(ctx, &journal, &copy.files) {
        Ok(prepared) => prepared,
        Err(e) => return Err(abandon_swap(ctx, definition, &store, &journal, e.into())),
    };
    if let Err(e) = game_tool_swap::engage(&journal) {
        return Err(abandon_swap(ctx, definition, &store, &journal, e.into()));
    }

    // The tool runs from the install folder, with the copy in place of `Data`: a program the recipe starts
    // from `Data` runs the copy's file, and a program from the install root runs the real file. The recipe
    // is resolved now, because resolving it checks that the program is there.
    let roots = LaunchRoots {
        runtime: install.clone(),
        install: Some(install.clone()),
        base: Some(base.location.clone()),
    };
    let resolved = match game_launch::resolve_recipe(&tool.launch, &roots) {
        Ok(resolved) => resolved,
        Err(e) => {
            return Err(put_back_after(
                ctx,
                definition,
                &store,
                &journal,
                ToolError::Launch(e),
            ))
        }
    };

    let mut prepared_launch = PreparedLaunch::undeployed(resolved, Vec::new(), None);
    prepared_launch.deployment = Some(DeployMode::Copies);

    let watch = InstallWatch::start(&install);
    let mut launched = match launcher.launch(&prepared_launch) {
        Ok(launched) => launched,
        Err(e) => {
            let _ = watch.finish();
            return Err(put_back_after(
                ctx,
                definition,
                &store,
                &journal,
                ToolError::Launch(e),
            ));
        }
    };
    let _ = game_user_files::record_process(ctx, &definition.id, &store, launched.identity.clone());

    let exit = wait_for_tree(&install, &mut launched, cancel);
    let installed = watch.finish();

    // Put the real Data back before capturing: the copy is moved home, then the aside comes back. If that
    // fails, the copy and the journal are kept for the next check.
    if let Err(e) = game_tool_swap::disengage(&journal) {
        let _ = release_session(ctx, definition, &store);
        return Err(e.into());
    }

    let captured = capture_from_copy(
        ctx,
        instance_id,
        definition,
        tool,
        &dir,
        &staging,
        &run,
        &fingerprint,
        &journal.copy_dir,
        &before,
        &exit,
    );
    let finished = game_tool_copy::finish_copy(ctx, definition, &journal);
    let released = release_session(ctx, definition, &store);
    let refreshed = finished?;
    game_tool_swap::remove_journal(ctx, &journal)?;
    released?;
    let (settled, counts) = captured?;

    let mut out = outcome(
        tool_id,
        tool,
        run,
        &exit,
        settled,
        CaptureMethod::Swap,
        reason,
    );
    out.install_changes = outside_data(installed);
    out.swap = Some(SwapRunReport {
        copy: copy.kind,
        copied_in_files: prepared.files,
        copied_in_bytes: prepared.bytes,
        captured_files: counts.changed,
        deleted_files: counts.missing,
        refreshed_files: refreshed.files,
        refreshed_bytes: refreshed.bytes,
    });
    Ok(out)
}

/// What a swap's capture found in the copy: the changed or new files written to the staging folder, and
/// the files the tool deleted.
#[derive(Debug, Clone, Copy, Default)]
struct CopyCounts {
    changed: usize,
    missing: usize,
}

/// Step 9 of a swap: compares the copy with the record it had before the run, by the change detection
/// link capture uses (size; then time; then the bytes against the source), and writes what changed into
/// the staging folder under `Data/`. A deletion becomes a whiteout. The copy's own marker is never
/// output. Then the run is settled as any other.
#[allow(clippy::too_many_arguments)]
fn capture_from_copy(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    tool: &ToolDefinition,
    dir: &Path,
    staging: &Path,
    run: &str,
    fingerprint: &str,
    copy_dir: &Path,
    before: &game_tool_copy::CopyRecord,
    exit: &TreeExit,
) -> Result<(Settled, CopyCounts), ToolError> {
    std::fs::create_dir_all(staging)?;
    let deployed = game_tool_copy::as_deployment_record(before);
    let diff = compare_farm(copy_dir, &deployed, &mut |file, recorded| {
        farm_file_differs(ctx, file, recorded)
    })?;

    let mut counts = CopyCounts::default();
    for change in diff.changes {
        let file = match change {
            FarmChange::Changed { file, .. } | FarmChange::Added(file) => file,
        };
        if file.rel_path.as_str() == game_tool_copy::MARKER_FILE {
            continue;
        }
        let rel = format!("Data/{}", file.rel_path.as_str());
        let dest = crate::game_ini::join_rel(staging, &rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&file.abs_path, &dest)?;
        counts.changed += 1;
    }
    for recorded in &diff.missing {
        let rel = RelPath::new(format!("Data/{}", recorded.path.as_str()))
            .map_err(|e| ToolError::Other(format!("invalid deletion: {e}")))?;
        write_whiteout(staging, &rel)?;
        counts.missing += 1;
    }

    let settled = settle_run(
        ctx,
        instance_id,
        definition,
        tool,
        dir,
        staging,
        run,
        fingerprint,
        exit,
    )?;
    Ok((settled, counts))
}

/// The changes under the real install folder that are not under `Data`: a swap's `Data` writes are the
/// copy's, and are captured, so they are not listed here.
fn outside_data(changes: InstallChanges) -> InstallChanges {
    let outside = |paths: Vec<String>| -> Vec<String> {
        paths
            .into_iter()
            .filter(|p| {
                !(p.eq_ignore_ascii_case("Data") || p.to_ascii_lowercase().starts_with("data/"))
            })
            .collect()
    };
    InstallChanges {
        created: outside(changes.created),
        changed: outside(changes.changed),
        deleted: outside(changes.deleted),
        may_be_incomplete: changes.may_be_incomplete,
        unavailable: changes.unavailable,
    }
}

/// A swap that did not get as far as the tool: the real Data is put back if it moved, the copy is
/// discarded, and the journal goes last. If the real Data cannot be put back, the copy and the journal
/// are kept for the next check and that error is returned instead.
fn abandon_swap(
    ctx: &Ctx,
    definition: &GameDefinition,
    store: &StoreId,
    journal: &SwapJournal,
    error: ToolError,
) -> ToolError {
    if let Err(e) = game_tool_swap::restore_and_discard(journal) {
        return e.into();
    }
    let _ = game_tool_swap::remove_journal(ctx, journal);
    let _ = release_session(ctx, definition, store);
    error
}

/// A swap whose tool did not start: the copy moves home, the kept copy is brought to the plan (or the
/// per-run copy deleted), and the journal goes. Returns `error`, or the error of the step that failed.
fn put_back_after(
    ctx: &Ctx,
    definition: &GameDefinition,
    store: &StoreId,
    journal: &SwapJournal,
    error: ToolError,
) -> ToolError {
    if let Err(e) = game_tool_swap::disengage(journal) {
        let _ = release_session(ctx, definition, store);
        return e.into();
    }
    if let Err(e) = game_tool_copy::finish_copy(ctx, definition, journal) {
        let _ = release_session(ctx, definition, store);
        return e.into();
    }
    if let Err(e) = game_tool_swap::remove_journal(ctx, journal) {
        let _ = release_session(ctx, definition, store);
        return e.into();
    }
    let _ = release_session(ctx, definition, store);
    error
}

fn now_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn outcome(
    tool_id: &ToolId,
    tool: &ToolDefinition,
    run: String,
    exit: &TreeExit,
    settled: Settled,
    capture: CaptureMethod,
    capture_reason: Option<String>,
) -> RunOutcome {
    RunOutcome {
        tool: tool_id.to_string(),
        name: tool.name.clone(),
        run,
        capture,
        capture_reason,
        promoted: settled.promoted,
        current: settled.current,
        previous: settled.previous,
        written: settled.written,
        deleted: settled
            .deleted
            .iter()
            .map(|d| d.as_str().to_string())
            .collect(),
        exit_code: exit.code,
        cancelled: exit.cancelled,
        failed_folder: settled.failed_folder,
        failure: settled.failure,
        copied_layer: None,
        install_changes: InstallChanges::default(),
        swap: None,
    }
}

/// Write the staging folder of a link-captured run from the difference between the farm and its
/// record, then settle it. A changed link is refused: the run is discarded and the paths named.
#[allow(clippy::too_many_arguments)]
fn capture_from_links(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    tool: &ToolDefinition,
    dir: &Path,
    staging: &Path,
    run: &str,
    fingerprint: &str,
    game_dir: &Path,
    before: &DeploymentRecord,
    exit: &TreeExit,
) -> Result<Settled, ToolError> {
    std::fs::create_dir_all(staging)?;
    // A copy is changed when its bytes differ, not when its time moves: a tool that rewrites the same
    // bytes changed nothing. A link is the content object itself, so a write through it shows in its
    // size and time, and the copy in the source cannot be compared with it.
    let diff = compare_farm(game_dir, before, &mut |file, recorded| {
        farm_file_differs(ctx, file, recorded)
    })?;

    let linked: Vec<String> = diff
        .changes
        .iter()
        .filter_map(|change| match change {
            FarmChange::Changed { recorded, file } if recorded.placement == Placement::Link => {
                Some(file.rel_path.as_str().to_string())
            }
            _ => None,
        })
        .collect();
    if !linked.is_empty() {
        let failed = discard(dir, staging, run)?;
        return Err(ToolError::LinkedFileChanged {
            tool: tool.id.to_string(),
            paths: linked,
            failed_folder: Some(failed),
        });
    }

    for change in diff.changes {
        let file = match change {
            FarmChange::Changed { file, .. } | FarmChange::Added(file) => file,
        };
        let dest = crate::game_ini::join_rel(staging, file.rel_path.as_str());
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&file.abs_path, &dest)?;
    }
    for recorded in &diff.missing {
        write_whiteout(staging, &recorded.path)?;
    }

    settle_run(
        ctx,
        instance_id,
        definition,
        tool,
        dir,
        staging,
        run,
        fingerprint,
        exit,
    )
}

/// The VFS's deletion marker for `rel` in a staging folder: `<staging>\.agvfs-wh\<rel>.wh`, the same
/// marker the VFS writes when a tool deletes a file.
pub(crate) fn write_whiteout(staging: &Path, rel: &RelPath) -> Result<(), ToolError> {
    let mut marker = staging
        .join(WHITEOUT_DIR)
        .join(rel.as_str())
        .into_os_string();
    marker.push(WHITEOUT_SUFFIX);
    let marker = PathBuf::from(marker);
    if let Some(parent) = marker.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&marker, b"")?;
    Ok(())
}

/// What became of a finished run's output.
struct Settled {
    written: Vec<String>,
    promoted: bool,
    current: Option<String>,
    previous: Option<String>,
    deleted: Vec<RelPath>,
    failed_folder: Option<PathBuf>,
    failure: Option<String>,
}

/// Promote a run that exited 0 and passed its output checks, or discard any other run, and say what
/// the output is now. A run that exits 0 is still discarded when a required output is missing or a
/// failure marker is in a file it wrote (MASTER_SPEC §26.9).
#[allow(clippy::too_many_arguments)]
fn settle_run(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    tool: &ToolDefinition,
    dir: &Path,
    staging: &Path,
    run: &str,
    fingerprint: &str,
    exit: &TreeExit,
) -> Result<Settled, ToolError> {
    let written = written_files(staging)?;
    let failure = if exit.cancelled {
        Some("the run was cancelled".to_string())
    } else if exit.code != Some(0) {
        Some(match exit.code {
            Some(code) => format!("the tool exited with code {code}"),
            None => "the tool exited with no exit code".to_string(),
        })
    } else {
        output_check(ctx, instance_id, definition, tool, staging, &written)?
    };
    let promoted = failure.is_none();
    if promoted {
        let (current, previous, deleted) = promote(
            ctx,
            instance_id,
            definition,
            tool,
            dir,
            staging,
            fingerprint,
        )?;
        return Ok(Settled {
            written,
            promoted,
            current: Some(current),
            previous,
            deleted,
            failed_folder: None,
            failure: None,
        });
    }
    let failed = discard(dir, staging, run)?;
    Ok(Settled {
        written,
        promoted,
        current: read_current_generation(ctx, instance_id, &tool.id)?,
        previous: read_record(dir)?.previous.map(|g| g.generation),
        deleted: Vec::new(),
        failed_folder: Some(failed),
        failure,
    })
}

/// The files the game would read once `tool`'s run is promoted, lower-cased and `/`-separated
/// (MASTER_SPEC §26.9). The stack is the instance's, with the tool's generated layer holding the run's
/// staging folder and the run's whiteouts. The layers above that layer are left out, the writable
/// layer among them: the game's own writes are not the tool's output. The planner does the work, so
/// what it hides or places is what the game would see.
///
/// The inner `Err` is a reason the output cannot be deployed at all (for example a file that is a
/// folder in another layer). That fails the run's checks; an outer `Err` is a failure to read the
/// instance.
fn would_be_files(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    tool: &ToolDefinition,
    staging: &Path,
) -> Result<Result<Vec<String>, String>, ToolError> {
    let deletions = read_markers(staging)?;
    let mut manifest = load_manifest(ctx, instance_id)?;
    let mut layers = manifest.layers.layers().to_vec();
    match generated_position(&layers, tool.id.as_str()) {
        Some(i) => {
            layers[i].source = LayerSource::Generated {
                tool: tool.id.clone(),
                generation: WOULD_BE_GENERATION.to_string(),
                inputs: InputFingerprint::Unknown,
            };
            layers[i].whiteouts = deletions;
        }
        None => layers.push(generated_layer(
            &tool.id,
            WOULD_BE_GENERATION,
            InputFingerprint::Unknown,
            deletions,
        )?),
    }
    let layers = arrange(ctx, definition, layers);
    let end = generated_position(&layers, tool.id.as_str()).unwrap_or(layers.len());
    manifest.layers = LayerStack::new(layers.into_iter().take(end + 1).collect())?;

    let would_be = crate::game_deploy::WouldBe {
        manifest,
        tool: tool.id.as_str(),
        folder: staging.to_path_buf(),
    };
    match crate::game_deploy::plan_would_be(ctx, instance_id, definition, would_be) {
        Ok(plan) => Ok(Ok(plan
            .files
            .iter()
            .map(|file| file.path.as_str().to_ascii_lowercase())
            .collect())),
        Err(e) => Ok(Err(format!(
            "the output the run would produce cannot be deployed: {e}"
        ))),
    }
}

/// The checks a run that exited 0 must pass (MASTER_SPEC §26.9): every required output is a file the
/// game would read once the run is promoted, and no failure marker is in a file the run wrote.
/// Returns why the run failed, or `None` when it passed. Nothing is planned when the tool declares no
/// required outputs, and nothing is read when it declares no markers.
fn output_check(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    tool: &ToolDefinition,
    staging: &Path,
    written: &[String],
) -> Result<Option<String>, ToolError> {
    if !tool.required_outputs.is_empty() {
        let files = match would_be_files(ctx, instance_id, definition, tool, staging)? {
            Ok(files) => files,
            Err(reason) => return Ok(Some(reason)),
        };
        for pattern in &tool.required_outputs {
            let wanted = pattern.to_ascii_lowercase();
            if !files.iter().any(|key| glob_match(&wanted, key)) {
                return Ok(Some(format!(
                    "required output '{pattern}' is missing from the output the run would produce"
                )));
            }
        }
    }

    for marker in &tool.failure_markers {
        if marker.contains.is_empty() {
            continue;
        }
        let wanted = marker.file.as_str().to_ascii_lowercase();
        let Some(rel) = written
            .iter()
            .find(|rel| rel.to_ascii_lowercase() == wanted)
        else {
            continue;
        };
        let path = rel
            .split('/')
            .fold(staging.to_path_buf(), |p, part| p.join(part));
        if std::fs::metadata(&path)?.len() > MARKER_SEARCH_LIMIT {
            continue;
        }
        let text = String::from_utf8_lossy(&std::fs::read(&path)?).into_owned();
        if text.contains(&marker.contains) {
            return Ok(Some(format!(
                "{rel} contains '{}', which marks a failed run",
                marker.contains
            )));
        }
    }
    Ok(None)
}

/// The generation a tool's layer points at now. Read through the manifest on demand: a discarded run
/// leaves the output as it was.
fn read_current_generation(
    ctx: &Ctx,
    instance_id: &str,
    tool: &ToolId,
) -> Result<Option<String>, ToolError> {
    let manifest = load_manifest(ctx, instance_id)?;
    Ok(manifest
        .layers
        .layers()
        .iter()
        .find_map(|layer| match &layer.source {
            LayerSource::Generated {
                tool: layer_tool,
                generation,
                ..
            } if layer_tool == tool => Some(generation.clone()),
            _ => None,
        }))
}

/// Swap a tool's current and previous generations, so the game reads the previous output on its
/// next deployment (MASTER_SPEC §26.9). Nothing is deleted.
pub fn rollback(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    tool_id: &ToolId,
) -> Result<ToolState, ToolError> {
    let _lock = ctx.lock_manager.acquire(
        LockResource::Instance(instance_id.to_string()),
        "tool-rollback",
    )?;
    let mut manifest = load_manifest(ctx, instance_id)?;
    let mut layers = manifest.layers.layers().to_vec();
    let position = generated_position(&layers, tool_id.as_str())
        .ok_or_else(|| ToolError::NoOutput(tool_id.to_string()))?;
    let instance_dir = ctx
        .paths
        .instance_dir(instance_id)
        .map_err(|e| ToolError::Other(e.to_string()))?;
    let dir = tool_dir(&instance_dir, tool_id.as_str());
    let mut record = read_record(&dir)?;
    let previous = record
        .previous
        .clone()
        .ok_or_else(|| ToolError::NoPrevious(tool_id.to_string()))?;
    let previous_dir = generation_dir(&instance_dir, tool_id.as_str(), &previous.generation)
        .map_err(ToolError::Other)?;
    if !previous_dir.is_dir() {
        return Err(ToolError::Other(format!(
            "the previous output of tool '{tool_id}' is missing from {}",
            previous_dir.display()
        )));
    }

    let current = match &layers[position].source {
        LayerSource::Generated {
            generation, inputs, ..
        } => Generation {
            generation: generation.clone(),
            inputs: inputs.clone(),
            whiteouts: layers[position].whiteouts.clone(),
        },
        _ => return Err(ToolError::NoOutput(tool_id.to_string())),
    };
    layers[position].source = LayerSource::Generated {
        tool: tool_id.clone(),
        generation: previous.generation.clone(),
        inputs: previous.inputs.clone(),
    };
    layers[position].whiteouts = previous.whiteouts.clone();
    save_layers(ctx, instance_id, definition, &mut manifest, layers)?;
    record.previous = Some(current);
    write_record(&dir, &record)?;

    list(ctx, instance_id, definition)?
        .into_iter()
        .find(|state| state.tool == tool_id.as_str())
        .ok_or_else(|| ToolError::NoOutput(tool_id.to_string()))
}

/// Remove a tool's output from an instance: its layer goes, and so do its generations.
/// Returns the generation folders that were deleted.
pub fn remove(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    tool_id: &ToolId,
) -> Result<Vec<String>, ToolError> {
    let _lock = ctx.lock_manager.acquire(
        LockResource::Instance(instance_id.to_string()),
        "tool-remove",
    )?;
    let mut manifest = load_manifest(ctx, instance_id)?;
    let mut layers = manifest.layers.layers().to_vec();
    let instance_dir = ctx
        .paths
        .instance_dir(instance_id)
        .map_err(|e| ToolError::Other(e.to_string()))?;
    let dir = tool_dir(&instance_dir, tool_id.as_str());
    let position = generated_position(&layers, tool_id.as_str());
    if position.is_none() && !dir.exists() {
        return Err(ToolError::NoOutput(tool_id.to_string()));
    }
    let removed = generation_numbers(&dir)?
        .into_iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>();
    if let Some(i) = position {
        layers.remove(i);
        save_layers(ctx, instance_id, definition, &mut manifest, layers)?;
    }
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    Ok(removed)
}

/// Files added, changed and removed between a tool's previous and current generations.
pub fn diff(ctx: &Ctx, instance_id: &str, tool_id: &ToolId) -> Result<DiffReport, ToolError> {
    let manifest = load_manifest(ctx, instance_id)?;
    let current = manifest
        .layers
        .layers()
        .iter()
        .find_map(|layer| match &layer.source {
            LayerSource::Generated {
                tool, generation, ..
            } if tool == tool_id => Some(generation.clone()),
            _ => None,
        })
        .ok_or_else(|| ToolError::NoOutput(tool_id.to_string()))?;
    let instance_dir = ctx
        .paths
        .instance_dir(instance_id)
        .map_err(|e| ToolError::Other(e.to_string()))?;
    let dir = tool_dir(&instance_dir, tool_id.as_str());
    let previous = read_record(&dir)?
        .previous
        .map(|g| g.generation)
        .ok_or_else(|| ToolError::NoPrevious(tool_id.to_string()))?;

    let before =
        generation_dir(&instance_dir, tool_id.as_str(), &previous).map_err(ToolError::Other)?;
    let after =
        generation_dir(&instance_dir, tool_id.as_str(), &current).map_err(ToolError::Other)?;
    let before_files = files_by_key(&before)?;
    let after_files = files_by_key(&after)?;

    let mut added = Vec::new();
    let mut changed = Vec::new();
    let mut removed = Vec::new();
    for (key, (rel, abs)) in &after_files {
        match before_files.get(key) {
            None => added.push(rel.clone()),
            Some((_, old_abs)) => {
                if std::fs::read(abs)? != std::fs::read(old_abs)? {
                    changed.push(rel.clone());
                }
            }
        }
    }
    for (key, (rel, _)) in &before_files {
        if !after_files.contains_key(key) {
            removed.push(rel.clone());
        }
    }
    Ok(DiffReport {
        tool: tool_id.to_string(),
        previous,
        current,
        added,
        changed,
        removed,
    })
}

/// A generation's files, keyed by lower-cased path, with the path as written and its location.
fn files_by_key(root: &Path) -> Result<BTreeMap<String, (String, PathBuf)>, ToolError> {
    let mut out = BTreeMap::new();
    for rel in written_files(root)? {
        let abs = rel
            .split('/')
            .fold(root.to_path_buf(), |p, part| p.join(part));
        out.insert(rel.to_ascii_lowercase(), (rel, abs));
    }
    Ok(out)
}
