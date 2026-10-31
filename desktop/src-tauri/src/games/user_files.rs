//! User files: the game's own files (configs, saves) that Agora backs up and can restore
//! when a deployment changes them. Owned by the `user_files` slice. A thin adapter over
//! `agora_core::game_user_files`, the service `agora games user-files` calls.

use agora_core::game_user_files::{self, RestoreReport, SessionStatus};
use serde::Serialize;

use super::error::{GamesError, CORE_FAILED, TASK_FAILED};

/// The game id is not one Agora supports.
const BAD_GAME: &str = "bad_game";
/// No file-swap session exists for the game and store the request named.
const NO_SESSION: &str = "no_session";

/// The file-swap sessions in progress, or recoverable, for one game (or every game).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserFilesStatus {
    pub sessions: Vec<SessionDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDto {
    pub game_id: String,
    pub store: String,
    pub instance_id: String,
    /// The game is running on the swapped files, so a restore is refused.
    pub running: bool,
    pub files: Vec<SessionFileDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionFileDto {
    pub real_path: String,
    /// The game's own file is currently replaced by Agora's copy.
    pub swapped: bool,
}

/// The result of putting a session's files back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreDto {
    pub game_id: String,
    pub store: String,
    pub instance_id: String,
    pub files: Vec<RestoredFileDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoredFileDto {
    pub real_path: String,
    /// The file differed from what Agora last wrote, so its contents were not the same as before.
    pub changed: bool,
    pub original_sha256: Option<String>,
    pub final_sha256: Option<String>,
}

/// The sessions for one game (`game_id`), or for every game when none is given.
#[tauri::command]
pub async fn games_user_files_status(
    app: tauri::AppHandle,
    game_id: Option<String>,
) -> Result<UserFilesStatus, GamesError> {
    tokio::task::spawn_blocking(move || -> Result<UserFilesStatus, GamesError> {
        let ctx = crate::core_context(&app)?;
        let statuses = match game_id {
            None => game_user_files::list_statuses(&ctx, None).map_err(core_failed)?,
            Some(id) => {
                let wanted = id.trim().to_string();
                let game = ctx
                    .games
                    .games()
                    .find(|game| game.id.as_str() == wanted)
                    .ok_or_else(|| unsupported_game(&wanted))?;
                game_user_files::list_statuses(&ctx, Some(&game.id)).map_err(core_failed)?
            }
        };
        Ok(UserFilesStatus {
            sessions: statuses
                .into_iter()
                .map(|(game, store, status)| session_dto(game.as_str(), store.as_str(), status))
                .collect(),
        })
    })
    .await
    .map_err(|_| {
        GamesError::new(
            TASK_FAILED,
            "Reading the user-file sessions did not finish.",
        )
    })?
}

/// Put one session's files back from Agora's backup. Refused while the game runs on them.
#[tauri::command]
pub async fn games_user_files_restore(
    app: tauri::AppHandle,
    game_id: String,
    store: String,
) -> Result<RestoreDto, GamesError> {
    tokio::task::spawn_blocking(move || -> Result<RestoreDto, GamesError> {
        let ctx = crate::core_context(&app)?;
        let wanted_game = game_id.trim().to_string();
        let wanted_store = store.trim().to_string();
        let game = ctx
            .games
            .games()
            .find(|game| game.id.as_str() == wanted_game)
            .ok_or_else(|| unsupported_game(&wanted_game))?;
        // The store id comes from the session that exists, never from the request alone, so a
        // restore only ever runs on a session the journal names.
        let store_id = game_user_files::list_statuses(&ctx, Some(&game.id))
            .map_err(core_failed)?
            .into_iter()
            .map(|(_, store_id, _)| store_id)
            .find(|candidate| candidate.as_str() == wanted_store)
            .ok_or_else(|| {
                GamesError::new(
                    NO_SESSION,
                    format!(
                        "There is no user-file session for {wanted_game} in {wanted_store}, so there is nothing to restore."
                    ),
                )
            })?;
        let report = game_user_files::restore(&ctx, game, &store_id).map_err(core_failed)?;
        Ok(restore_dto(report))
    })
    .await
    .map_err(|_| GamesError::new(TASK_FAILED, "Restoring the user files did not finish."))?
}

fn unsupported_game(game_id: &str) -> GamesError {
    GamesError::new(
        BAD_GAME,
        format!("Agora does not support a game called '{game_id}'."),
    )
}

fn session_dto(game: &str, store: &str, status: SessionStatus) -> SessionDto {
    SessionDto {
        game_id: game.to_string(),
        store: store.to_string(),
        instance_id: status.instance,
        running: status.running,
        files: status
            .files
            .into_iter()
            .map(|file| SessionFileDto {
                real_path: file.real_path.to_string_lossy().into_owned(),
                swapped: file.swapped,
            })
            .collect(),
    }
}

fn restore_dto(report: RestoreReport) -> RestoreDto {
    RestoreDto {
        game_id: report.game.as_str().to_string(),
        store: report.store.as_str().to_string(),
        instance_id: report.instance_id,
        files: report
            .files
            .into_iter()
            .map(|file| RestoredFileDto {
                real_path: file.real_path.to_string_lossy().into_owned(),
                changed: file.changed,
                original_sha256: file.original_sha256,
                final_sha256: file.final_sha256,
            })
            .collect(),
    }
}

fn core_failed(error: impl std::fmt::Display) -> GamesError {
    GamesError::new(CORE_FAILED, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A session as the journal stores it, read through the same serde shape.
    fn status_from_journal(running: bool) -> SessionStatus {
        serde_json::from_value(serde_json::json!({
            "instance": "inst-9",
            "running": running,
            "files": [{
                "real_path": "C:/Users/me/Documents/My Games/Skyrim.ini",
                "instance_path": "Skyrim.ini",
                "instance_copy_path": "C:/copy/Skyrim.ini",
                "existed": true,
                "written_sha256": "00",
                "swapped": true
            }]
        }))
        .expect("the journal's session shape reads back")
    }

    #[test]
    fn a_session_reports_its_files_and_whether_the_game_runs() {
        let dto = session_dto("skyrim-se", "steam", status_from_journal(true));
        assert_eq!(dto.game_id, "skyrim-se");
        assert_eq!(dto.store, "steam");
        assert_eq!(dto.instance_id, "inst-9");
        assert!(dto.running);
        assert_eq!(dto.files.len(), 1);
        assert!(dto.files[0].swapped);
        assert!(dto.files[0].real_path.ends_with("Skyrim.ini"));
    }

    #[test]
    fn a_session_that_is_not_running_says_so() {
        let dto = session_dto("skyrim-se", "gog", status_from_journal(false));
        assert!(!dto.running);
    }
}
