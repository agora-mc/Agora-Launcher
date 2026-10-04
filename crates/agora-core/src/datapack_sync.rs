//! Data packs reach worlds by sync (MASTER_SPEC §20.6).
//!
//! Minecraft only loads data packs from `<instance>/saves/<world>/datapacks/`.
//! The instance's own `datapacks/` folder (the one Agora installs into and
//! `manifest.datapacks` describes) is never read by the game. This module
//! copies each enabled data pack into every in-scope world, and removes what it
//! placed earlier when the pack is disabled, removed or out of scope.
//!
//! Safety rule: Agora only ever touches files it placed. Every world keeps a
//! record, `<world>/datapacks/.agora-managed.json`, of the files Agora copied
//! there and their SHA-256. A file is removed or overwritten only when it is in
//! that record *and* still has the recorded hash. A same-named file the user put
//! there is left alone and reported as a warning.
//!
//! The record lives beside the files it describes (not under the instance's
//! `.agora/`) so that restoring a full snapshot of `saves/` brings files and
//! record back together.

use crate::ctx::Ctx;
use crate::error::{LauncherError, LauncherResult};
use crate::lock_manager::LockResource;
use crate::models::{InstalledMod, InstanceManifest};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::Path;

/// Per-world record of the files Agora placed.
pub const MANAGED_RECORD_FILE: &str = ".agora-managed.json";

/// `manifest.user_preferences` key holding `{ <scope key>: [world folder, ...] }`.
///
/// Absent entry = "all worlds" (the default), so existing manifests need no
/// migration. It lives in `user_preferences`, like the other per-instance
/// settings (icon, wrapper command), rather than as a new `InstalledMod` field,
/// which would have to be added to every struct literal in the workspace.
const SCOPES_KEY: &str = "datapack_world_scopes";

const TEMP_SUFFIX: &str = ".agora-tmp";

/// Which worlds a data pack is synced into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatapackScope {
    AllWorlds,
    /// Specific world folder names. An empty list means no world.
    Worlds(Vec<String>),
}

impl DatapackScope {
    fn includes(&self, world: &str) -> bool {
        match self {
            DatapackScope::AllWorlds => true,
            DatapackScope::Worlds(list) => list.iter().any(|name| name == world),
        }
    }
}

/// What the UI shows for a data pack row.
#[derive(Debug, Clone, Serialize)]
pub struct DatapackWorldStatus {
    pub all_worlds: bool,
    /// Chosen world folders; only meaningful when `all_worlds` is false.
    pub selected_worlds: Vec<String>,
    /// Every existing world in the instance (folders under `saves/` with a
    /// `level.dat`).
    pub available_worlds: Vec<String>,
    /// How many existing worlds the pack is synced into.
    pub covered_worlds: usize,
}

/// Outcome of one sync pass. Failures are warnings; sync never blocks anything.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DatapackSyncReport {
    pub worlds: usize,
    pub copied: usize,
    pub removed: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ManagedRecord {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    files: Vec<ManagedFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ManagedFile {
    filename: String,
    sha256: String,
}

/// Existing worlds: folders under `saves/` that contain a `level.dat`, sorted.
pub fn list_worlds(instance_dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(instance_dir.join("saves")) else {
        return Vec::new();
    };
    let mut worlds: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().join("level.dat").is_file())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    worlds.sort_by_key(|name| name.to_lowercase());
    worlds
}

/// Stable key for a data pack's scope: its catalog identity when it has one, so
/// the choice survives an update that renames the file; otherwise the filename.
fn scope_key(entry: &InstalledMod) -> String {
    entry
        .registry_id
        .clone()
        .or_else(|| entry.modrinth_id.clone())
        .unwrap_or_else(|| entry.filename.clone())
}

