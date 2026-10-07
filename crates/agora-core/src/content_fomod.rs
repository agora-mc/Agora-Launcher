//! FOMOD installers (MASTER_SPEC §26.6, "Archives and installers are core").
//!
//! Many mods ship as an archive holding `fomod/ModuleConfig.xml`: steps with options, whose
//! choices decide which of the archive's files are installed and where. This module is the one
//! implementation of that format:
//!
//! 1. [`parse`] reads the config from an archive item's stored objects (the archive is never
//!    extracted again) into a typed [`FomodInstaller`].
//! 2. [`evaluate`] turns the user's [`Choice`]s into an [`InstallPlan`]; [`defaults`] proposes
//!    choices; [`FomodInstaller::resolve_choice`] reads a `Step/Group/Plugin` string.
//! 3. [`install`] derives a new content item from the plan (the objects are shared, nothing is
//!    copied) and records the choices in its [`ContentSource::FomodInstall`] so a reinstall can
//!    replay them.
//!
//! The config is untrusted input. The document, its nesting and every count in it are bounded
//! below, no index is taken from a number in the file, and every `source`, `destination` and
//! image path is checked with the content store's own path rules before it is used.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::Read;

use agora_game_api::{LayerSource, RelPath};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::content_store::{
    self, AddOutcome, ContentError, ContentFile, ContentItem, ContentSource,
};
use crate::ctx::Ctx;

// ---------------------------------------------------------------------------
// Limits
// ---------------------------------------------------------------------------

/// Largest `ModuleConfig.xml` that is read.
pub const MAX_DOCUMENT_BYTES: usize = 4 * 1024 * 1024;
/// Deepest element nesting accepted.
pub const MAX_DEPTH: usize = 32;
const MAX_INFO_BYTES: usize = 1024 * 1024;
const MAX_NODES: u32 = 200_000;
const MAX_STEPS: usize = 256;
const MAX_GROUPS_PER_STEP: usize = 256;
const MAX_PLUGINS_PER_GROUP: usize = 1024;
const MAX_PLUGINS: usize = 8192;
const MAX_ENTRIES: usize = 65_536;
const MAX_CONDITIONS: usize = 100_000;
const MAX_PATTERNS: usize = 8192;
const MAX_PLAN_CANDIDATES: usize = 500_000;
const MAX_NOTES: usize = 200;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum FomodError {
    #[error("item {0} has no fomod/ModuleConfig.xml")]
    NoInstaller(String),
    #[error("{what} is {size} bytes; the limit is {limit}")]
    TooLarge {
        what: String,
        size: u64,
        limit: usize,
    },
    #[error("the installer has more than {limit} {what}")]
    TooMany { what: &'static str, limit: usize },
    #[error("the installer nests deeper than {limit} levels")]
    TooDeep { limit: usize },
    #[error("cannot decode ModuleConfig.xml: {0}")]
    Encoding(String),
    #[error("ModuleConfig.xml is not valid XML: {0}")]
    Xml(String),
    #[error("{element}: {reason}")]
    Invalid { element: String, reason: String },
    #[error("{element}: path '{path}' is not allowed: {reason}")]
    UnsafePath {
        element: String,
        path: String,
        reason: String,
    },
    #[error("no step named '{0}'")]
    UnknownStep(String),
    #[error("step '{step}' has no group named '{group}'")]
    UnknownGroup { step: String, group: String },
    #[error("group '{step}/{group}' has no plugin named '{plugin}'")]
    UnknownPlugin {
        step: String,
        group: String,
        plugin: String,
    },
    #[error("'{spec}' does not name an option (expected Step/Group/Plugin)")]
    UnknownChoice { spec: String },
    #[error("'{spec}' names more than one option: {matches}")]
    AmbiguousChoice { spec: String, matches: String },
    #[error("group '{step}/{group}' must {rule}, but {chosen} chosen")]
    GroupRule {
        step: String,
        group: String,
        rule: &'static str,
        chosen: usize,
    },
    #[error(
        "plugin '{plugin}' in group '{step}/{group}' is marked NotUsable and cannot be chosen"
    )]
    NotUsable {
        step: String,
        group: String,
        plugin: String,
    },
    #[error("the chosen options install no files")]
    NothingToInstall,
    #[error("{0}")]
    Other(String),
    #[error(transparent)]
    Content(#[from] ContentError),
}

// ---------------------------------------------------------------------------
// The model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupType {
    SelectExactlyOne,
    SelectAtMostOne,
    SelectAtLeastOne,
    SelectAll,
    SelectAny,
}

impl GroupType {
    fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "selectexactlyone" => Some(Self::SelectExactlyOne),
            "selectatmostone" => Some(Self::SelectAtMostOne),
            "selectatleastone" => Some(Self::SelectAtLeastOne),
            "selectall" => Some(Self::SelectAll),
            "selectany" => Some(Self::SelectAny),
            _ => None,
        }
    }

    pub fn rule(&self) -> &'static str {
        match self {
            Self::SelectExactlyOne => "select exactly one",
            Self::SelectAtMostOne => "select at most one",
            Self::SelectAtLeastOne => "select at least one",
            Self::SelectAll => "select all",
            Self::SelectAny => "select any number",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginType {
    Required,
    Optional,
    Recommended,
    NotUsable,
    CouldBeUsable,
}

impl PluginType {
    fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "required" => Some(Self::Required),
            "optional" => Some(Self::Optional),
            "recommended" => Some(Self::Recommended),
            "notusable" => Some(Self::NotUsable),
            "couldbeusable" => Some(Self::CouldBeUsable),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileState {
    Active,
    Inactive,
    Missing,
}

impl FileState {
    fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "active" => Some(Self::Active),
            "inactive" => Some(Self::Inactive),
            "missing" => Some(Self::Missing),
            _ => None,
        }
    }
}

/// A condition on flags and files, as found in `visible`, `moduleDependencies`, plugin type
/// patterns and conditional installs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Condition {
    /// `operator="And"`: every condition holds (an empty list holds).
    All { conditions: Vec<Condition> },
    /// `operator="Or"`: one holds (an empty list holds, so a stray empty element never hides a
    /// step).
    Any { conditions: Vec<Condition> },
    /// `flagDependency`: the flag has this value (an unset flag equals `""`).
    Flag { flag: String, value: String },
    /// `fileDependency`: a file (relative to the game's data folder) is in this state.
    File { file: String, state: FileState },
    /// `gameDependency`, `fommDependency` and the script extenders' versions. Agora cannot check
    /// them yet, so they count as satisfied (and the plan says so).
    Version { kind: String, version: String },
}

