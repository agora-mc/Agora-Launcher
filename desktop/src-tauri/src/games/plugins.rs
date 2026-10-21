//! Plugins: an instance's plugin list, its load order, locks and checks.
//! Owned by the `plugins` slice. Every command here is a stub until that slice lands.

use super::error::GamesError;

/// Stub: an instance's plugins in load order.
#[tauri::command]
pub async fn games_plugins_list() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_plugins_list"))
}

/// Stub: enable or disable a plugin.
#[tauri::command]
pub async fn games_plugins_set_active() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_plugins_set_active"))
}

/// Stub: lock or unlock a plugin's position.
#[tauri::command]
pub async fn games_plugins_set_locked() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_plugins_set_locked"))
}

/// Stub: move a plugin within the load order.
#[tauri::command]
pub async fn games_plugins_move() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_plugins_move"))
}

/// Stub: sort the load order from the game's rules.
#[tauri::command]
pub async fn games_plugins_sort() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_plugins_sort"))
}

/// Stub: check the load order for problems.
#[tauri::command]
pub async fn games_plugins_check() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_plugins_check"))
}
