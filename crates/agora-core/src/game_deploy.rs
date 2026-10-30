//! Game content deployment for instances (MASTER_SPEC §26.5).
//!
//! Deploys base files and mod content into a physical game runtime directory
//! using hardlinks or copies, harvests writes into the instance's writable layer,
//! and tracks deployed file state.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use agora_game_api::{
    BaseReference, CompiledGlob, GameDefinition, Layer, LayerId, LayerSource, LayerStack, RelPath,
    DEFAULT_COPY_PATTERNS,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::content_store::protect;
use crate::ctx::Ctx;
use crate::game_base::{get_modified_unix_ms, BaseManifest};
use crate::game_discovery::volume::VolumeDetector;
use crate::lock_manager::LockResource;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeployMode {
    /// Hardlinks into the game folder (rung 2): safe for content, writes land in the folder.
    Links,
    /// Real copies (rung 3).
    Copies,
    /// The same link farm as `Links`, run under `agora_vfs.dll` (rung 1): every write lands in
    /// the instance's writable layer, which the VFS shows on top, so the farm holds neither the
    /// writable layer nor private copies of declared-write files.
    Virtual,
}

impl DeployMode {
    pub fn as_str(self) -> &'static str {
        match self {
            DeployMode::Links => "links",
            DeployMode::Copies => "copies",
            DeployMode::Virtual => "virtual",
        }
    }

    /// The rung below this one, which a failed launch can offer (MASTER_SPEC §26.5): the virtual
    /// file system steps down to linked files, links to copies. Copies have none, and direct
    /// install (rung 4) is never offered as a fallback.
    pub fn next_fallback(self) -> Option<DeployMode> {
        match self {
            DeployMode::Virtual => Some(DeployMode::Links),
            DeployMode::Links => Some(DeployMode::Copies),
            DeployMode::Copies => None,
        }
    }

    /// What the mode is called to a person: "linked files", not "links".
    pub fn plain_name(self) -> &'static str {
        match self {
            DeployMode::Virtual => "the virtual file system",
            DeployMode::Links => "linked files",
            DeployMode::Copies => "copied files",
        }
    }

    /// Parse a user-facing mode name (`virtual`, `links`, `copies`).
    pub fn parse(s: &str) -> Option<DeployMode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "virtual" => Some(DeployMode::Virtual),
            "links" => Some(DeployMode::Links),
            "copies" => Some(DeployMode::Copies),
            _ => None,
        }
    }
}

