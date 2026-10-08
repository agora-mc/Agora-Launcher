//! Plugin activation for games whose plugin list names what loads (MASTER_SPEC §26.6).
//!
//! Skyrim-family games load a plugin (`.esp`, `.esm`, `.esl`) only when the game's
//! `Plugins.txt` names it. A game definition declares that as a
//! [`PluginListRule`]; this module keeps the instance's copy of the list in step with the
//! plugins its content layers deploy. Ordering rules (masters, limits, sorting) belong to
//! [`crate::game_load_order`], which reads and writes the list through this module.
//!
//! The list is an instance's per-user file (`<instance dir>/<user_file>`), swapped in for a
//! launch session by [`crate::game_user_files`]. Agora records which plugins it managed, and
//! which the user locked, in `<instance dir>/plugin_list_state.json`, so a layer that is removed
//! or disabled takes its lines with it while a line the user wrote themselves stays.
//!
//! Every failure is an error: an unreadable or unparsable file is never read as "empty".

use std::path::{Path, PathBuf};

use agora_game_api::{
    glob_match, GameDefinition, GameId, PluginListRule, StoreId, UserFileMapping, UserFileStrategy,
};
use serde::{Deserialize, Serialize};

use crate::ctx::Ctx;
use crate::game_deploy::{DeploymentPlan, FileSource};
use crate::lock_manager::LockResource;

const STATE_FILE: &str = "plugin_list_state.json";

#[derive(Debug, thiserror::Error)]
pub enum PluginListError {
    #[error("game '{0}' does not keep a plugin list")]
    NoRule(GameId),
    #[error("instance '{0}' not found")]
    InstanceNotFound(String),
    #[error("'{0}' is not in the instance's plugin list")]
    NotInList(String),
    #[error("this game's plugin list has no inactive state (it has no active prefix), so '{0}' cannot be disabled")]
    NoInactiveState(String),
    #[error("instance '{0}' is running; close the game before changing its plugin list")]
    InstanceRunning(String),
    #[error("plugin list state file {path} cannot be read: {reason}")]
    CorruptState { path: PathBuf, reason: String },
    #[error("plugin list {path} cannot be read: {source}")]
    Unreadable {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    UserFiles(#[from] crate::game_user_files::UserFilesError),
    #[error("lock error: {0}")]
    Lock(#[from] crate::error::LauncherError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Other(String),
}

/// What changed in an instance's plugin list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginSyncReport {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub warnings: Vec<String>,
}

impl PluginSyncReport {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.warnings.is_empty()
    }
}

/// One plugin line of an instance's list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginEntry {
    pub name: String,
    pub active: bool,
    /// Whether Agora added this plugin because a content layer deploys it.
    pub managed: bool,
    /// Whether the user locked this plugin's place in the order (MASTER_SPEC §26.6). Sorting
    /// and moving never move a locked plugin.
    #[serde(default)]
    pub locked: bool,
}

/// An instance's plugin list, in load order among the listed plugins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginList {
    pub path: PathBuf,
    /// False when the instance has no copy of the list yet (it has never been deployed).
    pub exists: bool,
    pub entries: Vec<PluginEntry>,
}

/// `plugin_list_state.json`. A file written before locks existed has only `managed`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct ManagedState {
    managed: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    locked: Vec<String>,
}

// ---------------------------------------------------------------------------
// The plugin list file
// ---------------------------------------------------------------------------

const CRLF: &[u8] = b"\r\n";
const LF: &[u8] = b"\n";

/// A plugin list held as raw lines, so lines Agora does not understand (comments, other
/// encodings) are written back byte for byte.
struct ListFile {
    lines: Vec<Vec<u8>>,
    eol: &'static [u8],
}

