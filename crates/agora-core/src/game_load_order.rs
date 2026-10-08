//! The Creation Engine load order (MASTER_SPEC §26.6): the order the game loads an instance's
//! plugins in, the findings the engine's rules raise against that order, and the sort, move and
//! lock operations that change it.
//!
//! Core owns the ordered list, its locks and its writes ([`crate::game_plugins`]). The meaning
//! belongs to the game: a [`PluginListRule`] with `semantics = "creation_engine"` turns on the
//! master, light and limit rules here. Plugin headers come from LOOT's `esplugin` crate, read from
//! the files the game will see: the deployment plan's source for each plugin in the plugin folder.
//!
//! The order is the rule's `implicit` plugins (and those its `implicit_list_file` names) that are
//! present, then the instance's own list in its order. Implicit plugins are never written into
//! the list. Locked and implicit plugins keep their positions when others are sorted or moved:
//! free plugins trade places only among the positions that no locked or implicit plugin holds.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use agora_game_api::{glob_match, GameDefinition, PluginListRule};
use serde::{Deserialize, Serialize};

use crate::ctx::Ctx;
use crate::game_deploy::{self, DeployMode};
use crate::game_plugins::{self, PluginEntry, PluginListError};
use crate::lock_manager::LockResource;

/// The `semantics` value that turns the Creation Engine rules on.
pub const CREATION_ENGINE: &str = "creation_engine";
/// The engine loads at most this many full (non-light) plugins, implicit ones included.
pub const FULL_PLUGIN_LIMIT: usize = 254;
/// The engine loads at most this many light plugins (`.esl` or the light flag).
pub const LIGHT_PLUGIN_LIMIT: usize = 4096;

#[derive(Debug, thiserror::Error)]
pub enum LoadOrderError {
    #[error(transparent)]
    Plugins(#[from] PluginListError),
    #[error("game '{0}' declares no load order rules, so its plugins cannot be sorted")]
    NoRules(String),
    #[error("cannot read the game's files for instance '{instance_id}': {reason}")]
    Files { instance_id: String, reason: String },
    #[error("cannot read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("'{0}' is not in the instance's plugin list")]
    NotListed(String),
    #[error("'{0}' is always loaded first, so it cannot be moved or placed against")]
    AlwaysLoaded(String),
    #[error("'{0}' is locked; unlock it before moving it")]
    Locked(String),
    #[error("'{0}' cannot be placed relative to itself")]
    SelfTarget(String),
    #[error("position {position} is outside the list, which has {last} positions")]
    BadPosition { position: usize, last: usize },
    #[error("position {position} is held by '{holder}', which does not move, so '{plugin}' cannot go there")]
    Held {
        plugin: String,
        position: usize,
        holder: String,
    },
    #[error("'{plugin}' cannot go before or after '{target}': the plugins that do not move leave no free place there")]
    NoRoom { plugin: String, target: String },
    #[error("sort did not settle after {0} moves; nothing was written")]
    DidNotSettle(usize),
}

// ---------------------------------------------------------------------------
// Data
// ---------------------------------------------------------------------------

/// One plugin the game loads, in load order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadOrderEntry {
    /// The plugin's name as the list or the game's own rule writes it.
    pub name: String,
    /// Always loaded first, from the game's rule. Never written into the instance's list.
    pub implicit: bool,
    pub active: bool,
    /// Whether the plugin's file is in the plugin folder the game sees.
    pub present: bool,
    /// Whether Agora added this plugin because a content layer deploys it.
    pub managed: bool,
    /// Whether the user locked this plugin's position in the order.
    pub locked: bool,
    /// Whether the header was read. False for games without creation rules, for absent plugins,
    /// and for headers that failed (see `header_error`).
    pub header_read: bool,
    /// The master flag (`.esm`, or ESM-flagged). Meaningful only when `header_read`.
    pub master: bool,
    /// The light flag (`.esl`, or ESL-flagged). Meaningful only when `header_read`.
    pub light: bool,
    /// The masters the header names, in the header's order.
    pub masters: Vec<String>,
    /// Why the header could not be read. Such a plugin is a finding.
    pub header_error: Option<String>,
}

/// An instance's load order and what the game's rules say about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadOrder {
    /// Whether the Creation Engine rules apply to this game.
    pub engine: bool,
    /// Whether the instance has a copy of the plugin list yet.
    pub exists: bool,
    pub entries: Vec<LoadOrderEntry>,
    pub findings: Vec<Finding>,
}

/// A problem with the order that the game's rules name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Finding {
    /// An active plugin needs a master that is missing, inactive, or loads after it.
    MasterNotEarlier {
        plugin: String,
        master: String,
        problem: MasterProblem,
    },
    /// Plugins whose masters lead back to one another. The engine cannot order them.
    MasterCycle { plugins: Vec<String> },
    /// A plugin's header cannot be parsed, so its masters and flags are unknown.
    UnreadableHeader { plugin: String, reason: String },
    /// The same plugin is listed more than once.
    DuplicateListing { plugin: String },
    /// More full plugins are active than the engine loads.
    TooManyFullPlugins { active: usize, limit: usize },
    /// More light plugins are active than the engine loads.
    TooManyLightPlugins { active: usize, limit: usize },
}

/// Why a master does not come before its plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MasterProblem {
    /// Not in the list, or its file is not installed.
    Missing,
    /// Listed, but inactive.
    Inactive,
    /// Active, but loads after the plugin that needs it.
    Later,
}

impl Finding {
    /// The finding as one sentence for a person.
    pub fn message(&self) -> String {
        match self {
            Finding::MasterNotEarlier {
                plugin,
                master,
                problem,
            } => match problem {
                MasterProblem::Missing => format!(
                    "'{plugin}' needs '{master}', which is missing from the list or not installed"
                ),
                MasterProblem::Inactive => {
                    format!("'{plugin}' needs '{master}', which is inactive")
                }
                MasterProblem::Later => format!("'{plugin}' loads before its master '{master}'"),
            },
            Finding::MasterCycle { plugins } => format!(
                "these plugins need one another in a loop: {}",
                plugins.join(", ")
            ),
            Finding::UnreadableHeader { plugin, reason } => {
                format!("'{plugin}' has a header that cannot be read: {reason}")
            }
            Finding::DuplicateListing { plugin } => format!("'{plugin}' is listed more than once"),
            Finding::TooManyFullPlugins { active, limit } => {
                format!("{active} full plugins are active; the game loads at most {limit}")
            }
            Finding::TooManyLightPlugins { active, limit } => {
                format!("{active} light plugins are active; the game loads at most {limit}")
            }
        }
    }
}