impl std::fmt::Display for DeployMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The folder in the writable layer holding the VFS's whiteout markers (`.agvfs-wh\<path>.wh`).
const WHITEOUT_DIR: &str = ".agvfs-wh";
const WHITEOUT_SUFFIX: &str = ".wh";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentPlan {
    pub files: Vec<PlannedFile>,
    pub overrides: Vec<Override>,
    pub fingerprint: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedFile {
    pub path: RelPath,
    pub source: FileSource,
    pub placement: Placement,
    pub size: u64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_config_copy: bool,
    /// The layer whose file this is: the winner among the layers that hold the path.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub layer_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FileSource {
    Base {
        path: PathBuf,
        linked_to_store: bool,
    },
    Content {
        item_id: String,
        sha256: String,
    },
    Writable {
        path: PathBuf,
    },
    /// A file in a generated layer: the output of one tool run, kept in that generation's folder
    /// (MASTER_SPEC §26.9).
    Generated {
        tool: String,
        generation: String,
        path: PathBuf,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    Link,
    Copy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Override {
    pub path: RelPath,
    pub winner: String,
    pub hidden: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DeployOutcome {
    UpToDate {
        /// What changed in the plugin list; `None` when the game keeps none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plugins: Option<crate::game_plugins::PluginSyncReport>,
    },
    Built {
        linked: usize,
        copied: usize,
        copied_bytes: u64,
        #[serde(default)]
        config_copied: usize,
        /// Recorded files an in-place update kept as they were (0 for a full build).
        #[serde(default)]
        kept: usize,
        /// Files an in-place update took out of the farm, replaced ones included (0 for a full build).
        #[serde(default)]
        removed: usize,
        harvest: Option<HarvestReport>,
        /// What changed in the plugin list; `None` when the game keeps none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plugins: Option<crate::game_plugins::PluginSyncReport>,
    },
}

impl DeployOutcome {
    /// What this deploy changed in the instance's plugin list, if the game keeps one.
    pub fn plugins(&self) -> Option<&crate::game_plugins::PluginSyncReport> {
        match self {
            DeployOutcome::UpToDate { plugins } | DeployOutcome::Built { plugins, .. } => {
                plugins.as_ref()
            }
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarvestReport {
    pub copied_to_writable: Vec<RelPath>,
    pub base_files_changed: Vec<BaseFileChanged>,
    pub whiteouts_added: Vec<RelPath>,
    pub writable_files_removed: Vec<RelPath>,
}

impl HarvestReport {
    pub fn is_empty(&self) -> bool {
        self.copied_to_writable.is_empty()
            && self.base_files_changed.is_empty()
            && self.whiteouts_added.is_empty()
            && self.writable_files_removed.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseFileChanged {
    pub path: RelPath,
    pub linked_to_store: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployedFileRecord {
    pub path: RelPath,
    pub source: FileSource,
    pub placement: Placement,
    pub size: u64,
    pub modified_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentRecord {
    pub instance_id: String,
    pub base_id: String,
    pub mode: DeployMode,
    pub fingerprint: String,
    pub files: Vec<DeployedFileRecord>,
    pub deployed_at_unix_ms: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum DeployError {
    #[error("instance '{0}' not found")]
    InstanceNotFound(String),
    #[error("cannot deploy unpinned instance '{0}': direct install into the game folder is a later rung (§26.5, rung 4)")]
    UnpinnedInstance(String),
    #[error("base '{0}' not found")]
    BaseNotFound(String),
    #[error("base manifest is unreadable: {0}")]
    UnreadableBaseManifest(String),
    #[error("content item '{0}' not found in content store")]
    ContentNotFound(String),
    #[error("content item '{0}' is already present in instance")]
    ContentAlreadyPresent(String),
    #[error("id error: {0}")]
    Id(#[from] agora_game_api::IdError),
    #[error("layer error: {0}")]
    Layer(#[from] agora_game_api::LayerError),
    #[error("conflict: file '{file_path}' in layer '{file_layer}' conflicts with folder path '{folder_path}' in layer '{folder_layer}'")]
    FileFolderConflict {
        file_path: String,
        file_layer: String,
        folder_path: String,
        folder_layer: String,
    },
    /// A generated layer's folder is not on disk. Deployed without it, the game would silently lose
    /// the tool's output, so it is an error instead of an empty layer (MASTER_SPEC §26.9).
    #[error("the output of tool '{tool}' (generation {generation}) is missing from {}; rebuild it with `agora games instance tools run <instance> {tool}`", path.display())]
    GeneratedLayerMissing {
        tool: String,
        generation: String,
        path: PathBuf,
    },
    #[error("cannot deploy while game processes are running from deployment folder (PIDs: {0})")]
    ProcessesRunning(String),
    #[error("invalid deployment directory: {0}")]
    InvalidDeploymentDir(String),
    #[error("deployment record mismatch or corruption: {0}")]
    CorruptDeployment(String),
    #[error("lock error: {0}")]
    Lock(#[from] crate::error::LauncherError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("content error: {0}")]
    Content(#[from] crate::content_store::ContentError),
    #[error("plugin list: {0}")]
    Plugins(#[from] crate::game_plugins::PluginListError),
    #[error("{0}")]
    Other(String),
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_unix_ms() -> i64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
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

pub(crate) fn write_instance_manifest_atomic(
    ctx: &Ctx,
    instance_id: &str,
    manifest: &crate::game_instance::GameInstanceManifest,
) -> Result<(), DeployError> {
    let instance_dir = ctx
        .paths
        .instance_dir(instance_id)
        .map_err(|e| DeployError::Other(e.to_string()))?;
    let manifest_path = instance_dir.join("instance_manifest.json");
    let tmp_path = instance_dir.join(format!(
        "instance_manifest.json.{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));

    let manifest_bytes = serde_json::to_vec_pretty(manifest)?;
    std::fs::write(&tmp_path, manifest_bytes)?;
    std::fs::rename(&tmp_path, &manifest_path)?;
    Ok(())
}

fn validate_deployment_safety(
    deployment_dir: &Path,
    expected_instance_id: &str,
) -> Result<(), DeployError> {
    let game_dir = deployment_dir.join("game");
    if !game_dir.exists() {
        return Ok(());
    }

    // Must be inside a folder named "deployments"
    let is_inside_deployments = deployment_dir
        .parent()
        .and_then(|p| p.file_name())
        .map(|name| name == "deployments")
        .unwrap_or(false);

    if !is_inside_deployments {
        return Err(DeployError::InvalidDeploymentDir(format!(
            "refusing to delete '{}': parent is not a 'deployments' directory",
            game_dir.display()
        )));
    }

    let record_path = deployment_dir.join("deployment.json");
    if !record_path.exists() {
        return Err(DeployError::InvalidDeploymentDir(format!(
            "refusing to delete '{}': 'deployment.json' does not exist beside it",
            game_dir.display()
        )));
    }

    let record_content = std::fs::read_to_string(&record_path)?;
    let record: DeploymentRecord = serde_json::from_str(&record_content)
        .map_err(|e| DeployError::CorruptDeployment(format!("unreadable deployment.json: {e}")))?;

    if record.instance_id != expected_instance_id {
        return Err(DeployError::InvalidDeploymentDir(format!(
            "refusing to delete '{}': deployment.json names instance '{}' but expected '{}'",
            game_dir.display(),
            record.instance_id,
            expected_instance_id
        )));
    }

    Ok(())
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
// Planning
// ---------------------------------------------------------------------------

struct LayerFileEntry {
    path: RelPath,
    source: FileSource,
    size: u64,
    modified_unix_ms: i64,
}

struct LayerContribution {
    layer_id: String,
    files: Vec<LayerFileEntry>,
    own_copy: bool,
    /// Paths (lower-cased) this layer deletes from the layers below it. Applied before its own
    /// files are added, so a tool that deletes and rewrites a path keeps the rewritten file.
    whiteouts: Vec<String>,
}

pub fn plan(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    mode: DeployMode,
) -> Result<DeploymentPlan, DeployError> {
    plan_with_copy(ctx, instance_id, definition, mode, None)
}

/// [`plan`], with one content layer deployed as if its `own_copy` were on: its files are copies in
/// every mode. A tool's own mod is planned this way for the tool's run only (MASTER_SPEC §26.9).
pub(crate) fn plan_with_copy(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    mode: DeployMode,
    copy_layer: Option<&str>,
) -> Result<DeploymentPlan, DeployError> {
    plan_inner(ctx, instance_id, definition, mode, copy_layer, None)
}

/// A run's would-be output, planned in place of one tool's current output (MASTER_SPEC §26.9). The
/// manifest is the stack the game would have once the run is promoted, and `folder` is the run's
/// staging folder, which holds the files of the tool's generated layer.
pub(crate) struct WouldBe<'a> {
    pub manifest: crate::game_instance::GameInstanceManifest,
    pub tool: &'a str,
    pub folder: PathBuf,
}

/// The files the game would read if `would_be` were promoted. Planned as links, with the writable
/// layer left out: the game's own writes are not the tool's output.
pub(crate) fn plan_would_be(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    would_be: WouldBe<'_>,
) -> Result<DeploymentPlan, DeployError> {
    plan_inner(
        ctx,
        instance_id,
        definition,
        DeployMode::Links,
        None,
        Some(would_be),
    )
}

/// How many times the planner has run in this process. A measurement hook for the benchmark and the
/// launch test that count the plans one launch makes; the product never reads it.
#[doc(hidden)]
pub static PLAN_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn plan_inner(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    mode: DeployMode,
    copy_layer: Option<&str>,
    would_be: Option<WouldBe<'_>>,
) -> Result<DeploymentPlan, DeployError> {
    PLAN_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let manifest = match &would_be {
        Some(w) => w.manifest.clone(),
        None => crate::game_instance::get_manifest(ctx, instance_id).map_err(|e| match e {
            crate::game_instance::InstanceError::NotFound(id) => DeployError::InstanceNotFound(id),
            other => DeployError::Other(other.to_string()),
        })?,
    };

    let base_id = match &manifest.base {
        BaseReference::Pinned { id, .. } => id.clone(),
        BaseReference::Unpinned { .. } => {
            return Err(DeployError::UnpinnedInstance(instance_id.to_string()));
        }
    };

    let base_manifest_path = ctx.paths.base_manifest_path(&base_id);
    if !base_manifest_path.exists() {
        return Err(DeployError::BaseNotFound(base_id));
    }
    let base_content = std::fs::read_to_string(&base_manifest_path)
        .map_err(|e| DeployError::UnreadableBaseManifest(e.to_string()))?;
    let base_manifest: BaseManifest = serde_json::from_str(&base_content)
        .map_err(|e| DeployError::UnreadableBaseManifest(e.to_string()))?;

    let bases_root = base_manifest
        .location
        .parent()
        .ok_or_else(|| DeployError::Other("base location has no parent".into()))?;
    let deployment_dir = bases_root.join("deployments").join(instance_id);
    let game_dir = deployment_dir.join("game");
    let instance_dir = ctx
        .paths
        .instance_dir(instance_id)
        .map_err(|e| DeployError::Other(e.to_string()))?;

    let mut layers = Vec::new();
    let mut warnings = Vec::new();

    // 1. Base layer files
    {
        let mut base_files = Vec::with_capacity(base_manifest.files.len());
        for bf in &base_manifest.files {
            let rel_path = RelPath::new(&bf.path)
                .map_err(|e| DeployError::Other(format!("invalid base file path: {e}")))?;
            let src_path = base_manifest.location.join(&bf.path);
            base_files.push(LayerFileEntry {
                path: rel_path,
                source: FileSource::Base {
                    path: src_path,
                    linked_to_store: bf.linked,
                },
                size: bf.size,
                modified_unix_ms: bf.modified_unix_ms,
            });
        }
        layers.push(LayerContribution {
            layer_id: "base".to_string(),
            files: base_files,
            own_copy: false,
            whiteouts: Vec::new(),
        });
    }

    // 2. Enabled content layers in stack order (lowest first)
    for layer in manifest.layers.layers() {
        if !layer.enabled {
            continue;
        }
        if let LayerSource::Content { content: item_id } = &layer.source {
            let item = crate::content_store::get_item(ctx, item_id)
                .map_err(|_| DeployError::ContentNotFound(item_id.clone()))?;
            let mut content_files = Vec::new();
            let src_path = layer.source_path.as_str().trim_matches('/');
            let src_parts: Vec<&str> = if src_path.is_empty() {
                Vec::new()
            } else {
                src_path.split('/').filter(|s| !s.is_empty()).collect()
            };

            for cf in &item.files {
                let cf_parts: Vec<&str> = cf
                    .path
                    .as_str()
                    .split('/')
                    .filter(|s| !s.is_empty())
                    .collect();
                let stripped_rel = if src_parts.is_empty() {
                    Some(cf.path.as_str().to_string())
                } else if cf_parts.len() > src_parts.len()
                    && src_parts
                        .iter()
                        .zip(cf_parts.iter())
                        .all(|(sp, cp)| sp.eq_ignore_ascii_case(cp))
                {
                    Some(cf_parts[src_parts.len()..].join("/"))
                } else {
                    None
                };

                if let Some(rel) = stripped_rel {
                    let full_rel_str = if layer.mount_path.as_str().is_empty() {
                        rel
                    } else {
                        format!(
                            "{}/{}",
                            layer.mount_path.as_str().trim_end_matches('/'),
                            rel.trim_start_matches('/')
                        )
                    };
                    let rel_path = RelPath::new(full_rel_str)
                        .map_err(|e| DeployError::Other(format!("invalid mounted path: {e}")))?;
                    content_files.push(LayerFileEntry {
                        path: rel_path,
                        source: FileSource::Content {
                            item_id: item_id.clone(),
                            sha256: cf.sha256.clone(),
                        },
                        size: cf.size,
                        modified_unix_ms: 0,
                    });
                }
            }

            if content_files.is_empty() {
                warnings.push(format!(
                    "layer '{}': source_path '{}' matches no files",
                    layer.id, layer.source_path
                ));
            } else {
                layers.push(LayerContribution {
                    layer_id: layer.id.as_str().to_string(),
                    files: content_files,
                    own_copy: layer.own_copy || copy_layer == Some(layer.id.as_str()),
                    whiteouts: Vec::new(),
                });
            }
        }
    }

    // 3. Generated layers, in stack order (MASTER_SPEC §26.9): each one is the output of a tool
    // run, kept whole in its generation's folder. A folder that is not on disk is an error.
    for layer in manifest.layers.layers() {
        let LayerSource::Generated {
            tool, generation, ..
        } = &layer.source
        else {
            continue;
        };
        if !layer.enabled {
            continue;
        }
        // A would-be output is read from its staging folder, whose root may hold the VFS's whiteout
        // markers: those are not files of the output.
        let staged = matches!(&would_be, Some(w) if w.tool == tool.as_str());
        let folder = match &would_be {
            Some(w) if staged => w.folder.clone(),
            _ => {
                let folder =
                    crate::game_tools::generation_dir(&instance_dir, tool.as_str(), generation)
                        .map_err(DeployError::Other)?;
                if !folder.is_dir() {
                    return Err(DeployError::GeneratedLayerMissing {
                        tool: tool.to_string(),
                        generation: generation.clone(),
                        path: folder,
                    });
                }
                folder
            }
        };
        let mut generated_files = Vec::new();
        walk_generated_dir(
            &folder,
            Path::new(""),
            tool.as_str(),
            generation,
            staged,
            &mut generated_files,
        )?;
        layers.push(LayerContribution {
            layer_id: layer.id.as_str().to_string(),
            files: generated_files,
            own_copy: layer.own_copy,
            whiteouts: layer
                .whiteouts
                .iter()
                .map(|w| w.as_str().to_ascii_lowercase())
                .collect(),
        });
    }

    // 4. Writable layer files
    let writable_dir = instance_dir.join("writable");
    // Under the VFS the writable layer is shown on top of the farm, not placed in it, so the
    // farm (and its fingerprint) does not change when the game writes.
    if would_be.is_none() && mode != DeployMode::Virtual && writable_dir.exists() {
        let mut writable_files = Vec::new();
        walk_writable_dir(&writable_dir, Path::new(""), &mut writable_files)?;
        if !writable_files.is_empty() {
            layers.push(LayerContribution {
                layer_id: "writable".to_string(),
                files: writable_files,
                own_copy: false,
                whiteouts: Vec::new(),
            });
        }
    }

    // 5. Check for File-versus-Folder conflicts across layers
    // A path that is a file in one layer and a folder in another is an error naming both paths and layers.
    {
        #[derive(Clone)]
        struct FileLayerItem {
            lower_path: String,
            original_path: RelPath,
            layer_id: String,
        }
        let mut all_files = Vec::new();
        for l in &layers {
            for f in &l.files {
                all_files.push(FileLayerItem {
                    lower_path: f.path.as_str().to_ascii_lowercase(),
                    original_path: f.path.clone(),
                    layer_id: l.layer_id.clone(),
                });
            }
        }
        all_files.sort_by(|a, b| a.lower_path.cmp(&b.lower_path));

        for i in 0..all_files.len() {
            let cur = &all_files[i];
            let prefix = format!("{}/", cur.lower_path);
            if let Some(next) = all_files.get(i + 1) {
                if next.lower_path.starts_with(&prefix) {
                    return Err(DeployError::FileFolderConflict {
                        file_path: cur.original_path.to_string(),
                        file_layer: cur.layer_id.clone(),
                        folder_path: next.original_path.to_string(),
                        folder_layer: next.layer_id.clone(),
                    });
                }
            }
        }
    }

    // 6. Build winning map and record overrides
    struct Candidate {
        path: RelPath,
        source: FileSource,
        size: u64,
        modified_unix_ms: i64,
        layer_id: String,
        hidden_layers: Vec<String>,
        own_copy: bool,
    }

    let mut candidates: BTreeMap<String, Candidate> = BTreeMap::new();
    for l in layers {
        for marker in &l.whiteouts {
            let prefix = format!("{marker}/");
            candidates.retain(|key, _| !(*key == *marker || key.starts_with(&prefix)));
        }
        for f in l.files {
            let key = f.path.as_str().to_ascii_lowercase();
            if let Some(existing) = candidates.get_mut(&key) {
                let mut hidden = existing.hidden_layers.clone();
                hidden.push(existing.layer_id.clone());
                *existing = Candidate {
                    path: f.path,
                    source: f.source,
                    size: f.size,
                    modified_unix_ms: f.modified_unix_ms,
                    layer_id: l.layer_id.clone(),
                    hidden_layers: hidden,
                    own_copy: l.own_copy,
                };
            } else {
                candidates.insert(
                    key,
                    Candidate {
                        path: f.path,
                        source: f.source,
                        size: f.size,
                        modified_unix_ms: f.modified_unix_ms,
                        layer_id: l.layer_id.clone(),
                        hidden_layers: Vec::new(),
                        own_copy: l.own_copy,
                    },
                );
            }
        }
    }

    // 7. Remove whiteouts from writable layer
    for layer in manifest.layers.layers() {
        if matches!(layer.source, LayerSource::Writable { .. }) {
            for wh in &layer.whiteouts {
                let key = wh.as_str().to_ascii_lowercase();
                candidates.remove(&key);
            }
        }
    }
    // The VFS records a deletion as a marker in the writable layer; it hides lower files in every
    // mode. A writable file that is still there (the game recreated the path) is not hidden.
    let writable_markers = if would_be.is_some() {
        Vec::new()
    } else {
        read_whiteout_markers(&writable_dir)?
    };
    for marker in writable_markers {
        let prefix = format!("{marker}/");
        candidates.retain(|key, cand| {
            matches!(cand.source, FileSource::Writable { .. })
                || !(*key == marker || key.starts_with(&prefix))
        });
    }

    // 8. Determine placement & compute fingerprint
    let detector = VolumeDetector::new();
    let deploy_vol = detector.get_volume_info(&game_dir);
    // Every content object lives under the store's objects folder, so its volume is asked once:
    // asking per file (GetVolumePathNameW walks each path component) cost ~90 s per plan for a
    // 200,000-file MO2 import.
    let store_vol = detector.get_volume_info(&ctx.paths.content_objects_dir());
    let store_links_here = matches!(
        (&store_vol, &deploy_vol),
        (Some(ov), Some(dv))
            if ov.id.eq_ignore_ascii_case(&dv.id) && ov.supports_hardlinks && dv.supports_hardlinks
    );

    let mut planned_files = Vec::with_capacity(candidates.len());
    let mut overrides = Vec::new();
    let mut fingerprint_lines = Vec::with_capacity(candidates.len());
    // The generation is part of the farm's identity, so promoting or rolling back a tool's output
    // rebuilds the deployment even where the bytes happen to match.
    for layer in manifest.layers.layers() {
        if let (
            true,
            LayerSource::Generated {
                tool, generation, ..
            },
        ) = (layer.enabled, &layer.source)
        {
            fingerprint_lines.push(format!("generated-layer\t{tool}\t{generation}\n"));
        }
    }

    let rules = PathRules::new(definition);
    for (_key, cand) in candidates {
        // A generated layer's files are copies in every mode, as the writable layer's are.
        let generated = matches!(cand.source, FileSource::Generated { .. });
        let declared = rules.is_declared_write(cand.path.as_str());
        let is_config_copy = mode == DeployMode::Links
            && cand.size <= 1024 * 1024
            && rules.is_copy_pattern(cand.path.as_str())
            && !declared
            && !cand.own_copy
            && !generated
            && !matches!(cand.source, FileSource::Writable { .. });

        let placement = match mode {
            _ if generated => Placement::Copy,
            DeployMode::Copies => Placement::Copy,
            DeployMode::Virtual => {
                if cand.own_copy || declared {
                    Placement::Copy
                } else {
                    match &cand.source {
                        FileSource::Writable { .. } | FileSource::Generated { .. } => {
                            Placement::Copy
                        }
                        FileSource::Base { .. } => Placement::Link,
                        FileSource::Content { .. } if store_links_here => Placement::Link,
                        FileSource::Content { .. } => Placement::Copy,
                    }
                }
            }
            DeployMode::Links => {
                // A declared-write path gets the instance's own copy in every mode.
                // An own_copy layer deploys all its files as copies.
                // Small text files matching copy patterns (<= 1 MiB) are placed as copies too.
                if cand.own_copy || declared || is_config_copy {
                    Placement::Copy
                } else {
                    match &cand.source {
                        FileSource::Writable { .. } | FileSource::Generated { .. } => {
                            Placement::Copy
                        }
                        FileSource::Base { .. } => Placement::Link,
                        FileSource::Content { .. } if store_links_here => Placement::Link,
                        FileSource::Content { .. } => Placement::Copy,
                    }
                }
            }
        };

        let source_desc = match &cand.source {
            FileSource::Base {
                path,
                linked_to_store,
            } => format!("base:{}:{}", path.display(), linked_to_store),
            FileSource::Content { item_id, sha256 } => format!("content:{item_id}:{sha256}"),
            FileSource::Writable { path } => format!(
                "writable:{}:{}:{}",
                path.display(),
                cand.size,
                cand.modified_unix_ms
            ),
            FileSource::Generated {
                tool,
                generation,
                path,
            } => format!("generated:{tool}:{generation}:{}", path.display()),
        };

        let placement_desc = match placement {
            Placement::Link => "link",
            Placement::Copy => "copy",
        };

        fingerprint_lines.push(format!(
            "{}\t{}\t{}\n",
            cand.path.as_str(),
            source_desc,
            placement_desc
        ));

        if !cand.hidden_layers.is_empty() {
            overrides.push(Override {
                path: cand.path.clone(),
                winner: cand.layer_id.clone(),
                hidden: cand.hidden_layers,
            });
        }

        planned_files.push(PlannedFile {
            path: cand.path,
            source: cand.source,
            placement,
            size: cand.size,
            is_config_copy,
            layer_id: cand.layer_id,
        });
    }

    fingerprint_lines.sort();
    let mut hasher = Sha256::new();
    for line in fingerprint_lines {
        hasher.update(line.as_bytes());
    }
    let fingerprint = format!("{:x}", hasher.finalize());

    planned_files.sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));
    overrides.sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));

    Ok(DeploymentPlan {
        files: planned_files,
        overrides,
        fingerprint,
        warnings,
    })
}

/// Every file of one generation's folder, as a generated layer's files. A missing folder was
/// checked by the caller; a folder with nothing in it is an empty layer, which is a valid output.
fn walk_generated_dir(
    root: &Path,
    rel: &Path,
    tool: &str,
    generation: &str,
    staged: bool,
    out: &mut Vec<LayerFileEntry>,
) -> Result<(), DeployError> {
    let cur = if rel.as_os_str().is_empty() {
        root.to_path_buf()
    } else {
        root.join(rel)
    };
    for entry in std::fs::read_dir(&cur)? {
        let entry = entry?;
        let abs_path = entry.path();
        let meta = entry.metadata()?;
        if is_reparse_point_or_symlink(&meta) {
            continue;
        }
        if staged
            && rel.as_os_str().is_empty()
            && entry.file_name() == crate::game_tools::WHITEOUT_DIR
        {
            continue;
        }
        let child_rel = rel.join(entry.file_name());
        if meta.is_dir() {
            walk_generated_dir(root, &child_rel, tool, generation, staged, out)?;
        } else if meta.is_file() {
            let rel_str = child_rel.to_string_lossy().replace('\\', "/");
            let rel_path = RelPath::new(rel_str).map_err(|e| {
                DeployError::Other(format!("invalid path in generated output: {e}"))
            })?;
            out.push(LayerFileEntry {
                path: rel_path,
                source: FileSource::Generated {
                    tool: tool.to_string(),
                    generation: generation.to_string(),
                    path: abs_path.clone(),
                },
                size: meta.len(),
                modified_unix_ms: get_modified_unix_ms(&meta),
            });
        }
    }
    Ok(())
}

fn walk_writable_dir(
    root: &Path,
    rel: &Path,
    out: &mut Vec<LayerFileEntry>,
) -> Result<(), DeployError> {
    let cur = if rel.as_os_str().is_empty() {
        root.to_path_buf()
    } else {
        root.join(rel)
    };
    if !cur.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(&cur)? {
        let entry = entry?;
        let abs_path = entry.path();
        let meta = entry.metadata()?;
        if is_reparse_point_or_symlink(&meta) {
            continue;
        }
        let file_name = entry.file_name();
        if rel.as_os_str().is_empty()
            && file_name
                .to_string_lossy()
                .eq_ignore_ascii_case(WHITEOUT_DIR)
        {
            // The VFS's whiteout markers are bookkeeping, never game files.
            continue;
        }
        let child_rel = rel.join(&file_name);
        if meta.is_dir() {
            walk_writable_dir(root, &child_rel, out)?;
        } else if meta.is_file() {
            let rel_str = child_rel.to_string_lossy().replace('\\', "/");
            let rel_path = RelPath::new(rel_str)
                .map_err(|e| DeployError::Other(format!("invalid path in writable layer: {e}")))?;
            let mtime = get_modified_unix_ms(&meta);
            out.push(LayerFileEntry {
                path: rel_path,
                source: FileSource::Writable {
                    path: abs_path.clone(),
                },
                size: meta.len(),
                modified_unix_ms: mtime,
            });
        }
    }
    Ok(())
}

/// The paths (lower-cased, `/`-separated) the VFS has recorded as deleted in `writable_dir`.
fn read_whiteout_markers(writable_dir: &Path) -> Result<Vec<String>, DeployError> {
    fn walk(dir: &Path, rel: &str, out: &mut Vec<String>) -> Result<(), DeployError> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let meta = entry.metadata()?;
            if is_reparse_point_or_symlink(&meta) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let child = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            if meta.is_dir() {
                walk(&entry.path(), &child, out)?;
            } else if meta.is_file() {
                if let Some(path) = child.strip_suffix(WHITEOUT_SUFFIX) {
                    if !path.is_empty() {
                        out.push(path.to_ascii_lowercase());
                    }
                }
            }
        }
        Ok(())
    }

    let dir = writable_dir.join(WHITEOUT_DIR);
    let mut out = Vec::new();
    if dir.is_dir() {
        walk(&dir, "", &mut out)?;
    }
    Ok(out)
}

/// Remove the VFS's whiteout marker for `rel`, because a file now exists there in the writable
/// layer (a marker would hide it again under the VFS).
fn clear_whiteout_marker(writable_dir: &Path, rel: &RelPath) {
    let mut marker = writable_dir
        .join(WHITEOUT_DIR)
        .join(rel.as_str())
        .into_os_string();
    marker.push(WHITEOUT_SUFFIX);
    let _ = std::fs::remove_file(PathBuf::from(marker));
}

// ---------------------------------------------------------------------------
// Deploy and Undeploy
// ---------------------------------------------------------------------------

/// The file on disk that a planned file's bytes come from: a base file, a content object or a
/// writable-layer file.
pub fn source_path(ctx: &Ctx, source: &FileSource) -> PathBuf {
    match source {
        FileSource::Base { path, .. } => path.clone(),
        FileSource::Content { sha256, .. } => ctx.paths.content_object_path(sha256),
        FileSource::Writable { path } => path.clone(),
        FileSource::Generated { path, .. } => path.clone(),
    }
}

/// Every path the game will see under `mode`, `/`-separated, sorted and without case-only
/// duplicates: the deployment plan's files, plus the writable layer. The plan leaves the writable
/// layer out under the virtual file system, which shows it on top of the farm, so it is added
/// here. Whiteouts are already applied to the plan, and the writable layer's own files are never
/// hidden by them.
pub fn visible_paths(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    mode: DeployMode,
) -> Result<Vec<String>, DeployError> {
    let plan = plan(ctx, instance_id, definition, mode)?;
    visible_paths_of(ctx, instance_id, &plan)
}

/// [`visible_paths`] for a plan already made for `instance_id` in the deployment's mode.
pub(crate) fn visible_paths_of(
    ctx: &Ctx,
    instance_id: &str,
    plan: &DeploymentPlan,
) -> Result<Vec<String>, DeployError> {
    let mut paths: Vec<String> = plan
        .files
        .iter()
        .map(|file| file.path.as_str().to_string())
        .collect();
    let instance_dir = ctx
        .paths
        .instance_dir(instance_id)
        .map_err(|e| DeployError::Other(e.to_string()))?;
    let writable_dir = instance_dir.join("writable");
    if writable_dir.exists() {
        let mut writable_files = Vec::new();
        walk_writable_dir(&writable_dir, Path::new(""), &mut writable_files)?;
        paths.extend(
            writable_files
                .into_iter()
                .map(|file| file.path.as_str().to_string()),
        );
    }
    paths.sort_by_key(|path| path.to_ascii_lowercase());
    paths.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    Ok(paths)
}

/// The file on disk whose bytes the game sees at `rel` (a `/`-separated path relative to the game
/// root, matched case-insensitively) under `mode`. The writable layer is on top, so it wins. `None`
/// when no file the game would see has that path.
pub fn visible_file_source(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    mode: DeployMode,
    rel: &str,
) -> Result<Option<PathBuf>, DeployError> {
    let instance_dir = ctx
        .paths
        .instance_dir(instance_id)
        .map_err(|e| DeployError::Other(e.to_string()))?;
    let writable = instance_dir.join("writable").join(rel);
    if writable.is_file() {
        return Ok(Some(writable));
    }
    let plan = plan(ctx, instance_id, definition, mode)?;
    Ok(plan
        .files
        .iter()
        .find(|file| file.path.as_str().eq_ignore_ascii_case(rel))
        .map(|file| source_path(ctx, &file.source)))
}

pub fn deployment_dir(ctx: &Ctx, instance_id: &str) -> Result<Option<PathBuf>, DeployError> {
    let manifest = match crate::game_instance::get_manifest(ctx, instance_id) {
        Ok(m) => m,
        Err(crate::game_instance::InstanceError::NotFound(_)) => return Ok(None),
        Err(e) => return Err(DeployError::Other(e.to_string())),
    };
    let base_id = match &manifest.base {
        BaseReference::Pinned { id, .. } => id,
        BaseReference::Unpinned { .. } => return Ok(None),
    };
    let base_manifest_path = ctx.paths.base_manifest_path(base_id);
    if !base_manifest_path.exists() {
        return Ok(None);
    }
    let base_content = std::fs::read_to_string(&base_manifest_path)
        .map_err(|e| DeployError::UnreadableBaseManifest(e.to_string()))?;
    let base_manifest: BaseManifest = serde_json::from_str(&base_content)
        .map_err(|e| DeployError::UnreadableBaseManifest(e.to_string()))?;
    let Some(bases_root) = base_manifest.location.parent() else {
        return Ok(None);
    };
    let deployment_dir = bases_root.join("deployments").join(instance_id);
    let game_dir = deployment_dir.join("game");
    let record_path = deployment_dir.join("deployment.json");
    if game_dir.exists() && record_path.exists() {
        Ok(Some(game_dir))
    } else {
        Ok(None)
    }
}

pub fn deploy(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    mode: DeployMode,
) -> Result<DeployOutcome, DeployError> {
    let (outcome, _plan) =
        deploy_checked::<DeployError>(ctx, instance_id, definition, mode, |_| Ok(()))??;
    Ok(outcome)
}

/// [`deploy`] for a caller that wants to see the plan before anything is built (a launch). The plan
/// is made under the instance's lock, and `before_build` sees it first, so it can refuse the launch
/// with nothing built. Returns the outcome and the plan the deployment was built from. The outer
/// `Err` is `before_build`'s refusal; the inner one is the deploy's own error.
pub(crate) fn deploy_checked<E>(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    mode: DeployMode,
    before_build: impl FnOnce(&DeploymentPlan) -> Result<(), E>,
) -> Result<Result<(DeployOutcome, DeploymentPlan), DeployError>, E> {
    let _lock = match ctx
        .lock_manager
        .acquire(LockResource::Instance(instance_id.to_string()), "deploy")
    {
        Ok(lock) => lock,
        Err(e) => return Ok(Err(e.into())),
    };
    // An interrupted tool swap is put back before anything is deployed (MASTER_SPEC §26.9).
    if let Err(e) = crate::game_tool_swap::recover_game(ctx, definition) {
        return Ok(Err(DeployError::Other(e.to_string())));
    }
    deploy_locked_checked(ctx, instance_id, definition, mode, None, before_build)
}

/// [`deploy`] for a caller that already holds the instance's lock: a tool run holds it for the
/// whole run, and the lock is not re-entrant.
pub(crate) fn deploy_locked(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    mode: DeployMode,
) -> Result<DeployOutcome, DeployError> {
    deploy_locked_with_copy(ctx, instance_id, definition, mode, None)
}

/// What [`place_files`] did: the counts, and the record of each file as the folder now holds it.
pub(crate) struct PlacedFiles {
    pub linked: usize,
    pub copied: usize,
    pub copied_bytes: u64,
    pub config_copied: usize,
    pub recorded: Vec<DeployedFileRecord>,
}

/// The executor that writes a plan's files into `root`: a hard link where the plan places a link (a
/// copy when the link fails), a real copy otherwise. Each file is recorded with the placement, size
/// and time it has in `root`. The folders are created first, in one sequential pass; then the files
/// are placed on a bounded pool of threads (MASTER_SPEC §26.5). The record is in plan order, and the
/// first failure in plan order is the one returned. A deployment writes its staging folder with it,
/// and a swap's copy of `Data` writes its copy with it (MASTER_SPEC §26.9), so there is one copy loop.
pub(crate) fn place_files(
    ctx: &Ctx,
    files: &[PlannedFile],
    root: &Path,
) -> Result<PlacedFiles, DeployError> {
    create_planned_dirs(root, files)?;
    let sources: Vec<PathBuf> = files
        .iter()
        .map(|file| source_path(ctx, &file.source))
        .collect();
    let results = run_in_pool(files.len(), placement_threads(files.len()), |index| {
        place_one(
            &sources[index],
            &root.join(files[index].path.as_str()),
            &files[index],
        )
    })?;

    let mut placed = PlacedFiles {
        linked: 0,
        copied: 0,
        copied_bytes: 0,
        config_copied: 0,
        recorded: Vec::with_capacity(results.len()),
    };
    for (file, result) in files.iter().zip(results) {
        if file.is_config_copy {
            placed.config_copied += 1;
        }
        if result.linked {
            placed.linked += 1;
        } else {
            placed.copied += 1;
            placed.copied_bytes += result.size;
        }
        placed.recorded.push(result.record);
    }
    Ok(placed)
}

/// Whether a file in a farm differs from its record (MASTER_SPEC §26.9): a size change is a change;
/// an unchanged time is not; a linked file that moved in time is a change, since a link is the content
/// object itself; a copy whose time moved is compared byte for byte with its source. Link capture and
/// a swap's copy both decide change this way.
pub(crate) fn farm_file_differs(
    ctx: &Ctx,
    file: &FarmFile,
    recorded: &DeployedFileRecord,
) -> Result<bool, DeployError> {
    if file.size != recorded.size {
        return Ok(true);
    }
    if file.modified_unix_ms == recorded.modified_unix_ms {
        return Ok(false);
    }
    if recorded.placement == Placement::Link {
        return Ok(true);
    }
    let source = source_path(ctx, &recorded.source);
    Ok(std::fs::read(&file.abs_path)? != std::fs::read(&source)?)
}

/// [`deploy_locked`], with one content layer deployed as copies (see [`plan_with_copy`]). The
/// deployment's fingerprint records the copies, so a deployment made this way is not mistaken for
/// the ordinary one. The caller discards it when its run is over.
pub(crate) fn deploy_locked_with_copy(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    mode: DeployMode,
    copy_layer: Option<&str>,
) -> Result<DeployOutcome, DeployError> {
    let (outcome, _plan) = deploy_locked_checked::<DeployError>(
        ctx,
        instance_id,
        definition,
        mode,
        copy_layer,
        |_| Ok(()),
    )??;
    Ok(outcome)
}

/// Where a deployment goes: the instance, its base, and the folders the deployment uses.
struct DeployTarget {
    manifest: crate::game_instance::GameInstanceManifest,
    base_id: String,
    base_manifest: BaseManifest,
    deployment_dir: PathBuf,
    game_dir: PathBuf,
    record_path: PathBuf,
}

fn deploy_target(ctx: &Ctx, instance_id: &str) -> Result<DeployTarget, DeployError> {
    let manifest = crate::game_instance::get_manifest(ctx, instance_id).map_err(|e| match e {
        crate::game_instance::InstanceError::NotFound(id) => DeployError::InstanceNotFound(id),
        other => DeployError::Other(other.to_string()),
    })?;

    let base_id = match &manifest.base {
        BaseReference::Pinned { id, .. } => id.clone(),
        BaseReference::Unpinned { .. } => {
            return Err(DeployError::UnpinnedInstance(instance_id.to_string()));
        }
    };

    let base_manifest_path = ctx.paths.base_manifest_path(&base_id);
    if !base_manifest_path.exists() {
        return Err(DeployError::BaseNotFound(base_id));
    }
    let base_content = std::fs::read_to_string(&base_manifest_path)
        .map_err(|e| DeployError::UnreadableBaseManifest(e.to_string()))?;
    let base_manifest: BaseManifest = serde_json::from_str(&base_content)
        .map_err(|e| DeployError::UnreadableBaseManifest(e.to_string()))?;

    let bases_root = base_manifest
        .location
        .parent()
        .ok_or_else(|| DeployError::Other("base location has no parent".into()))?;
    let deployment_dir = bases_root.join("deployments").join(instance_id);
    let game_dir = deployment_dir.join("game");
    let record_path = deployment_dir.join("deployment.json");
    Ok(DeployTarget {
        manifest,
        base_id,
        base_manifest,
        deployment_dir,
        game_dir,
        record_path,
    })
}

/// The deploy, with the lock held. The plan is made once here, `before_build` sees it before the
/// farm is checked or built, and the deployment is built from that same plan (unless harvesting
/// changed the instance, in which case the plan is made again, as it always was).
fn deploy_locked_checked<E>(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    mode: DeployMode,
    copy_layer: Option<&str>,
    before_build: impl FnOnce(&DeploymentPlan) -> Result<(), E>,
) -> Result<Result<(DeployOutcome, DeploymentPlan), DeployError>, E> {
    let target = match deploy_target(ctx, instance_id) {
        Ok(target) => target,
        Err(e) => return Ok(Err(e)),
    };

    if target.game_dir.exists() {
        let running = crate::game_launch::processes_running_from(&target.game_dir);
        if !running.is_empty() {
            let pids: Vec<String> = running.iter().map(|p| p.pid.to_string()).collect();
            return Ok(Err(DeployError::ProcessesRunning(pids.join(", "))));
        }
    }

    let plan = match plan_with_copy(ctx, instance_id, definition, mode, copy_layer) {
        Ok(plan) => plan,
        Err(e) => return Ok(Err(e)),
    };
    before_build(&plan)?;
    Ok(build_deployment(
        ctx,
        instance_id,
        definition,
        mode,
        copy_layer,
        target,
        plan,
    ))
}

/// Build the deployment from `plan`, or keep the one in place when it is up to date. Runs under the
/// instance lock.
fn build_deployment(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    mode: DeployMode,
    copy_layer: Option<&str>,
    target: DeployTarget,
    plan: DeploymentPlan,
) -> Result<(DeployOutcome, DeploymentPlan), DeployError> {
    let DeployTarget {
        manifest,
        base_id,
        base_manifest,
        deployment_dir,
        game_dir,
        record_path,
    } = target;
    let mut current_plan = plan;

    // Check if current deployment is up to date. A marker left by an update that stopped partway
    // makes it not up to date (MASTER_SPEC §26.5).
    if record_path.exists() && game_dir.exists() && !updating_marker(&deployment_dir).exists() {
        if let Ok(content) = std::fs::read_to_string(&record_path) {
            if let Ok(record) = serde_json::from_str::<DeploymentRecord>(&content) {
                if record.fingerprint == current_plan.fingerprint
                    && record.mode == mode
                    && record.instance_id == instance_id
                {
                    let own_copy_items: std::collections::HashSet<&str> = manifest
                        .layers
                        .layers()
                        .iter()
                        .filter(|l| l.enabled && l.own_copy)
                        .filter_map(|l| match &l.source {
                            LayerSource::Content { content } => Some(content.as_str()),
                            _ => None,
                        })
                        .collect();
                    if farm_matches_record(
                        &game_dir,
                        &record,
                        &PathRules::new(definition),
                        &own_copy_items,
                    ) {
                        let plugins = activate_plugins(
                            ctx,
                            instance_id,
                            definition,
                            &manifest,
                            &base_manifest,
                            &current_plan,
                            links_view(mode, &current_plan),
                        )?;
                        return Ok((DeployOutcome::UpToDate { plugins }, current_plan));
                    }
                }
            }
        }
    }

    // Existing deployment is not up to date. A farm whose update stopped partway (the marker is
    // there) is not trusted: it is discarded without a harvest, and built again. Otherwise the farm
    // is harvested first, then updated in place where it can be, and discarded where it cannot.
    let harvest = if updating_marker(&deployment_dir).exists() {
        discard_untrusted_farm(&deployment_dir)?;
        None
    } else if record_path.exists() {
        let harvested = harvest_changes(ctx, instance_id, &deployment_dir, &manifest)?;
        // If anything was harvested into the writable layer, re-compute the plan
        if !harvested.report.is_empty() {
            current_plan = plan_with_copy(ctx, instance_id, definition, mode, copy_layer)?;
        }
        match plan_in_place(ctx, &game_dir, &harvested, &current_plan, &base_id) {
            Some((kept, remove, place)) => {
                let applied = apply_in_place(
                    ctx,
                    &deployment_dir,
                    InPlaceUpdate {
                        instance_id: instance_id.to_string(),
                        base_id: base_id.clone(),
                        mode,
                        plan: current_plan,
                        harvest: harvested.report,
                        kept,
                        remove,
                        place,
                    },
                )?;
                let plugins = activate_plugins(
                    ctx,
                    instance_id,
                    definition,
                    &manifest,
                    &base_manifest,
                    &applied.plan,
                    links_view(mode, &applied.plan),
                )?;
                let outcome = DeployOutcome::Built {
                    linked: applied.linked,
                    copied: applied.copied,
                    copied_bytes: applied.copied_bytes,
                    config_copied: applied.config_copied,
                    kept: applied.kept,
                    removed: applied.removed,
                    harvest: Some(applied.harvest),
                    plugins,
                };
                return Ok((outcome, applied.plan));
            }
            None => {
                // The farm is taken away as the full rebuild takes it (harvest, then delete).
                remove_farm_and_record(&deployment_dir)?;
                Some(harvested.report)
            }
        }
    } else {
        None
    };

    // Build the new deployment
    std::fs::create_dir_all(&deployment_dir)?;
    let unique = format!("{}-{}", std::process::id(), uuid::Uuid::new_v4().simple());
    let staging_dir = deployment_dir.join(format!("game.partial-{unique}"));
    std::fs::create_dir_all(&staging_dir)?;
    protect::grant_delete_child(&staging_dir)?;

    let mut guard = StagingGuard {
        path: &staging_dir,
        active: true,
    };

    let placed = place_files(ctx, &current_plan.files, &staging_dir)?;
    let linked_count = placed.linked;
    let copied_count = placed.copied;
    let copied_bytes = placed.copied_bytes;
    let config_copied_count = placed.config_copied;
    let recorded_files = placed.recorded;

    // Rename staging folder to game folder
    std::fs::rename(&staging_dir, &game_dir)?;
    guard.active = false;

    // Write deployment.json atomically
    let record = DeploymentRecord {
        instance_id: instance_id.to_string(),
        base_id,
        mode,
        fingerprint: current_plan.fingerprint.clone(),
        files: recorded_files,
        deployed_at_unix_ms: now_unix_ms(),
    };
    let tmp_record_path = deployment_dir.join(format!("deployment.json.tmp-{unique}"));
    let record_bytes = serde_json::to_vec(&record)?;
    std::fs::write(&tmp_record_path, record_bytes)?;
    std::fs::rename(&tmp_record_path, &record_path)?;

    // The deployment is in place; now the game's plugin list has to name its plugins.
    let plugins = activate_plugins(
        ctx,
        instance_id,
        definition,
        &manifest,
        &base_manifest,
        &current_plan,
        links_view(mode, &current_plan),
    )?;

    Ok((
        DeployOutcome::Built {
            linked: linked_count,
            copied: copied_count,
            copied_bytes,
            config_copied: config_copied_count,
            kept: 0,
            removed: 0,
            harvest,
            plugins,
        },
        current_plan,
    ))
}

/// The plan of this deployment when it is linked, which is the Links view the load order reads.
/// `None` for another mode: the caller plans the Links view itself.
fn links_view(mode: DeployMode, plan: &DeploymentPlan) -> Option<&DeploymentPlan> {
    (mode == DeployMode::Links).then_some(plan)
}

/// The most threads that place a deployment's files at once (MASTER_SPEC §26.5). Linking and copying
/// are disk-bound, so past a few threads more do not help.
const PLACEMENT_THREADS_MAX: usize = 8;

/// The planned files' folders, each created once, parents before children. This sequential pass
/// comes before the parallel placement, so no two threads create the same folder.
fn create_planned_dirs(staging: &Path, files: &[PlannedFile]) -> Result<(), DeployError> {
    let dirs: BTreeSet<PathBuf> = files
        .iter()
        .filter_map(|file| {
            staging
                .join(file.path.as_str())
                .parent()
                .map(Path::to_path_buf)
        })
        .collect();
    for dir in &dirs {
        std::fs::create_dir_all(dir)?;
    }
    Ok(())
}

/// One planned file as it was placed.
#[derive(Debug)]
struct PlacedFile {
    record: DeployedFileRecord,
    /// True when it was placed as a hard link; false when it was copied.
    linked: bool,
    /// The planned size, which is what a copy adds to the deployment's copied bytes.
    size: u64,
}

fn placement_threads(files: usize) -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(PLACEMENT_THREADS_MAX)
        .min(files)
        .max(1)
}

/// Place one file: a hard link when the plan says so and the link can be made, else a copy. The
/// record gets the placed file's size and time from the file itself.
fn place_one(source: &Path, dest: &Path, file: &PlannedFile) -> Result<PlacedFile, DeployError> {
    let (linked, placement) = match file.placement {
        Placement::Link => match std::fs::hard_link(source, dest) {
            Ok(()) => (true, Placement::Link),
            // A link that cannot be made is a copy, and the record says so.
            Err(_) => {
                copy_owned(source, dest)?;
                (false, Placement::Copy)
            }
        },
        Placement::Copy => {
            copy_owned(source, dest)?;
            (false, Placement::Copy)
        }
    };
    let meta = std::fs::metadata(dest)?;
    Ok(PlacedFile {
        record: DeployedFileRecord {
            path: file.path.clone(),
            source: file.source.clone(),
            placement,
            size: meta.len(),
            modified_unix_ms: get_modified_unix_ms(&meta),
        },
        linked,
        size: file.size,
    })
}

/// Run `job(0)` to `job(count - 1)` on `threads` threads, and return the results in index order.
/// Indices are handed out in order, and a failure stops new work from starting; the jobs already
/// started finish. Every index before a failed one has been handed out, so the first error in index
/// order is the error a sequential run would have stopped at.
fn run_in_pool<T, F>(count: usize, threads: usize, job: F) -> Result<Vec<T>, DeployError>
where
    T: Send,
    F: Fn(usize) -> Result<T, DeployError> + Sync,
{
    let next = AtomicUsize::new(0);
    let failed = AtomicBool::new(false);
    let finished: Vec<Vec<(usize, Result<T, DeployError>)>> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads.max(1))
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        if failed.load(Ordering::Acquire) {
                            break;
                        }
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        if index >= count {
                            break;
                        }
                        let result = job(index);
                        if result.is_err() {
                            failed.store(true, Ordering::Release);
                        }
                        done.push((index, result));
                    }
                    done
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| match worker.join() {
                Ok(done) => done,
                Err(panic) => std::panic::resume_unwind(panic),
            })
            .collect()
    });
    let mut results: Vec<(usize, Result<T, DeployError>)> =
        finished.into_iter().flatten().collect();
    results.sort_by_key(|(index, _)| *index);
    results.into_iter().map(|(_, result)| result).collect()
}

/// The size of every entry under `root`, keyed by its `/`-separated path relative to `root`, from
/// one listing: a directory's listing carries each entry's size, so no file is opened. Reparse points
/// are left out, and a folder that cannot be listed is skipped. A path missing from the map must be
/// looked up on its own, so an answer never depends on what the listing left out.
fn farm_sizes(root: &Path) -> std::collections::HashMap<String, u64> {
    let mut sizes = std::collections::HashMap::new();
    let mut folders = vec![(root.to_path_buf(), String::new())];
    while let Some((folder, rel)) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if is_reparse_point_or_symlink(&meta) {
                continue;
            }
            let key = if rel.is_empty() {
                name
            } else {
                format!("{rel}/{name}")
            };
            if meta.is_dir() {
                folders.push((entry.path(), key.clone()));
            }
            sizes.insert(key, meta.len());
        }
    }
    sizes
}

/// Whether the game folder still holds every file the record names, at the size it was deployed
/// with. A file the game is expected to rewrite (a declared write, a small config copy, or a file of
/// an own-copy layer) is not checked. A file the listing does not have is looked up on its own, so
/// the answer is the one a per-file lookup gives.
fn farm_matches_record(
    game_dir: &Path,
    record: &DeploymentRecord,
    rules: &PathRules,
    own_copy_items: &std::collections::HashSet<&str>,
) -> bool {
    let sizes = farm_sizes(game_dir);
    record.files.iter().all(|f| {
        let path = f.path.as_str();
        let is_own_copy = matches!(
            &f.source,
            FileSource::Content { item_id, .. } if own_copy_items.contains(item_id.as_str())
        );
        if f.placement == Placement::Copy
            && (rules.is_declared_write(path)
                || (f.size <= 1024 * 1024 && rules.is_copy_pattern(path))
                || is_own_copy)
        {
            return true;
        }
        let len = match sizes.get(path) {
            Some(&len) => Ok(len),
            None => std::fs::metadata(game_dir.join(path)).map(|m| m.len()),
        };
        matches!(len, Ok(len) if len == f.size)
    })
}

/// The game definition's path patterns, compiled once per plan or check. Gives the answers of
/// [`GameDefinition::is_copy_pattern`] and [`GameDefinition::is_declared_write`], which parse the
/// patterns again on every call.
struct PathRules {
    copy: Vec<CompiledGlob>,
    declared: Vec<CompiledGlob>,
}

impl PathRules {
    fn new(definition: &GameDefinition) -> Self {
        let copy = DEFAULT_COPY_PATTERNS
            .iter()
            .copied()
            .chain(definition.copy_patterns.iter().map(String::as_str))
            .map(|pattern| CompiledGlob::new(&pattern.to_ascii_lowercase()))
            .collect();
        let declared = definition
            .declared_writes
            .iter()
            .map(|pattern| CompiledGlob::new(&pattern.to_ascii_lowercase()))
            .collect();
        Self { copy, declared }
    }

    /// The path as the definition's matchers see it: a valid relative path, lower-cased.
    fn normalized(path: &str) -> Option<String> {
        RelPath::new(path)
            .ok()
            .map(|rel| rel.as_str().to_ascii_lowercase())
    }

    fn is_copy_pattern(&self, path: &str) -> bool {
        Self::normalized(path)
            .is_some_and(|normalized| self.copy.iter().any(|glob| glob.matches(&normalized)))
    }

    fn is_declared_write(&self, path: &str) -> bool {
        Self::normalized(path)
            .is_some_and(|normalized| self.declared.iter().any(|glob| glob.matches(&normalized)))
    }
}

/// Name the plugins the content layers deploy in the instance's plugin list (when the game
/// declares one), before the list is swapped in for a launch. Runs under the instance lock.
fn activate_plugins(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    manifest: &crate::game_instance::GameInstanceManifest,
    base_manifest: &BaseManifest,
    plan: &DeploymentPlan,
    links: Option<&DeploymentPlan>,
) -> Result<Option<crate::game_plugins::PluginSyncReport>, DeployError> {
    let Some(rule) = &definition.plugin_list else {
        return Ok(None);
    };
    let content_layer_items: Vec<String> = manifest
        .layers
        .layers()
        .iter()
        .filter_map(|l| match &l.source {
            LayerSource::Content { content } => Some(content.clone()),
            _ => None,
        })
        .collect();
    let desired = crate::game_plugins::desired_plugins(
        rule,
        &content_layer_items,
        plan,
        base_manifest.files.iter().map(|f| f.path.as_str()),
    );
    Ok(crate::game_plugins::sync_locked_with_view(
        ctx,
        instance_id,
        definition,
        &base_manifest.runtime.store,
        &desired,
        links,
    )?)
}

pub fn undeploy(ctx: &Ctx, instance_id: &str) -> Result<HarvestReport, DeployError> {
    let _lock = ctx
        .lock_manager
        .acquire(LockResource::Instance(instance_id.to_string()), "undeploy")?;

    let manifest = crate::game_instance::get_manifest(ctx, instance_id).map_err(|e| match e {
        crate::game_instance::InstanceError::NotFound(id) => DeployError::InstanceNotFound(id),
        other => DeployError::Other(other.to_string()),
    })?;

    let base_id = match &manifest.base {
        BaseReference::Pinned { id, .. } => id,
        BaseReference::Unpinned { .. } => return Ok(HarvestReport::default()),
    };

    let base_manifest_path = ctx.paths.base_manifest_path(base_id);
    if !base_manifest_path.exists() {
        return Ok(HarvestReport::default());
    }
    let base_content = std::fs::read_to_string(&base_manifest_path)
        .map_err(|e| DeployError::UnreadableBaseManifest(e.to_string()))?;
    let base_manifest: BaseManifest = serde_json::from_str(&base_content)
        .map_err(|e| DeployError::UnreadableBaseManifest(e.to_string()))?;

    let Some(bases_root) = base_manifest.location.parent() else {
        return Ok(HarvestReport::default());
    };
    let deployment_dir = bases_root.join("deployments").join(instance_id);
    let game_dir = deployment_dir.join("game");
    let record_path = deployment_dir.join("deployment.json");

    if game_dir.exists() {
        let running = crate::game_launch::processes_running_from(&game_dir);
        if !running.is_empty() {
            let pids: Vec<String> = running.iter().map(|p| p.pid.to_string()).collect();
            return Err(DeployError::ProcessesRunning(pids.join(", ")));
        }
    }

    if record_path.exists() {
        harvest_internal(ctx, instance_id, &deployment_dir, &manifest)
    } else {
        Ok(HarvestReport::default())
    }
}

/// A file in a deployed game folder, as it is on disk now.
pub(crate) struct FarmFile {
    pub rel_path: RelPath,
    pub size: u64,
    pub modified_unix_ms: i64,
    pub abs_path: PathBuf,
}

/// One difference between a deployed game folder and its record (MASTER_SPEC §26.9).
pub(crate) enum FarmChange<'a> {
    /// The record names the file, and the comparison says it changed.
    Changed {
        recorded: &'a DeployedFileRecord,
        file: FarmFile,
    },
    /// The folder holds a file the record does not name.
    Added(FarmFile),
}

/// How a deployed game folder differs from its record. Harvest and a tool run that captures from
/// links both compare through [`compare_farm`], so they cannot disagree about what changed.
pub(crate) struct FarmDiff<'a> {
    /// Changed and added files, in lower-cased path order.
    pub changes: Vec<FarmChange<'a>>,
    /// Recorded files the folder no longer has, in the record's order.
    pub missing: Vec<&'a DeployedFileRecord>,
}

/// Compare the game folder under `game_dir` with `record`. `differs` says whether a file the record
/// names has changed: harvest compares size and time, a tool run that captures from links compares
/// bytes where the file was copied.
pub(crate) fn compare_farm<'a>(
    game_dir: &Path,
    record: &'a DeploymentRecord,
    differs: &mut dyn FnMut(&FarmFile, &DeployedFileRecord) -> Result<bool, DeployError>,
) -> Result<FarmDiff<'a>, DeployError> {
    let mut current: BTreeMap<String, FarmFile> = BTreeMap::new();
    walk_farm(game_dir, Path::new(""), &mut current)?;

    let mut record_map: BTreeMap<String, &'a DeployedFileRecord> = BTreeMap::new();
    for f in &record.files {
        record_map.insert(f.path.as_str().to_ascii_lowercase(), f);
    }

    let missing = record
        .files
        .iter()
        .filter(|f| !current.contains_key(&f.path.as_str().to_ascii_lowercase()))
        .collect();

    let mut changes = Vec::new();
    for (lower_path, file) in current {
        match record_map.get(&lower_path) {
            Some(recorded) => {
                if differs(&file, recorded)? {
                    changes.push(FarmChange::Changed { recorded, file });
                }
            }
            None => changes.push(FarmChange::Added(file)),
        }
    }
    Ok(FarmDiff { changes, missing })
}

/// Every file under `root`, keyed by lower-cased path. Links and other reparse points are skipped.
fn walk_farm(
    root: &Path,
    rel: &Path,
    out: &mut BTreeMap<String, FarmFile>,
) -> Result<(), DeployError> {
    let cur = if rel.as_os_str().is_empty() {
        root.to_path_buf()
    } else {
        root.join(rel)
    };
    if !cur.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(&cur)? {
        let entry = entry?;
        let abs_path = entry.path();
        let meta = entry.metadata()?;
        if is_reparse_point_or_symlink(&meta) {
            continue;
        }
        let child_rel = rel.join(entry.file_name());
        if meta.is_dir() {
            walk_farm(root, &child_rel, out)?;
        } else if meta.is_file() {
            let rel_str = child_rel.to_string_lossy().replace('\\', "/");
            let rel_path = RelPath::new(rel_str)
                .map_err(|e| DeployError::Other(format!("invalid game file path: {e}")))?;
            let mtime = get_modified_unix_ms(&meta);
            out.insert(
                rel_path.as_str().to_ascii_lowercase(),
                FarmFile {
                    rel_path,
                    size: meta.len(),
                    modified_unix_ms: mtime,
                    abs_path,
                },
            );
        }
    }
    Ok(())
}

/// The record of what a deployment put in its game folder.
pub(crate) fn read_deployment_record(
    deployment_dir: &Path,
) -> Result<DeploymentRecord, DeployError> {
    let record_content = std::fs::read_to_string(deployment_dir.join("deployment.json"))?;
    serde_json::from_str(&record_content)
        .map_err(|e| DeployError::CorruptDeployment(format!("unreadable deployment.json: {e}")))
}

/// Take a deployment away without harvesting it: its game folder and record go, and nothing is
/// copied into the writable layer. For a tool run that has captured its writes already, whose
/// farm must not become the game's own (MASTER_SPEC §26.9).
pub(crate) fn discard_deployment(
    deployment_dir: &Path,
    instance_id: &str,
) -> Result<(), DeployError> {
    validate_deployment_safety(deployment_dir, instance_id)?;
    let game_dir = deployment_dir.join("game");
    if game_dir.exists() {
        std::fs::remove_dir_all(&game_dir)?;
    }
    let record_path = deployment_dir.join("deployment.json");
    if record_path.exists() {
        std::fs::remove_file(&record_path)?;
    }
    Ok(())
}

/// Copy a file the game may then write. `fs::copy` carries the source's permissions, and on Unix
/// a content store object is read-only; on Windows its protection is an ACL a copy does not take.
fn copy_owned(source: &Path, dest: &Path) -> std::io::Result<u64> {
    let bytes = std::fs::copy(source, dest)?;
    #[cfg(unix)]
    crate::content_store::protect::unprotect(dest)?;
    Ok(bytes)
}

/// Copy a changed or new game file into the writable layer, and clear any deletion marker for it.
fn copy_to_writable(
    writable_dir: &Path,
    file: &FarmFile,
    report: &mut HarvestReport,
) -> Result<(), DeployError> {
    let dest = writable_dir.join(file.rel_path.as_str());
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    copy_owned(&file.abs_path, &dest)?;
    clear_whiteout_marker(writable_dir, &file.rel_path);
    report.copied_to_writable.push(file.rel_path.clone());
    Ok(())
}

/// What harvesting a farm found (see [`harvest_changes`]).
struct HarvestedFarm {
    report: HarvestReport,
    /// The record the farm was compared with.
    record: DeploymentRecord,
    /// Lower-cased paths of the recorded files that are changed or missing on disk, whether or not
    /// the harvest copied or whited them out.
    differing: HashSet<String>,
    /// The files the farm has that the record does not name. The harvest copied each one to the
    /// writable layer.
    unrecorded: Vec<RelPath>,
}

/// The harvest of a deployed farm: its changes are copied into the writable layer and its
/// deletions whited out, exactly as a full rebuild always did. The farm and its record are left
/// where they are, for the caller to delete or update in place (MASTER_SPEC §26.5).
fn harvest_changes(
    ctx: &Ctx,
    instance_id: &str,
    deployment_dir: &Path,
    manifest: &crate::game_instance::GameInstanceManifest,
) -> Result<HarvestedFarm, DeployError> {
    let game_dir = deployment_dir.join("game");

    // Guard safety checks
    validate_deployment_safety(deployment_dir, instance_id)?;
    let record = read_deployment_record(deployment_dir)?;

    let instance_dir = ctx
        .paths
        .instance_dir(instance_id)
        .map_err(|e| DeployError::Other(e.to_string()))?;
    let writable_dir = instance_dir.join("writable");

    let mut report = HarvestReport::default();
    let mut new_whiteouts = Vec::new();
    let mut differing = HashSet::new();
    let mut unrecorded = Vec::new();

    // A missing game folder is a deployment to build again, not a user who deleted every file:
    // reading it as deletions would white out the whole instance and delete its writable files.
    if !game_dir.is_dir() {
        differing = record
            .files
            .iter()
            .map(|f| f.path.as_str().to_ascii_lowercase())
            .collect();
        return Ok(HarvestedFarm {
            report,
            record,
            differing,
            unrecorded,
        });
    }

    // 1. Files the game folder changed or added: copied to the writable layer. A recorded file is
    // changed when its size or time differs; a changed base file is also named in the report.
    let diff = compare_farm(&game_dir, &record, &mut |file, recorded| {
        Ok(file.size != recorded.size || file.modified_unix_ms != recorded.modified_unix_ms)
    })?;
    for change in diff.changes {
        match change {
            FarmChange::Changed { recorded, file } => {
                differing.insert(path_key(recorded.path.as_str()));
                match recorded.placement {
                    Placement::Copy => copy_to_writable(&writable_dir, &file, &mut report)?,
                    Placement::Link => {
                        if let FileSource::Base {
                            linked_to_store, ..
                        } = recorded.source
                        {
                            copy_to_writable(&writable_dir, &file, &mut report)?;
                            report.base_files_changed.push(BaseFileChanged {
                                path: file.rel_path.clone(),
                                linked_to_store,
                            });
                        }
                    }
                }
            }
            FarmChange::Added(file) => {
                unrecorded.push(file.rel_path.clone());
                copy_to_writable(&writable_dir, &file, &mut report)?;
            }
        }
    }

    // 2. Recorded files the game folder no longer has
    for rec_file in diff.missing {
        differing.insert(path_key(rec_file.path.as_str()));
        match rec_file.source {
            FileSource::Writable { .. } => {
                let dest = writable_dir.join(rec_file.path.as_str());
                if dest.exists() {
                    let _ = std::fs::remove_file(&dest);
                }
                report.writable_files_removed.push(rec_file.path.clone());
            }
            _ => {
                new_whiteouts.push(rec_file.path.clone());
                report.whiteouts_added.push(rec_file.path.clone());
            }
        }
    }

    // 3. Update writable layer in instance manifest if needed
    let has_writable_layer = manifest
        .layers
        .layers()
        .iter()
        .any(|l| matches!(l.source, LayerSource::Writable { .. }));

    if !report.copied_to_writable.is_empty() || !new_whiteouts.is_empty() || has_writable_layer {
        let mut layers_vec = manifest.layers.layers().to_vec();
        let existing_writable_idx = layers_vec
            .iter()
            .position(|l| matches!(l.source, LayerSource::Writable { .. }));

        if let Some(idx) = existing_writable_idx {
            let mut current_whiteouts = layers_vec[idx].whiteouts.clone();
            for nw in new_whiteouts {
                let nw_lower = nw.as_str().to_ascii_lowercase();
                if !current_whiteouts
                    .iter()
                    .any(|w| w.as_str().to_ascii_lowercase() == nw_lower)
                {
                    current_whiteouts.push(nw);
                }
            }
            // If any copied_to_writable path was previously whited out, un-whiteout it
            for cw in &report.copied_to_writable {
                let cw_lower = cw.as_str().to_ascii_lowercase();
                current_whiteouts.retain(|w| w.as_str().to_ascii_lowercase() != cw_lower);
            }
            layers_vec[idx].whiteouts = current_whiteouts;
        } else {
            let writable_layer = Layer {
                id: LayerId::new("writable").map_err(DeployError::Id)?,
                enabled: true,
                mount_path: RelPath::default(),
                source_path: RelPath::default(),
                source: LayerSource::Writable {
                    path: RelPath::new("writable")
                        .map_err(|e| DeployError::Other(format!("invalid rel path: {e}")))?,
                },
                whiteouts: new_whiteouts,
                own_copy: false,
            };
            let insert_pos = layers_vec
                .iter()
                .position(|l| l.source.rank() > 4)
                .unwrap_or(layers_vec.len());
            layers_vec.insert(insert_pos, writable_layer);
        }

        let mut updated_manifest = manifest.clone();
        updated_manifest.layers = LayerStack::new(layers_vec)?;
        write_instance_manifest_atomic(ctx, instance_id, &updated_manifest)?;
    }

    Ok(HarvestedFarm {
        report,
        record,
        differing,
        unrecorded,
    })
}

/// The whole harvest: [`harvest_changes`], then the farm and its record are deleted.
fn harvest_internal(
    ctx: &Ctx,
    instance_id: &str,
    deployment_dir: &Path,
    manifest: &crate::game_instance::GameInstanceManifest,
) -> Result<HarvestReport, DeployError> {
    let harvested = harvest_changes(ctx, instance_id, deployment_dir, manifest)?;
    remove_farm_and_record(deployment_dir)?;
    Ok(harvested.report)
}

/// Delete a deployment's game folder and its record.
fn remove_farm_and_record(deployment_dir: &Path) -> Result<(), DeployError> {
    let game_dir = deployment_dir.join("game");
    let record_path = deployment_dir.join("deployment.json");
    if game_dir.exists() {
        std::fs::remove_dir_all(&game_dir)?;
    }
    if record_path.exists() {
        std::fs::remove_file(&record_path)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// In-place updates (MASTER_SPEC §26.5)
// ---------------------------------------------------------------------------
//
// A deployment whose plan changed is updated in place where it can be: the recorded files the
// new plan still has, unchanged, stay; the others are removed and placed again. Before the first
// change to the farm the record is renamed to `deployment.json.updating`, and the new record is
// written, and the marker removed, only after the last one. A marker left behind (a crash, or an
// error part way) means the farm is not trusted: the next deploy discards it and builds it again.

/// The record, set aside while its farm is updated in place.
const UPDATING_FILE: &str = "deployment.json.updating";

/// The updating marker of a deployment folder.
fn updating_marker(deployment_dir: &Path) -> PathBuf {
    deployment_dir.join(UPDATING_FILE)
}

/// A path as the record and the plan compare it: lower-cased, as the farm's own paths are keyed.
fn path_key(path: &str) -> String {
    path.to_ascii_lowercase()
}

thread_local! {
    /// The test seam: how many more farm changes an in-place update on this thread may make before
    /// it stops. `None` (the default) is no stop.
    static UPDATE_FAULT_AFTER: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    /// Set when the seam stopped an update, so the update leaves its marker as a crash would.
    static SIMULATED_STOP: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Test seam for crash safety: the next in-place update on this thread stops once it has made
/// `changes` changes to the farm, and leaves its marker behind, as a process stop would. `None`
/// turns the seam off.
#[doc(hidden)]
pub fn set_update_fault_after(changes: Option<usize>) {
    UPDATE_FAULT_AFTER.with(|cell| cell.set(changes));
}

/// Stop the in-place update here if the seam asks for it.
fn update_fault_point() -> Result<(), DeployError> {
    UPDATE_FAULT_AFTER.with(|cell| match cell.get() {
        Some(0) => {
            cell.set(None);
            SIMULATED_STOP.with(|stopped| stopped.set(true));
            Err(DeployError::Other(
                "injected stop during an in-place update".into(),
            ))
        }
        Some(left) => {
            cell.set(Some(left - 1));
            Ok(())
        }
        None => Ok(()),
    })
}

/// Whether the seam stopped the update that just failed. Clears the flag.
fn take_simulated_stop() -> bool {
    SIMULATED_STOP.with(|stopped| stopped.replace(false))
}

/// The changes an in-place update makes to a farm: the recorded files it keeps (by path key), the
/// farm paths it removes, and the planned files it places.
type InPlaceChanges = (
    HashMap<String, DeployedFileRecord>,
    Vec<RelPath>,
    Vec<PlannedFile>,
);

/// What an in-place update is to do, worked out before the farm is changed.
struct InPlaceUpdate {
    instance_id: String,
    base_id: String,
    mode: DeployMode,
    plan: DeploymentPlan,
    harvest: HarvestReport,
    kept: HashMap<String, DeployedFileRecord>,
    remove: Vec<RelPath>,
    place: Vec<PlannedFile>,
}

/// What an in-place update did.
struct InPlaceApplied {
    plan: DeploymentPlan,
    harvest: HarvestReport,
    linked: usize,
    copied: usize,
    copied_bytes: u64,
    config_copied: usize,
    kept: usize,
    removed: usize,
}

/// Whether a record's path is a plain relative path: no empty, `.` or `..` parts, no root or drive.
fn is_plain_path(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

/// Whether a folder of a farm is a folder, and not a link to one: the folder itself, or nothing
/// there yet. Removing or placing through a link would act outside the farm.
fn folder_is_plain(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => meta.is_dir() && !is_reparse_point_or_symlink(&meta),
        Err(e) => e.kind() == std::io::ErrorKind::NotFound,
    }
}

/// Whether every folder on the way to `rel` (the folders above the file itself) is plain. `cache`
/// remembers each folder's answer, since a plan shares most of its folders.
fn folders_on_path_are_plain(
    game_dir: &Path,
    rel: &str,
    cache: &mut HashMap<String, bool>,
) -> bool {
    let parts: Vec<&str> = rel.split('/').collect();
    let mut folder = String::new();
    for part in parts.iter().take(parts.len().saturating_sub(1)) {
        if !folder.is_empty() {
            folder.push('/');
        }
        folder.push_str(part);
        let plain = *cache
            .entry(folder.clone())
            .or_insert_with(|| folder_is_plain(&game_dir.join(&folder)));
        if !plain {
            return false;
        }
    }
    true
}

/// Work out the in-place update of a farm from its harvest and the plan it must now match. `None`
/// when the farm must be built again instead: another base, a record that does not name plain
/// paths, a missing game folder, or a folder on the way to a changed file that is not a folder.
fn plan_in_place(
    ctx: &Ctx,
    game_dir: &Path,
    harvested: &HarvestedFarm,
    plan: &DeploymentPlan,
    base_id: &str,
) -> Option<InPlaceChanges> {
    if harvested.record.base_id != base_id || !game_dir.is_dir() {
        return None;
    }
    let mut recorded: HashMap<String, &DeployedFileRecord> = HashMap::new();
    for rec in &harvested.record.files {
        let path = rec.path.as_str();
        if !is_plain_path(path) || recorded.insert(path_key(path), rec).is_some() {
            return None;
        }
    }
    let mut planned: HashMap<String, &PlannedFile> = HashMap::new();
    for file in &plan.files {
        if planned.insert(path_key(file.path.as_str()), file).is_some() {
            return None;
        }
    }

    // A recorded file is kept when the plan has it the same way (source, placement and size), and
    // the harvest found it as the record has it. The writable layer is always placed again: the game
    // and the instance's own tools both write there, and the record cannot tell what changed.
    let mut kept = HashMap::new();
    let mut remove = Vec::new();
    let mut link_limit = HashMap::new();
    for (key, &rec) in &recorded {
        let same_in_plan = planned.get(key).is_some_and(|file| {
            file.path == rec.path
                && file.source == rec.source
                && (file.placement == rec.placement
                    || (file.placement == Placement::Link
                        && rec.placement == Placement::Copy
                        && at_link_limit(&mut link_limit, ctx, &file.source)))
                && file.size == rec.size
                && !matches!(file.source, FileSource::Writable { .. })
        });
        if same_in_plan && !harvested.differing.contains(key) {
            kept.insert(key.clone(), rec.clone());
        } else {
            remove.push(rec.path.clone());
        }
    }
    // A file the farm has that the record does not name was harvested just now; it leaves the farm.
    remove.extend(harvested.unrecorded.iter().cloned());
    remove.sort();

    let place: Vec<PlannedFile> = plan
        .files
        .iter()
        .filter(|file| !kept.contains_key(&path_key(file.path.as_str())))
        .cloned()
        .collect();

    // Removing a folder, or writing into one, must not go through a link out of the farm.
    for rel in &remove {
        let path = game_dir.join(rel.as_str());
        if std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_dir()) {
            return None;
        }
    }
    let mut folders = HashMap::new();
    let all_plain = remove
        .iter()
        .map(RelPath::as_str)
        .chain(place.iter().map(|file| file.path.as_str()))
        .all(|rel| folders_on_path_are_plain(game_dir, rel, &mut folders));
    if !all_plain {
        return None;
    }
    Some((kept, remove, place))
}

/// Whether the file a planned link points at already has as many links as the file system allows.
/// Then a deployment holds a copy of the same bytes where it planned a link, and an update in place
/// keeps that copy: NTFS allows 1,023 links to one file, and identical bytes share one content object,
/// so a mod with thousands of identical files (terrain `.btt` stubs) gets copies past the limit. On
/// the user's instance 15,533 such copies were placed again on every deploy. A copy kept for any other
/// reason, such as an own-copy setting since turned off, is replaced by the link a rebuild would make.
/// Answered once per source.
fn at_link_limit(cache: &mut HashMap<String, bool>, ctx: &Ctx, source: &FileSource) -> bool {
    const NTFS_LINK_LIMIT: u32 = 1023;
    let path = source_path(ctx, source);
    let key = path.to_string_lossy().into_owned();
    *cache
        .entry(key)
        .or_insert_with(|| link_count(&path).is_some_and(|n| n >= NTFS_LINK_LIMIT))
}

#[cfg(windows)]
fn link_count(path: &Path) -> Option<u32> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    let file = std::fs::File::open(path).ok()?;
    // SAFETY: an all-zero BY_HANDLE_FILE_INFORMATION is valid, and the handle is open for the call.
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) };
    (ok != 0).then_some(info.nNumberOfLinks)
}

#[cfg(unix)]
fn link_count(path: &Path) -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path)
        .ok()
        .map(|m| u32::try_from(m.nlink()).unwrap_or(u32::MAX))
}

