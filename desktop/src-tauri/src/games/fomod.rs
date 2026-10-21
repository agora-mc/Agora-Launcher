//! FOMOD installers: the option pages a mod archive asks the user to answer.
//! Owned by the `fomod` slice. Every command here is a stub until that slice lands.

use super::error::GamesError;

/// Stub: the installer steps and options for an archive.
#[tauri::command]
pub async fn games_fomod_show() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_fomod_show"))
}

/// Stub: install an archive with the choices the user made.
#[tauri::command]
pub async fn games_fomod_install() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_fomod_install"))
}
