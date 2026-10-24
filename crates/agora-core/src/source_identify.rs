//! Finds a download source for mods that were copied in without one.
//!
//! Another launcher's instance gives Agora jar files and nothing about where
//! they came from, so an Agora pack made from it could not be installed
//! anywhere. Modrinth can name a file from its hash, in one batched request,
//! and that gives the mod the Modrinth project id the pack importer needs.
//!
//! Nothing here can fail an import or an export: a mod that cannot be
//! identified (offline, not on Modrinth) simply keeps no source, as before.
//!
//! CurseForge's own project and file ids are deliberately not recorded: the
//! manifest has no field for them, and borrowing `provider` would make update
//! and install code look for a provider that does not exist. A CurseForge id
//! alone does not make a file downloadable anyway (its CDN needs an API key).

use crate::ctx::Ctx;
use crate::error::LauncherResult;
use crate::http_client::{self, ClientCategory, HttpClients};
use crate::lock_manager::LockResource;
use crate::models::{InstalledMod, InstanceManifest};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Hashes per request, so one reply stays a sensible size for a huge setup.
const BATCH: usize = 500;

/// What Modrinth knows about one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModrinthHit {
    pub project_id: String,
    pub version_number: Option<String>,
}

/// Where a mod's file is on disk, and whether it is the `.disabled` copy.
///
/// The manifest names the enabled file (`iris.jar`); a disabled mod is stored
/// as `iris.jar.disabled`. The file for the mod's recorded state is preferred
/// and the other is accepted, so a manifest that drifted from disk still finds
/// its file.
pub fn mod_file_on_disk(mods_dir: &Path, item: &InstalledMod) -> Option<(PathBuf, bool)> {
    let plain = mods_dir.join(&item.filename);
    let disabled = mods_dir.join(format!("{}.disabled", item.filename));
    let candidates = if item.enabled {
        [(plain, false), (disabled, true)]
    } else {
        [(disabled, true), (plain, false)]
    };
    candidates
        .into_iter()
        .find(|(path, _)| std::fs::symlink_metadata(path).is_ok())
}

fn sha1_of_file(path: &Path) -> Option<String> {
    use sha1::Digest;
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = sha1::Sha1::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Some(hex::encode(hasher.finalize()))
}

/// Reads the reply of `POST /v2/version_files`: an object keyed by the
/// requested hash, each value a version carrying its `project_id`.
pub fn parse_version_files(body: &serde_json::Value) -> HashMap<String, ModrinthHit> {
    let mut hits = HashMap::new();
    let Some(map) = body.as_object() else {
        return hits;
    };
    for (hash, version) in map {
        let Some(project_id) = version
            .get("project_id")
            .and_then(|v| v.as_str())
            .filter(|v| !v.trim().is_empty())
        else {
            continue;
        };
        hits.insert(
            hash.to_ascii_lowercase(),
            ModrinthHit {
                project_id: project_id.to_string(),
                version_number: version
                    .get("version_number")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            },
        );
    }
    hits
}

/// One `POST /v2/version_files` for `hashes`, as the parsed reply. `None` when
/// Modrinth cannot be asked or its reply is not JSON. Both callers then treat
/// every file as unknown: a mod stays sourceless, a file is bundled.
async fn query_version_files(
    clients: &HttpClients,
    hashes: &[String],
) -> Option<serde_json::Value> {
    let body = serde_json::json!({ "hashes": hashes, "algorithm": "sha1" });
    let response = match http_client::checked_post_json(
        clients,
        ClientCategory::Modrinth,
        "https://api.modrinth.com/v2/version_files",
        &body,
        &[("Accept".to_string(), "application/json".to_string())],
    )
    .await
    {
        Ok(response) if response.status().is_success() => response,
        _ => return None,
    };
    let bytes = http_client::checked_response_bytes(response, ClientCategory::Modrinth)
        .await
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

async fn lookup_batch(clients: &HttpClients, hashes: &[String]) -> HashMap<String, ModrinthHit> {
    query_version_files(clients, hashes)
        .await
        .map_or_else(HashMap::new, |value| parse_version_files(&value))
}

/// A file Modrinth hosts, identified by the hash of the copy on disk, so the
/// bytes at `url` are the bytes the user has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostedFile {
    pub url: String,
    pub sha1: String,
    pub sha512: String,
    pub size: u64,
}

