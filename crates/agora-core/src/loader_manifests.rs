use crate::error::{LauncherError, LauncherResult};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock, RwLock};

// ---------------------------------------------------------------------------
// Embedded compile-time data (fallback when registry.db is unavailable).
// ---------------------------------------------------------------------------

/// Embedded copy of `loader-manifests/loader_manifests.json`.
const LOADER_MANIFESTS: &str = include_str!("../../../loader-manifests/loader_manifests.json");

/// Embedded copy of `loader-manifests/minecraft_versions.json`.
const MC_VERSIONS: &str = include_str!("../../../loader-manifests/minecraft_versions.json");

// ---------------------------------------------------------------------------
// Manifest types
// ---------------------------------------------------------------------------

/// Release channel of a pinned loader distribution.
///
/// Defaults to [`Self::Stable`] when absent, so legacy catalog entries keep
/// their historical behavior: they are eligible for automatic recommendation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoaderReleaseChannel {
    #[default]
    Stable,
    Prerelease,
}

/// Capabilities provided by a pinned loader distribution.
///
/// `provided_versions` maps a normalized (lowercase) capability name to the
/// version this distribution provides for it. Explicit entries win over and
/// augment the legacy distribution identity; language-loader capabilities
/// (javafml/lowcodefml) are never synthesized from a Forge/NeoForge release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoaderCapabilities {
    /// The loader family, e.g. `fabric`, `quilt`, `forge`, `neoforge`.
    pub distribution_id: String,
    /// The distribution's own version (its capability under its own id).
    pub distribution_version: String,
    /// Explicitly provided capabilities, keyed by lowercase capability name.
    #[serde(default)]
    pub provided_versions: BTreeMap<String, String>,
}

/// Distribution capability provided by each known loader family. The legacy
/// catalog predates `provided_versions`, so these exact tuples are the only
/// capability identities that may be derived from the distribution itself.
const DISTRIBUTION_CAPABILITY: [(&str, &str); 4] = [
    ("fabric", "fabricloader"),
    ("quilt", "quilt_loader"),
    ("forge", "forge"),
    ("neoforge", "neoforge"),
];

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct LoaderEntry {
    pub mc_version: String,
    pub loader_version: String,
    pub source_url: String,
    pub sha256: String,
    pub file_name: String,
    pub file_type: String,
    /// SHA-256 of the version.json embedded inside an installer JAR.
    #[serde(default)]
    pub version_json_sha256: Option<String>,
    /// Install profile spec version (0 for legacy, 1+ for modern).
    #[serde(default)]
    pub installer_spec: Option<u64>,
    /// Capabilities explicitly provided by this pinned distribution, keyed
    /// by lowercase capability name. Absent in legacy catalogs; see
    /// [`LoaderEntry::capability_version`] for the safe fallback.
    #[serde(default)]
    pub provided_versions: BTreeMap<String, String>,
    /// Release channel of this distribution. Defaults to `Stable` so legacy
    /// entries remain eligible for recommendation.
    #[serde(default)]
    pub release_channel: LoaderReleaseChannel,
    /// Curator-set recommendation priority: a lower explicit rank is
    /// preferred over an unranked entry. Absent = unranked.
    #[serde(default)]
    pub recommendation_rank: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct LoaderEntrySerde {
    mc_version: String,
    loader_version: String,
    source_url: String,
    sha256: String,
    file_name: String,
    file_type: String,
    #[serde(default)]
    version_json_sha256: Option<String>,
    #[serde(default)]
    installer_spec: Option<u64>,
    #[serde(default)]
    provided_versions: BTreeMap<String, String>,
    #[serde(default)]
    release_channel: Option<LoaderReleaseChannel>,
    #[serde(default)]
    recommendation_rank: Option<u32>,
}

impl<'de> Deserialize<'de> for LoaderEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = LoaderEntrySerde::deserialize(deserializer)?;
        Ok(Self {
            release_channel: raw
                .release_channel
                .unwrap_or_else(|| infer_release_channel(&raw.loader_version)),
            mc_version: raw.mc_version,
            loader_version: raw.loader_version,
            source_url: raw.source_url,
            sha256: raw.sha256,
            file_name: raw.file_name,
            file_type: raw.file_type,
            version_json_sha256: raw.version_json_sha256,
            installer_spec: raw.installer_spec,
            provided_versions: raw.provided_versions,
            recommendation_rank: raw.recommendation_rank,
        })
    }
}

