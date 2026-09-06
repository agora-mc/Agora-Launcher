use crate::error::{LauncherError, LauncherResult};
use serde_json::{Map, Value};
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

static PROFILE_WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

pub struct LauncherProfileEntry {
    pub profile_id: String,
    pub name: String,
    pub last_version_id: String,
    pub game_dir: PathBuf,
    pub java_args: String,
    /// Stamp this profile as the most recently used one.
    ///
    /// Set only for a delegated handoff. Creating or cloning an instance must
    /// leave it `false`: those write a profile the user has not asked to play,
    /// and claiming "most recent" for them would move the official launcher off
    /// whatever the user actually last played.
    pub select: bool,
}

/// Timestamp in the format the official launcher writes — RFC 3339 with
/// milliseconds and a `Z` suffix. Agora used to emit chrono's default
/// `+00:00` offset here, which the launcher rewrote on every startup.
fn launcher_timestamp() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

impl LauncherProfileEntry {
    /// Merge this entry into whatever the launcher already has for the profile.
    ///
    /// Only the keys Agora owns are written. `created`, and any key the
    /// launcher added that Agora has no opinion about (window size, resolution,
    /// icon overrides), are carried across untouched — a full replacement used
    /// to discard them on every rename and every settings change.
    fn merge_into(&self, existing: Option<&Value>) -> Value {
        let mut obj = existing
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        obj.insert("name".to_string(), Value::String(self.name.clone()));
        obj.insert("type".to_string(), Value::String("custom".to_string()));
        obj.entry("created".to_string())
            .or_insert_with(|| Value::String(launcher_timestamp()));
        obj.entry("icon".to_string())
            .or_insert_with(|| Value::String("Furnace".to_string()));
        obj.insert(
            "lastVersionId".to_string(),
            Value::String(self.last_version_id.clone()),
        );
        obj.insert(
            "gameDir".to_string(),
            Value::String(self.game_dir.to_string_lossy().to_string()),
        );
        obj.insert(
            "javaArgs".to_string(),
            Value::String(self.java_args.clone()),
        );
        if self.select {
            obj.insert("lastUsed".to_string(), Value::String(launcher_timestamp()));
        }
        Value::Object(obj)
    }
}

pub fn upsert_profile(
    entry: &LauncherProfileEntry,
    profiles_path: &std::path::Path,
) -> LauncherResult<()> {
    let _guard = profile_write_guard()?;
    let mc_dir = profiles_path
        .parent()
        .ok_or(LauncherError::MojangNotFound)?;
    std::fs::create_dir_all(mc_dir).map_err(|_| LauncherError::ProfileWriteFailed)?;

    let mut root: Value = read_or_recover(profiles_path)?;

    let profiles_obj = root
        .as_object_mut()
        .ok_or(LauncherError::ProfileWriteFailed)?
        .entry("profiles".to_string())
        .or_insert_with(|| Value::Object(Map::new()));

    let profiles_map = profiles_obj
        .as_object_mut()
        .ok_or(LauncherError::ProfileWriteFailed)?;
    let existing = profiles_map.get(&entry.profile_id);
    let desired = entry.merge_into(existing);
    if !entry.select && existing.is_some_and(|existing| profile_values_match(existing, &desired)) {
        // Keep the existing JSON when the effective launch settings are already
        // identical so the official launcher does not see a needless profile
        // mutation. A selecting write always lands: its whole purpose is to
        // move `lastUsed`, which is excluded from the comparison below.
        return Ok(());
    }
    profiles_map.insert(entry.profile_id.clone(), desired);

    if entry.select {
        // Honored by the legacy launcher, which reads a top-level selection.
        // The current launcher ignores it and restores its own UI state
        // instead — see `crate::launcher_ui_state`.
        root.as_object_mut()
            .ok_or(LauncherError::ProfileWriteFailed)?
            .insert(
                "selectedProfile".to_string(),
                Value::String(entry.profile_id.clone()),
            );
    }

    atomic_write(profiles_path, &root)
}