/// Folders of content the user chose, as opposed to configs and settings,
/// that an exported `.mrpack` carries besides the mods.
pub const PACK_CONTENT_DIRS: [&str; 3] = ["resourcepacks", "shaderpacks", "datapacks"];

/// Reads the reply of `POST /v2/version_files` for the hosted file behind each
/// requested hash: the version's file whose own SHA-1 is that hash, with its
/// Modrinth CDN link. Anything else (another host, missing hashes) is skipped.
pub fn parse_hosted_files(body: &serde_json::Value) -> HashMap<String, HostedFile> {
    let mut hosted = HashMap::new();
    let Some(map) = body.as_object() else {
        return hosted;
    };
    for (hash, version) in map {
        let hash = hash.to_ascii_lowercase();
        let Some(files) = version.get("files").and_then(|f| f.as_array()) else {
            continue;
        };
        for file in files {
            let text = |value: Option<&serde_json::Value>| {
                value
                    .and_then(|v| v.as_str())
                    .map(|v| v.trim().to_ascii_lowercase())
                    .filter(|v| !v.is_empty())
            };
            let sha1 = text(file.pointer("/hashes/sha1"));
            let sha512 = text(file.pointer("/hashes/sha512"));
            let url = file
                .get("url")
                .and_then(|v| v.as_str())
                .filter(|u| u.starts_with("https://cdn.modrinth.com/"));
            if let (Some(sha1), Some(sha512), Some(url)) = (sha1, sha512, url) {
                if sha1 == hash {
                    hosted.insert(
                        hash.clone(),
                        HostedFile {
                            url: url.to_string(),
                            sha1,
                            sha512,
                            size: file.get("size").and_then(|v| v.as_u64()).unwrap_or(0),
                        },
                    );
                    break;
                }
            }
        }
    }
    hosted
}

async fn lookup_hosted_batch(
    clients: &HttpClients,
    hashes: &[String],
) -> HashMap<String, HostedFile> {
    query_version_files(clients, hashes)
        .await
        .map_or_else(HashMap::new, |value| parse_hosted_files(&value))
}

/// Which resource packs, shader packs and datapacks in `instance_dir` Modrinth hosts,
/// keyed by the SHA-1 of the file on disk. Never fails: offline, or a file
/// Modrinth does not know, simply leaves it out of the map (the export then
/// carries the file itself).
pub async fn find_hosted_pack_content(instance_dir: &Path) -> HashMap<String, HostedFile> {
    // Reading and hashing the files is blocking I/O, so it runs off the runtime.
    // A failed task means no hashes, so every file is bundled.
    let dir = instance_dir.to_path_buf();
    let Ok(mut hashes) = tokio::task::spawn_blocking(move || pack_content_hashes(&dir)).await
    else {
        return HashMap::new();
    };
    hashes.sort();
    hashes.dedup();
    if hashes.is_empty() {
        return HashMap::new();
    }
    let Ok(clients) = HttpClients::new() else {
        return HashMap::new();
    };
    let mut hosted = HashMap::new();
    for chunk in hashes.chunks(BATCH) {
        hosted.extend(lookup_hosted_batch(&clients, chunk).await);
    }
    hosted
}

/// SHA-1 of every file in the pack-content folders of `instance_dir`. Blocking
/// reads: call it from `spawn_blocking`.
fn pack_content_hashes(instance_dir: &Path) -> Vec<String> {
    let mut hashes: Vec<String> = Vec::new();
    for dir in PACK_CONTENT_DIRS {
        let Ok(read) = std::fs::read_dir(instance_dir.join(dir)) else {
            continue;
        };
        for entry in read.flatten() {
            let is_file = entry.file_type().map(|t| t.is_file()).unwrap_or(false);
            if !is_file {
                continue;
            }
            if let Some(hash) = sha1_of_file(&entry.path()) {
                hashes.push(hash);
            }
        }
    }
    hashes
}

/// Whether a mod has a route a pack could install it from.
pub fn has_source(item: &InstalledMod) -> bool {
    let non_empty = |value: &Option<String>| value.as_deref().is_some_and(|v| !v.trim().is_empty());
    non_empty(&item.registry_id) || non_empty(&item.modrinth_id) || item.provider.is_some()
}