fn infer_release_channel(version: &str) -> LoaderReleaseChannel {
    let lower = version.to_ascii_lowercase();
    let prerelease = lower
        .split(|character: char| matches!(character, '.' | '-' | '_'))
        .any(|token| {
            token == "alpha"
                || token == "beta"
                || token == "snapshot"
                || token == "pre"
                || token.starts_with("alpha")
                || token.starts_with("beta")
                || token.starts_with("rc")
                || token.starts_with("snapshot")
        });
    if prerelease {
        LoaderReleaseChannel::Prerelease
    } else {
        LoaderReleaseChannel::Stable
    }
}

/// Public catalog of modloader entries and domain allowlist.
///
/// Loaded from the signed `registry.db` at runtime when available, falling
/// back to the compile-time embedded copy.  Mirrors the `loader_catalog`
/// singleton table schema in the registry.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LoaderCatalog {
    #[serde(default)]
    pub domain_allowlist: Vec<String>,
    #[serde(default)]
    pub loaders: std::collections::BTreeMap<String, Vec<LoaderEntry>>,
}

// ---------------------------------------------------------------------------
// Static caches
// ---------------------------------------------------------------------------

/// Embedded fallback — parsed at first access and never changed.
static MANIFEST: OnceLock<Arc<LoaderCatalog>> = OnceLock::new();

/// Runtime catalog populated from the signed `loader_catalog` table in
/// `registry.db`. Replacing the `Arc` makes refreshes visible to new callers
/// without invalidating snapshots already held by an active operation.
static CATALOG_OVERRIDE: RwLock<Option<Arc<LoaderCatalog>>> = RwLock::new(None);

static MC_VERSIONS_LIST: OnceLock<Vec<String>> = OnceLock::new();

fn embedded_catalog() -> Arc<LoaderCatalog> {
    MANIFEST
        .get_or_init(|| {
            Arc::new(
                serde_json::from_str(LOADER_MANIFESTS).unwrap_or_else(|_| LoaderCatalog {
                    domain_allowlist: Vec::new(),
                    loaders: std::collections::BTreeMap::new(),
                }),
            )
        })
        .clone()
}

/// Return an immutable snapshot of the effective catalog.
fn catalog() -> Arc<LoaderCatalog> {
    CATALOG_OVERRIDE
        .read()
        .ok()
        .and_then(|guard| guard.clone())
        .unwrap_or_else(embedded_catalog)
}

/// Parse the embedded Mojang version list once and cache the result.
fn mc_versions_list() -> &'static [String] {
    MC_VERSIONS_LIST.get_or_init(|| serde_json::from_str(MC_VERSIONS).unwrap_or_default())
}

// ---------------------------------------------------------------------------
// LoaderCatalog — instance methods
// ---------------------------------------------------------------------------

impl LoaderEntry {
    /// Look up the version this entry provides for a capability.
    ///
    /// Explicit `provided_versions` entries always win (and may augment the
    /// legacy identity). When absent, a legacy catalog yields the
    /// distribution identity only for the exact pinned catalog tuple:
    /// `fabric` → `fabricloader`, `quilt` → `quilt_loader`, `forge` → `forge`,
    /// `neoforge` → `neoforge`, each equal to the pinned loader version.
    /// Language-loader capabilities (javafml/lowcodefml) are NEVER
    /// synthesized from a visible Forge/NeoForge release.
    ///
    /// Capability names are normalized to lowercase deterministically.
    pub fn capability_version(&self, loader: &str, capability: &str) -> Option<&str> {
        let capability = capability.to_lowercase();
        if let Some(version) = self.provided_versions.get(&capability) {
            return Some(version);
        }
        // Malformed catalogs might carry non-lowercase keys; resolve them
        // case-insensitively without mutating the map.
        if !self.provided_versions.is_empty() && !self.provided_versions.contains_key(&capability) {
            let folded = self
                .provided_versions
                .keys()
                .find(|k| k.eq_ignore_ascii_case(&capability));
            if let Some(folded) = folded {
                return self.provided_versions.get(folded).map(String::as_str);
            }
        }
        let legacy = DISTRIBUTION_CAPABILITY
            .iter()
            .any(|(family, own)| *family == loader && *own == capability);
        legacy.then_some(self.loader_version.as_str())
    }