pub fn scope_of(manifest: &InstanceManifest, entry: &InstalledMod) -> DatapackScope {
    manifest
        .user_preferences
        .get(SCOPES_KEY)
        .and_then(|scopes| scopes.get(scope_key(entry)))
        .and_then(|value| value.as_array())
        .map(|worlds| {
            DatapackScope::Worlds(
                worlds
                    .iter()
                    .filter_map(|world| world.as_str().map(str::to_owned))
                    .collect(),
            )
        })
        .unwrap_or(DatapackScope::AllWorlds)
}

/// Record the scope for the data pack `filename` in the manifest (in memory).
pub fn set_scope(
    manifest: &mut InstanceManifest,
    filename: &str,
    scope: DatapackScope,
) -> LauncherResult<()> {
    let Some(entry) = manifest
        .datapacks
        .iter()
        .find(|entry| entry.filename == filename)
    else {
        return Err(LauncherError::Generic {
            code: "ERR_DATAPACK_NOT_FOUND".into(),
            message: format!("'{filename}' is not an installed data pack of this instance."),
        });
    };
    let key = scope_key(entry);
    let live_keys: BTreeSet<String> = manifest.datapacks.iter().map(scope_key).collect();
    if !manifest.user_preferences.is_object() {
        manifest.user_preferences = serde_json::json!({});
    }
    let preferences = manifest
        .user_preferences
        .as_object_mut()
        .ok_or(LauncherError::InstanceCreateFailed)?;
    let mut scopes = preferences
        .get(SCOPES_KEY)
        .and_then(|value| value.as_object())
        .cloned()
        .unwrap_or_default();
    // Drop choices for data packs that are no longer installed.
    scopes.retain(|stale_key, _| live_keys.contains(stale_key));
    match scope {
        DatapackScope::AllWorlds => {
            scopes.remove(&key);
        }
        DatapackScope::Worlds(worlds) => {
            let worlds: BTreeSet<String> = worlds.into_iter().collect();
            scopes.insert(key, serde_json::json!(worlds));
        }
    }
    if scopes.is_empty() {
        preferences.remove(SCOPES_KEY);
    } else {
        preferences.insert(SCOPES_KEY.into(), serde_json::Value::Object(scopes));
    }
    Ok(())
}

pub fn world_status(
    manifest: &InstanceManifest,
    entry: &InstalledMod,
    available_worlds: &[String],
) -> DatapackWorldStatus {
    let scope = scope_of(manifest, entry);
    let covered_worlds = available_worlds
        .iter()
        .filter(|world| scope.includes(world))
        .count();
    let (all_worlds, selected_worlds) = match scope {
        DatapackScope::AllWorlds => (true, Vec::new()),
        DatapackScope::Worlds(worlds) => (false, worlds),
    };
    DatapackWorldStatus {
        all_worlds,
        selected_worlds,
        available_worlds: available_worlds.to_vec(),
        covered_worlds,
    }
}

fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name != MANAGED_RECORD_FILE
        && !name.ends_with(TEMP_SUFFIX)
        && !name.contains(['/', '\\', ':'])
        && !name.contains('\0')
}

fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

struct Desired {
    source: std::path::PathBuf,
    sha256: String,
    scope: DatapackScope,
}

