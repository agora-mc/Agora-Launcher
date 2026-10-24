//! Dependency reads and the enable-with-dependencies action for one instance.
//!
//! The desktop app and the CLI both call these, so "what needs what" is
//! answered by one implementation. The JAR-metadata cache and the auto-enable
//! logic used to live in the desktop adapter; they moved here unchanged except
//! that the loader is now read with `?` rather than defaulting to an empty
//! string when the manifest cannot be read.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::crash_service::CrashService;
use crate::ctx::Ctx;
use crate::dependency_ops::{
    self, DependencyEdge, DisablePlan, OrphanedDependency, PresenceExplanation,
};
use crate::error::{LauncherError, LauncherResult};
use crate::models::{InstalledMod, InstanceManifest};

/// One dependency that was disabled and could not be re-enabled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DependencyEnableFailure {
    pub filename: String,
    pub message: String,
}

/// What enabling a mod with its disabled required dependencies did.
///
/// Best-effort by design: a dependency that fails to re-enable is reported
/// here and does not stop the target from being enabled.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AutoEnableOutcome {
    /// Filenames of the dependencies that were disabled and are now enabled.
    pub enabled_dependencies: Vec<String>,
    /// Dependencies that were disabled and could not be re-enabled.
    pub failed_dependencies: Vec<DependencyEnableFailure>,
}

/// Dependency queries and actions for one instance.
pub struct DependencyService {
    ctx: Ctx,
}

impl DependencyService {
    pub fn new(ctx: Ctx) -> Self {
        Self { ctx }
    }

    /// Every installed dependency edge where both ends are usable right now.
    pub fn graph(&self, instance_id: &str) -> LauncherResult<Vec<DependencyEdge>> {
        let (manifest, instance_dir) = self.load_refreshed(instance_id)?;
        // Only entries that can satisfy a dependency are nodes: a disabled or
        // missing mod neither needs nor supplies anything right now.
        let on_disk = dependency_ops::mod_jars_on_disk(&instance_dir);
        let usable: Vec<InstalledMod> = manifest
            .mods
            .iter()
            .filter(|m| dependency_ops::satisfies_dependency(m, &on_disk))
            .cloned()
            .collect();
        Ok(dependency_ops::build_dependency_graph(&usable))
    }

    /// Dependency-installed mods that nothing reachable from a user's mod needs.
    pub fn orphans(&self, instance_id: &str) -> LauncherResult<Vec<OrphanedDependency>> {
        let (manifest, _) = self.load_refreshed(instance_id)?;
        Ok(dependency_ops::find_orphaned_dependencies(&manifest.mods))
    }

    /// Why one installed file is present. `None` when no installed mod has
    /// that filename.
    pub fn presence(
        &self,
        instance_id: &str,
        filename: &str,
    ) -> LauncherResult<Option<PresenceExplanation>> {
        let (manifest, instance_dir) = self.load_refreshed(instance_id)?;
        let on_disk = dependency_ops::mod_jars_on_disk(&instance_dir);
        Ok(dependency_ops::explain_presence(
            &manifest.mods,
            filename,
            &on_disk,
        ))
    }

    /// Which installed mods would be affected by disabling this one. Read-only.
    pub fn disable_plan(&self, instance_id: &str, filename: &str) -> LauncherResult<DisablePlan> {
        let (manifest, _) = self.load_refreshed(instance_id)?;
        let target = manifest
            .mods
            .iter()
            .find(|m| m.filename == filename)
            .ok_or_else(|| mod_not_found(filename))?;
        Ok(dependency_ops::build_disable_plan(&manifest.mods, target))
    }