impl Condition {
    /// The condition in plain words.
    pub fn describe(&self) -> String {
        match self {
            Condition::All { conditions } | Condition::Any { conditions } => {
                let word = if matches!(self, Condition::All { .. }) {
                    "all of"
                } else {
                    "any of"
                };
                match conditions.len() {
                    0 => "always".to_string(),
                    1 => conditions[0].describe(),
                    _ => format!(
                        "{word} ({})",
                        conditions
                            .iter()
                            .map(Condition::describe)
                            .collect::<Vec<_>>()
                            .join("; ")
                    ),
                }
            }
            Condition::Flag { flag, value } => format!("option flag '{flag}' is '{value}'"),
            Condition::File { file, state } => {
                let state = match state {
                    FileState::Active => "active",
                    FileState::Inactive => "installed but inactive",
                    FileState::Missing => "missing",
                };
                format!("'{file}' is {state}")
            }
            Condition::Version { kind, version } => {
                format!("{kind} version {version} (not checked)")
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    Folder,
}

/// A `<file>` or `<folder>` element. Paths are normalised to `/` and checked: `source` is relative
/// to the installer root, `destination` to the folder the archive's content goes to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallEntry {
    pub kind: EntryKind,
    pub source: String,
    /// For a file, the full destination path (a destination that is empty or ends in a separator
    /// names a folder, and the file keeps its name); for a folder, the destination folder (`""` is
    /// the root). Omitted in the config means the same path as `source`.
    pub destination: String,
    pub always_install: bool,
    pub install_if_usable: bool,
    pub priority: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TypeDescriptor {
    Simple {
        plugin_type: PluginType,
    },
    /// The type depends on conditions: the first pattern that holds, else the default.
    Dependent {
        default: PluginType,
        patterns: Vec<(Condition, PluginType)>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlagSetting {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plugin {
    pub name: String,
    pub description: String,
    pub image: Option<String>,
    pub files: Vec<InstallEntry>,
    pub flags: Vec<FlagSetting>,
    pub type_descriptor: TypeDescriptor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    pub name: String,
    pub group_type: GroupType,
    pub plugins: Vec<Plugin>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub name: String,
    pub visible: Option<Condition>,
    pub groups: Vec<Group>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConditionalInstall {
    pub condition: Condition,
    pub files: Vec<InstallEntry>,
}

/// `fomod/info.xml`, when present and readable; every field is optional.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FomodInfo {
    pub name: Option<String>,
    pub author: Option<String>,
    pub version: Option<String>,
    pub website: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FomodInstaller {
    /// The archive item the installer was read from.
    pub item_id: String,
    /// Where the installer lives inside the item: the parent of its `fomod` folder (`""` when the
    /// folder is at the top). Every `source` is relative to it.
    pub root: String,
    pub module_name: String,
    pub info: Option<FomodInfo>,
    pub module_dependencies: Option<Condition>,
    pub required_files: Vec<InstallEntry>,
    /// In install order (`order="Ascending"` and `"Descending"` are already applied).
    pub steps: Vec<Step>,
    pub conditional_installs: Vec<ConditionalInstall>,
    #[serde(skip)]
    files: Vec<ContentFile>,
}

/// One option the user picked, by name: the plugins chosen in a group of a step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    pub step: String,
    pub group: String,
    pub plugins: Vec<String>,
}

/// What the instance already has, to answer `fileDependency`. Paths are relative to the game's
/// data folder and compared case-insensitively. A file the context does not know is `Missing`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FomodContext {
    states: HashMap<String, FileState>,
}

impl FomodContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_active<I, S>(files: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut c = Self::new();
        for f in files {
            c.set(f.as_ref(), FileState::Active);
        }
        c
    }

    /// Record a state; `Active` is never downgraded to `Inactive`.
    pub fn set(&mut self, file: &str, state: FileState) {
        let key = norm_key(file);
        match (self.states.get(&key), state) {
            (Some(FileState::Active), FileState::Inactive) => {}
            _ => {
                self.states.insert(key, state);
            }
        }
    }

    pub fn state_of(&self, file: &str) -> FileState {
        self.states
            .get(&norm_key(file))
            .copied()
            .unwrap_or(FileState::Missing)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedFile {
    /// Relative to the folder the content goes to.
    pub destination: RelPath,
    /// The file's path relative to the installer root, as a `source` attribute names it (the
    /// archive item's path is `FomodInstaller::root` joined with this).
    pub source: RelPath,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallPlan {
    /// Sorted by destination.
    pub files: Vec<PlannedFile>,
    /// The choices as they were applied (defaults for required options filled in, hidden steps
    /// dropped), in installer order. Replaying them gives the same plan.
    pub choices: Vec<Choice>,
    /// Things the user should know: assumed conditions, ignored choices, skipped files, conflicts.
    pub notes: Vec<String>,
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

fn lc(s: &str) -> String {
    s.to_ascii_lowercase()
}

/// A path as `/`-separated components, without empty or `.` ones.
fn norm_path(path: &str) -> String {
    path.replace('\\', "/")
        .split('/')
        .filter(|c| !c.is_empty() && *c != ".")
        .collect::<Vec<_>>()
        .join("/")
}

/// A lookup key for a path: [`norm_path`], lowercase.
fn norm_key(path: &str) -> String {
    norm_path(path).to_ascii_lowercase()
}

fn join_path(a: &str, b: &str) -> String {
    match (a.is_empty(), b.is_empty()) {
        (true, _) => b.to_string(),
        (_, true) => a.to_string(),
        _ => format!("{a}/{b}"),
    }
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[derive(Default)]
struct Notes {
    list: Vec<String>,
    seen: HashSet<String>,
    overflow: bool,
}

impl Notes {
    fn add(&mut self, note: String) {
        if self.seen.contains(&note) {
            return;
        }
        if self.list.len() >= MAX_NOTES {
            self.overflow = true;
            return;
        }
        self.seen.insert(note.clone());
        self.list.push(note);
    }

    fn finish(mut self) -> Vec<String> {
        if self.overflow {
            self.list.push("(further notes omitted)".to_string());
        }
        self.list
    }
}

// ---------------------------------------------------------------------------
// Reading the config
// ---------------------------------------------------------------------------

/// Decode a config to UTF-8. A byte order mark decides; without one, a NUL in the second byte
/// means UTF-16 (a config starts with `<`), and anything else is UTF-8, falling back to Latin-1
/// when the bytes are not valid UTF-8 (hand-edited configs exist).
fn decode_document(bytes: &[u8]) -> Result<String, FomodError> {
    fn utf16(bytes: &[u8], le: bool) -> Result<String, FomodError> {
        if !bytes.len().is_multiple_of(2) {
            return Err(FomodError::Encoding(
                "UTF-16 document has an odd number of bytes".into(),
            ));
        }
        let units = bytes.as_chunks::<2>().0.iter().map(|c| {
            if le {
                u16::from_le_bytes([c[0], c[1]])
            } else {
                u16::from_be_bytes([c[0], c[1]])
            }
        });
        char::decode_utf16(units)
            .collect::<Result<String, _>>()
            .map_err(|_| FomodError::Encoding("invalid UTF-16".into()))
    }

    let text = if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8(rest.to_vec())
            .map_err(|_| FomodError::Encoding("invalid UTF-8 after the byte order mark".into()))?
    } else if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        utf16(rest, true)?
    } else if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        utf16(rest, false)?
    } else if bytes.len() >= 2 && bytes[0] != 0 && bytes[1] == 0 {
        utf16(bytes, true)?
    } else if bytes.len() >= 2 && bytes[0] == 0 && bytes[1] != 0 {
        utf16(bytes, false)?
    } else {
        match std::str::from_utf8(bytes) {
            Ok(s) => s.to_string(),
            Err(_) => bytes.iter().map(|&b| b as char).collect(),
        }
    };
    Ok(text.trim_start_matches('\u{feff}').to_string())
}

type Node<'a, 'i> = roxmltree::Node<'a, 'i>;

fn el_name(node: Node) -> String {
    node.tag_name().name().to_ascii_lowercase()
}

/// `<group name="Armor">`, for error messages.
fn describe_el(node: Node) -> String {
    let tag = node.tag_name().name();
    match attr(node, "name").or_else(|| attr(node, "source")) {
        Some(n) => {
            let key = if attr(node, "name").is_some() {
                "name"
            } else {
                "source"
            };
            format!("<{tag} {key}=\"{n}\">")
        }
        None => format!("<{tag}>"),
    }
}

fn attr<'a>(node: Node<'a, '_>, name: &str) -> Option<&'a str> {
    node.attributes()
        .find(|a| a.name().eq_ignore_ascii_case(name))
        .map(|a| a.value())
}

fn req_attr<'a>(node: Node<'a, '_>, name: &str) -> Result<&'a str, FomodError> {
    attr(node, name).ok_or_else(|| FomodError::Invalid {
        element: describe_el(node),
        reason: format!("missing the '{name}' attribute"),
    })
}

fn children_named<'a, 'i: 'a>(
    node: Node<'a, 'i>,
    name: &'a str,
) -> impl Iterator<Item = Node<'a, 'i>> + 'a {
    node.children()
        .filter(move |c| c.is_element() && c.tag_name().name().eq_ignore_ascii_case(name))
}

fn child_named<'a, 'i: 'a>(node: Node<'a, 'i>, name: &'a str) -> Option<Node<'a, 'i>> {
    children_named(node, name).next()
}

fn text_of(node: Node) -> String {
    node.children()
        .filter(|c| c.is_text())
        .filter_map(|c| c.text())
        .collect::<String>()
        .trim()
        .to_string()
}

fn parse_bool(node: Node, name: &str) -> Result<bool, FomodError> {
    match attr(node, name) {
        None => Ok(false),
        Some(v) => match v.trim().to_ascii_lowercase().as_str() {
            "true" | "1" => Ok(true),
            "false" | "0" | "" => Ok(false),
            other => Err(FomodError::Invalid {
                element: describe_el(node),
                reason: format!("'{name}' is '{other}', expected true or false"),
            }),
        },
    }
}

/// A path from the config, as a `/`-separated string with empty and `.` components dropped (the
/// format is full of `Folder\` and `.\Folder`). A `..` component, a drive letter or anything the
/// content store's path rules refuse is an error naming the element.
fn fomod_path(node: Node, raw: &str) -> Result<String, FomodError> {
    let unsafe_path = |reason: String| FomodError::UnsafePath {
        element: describe_el(node),
        path: raw.to_string(),
        reason,
    };
    let comps: Vec<&str> = raw
        .split(['\\', '/'])
        .filter(|c| !c.is_empty() && *c != ".")
        .collect();
    if comps.contains(&"..") {
        return Err(unsafe_path("it leaves the installer's folder".into()));
    }
    let joined = comps.join("/");
    if joined.is_empty() {
        return Ok(joined);
    }
    content_store::validate_entry_path(&joined)
        .map(|p| p.as_str().to_string())
        .map_err(|e| unsafe_path(e.to_string()))
}

fn parse_order<'a>(node: Node<'a, '_>) -> Result<Order, FomodError> {
    match attr(node, "order") {
        None => Ok(Order::Explicit),
        Some(v) => match v.trim().to_ascii_lowercase().as_str() {
            "explicit" | "" => Ok(Order::Explicit),
            "ascending" => Ok(Order::Ascending),
            "descending" => Ok(Order::Descending),
            other => Err(FomodError::Invalid {
                element: describe_el(node),
                reason: format!("unknown order '{other}'"),
            }),
        },
    }
}

#[derive(Clone, Copy)]
enum Order {
    Explicit,
    Ascending,
    Descending,
}

fn sort_by_order<T>(items: &mut [T], order: Order, name: impl Fn(&T) -> &str) {
    match order {
        Order::Explicit => {}
        Order::Ascending => items.sort_by_key(|i| name(i).to_lowercase()),
        Order::Descending => {
            items.sort_by_key(|i| std::cmp::Reverse(name(i).to_lowercase()));
        }
    }
}

#[derive(Default)]
struct Budget {
    plugins: usize,
    entries: usize,
    conditions: usize,
    patterns: usize,
}

impl Budget {
    fn count(slot: &mut usize, limit: usize, what: &'static str) -> Result<(), FomodError> {
        *slot += 1;
        if *slot > limit {
            return Err(FomodError::TooMany { what, limit });
        }
        Ok(())
    }
}

fn parse_condition(node: Node, b: &mut Budget) -> Result<Condition, FomodError> {
    let all = match attr(node, "operator") {
        None => true,
        Some(op) => match op.trim().to_ascii_lowercase().as_str() {
            "and" | "" => true,
            "or" => false,
            other => {
                return Err(FomodError::Invalid {
                    element: describe_el(node),
                    reason: format!("unknown operator '{other}'"),
                })
            }
        },
    };
    let mut conditions = Vec::new();
    for c in node.children().filter(|c| c.is_element()) {
        Budget::count(&mut b.conditions, MAX_CONDITIONS, "conditions")?;
        let name = el_name(c);
        let cond = match name.as_str() {
            "dependencies" | "compositedependency" => parse_condition(c, b)?,
            "flagdependency" => Condition::Flag {
                flag: req_attr(c, "flag")?.trim().to_string(),
                value: attr(c, "value").unwrap_or("").trim().to_string(),
            },
            "filedependency" => {
                let state = req_attr(c, "state")?;
                Condition::File {
                    file: norm_path(req_attr(c, "file")?),
                    state: FileState::parse(state).ok_or_else(|| FomodError::Invalid {
                        element: describe_el(c),
                        reason: format!("unknown state '{state}'"),
                    })?,
                }
            }
            other if other.ends_with("dependency") => Condition::Version {
                kind: other.trim_end_matches("dependency").to_string(),
                version: attr(c, "version").unwrap_or("").to_string(),
            },
            other => {
                return Err(FomodError::Invalid {
                    element: format!("<{}>", c.tag_name().name()),
                    reason: format!("'{other}' is not a dependency"),
                })
            }
        };
        conditions.push(cond);
    }
    Ok(if all {
        Condition::All { conditions }
    } else {
        Condition::Any { conditions }
    })
}

fn parse_entries(node: Node, b: &mut Budget) -> Result<Vec<InstallEntry>, FomodError> {
    let mut out = Vec::new();
    for c in node.children().filter(|c| c.is_element()) {
        let name = el_name(c);
        let kind = match name.as_str() {
            "file" => EntryKind::File,
            "folder" => EntryKind::Folder,
            other => {
                return Err(FomodError::Invalid {
                    element: format!("<{}>", c.tag_name().name()),
                    reason: format!("expected <file> or <folder>, found '{other}'"),
                })
            }
        };
        Budget::count(&mut b.entries, MAX_ENTRIES, "files and folders")?;
        let source = fomod_path(c, req_attr(c, "source")?)?;
        if kind == EntryKind::File && source.is_empty() {
            return Err(FomodError::Invalid {
                element: describe_el(c),
                reason: "a file needs a source".into(),
            });
        }
        let destination = match attr(c, "destination") {
            None => source.clone(),
            Some(raw) => {
                let dest = fomod_path(c, raw)?;
                let names_folder = raw.is_empty() || raw.ends_with(['\\', '/']);
                if kind == EntryKind::File && names_folder {
                    join_path(&dest, basename(&source))
                } else {
                    dest
                }
            }
        };
        let priority = match attr(c, "priority") {
            None => 0,
            Some(p) if p.trim().is_empty() => 0,
            Some(p) => p.trim().parse::<i32>().map_err(|_| FomodError::Invalid {
                element: describe_el(c),
                reason: format!("priority '{p}' is not a number"),
            })?,
        };
        out.push(InstallEntry {
            kind,
            source,
            destination,
            always_install: parse_bool(c, "alwaysInstall")?,
            install_if_usable: parse_bool(c, "installIfUsable")?,
            priority,
        });
    }
    Ok(out)
}

fn plugin_type_named(node: Node) -> Result<PluginType, FomodError> {
    let name = req_attr(node, "name")?;
    PluginType::parse(name).ok_or_else(|| FomodError::Invalid {
        element: describe_el(node),
        reason: format!("unknown plugin type '{name}'"),
    })
}

fn parse_type_descriptor(node: Option<Node>, b: &mut Budget) -> Result<TypeDescriptor, FomodError> {
    let Some(node) = node else {
        return Ok(TypeDescriptor::Simple {
            plugin_type: PluginType::Optional,
        });
    };
    if let Some(t) = child_named(node, "type") {
        return Ok(TypeDescriptor::Simple {
            plugin_type: plugin_type_named(t)?,
        });
    }
    if let Some(dt) = child_named(node, "dependencyType") {
        let default = match child_named(dt, "defaultType") {
            Some(d) => plugin_type_named(d)?,
            None => PluginType::Optional,
        };
        let mut patterns = Vec::new();
        if let Some(ps) = child_named(dt, "patterns") {
            for p in children_named(ps, "pattern") {
                Budget::count(&mut b.patterns, MAX_PATTERNS, "patterns")?;
                let cond = match child_named(p, "dependencies") {
                    Some(d) => parse_condition(d, b)?,
                    None => Condition::All { conditions: vec![] },
                };
                let t = child_named(p, "type").ok_or_else(|| FomodError::Invalid {
                    element: "<pattern>".into(),
                    reason: "missing <type>".into(),
                })?;
                patterns.push((cond, plugin_type_named(t)?));
            }
        }
        return Ok(TypeDescriptor::Dependent { default, patterns });
    }
    Ok(TypeDescriptor::Simple {
        plugin_type: PluginType::Optional,
    })
}

fn parse_plugin(node: Node, b: &mut Budget) -> Result<Plugin, FomodError> {
    Budget::count(&mut b.plugins, MAX_PLUGINS, "plugins")?;
    let name = req_attr(node, "name")?.trim().to_string();
    let description = child_named(node, "description")
        .map(text_of)
        .unwrap_or_default();
    let mut image = None;
    if let Some(img) = child_named(node, "image") {
        if let Some(p) = attr(img, "path") {
            let path = fomod_path(img, p)?;
            image = (!path.is_empty()).then_some(path);
        }
    }
    let mut files = Vec::new();
    for f in children_named(node, "files") {
        files.extend(parse_entries(f, b)?);
    }
    let mut flags = Vec::new();
    for cf in children_named(node, "conditionFlags") {
        for f in children_named(cf, "flag") {
            flags.push(FlagSetting {
                name: req_attr(f, "name")?.trim().to_string(),
                value: text_of(f),
            });
        }
    }
    let type_descriptor = parse_type_descriptor(child_named(node, "typeDescriptor"), b)?;
    Ok(Plugin {
        name,
        description,
        image,
        files,
        flags,
        type_descriptor,
    })
}

fn parse_group(node: Node, b: &mut Budget) -> Result<Group, FomodError> {
    let name = req_attr(node, "name")?.trim().to_string();
    let type_name = req_attr(node, "type")?;
    let group_type = GroupType::parse(type_name).ok_or_else(|| FomodError::Invalid {
        element: describe_el(node),
        reason: format!("unknown group type '{type_name}'"),
    })?;
    let mut plugins = Vec::new();
    for ps in children_named(node, "plugins") {
        let order = parse_order(ps)?;
        let mut these = Vec::new();
        for p in children_named(ps, "plugin") {
            if these.len() >= MAX_PLUGINS_PER_GROUP {
                return Err(FomodError::TooMany {
                    what: "plugins in one group",
                    limit: MAX_PLUGINS_PER_GROUP,
                });
            }
            these.push(parse_plugin(p, b)?);
        }
        sort_by_order(&mut these, order, |p| p.name.as_str());
        plugins.extend(these);
    }
    if plugins.len() > MAX_PLUGINS_PER_GROUP {
        return Err(FomodError::TooMany {
            what: "plugins in one group",
            limit: MAX_PLUGINS_PER_GROUP,
        });
    }
    let mut seen = HashSet::new();
    for p in &plugins {
        if !seen.insert(lc(&p.name)) {
            return Err(FomodError::Invalid {
                element: describe_el(node),
                reason: format!(
                    "two plugins are named '{}', so a choice could not tell them apart",
                    p.name
                ),
            });
        }
    }
    Ok(Group {
        name,
        group_type,
        plugins,
    })
}

fn parse_step(node: Node, b: &mut Budget) -> Result<Step, FomodError> {
    let name = req_attr(node, "name")?.trim().to_string();
    let visible = match child_named(node, "visible") {
        Some(v) => Some(parse_condition(v, b)?),
        None => None,
    };
    let mut groups = Vec::new();
    for gs in children_named(node, "optionalFileGroups") {
        let order = parse_order(gs)?;
        let mut these = Vec::new();
        for g in children_named(gs, "group") {
            if groups.len() + these.len() >= MAX_GROUPS_PER_STEP {
                return Err(FomodError::TooMany {
                    what: "groups in one step",
                    limit: MAX_GROUPS_PER_STEP,
                });
            }
            these.push(parse_group(g, b)?);
        }
        sort_by_order(&mut these, order, |g| g.name.as_str());
        groups.extend(these);
    }
    let mut seen = HashSet::new();
    for g in &groups {
        if !seen.insert(lc(&g.name)) {
            return Err(FomodError::Invalid {
                element: describe_el(node),
                reason: format!(
                    "two groups are named '{}', so a choice could not tell them apart",
                    g.name
                ),
            });
        }
    }
    Ok(Step {
        name,
        visible,
        groups,
    })
}

fn check_depth(doc: &roxmltree::Document) -> Result<(), FomodError> {
    // `descendants` is in document order, so a node's parent has been seen before it.
    let mut depth_of: HashMap<roxmltree::NodeId, usize> = HashMap::new();
    for node in doc.descendants() {
        let depth = match node.parent() {
            Some(p) => depth_of.get(&p.id()).copied().unwrap_or(0) + 1,
            None => 0,
        };
        if node.is_element() && depth > MAX_DEPTH {
            return Err(FomodError::TooDeep { limit: MAX_DEPTH });
        }
        depth_of.insert(node.id(), depth);
    }
    Ok(())
}

fn decode_checked(bytes: &[u8]) -> Result<String, FomodError> {
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(FomodError::TooLarge {
            what: "ModuleConfig.xml".into(),
            size: bytes.len() as u64,
            limit: MAX_DOCUMENT_BYTES,
        });
    }
    decode_document(bytes)
}

fn open_document(text: &str) -> Result<roxmltree::Document<'_>, FomodError> {
    let options = roxmltree::ParsingOptions {
        allow_dtd: false,
        nodes_limit: MAX_NODES,
    };
    let doc = roxmltree::Document::parse_with_options(text, options)
        .map_err(|e| FomodError::Xml(e.to_string()))?;
    check_depth(&doc)?;
    Ok(doc)
}

