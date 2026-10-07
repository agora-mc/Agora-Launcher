//! Game content deployment for instances (MASTER_SPEC §26.5).
//!
//! Deploys base files and mod content into a physical game runtime directory
//! using hardlinks or copies, harvests writes into the instance's writable layer,
//! and tracks deployed file state.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use agora_game_api::{
    BaseReference, GameDefinition, Layer, LayerId, LayerSource, LayerStack, RelPath,
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
}

pub fn plan(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    mode: DeployMode,
) -> Result<DeploymentPlan, DeployError> {
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
                    own_copy: layer.own_copy,
                });
            }
        }
    }

    // 3. Writable layer files
    let instance_dir = ctx
        .paths
        .instance_dir(instance_id)
        .map_err(|e| DeployError::Other(e.to_string()))?;
    let writable_dir = instance_dir.join("writable");
    // Under the VFS the writable layer is shown on top of the farm, not placed in it, so the
    // farm (and its fingerprint) does not change when the game writes.
    if mode != DeployMode::Virtual && writable_dir.exists() {
        let mut writable_files = Vec::new();
        walk_writable_dir(&writable_dir, Path::new(""), &mut writable_files)?;
        if !writable_files.is_empty() {
            layers.push(LayerContribution {
                layer_id: "writable".to_string(),
                files: writable_files,
                own_copy: false,
            });
        }
    }

    // 4. Check for File-versus-Folder conflicts across layers
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

    // 5. Build winning map and record overrides
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

    // 6. Remove whiteouts from writable layer
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
    for marker in read_whiteout_markers(&writable_dir)? {
        let prefix = format!("{marker}/");
        candidates.retain(|key, cand| {
            matches!(cand.source, FileSource::Writable { .. })
                || !(*key == marker || key.starts_with(&prefix))
        });
    }

    // 7. Determine placement & compute fingerprint
    let detector = VolumeDetector::new();
    let deploy_vol = detector.get_volume_info(&game_dir);

    let mut planned_files = Vec::with_capacity(candidates.len());
    let mut overrides = Vec::new();
    let mut fingerprint_lines = Vec::with_capacity(candidates.len());

    for (_key, cand) in candidates {
        let is_config_copy = mode == DeployMode::Links
            && cand.size <= 1024 * 1024
            && definition.is_copy_pattern(cand.path.as_str())
            && !definition.is_declared_write(cand.path.as_str())
            && !cand.own_copy
            && !matches!(cand.source, FileSource::Writable { .. });

        let placement = match mode {
            DeployMode::Copies => Placement::Copy,
            DeployMode::Virtual => {
                if cand.own_copy || definition.is_declared_write(cand.path.as_str()) {
                    Placement::Copy
                } else {
                    match &cand.source {
                        FileSource::Writable { .. } => Placement::Copy,
                        FileSource::Base { .. } => Placement::Link,
                        FileSource::Content { sha256, .. } => {
                            let obj_path = ctx.paths.content_object_path(sha256);
                            let obj_vol = detector.get_volume_info(&obj_path);
                            match (&obj_vol, &deploy_vol) {
                                (Some(ov), Some(dv))
                                    if ov.id.eq_ignore_ascii_case(&dv.id)
                                        && ov.supports_hardlinks
                                        && dv.supports_hardlinks =>
                                {
                                    Placement::Link
                                }
                                _ => Placement::Copy,
                            }
                        }
                    }
                }
            }
            DeployMode::Links => {
                // A declared-write path gets the instance's own copy in every mode.
                // An own_copy layer deploys all its files as copies.
                // Small text files matching copy patterns (<= 1 MiB) are placed as copies too.
                if cand.own_copy
                    || definition.is_declared_write(cand.path.as_str())
                    || is_config_copy
                {
                    Placement::Copy
                } else {
                    match &cand.source {
                        FileSource::Writable { .. } => Placement::Copy,
                        FileSource::Base { .. } => Placement::Link,
                        FileSource::Content { sha256, .. } => {
                            let obj_path = ctx.paths.content_object_path(sha256);
                            let obj_vol = detector.get_volume_info(&obj_path);
                            match (&obj_vol, &deploy_vol) {
                                (Some(ov), Some(dv))
                                    if ov.id.eq_ignore_ascii_case(&dv.id)
                                        && ov.supports_hardlinks
                                        && dv.supports_hardlinks =>
                                {
                                    Placement::Link
                                }
                                _ => Placement::Copy,
                            }
                        }
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
                winner: cand.layer_id,
                hidden: cand.hidden_layers,
            });
        }

        planned_files.push(PlannedFile {
            path: cand.path,
            source: cand.source,
            placement,
            size: cand.size,
            is_config_copy,
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
        let meta = std::fs::symlink_metadata(&abs_path)?;
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
            let meta = std::fs::symlink_metadata(entry.path())?;
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
    let _lock = ctx
        .lock_manager
        .acquire(LockResource::Instance(instance_id.to_string()), "deploy")?;

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

    if game_dir.exists() {
        let running = crate::game_launch::processes_running_from(&game_dir);
        if !running.is_empty() {
            let pids: Vec<String> = running.iter().map(|p| p.pid.to_string()).collect();
            return Err(DeployError::ProcessesRunning(pids.join(", ")));
        }
    }

    let mut current_plan = plan(ctx, instance_id, definition, mode)?;

    // Check if current deployment is up to date
    if record_path.exists() && game_dir.exists() {
        if let Ok(content) = std::fs::read_to_string(&record_path) {
            if let Ok(record) = serde_json::from_str::<DeploymentRecord>(&content) {
                if record.fingerprint == current_plan.fingerprint
                    && record.mode == mode
                    && record.instance_id == instance_id
                {
                    let mut up_to_date = true;
                    for f in &record.files {
                        // The instance's own copy of a path the game is declared to write is
                        // expected to change (an early write under the VFS lands in it); that
                        // is the game's state, not a damaged deployment.
                        // Small config files matching copy patterns and files from own_copy layers
                        // are likewise expected to be rewritten by the game/mod.
                        let is_own_copy = match &f.source {
                            FileSource::Content { item_id, .. } => {
                                manifest.layers.layers().iter().any(|l| {
                                    l.enabled
                                        && l.own_copy
                                        && match &l.source {
                                            LayerSource::Content { content } => content == item_id,
                                            _ => false,
                                        }
                                })
                            }
                            _ => false,
                        };
                        if f.placement == Placement::Copy
                            && (definition.is_declared_write(f.path.as_str())
                                || (f.size <= 1024 * 1024
                                    && definition.is_copy_pattern(f.path.as_str()))
                                || is_own_copy)
                        {
                            continue;
                        }
                        let path = game_dir.join(f.path.as_str());
                        match std::fs::metadata(&path) {
                            Ok(m) if m.len() == f.size => {}
                            _ => {
                                up_to_date = false;
                                break;
                            }
                        }
                    }
                    if up_to_date {
                        let plugins = activate_plugins(
                            ctx,
                            instance_id,
                            definition,
                            &manifest,
                            &base_manifest,
                            &current_plan,
                        )?;
                        return Ok(DeployOutcome::UpToDate { plugins });
                    }
                }
            }
        }
    }

    // Existing deployment is not up to date: harvest it first
    let harvest = if record_path.exists() {
        let rep = harvest_internal(ctx, instance_id, &deployment_dir, &manifest)?;
        // If anything was harvested into the writable layer, re-compute the plan
        if !rep.is_empty() {
            current_plan = plan(ctx, instance_id, definition, mode)?;
        }
        Some(rep)
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

    let mut linked_count = 0usize;
    let mut copied_count = 0usize;
    let mut copied_bytes = 0u64;
    let mut config_copied_count = 0usize;
    let mut recorded_files = Vec::with_capacity(current_plan.files.len());

    for file in &current_plan.files {
        if file.is_config_copy {
            config_copied_count += 1;
        }

        let dest = staging_dir.join(file.path.as_str());
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let source_path = match &file.source {
            FileSource::Base { path, .. } => path.clone(),
            FileSource::Content { sha256, .. } => ctx.paths.content_object_path(sha256),
            FileSource::Writable { path } => path.clone(),
        };

        let mut actual_placement = file.placement;
        if file.placement == Placement::Link {
            match std::fs::hard_link(&source_path, &dest) {
                Ok(()) => {
                    linked_count += 1;
                }
                Err(_) => {
                    // Fallback to copy if hard link fails
                    std::fs::copy(&source_path, &dest)?;
                    copied_count += 1;
                    copied_bytes += file.size;
                    actual_placement = Placement::Copy;
                }
            }
        } else {
            std::fs::copy(&source_path, &dest)?;
            copied_count += 1;
            copied_bytes += file.size;
        }

        let meta = std::fs::metadata(&dest)?;
        let mtime = get_modified_unix_ms(&meta);
        recorded_files.push(DeployedFileRecord {
            path: file.path.clone(),
            source: file.source.clone(),
            placement: actual_placement,
            size: meta.len(),
            modified_unix_ms: mtime,
        });
    }

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
    let record_bytes = serde_json::to_vec_pretty(&record)?;
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
    )?;

    Ok(DeployOutcome::Built {
        linked: linked_count,
        copied: copied_count,
        copied_bytes,
        config_copied: config_copied_count,
        harvest,
        plugins,
    })
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
    Ok(crate::game_plugins::sync_locked(
        ctx,
        instance_id,
        definition,
        &base_manifest.runtime.store,
        &desired,
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

fn harvest_internal(
    ctx: &Ctx,
    instance_id: &str,
    deployment_dir: &Path,
    manifest: &crate::game_instance::GameInstanceManifest,
) -> Result<HarvestReport, DeployError> {
    let game_dir = deployment_dir.join("game");
    let record_path = deployment_dir.join("deployment.json");

    // Guard safety checks
    validate_deployment_safety(deployment_dir, instance_id)?;

    let record_content = std::fs::read_to_string(&record_path)?;
    let record: DeploymentRecord = serde_json::from_str(&record_content)
        .map_err(|e| DeployError::CorruptDeployment(format!("unreadable deployment.json: {e}")))?;

    let mut record_map: BTreeMap<String, &DeployedFileRecord> = BTreeMap::new();
    for f in &record.files {
        record_map.insert(f.path.as_str().to_ascii_lowercase(), f);
    }

    // Walk current game_dir
    struct CurrentFile {
        rel_path: RelPath,
        size: u64,
        modified_unix_ms: i64,
        abs_path: PathBuf,
    }
    let mut current_files: BTreeMap<String, CurrentFile> = BTreeMap::new();

    fn walk_dir(
        root: &Path,
        rel: &Path,
        out: &mut BTreeMap<String, CurrentFile>,
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
            let meta = std::fs::symlink_metadata(&abs_path)?;
            if is_reparse_point_or_symlink(&meta) {
                continue;
            }
            let file_name = entry.file_name();
            let child_rel = rel.join(&file_name);
            if meta.is_dir() {
                walk_dir(root, &child_rel, out)?;
            } else if meta.is_file() {
                let rel_str = child_rel.to_string_lossy().replace('\\', "/");
                let rel_path = RelPath::new(rel_str)
                    .map_err(|e| DeployError::Other(format!("invalid game file path: {e}")))?;
                let mtime = get_modified_unix_ms(&meta);
                out.insert(
                    rel_path.as_str().to_ascii_lowercase(),
                    CurrentFile {
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

    walk_dir(&game_dir, Path::new(""), &mut current_files)?;

    let instance_dir = ctx
        .paths
        .instance_dir(instance_id)
        .map_err(|e| DeployError::Other(e.to_string()))?;
    let writable_dir = instance_dir.join("writable");

    let mut report = HarvestReport::default();
    let mut new_whiteouts = Vec::new();

    // 1. Files in current_files
    for (lower_path, cur_file) in &current_files {
        if let Some(rec_file) = record_map.get(lower_path) {
            match rec_file.placement {
                Placement::Copy => {
                    if cur_file.size != rec_file.size
                        || cur_file.modified_unix_ms != rec_file.modified_unix_ms
                    {
                        let dest = writable_dir.join(cur_file.rel_path.as_str());
                        if let Some(parent) = dest.parent() {
                            std::fs::create_dir_all(parent)?;
                        }
                        std::fs::copy(&cur_file.abs_path, &dest)?;
                        clear_whiteout_marker(&writable_dir, &cur_file.rel_path);
                        report.copied_to_writable.push(cur_file.rel_path.clone());
                    }
                }
                Placement::Link => {
                    if let FileSource::Base {
                        linked_to_store, ..
                    } = rec_file.source
                    {
                        if cur_file.size != rec_file.size
                            || cur_file.modified_unix_ms != rec_file.modified_unix_ms
                        {
                            let dest = writable_dir.join(cur_file.rel_path.as_str());
                            if let Some(parent) = dest.parent() {
                                std::fs::create_dir_all(parent)?;
                            }
                            std::fs::copy(&cur_file.abs_path, &dest)?;
                            clear_whiteout_marker(&writable_dir, &cur_file.rel_path);
                            report.copied_to_writable.push(cur_file.rel_path.clone());
                            report.base_files_changed.push(BaseFileChanged {
                                path: cur_file.rel_path.clone(),
                                linked_to_store,
                            });
                        }
                    }
                }
            }
        } else {
            // New file not in record
            let dest = writable_dir.join(cur_file.rel_path.as_str());
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&cur_file.abs_path, &dest)?;
            clear_whiteout_marker(&writable_dir, &cur_file.rel_path);
            report.copied_to_writable.push(cur_file.rel_path.clone());
        }
    }

    // 2. Missing files that were recorded
    for rec_file in &record.files {
        let lower = rec_file.path.as_str().to_ascii_lowercase();
        if !current_files.contains_key(&lower) {
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

    // 4. Safe delete game_dir and record_path
    if game_dir.exists() {
        std::fs::remove_dir_all(&game_dir)?;
    }
    if record_path.exists() {
        std::fs::remove_file(&record_path)?;
    }

    Ok(report)
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
