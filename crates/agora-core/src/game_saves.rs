//! An instance's save choice (MASTER_SPEC §26.5, *Saves are the user's choice per instance*).
//!
//! An instance keeps the game's shared save folder, or saves of its own. For its own saves, the
//! game's setting (`SLocalSavePath` under `[General]` in `Skyrim.ini`, for Skyrim) is set to a
//! folder named for the instance, relative to the game's `My Games` folder. Switching changes only
//! where the game looks: no save file is moved, copied or deleted.
//!
//! Turning the choice on records the value the setting held before, in `saves_state.json` in the
//! instance's folder. Turning it off puts that value back, or removes the setting when it was
//! absent, and only if the setting still holds the value Agora set. A setting changed by hand since
//! is left as it is, and the user is told.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use agora_game_api::{GameDefinition, GamePath, SaveLocationRule, StoreId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ctx::Ctx;
use crate::game_deploy::write_instance_manifest_atomic;
use crate::game_ini::{self, IniError, IniSource};
use crate::game_instance::{self, GameInstanceManifest, InstanceError, SavesChoice};
use crate::game_user_files::resolve_user_file_source;

const STATE_FILE: &str = "saves_state.json";
/// The longest plain instance id used as a folder name as it is. Longer ids are shortened and
/// given a hash, so a save path stays well inside the Windows path limit.
const PLAIN_NAME_MAX: usize = 48;
/// The most characters of an unusual instance id kept in its folder name, before the hash.
const CLEANED_NAME_MAX: usize = 40;

#[derive(Debug, thiserror::Error)]
pub enum SavesError {
    #[error("instance '{0}' not found")]
    NotFound(String),
    #[error("{game} declares no save location for the '{store}' store, so the instance cannot have saves of its own")]
    NoSaveLocation { game: String, store: String },
    #[error(transparent)]
    Ini(#[from] IniError),
    #[error("{0}")]
    Other(String),
    #[error("cannot write {}: {source}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot read {}: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// What Agora changed in the game's setting when the instance took its own saves: the section and
/// key, the value Agora set, and the value the key held before (`None` when it was absent).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavesState {
    /// The per-user file holding the setting (an `instance_path`).
    pub ini: String,
    pub section: String,
    pub key: String,
    pub set_to: String,
    pub previous: Option<String>,
}

/// A folder of saves and what is in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SaveFolder {
    pub path: PathBuf,
    pub exists: bool,
    pub saves: usize,
    /// The newest save's modification time, RFC 3339 in UTC.
    pub newest: Option<String>,
}

/// The save choice of an instance and the folders involved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SavesStatus {
    pub instance_id: String,
    pub choice: SavesChoice,
    /// The folder the game reads and writes for this instance.
    pub in_use: SaveFolder,
    /// The other folder, whose saves stay where they are when the choice changes.
    pub other: SaveFolder,
    /// The game's setting as the instance sees it now, and where that value came from.
    pub setting_file: String,
    pub setting: String,
    pub setting_value: Option<String>,
    pub setting_source: IniSource,
}

/// What a change of the save choice did to the game's setting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SettingChange {
    /// The setting now names the instance's folder.
    Set { value: String },
    /// The setting already named the instance's folder.
    Unchanged,
    /// The setting was put back to the value it had before.
    Restored { value: String },
    /// The setting was absent before, so it was removed.
    Removed,
    /// The setting no longer holds the value Agora set, so it was left as it is.
    LeftAlone { now: Option<String> },
    /// Nothing was recorded for this instance, so there was nothing to put back.
    NothingRecorded,
}

/// The result of a change of the save choice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SavesChange {
    pub instance_id: String,
    pub previous: SavesChoice,
    pub choice: SavesChoice,
    pub setting: SettingChange,
    /// The folder the game reads and writes now.
    pub in_use: SaveFolder,
    /// The other folder: its saves were not moved.
    pub other: SaveFolder,
}