fn parse_info(bytes: &[u8]) -> Option<FomodInfo> {
    let text = decode_document(bytes).ok()?;
    let doc = open_document(&text).ok()?;
    let root = doc.root_element();
    let get = |name: &str| {
        child_named(root, name)
            .map(text_of)
            .filter(|s| !s.is_empty())
    };
    Some(FomodInfo {
        name: get("Name"),
        author: get("Author"),
        version: get("Version"),
        website: get("Website"),
        description: get("Description"),
    })
}

/// Parse a `ModuleConfig.xml` already in memory. `files` is the item's file list (used later to
/// expand folders).
fn parse_installer(
    bytes: &[u8],
    item_id: &str,
    root: &str,
    files: Vec<ContentFile>,
    info: Option<FomodInfo>,
) -> Result<FomodInstaller, FomodError> {
    let text = decode_checked(bytes)?;
    let doc = open_document(&text)?;
    let config = doc.root_element();
    if !config.tag_name().name().eq_ignore_ascii_case("config") {
        return Err(FomodError::Invalid {
            element: format!("<{}>", config.tag_name().name()),
            reason: "the document's root element must be <config>".into(),
        });
    }
    let mut b = Budget::default();

    let module_name = child_named(config, "moduleName")
        .map(text_of)
        .unwrap_or_default();
    let module_dependencies = match child_named(config, "moduleDependencies") {
        Some(d) => Some(parse_condition(d, &mut b)?),
        None => None,
    };
    let mut required_files = Vec::new();
    for r in children_named(config, "requiredInstallFiles") {
        required_files.extend(parse_entries(r, &mut b)?);
    }

    let mut steps = Vec::new();
    for ss in children_named(config, "installSteps") {
        let order = parse_order(ss)?;
        let mut these = Vec::new();
        for s in children_named(ss, "installStep") {
            if steps.len() + these.len() >= MAX_STEPS {
                return Err(FomodError::TooMany {
                    what: "steps",
                    limit: MAX_STEPS,
                });
            }
            these.push(parse_step(s, &mut b)?);
        }
        sort_by_order(&mut these, order, |s| s.name.as_str());
        steps.extend(these);
    }
    let mut seen = HashSet::new();
    for s in &steps {
        if !seen.insert(lc(&s.name)) {
            return Err(FomodError::Invalid {
                element: "<installSteps>".into(),
                reason: format!(
                    "two steps are named '{}', so a choice could not tell them apart",
                    s.name
                ),
            });
        }
    }

    let mut conditional_installs = Vec::new();
    for ci in children_named(config, "conditionalFileInstalls") {
        for ps in children_named(ci, "patterns") {
            for p in children_named(ps, "pattern") {
                Budget::count(&mut b.patterns, MAX_PATTERNS, "patterns")?;
                let condition = match child_named(p, "dependencies") {
                    Some(d) => parse_condition(d, &mut b)?,
                    None => Condition::All { conditions: vec![] },
                };
                let mut files = Vec::new();
                for f in children_named(p, "files") {
                    files.extend(parse_entries(f, &mut b)?);
                }
                conditional_installs.push(ConditionalInstall { condition, files });
            }
        }
    }

    Ok(FomodInstaller {
        item_id: item_id.to_string(),
        root: root.to_string(),
        module_name,
        info,
        module_dependencies,
        required_files,
        steps,
        conditional_installs,
        files,
    })
}

