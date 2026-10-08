use crate::id::{FrameworkId, GameId, InstallId, LayerId, RelPath, StoreId, ToolId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeIdentity {
    pub game: GameId,
    pub store: StoreId,
    /// Exact opaque version: Minecraft snapshots and four-part PE versions are
    /// not semver. Never compare these lexicographically as a version range.
    pub version: String,
    #[serde(default)]
    pub build: Option<String>,
}

impl RuntimeIdentity {
    pub fn minecraft(version: impl Into<String>) -> Self {
        Self {
            game: GameId::minecraft(),
            store: StoreId::mojang(),
            version: version.into(),
            build: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum VersionConstraint {
    Any,
    Exact(Vec<String>),
    /// Use only when the game's version scheme really is semver.
    Semver(semver::VersionReq),
}

/// The answer to "does this constraint match that runtime?".
///
/// Three-valued:
/// - `Supported`: explicitly verified compatible
/// - `Unsupported`: incompatible (store, version, or build mismatch)
/// - `Indeterminate`: cannot be determined (e.g. unparseable version against semver range, missing build)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Support {
    Supported,
    Unsupported,
    Indeterminate,
}

impl Support {
    pub fn is_supported(&self) -> bool {
        matches!(self, Support::Supported)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeConstraint {
    pub game: GameId,
    /// Empty means any store/build. Multiple constraints are alternatives (OR);
    /// fields within a constraint are conjunctive (AND).
    pub stores: Vec<StoreId>,
    pub versions: VersionConstraint,
    pub builds: Vec<String>,
}

impl RuntimeConstraint {
    pub fn check(&self, runtime: &RuntimeIdentity) -> Support {
        if self.game != runtime.game {
            return Support::Unsupported;
        }
        if !self.stores.is_empty() && !self.stores.contains(&runtime.store) {
            return Support::Unsupported;
        }

        let mut indeterminate = false;

        match &self.versions {
            VersionConstraint::Any => {}
            VersionConstraint::Exact(versions) => {
                if !versions.contains(&runtime.version) {
                    return Support::Unsupported;
                }
            }
            VersionConstraint::Semver(requirement) => {
                match semver::Version::parse(&runtime.version) {
                    Ok(v) => {
                        if !requirement.matches(&v) {
                            return Support::Unsupported;
                        }
                    }
                    Err(_) => {
                        indeterminate = true;
                    }
                }
            }
        }

        if !self.builds.is_empty() {
            match &runtime.build {
                Some(b) => {
                    if !self.builds.contains(b) {
                        return Support::Unsupported;
                    }
                }
                None => {
                    indeterminate = true;
                }
            }
        }

        if indeterminate {
            Support::Indeterminate
        } else {
            Support::Supported
        }
    }

    pub fn matches(&self, runtime: &RuntimeIdentity) -> bool {
        self.check(runtime).is_supported()
    }
}

/// Check a list of alternative constraints: any one supporting the runtime is enough.
/// Empty constraints list means no constraints (supported).
pub fn check_any(constraints: &[RuntimeConstraint], runtime: &RuntimeIdentity) -> Support {
    if constraints.is_empty() {
        return Support::Supported;
    }
    let mut indeterminate = false;
    for constraint in constraints {
        match constraint.check(runtime) {
            Support::Supported => return Support::Supported,
            Support::Indeterminate => indeterminate = true,
            Support::Unsupported => {}
        }
    }
    if indeterminate {
        Support::Indeterminate
    } else {
        Support::Unsupported
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallKind {
    BaseGame,
    AddOn,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallCapabilities {
    pub executables_readable: bool,
    pub accepts_new_files: bool,
    pub relocatable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeInfo {
    pub id: String,
    pub filesystem: String,
    pub supports_hardlinks: bool,
    pub supports_file_clones: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameInstall {
    pub id: InstallId,
    pub runtime: RuntimeIdentity,
    pub kind: InstallKind,
    /// Display/discovery data only; services take an InstallId and a relative
    /// path, never trust this absolute path as permission to access the disk.
    pub location: String,
    /// `None` when the volume could not be inspected. Unknown is never the
    /// same volume as another install, so it never qualifies for hardlinks.
    pub volume: Option<VolumeInfo>,
    pub capabilities: InstallCapabilities,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreIdentifier {
    pub store: StoreId,
    pub product: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VersionSource {
    Executable { path: RelPath },
    StoreRecord,
    PackageBehaviour,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentStrategy {
    Redirect,
    VirtualFileSystem,
}

/// Journaled swaps supplement Redirect; they are not an alternative way to
/// deploy the game's mods. Core holds a game/store resource lock for the session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserFileStrategy {
    Redirect,
    VirtualFileSystem,
    JournaledSwap,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserFileMapping {
    pub source: GamePath,
    pub instance_path: RelPath,
    pub strategy: UserFileStrategy,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stores: Vec<StoreId>,
}

impl UserFileMapping {
    pub fn new(source: GamePath, instance_path: RelPath, strategy: UserFileStrategy) -> Self {
        Self {
            source,
            instance_path,
            strategy,
            stores: Vec::new(),
        }
    }

    pub fn with_stores(mut self, stores: Vec<StoreId>) -> Self {
        self.stores = stores;
        self
    }

    pub fn applies_to_store(&self, store: &StoreId) -> bool {
        self.stores.is_empty() || self.stores.contains(store)
    }
}

/// Host-resolved roots. Every `path` is relative to its root and checked by core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "root", rename_all = "snake_case")]
pub enum GamePath {
    /// The calling instance's selected runtime, resolved by the host to its
    /// pinned base or unpinned install through the deployment's visible stack.
    /// Declarative definitions can name this root before an install is chosen.
    Runtime {
        #[serde(default)]
        path: RelPath,
    },
    Install {
        install: InstallId,
        path: RelPath,
    },
    Base {
        base: String,
        path: RelPath,
    },
    Instance {
        path: RelPath,
    },
    Layer {
        layer: LayerId,
        path: RelPath,
    },
    RuntimeComponent {
        component: String,
        path: RelPath,
    },
    /// A downloaded file, including a jar on a Minecraft launch classpath.
    /// The artifact must belong to the calling package's host scope.
    Artifact {
        artifact: String,
    },
    /// Known user folders are resolved by the host, including when a store
    /// install is unreadable or its saves/logs live outside the game folder.
    UserData {
        location: UserDataLocation,
        path: RelPath,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserDataLocation {
    Documents,
    RoamingAppData,
    LocalAppData,
    Home,
}

/// Recipes refer to managed paths without inventing a string interpolation
/// language or asking a package for absolute host paths. Core resolves paths
/// and joins path lists with the platform separator (e.g. Java's classpath).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LaunchValue {
    Literal {
        value: String,
    },
    Path {
        path: GamePath,
        prefix: String,
        suffix: String,
    },
    PathList {
        paths: Vec<GamePath>,
        prefix: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchRecipe {
    pub executable: GamePath,
    pub arguments: Vec<LaunchValue>,
    pub environment: BTreeMap<String, LaunchValue>,
    pub working_directory: GamePath,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentLayout {
    pub data_path: RelPath, // "Data" for Skyrim, "" when content goes in the game root
    pub data_markers: Vec<String>, // globs on top-level names that mean "this is data-folder content"
    pub root_markers: Vec<String>, // globs on top-level names that mean "this is game-root content"
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub thunderstore_bepinex: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Suggestion {
    Place {
        source_path: RelPath,
        mount_path: RelPath,
        reason: String,
    },
    Installer {
        reason: String,
    },
    Unknown {
        top_level: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassificationRule {
    pub source_pattern: String,
    pub destination: RelPath,
    pub content_kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameDefinition {
    pub id: GameId,
    pub name: String,
    pub stores: Vec<StoreIdentifier>,
    pub version_sources: Vec<VersionSource>,
    pub deployment: DeploymentStrategy,
    pub content_rules: Vec<ClassificationRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_layout: Option<ContentLayout>,
    pub native_code_patterns: Vec<RelPath>,
    pub framework_ids: Vec<FrameworkId>,
    pub tool_ids: Vec<ToolId>,
    /// Declarative launch recipe for this game, if any.
    ///
    /// `None` means the package's [`GamePackage::prepare_launch`](crate::GamePackage::prepare_launch)
    /// provides the recipe (Minecraft builds its command line from version metadata, so a declarative
    /// recipe would be fiction).
    #[serde(default)]
    pub launch: Option<LaunchRecipe>,
    pub log_paths: Vec<GamePath>,
    pub crash_paths: Vec<GamePath>,
    pub user_files: Vec<UserFileMapping>,
    pub save_paths: Vec<GamePath>,
    pub linked_archive_patterns: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub declared_writes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub copy_patterns: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excluded_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_list: Option<PluginListRule>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub launch_alternatives: Vec<LaunchAlternative>,
    /// Files a framework ships once per game version (MASTER_SPEC §26.6), checked before launch.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime_files: Vec<RuntimeFileRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginListRule {
    pub user_file: RelPath,
    pub plugin_folder: RelPath,
    pub patterns: Vec<String>,
    pub active_prefix: String,
    pub header: Vec<String>,
    /// The load-order rules the game's plugin format obeys. `"creation_engine"` turns on the
    /// masters, light plugin and limit rules of MASTER_SPEC §26.6; any other value is refused
    /// at registration. Absent means the list only activates plugins, as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantics: Option<String>,
    /// Plugins the game always loads first, in this order, when they are present in the plugin
    /// folder. They are not written into the instance's list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub implicit: Vec<String>,
    /// A file (relative to the game's root) naming more always-loaded plugins, one per line, in
    /// the file's order. Each loads after `implicit`, and only when present in the plugin folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implicit_list_file: Option<RelPath>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchAlternative {
    pub id: String,
    pub when_present: RelPath,
    pub executable: GamePath,
    pub reason: String,
}

/// A file a framework ships once per game version (MASTER_SPEC §26.6). When the game's files hold
/// some of the family but not the one this runtime needs, the launch is refused with a
/// [`RuntimeFileFinding`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeFileRule {
    /// Stable identifier, e.g. `address-library`.
    pub id: String,
    /// Display name, e.g. `Address Library for SKSE Plugins`.
    pub name: String,
    /// Case-insensitive glob over deployed paths, `/`-separated, e.g. `Data/SKSE/Plugins/versionlib-*.bin`.
    pub family: String,
    /// The path this runtime needs. `{1}`..`{9}` are the dot-separated components of the runtime
    /// version, `{version}` the whole of it.
    pub expected: String,
    /// Version globs the rule applies to (e.g. `1.6.*`); empty means every version.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub applies_to: Vec<String>,
    /// What to do, shown with the finding; `{version}` is substituted.
    pub repair: String,
}

/// The most family file names one finding lists.
pub const RUNTIME_FILE_FINDING_MAX_FOUND: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuntimeFileProblem {
    /// Files of the family are present, but not the one this runtime needs.
    WrongVersion,
    /// The rule cannot be evaluated against this runtime. That is a bad definition, and it is
    /// shown as a finding rather than skipped.
    CannotCheck { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeFileFinding {
    pub rule_id: String,
    pub rule_name: String,
    /// The path this runtime needs, with the version substituted. For a rule that cannot be
    /// checked, the template as written.
    pub expected: String,
    /// Up to [`RUNTIME_FILE_FINDING_MAX_FOUND`] family files found, by file name.
    pub found: Vec<String>,
    /// What to do, with `{version}` substituted.
    pub repair: String,
    pub problem: RuntimeFileProblem,
}

impl RuntimeFileFinding {
    /// Only a framework built for another version refuses a launch. A rule that cannot be checked
    /// is a definition bug: it is carried as a warning, never a refusal.
    pub fn refuses_launch(&self) -> bool {
        matches!(self.problem, RuntimeFileProblem::WrongVersion)
    }

    /// One line for a person: what is wrong, what was found, and the repair.
    pub fn summary(&self) -> String {
        match &self.problem {
            RuntimeFileProblem::WrongVersion => {
                let found = if self.found.is_empty() {
                    "no file of its family".to_string()
                } else if self.found.len() >= RUNTIME_FILE_FINDING_MAX_FOUND {
                    format!("{}, ...", self.found.join(", "))
                } else {
                    self.found.join(", ")
                };
                format!(
                    "{} ({}) needs {} but the game has {}. Repair: {}",
                    self.rule_name, self.rule_id, self.expected, found, self.repair
                )
            }
            RuntimeFileProblem::CannotCheck { reason } => {
                format!(
                    "{} ({}) cannot be checked: {reason}",
                    self.rule_name, self.rule_id
                )
            }
        }
    }
}

/// Every finding's [`RuntimeFileFinding::summary`], joined for one message.
pub fn describe_runtime_findings(findings: &[RuntimeFileFinding]) -> String {
    findings
        .iter()
        .map(RuntimeFileFinding::summary)
        .collect::<Vec<_>>()
        .join("; ")
}

/// Check `rules` against the runtime `version` and the paths the game will see.
///
/// Pure: `deployed_paths` are `/`-separated paths relative to the game root, and matching is
/// case-insensitive. A rule that does not apply to `version` gives nothing. A rule whose family is
/// absent gives nothing (the framework is not installed, or the game is vanilla), and so does a
/// rule whose expected path is present. A rule that cannot be evaluated gives a
/// [`RuntimeFileProblem::CannotCheck`] finding, never a wrong-version one.
pub fn check_runtime_files<S: AsRef<str>>(
    rules: &[RuntimeFileRule],
    version: &str,
    deployed_paths: &[S],
) -> Vec<RuntimeFileFinding> {
    let mut findings = Vec::new();
    for rule in rules {
        if !rule.applies_to.is_empty()
            && !rule
                .applies_to
                .iter()
                .any(|pattern| runtime_path_matches(pattern, version))
        {
            continue;
        }

        let expected = match expected_runtime_path(rule, version) {
            Ok(expected) => expected,
            Err(reason) => {
                findings.push(cannot_check(rule, &reason));
                continue;
            }
        };

        let mut family_files: Vec<&str> = deployed_paths
            .iter()
            .map(AsRef::as_ref)
            .filter(|path| runtime_path_matches(&rule.family, path))
            .collect();
        if family_files.is_empty() {
            continue;
        }
        if deployed_paths
            .iter()
            .any(|path| path.as_ref().eq_ignore_ascii_case(&expected))
        {
            continue;
        }

        family_files.sort_by_key(|path| path.to_ascii_lowercase());
        family_files.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        let found = family_files
            .iter()
            .take(RUNTIME_FILE_FINDING_MAX_FOUND)
            .map(|path| path.rsplit('/').next().unwrap_or(path).to_string())
            .collect();
        findings.push(RuntimeFileFinding {
            rule_id: rule.id.clone(),
            rule_name: rule.name.clone(),
            expected,
            found,
            repair: rule.repair.replace("{version}", version),
            problem: RuntimeFileProblem::WrongVersion,
        });
    }
    findings
}

/// The path `rule` expects for `version`, or why the rule cannot say.
fn expected_runtime_path(rule: &RuntimeFileRule, version: &str) -> Result<String, String> {
    if rule.family.trim().is_empty() {
        return Err("its family is empty".to_string());
    }
    if rule.expected.trim().is_empty() {
        return Err("its expected path is empty".to_string());
    }
    expand_runtime_template(&rule.expected, version)
}

fn cannot_check(rule: &RuntimeFileRule, reason: &str) -> RuntimeFileFinding {
    RuntimeFileFinding {
        rule_id: rule.id.clone(),
        rule_name: rule.name.clone(),
        expected: rule.expected.clone(),
        found: Vec::new(),
        repair: rule.repair.clone(),
        problem: RuntimeFileProblem::CannotCheck {
            reason: reason.to_string(),
        },
    }
}

/// Case-insensitive match of a pattern against a path or a version. A pattern without `*` must be
/// equal to the text; one with `*` is matched by [`glob_match`].
fn runtime_path_matches(pattern: &str, text: &str) -> bool {
    let pattern = pattern.to_ascii_lowercase();
    let text = text.to_ascii_lowercase();
    if pattern.contains('*') {
        glob_match(&pattern, &text)
    } else {
        pattern == text
    }
}

/// Substitute `{1}`..`{9}` (the dot-separated components of `version`) and `{version}` into a
/// runtime file template. Any other placeholder, a missing component or an unbalanced brace is an
/// error that says what is wrong.
fn expand_runtime_template(template: &str, version: &str) -> Result<String, String> {
    if version.trim().is_empty() {
        return Err("the runtime has no version".to_string());
    }
    let components: Vec<&str> = version.split('.').collect();
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let Some(close_offset) = rest[open..].find('}') else {
            return Err(format!("'{template}' has a '{{' with no matching '}}'"));
        };
        let close = open + close_offset;
        out.push_str(&rest[..open]);
        let key = &rest[open + 1..close];
        if key == "version" {
            out.push_str(version);
        } else {
            let index = match key.parse::<usize>() {
                Ok(n) if (1..=9).contains(&n) => n,
                _ => {
                    return Err(format!(
                        "'{{{key}}}' is not a placeholder (use {{1}} to {{9}}, or {{version}})"
                    ))
                }
            };
            let Some(component) = components.get(index - 1) else {
                return Err(format!(
                    "runtime version {version} has no component {index}"
                ));
            };
            out.push_str(component);
        }
        rest = &rest[close + 1..];
    }
    if rest.contains('}') {
        return Err(format!("'{template}' has a '}}' with no matching '{{'"));
    }
    out.push_str(rest);
    Ok(out)
}

/// Default copy patterns applied to every game in Links mode (small text files at most 1 MiB).
pub const DEFAULT_COPY_PATTERNS: &[&str] = &[
    "**/*.ini",
    "**/*.cfg",
    "**/*.json",
    "**/*.toml",
    "**/*.xml",
    "**/*.yaml",
    "**/*.yml",
    "**/*.conf",
    "**/*.config",
    "**/*.properties",
];

impl GameDefinition {
    /// Whether native code is legitimate at `path` (relative to the install root).
    /// Runs on the normalized path, rejecting escapes such as `Data/SKSE/Plugins/../../x.dll`.
    pub fn allows_native_code_at(&self, path: &str) -> bool {
        let Ok(rel) = RelPath::new(path) else {
            return false;
        };
        let normalized = rel.as_str();
        self.native_code_patterns
            .iter()
            .any(|pattern| glob_match(pattern.as_str(), normalized))
    }

    /// Whether `path` (relative to the install root) matches a linked archive pattern.
    /// Runs on the normalized path, comparing ASCII case-insensitively.
    pub fn is_linked_archive(&self, path: &str) -> bool {
        let Ok(rel) = RelPath::new(path) else {
            return false;
        };
        let normalized = rel.as_str().to_ascii_lowercase();
        self.linked_archive_patterns
            .iter()
            .any(|pattern| glob_match(&pattern.to_ascii_lowercase(), &normalized))
    }

    /// Whether `path` (relative to the runtime root) matches a declared write pattern.
    /// Runs on the normalized path, comparing ASCII case-insensitively.
    pub fn is_declared_write(&self, path: &str) -> bool {
        let Ok(rel) = RelPath::new(path) else {
            return false;
        };
        let normalized = rel.as_str().to_ascii_lowercase();
        self.declared_writes
            .iter()
            .any(|pattern| glob_match(&pattern.to_ascii_lowercase(), &normalized))
    }

    /// Whether `path` (relative to the runtime root) matches either core default copy patterns
    /// or the game definition's copy patterns. Runs on normalized path, case-insensitively.
    pub fn is_copy_pattern(&self, path: &str) -> bool {
        let Ok(rel) = RelPath::new(path) else {
            return false;
        };
        let normalized = rel.as_str().to_ascii_lowercase();
        DEFAULT_COPY_PATTERNS
            .iter()
            .any(|pattern| glob_match(&pattern.to_ascii_lowercase(), &normalized))
            || self
                .copy_patterns
                .iter()
                .any(|pattern| glob_match(&pattern.to_ascii_lowercase(), &normalized))
    }

    /// Whether `path` (relative to the install root) matches an excluded path pattern.
    /// Runs on the normalized path, comparing ASCII case-insensitively.
    pub fn is_excluded(&self, path: &str) -> bool {
        let Ok(rel) = RelPath::new(path) else {
            return false;
        };
        let normalized = rel.as_str().to_ascii_lowercase();
        self.excluded_paths
            .iter()
            .any(|pattern| glob_match(&pattern.to_ascii_lowercase(), &normalized))
    }
}

pub fn suggest_placement(files: &[RelPath], layout: &ContentLayout) -> Suggestion {
    let mut current_source_components: Vec<String> = Vec::new();

    for level in 0..=8 {
        let mut files_at_level = BTreeSet::new();
        let mut folders_at_level = BTreeSet::new();

        for f in files {
            let comps: Vec<&str> = f.as_str().split('/').filter(|s| !s.is_empty()).collect();
            if comps.len() <= current_source_components.len() {
                continue;
            }
            let mut prefix_match = true;
            for (sc, fc) in current_source_components.iter().zip(comps.iter()) {
                if !sc.eq_ignore_ascii_case(fc) {
                    prefix_match = false;
                    break;
                }
            }
            if !prefix_match {
                continue;
            }

            let rem = &comps[current_source_components.len()..];
            if rem.len() == 1 {
                files_at_level.insert(rem[0].to_string());
            } else if rem.len() > 1 {
                folders_at_level.insert(rem[0].to_string());
            }
        }

        let mut top_level_names: Vec<String> = files_at_level
            .iter()
            .cloned()
            .chain(folders_at_level.iter().cloned())
            .collect();
        top_level_names.sort();
        top_level_names.dedup();

        if top_level_names.is_empty() {
            return Suggestion::Unknown { top_level: vec![] };
        }

        let unwrapped_prefix = if current_source_components.is_empty() {
            String::new()
        } else {
            format!("unwrapped `{}/`; ", current_source_components.join("/"))
        };

        let current_source_rel =
            RelPath::new(current_source_components.join("/")).unwrap_or_default();

        // 1. A fomod folder at that level (case-insensitive): Installer
        if folders_at_level
            .iter()
            .any(|f| f.eq_ignore_ascii_case("fomod"))
        {
            return Suggestion::Installer {
                reason: format!("{unwrapped_prefix}archive contains a FOMOD installer (`fomod/`)"),
            };
        }

        // 2. Else, if any top-level name equals data_path (when data_path is not empty) or matches a root marker: Place { mount_path: "" }
        let mut root_match: Option<String> = None;
        for name in &top_level_names {
            if !layout.data_path.as_str().is_empty()
                && name.eq_ignore_ascii_case(layout.data_path.as_str())
            {
                root_match = Some(name.clone());
                break;
            }
            if layout
                .root_markers
                .iter()
                .any(|m| glob_match(&m.to_ascii_lowercase(), &name.to_ascii_lowercase()))
            {
                root_match = Some(name.clone());
                break;
            }
        }

        if let Some(m) = root_match {
            let reason = if !layout.data_path.as_str().is_empty()
                && m.eq_ignore_ascii_case(layout.data_path.as_str())
            {
                format!("{unwrapped_prefix}`{m}` matches data folder, so it goes in game root")
            } else {
                format!("{unwrapped_prefix}`{m}` is game-root content, so it goes in game root")
            };
            return Suggestion::Place {
                source_path: current_source_rel,
                mount_path: RelPath::default(),
                reason,
            };
        }

        // 3. Else, if any matches a data marker: Place { mount_path: data_path }
        let mut data_match: Option<String> = None;
        for name in &top_level_names {
            if layout
                .data_markers
                .iter()
                .any(|m| glob_match(&m.to_ascii_lowercase(), &name.to_ascii_lowercase()))
            {
                data_match = Some(name.clone());
                break;
            }
        }

        if let Some(m) = data_match {
            let reason = if layout.data_path.as_str().is_empty() {
                format!("{unwrapped_prefix}`{m}` is data-folder content, so it goes in game root")
            } else {
                format!(
                    "{unwrapped_prefix}`{m}` is data-folder content, so it goes in `{}`",
                    layout.data_path.as_str()
                )
            };
            return Suggestion::Place {
                source_path: current_source_rel,
                mount_path: layout.data_path.clone(),
                reason,
            };
        }

        // 4. Else, if the level holds exactly one folder and no files, descend into it and repeat (at most 8 levels)
        if files_at_level.is_empty() && folders_at_level.len() == 1 && level < 8 {
            let only_folder = folders_at_level.into_iter().next().unwrap();
            current_source_components.push(only_folder);
            continue;
        }

        // 5. Else Unknown
        return Suggestion::Unknown {
            top_level: top_level_names,
        };
    }

    Suggestion::Unknown { top_level: vec![] }
}

pub fn glob_match(pattern: &str, path: &str) -> bool {
    if !pattern.contains('*') && !pattern.is_empty() {
        let prefix = pattern.trim_end_matches('/');
        return path == prefix || path.starts_with(&format!("{prefix}/"));
    }
    let pat_parts: Vec<&str> = pattern.split('/').collect();
    let path_parts: Vec<&str> = path.split('/').collect();
    match_parts(&pat_parts, &path_parts)
}

fn match_parts(pattern: &[&str], path: &[&str]) -> bool {
    match (pattern.first(), path.first()) {
        (None, None) => true,
        (Some(&"**"), _) => {
            match_parts(&pattern[1..], path)
                || (!path.is_empty() && match_parts(pattern, &path[1..]))
        }
        (Some(pat), Some(p)) => {
            if wildcard_match(pat, p) {
                match_parts(&pattern[1..], &path[1..])
            } else {
                false
            }
        }
        (Some(_), None) => pattern.iter().all(|&p| p == "**"),
        (None, Some(_)) => false,
    }
}

fn wildcard_match(pattern: &str, text: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if !pattern.contains('*') {
        return pattern == text;
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.is_empty() {
        return true;
    }
    if !text.starts_with(parts[0]) {
        return false;
    }
    let mut cur = &text[parts[0].len()..];
    for &part in &parts[1..parts.len() - 1] {
        if let Some(idx) = cur.find(part) {
            cur = &cur[idx + part.len()..];
        } else {
            return false;
        }
    }
    cur.ends_with(parts[parts.len() - 1])
}

// ---------------------------------------------------------------------------
// Thunderstore Package Support (BepInEx)
// ---------------------------------------------------------------------------

/// The fields of a Thunderstore `manifest.json` that placement needs. Parsing it is the caller's
/// job (core, with a parser that bounds nesting); other fields are ignored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThunderstoreManifest {
    pub name: String,
    pub version_number: String,
    pub dependencies: Vec<String>,
}

/// Thunderstore's rule for a namespace or package name: ASCII letters, digits and underscores.
/// Anything else (a `/`, a `..`, an empty name) never becomes part of a destination path.
pub fn is_thunderstore_name(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ThunderstoreMappingError {
    #[error("not a Thunderstore package")]
    NotPackage,
    #[error("destination path collision: {0}")]
    Collision(String),
    #[error("invalid path: {0}")]
    InvalidPath(String),
}

pub fn extract_thunderstore_package_id(
    file_stem_or_name: &str,
    manifest_name: &str,
    version: &str,
) -> String {
    let mut stem = file_stem_or_name;
    if let Some(s) = stem.strip_suffix(".zip") {
        stem = s;
    }
    let version_suffix = format!("-{version}");
    let base = if let Some(prefix) = stem.strip_suffix(&version_suffix) {
        prefix
    } else {
        stem
    };

    let name_suffix = format!("-{manifest_name}");
    if let Some(ns) = base.strip_suffix(&name_suffix) {
        if is_thunderstore_name(ns) {
            return format!("{ns}-{manifest_name}");
        }
    }
    manifest_name.to_string()
}

pub fn map_thunderstore_bepinex(
    paths: &[RelPath],
    pkg: &str,
) -> Result<Vec<(RelPath, RelPath)>, ThunderstoreMappingError> {
    let has_manifest = paths
        .iter()
        .any(|p| p.as_str().eq_ignore_ascii_case("manifest.json"));
    if !has_manifest {
        return Err(ThunderstoreMappingError::NotPackage);
    }
    // The id becomes a folder name: `Namespace-Name` or `Name`, in Thunderstore's characters only.
    if !pkg.split('-').all(is_thunderstore_name) || pkg.split('-').count() > 2 {
        return Err(ThunderstoreMappingError::InvalidPath(format!(
            "'{pkg}' is not a Thunderstore package id"
        )));
    }

    let is_preloader = |parts: &[&str]| {
        parts.len() == 3
            && parts[0].eq_ignore_ascii_case("BepInEx")
            && parts[1].eq_ignore_ascii_case("core")
            && parts[2].eq_ignore_ascii_case("BepInEx.Preloader.dll")
    };
    // A BepInEx pack: the folder holding `BepInEx/core/BepInEx.Preloader.dll`, which is usually a
    // top-level folder (`BepInExPack_Valheim/`) and sometimes the package itself (`Some("")`).
    let mut pack_folder: Option<String> = None;
    for p in paths {
        let parts: Vec<&str> = p.as_str().split('/').filter(|s| !s.is_empty()).collect();
        if is_preloader(&parts) {
            pack_folder = Some(String::new());
            break;
        }
        if parts.len() == 4 && is_preloader(&parts[1..]) {
            pack_folder = Some(parts[0].to_string());
            break;
        }
    }

    let mut mappings = Vec::new();

    if pack_folder.as_deref() == Some("") {
        // The package is the pack: everything but Thunderstore's own top-level files goes to the
        // game root.
        for p in paths {
            let parts: Vec<&str> = p.as_str().split('/').filter(|s| !s.is_empty()).collect();
            let thunderstore_file = parts.len() == 1
                && ["manifest.json", "icon.png", "readme.md", "changelog.md"]
                    .contains(&parts[0].to_ascii_lowercase().as_str());
            if !parts.is_empty() && !thunderstore_file {
                mappings.push((p.clone(), p.clone()));
            }
        }
    } else if let Some(folder) = pack_folder {
        for p in paths {
            let parts: Vec<&str> = p.as_str().split('/').filter(|s| !s.is_empty()).collect();
            if parts.is_empty() {
                continue;
            }
            if parts[0].eq_ignore_ascii_case(&folder) {
                let rel = parts[1..].join("/");
                if !rel.is_empty() {
                    let dest = RelPath::new(&rel)
                        .map_err(|e| ThunderstoreMappingError::InvalidPath(e.to_string()))?;
                    mappings.push((p.clone(), dest));
                }
            }
        }
    } else {
        for p in paths {
            let parts: Vec<&str> = p.as_str().split('/').filter(|s| !s.is_empty()).collect();
            if parts.is_empty() {
                continue;
            }
            let top = parts[0].to_ascii_lowercase();
            let dest_str = match top.as_str() {
                "plugins" => {
                    let sub = parts[1..].join("/");
                    if sub.is_empty() {
                        format!("BepInEx/plugins/{pkg}")
                    } else {
                        format!("BepInEx/plugins/{pkg}/{sub}")
                    }
                }
                "patchers" => {
                    let sub = parts[1..].join("/");
                    if sub.is_empty() {
                        format!("BepInEx/patchers/{pkg}")
                    } else {
                        format!("BepInEx/patchers/{pkg}/{sub}")
                    }
                }
                "monomod" => {
                    let sub = parts[1..].join("/");
                    if sub.is_empty() {
                        format!("BepInEx/monomod/{pkg}")
                    } else {
                        format!("BepInEx/monomod/{pkg}/{sub}")
                    }
                }
                "config" => {
                    let sub = parts[1..].join("/");
                    if sub.is_empty() {
                        "BepInEx/config".to_string()
                    } else {
                        format!("BepInEx/config/{sub}")
                    }
                }
                "core" => {
                    let sub = parts[1..].join("/");
                    if sub.is_empty() {
                        "BepInEx/core".to_string()
                    } else {
                        format!("BepInEx/core/{sub}")
                    }
                }
                _ => {
                    format!("BepInEx/plugins/{pkg}/{}", p.as_str())
                }
            };
            let dest = RelPath::new(&dest_str)
                .map_err(|e| ThunderstoreMappingError::InvalidPath(e.to_string()))?;
            mappings.push((p.clone(), dest));
        }
    }

    if mappings.is_empty() {
        return Err(ThunderstoreMappingError::NotPackage);
    }

    let mut seen_lower = std::collections::HashSet::with_capacity(mappings.len());
    for (_, dest) in &mappings {
        let lower = dest.as_str().to_ascii_lowercase();
        if !seen_lower.insert(lower) {
            return Err(ThunderstoreMappingError::Collision(format!(
                "equals another destination path case-insensitively: '{dest}'"
            )));
        }
    }

    for (_, dest) in &mappings {
        let parts: Vec<&str> = dest.as_str().split('/').collect();
        for i in 1..parts.len() {
            let prefix = parts[..i].join("/").to_ascii_lowercase();
            if seen_lower.contains(&prefix) {
                return Err(ThunderstoreMappingError::Collision(format!(
                    "prefix folder '{prefix}' matches another file"
                )));
            }
        }
    }

    Ok(mappings)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameworkDefinition {
    pub id: FrameworkId,
    pub game: GameId,
    pub name: String,
    pub version: String,
    /// Alternatives, not a semver range over an assumed game version scheme.
    pub supported_runtimes: Vec<RuntimeConstraint>,
    pub required_frameworks: Vec<FrameworkId>,
    pub content: Vec<String>,
    pub launch: Option<LaunchRecipe>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledFramework {
    pub id: FrameworkId,
    pub version: String,
    /// Alternatives (OR); empty means compatibility evidence was not recorded,
    /// not that every runtime is known compatible (legacy Minecraft migration).
    pub supported_runtimes: Vec<RuntimeConstraint>,
    pub layers: Vec<LayerId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub id: ToolId,
    pub game: GameId,
    pub name: String,
    pub launch: LaunchRecipe,
    pub input_layers: Vec<LayerId>,
    pub relevant_settings: Vec<String>,
    pub output_layer: LayerId,
    /// Generated output precedence is independent of plugin load order.
    pub after_tools: Vec<ToolId>,
}

/// This is plugin/module order, distinct from the order of content layers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LoadOrder {
    pub entries: Vec<LoadOrderEntry>,
    pub rules: Vec<LoadOrderRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadOrderEntry {
    pub id: String,
    pub enabled: bool,
    pub locked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LoadOrderRule {
    Before { first: String, second: String },
    Requires { item: String, master: String },
    Conflicts { first: String, second: String },
}

#[cfg(test)]
mod runtime_file_tests {
    use super::*;

    /// The three rules Skyrim SE declares in `agora-game-creation/data/package.json`.
    fn skyrim_rules() -> Vec<RuntimeFileRule> {
        vec![
            RuntimeFileRule {
                id: "skse".into(),
                name: "Skyrim Script Extender (SKSE)".into(),
                family: "skse64_*.dll".into(),
                expected: "skse64_{1}_{2}_{3}.dll".into(),
                applies_to: vec![],
                repair: "Install the SKSE build for Skyrim {version} from skse.silverlock.org"
                    .into(),
            },
            RuntimeFileRule {
                id: "address-library".into(),
                name: "Address Library for SKSE Plugins".into(),
                family: "Data/SKSE/Plugins/version*-*.bin".into(),
                expected: "Data/SKSE/Plugins/versionlib-{1}-{2}-{3}-{4}.bin".into(),
                applies_to: vec!["1.6.*".into()],
                repair: "Install the Address Library build for Skyrim {version}".into(),
            },
            RuntimeFileRule {
                id: "address-library-se".into(),
                name: "Address Library for SKSE Plugins".into(),
                family: "Data/SKSE/Plugins/version*-*.bin".into(),
                expected: "Data/SKSE/Plugins/version-{1}-{2}-{3}-{4}.bin".into(),
                applies_to: vec!["1.5.*".into()],
                repair: "Install the Address Library build for Skyrim {version}".into(),
            },
        ]
    }

    /// Paths of the working Skyrim SE 1.6.1170.0 instance: the SKSE and Address Library files.
    fn skyrim_1_6_1170_files() -> Vec<&'static str> {
        vec![
            "SkyrimSE.exe",
            "skse64_1_6_1170.dll",
            "skse64_loader.exe",
            "Data/Skyrim.esm",
            "Data/SKSE/Plugins/versionlib-1-6-1170-0.bin",
            "Data/SKSE/Plugins/versionlib-1-6-1170-0-1.bin",
        ]
    }

    fn ids(findings: &[RuntimeFileFinding]) -> Vec<&str> {
        findings.iter().map(|f| f.rule_id.as_str()).collect()
    }

    #[test]
    fn the_matching_runtime_gives_no_findings() {
        let findings = check_runtime_files(&skyrim_rules(), "1.6.1170.0", &skyrim_1_6_1170_files());
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn a_newer_runtime_names_skse_and_the_address_library_it_lacks() {
        let findings = check_runtime_files(&skyrim_rules(), "1.6.1179.0", &skyrim_1_6_1170_files());
        assert_eq!(ids(&findings), vec!["skse", "address-library"]);

        let skse = &findings[0];
        assert_eq!(skse.expected, "skse64_1_6_1179.dll");
        assert_eq!(skse.found, vec!["skse64_1_6_1170.dll"]);
        assert_eq!(skse.problem, RuntimeFileProblem::WrongVersion);
        assert_eq!(
            skse.repair,
            "Install the SKSE build for Skyrim 1.6.1179.0 from skse.silverlock.org"
        );

        let library = &findings[1];
        assert_eq!(
            library.expected,
            "Data/SKSE/Plugins/versionlib-1-6-1179-0.bin"
        );
        // Sorted by path: "-0-1.bin" sorts before "-0.bin" because '-' is before '.'.
        assert_eq!(
            library.found,
            vec!["versionlib-1-6-1170-0-1.bin", "versionlib-1-6-1170-0.bin"]
        );
    }

    #[test]
    fn the_older_runtime_names_skse_and_the_1_5_address_library() {
        let findings = check_runtime_files(&skyrim_rules(), "1.5.97.0", &skyrim_1_6_1170_files());
        assert_eq!(ids(&findings), vec!["skse", "address-library-se"]);
        assert_eq!(
            findings[1].expected,
            "Data/SKSE/Plugins/version-1-5-97-0.bin"
        );
    }

    #[test]
    fn matching_ignores_case() {
        let files = [
            "SKSE64_1_6_1170.DLL",
            "data/skse/plugins/VERSIONLIB-1-6-1170-0.BIN",
        ];
        assert!(check_runtime_files(&skyrim_rules(), "1.6.1170.0", &files).is_empty());

        let findings = check_runtime_files(&skyrim_rules(), "1.6.1179.0", &files);
        assert_eq!(ids(&findings), vec!["skse", "address-library"]);
        assert_eq!(findings[0].found, vec!["SKSE64_1_6_1170.DLL"]);
    }

    #[test]
    fn no_family_files_gives_no_findings_for_a_vanilla_game() {
        let files = ["SkyrimSE.exe", "Data/Skyrim.esm"];
        assert!(check_runtime_files(&skyrim_rules(), "1.6.1179.0", &files).is_empty());
        let none: [&str; 0] = [];
        assert!(check_runtime_files(&skyrim_rules(), "1.6.1179.0", &none).is_empty());
    }

    #[test]
    fn a_three_part_version_cannot_fill_component_four() {
        let findings = check_runtime_files(&skyrim_rules(), "1.6.1170", &skyrim_1_6_1170_files());
        assert_eq!(findings.len(), 1, "{findings:?}");
        let finding = &findings[0];
        assert_eq!(finding.rule_id, "address-library");
        assert_eq!(
            finding.expected,
            "Data/SKSE/Plugins/versionlib-{1}-{2}-{3}-{4}.bin"
        );
        assert!(finding.found.is_empty());
        match &finding.problem {
            RuntimeFileProblem::CannotCheck { reason } => {
                assert!(reason.contains("component 4"), "{reason}")
            }
            other => panic!("expected CannotCheck, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_family_or_expected_path_is_reported_as_unchecked() {
        let mut rules = skyrim_rules();
        rules[0].family = "  ".into();
        rules[1].expected = String::new();
        let findings = check_runtime_files(&rules, "1.6.1170.0", &skyrim_1_6_1170_files());
        assert_eq!(ids(&findings), vec!["skse", "address-library"]);
        assert!(findings
            .iter()
            .all(|f| matches!(f.problem, RuntimeFileProblem::CannotCheck { .. })));
    }

    #[test]
    fn a_rule_whose_applies_to_excludes_the_version_is_silent() {
        // The game has only the 1.5 library. The 1.5 rule does not apply to a 1.6 runtime, so it
        // says nothing; the 1.6 rule shares the family, finds the 1.5 file and names it.
        let files = ["Data/SKSE/Plugins/version-1-5-97-0.bin"];
        let findings = check_runtime_files(&skyrim_rules(), "1.6.1170.0", &files);
        assert_eq!(ids(&findings), vec!["address-library"]);
        assert_eq!(findings[0].found, vec!["version-1-5-97-0.bin"]);

        // A broken rule that does not apply to this version is not reported either.
        let mut rules = skyrim_rules();
        rules[1].family = String::new();
        rules[1].applies_to = vec!["1.5.*".into()];
        let findings = check_runtime_files(&rules, "1.6.1170.0", &skyrim_1_6_1170_files());
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn unknown_placeholders_and_unbalanced_braces_are_reported() {
        let mut rules = skyrim_rules();
        rules[0].expected = "skse64_{0}_{2}.dll".into();
        rules[2].expected = "Data/{version".into();
        let files = [
            "skse64_1_6_1170.dll",
            "Data/SKSE/Plugins/version-1-5-97-0.bin",
        ];
        let findings = check_runtime_files(&rules, "1.5.97.0", &files);
        assert_eq!(ids(&findings), vec!["skse", "address-library-se"]);
        for finding in &findings {
            assert!(matches!(
                finding.problem,
                RuntimeFileProblem::CannotCheck { .. }
            ));
        }
    }

    #[test]
    fn summary_names_the_framework_what_was_found_and_the_repair() {
        let findings = check_runtime_files(&skyrim_rules(), "1.6.1179.0", &skyrim_1_6_1170_files());
        let text = describe_runtime_findings(&findings);
        assert!(text.contains("Skyrim Script Extender (SKSE)"), "{text}");
        assert!(text.contains("skse64_1_6_1170.dll"), "{text}");
        assert!(
            text.contains("Repair: Install the SKSE build for Skyrim 1.6.1179.0"),
            "{text}"
        );
    }

    #[test]
    fn a_finding_lists_at_most_five_family_files() {
        let names: Vec<String> = (0..8)
            .map(|i| format!("Data/SKSE/Plugins/version-{i}.bin"))
            .collect();
        let rules = vec![RuntimeFileRule {
            id: "lib".into(),
            name: "Library".into(),
            family: "Data/SKSE/Plugins/version*.bin".into(),
            expected: "Data/SKSE/Plugins/versionlib-{1}-{2}-{3}-{4}.bin".into(),
            applies_to: vec![],
            repair: "Install it".into(),
        }];
        let findings = check_runtime_files(&rules, "1.6.1179.0", &names);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].found.len(), RUNTIME_FILE_FINDING_MAX_FOUND);
        assert!(findings[0].summary().contains(", ..."));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_matching_keeps_store_build_and_opaque_versions() {
        let mut runtime = RuntimeIdentity {
            game: GameId::new("skyrim").unwrap(),
            store: StoreId::new("steam").unwrap(),
            version: "1.6.1170.0".into(),
            build: Some("12345".into()),
        };
        let constraint = RuntimeConstraint {
            game: GameId::new("skyrim").unwrap(),
            stores: vec![StoreId::new("steam").unwrap()],
            versions: VersionConstraint::Exact(vec!["1.6.1170.0".into()]),
            builds: vec!["12345".into()],
        };
        assert_eq!(constraint.check(&runtime), Support::Supported);
        assert!(constraint.matches(&runtime));

        runtime.store = StoreId::new("gog").unwrap();
        assert_eq!(constraint.check(&runtime), Support::Unsupported);
        assert!(!constraint.matches(&runtime));

        runtime.store = StoreId::new("steam").unwrap();
        runtime.build = None;
        assert_eq!(constraint.check(&runtime), Support::Indeterminate);
        assert!(!constraint.matches(&runtime));

        runtime.build = Some("12345".into());
        runtime.version = "24w14a".into();
        assert_eq!(constraint.check(&runtime), Support::Unsupported);
        assert!(!constraint.matches(&runtime));
    }

    #[test]
    fn semver_is_explicit_and_never_coerces_four_part_versions() {
        let constraint = RuntimeConstraint {
            game: GameId::new("valheim").unwrap(),
            stores: vec![],
            builds: vec![],
            versions: VersionConstraint::Semver(">=0.218.0, <0.219.0".parse().unwrap()),
        };
        let mut runtime = RuntimeIdentity {
            game: GameId::new("valheim").unwrap(),
            store: StoreId::new("community-store").unwrap(),
            version: "0.218.15".into(),
            build: None,
        };
        assert_eq!(constraint.check(&runtime), Support::Supported);

        runtime.version = "0.218.15.0".into();
        // Four-part version cannot parse as semver -> Indeterminate
        assert_eq!(constraint.check(&runtime), Support::Indeterminate);
    }

    #[test]
    fn native_code_checks_reject_path_traversal() {
        let def = GameDefinition {
            id: GameId::new("skyrim").unwrap(),
            name: "Skyrim".into(),
            stores: vec![],
            version_sources: vec![],
            deployment: DeploymentStrategy::VirtualFileSystem,
            content_rules: vec![],
            content_layout: None,
            native_code_patterns: vec![
                RelPath::new("Data/SKSE/Plugins/*.dll").unwrap(),
                RelPath::new("mods/**/*.dll").unwrap(),
            ],
            framework_ids: vec![],
            tool_ids: vec![],
            launch: Some(LaunchRecipe {
                executable: GamePath::Runtime {
                    path: RelPath::new("SkyrimSE.exe").unwrap(),
                },
                arguments: vec![],
                environment: BTreeMap::new(),
                working_directory: GamePath::Runtime {
                    path: RelPath::default(),
                },
            }),
            log_paths: vec![],
            crash_paths: vec![],
            user_files: vec![],
            save_paths: vec![],
            linked_archive_patterns: vec![],
            declared_writes: vec![],
            excluded_paths: vec![],
            plugin_list: None,
            runtime_files: Vec::new(),
            launch_alternatives: Vec::new(),
            copy_patterns: Vec::new(),
        };

        // Normal valid paths
        assert!(def.allows_native_code_at("Data/SKSE/Plugins/plugin.dll"));
        assert!(def.allows_native_code_at("Data\\SKSE\\Plugins\\plugin.dll"));
        assert!(def.allows_native_code_at("mods/sub/nested/mod.dll"));

        // Path traversal attempts must NOT be accepted!
        assert!(!def.allows_native_code_at("Data/SKSE/Plugins/../../x.dll"));
        assert!(!def.allows_native_code_at("Data\\SKSE\\Plugins\\..\\..\\x.dll"));
        assert!(!def.allows_native_code_at("../mods/evil.dll"));
        assert!(!def.allows_native_code_at("C:/evil.dll"));
        assert!(!def.allows_native_code_at("Data/foo.dll"));
    }

    #[test]
    fn linked_archive_matching_is_case_insensitive() {
        let def = GameDefinition {
            id: GameId::new("skyrim-se").unwrap(),
            name: "Skyrim SE".into(),
            stores: vec![],
            version_sources: vec![],
            deployment: DeploymentStrategy::VirtualFileSystem,
            content_rules: vec![],
            content_layout: None,
            native_code_patterns: vec![],
            framework_ids: vec![],
            tool_ids: vec![],
            launch: None,
            log_paths: vec![],
            crash_paths: vec![],
            user_files: vec![],
            save_paths: vec![],
            linked_archive_patterns: vec![
                "Data/*.bsa".into(),
                "Data/*.esm".into(),
                "Data/*.esl".into(),
                "Data/*.bik".into(),
            ],
            declared_writes: vec![],
            excluded_paths: vec![],
            plugin_list: None,
            runtime_files: Vec::new(),
            launch_alternatives: Vec::new(),
            copy_patterns: Vec::new(),
        };

        // Matches exact and mixed cases
        assert!(def.is_linked_archive("Data/Skyrim - Textures.bsa"));
        assert!(def.is_linked_archive("data/skyrim - textures.BSA"));
        assert!(def.is_linked_archive("DATA\\b.ESM"));
        assert!(def.is_linked_archive("data/c.esl"));
        assert!(def.is_linked_archive("Data/video.bik"));
        assert!(def.is_linked_archive("DATA/VIDEO.BIK"));

        // Non-matches
        assert!(!def.is_linked_archive("SkyrimSE.exe"));
        assert!(!def.is_linked_archive("Data/Sub/d.txt"));
        assert!(!def.is_linked_archive("Data/plugin.dll"));
        // Path traversal rejected
        assert!(!def.is_linked_archive("Data/../../evil.bsa"));
    }

    #[test]
    fn declared_write_matching_is_case_insensitive() {
        let def = GameDefinition {
            id: GameId::new("skyrim-se").unwrap(),
            name: "Skyrim SE".into(),
            stores: vec![],
            version_sources: vec![],
            deployment: DeploymentStrategy::VirtualFileSystem,
            content_rules: vec![],
            content_layout: None,
            native_code_patterns: vec![],
            framework_ids: vec![],
            tool_ids: vec![],
            launch: None,
            log_paths: vec![],
            crash_paths: vec![],
            user_files: vec![],
            save_paths: vec![],
            linked_archive_patterns: vec![],
            declared_writes: vec!["d3dx9_42.log".into(), "logs/*.log".into()],
            excluded_paths: vec![],
            plugin_list: None,
            runtime_files: Vec::new(),
            launch_alternatives: Vec::new(),
            copy_patterns: Vec::new(),
        };

        assert!(def.is_declared_write("d3dx9_42.log"));
        assert!(def.is_declared_write("D3DX9_42.LOG"));
        assert!(def.is_declared_write("D3dx9_42.log"));
        assert!(def.is_declared_write("logs/render.log"));
        assert!(def.is_declared_write("LOGS\\RENDER.LOG"));

        assert!(!def.is_declared_write("SkyrimSE.exe"));
        assert!(!def.is_declared_write("Data/d3dx9_42.log"));
        assert!(!def.is_declared_write("../d3dx9_42.log"));
    }

    #[test]
    fn excluded_paths_matching_is_case_insensitive() {
        let def = GameDefinition {
            id: GameId::new("skyrim-se").unwrap(),
            name: "Skyrim SE".into(),
            stores: vec![],
            version_sources: vec![],
            deployment: DeploymentStrategy::VirtualFileSystem,
            content_rules: vec![],
            content_layout: None,
            native_code_patterns: vec![],
            framework_ids: vec![],
            tool_ids: vec![],
            launch: None,
            log_paths: vec![],
            crash_paths: vec![],
            user_files: vec![],
            save_paths: vec![],
            linked_archive_patterns: vec![],
            declared_writes: vec![],
            excluded_paths: vec!["Data/SSEEdit Backups/**".into()],
            plugin_list: None,
            runtime_files: Vec::new(),
            launch_alternatives: Vec::new(),
            copy_patterns: Vec::new(),
        };

        assert!(def.is_excluded("Data/SSEEdit Backups/x.esm.backup"));
        assert!(def.is_excluded("data/sseedit backups/x.esm.backup"));
        assert!(def.is_excluded("DATA\\SSEEDIT BACKUPS\\SUB\\Y.ESM.BACKUP"));
        assert!(!def.is_excluded("Data/Skyrim.esm"));
        assert!(!def.is_excluded("SkyrimSE.exe"));
    }

    #[test]
    fn copy_patterns_match_defaults_and_custom_case_insensitively() {
        let def = GameDefinition {
            id: GameId::new("valheim").unwrap(),
            name: "Valheim".into(),
            stores: vec![],
            version_sources: vec![],
            deployment: DeploymentStrategy::VirtualFileSystem,
            content_rules: vec![],
            content_layout: None,
            native_code_patterns: vec![],
            framework_ids: vec![],
            tool_ids: vec![],
            launch: None,
            log_paths: vec![],
            crash_paths: vec![],
            user_files: vec![],
            save_paths: vec![],
            linked_archive_patterns: vec![],
            declared_writes: vec![],
            excluded_paths: vec![],
            plugin_list: None,
            runtime_files: Vec::new(),
            launch_alternatives: Vec::new(),
            copy_patterns: vec!["**/*.dat".into()],
        };

        // Core defaults
        assert!(def.is_copy_pattern("BepInEx/config/BepInEx.cfg"));
        assert!(def.is_copy_pattern("settings.ini"));
        assert!(def.is_copy_pattern("SETTINGS.INI"));
        assert!(def.is_copy_pattern("Mod\\config.json"));
        assert!(def.is_copy_pattern("mod.toml"));
        assert!(def.is_copy_pattern("sub/file.xml"));
        assert!(def.is_copy_pattern("a/b/c.yaml"));
        assert!(def.is_copy_pattern("c.yml"));
        assert!(def.is_copy_pattern("foo.conf"));
        assert!(def.is_copy_pattern("bar.config"));
        assert!(def.is_copy_pattern("server.properties"));

        // Custom game copy_patterns
        assert!(def.is_copy_pattern("data/save.dat"));
        assert!(def.is_copy_pattern("DATA/SAVE.DAT"));

        // Non-matches
        assert!(!def.is_copy_pattern("plugin.dll"));
        assert!(!def.is_copy_pattern("Data/mod.esp"));
    }

    #[test]
    fn test_suggest_placement_skyrim_and_cyberpunk() {
        let skyrim = ContentLayout {
            data_path: RelPath::new("Data").unwrap(),
            data_markers: vec![
                "*.esp".into(),
                "*.esm".into(),
                "*.esl".into(),
                "*.bsa".into(),
                "textures".into(),
                "meshes".into(),
                "scripts".into(),
                "interface".into(),
                "sound".into(),
                "music".into(),
                "skse".into(),
                "strings".into(),
                "video".into(),
                "materials".into(),
                "lodsettings".into(),
                "seq".into(),
                "grass".into(),
                "shadersfx".into(),
                "facegen".into(),
                "lod".into(),
                "terrain".into(),
                "dyndolod".into(),
                "nemesis_engine".into(),
                "calientetools".into(),
                "tools".into(),
                "source".into(),
                "platform".into(),
            ],
            root_markers: vec![
                "*.exe".into(),
                "*.dll".into(),
                "enbseries".into(),
                "enb*.ini".into(),
                "reshade-shaders".into(),
            ],
            thunderstore_bepinex: false,
        };

        // 1. Loose MyMod.esp + textures/a.dds -> Data
        let files = vec![
            RelPath::new("MyMod.esp").unwrap(),
            RelPath::new("textures/a.dds").unwrap(),
        ];
        match suggest_placement(&files, &skyrim) {
            Suggestion::Place {
                source_path,
                mount_path,
                reason,
            } => {
                assert_eq!(source_path.as_str(), "");
                assert_eq!(mount_path.as_str(), "Data");
                assert!(reason.contains("Data"));
            }
            other => panic!("expected Place, got {other:?}"),
        }

        // 2. Data/MyMod.esp -> root
        let files = vec![RelPath::new("Data/MyMod.esp").unwrap()];
        match suggest_placement(&files, &skyrim) {
            Suggestion::Place {
                source_path,
                mount_path,
                ..
            } => {
                assert_eq!(source_path.as_str(), "");
                assert_eq!(mount_path.as_str(), "");
            }
            other => panic!("expected Place at root, got {other:?}"),
        }

        // 3. MyMod v1.2/textures/a.dds -> Data from MyMod v1.2
        let files = vec![RelPath::new("MyMod v1.2/textures/a.dds").unwrap()];
        match suggest_placement(&files, &skyrim) {
            Suggestion::Place {
                source_path,
                mount_path,
                reason,
            } => {
                assert_eq!(source_path.as_str(), "MyMod v1.2");
                assert_eq!(mount_path.as_str(), "Data");
                assert_eq!(
                    reason,
                    "unwrapped `MyMod v1.2/`; `textures` is data-folder content, so it goes in `Data`"
                );
            }
            other => panic!("expected Place, got {other:?}"),
        }

        // 4. skse64_2_02_06/skse64_loader.exe + skse64_2_02_06/Data/Scripts/x.pex -> root from skse64_2_02_06
        let files = vec![
            RelPath::new("skse64_2_02_06/skse64_loader.exe").unwrap(),
            RelPath::new("skse64_2_02_06/Data/Scripts/x.pex").unwrap(),
        ];
        match suggest_placement(&files, &skyrim) {
            Suggestion::Place {
                source_path,
                mount_path,
                ..
            } => {
                assert_eq!(source_path.as_str(), "skse64_2_02_06");
                assert_eq!(mount_path.as_str(), "");
            }
            other => panic!("expected Place at root, got {other:?}"),
        }

        // 5. two nested wrappers
        let files = vec![RelPath::new("Wrap1/Wrap2/textures/a.dds").unwrap()];
        match suggest_placement(&files, &skyrim) {
            Suggestion::Place {
                source_path,
                mount_path,
                ..
            } => {
                assert_eq!(source_path.as_str(), "Wrap1/Wrap2");
                assert_eq!(mount_path.as_str(), "Data");
            }
            other => panic!("expected Place, got {other:?}"),
        }

        // 6. TEXTURES/a.dds (case)
        let files = vec![RelPath::new("TEXTURES/a.dds").unwrap()];
        match suggest_placement(&files, &skyrim) {
            Suggestion::Place {
                source_path,
                mount_path,
                ..
            } => {
                assert_eq!(source_path.as_str(), "");
                assert_eq!(mount_path.as_str(), "Data");
            }
            other => panic!("expected Place, got {other:?}"),
        }

        // 7. fomod/ModuleConfig.xml -> Installer
        let files = vec![RelPath::new("fomod/ModuleConfig.xml").unwrap()];
        assert!(matches!(
            suggest_placement(&files, &skyrim),
            Suggestion::Installer { .. }
        ));

        // 8. readme.txt alone -> Unknown
        let files = vec![RelPath::new("readme.txt").unwrap()];
        match suggest_placement(&files, &skyrim) {
            Suggestion::Unknown { top_level } => {
                assert_eq!(top_level, vec!["readme.txt"]);
            }
            other => panic!("expected Unknown, got {other:?}"),
        }

        // 9. nine nested wrappers -> Unknown
        let files = vec![RelPath::new("w1/w2/w3/w4/w5/w6/w7/w8/w9/textures/a.dds").unwrap()];
        assert!(matches!(
            suggest_placement(&files, &skyrim),
            Suggestion::Unknown { .. }
        ));

        // Cyberpunk: archive/pc/mod/x.archive -> root
        let cyberpunk = ContentLayout {
            data_path: RelPath::default(),
            data_markers: vec![],
            root_markers: vec![
                "archive".into(),
                "bin".into(),
                "r6".into(),
                "red4ext".into(),
                "engine".into(),
                "mods".into(),
            ],
            thunderstore_bepinex: false,
        };
        let files = vec![RelPath::new("archive/pc/mod/x.archive").unwrap()];
        match suggest_placement(&files, &cyberpunk) {
            Suggestion::Place {
                source_path,
                mount_path,
                ..
            } => {
                assert_eq!(source_path.as_str(), "");
                assert_eq!(mount_path.as_str(), "");
            }
            other => panic!("expected Place at root, got {other:?}"),
        }
    }
}