/// One move a sort made. Positions are 1-based positions in the list, as shown to the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SortMove {
    pub plugin: String,
    pub from: usize,
    pub to: usize,
}

/// A plugin that loads above a master the sort may not move, so the sort could not fix that
/// master finding. The plugin would have to move, and it is locked or always loaded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Blocked {
    pub plugin: String,
    pub master: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SortReport {
    pub moves: Vec<SortMove>,
    pub blocked: Vec<Blocked>,
    /// Whether the moves were written to the instance's list (false for a dry run).
    pub written: bool,
}

/// Where `move` puts a plugin. Positions are 1-based and count the always-loaded plugins too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveTarget {
    Position(usize),
    Before(String),
    After(String),
}

/// The result of moving one plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveReport {
    pub plugin: String,
    pub from: usize,
    pub to: usize,
    pub written: bool,
}

/// The plugin folder as the game sees it: lowercase file name to the file that holds its bytes.
pub type Installed = BTreeMap<String, PathBuf>;

/// The header facts the rules use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginHeader {
    pub masters: Vec<String>,
    pub master: bool,
    pub light: bool,
}

// ---------------------------------------------------------------------------
// Headers
// ---------------------------------------------------------------------------

/// Read a plugin's header with LOOT's `esplugin`, header only. Creation Engine plugins share one
/// header layout, so the Skyrim SE format is read for all of them. Any failure is returned as
/// text, never a panic and never an empty header.
pub fn read_header(path: &Path) -> Result<PluginHeader, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut plugin = esplugin::Plugin::new(esplugin::GameId::SkyrimSE, path);
    plugin
        .parse_reader(file, esplugin::ParseOptions::header_only())
        .map_err(|e| e.to_string())?;
    let masters = plugin.masters().map_err(|e| e.to_string())?;
    Ok(PluginHeader {
        masters,
        master: plugin.is_master_file(),
        light: plugin.is_light_plugin(),
    })
}

// ---------------------------------------------------------------------------
// Building the order
// ---------------------------------------------------------------------------

fn key(name: &str) -> String {
    name.to_ascii_lowercase()
}

/// Whether the game declares the Creation Engine rules.
pub fn is_engine(rule: &PluginListRule) -> bool {
    rule.semantics.as_deref() == Some(CREATION_ENGINE)
}

/// The plugin names in a game's `implicit_list_file`, one per line, in the file's order.
pub fn parse_implicit_list(text: &str) -> Vec<String> {
    text.trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Build the effective order from the instance's list and the plugin folder the game sees.
///
/// `installed` is `None` when the game's files are not known (a game without creation rules); then
/// every plugin is taken as present and no header is read. `implicit_names` are the names read
/// from the rule's `implicit_list_file`. A line in the list that names an always-loaded plugin is
/// dropped from the order: the game loads that plugin first regardless.
pub fn build_order(
    rule: &PluginListRule,
    listed: &[PluginEntry],
    installed: Option<&Installed>,
    implicit_names: &[String],
    exists: bool,
) -> LoadOrder {
    let engine = is_engine(rule);
    let mut implicit_keys = HashSet::new();
    let mut implicit = Vec::new();
    for name in rule.implicit.iter().chain(implicit_names) {
        if implicit_keys.insert(key(name)) {
            implicit.push(name.clone());
        }
    }

    let mut headers: HashMap<String, Result<PluginHeader, String>> = HashMap::new();
    let mut entries = Vec::new();
    let mut make = |name: &str, is_implicit: bool, active: bool, managed: bool, locked: bool| {
        let k = key(name);
        let file: Option<&PathBuf> = installed.and_then(|map| map.get(&k));
        let present = installed.is_none() || file.is_some();
        if !present && is_implicit {
            return None;
        }
        let mut entry = LoadOrderEntry {
            name: name.to_string(),
            implicit: is_implicit,
            active: active || is_implicit,
            present,
            managed,
            locked,
            header_read: false,
            master: false,
            light: false,
            masters: Vec::new(),
            header_error: None,
        };
        if let (true, Some(path)) = (engine && present, file) {
            let read = headers
                .entry(k)
                .or_insert_with(|| read_header(path))
                .clone();
            match read {
                Ok(header) => {
                    entry.header_read = true;
                    entry.master = header.master;
                    entry.light = header.light;
                    entry.masters = header.masters;
                }
                Err(reason) => entry.header_error = Some(reason),
            }
        }
        Some(entry)
    };

    for name in &implicit {
        if let Some(entry) = make(name, true, true, false, false) {
            entries.push(entry);
        }
    }
    for item in listed {
        if implicit_keys.contains(&key(&item.name)) {
            continue;
        }
        if let Some(entry) = make(&item.name, false, item.active, item.managed, item.locked) {
            entries.push(entry);
        }
    }

    let findings = findings(&entries);
    LoadOrder {
        engine,
        exists,
        entries,
        findings,
    }
}

// ---------------------------------------------------------------------------
// Findings
// ---------------------------------------------------------------------------

/// Whether the entry loads: active, and its file present.
fn loads(entry: &LoadOrderEntry) -> bool {
    entry.active && entry.present
}

/// Whether the entry's masters and flags are known, so the rules apply to it.
fn checked(entry: &LoadOrderEntry) -> bool {
    loads(entry) && entry.header_read
}

/// Whether some loading plugin named `master` comes before position `at`.
fn master_loads_before(entries: &[LoadOrderEntry], at: usize, master: &str) -> bool {
    entries[..at]
        .iter()
        .any(|e| loads(e) && key(&e.name) == master)
}

/// Why `master` does not come before the plugin at `at`.
fn master_problem(entries: &[LoadOrderEntry], at: usize, master: &str) -> MasterProblem {
    let mut inactive = false;
    for (index, e) in entries.iter().enumerate() {
        if key(&e.name) != master {
            continue;
        }
        if loads(e) && index > at {
            return MasterProblem::Later;
        }
        if !loads(e) && e.present {
            inactive = true;
        }
    }
    if inactive {
        MasterProblem::Inactive
    } else {
        MasterProblem::Missing
    }
}

/// The groups of loading plugins that reach one another through their masters, as indices into
/// `entries`. Only groups that are a loop are returned: a plugin in one, or one that needs one.
fn cycle_groups(entries: &[LoadOrderEntry]) -> Vec<Vec<usize>> {
    let n = entries.len();
    let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, e) in entries.iter().enumerate() {
        if loads(e) {
            by_name.entry(key(&e.name)).or_default().push(index);
        }
    }
    let adjacent: Vec<Vec<usize>> = entries
        .iter()
        .map(|e| {
            if !checked(e) {
                return Vec::new();
            }
            e.masters
                .iter()
                .filter_map(|m| by_name.get(&key(m)))
                .flatten()
                .copied()
                .collect()
        })
        .collect();

    // reach[i][j]: j is reached from i by one or more master links.
    let mut reach = vec![vec![false; n]; n];
    for (start, row) in reach.iter_mut().enumerate() {
        let mut stack: Vec<usize> = adjacent[start].clone();
        while let Some(node) = stack.pop() {
            if row[node] {
                continue;
            }
            row[node] = true;
            stack.extend(adjacent[node].iter().copied());
        }
    }

    let mut assigned = vec![false; n];
    let mut groups = Vec::new();
    for i in 0..n {
        if assigned[i] || !reach[i][i] {
            continue;
        }
        let group: Vec<usize> = (0..n).filter(|&j| reach[i][j] && reach[j][i]).collect();
        for &j in &group {
            assigned[j] = true;
        }
        groups.push(group);
    }
    groups
}