/// Read a stored object, checking its size and hash on the way.
fn read_object(
    ctx: &Ctx,
    file: &ContentFile,
    limit: usize,
    what: &str,
) -> Result<Vec<u8>, FomodError> {
    if file.size > limit as u64 {
        return Err(FomodError::TooLarge {
            what: what.to_string(),
            size: file.size,
            limit,
        });
    }
    let path = ctx.paths.content_object_path(&file.sha256);
    let f = std::fs::File::open(&path).map_err(ContentError::from)?;
    let mut bytes = Vec::with_capacity(file.size as usize);
    f.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(ContentError::from)?;
    if bytes.len() as u64 != file.size || format!("{:x}", Sha256::digest(&bytes)) != file.sha256 {
        return Err(FomodError::Other(format!(
            "the stored object for '{}' does not match its recorded hash; verify the item",
            file.path
        )));
    }
    Ok(bytes)
}

/// The installer inside an item: the shallowest `fomod/ModuleConfig.xml` (case-insensitive), and
/// the folder it sits in.
fn find_config(files: &[ContentFile]) -> Option<(String, &ContentFile)> {
    const SUFFIX: &str = "fomod/moduleconfig.xml";
    let mut best: Option<(usize, String, &ContentFile)> = None;
    for f in files {
        let lower = lc(f.path.as_str());
        let Some(prefix) = lower.strip_suffix(SUFFIX) else {
            continue;
        };
        if !(prefix.is_empty() || prefix.ends_with('/')) {
            continue;
        }
        let root = f.path.as_str()[..prefix.len()]
            .trim_end_matches('/')
            .to_string();
        let depth = if root.is_empty() {
            0
        } else {
            root.split('/').count()
        };
        let better = match &best {
            None => true,
            Some((d, r, _)) => (depth, &root) < (*d, r),
        };
        if better {
            best = Some((depth, root, f));
        }
    }
    best.map(|(_, root, f)| (root, f))
}

