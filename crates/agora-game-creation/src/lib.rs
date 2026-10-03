//! Creation Engine (Skyrim SE) support for Agora (MASTER_SPEC §26.11-§26.12).

use agora_game_api::{GamePackage, PackageDefinition};
use std::sync::{Arc, OnceLock};

struct CreationEnginePackage(PackageDefinition);

impl GamePackage for CreationEnginePackage {
    fn definition(&self) -> &PackageDefinition {
        &self.0
    }
}

/// The compiled game package for Creation Engine games (Skyrim Special Edition).
pub fn game_package() -> Arc<dyn GamePackage> {
    static PACKAGE: OnceLock<Arc<dyn GamePackage>> = OnceLock::new();
    PACKAGE
        .get_or_init(|| {
            let json_str = include_str!("../data/package.json");
            let def: PackageDefinition = serde_json::from_str(json_str)
                .expect("failed to deserialize agora-game-creation package.json");
            Arc::new(CreationEnginePackage(def))
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use agora_game_api::{DeploymentStrategy, GameId, GamePath, RelPath, StoreId, VersionSource};

    #[test]
    fn parses_embedded_package_json() {
        let pkg = game_package();
        let def = pkg.definition();

        assert_eq!(def.id, "agora.creation-engine");
        assert_eq!(def.version.major, 0);
        assert_eq!(def.version.minor, 1);
        assert_eq!(def.version.patch, 0);
        assert!(def.api_range.matches(&agora_game_api::GAME_API_VERSION));
        assert_eq!(def.games.len(), 1);

        let game = &def.games[0];
        assert_eq!(game.id, GameId::new("skyrim-se").unwrap());
        assert_eq!(game.name, "The Elder Scrolls V: Skyrim Special Edition");
        assert_eq!(game.deployment, DeploymentStrategy::VirtualFileSystem);

        // Stores
        assert_eq!(game.stores.len(), 2);
        assert_eq!(game.stores[0].store, StoreId::steam());
        assert_eq!(game.stores[0].product, "489830");
        assert_eq!(game.stores[1].store, StoreId::gog());
        assert_eq!(game.stores[1].product, "1711230643");

        // Version sources
        assert_eq!(
            game.version_sources,
            vec![
                VersionSource::Executable {
                    path: RelPath::new("SkyrimSE.exe").unwrap()
                },
                VersionSource::StoreRecord,
            ]
        );

        // Native code & archives
        assert_eq!(
            game.native_code_patterns,
            vec![RelPath::new("Data/SKSE/Plugins/*.dll").unwrap()]
        );
        assert_eq!(
            game.linked_archive_patterns,
            vec!["Data/*.bsa", "Data/*.esm", "Data/*.esl", "Data/*.bik",]
        );
        assert_eq!(game.declared_writes, vec!["d3dx9_42.log"]);
        assert!(game.is_declared_write("d3dx9_42.log"));
        assert!(game.is_declared_write("D3DX9_42.LOG"));

        // Launch recipe
        let launch = game.launch.as_ref().expect("launch recipe should be Some");
        assert_eq!(
            launch.executable,
            GamePath::Runtime {
                path: RelPath::new("SkyrimSE.exe").unwrap()
            }
        );
        assert_eq!(
            launch.working_directory,
            GamePath::Runtime {
                path: RelPath::default()
            }
        );
        assert!(launch.arguments.is_empty());
        assert!(launch.environment.is_empty());

        // Empty lists
        assert!(game.content_rules.is_empty());
        assert!(game.framework_ids.is_empty());
        assert!(game.tool_ids.is_empty());
        assert!(game.log_paths.is_empty());
        assert!(game.crash_paths.is_empty());
        assert!(game.user_files.is_empty());
        assert!(game.save_paths.is_empty());
    }
}
