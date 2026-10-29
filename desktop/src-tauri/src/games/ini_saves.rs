//! INI and saves (MASTER_SPEC §26.7): an instance's per-user INI files and its save choice. Owned by
//! the `plugins` slice (the "INI & saves" tab).
//!
//! Each command is an adapter over `agora_core::game_ini` and `game_saves`: the same functions
//! `agora games instance ini|saves` calls, so the desktop and the CLI cannot disagree about a key's
//! value or which save folder the game uses. A core error reaches the user as its message.

use agora_core::game_ini::{self, IniFile, IniRead, IniSource};
use agora_core::game_instance::SavesChoice;
use agora_core::game_saves::{self, SaveFolder, SavesStatus, SettingChange};
use serde::Serialize;

use super::error::GamesError;
use super::plugins::{core_failed, game_definition, run_blocking};

/// The request cannot be done as asked: a save choice that is neither `own` nor `shared`. It is
/// not a core code, so it is named here rather than added to `error.rs`.
const INVALID_REQUEST: &str = "invalid_request";

// ---------------------------------------------------------------------------
// INI files
// ---------------------------------------------------------------------------

/// One per-user file the game keeps for the instance's store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IniFileRow {
    /// The file's name inside the instance, as the game names it (for example `Skyrim.ini`).
    pub instance_path: String,
    /// Whether the instance has its own copy of the file yet. A copy is made on the first edit.
    pub copy_exists: bool,
    /// Where the game keeps its own file for this store.
    pub game_file: String,
    pub game_file_exists: bool,
}

/// One key of an INI file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IniEntryRow {
    pub section: String,
    pub key: String,
    pub value: String,
}

/// A file as the instance sees it: its keys, and where they came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IniContents {
    pub instance_path: String,
    /// `copy` (the instance's own file), `game_file` (the game's file, as the instance has no copy
    /// yet) or `nothing` (neither exists, so the file reads as empty).
    pub source: String,
    pub entries: Vec<IniEntryRow>,
}

/// The instance's INI files, and the file the request named, if one was named.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IniView {
    pub files: Vec<IniFileRow>,
    /// Set when the request named a file. Reading never creates the instance's copy.
    pub contents: Option<IniContents>,
}

/// A key set or removed, with the file as it is after the change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IniChange {
    /// Whether the file actually changed. False when the key already had that value.
    pub changed: bool,
    pub view: IniView,
}

fn ini_source_name(source: IniSource) -> &'static str {
    match source {
        IniSource::Copy => "copy",
        IniSource::GameFile => "game_file",
        IniSource::Nothing => "nothing",
    }
}

fn ini_file_row(file: &IniFile) -> IniFileRow {
    IniFileRow {
        instance_path: file.instance_path.clone(),
        copy_exists: file.copy_exists,
        game_file: file.game_file.display().to_string(),
        game_file_exists: file.game_file_exists,
    }
}

fn ini_contents(read: IniRead) -> IniContents {
    IniContents {
        instance_path: read.instance_path,
        source: ini_source_name(read.source).to_string(),
        entries: read
            .document
            .entries()
            .into_iter()
            .map(|entry| IniEntryRow {
                section: entry.section,
                key: entry.key,
                value: entry.value,
            })
            .collect(),
    }
}

/// The INI files an instance's store has, and the named file's keys when a file is named. An
/// empty `file` is refused by core as a bad path, not read as "no file".
#[tauri::command]
pub async fn games_ini_get(
    app: tauri::AppHandle,
    instance_id: String,
    file: Option<String>,
) -> Result<IniView, GamesError> {
    run_blocking(app, "Reading the INI files", move |ctx| {
        game_definition!(&ctx, &instance_id, definition);
        let files = game_ini::list_files(&ctx, &instance_id, definition).map_err(core_failed)?;
        let contents = match file {
            None => None,
            Some(name) => {
                let read =
                    game_ini::read(&ctx, &instance_id, definition, &name).map_err(core_failed)?;
                Some(ini_contents(read))
            }
        };
        Ok(IniView {
            files: files.iter().map(ini_file_row).collect(),
            contents,
        })
    })
    .await
}

/// Set a key in one of the instance's INI files, or remove it when `value` is `None` (the CLI's
/// `--unset`). Refused while the game is running. Returns the file as it is after the change.
#[tauri::command]
pub async fn games_ini_set(
    app: tauri::AppHandle,
    instance_id: String,
    file: String,
    section: String,
    key: String,
    value: Option<String>,
) -> Result<IniChange, GamesError> {
    run_blocking(app, "Changing the INI file", move |ctx| {
        game_definition!(&ctx, &instance_id, definition);
        let changed = match &value {
            Some(value) => {
                game_ini::set_value(&ctx, &instance_id, definition, &file, &section, &key, value)
            }
            None => game_ini::unset_value(&ctx, &instance_id, definition, &file, &section, &key),
        }
        .map_err(core_failed)?;
        let files = game_ini::list_files(&ctx, &instance_id, definition).map_err(core_failed)?;
        let read = game_ini::read(&ctx, &instance_id, definition, &file).map_err(core_failed)?;
        Ok(IniChange {
            changed,
            view: IniView {
                files: files.iter().map(ini_file_row).collect(),
                contents: Some(ini_contents(read)),
            },
        })
    })
    .await
}