/// Read the FOMOD installer of an archive item from its stored objects.
pub fn parse(ctx: &Ctx, item_id: &str) -> Result<FomodInstaller, FomodError> {
    let item = content_store::get_item(ctx, item_id)?;
    parse_item(ctx, &item)
}

fn parse_item(ctx: &Ctx, item: &ContentItem) -> Result<FomodInstaller, FomodError> {
    let (root, config) =
        find_config(&item.files).ok_or_else(|| FomodError::NoInstaller(item.item_id.clone()))?;
    let bytes = read_object(ctx, config, MAX_DOCUMENT_BYTES, "ModuleConfig.xml")?;

    let info_key = join_path(&root, "fomod/info.xml");
    let info = item
        .files
        .iter()
        .find(|f| f.path.as_str().eq_ignore_ascii_case(&info_key))
        .and_then(|f| read_object(ctx, f, MAX_INFO_BYTES, "info.xml").ok())
        .and_then(|b| parse_info(&b));

    parse_installer(&bytes, &item.item_id, &root, item.files.clone(), info)
}

// ---------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------

type Flags = HashMap<String, String>;

fn eval_condition(
    cond: &Condition,
    flags: &Flags,
    context: Option<&FomodContext>,
    notes: &mut Notes,
) -> bool {
    match cond {
        Condition::All { conditions } => conditions
            .iter()
            .all(|c| eval_condition(c, flags, context, notes)),
        Condition::Any { conditions } => {
            conditions.is_empty()
                || conditions
                    .iter()
                    .any(|c| eval_condition(c, flags, context, notes))
        }
        Condition::Flag { flag, value } => flags
            .get(&lc(flag))
            .map(String::as_str)
            .unwrap_or("")
            .eq_ignore_ascii_case(value),
        Condition::File { file, state } => match context {
            Some(c) => c.state_of(file) == *state,
            None => {
                notes.add(format!(
                    "the installer checks whether '{file}' is {state:?}, which cannot be known \
                     without an instance (use --instance); treated as not satisfied"
                ));
                false
            }
        },
        Condition::Version { kind, version } => {
            notes.add(format!(
                "the installer requires {kind} version {version}; Agora does not check it, so it \
                 is treated as satisfied"
            ));
            true
        }
    }
}

fn resolve_type(
    td: &TypeDescriptor,
    flags: &Flags,
    context: Option<&FomodContext>,
    notes: &mut Notes,
) -> PluginType {
    match td {
        TypeDescriptor::Simple { plugin_type } => *plugin_type,
        TypeDescriptor::Dependent { default, patterns } => {
            for (cond, t) in patterns {
                if eval_condition(cond, flags, context, notes) {
                    return *t;
                }
            }
            *default
        }
    }
}

type Preset = HashMap<(String, String), Vec<String>>;

fn build_preset(inst: &FomodInstaller, choices: &[Choice]) -> Result<Preset, FomodError> {
    let mut preset = Preset::new();
    for c in choices {
        let step = inst
            .steps
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(&c.step))
            .ok_or_else(|| FomodError::UnknownStep(c.step.clone()))?;
        let group = step
            .groups
            .iter()
            .find(|g| g.name.eq_ignore_ascii_case(&c.group))
            .ok_or_else(|| FomodError::UnknownGroup {
                step: step.name.clone(),
                group: c.group.clone(),
            })?;
        preset
            .entry((lc(&step.name), lc(&group.name)))
            .or_default()
            .extend(c.plugins.iter().cloned());
    }
    Ok(preset)
}

struct GroupSelection {
    step: usize,
    group: usize,
    selected: Vec<usize>,
    types: Vec<PluginType>,
}

struct Walk {
    groups: Vec<GroupSelection>,
    flags: Flags,
}

