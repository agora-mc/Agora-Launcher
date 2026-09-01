//! Desktop shim for dependency resolution.
//!
//! Re-exports all types and functions from `agora_core::dependency_ops`. The
//! JAR metadata parser has been deduplicated to `agora_core::jar_metadata`;
//! callers use `agora_core::jar_metadata::parse_jar_metadata` directly.

use agora_core::dependency_ops::JarDeps;

// ---------------------------------------------------------------------------
// 1. Re-export all public types from core
// ---------------------------------------------------------------------------

pub use agora_core::dependency_ops::{
    AliasMap, DepCandidate, DepConflict, DepSource, DependencyEdge, DependentInfo, DisablePlan,
    IncompatibilityDecl, IncompatibilitySource, InstallPlan, RemovalPlan, Requirement,
    ResolvedInstallDeps,
};

// ---------------------------------------------------------------------------
// 2. Re-export core functions that callers reference
// ---------------------------------------------------------------------------

pub use agora_core::dependency_ops::{
    build_dependency_graph, build_dependency_graph_with_aliases, build_disable_plan_with_aliases,
    build_install_plan_with_aliases, build_removal_plan_with_aliases, detect_source_disagreement,
    find_dependents, find_dependents_with_aliases, resolve_install_deps,
    resolve_install_deps_with_aliases,
};

// ---------------------------------------------------------------------------
// 3. Desktop-specific wrappers preserving original signatures
// ---------------------------------------------------------------------------

/// Build a disable plan for a target mod.
///
/// Preserves the original signature used by `commands::get_disable_plan`.
pub fn build_disable_plan(
    installed: &[crate::models::InstalledMod],
    target: &crate::models::InstalledMod,
) -> DisablePlan {
    agora_core::dependency_ops::build_disable_plan(installed, target)
}

/// Build a removal plan for a target mod.
///
/// Preserves the original signature used by `commands::get_removal_plan`.
pub fn build_removal_plan(
    installed: &[crate::models::InstalledMod],
    target: &crate::models::InstalledMod,
) -> RemovalPlan {
    agora_core::dependency_ops::build_removal_plan(installed, target)
}

/// Build an install plan for a target mod.
///
/// Delegates directly to `agora_core::dependency_ops::build_install_plan`.
/// Callers pass `agora_core::dependency_ops::JarDeps` (from
/// `agora_core::jar_metadata::parse_jar_metadata`) directly.
pub fn build_install_plan(
    target_manifest_deps: Option<crate::registry::ManifestDeps>,
    target_jar_deps: &JarDeps,
    installed: &[crate::models::InstalledMod],
) -> InstallPlan {
    agora_core::dependency_ops::build_install_plan(target_manifest_deps, target_jar_deps, installed)
}

/// One jar's parsed dependency identity, plus what it was parsed from.
///
/// `size` + `mtime_ms` is the freshness key. A jar is content-addressed by its
/// sha256 in the manifest, but hashing 130 files costs about as much as parsing
/// them; a stat call costs nothing, and a mod jar that changes on disk without
/// changing size *or* timestamp is not a case worth paying for on every read.
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

fn file_stamp(path: &std::path::Path) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime_ms = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis() as u64;
    Some((meta.len(), mtime_ms))
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
pub fn refresh_installed_jar_metadata<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    instance_id: &str,
    installed: &mut [crate::models::InstalledMod],
) -> crate::error::LauncherResult<()> {
    let instance_dir = crate::paths::instance_dir(app, instance_id)
        .map_err(|_| crate::error::LauncherError::InstanceCreateFailed)?;
    let mods_dir = instance_dir.join("mods");
    let manifest_path = instance_dir.join("instance_manifest.json");
    let loader = agora_core::helpers::read_manifest(&manifest_path)
        .ok()
        .map(|manifest| manifest.loader)
        .unwrap_or_default();

    let cache_path = instance_dir.join(JAR_META_CACHE_FILE);
    let mut cache: JarMetaCache = std::fs::read_to_string(&cache_path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .filter(|c: &JarMetaCache| c.version == JAR_META_CACHE_VERSION)
        .unwrap_or_default();
    cache.version = JAR_META_CACHE_VERSION;
    let mut dirty = false;
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

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

        let parsed = agora_core::jar_metadata::parse_jar_metadata_for_loader(&jar_path, &loader);
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
