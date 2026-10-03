use serde::{Deserialize, Serialize};

/// A row in `local_state.db`'s `user_instances` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceRow {
    pub instance_id: String,
    pub name: String,
    pub minecraft_version: String,
    pub loader: String,
    pub loader_version: String,
    pub is_modpack: bool,
    pub is_locked: bool,
    pub last_launched_at: Option<String>,
    pub jvm_memory_mb: i64,
    pub jvm_memory_mode: String,
    pub jvm_gc: String,
    pub jvm_custom_args: String,
    pub jvm_always_pre_touch: bool,
    pub created_at: String,
    /// Per-instance Java path override (TEXT NULL).
    /// When set, overrides the global `java_path` setting for this instance.
    #[serde(default)]
    pub java_path: Option<String>,
    /// Whether to allow incompatible Java version for this instance.
    /// Default: false (0).
    #[serde(default)]
    pub java_incompatible_override: bool,
    /// Agora-owned local icon path. Source launcher paths are never exposed here.
    #[serde(default)]
    pub icon_path: Option<String>,
    /// Per-instance launch policy: `auto`, `direct`, or `delegated`.
    #[serde(default = "default_launch_mode_override")]
    pub launch_mode_override: String,
    /// Local-only provenance label populated from `instance_imports`.
    #[serde(default)]
    pub import_source: Option<String>,
}

fn default_launch_mode_override() -> String {
    "auto".to_string()
}

fn default_true() -> bool {
    true
}
fn default_mod_content_type() -> String {
    "mod".to_string()
}

/// Where a provider-installed file came from: which provider, and which of
/// its projects and versions. Recorded at install time so the choice to trust
/// that provider stays visible afterwards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderOrigin {
    /// `modrinth`, `technic`, or `<plugin-id>/<provider-id>`.
    pub provider_id: String,
    pub project_id: String,
    pub version_id: String,
}

/// An installed mod tracked by `instance_manifest.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledMod {
    /// Which content provider this file came from, for anything installed
    /// through a provider plugin. Absent for curated, Modrinth-legacy, manual
    /// and pack-managed content, which carry their provenance in `source`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderOrigin>,
    pub filename: String,
    pub registry_id: Option<String>,
    pub modrinth_id: Option<String>,
    pub source: String,
    #[serde(default)]
    pub source_url: Option<String>,
    pub version: Option<String>,
    pub sha256: String,
    pub installed_at: String,
    #[serde(default)]
    pub java_packages: Vec<String>,
    #[serde(default)]
    pub mod_jar_id: Option<String>,
    /// Additional loader-visible IDs supplied by this physical JAR (Fabric/
    /// Quilt `provides` aliases and explicitly declared nested modules).
    /// This is a cache only; health checks re-parse JAR metadata directly.
    #[serde(default)]
    pub provided_mod_ids: Vec<String>,
    /// Whether this mod is enabled. Disabled mods have their `.jar` renamed to
    /// `.jar.disabled` so the game does not load them, but the manifest entry is
    /// preserved for easy re-enabling.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Content type discriminator: `"mod"`, `"resourcepack"`, `"shader"`,
    /// `"datapack"`, or `"world"`.  Legacy manifests without this field
    /// deserialize as `"mod"`.
    #[serde(default = "default_mod_content_type")]
    pub content_type: String,
    /// Whether the user pinned this entry against updates.
    ///
    /// Pinned entries are excluded from "Update All" and show no update badge,
    /// but remain installable and updatable explicitly. Lives in the manifest
    /// beside `enabled` because it is the same kind of durable user intent and
    /// should travel with an exported instance.
    #[serde(default)]
    pub update_pinned: bool,
    /// Whether this entry was contributed by the instance's modpack rather than
    /// added by the user.
    ///
    /// Kept separate from `source` deliberately: `source` is a flat string that
    /// mixes origin with acquisition method and is rendered to the UI by
    /// `installed_content::source_label`, so overloading it would couple pack
    /// semantics to a display concern. Legacy manifests deserialize as `false`
    /// and are healed on load by [`heal_pack_managed`].
    #[serde(default)]
    pub pack_managed: bool,
    /// Whether Agora installed this entry only to satisfy another mod's
    /// dependency, rather than because the user asked for it.
    ///
    /// This is the provenance that makes orphan cleanup safe: without it every
    /// installed mod looks equally deliberate, and "nothing depends on this"
    /// would flag standalone mods like Sodium. Legacy manifests deserialize as
    /// `false`, which errs toward never proposing a removal.
    #[serde(default)]
    pub installed_as_dependency: bool,
    /// REQUIRED dependencies only (Fabric `depends`, Forge type=required);
    /// see `optional_deps` and `incompatible_deps` for non-required dep types.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Optional dependencies (Fabric `recommends`/`suggests`; Forge type=optional).
    #[serde(default)]
    pub optional_deps: Vec<String>,
    /// Incompatible dependencies (Forge type=incompatible). Stored but not
    /// used in install/remove flow for v1.
    #[serde(default)]
    pub incompatible_deps: Vec<String>,
}

