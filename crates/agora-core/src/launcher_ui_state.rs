//! The official launcher's saved installation selection.
//!
//! Delegated launch hands off to the official Minecraft Launcher, and Agora
//! needs it to open on the pack the user actually clicked. That selection is
//! **not** stored in `launcher_profiles.json`: the launcher restores it from
//! `homePageLastSelected.javaConfiguration` inside its own UI-state file, and
//! measurement on launcher core 3.39.31 showed the `--profile` argument Agora
//! passes is ignored entirely — a cold start with `--profile <id>` still opened
//! on the previously selected installation.
//!
//! The file is launcher-internal and carries a `DO NOT EDIT` banner, so every
//! function here is written to **fail closed**: anything unexpected about the
//! layout (missing sentinel, unparseable JSON, absent selection object) skips
//! the file without writing. Agora only ever rewrites a selection that is
//! already there; it never creates the file, never adds the key, and never
//! touches any other setting in it.
//!
//! The launcher owns the file while it runs and rewrites it from memory, so
//! callers must only write when the launcher is closed — see
//! [`crate::official_launcher`].

use crate::error::{LauncherError, LauncherResult};
use crate::launcher_profiles::LauncherProfileEntry;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// Separates the launcher's `DO NOT EDIT` banner from the JSON body.
const BANNER_SENTINEL: &str = "$#";

/// The launcher writes one UI-state file per distribution. The Microsoft Store
/// build uses the `_microsoft_store` suffix; the standalone build has its own.
/// Only files that already exist are considered.
const UI_STATE_FILENAMES: &[&str] = &[
    "launcher_ui_state_microsoft_store.json",
    "launcher_ui_state.json",
];

/// Point every existing UI-state file at `entry` so the launcher opens on it.
///
/// Returns the files that were rewritten. An empty vector is a normal outcome
/// — it means no UI-state file was present or none had a selection to update,
/// and the caller should treat that as "selection could not be set", not as an
/// error.
pub fn select_installation(
    minecraft_dir: &Path,
    entry: &LauncherProfileEntry,
    last_used: &str,
) -> LauncherResult<Vec<PathBuf>> {
    let mut updated = Vec::new();
    for path in existing_ui_state_files(minecraft_dir) {
        if update_selection(&path, |selection| {
            apply_entry(selection, entry, last_used);
            true
        })? {
            updated.push(path);
        }
    }
    Ok(updated)
}

/// Move the saved selection off `profile_id` when it points at a profile that
/// no longer exists.
///
/// Deleting a pack removes its `launcher_profiles.json` entry, but the launcher
/// restores its selection from this file — which would leave it opening on an
/// installation that is gone. `replacement` is used when available; without one
/// the file is left untouched, because an absent selection object is a shape
/// this code has not verified the launcher tolerates.
pub fn clear_selection_for(
    minecraft_dir: &Path,
    profile_id: &str,
    replacement: Option<(&LauncherProfileEntry, &str)>,
) -> LauncherResult<Vec<PathBuf>> {
    let Some((entry, last_used)) = replacement else {
        return Ok(Vec::new());
    };
    let mut updated = Vec::new();
    for path in existing_ui_state_files(minecraft_dir) {
        if update_selection(&path, |selection| {
            if selection.get("id").and_then(Value::as_str) != Some(profile_id) {
                return false;
            }
            apply_entry(selection, entry, last_used);
            true
        })? {
            updated.push(path);
        }
    }
    Ok(updated)
}

/// Read the profile id the launcher will open on, if one is recorded.
pub fn selected_profile_id(minecraft_dir: &Path) -> Option<String> {
    for path in existing_ui_state_files(minecraft_dir) {
        let Ok(document) = parse_document(&path) else {
            continue;
        };
        if let Some(id) = document
            .selection
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_owned)
        {
            return Some(id);
        }
    }
    None
}