impl ListFile {
    fn parse(bytes: &[u8]) -> Self {
        let eol = if bytes.windows(2).any(|w| w == CRLF) || !bytes.contains(&b'\n') {
            CRLF
        } else {
            LF
        };
        let mut lines: Vec<Vec<u8>> = bytes
            .split(|b| *b == b'\n')
            .map(|l| l.strip_suffix(b"\r").unwrap_or(l).to_vec())
            .collect();
        if lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        Self { lines, eol }
    }

    fn from_header(rule: &PluginListRule) -> Self {
        Self {
            lines: rule.header.iter().map(|l| l.as_bytes().to_vec()).collect(),
            eol: CRLF,
        }
    }

    fn render(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for line in &self.lines {
            out.extend_from_slice(line);
            out.extend_from_slice(self.eol);
        }
        out
    }
}

/// A line that names a plugin: its state and the name as written.
fn plugin_line<'a>(line: &'a [u8], rule: &PluginListRule) -> Option<(bool, &'a [u8])> {
    let trimmed = line.trim_ascii();
    if trimmed.is_empty() || trimmed.starts_with(b"#") {
        return None;
    }
    let prefix = rule.active_prefix.as_bytes();
    let (active, name) = if prefix.is_empty() {
        (true, trimmed)
    } else if let Some(rest) = trimmed.strip_prefix(prefix) {
        (true, rest.trim_ascii())
    } else {
        (false, trimmed)
    };
    if name.is_empty() {
        None
    } else {
        Some((active, name))
    }
}

fn key(name: &[u8]) -> Vec<u8> {
    name.to_ascii_lowercase()
}

fn render_line(rule: &PluginListRule, name: &[u8], active: bool) -> Vec<u8> {
    let mut line = Vec::new();
    if active {
        line.extend_from_slice(rule.active_prefix.as_bytes());
    }
    line.extend_from_slice(name);
    line
}

/// Bring `file` in step with the plugins `desired` (in the order new ones are appended) and
/// the plugins Agora `old_managed`. Returns what was added and removed.
fn apply_sync(
    file: &mut ListFile,
    rule: &PluginListRule,
    desired: &[String],
    old_managed: &[String],
) -> (Vec<String>, Vec<String>) {
    let desired_keys: std::collections::HashSet<Vec<u8>> =
        desired.iter().map(|n| key(n.as_bytes())).collect();
    let old_keys: std::collections::HashSet<Vec<u8>> =
        old_managed.iter().map(|n| key(n.as_bytes())).collect();

    // A line for a plugin Agora used to deploy and no longer does goes away.
    let mut removed = Vec::new();
    file.lines.retain(|line| {
        let Some((_, name)) = plugin_line(line, rule) else {
            return true;
        };
        let k = key(name);
        if old_keys.contains(&k) && !desired_keys.contains(&k) {
            removed.push(String::from_utf8_lossy(name).into_owned());
            false
        } else {
            true
        }
    });

    // A plugin entering the managed set for the first time is made active even if a line for
    // it exists: Skyrim itself appends plugins it finds in Data to the list, inactive, so an
    // inactive line Agora never managed is the game's, not the user's choice. Once managed, a
    // line's state is the user's and is kept.
    let mut added = Vec::new();
    for line in file.lines.iter_mut() {
        let Some((active, name)) = plugin_line(line, rule) else {
            continue;
        };
        let k = key(name);
        if !active && desired_keys.contains(&k) && !old_keys.contains(&k) {
            let name = name.to_vec();
            *line = render_line(rule, &name, true);
            added.push(String::from_utf8_lossy(&name).into_owned());
        }
    }

    // A deployed plugin the list does not name yet is appended, active.
    let mut present: std::collections::HashSet<Vec<u8>> = file
        .lines
        .iter()
        .filter_map(|l| plugin_line(l, rule).map(|(_, n)| key(n)))
        .collect();
    for name in desired {
        if present.insert(key(name.as_bytes())) {
            file.lines.push(render_line(rule, name.as_bytes(), true));
            added.push(name.clone());
        }
    }
    (added, removed)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn instance_dir(ctx: &Ctx, instance_id: &str) -> Result<PathBuf, PluginListError> {
    ctx.paths
        .instance_dir(instance_id)
        .map_err(|e| PluginListError::Other(e.to_string()))
}

/// Read a file that may not exist. Any failure other than "not found" is an error.
fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, PluginListError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(PluginListError::Unreadable {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), PluginListError> {
    let Some(parent) = path.parent() else {
        return Err(PluginListError::Other(format!(
            "{} has no parent folder",
            path.display()
        )));
    };
    std::fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = parent.join(format!("{file_name}.{}.tmp", uuid::Uuid::new_v4().simple()));
    let result = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    Ok(result?)
}

fn read_state(instance_dir: &Path) -> Result<ManagedState, PluginListError> {
    let path = instance_dir.join(STATE_FILE);
    match read_optional(&path)? {
        None => Ok(ManagedState::default()),
        Some(bytes) => serde_json::from_slice(&bytes).map_err(|e| PluginListError::CorruptState {
            path,
            reason: e.to_string(),
        }),
    }
}

fn write_state(instance_dir: &Path, state: &ManagedState) -> Result<(), PluginListError> {
    write_atomic(
        &instance_dir.join(STATE_FILE),
        &serde_json::to_vec_pretty(state)?,
    )
}

fn user_file_mapping<'a>(
    definition: &'a GameDefinition,
    rule: &PluginListRule,
    store: &StoreId,
) -> Option<&'a UserFileMapping> {
    definition.user_files.iter().find(|m| {
        m.strategy == UserFileStrategy::JournaledSwap
            && m.instance_path == rule.user_file
            && m.applies_to_store(store)
    })
}