/// The rule that applies to this store, if the game has one.
pub fn rule_for<'a>(
    definition: &'a GameDefinition,
    store: &StoreId,
) -> Option<&'a SaveLocationRule> {
    definition
        .save_location
        .iter()
        .find(|rule| rule.applies_to_store(store))
}

/// The save folder name for an instance. A plain id is its own name. Any other id (one with spaces,
/// punctuation, a Windows reserved name, or a length that is too long) becomes a name of safe
/// characters followed by a hash of the whole id, so two ids never share a folder.
pub fn save_folder_name(instance_id: &str) -> String {
    let plain = !instance_id.is_empty()
        && instance_id.len() <= PLAIN_NAME_MAX
        && instance_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        && !is_reserved_device_name(instance_id);
    if plain {
        return instance_id.to_string();
    }
    let cleaned: String = instance_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(CLEANED_NAME_MAX)
        .collect();
    let cleaned = if cleaned.is_empty() {
        "instance".to_string()
    } else {
        cleaned
    };
    let digest = format!("{:x}", Sha256::digest(instance_id.as_bytes()));
    format!("{cleaned}-{}", &digest[..12])
}

/// Windows refuses these names as a path component, whatever comes after them.
fn is_reserved_device_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    let base = upper.split('.').next().unwrap_or("");
    matches!(base, "CON" | "PRN" | "AUX" | "NUL")
        || (base.len() == 4
            && (base.starts_with("COM") || base.starts_with("LPT"))
            && base[3..].bytes().all(|b| (b'1'..=b'9').contains(&b)))
}

/// The setting's value for this instance: the rule's `own_value` with the folder name filled in.
pub fn own_value(rule: &SaveLocationRule, instance_id: &str) -> String {
    rule.own_value
        .replace("{instance}", &save_folder_name(instance_id))
}

fn resolve(path: &GamePath) -> Result<PathBuf, SavesError> {
    resolve_user_file_source(path).map_err(|e| SavesError::Other(e.to_string()))
}

/// The game's shared save folder for the rule.
pub fn shared_folder(rule: &SaveLocationRule) -> Result<PathBuf, SavesError> {
    resolve(&rule.shared_dir)
}

/// The instance's own save folder: `own_value` under the rule's `relative_to` folder.
pub fn own_folder(rule: &SaveLocationRule, instance_id: &str) -> Result<PathBuf, SavesError> {
    let mut folder = resolve(&rule.relative_to)?;
    for part in own_value(rule, instance_id)
        .split(['\\', '/'])
        .filter(|p| !p.is_empty())
    {
        folder.push(part);
    }
    Ok(folder)
}

fn survey(path: &Path) -> Result<SaveFolder, SavesError> {
    let mut saves = 0usize;
    let mut newest: Option<SystemTime> = None;
    let exists = path.is_dir();
    if exists {
        let entries = std::fs::read_dir(path).map_err(|source| SavesError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        for entry in entries.flatten() {
            let file = entry.path();
            let is_save = file
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("ess"))
                && file.is_file();
            if !is_save {
                continue;
            }
            saves += 1;
            if let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
                newest = Some(newest.map_or(modified, |n| n.max(modified)));
            }
        }
    }
    Ok(SaveFolder {
        path: path.to_path_buf(),
        exists,
        saves,
        newest: newest.map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339()),
    })
}

fn manifest_of(ctx: &Ctx, instance_id: &str) -> Result<GameInstanceManifest, SavesError> {
    game_instance::get_manifest(ctx, instance_id).map_err(|e| match e {
        InstanceError::NotFound(id) => SavesError::NotFound(id),
        other => SavesError::Other(other.to_string()),
    })
}

fn state_path(ctx: &Ctx, instance_id: &str) -> Result<PathBuf, SavesError> {
    Ok(ctx
        .paths
        .instance_dir(instance_id)
        .map_err(|e| SavesError::Other(e.to_string()))?
        .join(STATE_FILE))
}

fn read_state(path: &Path) -> Result<Option<SavesState>, SavesError> {
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path).map_err(|source| SavesError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|e| SavesError::Other(format!("cannot read {}: {e}", path.display())))
}