/// Remove one file of a farm. A file that is already gone is not an error: the record may name a
/// file the game deleted, which the harvest has whited out.
///
/// A link to a protected content object can be deleted only through its folder's
/// `FILE_DELETE_CHILD` grant, which a farm's root receives when it is built and its folders inherit.
/// Granting it again on every update would cost a walk of the whole farm (Windows propagates the
/// inheritable entry to every file: ~24 s for 198k files), so it is granted only when a removal is
/// refused, and the removal is tried once more.
fn remove_farm_file(game_dir: &Path, rel: &RelPath) -> Result<(), DeployError> {
    let path = game_dir.join(rel.as_str());
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            protect::grant_delete_child(game_dir)?;
            match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e.into()),
            }
        }
        Err(e) => Err(e.into()),
    }
}

/// Remove the folders that removing `removed` left empty, from the file's folder upward. A folder
/// that still holds something stops the climb, and so does the farm's root.
fn remove_emptied_folders(game_dir: &Path, removed: &[RelPath]) {
    for rel in removed {
        let mut folder = Path::new(rel.as_str()).parent();
        while let Some(rel_folder) = folder {
            if rel_folder.as_os_str().is_empty()
                || std::fs::remove_dir(game_dir.join(rel_folder)).is_err()
            {
                break;
            }
            folder = rel_folder.parent();
        }
    }
}