/// The findings the Creation Engine rules and the duplicate and header checks raise against a
/// load order. Pure: it reads the entries as they are.
pub fn findings(entries: &[LoadOrderEntry]) -> Vec<Finding> {
    let mut out = Vec::new();

    for e in entries {
        if let Some(reason) = &e.header_error {
            out.push(Finding::UnreadableHeader {
                plugin: e.name.clone(),
                reason: reason.clone(),
            });
        }
    }

    let mut seen: HashMap<String, usize> = HashMap::new();
    for e in entries.iter().filter(|e| !e.implicit) {
        let count = seen.entry(key(&e.name)).or_insert(0);
        *count += 1;
        if *count == 2 {
            out.push(Finding::DuplicateListing {
                plugin: e.name.clone(),
            });
        }
    }

    for (at, e) in entries.iter().enumerate() {
        if !checked(e) {
            continue;
        }
        let mut reported: HashSet<String> = HashSet::new();
        for master in &e.masters {
            let k = key(master);
            if !reported.insert(k.clone()) || master_loads_before(entries, at, &k) {
                continue;
            }
            out.push(Finding::MasterNotEarlier {
                plugin: e.name.clone(),
                master: master.clone(),
                problem: master_problem(entries, at, &k),
            });
        }
    }

    for group in cycle_groups(entries) {
        out.push(Finding::MasterCycle {
            plugins: group.iter().map(|&i| entries[i].name.clone()).collect(),
        });
    }

    let full = entries.iter().filter(|e| checked(e) && !e.light).count();
    if full > FULL_PLUGIN_LIMIT {
        out.push(Finding::TooManyFullPlugins {
            active: full,
            limit: FULL_PLUGIN_LIMIT,
        });
    }
    let light = entries.iter().filter(|e| checked(e) && e.light).count();
    if light > LIGHT_PLUGIN_LIMIT {
        out.push(Finding::TooManyLightPlugins {
            active: light,
            limit: LIGHT_PLUGIN_LIMIT,
        });
    }

    out
}

// ---------------------------------------------------------------------------
// Sort and move
//
// Positions that a locked or always-loaded plugin holds never change. The other positions are
// the free slots, and a free plugin only trades places among them. `free_slots` lists those
// positions in order, and "slot index" below means an index into that list.
// ---------------------------------------------------------------------------

/// The entries a sort may not move: always loaded, locked, or in a master loop.
struct Fixed {
    cyclic: HashSet<String>,
}

impl Fixed {
    fn of(entries: &[LoadOrderEntry]) -> Self {
        let cyclic = cycle_groups(entries)
            .into_iter()
            .flatten()
            .map(|i| key(&entries[i].name))
            .collect();
        Self { cyclic }
    }

    fn cyclic(&self, e: &LoadOrderEntry) -> bool {
        self.cyclic.contains(&key(&e.name))
    }

    fn fixed(&self, e: &LoadOrderEntry) -> bool {
        e.implicit || e.locked || self.cyclic(e)
    }
}

/// The positions whose entries are not fixed, by `is_fixed`.
fn free_slots_of(
    entries: &[LoadOrderEntry],
    is_fixed: impl Fn(&LoadOrderEntry) -> bool,
) -> Vec<usize> {
    (0..entries.len())
        .filter(|&i| !is_fixed(&entries[i]))
        .collect()
}

/// Move the free entry at position `from` to slot index `place`, shifting the other free entries
/// along the free slots. Fixed entries keep their positions. Returns the position it now holds.
fn place_free(
    entries: &mut [LoadOrderEntry],
    free_slots: &[usize],
    from: usize,
    place: usize,
) -> usize {
    let mut items: Vec<LoadOrderEntry> = free_slots.iter().map(|&s| entries[s].clone()).collect();
    let taken = free_slots.iter().take_while(|&&s| s < from).count();
    let item = items.remove(taken);
    items.insert(place, item);
    for (slot, item) in free_slots.iter().zip(items) {
        entries[*slot] = item;
    }
    free_slots[place]
}

/// The first place where a plugin needs a master that may move, as (master position, slot index
/// the master takes). A plugin that does not move takes the master just above it; a fixed plugin
/// (locked, or always loaded) takes the slot just above its own position, and if there is none,
/// the master is blocked instead.
fn first_move(
    entries: &[LoadOrderEntry],
    fixed: &Fixed,
    free_slots: &[usize],
) -> Option<(usize, usize)> {
    for (at, dependent) in entries.iter().enumerate() {
        if !checked(dependent) || fixed.cyclic(dependent) {
            continue;
        }
        let above = free_slots.iter().take_while(|&&s| s < at).count();
        let place = if fixed.fixed(dependent) {
            match above.checked_sub(1) {
                Some(place) => place,
                None => continue,
            }
        } else {
            above
        };
        for master in &dependent.masters {
            let k = key(master);
            if master_loads_before(entries, at, &k) {
                continue;
            }
            let later = entries
                .iter()
                .enumerate()
                .skip(at + 1)
                .find(|(_, m)| loads(m) && key(&m.name) == k && !fixed.fixed(m))
                .map(|(index, _)| index);
            if let Some(from) = later {
                return Some((from, place));
            }
        }
    }
    None
}

