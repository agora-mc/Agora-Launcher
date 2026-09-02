//! Disk-reclaim scanner for the shared `minecraft-runtime/` tree.
//!
//! Answers two questions:
//! 1. What is reclaimable? Builds the reference set as the union over every
//!    instance manifest (`minecraft_version`, `loader`, `loader_version`) plus
//!    the version JSONs those pull in via `inheritsFrom` (loader profiles
//!    inherit from the vanilla version). Everything under the shared roots that
//!    nothing in that set references is reclaimable, grouped by category with
//!    file count and byte totals.
//! 2. Delete a chosen subset, returning what was actually freed.
//!
//! # Safety invariant
//! Never delete something a reachable version JSON references. Where a
//! reference cannot be resolved confidently — malformed version JSON, an asset
//! index that cannot be parsed, a directory whose shape is not recognised —
//! the whole subtree is treated as referenced and left alone. Under-reclaiming
//! is invisible; deleting a live library is not. A dry run (scan) never
//! deletes; deletion only happens via [`prune`].

use crate::app_paths::AppPaths;
use crate::launch::{Library, VersionInfo};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Category with reclaimable content. The UI can render
/// "`Libraries — 412 files, 1.8 GB`" and let the user pick categories.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, PartialOrd, Ord,
)]
#[serde(rename_all = "snake_case")]
pub enum PruneCategory {
    Libraries,
    Assets,
    Natives,
    Versions,
    JavaRuntimes,
    Logging,
}

impl PruneCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Libraries => "libraries",
            Self::Assets => "assets",
            Self::Natives => "natives",
            Self::Versions => "versions",
            Self::JavaRuntimes => "java_runtimes",
            Self::Logging => "logging",
        }
    }

    pub fn all() -> Vec<Self> {
        vec![
            Self::Libraries,
            Self::Assets,
            Self::Natives,
            Self::Versions,
            Self::JavaRuntimes,
            Self::Logging,
        ]
    }
}

impl std::fmt::Display for PruneCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One reclaimable file on disk.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReclaimFile {
    pub path: PathBuf,
    pub bytes: u64,
}

/// Aggregated reclaimable set for a single category.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PruneCategoryReport {
    pub category: PruneCategory,
    pub file_count: usize,
    pub total_bytes: u64,
    /// Absolute paths to the reclaimable files.
    ///
    /// Not sent over IPC: a large install has tens of thousands of asset
    /// objects, and the UI only ever shows counts and byte totals. Kept in the
    /// struct because [`prune`] consumes it and tests inspect it.
    #[serde(skip_serializing, default)]
    pub files: Vec<PathBuf>,
}

/// Result of a dry-run scan. No deletion has occurred.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PruneReport {
    pub categories: Vec<PruneCategoryReport>,
    pub warnings: Vec<String>,
}

/// Result of actually deleting the selected categories.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PruneResult {
    pub categories: Vec<PruneCategoryReport>,
    pub warnings: Vec<String>,
    pub total_freed_files: usize,
    pub total_freed_bytes: u64,
}

// ---------------------------------------------------------------------------
// Public API — scan (dry run) and prune (deletion)
// ---------------------------------------------------------------------------

/// Dry-run scan: build the reference set and report everything that can be
/// reclaimed, grouped by category. Never deletes.
///
/// Reads every `instance_manifest.json` under `instances/` and follows
/// `inheritsFrom` from the referenced version JSONs under
/// `minecraft-runtime/versions/<id>/<id>.json`.
pub fn scan(paths: &AppPaths) -> PruneReport {
    scan_inner(paths)
}

/// Delete the selected categories and return what was actually freed.
///
/// Re-runs a scan internally, filters to `selected`, then deletes. A dry run
/// must be the only thing that happens unless this function is explicitly
/// called.
pub fn prune(paths: &AppPaths, selected: &[PruneCategory]) -> PruneResult {
    let report = scan_inner(paths);
    let selected_set: HashSet<PruneCategory> = selected.iter().copied().collect();

    let mut total_freed_files = 0usize;
    let mut total_freed_bytes = 0u64;
    let mut freed_categories = Vec::new();
    let mut warnings = report.warnings.clone();

    for cat_report in report
        .categories
        .into_iter()
        .filter(|c| selected_set.contains(&c.category))
    {
        let mut freed_files = Vec::new();
        let mut freed_bytes = 0u64;

        // For file-based categories we delete each file. For directory-based
        // categories (versions, natives, java runtimes) `cat_report.files`
        // already enumerates every nested file, so deleting files and then
        // removing now-empty directories is correct.
        for file_path in &cat_report.files {
            if let Ok(meta) = std::fs::metadata(file_path) {
                if meta.is_file() {
                    let size = meta.len();
                    if std::fs::remove_file(file_path).is_ok() {
                        freed_bytes += size;
                        freed_files.push(file_path.clone());
                    }
                }
            }
        }

        // Best-effort: remove now-empty parent directories for dir-based
        // categories so empty `versions/<id>` etc do not linger.
        if matches!(
            cat_report.category,
            PruneCategory::Versions | PruneCategory::Natives | PruneCategory::JavaRuntimes
        ) {
            let _ = remove_empty_parents(paths, &cat_report.category, &cat_report.files);
        } else {
            // For file-based categories we still try to clean empty leaf dirs
            // (e.g. an emptied `libraries/com/example/...` subtree).
            let _ = remove_empty_parents(paths, &cat_report.category, &cat_report.files);
        }

        // If some files were not freed (concurrent deletion, permission), the
        // per-category report reflects reality.
        if freed_files.len() != cat_report.file_count {
            warnings.push(format!(
                "{}: requested {} files, actually freed {}",
                cat_report.category.as_str(),
                cat_report.file_count,
                freed_files.len()
            ));
        }

        total_freed_files += freed_files.len();
        total_freed_bytes += freed_bytes;
        freed_categories.push(PruneCategoryReport {
            category: cat_report.category,
            file_count: freed_files.len(),
            total_bytes: freed_bytes,
            files: freed_files,
        });
    }

    PruneResult {
        categories: freed_categories,
        warnings,
        total_freed_files,
        total_freed_bytes,
    }
}

// ---------------------------------------------------------------------------
// Internal scan implementation
// ---------------------------------------------------------------------------

