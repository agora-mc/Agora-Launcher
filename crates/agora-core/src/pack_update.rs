//! Drive the three-way pack merge planner ([`crate::pack_merge`]) from real
//! inputs and make it actually usable.
//!
//! `pack_merge` is a pure function over three inventories. It cannot be called
//! today because the pieces around it do not exist: nobody builds THEIRS from a
//! new pack version, nobody pulls BASE/OURS, and nobody records the new baseline
//! after a merge. This module is that missing glue, kept deliberately separate
//! from the single-artifact [`crate::install_pipeline`] (see the design notes in
//! the module docs and the answer to the "where should staging live" question).
//!
//! # What is here
//!
//! * [`theirs_from_mrpack`] — build a THEIRS inventory from a local `.mrpack`
//!   file **without any network access**. Override/config files are embedded in
//!   the archive and get byte-verified SHA-256s. Mod jars are not in the archive
//!   (an `.mrpack` index publishes only paths, SHA-1/SHA-512 and download URLs),
//!   so their SHA-256 is not known until the jar is fetched — they are emitted
//!   as *unverified* entries whose content identity is a strong placeholder, with
//!   the limitation surfaced explicitly. This is the honest answer to "can we
//!   plan without downloading every jar?": yes for config, estimated for mods.
//!
//! * [`preview_pack_update`] — the offline preview. Pulls BASE from the DB,
//!   OURS from the instance directory, THEIRS from the `.mrpack`, wires up the
//!   mod-id identity channel (using the Modrinth project id from the download
//!   URL, which is stable across jar renames), runs the planner, and annotates
//!   which decisions rest on byte-verified vs estimated mod content.
//!
//! * [`new_pack_baseline`] — the decision for what `instance_pack_files` becomes
//!   after a successful merge: **the new pack's contribution (THEIRS), not the
//!   post-merge on-disk state**. Recording the on-disk state would launder a
//!   user-resolved keep into the baseline and silently reclassify their edit as a
//!   pack original on the next update; recording THEIRS keeps the divergence
//!   visible. Pinned down by `baseline_records_theirs_not_ours`.
//!
//! * [`apply_merge`] — apply a plan against a **fully staged THEIRS tree** with
//!   a mandatory snapshot, rollback on failure, and a new-baseline write. The
//!   network part of staging lives behind the [`PackFileFetcher`] trait so tests
//!   stay offline.
//!
//! # What this pass deliberately does not do
//!
//! * It does not reconcile `instance_manifest.json` after a merge (the
//!   `InstalledMod` list stays as it was until a later reconcile step), and it
//!   does not run a post-apply health scan. File state and the `instance_pack_files`
//!   baseline are made correct; manifest reconciliation belongs beside the health
//!   scan and the existing install flow's manifest logic.
//! * [`apply_merge`] consumes a pre-staged tree; fetching jars from the network is
//!   provided by [`stage_pack`] behind [`PackFileFetcher`], not folded into apply.
//!
//! # The blocker (design answer)
//!
//! A byte-exact THEIRS needs every jar. But a *planable* THEIRS does not, and
//! the loss is confined to the mod list:
//!
//! * Override/config files (the prime target of user edits) are embedded in the
//!   archive, so every config decision is exact with zero downloads.
//! * For mods, the index gives path + SHA-1/SHA-512 + URL. The project id in the
//!   URL is a stable "same mod" identity, so **rename detection works without the
//!   jar** (the planner's mod-id channel, fed project ids on every side).
//!   Presence (add/remove) is exact. Whether a *particular mod's content* changed
//!   is not decidable from the index alone — we can only compare our local jar's
//!   SHA-512 against the index's SHA-512 (proving convergence, or its absence).
//!   Those are the entries marked unverified.
//!
//! The preview therefore reports: config changes exactly, mod add/remove/rename
//! exactly, and mod content changes as *estimated*, with the byte cost stated as
//! a file count (sizes are only present in the index when the author set
//! `fileSize`, which is not guaranteed).

use crate::db::{list_instance_pack_files, replace_instance_pack_files, InstancePackFile};
use crate::pack_merge::{plan_pack_update_with_mod_ids, PackMergePlan};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

// ---------------------------------------------------------------------------
// Disabled-suffix and pack-scope helpers (mirror pack_merge + pack_inventory)
// ---------------------------------------------------------------------------

const DISABLED_SUFFIX: &str = ".disabled";

fn logical_path(path: &str) -> &str {
    path.strip_suffix(DISABLED_SUFFIX).unwrap_or(path)
}

/// Whether `rel` is a path a pack can contribute, matching
/// [`crate::pack_inventory::collect_pack_inventory`]'s scope so BASE/OURS/THEIRS
/// stay aligned.
fn in_pack_scope(rel: &str) -> bool {
    if rel == "options.txt" {
        return true;
    }
    if rel.starts_with("mods/") {
        return true;
    }
    crate::import::ALLOWED_OVERRIDE_PREFIXES
        .iter()
        .any(|prefix| rel.starts_with(prefix))
}

/// Reject paths that escape `base`, mirroring `import::assert_safe_path`.
fn safe_join(base: &Path, rel: &str) -> Result<PathBuf, String> {
    let rel = rel.replace('\\', "/");
    let candidate = Path::new(&rel);
    if candidate.is_absolute() {
        return Err(format!("absolute path is not allowed: {rel}"));
    }
    for component in candidate.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!("unsafe path traversal: {rel}"));
            }
        }
    }
    Ok(base.join(rel))
}

// ---------------------------------------------------------------------------
// .mrpack index parsing
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct MrpackIndex {
    #[serde(default)]
    name: String,
    #[serde(default, alias = "versionId")]
    version_id: Option<String>,
    #[serde(default)]
    files: Vec<MrpackFile>,
    #[serde(default)]
    overrides: String,
}

#[derive(Deserialize)]
struct MrpackFile {
    path: String,
    #[serde(default)]
    downloads: Vec<String>,
    #[serde(default)]
    hashes: BTreeMap<String, String>,
    #[serde(default)]
    file_size: Option<u64>,
}

/// The digest algorithm a file's index hash actually uses (the mrpack format
/// carries SHA-1 and/or SHA-512; never SHA-256 for downloadables).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DigestAlgo {
    Sha1,
    Sha512,
}

impl DigestAlgo {
    fn hash(self, bytes: &[u8]) -> String {
        use sha2::Digest as _;
        match self {
            DigestAlgo::Sha1 => {
                let mut h = sha1::Sha1::new();
                h.update(bytes);
                hex::encode(h.finalize())
            }
            DigestAlgo::Sha512 => {
                let mut h = sha2::Sha512::new();
                h.update(bytes);
                hex::encode(h.finalize())
            }
        }
    }
}

fn pick_digest(file: &MrpackFile) -> Option<(DigestAlgo, String)> {
    file.hashes
        .get("sha512")
        .map(|h| (DigestAlgo::Sha512, h.clone()))
        .or_else(|| {
            file.hashes
                .get("sha1")
                .map(|h| (DigestAlgo::Sha1, h.clone()))
        })
}

fn verify_bytes(file: &MrpackFile, bytes: &[u8]) -> Result<(), String> {
    let mut checked = false;
    if let Some(expected) = file.hashes.get("sha1") {
        let actual = DigestAlgo::Sha1.hash(bytes);
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(format!(
                "sha1 mismatch for {}: expected {expected} got {actual}",
                file.path
            ));
        }
        checked = true;
    }
    if let Some(expected) = file.hashes.get("sha512") {
        let actual = DigestAlgo::Sha512.hash(bytes);
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(format!(
                "sha512 mismatch for {}: expected {expected} got {actual}",
                file.path
            ));
        }
        checked = true;
    }
    if !checked {
        return Err(format!("pack entry {} has no integrity hash", file.path));
    }
    Ok(())
}

/// The Modrinth project id from a canonical CDN download URL, if any. Used as the
/// stable "same mod" identity for the planner's mod-id channel without parsing
/// the jar (which would require downloading it).
fn project_id_from_url(raw: &str) -> Option<String> {
    let url = reqwest::Url::parse(raw).ok()?;
    if url.scheme() != "https"
        || url.host_str() != Some("cdn.modrinth.com")
        || url.port_or_known_default() != Some(443)
    {
        return None;
    }
    let segments: Vec<_> = url.path_segments()?.collect();
    if segments.len() < 5 || segments[0] != "data" || segments[2] != "versions" {
        return None;
    }
    let project_id = segments[1];
    if project_id.is_empty()
        || !project_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return None;
    }
    Some(project_id.to_string())
}

/// A THEIRS inventory plus the metadata needed to state its limits honestly.
#[derive(Debug, Clone, Default)]
pub struct MrpackTheirs {
    /// Full THEIRS inventory (`relative_path`, `sha256`, `size`). For verified
    /// entries `sha256` is a real file hash. For unverified mods it is a strong
    /// placeholder derived from the index hash (never equal to any real SHA-256),
    /// so the planner treats the content as "changed" until the jar is fetched.
    pub files: Vec<InstancePackFile>,
    /// Logical paths whose content decision is an *estimate* (jar not fetched).
    pub unverified: BTreeSet<String>,
    /// Logical paths (from `unverified`) confirmed byte-identical to a local file
    /// during the preview — these need no download to be applied.
    pub converged: BTreeSet<String>,
    /// `logical path -> (digest algo, hex)` for provisional entries, used for
    /// convergence detection. Module-private; the preview driver consumes it.
    provisional_hash: BTreeMap<String, (DigestAlgo, String)>,
    /// `logical path -> download URL` for files that would need fetching.
    pub download_urls: BTreeMap<String, String>,
    /// `logical path -> modrinth project id` for the planner's mod-id channel.
    pub mod_ids: HashMap<String, String>,
    /// Number of files that are neither verified nor converged (need a download).
    pub files_needing_download: usize,
    /// Best-effort byte cost of those files (0 for entries without `fileSize`).
    pub download_bytes: u64,
    /// How many of `files_needing_download` have no known size.
    pub size_unknown_count: usize,
    pub pack_name: String,
    pub pack_version_id: Option<String>,
}

/// Build a THEIRS inventory from a local `.mrpack` **with no network access**.
///
/// Override/config files embedded in the archive are hashed and byte-verified.
/// Index-listed files that are embedded in the archive are likewise verified.
/// Index-listed files that are not embedded (the mod jars) become unverified
/// provisional entries.
pub fn theirs_from_mrpack(mrpack_path: &Path) -> Result<MrpackTheirs, String> {
    let file = fs::File::open(mrpack_path)
        .map_err(|e| format!("cannot open mrpack {}: {e}", mrpack_path.display()))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("invalid mrpack zip: {e}"))?;

    let index = {
        let mut entry = archive
            .by_name("modrinth.index.json")
            .map_err(|_| "missing modrinth.index.json".to_string())?;
        let mut text = String::new();
        entry
            .read_to_string(&mut text)
            .map_err(|e| format!("cannot read modrinth.index.json: {e}"))?;
        serde_json::from_str::<MrpackIndex>(&text)
            .map_err(|e| format!("invalid modrinth.index.json: {e}"))?
    };

    let mut files: BTreeMap<String, InstancePackFile> = BTreeMap::new();
    let mut unverified: BTreeSet<String> = BTreeSet::new();
    let mut provisional_hash: BTreeMap<String, (DigestAlgo, String)> = BTreeMap::new();
    let mut download_urls: BTreeMap<String, String> = BTreeMap::new();
    let mut mod_ids: HashMap<String, String> = HashMap::new();
    let mut files_needing_download = 0usize;
    let mut download_bytes = 0u64;
    let mut size_unknown_count = 0usize;

    // Index-listed files (mods and other downloadable content).
    for file_entry in &index.files {
        let rel = file_entry.path.replace('\\', "/");
        if !in_pack_scope(&rel) {
            continue;
        }
        let logical = logical_path(&rel).to_string();

        // Embedded fallback: some packs ship the file inside the archive.
        let mut embedded = None;
        if let Ok(mut entry) = archive.by_name(&rel) {
            let mut bytes = Vec::new();
            entry
                .read_to_end(&mut bytes)
                .map_err(|e| format!("cannot read embedded {}: {e}", file_entry.path))?;
            embedded = Some(bytes);
        }

        if let Some(bytes) = embedded {
            verify_bytes(file_entry, &bytes)?;
            let sha256 = sha2_256_hex(&bytes);
            files.insert(
                logical.clone(),
                InstancePackFile {
                    relative_path: rel.clone(),
                    sha256,
                    size: bytes.len() as u64,
                },
            );
        } else {
            let Some((algo, digest)) = pick_digest(file_entry) else {
                return Err(format!(
                    "pack entry {} has no usable hash (needs sha1 or sha512)",
                    file_entry.path
                ));
            };
            // A strong placeholder that can never collide with a real SHA-256 of
            // base or ours, so the planner treats the mod as content-changed.
            let placeholder = format!("{algo:?}:{digest}");
            let size = file_entry.file_size.unwrap_or(0);
            if file_entry.file_size.is_none() {
                size_unknown_count += 1;
            }
            let url = file_entry.downloads.first().cloned();
            if let Some(url) = &url {
                download_urls.insert(logical.clone(), url.clone());
                if let Some(pid) = project_id_from_url(url) {
                    mod_ids.insert(logical.clone(), pid);
                }
            }
            files.insert(
                logical.clone(),
                InstancePackFile {
                    relative_path: rel.clone(),
                    sha256: placeholder,
                    size,
                },
            );
            unverified.insert(logical.clone());
            provisional_hash.insert(logical.clone(), (algo, digest));
            files_needing_download += 1;
            download_bytes = download_bytes.saturating_add(size);
        }
    }

    // Override tree embedded in the archive.
    for i in 0..archive.len() {
        let mut entry = match archive.by_index(i) {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.is_dir() {
            continue;
        }
        let entry_name = entry.name().replace('\\', "/");
        let Some(rel) = mrpack_override_relative(&entry_name, &index.overrides) else {
            continue;
        };
        if !in_pack_scope(&rel) {
            continue;
        }
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|e| format!("cannot read override {entry_name}: {e}"))?;
        let sha256 = sha2_256_hex(&bytes);
        files.insert(
            rel.clone(),
            InstancePackFile {
                relative_path: rel.clone(),
                sha256,
                size: bytes.len() as u64,
            },
        );
    }

    let mut files: Vec<InstancePackFile> = files.into_values().collect();
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

    Ok(MrpackTheirs {
        files,
        unverified,
        converged: BTreeSet::new(),
        provisional_hash,
        download_urls,
        mod_ids,
        files_needing_download,
        download_bytes,
        size_unknown_count,
        pack_name: index.name,
        pack_version_id: index.version_id,
    })
}

/// Resolve an archive entry name to an in-scope override path, mirroring
/// `import::mrpack_override_path` (standard override prefixes + custom prefix,
/// restricted to the allowed override prefixes).
fn mrpack_override_relative(entry_name: &str, custom_prefix: &str) -> Option<String> {
    let standard = ["overrides/", "client-overrides/", "client_overrides/"];
    let relative = standard
        .iter()
        .find_map(|prefix| entry_name.strip_prefix(prefix))
        .or_else(|| {
            let prefix = custom_prefix.trim_matches('/');
            (!prefix.is_empty())
                .then(|| entry_name.strip_prefix(&format!("{prefix}/")))
                .flatten()
        })?;
    if relative.is_empty() {
        return None;
    }
    let relative = relative.to_string();
    if in_pack_scope(&relative) {
        Some(relative)
    } else {
        None
    }
}