/// The picks a group gets when nobody chose: every Required and Recommended plugin; a group that
/// takes at most one keeps only the first; a group that needs one and has none takes its first
/// usable plugin.
fn default_picks(kind: GroupType, types: &[PluginType]) -> Vec<usize> {
    let usable = |t: &PluginType| *t != PluginType::NotUsable;
    let mut picks: Vec<usize> = types
        .iter()
        .enumerate()
        .filter(|(_, t)| matches!(t, PluginType::Required | PluginType::Recommended))
        .map(|(i, _)| i)
        .collect();
    match kind {
        GroupType::SelectExactlyOne | GroupType::SelectAtMostOne => {
            let first_required = types.iter().position(|t| *t == PluginType::Required);
            picks = match (first_required, picks.first()) {
                (Some(r), _) => vec![r],
                (None, Some(&p)) => vec![p],
                _ => vec![],
            };
            if picks.is_empty() && kind == GroupType::SelectExactlyOne {
                picks.extend(types.iter().position(usable));
            }
        }
        GroupType::SelectAtLeastOne => {
            if picks.is_empty() {
                picks.extend(types.iter().position(usable));
            }
        }
        GroupType::SelectAll | GroupType::SelectAny => {}
    }
    picks
}

/// Walk the steps in order, deciding which are visible, what each group selects and which flags
/// that sets. `preset` names the plugins the user chose per group; with `fill_defaults`, groups
/// the preset does not mention take their defaults; with `validate`, every group must satisfy its
/// type and no NotUsable plugin may be chosen.
fn walk(
    inst: &FomodInstaller,
    context: Option<&FomodContext>,
    preset: &Preset,
    fill_defaults: bool,
    validate: bool,
    notes: &mut Notes,
) -> Result<Walk, FomodError> {
    let mut flags = Flags::new();
    let mut groups = Vec::new();
    for (si, step) in inst.steps.iter().enumerate() {
        let visible = match &step.visible {
            None => true,
            Some(c) => eval_condition(c, &flags, context, notes),
        };
        if !visible {
            for g in &step.groups {
                if preset
                    .get(&(lc(&step.name), lc(&g.name)))
                    .is_some_and(|v| !v.is_empty())
                {
                    notes.add(format!(
                        "step '{}' is hidden by the options chosen before it, so the choices for \
                         group '{}' were ignored",
                        step.name, g.name
                    ));
                }
            }
            continue;
        }
        for (gi, group) in step.groups.iter().enumerate() {
            let types: Vec<PluginType> = group
                .plugins
                .iter()
                .map(|p| resolve_type(&p.type_descriptor, &flags, context, notes))
                .collect();
            let mut selected: BTreeSet<usize> = BTreeSet::new();
            match preset.get(&(lc(&step.name), lc(&group.name))) {
                Some(names) => {
                    for n in names {
                        let idx = group
                            .plugins
                            .iter()
                            .position(|p| p.name.eq_ignore_ascii_case(n))
                            .ok_or_else(|| FomodError::UnknownPlugin {
                                step: step.name.clone(),
                                group: group.name.clone(),
                                plugin: n.clone(),
                            })?;
                        selected.insert(idx);
                    }
                }
                None if fill_defaults => {
                    selected.extend(default_picks(group.group_type, &types));
                }
                None => {}
            }
            for (i, t) in types.iter().enumerate() {
                if *t == PluginType::Required {
                    selected.insert(i);
                }
            }
            if group.group_type == GroupType::SelectAll {
                for (i, t) in types.iter().enumerate() {
                    if *t == PluginType::NotUsable {
                        notes.add(format!(
                            "'{}/{}/{}' is not usable here, so it was left out of a group that \
                             selects everything",
                            step.name, group.name, group.plugins[i].name
                        ));
                    } else {
                        selected.insert(i);
                    }
                }
            }
            if validate {
                for &i in &selected {
                    if types[i] == PluginType::NotUsable {
                        return Err(FomodError::NotUsable {
                            step: step.name.clone(),
                            group: group.name.clone(),
                            plugin: group.plugins[i].name.clone(),
                        });
                    }
                }
                let n = selected.len();
                let ok = match group.group_type {
                    GroupType::SelectExactlyOne => n == 1,
                    GroupType::SelectAtMostOne => n <= 1,
                    GroupType::SelectAtLeastOne => n >= 1,
                    GroupType::SelectAll | GroupType::SelectAny => true,
                };
                if !ok {
                    return Err(FomodError::GroupRule {
                        step: step.name.clone(),
                        group: group.name.clone(),
                        rule: group.group_type.rule(),
                        chosen: n,
                    });
                }
            }
            for &i in &selected {
                for f in &group.plugins[i].flags {
                    flags.insert(lc(&f.name), f.value.clone());
                }
            }
            groups.push(GroupSelection {
                step: si,
                group: gi,
                selected: selected.into_iter().collect(),
                types,
            });
        }
    }
    Ok(Walk { groups, flags })
}

fn choices_of(inst: &FomodInstaller, walk: &Walk) -> Vec<Choice> {
    walk.groups
        .iter()
        .filter(|g| !g.selected.is_empty())
        .map(|g| {
            let step = &inst.steps[g.step];
            let group = &step.groups[g.group];
            Choice {
                step: step.name.clone(),
                group: group.name.clone(),
                plugins: g
                    .selected
                    .iter()
                    .map(|&i| group.plugins[i].name.clone())
                    .collect(),
            }
        })
        .collect()
}

/// Choices for every option the installer marks Required or Recommended, and for a group that
/// needs a selection and has no recommendation, its first usable plugin. Steps are walked in
/// order, so a step an earlier default hides gets none.
pub fn defaults(inst: &FomodInstaller, context: Option<&FomodContext>) -> Vec<Choice> {
    defaults_over(inst, &[], context).unwrap_or_default()
}

/// Like [`defaults`], but the groups `explicit` mentions keep exactly what it chose, and the rest
/// take their defaults (a step the explicit choices hide gets none, one they reveal gets its
/// defaults). The result is not validated; [`evaluate`] does that.
pub fn defaults_over(
    inst: &FomodInstaller,
    explicit: &[Choice],
    context: Option<&FomodContext>,
) -> Result<Vec<Choice>, FomodError> {
    let preset = build_preset(inst, explicit)?;
    let mut notes = Notes::default();
    let w = walk(inst, context, &preset, true, false, &mut notes)?;
    Ok(choices_of(inst, &w))
}

struct Candidate {
    destination: String,
    source: usize,
    priority: i32,
    origin: String,
}

struct FileIndex<'a> {
    files: &'a [ContentFile],
    by_key: BTreeMap<String, usize>,
}

impl<'a> FileIndex<'a> {
    fn new(files: &'a [ContentFile]) -> Self {
        let by_key = files
            .iter()
            .enumerate()
            .map(|(i, f)| (lc(f.path.as_str()), i))
            .collect();
        Self { files, by_key }
    }
}