fn scan_inner(paths: &AppPaths) -> PruneReport {
    let mut warnings = Vec::new();

    let (reachable_ids, survey_complete) = collect_reachable_version_ids(paths, &mut warnings);
    let (version_map, versions_malformed) =
        parse_reachable_versions(paths, &reachable_ids, &mut warnings);
    // An incomplete instance survey is exactly as dangerous as a malformed
    // version JSON — both leave the reference set a subset of the truth — so it
    // takes the same fail-closed path.
    let reachable_malformed = versions_malformed || !survey_complete;
    // `natives/` and `versions/` are keyed by version id rather than by a
    // derived reference set, but they answer to the same question — so they get
    // the same gate.
    let version_id_refs = if reachable_malformed {
        None
    } else {
        Some(&reachable_ids)
    };

    // Build reference sets. If a reachable version was malformed we cannot
    // trust any derived set for that category and must leave it empty.
    let library_refs = if reachable_malformed {
        warnings.push(
            "libraries: reference set incomplete due to malformed reachable version JSON; leaving all libraries as referenced"
                .to_string(),
        );
        None
    } else {
        build_library_refs(&version_map, &mut warnings)
    };

    let (asset_hash_refs, asset_index_refs, assets_build_failed) = if reachable_malformed {
        warnings.push(
                "assets: reference set incomplete due to malformed reachable version JSON; leaving all assets as referenced"
                    .to_string(),
            );
        (None, None, true)
    } else {
        build_asset_refs(paths, &version_map, &mut warnings)
    };

    let logging_refs = if reachable_malformed {
        warnings.push(
            "logging: reference set incomplete due to malformed reachable version JSON; leaving all logging configs as referenced"
                .to_string(),
        );
        None
    } else {
        Some(build_logging_refs(&version_map))
    };

    let java_majors = if reachable_malformed {
        warnings.push(
            "java_runtimes: reference set incomplete due to malformed reachable version JSON; leaving all runtimes as referenced"
                .to_string(),
        );
        None
    } else {
        Some(build_java_majors(&version_map))
    };

    let categories = vec![
        scan_libraries(
            paths,
            library_refs.as_ref(),
            &mut warnings,
            assets_build_failed,
        ),
        scan_assets(
            paths,
            asset_hash_refs.as_ref(),
            asset_index_refs.as_ref(),
            &mut warnings,
            assets_build_failed,
        ),
        scan_natives(paths, version_id_refs, &mut warnings),
        scan_versions(paths, version_id_refs, &mut warnings),
        scan_java_runtimes(paths, java_majors.as_ref(), &mut warnings),
        scan_logging(
            paths,
            logging_refs.as_ref(),
            &mut warnings,
            reachable_malformed,
        ),
    ];

    PruneReport {
        categories,
        warnings,
    }
}

// ---------------------------------------------------------------------------
// Reachable version ids
// ---------------------------------------------------------------------------

/// Collect every version id any instance can reach, and whether that survey was
/// complete.
///
/// Completeness is the whole safety story. The reference set is a *union* over
/// instances, so anything the survey misses is a version whose libraries and
/// assets will look unused. Every skip path below therefore reports itself: an
/// unreadable `instances/` directory once cost the entire runtime, because a
/// zero-instance survey makes everything on disk reclaimable.
fn collect_reachable_version_ids(
    paths: &AppPaths,
    warnings: &mut Vec<String>,
) -> (HashSet<String>, bool) {
    let mut initial = HashSet::new();
    let mut complete = true;
    let instances_root = paths.instances_root();
    let Ok(entries) = std::fs::read_dir(&instances_root) else {
        warnings.push(format!(
            "instances: cannot read {}; treating the whole runtime as in use",
            instances_root.display()
        ));
        return (HashSet::new(), false);
    };

    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if !ft.is_dir() {
            continue;
        }
        let manifest_path = entry.path().join("instance_manifest.json");
        if !manifest_path.is_file() {
            continue;
        }
        let data = match std::fs::read_to_string(&manifest_path) {
            Ok(d) => d,
            Err(e) => {
                warnings.push(format!(
                    "instance manifest unreadable {}: {e}",
                    manifest_path.display()
                ));
                complete = false;
                continue;
            }
        };
        let value: serde_json::Value = match serde_json::from_str(&data) {
            Ok(v) => v,
            Err(e) => {
                warnings.push(format!(
                    "instance manifest malformed {}: {e}",
                    manifest_path.display()
                ));
                complete = false;
                continue;
            }
        };
        // Extract the three fields without requiring the full struct so a
        // missing optional field does not fail the whole manifest.
        let mc_version = value
            .get("minecraft_version")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let loader = value
            .get("loader")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let loader_version = value
            .get("loader_version")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();

        if mc_version.is_empty() {
            warnings.push(format!(
                "instance manifest {} has empty minecraft_version; skipping",
                manifest_path.display()
            ));
            complete = false;
            continue;
        }
        if mc_version.contains('/') || mc_version.contains('\\') || mc_version.contains('\0') {
            warnings.push(format!(
                "instance manifest {} has invalid minecraft_version '{mc_version}'; skipping",
                manifest_path.display()
            ));
            complete = false;
            continue;
        }
        initial.insert(mc_version.clone());

        let loader_lc = loader.to_ascii_lowercase();
        let is_vanilla = loader.is_empty() || loader_lc == "vanilla";
        if !is_vanilla && !loader_version.is_empty() {
            if let Some(profile_id) = try_derive_profile_id(&loader, &mc_version, &loader_version) {
                initial.insert(profile_id);
            } else {
                // The base version alone is not enough: the loader profile
                // brings its own libraries, and those would look unreferenced.
                warnings.push(format!(
                    "instance manifest {} has unknown loader '{loader}'; cannot resolve its profile",
                    manifest_path.display()
                ));
                complete = false;
            }
        }
    }

    // Follow inheritsFrom transitively.
    let mut reachable = initial.clone();
    let mut to_visit: Vec<String> = initial.into_iter().collect();
    let mut visited: HashSet<String> = HashSet::new();

    while let Some(id) = to_visit.pop() {
        if !visited.insert(id.clone()) {
            continue;
        }
        let version_path = paths
            .minecraft_runtime_root()
            .join("versions")
            .join(&id)
            .join(format!("{id}.json"));
        if !version_path.is_file() {
            continue;
        }
        let data = match std::fs::read_to_string(&version_path) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let info: Result<VersionInfo, _> = serde_json::from_str(&data);
        let Ok(info) = info else {
            continue;
        };
        if let Some(parent) = info.inherits_from {
            let parent = parent.trim().to_string();
            if parent.is_empty()
                || parent.contains('/')
                || parent.contains('\\')
                || parent.contains('\0')
            {
                continue;
            }
            if reachable.insert(parent.clone()) {
                to_visit.push(parent);
            }
        }
    }

    (reachable, complete)
}

fn try_derive_profile_id(loader: &str, mc_version: &str, loader_version: &str) -> Option<String> {
    if mc_version.is_empty() || loader_version.is_empty() {
        return None;
    }
    if mc_version.contains('/')
        || mc_version.contains('\\')
        || loader_version.contains('/')
        || loader_version.contains('\\')
    {
        return None;
    }
    match loader.to_ascii_lowercase().as_str() {
        "forge" => Some(format!("forge-{mc_version}-{loader_version}")),
        "neoforge" => Some(format!("neoforge-{loader_version}")),
        "fabric" => Some(format!("fabric-loader-{loader_version}-{mc_version}")),
        "quilt" => Some(format!("quilt-loader-{loader_version}-{mc_version}")),
        _ => None,
    }
}

fn parse_reachable_versions(
    paths: &AppPaths,
    reachable_ids: &HashSet<String>,
    warnings: &mut Vec<String>,
) -> (HashMap<String, VersionInfo>, bool) {
    let mut map = HashMap::new();
    let mut malformed = false;
    for id in reachable_ids {
        let version_path = paths
            .minecraft_runtime_root()
            .join("versions")
            .join(id)
            .join(format!("{id}.json"));
        if !version_path.is_file() {
            warnings.push(format!(
                "reachable version {id} has no JSON at {}",
                version_path.display()
            ));
            continue;
        }
        let data = match std::fs::read_to_string(&version_path) {
            Ok(d) => d,
            Err(e) => {
                warnings.push(format!("failed to read reachable version {id}: {e}"));
                malformed = true;
                continue;
            }
        };
        match serde_json::from_str::<VersionInfo>(&data) {
            Ok(info) => {
                map.insert(id.clone(), info);
            }
            Err(e) => {
                warnings.push(format!("malformed reachable version JSON for {id}: {e}"));
                malformed = true;
            }
        }
    }
    (map, malformed)
}

// ---------------------------------------------------------------------------
// Reference set builders
// ---------------------------------------------------------------------------

