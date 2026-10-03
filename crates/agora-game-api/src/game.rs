use crate::id::{FrameworkId, GameId, InstallId, LayerId, RelPath, StoreId, ToolId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
}

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
}

fn glob_match(pattern: &str, path: &str) -> bool {
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
}