fn expand_entry(
    entry: &InstallEntry,
    root: &str,
    index: &FileIndex,
    origin: &str,
    out: &mut Vec<Candidate>,
    notes: &mut Notes,
) -> Result<(), FomodError> {
    let full = join_path(root, &entry.source);
    let full_key = lc(&full);
    let push = |dest: String, source: usize, out: &mut Vec<Candidate>| {
        if out.len() >= MAX_PLAN_CANDIDATES {
            return Err(FomodError::TooMany {
                what: "files in the plan",
                limit: MAX_PLAN_CANDIDATES,
            });
        }
        out.push(Candidate {
            destination: dest,
            source,
            priority: entry.priority,
            origin: origin.to_string(),
        });
        Ok(())
    };

    if !full_key.is_empty() {
        if let Some(&i) = index.by_key.get(&full_key) {
            // A file. (A <folder> pointing at one file is a common authoring slip: it goes into
            // the destination folder under its own name.)
            let dest = match entry.kind {
                EntryKind::File => entry.destination.clone(),
                EntryKind::Folder => join_path(&entry.destination, basename(&entry.source)),
            };
            return push(dest, i, out);
        }
    }
    if entry.kind == EntryKind::File {
        notes.add(format!(
            "{origin}: '{}' is not in the archive, so it was skipped",
            entry.source
        ));
        return Ok(());
    }

    let prefix = if full_key.is_empty() {
        String::new()
    } else {
        format!("{full_key}/")
    };
    let mut any = false;
    // Every key under `prefix` sorts between `prefix` and `prefix` with its last byte bumped.
    let range: Box<dyn Iterator<Item = (&String, &usize)>> = if prefix.is_empty() {
        Box::new(index.by_key.iter())
    } else {
        let mut upper = prefix.clone();
        upper.pop();
        upper.push('0'); // '/' + 1
        Box::new(index.by_key.range(prefix.clone()..upper))
    };
    // The installer's own folder (config, images) is never part of what a folder entry installs,
    // unless the entry points into it on purpose.
    let own_dir = join_path(&lc(root), "fomod");
    let own_prefix = format!("{own_dir}/");
    let skip_own = !(full_key == own_dir || full_key.starts_with(&own_prefix));
    let mut skipped_own = false;
    for (key, &i) in range {
        if skip_own && key.starts_with(&own_prefix) {
            skipped_own = true;
            continue;
        }
        any = true;
        let original = index.files[i].path.as_str();
        let rest = &original[prefix.len()..];
        push(join_path(&entry.destination, rest), i, out)?;
    }
    if skipped_own {
        notes.add(format!(
            "{origin}: folder '{}' covers the installer's own fomod folder, which is not installed",
            entry.source
        ));
    }
    if !any && !skipped_own {
        notes.add(format!(
            "{origin}: folder '{}' holds no files in the archive, so it was skipped",
            entry.source
        ));
    }
    Ok(())
}

/// Turn the user's choices into the files to install.
///
/// Steps are walked in order; a step whose `visible` condition fails is skipped and its choices
/// are ignored (with a note). Each group must satisfy its type, Required plugins are always
/// included, and NotUsable ones cannot be chosen. `context` answers `fileDependency` (`None`
/// means unknown, and every such dependency then fails, with a note).
pub fn evaluate(
    inst: &FomodInstaller,
    choices: &[Choice],
    context: Option<&FomodContext>,
) -> Result<InstallPlan, FomodError> {
    let mut notes = Notes::default();
    let preset = build_preset(inst, choices)?;

    if let Some(deps) = &inst.module_dependencies {
        if !eval_condition(deps, &Flags::new(), context, &mut notes) {
            notes.add(format!(
                "the installer's own requirements are not met ({}); installing anyway",
                deps.describe()
            ));
        }
    }

    let w = walk(inst, context, &preset, false, true, &mut notes)?;

    // Entries in install order: required, then each visible step's plugins, then conditional.
    let mut entries: Vec<(&InstallEntry, String)> = Vec::new();
    for e in &inst.required_files {
        entries.push((e, "required files".to_string()));
    }
    for gs in &w.groups {
        let step = &inst.steps[gs.step];
        let group = &step.groups[gs.group];
        for (pi, plugin) in group.plugins.iter().enumerate() {
            let selected = gs.selected.contains(&pi);
            for e in &plugin.files {
                if selected
                    || e.always_install
                    || (e.install_if_usable && gs.types[pi] != PluginType::NotUsable)
                {
                    entries.push((e, format!("'{}/{}/{}'", step.name, group.name, plugin.name)));
                }
            }
        }
    }
    for ci in &inst.conditional_installs {
        if eval_condition(&ci.condition, &w.flags, context, &mut notes) {
            for e in &ci.files {
                entries.push((e, "a conditional install".to_string()));
            }
        }
    }

    let index = FileIndex::new(&inst.files);
    let mut candidates: Vec<Candidate> = Vec::new();
    for (entry, origin) in entries {
        expand_entry(
            entry,
            &inst.root,
            &index,
            &origin,
            &mut candidates,
            &mut notes,
        )?;
    }

    // The higher priority wins a destination; equal priorities: the later one.
    let mut winners: HashMap<String, Candidate> = HashMap::new();
    for cand in candidates {
        let key = lc(&cand.destination);
        match winners.get(&key) {
            Some(old) if old.priority > cand.priority => {
                if inst.files[old.source].sha256 != inst.files[cand.source].sha256 {
                    notes.add(format!(
                        "'{}' comes from {} and from {}; {} wins by priority",
                        cand.destination, old.origin, cand.origin, old.origin
                    ));
                }
            }
            Some(old) => {
                if inst.files[old.source].sha256 != inst.files[cand.source].sha256 {
                    notes.add(format!(
                        "'{}' comes from {} and from {}; {} wins",
                        cand.destination, old.origin, cand.origin, cand.origin
                    ));
                }
                winners.insert(key, cand);
            }
            None => {
                winners.insert(key, cand);
            }
        }
    }

    let mut files = Vec::with_capacity(winners.len());
    for cand in winners.into_values() {
        let src = &inst.files[cand.source];
        let destination = RelPath::new(&cand.destination).map_err(|e| FomodError::UnsafePath {
            element: cand.origin.clone(),
            path: cand.destination.clone(),
            reason: e.to_string(),
        })?;
        let full = src.path.as_str();
        let relative = if inst.root.is_empty() {
            full
        } else {
            &full[(inst.root.len() + 1).min(full.len())..]
        };
        files.push(PlannedFile {
            destination,
            source: RelPath::new(relative).map_err(|e| FomodError::UnsafePath {
                element: cand.origin.clone(),
                path: relative.to_string(),
                reason: e.to_string(),
            })?,
            sha256: src.sha256.clone(),
            size: src.size,
        });
    }
    files.sort_by(|a, b| a.destination.as_str().cmp(b.destination.as_str()));
    if files.is_empty() {
        notes.add("the chosen options install no files".to_string());
    } else {
        let paths: Vec<RelPath> = files.iter().map(|f| f.destination.clone()).collect();
        content_store::validate_path_set(&paths)?;
    }

    Ok(InstallPlan {
        files,
        choices: choices_of(inst, &w),
        notes: notes.finish(),
    })
}

/// Replace, group by group, the choices in `base` with those in `over`.
pub fn merge_choices(base: Vec<Choice>, over: Vec<Choice>) -> Vec<Choice> {
    let mut merged: Vec<Choice> = Vec::new();
    for o in over {
        match merged.iter_mut().find(|c| {
            c.step.eq_ignore_ascii_case(&o.step) && c.group.eq_ignore_ascii_case(&o.group)
        }) {
            Some(c) => c.plugins.extend(o.plugins),
            None => merged.push(o),
        }
    }
    let mut out: Vec<Choice> = base
        .into_iter()
        .filter(|b| {
            !merged.iter().any(|c| {
                c.step.eq_ignore_ascii_case(&b.step) && c.group.eq_ignore_ascii_case(&b.group)
            })
        })
        .collect();
    out.extend(merged);
    out
}

