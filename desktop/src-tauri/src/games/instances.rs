//! Game instances: the list, an instance's page, its checks and its launch.
//! Owned by the `instances` slice. Every command here is a stub until that slice lands.

use super::error::GamesError;

/// Stub: every instance, across games.
#[tauri::command]
pub async fn games_instance_list() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_instance_list"))
}

/// Stub: one instance, for its page.
#[tauri::command]
pub async fn games_instance_get() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_instance_get"))
}

/// Stub: create an instance from a base.
#[tauri::command]
pub async fn games_instance_create() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_instance_create"))
}

/// Stub: delete an instance.
#[tauri::command]
pub async fn games_instance_delete() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_instance_delete"))
}

/// Stub: run the launch checks for an instance.
#[tauri::command]
pub async fn games_instance_check() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_instance_check"))
}

/// Stub: choose how an instance deploys its content (links, copies, virtual).
#[tauri::command]
pub async fn games_instance_set_deployment() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_instance_set_deployment"))
}

/// Stub: launch an instance.
#[tauri::command]
pub async fn games_instance_launch() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_instance_launch"))
}