    /// Enable a mod, and re-enable any of its required dependencies that are
    /// currently disabled. Fails only when the target itself cannot be enabled.
    pub fn enable_with_auto_deps(
        &self,
        instance_id: &str,
        filename: &str,
    ) -> LauncherResult<AutoEnableOutcome> {
        let (manifest, instance_dir) = self.load_refreshed(instance_id)?;
        let target = manifest
            .mods
            .iter()
            .find(|m| m.filename == filename)
            .ok_or_else(|| mod_not_found(filename))?;
        let crash = CrashService::new(self.ctx.clone());
        let mods_dir = instance_dir.join("mods");

        let mut outcome = AutoEnableOutcome::default();

        // Resolve the target mod's required deps from jar metadata.
        let depends_on = match &target.mod_jar_id {
            Some(_) => target.depends_on.clone(),
            None => Vec::new(),
        };

        // For each required dep, find the corresponding installed mod and check
        // if it's disabled (`.disabled` file exists). If so, enable it.
        for dep_jar_id in &depends_on {
            let dep_lower = dep_jar_id.to_lowercase();

            // Find the physical JAR whose primary or dynamically provided ID
            // matches this dependency.
            let dep_mod = manifest.mods.iter().find(|m| {
                m.mod_jar_id
                    .iter()
                    .chain(m.provided_mod_ids.iter())
                    .any(|jid| jid.to_lowercase() == dep_lower)
            });

            // Missing entirely: skipped, since it cannot be auto-installed here.
            let Some(dep_mod) = dep_mod else {
                continue;
            };

            // Already enabled when no `.disabled` copy is on disk.
            let disabled_path = mods_dir.join(format!("{}.disabled", dep_mod.filename));
            if !disabled_path.exists() {
                continue;
            }

            // Best-effort enable: a failure is reported, not fatal.
            match crash.enable_mod(instance_id, &dep_mod.filename) {
                Ok(()) => outcome.enabled_dependencies.push(dep_mod.filename.clone()),
                Err(error) => outcome.failed_dependencies.push(DependencyEnableFailure {
                    filename: dep_mod.filename.clone(),
                    message: error.to_string(),
                }),
            }
        }

        // Now enable the target mod itself. This one is not best-effort.
        if let Err(error) = crash.enable_mod(instance_id, filename) {
            return Err(LauncherError::Generic {
                code: "ERR_ENABLE_FAILED".to_string(),
                message: format!("Failed to enable '{filename}': {error}"),
            });
        }

        Ok(outcome)
    }

    /// Read the manifest and refresh its dependency fields from the JARs on
    /// disk. Returns the instance directory alongside, since every query needs it.
    fn load_refreshed(&self, instance_id: &str) -> LauncherResult<(InstanceManifest, PathBuf)> {
        let sanitized = crate::paths::sanitize_id(instance_id);
        crate::app_paths::validate_path_component(&sanitized)?;
        let instance_dir = self.ctx.paths.instance_dir(&sanitized)?;
        let manifest_path = self.ctx.paths.instance_manifest(&sanitized)?;
        let mut manifest = crate::helpers::read_manifest(&manifest_path)?;
        refresh_installed_jar_metadata(&instance_dir, &mut manifest.mods)?;
        Ok((manifest, instance_dir))
    }
}

fn mod_not_found(filename: &str) -> LauncherError {
    LauncherError::Generic {
        code: "ERR_MOD_NOT_FOUND".to_string(),
        message: format!("Mod '{filename}' not found in instance manifest."),
    }
}