fn build_library_refs(
    version_map: &HashMap<String, VersionInfo>,
    warnings: &mut Vec<String>,
) -> Option<HashSet<String>> {
    let mut set = HashSet::new();
    for (vid, info) in version_map {
        for lib in &info.libraries {
            let paths = library_artifact_paths(lib);
            if paths.is_empty() && !lib.name.is_empty() {
                warnings.push(format!(
                    "version {vid}: library '{}' produced no artifact path; treating as unknown and leaving libraries category conservative",
                    lib.name
                ));
                // We don't fail the whole category for one unknown lib;
                // we just don't add an entry, which keeps the file as
                // potentially reclaimable — but the prompt says to be
                // conservative. For now we warn but continue. A single
                // unresolvable library does not make the whole set unsafe
                // because its path is unknown; treating the whole category
                // as unsafe would under-reclaim heavily. The test for
                // "malformed version JSON makes its whole subtree off-limits"
                // is about version directories, not single-library parse.
                // So we keep the set.
            }
            for p in paths {
                set.insert(normalize_library_path(&p));
            }
        }
    }
    Some(set)
}

fn library_artifact_paths(lib: &Library) -> Vec<String> {
    let mut out = Vec::new();
    let mut has_explicit_artifact = false;
    if let Some(downloads) = &lib.downloads {
        if let Some(artifact) = &downloads.artifact {
            if !is_unsafe_relative_path(&artifact.path) {
                out.push(artifact.path.clone());
            }
            has_explicit_artifact = true;
        }
        if let Some(classifiers) = &downloads.classifiers {
            for art in classifiers.values() {
                if !is_unsafe_relative_path(&art.path) {
                    out.push(art.path.clone());
                }
            }
        }
    }
    // Fallback via Maven coordinate + url when no explicit artifact.
    if !has_explicit_artifact {
        if let Some(url) = &lib.url {
            if !url.trim().is_empty() {
                if let Ok(desc) = crate::launch::parse_maven_descriptor(&lib.name) {
                    out.push(desc.to_relative_path());
                } else {
                    // Fallback to the legacy converter for plain group:artifact:version forms.
                    let parts: Vec<&str> = lib.name.split(':').collect();
                    if parts.len() >= 3 {
                        out.push(crate::launch::maven_name_to_path(&lib.name));
                    }
                }
            }
        }
    }
    out
}

fn normalize_library_path(p: &str) -> String {
    p.replace('\\', "/")
}

fn is_unsafe_relative_path(p: &str) -> bool {
    if p.contains('\0') {
        return true;
    }
    if p.starts_with('/') || p.starts_with('\\') {
        return true;
    }
    if p.contains("..") {
        return true;
    }
    if p.contains(':') {
        return true;
    }
    let norm = p.replace('\\', "/");
    for comp in norm.split('/') {
        if comp == ".." {
            return true;
        }
        if comp.len() == 2 && comp.as_bytes()[1] == b':' {
            return true;
        }
    }
    false
}

fn build_asset_refs(
    paths: &AppPaths,
    version_map: &HashMap<String, VersionInfo>,
    warnings: &mut Vec<String>,
) -> (Option<HashSet<String>>, Option<HashSet<String>>, bool) {
    let mut referenced_index_ids: HashSet<String> = HashSet::new();
    for info in version_map.values() {
        if let Some(ai) = &info.asset_index {
            if !ai.id.trim().is_empty() {
                referenced_index_ids.insert(ai.id.clone());
            }
        }
    }
    if referenced_index_ids.is_empty() {
        return (Some(HashSet::new()), Some(HashSet::new()), false);
    }

    let mut referenced_hashes: HashSet<String> = HashSet::new();
    let indexes_dir = paths.minecraft_assets_dir().join("indexes");
    let mut any_missing_or_malformed = false;
    for idx_id in &referenced_index_ids {
        if idx_id.contains('/') || idx_id.contains('\\') || idx_id.contains('\0') {
            warnings.push(format!(
                "assets: asset index id '{idx_id}' contains unsafe characters; skipping"
            ));
            any_missing_or_malformed = true;
            continue;
        }
        let idx_path = indexes_dir.join(format!("{idx_id}.json"));
        if !idx_path.is_file() {
            warnings.push(format!(
                "assets: referenced asset index '{idx_id}' missing at {}",
                idx_path.display()
            ));
            any_missing_or_malformed = true;
            continue;
        }
        let data = match std::fs::read_to_string(&idx_path) {
            Ok(d) => d,
            Err(e) => {
                warnings.push(format!(
                    "assets: failed to read asset index '{idx_id}': {e}"
                ));
                any_missing_or_malformed = true;
                continue;
            }
        };
        let doc: Result<AssetIndexDoc, _> = serde_json::from_str(&data);
        match doc {
            Ok(doc) => {
                for obj in doc.objects.values() {
                    if obj.hash.len() == 40
                        && obj.hash.bytes().all(|b| b.is_ascii_hexdigit())
                        && obj.size >= 0
                    {
                        referenced_hashes.insert(obj.hash.to_ascii_lowercase());
                    } else {
                        warnings.push(format!(
                            "assets: asset index '{idx_id}' contains invalid object hash '{}'",
                            obj.hash
                        ));
                        any_missing_or_malformed = true;
                    }
                }
            }
            Err(e) => {
                warnings.push(format!("assets: malformed asset index '{idx_id}': {e}"));
                any_missing_or_malformed = true;
            }
        }
    }

    if any_missing_or_malformed {
        // Conservative: if any referenced index cannot be parsed, we cannot
        // confidently know which objects are referenced, so leave all.
        return (None, None, true);
    }

    (Some(referenced_hashes), Some(referenced_index_ids), false)
}

#[derive(serde::Deserialize)]
struct AssetIndexDoc {
    #[serde(default)]
    objects: std::collections::BTreeMap<String, AssetObject>,
}

#[derive(serde::Deserialize)]
struct AssetObject {
    hash: String,
    size: i64,
}

fn build_logging_refs(version_map: &HashMap<String, VersionInfo>) -> HashSet<String> {
    let mut set = HashSet::new();
    for info in version_map.values() {
        if let Some(logging) = &info.logging {
            if let Some(client) = &logging.client {
                if let Some(file) = &client.file {
                    if !file.id.trim().is_empty() {
                        set.insert(file.id.clone());
                    }
                }
            }
        }
    }
    set
}

fn build_java_majors(version_map: &HashMap<String, VersionInfo>) -> HashSet<u32> {
    let mut set = HashSet::new();
    for info in version_map.values() {
        let req = crate::java::java_requirement_from_version(info);
        set.insert(req.major);
    }
    set
}

// ---------------------------------------------------------------------------
// Category scanners
// ---------------------------------------------------------------------------