fn write_state(path: &Path, state: &SavesState) -> Result<(), SavesError> {
    let bytes = serde_json::to_vec_pretty(state)
        .map_err(|e| SavesError::Other(format!("cannot record the save choice: {e}")))?;
    game_ini::write_atomic(path, &bytes).map_err(|e| match e {
        IniError::Write { path, source } => SavesError::Write { path, source },
        other => SavesError::Other(other.to_string()),
    })
}

/// The instance's save choice, its folders, and the game's setting as the instance sees it.
pub fn status(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
) -> Result<SavesStatus, SavesError> {
    let manifest = manifest_of(ctx, instance_id)?;
    let store = game_ini::store_of(ctx, instance_id)?;
    let rule = no_rule_is_an_error(definition, &store)?;
    let shared = survey(&shared_folder(rule)?)?;
    let own = survey(&own_folder(rule, instance_id)?)?;
    let (in_use, other) = match manifest.saves {
        SavesChoice::Shared => (shared, own),
        SavesChoice::Own => (own, shared),
    };
    let read = game_ini::read(ctx, instance_id, definition, rule.ini.as_str())?;
    Ok(SavesStatus {
        instance_id: instance_id.to_string(),
        choice: manifest.saves,
        in_use,
        other,
        setting_file: rule.ini.as_str().to_string(),
        setting: format!("{}/{}", rule.section, rule.key),
        setting_value: read.document.get(&rule.section, &rule.key),
        setting_source: read.source,
    })
}

fn no_rule_is_an_error<'a>(
    definition: &'a GameDefinition,
    store: &StoreId,
) -> Result<&'a SaveLocationRule, SavesError> {
    rule_for(definition, store).ok_or_else(|| SavesError::NoSaveLocation {
        game: definition.id.as_str().to_string(),
        store: store.as_str().to_string(),
    })
}

/// Make the instance's save choice `choice`. Refused while the game's session is running, like any
/// edit of the instance's INI files. Never moves, copies or deletes a save.
pub fn set_choice(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    choice: SavesChoice,
) -> Result<SavesChange, SavesError> {
    set_choice_inner(ctx, instance_id, definition, choice, true)
}