fn existing_ui_state_files(minecraft_dir: &Path) -> Vec<PathBuf> {
    UI_STATE_FILENAMES
        .iter()
        .map(|name| minecraft_dir.join(name))
        .filter(|path| path.is_file())
        .collect()
}

fn apply_entry(selection: &mut Map<String, Value>, entry: &LauncherProfileEntry, last_used: &str) {
    // Only the keys the launcher itself writes for a custom installation are
    // set; anything else already in the object is preserved untouched.
    selection.insert("id".into(), Value::String(entry.profile_id.clone()));
    selection.insert("name".into(), Value::String(entry.name.clone()));
    selection.insert(
        "versionId".into(),
        Value::String(entry.last_version_id.clone()),
    );
    selection.insert(
        "gameDir".into(),
        Value::String(entry.game_dir.to_string_lossy().to_string()),
    );
    selection.insert("javaArgs".into(), Value::String(entry.java_args.clone()));
    selection.insert("type".into(), Value::String("custom".into()));
    selection.insert("lastUsed".into(), Value::String(last_used.to_owned()));
}

/// A UI-state file decomposed into the pieces needed to rewrite one field.
///
/// `UiSettings` is JSON encoded *as a string* inside the outer document, so a
/// round trip has to re-encode it at the same nesting.
struct UiStateDocument {
    banner: String,
    root: Value,
    settings: Value,
    selection: Map<String, Value>,
}

fn parse_document(path: &Path) -> LauncherResult<UiStateDocument> {
    let raw = std::fs::read_to_string(path).map_err(|_| LauncherError::ProfileWriteFailed)?;
    // The banner ends at the first `$#`; the opening `#$` marker does not
    // contain that sequence, so the first match is the closing one.
    let split = raw
        .find(BANNER_SENTINEL)
        .map(|index| index + BANNER_SENTINEL.len())
        .ok_or(LauncherError::ProfileWriteFailed)?;
    let (banner, body) = raw.split_at(split);

    let root: Value = serde_json::from_str(body).map_err(|_| LauncherError::ProfileWriteFailed)?;
    let settings_text = root
        .get("data")
        .and_then(|data| data.get("UiSettings"))
        .and_then(Value::as_str)
        .ok_or(LauncherError::ProfileWriteFailed)?;
    let settings: Value =
        serde_json::from_str(settings_text).map_err(|_| LauncherError::ProfileWriteFailed)?;
    let selection = settings
        .get("homePageLastSelected")
        .and_then(|last| last.get("javaConfiguration"))
        .and_then(Value::as_object)
        .cloned()
        .ok_or(LauncherError::ProfileWriteFailed)?;

    Ok(UiStateDocument {
        banner: banner.to_owned(),
        root,
        settings,
        selection,
    })
}

/// Apply `edit` to the saved selection and write the file back.
///
/// Returns `false` without writing when the file does not have the expected
/// shape or when `edit` reports there was nothing to change. Parse failures are
/// deliberately not surfaced as errors: an unrecognized launcher build must
/// degrade to "selection not set", never to a failed launch.
fn update_selection(
    path: &Path,
    edit: impl FnOnce(&mut Map<String, Value>) -> bool,
) -> LauncherResult<bool> {
    let Ok(mut document) = parse_document(path) else {
        return Ok(false);
    };
    if !edit(&mut document.selection) {
        return Ok(false);
    }

    let Some(settings) = document.settings.as_object_mut() else {
        return Ok(false);
    };
    let Some(last_selected) = settings
        .get_mut("homePageLastSelected")
        .and_then(Value::as_object_mut)
    else {
        return Ok(false);
    };
    last_selected.insert(
        "javaConfiguration".into(),
        Value::Object(document.selection),
    );

    let encoded =
        serde_json::to_string(&document.settings).map_err(|_| LauncherError::ProfileWriteFailed)?;
    let Some(data) = document.root.get_mut("data").and_then(Value::as_object_mut) else {
        return Ok(false);
    };
    data.insert("UiSettings".into(), Value::String(encoded));

    let body = serde_json::to_string_pretty(&document.root)
        .map_err(|_| LauncherError::ProfileWriteFailed)?;
    write_atomic(path, format!("{}\r\n{body}", document.banner).as_bytes())?;
    Ok(true)
}