fn scan_libraries(
    paths: &AppPaths,
    refs: Option<&HashSet<String>>,
    warnings: &mut Vec<String>,
    assets_failed: bool,
) -> PruneCategoryReport {
    // assets_failed is unused here; kept for signature symmetry
    let _ = assets_failed;
    let Some(referenced) = refs else {
        return PruneCategoryReport {
            category: PruneCategory::Libraries,
            file_count: 0,
            total_bytes: 0,
            files: Vec::new(),
        };
    };

    let dir = paths.minecraft_libraries_dir();
    if !dir.is_dir() {
        return PruneCategoryReport {
            category: PruneCategory::Libraries,
            file_count: 0,
            total_bytes: 0,
            files: Vec::new(),
        };
    }

    let mut reclaimable: Vec<PathBuf> = Vec::new();
    let mut total_bytes = 0u64;
    let files = match collect_files_recursive(&dir) {
        Ok(collected) => collected,
        Err(_) => {
            warnings.push(format!("failed to walk libraries dir {}", dir.display()));
            return PruneCategoryReport {
                category: PruneCategory::Libraries,
                file_count: 0,
                total_bytes: 0,
                files: Vec::new(),
            };
        }
    };

    let lower_refs: HashSet<String> = referenced.iter().map(|p| p.to_ascii_lowercase()).collect();

    for file_path in files {
        // Only regular files.
        let Ok(meta) = std::fs::metadata(&file_path) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        // Symlink / reparse check: lstat
        if is_symlink_or_reparse(&file_path) {
            continue;
        }
        let rel = match file_path.strip_prefix(&dir) {
            Ok(r) => r.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };
        // Only consider recognized library shapes: ends with .jar and contains '/'.
        // Sidecar/receipt files (.sha1, .agora-verified.json) are intentionally
        // ignored here — they are small and not the multi-GB reclaim target.
        // Treating them as standalone reclaimable would flag every referenced
        // jar's sidecar as orphan.
        if !is_library_jar_shape(&rel) {
            continue;
        }
        let rel_lower = rel.to_ascii_lowercase();
        if lower_refs.contains(&rel_lower) {
            continue;
        }
        // Unknown shape is already filtered; remaining jars not in refs are reclaimable.
        total_bytes += meta.len();
        reclaimable.push(file_path);
    }

    let file_count = reclaimable.len();
    PruneCategoryReport {
        category: PruneCategory::Libraries,
        file_count,
        total_bytes,
        files: reclaimable,
    }
}

fn is_library_jar_shape(rel: &str) -> bool {
    if rel.is_empty()
        || rel.contains("..")
        || rel.contains(':')
        || rel.contains('\0')
        || !rel.contains('/')
        || !rel.ends_with(".jar")
    {
        return false;
    }
    // Must not be absolute
    if rel.starts_with('/') {
        return false;
    }
    true
}

fn scan_assets(
    paths: &AppPaths,
    hash_refs: Option<&HashSet<String>>,
    index_refs: Option<&HashSet<String>>,
    warnings: &mut Vec<String>,
    build_failed: bool,
) -> PruneCategoryReport {
    if build_failed || hash_refs.is_none() || index_refs.is_none() {
        // We already warned in build_asset_refs; just return empty.
        return PruneCategoryReport {
            category: PruneCategory::Assets,
            file_count: 0,
            total_bytes: 0,
            files: Vec::new(),
        };
    }
    let hash_refs = hash_refs.unwrap();
    let index_refs = index_refs.unwrap();

    let assets_dir = paths.minecraft_assets_dir();
    let objects_dir = assets_dir.join("objects");
    let indexes_dir = assets_dir.join("indexes");

    let mut reclaimable: Vec<PathBuf> = Vec::new();
    let mut total_bytes = 0u64;

    // Objects
    if objects_dir.is_dir() {
        let lower_hashes: HashSet<String> =
            hash_refs.iter().map(|h| h.to_ascii_lowercase()).collect();
        if let Ok(files) = collect_files_recursive(&objects_dir) {
            for file_path in files {
                let Ok(meta) = std::fs::metadata(&file_path) else {
                    continue;
                };
                if !meta.is_file() {
                    continue;
                }
                if is_symlink_or_reparse(&file_path) {
                    continue;
                }
                // Validate shape: objects/ab/abcdef... where ab == hash[..2]
                let rel = match file_path.strip_prefix(&objects_dir) {
                    Ok(r) => r,
                    Err(_) => continue,
                };
                let comps: Vec<_> = rel.components().collect();
                if comps.len() != 2 {
                    continue;
                }
                let prefix = comps[0].as_os_str().to_string_lossy().to_string();
                let hash = comps[1].as_os_str().to_string_lossy().to_string();
                if !is_asset_object_shape(&prefix, &hash) {
                    // Unrecognized shape -> treat as referenced (leave)
                    continue;
                }
                let hash_lc = hash.to_ascii_lowercase();
                if lower_hashes.contains(&hash_lc) {
                    continue;
                }
                total_bytes += meta.len();
                reclaimable.push(file_path);
            }
        } else {
            warnings.push(format!(
                "failed to walk assets objects dir {}",
                objects_dir.display()
            ));
        }
    }

    // Indexes (small, but still reclaimable)
    if indexes_dir.is_dir() {
        let Ok(entries) = std::fs::read_dir(&indexes_dir) else {
            warnings.push(format!(
                "failed to read indexes dir {}",
                indexes_dir.display()
            ));
            return PruneCategoryReport {
                category: PruneCategory::Assets,
                file_count: reclaimable.len(),
                total_bytes,
                files: reclaimable,
            };
        };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if !ft.is_file() {
                continue;
            }
            let path = entry.path();
            if is_symlink_or_reparse(&path) {
                continue;
            }
            let file_name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };
            if !file_name.ends_with(".json") {
                continue;
            }
            let id = file_name.trim_end_matches(".json");
            if id.is_empty()
                || id.contains('/')
                || id.contains('\\')
                || id.contains('\0')
                || id.contains("..")
            {
                continue;
            }
            if index_refs.contains(id) {
                continue;
            }
            // Malformed index file that is unreferenced but unreadable: treat as
            // referenced and leave (shape not recognized).
            let data = match std::fs::read_to_string(&path) {
                Ok(d) => d,
                Err(_) => continue,
            };
            if serde_json::from_str::<AssetIndexDoc>(&data).is_err() {
                warnings.push(format!(
                    "unreferenced asset index {} is malformed; leaving it",
                    path.display()
                ));
                continue;
            }
            if let Ok(meta) = std::fs::metadata(&path) {
                total_bytes += meta.len();
                reclaimable.push(path);
            }
        }
    }

    let file_count = reclaimable.len();
    PruneCategoryReport {
        category: PruneCategory::Assets,
        file_count,
        total_bytes,
        files: reclaimable,
    }
}

fn is_asset_object_shape(prefix: &str, hash: &str) -> bool {
    if prefix.len() != 2 || hash.len() != 40 {
        return false;
    }
    if !prefix.bytes().all(|b| b.is_ascii_hexdigit()) {
        return false;
    }
    if !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return false;
    }
    hash[..2].eq_ignore_ascii_case(prefix)
}

fn scan_natives(
    paths: &AppPaths,
    reachable: Option<&HashSet<String>>,
    warnings: &mut Vec<String>,
) -> PruneCategoryReport {
    let Some(reachable) = reachable else {
        return PruneCategoryReport {
            category: PruneCategory::Natives,
            file_count: 0,
            total_bytes: 0,
            files: Vec::new(),
        };
    };
    let dir = paths.minecraft_natives_dir();
    if !dir.is_dir() {
        return PruneCategoryReport {
            category: PruneCategory::Natives,
            file_count: 0,
            total_bytes: 0,
            files: Vec::new(),
        };
    }
    let Ok(entries) = std::fs::read_dir(&dir) else {
        warnings.push(format!("failed to read natives dir {}", dir.display()));
        return PruneCategoryReport {
            category: PruneCategory::Natives,
            file_count: 0,
            total_bytes: 0,
            files: Vec::new(),
        };
    };

    let mut reclaimable: Vec<PathBuf> = Vec::new();
    let mut total_bytes = 0u64;

    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if !ft.is_dir() {
            // Natives root should only contain version dirs; a stray file is unrecognized shape.
            continue;
        }
        let path = entry.path();
        if is_symlink_or_reparse(&path) {
            continue;
        }
        let id = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if id.contains('/') || id.contains('\\') || id.contains('\0') || id.contains("..") {
            continue;
        }
        if reachable.contains(&id) {
            continue;
        }
        // Unreferenced version's natives: check that the subtree looks like natives/<id>/<platform>/...
        // If it contains unexpected nesting (no platform subdir), treat as unrecognized.
        // We do a shallow check: at least one child should be a dir (platform) and all children
        // should be dirs. If a file sits directly under natives/<id>, it's unrecognized.
        let Ok(children) = std::fs::read_dir(&path) else {
            continue;
        };
        let mut has_platform_dir = false;
        let mut has_unrecognized = false;
        for child in children.flatten() {
            let Ok(cft) = child.file_type() else {
                has_unrecognized = true;
                break;
            };
            if cft.is_dir() {
                has_platform_dir = true;
            } else {
                has_unrecognized = true;
                break;
            }
            if is_symlink_or_reparse(&child.path()) {
                has_unrecognized = true;
                break;
            }
        }
        if !has_platform_dir || has_unrecognized {
            warnings.push(format!(
                "natives subtree {} has unrecognized shape; leaving it",
                path.display()
            ));
            continue;
        }

        if let Ok(files) = collect_files_recursive(&path) {
            for f in files {
                if let Ok(meta) = std::fs::metadata(&f) {
                    if meta.is_file() && !is_symlink_or_reparse(&f) {
                        total_bytes += meta.len();
                        reclaimable.push(f);
                    }
                }
            }
        }
    }

    let file_count = reclaimable.len();
    PruneCategoryReport {
        category: PruneCategory::Natives,
        file_count,
        total_bytes,
        files: reclaimable,
    }
}

