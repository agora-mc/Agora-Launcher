//! The curated catalog for games other than Minecraft (`agora games catalog`).
//! Owned by the `catalog` slice. Every command here is a stub until that slice lands.

use super::error::GamesError;

/// Stub: the catalog entries for one game.
#[tauri::command]
pub async fn games_catalog_list() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_catalog_list"))
}

/// Stub: install a catalog entry.
#[tauri::command]
pub async fn games_catalog_install() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_catalog_install"))
}
