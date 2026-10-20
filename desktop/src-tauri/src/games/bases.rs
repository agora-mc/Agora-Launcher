//! Pinned bases: the verified copies of a game that instances are built from.
//! Owned by the `bases` slice. Every command here is a stub until that slice lands.

use super::error::GamesError;

/// Stub: list the bases built or known for each install.
#[tauri::command]
pub async fn games_base_list() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_base_list"))
}

/// Stub: build a base from an install.
#[tauri::command]
pub async fn games_base_build() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_base_build"))
}

/// Stub: check a base still matches the files it was built from.
#[tauri::command]
pub async fn games_base_verify() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_base_verify"))
}

/// Stub: remove a base no instance uses.
#[tauri::command]
pub async fn games_base_remove() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_base_remove"))
}