/// Sort a load order so every master comes before the plugins that need it, with as few moves as
/// the rules allow: each late master moves up to just before the first plugin that needs it, and
/// everything else keeps its relative order. Locked and always-loaded plugins never move. A plugin
/// that is above a master it cannot get past is reported as blocked. Plugins in a master loop are
/// left alone. Sorting a sorted order makes no moves.
pub fn sort_entries(
    entries: &mut [LoadOrderEntry],
) -> Result<(Vec<SortMove>, Vec<Blocked>), LoadOrderError> {
    let fixed = Fixed::of(entries);
    let free_slots = free_slots_of(entries, |e| fixed.fixed(e));
    let limit = entries
        .len()
        .saturating_mul(entries.len())
        .saturating_add(16);
    let mut moves = Vec::new();
    while let Some((from, place)) = first_move(entries, &fixed, &free_slots) {
        if moves.len() >= limit {
            return Err(LoadOrderError::DidNotSettle(moves.len()));
        }
        let plugin = entries[from].name.clone();
        let to = place_free(entries, &free_slots, from, place) + 1;
        moves.push(SortMove {
            plugin,
            from: from + 1,
            to,
        });
    }

    let mut blocked: Vec<Blocked> = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    for (at, dependent) in entries.iter().enumerate() {
        if !checked(dependent) || fixed.cyclic(dependent) {
            continue;
        }
        for master in &dependent.masters {
            let k = key(master);
            if master_loads_before(entries, at, &k) {
                continue;
            }
            let stuck = entries
                .iter()
                .enumerate()
                .skip(at + 1)
                .any(|(_, m)| loads(m) && key(&m.name) == k && !fixed.cyclic(m));
            if stuck && seen.insert((key(&dependent.name), k.clone())) {
                blocked.push(Blocked {
                    plugin: dependent.name.clone(),
                    master: master.clone(),
                });
            }
        }
    }
    Ok((moves, blocked))
}