/// Heuristic: whether a version string denotes a pre-release (alpha/beta/rc/snapshot).
pub fn is_prerelease_version(version: &str) -> bool {
    let lower = version.to_ascii_lowercase();
    lower.contains("alpha")
        || lower.contains("beta")
        || lower.contains("snapshot")
        || lower.contains("-rc")
        || lower.contains(".rc")
        || lower.contains("_rc")
        || lower.contains("-pre")
        || lower.contains(".pre")
        || lower.contains("_pre")
        || lower.contains("-dev")
        || lower.contains(".dev")
}

/// A candidate version returned by the mod version resolution API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModVersionCandidate {
    pub version: String,
    pub filename: String,
    pub download_url: String,
    pub mc_version: Option<String>,
    pub loader: Option<String>,
    pub release_date: Option<String>,
    pub is_compatible: bool,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub sha512: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub version_compat: String,
    /// Whether this version is a pre-release (alpha/beta/rc/snapshot).
    /// `true` defers it below stable releases when the stable-first sort is active.
    #[serde(default)]
    pub is_prerelease: bool,
    /// Which of the item's download sources produced this candidate
    /// (`github_release`, `modrinth_id`, `direct_hash`, ...).
    ///
    /// An item may list several sources in preference order, so the origin is
    /// a property of the candidate, not of the item. Install-time policy —
    /// whether the registry's pinned SHA-256 applies, which host is authorized
    /// — keys off this rather than the item's preferred strategy, otherwise a
    /// candidate served by a fallback would be judged against the wrong rules.
    #[serde(default)]
    pub source_strategy: Option<String>,
    /// The curator-reviewed identifier of that source (repo, project id, or
    /// pinned URL). Taken from the signed registry row, never re-derived from
    /// the resolved download URL.
    #[serde(default)]
    pub source_identifier: Option<String>,
}

/// Where an instance's modpack came from.
///
/// `Unknown` is a legitimate answer, not a failure: `.agora-pack.json`, plain
/// directory imports and Prism zips genuinely carry nothing but a display name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PackPlatform {
    Modrinth,
    TechnicSolder,
    TechnicZip,
    AgoraCurated,
    /// Copied out of another launcher (Prism, CurseForge, Modrinth app).
    Launcher,
    /// A local artifact with no upstream identity: `.agora-pack.json`, a Prism
    /// zip, or a directory import.
    LocalFile,
    /// Installed from a content provider's pack plan. `source_key` names the
    /// provider; `project_id` and `version_id` are the provider's own ids.
    Provider,
    #[default]
    Unknown,
}

/// Durable provenance for the pack an instance was created from.
///
/// Every identifier is optional on purpose. Several import paths only ever see
/// a display name, and recording `Some("")` for the rest would make "we don't
/// know" indistinguishable from "the pack has an empty id" — which would in
/// turn make pack updates silently wrong rather than merely unavailable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackOrigin {
    #[serde(default)]
    pub platform: PackPlatform,
    pub pack_name: String,
    /// Pack listing id (Modrinth project id, Technic slug, launcher source key).
    #[serde(default)]
    pub project_id: Option<String>,
    /// The specific release: Modrinth `versionId`, Technic build, CF file id.
    #[serde(default)]
    pub version_id: Option<String>,
    /// Human-readable version, e.g. `1.4.2`. Distinct from `version_id`, which
    /// is opaque; "is there a newer version?" is answered with this one.
    #[serde(default)]
    pub version_number: Option<String>,
    /// Where the pack artifact came from, **query string stripped**. Modrinth
    /// CDN links can be presigned, and manifests must never carry tokens.
    #[serde(default)]
    pub origin_url: Option<String>,
    /// Hash over the sorted `(relative_path, sha256)` inventory stored in
    /// `instance_pack_files`. Lets drift be detected even if the local database
    /// is lost, without carrying the whole list in every manifest.
    #[serde(default)]
    pub pack_content_hash: Option<String>,
    /// What the pack itself asked for, which can differ from what the instance
    /// currently runs after a loader change.
    #[serde(default)]
    pub pack_minecraft_version: Option<String>,
    #[serde(default)]
    pub pack_loader: Option<String>,
    #[serde(default)]
    pub pack_loader_version: Option<String>,
    /// Set only when `platform` is [`PackPlatform::Launcher`].
    #[serde(default)]
    pub launcher_kind: Option<String>,
    #[serde(default)]
    pub installation_key: Option<String>,
    #[serde(default)]
    pub source_key: Option<String>,
    /// Source instance id when this instance was cloned from another. The clone
    /// keeps pack identity so it can still be updated, but is not the original.
    #[serde(default)]
    pub cloned_from: Option<String>,
    /// RFC3339 timestamp of when the pack was last applied to this instance.
    pub installed_at: String,
}

/// Manifest revision written by the current code.
///
/// `1` is the implicit version of every manifest written before pack
/// provenance existed; those files have no version field at all, so absence is
/// what identifies them.
pub const CURRENT_MANIFEST_VERSION: u32 = 3;

/// Version assumed for a manifest that predates the field.
fn legacy_manifest_version() -> u32 {
    1
}