/// Materialize a managed Minecraft version JSON where the official launcher
/// can read it.
///
/// Agora keeps the direct-launch runtime separate from `%APPDATA%/.minecraft`.
/// Delegated launches still use the official launcher, so its selected profile
/// and every inherited version must exist in the official `versions` tree.
/// A valid existing base version is preserved; managed loader profiles are
/// refreshed so the official launcher cannot select stale launch metadata.
pub fn materialize_version_json(
    source_root: &Path,
    official_root: &Path,
    version_id: &str,
    overwrite_existing: bool,
) -> LauncherResult<()> {
    crate::app_paths::validate_path_component(version_id)?;

    let source_path = source_root
        .join("versions")
        .join(version_id)
        .join(format!("{version_id}.json"));
    let target_path = official_root
        .join("versions")
        .join(version_id)
        .join(format!("{version_id}.json"));

    let source_bytes = match std::fs::read(&source_path) {
        Ok(bytes) => bytes,
        Err(_error) if target_path.is_file() => {
            // An existing valid official profile is enough when the managed
            // cache was cleaned between instance creation and handoff.
            let existing = std::fs::read(&target_path).map_err(|read_error| {
                LauncherError::Generic {
                    code: "ERR_DELEGATED_PROFILE_MISSING".into(),
                    message: format!(
                        "The official Minecraft launcher profile {version_id} could not be read: {read_error}"
                    ),
                }
            })?;
            validate_version_json(&existing, version_id)?;
            return Ok(());
        }
        Err(error) => {
            return Err(LauncherError::Generic {
                code: "ERR_DELEGATED_PROFILE_MISSING".into(),
                message: format!(
                    "Agora could not find the launch profile {version_id} in its managed runtime: {error}"
                ),
            });
        }
    };
    validate_version_json(&source_bytes, version_id)?;

    if target_path.is_file() {
        let existing = std::fs::read(&target_path).map_err(|error| LauncherError::Generic {
            code: "ERR_DELEGATED_PROFILE_MISSING".into(),
            message: format!(
                "The official Minecraft launcher profile {version_id} could not be read: {error}"
            ),
        })?;
        if existing == source_bytes
            || (!overwrite_existing && validate_version_json(&existing, version_id).is_ok())
        {
            return Ok(());
        }
    }

    atomic_write_bytes(&target_path, &source_bytes)
}

fn validate_version_json(bytes: &[u8], version_id: &str) -> LauncherResult<()> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| LauncherError::Generic {
        code: "ERR_DELEGATED_PROFILE_INVALID".into(),
        message: format!("The Minecraft launch profile {version_id} is not valid JSON: {error}"),
    })?;
    if value.get("id").and_then(Value::as_str) != Some(version_id) {
        return Err(LauncherError::Generic {
            code: "ERR_DELEGATED_PROFILE_INVALID".into(),
            message: format!("The Minecraft launch profile {version_id} has the wrong version ID."),
        });
    }
    Ok(())
}

fn atomic_write_bytes(path: &Path, bytes: &[u8]) -> LauncherResult<()> {
    let parent = path.parent().ok_or(LauncherError::ProfileWriteFailed)?;
    std::fs::create_dir_all(parent).map_err(|_| LauncherError::ProfileWriteFailed)?;
    let temp = temp_path(path)?;
    std::fs::write(&temp, bytes).map_err(|_| LauncherError::ProfileWriteFailed)?;
    if std::fs::rename(&temp, path).is_err() {
        let _ = std::fs::remove_file(&temp);
        return Err(LauncherError::ProfileWriteFailed);
    }
    Ok(())
}

fn profile_values_match(existing: &Value, desired: &Value) -> bool {
    const STABLE_KEYS: &[&str] = &[
        "name",
        "type",
        "lastVersionId",
        "gameDir",
        "javaArgs",
        "icon",
    ];
    STABLE_KEYS
        .iter()
        .all(|key| existing.get(*key) == desired.get(*key))
}

pub fn remove_profile(profile_id: &str, profiles_path: &std::path::Path) -> LauncherResult<()> {
    let _guard = profile_write_guard()?;
    if !profiles_path.exists() {
        return Ok(());
    }

    let mc_dir = profiles_path
        .parent()
        .ok_or(LauncherError::MojangNotFound)?;
    std::fs::create_dir_all(mc_dir).map_err(|_| LauncherError::ProfileWriteFailed)?;

    let mut root: Value = read_or_recover(profiles_path)?;

    if let Some(profiles) = root
        .as_object_mut()
        .and_then(|o| o.get_mut("profiles"))
        .and_then(|p| p.as_object_mut())
    {
        profiles.remove(profile_id);
    }
    clear_selected_profile(&mut root, profile_id);

    atomic_write(profiles_path, &root)
}