/// Copy `source` to `dest` through a temp file in the same directory, checking
/// the hash before the rename so a torn or corrupted copy is never exposed.
fn atomic_copy_verified(source: &Path, dest: &Path, expected_sha256: &str) -> Result<(), String> {
    let name = dest
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temp = dest.with_file_name(format!(".{name}{TEMP_SUFFIX}"));
    let result = (|| -> Result<(), String> {
        std::fs::copy(source, &temp).map_err(|error| format!("copy failed: {error}"))?;
        let copied = hash_file(&temp).map_err(|error| format!("verification failed: {error}"))?;
        if copied != expected_sha256 {
            return Err("the copy did not match the source data pack".into());
        }
        std::fs::rename(&temp, dest).map_err(|error| format!("could not move into place: {error}"))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

fn read_record(path: &Path, warnings: &mut Vec<String>, world: &str) -> BTreeMap<String, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return BTreeMap::new(),
        Err(error) => {
            warnings.push(format!(
                "World '{world}': could not read Agora's data pack record ({error}); \
                 nothing was removed there."
            ));
            return BTreeMap::new();
        }
    };
    match serde_json::from_str::<ManagedRecord>(&text) {
        Ok(record) => record
            .files
            .into_iter()
            .filter(|file| safe_name(&file.filename))
            .map(|file| (file.filename, file.sha256))
            .collect(),
        Err(error) => {
            warnings.push(format!(
                "World '{world}': Agora's data pack record is unreadable ({error}); \
                 data packs already there are treated as yours and left alone."
            ));
            BTreeMap::new()
        }
    }
}

fn write_record(path: &Path, files: &BTreeMap<String, String>) -> Result<(), String> {
    if files.is_empty() {
        return match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        };
    }
    let record = ManagedRecord {
        version: 1,
        files: files
            .iter()
            .map(|(filename, sha256)| ManagedFile {
                filename: filename.clone(),
                sha256: sha256.clone(),
            })
            .collect(),
    };
    let text = serde_json::to_string_pretty(&record).map_err(|error| error.to_string())?;
    let temp = path.with_file_name(format!("{MANAGED_RECORD_FILE}{TEMP_SUFFIX}"));
    std::fs::write(&temp, text).map_err(|error| error.to_string())?;
    std::fs::rename(&temp, path).map_err(|error| {
        let _ = std::fs::remove_file(&temp);
        error.to_string()
    })
}

fn sync_world(
    saves: &Path,
    world: &str,
    desired: &BTreeMap<String, Desired>,
    unreadable: &BTreeSet<String>,
    report: &mut DatapackSyncReport,
) {
    let datapacks_dir = saves.join(world).join("datapacks");
    let record_path = datapacks_dir.join(MANAGED_RECORD_FILE);
    let previous = read_record(&record_path, &mut report.warnings, world);
    let mut next: BTreeMap<String, String> = BTreeMap::new();

    // 1. Remove what Agora placed and no longer wants here.
    for (name, recorded_sha) in &previous {
        let wanted_here = desired
            .get(name)
            .is_some_and(|pack| pack.scope.includes(world));
        if wanted_here {
            continue;
        }
        if unreadable.contains(name) {
            // The source could not be read this time; do not treat that as a
            // removal. Keep the record and try again at the next sync.
            next.insert(name.clone(), recorded_sha.clone());
            continue;
        }
        let dest = datapacks_dir.join(name);
        match hash_file(&dest) {
            Ok(current) if &current == recorded_sha => match std::fs::remove_file(&dest) {
                Ok(()) => report.removed += 1,
                Err(error) => {
                    report.warnings.push(format!(
                        "World '{world}': could not remove data pack '{name}' ({error}); \
                         will retry at the next sync."
                    ));
                    next.insert(name.clone(), recorded_sha.clone());
                }
            },
            Ok(_) => report.warnings.push(format!(
                "World '{world}': data pack '{name}' was changed after Agora added it, \
                 so it was left in place."
            )),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                report.warnings.push(format!(
                    "World '{world}': could not check data pack '{name}' ({error}); left in place."
                ));
                next.insert(name.clone(), recorded_sha.clone());
            }
        }
    }

    // 2. Add or refresh what is wanted here.
    for (name, pack) in desired {
        if !pack.scope.includes(world) {
            continue;
        }
        let dest = datapacks_dir.join(name);
        let recorded = previous.get(name);
        let place = |report: &mut DatapackSyncReport| -> bool {
            if let Err(error) = std::fs::create_dir_all(&datapacks_dir) {
                report.warnings.push(format!(
                    "World '{world}': could not create its datapacks folder ({error})."
                ));
                return false;
            }
            match atomic_copy_verified(&pack.source, &dest, &pack.sha256) {
                Ok(()) => {
                    report.copied += 1;
                    true
                }
                Err(error) => {
                    report.warnings.push(format!(
                        "World '{world}': could not add data pack '{name}' ({error})."
                    ));
                    false
                }
            }
        };
        match hash_file(&dest) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if place(report) {
                    next.insert(name.clone(), pack.sha256.clone());
                }
            }
            Err(error) => report.warnings.push(format!(
                "World '{world}': could not check data pack '{name}' ({error})."
            )),
            Ok(current) if current == pack.sha256 => {
                // Already the right content. Keep managing it only if Agora put
                // it there; an identical file the user placed stays theirs.
                if recorded.is_some() {
                    next.insert(name.clone(), pack.sha256.clone());
                }
            }
            Ok(current) if recorded == Some(&current) => {
                // Agora's own unmodified copy of an older version: replace it.
                if place(report) {
                    next.insert(name.clone(), pack.sha256.clone());
                } else {
                    next.insert(name.clone(), current);
                }
            }
            Ok(_) => report.warnings.push(format!(
                "World '{world}' already has a different data pack named '{name}'. \
                 Agora left it untouched; rename or remove it to let Agora manage this one."
            )),
        }
    }

    if next != previous {
        let result = if next.is_empty() && !datapacks_dir.is_dir() {
            Ok(())
        } else {
            write_record(&record_path, &next)
        };
        if let Err(error) = result {
            report.warnings.push(format!(
                "World '{world}': could not update Agora's data pack record ({error})."
            ));
        }
    }
}