/// A session left behind by a finished or crashed run still holds this game's real list.
/// Put it back before reading the real file or editing an instance copy, exactly as the next
/// swap would. A session that is still running is left alone.
fn recover_stale_session(
    ctx: &Ctx,
    definition: &GameDefinition,
    store: &StoreId,
) -> Result<(), PluginListError> {
    use crate::game_user_files::UserFilesError;
    if !ctx
        .paths
        .user_files_journal_path(definition.id.as_str(), store.as_str())
        .exists()
    {
        return Ok(());
    }
    match crate::game_user_files::restore(ctx, definition, store) {
        Ok(_)
        | Err(UserFilesError::CannotRestoreRunning { .. })
        | Err(UserFilesError::NoSession { .. }) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// The contents a new instance copy starts from: the user's real list when it is safe to read
/// it, else the rule's header.
fn starting_contents(
    ctx: &Ctx,
    definition: &GameDefinition,
    store: &StoreId,
    mapping: &UserFileMapping,
    rule: &PluginListRule,
    report: &mut PluginSyncReport,
) -> Result<ListFile, PluginListError> {
    let journal = ctx
        .paths
        .user_files_journal_path(definition.id.as_str(), store.as_str());
    if journal.exists() {
        // A session is running: the real file holds that session's list, not the user's.
        report.warnings.push(
            "another session of this game is running, so the plugin list starts from the \
             game's header instead of your current list"
                .to_string(),
        );
        return Ok(ListFile::from_header(rule));
    }
    let real = crate::game_user_files::resolve_user_file_source(&mapping.source)?;
    Ok(match read_optional(&real)? {
        Some(bytes) => ListFile::parse(&bytes),
        None => ListFile::from_header(rule),
    })
}

fn same_names(a: &[String], b: &[String]) -> bool {
    let norm = |v: &[String]| {
        let mut k: Vec<Vec<u8>> = v.iter().map(|n| key(n.as_bytes())).collect();
        k.sort();
        k.dedup();
        k
    };
    norm(a) == norm(b)
}

// ---------------------------------------------------------------------------
// Which plugins a deployment provides
// ---------------------------------------------------------------------------

/// The plugins a deployment gets from content layers, in the order new ones are appended
/// to the list.
///
/// Only plugins directly inside `plugin_folder` that match one of the rule's patterns count,
/// and only when a content layer wins the path: plugins from the base, from the writable
/// layer, or that the base also has (the game's own masters) are never managed. Order is the
/// layer's position (lowest first), then the position of the first matching pattern (so a
/// definition lists its patterns in load priority: masters before plain plugins), then name.
pub fn desired_plugins<'a>(
    rule: &PluginListRule,
    content_layer_items: &[String],
    plan: &DeploymentPlan,
    base_paths: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    let base_keys: std::collections::HashSet<String> = base_paths
        .into_iter()
        .map(|p| p.to_ascii_lowercase())
        .collect();
    let folder = rule.plugin_folder.as_str().trim_matches('/');
    let patterns: Vec<String> = rule
        .patterns
        .iter()
        .map(|p| p.trim().to_ascii_lowercase())
        .collect();

    let mut found: Vec<(usize, usize, String, String)> = Vec::new();
    for file in &plan.files {
        let FileSource::Content { item_id, .. } = &file.source else {
            continue;
        };
        let path = file.path.as_str();
        let (parent, name) = match path.rsplit_once('/') {
            Some((parent, name)) => (parent, name),
            None => ("", path),
        };
        if !parent.eq_ignore_ascii_case(folder) {
            continue;
        }
        let lower = name.to_ascii_lowercase();
        let Some(pattern_rank) = patterns
            .iter()
            .position(|p| !p.is_empty() && glob_match(p, &lower))
        else {
            continue;
        };
        if base_keys.contains(&path.to_ascii_lowercase()) {
            continue;
        }
        let Some(layer_rank) = content_layer_items.iter().position(|i| i == item_id) else {
            continue;
        };
        found.push((layer_rank, pattern_rank, lower, name.to_string()));
    }
    found.sort();
    found.into_iter().map(|(_, _, _, name)| name).collect()
}

// ---------------------------------------------------------------------------
// Sync
// ---------------------------------------------------------------------------

/// Bring an instance's plugin list in step with the plugins its deployment provides.
///
/// The caller holds the instance lock. `None` when the game keeps no plugin list.
pub fn sync_locked(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    store: &StoreId,
    desired: &[String],
) -> Result<Option<PluginSyncReport>, PluginListError> {
    let Some(rule) = &definition.plugin_list else {
        return Ok(None);
    };
    let dir = instance_dir(ctx, instance_id)?;
    let copy_path = dir.join(rule.user_file.as_str());
    let old = read_state(&dir)?;
    let mut report = PluginSyncReport::default();

    let Some(mapping) = user_file_mapping(definition, rule, store) else {
        if !desired.is_empty() || !old.managed.is_empty() {
            report.warnings.push(format!(
                "the plugin list is not swapped in for the '{store}' store, so its plugins \
                 will not be activated"
            ));
        }
        return Ok(Some(report));
    };

    let existing = read_optional(&copy_path)?;
    if existing.is_none() && desired.is_empty() && old.managed.is_empty() {
        return Ok(Some(report));
    }
    recover_stale_session(ctx, definition, store)?;
    // Recovery rewrites the instance copy from the session's file.
    let existing = read_optional(&copy_path)?;
    let have_copy = existing.is_some();
    let mut file = match existing {
        Some(bytes) => ListFile::parse(&bytes),
        None => starting_contents(ctx, definition, store, mapping, rule, &mut report)?,
    };

    let (added, removed) = apply_sync(&mut file, rule, desired, &old.managed);
    let changed = !added.is_empty() || !removed.is_empty();
    // The list goes first: a state file that is behind only delays a removal, one that is
    // ahead would forget lines Agora still owns.
    if changed || (!have_copy && !desired.is_empty()) {
        write_atomic(&copy_path, &file.render())?;
    }
    if !same_names(&old.managed, desired) {
        write_state(
            &dir,
            &ManagedState {
                managed: desired.to_vec(),
                locked: old.locked,
            },
        )?;
    }
    report.added = added;
    report.removed = removed;
    Ok(Some(report))
}

/// [`sync_locked`] for a launch of an instance that deploys no content: layers removed since
/// the last deploy still take their lines with them.
pub fn sync_without_content(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    store: &StoreId,
) -> Result<Option<PluginSyncReport>, PluginListError> {
    if definition.plugin_list.is_none() {
        return Ok(None);
    }
    let _lock = ctx.lock_manager.acquire(
        LockResource::Instance(instance_id.to_string()),
        "plugins-sync",
    )?;
    sync_locked(ctx, instance_id, definition, store, &[])
}

// ---------------------------------------------------------------------------
// Inspect and toggle
// ---------------------------------------------------------------------------

fn rule_of(definition: &GameDefinition) -> Result<&PluginListRule, PluginListError> {
    definition
        .plugin_list
        .as_ref()
        .ok_or_else(|| PluginListError::NoRule(definition.id.clone()))
}

/// The instance's plugin list with each line's state and whether Agora manages it.
pub fn list(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
) -> Result<PluginList, PluginListError> {
    let rule = rule_of(definition)?;
    crate::game_instance::get_manifest(ctx, instance_id).map_err(|e| match e {
        crate::game_instance::InstanceError::NotFound(id) => PluginListError::InstanceNotFound(id),
        other => PluginListError::Other(other.to_string()),
    })?;
    let dir = instance_dir(ctx, instance_id)?;
    let path = dir.join(rule.user_file.as_str());
    let state = read_state(&dir)?;
    let managed: std::collections::HashSet<Vec<u8>> =
        state.managed.iter().map(|n| key(n.as_bytes())).collect();
    let locked: std::collections::HashSet<Vec<u8>> =
        state.locked.iter().map(|n| key(n.as_bytes())).collect();
    let Some(bytes) = read_optional(&path)? else {
        return Ok(PluginList {
            path,
            exists: false,
            entries: Vec::new(),
        });
    };
    let file = ListFile::parse(&bytes);
    let entries = file
        .lines
        .iter()
        .filter_map(|line| {
            plugin_line(line, rule).map(|(active, name)| PluginEntry {
                name: String::from_utf8_lossy(name).into_owned(),
                active,
                managed: managed.contains(&key(name)),
                locked: locked.contains(&key(name)),
            })
        })
        .collect();
    Ok(PluginList {
        path,
        exists: true,
        entries,
    })
}

/// The store an instance runs under, when it can be told.
pub fn instance_store(ctx: &Ctx, instance_id: &str) -> Result<Option<StoreId>, PluginListError> {
    let manifest = crate::game_instance::get_manifest(ctx, instance_id).map_err(|e| match e {
        crate::game_instance::InstanceError::NotFound(id) => PluginListError::InstanceNotFound(id),
        other => PluginListError::Other(other.to_string()),
    })?;
    match &manifest.base {
        agora_game_api::BaseReference::Pinned { id, .. } => {
            let path = ctx.paths.base_manifest_path(id);
            let text =
                std::fs::read_to_string(&path).map_err(|source| PluginListError::Unreadable {
                    path: path.clone(),
                    source,
                })?;
            let base: crate::game_base::BaseManifest = serde_json::from_str(&text)?;
            Ok(Some(base.runtime.store))
        }
        agora_game_api::BaseReference::Unpinned { .. } => {
            Ok(manifest.runtime_identity.map(|r| r.store))
        }
    }
}

/// Activate or deactivate one plugin in an instance's list. Returns whether the line changed.
pub fn set_active(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    name: &str,
    active: bool,
) -> Result<bool, PluginListError> {
    let rule = rule_of(definition)?;
    if !active && rule.active_prefix.is_empty() {
        return Err(PluginListError::NoInactiveState(name.to_string()));
    }
    let _lock = ctx.lock_manager.acquire(
        LockResource::Instance(instance_id.to_string()),
        "plugins-toggle",
    )?;
    ensure_copy_editable(ctx, instance_id, definition)?;

    let dir = instance_dir(ctx, instance_id)?;
    let path = dir.join(rule.user_file.as_str());
    let Some(bytes) = read_optional(&path)? else {
        return Err(PluginListError::NotInList(name.to_string()));
    };
    let mut file = ListFile::parse(&bytes);
    let wanted = key(name.trim().as_bytes());
    let mut found = false;
    let mut changed = false;
    for line in &mut file.lines {
        let Some((is_active, line_name)) = plugin_line(line, rule) else {
            continue;
        };
        if key(line_name) != wanted {
            continue;
        }
        found = true;
        if is_active != active {
            let rewritten = render_line(rule, line_name, active);
            *line = rewritten;
            changed = true;
        }
    }
    if !found {
        return Err(PluginListError::NotInList(name.to_string()));
    }
    if changed {
        write_atomic(&path, &file.render())?;
    }
    Ok(changed)
}

/// Refuse to edit an instance's copy of the list while its game runs, after putting back any
/// session a finished run left behind. The caller holds the instance lock.
fn ensure_copy_editable(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
) -> Result<(), PluginListError> {
    if let Some(store) = instance_store(ctx, instance_id)? {
        recover_stale_session(ctx, definition, &store)?;
        // A running session of this instance rewrites its copy of the list when it ends.
        if let Some(status) = crate::game_user_files::status(ctx, &definition.id, &store) {
            if status.running && status.instance == instance_id {
                return Err(PluginListError::InstanceRunning(instance_id.to_string()));
            }
        }
    }
    Ok(())
}

/// Lock or unlock a listed plugin's place in the order. Returns whether the state changed.
/// Only the state file is written, so a running game is not a reason to refuse.
pub fn set_locked(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    name: &str,
    locked: bool,
) -> Result<bool, PluginListError> {
    rule_of(definition)?;
    let _lock = ctx.lock_manager.acquire(
        LockResource::Instance(instance_id.to_string()),
        "plugins-lock",
    )?;
    let wanted = key(name.trim().as_bytes());
    let listed = list(ctx, instance_id, definition)?;
    let Some(entry) = listed
        .entries
        .iter()
        .find(|e| key(e.name.as_bytes()) == wanted)
    else {
        return Err(PluginListError::NotInList(name.to_string()));
    };
    let canonical = entry.name.clone();
    let dir = instance_dir(ctx, instance_id)?;
    let mut state = read_state(&dir)?;
    let is_locked = state.locked.iter().any(|n| key(n.as_bytes()) == wanted);
    if is_locked == locked {
        return Ok(false);
    }
    if locked {
        state.locked.push(canonical);
    } else {
        state.locked.retain(|n| key(n.as_bytes()) != wanted);
    }
    write_state(&dir, &state)?;
    Ok(true)
}

/// Rewrite the order of an instance's listed plugins. `order` names every plugin line the caller
/// reorders, in its new order, with each line's active state. A line keeps its own bytes; the
/// lines `order` does not name (comments, lines for always-loaded plugins) keep their places.
/// Returns whether the file changed. Refused while the game runs. The caller holds the instance
/// lock.
pub fn write_order_locked(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    order: &[(String, bool)],
) -> Result<bool, PluginListError> {
    let rule = rule_of(definition)?;
    ensure_copy_editable(ctx, instance_id, definition)?;
    let dir = instance_dir(ctx, instance_id)?;
    let path = dir.join(rule.user_file.as_str());
    let Some(bytes) = read_optional(&path)? else {
        if order.is_empty() {
            return Ok(false);
        }
        return Err(PluginListError::Other(format!(
            "instance '{instance_id}' has no plugin list to reorder"
        )));
    };
    let mut file = ListFile::parse(&bytes);
    let before = file.lines.clone();

    // Names in the order are lossy strings from `list`, so both sides use the lossy key.
    let lossy_key = |raw: &[u8]| key(String::from_utf8_lossy(raw).as_bytes());
    let wanted: std::collections::HashSet<Vec<u8>> =
        order.iter().map(|(name, _)| key(name.as_bytes())).collect();
    let mut slots = Vec::new();
    let mut originals: std::collections::HashMap<Vec<u8>, std::collections::VecDeque<Vec<u8>>> =
        std::collections::HashMap::new();
    for (index, line) in file.lines.iter().enumerate() {
        let Some((_, name)) = plugin_line(line, rule) else {
            continue;
        };
        let k = lossy_key(name);
        originals
            .entry(k.clone())
            .or_default()
            .push_back(name.to_vec());
        if wanted.contains(&k) {
            slots.push(index);
        }
    }
    if slots.len() != order.len() {
        return Err(PluginListError::Other(format!(
            "the plugin list of instance '{instance_id}' changed while it was being reordered; nothing was written"
        )));
    }
    for (slot, (name, active)) in slots.into_iter().zip(order) {
        let k = key(name.as_bytes());
        let original = originals
            .get_mut(&k)
            .and_then(std::collections::VecDeque::pop_front)
            .ok_or_else(|| {
                PluginListError::Other(format!(
                    "the plugin list of instance '{instance_id}' changed while it was being reordered; nothing was written"
                ))
            })?;
        file.lines[slot] = render_line(rule, &original, *active);
    }
    if file.lines == before {
        return Ok(false);
    }
    write_atomic(&path, &file.render())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agora_game_api::RelPath;

    fn rule() -> PluginListRule {
        PluginListRule {
            user_file: RelPath::new("user/Plugins.txt").unwrap(),
            plugin_folder: RelPath::new("Data").unwrap(),
            patterns: vec!["*.esm".into(), "*.esl".into(), "*.esp".into()],
            active_prefix: "*".into(),
            header: vec!["# header".into()],
            semantics: None,
            implicit: Vec::new(),
            implicit_list_file: None,
        }
    }

    fn lines(file: &ListFile) -> Vec<String> {
        file.lines
            .iter()
            .map(|l| String::from_utf8_lossy(l).into_owned())
            .collect()
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_keeps_crlf_and_drops_the_trailing_blank() {
        let f = ListFile::parse(b"# c\r\n*A.esp\r\nB.esp\r\n");
        assert_eq!(lines(&f), ["# c", "*A.esp", "B.esp"]);
        assert_eq!(f.render(), b"# c\r\n*A.esp\r\nB.esp\r\n");
    }

    #[test]
    fn parse_empty_input_is_an_empty_list_that_renders_to_nothing() {
        let f = ListFile::parse(b"");
        assert!(f.lines.is_empty());
        assert!(f.render().is_empty());
    }

    #[test]
    fn lf_files_stay_lf() {
        let f = ListFile::parse(b"*A.esp\n");
        assert_eq!(f.render(), b"*A.esp\n");
    }

    #[test]
    fn plugin_lines_tell_active_inactive_and_noise_apart() {
        let r = rule();
        assert_eq!(plugin_line(b"*A.esp", &r), Some((true, &b"A.esp"[..])));
        assert_eq!(plugin_line(b"A.esp", &r), Some((false, &b"A.esp"[..])));
        assert_eq!(
            plugin_line(b"  *  A.esp  ", &r),
            Some((true, &b"A.esp"[..]))
        );
        assert_eq!(plugin_line(b"# *A.esp", &r), None);
        assert_eq!(plugin_line(b"", &r), None);
        assert_eq!(plugin_line(b"   ", &r), None);
        assert_eq!(plugin_line(b"*", &r), None);
    }

    #[test]
    fn sync_appends_new_plugins_active_after_existing_lines() {
        let mut f = ListFile::parse(b"# c\r\n*Old.esp\r\n");
        let (added, removed) = apply_sync(&mut f, &rule(), &names(&["B.esm", "A.esp"]), &[]);
        assert_eq!(added, ["B.esm", "A.esp"]);
        assert!(removed.is_empty());
        assert_eq!(lines(&f), ["# c", "*Old.esp", "*B.esm", "*A.esp"]);
    }

    #[test]
    fn a_line_the_game_added_inactive_is_activated_when_agora_starts_managing_it() {
        // Skyrim appends plugins it finds in Data, inactive, before Agora has managed them.
        let r = rule();
        let mut f = ListFile::parse(b"# header\r\nCoolMod.esp\r\n");
        let (added, removed) = apply_sync(&mut f, &r, &["CoolMod.esp".to_string()], &[]);
        assert_eq!(added, vec!["CoolMod.esp".to_string()]);
        assert!(removed.is_empty());
        assert_eq!(f.render(), b"# header\r\n*CoolMod.esp\r\n".to_vec());
        // Once managed, the user's later choice to deactivate it is kept.
        let mut f = ListFile::parse(b"CoolMod.esp\r\n");
        let managed = ["CoolMod.esp".to_string()];
        let (added, _) = apply_sync(&mut f, &r, &managed, &managed);
        assert!(added.is_empty());
        assert_eq!(f.render(), b"CoolMod.esp\r\n".to_vec());
    }

    #[test]
    fn sync_keeps_an_inactive_line_inactive_and_in_place() {
        let mut f = ListFile::parse(b"*X.esp\r\nA.esp\r\n*Y.esp\r\n");
        let (added, _) = apply_sync(&mut f, &rule(), &names(&["A.esp"]), &names(&["A.esp"]));
        assert!(added.is_empty());
        assert_eq!(lines(&f), ["*X.esp", "A.esp", "*Y.esp"]);
    }

    #[test]
    fn sync_matches_names_case_insensitively_and_keeps_one_line() {
        let mut f = ListFile::parse(b"*a.ESP\r\n");
        let (added, removed) = apply_sync(&mut f, &rule(), &names(&["A.esp"]), &names(&["A.esp"]));
        assert!(added.is_empty() && removed.is_empty());
        assert_eq!(lines(&f), ["*a.ESP"]);
    }

    #[test]
    fn sync_removes_only_lines_agora_managed() {
        let mut f = ListFile::parse(b"*Gone.esp\r\n*Mine.esp\r\n# note\r\n*Kept.esp\r\n");
        let (added, removed) = apply_sync(
            &mut f,
            &rule(),
            &names(&["Kept.esp"]),
            &names(&["gone.ESP", "Kept.esp"]),
        );
        assert!(added.is_empty());
        assert_eq!(removed, ["Gone.esp"]);
        assert_eq!(lines(&f), ["*Mine.esp", "# note", "*Kept.esp"]);
    }

    #[test]
    fn sync_with_nothing_desired_and_nothing_managed_changes_nothing() {
        let mut f = ListFile::parse(b"*A.esp\r\n");
        let (added, removed) = apply_sync(&mut f, &rule(), &[], &[]);
        assert!(added.is_empty() && removed.is_empty());
        assert_eq!(lines(&f), ["*A.esp"]);
    }

    #[test]
    fn non_utf8_lines_survive_byte_for_byte() {
        let raw = b"*Caf\xe9.esp\r\n";
        let mut f = ListFile::parse(raw);
        apply_sync(&mut f, &rule(), &names(&["New.esp"]), &[]);
        assert!(f.render().starts_with(raw));
    }

    #[test]
    fn a_rule_without_a_prefix_treats_every_line_as_active() {
        let mut r = rule();
        r.active_prefix = String::new();
        assert_eq!(plugin_line(b"A.esp", &r), Some((true, &b"A.esp"[..])));
        let mut f = ListFile::parse(b"A.esp\r\n");
        apply_sync(&mut f, &r, &names(&["B.esp"]), &[]);
        assert_eq!(lines(&f), ["A.esp", "B.esp"]);
    }
}