/// Drop Agora-owned profiles whose instance is gone.
///
/// Deleting a pack removes its profile, but profiles still outlive their
/// instances: a delete that races the launcher's startup rewrite is undone, and
/// moving the Agora data root strands every profile written under the old one.
/// The launcher then lists installations that cannot launch, and can even
/// restore one of them as the current selection.
///
/// Only two kinds of profile are removed, so a second Agora install — or a
/// portable profile pointing at a different data root — is never touched:
///
/// * `agora-*` profiles whose `gameDir` is inside `instances_root` and whose
///   instance id is not `live`, and
/// * `agora-*` profiles whose `gameDir` no longer exists on disk at all.
///
/// Returns the profile ids that were removed.
pub fn prune_orphan_profiles(
    profiles_path: &std::path::Path,
    instances_root: &std::path::Path,
    live: &dyn Fn(&str) -> bool,
) -> LauncherResult<Vec<String>> {
    let _guard = profile_write_guard()?;
    if !profiles_path.exists() {
        return Ok(Vec::new());
    }
    let mut root: Value = read_or_recover(profiles_path)?;
    let Some(profiles) = root
        .as_object_mut()
        .and_then(|object| object.get_mut("profiles"))
        .and_then(Value::as_object_mut)
    else {
        return Ok(Vec::new());
    };

    let orphans: Vec<String> = profiles
        .iter()
        .filter(|(profile_id, profile)| {
            let Some(instance_id) = profile_id.strip_prefix(PROFILE_ID_PREFIX) else {
                return false;
            };
            let game_dir = profile
                .get("gameDir")
                .and_then(Value::as_str)
                .map(Path::new);
            match game_dir {
                Some(dir) if dir.starts_with(instances_root) => !live(instance_id),
                Some(dir) => !dir.exists(),
                // A profile with no game directory is not one Agora wrote.
                None => false,
            }
        })
        .map(|(profile_id, _)| profile_id.clone())
        .collect();

    if orphans.is_empty() {
        return Ok(Vec::new());
    }
    for profile_id in &orphans {
        profiles.remove(profile_id);
    }
    for profile_id in &orphans {
        clear_selected_profile(&mut root, profile_id);
    }
    atomic_write(profiles_path, &root)?;
    Ok(orphans)
}

/// The profile id Agora uses for an instance.
pub fn profile_id_for(instance_id: &str) -> String {
    format!("{PROFILE_ID_PREFIX}{instance_id}")
}

/// Read one profile's stored fields, if the launcher has it.
pub fn read_profile(profiles_path: &std::path::Path, profile_id: &str) -> Option<Value> {
    read_json(profiles_path)
        .ok()?
        .get("profiles")?
        .get(profile_id)
        .cloned()
}

/// The most recently used profile other than `exclude`, as an entry plus its
/// `lastUsed` stamp.
///
/// Used to move the official launcher's saved selection off a profile that was
/// just deleted, onto something that can actually launch.
pub fn most_recent_profile_except(
    profiles_path: &std::path::Path,
    exclude: &str,
) -> Option<(LauncherProfileEntry, String)> {
    let document = read_json(profiles_path).ok()?;
    let profiles = document.get("profiles")?.as_object()?;
    profiles
        .iter()
        .filter(|(profile_id, _)| profile_id.as_str() != exclude)
        .filter_map(|(profile_id, profile)| {
            let last_used = profile
                .get("lastUsed")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let entry = LauncherProfileEntry {
                profile_id: profile_id.clone(),
                name: profile.get("name").and_then(Value::as_str)?.to_owned(),
                last_version_id: profile
                    .get("lastVersionId")
                    .and_then(Value::as_str)?
                    .to_owned(),
                game_dir: PathBuf::from(profile.get("gameDir").and_then(Value::as_str)?),
                java_args: profile
                    .get("javaArgs")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                select: false,
            };
            Some((entry, last_used))
        })
        .max_by(|left, right| left.1.cmp(&right.1))
}

