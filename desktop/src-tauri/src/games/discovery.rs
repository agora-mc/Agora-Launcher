//! Finding installs: what is on this machine, grouped by game, and the game folders the user
//! added by hand (MASTER_SPEC §26.3).
//!
//! Every command here is a thin adapter over `agora_core::game_discovery`, the same services
//! `agora games discover`, `agora games list` and `agora games folder` call, so the desktop and the
//! CLI cannot disagree about what is installed or which folders are searched.

use agora_core::game_discovery::custom_folders::{self, AddedFolder, ListedFolder};
use agora_core::game_discovery::DiscoveryWarning;
use agora_core::game_registry::{GameInventory, GameRegistry};
use serde::Serialize;

use super::dto::{GameListing, GamesList, InstallSummary};
use super::error::{GamesError, TASK_FAILED};

/// Installs found on this machine, grouped by game. Blocking filesystem and registry
/// reads, so the work runs on a blocking thread.
#[tauri::command]
pub async fn games_list(app: tauri::AppHandle) -> Result<GamesList, GamesError> {
    tokio::task::spawn_blocking(move || -> Result<GamesList, GamesError> {
        let ctx = crate::core_context(&app)?;
        let inventory = agora_core::game_registry::installed_games(&ctx);
        Ok(group_by_game(&ctx.games, &inventory))
    })
    .await
    .map_err(|_| GamesError::new(TASK_FAILED, "Looking for installed games did not finish."))?
}

/// What a fresh scan found besides the installs: the game folders the user added, and the
/// warnings discovery raised (a broken store file, a folder that is gone).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryScan {
    pub folders: Vec<ListedFolder>,
    pub warnings: Vec<ScanWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanWarning {
    pub store: String,
    pub message: String,
}

impl From<&DiscoveryWarning> for ScanWarning {
    fn from(warning: &DiscoveryWarning) -> Self {
        Self {
            store: warning.store.to_string(),
            message: warning.message.clone(),
        }
    }
}

/// Scan again and report the folders and warnings. The installs come from `games_list`, so the
/// Rescan button calls both.
#[tauri::command]
pub async fn games_discover(app: tauri::AppHandle) -> Result<DiscoveryScan, GamesError> {
    tokio::task::spawn_blocking(move || -> Result<DiscoveryScan, GamesError> {
        let ctx = crate::core_context(&app)?;
        let report = agora_core::game_discovery::discover_with_folders(&ctx.paths);
        let folders = custom_folders::list_folder_status(&ctx.paths)?;
        Ok(DiscoveryScan {
            folders,
            warnings: report.warnings.iter().map(ScanWarning::from).collect(),
        })
    })
    .await
    .map_err(|_| GamesError::new(TASK_FAILED, "Looking for installed games did not finish."))?
}

/// Add a folder the user picked. Core refuses a folder that is not a game Agora knows, is
/// already listed, or is a store install; the refusal's message says which.
#[tauri::command]
pub async fn games_add_folder(
    app: tauri::AppHandle,
    path: String,
) -> Result<AddedFolder, GamesError> {
    tokio::task::spawn_blocking(move || -> Result<AddedFolder, GamesError> {
        let ctx = crate::core_context(&app)?;
        Ok(custom_folders::add_folder(&ctx.paths, &ctx.games, &path)?)
    })
    .await
    .map_err(|_| GamesError::new(TASK_FAILED, "Adding the game folder did not finish."))?
}

/// Stop searching a folder the user added. Its files are not touched. Returns the path that was
/// removed, as it is stored.
#[tauri::command]
pub async fn games_remove_folder(
    app: tauri::AppHandle,
    path: String,
) -> Result<String, GamesError> {
    tokio::task::spawn_blocking(move || -> Result<String, GamesError> {
        let ctx = crate::core_context(&app)?;
        Ok(custom_folders::remove_folder(&ctx.paths, &path)?)
    })
    .await
    .map_err(|_| GamesError::new(TASK_FAILED, "Removing the game folder did not finish."))?
}

/// What a base built from this install would hold. `None` when the game is unknown or the folder
/// cannot be walked (a missing folder, a denied one): the screen then shows no size rather than 0.
fn install_bytes(
    registry: &GameRegistry,
    install: &agora_core::game_registry::IdentifiedInstall,
) -> Option<u64> {
    let definition = registry.game(&install.game)?;
    agora_core::game_base::install_size(
        &install.discovered.location,
        definition,
        agora_core::game_base::BuildOptions::default(),
    )
    .ok()
}

/// Group identified installs by game. Only games with an install appear, and an
/// unsupported install (a game Agora cannot manage yet) is never listed as one.
pub fn group_by_game(registry: &GameRegistry, inventory: &GameInventory) -> GamesList {
    let mut games: Vec<GameListing> = Vec::new();
    for install in &inventory.installs {
        let mut summary = InstallSummary::from(install);
        summary.install_bytes = install_bytes(registry, install);
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

    #[test]
    fn a_warning_keeps_its_store_and_message() {
        let warning: DiscoveryWarning = serde_json::from_value(serde_json::json!({
            "store": "steam",
            "message": "appmanifest_489830.acf could not be read",
        }))
        .expect("a warning in the core's shape");
        let scan = ScanWarning::from(&warning);
        assert_eq!(scan.store, "steam");
        assert_eq!(scan.message, "appmanifest_489830.acf could not be read");
    }

    #[test]
    fn a_refused_folder_keeps_its_core_code_for_the_screen() {
        // An empty path is refused before any folder is touched, so the root never has to exist.
        let paths = agora_core::app_paths::AppPaths::from_root(std::path::PathBuf::from("unused"));
        let error = custom_folders::add_folder(&paths, &registry(), "   ")
            .expect_err("an empty path is refused");
        let mapped = GamesError::from(error);
        assert_eq!(mapped.code, super::super::error::CORE_FAILED);
        assert_eq!(
            mapped.core_code.as_deref(),
            Some(custom_folders::ERR_FOLDER_INVALID)
        );
        assert!(
            mapped.message.contains("No folder was given"),
            "{}",
            mapped.message
        );
    }

    #[test]
    fn a_scan_serialises_in_the_shape_the_screen_reads() {
        let scan = DiscoveryScan {
            folders: vec![ListedFolder {
                path: "D:\\Games\\Skyrim".to_string(),
                present: false,
            }],
            warnings: Vec::new(),
        };
        let json = serde_json::to_value(&scan).expect("a scan serialises");
        assert_eq!(json["folders"][0]["path"], "D:\\Games\\Skyrim");
        assert_eq!(json["folders"][0]["present"], false);
        assert!(json["warnings"].as_array().expect("a list").is_empty());
    }
}
