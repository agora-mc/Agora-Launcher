//! Content: the store of mod archives and what each instance has enabled, in load order.
//! Owned by the `content` slice.
//!
//! Every command is a thin adapter over the agora-core content store (`content_store`) and the
//! instance's layers (`game_deploy`), the same calls `agora games content` and
//! `agora games instance content` make. The store is shared by every game: the core records a
//! game on an item only for a curated catalog entry, so the Library lists every stored item.

use std::path::{Path, PathBuf};

use agora_core::content_store::{
    self, AddOutcome, ContentError, ContentItem, ContentProblem, ContentSource, ItemEntry,
    ProblemKind, VerifyDepth,
};
use agora_core::content_thunderstore::{self, ThunderstoreInstallError};
use agora_core::game_deploy::{self, DeployError, PlacementDecision};
use agora_core::game_instance::{self, InstanceError};
use agora_game_api::{Layer, LayerSource};
use serde::Serialize;

use super::error::{GamesError, CORE_FAILED, TASK_FAILED};

/// The item is installed in at least one instance, so the store refuses to remove it.
pub const IN_USE: &str = "in_use";
/// The request is missing something the command needs (a blank path, a position of 0).
pub const INVALID_INPUT: &str = "invalid_input";
/// The archive's files do not say where they go, and the game has no rule for them.
pub const PLACEMENT_UNKNOWN: &str = "placement_unknown";

// ---------------------------------------------------------------------------
// Replies
// ---------------------------------------------------------------------------

/// One item in the content store, as the Library shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreItem {
    pub item_id: String,
    pub name: String,
    pub file_count: usize,
    pub total_size: u64,
    /// `archive`, `folder`, `fomod`, `thunderstore`, `mo2`, `catalog`, or `unknown`.
    pub source_kind: &'static str,
    /// Where the item came from, in a sentence a person can read.
    pub source_label: String,
    pub added_at_unix_ms: i64,
    /// The games the store recorded for this item (a curated catalog entry names one). Empty for
    /// an item added by hand: nothing says which game it is for, so every game's Library shows it.
    pub game_ids: Vec<String>,
}

/// A record in the store that cannot be read as an item. Listed so the user can see why and
/// remove it, instead of the Library looking emptier than the store is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DamagedStoreItem {
    pub item_id: String,
    /// Why the record cannot be used, naming the file and the cause.
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreList {
    pub items: Vec<StoreItem>,
    pub damaged: Vec<DamagedStoreItem>,
}