fn sha2_256_hex(data: &[u8]) -> String {
    use sha2::Digest as _;
    let mut h = sha2::Sha256::new();
    h.update(data);
    hex::encode(h.finalize())
}

// ---------------------------------------------------------------------------
// Preview
// ---------------------------------------------------------------------------

/// The honest, offline preview of what a pack update would do.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackUpdatePreview {
    pub plan: PackMergePlan,
    /// Logical paths whose mod-content decision is an estimate (jar not fetched).
    pub unverified: BTreeSet<String>,
    /// Unverified paths confirmed byte-identical to a local file.
    pub converged: BTreeSet<String>,
    pub files_needing_download: usize,
    pub download_bytes: u64,
    pub size_unknown_count: usize,
    pub pack_name: String,
    pub pack_version_id: Option<String>,
}

impl PackUpdatePreview {
    /// Number of files whose *content* decision is uncertain because the new jar
    /// has not been fetched. Config/override decisions are never in here.
    /// Converged entries (already byte-identical locally) are excluded.
    pub fn content_uncertain(&self) -> usize {
        self.unverified.len()
    }
}

/// Produce an offline preview of updating `instance_id` to the pack in
/// `mrpack_path`. No network is used; mod-jar content decisions are estimated and
/// flagged via [`PackUpdatePreview::unverified`].
pub fn preview_pack_update(
    conn: &rusqlite::Connection,
    instance_id: &str,
    instance_dir: &Path,
    mrpack_path: &Path,
) -> Result<PackUpdatePreview, String> {
    let base = list_instance_pack_files(conn, instance_id)
        .map_err(|e| format!("cannot read instance pack inventory: {e}"))?;
    let ours = crate::pack_inventory::collect_pack_inventory(instance_dir)?;
    let mut theirs_side = theirs_from_mrpack(mrpack_path)?;

    // Convergence: if a local file is byte-identical to the new pack's jar, we
    // need not download it and the content decision is exact.
    let ours_by_logical: BTreeMap<String, &InstancePackFile> = ours
        .iter()
        .map(|f| (logical_path(&f.relative_path).to_string(), f))
        .collect();
    let converged_paths: Vec<String> = theirs_side
        .unverified
        .iter()
        .filter_map(|logical| {
            let ours_file = ours_by_logical.get(logical)?;
            let (algo, expected) = theirs_side.provisional_hash.get(logical)?;
            let actual = algo.hash(&read_all(instance_dir.join(&ours_file.relative_path)).ok()?);
            actual
                .eq_ignore_ascii_case(expected)
                .then_some(logical.clone())
        })
        .collect();
    for logical in &converged_paths {
        // Refine the THEIRS entry to a real SHA-256 so the planner sees Keep /
        // converged rather than an estimate.
        if let Some(ours_file) = ours_by_logical.get(logical) {
            if let Some(entry) = theirs_side
                .files
                .iter_mut()
                .find(|f| logical_path(&f.relative_path) == logical.as_str())
            {
                entry.sha256 = ours_file.sha256.clone();
                entry.size = ours_file.size;
            }
        }
        theirs_side.unverified.remove(logical);
        theirs_side.converged.insert(logical.clone());
        theirs_side.files_needing_download = theirs_side.files_needing_download.saturating_sub(1);
    }

    let manifest = crate::helpers::read_manifest(&instance_dir.join("instance_manifest.json"))
        .map_err(|e| format!("cannot read instance manifest: {e}"))?;

    // Mod-id identity channel. THEIRS uses the Modrinth project id from the CDN
    // URL; BASE/OURS use the project id the import recorded in the manifest. This
    // makes rename detection work without parsing the new jars. Paths without an
    // id (user-added mods, non-CDN jars) fall back to path equality inside the
    // planner, which is the documented degradation.
    let base_ids = manifest_mod_ids(&manifest);
    let ours_ids = manifest_mod_ids(&manifest);

    let plan = plan_pack_update_with_mod_ids(
        &base,
        &theirs_side.files,
        &ours,
        Some(&base_ids),
        Some(&theirs_side.mod_ids),
        Some(&ours_ids),
    );

    Ok(PackUpdatePreview {
        plan,
        unverified: theirs_side.unverified.clone(),
        converged: theirs_side.converged,
        files_needing_download: theirs_side.files_needing_download,
        download_bytes: theirs_side.download_bytes,
        size_unknown_count: theirs_side.size_unknown_count,
        pack_name: theirs_side.pack_name,
        pack_version_id: theirs_side.pack_version_id,
    })
}

/// Build `logical path -> modrinth project id` for the planner's mod-id channel,
/// from the manifest's recorded mod identities.
fn manifest_mod_ids(manifest: &crate::models::InstanceManifest) -> HashMap<String, String> {
    let mut ids = HashMap::new();
    for m in manifest
        .mods
        .iter()
        .chain(manifest.resourcepacks.iter())
        .chain(manifest.shaders.iter())
        .chain(manifest.datapacks.iter())
    {
        if let Some(pid) = m
            .modrinth_id
            .as_deref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            let filename = m.filename.trim();
            let logical = format!("mods/{}", logical_path(filename));
            ids.insert(logical, pid.to_string());
        }
    }
    ids
}

fn read_all(path: PathBuf) -> std::io::Result<Vec<u8>> {
    fs::read(path)
}

// ---------------------------------------------------------------------------
// New baseline decision (Q3)
// ---------------------------------------------------------------------------

/// What `instance_pack_files` becomes after a successful merge: **the new pack's
/// contribution (THEIRS), not the post-merge on-disk state**.
///
/// Why not the on-disk state? The baseline answers "what would the pack give me
/// fresh", and every merge decision derives from comparing the user's current
/// file against it. If the user resolved a conflict by keeping their own file,
/// the on-disk content differs from the pack's. Recording the on-disk content as
/// the baseline would make `base.sha == ours.sha` on the next update, so a later
/// pack change to that file would read as "unchanged locally, take theirs" — the
/// user's edit silently reclassified as a pack original. Recording THEIRS keeps
/// `base.sha != ours.sha`, so the divergence stays visible and any future pack
/// change re-raises the conflict instead of clobbering the edit.
///
/// This is intentionally just `theirs` (sorted/deduped): after a fully staged
/// apply the staged tree is byte-authoritative, so [`apply_merge`] writes
/// `collect_pack_inventory(staged_dir)` which is exactly THEIRS with real hashes.
pub fn new_pack_baseline(theirs: &[InstancePackFile]) -> Vec<InstancePackFile> {
    let mut out = theirs.to_vec();
    out.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    out.dedup_by(|a, b| a.relative_path == b.relative_path);
    out
}

// ---------------------------------------------------------------------------
// Staged apply + rollback
// ---------------------------------------------------------------------------

/// How a *conflict* (which the planner never auto-resolves) is settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictResolution {
    /// Keep the user's current on-disk file (no change for this key).
    KeepOurs,
    /// Adopt the new pack's file for this key.
    TakeTheirs,
}

/// Result of applying a merge, mirroring the honesty of the install pipeline's
/// outcome but scoped to what this module owns.
#[derive(Debug)]
pub struct MergeOutcome {
    /// Snapshot taken before any mutation; restorable if the caller needs to.
    pub snapshot_id: String,
    /// Whether a failure occurred and the pre-merge snapshot was restored.
    pub rollback_performed: bool,
}

// ---------------------------------------------------------------------------
// Manifest reconciliation - projects the plan onto the old manifest without
// scanning the live instance directory. See module docs for design.
// ---------------------------------------------------------------------------

fn content_type_for_logical(logical: &str) -> Option<&'static str> {
    if logical.starts_with("mods/") {
        Some("mod")
    } else if logical.starts_with("resourcepacks/") {
        Some("resourcepack")
    } else if logical.starts_with("shaderpacks/") {
        Some("shader")
    } else if logical.starts_with("datapacks/") {
        Some("datapack")
    } else if logical.starts_with("saves/") {
        Some("world")
    } else {
        None
    }
}

fn filename_from_path(path: &str) -> String {
    let stripped = path.strip_suffix(DISABLED_SUFFIX).unwrap_or(path);
    stripped.rsplit('/').next().unwrap_or(stripped).to_string()
}

fn hash_staged_file(staged_dir: &Path, logical: &str) -> Result<String, String> {
    let staged_path = safe_join(staged_dir, logical)?;
    if !staged_path.is_file() {
        return Err(format!(
            "staged file missing for '{}': {}",
            logical, logical
        ));
    }
    let bytes = fs::read(&staged_path)
        .map_err(|e| format!("cannot read staged {}: {e}", staged_path.display()))?;
    Ok(sha2_256_hex(&bytes))
}

fn matches_mod_id(entry: &crate::models::InstalledMod, mod_id: &str) -> bool {
    entry
        .modrinth_id
        .as_deref()
        .map(|s| s.eq_ignore_ascii_case(mod_id))
        .unwrap_or(false)
        || entry
            .mod_jar_id
            .as_deref()
            .map(|s| s.eq_ignore_ascii_case(mod_id))
            .unwrap_or(false)
}

/// Every manifest content entry, in one mutable pass.
///
/// Mirrors `loadout::all_content_entries_mut`. Chaining the arrays in a single
/// expression is what makes the disjoint field borrows legal — an earlier
/// version of these helpers reached for raw pointers to "avoid borrow checker
/// disjoint field issues", which was both unnecessary and unsound: it handed
/// out `&mut` derived from a pointer that was then re-borrowed while those
/// references were still live.
fn all_entries_mut(
    manifest: &mut crate::models::InstanceManifest,
) -> impl Iterator<Item = &mut crate::models::InstalledMod> {
    manifest
        .mods
        .iter_mut()
        .chain(manifest.resourcepacks.iter_mut())
        .chain(manifest.shaders.iter_mut())
        .chain(manifest.datapacks.iter_mut())
        .chain(manifest.worlds.iter_mut())
}

/// Whether an entry belongs to the bucket named by `ct`.
///
/// `None` means "any bucket"; the caller only narrows when the plan told it
/// which content type the path belongs to.
fn entry_in_bucket(entry: &crate::models::InstalledMod, ct: Option<&str>) -> bool {
    // `content_type_for_logical` yields exactly the strings the manifest stores
    // in `InstalledMod.content_type`, so a direct comparison is the same test
    // the array-per-bucket version made.
    match ct {
        None => true,
        Some(want) => entry.content_type == want,
    }
}

fn find_by_filename_mut<'a>(
    manifest: &'a mut crate::models::InstanceManifest,
    filename: &str,
    ct: Option<&str>,
) -> Option<&'a mut crate::models::InstalledMod> {
    all_entries_mut(manifest).find(|entry| entry.filename == filename && entry_in_bucket(entry, ct))
}

/// Locate an entry by mod id, falling back to filename.
///
/// Resolved in one pass rather than two sequential `find`s, because two
/// mutable searches over the same manifest cannot both be live — which is the
/// borrow conflict the raw-pointer version was papering over.
fn find_entry_mut<'a>(
    manifest: &'a mut crate::models::InstanceManifest,
    mod_id: Option<&str>,
    filename: &str,
    ct: Option<&str>,
) -> Option<&'a mut crate::models::InstalledMod> {
    // Decide *which* entry to return using immutable borrows, then take the
    // single mutable borrow the caller asked for.
    let by_mod_id = mod_id.and_then(|wanted| {
        manifest
            .mods
            .iter()
            .chain(manifest.resourcepacks.iter())
            .chain(manifest.shaders.iter())
            .chain(manifest.datapacks.iter())
            .chain(manifest.worlds.iter())
            .position(|entry| matches_mod_id(entry, wanted))
    });
    match by_mod_id {
        Some(index) => all_entries_mut(manifest).nth(index),
        None => find_by_filename_mut(manifest, filename, ct),
    }
}

fn remove_by_mod_id(manifest: &mut crate::models::InstanceManifest, mod_id: &str) -> bool {
    if let Some(pos) = manifest.mods.iter().position(|e| matches_mod_id(e, mod_id)) {
        manifest.mods.remove(pos);
        return true;
    }
    if let Some(pos) = manifest
        .resourcepacks
        .iter()
        .position(|e| matches_mod_id(e, mod_id))
    {
        manifest.resourcepacks.remove(pos);
        return true;
    }
    if let Some(pos) = manifest
        .shaders
        .iter()
        .position(|e| matches_mod_id(e, mod_id))
    {
        manifest.shaders.remove(pos);
        return true;
    }
    if let Some(pos) = manifest
        .datapacks
        .iter()
        .position(|e| matches_mod_id(e, mod_id))
    {
        manifest.datapacks.remove(pos);
        return true;
    }
    if let Some(pos) = manifest
        .worlds
        .iter()
        .position(|e| matches_mod_id(e, mod_id))
    {
        manifest.worlds.remove(pos);
        return true;
    }
    false
}

fn remove_by_filename(
    manifest: &mut crate::models::InstanceManifest,
    filename: &str,
    ct: Option<&str>,
) -> bool {
    if let Some(ct_val) = ct {
        let removed = match ct_val {
            "resourcepack" => {
                if let Some(pos) = manifest
                    .resourcepacks
                    .iter()
                    .position(|e| e.filename == filename)
                {
                    manifest.resourcepacks.remove(pos);
                    true
                } else {
                    false
                }
            }
            "shader" => {
                if let Some(pos) = manifest.shaders.iter().position(|e| e.filename == filename) {
                    manifest.shaders.remove(pos);
                    true
                } else {
                    false
                }
            }
            "datapack" => {
                if let Some(pos) = manifest
                    .datapacks
                    .iter()
                    .position(|e| e.filename == filename)
                {
                    manifest.datapacks.remove(pos);
                    true
                } else {
                    false
                }
            }
            "world" => {
                if let Some(pos) = manifest.worlds.iter().position(|e| e.filename == filename) {
                    manifest.worlds.remove(pos);
                    true
                } else {
                    false
                }
            }
            _ => {
                if let Some(pos) = manifest.mods.iter().position(|e| e.filename == filename) {
                    manifest.mods.remove(pos);
                    true
                } else {
                    false
                }
            }
        };
        if removed {
            return true;
        }
    }
    if let Some(pos) = manifest.mods.iter().position(|e| e.filename == filename) {
        manifest.mods.remove(pos);
        return true;
    }
    if let Some(pos) = manifest
        .resourcepacks
        .iter()
        .position(|e| e.filename == filename)
    {
        manifest.resourcepacks.remove(pos);
        return true;
    }
    if let Some(pos) = manifest.shaders.iter().position(|e| e.filename == filename) {
        manifest.shaders.remove(pos);
        return true;
    }
    if let Some(pos) = manifest
        .datapacks
        .iter()
        .position(|e| e.filename == filename)
    {
        manifest.datapacks.remove(pos);
        return true;
    }
    if let Some(pos) = manifest.worlds.iter().position(|e| e.filename == filename) {
        manifest.worlds.remove(pos);
        return true;
    }
    false
}