/// Carry out an in-place update: remove, place, then write the new record and remove the marker.
/// Runs under the instance lock, with the harvest already done. An error part way takes the
/// half-updated farm away, as a failed full build always left no farm behind; a stop (a crash, or
/// the test seam's) leaves the marker for the next deploy to discard.
fn apply_in_place(
    ctx: &Ctx,
    deployment_dir: &Path,
    update: InPlaceUpdate,
) -> Result<InPlaceApplied, DeployError> {
    let record_path = deployment_dir.join("deployment.json");
    let marker = updating_marker(deployment_dir);

    // The record is set aside before the first change. From here, a stop leaves the marker behind.
    std::fs::rename(&record_path, &marker)?;

    match apply_marked(ctx, deployment_dir, update) {
        Ok(applied) => Ok(applied),
        Err(err) => {
            if !take_simulated_stop() {
                // Best effort: if this fails the marker is still there, and the next deploy discards it.
                let _ = discard_untrusted_farm(deployment_dir);
            }
            Err(err)
        }
    }
}

/// The changes of an in-place update, once the record is set aside: remove, place, and write the
/// new record and remove the marker.
fn apply_marked(
    ctx: &Ctx,
    deployment_dir: &Path,
    update: InPlaceUpdate,
) -> Result<InPlaceApplied, DeployError> {
    let game_dir = deployment_dir.join("game");
    let record_path = deployment_dir.join("deployment.json");
    let marker = updating_marker(deployment_dir);
    let InPlaceUpdate {
        instance_id,
        base_id,
        mode,
        plan,
        harvest,
        mut kept,
        remove,
        place,
    } = update;

    for rel in &remove {
        update_fault_point()?;
        remove_farm_file(&game_dir, rel)?;
    }
    update_fault_point()?;
    let placed = place_files(ctx, &place, &game_dir)?;
    remove_emptied_folders(&game_dir, &remove);

    let kept_count = kept.len();
    let PlacedFiles {
        linked,
        copied,
        copied_bytes,
        config_copied,
        recorded,
    } = placed;
    let mut placed_by_key: HashMap<String, DeployedFileRecord> = recorded
        .into_iter()
        .map(|rec| (path_key(rec.path.as_str()), rec))
        .collect();
    // The new record lists the files in plan order, as a full build does.
    let mut files = Vec::with_capacity(plan.files.len());
    for file in &plan.files {
        let key = path_key(file.path.as_str());
        let rec = kept
            .remove(&key)
            .or_else(|| placed_by_key.remove(&key))
            .ok_or_else(|| {
                DeployError::Other(format!(
                    "in-place update left '{}' without a record",
                    file.path
                ))
            })?;
        files.push(rec);
    }

    let record = DeploymentRecord {
        instance_id,
        base_id,
        mode,
        fingerprint: plan.fingerprint.clone(),
        files,
        deployed_at_unix_ms: now_unix_ms(),
    };
    let unique = format!("{}-{}", std::process::id(), uuid::Uuid::new_v4().simple());
    let tmp_record_path = deployment_dir.join(format!("deployment.json.tmp-{unique}"));
    std::fs::write(&tmp_record_path, serde_json::to_vec(&record)?)?;
    std::fs::rename(&tmp_record_path, &record_path)?;
    std::fs::remove_file(&marker)?;

    Ok(InPlaceApplied {
        plan,
        harvest,
        linked,
        copied,
        copied_bytes,
        config_copied,
        kept: kept_count,
        removed: remove.len(),
    })
}