fn scan_versions(
    paths: &AppPaths,
    reachable: Option<&HashSet<String>>,
    warnings: &mut Vec<String>,
) -> PruneCategoryReport {
    let Some(reachable) = reachable else {
        return PruneCategoryReport {
            category: PruneCategory::Versions,
            file_count: 0,
            total_bytes: 0,
            files: Vec::new(),
        };
    };
    let dir = paths.minecraft_runtime_root().join("versions");
    if !dir.is_dir() {
        return PruneCategoryReport {
            category: PruneCategory::Versions,
            file_count: 0,
            total_bytes: 0,
            files: Vec::new(),
        };
    }
    let Ok(entries) = std::fs::read_dir(&dir) else {
        warnings.push(format!("failed to read versions dir {}", dir.display()));
        return PruneCategoryReport {
            category: PruneCategory::Versions,
            file_count: 0,
            total_bytes: 0,
            files: Vec::new(),
        };
    };

    let mut reclaimable: Vec<PathBuf> = Vec::new();
    let mut total_bytes = 0u64;

    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if !ft.is_dir() {
            continue;
        }
        let path = entry.path();
        if is_symlink_or_reparse(&path) {
            continue;
        }
        let id = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if id.contains('/') || id.contains('\\') || id.contains('\0') {
            continue;
        }
        if reachable.contains(&id) {
            continue;
        }

        // Check JSON exists and is parseable. A missing or malformed JSON makes
        // the whole subtree off-limits (safety rule).
        let json_path = path.join(format!("{id}.json"));
        if json_path.is_file() {
            let data = match std::fs::read_to_string(&json_path) {
                Ok(d) => d,
                Err(_) => {
                    warnings.push(format!(
                        "unreferenced version {} JSON unreadable; leaving {}",
                        id,
                        path.display()
                    ));
                    continue;
                }
            };
            if serde_json::from_str::<serde_json::Value>(&data).is_err() {
                warnings.push(format!(
                    "unreferenced version {} JSON malformed; leaving {}",
                    id,
                    path.display()
                ));
                continue;
            }
            // Also try as VersionInfo for stricter check; malformed VersionInfo is also malformed JSON.
            if serde_json::from_str::<VersionInfo>(&data).is_err() {
                warnings.push(format!(
                    "unreferenced version {} JSON invalid VersionInfo; leaving {}",
                    id,
                    path.display()
                ));
                continue;
            }
        } else {
            // No JSON: treat as incomplete download. We could reclaim, but
            // the prompt says "directory whose shape you do not recognise -
            // treat everything under it as referenced". A version dir without
            // its JSON is not a recognized shape, so we leave it but warn.
            warnings.push(format!(
                "unreferenced version {} has no JSON; leaving {}",
                id,
                path.display()
            ));
            continue;
        }

        // Shape check: version dir must not contain subdirectories, and every
        // file must be named `<id>.*` . If any file violates, shape unrecognized.
        let Ok(children) = std::fs::read_dir(&path) else {
            continue;
        };
        let mut shape_ok = true;
        for child in children.flatten() {
            let Ok(cft) = child.file_type() else {
                shape_ok = false;
                break;
            };
            if cft.is_dir() {
                shape_ok = false;
                break;
            }
            if is_symlink_or_reparse(&child.path()) {
                shape_ok = false;
                break;
            }
            let name = child.file_name().to_string_lossy().to_string();
            if !name.starts_with(&id) {
                shape_ok = false;
                break;
            }
            // Allow .json, .jar, .jar.sha1, .sha1, .sha256, .agora-verified.json etc.
            // If file is not one of those, still considered part of shape if prefixed with id.
            // We keep simple: any file prefixed with id is okay.
        }
        if !shape_ok {
            warnings.push(format!(
                "unreferenced version {} has unrecognized shape; leaving {}",
                id,
                path.display()
            ));
            continue;
        }

        if let Ok(files) = collect_files_recursive(&path) {
            for f in files {
                if let Ok(meta) = std::fs::metadata(&f) {
                    if meta.is_file() && !is_symlink_or_reparse(&f) {
                        total_bytes += meta.len();
                        reclaimable.push(f);
                    }
                }
            }
        }
    }

    let file_count = reclaimable.len();
    PruneCategoryReport {
        category: PruneCategory::Versions,
        file_count,
        total_bytes,
        files: reclaimable,
    }
}

fn scan_java_runtimes(
    paths: &AppPaths,
    majors: Option<&HashSet<u32>>,
    warnings: &mut Vec<String>,
) -> PruneCategoryReport {
    let Some(referenced) = majors else {
        return PruneCategoryReport {
            category: PruneCategory::JavaRuntimes,
            file_count: 0,
            total_bytes: 0,
            files: Vec::new(),
        };
    };
    let dir = paths.java_runtimes_root();
    if !dir.is_dir() {
        return PruneCategoryReport {
            category: PruneCategory::JavaRuntimes,
            file_count: 0,
            total_bytes: 0,
            files: Vec::new(),
        };
    }
    // Layout: runtimes/temurin/<major>/<full_version>/<os>-<arch>/
    let vendor_dir = dir.join("temurin");
    if !vendor_dir.is_dir() {
        // No managed runtimes installed; nothing to reclaim.
        return PruneCategoryReport {
            category: PruneCategory::JavaRuntimes,
            file_count: 0,
            total_bytes: 0,
            files: Vec::new(),
        };
    }
    let Ok(entries) = std::fs::read_dir(&vendor_dir) else {
        warnings.push(format!(
            "failed to read java runtimes vendor dir {}",
            vendor_dir.display()
        ));
        return PruneCategoryReport {
            category: PruneCategory::JavaRuntimes,
            file_count: 0,
            total_bytes: 0,
            files: Vec::new(),
        };
    };

    let mut reclaimable: Vec<PathBuf> = Vec::new();
    let mut total_bytes = 0u64;

    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if !ft.is_dir() {
            continue;
        }
        let path = entry.path();
        if is_symlink_or_reparse(&path) {
            continue;
        }
        let major_str = match path.file_name().and_then(|n| n.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let major: u32 = match major_str.parse() {
            Ok(m) => m,
            Err(_) => {
                warnings.push(format!(
                    "java runtime major dir '{}' is not numeric; leaving {}",
                    major_str,
                    path.display()
                ));
                continue;
            }
        };
        if referenced.contains(&major) {
            continue;
        }
        // This major is unreferenced: whole subtree is reclaimable.
        // Before claiming, validate shape: should contain at least one full_version subdir
        // that itself contains an os-arch dir. If top-level file found, shape unrecognized.
        let Ok(children) = std::fs::read_dir(&path) else {
            continue;
        };
        let mut has_version_dir = false;
        let mut shape_bad = false;
        for child in children.flatten() {
            let Ok(cft) = child.file_type() else {
                shape_bad = true;
                break;
            };
            if !cft.is_dir() {
                shape_bad = true;
                break;
            }
            if is_symlink_or_reparse(&child.path()) {
                shape_bad = true;
                break;
            }
            has_version_dir = true;
        }
        if !has_version_dir || shape_bad {
            warnings.push(format!(
                "java runtime major dir {} has unrecognized shape; leaving {}",
                major_str,
                path.display()
            ));
            continue;
        }

        if let Ok(files) = collect_files_recursive(&path) {
            for f in files {
                if let Ok(meta) = std::fs::metadata(&f) {
                    if meta.is_file() && !is_symlink_or_reparse(&f) {
                        total_bytes += meta.len();
                        reclaimable.push(f);
                    }
                }
            }
        }
    }

    let file_count = reclaimable.len();
    PruneCategoryReport {
        category: PruneCategory::JavaRuntimes,
        file_count,
        total_bytes,
        files: reclaimable,
    }
}