/// The result of adding an archive or folder to the store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreAdded {
    /// `added` for new bytes, `existing` when the store already held this content.
    pub status: &'static str,
    pub item: StoreItem,
    pub objects_new: usize,
    pub objects_restored: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyProblem {
    pub path: String,
    /// `missing`, `size_mismatch`, `hash_mismatch`, `unprotected`, or `corrupt_manifest`.
    pub kind: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyReport {
    pub item_id: String,
    pub checked: usize,
    pub hashed: usize,
    pub problems: Vec<VerifyProblem>,
}

/// One content layer of an instance, in load order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceContentRow {
    /// 1-based, lowest priority first, as `move` takes it.
    pub position: usize,
    pub item_id: String,
    pub layer_id: String,
    pub name: String,
    pub source_kind: &'static str,
    pub source_label: String,
    pub enabled: bool,
    /// Where the layer mounts, relative to the game folder (`""` is the root).
    pub mount_path: String,
    /// The folder inside the item the layer reads from (`""` is the item's root).
    pub source_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceContentList {
    pub instance_id: String,
    pub items: Vec<InstanceContentRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThunderstoreAdded {
    pub package_id: String,
    pub version: String,
    pub summary: String,
    pub missing_dependencies: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddedToInstance {
    /// The item that was added. For a Thunderstore package this is the derived item it installed as.
    pub item_id: String,
    pub layer_id: String,
    pub mount_path: String,
    pub source_path: String,
    /// Why Agora put the item where it did, when it chose a place itself.
    pub placement_reason: Option<String>,
    pub thunderstore: Option<ThunderstoreAdded>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NeedsInstaller {
    pub item_id: String,
    pub reason: String,
    pub top_level: Vec<String>,
}

/// What adding a stored item to an instance did. `NeedsInstaller` is not a failure: the item
/// has a FOMOD installer, and the user answers its questions before anything is added.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum InstanceAddOutcome {
    Added(AddedToInstance),
    NeedsInstaller(NeedsInstaller),
}

// ---------------------------------------------------------------------------
// Pure mappings (core values to replies), tested below
// ---------------------------------------------------------------------------

/// The item's origin, from its oldest source (sources are kept oldest first).
pub fn store_item(item: &ContentItem) -> StoreItem {
    let (source_kind, source_label) = match item.sources.first() {
        None => (
            "unknown",
            "No source was recorded for this item.".to_string(),
        ),
        Some(ContentSource::Archive { path, .. }) => {
            ("archive", format!("Archive: {}", last_component(path)))
        }
        Some(ContentSource::Folder { path, .. }) => {
            ("folder", format!("Folder: {}", last_component(path)))
        }
        Some(ContentSource::FomodInstall { from_item, .. }) => (
            "fomod",
            format!("Made by an installer from item {}", short_id(from_item)),
        ),
        Some(ContentSource::Thunderstore {
            package, version, ..
        }) => (
            "thunderstore",
            format!("Thunderstore package {package} {version}"),
        ),
        Some(ContentSource::Mo2Import { mo2_folder, .. }) => {
            ("mo2", format!("Mod Organizer 2 folder: {mo2_folder}"))
        }
        Some(ContentSource::Catalog { game, verified, .. }) => {
            (
                "catalog",
                if *verified {
                    format!("Curated catalog entry for {game}; its source published a hash that matched")
                } else {
                    format!("Curated catalog entry for {game}; its source published no hash")
                },
            )
        }
        Some(_) => (
            "unknown",
            "Added from a source this version does not describe.".to_string(),
        ),
    };
    StoreItem {
        item_id: item.item_id.clone(),
        name: item.name.clone(),
        file_count: item.files.len(),
        total_size: item.total_size,
        source_kind,
        source_label,
        added_at_unix_ms: item.added_at_unix_ms,
        game_ids: item_game_ids(item),
    }
}

/// The games the store recorded on the item: one per curated catalog source, in order, once each.
fn item_game_ids(item: &ContentItem) -> Vec<String> {
    let mut games: Vec<String> = Vec::new();
    for source in &item.sources {
        if let ContentSource::Catalog { game, .. } = source {
            if !games.contains(game) {
                games.push(game.clone());
            }
        }
    }
    games
}

/// The Library's rows: readable items as rows, unreadable records beside them.
fn store_list(entries: &[ItemEntry]) -> StoreList {
    let mut items = Vec::new();
    let mut damaged = Vec::new();
    for entry in entries {
        match entry {
            ItemEntry::Readable(item) => items.push(store_item(item)),
            ItemEntry::Damaged(record) => damaged.push(DamagedStoreItem {
                item_id: record.item_id.clone(),
                message: record.error.clone(),
            }),
        }
    }
    StoreList { items, damaged }
}

pub fn store_added(outcome: &AddOutcome) -> StoreAdded {
    let (status, objects_new, objects_restored) = match outcome {
        AddOutcome::Added { objects_new, .. } => ("added", *objects_new, 0),
        AddOutcome::Existing {
            objects_new,
            objects_restored,
            ..
        } => ("existing", *objects_new, *objects_restored),
    };
    StoreAdded {
        status,
        item: store_item(outcome.item()),
        objects_new,
        objects_restored,
    }
}

pub fn verify_report(verification: &content_store::ContentVerification) -> VerifyReport {
    VerifyReport {
        item_id: verification.item_id.clone(),
        checked: verification.checked,
        hashed: verification.hashed,
        problems: verification.problems.iter().map(verify_problem).collect(),
    }
}

fn verify_problem(problem: &ContentProblem) -> VerifyProblem {
    let (kind, message) = match &problem.kind {
        ProblemKind::Missing => ("missing", "The file is missing from the store.".to_string()),
        ProblemKind::SizeMismatch { expected, actual } => (
            "size_mismatch",
            format!("The file is {actual} bytes, but the store recorded {expected}."),
        ),
        ProblemKind::HashMismatch { .. } => (
            "hash_mismatch",
            "The file changed: its contents no longer match the recorded hash.".to_string(),
        ),
        ProblemKind::Unprotected => (
            "unprotected",
            "The file is not protected from changes.".to_string(),
        ),
        ProblemKind::CorruptManifest { error } => (
            "corrupt_manifest",
            format!("The item's record cannot be read: {error}"),
        ),
    };
    VerifyProblem {
        path: problem.path.clone(),
        kind,
        message,
    }
}

/// The content layers of an instance, in load order, as rows. `items` holds each content
/// layer's stored item, in the same order. Other layers (the base, a game's own writes) are not
/// content and are left out, as `agora games instance content list` leaves them out.
pub fn instance_content_rows(
    layers: &[Layer],
    items: &[ContentItem],
) -> Result<Vec<InstanceContentRow>, GamesError> {
    let disagree = || {
        GamesError::new(
            CORE_FAILED,
            "The instance's layers and the content store disagree about its items.",
        )
    };
    let content_layers: Vec<(&Layer, &str)> = layers
        .iter()
        .filter_map(|layer| match &layer.source {
            LayerSource::Content { content } => Some((layer, content.as_str())),
            _ => None,
        })
        .collect();
    if content_layers.len() != items.len() {
        return Err(disagree());
    }
    let mut rows = Vec::with_capacity(items.len());
    for (index, ((layer, content), item)) in content_layers.iter().zip(items).enumerate() {
        if *content != item.item_id {
            return Err(disagree());
        }
        let store = store_item(item);
        rows.push(InstanceContentRow {
            position: index + 1,
            item_id: item.item_id.clone(),
            layer_id: layer.id.as_str().to_string(),
            name: item.name.clone(),
            source_kind: store.source_kind,
            source_label: store.source_label,
            enabled: layer.enabled,
            mount_path: layer.mount_path.as_str().to_string(),
            source_path: layer.source_path.as_str().to_string(),
        });
    }
    Ok(rows)
}

/// The 0-based index `move` takes, from the 1-based position the user sees. Position 0 names no
/// place, so it is refused rather than wrapped around.
pub fn move_index(position: usize) -> Result<usize, GamesError> {
    if position == 0 {
        return Err(GamesError::new(
            INVALID_INPUT,
            "The position must be 1 or more (1 is the lowest priority).",
        ));
    }
    Ok(position - 1)
}

/// A path the user chose. Only a blank path is refused here; the core checks that the file or
/// folder exists and can be read.
pub fn checked_source_path(raw: &str) -> Result<PathBuf, GamesError> {
    if raw.trim().is_empty() {
        return Err(GamesError::new(
            INVALID_INPUT,
            "Choose an archive or a folder first.",
        ));
    }
    Ok(PathBuf::from(raw))
}

/// A stored item or instance id. Blank is refused before the core sees it.
pub(super) fn checked_id(raw: &str) -> Result<String, GamesError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(GamesError::new(INVALID_INPUT, "No id was given."));
    }
    Ok(trimmed.to_string())
}

pub(super) fn content_error(error: ContentError) -> GamesError {
    match error {
        ContentError::InUse { instances, .. } => GamesError::new(
            IN_USE,
            format!(
                "This item is used by {instances}. Remove it from those instances first, then remove it here."
            ),
        ),
        other => GamesError::new(CORE_FAILED, other.to_string()),
    }
}

pub(super) fn deploy_error(error: DeployError) -> GamesError {
    GamesError::new(CORE_FAILED, error.to_string())
}

pub(super) fn instance_error(error: InstanceError) -> GamesError {
    GamesError::new(CORE_FAILED, error.to_string())
}

fn thunderstore_error(error: ThunderstoreInstallError) -> GamesError {
    GamesError::new(CORE_FAILED, error.to_string())
}

fn last_component(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| path.to_string())
}