/// Discard a farm whose update stopped partway. Nothing is harvested from it: its state is not
/// known. The marker goes last, so a stop during the discard leaves the next deploy to discard again.
fn discard_untrusted_farm(deployment_dir: &Path) -> Result<(), DeployError> {
    let record_path = deployment_dir.join("deployment.json");
    if record_path.exists() {
        std::fs::remove_file(&record_path)?;
    }
    let game_dir = deployment_dir.join("game");
    if game_dir.exists() {
        std::fs::remove_dir_all(&game_dir)?;
    }
    let marker = updating_marker(deployment_dir);
    if marker.exists() {
        std::fs::remove_file(&marker)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Content Layer Management
// ---------------------------------------------------------------------------

pub fn add_content(
    ctx: &Ctx,
    instance_id: &str,
    item_id_or_prefix: &str,
    mount_path: Option<&str>,
    source_path: Option<&str>,
) -> Result<Layer, DeployError> {
    let _lock = ctx.lock_manager.acquire(
        LockResource::Instance(instance_id.to_string()),
        "content-add",
    )?;

    let item_id = crate::content_store::resolve_item_id(ctx, item_id_or_prefix)?;
    let _item = crate::content_store::get_item(ctx, &item_id)?;

    let mut manifest =
        crate::game_instance::get_manifest(ctx, instance_id).map_err(|e| match e {
            crate::game_instance::InstanceError::NotFound(id) => DeployError::InstanceNotFound(id),
            other => DeployError::Other(other.to_string()),
        })?;

    for l in manifest.layers.layers() {
        if let LayerSource::Content { content } = &l.source {
            if content == &item_id {
                return Err(DeployError::ContentAlreadyPresent(item_id));
            }
        }
    }

    let prefix_16 = &item_id[..item_id.len().min(16)];
    let layer_id = LayerId::new(format!("content:{prefix_16}")).map_err(DeployError::Id)?;

    let mount_rel = match mount_path {
        Some(p) if !p.trim().is_empty() => {
            RelPath::new(p).map_err(|e| DeployError::Other(format!("invalid mount path: {e}")))?
        }
        _ => RelPath::default(),
    };

    let source_rel = match source_path {
        Some(p) if !p.trim().is_empty() => {
            RelPath::new(p).map_err(|e| DeployError::Other(format!("invalid source path: {e}")))?
        }
        _ => RelPath::default(),
    };

    let new_layer = Layer {
        id: layer_id,
        enabled: true,
        mount_path: mount_rel,
        source_path: source_rel,
        source: LayerSource::Content { content: item_id },
        whiteouts: Vec::new(),
        own_copy: false,
    };

    let mut layers = manifest.layers.layers().to_vec();
    let insert_pos = layers
        .iter()
        .position(|l| l.source.rank() > 2)
        .unwrap_or(layers.len());
    layers.insert(insert_pos, new_layer.clone());

    manifest.layers = LayerStack::new(layers)?;
    write_instance_manifest_atomic(ctx, instance_id, &manifest)?;

    Ok(new_layer)
}

/// Where a content item goes in an instance, by its game's content layout. This is the one placement
/// rule: `games instance content add` and the catalog install both apply it (MASTER_SPEC §26.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlacementDecision {
    /// Place the item: under `mount_path` (empty for the game root or the data folder) and from
    /// `source_path` inside the archive (empty for the archive's root).
    Place {
        mount_path: String,
        source_path: String,
        reason: String,
    },
    /// The archive has a FOMOD installer, so it is not placed as it stands.
    Installer {
        reason: String,
        top_level: Vec<String>,
    },
    /// No rule places these files; the caller must say where they go.
    Unknown { top_level: Vec<String> },
}

pub fn decide_placement(
    item: &crate::content_store::ContentItem,
    layout: &agora_game_api::ContentLayout,
) -> PlacementDecision {
    let files: Vec<RelPath> = item.files.iter().map(|file| file.path.clone()).collect();
    match agora_game_api::suggest_placement(&files, layout) {
        agora_game_api::Suggestion::Place {
            source_path,
            mount_path,
            reason,
        } => PlacementDecision::Place {
            mount_path: mount_path.as_str().to_string(),
            source_path: source_path.as_str().to_string(),
            reason,
        },
        agora_game_api::Suggestion::Installer { reason } => {
            let mut top_level: Vec<String> = item
                .files
                .iter()
                .filter_map(|file| file.path.as_str().split('/').next().map(str::to_string))
                .collect();
            top_level.sort();
            top_level.dedup();
            PlacementDecision::Installer { reason, top_level }
        }
        agora_game_api::Suggestion::Unknown { top_level } => {
            PlacementDecision::Unknown { top_level }
        }
    }
}

// ---------------------------------------------------------------------------
// Adding a stored item to an instance by the game's content rules
// ---------------------------------------------------------------------------

/// A Thunderstore package that was added the way the game's BepInEx layout describes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThunderstoreAdded {
    /// The package's name from its `manifest.json`.
    pub package_name: String,
    pub package_id: String,
    pub version: String,
    pub summary: String,
    /// Dependencies the package names that the instance does not have.
    pub missing_dependencies: Vec<String>,
}