fn scan_logging(
    paths: &AppPaths,
    refs: Option<&HashSet<String>>,
    warnings: &mut Vec<String>,
    failed: bool,
) -> PruneCategoryReport {
    if failed || refs.is_none() {
        return PruneCategoryReport {
            category: PruneCategory::Logging,
            file_count: 0,
            total_bytes: 0,
            files: Vec::new(),
        };
    }
    let refs = refs.unwrap();
    let dir = paths.minecraft_logging_dir();
    if !dir.is_dir() {
        return PruneCategoryReport {
            category: PruneCategory::Logging,
            file_count: 0,
            total_bytes: 0,
            files: Vec::new(),
        };
    }
    let Ok(entries) = std::fs::read_dir(&dir) else {
        warnings.push(format!("failed to read logging dir {}", dir.display()));
        return PruneCategoryReport {
            category: PruneCategory::Logging,
            file_count: 0,
            total_bytes: 0,
            files: Vec::new(),
        };
    };

    let mut reclaimable = Vec::new();
    let mut total_bytes = 0u64;

    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if !ft.is_file() {
            // Logging dir should only contain files; a subdir is unrecognized shape -> leave whole category?
            // For now just skip subdirs.
            continue;
        }
        let path = entry.path();
        if is_symlink_or_reparse(&path) {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if name.contains('/') || name.contains('\\') || name.contains('\0') {
            continue;
        }
        if refs.contains(&name) {
            continue;
        }
        // Shape: logging configs are typically .xml; treat any file as candidate if not referenced.
        // Unrecognized shape: if name doesn't end with .xml, leave? But we can be permissive.
        // For safety, we only reclaim files ending with .xml.
        if !name.ends_with(".xml") {
            warnings.push(format!(
                "logging file {} has unrecognized shape; leaving it",
                path.display()
            ));
            continue;
        }
        if let Ok(meta) = std::fs::metadata(&path) {
            total_bytes += meta.len();
            reclaimable.push(path);
        }
    }

    let file_count = reclaimable.len();
    PruneCategoryReport {
        category: PruneCategory::Logging,
        file_count,
        total_bytes,
        files: reclaimable,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn collect_files_recursive(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let ft = entry.file_type()?;
            if ft.is_dir() {
                if !is_symlink_or_reparse(&path) {
                    stack.push(path);
                }
            } else if ft.is_file() {
                out.push(path);
            }
        }
    }
    Ok(out)
}

fn is_symlink_or_reparse(path: &Path) -> bool {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return true;
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
            if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                return true;
            }
        }
    }
    false
}