// ---------------------------------------------------------------------------
// Saves
// ---------------------------------------------------------------------------

/// One save folder: its path, how many saves it holds and the newest one's time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveFolderRow {
    pub path: String,
    /// False when the folder has not been made yet. That is not an error: the game makes it.
    pub exists: bool,
    pub saves: usize,
    /// The newest save's modification time, RFC 3339 in UTC.
    pub newest: Option<String>,
}

/// The instance's save choice, its two folders, and the game's setting that points at one of them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavesView {
    pub instance_id: String,
    /// `own` (the instance's own saves) or `shared` (the game's shared save folder).
    pub choice: String,
    /// The folder the game reads and writes for this instance.
    pub in_use: SaveFolderRow,
    /// The other folder. Its saves stay where they are when the choice changes.
    pub other: SaveFolderRow,
    /// The per-user file that holds the game's save setting, and `section/key` of the setting.
    pub setting_file: String,
    pub setting: String,
    /// The setting's value as the instance sees it. `None` when the key is not set.
    pub setting_value: Option<String>,
    pub setting_source: String,
}

/// What a change of the save choice did to the game's setting, in words the UI can show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingChangeRow {
    /// `set`, `unchanged`, `restored`, `removed`, `left_alone` or `nothing_recorded`.
    pub action: String,
    /// The value the setting holds now, when it holds one.
    pub value: Option<String>,
}

/// A change of the save choice, with the saves view as it is after the change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavesChange {
    /// The choice before the change: `own` or `shared`.
    pub previous: String,
    pub setting: SettingChangeRow,
    pub saves: SavesView,
}

fn save_folder_row(folder: &SaveFolder) -> SaveFolderRow {
    SaveFolderRow {
        path: folder.path.display().to_string(),
        exists: folder.exists,
        saves: folder.saves,
        newest: folder.newest.clone(),
    }
}

fn saves_view(status: &SavesStatus) -> SavesView {
    SavesView {
        instance_id: status.instance_id.clone(),
        choice: status.choice.as_str().to_string(),
        in_use: save_folder_row(&status.in_use),
        other: save_folder_row(&status.other),
        setting_file: status.setting_file.clone(),
        setting: status.setting.clone(),
        setting_value: status.setting_value.clone(),
        setting_source: ini_source_name(status.setting_source).to_string(),
    }
}

fn setting_change_row(change: &SettingChange) -> SettingChangeRow {
    let (action, value) = match change {
        SettingChange::Set { value } => ("set", Some(value.clone())),
        SettingChange::Unchanged => ("unchanged", None),
        SettingChange::Restored { value } => ("restored", Some(value.clone())),
        SettingChange::Removed => ("removed", None),
        SettingChange::LeftAlone { now } => ("left_alone", now.clone()),
        SettingChange::NothingRecorded => ("nothing_recorded", None),
    };
    SettingChangeRow {
        action: action.to_string(),
        value,
    }
}

/// A save choice from the request. Only the two names the CLI accepts; anything else is refused
/// before core is asked, so no folder is touched for a choice Agora does not know.
fn parse_choice(choice: &str) -> Result<SavesChoice, GamesError> {
    match choice {
        "own" => Ok(SavesChoice::Own),
        "shared" => Ok(SavesChoice::Shared),
        other => Err(GamesError::new(
            INVALID_REQUEST,
            format!("'{other}' is not a save choice: use own or shared."),
        )),
    }
}

/// The instance's save choice, its folders, and the game's save setting as the instance sees it.
#[tauri::command]
pub async fn games_saves_get(
    app: tauri::AppHandle,
    instance_id: String,
) -> Result<SavesView, GamesError> {
    run_blocking(app, "Reading the save choice", move |ctx| {
        game_definition!(&ctx, &instance_id, definition);
        let status = game_saves::status(&ctx, &instance_id, definition).map_err(core_failed)?;
        Ok(saves_view(&status))
    })
    .await
}

