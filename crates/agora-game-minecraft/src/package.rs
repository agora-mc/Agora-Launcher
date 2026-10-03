use agora_game_api::{
    DeploymentStrategy, GameDefinition, GameId, GamePackage, PackageDefinition, VersionSource,
};
use std::sync::{Arc, OnceLock};

struct MinecraftPackage(PackageDefinition);

impl GamePackage for MinecraftPackage {
    fn definition(&self) -> &PackageDefinition {
        &self.0
    }
}

/// The compiled game package for Minecraft: Java Edition.
pub fn game_package() -> Arc<dyn GamePackage> {
    static PACKAGE: OnceLock<Arc<dyn GamePackage>> = OnceLock::new();
    PACKAGE
        .get_or_init(|| {
            let def = PackageDefinition {
                id: "agora.minecraft".to_string(),
                version: semver::Version::new(0, 1, 0),
                api_range: semver::VersionReq::parse(">=0.1, <0.2").unwrap(),
                parents: Vec::new(),
                games: vec![GameDefinition {
                    id: GameId::minecraft(),
                    name: "Minecraft: Java Edition".to_string(),
                    stores: Vec::new(),
                    version_sources: vec![VersionSource::PackageBehaviour],
                    deployment: DeploymentStrategy::Redirect,
                    content_rules: Vec::new(),
                    native_code_patterns: Vec::new(),
                    framework_ids: Vec::new(),
                    tool_ids: Vec::new(),
                    launch: None,
                    log_paths: Vec::new(),
                    crash_paths: Vec::new(),
                    user_files: Vec::new(),
                    save_paths: Vec::new(),
                    linked_archive_patterns: Vec::new(),
                }],
                frameworks: Vec::new(),
                tools: Vec::new(),
            };
            Arc::new(MinecraftPackage(def))
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minecraft_package_conforms_to_contract() {
        let pkg = game_package();
        let def = pkg.definition();
        assert_eq!(def.id, "agora.minecraft");
        assert_eq!(def.games.len(), 1);
        let game = &def.games[0];
        assert_eq!(game.id, GameId::minecraft());
        assert_eq!(game.name, "Minecraft: Java Edition");
        assert!(game.stores.is_empty());
        assert_eq!(game.version_sources, vec![VersionSource::PackageBehaviour]);
        assert_eq!(game.deployment, DeploymentStrategy::Redirect);
        assert!(game.launch.is_none());
    }
}
