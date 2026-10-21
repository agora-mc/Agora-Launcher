//! Import: bringing in a Mod Organizer 2 setup (MASTER_SPEC §26.10).
//! Owned by the `import` slice. Every command here is a stub until that slice lands.

use super::error::GamesError;

/// Stub: find the Mod Organizer 2 setups on this machine.
#[tauri::command]
pub async fn games_import_mo2_scan() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_import_mo2_scan"))
}

/// Stub: import one Mod Organizer 2 profile.
#[tauri::command]
pub async fn games_import_mo2() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_import_mo2"))
}