const PROFILE_ID_PREFIX: &str = "agora-";

fn clear_selected_profile(root: &mut Value, profile_id: &str) {
    let Some(object) = root.as_object_mut() else {
        return;
    };
    if object.get("selectedProfile").and_then(Value::as_str) == Some(profile_id) {
        object.remove("selectedProfile");
    }
}

fn read_or_recover(profiles_path: &std::path::Path) -> LauncherResult<Value> {
    match read_json(profiles_path) {
        Ok(v) => Ok(v),
        Err(_) => {
            let bak = bak_path(profiles_path);
            if bak.exists() {
                if let Ok(v) = read_json(&bak) {
                    restore_live(profiles_path, &v)?;
                    return Ok(v);
                }
            }
            if !profiles_path.exists() && !bak.exists() {
                return Ok(minimal_profiles());
            }
            // TODO: surface this in the UI as a notification banner (spec 8.3.1 Recovery step 2):
            //   "launcher_profiles.json was corrupted and has been regenerated with your curated profiles."
            eprintln!("[launcher_profiles] WARNING: live file + .bak both invalid; regenerated minimal profiles.");
            Ok(minimal_profiles())
        }
    }
}

fn read_json(path: &std::path::Path) -> LauncherResult<Value> {
    let text = std::fs::read_to_string(path).map_err(|_| LauncherError::ProfileWriteFailed)?;
    serde_json::from_str(&text).map_err(|_| LauncherError::ProfileWriteFailed)
}

fn minimal_profiles() -> Value {
    let mut root = Map::new();
    root.insert("profiles".to_string(), Value::Object(Map::new()));
    root.insert("settings".to_string(), Value::Object(Map::new()));
    Value::Object(root)
}

fn atomic_write(profiles_path: &std::path::Path, root: &Value) -> LauncherResult<()> {
    let serialized =
        serde_json::to_string_pretty(root).map_err(|_| LauncherError::ProfileWriteFailed)?;
    let tmp = temp_path(profiles_path)?;
    let bak = bak_path(profiles_path);

    std::fs::write(&tmp, serialized).map_err(|_| LauncherError::ProfileWriteFailed)?;

    if profiles_path.exists() {
        let live = std::fs::read_to_string(profiles_path).unwrap_or_default();
        if serde_json::from_str::<Value>(&live).is_ok() {
            let _ = std::fs::copy(profiles_path, &bak);
        }
    }

    if std::fs::rename(&tmp, profiles_path).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return Err(LauncherError::ProfileWriteFailed);
    }
    Ok(())
}

fn restore_live(profiles_path: &std::path::Path, root: &Value) -> LauncherResult<()> {
    let serialized =
        serde_json::to_string_pretty(root).map_err(|_| LauncherError::ProfileWriteFailed)?;
    let tmp = temp_path(profiles_path)?;
    std::fs::write(&tmp, serialized).map_err(|_| LauncherError::ProfileWriteFailed)?;
    if std::fs::rename(&tmp, profiles_path).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return Err(LauncherError::ProfileWriteFailed);
    }
    Ok(())
}

fn profile_write_guard() -> LauncherResult<std::sync::MutexGuard<'static, ()>> {
    PROFILE_WRITE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| LauncherError::ProfileWriteFailed)
}

fn temp_path(profiles_path: &std::path::Path) -> LauncherResult<PathBuf> {
    let name = profiles_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(LauncherError::ProfileWriteFailed)?;
    let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    Ok(profiles_path.with_file_name(format!(".{name}.agora-{}-{id}.tmp", std::process::id())))
}

