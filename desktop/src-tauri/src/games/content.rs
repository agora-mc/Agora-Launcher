//! Content: the store of mod archives and what each instance has enabled, in load order.
//! Owned by the `content` slice. Every command here is a stub until that slice lands.

use super::error::GamesError;

/// Stub: the content store.
#[tauri::command]
pub async fn games_content_list() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_content_list"))
}

/// Stub: add an archive to the content store.
#[tauri::command]
pub async fn games_content_add() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_content_add"))
}

/// Stub: check a stored archive against its recorded hash.
#[tauri::command]
pub async fn games_content_verify() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_content_verify"))
}

/// Stub: remove an archive from the content store.
#[tauri::command]
pub async fn games_content_remove() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_content_remove"))
}

/// Stub: the content an instance has, in load order.
#[tauri::command]
pub async fn games_instance_content_list() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_instance_content_list"))
}

/// Stub: add a stored item to an instance.
#[tauri::command]
pub async fn games_instance_content_add() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_instance_content_add"))
}

/// Stub: remove an item from an instance.
#[tauri::command]
pub async fn games_instance_content_remove() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_instance_content_remove"))
}

/// Stub: turn an instance's item on or off.
#[tauri::command]
pub async fn games_instance_content_set_enabled() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented(
        "games_instance_content_set_enabled",
    ))
}

/// Stub: move an item within an instance's load order.
#[tauri::command]
pub async fn games_instance_content_move() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_instance_content_move"))
}