    /// The capabilities this entry provides for its distribution.
    ///
    /// The distribution identity (the family's own capability) is always
    /// present: explicitly via `provided_versions`, or via the legacy
    /// fallback for the exact pinned catalog tuple. Language-loader
    /// capabilities are never synthesized.
    pub fn capabilities(&self, loader: &str) -> LoaderCapabilities {
        let distribution_id = loader.to_lowercase();
        let mut provided_versions = self.provided_versions.clone();
        if let Some((_, own)) = DISTRIBUTION_CAPABILITY
            .iter()
            .find(|(family, _)| *family == distribution_id.as_str())
        {
            provided_versions
                .entry(own.to_string())
                .or_insert_with(|| self.loader_version.clone());
        }
        let distribution_version = DISTRIBUTION_CAPABILITY
            .iter()
            .find(|(family, _)| *family == distribution_id.as_str())
            .and_then(|(_, own)| provided_versions.get(*own).cloned())
            .unwrap_or_else(|| self.loader_version.clone());
        LoaderCapabilities {
            distribution_id,
            distribution_version,
            provided_versions,
        }
    }
}

impl LoaderCatalog {
    /// Parse the embeded compile-time copy.
    ///
    /// Panics if the embedded JSON is structurally invalid (this is a
    /// compile-time invariant checked by tests).
    pub fn embedded() -> Self {
        serde_json::from_str(LOADER_MANIFESTS)
            .expect("embedded loader_manifests.json should be valid")
    }

    /// Load the loader catalog from the `loader_catalog` table in a signed
    /// `registry.db`.
    ///
    /// Returns `Ok(None)` when no loader_catalog row exists (pre-catalog
    /// registry releases).
    pub fn from_registry(conn: &rusqlite::Connection) -> LauncherResult<Option<Self>> {
        let json: Option<String> = conn
            .query_row(
                "SELECT catalog_json FROM loader_catalog WHERE singleton_id = 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| LauncherError::Generic {
                code: "ERR_LOADER_CATALOG_QUERY".into(),
                message: format!("Failed to query loader_catalog: {e}"),
            })?;

        let Some(json) = json else { return Ok(None) };

        let catalog: Self = serde_json::from_str(&json).map_err(|e| LauncherError::Generic {
            code: "ERR_LOADER_CATALOG_PARSE".into(),
            message: format!("Failed to parse loader_catalog JSON: {e}"),
        })?;

