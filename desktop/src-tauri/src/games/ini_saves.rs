//! INI and saves: the game's configuration files and save folder, per instance.
//! Owned by the `ini_saves` slice. Every command here is a stub until that slice lands.

use super::error::GamesError;

/// Stub: read a value from one of the game's INI files.
#[tauri::command]
pub async fn games_ini_get() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_ini_get"))
}

/// Stub: write a value to one of the game's INI files.
#[tauri::command]
pub async fn games_ini_set() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_ini_set"))
}

/// Stub: the save folder an instance uses.
#[tauri::command]
pub async fn games_saves_get() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_saves_get"))
}

/// Stub: change which save folder an instance uses.
#[tauri::command]
pub async fn games_saves_set() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_saves_set"))
}