/// Write through a temp file, keeping one Agora-owned backup of the launcher's
/// original so a bad write is recoverable by hand.
fn write_atomic(path: &Path, bytes: &[u8]) -> LauncherResult<()> {
    let backup = path.with_extension("json.agora-bak");
    if !backup.exists() {
        let _ = std::fs::copy(path, &backup);
    }
    let temp = path.with_extension(format!("json.agora-{}.tmp", std::process::id()));
    std::fs::write(&temp, bytes).map_err(|_| LauncherError::ProfileWriteFailed)?;
    if std::fs::rename(&temp, path).is_err() {
        let _ = std::fs::remove_file(&temp);
        return Err(LauncherError::ProfileWriteFailed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BANNER: &str = "#$\r\nThis file is automatically generated by the Minecraft Launcher and is intended for internal use only.\r\nDO NOT EDIT\r\n$#";

    fn entry(profile_id: &str, name: &str) -> LauncherProfileEntry {
        LauncherProfileEntry {
            profile_id: profile_id.into(),
            name: name.into(),
            last_version_id: "fabric-loader-0.19.5-26.2".into(),
            game_dir: PathBuf::from("C:\\agora\\instances\\apples"),
            java_args: "-Xmx4096M".into(),
            select: true,
        }
    }

    /// Mirrors the real file: a banner, then a document whose `UiSettings` is a
    /// JSON *string*, with the selection nested two levels inside that.
    fn write_ui_state(dir: &Path, selected_id: &str) -> PathBuf {
        let settings = serde_json::json!({
            "lastVisitedPage": "java",
            "isSFXEnabled": true,
            "homePageLastSelected": {
                "productId": "java",
                "displayName": "Minecraft: Java Edition",
                "javaConfiguration": {
                    "id": selected_id,
                    "name": "pack 1 (Agora)",
                    "versionId": "fabric-loader-0.19.3-26.2",
                    "gameDir": "C:\\agora\\instances\\pack-1",
                    "javaArgs": "-Xmx2048M",
                    "type": "custom",
                    "icon": "Furnace",
                    "lastUsed": "2026-08-21T03:30:11.665Z",
                    "created": "2026-08-21T03:29:59.855Z"
                },
                "gameIconImage": "data:image/png;base64,AAAA"
            }
        });
        let root = serde_json::json!({
            "data": {
                "UiEvents": "{\"hasSeenDialog\":{}}",
                "UiSettings": serde_json::to_string(&settings).unwrap(),
            }
        });
        let path = dir.join("launcher_ui_state_microsoft_store.json");
        std::fs::write(
            &path,
            format!(
                "{BANNER}\r\n{}",
                serde_json::to_string_pretty(&root).unwrap()
            ),
        )
        .unwrap();
        path
    }

    fn read_selection(path: &Path) -> Map<String, Value> {
        parse_document(path).unwrap().selection
    }

    #[test]
    fn selects_the_requested_installation() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_ui_state(dir.path(), "agora-pack-1");

        let updated =
            select_installation(dir.path(), &entry("agora-apples", "apples (Agora)"), "NOW")
                .unwrap();

        assert_eq!(updated, vec![path.clone()]);
        let selection = read_selection(&path);
        assert_eq!(selection["id"], "agora-apples");
        assert_eq!(selection["name"], "apples (Agora)");
        assert_eq!(selection["versionId"], "fabric-loader-0.19.5-26.2");
        assert_eq!(selection["lastUsed"], "NOW");
        // Keys the launcher owns and Agora has no opinion about survive.
        assert_eq!(selection["icon"], "Furnace");
        assert_eq!(selection["created"], "2026-08-21T03:29:59.855Z");
    }

    #[test]
    fn preserves_the_banner_and_every_other_setting() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_ui_state(dir.path(), "agora-pack-1");

        select_installation(dir.path(), &entry("agora-apples", "apples (Agora)"), "NOW").unwrap();

        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(
            raw.starts_with(BANNER),
            "banner must be written back verbatim"
        );
        let document = parse_document(&path).unwrap();
        assert_eq!(document.settings["lastVisitedPage"], "java");
        assert_eq!(document.settings["isSFXEnabled"], true);
        assert_eq!(
            document.settings["homePageLastSelected"]["gameIconImage"],
            "data:image/png;base64,AAAA"
        );
        assert_eq!(document.root["data"]["UiEvents"], "{\"hasSeenDialog\":{}}");
    }

    #[test]
    fn unrecognized_file_is_skipped_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("launcher_ui_state_microsoft_store.json");
        // No banner, and a shape this code does not understand.
        std::fs::write(&path, r#"{"data":{"UiSettings":"{}"}}"#).unwrap();

        let updated =
            select_installation(dir.path(), &entry("agora-apples", "apples (Agora)"), "NOW")
                .unwrap();

        assert!(updated.is_empty());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            r#"{"data":{"UiSettings":"{}"}}"#,
            "a file Agora cannot parse must be left byte-identical"
        );
    }

    #[test]
    fn missing_file_is_not_created() {
        let dir = tempfile::tempdir().unwrap();

        let updated =
            select_installation(dir.path(), &entry("agora-apples", "apples (Agora)"), "NOW")
                .unwrap();

        assert!(updated.is_empty());
        assert!(!dir
            .path()
            .join("launcher_ui_state_microsoft_store.json")
            .exists());
    }

    #[test]
    fn clearing_only_touches_the_deleted_profile() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_ui_state(dir.path(), "agora-pack-1");
        let survivor = entry("agora-apples", "apples (Agora)");

        // A different profile was deleted — the selection must stay put.
        let untouched =
            clear_selection_for(dir.path(), "agora-other", Some((&survivor, "NOW"))).unwrap();
        assert!(untouched.is_empty());
        assert_eq!(read_selection(&path)["id"], "agora-pack-1");

        let updated =
            clear_selection_for(dir.path(), "agora-pack-1", Some((&survivor, "NOW"))).unwrap();
        assert_eq!(updated, vec![path.clone()]);
        assert_eq!(read_selection(&path)["id"], "agora-apples");
    }

    #[test]
    fn clearing_without_a_replacement_leaves_the_file_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_ui_state(dir.path(), "agora-pack-1");
        let before = std::fs::read_to_string(&path).unwrap();

        let updated = clear_selection_for(dir.path(), "agora-pack-1", None).unwrap();

        assert!(updated.is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn reports_the_selected_profile_id() {
        let dir = tempfile::tempdir().unwrap();
        write_ui_state(dir.path(), "agora-pack-1");

        assert_eq!(
            selected_profile_id(dir.path()),
            Some("agora-pack-1".to_owned())
        );
        assert_eq!(selected_profile_id(&dir.path().join("nowhere")), None);
    }

    #[test]
    fn keeps_a_backup_of_the_launchers_original_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_ui_state(dir.path(), "agora-pack-1");
        let original = std::fs::read_to_string(&path).unwrap();

        select_installation(dir.path(), &entry("agora-apples", "apples (Agora)"), "NOW").unwrap();
        select_installation(
            dir.path(),
            &entry("agora-grapes", "grapes (Agora)"),
            "LATER",
        )
        .unwrap();

        let backup = path.with_extension("json.agora-bak");
        assert_eq!(
            std::fs::read_to_string(backup).unwrap(),
            original,
            "the backup must stay the launcher's own file, not the previous Agora write"
        );
    }
}