        Ok(Some(catalog))
    }

    /// Merge a signed registry catalog with the catalog bundled into the app.
    /// Both sources contain pinned, reviewed artifacts. Their union prevents an
    /// older cached registry from removing loader versions shipped by a newer
    /// app, while conflicting definitions for the same tuple fail closed.
    pub fn merge_with_embedded(registry: Option<Self>) -> LauncherResult<Self> {
        let mut merged = Self::embedded();
        let Some(registry) = registry else {
            return Ok(merged);
        };

        for domain in registry.domain_allowlist {
            if !merged.domain_allowlist.contains(&domain) {
                merged.domain_allowlist.push(domain);
            }
        }
        merged.domain_allowlist.sort();

        for (loader, entries) in registry.loaders {
            let target = merged.loaders.entry(loader.clone()).or_default();
            for entry in entries {
                Self::merge_entry_into(target, entry, &loader)?;
            }
        }
        Ok(merged)
    }

    /// Load the verified union of the signed registry and embedded catalogs.
    ///
    /// Pass `Some(&conn)` when a registry database is available; pass `None`
    /// to always use the embedded copy.
    pub fn effective(registry: Option<&rusqlite::Connection>) -> LauncherResult<Self> {
        if let Some(conn) = registry {
            match Self::from_registry(conn) {
                Ok(catalog) => return Self::merge_with_embedded(catalog),
                Err(e) => {
                    #[cfg(debug_assertions)]
                    eprintln!("[agora-core] Registry loader_catalog invalid: {e}");
                }
            }
        }
        Self::merge_with_embedded(None)
    }

    /// Replace the process-wide runtime snapshot with a registry-sourced
    /// version. Existing operation snapshots remain valid while subsequent
    /// free-function callers observe the new catalog.
    ///
    /// Returns `Ok(true)` when the override was applied, `Ok(false)` when
    /// the registry has no loader_catalog row, or `Err` on parse/validation
    /// failure.
    ///
    pub fn init_from_registry(conn: &rusqlite::Connection) -> LauncherResult<bool> {
        let catalog = Self::from_registry(conn)?;
        let has_registry_catalog = catalog.is_some();
        Self::replace_active(Some(Self::merge_with_embedded(catalog)?))?;
        Ok(has_registry_catalog)
    }

    /// Atomically replace the active registry override after all coordinated
    /// catalogs have been parsed and validated.
    pub fn replace_active(catalog: Option<Self>) -> LauncherResult<()> {
        let mut active = CATALOG_OVERRIDE
            .write()
            .map_err(|_| LauncherError::Generic {
                code: "ERR_LOADER_CATALOG_LOCK".into(),
                message: "Loader catalog state is unavailable.".into(),
            })?;
        *active = catalog.map(Arc::new);
        Ok(())
    }

    /// Merge one registry entry into a loader's entry list.
    ///
    /// Pinned identity and artifact fields must match exactly for an existing
    /// tuple; any difference fails closed. Enrichment fields (capabilities,
    /// release channel, recommendation rank) may be *added* to a legacy entry,
    /// but once a value exists it must never change. Legacy channel values are
    /// inferred during deserialization, so a refresh cannot silently
    /// reclassify an existing tuple during merge.
    fn merge_entry_into(
        target: &mut Vec<LoaderEntry>,
        entry: LoaderEntry,
        loader: &str,
    ) -> LauncherResult<()> {
        let conflict = || LauncherError::Generic {
            code: "ERR_LOADER_CATALOG_CONFLICT".into(),
            message: format!(
                "Conflicting pinned metadata for {loader} {} {}",
                entry.mc_version, entry.loader_version
            ),
        };
        let Some(existing) = target.iter_mut().find(|existing| {
            existing.mc_version == entry.mc_version
                && existing.loader_version == entry.loader_version
        }) else {
            target.push(entry);
            return Ok(());
        };
        // Pinned identity and artifact fields must match exactly.
        if existing.mc_version != entry.mc_version
            || existing.loader_version != entry.loader_version
            || existing.source_url != entry.source_url
            || existing.sha256 != entry.sha256
            || existing.file_name != entry.file_name
            || existing.file_type != entry.file_type
            || existing.version_json_sha256 != entry.version_json_sha256
            || existing.installer_spec != entry.installer_spec
        {
            return Err(conflict());
        }
        // Enrichment: missing capability/channel/rank may be added; an existing
        // value must never change.
        for (capability, version) in &entry.provided_versions {
            if let Some(previous) = existing.provided_versions.get(capability) {
                if previous != version {
                    return Err(conflict());
                }
            } else {
                existing
                    .provided_versions
                    .insert(capability.clone(), version.clone());
            }
        }
        if existing.release_channel != entry.release_channel {
            return Err(conflict());
        }
        match (existing.recommendation_rank, entry.recommendation_rank) {
            (None, Some(rank)) => existing.recommendation_rank = Some(rank),
            (Some(a), Some(b)) if a != b => return Err(conflict()),
            _ => {}
        }
        Ok(())
    }

    /// Find a pinned loader entry for a `(loader, mc_version, loader_version)` triple.
    pub fn find_entry(
        &self,
        loader: &str,
        mc_version: &str,
        loader_version: &str,
    ) -> Option<&LoaderEntry> {
        self.loaders.get(loader).and_then(|entries| {
            entries
                .iter()
                .find(|e| e.mc_version == mc_version && e.loader_version == loader_version)
        })
    }

    /// Verify that a URL's host is on the modloader domain allowlist.
    pub fn ensure_allowed_domain(&self, raw_url: &str) -> LauncherResult<()> {
        let host = reqwest::Url::parse(raw_url)
            .map_err(|e| LauncherError::Generic {
                code: "ERR_UNTRUSTED_SOURCE".to_string(),
                message: format!("Invalid loader URL: {e}"),
            })?
            .host_str()
            .ok_or(LauncherError::UntrustedSource)?
            .to_string();

        if self.is_allowed_host(&host) {
            Ok(())
        } else {
            Err(LauncherError::UntrustedSource)
        }
    }

    /// Whether a host is on the loader domain allowlist.
    pub fn is_allowed_host(&self, host: &str) -> bool {
        self.domain_allowlist.iter().any(|d| d == host)
    }

    /// List pinned loader entries for a loader + Minecraft version.
    pub fn list_versions(&self, loader: &str, mc_version: &str) -> Vec<&LoaderEntry> {
        self.loaders
            .get(loader)
            .map(|entries| {
                entries
                    .iter()
                    .filter(|e| e.mc_version == mc_version)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Distinct loader names present in the catalog (sorted A→Z).
    pub fn list_loaders(&self) -> Vec<&str> {
        self.loaders.keys().map(|k| k.as_str()).collect()
    }

    /// All stable Minecraft versions (from Mojang's manifest), or only those
    /// supported by a specific loader. Sorted newest-first.
    pub fn list_mc_versions(&self, loader: Option<&str>) -> Vec<String> {
        let all_versions = mc_versions_list();
        match loader {
            None => all_versions.to_vec(),
            Some(l) => {
                let supported: std::collections::HashSet<&str> = self
                    .loaders
                    .get(l)
                    .map(|entries| entries.iter().map(|e| e.mc_version.as_str()).collect())
                    .unwrap_or_default();
                all_versions
                    .iter()
                    .filter(|v| supported.contains(v.as_str()))
                    .cloned()
                    .collect()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Backward-compatible free functions (delegate to global catalog).
//
// These read from the effective catalog (override > embedded) and are the
// default entry-point for all existing callers.  New code should prefer the
// instance methods on `LoaderCatalog` when a pre-resolved catalog is available.
// ---------------------------------------------------------------------------

/// Find a pinned loader entry (delegates to global effective catalog).
pub fn find_entry(loader: &str, mc_version: &str, loader_version: &str) -> Option<LoaderEntry> {
    catalog()
        .find_entry(loader, mc_version, loader_version)
        .cloned()
}

/// Verify URL host is allowed (delegates).
pub fn ensure_allowed_domain(raw_url: &str) -> LauncherResult<()> {
    catalog().ensure_allowed_domain(raw_url)
}

/// Whether a host is on the loader domain allowlist (delegates).
pub fn is_allowed_host(host: &str) -> bool {
    catalog().is_allowed_host(host)
}

/// List pinned loader versions (delegates).
pub fn list_versions(loader: &str, mc_version: &str) -> Vec<LoaderEntry> {
    catalog()
        .list_versions(loader, mc_version)
        .into_iter()
        .cloned()
        .collect()
}

/// Distinct loader names (delegates).
pub fn list_loaders() -> Vec<String> {
    catalog()
        .list_loaders()
        .into_iter()
        .map(str::to_string)
        .collect()
}

/// All stable Minecraft versions (delegates).
pub fn list_mc_versions(loader: Option<&str>) -> Vec<String> {
    catalog().list_mc_versions(loader)
}

/// Convert a `sha256:hex` string to raw lowercase hex.
pub fn strip_sha_prefix(s: &str) -> &str {
    s.strip_prefix("sha256:").unwrap_or(s)
}

/// Return an immutable snapshot of the process-wide effective loader catalog
/// (signed registry override when present, otherwise the embedded fallback).
///
/// Consumers that need to evaluate several tuples against one coherent view
/// (for example loader compatibility checks) should use this snapshot rather
/// than re-reading the free functions, which may observe a concurrent refresh.
pub fn active_catalog() -> Arc<LoaderCatalog> {
    catalog()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allowed_loader_domain() {
        assert!(is_allowed_host("files.minecraftforge.net"));
        assert!(is_allowed_host("maven.fabricmc.net"));
        assert!(is_allowed_host("maven.neoforged.net"));
        assert!(is_allowed_host("maven.quiltmc.org"));
        assert!(is_allowed_host("neoforged.net"));
    }

    #[test]
    fn test_disallowed_localhost() {
        assert!(!is_allowed_host("127.0.0.1"));
    }

    #[test]
    fn test_disallowed_metadata_ip() {
        assert!(!is_allowed_host("169.254.169.254"));
    }

    #[test]
    fn test_disallowed_random_host() {
        assert!(!is_allowed_host("evil.com"));
    }

    #[test]
    fn test_disallowed_file_scheme() {
        let result = ensure_allowed_domain("file:///etc/passwd");
        assert!(result.is_err());
    }

    #[test]
    fn test_manifest_allowlist_nonempty() {
        let m = catalog();
        assert!(!m.domain_allowlist.is_empty());
    }

    #[test]
    fn test_registry_override_can_reload_without_panic() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE loader_catalog (singleton_id INTEGER PRIMARY KEY, catalog_json TEXT NOT NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO loader_catalog (singleton_id, catalog_json) VALUES (1, ?1)",
            [LOADER_MANIFESTS],
        )
        .unwrap();

        assert!(LoaderCatalog::init_from_registry(&conn).unwrap());
        assert!(LoaderCatalog::init_from_registry(&conn).unwrap());
        assert!(!catalog().domain_allowlist.is_empty());
    }

    #[test]
    fn stale_registry_catalog_cannot_remove_embedded_loader_pins() {
        let embedded = LoaderCatalog::embedded();
        let registry_entry = embedded
            .find_entry("fabric", "1.21.1", "0.18.6")
            .unwrap()
            .clone();
        let registry = LoaderCatalog {
            domain_allowlist: embedded.domain_allowlist.clone(),
            loaders: std::collections::BTreeMap::from([("fabric".into(), vec![registry_entry])]),
        };

        let merged = LoaderCatalog::merge_with_embedded(Some(registry)).unwrap();
        assert!(merged.find_entry("fabric", "1.21.1", "0.18.4").is_some());
    }

    #[test]
    fn conflicting_registry_loader_pin_fails_closed() {
        let embedded = LoaderCatalog::embedded();
        let mut conflicting = embedded
            .find_entry("fabric", "1.21.1", "0.18.6")
            .unwrap()
            .clone();
        conflicting.sha256 = "0".repeat(64);
        let registry = LoaderCatalog {
            domain_allowlist: embedded.domain_allowlist.clone(),
            loaders: std::collections::BTreeMap::from([("fabric".into(), vec![conflicting])]),
        };

        let error = LoaderCatalog::merge_with_embedded(Some(registry)).unwrap_err();
        assert!(error.to_string().contains("Conflicting pinned metadata"));
    }

    #[test]
    fn test_ensure_allowed_domain_valid() {
        let result = ensure_allowed_domain(
            "https://maven.fabricmc.net/v2/versions/loader/1.21/0.19.0/profile/json",
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_ensure_allowed_domain_invalid() {
        let result = ensure_allowed_domain("https://evil.example.com/loader.jar");
        assert!(result.is_err());
    }

    #[test]
    fn test_ensure_allowed_domain_invalid_url() {
        let result = ensure_allowed_domain("not-a-valid-url");
        assert!(result.is_err());
    }

    #[test]
    fn test_strip_sha_prefix() {
        assert_eq!(strip_sha_prefix("sha256:abc123"), "abc123");
        assert_eq!(strip_sha_prefix("abc123"), "abc123");
    }

    #[test]
    fn test_list_mc_versions_includes_legacy() {
        let versions = list_mc_versions(None);
        assert!(
            versions.len() > 50,
            "Expected 50+ versions, got {}",
            versions.len()
        );
        assert!(
            versions.contains(&"1.12.2".to_string()),
            "1.12.2 should be in the list"
        );
        assert!(
            versions.contains(&"1.7.10".to_string()),
            "1.7.10 should be in the list"
        );
    }

    #[test]
    fn test_list_mc_versions_filtered_by_loader() {
        let all = list_mc_versions(None);
        let fabric = list_mc_versions(Some("fabric"));
        assert!(
            fabric.len() < all.len(),
            "Fabric should have fewer versions than the full list"
        );
        assert!(
            !fabric.contains(&"1.7.10".to_string()),
            "Fabric should not support 1.7.10"
        );
    }

    // -----------------------------------------------------------------------
    // New field: LoaderEntry serde defaults
    // -----------------------------------------------------------------------

    #[test]
    fn loader_entry_version_json_sha256_defaults_none() {
        let json = r#"{
            "mc_version": "1.21",
            "loader_version": "0.19.0",
            "source_url": "https://example.com/profile.json",
            "sha256": "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
            "file_name": "test.json",
            "file_type": "profile_json"
        }"#;
        let entry: LoaderEntry = serde_json::from_str(json).unwrap();
        assert!(entry.version_json_sha256.is_none());
        assert!(entry.installer_spec.is_none());
    }

    #[test]
    fn loader_entry_parses_new_fields() {
        let json = r#"{
            "mc_version": "1.20.1",
            "loader_version": "47.4.21",
            "source_url": "https://example.com/forge-installer.jar",
            "sha256": "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
            "file_name": "forge-1.20.1-47.4.21-installer.jar",
            "file_type": "installer_jar",
            "version_json_sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "installer_spec": 1
        }"#;
        let entry: LoaderEntry = serde_json::from_str(json).unwrap();
        assert_eq!(
            entry.version_json_sha256.as_deref(),
            Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
        );
        assert_eq!(entry.installer_spec, Some(1));
    }

    // -----------------------------------------------------------------------
    // New fields: capabilities / release channel / recommendation rank
    // -----------------------------------------------------------------------

    #[test]
    fn loader_entry_new_fields_default_in_legacy_catalog() {
        let json = r#"{
            "mc_version": "1.21",
            "loader_version": "0.18.6",
            "source_url": "https://example.com/profile.json",
            "sha256": "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
            "file_name": "fabric-loader-0.18.6-1.21.json",
            "file_type": "profile_json"
        }"#;
        let entry: LoaderEntry = serde_json::from_str(json).unwrap();
        assert!(entry.provided_versions.is_empty());
        assert_eq!(entry.release_channel, LoaderReleaseChannel::Stable);
        assert_eq!(entry.recommendation_rank, None);
    }

    #[test]
    fn legacy_prerelease_version_is_not_treated_as_stable() {
        let json = r#"{
            "mc_version": "1.21",
            "loader_version": "0.30.0-beta.4",
            "source_url": "https://example.com/profile.json",
            "sha256": "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
            "file_name": "quilt-loader-0.30.0-beta.4-1.21.json",
            "file_type": "profile_json"
        }"#;
        let entry: LoaderEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.release_channel, LoaderReleaseChannel::Prerelease);
    }

    #[test]
    fn explicit_release_channel_overrides_legacy_version_inference() {
        let json = r#"{
            "mc_version": "1.21",
            "loader_version": "0.30.0-beta.4",
            "source_url": "https://example.com/profile.json",
            "sha256": "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
            "file_name": "quilt-loader-0.30.0-beta.4-1.21.json",
            "file_type": "profile_json",
            "release_channel": "stable"
        }"#;
        let entry: LoaderEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.release_channel, LoaderReleaseChannel::Stable);
    }

    #[test]
    fn loader_entry_parses_capabilities_channel_and_rank() {
        let json = r#"{
            "mc_version": "1.21",
            "loader_version": "0.19.0",
            "source_url": "https://example.com/profile.json",
            "sha256": "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
            "file_name": "fabric-loader-0.19.0-1.21.json",
            "file_type": "profile_json",
            "provided_versions": {"fabricloader": "0.19.0", "javafml": "3.0.0"},
            "release_channel": "prerelease",
            "recommendation_rank": 2
        }"#;
        let entry: LoaderEntry = serde_json::from_str(json).unwrap();
        assert_eq!(
            entry
                .provided_versions
                .get("fabricloader")
                .map(String::as_str),
            Some("0.19.0")
        );
        assert_eq!(entry.release_channel, LoaderReleaseChannel::Prerelease);
        assert_eq!(entry.recommendation_rank, Some(2));
    }

    fn test_entry(loader_version: &str) -> LoaderEntry {
        LoaderEntry {
            mc_version: "1.21".into(),
            loader_version: loader_version.into(),
            source_url: "https://example.com/pin".into(),
            sha256: "0".repeat(64),
            file_name: format!("loader-{loader_version}.jar"),
            file_type: "installer_jar".into(),
            version_json_sha256: None,
            installer_spec: None,
            provided_versions: BTreeMap::new(),
            release_channel: LoaderReleaseChannel::Stable,
            recommendation_rank: None,
        }
    }

    #[test]
    fn legacy_catalog_distribution_identity_fabric_and_quilt() {
        let fabric = test_entry("0.18.6");
        assert_eq!(
            fabric.capability_version("fabric", "fabricloader"),
            Some("0.18.6")
        );
        let quilt = test_entry("0.27.1");
        assert_eq!(
            quilt.capability_version("quilt", "quilt_loader"),
            Some("0.27.1")
        );
        let forge = test_entry("51.0.0");
        assert_eq!(forge.capability_version("forge", "forge"), Some("51.0.0"));
        let neoforge = test_entry("21.1.181");
        assert_eq!(
            neoforge.capability_version("neoforge", "neoforge"),
            Some("21.1.181")
        );
    }

    #[test]
    fn legacy_catalog_never_synthesizes_language_loader_capability() {
        let forge = test_entry("51.0.0");
        assert_eq!(forge.capability_version("forge", "javafml"), None);
        assert_eq!(forge.capability_version("forge", "lowcodefml"), None);
        let neoforge = test_entry("21.1.181");
        assert_eq!(neoforge.capability_version("neoforge", "javafml"), None);
        // Unrelated capabilities are never fabricated either.
        assert_eq!(
            neoforge.capability_version("neoforge", "quilt_loader"),
            None
        );
    }

    #[test]
    fn capability_names_normalize_lowercase_and_explicit_wins() {
        let mut entry = test_entry("0.18.6");
        entry
            .provided_versions
            .insert("fabricloader".into(), "0.19.0".into());
        entry
            .provided_versions
            .insert("javafml".into(), "3.0.0".into());
        // Explicit entries win over the legacy identity.
        assert_eq!(
            entry.capability_version("fabric", "FabricLoader"),
            Some("0.19.0")
        );
        assert_eq!(
            entry.capability_version("fabric", "fabricloader"),
            Some("0.19.0")
        );
        // Explicit language capability is honored when present.
        assert_eq!(entry.capability_version("fabric", "javafml"), Some("3.0.0"));
    }

    #[test]
    fn capabilities_accessor_merges_identity_and_explicit_provided() {
        let mut entry = test_entry("0.18.6");
        entry
            .provided_versions
            .insert("javafml".into(), "2.0.0".into());
        let caps = entry.capabilities("fabric");
        assert_eq!(caps.distribution_id, "fabric");
        assert_eq!(caps.distribution_version, "0.18.6");
        assert_eq!(
            caps.provided_versions
                .get("fabricloader")
                .map(String::as_str),
            Some("0.18.6")
        );
        assert_eq!(
            caps.provided_versions.get("javafml").map(String::as_str),
            Some("2.0.0")
        );

        // An explicit distribution version overrides the legacy identity.
        let mut neoforge = test_entry("21.1.181");
        neoforge
            .provided_versions
            .insert("neoforge".into(), "21.1.182".into());
        let caps = neoforge.capabilities("neoforge");
        assert_eq!(caps.distribution_version, "21.1.182");
    }

    #[test]
    fn merge_accepts_enrichment_of_legacy_entry() {
        // A registry catalog carrying the same pinned tuple as the embedded
        // catalog plus enrichment fields must merge without conflict, and the
        // merged entry must carry the enrichment.
        let embedded_entry = LoaderCatalog::embedded()
            .find_entry("fabric", "1.21", "0.18.6")
            .expect("embedded fabric 1.21 0.18.6 pin exists")
            .clone();
        let mut enriched = embedded_entry.clone();
        enriched
            .provided_versions
            .insert("fabricloader".into(), "0.18.6".into());
        let registry = LoaderCatalog {
            domain_allowlist: vec!["example.com".into()],
            loaders: BTreeMap::from([("fabric".into(), vec![enriched])]),
        };

        let merged = LoaderCatalog::merge_with_embedded(Some(registry)).unwrap();
        let entry = merged.find_entry("fabric", "1.21", "0.18.6").unwrap();
        assert_eq!(
            entry
                .provided_versions
                .get("fabricloader")
                .map(String::as_str),
            Some("0.18.6")
        );
        assert_eq!(entry.release_channel, LoaderReleaseChannel::Stable);
    }

    #[test]
    fn merge_rejects_conflicting_capability_metadata() {
        // Once a capability exists on a tuple, changing it fails closed even
        // when every pinned artifact field still matches.
        let mut existing = test_entry("0.18.6");
        existing
            .provided_versions
            .insert("fabricloader".into(), "0.18.6".into());
        let mut incoming = test_entry("0.18.6");
        incoming
            .provided_versions
            .insert("fabricloader".into(), "0.99.0".into());

        let error =
            LoaderCatalog::merge_entry_into(&mut vec![existing], incoming, "fabric").unwrap_err();
        assert!(error.to_string().contains("Conflicting pinned metadata"));
    }

    #[test]
    fn merge_rejects_conflicting_rank_metadata() {
        let mut existing = test_entry("0.18.6");
        existing.recommendation_rank = Some(2);
        let mut incoming = test_entry("0.18.6");
        incoming.recommendation_rank = Some(9);

        let error =
            LoaderCatalog::merge_entry_into(&mut vec![existing], incoming, "fabric").unwrap_err();
        assert!(error.to_string().contains("Conflicting pinned metadata"));
    }

    #[test]
    fn merge_entry_adds_missing_enrichment_fields() {
        let mut target = vec![test_entry("0.18.6")];
        let mut incoming = test_entry("0.18.6");
        incoming
            .provided_versions
            .insert("fabricloader".into(), "0.18.6".into());
        incoming.recommendation_rank = Some(1);

        LoaderCatalog::merge_entry_into(&mut target, incoming, "fabric").unwrap();
        assert_eq!(target.len(), 1);
        assert_eq!(
            target[0]
                .provided_versions
                .get("fabricloader")
                .map(String::as_str),
            Some("0.18.6")
        );
        assert_eq!(target[0].recommendation_rank, Some(1));
    }

    #[test]
    fn embedded_catalog_legacy_identity_holds_for_every_entry() {
        // The compile-time embedded catalog must parse with all new fields
        // defaulted, and every pinned entry must still expose its legacy
        // distribution identity.
        let catalog = LoaderCatalog::embedded();
        assert!(!catalog.loaders.is_empty());
        for (family, entries) in &catalog.loaders {
            let Some(own) = DISTRIBUTION_CAPABILITY
                .iter()
                .find(|(f, _)| f == family)
                .map(|(_, own)| *own)
            else {
                continue;
            };
            for entry in entries {
                assert_eq!(
                    entry.capability_version(family, own),
                    Some(entry.loader_version.as_str()),
                    "{} {} {} lost its legacy identity",
                    family,
                    entry.mc_version,
                    entry.loader_version
                );
            }
        }
    }
}