/// Bring every existing world in line with the instance's enabled data packs.
///
/// Idempotent and cheap when nothing changed. Never fails: problems are
/// returned as warnings. The caller must hold the instance lock (or be the
/// launch pipeline, which does).
pub fn sync_instance_datapacks(
    instance_dir: &Path,
    manifest: &InstanceManifest,
) -> DatapackSyncReport {
    let mut report = DatapackSyncReport::default();
    let worlds = list_worlds(instance_dir);
    report.worlds = worlds.len();
    if worlds.is_empty() {
        return report;
    }

    let mut desired: BTreeMap<String, Desired> = BTreeMap::new();
    let mut unreadable: BTreeSet<String> = BTreeSet::new();
    for entry in manifest.datapacks.iter().filter(|entry| entry.enabled) {
        if !safe_name(&entry.filename) {
            continue;
        }
        let source = instance_dir.join("datapacks").join(&entry.filename);
        match hash_file(&source) {
            Ok(sha256) => {
                desired.insert(
                    entry.filename.clone(),
                    Desired {
                        source,
                        sha256,
                        scope: scope_of(manifest, entry),
                    },
                );
            }
            // Enabled in the manifest but the file is gone: nothing to place.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                report.warnings.push(format!(
                    "Could not read data pack '{}' ({error}); worlds were not changed for it.",
                    entry.filename
                ));
                unreadable.insert(entry.filename.clone());
            }
        }
    }

    let saves = instance_dir.join("saves");
    for world in &worlds {
        sync_world(&saves, world, &desired, &unreadable, &mut report);
    }
    report
}

/// Sync one instance by id, for the explicit "sync now" action. Refuses while
/// the game is running (its worlds hold the data pack files open).
pub fn sync_instance(ctx: &Ctx, instance_id: &str) -> LauncherResult<DatapackSyncReport> {
    let id = crate::paths::sanitize_id(instance_id);
    let _guard = ctx
        .lock_manager
        .acquire(LockResource::Instance(id.clone()), "sync_datapacks")?;
    crate::instance_runtime::check_idle(&ctx.paths, &id)?;
    let dir = ctx.paths.instance_dir(&id)?;
    let manifest = crate::helpers::read_manifest(&ctx.paths.instance_manifest(&id)?)?;
    Ok(sync_instance_datapacks(&dir, &manifest))
}