/// Refresh dependency identity metadata from the physical JARs before a
/// user-initiated install/disable/removal plan, or a dependency-graph read.
///
/// `provided_mod_ids` is persisted as a cache for new installs, but manifests
/// created by older Agora versions do not contain it. Correctness therefore
/// comes from re-reading authoritative local JAR metadata rather than requiring
/// a migration or hardcoded alias table.
///
/// **Parsing is cached on disk per instance.** Opening and reading the metadata
/// file out of every jar is the expensive part, and this used to happen on every
/// single call — install plans, disable plans, removal plans and the dependency
/// graph each re-parsed the whole mods folder and then threw the result away. On
/// a 130-mod instance that is 130 ZIP reads to answer "what needs what", which
/// made the UI visibly wait. Now each jar is parsed once and re-parsed only when
/// its size or mtime changes, so the warm path is a stat per file.
pub fn refresh_installed_jar_metadata(
    instance_dir: &Path,
    installed: &mut [InstalledMod],
) -> LauncherResult<()> {
    let mods_dir = instance_dir.join("mods");
    let manifest_path = instance_dir.join("instance_manifest.json");
    // The loader decides which metadata file is authoritative, so it must be
    // known. Failing here is correct: parsing with an unknown loader would
    // cache a wrong answer.
    let loader = crate::helpers::read_manifest(&manifest_path)?.loader;

    let cache_path = instance_dir.join(JAR_META_CACHE_FILE);
    let mut cache: JarMetaCache = std::fs::read_to_string(&cache_path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .filter(|c: &JarMetaCache| c.version == JAR_META_CACHE_VERSION)
        .unwrap_or_default();
    cache.version = JAR_META_CACHE_VERSION;
    let mut dirty = false;
    let mut seen: HashSet<String> = HashSet::new();

    for installed_mod in installed {
        if installed_mod.content_type != "mod" {
            continue;
        }

        let active_path = mods_dir.join(&installed_mod.filename);
        let disabled_path = mods_dir.join(format!("{}.disabled", installed_mod.filename));
        let jar_path = if active_path.is_file() {
            active_path
        } else if disabled_path.is_file() {
            disabled_path
        } else {
            continue;
        };
        seen.insert(installed_mod.filename.clone());

        let stamp = file_stamp(&jar_path);
        // Warm path: the jar is byte-identical in size and timestamp to what we
        // parsed last time, and for the same loader.
        if let (Some((size, mtime_ms)), Some(hit)) =
            (stamp, cache.entries.get(&installed_mod.filename))
        {
            if cache_entry_is_fresh(hit, size, mtime_ms, &loader) {
                installed_mod.java_packages = hit.java_packages.clone();
                installed_mod.mod_jar_id = hit.mod_jar_id.clone();
                installed_mod.depends_on = hit.depends_on.clone();
                installed_mod.optional_deps = hit.optional_deps.clone();
                installed_mod.incompatible_deps = hit.incompatible_deps.clone();
                installed_mod.provided_mod_ids = hit.provided_mod_ids.clone();
                continue;
            }
        }

        let parsed = crate::jar_metadata::parse_jar_metadata_for_loader(&jar_path, &loader);
        // A valid mod metadata file has a primary ID. If parsing failed or the
        // file is not a recognized mod JAR, retain the manifest's cached data.
        if parsed.mod_jar_id.is_none() {
            continue;
        }

        installed_mod.java_packages = parsed.java_packages;
        installed_mod.mod_jar_id = parsed.mod_jar_id;
        installed_mod.depends_on = parsed.depends_on;
        installed_mod.optional_deps = parsed.optional_deps;
        installed_mod.incompatible_deps = parsed.incompatible_deps;
        installed_mod.provided_mod_ids = parsed
            .provided_mods
            .into_iter()
            .map(|provided| provided.mod_id)
            .collect();

        if let Some((size, mtime_ms)) = stamp {
            cache.entries.insert(
                installed_mod.filename.clone(),
                JarMetaCacheEntry {
                    size,
                    mtime_ms,
                    loader: loader.clone(),
                    mod_jar_id: installed_mod.mod_jar_id.clone(),
                    java_packages: installed_mod.java_packages.clone(),
                    depends_on: installed_mod.depends_on.clone(),
                    optional_deps: installed_mod.optional_deps.clone(),
                    incompatible_deps: installed_mod.incompatible_deps.clone(),
                    provided_mod_ids: installed_mod.provided_mod_ids.clone(),
                },
            );
            dirty = true;
        }
    }

    // Drop entries for jars that are no longer installed, so an instance that
    // churns through mods does not grow a cache forever.
    let before = cache.entries.len();
    cache.entries.retain(|filename, _| seen.contains(filename));
    if cache.entries.len() != before {
        dirty = true;
    }

    if dirty {
        // Best-effort: a cache that cannot be written must never fail the read
        // it was speeding up.
        if let Ok(text) = serde_json::to_string(&cache) {
            let _ = std::fs::write(&cache_path, text);
        }
    }

    Ok(())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct JarMetaCacheEntry {
    size: u64,
    mtime_ms: u64,
    loader: String,
    mod_jar_id: Option<String>,
    #[serde(default)]
    java_packages: Vec<String>,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    optional_deps: Vec<String>,
    #[serde(default)]
    incompatible_deps: Vec<String>,
    #[serde(default)]
    provided_mod_ids: Vec<String>,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct JarMetaCache {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    entries: std::collections::HashMap<String, JarMetaCacheEntry>,
}

const JAR_META_CACHE_VERSION: u32 = 1;
const JAR_META_CACHE_FILE: &str = "dependency_cache.json";

/// Is a cache entry still good for this jar?
///
/// Extracted so the decision is testable without a Tauri AppHandle: the risk in
/// a cache is not the plumbing, it is answering "unchanged" when something did
/// change. Any of size, mtime or the loader differing means re-parse.
fn cache_entry_is_fresh(entry: &JarMetaCacheEntry, size: u64, mtime_ms: u64, loader: &str) -> bool {
    entry.size == size && entry.mtime_ms == mtime_ms && entry.loader == loader
}

fn file_stamp(path: &Path) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime_ms = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis() as u64;
    Some((meta.len(), mtime_ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(size: u64, mtime_ms: u64, loader: &str) -> JarMetaCacheEntry {
        JarMetaCacheEntry {
            size,
            mtime_ms,
            loader: loader.to_string(),
            mod_jar_id: Some("x".into()),
            java_packages: vec![],
            depends_on: vec!["corelib".into()],
            optional_deps: vec![],
            incompatible_deps: vec![],
            provided_mod_ids: vec![],
        }
    }

    #[test]
    fn unchanged_jar_reuses_the_cached_parse() {
        assert!(cache_entry_is_fresh(
            &entry(100, 5, "fabric"),
            100,
            5,
            "fabric"
        ));
    }

    #[test]
    fn any_change_forces_a_reparse() {
        let e = entry(100, 5, "fabric");
        assert!(!cache_entry_is_fresh(&e, 101, 5, "fabric"), "size changed");
        assert!(!cache_entry_is_fresh(&e, 100, 6, "fabric"), "mtime changed");
        // The loader decides which metadata file is authoritative, so the same
        // jar parsed under a different loader is a different answer.
        assert!(!cache_entry_is_fresh(&e, 100, 5, "forge"), "loader changed");
    }

    #[test]
    fn cache_survives_a_round_trip_through_json() {
        let mut cache = JarMetaCache {
            version: JAR_META_CACHE_VERSION,
            entries: Default::default(),
        };
        cache.entries.insert("a.jar".into(), entry(7, 8, "fabric"));
        let text = serde_json::to_string(&cache).unwrap();
        let back: JarMetaCache = serde_json::from_str(&text).unwrap();
        assert_eq!(back.version, JAR_META_CACHE_VERSION);
        assert_eq!(
            back.entries["a.jar"].depends_on,
            vec!["corelib".to_string()]
        );
        assert!(cache_entry_is_fresh(&back.entries["a.jar"], 7, 8, "fabric"));
    }

    #[test]
    fn a_cache_from_a_future_version_is_discarded_not_trusted() {
        let text = r#"{"version":999,"entries":{"a.jar":{"size":1,"mtime_ms":1,"loader":"fabric","mod_jar_id":"x"}}}"#;
        let parsed: Option<JarMetaCache> = serde_json::from_str(text)
            .ok()
            .filter(|c: &JarMetaCache| c.version == JAR_META_CACHE_VERSION);
        assert!(parsed.is_none(), "a newer on-disk format must be ignored");
    }
}