/// Move a listed plugin to a position, or before or after another plugin. Returns the plugin's
/// name and its 1-based positions before and after. Locked plugins do not move, and always-loaded
/// plugins cannot be moved or placed against; a position that a locked or always-loaded plugin
/// holds cannot be taken.
pub fn move_entry(
    entries: &mut [LoadOrderEntry],
    plugin: &str,
    target: &MoveTarget,
) -> Result<(String, usize, usize), LoadOrderError> {
    let wanted = key(plugin.trim());
    let from = entries
        .iter()
        .position(|e| key(&e.name) == wanted)
        .ok_or_else(|| LoadOrderError::NotListed(plugin.to_string()))?;
    if entries[from].implicit {
        return Err(LoadOrderError::AlwaysLoaded(entries[from].name.clone()));
    }
    if entries[from].locked {
        return Err(LoadOrderError::Locked(entries[from].name.clone()));
    }
    let free_slots = free_slots_of(entries, |e| e.implicit || e.locked);
    let above = |position: usize| free_slots.iter().take_while(|&&s| s < position).count();
    let slots = free_slots.len();

    let place = match target {
        MoveTarget::Position(position) => {
            if *position == 0 || *position > entries.len() {
                return Err(LoadOrderError::BadPosition {
                    position: *position,
                    last: entries.len(),
                });
            }
            let slot = position - 1;
            match free_slots.iter().position(|&s| s == slot) {
                Some(place) => place,
                None => {
                    return Err(LoadOrderError::Held {
                        plugin: entries[from].name.clone(),
                        position: *position,
                        holder: entries[slot].name.clone(),
                    })
                }
            }
        }
        MoveTarget::Before(other) | MoveTarget::After(other) => {
            let other_key = key(other.trim());
            if other_key == wanted {
                return Err(LoadOrderError::SelfTarget(entries[from].name.clone()));
            }
            let k = entries
                .iter()
                .position(|e| key(&e.name) == other_key)
                .ok_or_else(|| LoadOrderError::NotListed(other.clone()))?;
            let before = matches!(target, MoveTarget::Before(_));
            if before && entries[k].implicit {
                return Err(LoadOrderError::AlwaysLoaded(entries[k].name.clone()));
            }
            let slot_above_k = above(k);
            let no_room = || LoadOrderError::NoRoom {
                plugin: entries[from].name.clone(),
                target: other.clone(),
            };
            if free_slots.contains(&k) {
                // The other plugin moves too: place beside its slot index, counting the moved
                // plugin out of the way when it sat above it.
                let beside = slot_above_k - usize::from(from < k);
                if before {
                    beside
                } else {
                    beside + 1
                }
            } else if before {
                match slot_above_k.checked_sub(1) {
                    Some(place) => place,
                    None => return Err(no_room()),
                }
            } else if slot_above_k < slots {
                slot_above_k
            } else {
                return Err(no_room());
            }
        }
    };
    let name = entries[from].name.clone();
    let to_position = place_free(entries, &free_slots, from, place);
    Ok((name, from + 1, to_position + 1))
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

fn rule_of(definition: &GameDefinition) -> Result<&PluginListRule, LoadOrderError> {
    definition
        .plugin_list
        .as_ref()
        .ok_or_else(|| PluginListError::NoRule(definition.id.clone()).into())
}

/// The plugin folder the game sees, from the deployment plan, and the names its
/// `implicit_list_file` holds. The plan includes the writable layer, which the game also sees.
fn installed_files(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    rule: &PluginListRule,
) -> Result<(Installed, Vec<String>), LoadOrderError> {
    let plan = game_deploy::plan(ctx, instance_id, definition, DeployMode::Links).map_err(|e| {
        LoadOrderError::Files {
            instance_id: instance_id.to_string(),
            reason: e.to_string(),
        }
    })?;
    let folder = rule.plugin_folder.as_str().trim_matches('/');
    let patterns: Vec<String> = rule
        .patterns
        .iter()
        .map(|p| p.trim().to_ascii_lowercase())
        .filter(|p| !p.is_empty())
        .collect();
    let list_file = rule
        .implicit_list_file
        .as_ref()
        .map(|p| p.as_str().to_ascii_lowercase());

    let mut installed = Installed::new();
    let mut list_path = None;
    for file in &plan.files {
        let path = file.path.as_str();
        if list_file.as_deref() == Some(path.to_ascii_lowercase().as_str()) {
            list_path = Some(game_deploy::source_path(ctx, &file.source));
            continue;
        }
        let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
        if !parent.eq_ignore_ascii_case(folder) {
            continue;
        }
        let lower = key(name);
        if patterns.iter().any(|p| glob_match(p, &lower)) {
            installed.insert(lower, game_deploy::source_path(ctx, &file.source));
        }
    }

    let names = match list_path {
        None => Vec::new(),
        Some(path) => {
            let bytes = std::fs::read(&path).map_err(|source| LoadOrderError::Io {
                path: path.clone(),
                source,
            })?;
            parse_implicit_list(&String::from_utf8_lossy(&bytes))
        }
    };
    Ok((installed, names))
}

fn order_of(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
) -> Result<LoadOrder, LoadOrderError> {
    let rule = rule_of(definition)?;
    let listed = game_plugins::list(ctx, instance_id, definition)?;
    if is_engine(rule) {
        let (installed, names) = installed_files(ctx, instance_id, definition, rule)?;
        Ok(build_order(
            rule,
            &listed.entries,
            Some(&installed),
            &names,
            listed.exists,
        ))
    } else {
        Ok(build_order(rule, &listed.entries, None, &[], listed.exists))
    }
}

/// The instance's load order, with what the game's rules say about it.
pub fn order(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
) -> Result<LoadOrder, LoadOrderError> {
    order_of(ctx, instance_id, definition)
}

/// The findings against the instance's order. A launch check calls this and acts on the result.
pub fn check(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
) -> Result<Vec<Finding>, LoadOrderError> {
    Ok(order_of(ctx, instance_id, definition)?.findings)
}

fn write_entries(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    entries: &[LoadOrderEntry],
) -> Result<(), LoadOrderError> {
    let order: Vec<(String, bool)> = entries
        .iter()
        .filter(|e| !e.implicit)
        .map(|e| (e.name.clone(), e.active))
        .collect();
    game_plugins::write_order_locked(ctx, instance_id, definition, &order)?;
    Ok(())
}

/// Sort the instance's list so masters come before the plugins that need them. With `dry_run` it
/// reports the moves and writes nothing. Requires the Creation Engine rules.
pub fn sort(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    dry_run: bool,
) -> Result<SortReport, LoadOrderError> {
    if !is_engine(rule_of(definition)?) {
        return Err(LoadOrderError::NoRules(definition.id.as_str().to_string()));
    }
    let _lock = ctx
        .lock_manager
        .acquire(
            LockResource::Instance(instance_id.to_string()),
            "plugins-order",
        )
        .map_err(PluginListError::from)?;
    let mut entries = order_of(ctx, instance_id, definition)?.entries;
    let (moves, blocked) = sort_entries(&mut entries)?;
    let written = !dry_run && !moves.is_empty();
    if written {
        write_entries(ctx, instance_id, definition, &entries)?;
    }
    Ok(SortReport {
        moves,
        blocked,
        written,
    })
}

/// Move one listed plugin to a position, or before or after another plugin.
pub fn move_plugin(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    plugin: &str,
    target: &MoveTarget,
) -> Result<MoveReport, LoadOrderError> {
    rule_of(definition)?;
    let _lock = ctx
        .lock_manager
        .acquire(
            LockResource::Instance(instance_id.to_string()),
            "plugins-order",
        )
        .map_err(PluginListError::from)?;
    let mut entries = order_of(ctx, instance_id, definition)?.entries;
    let (name, from, to) = move_entry(&mut entries, plugin, target)?;
    let written = from != to;
    if written {
        write_entries(ctx, instance_id, definition, &entries)?;
    }
    Ok(MoveReport {
        plugin: name,
        from,
        to,
        written,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, masters: &[&str]) -> LoadOrderEntry {
        LoadOrderEntry {
            name: name.to_string(),
            implicit: false,
            active: true,
            present: true,
            managed: false,
            locked: false,
            header_read: true,
            master: name.to_ascii_lowercase().ends_with(".esm"),
            light: false,
            masters: masters.iter().map(|m| m.to_string()).collect(),
            header_error: None,
        }
    }

    fn implicit(name: &str) -> LoadOrderEntry {
        LoadOrderEntry {
            implicit: true,
            master: true,
            ..entry(name, &[])
        }
    }

    fn locked(name: &str, masters: &[&str]) -> LoadOrderEntry {
        LoadOrderEntry {
            locked: true,
            ..entry(name, masters)
        }
    }

    fn names(entries: &[LoadOrderEntry]) -> Vec<String> {
        entries.iter().map(|e| e.name.clone()).collect()
    }

    #[test]
    fn the_implicit_list_file_skips_blanks_and_comments_and_keeps_order() {
        let text = "\u{feff}# header\r\n\r\nccBGSSSE001-Fish.esm\r\n  ccbgssse037-curios.esl  \r\n";
        assert_eq!(
            parse_implicit_list(text),
            ["ccBGSSSE001-Fish.esm", "ccbgssse037-curios.esl"]
        );
    }

    #[test]
    fn a_late_master_is_a_finding_and_names_its_problem() {
        let late = [entry("A.esp", &["B.esm"]), entry("B.esm", &[])];
        assert_eq!(
            findings(&late),
            [Finding::MasterNotEarlier {
                plugin: "A.esp".into(),
                master: "B.esm".into(),
                problem: MasterProblem::Later,
            }]
        );
    }

    #[test]
    fn a_missing_or_inactive_master_is_a_finding() {
        let mut inactive = entry("B.esm", &[]);
        inactive.active = false;
        let inactive_list = [inactive, entry("A.esp", &["B.esm"])];
        assert_eq!(
            findings(&inactive_list),
            [Finding::MasterNotEarlier {
                plugin: "A.esp".into(),
                master: "B.esm".into(),
                problem: MasterProblem::Inactive,
            }]
        );
        let missing = [entry("A.esp", &["Gone.esm"])];
        assert_eq!(
            findings(&missing),
            [Finding::MasterNotEarlier {
                plugin: "A.esp".into(),
                master: "Gone.esm".into(),
                problem: MasterProblem::Missing,
            }]
        );
    }

    #[test]
    fn a_master_listed_but_not_installed_is_missing_not_inactive() {
        let mut absent = entry("B.esm", &[]);
        absent.present = false;
        let list = [absent, entry("A.esp", &["B.esm"])];
        assert!(matches!(
            findings(&list).as_slice(),
            [Finding::MasterNotEarlier {
                problem: MasterProblem::Missing,
                ..
            }]
        ));
    }

    #[test]
    fn an_esp_above_an_esm_with_no_dependency_is_not_a_finding() {
        let list = [entry("A.esp", &[]), entry("B.esm", &[])];
        assert!(findings(&list).is_empty());
    }

    #[test]
    fn a_master_cycle_is_reported_and_sorting_it_terminates_without_moving_it() {
        let list = [entry("A.esp", &["B.esp"]), entry("B.esp", &["A.esp"])];
        assert!(findings(&list).contains(&Finding::MasterCycle {
            plugins: vec!["A.esp".into(), "B.esp".into()],
        }));
        let mut sorted = list.to_vec();
        let (moves, _) = sort_entries(&mut sorted).unwrap();
        assert!(moves.is_empty());
        assert_eq!(sorted, list);
    }

    #[test]
    fn a_plugin_listed_twice_is_a_finding_once() {
        let list = [entry("A.esp", &[]), entry("a.ESP", &[])];
        assert_eq!(
            findings(&list),
            [Finding::DuplicateListing {
                plugin: "a.ESP".into()
            }]
        );
    }

    #[test]
    fn names_compare_case_insensitively_for_masters() {
        let list = [entry("B.ESM", &[]), entry("A.esp", &["b.esm"])];
        assert!(findings(&list).is_empty());
    }

    #[test]
    fn an_unreadable_header_is_a_finding_for_active_and_inactive_plugins() {
        let mut bad = entry("Bad.esp", &[]);
        bad.header_read = false;
        bad.header_error = Some("truncated".into());
        let mut inactive_bad = bad.clone();
        inactive_bad.name = "Other.esp".into();
        inactive_bad.active = false;
        assert_eq!(
            findings(&[bad, inactive_bad]),
            [
                Finding::UnreadableHeader {
                    plugin: "Bad.esp".into(),
                    reason: "truncated".into()
                },
                Finding::UnreadableHeader {
                    plugin: "Other.esp".into(),
                    reason: "truncated".into()
                },
            ]
        );
    }

    #[test]
    fn full_plugins_over_254_are_a_finding_and_254_are_not() {
        let at_limit: Vec<LoadOrderEntry> = (0..FULL_PLUGIN_LIMIT)
            .map(|i| entry(&format!("P{i}.esp"), &[]))
            .collect();
        assert!(findings(&at_limit).is_empty());
        let mut over = at_limit;
        over.push(entry("Extra.esp", &[]));
        assert_eq!(
            findings(&over),
            [Finding::TooManyFullPlugins {
                active: 255,
                limit: 254
            }]
        );
    }

    #[test]
    fn light_plugins_are_limited_separately_from_full_ones() {
        let mut lights: Vec<LoadOrderEntry> = (0..=LIGHT_PLUGIN_LIMIT)
            .map(|i| {
                let mut e = entry(&format!("L{i}.esl"), &[]);
                e.light = true;
                e
            })
            .collect();
        assert_eq!(
            findings(&lights),
            [Finding::TooManyLightPlugins {
                active: LIGHT_PLUGIN_LIMIT + 1,
                limit: LIGHT_PLUGIN_LIMIT,
            }]
        );
        lights.truncate(LIGHT_PLUGIN_LIMIT);
        assert!(findings(&lights).is_empty());
    }

    #[test]
    fn an_inactive_plugin_does_not_count_toward_the_limit() {
        let mut list: Vec<LoadOrderEntry> = (0..FULL_PLUGIN_LIMIT + 5)
            .map(|i| entry(&format!("P{i}.esp"), &[]))
            .collect();
        for e in list.iter_mut().take(5) {
            e.active = false;
        }
        assert!(findings(&list).is_empty());
    }

    #[test]
    fn sort_fixes_a_late_master_with_one_move_and_keeps_the_rest() {
        let mut list = vec![
            entry("A.esp", &["B.esm"]),
            entry("C.esp", &[]),
            entry("B.esm", &[]),
            entry("D.esp", &[]),
        ];
        let (moves, blocked) = sort_entries(&mut list).unwrap();
        assert_eq!(
            moves,
            [SortMove {
                plugin: "B.esm".into(),
                from: 3,
                to: 1
            }]
        );
        assert!(blocked.is_empty());
        assert_eq!(names(&list), ["B.esm", "A.esp", "C.esp", "D.esp"]);
        assert!(findings(&list).is_empty());
    }

    #[test]
    fn a_chain_of_late_masters_is_sorted_and_the_second_sort_does_nothing() {
        let mut list = vec![
            entry("A.esp", &["B.esm"]),
            entry("B.esm", &["C.esm"]),
            entry("C.esm", &["D.esm"]),
            entry("D.esm", &[]),
        ];
        let (first, _) = sort_entries(&mut list).unwrap();
        assert_eq!(first.len(), 3);
        assert_eq!(names(&list), ["D.esm", "C.esm", "B.esm", "A.esp"]);
        let settled = list.clone();
        let (second, _) = sort_entries(&mut list).unwrap();
        assert!(second.is_empty());
        assert_eq!(list, settled);
    }

    #[test]
    fn sort_is_idempotent() {
        let mut list = vec![
            entry("A.esp", &["C.esm"]),
            entry("B.esp", &["D.esm"]),
            entry("C.esm", &[]),
            entry("D.esm", &["C.esm"]),
            entry("E.esp", &[]),
        ];
        let (first, _) = sort_entries(&mut list).unwrap();
        assert!(!first.is_empty());
        let once = list.clone();
        let (second, _) = sort_entries(&mut list).unwrap();
        assert!(second.is_empty());
        assert_eq!(list, once);
        assert!(findings(&list).is_empty());
    }

    #[test]
    fn a_locked_master_below_its_dependent_is_blocked_and_nothing_moves() {
        let mut list = vec![entry("A.esp", &["B.esm"]), locked("B.esm", &[])];
        let (moves, blocked) = sort_entries(&mut list).unwrap();
        assert!(moves.is_empty());
        assert_eq!(
            blocked,
            [Blocked {
                plugin: "A.esp".into(),
                master: "B.esm".into()
            }]
        );
        assert_eq!(names(&list), ["A.esp", "B.esm"]);
    }

    #[test]
    fn a_locked_plugin_with_no_free_place_above_it_is_blocked_not_moved() {
        // A is locked above its master B. Nothing free sits above A, so B cannot go there.
        let mut list = vec![locked("A.esp", &["B.esm"]), entry("B.esm", &[])];
        let (moves, blocked) = sort_entries(&mut list).unwrap();
        assert!(moves.is_empty());
        assert_eq!(blocked.len(), 1);
        assert_eq!(names(&list), ["A.esp", "B.esm"]);
    }

    #[test]
    fn a_locked_plugin_keeps_its_position_while_a_free_master_moves_above_it() {
        // F is free, L is locked above its master M, and M is free: M takes the free place above L.
        let mut list = vec![
            entry("F.esp", &[]),
            locked("L.esp", &["M.esm"]),
            entry("M.esm", &[]),
        ];
        let (moves, blocked) = sort_entries(&mut list).unwrap();
        assert_eq!(
            moves,
            [SortMove {
                plugin: "M.esm".into(),
                from: 3,
                to: 1
            }]
        );
        assert!(blocked.is_empty());
        assert_eq!(names(&list), ["M.esm", "L.esp", "F.esp"]);
        assert!(list[1].locked);
    }

    #[test]
    fn always_loaded_plugins_are_never_moved_and_stay_first() {
        let mut list = vec![
            implicit("Skyrim.esm"),
            entry("A.esp", &["Skyrim.esm"]),
            implicit("Dawnguard.esm"),
        ];
        let (moves, _) = sort_entries(&mut list).unwrap();
        assert!(moves.is_empty());
        assert_eq!(names(&list), ["Skyrim.esm", "A.esp", "Dawnguard.esm"]);
    }

    #[test]
    fn a_listed_master_cannot_be_placed_above_an_always_loaded_one() {
        // Dawnguard needs a listed plugin: the listed one cannot go above the always-loaded one.
        let mut list = vec![implicit("Dawnguard.esm"), entry("Mod.esm", &[])];
        list[0].masters = vec!["Mod.esm".into()];
        let (moves, blocked) = sort_entries(&mut list).unwrap();
        assert!(moves.is_empty());
        assert_eq!(
            blocked,
            [Blocked {
                plugin: "Dawnguard.esm".into(),
                master: "Mod.esm".into()
            }]
        );
    }

    #[test]
    fn move_to_a_position_before_and_after() {
        let base = || {
            vec![
                implicit("Skyrim.esm"),
                entry("A.esp", &[]),
                entry("B.esp", &[]),
                entry("C.esp", &[]),
            ]
        };
        let mut list = base();
        let moved = move_entry(&mut list, "C.esp", &MoveTarget::Position(2)).unwrap();
        assert_eq!(moved, ("C.esp".to_string(), 4, 2));
        assert_eq!(names(&list), ["Skyrim.esm", "C.esp", "A.esp", "B.esp"]);

        let mut list = base();
        let moved = move_entry(&mut list, "A.esp", &MoveTarget::Before("C.esp".into())).unwrap();
        assert_eq!(moved, ("A.esp".to_string(), 2, 3));
        assert_eq!(names(&list), ["Skyrim.esm", "B.esp", "A.esp", "C.esp"]);

        let mut list = base();
        let moved = move_entry(&mut list, "A.esp", &MoveTarget::After("C.esp".into())).unwrap();
        assert_eq!(moved, ("A.esp".to_string(), 2, 4));
        assert_eq!(names(&list), ["Skyrim.esm", "B.esp", "C.esp", "A.esp"]);

        let mut list = base();
        move_entry(&mut list, "C.esp", &MoveTarget::Before("A.esp".into())).unwrap();
        assert_eq!(names(&list), ["Skyrim.esm", "C.esp", "A.esp", "B.esp"]);
    }

    #[test]
    fn move_keeps_a_locked_plugin_in_its_position() {
        let mut list = vec![
            implicit("Skyrim.esm"),
            entry("A.esp", &[]),
            locked("L.esp", &[]),
            entry("B.esp", &[]),
        ];
        // B goes before A: the locked plugin between them does not move.
        move_entry(&mut list, "B.esp", &MoveTarget::Before("A.esp".into())).unwrap();
        assert_eq!(names(&list), ["Skyrim.esm", "B.esp", "L.esp", "A.esp"]);
        assert!(list[2].locked);
    }

    #[test]
    fn move_refuses_what_it_must() {
        let base = || {
            vec![
                implicit("Skyrim.esm"),
                entry("A.esp", &[]),
                entry("B.esp", &[]),
            ]
        };
        let mut list = base();
        assert!(matches!(
            move_entry(&mut list, "Skyrim.esm", &MoveTarget::Position(3)),
            Err(LoadOrderError::AlwaysLoaded(_))
        ));
        let mut list = base();
        assert!(matches!(
            move_entry(&mut list, "A.esp", &MoveTarget::Position(1)),
            Err(LoadOrderError::Held { .. })
        ));
        let mut list = base();
        assert!(matches!(
            move_entry(&mut list, "A.esp", &MoveTarget::Position(9)),
            Err(LoadOrderError::BadPosition { .. })
        ));
        let mut list = base();
        assert!(matches!(
            move_entry(&mut list, "A.esp", &MoveTarget::Before("Skyrim.esm".into())),
            Err(LoadOrderError::AlwaysLoaded(_))
        ));
        let mut list = base();
        assert!(matches!(
            move_entry(&mut list, "A.esp", &MoveTarget::After("A.esp".into())),
            Err(LoadOrderError::SelfTarget(_))
        ));
        let mut list = base();
        assert!(matches!(
            move_entry(&mut list, "Nope.esp", &MoveTarget::Position(3)),
            Err(LoadOrderError::NotListed(_))
        ));
        let mut list = base();
        list[1].locked = true;
        assert!(matches!(
            move_entry(&mut list, "A.esp", &MoveTarget::Position(3)),
            Err(LoadOrderError::Locked(_))
        ));
    }

    #[test]
    fn header_reading_fails_as_text_for_bytes_that_are_not_a_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Empty.esp");
        std::fs::write(&path, b"").unwrap();
        assert!(read_header(&path).is_err());
        let path = dir.path().join("Junk.esp");
        std::fs::write(&path, b"this is not a plugin at all, just words").unwrap();
        assert!(read_header(&path).is_err());
    }

    /// A Creation Engine plugin's bytes: a TES4 record with flags, an HEDR subrecord, and a MAST and
    /// DATA pair per master. Header layout as esplugin reads it for Skyrim SE: a 24-byte record
    /// header (type, data size, flags at offset 8, form ID, two unused words), then subrecords of a
    /// 4-byte type, a u16 size and the data.
    fn plugin_bytes(flags: u32, masters: &[&str]) -> Vec<u8> {
        fn subrecord(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
            out.extend_from_slice(kind);
            out.extend_from_slice(&u16::try_from(data.len()).unwrap().to_le_bytes());
            out.extend_from_slice(data);
        }
        let mut body = Vec::new();
        subrecord(&mut body, b"HEDR", &[0u8; 12]);
        for master in masters {
            let mut name = master.as_bytes().to_vec();
            name.push(0);
            subrecord(&mut body, b"MAST", &name);
            subrecord(&mut body, b"DATA", &[0u8; 8]);
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"TES4");
        out.extend_from_slice(&u32::try_from(body.len()).unwrap().to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&[0u8; 12]);
        out.extend_from_slice(&body);
        out
    }

    fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn the_header_gives_masters_and_the_master_and_light_flags() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "Mod.esp",
            &plugin_bytes(0, &["Skyrim.esm", "Dawnguard.esm"]),
        );
        let header = read_header(&path).unwrap();
        assert_eq!(header.masters, ["Skyrim.esm", "Dawnguard.esm"]);
        assert!(!header.master);
        assert!(!header.light);

        let esm = write(dir.path(), "Base.esp", &plugin_bytes(0x1, &[]));
        assert!(read_header(&esm).unwrap().master, "the master flag is read");

        let flagged_light = write(dir.path(), "Flagged.esp", &plugin_bytes(0x200, &[]));
        assert!(
            read_header(&flagged_light).unwrap().light,
            "the light flag is read"
        );

        let by_extension = write(dir.path(), "Light.esl", &plugin_bytes(0, &[]));
        let header = read_header(&by_extension).unwrap();
        assert!(header.light, "an .esl is light by its extension");
        assert!(header.master, "an .esl is a master by its extension");
    }

    #[test]
    fn a_truncated_header_is_an_error_not_an_empty_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let mut bytes = plugin_bytes(0, &["Skyrim.esm"]);
        bytes.truncate(bytes.len() - 6);
        let path = write(dir.path(), "Cut.esp", &bytes);
        assert!(read_header(&path).is_err());
        let short = write(dir.path(), "Short.esp", &plugin_bytes(0, &[])[..10]);
        assert!(read_header(&short).is_err());
    }

    fn installed_of(files: &[(&str, PathBuf)]) -> Installed {
        files
            .iter()
            .map(|(name, path)| (name.to_ascii_lowercase(), path.clone()))
            .collect()
    }

    fn plugin_line_of(name: &str, active: bool) -> PluginEntry {
        PluginEntry {
            name: name.to_string(),
            active,
            managed: false,
            locked: false,
        }
    }

    fn engine_rule() -> PluginListRule {
        PluginListRule {
            user_file: agora_game_api::RelPath::new("user/Plugins.txt").unwrap(),
            plugin_folder: agora_game_api::RelPath::new("Data").unwrap(),
            patterns: vec!["*.esm".into(), "*.esl".into(), "*.esp".into()],
            active_prefix: "*".into(),
            header: Vec::new(),
            semantics: Some(CREATION_ENGINE.into()),
            implicit: vec!["Skyrim.esm".into(), "Dawnguard.esm".into()],
            implicit_list_file: Some(agora_game_api::RelPath::new("Skyrim.ccc").unwrap()),
        }
    }

    #[test]
    fn the_order_puts_present_implicit_plugins_first_and_drops_listed_copies_of_them() {
        let dir = tempfile::tempdir().unwrap();
        let skyrim = write(dir.path(), "Skyrim.esm", &plugin_bytes(0x1, &[]));
        let cc = write(
            dir.path(),
            "ccbgssse037-curios.esl",
            &plugin_bytes(0x200, &["Skyrim.esm"]),
        );
        let mod_esp = write(dir.path(), "Mod.esp", &plugin_bytes(0, &["Skyrim.esm"]));
        let installed = installed_of(&[
            ("Skyrim.esm", skyrim),
            ("ccbgssse037-curios.esl", cc),
            ("Mod.esp", mod_esp),
        ]);
        // Dawnguard is implicit but not installed; the Skyrim.ccc name for Fish is not installed.
        let names = vec![
            "ccBGSSSE001-Fish.esm".to_string(),
            "ccBGSSSE037-Curios.esl".to_string(),
        ];
        let listed = [
            plugin_line_of("Skyrim.esm", true),
            plugin_line_of("Mod.esp", true),
        ];
        let order = build_order(&engine_rule(), &listed, Some(&installed), &names, true);

        let order_names: Vec<&str> = order.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            order_names,
            ["Skyrim.esm", "ccBGSSSE037-Curios.esl", "Mod.esp"]
        );
        assert!(order.entries[0].implicit && order.entries[1].implicit);
        assert!(!order.entries[2].implicit);
        assert!(order.findings.is_empty(), "{:?}", order.findings);
    }

    #[test]
    fn a_plugin_whose_header_is_junk_is_a_finding_that_names_it() {
        let dir = tempfile::tempdir().unwrap();
        let skyrim = write(dir.path(), "Skyrim.esm", &plugin_bytes(0x1, &[]));
        let junk = write(dir.path(), "Junk.esp", b"not a plugin");
        let installed = installed_of(&[("Skyrim.esm", skyrim), ("Junk.esp", junk)]);
        let listed = [plugin_line_of("Junk.esp", true)];
        let order = build_order(&engine_rule(), &listed, Some(&installed), &[], true);
        let junk_entry = order.entries.iter().find(|e| e.name == "Junk.esp").unwrap();
        assert!(junk_entry.header_error.is_some());
        assert!(matches!(
            order.findings.as_slice(),
            [Finding::UnreadableHeader { plugin, .. }] if plugin == "Junk.esp"
        ));
    }

    #[test]
    fn a_plugin_that_is_listed_but_not_installed_is_shown_missing_and_is_not_a_finding() {
        let dir = tempfile::tempdir().unwrap();
        let skyrim = write(dir.path(), "Skyrim.esm", &plugin_bytes(0x1, &[]));
        let installed = installed_of(&[("Skyrim.esm", skyrim)]);
        let listed = [plugin_line_of("Gone.esp", true)];
        let order = build_order(&engine_rule(), &listed, Some(&installed), &[], true);
        let gone = order.entries.iter().find(|e| e.name == "Gone.esp").unwrap();
        assert!(!gone.present);
        assert!(order.findings.is_empty());
    }

    #[test]
    fn a_game_without_creation_rules_reads_no_headers_and_has_no_findings() {
        let mut rule = engine_rule();
        rule.semantics = None;
        let listed = [plugin_line_of("A.esp", true), plugin_line_of("B.esp", true)];
        let order = build_order(&rule, &listed, None, &[], true);
        assert!(!order.engine);
        assert!(order.entries.iter().all(|e| !e.header_read && e.present));
        assert!(order.findings.is_empty());
    }
}
