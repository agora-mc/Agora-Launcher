//! User files: the game's own files (configs, saves) that Agora backs up and can restore
//! when a deployment changes them. Owned by the `user_files` slice. Every command here is
//! a stub until that slice lands.

use super::error::GamesError;

/// Stub: which of the game's user files are backed up, and which are out of place.
#[tauri::command]
pub async fn games_user_files_status() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_user_files_status"))
}

/// Stub: restore a user file from its backup.
#[tauri::command]
pub async fn games_user_files_restore() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_user_files_restore"))
}