/// The lightweight JSON manifest that lives in each instance directory.
#[derive(Debug, Clone, Serialize)]
pub struct InstanceManifest {
    /// Absent in manifests written before pack provenance; see
    /// [`CURRENT_MANIFEST_VERSION`].
    #[serde(default = "legacy_manifest_version")]
    pub manifest_version: u32,
    /// Generic v3 fields. Minecraft callers keep their existing field access and
    /// IPC shape until the package move; disk writes use `to_disk_value`.
    #[serde(flatten)]
    pub game_data: GameManifestFields,
    pub instance_id: String,
    pub name: String,
    /// Display-name-only provenance from before [`PackOrigin`] existed. Kept
    /// for backward compatibility and as the backfill source; new code should
    /// read `pack_origin`.
    #[serde(default)]
    pub created_from_pack: Option<String>,
    /// `None` on every manifest written before this field existed, which is not
    /// the same as "not from a pack" — plenty of legacy pack instances have it.
    #[serde(default)]
    pub pack_origin: Option<PackOrigin>,
    pub minecraft_version: String,
    pub loader: String,
    pub loader_version: String,
    #[serde(default)]
    pub is_locked: bool,
    pub mods: Vec<InstalledMod>,
    #[serde(default)]
    pub resourcepacks: Vec<InstalledMod>,
    #[serde(default)]
    pub shaders: Vec<InstalledMod>,
    #[serde(default)]
    pub datapacks: Vec<InstalledMod>,
    #[serde(default)]
    pub worlds: Vec<InstalledMod>,
    #[serde(default)]
    pub user_preferences: serde_json::Value,
}

/// Generic manifest fields shared with future game packages. The flattened
/// extension map preserves fields this build does not own on normal writes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameManifestFields {
    #[serde(default = "minecraft_game_id", rename = "game")]
    pub game: agora_game_api::GameId,
    #[serde(default)]
    pub runtime_identity: Option<agora_game_api::RuntimeIdentity>,
    #[serde(default)]
    pub base: Option<agora_game_api::BaseReference>,
    #[serde(default)]
    pub frameworks: Vec<agora_game_api::InstalledFramework>,
    #[serde(default)]
    pub layers: agora_game_api::LayerStack,
    #[serde(flatten)]
    pub extensions: std::collections::BTreeMap<String, serde_json::Value>,
    /// Retain the entire future document, even if its known fields cannot be
    /// decoded. Its presence also prevents a caller lowering the version to write.
    #[serde(skip)]
    pub read_only_source: Option<serde_json::Value>,
    #[serde(skip)]
    pub minecraft_extensions: serde_json::Map<String, serde_json::Value>,
}

fn minecraft_game_id() -> agora_game_api::GameId {
    agora_game_api::GameId::minecraft()
}

impl Default for GameManifestFields {
    fn default() -> Self {
        Self {
            game: minecraft_game_id(),
            runtime_identity: None,
            base: None,
            frameworks: Vec::new(),
            layers: Default::default(),
            extensions: Default::default(),
            read_only_source: None,
            minecraft_extensions: Default::default(),
        }
    }
}

const MINECRAFT_MANIFEST_FIELDS: &[&str] = &[
    "minecraft_version",
    "loader",
    "loader_version",
    "mods",
    "resourcepacks",
    "shaders",
    "datapacks",
    "worlds",
];

impl InstanceManifest {
    pub fn is_read_only(&self) -> bool {
        self.manifest_version > CURRENT_MANIFEST_VERSION
            || self.game_data.read_only_source.is_some()
    }

    /// Populate the generic description without moving existing content.
    pub(crate) fn refresh_game_projection(&mut self) {
        let current = self;
        if current.game_data.game == "minecraft" {
            let store = current
                .game_data
                .runtime_identity
                .as_ref()
                .map(|r| r.store.clone())
                .unwrap_or_else(agora_game_api::StoreId::mojang);
            let build = current
                .game_data
                .runtime_identity
                .as_ref()
                .filter(|r| r.version == current.minecraft_version)
                .and_then(|r| r.build.clone());
            current.game_data.runtime_identity = Some(agora_game_api::RuntimeIdentity {
                game: agora_game_api::GameId::minecraft(),
                store,
                version: current.minecraft_version.clone(),
                build,
            });
            // For Minecraft, `frameworks` is derived entirely from `loader`: an
            // instance runs one loader at a time, and things layered on top of it
            // (Fabric API, Connector) are mods, not frameworks. So the projection
            // keeps only the current loader, which also drops a previous loader
            // whatever its id. Vanilla is the absence of a framework, not one
            // named "vanilla".
            if current.loader.is_empty() || current.loader == "vanilla" {
                current.game_data.frameworks.clear();
            } else {
                current
                    .game_data
                    .frameworks
                    .retain(|f| f.id == current.loader);
                if let Some(framework) = current
                    .game_data
                    .frameworks
                    .iter_mut()
                    .find(|f| f.id == current.loader)
                {
                    if framework.version != current.loader_version {
                        framework.version = current.loader_version.clone();
                        framework.supported_runtimes.clear();
                        framework.layers.clear();
                    }
                } else if let Ok(loader_id) = agora_game_api::FrameworkId::new(&current.loader) {
                    current
                        .game_data
                        .frameworks
                        .push(agora_game_api::InstalledFramework {
                            id: loader_id,
                            version: current.loader_version.clone(),
                            supported_runtimes: Vec::new(),
                            layers: Vec::new(),
                        });
                }
            }
            if current.game_data.layers.is_empty() {
                let default_layers: Vec<agora_game_api::Layer> = [
                    ("mods", "mod"),
                    ("resourcepacks", "resourcepack"),
                    ("shaderpacks", "shader"),
                    ("datapacks", "datapack"),
                    ("saves", "world"),
                ]
                .into_iter()
                .map(|(path, kind)| agora_game_api::Layer {
                    id: format!("minecraft:{kind}").try_into().unwrap(),
                    enabled: true,
                    mount_path: path.try_into().unwrap(),
                    source: agora_game_api::LayerSource::InstanceContent {
                        path: path.try_into().unwrap(),
                        content_kind: kind.into(),
                    },
                    whiteouts: Vec::new(),
                })
                .collect();
                current.game_data.layers = agora_game_api::LayerStack::new(default_layers)
                    .expect("default minecraft layers form a valid layer stack");
            }
        }
    }