/// What [`add_to_instance`] did.
#[derive(Debug, Clone)]
pub enum AddToInstance {
    /// The item is a layer of the instance now.
    Added {
        /// The item the layer holds: the one asked for, or for a Thunderstore package the item
        /// derived from it.
        item_id: String,
        layer: Layer,
        /// Why the item went where it did, when the content rules chose (not for a package).
        placement_reason: Option<String>,
        thunderstore: Option<ThunderstoreAdded>,
    },
    /// The archive has a FOMOD installer, so nothing was added: its questions come first.
    NeedsInstaller {
        item_id: String,
        reason: String,
        top_level: Vec<String>,
    },
}

/// Why [`add_to_instance`] did not add the item. The text is for every adapter; an adapter that
/// has its own words for a case matches the variant.
#[derive(Debug, thiserror::Error)]
pub enum AddToInstanceError {
    #[error(transparent)]
    Instance(#[from] crate::game_instance::InstanceError),
    #[error("the game '{0}' is not known")]
    UnknownGame(String),
    #[error("{game_name} has no content rules, so Agora cannot say where this item goes")]
    NoContentLayout { game_id: String, game_name: String },
    /// The store item, or a Thunderstore `manifest.json` in it, cannot be read. A manifest that
    /// cannot be read is an error, not "not a package".
    #[error(transparent)]
    Content(#[from] crate::content_store::ContentError),
    #[error(transparent)]
    Thunderstore(#[from] crate::content_thunderstore::ThunderstoreInstallError),
    #[error(transparent)]
    Deploy(#[from] DeployError),
    #[error("no rule says where these files go (top-level entries: {})", .top_level.join(", "))]
    PlacementUnknown { top_level: Vec<String> },
}

/// Add a stored item to an instance where the game's content rules put it: the one placement path
/// the CLI (`games instance content add`) and the desktop both call. A Thunderstore package in a
/// game with a BepInEx layout installs as that layout describes; an archive with a FOMOD installer
/// is not added (`NeedsInstaller`); anything else is placed by [`decide_placement`]. A caller that
/// names the place itself calls [`add_content`] instead.
pub fn add_to_instance(
    ctx: &Ctx,
    instance_id: &str,
    item_id: &str,
) -> Result<AddToInstance, AddToInstanceError> {
    let manifest = crate::game_instance::get_manifest(ctx, instance_id)?;
    let game = ctx
        .games
        .game(&manifest.game)
        .ok_or_else(|| AddToInstanceError::UnknownGame(manifest.game.to_string()))?;
    let layout =
        game.content_layout
            .as_ref()
            .ok_or_else(|| AddToInstanceError::NoContentLayout {
                game_id: game.id.to_string(),
                game_name: game.name.clone(),
            })?;

    if layout.thunderstore_bepinex {
        if let Some(package) = crate::content_thunderstore::parse_manifest(ctx, item_id)? {
            let outcome =
                crate::content_thunderstore::install_thunderstore(ctx, instance_id, item_id)?;
            return Ok(AddToInstance::Added {
                item_id: outcome.derived_item.item_id.clone(),
                layer: outcome.layer,
                placement_reason: None,
                thunderstore: Some(ThunderstoreAdded {
                    package_name: package.name,
                    package_id: outcome.package_id,
                    version: outcome.version,
                    summary: outcome.summary,
                    missing_dependencies: outcome.missing_dependencies,
                }),
            });
        }
    }

    let item = crate::content_store::get_item(ctx, item_id)?;
    match decide_placement(&item, layout) {
        PlacementDecision::Place {
            mount_path,
            source_path,
            reason,
        } => {
            let mount = (!mount_path.is_empty()).then_some(mount_path.as_str());
            let source = (!source_path.is_empty()).then_some(source_path.as_str());
            let layer = add_content(ctx, instance_id, item_id, mount, source)?;
            Ok(AddToInstance::Added {
                item_id: item_id.to_string(),
                layer,
                placement_reason: Some(reason),
                thunderstore: None,
            })
        }
        PlacementDecision::Installer { reason, top_level } => Ok(AddToInstance::NeedsInstaller {
            item_id: item_id.to_string(),
            reason,
            top_level,
        }),
        PlacementDecision::Unknown { top_level } => {
            Err(AddToInstanceError::PlacementUnknown { top_level })
        }
    }
}

/// Choose (or, with `None`, stop choosing) how an instance is deployed and run. A chosen mode is
/// never replaced by a fallback; `None` leaves the choice to the game definition and the machine.
pub fn set_deployment(
    ctx: &Ctx,
    instance_id: &str,
    mode: Option<DeployMode>,
) -> Result<(), DeployError> {
    let _lock = ctx.lock_manager.acquire(
        LockResource::Instance(instance_id.to_string()),
        "set-deployment",
    )?;

    let mut manifest =
        crate::game_instance::get_manifest(ctx, instance_id).map_err(|e| match e {
            crate::game_instance::InstanceError::NotFound(id) => DeployError::InstanceNotFound(id),
            other => DeployError::Other(other.to_string()),
        })?;
    manifest.deployment = mode;
    write_instance_manifest_atomic(ctx, instance_id, &manifest)
}

/// The index in `layers` of the content layer `item_id_or_prefix` names: an exact item id, else a
/// prefix exactly one content layer's id starts with. An empty prefix names nothing, and a prefix
/// two layers share is refused rather than resolved to whichever comes first.
fn resolve_content_layer(layers: &[Layer], item_id_or_prefix: &str) -> Result<usize, DeployError> {
    let content_of = |l: &Layer| match &l.source {
        LayerSource::Content { content } => Some(content.clone()),
        _ => None,
    };
    if item_id_or_prefix.is_empty() {
        return Err(DeployError::ContentNotFound(item_id_or_prefix.to_string()));
    }
    if let Some(idx) = layers
        .iter()
        .position(|l| content_of(l).as_deref() == Some(item_id_or_prefix))
    {
        return Ok(idx);
    }
    let matches: Vec<usize> = layers
        .iter()
        .enumerate()
        .filter(|(_, l)| content_of(l).is_some_and(|c| c.starts_with(item_id_or_prefix)))
        .map(|(i, _)| i)
        .collect();
    match matches.as_slice() {
        [idx] => Ok(*idx),
        [] => Err(DeployError::ContentNotFound(item_id_or_prefix.to_string())),
        _ => Err(DeployError::Other(format!(
            "'{item_id_or_prefix}' matches {} content items in this instance; give more of the id",
            matches.len()
        ))),
    }
}

pub fn remove_content(
    ctx: &Ctx,
    instance_id: &str,
    item_id_or_prefix: &str,
) -> Result<(), DeployError> {
    let _lock = ctx.lock_manager.acquire(
        LockResource::Instance(instance_id.to_string()),
        "content-remove",
    )?;

    let mut manifest =
        crate::game_instance::get_manifest(ctx, instance_id).map_err(|e| match e {
            crate::game_instance::InstanceError::NotFound(id) => DeployError::InstanceNotFound(id),
            other => DeployError::Other(other.to_string()),
        })?;

    let mut layers = manifest.layers.layers().to_vec();
    let idx = resolve_content_layer(&layers, item_id_or_prefix)?;

    layers.remove(idx);
    manifest.layers = LayerStack::new(layers)?;
    write_instance_manifest_atomic(ctx, instance_id, &manifest)?;

    Ok(())
}

pub fn set_content_enabled(
    ctx: &Ctx,
    instance_id: &str,
    item_id_or_prefix: &str,
    enabled: bool,
) -> Result<(), DeployError> {
    let _lock = ctx.lock_manager.acquire(
        LockResource::Instance(instance_id.to_string()),
        "content-enable",
    )?;

    let mut manifest =
        crate::game_instance::get_manifest(ctx, instance_id).map_err(|e| match e {
            crate::game_instance::InstanceError::NotFound(id) => DeployError::InstanceNotFound(id),
            other => DeployError::Other(other.to_string()),
        })?;

    let mut layers = manifest.layers.layers().to_vec();
    let idx = resolve_content_layer(&layers, item_id_or_prefix)?;

    layers[idx].enabled = enabled;
    manifest.layers = LayerStack::new(layers)?;
    write_instance_manifest_atomic(ctx, instance_id, &manifest)?;

    Ok(())
}

pub fn set_content_own_copy(
    ctx: &Ctx,
    instance_id: &str,
    item_id_or_prefix: &str,
    own_copy: bool,
) -> Result<(), DeployError> {
    let _lock = ctx.lock_manager.acquire(
        LockResource::Instance(instance_id.to_string()),
        "content-own-copy",
    )?;

    let mut manifest =
        crate::game_instance::get_manifest(ctx, instance_id).map_err(|e| match e {
            crate::game_instance::InstanceError::NotFound(id) => DeployError::InstanceNotFound(id),
            other => DeployError::Other(other.to_string()),
        })?;

    let mut layers = manifest.layers.layers().to_vec();
    let idx = resolve_content_layer(&layers, item_id_or_prefix)?;
    layers[idx].own_copy = own_copy;

    manifest.layers = LayerStack::new(layers)?;
    write_instance_manifest_atomic(ctx, instance_id, &manifest)?;

    Ok(())
}

pub fn move_content(
    ctx: &Ctx,
    instance_id: &str,
    item_id_or_prefix: &str,
    new_index: usize,
) -> Result<(), DeployError> {
    let _lock = ctx.lock_manager.acquire(
        LockResource::Instance(instance_id.to_string()),
        "content-move",
    )?;

    let mut manifest =
        crate::game_instance::get_manifest(ctx, instance_id).map_err(|e| match e {
            crate::game_instance::InstanceError::NotFound(id) => DeployError::InstanceNotFound(id),
            other => DeployError::Other(other.to_string()),
        })?;

    let mut layers = manifest.layers.layers().to_vec();

    // Collect indices of content layers
    let content_indices: Vec<usize> = layers
        .iter()
        .enumerate()
        .filter_map(|(i, l)| {
            if matches!(l.source, LayerSource::Content { .. }) {
                Some(i)
            } else {
                None
            }
        })
        .collect();

    if content_indices.is_empty() {
        return Err(DeployError::ContentNotFound(item_id_or_prefix.to_string()));
    }

    if new_index >= content_indices.len() {
        return Err(DeployError::Other(format!(
            "target position {} is out of bounds (instance has {} content item(s))",
            new_index + 1,
            content_indices.len()
        )));
    }

    let source_idx = resolve_content_layer(&layers, item_id_or_prefix)?;
    let Some(rel_idx) = content_indices.iter().position(|&idx| idx == source_idx) else {
        return Err(DeployError::ContentNotFound(item_id_or_prefix.to_string()));
    };

    let actual_source_idx = content_indices[rel_idx];
    let layer_to_move = layers.remove(actual_source_idx);

    // Recompute content indices after removal
    let updated_content_indices: Vec<usize> = layers
        .iter()
        .enumerate()
        .filter_map(|(i, l)| {
            if matches!(l.source, LayerSource::Content { .. }) {
                Some(i)
            } else {
                None
            }
        })
        .collect();

    let target_insert_idx = if new_index < updated_content_indices.len() {
        updated_content_indices[new_index]
    } else {
        // Insert after the last content item, or before first layer of rank > 2
        layers
            .iter()
            .position(|l| l.source.rank() > 2)
            .unwrap_or(layers.len())
    };

    layers.insert(target_insert_idx, layer_to_move);
    manifest.layers = LayerStack::new(layers)?;
    write_instance_manifest_atomic(ctx, instance_id, &manifest)?;

    Ok(())
}

#[cfg(test)]
mod placement_tests {
    use super::*;
    use std::collections::BTreeMap;

    fn base_file(path: &Path, rel: &str, placement: Placement, size: u64) -> PlannedFile {
        PlannedFile {
            path: RelPath::new(rel).unwrap(),
            source: FileSource::Base {
                path: path.to_path_buf(),
                linked_to_store: false,
            },
            placement,
            size,
            is_config_copy: false,
            layer_id: "base".to_string(),
        }
    }

    /// Every file under `root` with its bytes, keyed by its relative path.
    fn read_tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
        let mut out = BTreeMap::new();
        let mut folders = vec![root.to_path_buf()];
        while let Some(folder) = folders.pop() {
            for entry in std::fs::read_dir(&folder).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                if path.is_dir() {
                    folders.push(path);
                } else {
                    let rel = path
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/");
                    out.insert(rel, std::fs::read(&path).unwrap());
                }
            }
        }
        out
    }

    #[test]
    fn run_in_pool_returns_results_in_index_order_for_any_thread_count() {
        for threads in [1, 2, 8] {
            let out = run_in_pool(100, threads, |i| Ok(i * 2)).unwrap();
            assert_eq!(out, (0..100).map(|i| i * 2).collect::<Vec<_>>());
        }
        let none: Vec<usize> = run_in_pool(0, 4, |_| unreachable!()).unwrap();
        assert!(none.is_empty());
    }

    #[test]
    fn run_in_pool_returns_the_first_failure_in_index_order() {
        // A sequential run stops at index 37; a parallel one must report that same failure, not
        // whichever failing job happened to finish first.
        for threads in [1, 4, 8] {
            let err = run_in_pool(200, threads, |i| {
                if i == 37 || i == 90 {
                    Err(DeployError::Other(format!("job {i}")))
                } else {
                    Ok(i)
                }
            })
            .unwrap_err();
            assert_eq!(err.to_string(), "job 37", "threads = {threads}");
        }
    }

    #[test]
    fn parallel_placement_produces_the_same_farm_and_records_as_sequential() {
        let tmp = tempfile::TempDir::new().unwrap();
        let src = tmp.path().join("src");
        let mut files = Vec::new();
        for i in 0..60usize {
            let rel = format!("Data/dir{}/file{}.dat", i % 7, i);
            let path = src.join(&rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let body = format!("content {i} ").repeat(i + 1).into_bytes();
            std::fs::write(&path, &body).unwrap();
            let placement = if i % 3 == 0 {
                Placement::Copy
            } else {
                Placement::Link
            };
            files.push(base_file(&path, &rel, placement, body.len() as u64));
        }

        let place_all = |threads: usize, name: &str| -> (Vec<PlacedFile>, PathBuf) {
            let staging = tmp.path().join(name);
            std::fs::create_dir_all(&staging).unwrap();
            create_planned_dirs(&staging, &files).unwrap();
            let placed = run_in_pool(files.len(), threads, |i| {
                let FileSource::Base { path, .. } = &files[i].source else {
                    unreachable!("the test plans base files only");
                };
                place_one(path, &staging.join(files[i].path.as_str()), &files[i])
            })
            .unwrap();
            (placed, staging)
        };

        let (sequential, seq_dir) = place_all(1, "sequential");
        let (parallel, par_dir) = place_all(8, "parallel");
        assert_eq!(sequential.len(), files.len());
        assert_eq!(parallel.len(), files.len());
        for ((seq, par), planned) in sequential.iter().zip(&parallel).zip(&files) {
            // Plan order, the same placement and size. A copy's time is when it was copied, so it is
            // not compared.
            assert_eq!(seq.record.path, planned.path);
            assert_eq!(par.record.path, planned.path);
            assert_eq!(seq.record.placement, par.record.placement);
            assert_eq!(seq.record.size, par.record.size);
            assert_eq!(seq.linked, par.linked);
            assert_eq!(seq.size, par.size);
            // Links are links here: the same placement as planned.
            assert_eq!(seq.record.placement, planned.placement);
        }
        assert_eq!(read_tree(&seq_dir), read_tree(&par_dir));
    }

    #[test]
    fn a_parallel_placement_failure_is_the_sequential_failure() {
        let tmp = tempfile::TempDir::new().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        let mut files = Vec::new();
        for i in 0..40usize {
            let path = src.join(format!("f{i}.dat"));
            // Files 12 and 30 have no source: placing them fails.
            if i != 12 && i != 30 {
                std::fs::write(&path, b"ok").unwrap();
            }
            files.push(base_file(&path, &format!("f{i}.dat"), Placement::Copy, 2));
        }
        for threads in [1, 8] {
            let staging = tmp.path().join(format!("staging-{threads}"));
            std::fs::create_dir_all(&staging).unwrap();
            let err = run_in_pool(files.len(), threads, |i| {
                let FileSource::Base { path, .. } = &files[i].source else {
                    unreachable!();
                };
                place_one(path, &staging.join(files[i].path.as_str()), &files[i])
            })
            .unwrap_err();
            assert!(
                matches!(&err, DeployError::Io(e) if e.kind() == std::io::ErrorKind::NotFound),
                "threads = {threads}: {err:?}"
            );
        }
    }

    #[test]
    fn farm_sizes_lists_every_entry_with_its_metadata_size() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("game");
        for (rel, body) in [
            ("Game.exe", &b"exe"[..]),
            ("Data/a.esp", &b"plugin bytes"[..]),
            ("Data/deep/b.bin", &b""[..]),
            ("Data/deep/er/c.ini", &b"[x]"[..]),
        ] {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, body).unwrap();
        }
        let sizes = farm_sizes(&root);
        for (rel, _) in [
            ("Game.exe", 0),
            ("Data/a.esp", 0),
            ("Data/deep/b.bin", 0),
            ("Data/deep/er/c.ini", 0),
        ] {
            let meta = std::fs::metadata(root.join(rel)).unwrap();
            assert_eq!(sizes.get(rel), Some(&meta.len()), "{rel}");
        }
        // Folders are listed too, as the per-path lookup would find them.
        assert!(sizes.contains_key("Data"));
        assert!(sizes.contains_key("Data/deep/er"));
        assert_eq!(sizes.len(), 7);
        // A path the listing does not have is absent, so the caller looks it up itself.
        assert!(!sizes.contains_key("Data/missing.txt"));
    }

    #[test]
    fn precompiled_path_rules_answer_as_the_definition_does() {
        let definition = GameDefinition {
            mo2_game_name: None,
            id: agora_game_api::GameId::new("rules-game").unwrap(),
            name: "Rules Game".into(),
            stores: vec![],
            version_sources: vec![],
            deployment: agora_game_api::DeploymentStrategy::VirtualFileSystem,
            content_rules: vec![],
            native_code_patterns: vec![],
            framework_ids: vec![],
            tool_ids: vec![],
            launch: None,
            log_paths: vec![],
            crash_paths: vec![],
            user_files: vec![],
            save_paths: vec![],
            linked_archive_patterns: vec![],
            declared_writes: vec![
                "writeable_base.txt".into(),
                "saves/*".into(),
                "Data/**/Config.ini".into(),
            ],
            excluded_paths: vec![],
            plugin_list: None,
            runtime_files: Vec::new(),
            save_location: Vec::new(),
            launch_alternatives: Vec::new(),
            content_layout: None,
            copy_patterns: vec!["custom/**".into(), "*.dat".into()],
        };
        let rules = PathRules::new(&definition);
        let paths = [
            "Data/a.ini",
            "Data/a.INI",
            "Data/x/y.json",
            "Data/x/y.txt",
            "writeable_base.txt",
            "Writeable_Base.txt",
            "Data/cfg/a.cfg",
            "saves/x.xml",
            "saves/deep/x.xml",
            "Data/x.yml.bak",
            "a/b/c.toml",
            "custom/keep.bin",
            "custom/deep/keep.bin",
            "top.dat",
            "Data/deep/config.ini",
            "Data/config.ini",
            "../escape.ini",
            "Data/Skyrim.bsa",
        ];
        for path in paths {
            assert_eq!(
                rules.is_copy_pattern(path),
                definition.is_copy_pattern(path),
                "copy pattern: {path}"
            );
            assert_eq!(
                rules.is_declared_write(path),
                definition.is_declared_write(path),
                "declared write: {path}"
            );
        }
    }
}