/// [`set_choice`]; with `create_folder` false it does not make the instance's own save folder. That
/// folder is under the player's Documents, so an import that was not asked to copy saves leaves it
/// alone. The setting still points at it.
pub(crate) fn set_choice_inner(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    choice: SavesChoice,
    create_folder: bool,
) -> Result<SavesChange, SavesError> {
    let manifest = manifest_of(ctx, instance_id)?;
    let store = game_ini::store_of(ctx, instance_id)?;
    let rule = no_rule_is_an_error(definition, &store)?.clone();
    let _locks = game_ini::lock_for_edit(ctx, instance_id, definition, &store)?;
    let state_file = state_path(ctx, instance_id)?;
    let recorded = read_state(&state_file)?;
    let own = own_value(&rule, instance_id);

    let setting = match choice {
        SavesChoice::Own => {
            let (before, changed) = game_ini::edit_locked(
                ctx,
                instance_id,
                definition,
                &store,
                rule.ini.as_str(),
                |doc| {
                    let before = doc.get(&rule.section, &rule.key);
                    let changed = doc.set(&rule.section, &rule.key, &own)?;
                    Ok(((before, changed), changed))
                },
            )?;
            // A second `own` keeps the value from before the first one, so toggling is stable.
            if recorded.is_none() {
                write_state(
                    &state_file,
                    &SavesState {
                        ini: rule.ini.as_str().to_string(),
                        section: rule.section.clone(),
                        key: rule.key.clone(),
                        set_to: own.clone(),
                        previous: before,
                    },
                )?;
            }
            if create_folder {
                let folder = own_folder(&rule, instance_id)?;
                std::fs::create_dir_all(&folder).map_err(|source| SavesError::Write {
                    path: folder.clone(),
                    source,
                })?;
            }
            if changed {
                SettingChange::Set { value: own }
            } else {
                SettingChange::Unchanged
            }
        }
        SavesChoice::Shared => match recorded {
            None => SettingChange::NothingRecorded,
            Some(state) => {
                let change = game_ini::edit_locked(
                    ctx,
                    instance_id,
                    definition,
                    &store,
                    &state.ini,
                    |doc| {
                        let now = doc.get(&state.section, &state.key);
                        if now.as_deref() != Some(state.set_to.as_str()) {
                            return Ok((SettingChange::LeftAlone { now }, false));
                        }
                        match &state.previous {
                            Some(value) => {
                                let changed = doc.set(&state.section, &state.key, value)?;
                                Ok((
                                    SettingChange::Restored {
                                        value: value.clone(),
                                    },
                                    changed,
                                ))
                            }
                            None => {
                                let changed = doc.unset(&state.section, &state.key)?;
                                Ok((SettingChange::Removed, changed))
                            }
                        }
                    },
                )?;
                // The record is spent either way: a later `own` records afresh.
                if state_file.exists() {
                    std::fs::remove_file(&state_file).map_err(|source| SavesError::Write {
                        path: state_file.clone(),
                        source,
                    })?;
                }
                change
            }
        },
    };

    if manifest.saves != choice {
        let mut updated = manifest.clone();
        updated.saves = choice;
        write_instance_manifest_atomic(ctx, instance_id, &updated)
            .map_err(|e| SavesError::Other(e.to_string()))?;
    }

    let shared = survey(&shared_folder(&rule)?)?;
    let own_survey = survey(&own_folder(&rule, instance_id)?)?;
    let (in_use, other) = match choice {
        SavesChoice::Shared => (shared, own_survey),
        SavesChoice::Own => (own_survey, shared),
    };
    Ok(SavesChange {
        instance_id: instance_id.to_string(),
        previous: manifest.saves,
        choice,
        setting,
        in_use,
        other,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_id_is_its_own_folder_name() {
        assert_eq!(save_folder_name("survival-run"), "survival-run");
        assert_eq!(save_folder_name("p3_modded"), "p3_modded");
    }

    #[test]
    fn unusual_ids_get_safe_distinct_names() {
        let long = "long".repeat(40);
        let ids = [
            "a:b",
            "a_b",
            "a b",
            "a=b;c#d",
            "CON",
            "con",
            "NUL.txt",
            "COM1",
            "LPT9",
            "trailing.",
            "  leading",
            "ünïcode ✓",
            "x\"y<z>|w?*",
            "a\u{0}b",
            "Survival run #2 (x=y); ok",
            long.as_str(),
        ];
        let mut seen = std::collections::BTreeSet::new();
        for id in ids {
            let name = save_folder_name(id);
            assert!(!name.is_empty());
            assert!(
                name.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
                "{id:?} gave {name:?}"
            );
            assert!(!is_reserved_device_name(&name), "{id:?} gave {name:?}");
            assert!(name.len() <= CLEANED_NAME_MAX + 13, "{id:?} gave {name:?}");
            assert!(seen.insert(name.clone()), "{id:?} collides at {name:?}");
        }
    }

    #[test]
    fn the_own_value_names_no_separator_or_setting_character() {
        let rule = SaveLocationRule {
            ini: agora_game_api::RelPath::new("user/Skyrim.ini").unwrap(),
            section: "General".into(),
            key: "SLocalSavePath".into(),
            own_value: "Saves\\Agora\\{instance}\\".into(),
            shared_dir: GamePath::Runtime {
                path: agora_game_api::RelPath::default(),
            },
            relative_to: GamePath::Runtime {
                path: agora_game_api::RelPath::default(),
            },
            stores: Vec::new(),
        };
        let value = own_value(&rule, "Run #2; x=y: \"ok\" .");
        assert!(value.starts_with("Saves\\Agora\\"));
        assert!(value.ends_with('\\'));
        let folder = value
            .trim_start_matches("Saves\\Agora\\")
            .trim_end_matches('\\');
        assert!(!folder.contains(['\\', '/', ':', ';', '#', '=', '"', ' ', '\r', '\n']));
        assert!(!folder.is_empty());
    }
}
