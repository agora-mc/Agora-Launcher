//! Finding installs: what is on this machine, grouped by game.
//!
//! `games_list` is the one command here that is real. It runs the same discovery and
//! identification as `agora games list`, so the desktop and the CLI cannot disagree about
//! what is installed. The rest are stubs for the discovery slice.

use agora_core::game_registry::{GameInventory, GameRegistry};

use super::dto::{GameListing, GamesList, InstallSummary};
use super::error::{GamesError, TASK_FAILED};

/// Installs found on this machine, grouped by game. Blocking filesystem and registry
/// reads, so the work runs on a blocking thread.
#[tauri::command]
pub async fn games_list(app: tauri::AppHandle) -> Result<GamesList, GamesError> {
    tokio::task::spawn_blocking(move || -> Result<GamesList, GamesError> {
        let ctx = crate::core_context(&app)?;
        let report = agora_core::game_discovery::discover_all();
        let inventory = agora_core::game_registry::identify_installs(
            &ctx.games,
            &report,
            &agora_core::game_discovery::file_version::read_file_version,
        );
        Ok(group_by_game(&ctx.games, &inventory))
    })
    .await
    .map_err(|_| GamesError::new(TASK_FAILED, "Looking for installed games did not finish."))?
}

/// Group identified installs by game. Only games with an install appear, and an
/// unsupported install (a game Agora cannot manage yet) is never listed as one.
pub fn group_by_game(registry: &GameRegistry, inventory: &GameInventory) -> GamesList {
    let mut games: Vec<GameListing> = Vec::new();
    for install in &inventory.installs {
        let summary = InstallSummary::from(install);
        let game_id = install.game.to_string();
        match games.iter_mut().find(|listing| listing.game_id == game_id) {
            Some(listing) => listing.installs.push(summary),
            None => {
                let name = registry
                    .game(&install.game)
                    .map(|definition| definition.name.clone())
                    .unwrap_or_else(|| game_id.clone());
                games.push(GameListing {
                    game_id,
                    name,
                    installs: vec![summary],
                });
            }
        }
    }
    GamesList { games }
}

/// Stub: scan a fresh discovery and report the warnings it found.
#[tauri::command]
pub async fn games_discover() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_discover"))
}

/// Stub: add a folder to search for installs that stores do not know about.
#[tauri::command]
pub async fn games_add_folder() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_add_folder"))
}

/// Stub: stop searching a folder added with `games_add_folder`.
#[tauri::command]
pub async fn games_remove_folder() -> Result<serde_json::Value, GamesError> {
    Err(GamesError::not_implemented("games_remove_folder"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agora_core::game_discovery::DiscoveryReport;

    fn registry() -> GameRegistry {
        let mut builder = GameRegistry::builder();
        builder
            .add(
                agora_core::game_registry::PackageSource::Compiled {
                    crate_name: "agora-game-creation".to_string(),
                },
                agora_game_creation::game_package(),
            )
            .expect("the Creation Engine package registers");
        builder.build()
    }

    #[test]
    fn no_installs_means_no_games() {
        let registry = registry();
        let inventory = agora_core::game_registry::identify_installs(
            &registry,
            &DiscoveryReport::new(),
            &|_: &std::path::Path| None::<String>,
        );
        assert!(group_by_game(&registry, &inventory).games.is_empty());
    }
}