/// Change which worlds a data pack goes to (`None` = all worlds), then sync.
///
/// Allowed on pack-managed (locked) instances: the scope is a per-instance
/// preference and does not change the instance's declared content. If the game
/// is running the choice is saved and applied at the next sync instead.
pub fn set_world_scope(
    ctx: &Ctx,
    instance_id: &str,
    filename: &str,
    worlds: Option<Vec<String>>,
) -> LauncherResult<DatapackSyncReport> {
    let id = crate::paths::sanitize_id(instance_id);
    let _guard = ctx
        .lock_manager
        .acquire(LockResource::Instance(id.clone()), "set_datapack_scope")?;
    let dir = ctx.paths.instance_dir(&id)?;
    let manifest_path = ctx.paths.instance_manifest(&id)?;
    let mut manifest = crate::helpers::read_manifest(&manifest_path)?;
    let scope = match worlds {
        None => DatapackScope::AllWorlds,
        Some(list) => DatapackScope::Worlds(list),
    };
    set_scope(&mut manifest, filename, scope)?;
    crate::helpers::atomic_write_manifest(&manifest_path, &manifest)?;
    let _ = crate::snapshot::mark_instance_mutated(&dir);
    if crate::instance_runtime::check_idle(&ctx.paths, &id).is_err() {
        return Ok(DatapackSyncReport {
            warnings: vec![
                "Minecraft is running; the new world choice will be applied at the next sync \
                 or launch."
                    .into(),
            ],
            ..Default::default()
        });
    }
    Ok(sync_instance_datapacks(&dir, &manifest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn pack(filename: &str) -> InstalledMod {
        serde_json::from_value(serde_json::json!({
            "filename": filename,
            "source": "local",
            "sha256": "ignored",
            "installed_at": "2024-01-01T00:00:00Z",
            "content_type": "datapack",
        }))
        .unwrap()
    }

    fn manifest(packs: Vec<InstalledMod>) -> InstanceManifest {
        serde_json::from_value(serde_json::json!({
            "instance_id": "t", "name": "t", "minecraft_version": "1.21",
            "loader": "fabric", "loader_version": "0.1", "mods": [],
            "datapacks": packs,
        }))
        .unwrap()
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().to_path_buf();
            fs::create_dir_all(root.join("datapacks")).unwrap();
            Fixture { _dir: dir, root }
        }
        fn world(&self, name: &str) {
            let world = self.root.join("saves").join(name);
            fs::create_dir_all(&world).unwrap();
            fs::write(world.join("level.dat"), b"level").unwrap();
        }
        fn source(&self, filename: &str, bytes: &[u8]) {
            fs::write(self.root.join("datapacks").join(filename), bytes).unwrap();
        }
        fn in_world(&self, world: &str, filename: &str) -> PathBuf {
            self.root
                .join("saves")
                .join(world)
                .join("datapacks")
                .join(filename)
        }
    }

    #[test]
    fn fresh_world_gets_the_pack() {
        let fx = Fixture::new();
        fx.world("Alpha");
        fx.world("Beta");
        fx.source("vm.zip", b"pack-bytes");
        let m = manifest(vec![pack("vm.zip")]);
        let report = sync_instance_datapacks(&fx.root, &m);
        assert_eq!(report.copied, 2);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert_eq!(
            fs::read(fx.in_world("Alpha", "vm.zip")).unwrap(),
            b"pack-bytes"
        );
        assert_eq!(
            fs::read(fx.in_world("Beta", "vm.zip")).unwrap(),
            b"pack-bytes"
        );
        assert!(fx.in_world("Alpha", MANAGED_RECORD_FILE).is_file());
    }

    #[test]
    fn folders_without_level_dat_are_not_worlds() {
        let fx = Fixture::new();
        fs::create_dir_all(fx.root.join("saves").join("NotAWorld")).unwrap();
        fx.source("vm.zip", b"x");
        let report = sync_instance_datapacks(&fx.root, &manifest(vec![pack("vm.zip")]));
        assert_eq!(report.worlds, 0);
        assert!(!fx.in_world("NotAWorld", "vm.zip").exists());
    }

    #[test]
    fn second_run_is_a_no_op() {
        let fx = Fixture::new();
        fx.world("Alpha");
        fx.source("vm.zip", b"pack-bytes");
        let m = manifest(vec![pack("vm.zip")]);
        sync_instance_datapacks(&fx.root, &m);
        let record_before = fs::read(fx.in_world("Alpha", MANAGED_RECORD_FILE)).unwrap();
        let report = sync_instance_datapacks(&fx.root, &m);
        assert_eq!((report.copied, report.removed), (0, 0));
        assert!(report.warnings.is_empty());
        assert_eq!(
            fs::read(fx.in_world("Alpha", MANAGED_RECORD_FILE)).unwrap(),
            record_before
        );
    }

    #[test]
    fn disabling_removes_only_the_agora_copy() {
        let fx = Fixture::new();
        fx.world("Alpha");
        fx.source("vm.zip", b"pack-bytes");
        let mut m = manifest(vec![pack("vm.zip")]);
        sync_instance_datapacks(&fx.root, &m);
        // The user's own unrelated data pack sits in the same folder.
        fs::write(fx.in_world("Alpha", "mine.zip"), b"mine").unwrap();

        m.datapacks[0].enabled = false;
        let report = sync_instance_datapacks(&fx.root, &m);
        assert_eq!(report.removed, 1);
        assert!(!fx.in_world("Alpha", "vm.zip").exists());
        assert!(fx.in_world("Alpha", "mine.zip").is_file());
        assert!(
            !fx.in_world("Alpha", MANAGED_RECORD_FILE).exists(),
            "an empty record is removed"
        );
    }

    #[test]
    fn removing_a_pack_removes_only_agora_placed_copies() {
        let fx = Fixture::new();
        fx.world("Alpha");
        fx.world("Beta");
        fx.source("vm.zip", b"pack-bytes");
        // Beta already had an identical file from the user before Agora ran.
        fs::create_dir_all(fx.root.join("saves/Beta/datapacks")).unwrap();
        fs::write(fx.in_world("Beta", "vm.zip"), b"pack-bytes").unwrap();
        let mut m = manifest(vec![pack("vm.zip")]);
        sync_instance_datapacks(&fx.root, &m);
        assert!(fx.in_world("Alpha", "vm.zip").is_file());

        m.datapacks.clear();
        let report = sync_instance_datapacks(&fx.root, &m);
        assert_eq!(report.removed, 1);
        assert!(!fx.in_world("Alpha", "vm.zip").exists());
        assert_eq!(
            fs::read(fx.in_world("Beta", "vm.zip")).unwrap(),
            b"pack-bytes",
            "the user's identical file was never Agora's to delete"
        );
    }

    #[test]
    fn user_placed_same_name_file_is_left_alone_with_a_warning() {
        let fx = Fixture::new();
        fx.world("Alpha");
        fx.source("vm.zip", b"agora-version");
        fs::create_dir_all(fx.root.join("saves/Alpha/datapacks")).unwrap();
        fs::write(fx.in_world("Alpha", "vm.zip"), b"user-version").unwrap();
        let m = manifest(vec![pack("vm.zip")]);

        let report = sync_instance_datapacks(&fx.root, &m);
        assert_eq!(report.copied, 0);
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(
            fs::read(fx.in_world("Alpha", "vm.zip")).unwrap(),
            b"user-version"
        );
        assert!(!fx.in_world("Alpha", MANAGED_RECORD_FILE).exists());

        // ...and disabling the pack must not delete the user's file either.
        let mut m = m;
        m.datapacks[0].enabled = false;
        sync_instance_datapacks(&fx.root, &m);
        assert_eq!(
            fs::read(fx.in_world("Alpha", "vm.zip")).unwrap(),
            b"user-version"
        );
    }

    #[test]
    fn a_copy_the_user_edited_is_not_removed() {
        let fx = Fixture::new();
        fx.world("Alpha");
        fx.source("vm.zip", b"pack-bytes");
        let mut m = manifest(vec![pack("vm.zip")]);
        sync_instance_datapacks(&fx.root, &m);
        fs::write(fx.in_world("Alpha", "vm.zip"), b"edited by user").unwrap();

        m.datapacks[0].enabled = false;
        let report = sync_instance_datapacks(&fx.root, &m);
        assert_eq!(report.removed, 0);
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(
            fs::read(fx.in_world("Alpha", "vm.zip")).unwrap(),
            b"edited by user"
        );
    }

    #[test]
    fn scope_to_one_world_and_back() {
        let fx = Fixture::new();
        fx.world("Alpha");
        fx.world("Beta");
        fx.source("vm.zip", b"pack-bytes");
        let mut m = manifest(vec![pack("vm.zip")]);
        set_scope(&mut m, "vm.zip", DatapackScope::Worlds(vec!["Beta".into()])).unwrap();
        sync_instance_datapacks(&fx.root, &m);
        assert!(!fx.in_world("Alpha", "vm.zip").exists());
        assert!(fx.in_world("Beta", "vm.zip").is_file());

        // Narrowing an existing sync removes the copy from the world left out.
        set_scope(
            &mut m,
            "vm.zip",
            DatapackScope::Worlds(vec!["Alpha".into()]),
        )
        .unwrap();
        let report = sync_instance_datapacks(&fx.root, &m);
        assert_eq!((report.copied, report.removed), (1, 1));
        assert!(fx.in_world("Alpha", "vm.zip").is_file());
        assert!(!fx.in_world("Beta", "vm.zip").exists());

        set_scope(&mut m, "vm.zip", DatapackScope::AllWorlds).unwrap();
        assert!(m.user_preferences.get(SCOPES_KEY).is_none());
        sync_instance_datapacks(&fx.root, &m);
        assert!(fx.in_world("Beta", "vm.zip").is_file());
    }

    #[test]
    fn scope_is_keyed_by_identity_so_updates_keep_it() {
        let mut first = pack("vm-1.0.zip");
        first.modrinth_id = Some("proj".into());
        let mut m = manifest(vec![first]);
        set_scope(
            &mut m,
            "vm-1.0.zip",
            DatapackScope::Worlds(vec!["A".into()]),
        )
        .unwrap();
        let mut updated = pack("vm-1.1.zip");
        updated.modrinth_id = Some("proj".into());
        m.datapacks = vec![updated];
        assert_eq!(
            scope_of(&m, &m.datapacks[0]),
            DatapackScope::Worlds(vec!["A".into()])
        );
    }

    #[test]
    fn existing_manifests_default_to_all_worlds() {
        let m = manifest(vec![pack("vm.zip")]);
        assert_eq!(scope_of(&m, &m.datapacks[0]), DatapackScope::AllWorlds);
    }

    #[test]
    fn updated_pack_replaces_agora_copy_in_place() {
        let fx = Fixture::new();
        fx.world("Alpha");
        fx.source("vm.zip", b"v1");
        let m = manifest(vec![pack("vm.zip")]);
        sync_instance_datapacks(&fx.root, &m);
        fx.source("vm.zip", b"version-2");
        let report = sync_instance_datapacks(&fx.root, &m);
        assert_eq!(report.copied, 1);
        assert_eq!(
            fs::read(fx.in_world("Alpha", "vm.zip")).unwrap(),
            b"version-2"
        );
    }

    #[test]
    fn a_world_created_later_is_picked_up_at_the_next_sync() {
        let fx = Fixture::new();
        fx.source("vm.zip", b"pack-bytes");
        let m = manifest(vec![pack("vm.zip")]);
        assert_eq!(sync_instance_datapacks(&fx.root, &m).copied, 0);
        fx.world("Fresh");
        assert_eq!(sync_instance_datapacks(&fx.root, &m).copied, 1);
        assert!(fx.in_world("Fresh", "vm.zip").is_file());
    }

    #[test]
    fn unreadable_record_removes_nothing() {
        let fx = Fixture::new();
        fx.world("Alpha");
        fx.source("vm.zip", b"pack-bytes");
        let mut m = manifest(vec![pack("vm.zip")]);
        sync_instance_datapacks(&fx.root, &m);
        fs::write(fx.in_world("Alpha", MANAGED_RECORD_FILE), b"{not json").unwrap();
        m.datapacks[0].enabled = false;
        let report = sync_instance_datapacks(&fx.root, &m);
        assert_eq!(report.removed, 0);
        assert!(fx.in_world("Alpha", "vm.zip").is_file());
        assert!(!report.warnings.is_empty());
    }

    #[test]
    fn world_status_counts_covered_worlds() {
        let mut m = manifest(vec![pack("vm.zip")]);
        let worlds = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let status = world_status(&m, &m.datapacks[0], &worlds);
        assert!(status.all_worlds);
        assert_eq!(status.covered_worlds, 3);
        set_scope(
            &mut m,
            "vm.zip",
            DatapackScope::Worlds(vec!["B".into(), "Gone".into()]),
        )
        .unwrap();
        let status = world_status(&m, &m.datapacks[0], &worlds);
        assert!(!status.all_worlds);
        assert_eq!(status.covered_worlds, 1);
    }

    fn service_instance(tmp: &Path) -> (crate::ctx::CoreContext, PathBuf) {
        let ctx = crate::ctx::CoreContext::for_testing(tmp.to_path_buf());
        let dir = ctx.paths.instance_dir("inst").unwrap();
        fs::create_dir_all(dir.join("datapacks")).unwrap();
        fs::write(dir.join("datapacks").join("vm.zip"), b"pack-bytes").unwrap();
        for world in ["Alpha", "Beta"] {
            fs::create_dir_all(dir.join("saves").join(world)).unwrap();
            fs::write(dir.join("saves").join(world).join("level.dat"), b"l").unwrap();
        }
        let m = manifest(vec![pack("vm.zip")]);
        crate::helpers::atomic_write_manifest(&ctx.paths.instance_manifest("inst").unwrap(), &m)
            .unwrap();
        (ctx, dir)
    }

    #[test]
    fn set_world_scope_persists_and_syncs() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, dir) = service_instance(tmp.path());
        let report = set_world_scope(&ctx, "inst", "vm.zip", Some(vec!["Beta".into()])).unwrap();
        assert_eq!(report.copied, 1);
        assert!(!dir.join("saves/Alpha/datapacks/vm.zip").exists());
        assert!(dir.join("saves/Beta/datapacks/vm.zip").is_file());
        let reloaded =
            crate::helpers::read_manifest(&ctx.paths.instance_manifest("inst").unwrap()).unwrap();
        assert_eq!(
            scope_of(&reloaded, &reloaded.datapacks[0]),
            DatapackScope::Worlds(vec!["Beta".into()])
        );

        let report = set_world_scope(&ctx, "inst", "vm.zip", None).unwrap();
        assert_eq!(report.copied, 1);
        assert!(dir.join("saves/Alpha/datapacks/vm.zip").is_file());
        assert!(set_world_scope(&ctx, "inst", "nope.zip", None).is_err());
    }

    #[test]
    fn toggling_a_data_pack_updates_worlds_immediately() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, dir) = service_instance(tmp.path());
        sync_instance(&ctx, "inst").unwrap();
        assert!(dir.join("saves/Alpha/datapacks/vm.zip").is_file());

        let svc = crate::crash_service::CrashService::new(ctx.clone());
        svc.disable_artifact("inst", "vm.zip").unwrap();
        assert!(!dir.join("saves/Alpha/datapacks/vm.zip").exists());
        assert!(!dir.join("saves/Beta/datapacks/vm.zip").exists());

        svc.enable_artifact("inst", "vm.zip").unwrap();
        assert!(dir.join("saves/Alpha/datapacks/vm.zip").is_file());
        assert!(dir.join("saves/Beta/datapacks/vm.zip").is_file());
    }
}