fn bak_path(profiles_path: &std::path::Path) -> PathBuf {
    let mut p = profiles_path.to_path_buf();
    p.set_extension("json.bak");
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn profile(profile_id: &str, root: &std::path::Path) -> LauncherProfileEntry {
        LauncherProfileEntry {
            profile_id: profile_id.to_owned(),
            name: profile_id.to_owned(),
            last_version_id: "1.21".into(),
            game_dir: root.join(profile_id),
            java_args: "-Xmx2G".into(),
            select: false,
        }
    }

    fn profiles_of(path: &std::path::Path) -> Map<String, Value> {
        read_json(path).unwrap()["profiles"]
            .as_object()
            .unwrap()
            .clone()
    }

    #[test]
    fn concurrent_upserts_preserve_every_profile() {
        let root = std::env::temp_dir().join(format!(
            "agora-launcher-profiles-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let path = Arc::new(root.join("launcher_profiles.json"));
        let mut threads = Vec::new();

        for index in 0..8 {
            let path = Arc::clone(&path);
            let root = root.clone();
            threads.push(std::thread::spawn(move || {
                let id = format!("profile-{index}");
                upsert_profile(&profile(&id, &root), &path).unwrap();
            }));
        }
        for thread in threads {
            thread.join().unwrap();
        }

        let root_json: Value =
            serde_json::from_str(&std::fs::read_to_string(&*path).unwrap()).unwrap();
        let profiles = root_json["profiles"].as_object().unwrap();
        for index in 0..8 {
            assert!(profiles.contains_key(&format!("profile-{index}")));
        }

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unchanged_profile_is_not_rewritten() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("launcher_profiles.json");
        let entry = profile("stable", root.path());

        upsert_profile(&entry, &path).unwrap();
        let before = std::fs::read(&path).unwrap();
        upsert_profile(&entry, &path).unwrap();
        let after = std::fs::read(&path).unwrap();

        assert_eq!(before, after);
    }

    #[test]
    fn selecting_write_stamps_last_used_even_when_nothing_else_changed() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("launcher_profiles.json");
        let mut entry = profile("stable", root.path());

        upsert_profile(&entry, &path).unwrap();
        assert!(
            profiles_of(&path)["stable"].get("lastUsed").is_none(),
            "creating an instance must not claim the launcher's selection"
        );

        entry.select = true;
        upsert_profile(&entry, &path).unwrap();

        let written = profiles_of(&path);
        let last_used = written["stable"]["lastUsed"].as_str().unwrap();
        assert!(
            last_used.ends_with('Z'),
            "launcher timestamps are RFC 3339 with a Z suffix, got {last_used}"
        );
        assert_eq!(
            read_json(&path).unwrap()["selectedProfile"],
            "stable",
            "the legacy launcher reads the top-level selection"
        );
    }

    #[test]
    fn rewriting_a_profile_preserves_launcher_owned_keys() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("launcher_profiles.json");
        let mut entry = profile("keep", root.path());
        upsert_profile(&entry, &path).unwrap();

        // Stand in for what the launcher adds on its own.
        let mut document = read_json(&path).unwrap();
        let stored = document["profiles"]["keep"].as_object_mut().unwrap();
        stored.insert(
            "lastUsed".into(),
            Value::String("2026-01-01T00:00:00.000Z".into()),
        );
        stored.insert(
            "resolution".into(),
            serde_json::json!({ "width": 1280, "height": 720 }),
        );
        let created = stored["created"].as_str().unwrap().to_owned();
        atomic_write(&path, &document).unwrap();

        entry.name = "Renamed".into();
        upsert_profile(&entry, &path).unwrap();

        let written = profiles_of(&path);
        assert_eq!(written["keep"]["name"], "Renamed");
        assert_eq!(written["keep"]["resolution"]["width"], 1280);
        assert_eq!(written["keep"]["lastUsed"], "2026-01-01T00:00:00.000Z");
        assert_eq!(written["keep"]["created"], created.as_str());
    }

    #[test]
    fn removing_a_profile_drops_a_selection_pointing_at_it() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("launcher_profiles.json");
        let mut entry = profile("doomed", root.path());
        entry.select = true;
        upsert_profile(&entry, &path).unwrap();
        assert_eq!(read_json(&path).unwrap()["selectedProfile"], "doomed");

        remove_profile("doomed", &path).unwrap();

        let document = read_json(&path).unwrap();
        assert!(document["profiles"].get("doomed").is_none());
        assert!(document.get("selectedProfile").is_none());
    }

    #[test]
    fn pruning_removes_dead_instances_but_spares_other_data_roots() {
        let root = tempfile::tempdir().unwrap();
        let instances = root.path().join("instances");
        let path = root.path().join("launcher_profiles.json");

        let live_dir = instances.join("alive");
        std::fs::create_dir_all(&live_dir).unwrap();
        for id in ["alive", "deleted"] {
            let mut entry = profile(id, &instances);
            entry.profile_id = profile_id_for(id);
            entry.game_dir = instances.join(id);
            upsert_profile(&entry, &path).unwrap();
        }
        // Another Agora install: inside no known instances root, still on disk.
        let other_root = root.path().join("portable-instances");
        std::fs::create_dir_all(other_root.join("elsewhere")).unwrap();
        let mut foreign = profile("elsewhere", &other_root);
        foreign.profile_id = profile_id_for("elsewhere");
        foreign.game_dir = other_root.join("elsewhere");
        upsert_profile(&foreign, &path).unwrap();
        // A profile from an Agora data root that no longer exists anywhere.
        let mut stranded = profile("stranded", Path::new("C:\\gone\\instances"));
        stranded.profile_id = profile_id_for("stranded");
        stranded.game_dir = root.path().join("removed-root/instances/stranded");
        upsert_profile(&stranded, &path).unwrap();
        // Not ours at all.
        let mut vanilla = profile("Forge", root.path());
        vanilla.profile_id = "Forge".into();
        upsert_profile(&vanilla, &path).unwrap();

        let removed = prune_orphan_profiles(&path, &instances, &|id| id == "alive").unwrap();

        assert_eq!(
            removed.iter().collect::<std::collections::BTreeSet<_>>(),
            ["agora-deleted".to_string(), "agora-stranded".to_string()]
                .iter()
                .collect()
        );
        let written = profiles_of(&path);
        assert!(written.contains_key("agora-alive"));
        assert!(written.contains_key("agora-elsewhere"));
        assert!(written.contains_key("Forge"));
        assert!(!written.contains_key("agora-deleted"));
        assert!(!written.contains_key("agora-stranded"));
    }

    #[test]
    fn pruning_a_clean_file_writes_nothing() {
        let root = tempfile::tempdir().unwrap();
        let instances = root.path().join("instances");
        let path = root.path().join("launcher_profiles.json");
        std::fs::create_dir_all(instances.join("alive")).unwrap();
        let mut entry = profile("alive", &instances);
        entry.profile_id = profile_id_for("alive");
        entry.game_dir = instances.join("alive");
        upsert_profile(&entry, &path).unwrap();
        let before = std::fs::read(&path).unwrap();

        let removed = prune_orphan_profiles(&path, &instances, &|_| true).unwrap();

        assert!(removed.is_empty());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn materializes_managed_version_for_official_launcher() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("minecraft-runtime");
        let official = root.path().join("official-minecraft");
        let version_id = "fabric-loader-0.19.3-26.2";
        let source_path = source
            .join("versions")
            .join(version_id)
            .join(format!("{version_id}.json"));
        std::fs::create_dir_all(source_path.parent().unwrap()).unwrap();
        std::fs::write(
            &source_path,
            format!(r#"{{"id":"{version_id}","inheritsFrom":"26.2","libraries":[]}}"#),
        )
        .unwrap();

        materialize_version_json(&source, &official, version_id, true).unwrap();
        materialize_version_json(&source, &official, version_id, true).unwrap();

        let target = official
            .join("versions")
            .join(version_id)
            .join(format!("{version_id}.json"));
        assert_eq!(
            std::fs::read(target).unwrap(),
            std::fs::read(source_path).unwrap()
        );
    }

    #[test]
    fn preserves_existing_valid_base_version() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("minecraft-runtime");
        let official = root.path().join("official-minecraft");
        let version_id = "26.2";
        let source_path = source
            .join("versions")
            .join(version_id)
            .join(format!("{version_id}.json"));
        let target_path = official
            .join("versions")
            .join(version_id)
            .join(format!("{version_id}.json"));
        std::fs::create_dir_all(source_path.parent().unwrap()).unwrap();
        std::fs::create_dir_all(target_path.parent().unwrap()).unwrap();
        std::fs::write(&source_path, br#"{"id":"26.2","source":"agora"}"#).unwrap();
        std::fs::write(&target_path, br#"{"id":"26.2","source":"official"}"#).unwrap();

        materialize_version_json(&source, &official, version_id, false).unwrap();

        assert_eq!(
            std::fs::read_to_string(target_path).unwrap(),
            r#"{"id":"26.2","source":"official"}"#
        );
    }
}