/// Make the instance's save choice `own` or `shared`. Never moves, copies or deletes a save. Refused
/// while the game is running.
#[tauri::command]
pub async fn games_saves_set(
    app: tauri::AppHandle,
    instance_id: String,
    choice: String,
) -> Result<SavesChange, GamesError> {
    let wanted = parse_choice(&choice)?;
    run_blocking(app, "Changing the save choice", move |ctx| {
        game_definition!(&ctx, &instance_id, definition);
        let change =
            game_saves::set_choice(&ctx, &instance_id, definition, wanted).map_err(core_failed)?;
        let status = game_saves::status(&ctx, &instance_id, definition).map_err(core_failed)?;
        Ok(SavesChange {
            previous: change.previous.as_str().to_string(),
            setting: setting_change_row(&change.setting),
            saves: saves_view(&status),
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use agora_core::game_ini::IniDocument;

    fn folder(exists: bool, saves: usize) -> SaveFolder {
        SaveFolder {
            path: std::path::PathBuf::from(
                "C:/Users/player/Documents/My Games/Skyrim Special Edition/Saves",
            ),
            exists,
            saves,
            newest: None,
        }
    }

    #[test]
    fn only_own_and_shared_are_save_choices() {
        assert_eq!(parse_choice("own").unwrap(), SavesChoice::Own);
        assert_eq!(parse_choice("shared").unwrap(), SavesChoice::Shared);
    }

    #[test]
    fn a_choice_that_is_not_exactly_own_or_shared_is_refused_with_its_code() {
        // Hostile input: empty, padded, wrong case, a plural, a path, and a near miss. None of
        // them may fall through to a folder.
        for bad in [
            "",
            " own",
            "own ",
            "Own",
            "SHARED",
            "both",
            "own/shared",
            "..",
            "sh\u{0}ared",
        ] {
            let error = parse_choice(bad).expect_err(bad);
            assert_eq!(error.code, INVALID_REQUEST, "for {bad:?}");
            assert!(error.message.contains("use own or shared"), "for {bad:?}");
            assert_eq!(error.core_code, None);
        }
    }

    #[test]
    fn every_core_setting_action_has_a_named_row() {
        let cases = [
            (SettingChange::Set { value: "a".into() }, "set", Some("a")),
            (SettingChange::Unchanged, "unchanged", None),
            (
                SettingChange::Restored { value: "b".into() },
                "restored",
                Some("b"),
            ),
            (SettingChange::Removed, "removed", None),
            (
                SettingChange::LeftAlone {
                    now: Some("c".into()),
                },
                "left_alone",
                Some("c"),
            ),
            (SettingChange::LeftAlone { now: None }, "left_alone", None),
            (SettingChange::NothingRecorded, "nothing_recorded", None),
        ];
        for (change, action, value) in cases {
            let row = setting_change_row(&change);
            assert_eq!(row.action, action);
            assert_eq!(row.value.as_deref(), value);
        }
    }

    #[test]
    fn a_missing_save_folder_reads_as_not_made_yet_not_as_an_error() {
        let row = save_folder_row(&folder(false, 0));
        assert!(!row.exists);
        assert_eq!(row.saves, 0);
        assert!(row.path.ends_with("Saves"));
    }

    #[test]
    fn each_ini_source_has_its_own_name() {
        assert_eq!(ini_source_name(IniSource::Copy), "copy");
        assert_eq!(ini_source_name(IniSource::GameFile), "game_file");
        assert_eq!(ini_source_name(IniSource::Nothing), "nothing");
    }

    #[test]
    fn ini_contents_keep_every_key_in_file_order() {
        let read = IniRead {
            instance_path: "Skyrim.ini".to_string(),
            source: IniSource::GameFile,
            document: IniDocument::parse(
                b"[General]\r\nsLanguage=ENGLISH\r\n[Display]\nfGamma=1.0\n",
            ),
        };
        let contents = ini_contents(read);
        assert_eq!(contents.source, "game_file");
        let keys: Vec<(&str, &str, &str)> = contents
            .entries
            .iter()
            .map(|e| (e.section.as_str(), e.key.as_str(), e.value.as_str()))
            .collect();
        assert_eq!(
            keys,
            vec![
                ("General", "sLanguage", "ENGLISH"),
                ("Display", "fGamma", "1.0")
            ]
        );
    }

    #[test]
    fn the_saves_view_serialises_with_the_names_the_frontend_reads() {
        let view = SavesView {
            instance_id: "abc".to_string(),
            choice: "own".to_string(),
            in_use: save_folder_row(&folder(true, 2)),
            other: save_folder_row(&folder(false, 0)),
            setting_file: "Skyrim.ini".to_string(),
            setting: "General/sSaveFolder".to_string(),
            setting_value: None,
            setting_source: "nothing".to_string(),
        };
        let json = serde_json::to_value(&view).expect("serialises");
        assert_eq!(json["instanceId"], "abc");
        assert_eq!(json["inUse"]["exists"], true);
        assert!(
            json["settingValue"].is_null(),
            "an unset key is null, not an empty string"
        );
        assert!(json.get("instance_id").is_none());
    }
}