/// At most 12 characters of an id, as the CLI shows them. Counted in characters, so a stray
/// non-ASCII byte in a record cannot make the slice panic.
fn short_id(id: &str) -> String {
    id.chars().take(12).collect()
}

fn added_layer(layer: &Layer, item_id: &str, placement_reason: Option<String>) -> AddedToInstance {
    AddedToInstance {
        item_id: item_id.to_string(),
        layer_id: layer.id.as_str().to_string(),
        mount_path: layer.mount_path.as_str().to_string(),
        source_path: layer.source_path.as_str().to_string(),
        placement_reason,
        thunderstore: None,
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Every item in the content store, oldest first by id.
#[tauri::command]
pub async fn games_content_list(app: tauri::AppHandle) -> Result<StoreList, GamesError> {
    tokio::task::spawn_blocking(move || -> Result<StoreList, GamesError> {
        let ctx = crate::core_context(&app)?;
        let entries = content_store::list_items(&ctx).map_err(content_error)?;
        Ok(store_list(&entries))
    })
    .await
    .map_err(|_| GamesError::new(TASK_FAILED, "Reading the library did not finish."))?
}

/// Add an archive, or a folder, to the content store. A blank `name` keeps the item's own name.
#[tauri::command]
pub async fn games_content_add(
    app: tauri::AppHandle,
    source_path: String,
    name: Option<String>,
) -> Result<StoreAdded, GamesError> {
    let path = checked_source_path(&source_path)?;
    let name = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
    tokio::task::spawn_blocking(move || -> Result<StoreAdded, GamesError> {
        let ctx = crate::core_context(&app)?;
        let outcome = if path.is_dir() {
            content_store::add_folder(&ctx, &path, name.as_deref())
        } else {
            content_store::add_archive(&ctx, &path, name.as_deref())
        }
        .map_err(content_error)?;
        Ok(store_added(&outcome))
    })
    .await
    .map_err(|_| GamesError::new(TASK_FAILED, "Adding to the library did not finish."))?
}

/// Check a stored item against the hashes it was stored with. `full` hashes every file, which is
/// slower and finds changes a quick check would miss.
#[tauri::command]
pub async fn games_content_verify(
    app: tauri::AppHandle,
    item_id: String,
    full: bool,
) -> Result<VerifyReport, GamesError> {
    let item_id = checked_id(&item_id)?;
    let depth = if full {
        VerifyDepth::Full
    } else {
        VerifyDepth::Quick
    };
    tokio::task::spawn_blocking(move || -> Result<VerifyReport, GamesError> {
        let ctx = crate::core_context(&app)?;
        let verification =
            content_store::verify_item(&ctx, &item_id, depth).map_err(content_error)?;
        Ok(verify_report(&verification))
    })
    .await
    .map_err(|_| GamesError::new(TASK_FAILED, "Checking the item did not finish."))?
}

/// Remove an item from the store. Refused while an instance uses it; the error names them.
#[tauri::command]
pub async fn games_content_remove(
    app: tauri::AppHandle,
    item_id: String,
) -> Result<(), GamesError> {
    let item_id = checked_id(&item_id)?;
    tokio::task::spawn_blocking(move || -> Result<(), GamesError> {
        let ctx = crate::core_context(&app)?;
        content_store::remove_item(&ctx, &item_id).map_err(content_error)
    })
    .await
    .map_err(|_| GamesError::new(TASK_FAILED, "Removing the item did not finish."))?
}

/// The content layers an instance has, in load order.
#[tauri::command]
pub async fn games_instance_content_list(
    app: tauri::AppHandle,
    instance_id: String,
) -> Result<InstanceContentList, GamesError> {
    let instance_id = checked_id(&instance_id)?;
    tokio::task::spawn_blocking(move || -> Result<InstanceContentList, GamesError> {
        let ctx = crate::core_context(&app)?;
        let manifest = game_instance::get_manifest(&ctx, &instance_id).map_err(instance_error)?;
        let layers = manifest.layers.layers();
        let mut items = Vec::new();
        for layer in layers {
            if let LayerSource::Content { content } = &layer.source {
                // A missing or unreadable store item is an error, never a row with no name.
                items.push(content_store::get_item(&ctx, content).map_err(content_error)?);
            }
        }
        Ok(InstanceContentList {
            instance_id: manifest.instance_id.clone(),
            items: instance_content_rows(layers, &items)?,
        })
    })
    .await
    .map_err(|_| {
        GamesError::new(
            TASK_FAILED,
            "Reading the instance's content did not finish.",
        )
    })?
}

/// Add a stored item to an instance, placed by the game's content rules. An archive with a FOMOD
/// installer is not added: the reply says so, and the UI opens the installer's questions.
#[tauri::command]
pub async fn games_instance_content_add(
    app: tauri::AppHandle,
    instance_id: String,
    item_id: String,
) -> Result<InstanceAddOutcome, GamesError> {
    let instance_id = checked_id(&instance_id)?;
    let item_id = checked_id(&item_id)?;
    tokio::task::spawn_blocking(move || -> Result<InstanceAddOutcome, GamesError> {
        let ctx = crate::core_context(&app)?;
        let manifest = game_instance::get_manifest(&ctx, &instance_id).map_err(instance_error)?;
        let game_def = ctx.games.game(&manifest.game).ok_or_else(|| {
            GamesError::new(
                CORE_FAILED,
                format!("Agora does not know the game '{}'.", manifest.game),
            )
        })?;
        let layout = game_def.content_layout.as_ref().ok_or_else(|| {
            GamesError::new(
                CORE_FAILED,
                format!(
                    "Agora has no content rules for {}, so it cannot say where this item goes.",
                    game_def.name
                ),
            )
        })?;

        // A Thunderstore package installs the way the game's BepInEx layout describes, as the
        // CLI does. A manifest that cannot be read is an error here, not "not a package".
        if layout.thunderstore_bepinex
            && content_thunderstore::parse_manifest(&ctx, &item_id)
                .map_err(content_error)?
                .is_some()
        {
            let outcome = content_thunderstore::install_thunderstore(&ctx, &instance_id, &item_id)
                .map_err(thunderstore_error)?;
            return Ok(InstanceAddOutcome::Added(AddedToInstance {
                item_id: outcome.derived_item.item_id.clone(),
                layer_id: outcome.layer.id.as_str().to_string(),
                mount_path: outcome.layer.mount_path.as_str().to_string(),
                source_path: outcome.layer.source_path.as_str().to_string(),
                placement_reason: None,
                thunderstore: Some(ThunderstoreAdded {
                    package_id: outcome.package_id,
                    version: outcome.version,
                    summary: outcome.summary,
                    missing_dependencies: outcome.missing_dependencies,
                }),
            }));
        }

        let item = content_store::get_item(&ctx, &item_id).map_err(content_error)?;
        match game_deploy::decide_placement(&item, layout) {
            PlacementDecision::Place {
                mount_path,
                source_path,
                reason,
            } => {
                let mount = (!mount_path.is_empty()).then_some(mount_path.as_str());
                let source = (!source_path.is_empty()).then_some(source_path.as_str());
                let layer = game_deploy::add_content(&ctx, &instance_id, &item_id, mount, source)
                    .map_err(deploy_error)?;
                Ok(InstanceAddOutcome::Added(added_layer(
                    &layer,
                    &item_id,
                    Some(reason),
                )))
            }
            PlacementDecision::Installer { reason, top_level } => {
                Ok(InstanceAddOutcome::NeedsInstaller(NeedsInstaller {
                    item_id,
                    reason,
                    top_level,
                }))
            }
            PlacementDecision::Unknown { top_level } => Err(GamesError::new(
                PLACEMENT_UNKNOWN,
                format!(
                    "Agora cannot tell where these files go (top-level entries: {}), and this game has no rule for them.",
                    top_level.join(", ")
                ),
            )),
        }
    })
    .await
    .map_err(|_| GamesError::new(TASK_FAILED, "Adding the item to the instance did not finish."))?
}

/// Remove an item's layer from an instance. The stored item stays in the library.
#[tauri::command]
pub async fn games_instance_content_remove(
    app: tauri::AppHandle,
    instance_id: String,
    item_id: String,
) -> Result<(), GamesError> {
    let instance_id = checked_id(&instance_id)?;
    let item_id = checked_id(&item_id)?;
    tokio::task::spawn_blocking(move || -> Result<(), GamesError> {
        let ctx = crate::core_context(&app)?;
        game_deploy::remove_content(&ctx, &instance_id, &item_id).map_err(deploy_error)
    })
    .await
    .map_err(|_| {
        GamesError::new(
            TASK_FAILED,
            "Removing the item from the instance did not finish.",
        )
    })?
}

/// Turn an instance's item on or off without removing it.
#[tauri::command]
pub async fn games_instance_content_set_enabled(
    app: tauri::AppHandle,
    instance_id: String,
    item_id: String,
    enabled: bool,
) -> Result<(), GamesError> {
    let instance_id = checked_id(&instance_id)?;
    let item_id = checked_id(&item_id)?;
    tokio::task::spawn_blocking(move || -> Result<(), GamesError> {
        let ctx = crate::core_context(&app)?;
        game_deploy::set_content_enabled(&ctx, &instance_id, &item_id, enabled)
            .map_err(deploy_error)
    })
    .await
    .map_err(|_| GamesError::new(TASK_FAILED, "Changing the item did not finish."))?
}

/// Move an item to a 1-based position in the instance's load order (1 is the lowest priority).
#[tauri::command]
pub async fn games_instance_content_move(
    app: tauri::AppHandle,
    instance_id: String,
    item_id: String,
    position: usize,
) -> Result<(), GamesError> {
    let instance_id = checked_id(&instance_id)?;
    let item_id = checked_id(&item_id)?;
    let new_index = move_index(position)?;
    tokio::task::spawn_blocking(move || -> Result<(), GamesError> {
        let ctx = crate::core_context(&app)?;
        game_deploy::move_content(&ctx, &instance_id, &item_id, new_index).map_err(deploy_error)
    })
    .await
    .map_err(|_| GamesError::new(TASK_FAILED, "Moving the item did not finish."))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use agora_game_api::{LayerId, RelPath};
    use serde_json::json;

    /// A stored item as the store writes it: snake_case keys, as `ContentItem` declares them.
    fn item(id: &str, sources: serde_json::Value) -> ContentItem {
        serde_json::from_value(json!({
            "item_id": id,
            "name": format!("Item {id}"),
            "files": [{ "path": "a/readme.txt", "size": 3, "sha256": "00" }],
            "total_size": 3,
            "sources": sources,
            "added_at_unix_ms": 1000
        }))
        .expect("a content item deserializes")
    }

    fn layer(source: LayerSource, id: &str, enabled: bool) -> Layer {
        Layer {
            id: LayerId::new(id).expect("a layer id"),
            enabled,
            mount_path: RelPath::new("").expect("a root path"),
            source_path: RelPath::new("").expect("a root path"),
            source,
            whiteouts: Vec::new(),
            own_copy: false,
        }
    }

    fn content_layer(item_id: &str, enabled: bool) -> Layer {
        layer(
            LayerSource::Content {
                content: item_id.to_string(),
            },
            &format!("content:{}", &item_id[..item_id.len().min(16)]),
            enabled,
        )
    }

    fn writable_layer() -> Layer {
        layer(
            LayerSource::Writable {
                path: RelPath::new("saves").expect("a relative path"),
            },
            "writable",
            true,
        )
    }

    #[test]
    fn archive_origin_names_the_file_not_the_whole_path() {
        let stored = store_item(&item(
            "aaaa1111",
            json!([{ "kind": "archive", "path": "D:/Downloads/Mod-1.2.zip", "sha256": "ff", "added_at_unix_ms": 1 }]),
        ));
        assert_eq!(stored.source_kind, "archive");
        assert_eq!(stored.source_label, "Archive: Mod-1.2.zip");
        assert_eq!(stored.file_count, 1);
    }

    #[test]
    fn a_catalog_item_names_its_game_and_a_hand_added_one_names_none() {
        let catalog = store_item(&item(
            "dddd",
            json!([
                { "kind": "catalog", "item_id": "x", "game": "skyrim-se", "release": null, "asset": "a.7z", "sha256": "ff", "verified": true, "added_at_unix_ms": 1 },
                { "kind": "catalog", "item_id": "y", "game": "skyrim-se", "release": null, "asset": "b.7z", "sha256": "ee", "verified": true, "added_at_unix_ms": 2 },
            ]),
        ));
        assert_eq!(catalog.game_ids, vec!["skyrim-se".to_string()]);
        let by_hand = store_item(&item(
            "eeee",
            json!([{ "kind": "folder", "path": "D:/Mods/x", "added_at_unix_ms": 1 }]),
        ));
        assert!(by_hand.game_ids.is_empty());
    }

    #[test]
    fn an_unreadable_record_is_listed_as_damaged_not_dropped() {
        let entries = vec![
            ItemEntry::Readable(Box::new(item("aaaa", json!([])))),
            ItemEntry::Damaged(content_store::DamagedItem {
                item_id: "bbbb".into(),
                manifest_path: "items/bbbb.json".into(),
                error: "items/bbbb.json is not a valid item record: EOF".into(),
            }),
        ];
        let list = store_list(&entries);
        assert_eq!(list.items.len(), 1);
        assert_eq!(list.damaged.len(), 1);
        assert_eq!(list.damaged[0].item_id, "bbbb");
        assert!(list.damaged[0].message.contains("not a valid item record"));
    }

    #[test]
    fn every_known_source_kind_has_its_own_origin() {
        let fomod = store_item(&item(
            "bbbb2222cccc3333dddd",
            json!([{ "kind": "fomod_install", "from_item": "bbbb2222cccc3333dddd", "choices": [], "added_at_unix_ms": 1 }]),
        ));
        assert_eq!(fomod.source_kind, "fomod");
        assert!(fomod.source_label.contains("bbbb2222cccc"));
        assert!(
            !fomod.source_label.contains("dddd"),
            "the label shows at most 12 characters of an id"
        );

        let thunder = store_item(&item(
            "cccc",
            json!([{ "kind": "thunderstore", "from_item": "x", "package": "Owner-Pkg", "version": "1.0.0", "added_at_unix_ms": 1 }]),
        ));
        assert_eq!(thunder.source_kind, "thunderstore");
        assert_eq!(thunder.source_label, "Thunderstore package Owner-Pkg 1.0.0");

        let catalog = store_item(&item(
            "dddd",
            json!([{ "kind": "catalog", "item_id": "x", "game": "skyrim-se", "release": null, "asset": "a.7z", "sha256": "ff", "verified": false, "added_at_unix_ms": 1 }]),
        ));
        assert_eq!(catalog.source_kind, "catalog");
        assert!(catalog.source_label.contains("published no hash"));
    }

    #[test]
    fn a_non_ascii_source_id_cannot_panic_the_label() {
        // Ids are hex in practice; a record with other characters must still produce a label.
        let stored = store_item(&item(
            "ffff",
            json!([{ "kind": "fomod_install", "from_item": "é".repeat(20), "choices": [], "added_at_unix_ms": 1 }]),
        ));
        assert_eq!(stored.source_kind, "fomod");
        assert_eq!(
            stored.source_label,
            format!("Made by an installer from item {}", "é".repeat(12))
        );
    }

    #[test]
    fn an_item_with_no_recorded_source_says_so_rather_than_inventing_one() {
        let stored = store_item(&item("eeee", json!([])));
        assert_eq!(stored.source_kind, "unknown");
        assert_eq!(stored.source_label, "No source was recorded for this item.");
    }

    #[test]
    fn instance_rows_number_only_content_layers_from_one() {
        let layers = vec![
            writable_layer(),
            content_layer("aaaa1111", true),
            content_layer("bbbb2222", false),
        ];
        let items = vec![item("aaaa1111", json!([])), item("bbbb2222", json!([]))];
        let rows = instance_content_rows(&layers, &items).expect("rows build");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].position, 1);
        assert_eq!(rows[0].item_id, "aaaa1111");
        assert!(rows[0].enabled);
        assert_eq!(rows[1].position, 2);
        assert!(
            !rows[1].enabled,
            "a disabled layer is still listed, and says so"
        );
    }

    #[test]
    fn a_layer_and_store_that_disagree_are_an_error_not_a_guess() {
        let layers = vec![
            content_layer("aaaa1111", true),
            content_layer("bbbb2222", true),
        ];
        // One item is missing: the rows cannot all be named, so the list must fail.
        let short = vec![item("aaaa1111", json!([]))];
        assert!(instance_content_rows(&layers, &short).is_err());
        // The items are in the wrong order: a row would carry another layer's name.
        let swapped = vec![item("bbbb2222", json!([])), item("aaaa1111", json!([]))];
        assert!(instance_content_rows(&layers, &swapped).is_err());
    }

    #[test]
    fn move_position_is_one_based_and_zero_is_refused() {
        assert_eq!(move_index(1).expect("the lowest priority is index 0"), 0);
        assert_eq!(move_index(3).expect("position 3 is index 2"), 2);
        let refused = move_index(0).expect_err("position 0 names no place");
        assert_eq!(refused.code, INVALID_INPUT);
    }

    #[test]
    fn blank_paths_and_ids_are_refused_before_the_core_sees_them() {
        for blank in ["", "   ", "\t"] {
            assert_eq!(
                checked_source_path(blank).expect_err("blank path").code,
                INVALID_INPUT
            );
            assert_eq!(checked_id(blank).expect_err("blank id").code, INVALID_INPUT);
        }
        assert_eq!(
            checked_id("  abc123  ").expect("an id is trimmed"),
            "abc123"
        );
        assert_eq!(
            checked_source_path("D:/Mods/My Mod.zip").expect("a real path is kept as given"),
            PathBuf::from("D:/Mods/My Mod.zip")
        );
    }

    #[test]
    fn verify_reports_every_problem_kind_in_plain_words() {
        let kinds = [
            (ProblemKind::Missing, "missing"),
            (
                ProblemKind::SizeMismatch {
                    expected: 10,
                    actual: 4,
                },
                "size_mismatch",
            ),
            (
                ProblemKind::HashMismatch {
                    expected: "aa".into(),
                    actual: "bb".into(),
                },
                "hash_mismatch",
            ),
            (ProblemKind::Unprotected, "unprotected"),
            (
                ProblemKind::CorruptManifest {
                    error: "eof".into(),
                },
                "corrupt_manifest",
            ),
        ];
        for (kind, expected) in kinds {
            let problem = verify_problem(&ContentProblem {
                item_id: "aaaa".into(),
                path: "a/readme.txt".into(),
                kind,
            });
            assert_eq!(problem.kind, expected);
            assert!(!problem.message.is_empty());
        }
    }

    #[test]
    fn a_store_refusal_names_the_instances_and_is_not_a_core_failure() {
        let error = content_error(ContentError::InUse {
            item_id: "aaaa".into(),
            instances: "skyrim-a, skyrim-b".into(),
        });
        assert_eq!(error.code, IN_USE);
        assert!(error.message.contains("skyrim-a, skyrim-b"));
    }

    /// Reads two real instances from the bench (read only, nothing is written) and checks that
    /// their content rows build from the real item records. Run with
    /// `cargo test -p agora-desktop real_bench -- --ignored`.
    #[test]
    #[ignore = "reads the bench data under D: (read only)"]
    fn real_bench_instance_rows_build_from_the_store_records() {
        let root = Path::new(r"D:\Agora-bench\p3-done");
        for instance in ["p3-modded", "grounded-apocalypse-mo2-import-6d97f2"] {
            let manifest_path = root
                .join("instances")
                .join(instance)
                .join("instance_manifest.json");
            let manifest_text = std::fs::read_to_string(&manifest_path)
                .unwrap_or_else(|e| panic!("{} is readable: {e}", manifest_path.display()));
            let manifest: game_instance::GameInstanceManifest =
                serde_json::from_str(&manifest_text).expect("the real manifest deserializes");

            let mut items = Vec::new();
            for layer in manifest.layers.layers() {
                if let LayerSource::Content { content } = &layer.source {
                    let item_path = root
                        .join("content")
                        .join("items")
                        .join(format!("{content}.json"));
                    let text = std::fs::read_to_string(&item_path)
                        .unwrap_or_else(|e| panic!("{} is readable: {e}", item_path.display()));
                    items.push(
                        serde_json::from_str::<ContentItem>(&text)
                            .expect("the real item deserializes"),
                    );
                }
            }

            let rows = instance_content_rows(manifest.layers.layers(), &items)
                .expect("every content layer has its real item, in order");
            assert_eq!(rows.len(), items.len());
            for (position, row) in rows.iter().enumerate() {
                assert_eq!(row.position, position + 1);
                assert!(!row.name.is_empty(), "{instance} row {position} has a name");
                assert_ne!(
                    row.source_kind, "unknown",
                    "{instance} row {position} names its source"
                );
            }
            println!("{instance}: {} content rows", rows.len());
        }
    }

    #[test]
    fn any_other_store_error_keeps_the_core_message() {
        let error = content_error(ContentError::NotFound("ffff".into()));
        assert_eq!(error.code, CORE_FAILED);
        assert_eq!(error.message, "item not found: ffff");
    }
}