fn manifest_contains_filename(manifest: &crate::models::InstanceManifest, filename: &str) -> bool {
    manifest.mods.iter().any(|e| e.filename == filename)
        || manifest
            .resourcepacks
            .iter()
            .any(|e| e.filename == filename)
        || manifest.shaders.iter().any(|e| e.filename == filename)
        || manifest.datapacks.iter().any(|e| e.filename == filename)
        || manifest.worlds.iter().any(|e| e.filename == filename)
}

fn manifest_contains_mod_id(manifest: &crate::models::InstanceManifest, mod_id: &str) -> bool {
    manifest.mods.iter().any(|e| matches_mod_id(e, mod_id))
        || manifest
            .resourcepacks
            .iter()
            .any(|e| matches_mod_id(e, mod_id))
        || manifest.shaders.iter().any(|e| matches_mod_id(e, mod_id))
        || manifest.datapacks.iter().any(|e| matches_mod_id(e, mod_id))
        || manifest.worlds.iter().any(|e| matches_mod_id(e, mod_id))
}

/// Reconcile `instance_manifest.json` after a pack merge by projecting the
/// `PackMergePlan` onto the `old` manifest. Pure: never scans the live
/// instance directory, may read staged jars for metadata.
///
/// - `Keep` / `KeepUserAdded` -> carry old entry untouched
/// - `Update` / `UpdateKeepDisabled` / `RenameUpdate` / `RenameUpdateKeepDisabled` -> update filename/sha256/enabled
/// - `Add` -> new pack-managed entry, `modrinth_id` from `theirs.mod_ids`, jar metadata parsed from staged file
/// - `Remove` -> drop entry
/// - Conflicts: `KeepOurs` -> untouched, `TakeTheirs` -> as Update/Add
///
/// Config/override paths (non-tracked content types) are skipped entirely.
/// Build the manifest entry for a file the pack ships.
///
/// `version` is deliberately `None`: an `.mrpack` index carries no per-file
/// version string, and inferring one from the filename would be fabricating
/// data the pack never stated.
#[allow(clippy::too_many_arguments)]
fn build_pack_entry(
    old: &crate::models::InstanceManifest,
    logical_path: &str,
    filename: String,
    sha256: String,
    enabled: bool,
    ct: &str,
    theirs: &MrpackTheirs,
    staged_dir: &Path,
) -> Result<crate::models::InstalledMod, String> {
    let modrinth_id = theirs.mod_ids.get(logical_path).cloned();
    let source_url = theirs.download_urls.get(logical_path).cloned();
    let staged_path = safe_join(staged_dir, logical_path)?;
    let jar = if ct == "mod" && staged_path.is_file() {
        crate::jar_metadata::parse_jar_metadata_for_loader(&staged_path, &old.loader)
    } else {
        crate::dependency_ops::JarDeps::default()
    };
    let mod_jar_id = jar.mod_jar_id.clone().or_else(|| modrinth_id.clone());
    Ok(crate::models::InstalledMod {
        filename,
        registry_id: None,
        modrinth_id,
        source: "modrinth-pack".to_string(),
        source_url,
        version: None,
        sha256,
        installed_at: chrono::Utc::now().to_rfc3339(),
        java_packages: jar.java_packages,
        mod_jar_id,
        provided_mod_ids: jar
            .provided_mods
            .iter()
            .map(|pm| pm.mod_id.clone())
            .collect(),
        enabled,
        content_type: ct.to_string(),
        update_pinned: false,
        pack_managed: true,
        installed_as_dependency: false,
        depends_on: jar.depends_on,
        optional_deps: jar.optional_deps,
        incompatible_deps: jar.incompatible_deps,
    })
}

/// Insert a pack entry into the array its content type belongs to, replacing
/// any entry that already claims the same filename.
fn push_pack_entry(
    manifest: &mut crate::models::InstanceManifest,
    ct: &str,
    installed: crate::models::InstalledMod,
) {
    let arr: &mut Vec<crate::models::InstalledMod> = match ct {
        "resourcepack" => &mut manifest.resourcepacks,
        "shader" => &mut manifest.shaders,
        "datapack" => &mut manifest.datapacks,
        "world" => &mut manifest.worlds,
        _ => &mut manifest.mods,
    };
    if let Some(pos) = arr.iter().position(|e| e.filename == installed.filename) {
        arr.remove(pos);
    }
    arr.push(installed);
}

pub fn reconcile_manifest(
    old: &crate::models::InstanceManifest,
    plan: &PackMergePlan,
    resolutions: &BTreeMap<String, ConflictResolution>,
    theirs: &MrpackTheirs,
    staged_dir: &Path,
) -> Result<crate::models::InstanceManifest, String> {
    let mut new = old.clone();

    // ---- Non-conflicting actions ----
    for action in &plan.actions {
        let ct_opt = content_type_for_logical(&action.logical_path);
        if ct_opt.is_none() {
            continue;
        }
        let ct = ct_opt.unwrap();
        match action.kind {
            crate::pack_merge::PlanActionKind::Keep
            | crate::pack_merge::PlanActionKind::KeepUserAdded => {}
            crate::pack_merge::PlanActionKind::Remove => {
                if let Some(mod_id) = &action.mod_id {
                    if !remove_by_mod_id(&mut new, mod_id) {
                        let fname = filename_from_path(&action.target_path);
                        remove_by_filename(&mut new, &fname, Some(ct));
                    }
                } else {
                    let fname = filename_from_path(&action.target_path);
                    remove_by_filename(&mut new, &fname, Some(ct));
                }
            }
            crate::pack_merge::PlanActionKind::Update
            | crate::pack_merge::PlanActionKind::UpdateKeepDisabled
            | crate::pack_merge::PlanActionKind::RenameUpdate
            | crate::pack_merge::PlanActionKind::RenameUpdateKeepDisabled => {
                let lookup_fname = if let Some(prev) = &action.previous_path {
                    filename_from_path(prev)
                } else {
                    filename_from_path(&action.logical_path)
                };
                let new_filename = filename_from_path(&action.target_path);
                let new_sha256 = hash_staged_file(staged_dir, &action.logical_path)?;
                match find_entry_mut(&mut new, action.mod_id.as_deref(), &lookup_fname, Some(ct)) {
                    Some(entry) => {
                        entry.filename = new_filename;
                        entry.sha256 = new_sha256;
                        entry.enabled = action.enabled;
                        if entry.content_type != ct {
                            entry.content_type = ct.to_string();
                        }
                    }
                    None => {
                        // The manifest has no entry for a file the pack
                        // demonstrably manages — drift that predates this
                        // merge. Treat it as an add rather than refusing:
                        // erroring would make a recoverable inconsistency
                        // permanently block every future pack update, and the
                        // provenance we stamp (pack_managed, from THEIRS) is
                        // exactly what the add path would have written anyway.
                        let installed = build_pack_entry(
                            old,
                            action.logical_path.as_str(),
                            new_filename,
                            new_sha256,
                            action.enabled,
                            ct,
                            theirs,
                            staged_dir,
                        )?;
                        push_pack_entry(&mut new, ct, installed);
                    }
                }
            }
            crate::pack_merge::PlanActionKind::Add => {
                let new_filename = filename_from_path(&action.target_path);
                let new_sha256 = hash_staged_file(staged_dir, &action.logical_path)?;
                let installed = build_pack_entry(
                    old,
                    action.logical_path.as_str(),
                    new_filename,
                    new_sha256,
                    action.enabled,
                    ct,
                    theirs,
                    staged_dir,
                )?;
                push_pack_entry(&mut new, ct, installed);
            }
        }
    }

    // ---- Conflicts ----
    for conflict in &plan.conflicts {
        let resolution = resolutions.get(&conflict.key);
        let resolution = match resolution {
            Some(r) => r,
            None => {
                return Err(format!(
                    "conflict '{}' is unresolved; provide a resolution before reconciling",
                    conflict.key
                ))
            }
        };
        match resolution {
            ConflictResolution::KeepOurs => {
                let logical = conflict.logical_path.as_str();
                let theirs_opt = conflict.theirs_path.as_deref();
                let is_tracked = content_type_for_logical(logical).is_some()
                    || theirs_opt
                        .map(|p| content_type_for_logical(p).is_some())
                        .unwrap_or(false);
                if !is_tracked {
                    continue;
                }
            }
            ConflictResolution::TakeTheirs => {
                let theirs_path_str = conflict
                    .theirs_path
                    .as_deref()
                    .unwrap_or(&conflict.logical_path);
                let ct_opt = content_type_for_logical(theirs_path_str);
                if ct_opt.is_none() {
                    continue;
                }
                let ct = ct_opt.unwrap();
                if conflict.theirs_path.is_none() {
                    let fname = filename_from_path(&conflict.logical_path);
                    if let Some(mod_id) = &conflict.mod_id {
                        if !remove_by_mod_id(&mut new, mod_id) {
                            remove_by_filename(&mut new, &fname, Some(ct));
                        }
                    } else {
                        remove_by_filename(&mut new, &fname, Some(ct));
                    }
                    continue;
                }

                let lookup_fname = if let Some(ours) = &conflict.ours_path {
                    filename_from_path(ours)
                } else if let Some(base) = &conflict.base_path {
                    filename_from_path(base)
                } else {
                    filename_from_path(&conflict.logical_path)
                };

                let exists = if let Some(mod_id) = &conflict.mod_id {
                    manifest_contains_mod_id(&new, mod_id)
                        || manifest_contains_filename(&new, &lookup_fname)
                } else {
                    manifest_contains_filename(&new, &lookup_fname)
                };

                if exists {
                    let entry = find_entry_mut(
                        &mut new,
                        conflict.mod_id.as_deref(),
                        &lookup_fname,
                        Some(ct),
                    )
                    .ok_or_else(|| {
                        format!(
                            "cannot find manifest entry for conflict TakeTheirs '{}'",
                            conflict.key
                        )
                    })?;
                    let new_filename = filename_from_path(theirs_path_str);
                    let new_sha256 = hash_staged_file(staged_dir, theirs_path_str)?;
                    entry.filename = new_filename;
                    entry.sha256 = new_sha256;
                    entry.enabled = !theirs_path_str.ends_with(DISABLED_SUFFIX);
                    if entry.content_type != ct {
                        entry.content_type = ct.to_string();
                    }
                } else {
                    let new_filename = filename_from_path(theirs_path_str);
                    let new_sha256 = hash_staged_file(staged_dir, theirs_path_str)?;
                    let modrinth_id = theirs
                        .mod_ids
                        .get(theirs_path_str)
                        .cloned()
                        .or_else(|| conflict.mod_id.clone());
                    let source_url = theirs.download_urls.get(theirs_path_str).cloned();
                    let staged_path = safe_join(staged_dir, theirs_path_str)?;
                    let jar = if ct == "mod" {
                        if staged_path.is_file() {
                            crate::jar_metadata::parse_jar_metadata_for_loader(
                                &staged_path,
                                &old.loader,
                            )
                        } else {
                            crate::dependency_ops::JarDeps::default()
                        }
                    } else {
                        crate::dependency_ops::JarDeps::default()
                    };
                    let mod_jar_id = jar.mod_jar_id.clone().or_else(|| modrinth_id.clone());
                    let installed = crate::models::InstalledMod {
                        filename: new_filename,
                        registry_id: None,
                        modrinth_id,
                        source: "modrinth-pack".to_string(),
                        source_url,
                        version: None,
                        sha256: new_sha256,
                        installed_at: chrono::Utc::now().to_rfc3339(),
                        java_packages: jar.java_packages,
                        mod_jar_id,
                        provided_mod_ids: jar
                            .provided_mods
                            .iter()
                            .map(|pm| pm.mod_id.clone())
                            .collect(),
                        enabled: !theirs_path_str.ends_with(DISABLED_SUFFIX),
                        content_type: ct.to_string(),
                        update_pinned: false,
                        pack_managed: true,
                        installed_as_dependency: false,
                        depends_on: jar.depends_on,
                        optional_deps: jar.optional_deps,
                        incompatible_deps: jar.incompatible_deps,
                    };
                    let arr: &mut Vec<crate::models::InstalledMod> = match ct {
                        "resourcepack" => &mut new.resourcepacks,
                        "shader" => &mut new.shaders,
                        "datapack" => &mut new.datapacks,
                        "world" => &mut new.worlds,
                        _ => &mut new.mods,
                    };
                    if let Some(pos) = arr.iter().position(|e| e.filename == installed.filename) {
                        arr.remove(pos);
                    }
                    arr.push(installed);
                }
            }
        }
    }

    Ok(new)
}

/// Apply a merge plan against a fully staged THEIRS tree.
///
/// `staged_dir` must already contain every file the plan references (jars and
/// overrides extracted with real bytes — see [`stage_pack`]). The mandatory
/// snapshot is taken before any mutation; if any step fails the snapshot is
/// restored, leaving the instance exactly as it was. On success the new baseline
/// (THEIRS) is written to `instance_pack_files`.
///
/// `resolutions` maps every conflicting key to how it was settled. A plan with an
/// unresolved conflict is rejected rather than guessed at.
pub fn apply_merge(
    conn: &rusqlite::Connection,
    instance_id: &str,
    instance_dir: &Path,
    staged_dir: &Path,
    plan: &PackMergePlan,
    resolutions: &BTreeMap<String, ConflictResolution>,
    reconciled_manifest: &crate::models::InstanceManifest,
) -> Result<MergeOutcome, String> {
    if !staged_dir.is_dir() {
        return Err(format!(
            "staged THEIRS tree is not a directory: {}",
            staged_dir.display()
        ));
    }
    // The new baseline is read back out of the staged tree, so an empty stage
    // would record an empty baseline — and an empty baseline is precisely what
    // makes the *next* update unmergeable, turning every pack file into a
    // conflict the user cannot answer. A stage with nothing in it is a staging
    // failure, not a pack that legitimately ships zero files, so refuse here
    // rather than after the instance has been mutated.
    if crate::pack_inventory::collect_pack_inventory(staged_dir)?.is_empty() {
        return Err(format!(
            "staged THEIRS tree is empty: {}; refusing to apply a merge that would erase the pack baseline",
            staged_dir.display()
        ));
    }
    for conflict in &plan.conflicts {
        if !resolutions.contains_key(&conflict.key) {
            return Err(format!(
                "conflict '{}' is unresolved; provide a resolution before applying",
                conflict.key
            ));
        }
    }

    // Pre-validate that every file the plan needs actually exists in staging, so
    // a missing staged jar fails before the snapshot (no rollback needed).
    let need_source: Vec<&crate::pack_merge::PlanAction> = plan
        .actions
        .iter()
        .filter(|a| needs_staged_source(a.kind.as_str()))
        .collect();
    for conflict in &plan.conflicts {
        if resolutions.get(&conflict.key) == Some(&ConflictResolution::TakeTheirs) {
            if let Some(theirs_path) = &conflict.theirs_path {
                let src = safe_join(staged_dir, theirs_path)?;
                if !src.is_file() {
                    return Err(format!(
                        "staged file missing for resolved conflict '{}': {}",
                        conflict.key, theirs_path
                    ));
                }
            } else {
                return Err(format!(
                    "cannot TakeTheirs for '{}': the new pack does not ship this file",
                    conflict.key
                ));
            }
        }
    }
    for action in &need_source {
        let src = safe_join(staged_dir, &action.logical_path)?;
        if !src.is_file() {
            return Err(format!(
                "staged file missing for '{}': {}",
                action.key, action.logical_path
            ));
        }
    }

    let snapshot = crate::snapshot::create_snapshot(instance_dir, Some("pack-merge"))?;

    // Everything that can leave the instance inconsistent belongs inside this
    // closure, because everything inside it is undone by the snapshot restore
    // below. The baseline write used to sit *after* it: a failure there left
    // the files merged but `instance_pack_files` still describing the old pack,
    // so the next update would diff against a baseline that never existed and
    // read the whole previous update as the user's own edits.
    let apply_result = (|| -> Result<(), String> {
        for action in &plan.actions {
            apply_one_action(instance_dir, staged_dir, action)?;
        }
        for conflict in &plan.conflicts {
            if resolutions.get(&conflict.key) == Some(&ConflictResolution::TakeTheirs) {
                apply_theirs_for_conflict(instance_dir, staged_dir, conflict)?;
            }
            // KeepOurs: no change.
        }

        // Reconciled manifest write is inside the protected region so the
        // snapshot's `instance_manifest.json` coverage makes rollback free.
        crate::helpers::atomic_write_manifest(
            &instance_dir.join("instance_manifest.json"),
            reconciled_manifest,
        )
        .map_err(|e| format!("failed to write reconciled manifest: {e:?}"))?;

        // New baseline = THEIRS, read back from the staged tree so the hashes
        // are of the bytes actually installed.
        let baseline = crate::pack_inventory::collect_pack_inventory(staged_dir)?;
        replace_instance_pack_files(conn, instance_id, &baseline)
            .map_err(|e| format!("failed to record the new pack baseline: {e}"))?;

        // Bump the mutation generation, exactly as the install pipeline does
        // after its own apply. This is what tells the pre-launch snapshot logic
        // that an existing recovery point no longer describes the instance;
        // without it a merge is invisible to that check and a stale pre-launch
        // snapshot stays eligible for reuse.
        crate::snapshot::mark_instance_mutated(instance_dir)?;
        Ok(())
    })();

    if let Err(error) = apply_result {
        let restore = crate::snapshot::restore_snapshot(instance_dir, &snapshot.id);
        return match restore {
            Ok(()) => Err(format!(
                "merge apply failed and the pre-merge snapshot was restored: {error}"
            )),
            Err(restore_error) => Err(format!(
                "merge apply failed and automatic restore could not complete; original state is protected in recovery storage. apply: {error}; restore: {restore_error}"
            )),
        };
    }

    Ok(MergeOutcome {
        snapshot_id: snapshot.id,
        rollback_performed: false,
    })
}