/// Sets `modrinth_id` on every mod that has no source and whose file Modrinth
/// recognises by hash. Returns how many were identified. Never fails: any
/// network or read problem leaves the mods as they were.
pub async fn identify_mods_on_modrinth(instance_dir: &Path, mods: &mut [InstalledMod]) -> usize {
    // Hashing reads every sourceless jar, so it runs off the runtime.
    let dir = instance_dir.to_path_buf();
    let snapshot = mods.to_vec();
    let Ok(pending) = tokio::task::spawn_blocking(move || pending_hashes(&dir, &snapshot)).await
    else {
        return 0;
    };
    if pending.is_empty() {
        return 0;
    }
    let Ok(clients) = HttpClients::new() else {
        return 0;
    };
    let mut hits: HashMap<String, ModrinthHit> = HashMap::new();
    let mut unique: Vec<String> = pending.iter().map(|(_, hash)| hash.clone()).collect();
    unique.sort();
    unique.dedup();
    for chunk in unique.chunks(BATCH) {
        hits.extend(lookup_batch(&clients, chunk).await);
    }
    apply_hits(mods, &pending, &hits)
}

/// `(index, sha1)` for every mod that has no source and whose file is on disk.
/// Blocking reads: call it from `spawn_blocking`.
fn pending_hashes(instance_dir: &Path, mods: &[InstalledMod]) -> Vec<(usize, String)> {
    let mods_dir = instance_dir.join("mods");
    mods.iter()
        .enumerate()
        .filter(|(_, item)| !has_source(item) && item.content_type == "mod")
        .filter_map(|(index, item)| {
            let (path, _) = mod_file_on_disk(&mods_dir, item)?;
            Some((index, sha1_of_file(&path)?))
        })
        .collect()
}

/// Writes the Modrinth ids in `found` back into the instance manifest, so the
/// next export and sourceless check do not hash and ask again for those mods.
/// Takes the instance lock and re-reads the manifest, so only mods that still
/// have no source are filled in and nothing else changes.
pub fn record_identified_mods(
    ctx: &Ctx,
    instance_id: &str,
    found: &[InstalledMod],
) -> LauncherResult<()> {
    if found.is_empty() {
        return Ok(());
    }
    let sanitized = crate::paths::sanitize_id(instance_id);
    let _guard = ctx.lock_manager.acquire(
        LockResource::Instance(sanitized.clone()),
        "record_modrinth_ids",
    )?;
    let manifest_path = ctx.paths.instance_manifest(&sanitized)?;
    if !manifest_path.exists() {
        return Ok(());
    }
    let mut manifest = crate::helpers::read_manifest(&manifest_path)?;
    let mut changed = false;
    for item in manifest.mods.iter_mut() {
        if has_source(item) || item.content_type != "mod" {
            continue;
        }
        let Some(hit) = found.iter().find(|hit| hit.filename == item.filename) else {
            continue;
        };
        let Some(project_id) = hit.modrinth_id.as_deref() else {
            continue;
        };
        item.modrinth_id = Some(project_id.to_string());
        if item.version.is_none() {
            item.version = hit.version.clone();
        }
        changed = true;
    }
    if changed {
        crate::helpers::atomic_write_manifest(&manifest_path, &manifest)?;
    }
    Ok(())
}

/// Writes the identified project ids onto the mods. Split from the network
/// call so it can be tested with a recorded reply.
pub fn apply_hits(
    mods: &mut [InstalledMod],
    pending: &[(usize, String)],
    hits: &HashMap<String, ModrinthHit>,
) -> usize {
    let mut identified = 0;
    for (index, hash) in pending {
        let Some(hit) = hits.get(hash) else { continue };
        let item = &mut mods[*index];
        item.modrinth_id = Some(hit.project_id.clone());
        if item.version.is_none() {
            item.version = hit.version_number.clone();
        }
        identified += 1;
    }
    identified
}

