//! Tools: external programs (such as BodySlide or Nemesis) that write into an instance,
//! with their output tracked so it can be diffed, rolled back or swapped.
//! Owned by the `tools` slice. Every command here is a stub until that slice lands.

use super::error::GamesError;

/// Stub: the tools an instance can run.
#[tauri::command]
pub async fn games_tools_list() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_tools_list"))
}

/// Stub: run a tool against an instance.
#[tauri::command]
pub async fn games_tools_run() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_tools_run"))
}

/// Stub: roll back the output a tool wrote.
#[tauri::command]
pub async fn games_tools_rollback() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_tools_rollback"))
}

/// Stub: remove a tool's tracked output.
#[tauri::command]
pub async fn games_tools_remove() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_tools_remove"))
}

/// Stub: diff a tool's output against the files it replaced.
#[tauri::command]
pub async fn games_tools_diff() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_tools_diff"))
}

/// Stub: what a tool's in-place swap would cost in disk space.
#[tauri::command]
pub async fn games_tools_swap_costs() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_tools_swap_costs"))
}

/// Stub: whether a tool's swap is in place, and whether it is stale.
#[tauri::command]
pub async fn games_tools_swap_status() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_tools_swap_status"))
}

/// Stub: put back the files a tool's swap replaced.
#[tauri::command]
pub async fn games_tools_swap_restore() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_tools_swap_restore"))
}