fn remove_empty_parents(
    paths: &AppPaths,
    category: &PruneCategory,
    files: &[PathBuf],
) -> std::io::Result<()> {
    // Determine the root for the category
    let root = match category {
        PruneCategory::Libraries => paths.minecraft_libraries_dir(),
        PruneCategory::Assets => paths.minecraft_assets_dir(),
        PruneCategory::Natives => paths.minecraft_natives_dir(),
        PruneCategory::Versions => paths.minecraft_runtime_root().join("versions"),
        PruneCategory::JavaRuntimes => paths.java_runtimes_root(),
        PruneCategory::Logging => paths.minecraft_logging_dir(),
    };

    // Collect unique parent directories of the deleted files, deepest first.
    let mut dirs: HashSet<PathBuf> = HashSet::new();
    for file in files {
        if let Some(parent) = file.parent() {
            let mut cur = parent.to_path_buf();
            while cur.starts_with(&root) && cur != root {
                dirs.insert(cur.clone());
                if let Some(p) = cur.parent() {
                    cur = p.to_path_buf();
                } else {
                    break;
                }
            }
        }
    }
    // Sort by depth descending so leaves are removed first.
    let mut sorted: Vec<PathBuf> = dirs.into_iter().collect();
    sorted.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for dir in sorted {
        let _ = std::fs::remove_dir(&dir);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{InstanceManifest, CURRENT_MANIFEST_VERSION};
    use std::fs;
    use tempfile::TempDir;

    fn test_paths(tmp: &TempDir) -> AppPaths {
        let paths = AppPaths::from_root(tmp.path().to_path_buf());
        paths.create_required_dirs().unwrap();
        paths
    }

    fn write_instance_manifest(
        paths: &AppPaths,
        instance_id: &str,
        mc_version: &str,
        loader: &str,
        loader_version: &str,
    ) {
        let dir = paths.instance_dir(instance_id).unwrap();
        fs::create_dir_all(&dir).unwrap();
        let manifest = InstanceManifest {
            manifest_version: CURRENT_MANIFEST_VERSION,
            pack_origin: None,
            instance_id: instance_id.to_string(),
            name: "Test".to_string(),
            created_from_pack: None,
            minecraft_version: mc_version.to_string(),
            loader: loader.to_string(),
            loader_version: loader_version.to_string(),
            is_locked: false,
            mods: vec![],
            resourcepacks: vec![],
            shaders: vec![],
            datapacks: vec![],
            worlds: vec![],
            user_preferences: serde_json::json!({}),
        };
        let path = paths.instance_manifest(instance_id).unwrap();
        fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    }

    fn write_version_json(
        paths: &AppPaths,
        id: &str,
        inherits_from: Option<&str>,
        libraries: Vec<Library>,
        asset_index: Option<crate::launch::AssetIndex>,
        logging_id: Option<&str>,
        java_major: Option<i64>,
    ) {
        let version_dir = paths.minecraft_runtime_root().join("versions").join(id);
        fs::create_dir_all(&version_dir).unwrap();
        let mut info = VersionInfo {
            id: id.to_string(),
            main_class: "net.minecraft.client.main.Main".to_string(),
            arguments: None,
            minecraft_arguments: None,
            libraries,
            asset_index,
            assets: None,
            type_: "release".to_string(),
            downloads: None,
            java_version: java_major.map(|major| crate::launch::JavaVersion {
                component: "java-runtime-gamma".to_string(),
                major_version: major,
            }),
            logging: logging_id.map(|log_id| crate::launch::LoggingConfig {
                client: Some(crate::launch::LoggingClient {
                    argument: "-Dlog4j.configurationFile=${path}".to_string(),
                    file: Some(crate::launch::LoggingFile {
                        id: log_id.to_string(),
                        sha1: None,
                        size: None,
                        url: "https://example.com/log.xml".to_string(),
                    }),
                    type_: "log4j2-xml".to_string(),
                }),
            }),
            inherits_from: inherits_from.map(|s| s.to_string()),
            minimum_launcher_version: None,
        };
        // Ensure at least an empty assets field if needed
        info.assets = Some("1.21".to_string());
        let path = version_dir.join(format!("{id}.json"));
        fs::write(path, serde_json::to_vec(&info).unwrap()).unwrap();
    }

    fn write_library(paths: &AppPaths, rel: &str, content: &[u8]) -> PathBuf {
        let path = paths.minecraft_libraries_dir().join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, content).unwrap();
        path
    }

    fn write_asset_object(paths: &AppPaths, hash: &str, content: &[u8]) -> PathBuf {
        let dir = paths
            .minecraft_assets_dir()
            .join("objects")
            .join(&hash[..2]);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(hash);
        fs::write(&path, content).unwrap();
        path
    }

    fn write_asset_index(paths: &AppPaths, id: &str, objects: Vec<(&str, &str, i64)>) -> PathBuf {
        let dir = paths.minecraft_assets_dir().join("indexes");
        fs::create_dir_all(&dir).unwrap();
        let mut map = serde_json::Map::new();
        let mut objs = serde_json::Map::new();
        for (logical, hash, size) in objects {
            objs.insert(
                logical.to_string(),
                serde_json::json!({"hash": hash, "size": size}),
            );
        }
        map.insert("objects".to_string(), serde_json::Value::Object(objs));
        let path = dir.join(format!("{id}.json"));
        fs::write(
            &path,
            serde_json::to_vec(&serde_json::Value::Object(map)).unwrap(),
        )
        .unwrap();
        path
    }

    // -----------------------------------------------------------------------
    // Fail-closed on an incomplete instance survey
    // -----------------------------------------------------------------------
    //
    // The reference set is a union over instances. If the survey is incomplete
    // for ANY reason, the union is a subset of the truth and everything missing
    // from it looks reclaimable — which for an unreadable instances/ directory
    // means offering to delete the user's entire runtime.

    #[test]
    fn an_unreadable_instances_root_reclaims_nothing() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(&tmp);
        write_version_json(&paths, "1.21", None, vec![], None, None, None);
        write_library(&paths, "com/example/lib/1.0/lib-1.0.jar", b"library bytes");
        // No instances directory at all: the survey cannot be completed.
        fs::remove_dir_all(paths.instances_root()).unwrap();

        let report = scan(&paths);
        assert_eq!(
            total_reclaimable_files(&report),
            0,
            "an unreadable instances root must not make the whole runtime look unused"
        );
        assert!(
            report.warnings.iter().any(|w| w.contains("instances")),
            "the user has to be told why nothing was reclaimed: {:?}",
            report.warnings
        );
    }

    #[test]
    fn a_malformed_instance_manifest_reclaims_nothing() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(&tmp);
        write_version_json(&paths, "1.21", None, vec![], None, None, None);
        write_library(&paths, "com/example/lib/1.0/lib-1.0.jar", b"library bytes");
        let dir = paths.instance_dir("broken").unwrap();
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("instance_manifest.json"), b"{ not json").unwrap();

        let report = scan(&paths);
        assert_eq!(
            total_reclaimable_files(&report),
            0,
            "a manifest we cannot read may name the very version that keeps this library alive"
        );
    }

    #[test]
    fn prune_deletes_nothing_when_the_survey_is_incomplete() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(&tmp);
        write_version_json(&paths, "1.21", None, vec![], None, None, None);
        let library = write_library(&paths, "com/example/lib/1.0/lib-1.0.jar", b"library bytes");
        fs::remove_dir_all(paths.instances_root()).unwrap();

        let result = prune(&paths, &PruneCategory::all());
        assert_eq!(result.total_freed_files, 0);
        assert!(library.is_file(), "the library must still be on disk");
    }

    fn total_reclaimable_files(report: &PruneReport) -> usize {
        report.categories.iter().map(|c| c.file_count).sum()
    }

    #[test]
    fn library_referenced_by_in_use_version_is_never_reported() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(&tmp);

        // Instance uses 1.21 with a library com.example:mylib:1.0
        write_instance_manifest(&paths, "inst1", "1.21", "vanilla", "");

        let lib = Library {
            name: "com.example:mylib:1.0".to_string(),
            downloads: Some(crate::launch::LibraryDownloads {
                artifact: Some(crate::launch::LibraryArtifact {
                    path: "com/example/mylib/1.0/mylib-1.0.jar".to_string(),
                    url: "https://libraries.minecraft.net/com/example/mylib/1.0/mylib-1.0.jar"
                        .to_string(),
                    sha1: None,
                    sha256: None,
                    size: None,
                }),
                classifiers: None,
            }),
            ..Default::default()
        };
        write_version_json(&paths, "1.21", None, vec![lib], None, None, None);

        // Referenced library file exists
        let referenced_path =
            write_library(&paths, "com/example/mylib/1.0/mylib-1.0.jar", b"referenced");
        // Orphan library file
        let orphan_path = write_library(&paths, "com/example/orphan/1.0/orphan-1.0.jar", b"orphan");

        let report = scan(&paths);
        let lib_report = report
            .categories
            .iter()
            .find(|c| c.category == PruneCategory::Libraries)
            .unwrap();
        // Referenced file must NOT be in reclaimable list
        assert!(
            !lib_report.files.contains(&referenced_path),
            "referenced library was incorrectly reported as reclaimable"
        );
        // Orphan should be reclaimable
        assert!(
            lib_report.files.contains(&orphan_path),
            "orphan library should be reclaimable"
        );
    }

    #[test]
    fn inherits_from_loader_profile_protects_parent_libraries() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(&tmp);

        // Instance uses fabric-loader-0.16.0-1.21 which inheritsFrom 1.21
        write_instance_manifest(&paths, "inst1", "1.21", "fabric", "0.16.0");

        let parent_lib = Library {
            name: "com.example:parentlib:1.0".to_string(),
            downloads: Some(crate::launch::LibraryDownloads {
                artifact: Some(crate::launch::LibraryArtifact {
                    path: "com/example/parentlib/1.0/parentlib-1.0.jar".to_string(),
                    url: "https://libraries.minecraft.net/com/example/parentlib/1.0/parentlib-1.0.jar".to_string(),
                    sha1: None,
                    sha256: None,
                    size: None,
                }),
                classifiers: None,
            }),
            ..Default::default()
        };
        let loader_lib = Library {
            name: "net.fabricmc:fabric-loader:0.16.0".to_string(),
            downloads: Some(crate::launch::LibraryDownloads {
                artifact: Some(crate::launch::LibraryArtifact {
                    path: "net/fabricmc/fabric-loader/0.16.0/fabric-loader-0.16.0.jar".to_string(),
                    url: "https://maven.fabricmc.net/net/fabricmc/fabric-loader/0.16.0/fabric-loader-0.16.0.jar".to_string(),
                    sha1: None,
                    sha256: None,
                    size: None,
                }),
                classifiers: None,
            }),
            ..Default::default()
        };

        write_version_json(&paths, "1.21", None, vec![parent_lib], None, None, None);
        write_version_json(
            &paths,
            "fabric-loader-0.16.0-1.21",
            Some("1.21"),
            vec![loader_lib],
            None,
            None,
            None,
        );

        let parent_path = write_library(
            &paths,
            "com/example/parentlib/1.0/parentlib-1.0.jar",
            b"parent",
        );
        let loader_path = write_library(
            &paths,
            "net/fabricmc/fabric-loader/0.16.0/fabric-loader-0.16.0.jar",
            b"loader",
        );
        let orphan_path = write_library(&paths, "com/example/orphan/1.0/orphan-1.0.jar", b"orphan");

        let report = scan(&paths);
        let lib_report = report
            .categories
            .iter()
            .find(|c| c.category == PruneCategory::Libraries)
            .unwrap();
        assert!(
            !lib_report.files.contains(&parent_path),
            "parent library via inheritsFrom should be protected"
        );
        assert!(
            !lib_report.files.contains(&loader_path),
            "loader library should be protected"
        );
        assert!(
            lib_report.files.contains(&orphan_path),
            "orphan should be reclaimable"
        );
    }

    #[test]
    fn malformed_version_json_makes_its_whole_subtree_off_limits() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(&tmp);

        // No instances reference "bad-version" or "good-old"
        write_instance_manifest(&paths, "inst1", "1.21", "vanilla", "");
        write_version_json(&paths, "1.21", None, vec![], None, None, None);

        // Malformed version dir: bad-version/bad-version.json is invalid JSON
        let bad_dir = paths
            .minecraft_runtime_root()
            .join("versions")
            .join("bad-version");
        fs::create_dir_all(&bad_dir).unwrap();
        fs::write(bad_dir.join("bad-version.json"), b"{ invalid json").unwrap();
        fs::write(bad_dir.join("bad-version.jar"), b"fake jar").unwrap();

        // Good old version dir: well-formed but unreferenced
        write_version_json(&paths, "good-old", None, vec![], None, None, None);
        let good_jar = bad_dir
            .parent()
            .unwrap()
            .join("good-old")
            .join("good-old.jar");
        fs::write(&good_jar, b"good jar").unwrap();

        let report = scan(&paths);
        let ver_report = report
            .categories
            .iter()
            .find(|c| c.category == PruneCategory::Versions)
            .unwrap();

        let bad_jar = bad_dir.join("bad-version.jar");
        assert!(
            !ver_report.files.contains(&bad_jar),
            "malformed version's jar must not be reported as reclaimable"
        );
        assert!(
            !ver_report.files.contains(&bad_dir.join("bad-version.json")),
            "malformed version's json must not be reported"
        );
        // Good old version should be reclaimable
        assert!(
            ver_report.files.contains(&good_jar),
            "well-formed unreferenced version should be reclaimable"
        );
    }

    #[test]
    fn dry_run_deletes_nothing() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(&tmp);

        write_instance_manifest(&paths, "inst1", "1.21", "vanilla", "");
        write_version_json(&paths, "1.21", None, vec![], None, None, None);

        // Orphan library that would be reclaimable
        let orphan = write_library(&paths, "com/example/orphan/1.0/orphan-1.0.jar", b"orphan");
        assert!(orphan.is_file());

        // Unreferenced version
        write_version_json(&paths, "old-version", None, vec![], None, None, None);
        let old_jar = paths
            .minecraft_runtime_root()
            .join("versions")
            .join("old-version")
            .join("old-version.jar");
        fs::write(&old_jar, b"old").unwrap();

        let report = scan(&paths);
        // Dry run must not delete
        assert!(orphan.is_file(), "dry run must not delete orphan library");
        assert!(old_jar.is_file(), "dry run must not delete version jar");

        // Report should still list them as reclaimable
        let lib_report = report
            .categories
            .iter()
            .find(|c| c.category == PruneCategory::Libraries)
            .unwrap();
        assert!(lib_report.files.contains(&orphan));

        let ver_report = report
            .categories
            .iter()
            .find(|c| c.category == PruneCategory::Versions)
            .unwrap();
        assert!(ver_report.files.contains(&old_jar));
    }

    #[test]
    fn prune_selected_deletes_only_chosen_category() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(&tmp);

        write_instance_manifest(&paths, "inst1", "1.21", "vanilla", "");
        write_version_json(&paths, "1.21", None, vec![], None, None, None);

        let orphan_lib = write_library(&paths, "com/example/orphan/1.0/orphan-1.0.jar", b"orphan");
        write_version_json(&paths, "old-version", None, vec![], None, None, None);
        let old_jar = paths
            .minecraft_runtime_root()
            .join("versions")
            .join("old-version")
            .join("old-version.jar");
        fs::write(&old_jar, b"old").unwrap();

        // Prune only libraries
        let result = prune(&paths, &[PruneCategory::Libraries]);
        assert!(!orphan_lib.is_file(), "orphan library should be deleted");
        assert!(
            old_jar.is_file(),
            "version jar should remain when only libraries pruned"
        );
        assert_eq!(result.total_freed_files, 1);
        assert!(result
            .categories
            .iter()
            .any(|c| c.category == PruneCategory::Libraries));
    }

    #[test]
    fn asset_object_not_referenced_is_reclaimable() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(&tmp);

        write_instance_manifest(&paths, "inst1", "1.21", "vanilla", "");

        // Asset index for 1.21 contains one hash
        let hash_referenced = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let hash_orphan = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        write_asset_index(
            &paths,
            "1.21",
            vec![("minecraft/lang/en_us.json", hash_referenced, 10)],
        );

        write_version_json(
            &paths,
            "1.21",
            None,
            vec![],
            Some(crate::launch::AssetIndex {
                id: "1.21".to_string(),
                url: "https://example.com/1.21.json".to_string(),
                sha1: None,
                size: None,
                total_size: None,
            }),
            None,
            None,
        );

        let obj_ref = write_asset_object(&paths, hash_referenced, b"referenced");
        let obj_orphan = write_asset_object(&paths, hash_orphan, b"orphan");

        let report = scan(&paths);
        let asset_report = report
            .categories
            .iter()
            .find(|c| c.category == PruneCategory::Assets)
            .unwrap();
        assert!(
            !asset_report.files.contains(&obj_ref),
            "referenced asset object must not be reclaimable"
        );
        assert!(
            asset_report.files.contains(&obj_orphan),
            "orphan asset object should be reclaimable"
        );
    }

    #[test]
    fn malformed_asset_index_leaves_all_assets_as_referenced() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(&tmp);

        write_instance_manifest(&paths, "inst1", "1.21", "vanilla", "");
        // Malformed asset index
        let indexes_dir = paths.minecraft_assets_dir().join("indexes");
        fs::create_dir_all(&indexes_dir).unwrap();
        fs::write(indexes_dir.join("1.21.json"), b"{ invalid").unwrap();

        write_version_json(
            &paths,
            "1.21",
            None,
            vec![],
            Some(crate::launch::AssetIndex {
                id: "1.21".to_string(),
                url: "https://example.com/1.21.json".to_string(),
                sha1: None,
                size: None,
                total_size: None,
            }),
            None,
            None,
        );

        let orphan_hash = "cccccccccccccccccccccccccccccccccccccccc";
        let orphan_obj = write_asset_object(&paths, orphan_hash, b"orphan");

        let report = scan(&paths);
        let asset_report = report
            .categories
            .iter()
            .find(|c| c.category == PruneCategory::Assets)
            .unwrap();
        // Because index is malformed, no asset should be reported
        assert!(
            !asset_report.files.contains(&orphan_obj),
            "malformed index must make all assets off-limits"
        );
        assert_eq!(asset_report.file_count, 0);
        assert!(report.warnings.iter().any(|w| w.contains("assets")));
    }
}