impl FomodInstaller {
    /// Read `Step/Group/Plugin` (names compared case-insensitively; a name may itself contain
    /// `/`) as a choice of that one plugin.
    pub fn resolve_choice(&self, spec: &str) -> Result<Choice, FomodError> {
        let want = spec.trim().to_ascii_lowercase();
        let mut found: Vec<Choice> = Vec::new();
        for s in &self.steps {
            for g in &s.groups {
                for p in &g.plugins {
                    let full = format!("{}/{}/{}", s.name, g.name, p.name);
                    if full.to_ascii_lowercase() == want {
                        found.push(Choice {
                            step: s.name.clone(),
                            group: g.name.clone(),
                            plugins: vec![p.name.clone()],
                        });
                    }
                }
            }
        }
        match found.len() {
            0 => Err(FomodError::UnknownChoice {
                spec: spec.to_string(),
            }),
            1 => Ok(found.remove(0)),
            _ => Err(FomodError::AmbiguousChoice {
                spec: spec.to_string(),
                matches: found
                    .iter()
                    .map(|c| format!("{}/{}/{}", c.step, c.group, c.plugins[0]))
                    .collect::<Vec<_>>()
                    .join(", "),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Installing
// ---------------------------------------------------------------------------

/// The choices a FOMOD-derived item was installed with (its newest record) and the archive item it
/// came from, so a reinstall can replay them.
pub fn recorded_install(item: &ContentItem) -> Option<(&str, &[Choice])> {
    item.sources.iter().rev().find_map(|s| match s {
        ContentSource::FomodInstall {
            from_item, choices, ..
        } => Some((from_item.as_str(), choices.as_slice())),
        _ => None,
    })
}

/// Evaluate `choices` against the installer of `from_item` and derive a new content item from
/// the plan, sharing the archive's objects. The item's paths are relative to the data folder.
pub fn install(
    ctx: &Ctx,
    from_item: &str,
    choices: &[Choice],
    context: Option<&FomodContext>,
) -> Result<(FomodInstaller, InstallPlan, AddOutcome), FomodError> {
    let installer = parse(ctx, from_item)?;
    let plan = evaluate(&installer, choices, context)?;
    if plan.files.is_empty() {
        return Err(FomodError::NothingToInstall);
    }
    let name = if !installer.module_name.is_empty() {
        installer.module_name.clone()
    } else {
        content_store::get_item(ctx, &installer.item_id)?.name
    };
    let files = plan
        .files
        .iter()
        .map(|f| (f.destination.clone(), f.sha256.clone()))
        .collect();
    let source = ContentSource::FomodInstall {
        from_item: installer.item_id.clone(),
        choices: plan.choices.clone(),
        added_at_unix_ms: content_store::now_unix_ms(),
    };
    let outcome = content_store::derive_item(
        ctx,
        &installer.item_id,
        files,
        &format!("{name} (FOMOD)"),
        source,
    )?;
    Ok((installer, plan, outcome))
}

// ---------------------------------------------------------------------------
// The instance as a context
// ---------------------------------------------------------------------------

/// A file's place relative to the game's data folder, given where its layer mounts it.
fn data_relative(mount: &str, source_path: &str, file: &str, data_path: &str) -> Option<String> {
    let comps = |s: &str| -> Vec<String> {
        s.replace('\\', "/")
            .split('/')
            .filter(|c| !c.is_empty() && *c != ".")
            .map(str::to_string)
            .collect()
    };
    let mut rest = comps(file);
    let src = comps(source_path);
    if !src.is_empty() {
        if rest.len() <= src.len()
            || !src
                .iter()
                .zip(&rest)
                .all(|(a, b)| a.eq_ignore_ascii_case(b))
        {
            return None;
        }
        rest.drain(..src.len());
    }
    let mut full = comps(mount);
    full.extend(rest);
    let data = comps(data_path);
    if !data.is_empty() {
        if full.len() <= data.len()
            || !data
                .iter()
                .zip(&full)
                .all(|(a, b)| a.eq_ignore_ascii_case(b))
        {
            return None;
        }
        full.drain(..data.len());
    }
    Some(full.join("/"))
}

/// What an instance has, for `fileDependency`: the files its enabled layers put in the data
/// folder are Active, those of disabled layers Inactive. The instance's pinned base counts; an
/// unpinned base cannot be listed and is not seen (so a dependency on a master file such as
/// `Skyrim.esm` is only satisfied with a pinned base).
pub fn instance_context(ctx: &Ctx, instance_id: &str) -> Result<FomodContext, FomodError> {
    let manifest = crate::game_instance::get_manifest(ctx, instance_id)
        .map_err(|e| FomodError::Other(e.to_string()))?;
    let data_path = ctx
        .games
        .game(&manifest.game)
        .and_then(|g| g.content_layout.as_ref())
        .map(|l| l.data_path.as_str().to_string())
        .unwrap_or_default();

    let mut out = FomodContext::new();

    // The pinned base sits under every layer and is always on. (An unpinned base cannot be
    // listed from here, so it is not seen.)
    if let agora_game_api::BaseReference::Pinned { id, .. } = &manifest.base {
        let base = std::fs::read_to_string(ctx.paths.base_manifest_path(id))
            .ok()
            .and_then(|text| serde_json::from_str::<crate::game_base::BaseManifest>(&text).ok());
        for f in base.into_iter().flat_map(|b| b.files) {
            if let Some(rel) = data_relative("", "", &f.path, &data_path) {
                out.set(&rel, FileState::Active);
            }
        }
    }

    for layer in manifest.layers.layers() {
        let LayerSource::Content { content } = &layer.source else {
            continue;
        };
        let state = if layer.enabled {
            FileState::Active
        } else {
            FileState::Inactive
        };
        for f in &content_store::get_item(ctx, content)?.files {
            if let Some(rel) = data_relative(
                layer.mount_path.as_str(),
                layer.source_path.as_str(),
                f.path.as_str(),
                &data_path,
            ) {
                out.set(&rel, state);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_relative_applies_source_mount_and_data_path() {
        assert_eq!(
            data_relative("", "Wrapper", "Wrapper/Data/a.esp", "Data").as_deref(),
            Some("a.esp")
        );
        assert_eq!(
            data_relative("Data", "", "sub/b.esp", "data").as_deref(),
            Some("sub/b.esp")
        );
        // Outside the data folder, or outside the layer's source folder: not a data file.
        assert_eq!(data_relative("", "", "skse64_loader.exe", "Data"), None);
        assert_eq!(data_relative("", "Wrapper", "Other/x.esp", ""), None);
        assert_eq!(
            data_relative("", "", "Plain.esp", "").as_deref(),
            Some("Plain.esp")
        );
    }

    #[test]
    fn decodes_each_encoding() {
        let xml = "<config><moduleName>Caf\u{e9}</moduleName></config>";
        let mut le = vec![0xFF, 0xFE];
        le.extend(xml.encode_utf16().flat_map(|u| u.to_le_bytes()));
        let mut be = vec![0xFE, 0xFF];
        be.extend(xml.encode_utf16().flat_map(|u| u.to_be_bytes()));
        let mut bom8 = vec![0xEF, 0xBB, 0xBF];
        bom8.extend(xml.as_bytes());
        let bare_le: Vec<u8> = xml.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        for bytes in [le, be, bom8, bare_le, xml.as_bytes().to_vec()] {
            assert_eq!(decode_document(&bytes).unwrap(), xml);
        }
        // Not UTF-8: read as Latin-1 rather than refused.
        assert_eq!(
            decode_document(b"<a>caf\xe9</a>").unwrap(),
            "<a>caf\u{e9}</a>"
        );
        assert!(decode_document(&[0xFF, 0xFE, 0x3C]).is_err());
    }

    #[test]
    fn plain_file_destination_forms() {
        let xml = r#"<config><requiredInstallFiles>
            <file source="Core\a.esp"/>
            <file source="Core\b.esp" destination=""/>
            <file source="Core\c.esp" destination="textures\sub\"/>
            <file source="Core\d.esp" destination="renamed.esp"/>
            <folder source="Core\f" destination=""/>
            <folder source=".\Core\g\"/>
        </requiredInstallFiles></config>"#;
        let inst = parse_installer(xml.as_bytes(), "id", "", vec![], None).unwrap();
        let d: Vec<(&str, &str)> = inst
            .required_files
            .iter()
            .map(|e| (e.source.as_str(), e.destination.as_str()))
            .collect();
        assert_eq!(
            d,
            vec![
                ("Core/a.esp", "Core/a.esp"),
                ("Core/b.esp", "b.esp"),
                ("Core/c.esp", "textures/sub/c.esp"),
                ("Core/d.esp", "renamed.esp"),
                ("Core/f", ""),
                ("Core/g", "Core/g"),
            ]
        );
    }
}