/// Names of the mods in `manifest` that no pack could install anywhere.
pub fn sourceless_mod_names(manifest: &InstanceManifest) -> Vec<String> {
    manifest
        .mods
        .iter()
        .filter(|item| item.content_type == "mod" && !has_source(item))
        .map(|item| item.filename.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(filename: &str, enabled: bool) -> InstalledMod {
        serde_json::from_value(serde_json::json!({
            "filename": filename, "registry_id": null, "modrinth_id": null,
            "source": "imported_curseforge", "version": null, "sha256": "00",
            "installed_at": "t", "enabled": enabled,
        }))
        .unwrap()
    }

    #[test]
    fn a_disabled_mod_is_found_under_its_disabled_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("iris.jar.disabled"), b"x").unwrap();
        let (path, disabled) = mod_file_on_disk(dir.path(), &item("iris.jar", false)).unwrap();
        assert!(disabled);
        assert!(path.ends_with("iris.jar.disabled"));
        // A manifest that says enabled still finds the file that is there.
        assert!(mod_file_on_disk(dir.path(), &item("iris.jar", true)).is_some());
        assert!(mod_file_on_disk(dir.path(), &item("gone.jar", true)).is_none());
    }

    #[test]
    fn modrinth_reply_gives_project_ids_by_hash() {
        let reply = serde_json::json!({
            "AAAA": { "project_id": "YL57xq9U", "id": "v1", "version_number": "1.8.0" },
            "bbbb": { "id": "v2" }
        });
        let hits = parse_version_files(&reply);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits["aaaa"].project_id, "YL57xq9U");
    }

    #[test]
    fn identified_mods_gain_a_modrinth_id_and_unknown_ones_stay_sourceless() {
        let mut mods = vec![item("iris.jar", true), item("homebrew.jar", true)];
        let pending = vec![(0, "aaaa".to_string()), (1, "cccc".to_string())];
        let mut hits = HashMap::new();
        hits.insert(
            "aaaa".to_string(),
            ModrinthHit {
                project_id: "YL57xq9U".into(),
                version_number: Some("1.8.0".into()),
            },
        );
        assert_eq!(apply_hits(&mut mods, &pending, &hits), 1);
        assert_eq!(mods[0].modrinth_id.as_deref(), Some("YL57xq9U"));
        assert!(has_source(&mods[0]));
        assert!(!has_source(&mods[1]));
    }

    #[test]
    fn sourceless_names_lists_only_mods_nothing_can_install() {
        let mut manifest: InstanceManifest = serde_json::from_value(serde_json::json!({
            "instance_id": "i", "name": "I", "minecraft_version": "1.21",
            "loader": "fabric", "loader_version": "0.16", "is_locked": false, "mods": []
        }))
        .unwrap();
        let mut known = item("sodium.jar", true);
        known.modrinth_id = Some("AANobbMI".into());
        manifest.mods = vec![known, item("homebrew.jar", true)];
        assert_eq!(sourceless_mod_names(&manifest), vec!["homebrew.jar"]);
    }

    /// An instance on disk with one known mod, one copied-in mod Modrinth
    /// will know, and one it will not. Returns the context and instance dir.
    fn instance_on_disk(root: &Path) -> (crate::ctx::CoreContext, PathBuf) {
        let ctx = crate::ctx::CoreContext::for_testing(root.to_path_buf());
        let dir = ctx.paths.instance_dir("inst").unwrap();
        std::fs::create_dir_all(dir.join("mods")).unwrap();
        std::fs::write(dir.join("mods").join("iris.jar"), b"iris bytes").unwrap();
        std::fs::write(dir.join("mods").join("homebrew.jar"), b"own bytes").unwrap();
        let mut known = item("sodium.jar", true);
        known.modrinth_id = Some("AANobbMI".into());
        let manifest: InstanceManifest = serde_json::from_value(serde_json::json!({
            "instance_id": "inst", "name": "I", "minecraft_version": "1.21",
            "loader": "fabric", "loader_version": "0.16", "is_locked": false, "mods": []
        }))
        .unwrap();
        let manifest = InstanceManifest {
            mods: vec![known, item("iris.jar", true), item("homebrew.jar", true)],
            ..manifest
        };
        let path = ctx.paths.instance_manifest("inst").unwrap();
        crate::helpers::atomic_write_manifest(&path, &manifest).unwrap();
        (ctx, dir)
    }

    fn reload(ctx: &crate::ctx::CoreContext) -> InstanceManifest {
        let path = ctx.paths.instance_manifest("inst").unwrap();
        crate::helpers::read_manifest(&path).unwrap()
    }

    /// What `identify_mods_on_modrinth` does after its network reply, with the
    /// reply supplied: hash, match `iris.jar` only, then record.
    fn identify_iris_offline(ctx: &crate::ctx::CoreContext, dir: &Path) -> Vec<InstalledMod> {
        let mut mods = reload(ctx).mods;
        let sourceless_before: Vec<bool> = mods.iter().map(|m| !has_source(m)).collect();
        let pending = pending_hashes(dir, &mods);
        let iris_hash = pending
            .iter()
            .find(|(index, _)| mods[*index].filename == "iris.jar")
            .map(|(_, hash)| hash.clone())
            .unwrap();
        let mut hits = HashMap::new();
        hits.insert(
            iris_hash,
            ModrinthHit {
                project_id: "YL57xq9U".into(),
                version_number: Some("1.8.0".into()),
            },
        );
        apply_hits(&mut mods, &pending, &hits);
        let found: Vec<InstalledMod> = mods
            .iter()
            .zip(sourceless_before)
            .filter(|(m, was)| *was && m.modrinth_id.is_some())
            .map(|(m, _)| m.clone())
            .collect();
        record_identified_mods(ctx, "inst", &found).unwrap();
        found
    }

    #[test]
    fn an_identified_mod_is_written_back_and_not_hashed_again() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, dir) = instance_on_disk(tmp.path());

        identify_iris_offline(&ctx, &dir);

        let after = reload(&ctx);
        let iris = after
            .mods
            .iter()
            .find(|m| m.filename == "iris.jar")
            .unwrap();
        assert_eq!(iris.modrinth_id.as_deref(), Some("YL57xq9U"));
        assert_eq!(iris.version.as_deref(), Some("1.8.0"));
        // Only the mod Modrinth did not know still needs a hash and a question.
        let still_pending: Vec<&str> = pending_hashes(&dir, &after.mods)
            .iter()
            .map(|(index, _)| after.mods[*index].filename.as_str())
            .collect();
        assert_eq!(still_pending, vec!["homebrew.jar"]);
        // Nothing else in the manifest moved: the name, the known mod's id.
        assert_eq!(after.name, "I");
        let sodium = after
            .mods
            .iter()
            .find(|m| m.filename == "sodium.jar")
            .unwrap();
        assert_eq!(sodium.modrinth_id.as_deref(), Some("AANobbMI"));
    }

    #[test]
    fn recording_never_overwrites_a_source_that_landed_meanwhile() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, _dir) = instance_on_disk(tmp.path());
        // Another writer gave iris a source after the lookup began.
        let path = ctx.paths.instance_manifest("inst").unwrap();
        let mut manifest = reload(&ctx);
        for item in manifest.mods.iter_mut() {
            if item.filename == "iris.jar" {
                item.modrinth_id = Some("OTHERid".into());
            }
        }
        crate::helpers::atomic_write_manifest(&path, &manifest).unwrap();
        let mut found = item("iris.jar", true);
        found.modrinth_id = Some("YL57xq9U".into());

        record_identified_mods(&ctx, "inst", &[found]).unwrap();

        let iris = reload(&ctx)
            .mods
            .into_iter()
            .find(|m| m.filename == "iris.jar")
            .unwrap();
        assert_eq!(iris.modrinth_id.as_deref(), Some("OTHERid"));
    }

    #[test]
    fn recording_nothing_or_into_a_missing_manifest_changes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, _dir) = instance_on_disk(tmp.path());
        let before = std::fs::read(ctx.paths.instance_manifest("inst").unwrap()).unwrap();

        record_identified_mods(&ctx, "inst", &[]).unwrap();
        assert_eq!(
            std::fs::read(ctx.paths.instance_manifest("inst").unwrap()).unwrap(),
            before
        );

        let missing = tempfile::tempdir().unwrap();
        let ctx2 = crate::ctx::CoreContext::for_testing(missing.path().to_path_buf());
        let mut found = item("iris.jar", true);
        found.modrinth_id = Some("YL57xq9U".into());
        record_identified_mods(&ctx2, "nobody", &[found]).unwrap();
        assert!(!ctx2.paths.instance_manifest("nobody").unwrap().exists());
    }
}