fn needs_staged_source(kind: &str) -> bool {
    matches!(
        kind,
        "add" | "update" | "update_keep_disabled" | "rename_update" | "rename_update_keep_disabled"
    )
}

fn apply_one_action(
    instance_dir: &Path,
    staged_dir: &Path,
    action: &crate::pack_merge::PlanAction,
) -> Result<(), String> {
    match action.kind.as_str() {
        "keep" | "keep_user_added" => Ok(()),
        "remove" => {
            let dest = safe_join(instance_dir, &action.target_path)?;
            if dest.exists() {
                remove_file(&dest)?;
            }
            Ok(())
        }
        "add" | "update" | "update_keep_disabled" => {
            let src = safe_join(staged_dir, &action.logical_path)?;
            let dest = safe_join(instance_dir, &action.target_path)?;
            copy_file(&src, &dest)
        }
        "rename_update" | "rename_update_keep_disabled" => {
            if let Some(previous) = &action.previous_path {
                let prev = safe_join(instance_dir, previous)?;
                if prev.exists() {
                    remove_file(&prev)?;
                }
            }
            let src = safe_join(staged_dir, &action.logical_path)?;
            let dest = safe_join(instance_dir, &action.target_path)?;
            copy_file(&src, &dest)
        }
        other => Err(format!("unhandled plan action kind: {other}")),
    }
}

fn apply_theirs_for_conflict(
    instance_dir: &Path,
    staged_dir: &Path,
    conflict: &crate::pack_merge::PlanConflict,
) -> Result<(), String> {
    let theirs_path = conflict.theirs_path.as_deref().ok_or_else(|| {
        format!(
            "cannot TakeTheirs for '{}': pack removed this file",
            conflict.key
        )
    })?;
    let src = safe_join(staged_dir, theirs_path)?;
    let dest = safe_join(instance_dir, theirs_path)?;
    if let Some(ours_path) = &conflict.ours_path {
        if logical_path(ours_path) != logical_path(theirs_path) {
            let old = safe_join(instance_dir, ours_path)?;
            if old.exists() {
                remove_file(&old)?;
            }
        }
    }
    copy_file(&src, &dest)
}

fn copy_file(src: &Path, dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create dir {}: {e}", parent.display()))?;
    }
    fs::copy(src, dest)
        .map_err(|e| format!("cannot copy {} -> {}: {e}", src.display(), dest.display()))?;
    // Fail after the write so a rollback test exercises restoring an already-
    // mutated file rather than failing before any mutation.
    check_failpoint("apply-one")?;
    Ok(())
}

#[cfg(test)]
thread_local! {
    static PACK_UPDATE_TEST_FAILPOINT: std::cell::RefCell<Option<&'static str>> =
        const { std::cell::RefCell::new(None) };
}

fn check_failpoint(name: &'static str) -> Result<(), String> {
    #[cfg(test)]
    {
        if PACK_UPDATE_TEST_FAILPOINT.with(|s| *s.borrow() == Some(name)) {
            return Err(format!("injected pack-update failpoint: {name}"));
        }
    }
    #[cfg(not(test))]
    let _ = name;
    Ok(())
}

