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
}

impl LauncherProfileEntry {
    fn to_json(&self) -> Value {
        let mut obj = Map::new();
        obj.insert("name".to_string(), Value::String(self.name.clone()));
        obj.insert("type".to_string(), Value::String("custom".to_string()));
        obj.insert(
            "created".to_string(),
            Value::String(chrono::Utc::now().to_rfc3339()),
        );
        obj.insert(
            "lastVersionId".to_string(),
            Value::String(self.last_version_id.clone()),
        );
        obj.insert("icon".to_string(), Value::String("Furnace".to_string()));
        obj.insert(
            "gameDir".to_string(),
            Value::String(self.game_dir.to_string_lossy().to_string()),
        );
        obj.insert(
            "javaArgs".to_string(),
            Value::String(self.java_args.clone()),
        );
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
    let desired = entry.to_json();
    if profiles_map
        .get(&entry.profile_id)
        .is_some_and(|existing| profile_values_match(existing, &desired))
    {
        // Delegated launches reconcile this profile on every handoff.  Keep
        // the existing JSON and timestamp when the effective launch settings
        // are already identical so the official launcher does not see a
        // needless profile mutation on every click.
        return Ok(());
    }
    profiles_map.insert(entry.profile_id.clone(), desired);

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

    atomic_write(profiles_path, &root)
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
        }
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