    /// Canonical disk schema. Minecraft adapter fields go in the Minecraft
    /// section; this slice derives generic runtime/framework metadata from them.
    /// No file or directory is inspected or materialised here.
    pub fn to_disk_value(&self) -> Result<serde_json::Value, serde_json::Error> {
        if self.is_read_only() {
            return Err(<serde_json::Error as serde::ser::Error>::custom(
                "a manifest from a newer Agora is read-only",
            ));
        }
        let mut current = self.clone();
        heal_pack_managed(&mut current);
        current.refresh_game_projection();
        current.manifest_version = CURRENT_MANIFEST_VERSION;
        let mut value = serde_json::to_value(&current)?;
        let object = value
            .as_object_mut()
            .expect("manifest serializes as an object");
        let mut minecraft = current.game_data.minecraft_extensions;
        for field in MINECRAFT_MANIFEST_FIELDS {
            if let Some(value) = object.remove(*field) {
                minecraft.insert((*field).into(), value);
            }
        }
        if current.game_data.game == "minecraft" {
            object.insert("minecraft".into(), serde_json::Value::Object(minecraft));
        }
        Ok(value)
    }
}

#[derive(Deserialize)]
#[serde(remote = "InstanceManifest")]
struct InstanceManifestWire {
    #[serde(default = "legacy_manifest_version")]
    pub manifest_version: u32,
    #[serde(flatten)]
    pub game_data: GameManifestFields,
    pub instance_id: String,
    pub name: String,
    #[serde(default)]
    pub created_from_pack: Option<String>,
    #[serde(default)]
    pub pack_origin: Option<PackOrigin>,
    pub minecraft_version: String,
    pub loader: String,
    pub loader_version: String,
    #[serde(default)]
    pub is_locked: bool,
    pub mods: Vec<InstalledMod>,
    #[serde(default)]
    pub resourcepacks: Vec<InstalledMod>,
    #[serde(default)]
    pub shaders: Vec<InstalledMod>,
    #[serde(default)]
    pub datapacks: Vec<InstalledMod>,
    #[serde(default)]
    pub worlds: Vec<InstalledMod>,
    #[serde(default)]
    pub user_preferences: serde_json::Value,
}

impl<'de> Deserialize<'de> for InstanceManifest {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let original = serde_json::Value::deserialize(deserializer)?;
        let version = original
            .get("manifest_version")
            .and_then(|v| v.as_u64())
            .unwrap_or(1);
        let future = version > u64::from(CURRENT_MANIFEST_VERSION);
        let mut value = original.clone();
        let object = value
            .as_object_mut()
            .ok_or_else(|| D::Error::custom("manifest must be an object"))?;
        let mut minecraft_extensions = serde_json::Map::new();
        if let Some(serde_json::Value::Object(mut minecraft)) = object.remove("minecraft") {
            for field in MINECRAFT_MANIFEST_FIELDS {
                if let Some(value) = minecraft.remove(*field) {
                    object.insert((*field).into(), value);
                }
            }
            minecraft_extensions = minecraft;
        }
        // Non-Minecraft and future documents need no Minecraft-only fields.
        if future
            || object
                .get("game")
                .and_then(|v| v.as_str())
                .is_some_and(|g| g != "minecraft")
        {
            for field in ["minecraft_version", "loader", "loader_version"] {
                object.entry(field).or_insert_with(|| serde_json::json!(""));
            }
            object
                .entry("mods")
                .or_insert_with(|| serde_json::json!([]));
        }
        let decoded = InstanceManifestWire::deserialize(value);
        let mut manifest = match decoded {
            Ok(manifest) => manifest,
            Err(_) if future => {
                // A future schema may change enums/types or remove our fields.
                // Expose a read-only identity/summary without interpreting it.
                InstanceManifestWire::deserialize(serde_json::json!({
                    "manifest_version": version.min(u64::from(u32::MAX)),
                    "instance_id": original.get("instance_id").and_then(|v| v.as_str()).unwrap_or(""),
                    "name": original.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                    "game": original.get("game").and_then(|v| v.as_str()).unwrap_or("minecraft"),
                    "minecraft_version": "", "loader": "", "loader_version": "", "mods": []
                })).map_err(D::Error::custom)?
            }
            Err(error) => return Err(D::Error::custom(error)),
        };
        manifest.game_data.minecraft_extensions = minecraft_extensions;
        if future {
            manifest.game_data.read_only_source = Some(original);
        }
        Ok(manifest)
    }
}

/// Whether a legacy `source` string implies the entry came from a pack.
///
/// Only used to heal manifests written before `pack_managed` existed. Hyphens
/// are normalized because `.mrpack` import writes `modrinth-pack` while the
/// launcher importer writes `imported_<launcher>`.
pub fn source_implies_pack_managed(source: &str) -> bool {
    let normalized = source.trim().to_ascii_lowercase().replace('-', "_");
    normalized == "modrinth_pack" || normalized.starts_with("imported_")
}