fn remove_file(path: &Path) -> Result<(), String> {
    fs::remove_file(path).map_err(|e| format!("cannot remove {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// Staging (network behind a trait)
// ---------------------------------------------------------------------------

/// Fetches the bytes for a pack download. Tests inject a fake; production uses
/// [`NetworkFetcher`].
/// What happened to a pack update, end to end.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum PackUpdateOutcome {
    /// Files, baseline and manifest all moved to the new pack version.
    Updated {
        snapshot_id: String,
        /// Paths the merge changed, for a summary line.
        changed: usize,
        /// Entries left exactly as the user had them.
        kept: usize,
        health: Option<crate::health::HealthReport>,
    },
    /// Post-apply health found blockers. The update is **kept** so the user can
    /// read the report and decide — same posture as an ordinary install.
    ///
    /// Rolling back here would destroy conflict resolutions the user just made,
    /// and the pre-update instance may well have been equally unhealthy.
    HealthBlocked {
        snapshot_id: String,
        health: crate::health::HealthReport,
    },
    /// Refused before any mutation, or mutated and verifiably restored.
    Failed {
        phase: String,
        error: String,
        rolled_back: bool,
        snapshot_id: Option<String>,
    },
}

/// Plan, stage, apply and health-check a pack update in one call.
///
/// Ordering is the whole contract:
/// 1. preview (offline) — decides what would change, and what conflicts
/// 2. every conflict must carry a resolution, or nothing happens
/// 3. stage and hash-verify the new pack
/// 4. reconcile the manifest *from the plan*, before any mutation
/// 5. apply — files, baseline, manifest and the mutation bump, all inside one
///    snapshot-protected closure
/// 6. health scan, read-only, after the commit
///
/// Health runs last and never rolls back. It is a report on a committed
/// instance, and a red result usually means the new pack is broken rather than
/// that the merge went wrong — reverting a correct merge would throw away the
/// user's conflict resolutions and could loop forever if the instance was
/// already unhealthy.
#[allow(clippy::too_many_arguments)]
pub fn update_pack<F: PackFileFetcher>(
    conn: &rusqlite::Connection,
    instance_id: &str,
    instance_dir: &Path,
    mrpack_path: &Path,
    staged_dir: &Path,
    resolutions: &BTreeMap<String, ConflictResolution>,
    fetcher: &F,
    skip_health_scan: bool,
) -> PackUpdateOutcome {
    let failed = |phase: &str, error: String| PackUpdateOutcome::Failed {
        phase: phase.to_string(),
        error,
        rolled_back: false,
        snapshot_id: None,
    };

    let preview = match preview_pack_update(conn, instance_id, instance_dir, mrpack_path) {
        Ok(preview) => preview,
        Err(error) => return failed("preview", error),
    };
    for conflict in &preview.plan.conflicts {
        if !resolutions.contains_key(&conflict.key) {
            return failed(
                "conflicts",
                format!(
                    "'{}' is unresolved; every conflict needs an answer before applying",
                    conflict.key
                ),
            );
        }
    }

    if let Err(error) = stage_pack(mrpack_path, staged_dir, fetcher) {
        return failed("stage", error);
    }
    let theirs = match theirs_from_mrpack(mrpack_path) {
        Ok(theirs) => theirs,
        Err(error) => return failed("stage", error),
    };

    let manifest_path = instance_dir.join("instance_manifest.json");
    let old_manifest = match crate::helpers::read_manifest(&manifest_path) {
        Ok(manifest) => manifest,
        Err(error) => return failed("manifest", error.to_string()),
    };
    let reconciled = match reconcile_manifest(
        &old_manifest,
        &preview.plan,
        resolutions,
        &theirs,
        staged_dir,
    ) {
        Ok(manifest) => manifest,
        Err(error) => return failed("manifest", error),
    };

    let outcome = match apply_merge(
        conn,
        instance_id,
        instance_dir,
        staged_dir,
        &preview.plan,
        resolutions,
        &reconciled,
    ) {
        Ok(outcome) => outcome,
        Err(error) => {
            // apply_merge restores the snapshot itself on failure and says so
            // in the message; anything it could not restore is flagged there.
            return PackUpdateOutcome::Failed {
                phase: "apply".into(),
                error,
                rolled_back: true,
                snapshot_id: None,
            };
        }
    };

    let changed = preview.plan.actions.len();
    let kept = preview
        .plan
        .actions
        .iter()
        .filter(|action| {
            matches!(
                action.kind,
                crate::pack_merge::PlanActionKind::Keep
                    | crate::pack_merge::PlanActionKind::KeepUserAdded
            )
        })
        .count();

    if skip_health_scan {
        return PackUpdateOutcome::Updated {
            snapshot_id: outcome.snapshot_id,
            changed,
            kept,
            health: None,
        };
    }

    // Read-only, and deliberately after the commit. The manifest is already
    // reconciled, so this cannot fire spurious drift warnings against the files
    // the merge just installed.
    let report = crate::health::cached_health(instance_dir, &reconciled, None, None);
    if report.blockers.is_empty() {
        PackUpdateOutcome::Updated {
            snapshot_id: outcome.snapshot_id,
            changed,
            kept,
            health: Some(report),
        }
    } else {
        PackUpdateOutcome::HealthBlocked {
            snapshot_id: outcome.snapshot_id,
            health: report,
        }
    }
}

pub trait PackFileFetcher: Send + Sync {
    fn fetch(&self, url: &str) -> Result<Vec<u8>, String>;
}

const MRPACK_DOWNLOAD_ALLOWLIST: &[&str] = &[
    "cdn.modrinth.com",
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];

fn validate_download_url(raw: &str) -> Result<(), String> {
    let url = reqwest::Url::parse(raw).map_err(|_| format!("invalid download URL: {raw}"))?;
    let host = url
        .host_str()
        .ok_or_else(|| format!("URL has no host: {raw}"))?;
    if url.scheme() != "https"
        || url.port_or_known_default() != Some(443)
        || !MRPACK_DOWNLOAD_ALLOWLIST.contains(&host)
    {
        return Err(format!("untrusted download origin: {raw}"));
    }
    Ok(())
}

/// Real network fetcher, using the project's allowlist and Modpack client
/// category (large responses allowed).
pub struct NetworkFetcher;

impl PackFileFetcher for NetworkFetcher {
    fn fetch(&self, url: &str) -> Result<Vec<u8>, String> {
        validate_download_url(url)?;
        let clients = crate::http_client::HttpClients::new().map_err(|e| e.to_string())?;
        crate::http_client::blocking_checked_get_bytes(
            &clients,
            crate::http_client::ClientCategory::Modpack,
            url,
        )
        .map_err(|e| e.to_string())
    }
}

/// Stage a `.mrpack` into `staged_dir`: extract the embedded override tree and
/// every index-listed file, downloading non-embedded jars through `fetcher`,
/// verifying each against its index hash.
pub fn stage_pack<F: PackFileFetcher>(
    mrpack_path: &Path,
    staged_dir: &Path,
    fetcher: &F,
) -> Result<(), String> {
    if staged_dir.exists() {
        fs::remove_dir_all(staged_dir).map_err(|e| format!("cannot clear staging dir: {e}"))?;
    }
    fs::create_dir_all(staged_dir).map_err(|e| format!("cannot create staging dir: {e}"))?;

    let file = fs::File::open(mrpack_path).map_err(|e| format!("cannot open mrpack: {e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("invalid mrpack zip: {e}"))?;
    let index = {
        let mut entry = archive
            .by_name("modrinth.index.json")
            .map_err(|_| "missing modrinth.index.json".to_string())?;
        let mut text = String::new();
        entry
            .read_to_string(&mut text)
            .map_err(|e| format!("cannot read modrinth.index.json: {e}"))?;
        serde_json::from_str::<MrpackIndex>(&text)
            .map_err(|e| format!("invalid modrinth.index.json: {e}"))?
    };

    // Overrides embedded in the archive.
    for i in 0..archive.len() {
        let mut entry = match archive.by_index(i) {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.is_dir() {
            continue;
        }
        let entry_name = entry.name().replace('\\', "/");
        let Some(rel) = mrpack_override_relative(&entry_name, &index.overrides) else {
            continue;
        };
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|e| format!("cannot read override {entry_name}: {e}"))?;
        write_staged(staged_dir, &rel, &bytes)?;
    }

    // Index-listed files (mods and other content).
    for file_entry in &index.files {
        let rel = file_entry.path.replace('\\', "/");
        if !in_pack_scope(&rel) {
            continue;
        }
        let bytes = if let Ok(mut entry) = archive.by_name(&rel) {
            let mut b = Vec::new();
            entry
                .read_to_end(&mut b)
                .map_err(|e| format!("cannot read embedded {}: {e}", file_entry.path))?;
            b
        } else {
            let url = file_entry.downloads.first().ok_or_else(|| {
                format!(
                    "{} has no download URL and is not embedded",
                    file_entry.path
                )
            })?;
            fetcher.fetch(url)?
        };
        verify_bytes(file_entry, &bytes)?;
        write_staged(staged_dir, &rel, &bytes)?;
    }

    Ok(())
}

fn write_staged(staged_dir: &Path, rel: &str, bytes: &[u8]) -> Result<(), String> {
    let dest = safe_join(staged_dir, rel)?;
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create dir {}: {e}", parent.display()))?;
    }
    fs::write(&dest, bytes).map_err(|e| format!("cannot write {}: {e}", dest.display()))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{run_migrations, InstancePackFile};
    use rusqlite::Connection;
    use std::io::Write;
    use tempfile::TempDir;

    fn sha(c: char) -> String {
        std::iter::repeat_n(c, 64).collect()
    }

    fn test_conn() -> (Connection, TempDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("state.db");
        let conn = Connection::open(&path).unwrap();
        run_migrations(&conn).unwrap();
        (conn, tmp)
    }

    /// `instance_pack_files` has an FK to `user_instances`; a row must exist
    /// before any inventory write.
    fn seed_instance_row(conn: &Connection, id: &str) {
        let row = crate::models::InstanceRow {
            instance_id: id.into(),
            name: id.into(),
            minecraft_version: "1.21".into(),
            loader: "fabric".into(),
            loader_version: "0.15.0".into(),
            is_modpack: true,
            is_locked: false,
            last_launched_at: None,
            jvm_memory_mb: 4096,
            jvm_memory_mode: "manual".into(),
            jvm_gc: "auto".into(),
            jvm_custom_args: String::new(),
            jvm_always_pre_touch: false,
            created_at: chrono::Utc::now().to_rfc3339(),
            java_path: None,
            java_incompatible_override: false,
            icon_path: None,
            launch_mode_override: "auto".into(),
            import_source: None,
        };
        crate::db::upsert_instance(conn, &row).unwrap();
    }

    fn write(dir: &Path, rel: &str, bytes: &[u8]) {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&p, bytes).unwrap();
    }

    fn write_manifest(instance_dir: &Path, id: &str) {
        write_manifest_with_mods(instance_dir, id, vec![]);
    }

    fn write_manifest_with_mods(
        instance_dir: &Path,
        id: &str,
        mods: Vec<crate::models::InstalledMod>,
    ) {
        let m = crate::models::InstanceManifest {
            manifest_version: crate::models::CURRENT_MANIFEST_VERSION,
            instance_id: id.to_string(),
            name: id.to_string(),
            created_from_pack: None,
            pack_origin: None,
            minecraft_version: "1.21".to_string(),
            loader: "fabric".to_string(),
            loader_version: "0.15.0".to_string(),
            is_locked: false,
            mods,
            resourcepacks: vec![],
            shaders: vec![],
            datapacks: vec![],
            worlds: vec![],
            user_preferences: serde_json::json!({}),
        };
        write(
            instance_dir,
            "instance_manifest.json",
            serde_json::to_string_pretty(&m).unwrap().as_bytes(),
        );
    }

    /// Build a minimal .mrpack (zip) with an index + optional embedded overrides.
    /// `index_files` is a list of `(path, sha1, sha512, downloads_json)` — the
    /// hashes must match the bytes if `bytes` is `Some` (embedded).
    fn build_mrpack(
        dir: &Path,
        name: &str,
        files: Vec<(&str, &str, &str, &str)>,
        overrides: Vec<(&str, &[u8])>,
    ) -> PathBuf {
        use zip::write::FileOptions;
        let path = dir.join(format!("{name}.mrpack"));
        let f = fs::File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(f);
        let opts = FileOptions::default();

        let mut index_files = Vec::new();
        for (p, sha1, sha512, downloads) in files {
            index_files.push(serde_json::json!({
                "path": p,
                "hashes": { "sha1": sha1, "sha512": sha512 },
                "downloads": serde_json::from_str::<serde_json::Value>(downloads).unwrap(),
            }));
        }
        let index = serde_json::json!({
            "formatVersion": 1,
            "game": "minecraft",
            "versionId": "2.0.0",
            "name": name,
            "dependencies": { "minecraft": "1.21", "fabric-loader": "0.15.0" },
            "files": index_files,
        });
        zip.start_file("modrinth.index.json", opts).unwrap();
        zip.write_all(serde_json::to_string_pretty(&index).unwrap().as_bytes())
            .unwrap();
        for (rel, bytes) in overrides {
            zip.start_file(rel, opts).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
        path
    }

    fn sha1_hex(data: &[u8]) -> String {
        DigestAlgo::Sha1.hash(data)
    }
    fn sha512_hex(data: &[u8]) -> String {
        DigestAlgo::Sha512.hash(data)
    }

    // ---- Q1: offline THEIRS from an mrpack ---------------------------------

    #[test]
    fn theirs_hashes_overrides_but_marks_mods_unverified() {
        let tmp = TempDir::new().unwrap();
        let config_bytes = b"some config";
        let mrpack = build_mrpack(
            tmp.path(),
            "p",
            vec![(
                "mods/sodium-0.6.jar",
                &sha1_hex(b"newjar"),
                &sha512_hex(b"newjar"),
                r#"["https://cdn.modrinth.com/data/abc/versions/v1/sodium-0.6.jar"]"#,
            )],
            vec![("overrides/config/foo.toml", config_bytes)],
        );

        let theirs = theirs_from_mrpack(&mrpack).unwrap();
        // config/foo.toml verified with a real sha256.
        let config = theirs
            .files
            .iter()
            .find(|f| f.relative_path == "config/foo.toml")
            .unwrap();
        assert_eq!(config.sha256, sha2_256_hex(config_bytes));
        assert!(!theirs.unverified.contains("config/foo.toml"));

        // The mod is unverified and carries the sha512-derived placeholder.
        let mod_entry = theirs
            .files
            .iter()
            .find(|f| f.relative_path == "mods/sodium-0.6.jar")
            .unwrap();
        assert!(theirs.unverified.contains("mods/sodium-0.6.jar"));
        assert_ne!(
            mod_entry.sha256.len(),
            64,
            "placeholder must not look like a real sha256"
        );
        assert_eq!(theirs.files_needing_download, 1);

        // Mod-id channel from the URL.
        assert_eq!(
            theirs
                .mod_ids
                .get("mods/sodium-0.6.jar")
                .map(String::as_str),
            Some("abc")
        );
    }

    #[test]
    fn embedded_index_file_is_verified() {
        let tmp = TempDir::new().unwrap();
        let bytes = b"embedded jar bytes";
        // An index file that is ALSO embedded in the archive gets verified bytes:
        // build a zip that contains both the index and the actual jar file.
        let out = tmp.path().join("p2.mrpack");
        let f2 = fs::File::create(&out).unwrap();
        let mut zw = zip::ZipWriter::new(f2);
        let opts = zip::write::FileOptions::default();
        let index = serde_json::json!({
            "formatVersion": 1, "game": "minecraft", "name": "p",
            "files": [{ "path": "mods/embedded.jar", "hashes": { "sha1": sha1_hex(bytes), "sha512": sha512_hex(bytes) }, "downloads": [] }]
        });
        zw.start_file("modrinth.index.json", opts).unwrap();
        zw.write_all(serde_json::to_string_pretty(&index).unwrap().as_bytes())
            .unwrap();
        zw.start_file("mods/embedded.jar", opts).unwrap();
        zw.write_all(bytes).unwrap();
        zw.finish().unwrap();

        let theirs = theirs_from_mrpack(&out).unwrap();
        let e = theirs
            .files
            .iter()
            .find(|f| f.relative_path == "mods/embedded.jar")
            .unwrap();
        assert_eq!(e.sha256, sha2_256_hex(bytes));
        assert!(!theirs.unverified.contains("mods/embedded.jar"));
        assert_eq!(theirs.files_needing_download, 0);
    }

    // ---- Q3: baseline decision ---------------------------------------------

    #[test]
    fn baseline_records_theirs_not_ours() {
        // First merge: pack ships foo.toml at hash B (updated from A); the user
        // resolves the conflict by keeping their own file at hash U.
        let theirs = vec![InstancePackFile {
            relative_path: "config/foo.toml".into(),
            sha256: sha('b'),
            size: 1,
        }];
        let new_baseline = new_pack_baseline(&theirs);
        assert_eq!(
            new_baseline, theirs,
            "new baseline must be THEIRS, not OURS"
        );

        // Simulate the SECOND update. The pack changes the file again (B').
        let ours = vec![InstancePackFile {
            relative_path: "config/foo.toml".into(),
            sha256: sha('u'), // user's kept file
            size: 1,
        }];
        let theirs_second = vec![InstancePackFile {
            relative_path: "config/foo.toml".into(),
            sha256: sha('c'), // pack moved on again
            size: 1,
        }];

        // With the correct baseline (THEIRS = pack hash B), the user's divergence
        // stays visible -> conflict, never a silent overwrite.
        let plan = crate::pack_merge::plan_pack_update(&new_baseline, &theirs_second, &ours);
        assert_eq!(plan.conflicts.len(), 1);
        assert_eq!(
            plan.conflicts[0].kind,
            crate::pack_merge::ConflictKind::BothModified
        );

        // Contrast: if we had wrongly recorded OURS (hash U) as the baseline, the
        // planner would see base==ours, so a pack change reads as "unchanged
        // locally -> take theirs", silently clobbering the user's edit.
        let wrong_baseline = vec![InstancePackFile {
            relative_path: "config/foo.toml".into(),
            sha256: sha('u'),
            size: 1,
        }];
        let bad_plan = crate::pack_merge::plan_pack_update(&wrong_baseline, &theirs_second, &ours);
        assert!(bad_plan.conflicts.is_empty());
        assert!(
            bad_plan
                .actions
                .iter()
                .any(|a| a.kind.as_str() == "update" && a.logical_path == "config/foo.toml"),
            "recording OURS would silently update over the user's edit"
        );
    }

    #[test]
    fn second_update_with_pack_unchanged_keeps_user_edit() {
        let theirs_first = vec![InstancePackFile {
            relative_path: "config/foo.toml".into(),
            sha256: sha('b'),
            size: 1,
        }];
        let baseline = new_pack_baseline(&theirs_first);
        // User kept their edit (U); pack does NOT change next time.
        let ours = vec![InstancePackFile {
            relative_path: "config/foo.toml".into(),
            sha256: sha('u'),
            size: 1,
        }];
        let theirs_second = theirs_first.clone();
        let plan = crate::pack_merge::plan_pack_update(&baseline, &theirs_second, &ours);
        assert!(plan.conflicts.is_empty());
        assert!(
            plan.actions
                .iter()
                .any(|a| a.kind.as_str() == "keep" && a.logical_path == "config/foo.toml"),
            "user edit must be kept when the pack is unchanged"
        );
    }

    // ---- Apply -------------------------------------------------------------

    fn seed_instance(tmp: &TempDir, id: &str) -> PathBuf {
        let inst = tmp.path().join(id);
        fs::create_dir_all(inst.join("mods")).unwrap();
        fs::create_dir_all(inst.join("config")).unwrap();
        write_manifest(&inst, id);
        inst
    }

    #[test]
    fn apply_keeps_user_added_mod_and_updates_pack_mod() {
        let tmp = TempDir::new().unwrap();
        let inst = seed_instance(&tmp, "inst");
        // OURS on disk: packmod at A (unchanged), user-added usermod.jar, config at A.
        write(&inst, "mods/packmod.jar", b"packA");
        write(&inst, "mods/usermod.jar", b"user");
        write(&inst, "config/foo.toml", b"configA");
        // BASE: the pack's hashes of the A-content it originally installed.
        let base = vec![
            InstancePackFile {
                relative_path: "mods/packmod.jar".into(),
                sha256: sha2_256_hex(b"packA"),
                size: 5,
            },
            InstancePackFile {
                relative_path: "config/foo.toml".into(),
                sha256: sha2_256_hex(b"configA"),
                size: 7,
            },
        ];
        // THEIRS (new pack): packmod at B, foo.toml at B, no usermod.
        let theirs = vec![
            InstancePackFile {
                relative_path: "mods/packmod.jar".into(),
                sha256: sha('b'),
                size: 1,
            },
            InstancePackFile {
                relative_path: "config/foo.toml".into(),
                sha256: sha('b'),
                size: 1,
            },
        ];
        let ours = crate::pack_inventory::collect_pack_inventory(&inst).unwrap();
        let plan = crate::pack_merge::plan_pack_update(&base, &theirs, &ours);
        assert!(plan.conflicts.is_empty(), "{:?}", plan.conflicts);

        // Stage the new pack's real bytes.
        let staged = tmp.path().join("staged");
        write(&staged, "mods/packmod.jar", b"packB");
        write(&staged, "config/foo.toml", b"configB");

        let (conn, _dbtmp) = test_conn();
        seed_instance_row(&conn, "inst");
        let old_manifest =
            crate::helpers::read_manifest(&inst.join("instance_manifest.json")).unwrap();
        let theirs_struct = MrpackTheirs {
            files: theirs.clone(),
            unverified: BTreeSet::new(),
            converged: BTreeSet::new(),
            provisional_hash: BTreeMap::new(),
            download_urls: BTreeMap::new(),
            mod_ids: HashMap::new(),
            files_needing_download: 0,
            download_bytes: 0,
            size_unknown_count: 0,
            pack_name: "test".into(),
            pack_version_id: None,
        };
        let reconciled = reconcile_manifest(
            &old_manifest,
            &plan,
            &BTreeMap::new(),
            &theirs_struct,
            &staged,
        )
        .unwrap();
        let outcome = apply_merge(
            &conn,
            "inst",
            &inst,
            &staged,
            &plan,
            &BTreeMap::new(),
            &reconciled,
        )
        .unwrap();
        assert!(!outcome.rollback_performed);

        // User-added mod still present.
        assert_eq!(
            fs::read(inst.join("mods/usermod.jar")).unwrap(),
            b"user",
            "user-added mod must survive the update"
        );
        // Pack mod updated.
        assert_eq!(fs::read(inst.join("mods/packmod.jar")).unwrap(), b"packB");
        assert_eq!(fs::read(inst.join("config/foo.toml")).unwrap(), b"configB");

        // New baseline written = THEIRS (pack files), NOT including the user mod.
        let baseline = list_instance_pack_files(&conn, "inst").unwrap();
        assert!(
            baseline
                .iter()
                .any(|f| f.relative_path == "mods/packmod.jar"),
            "baseline must contain the updated pack mod"
        );
        assert!(
            !baseline
                .iter()
                .any(|f| f.relative_path == "mods/usermod.jar"),
            "baseline must NOT contain the user-added mod"
        );
    }

    #[test]
    fn apply_failure_partway_restores_launchable_state() {
        let tmp = TempDir::new().unwrap();
        let inst = seed_instance(&tmp, "inst");
        write(&inst, "mods/packmod.jar", b"packA");
        write(&inst, "mods/usermod.jar", b"user");
        // BASE hash matches the on-disk A content, so this is a plain Update.
        let base = vec![InstancePackFile {
            relative_path: "mods/packmod.jar".into(),
            sha256: sha2_256_hex(b"packA"),
            size: 5,
        }];
        let theirs = vec![InstancePackFile {
            relative_path: "mods/packmod.jar".into(),
            sha256: sha('b'),
            size: 1,
        }];
        let ours = crate::pack_inventory::collect_pack_inventory(&inst).unwrap();
        let plan = crate::pack_merge::plan_pack_update(&base, &theirs, &ours);

        // Stage is missing the packmod jar -> pre-validation fails BEFORE snapshot,
        // so nothing changes at all.
        let staged = tmp.path().join("staged");
        write(&staged, "config/foo.toml", b"x"); // wrong file present
        let before = crate::snapshot::live_file_index(&inst).unwrap();
        let (conn, _dbtmp) = test_conn();
        seed_instance_row(&conn, "inst");
        let old_manifest =
            crate::helpers::read_manifest(&inst.join("instance_manifest.json")).unwrap();
        let err = apply_merge(
            &conn,
            "inst",
            &inst,
            &staged,
            &plan,
            &BTreeMap::new(),
            &old_manifest,
        )
        .unwrap_err();
        assert!(err.contains("staged file missing"));
        assert_eq!(
            crate::snapshot::live_file_index(&inst).unwrap(),
            before,
            "no mutation before the mandatory snapshot"
        );
        assert_eq!(fs::read(inst.join("mods/usermod.jar")).unwrap(), b"user");
    }

    #[test]
    fn an_empty_staged_tree_never_wipes_a_good_baseline() {
        // The new baseline is read from the staged tree, so an empty stage
        // would record an empty baseline — and an empty baseline is exactly
        // what makes the *next* update unmergeable, turning every pack file
        // into a conflict. A stage with nothing in it is a staging failure, not
        // a pack that legitimately ships zero files.
        let tmp = TempDir::new().unwrap();
        let inst = seed_instance(&tmp, "inst");
        write(&inst, "mods/a.jar", b"a-old");
        let base = vec![InstancePackFile {
            relative_path: "mods/a.jar".into(),
            sha256: sha2_256_hex(b"a-old"),
            size: 5,
        }];
        let (conn, _dbtmp) = test_conn();
        seed_instance_row(&conn, "inst");
        crate::db::replace_instance_pack_files(&conn, "inst", &base).unwrap();

        // Nothing to do, and nothing staged.
        let ours = crate::pack_inventory::collect_pack_inventory(&inst).unwrap();
        let plan = crate::pack_merge::plan_pack_update(&base, &base, &ours);
        let staged = tmp.path().join("empty-stage");
        std::fs::create_dir_all(&staged).unwrap();

        let old_manifest =
            crate::helpers::read_manifest(&inst.join("instance_manifest.json")).unwrap();
        let theirs_struct = MrpackTheirs {
            files: base.clone(),
            unverified: BTreeSet::new(),
            converged: BTreeSet::new(),
            provisional_hash: BTreeMap::new(),
            download_urls: BTreeMap::new(),
            mod_ids: HashMap::new(),
            files_needing_download: 0,
            download_bytes: 0,
            size_unknown_count: 0,
            pack_name: "test".into(),
            pack_version_id: None,
        };
        let reconciled = reconcile_manifest(
            &old_manifest,
            &plan,
            &BTreeMap::new(),
            &theirs_struct,
            &staged,
        )
        .unwrap();
        let error = apply_merge(
            &conn,
            "inst",
            &inst,
            &staged,
            &plan,
            &BTreeMap::new(),
            &reconciled,
        )
        .expect_err("an empty stage is a staging failure, not a valid merge");
        assert!(error.contains("empty"), "{error}");
        let after = crate::db::list_instance_pack_files(&conn, "inst").unwrap();
        assert_eq!(after, base, "the recorded baseline must survive");
    }

    #[test]
    fn a_merge_bumps_the_mutation_generation() {
        // The pre-launch snapshot logic decides whether an existing recovery
        // point still describes the instance by reading this generation. A
        // merge that does not bump it is invisible to that check, so a snapshot
        // taken before the merge stays eligible for reuse — the safety net
        // silently stops describing the thing it is meant to protect.
        let tmp = TempDir::new().unwrap();
        let inst = seed_instance(&tmp, "inst");
        write(&inst, "mods/a.jar", b"a-old");
        let base = vec![InstancePackFile {
            relative_path: "mods/a.jar".into(),
            sha256: sha2_256_hex(b"a-old"),
            size: 5,
        }];
        let theirs = vec![InstancePackFile {
            relative_path: "mods/a.jar".into(),
            sha256: sha2_256_hex(b"a-new"),
            size: 5,
        }];
        let ours = crate::pack_inventory::collect_pack_inventory(&inst).unwrap();
        let plan = crate::pack_merge::plan_pack_update(&base, &theirs, &ours);
        let staged = tmp.path().join("staged");
        write(&staged, "mods/a.jar", b"a-new");

        let (conn, _dbtmp) = test_conn();
        seed_instance_row(&conn, "inst");
        let before = crate::snapshot::live_metadata_fingerprint(&inst).unwrap();
        let old_manifest =
            crate::helpers::read_manifest(&inst.join("instance_manifest.json")).unwrap();
        let theirs_struct = MrpackTheirs {
            files: theirs.clone(),
            unverified: BTreeSet::new(),
            converged: BTreeSet::new(),
            provisional_hash: BTreeMap::new(),
            download_urls: BTreeMap::new(),
            mod_ids: HashMap::new(),
            files_needing_download: 0,
            download_bytes: 0,
            size_unknown_count: 0,
            pack_name: "test".into(),
            pack_version_id: None,
        };
        let reconciled = reconcile_manifest(
            &old_manifest,
            &plan,
            &BTreeMap::new(),
            &theirs_struct,
            &staged,
        )
        .unwrap();
        apply_merge(
            &conn,
            "inst",
            &inst,
            &staged,
            &plan,
            &BTreeMap::new(),
            &reconciled,
        )
        .unwrap();
        let after = crate::snapshot::live_metadata_fingerprint(&inst).unwrap();
        assert_ne!(
            before, after,
            "a merge must be visible to the pre-launch snapshot reuse check"
        );
    }

    #[test]
    fn a_baseline_write_failure_rolls_the_files_back_too() {
        // The baseline used to be written outside the rollback closure, so a
        // failure here left files merged against a baseline describing the
        // *old* pack — and the next update would then read the whole previous
        // update as the user's own edits.
        let tmp = TempDir::new().unwrap();
        let inst = seed_instance(&tmp, "inst");
        write(&inst, "mods/a.jar", b"a-old");
        let base = vec![InstancePackFile {
            relative_path: "mods/a.jar".into(),
            sha256: sha2_256_hex(b"a-old"),
            size: 5,
        }];
        let theirs = vec![InstancePackFile {
            relative_path: "mods/a.jar".into(),
            sha256: sha2_256_hex(b"a-new"),
            size: 5,
        }];
        let ours = crate::pack_inventory::collect_pack_inventory(&inst).unwrap();
        let plan = crate::pack_merge::plan_pack_update(&base, &theirs, &ours);
        let staged = tmp.path().join("staged");
        write(&staged, "mods/a.jar", b"a-new");

        // A connection with no `user_instances` row: the FK on
        // instance_pack_files rejects the baseline write.
        let (conn, _dbtmp) = test_conn();
        let old_manifest =
            crate::helpers::read_manifest(&inst.join("instance_manifest.json")).unwrap();
        let theirs_struct = MrpackTheirs {
            files: theirs.clone(),
            unverified: BTreeSet::new(),
            converged: BTreeSet::new(),
            provisional_hash: BTreeMap::new(),
            download_urls: BTreeMap::new(),
            mod_ids: HashMap::new(),
            files_needing_download: 0,
            download_bytes: 0,
            size_unknown_count: 0,
            pack_name: "test".into(),
            pack_version_id: None,
        };
        let reconciled = reconcile_manifest(
            &old_manifest,
            &plan,
            &BTreeMap::new(),
            &theirs_struct,
            &staged,
        )
        .unwrap();
        let error = apply_merge(
            &conn,
            "inst",
            &inst,
            &staged,
            &plan,
            &BTreeMap::new(),
            &reconciled,
        )
        .expect_err("a failed baseline write must fail the merge");
        assert!(error.contains("restored"), "{error}");
        assert_eq!(
            std::fs::read(inst.join("mods/a.jar")).unwrap(),
            b"a-old",
            "the file must be back at its pre-merge content"
        );
    }

    #[test]
    fn apply_failure_after_snapshot_rolls_back() {
        // Force a failure DURING apply (after the snapshot) and assert rollback
        // restores the exact pre-merge state.
        let tmp = TempDir::new().unwrap();
        let inst = seed_instance(&tmp, "inst");
        write(&inst, "mods/a.jar", b"a-old");
        // BASE hash matches the on-disk content -> plan is a single Update.
        let base = vec![InstancePackFile {
            relative_path: "mods/a.jar".into(),
            sha256: sha2_256_hex(b"a-old"),
            size: 5,
        }];
        let theirs = vec![InstancePackFile {
            relative_path: "mods/a.jar".into(),
            sha256: sha('b'),
            size: 1,
        }];
        let ours = crate::pack_inventory::collect_pack_inventory(&inst).unwrap();
        let plan = crate::pack_merge::plan_pack_update(&base, &theirs, &ours);
        assert_eq!(plan.actions.len(), 1);
        assert_eq!(plan.actions[0].kind.as_str(), "update");

        let staged = tmp.path().join("staged");
        write(&staged, "mods/a.jar", b"a-new");

        let before = crate::snapshot::live_file_index(&inst).unwrap();
        // Inject a failure after one file has been applied.
        PACK_UPDATE_TEST_FAILPOINT.with(|s| *s.borrow_mut() = Some("apply-one"));
        let (conn, _dbtmp) = test_conn();
        seed_instance_row(&conn, "inst");
        let old_manifest =
            crate::helpers::read_manifest(&inst.join("instance_manifest.json")).unwrap();
        let theirs_struct = MrpackTheirs {
            files: theirs.clone(),
            unverified: BTreeSet::new(),
            converged: BTreeSet::new(),
            provisional_hash: BTreeMap::new(),
            download_urls: BTreeMap::new(),
            mod_ids: HashMap::new(),
            files_needing_download: 0,
            download_bytes: 0,
            size_unknown_count: 0,
            pack_name: "test".into(),
            pack_version_id: None,
        };
        let reconciled = reconcile_manifest(
            &old_manifest,
            &plan,
            &BTreeMap::new(),
            &theirs_struct,
            &staged,
        )
        .unwrap();
        let err = apply_merge(
            &conn,
            "inst",
            &inst,
            &staged,
            &plan,
            &BTreeMap::new(),
            &reconciled,
        )
        .unwrap_err();
        PACK_UPDATE_TEST_FAILPOINT.with(|s| *s.borrow_mut() = None);

        assert!(
            err.contains("restored"),
            "apply must restore on failure: {err}"
        );
        assert_eq!(
            crate::snapshot::live_file_index(&inst).unwrap(),
            before,
            "instance must be exactly as it was before the merge"
        );
        // The instance manifest still exists and the pre-merge file is back.
        assert!(inst.join("instance_manifest.json").is_file());
        assert_eq!(fs::read(inst.join("mods/a.jar")).unwrap(), b"a-old");
    }

    // ---- Staging (offline, fake fetcher) -----------------------------------

    #[derive(Default)]
    struct FakeFetcher {
        map: HashMap<String, Vec<u8>>,
    }
    impl PackFileFetcher for FakeFetcher {
        fn fetch(&self, url: &str) -> Result<Vec<u8>, String> {
            self.map
                .get(url)
                .cloned()
                .ok_or_else(|| format!("no fake bytes for {url}"))
        }
    }

    #[test]
    fn stage_pack_downloads_and_verifies_via_injected_fetcher() {
        let tmp = TempDir::new().unwrap();
        let jar_url = "https://cdn.modrinth.com/data/xyz/versions/v1/sodium-0.6.jar";
        let jar_bytes = b"newjar".to_vec();
        let mrpack = build_mrpack(
            tmp.path(),
            "p",
            vec![(
                "mods/sodium-0.6.jar",
                &sha1_hex(&jar_bytes),
                &sha512_hex(&jar_bytes),
                &format!(r#"["{jar_url}"]"#),
            )],
            vec![("overrides/config/foo.toml", b"cfg")],
        );

        let fetcher = FakeFetcher {
            map: HashMap::from([(jar_url.to_string(), jar_bytes.clone())]),
        };
        let staged = tmp.path().join("staged");
        stage_pack(&mrpack, &staged, &fetcher).unwrap();
        assert_eq!(
            fs::read(staged.join("mods/sodium-0.6.jar")).unwrap(),
            jar_bytes
        );
        assert_eq!(fs::read(staged.join("config/foo.toml")).unwrap(), b"cfg");
    }

    #[test]
    fn stage_pack_rejects_missing_fetcher_bytes() {
        let tmp = TempDir::new().unwrap();
        let jar_url = "https://cdn.modrinth.com/data/xyz/versions/v1/sodium-0.6.jar";
        let jar_bytes = b"newjar".to_vec();
        let mrpack = build_mrpack(
            tmp.path(),
            "p",
            vec![(
                "mods/sodium-0.6.jar",
                &sha1_hex(&jar_bytes),
                &sha512_hex(&jar_bytes),
                &format!(r#"["{jar_url}"]"#),
            )],
            vec![],
        );
        let fetcher = FakeFetcher::default();
        let staged = tmp.path().join("staged");
        assert!(stage_pack(&mrpack, &staged, &fetcher).is_err());
    }

    // ---- Preview -----------------------------------------------------------

    #[test]
    fn preview_reports_user_mod_kept_and_unverified_mod() {
        let tmp = TempDir::new().unwrap();
        let (conn, _dbtmp) = test_conn();
        seed_instance_row(&conn, "inst");

        // A pack-installed sodium recorded in the manifest (modrinth_id "abc"),
        // with on-disk content at hash A.
        let old_bytes = b"old-sodium";
        let sodium = crate::models::InstalledMod {
            filename: "sodium-0.5.jar".into(),
            registry_id: None,
            modrinth_id: Some("abc".into()),
            source: "modrinth-pack".into(),
            source_url: None,
            version: None,
            sha256: sha2_256_hex(old_bytes),
            installed_at: "2026-01-01T00:00:00Z".into(),
            java_packages: vec![],
            mod_jar_id: None,
            provided_mod_ids: vec![],
            enabled: true,
            content_type: "mod".into(),
            update_pinned: false,
            pack_managed: true,
            installed_as_dependency: false,
            depends_on: vec![],
            optional_deps: vec![],
            incompatible_deps: vec![],
        };
        let inst = seed_instance(&tmp, "inst");
        write_manifest_with_mods(&inst, "inst", vec![sodium]);

        // BASE from the DB (pack's original hash of the old jar).
        crate::db::replace_instance_pack_files(
            &conn,
            "inst",
            &[InstancePackFile {
                relative_path: "mods/sodium-0.5.jar".into(),
                sha256: sha2_256_hex(old_bytes),
                size: 10,
            }],
        )
        .unwrap();
        // OURS on disk: old sodium (A) + user mod.
        write(&inst, "mods/sodium-0.5.jar", old_bytes);
        write(&inst, "mods/usermod.jar", b"user");
        // THEIRS: new sodium renamed to 0.6 (B), from a modrinth CDN URL.
        let new_bytes = b"new-sodium";
        let url = "https://cdn.modrinth.com/data/abc/versions/v1/sodium-0.6.jar";
        let mrpack = build_mrpack(
            tmp.path(),
            "p",
            vec![(
                "mods/sodium-0.6.jar",
                &sha1_hex(new_bytes),
                &sha512_hex(new_bytes),
                &format!(r#"["{url}"]"#),
            )],
            vec![],
        );

        let preview = preview_pack_update(&conn, "inst", &inst, &mrpack).unwrap();
        assert!(
            preview.plan.conflicts.is_empty(),
            "{:?}",
            preview.plan.conflicts
        );
        // User mod is kept.
        assert!(preview
            .plan
            .actions
            .iter()
            .any(|a| a.logical_path == "mods/usermod.jar" && a.kind.as_str() == "keep_user_added"));
        // Sodium rename is detected via the mod-id channel (project id on every side).
        assert!(preview
            .plan
            .actions
            .iter()
            .any(|a| a.mod_id.as_deref() == Some("abc") && a.kind.as_str() == "rename_update"));
        // The mod content decision is unverified (jar not fetched).
        assert!(preview.unverified.contains("mods/sodium-0.6.jar"));
        assert_eq!(preview.files_needing_download, 1);
    }

    #[test]
    fn preview_marks_converged_mod_as_no_download() {
        let tmp = TempDir::new().unwrap();
        let (conn, _dbtmp) = test_conn();
        seed_instance_row(&conn, "inst");
        let inst = seed_instance(&tmp, "inst");

        crate::db::replace_instance_pack_files(
            &conn,
            "inst",
            &[InstancePackFile {
                relative_path: "mods/sodium-0.5.jar".into(),
                sha256: sha('a'),
                size: 1,
            }],
        )
        .unwrap();
        // The user already updated to the new version locally: on-disk jar IS the
        // new pack's jar.
        let new_bytes = b"new-sodium";
        write(&inst, "mods/sodium-0.6.jar", new_bytes);
        let url = "https://cdn.modrinth.com/data/abc/versions/v1/sodium-0.6.jar";
        let mrpack = build_mrpack(
            tmp.path(),
            "p",
            vec![(
                "mods/sodium-0.6.jar",
                &sha1_hex(new_bytes),
                &sha512_hex(new_bytes),
                &format!(r#"["{url}"]"#),
            )],
            vec![],
        );

        let preview = preview_pack_update(&conn, "inst", &inst, &mrpack).unwrap();
        assert!(
            preview.converged.contains("mods/sodium-0.6.jar"),
            "local file matches the new pack jar -> no download needed"
        );
        assert!(!preview.unverified.contains("mods/sodium-0.6.jar"));
        assert_eq!(preview.files_needing_download, 0);
        assert_eq!(preview.content_uncertain(), 0);
    }

    // ---- Reconcile ---------------------------------------------------------

    #[test]
    fn reconcile_keeps_user_added_mod_with_provenance() {
        let tmp = TempDir::new().unwrap();
        let staged = tmp.path().join("staged");
        fs::create_dir_all(staged.join("mods")).unwrap();
        // New packmod content in staged
        write(&staged, "mods/packmod.jar", b"new-pack-content");

        let user_mod = crate::models::InstalledMod {
            filename: "usermod.jar".into(),
            registry_id: Some("my-reg".into()),
            modrinth_id: Some("my-mr".into()),
            source: "manual".into(),
            source_url: Some("https://example.com/usermod.jar".into()),
            version: Some("1.0.0".into()),
            sha256: sha2_256_hex(b"old-user"),
            installed_at: "2020-01-01T00:00:00Z".into(),
            java_packages: vec!["com.example".into()],
            mod_jar_id: Some("usermodid".into()),
            provided_mod_ids: vec!["provided".into()],
            enabled: true,
            content_type: "mod".into(),
            update_pinned: true,
            pack_managed: false,
            installed_as_dependency: false,
            depends_on: vec!["dep".into()],
            optional_deps: vec![],
            incompatible_deps: vec![],
        };
        let pack_mod = crate::models::InstalledMod {
            filename: "packmod.jar".into(),
            registry_id: None,
            modrinth_id: Some("pack-mr".into()),
            source: "modrinth-pack".into(),
            source_url: None,
            version: None,
            sha256: sha2_256_hex(b"old-pack"),
            installed_at: "2021-06-01T00:00:00Z".into(),
            java_packages: vec![],
            mod_jar_id: Some("packmodid".into()),
            provided_mod_ids: vec![],
            enabled: true,
            content_type: "mod".into(),
            update_pinned: false,
            pack_managed: true,
            installed_as_dependency: false,
            depends_on: vec![],
            optional_deps: vec![],
            incompatible_deps: vec![],
        };
        let old = crate::models::InstanceManifest {
            manifest_version: crate::models::CURRENT_MANIFEST_VERSION,
            instance_id: "inst".into(),
            name: "Inst".into(),
            created_from_pack: None,
            pack_origin: None,
            minecraft_version: "1.21".into(),
            loader: "fabric".into(),
            loader_version: "0.15.0".into(),
            is_locked: false,
            mods: vec![user_mod.clone(), pack_mod.clone()],
            resourcepacks: vec![],
            shaders: vec![],
            datapacks: vec![],
            worlds: vec![],
            user_preferences: serde_json::json!({}),
        };

        // Plan: KeepUserAdded for usermod, Update for packmod
        let plan = PackMergePlan {
            actions: vec![
                crate::pack_merge::PlanAction {
                    key: "mods/usermod.jar".into(),
                    logical_path: "mods/usermod.jar".into(),
                    target_path: "mods/usermod.jar".into(),
                    previous_path: None,
                    kind: crate::pack_merge::PlanActionKind::KeepUserAdded,
                    base_sha: None,
                    ours_sha: Some(sha2_256_hex(b"old-user")),
                    theirs_sha: None,
                    enabled: true,
                    mod_id: None,
                },
                crate::pack_merge::PlanAction {
                    key: "mods/packmod.jar".into(),
                    logical_path: "mods/packmod.jar".into(),
                    target_path: "mods/packmod.jar".into(),
                    previous_path: None,
                    kind: crate::pack_merge::PlanActionKind::Update,
                    base_sha: Some(sha2_256_hex(b"old-pack")),
                    ours_sha: Some(sha2_256_hex(b"old-pack")),
                    theirs_sha: Some(sha2_256_hex(b"new-pack-content")),
                    enabled: true,
                    mod_id: None,
                },
            ],
            conflicts: vec![],
            all_keys: vec!["mods/packmod.jar".into(), "mods/usermod.jar".into()],
            baseline_missing: false,
        };
        let theirs = MrpackTheirs {
            files: vec![],
            unverified: BTreeSet::new(),
            converged: BTreeSet::new(),
            provisional_hash: BTreeMap::new(),
            download_urls: BTreeMap::new(),
            mod_ids: HashMap::new(),
            files_needing_download: 0,
            download_bytes: 0,
            size_unknown_count: 0,
            pack_name: "test".into(),
            pack_version_id: None,
        };

        let reconciled =
            reconcile_manifest(&old, &plan, &BTreeMap::new(), &theirs, &staged).unwrap();
        assert_eq!(reconciled.mods.len(), 2, "both entries should survive");
        // User mod provenance intact
        let kept = reconciled
            .mods
            .iter()
            .find(|m| m.filename == "usermod.jar")
            .expect("usermod preserved");
        assert!(!kept.pack_managed);
        assert_eq!(kept.source, "manual");
        assert_eq!(kept.registry_id, Some("my-reg".into()));
        assert_eq!(kept.modrinth_id, Some("my-mr".into()));
        assert_eq!(kept.installed_at, "2020-01-01T00:00:00Z");
        assert!(kept.update_pinned);
        assert_eq!(kept.depends_on, vec!["dep"]);
        assert_eq!(kept.java_packages, vec!["com.example"]);
        assert_eq!(
            kept.sha256,
            sha2_256_hex(b"old-user"),
            "user mod sha unchanged"
        );
        // Pack mod updated
        let updated = reconciled
            .mods
            .iter()
            .find(|m| m.filename == "packmod.jar")
            .expect("packmod present");
        assert_eq!(updated.sha256, sha2_256_hex(b"new-pack-content"));
        assert!(updated.enabled);
        assert!(updated.pack_managed, "pack_managed preserved");
        assert_eq!(
            updated.installed_at, "2021-06-01T00:00:00Z",
            "installed_at preserved"
        );
    }

    #[test]
    fn reconcile_rename_updates_entry_not_recreate() {
        let tmp = TempDir::new().unwrap();
        let staged = tmp.path().join("staged");
        fs::create_dir_all(staged.join("mods")).unwrap();
        write(&staged, "mods/newname.jar", b"new-bytes");

        let old_entry = crate::models::InstalledMod {
            filename: "oldname.jar".into(),
            registry_id: None,
            modrinth_id: Some("abc".into()),
            source: "modrinth-pack".into(),
            source_url: None,
            version: Some("1.0".into()),
            sha256: sha2_256_hex(b"old-bytes"),
            installed_at: "2022-02-02T00:00:00Z".into(),
            java_packages: vec![],
            mod_jar_id: Some("testmod".into()),
            provided_mod_ids: vec![],
            enabled: true,
            content_type: "mod".into(),
            update_pinned: false,
            pack_managed: true,
            installed_as_dependency: false,
            depends_on: vec!["dep-a".into()],
            optional_deps: vec![],
            incompatible_deps: vec![],
        };
        let old = crate::models::InstanceManifest {
            manifest_version: crate::models::CURRENT_MANIFEST_VERSION,
            instance_id: "inst".into(),
            name: "Inst".into(),
            created_from_pack: None,
            pack_origin: None,
            minecraft_version: "1.21".into(),
            loader: "fabric".into(),
            loader_version: "0.15.0".into(),
            is_locked: false,
            mods: vec![old_entry.clone()],
            resourcepacks: vec![],
            shaders: vec![],
            datapacks: vec![],
            worlds: vec![],
            user_preferences: serde_json::json!({}),
        };

        let plan = PackMergePlan {
            actions: vec![crate::pack_merge::PlanAction {
                key: "mod:abc".into(),
                logical_path: "mods/newname.jar".into(),
                target_path: "mods/newname.jar".into(),
                previous_path: Some("mods/oldname.jar".into()),
                kind: crate::pack_merge::PlanActionKind::RenameUpdate,
                base_sha: Some(sha2_256_hex(b"old-bytes")),
                ours_sha: Some(sha2_256_hex(b"old-bytes")),
                theirs_sha: Some(sha2_256_hex(b"new-bytes")),
                enabled: true,
                mod_id: Some("abc".into()),
            }],
            conflicts: vec![],
            all_keys: vec!["mod:abc".into()],
            baseline_missing: false,
        };
        let mut mod_ids = HashMap::new();
        mod_ids.insert("mods/newname.jar".into(), "abc".into());
        let theirs = MrpackTheirs {
            files: vec![],
            unverified: BTreeSet::new(),
            converged: BTreeSet::new(),
            provisional_hash: BTreeMap::new(),
            download_urls: BTreeMap::new(),
            mod_ids,
            files_needing_download: 0,
            download_bytes: 0,
            size_unknown_count: 0,
            pack_name: "test".into(),
            pack_version_id: None,
        };

        let reconciled =
            reconcile_manifest(&old, &plan, &BTreeMap::new(), &theirs, &staged).unwrap();
        assert_eq!(reconciled.mods.len(), 1, "rename should not duplicate");
        let entry = &reconciled.mods[0];
        assert_eq!(entry.filename, "newname.jar");
        assert_eq!(entry.sha256, sha2_256_hex(b"new-bytes"));
        assert_eq!(
            entry.installed_at, "2022-02-02T00:00:00Z",
            "provenance preserved, not recreated"
        );
        assert_eq!(entry.depends_on, vec!["dep-a"]);
        assert!(entry.pack_managed);
        assert_eq!(entry.version, Some("1.0".into()), "version preserved");
    }

    #[test]
    fn reconcile_disabled_stays_disabled() {
        let tmp = TempDir::new().unwrap();
        let staged = tmp.path().join("staged");
        fs::create_dir_all(staged.join("mods")).unwrap();
        write(&staged, "mods/disabled.jar", b"new-content");

        let old_entry = crate::models::InstalledMod {
            filename: "disabled.jar".into(),
            registry_id: None,
            modrinth_id: Some("dis".into()),
            source: "modrinth-pack".into(),
            source_url: None,
            version: None,
            sha256: sha2_256_hex(b"old-content"),
            installed_at: "2023-03-03T00:00:00Z".into(),
            java_packages: vec![],
            mod_jar_id: Some("disid".into()),
            provided_mod_ids: vec![],
            enabled: false,
            content_type: "mod".into(),
            update_pinned: false,
            pack_managed: true,
            installed_as_dependency: false,
            depends_on: vec![],
            optional_deps: vec![],
            incompatible_deps: vec![],
        };
        let old = crate::models::InstanceManifest {
            manifest_version: crate::models::CURRENT_MANIFEST_VERSION,
            instance_id: "inst".into(),
            name: "Inst".into(),
            created_from_pack: None,
            pack_origin: None,
            minecraft_version: "1.21".into(),
            loader: "fabric".into(),
            loader_version: "0.15.0".into(),
            is_locked: false,
            mods: vec![old_entry],
            resourcepacks: vec![],
            shaders: vec![],
            datapacks: vec![],
            worlds: vec![],
            user_preferences: serde_json::json!({}),
        };

        let plan = PackMergePlan {
            actions: vec![crate::pack_merge::PlanAction {
                key: "mods/disabled.jar".into(),
                logical_path: "mods/disabled.jar".into(),
                target_path: "mods/disabled.jar.disabled".into(),
                previous_path: None,
                kind: crate::pack_merge::PlanActionKind::UpdateKeepDisabled,
                base_sha: Some(sha2_256_hex(b"old-content")),
                ours_sha: Some(sha2_256_hex(b"old-content")),
                theirs_sha: Some(sha2_256_hex(b"new-content")),
                enabled: false,
                mod_id: None,
            }],
            conflicts: vec![],
            all_keys: vec!["mods/disabled.jar".into()],
            baseline_missing: false,
        };
        let theirs = MrpackTheirs {
            files: vec![],
            unverified: BTreeSet::new(),
            converged: BTreeSet::new(),
            provisional_hash: BTreeMap::new(),
            download_urls: BTreeMap::new(),
            mod_ids: HashMap::new(),
            files_needing_download: 0,
            download_bytes: 0,
            size_unknown_count: 0,
            pack_name: "test".into(),
            pack_version_id: None,
        };

        let reconciled =
            reconcile_manifest(&old, &plan, &BTreeMap::new(), &theirs, &staged).unwrap();
        let entry = reconciled
            .mods
            .iter()
            .find(|m| m.filename == "disabled.jar")
            .expect("disabled entry present");
        assert!(!entry.enabled, "disabled pack mod stays disabled");
        assert_eq!(entry.sha256, sha2_256_hex(b"new-content"));
        assert!(entry.pack_managed);
    }

    fn bare_manifest(mods: Vec<crate::models::InstalledMod>) -> crate::models::InstanceManifest {
        crate::models::InstanceManifest {
            manifest_version: crate::models::CURRENT_MANIFEST_VERSION,
            instance_id: "inst".into(),
            name: "Inst".into(),
            created_from_pack: None,
            pack_origin: None,
            minecraft_version: "1.21".into(),
            loader: "fabric".into(),
            loader_version: "0.15.0".into(),
            is_locked: false,
            mods,
            resourcepacks: vec![],
            shaders: vec![],
            datapacks: vec![],
            worlds: vec![],
            user_preferences: serde_json::json!({}),
        }
    }

    fn empty_theirs() -> MrpackTheirs {
        MrpackTheirs::default()
    }

    #[test]
    fn update_pack_refuses_before_touching_anything_when_a_conflict_is_unanswered() {
        // Applying with an unresolved conflict would mean Agora silently
        // picking a side on a file the user edited — the one thing this whole
        // feature exists to avoid.
        let tmp = TempDir::new().unwrap();
        let inst = seed_instance(&tmp, "inst");
        write(&inst, "config/foo.toml", b"user edited this");
        let (conn, _dbtmp) = test_conn();
        seed_instance_row(&conn, "inst");
        crate::db::replace_instance_pack_files(
            &conn,
            "inst",
            &[InstancePackFile {
                relative_path: "config/foo.toml".into(),
                sha256: sha2_256_hex(b"pack original"),
                size: 13,
            }],
        )
        .unwrap();

        // Overrides only, so no jar has to be fetched.
        let mrpack = build_mrpack(
            tmp.path(),
            "new",
            vec![],
            vec![("config/foo.toml", b"pack changed it too".as_slice())],
        );

        let before = crate::snapshot::live_file_index(&inst).unwrap();
        let outcome = update_pack(
            &conn,
            "inst",
            &inst,
            &mrpack,
            &tmp.path().join("staged"),
            &BTreeMap::new(),
            &FakeFetcher {
                map: HashMap::new(),
            },
            true,
        );
        match outcome {
            PackUpdateOutcome::Failed { phase, .. } => assert_eq!(phase, "conflicts"),
            other => panic!("expected a refusal, got {other:?}"),
        }
        assert_eq!(
            crate::snapshot::live_file_index(&inst).unwrap(),
            before,
            "a refusal must not touch the instance"
        );
    }

    #[test]
    fn reconcile_creates_an_entry_the_manifest_had_drifted_out_of() {
        // A manifest missing an entry for a file the pack demonstrably manages
        // is drift that predates this merge. Refusing would make a recoverable
        // inconsistency permanently block every future pack update, so the
        // update falls back to the add path and stamps the same provenance.
        let tmp = TempDir::new().unwrap();
        let staged = tmp.path().join("staged");
        write(&staged, "mods/packmod.jar", b"new-bytes");

        let old = bare_manifest(vec![]);
        let plan = PackMergePlan {
            actions: vec![crate::pack_merge::PlanAction {
                key: "mods/packmod.jar".into(),
                logical_path: "mods/packmod.jar".into(),
                target_path: "mods/packmod.jar".into(),
                previous_path: None,
                kind: crate::pack_merge::PlanActionKind::Update,
                base_sha: None,
                ours_sha: None,
                theirs_sha: None,
                enabled: true,
                mod_id: None,
            }],
            conflicts: vec![],
            all_keys: vec!["mods/packmod.jar".into()],
            baseline_missing: false,
        };

        let out =
            reconcile_manifest(&old, &plan, &BTreeMap::new(), &empty_theirs(), &staged).unwrap();
        assert_eq!(out.mods.len(), 1, "the drifted file gets an entry");
        assert_eq!(out.mods[0].filename, "packmod.jar");
        assert!(out.mods[0].pack_managed, "the pack ships it, so it owns it");
        assert!(
            out.mods[0].version.is_none(),
            "an mrpack index carries no per-file version; inventing one would be fabrication"
        );
    }

    #[test]
    fn reconcile_of_an_empty_plan_returns_the_manifest_unchanged() {
        // The degenerate case: nothing to do must mean nothing changes, not a
        // manifest rebuilt from whatever happens to be lying in the staged tree.
        let tmp = TempDir::new().unwrap();
        let staged = tmp.path().join("staged");
        write(&staged, "mods/stray.jar", b"not in the plan");

        let user_mod = crate::models::InstalledMod {
            filename: "mine.jar".into(),
            registry_id: None,
            modrinth_id: None,
            source: "manual".into(),
            source_url: None,
            version: Some("1.2.3".into()),
            sha256: sha2_256_hex(b"mine"),
            installed_at: "2020-01-01T00:00:00Z".into(),
            java_packages: vec![],
            mod_jar_id: None,
            provided_mod_ids: vec![],
            enabled: false,
            content_type: "mod".into(),
            update_pinned: true,
            pack_managed: false,
            installed_as_dependency: true,
            depends_on: vec![],
            optional_deps: vec![],
            incompatible_deps: vec![],
        };
        let old = bare_manifest(vec![user_mod]);
        let plan = PackMergePlan {
            actions: vec![],
            conflicts: vec![],
            all_keys: vec![],
            baseline_missing: false,
        };

        let out =
            reconcile_manifest(&old, &plan, &BTreeMap::new(), &empty_theirs(), &staged).unwrap();
        assert_eq!(out.mods.len(), old.mods.len());
        let (before, after) = (&old.mods[0], &out.mods[0]);
        assert_eq!(after.filename, before.filename);
        assert_eq!(after.version, before.version);
        assert_eq!(after.installed_at, before.installed_at);
        assert_eq!(after.enabled, before.enabled);
        assert_eq!(after.update_pinned, before.update_pinned);
        assert_eq!(after.pack_managed, before.pack_managed);
        assert_eq!(
            after.installed_as_dependency,
            before.installed_as_dependency
        );
        assert_eq!(after.source, before.source);
    }

    #[test]
    fn reconcile_remove_drops_entry() {
        let tmp = TempDir::new().unwrap();
        let staged = tmp.path().join("staged");
        fs::create_dir_all(&staged).unwrap();

        let keep = crate::models::InstalledMod {
            filename: "keep.jar".into(),
            registry_id: None,
            modrinth_id: None,
            source: "manual".into(),
            source_url: None,
            version: None,
            sha256: sha2_256_hex(b"keep"),
            installed_at: "2020-01-01T00:00:00Z".into(),
            java_packages: vec![],
            mod_jar_id: None,
            provided_mod_ids: vec![],
            enabled: true,
            content_type: "mod".into(),
            update_pinned: false,
            pack_managed: false,
            installed_as_dependency: false,
            depends_on: vec![],
            optional_deps: vec![],
            incompatible_deps: vec![],
        };
        let remove = crate::models::InstalledMod {
            filename: "remove.jar".into(),
            registry_id: None,
            modrinth_id: Some("rem".into()),
            source: "modrinth-pack".into(),
            source_url: None,
            version: None,
            sha256: sha2_256_hex(b"remove-old"),
            installed_at: "2020-01-01T00:00:00Z".into(),
            java_packages: vec![],
            mod_jar_id: None,
            provided_mod_ids: vec![],
            enabled: true,
            content_type: "mod".into(),
            update_pinned: false,
            pack_managed: true,
            installed_as_dependency: false,
            depends_on: vec![],
            optional_deps: vec![],
            incompatible_deps: vec![],
        };
        let old = crate::models::InstanceManifest {
            manifest_version: crate::models::CURRENT_MANIFEST_VERSION,
            instance_id: "inst".into(),
            name: "Inst".into(),
            created_from_pack: None,
            pack_origin: None,
            minecraft_version: "1.21".into(),
            loader: "fabric".into(),
            loader_version: "0.15.0".into(),
            is_locked: false,
            mods: vec![keep.clone(), remove.clone()],
            resourcepacks: vec![],
            shaders: vec![],
            datapacks: vec![],
            worlds: vec![],
            user_preferences: serde_json::json!({}),
        };

        let plan = PackMergePlan {
            actions: vec![
                crate::pack_merge::PlanAction {
                    key: "mods/keep.jar".into(),
                    logical_path: "mods/keep.jar".into(),
                    target_path: "mods/keep.jar".into(),
                    previous_path: None,
                    kind: crate::pack_merge::PlanActionKind::Keep,
                    base_sha: None,
                    ours_sha: None,
                    theirs_sha: None,
                    enabled: true,
                    mod_id: None,
                },
                crate::pack_merge::PlanAction {
                    key: "mods/remove.jar".into(),
                    logical_path: "mods/remove.jar".into(),
                    target_path: "mods/remove.jar".into(),
                    previous_path: None,
                    kind: crate::pack_merge::PlanActionKind::Remove,
                    base_sha: Some(sha2_256_hex(b"remove-old")),
                    ours_sha: Some(sha2_256_hex(b"remove-old")),
                    theirs_sha: None,
                    enabled: true,
                    mod_id: None,
                },
            ],
            conflicts: vec![],
            all_keys: vec!["mods/keep.jar".into(), "mods/remove.jar".into()],
            baseline_missing: false,
        };
        let theirs = MrpackTheirs {
            files: vec![],
            unverified: BTreeSet::new(),
            converged: BTreeSet::new(),
            provisional_hash: BTreeMap::new(),
            download_urls: BTreeMap::new(),
            mod_ids: HashMap::new(),
            files_needing_download: 0,
            download_bytes: 0,
            size_unknown_count: 0,
            pack_name: "test".into(),
            pack_version_id: None,
        };

        let reconciled =
            reconcile_manifest(&old, &plan, &BTreeMap::new(), &theirs, &staged).unwrap();
        assert!(
            reconciled.mods.iter().any(|m| m.filename == "keep.jar"),
            "keep remains"
        );
        assert!(
            !reconciled.mods.iter().any(|m| m.filename == "remove.jar"),
            "Remove drops entry"
        );
        assert_eq!(reconciled.mods.len(), 1);
    }

    #[test]
    fn reconcile_config_paths_do_not_create_manifest_entries() {
        let tmp = TempDir::new().unwrap();
        let staged = tmp.path().join("staged");
        fs::create_dir_all(staged.join("config")).unwrap();
        write(&staged, "config/foo.toml", b"new-config");

        let old = crate::models::InstanceManifest {
            manifest_version: crate::models::CURRENT_MANIFEST_VERSION,
            instance_id: "inst".into(),
            name: "Inst".into(),
            created_from_pack: None,
            pack_origin: None,
            minecraft_version: "1.21".into(),
            loader: "fabric".into(),
            loader_version: "0.15.0".into(),
            is_locked: false,
            mods: vec![],
            resourcepacks: vec![],
            shaders: vec![],
            datapacks: vec![],
            worlds: vec![],
            user_preferences: serde_json::json!({}),
        };
        let plan = PackMergePlan {
            actions: vec![crate::pack_merge::PlanAction {
                key: "config/foo.toml".into(),
                logical_path: "config/foo.toml".into(),
                target_path: "config/foo.toml".into(),
                previous_path: None,
                kind: crate::pack_merge::PlanActionKind::Update,
                base_sha: Some(sha('a')),
                ours_sha: Some(sha('a')),
                theirs_sha: Some(sha('b')),
                enabled: true,
                mod_id: None,
            }],
            conflicts: vec![],
            all_keys: vec!["config/foo.toml".into()],
            baseline_missing: false,
        };
        let theirs = MrpackTheirs {
            files: vec![],
            unverified: BTreeSet::new(),
            converged: BTreeSet::new(),
            provisional_hash: BTreeMap::new(),
            download_urls: BTreeMap::new(),
            mod_ids: HashMap::new(),
            files_needing_download: 0,
            download_bytes: 0,
            size_unknown_count: 0,
            pack_name: "test".into(),
            pack_version_id: None,
        };
        let reconciled =
            reconcile_manifest(&old, &plan, &BTreeMap::new(), &theirs, &staged).unwrap();
        assert!(reconciled.mods.is_empty());
        assert!(reconciled.resourcepacks.is_empty());
        assert_eq!(reconciled.mods.len(), 0);
    }

    #[test]
    fn reconcile_take_theirs_updates_and_keep_ours_preserves() {
        let tmp = TempDir::new().unwrap();
        let staged = tmp.path().join("staged");
        fs::create_dir_all(staged.join("mods")).unwrap();
        write(&staged, "mods/conflict.jar", b"theirs-bytes");

        let old_entry = crate::models::InstalledMod {
            filename: "conflict.jar".into(),
            registry_id: None,
            modrinth_id: Some("conf".into()),
            source: "modrinth-pack".into(),
            source_url: None,
            version: None,
            sha256: sha2_256_hex(b"old-bytes"),
            installed_at: "2024-01-01T00:00:00Z".into(),
            java_packages: vec![],
            mod_jar_id: Some("confid".into()),
            provided_mod_ids: vec![],
            enabled: true,
            content_type: "mod".into(),
            update_pinned: false,
            pack_managed: true,
            installed_as_dependency: false,
            depends_on: vec![],
            optional_deps: vec![],
            incompatible_deps: vec![],
        };
        let old = crate::models::InstanceManifest {
            manifest_version: crate::models::CURRENT_MANIFEST_VERSION,
            instance_id: "inst".into(),
            name: "Inst".into(),
            created_from_pack: None,
            pack_origin: None,
            minecraft_version: "1.21".into(),
            loader: "fabric".into(),
            loader_version: "0.15.0".into(),
            is_locked: false,
            mods: vec![old_entry.clone()],
            resourcepacks: vec![],
            shaders: vec![],
            datapacks: vec![],
            worlds: vec![],
            user_preferences: serde_json::json!({}),
        };

        let conflict = crate::pack_merge::PlanConflict {
            key: "mods/conflict.jar".into(),
            logical_path: "mods/conflict.jar".into(),
            kind: crate::pack_merge::ConflictKind::BothModified,
            base_path: Some("mods/conflict.jar".into()),
            ours_path: Some("mods/conflict.jar".into()),
            theirs_path: Some("mods/conflict.jar".into()),
            base_sha: Some(sha2_256_hex(b"old-bytes")),
            ours_sha: Some(sha2_256_hex(b"ours-bytes")),
            theirs_sha: Some(sha2_256_hex(b"theirs-bytes")),
            message: "both modified".into(),
            mod_id: None,
        };
        let plan = PackMergePlan {
            actions: vec![],
            conflicts: vec![conflict],
            all_keys: vec!["mods/conflict.jar".into()],
            baseline_missing: false,
        };
        let mod_ids: HashMap<String, String> = HashMap::new();
        let download_urls: BTreeMap<String, String> = BTreeMap::new();
        let theirs = MrpackTheirs {
            files: vec![],
            unverified: BTreeSet::new(),
            converged: BTreeSet::new(),
            provisional_hash: BTreeMap::new(),
            download_urls: download_urls.clone(),
            mod_ids: mod_ids.clone(),
            files_needing_download: 0,
            download_bytes: 0,
            size_unknown_count: 0,
            pack_name: "test".into(),
            pack_version_id: None,
        };

        // KeepOurs preserves old sha and pack_managed
        let mut keep_res = BTreeMap::new();
        keep_res.insert("mods/conflict.jar".into(), ConflictResolution::KeepOurs);
        let kept = reconcile_manifest(&old, &plan, &keep_res, &theirs, &staged).unwrap();
        let kept_entry = kept
            .mods
            .iter()
            .find(|m| m.filename == "conflict.jar")
            .unwrap();
        assert_eq!(kept_entry.sha256, sha2_256_hex(b"old-bytes"));
        assert!(kept_entry.pack_managed, "KeepOurs preserves pack_managed");

        // TakeTheirs updates to staged sha
        let mut take_res = BTreeMap::new();
        take_res.insert("mods/conflict.jar".into(), ConflictResolution::TakeTheirs);
        let taken = reconcile_manifest(&old, &plan, &take_res, &theirs, &staged).unwrap();
        let taken_entry = taken
            .mods
            .iter()
            .find(|m| m.filename == "conflict.jar")
            .unwrap();
        assert_eq!(taken_entry.sha256, sha2_256_hex(b"theirs-bytes"));
        assert!(taken_entry.enabled);
    }
}