/// Populate `pack_managed` on entries loaded from a pre-v2 manifest.
///
/// Deliberately one-way: it can only set the flag, never clear it, so a
/// deliberate user override survives a reload. Newer manifests already carry
/// the field and are left alone.
pub fn heal_pack_managed(manifest: &mut InstanceManifest) {
    if manifest.manifest_version >= 2 {
        return;
    }
    for entry in manifest
        .mods
        .iter_mut()
        .chain(manifest.resourcepacks.iter_mut())
        .chain(manifest.shaders.iter_mut())
        .chain(manifest.datapacks.iter_mut())
        .chain(manifest.worlds.iter_mut())
    {
        if !entry.pack_managed && source_implies_pack_managed(&entry.source) {
            entry.pack_managed = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_installed_mod_missing_java_packages() {
        let json = r#"{
            "filename": "test.jar",
            "source": "local",
            "sha256": "abc123",
            "installed_at": "2024-01-01T00:00:00Z"
        }"#;
        let mod_: InstalledMod = serde_json::from_str(json).unwrap();
        assert_eq!(mod_.java_packages, Vec::<String>::new());
    }

    #[test]
    fn test_installed_mod_missing_mod_jar_id() {
        let json = r#"{
            "filename": "test.jar",
            "source": "local",
            "sha256": "abc123",
            "installed_at": "2024-01-01T00:00:00Z"
        }"#;
        let mod_: InstalledMod = serde_json::from_str(json).unwrap();
        assert!(mod_.mod_jar_id.is_none());
    }

    #[test]
    fn test_installed_mod_missing_depends_on() {
        let json = r#"{
            "filename": "test.jar",
            "source": "local",
            "sha256": "abc123",
            "installed_at": "2024-01-01T00:00:00Z"
        }"#;
        let mod_: InstalledMod = serde_json::from_str(json).unwrap();
        assert_eq!(mod_.depends_on, Vec::<String>::new());
    }

    #[test]
    fn test_installed_mod_missing_optional_deps() {
        let json = r#"{
            "filename": "test.jar",
            "source": "local",
            "sha256": "abc123",
            "installed_at": "2024-01-01T00:00:00Z"
        }"#;
        let mod_: InstalledMod = serde_json::from_str(json).unwrap();
        assert_eq!(mod_.optional_deps, Vec::<String>::new());
    }

    #[test]
    fn test_installed_mod_missing_incompatible_deps() {
        let json = r#"{
            "filename": "test.jar",
            "source": "local",
            "sha256": "abc123",
            "installed_at": "2024-01-01T00:00:00Z"
        }"#;
        let mod_: InstalledMod = serde_json::from_str(json).unwrap();
        assert_eq!(mod_.incompatible_deps, Vec::<String>::new());
    }

    #[test]
    fn test_installed_mod_minimal_fields() {
        let json = r#"{
            "filename": "test.jar",
            "source": "local",
            "sha256": "abc123",
            "installed_at": "2024-01-01T00:00:00Z"
        }"#;
        let mod_: InstalledMod = serde_json::from_str(json).unwrap();
        assert_eq!(mod_.filename, "test.jar");
        assert_eq!(mod_.source, "local");
        assert_eq!(mod_.sha256, "abc123");
        assert_eq!(mod_.installed_at, "2024-01-01T00:00:00Z");
        assert_eq!(mod_.registry_id, None);
        assert_eq!(mod_.modrinth_id, None);
        assert_eq!(mod_.version, None);
        assert_eq!(mod_.java_packages, Vec::<String>::new());
        assert_eq!(mod_.mod_jar_id, None);
        assert_eq!(mod_.provided_mod_ids, Vec::<String>::new());
        assert_eq!(mod_.depends_on, Vec::<String>::new());
        assert_eq!(mod_.optional_deps, Vec::<String>::new());
        assert_eq!(mod_.incompatible_deps, Vec::<String>::new());
    }

    #[test]
    fn test_instance_manifest_with_mods() {
        let json = r#"{
            "instance_id": "my-instance",
            "name": "My Instance",
            "minecraft_version": "1.20.1",
            "loader": "fabric",
            "loader_version": "0.15.0",
            "mods": [
                {
                    "filename": "cloth-config.jar",
                    "source": "modrinth",
                    "sha256": "def456",
                    "installed_at": "2024-01-01T00:00:00Z",
                    "depends_on": ["fabric-api"]
                }
            ],
            "user_preferences": {}
        }"#;
        let manifest: InstanceManifest = serde_json::from_str(json).unwrap();
        assert_eq!(manifest.instance_id, "my-instance");
        assert_eq!(manifest.name, "My Instance");
        assert_eq!(manifest.mods.len(), 1);
        assert_eq!(manifest.mods[0].filename, "cloth-config.jar");
        assert_eq!(manifest.mods[0].depends_on, vec!["fabric-api"]);
    }

    #[test]
    fn test_instance_manifest_roundtrip() {
        let manifest = InstanceManifest {
            manifest_version: crate::models::CURRENT_MANIFEST_VERSION,
            game_data: Default::default(),
            pack_origin: None,
            instance_id: "rt-instance".to_string(),
            name: "RoundTrip".to_string(),
            created_from_pack: Some("some-pack".to_string()),
            minecraft_version: "1.21.0".to_string(),
            loader: "forge".to_string(),
            loader_version: "52.0.0".to_string(),
            is_locked: true,
            mods: vec![InstalledMod {
                provider: None,
                update_pinned: false,
                pack_managed: false,
                installed_as_dependency: false,
                filename: "rt-mod.jar".to_string(),
                registry_id: Some("reg-1".to_string()),
                modrinth_id: None,
                source: "github".to_string(),
                source_url: Some("https://example.com/rt-mod.jar".to_string()),
                version: Some("1.0.0".to_string()),
                sha256: "sha123".to_string(),
                installed_at: "2024-06-01T12:00:00Z".to_string(),
                java_packages: vec!["com.example.mod".to_string()],
                mod_jar_id: Some("jar-1".to_string()),
                depends_on: vec!["core-lib".to_string()],
                optional_deps: vec!["opt-mod".to_string()],
                incompatible_deps: vec!["bad-mod".to_string()],
                provided_mod_ids: vec!["nested_api".to_string(), "legacy_alias".to_string()],
                enabled: true,
                content_type: "mod".to_string(),
            }],
            resourcepacks: vec![],
            shaders: vec![],
            datapacks: vec![],
            worlds: vec![],
            user_preferences: serde_json::json!({"key": "value"}),
        };

        let serialized = serde_json::to_string(&manifest).unwrap();
        let deserialized: InstanceManifest = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized.instance_id, manifest.instance_id);
        assert_eq!(deserialized.name, manifest.name);
        assert_eq!(deserialized.created_from_pack, manifest.created_from_pack);
        assert_eq!(deserialized.minecraft_version, manifest.minecraft_version);
        assert_eq!(deserialized.loader, manifest.loader);
        assert_eq!(deserialized.loader_version, manifest.loader_version);
        assert_eq!(deserialized.is_locked, manifest.is_locked);
        assert_eq!(deserialized.mods.len(), manifest.mods.len());
        assert_eq!(deserialized.mods[0].filename, manifest.mods[0].filename);
        assert_eq!(
            deserialized.mods[0].registry_id,
            manifest.mods[0].registry_id
        );
        assert_eq!(
            deserialized.mods[0].modrinth_id,
            manifest.mods[0].modrinth_id
        );
        assert_eq!(deserialized.mods[0].source, manifest.mods[0].source);
        assert_eq!(deserialized.mods[0].version, manifest.mods[0].version);
        assert_eq!(deserialized.mods[0].sha256, manifest.mods[0].sha256);
        assert_eq!(
            deserialized.mods[0].installed_at,
            manifest.mods[0].installed_at
        );
        assert_eq!(
            deserialized.mods[0].java_packages,
            manifest.mods[0].java_packages
        );
        assert_eq!(deserialized.mods[0].mod_jar_id, manifest.mods[0].mod_jar_id);
        assert_eq!(
            deserialized.mods[0].provided_mod_ids,
            manifest.mods[0].provided_mod_ids
        );
        assert_eq!(deserialized.mods[0].depends_on, manifest.mods[0].depends_on);
        assert_eq!(
            deserialized.mods[0].optional_deps,
            manifest.mods[0].optional_deps
        );
        assert_eq!(
            deserialized.mods[0].incompatible_deps,
            manifest.mods[0].incompatible_deps
        );
        assert_eq!(deserialized.user_preferences, manifest.user_preferences);
    }

    /// A manifest exactly as written before pack provenance existed: no
    /// `manifest_version`, no `pack_origin`, and no `pack_managed` on entries.
    /// If this ever fails to load, every existing user instance is bricked.
    fn legacy_manifest_json() -> &'static str {
        r#"{
            "instance_id": "legacy-inst",
            "name": "Legacy Pack",
            "created_from_pack": "Some Pack",
            "minecraft_version": "1.20.1",
            "loader": "fabric",
            "loader_version": "0.15.0",
            "mods": [
                {"filename":"a.jar","registry_id":null,"modrinth_id":null,
                 "source":"modrinth-pack","version":"1","sha256":"aa","installed_at":"t"},
                {"filename":"b.jar","registry_id":"sodium","modrinth_id":null,
                 "source":"registry","version":"1","sha256":"bb","installed_at":"t"},
                {"filename":"c.jar","registry_id":null,"modrinth_id":"x",
                 "source":"imported_prism","version":"1","sha256":"cc","installed_at":"t"},
                {"filename":"d.jar","registry_id":null,"modrinth_id":"y",
                 "source":"modrinth_raw","version":"1","sha256":"dd","installed_at":"t"}
            ]
        }"#
    }

    #[test]
    fn legacy_manifest_without_new_fields_still_loads() {
        let manifest: InstanceManifest = serde_json::from_str(legacy_manifest_json()).unwrap();
        assert_eq!(manifest.manifest_version, 1, "absence must read as legacy");
        assert!(manifest.pack_origin.is_none());
        assert_eq!(manifest.created_from_pack.as_deref(), Some("Some Pack"));
        assert!(manifest.mods.iter().all(|entry| !entry.pack_managed));
        // No provenance recorded means "the user wanted it", which is the only
        // safe reading: orphan cleanup must never propose removing a mod it
        // cannot prove was auto-installed.
        assert!(manifest
            .mods
            .iter()
            .all(|entry| !entry.installed_as_dependency));
    }

    #[test]
    fn heal_marks_pack_sources_and_leaves_user_installs_alone() {
        let mut manifest: InstanceManifest = serde_json::from_str(legacy_manifest_json()).unwrap();
        heal_pack_managed(&mut manifest);
        let flags: Vec<(&str, bool)> = manifest
            .mods
            .iter()
            .map(|entry| (entry.source.as_str(), entry.pack_managed))
            .collect();
        assert_eq!(
            flags,
            vec![
                ("modrinth-pack", true),
                ("registry", false),
                ("imported_prism", true),
                ("modrinth_raw", false),
            ]
        );
    }

    #[test]
    fn heal_is_a_noop_once_the_manifest_is_current() {
        let mut manifest: InstanceManifest = serde_json::from_str(legacy_manifest_json()).unwrap();
        manifest.manifest_version = CURRENT_MANIFEST_VERSION;
        heal_pack_managed(&mut manifest);
        assert!(
            manifest.mods.iter().all(|entry| !entry.pack_managed),
            "a current manifest already carries the field; healing must not second-guess it"
        );
    }

    #[test]
    fn heal_never_clears_an_existing_flag() {
        let mut manifest: InstanceManifest = serde_json::from_str(legacy_manifest_json()).unwrap();
        manifest.mods[1].pack_managed = true; // `registry`, which the rule would not set
        heal_pack_managed(&mut manifest);
        assert!(manifest.mods[1].pack_managed, "healing is one-way");
    }

    #[test]
    fn pack_origin_round_trips_fully_populated() {
        let origin = PackOrigin {
            platform: PackPlatform::TechnicSolder,
            pack_name: "Hexxit".into(),
            project_id: Some("hexxit".into()),
            version_id: Some("build-42".into()),
            version_number: Some("1.4.2".into()),
            origin_url: Some("https://example.invalid/pack.zip".into()),
            pack_content_hash: Some("deadbeef".into()),
            pack_minecraft_version: Some("1.20.1".into()),
            pack_loader: Some("forge".into()),
            pack_loader_version: Some("47.2.0".into()),
            launcher_kind: None,
            installation_key: None,
            source_key: None,
            cloned_from: Some("source-inst".into()),
            installed_at: "2026-08-31T00:00:00Z".into(),
        };
        let json = serde_json::to_string(&origin).unwrap();
        assert_eq!(serde_json::from_str::<PackOrigin>(&json).unwrap(), origin);
    }

    #[test]
    fn pack_platform_serializes_as_snake_case() {
        // These strings go to disk; renaming one later is a migration.
        for (platform, expected) in [
            (PackPlatform::Modrinth, "\"modrinth\""),
            (PackPlatform::TechnicSolder, "\"technic_solder\""),
            (PackPlatform::TechnicZip, "\"technic_zip\""),
            (PackPlatform::AgoraCurated, "\"agora_curated\""),
            (PackPlatform::Launcher, "\"launcher\""),
            (PackPlatform::LocalFile, "\"local_file\""),
            (PackPlatform::Unknown, "\"unknown\""),
        ] {
            assert_eq!(serde_json::to_string(&platform).unwrap(), expected);
        }
        assert_eq!(PackPlatform::default(), PackPlatform::Unknown);
    }

    #[test]
    fn pack_origin_omitted_fields_default_rather_than_failing() {
        // A future writer may emit only what it knows.
        let minimal = r#"{"pack_name":"Minimal","installed_at":"t"}"#;
        let origin: PackOrigin = serde_json::from_str(minimal).unwrap();
        assert_eq!(origin.platform, PackPlatform::Unknown);
        assert!(origin.project_id.is_none());
        assert!(origin.version_id.is_none());
    }

    #[test]
    fn game_manifest_contract_retains_skyrim_and_bepinex_tracer_data() {
        for (game, version, framework, store) in [
            ("skyrim", "1.6.1170.0", "skse", "steam"),
            ("valheim", "0.218.15", "bepinex", "community-store"),
        ] {
            let runtime = serde_json::json!({"game": game, "store": store, "version": version, "build": "build-7"});
            let value = serde_json::json!({
                "manifest_version": 3, "instance_id": "tracer", "name": "Tracer", "game": game,
                "runtime_identity": runtime,
                "base": {"kind": "pinned", "id": "base-7", "runtime": runtime, "mode": "linked"},
                "frameworks": [{"id": framework, "version": "2.2.6", "layers": ["framework"],
                    "supported_runtimes": [{"game": game, "stores": [store],
                        "versions": {"kind": "exact", "value": [version]}, "builds": ["build-7"]}]}],
                "layers": [
                    {"id": "mod-2", "enabled": true, "mount_path": "Data", "whiteouts": [],
                        "source": {"kind": "content", "content": "sha256:content"}},
                    {"id": "nemesis", "enabled": true, "mount_path": "Data", "whiteouts": [],
                        "source": {"kind": "generated", "tool": "nemesis", "generation": "imported",
                            "inputs": {"kind": "unknown"}}},
                    {"id": "writes", "enabled": true, "mount_path": "", "whiteouts": ["removed.txt"],
                        "source": {"kind": "writable", "path": "writable"}},
                    {"id": "tool-run", "enabled": true, "mount_path": "", "whiteouts": [],
                        "source": {"kind": "staging", "tool": "nemesis", "run": "7", "path": "staging/7"}}
                ],
                "load_order": {"entries": [{"id": "plugin-1", "enabled": true, "locked": true}],
                    "rules": [{"kind": "requires", "item": "plugin-1", "master": "master.esm"}]},
                "user_preferences": {"local_saves": true}
            });
            let manifest: InstanceManifest = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(manifest.game_data.game, game);
            assert!(
                manifest.mods.is_empty(),
                "other games need no Minecraft content section"
            );
            let identity = manifest.game_data.runtime_identity.as_ref().unwrap();
            assert_eq!(
                manifest.game_data.frameworks[0].supported_runtimes[0].check(identity),
                agora_game_api::Support::Supported
            );
            assert!(manifest.game_data.frameworks[0].supported_runtimes[0].matches(identity));
            assert!(!manifest.game_data.layers[1].is_write_target());
            assert!(manifest.game_data.layers[2].is_write_target());
            let order: agora_game_api::LoadOrder =
                serde_json::from_value(value["load_order"].clone()).unwrap();
            assert_eq!(order.entries[0].id, "plugin-1");
            assert_eq!(
                manifest.game_data.layers[0].id, "mod-2",
                "the two orders are independent"
            );
            let written = manifest.to_disk_value().unwrap();
            for field in [
                "runtime_identity",
                "base",
                "frameworks",
                "layers",
                "load_order",
                "user_preferences",
            ] {
                assert_eq!(written[field], value[field], "{game}: preserve {field}");
            }
            assert!(written.get("minecraft").is_none());
            assert!(written.get("minecraft_version").is_none());
        }
    }

    #[test]
    fn test_refresh_game_projection_removes_unfamiliar_loader_when_switching() {
        let mut manifest = InstanceManifest {
            manifest_version: 3,
            game_data: Default::default(),
            instance_id: "mc-1".into(),
            name: "Test".into(),
            created_from_pack: None,
            pack_origin: None,
            minecraft_version: "1.20.1".into(),
            loader: "liteloader".into(),
            loader_version: "0.1.0".into(),
            is_locked: false,
            mods: Vec::new(),
            resourcepacks: Vec::new(),
            shaders: Vec::new(),
            datapacks: Vec::new(),
            worlds: Vec::new(),
            user_preferences: serde_json::Value::Null,
        };

        // First projection with unfamiliar loader
        manifest.refresh_game_projection();
        assert_eq!(manifest.game_data.frameworks.len(), 1);
        assert_eq!(manifest.game_data.frameworks[0].id, "liteloader");

        // Switch to fabric: unfamiliar loader must be removed
        manifest.loader = "fabric".into();
        manifest.loader_version = "0.15.11".into();
        manifest.refresh_game_projection();
        assert_eq!(manifest.game_data.frameworks.len(), 1);
        assert_eq!(manifest.game_data.frameworks[0].id, "fabric");
        assert_eq!(manifest.game_data.frameworks[0].version, "0.15.11");

        // Switch to vanilla: all loader frameworks must be removed
        manifest.loader = "vanilla".into();
        manifest.loader_version = "".into();
        manifest.refresh_game_projection();
        assert!(manifest.game_data.frameworks.is_empty());
    }

    #[test]
    fn test_refresh_game_projection_version_bump_clears_stale_runtimes_and_layers() {
        let mut manifest = InstanceManifest {
            manifest_version: 3,
            game_data: Default::default(),
            instance_id: "mc-1".into(),
            name: "Test".into(),
            created_from_pack: None,
            pack_origin: None,
            minecraft_version: "1.20.1".into(),
            loader: "fabric".into(),
            loader_version: "0.14.0".into(),
            is_locked: false,
            mods: Vec::new(),
            resourcepacks: Vec::new(),
            shaders: Vec::new(),
            datapacks: Vec::new(),
            worlds: Vec::new(),
            user_preferences: serde_json::Value::Null,
        };

        manifest.refresh_game_projection();
        assert_eq!(manifest.game_data.frameworks.len(), 1);

        // Record supported_runtimes and layers describing 0.14.0
        manifest.game_data.frameworks[0].supported_runtimes.push(
            agora_game_api::RuntimeConstraint {
                game: agora_game_api::GameId::minecraft(),
                stores: vec![],
                versions: agora_game_api::VersionConstraint::Exact(vec!["1.20.1".into()]),
                builds: vec![],
            },
        );
        manifest.game_data.frameworks[0]
            .layers
            .push(agora_game_api::LayerId::new("fabric-layer").unwrap());

        // Refresh without bumping version: records must be kept
        manifest.refresh_game_projection();
        assert_eq!(manifest.game_data.frameworks[0].supported_runtimes.len(), 1);
        assert_eq!(manifest.game_data.frameworks[0].layers.len(), 1);

        // Bump loader version: records must be cleared
        manifest.loader_version = "0.15.0".into();
        manifest.refresh_game_projection();
        assert_eq!(manifest.game_data.frameworks[0].version, "0.15.0");
        assert!(manifest.game_data.frameworks[0]
            .supported_runtimes
            .is_empty());
        assert!(manifest.game_data.frameworks[0].layers.is_empty());
    }
}
