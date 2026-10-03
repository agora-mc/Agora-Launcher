//! Game registry and install identification (MASTER_SPEC §26.3, §26.11).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::sync::Arc;

use agora_game_api::{
    FrameworkId, GameDefinition, GameId, GameInstall, GamePackage, InstallId, InstallKind, RelPath,
    RuntimeIdentity, StoreId, ToolId, GAME_API_VERSION,
};
use serde::{Deserialize, Serialize};

use crate::game_discovery::{DiscoveredInstall, DiscoveryReport};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PackageSource {
    Compiled { crate_name: String },
    Plugin { plugin_id: String },
}

#[derive(Debug, thiserror::Error)]
pub enum GameRegistryError {
    #[error("package {package_id} api_range {api_range} does not match GAME_API_VERSION {current_version}")]
    IncompatibleApiRange {
        package_id: String,
        api_range: semver::VersionReq,
        current_version: semver::Version,
    },
    #[error("package {package_id} defines no games")]
    NoGamesDefined { package_id: String },
    #[error("game id {game_id} is already registered")]
    GameAlreadyRegistered { game_id: GameId },
    #[error("store product ({store}, {product}) is already claimed by game {claimed_by}")]
    StoreProductAlreadyClaimed {
        store: StoreId,
        product: String,
        claimed_by: GameId,
    },
    #[error("framework {framework_id} names undefined game {game_id}")]
    UndefinedGameInFramework {
        framework_id: FrameworkId,
        game_id: GameId,
    },
    #[error("tool {tool_id} names undefined game {game_id}")]
    UndefinedGameInTool { tool_id: ToolId, game_id: GameId },
}

pub struct GameRegistry {
    packages: Vec<(PackageSource, Arc<dyn GamePackage>)>,
    games: BTreeMap<GameId, (usize, GameDefinition)>,
    store_products: HashMap<(StoreId, String), GameId>,
}

#[derive(Default)]
pub struct GameRegistryBuilder {
    packages: Vec<(PackageSource, Arc<dyn GamePackage>)>,
    games: BTreeMap<GameId, (usize, GameDefinition)>,
    store_products: HashMap<(StoreId, String), GameId>,
}

impl GameRegistryBuilder {
    pub fn add(
        &mut self,
        source: PackageSource,
        package: Arc<dyn GamePackage>,
    ) -> Result<(), GameRegistryError> {
        let def = package.definition();

        // 1. API range check
        if !def.api_range.matches(&GAME_API_VERSION) {
            return Err(GameRegistryError::IncompatibleApiRange {
                package_id: def.id.clone(),
                api_range: def.api_range.clone(),
                current_version: GAME_API_VERSION,
            });
        }

        // 2. Defines no game
        if def.games.is_empty() {
            return Err(GameRegistryError::NoGamesDefined {
                package_id: def.id.clone(),
            });
        }

        // 3. Game IDs check (against existing and intra-package)
        let mut new_game_ids = BTreeSet::new();
        for game in &def.games {
            if self.games.contains_key(&game.id) || !new_game_ids.insert(game.id.clone()) {
                return Err(GameRegistryError::GameAlreadyRegistered {
                    game_id: game.id.clone(),
                });
            }
        }

        // 4. (store, product) claims check
        let mut new_store_claims: HashMap<(StoreId, String), GameId> = HashMap::new();
        for game in &def.games {
            for store_id in &game.stores {
                let key = (store_id.store.clone(), store_id.product.clone());
                if let Some(claimed_by) = self.store_products.get(&key) {
                    return Err(GameRegistryError::StoreProductAlreadyClaimed {
                        store: store_id.store.clone(),
                        product: store_id.product.clone(),
                        claimed_by: claimed_by.clone(),
                    });
                }
                if let Some(claimed_by) = new_store_claims.get(&key) {
                    return Err(GameRegistryError::StoreProductAlreadyClaimed {
                        store: store_id.store.clone(),
                        product: store_id.product.clone(),
                        claimed_by: claimed_by.clone(),
                    });
                }
                new_store_claims.insert(key, game.id.clone());
            }
        }

        // 5. Frameworks name defined games
        for framework in &def.frameworks {
            if !new_game_ids.contains(&framework.game) {
                return Err(GameRegistryError::UndefinedGameInFramework {
                    framework_id: framework.id.clone(),
                    game_id: framework.game.clone(),
                });
            }
        }

        // 6. Tools name defined games
        for tool in &def.tools {
            if !new_game_ids.contains(&tool.game) {
                return Err(GameRegistryError::UndefinedGameInTool {
                    tool_id: tool.id.clone(),
                    game_id: tool.game.clone(),
                });
            }
        }

        // Commit mutations only after all checks pass
        let pkg_idx = self.packages.len();
        for game in &def.games {
            self.games.insert(game.id.clone(), (pkg_idx, game.clone()));
        }
        for (key, game_id) in new_store_claims {
            self.store_products.insert(key, game_id);
        }
        self.packages.push((source, package));

        Ok(())
    }

    pub fn build(self) -> GameRegistry {
        GameRegistry {
            packages: self.packages,
            games: self.games,
            store_products: self.store_products,
        }
    }
}

impl GameRegistry {
    pub fn builder() -> GameRegistryBuilder {
        GameRegistryBuilder::default()
    }

    pub fn empty() -> Self {
        Self {
            packages: Vec::new(),
            games: BTreeMap::new(),
            store_products: HashMap::new(),
        }
    }

    /// Iterator over registered game definitions, ordered by game id.
    pub fn games(&self) -> impl Iterator<Item = &GameDefinition> {
        self.games.values().map(|(_, def)| def)
    }

    pub fn game(&self, id: &GameId) -> Option<&GameDefinition> {
        self.games.get(id).map(|(_, def)| def)
    }

    pub fn package_for(&self, id: &GameId) -> Option<&Arc<dyn GamePackage>> {
        self.games.get(id).map(|(idx, _)| &self.packages[*idx].1)
    }

    pub fn source_for(&self, id: &GameId) -> Option<&PackageSource> {
        self.games.get(id).map(|(idx, _)| &self.packages[*idx].0)
    }

    pub fn game_for_store_product(&self, store: &StoreId, product: &str) -> Option<&GameId> {
        self.store_products
            .get(&(store.clone(), product.to_string()))
    }
}

// ---------------------------------------------------------------------------
// Runtime matching and install identification
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameInventory {
    pub installs: Vec<IdentifiedInstall>,
    pub unsupported: Vec<DiscoveredInstall>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentifiedInstall {
    pub game: GameId,
    pub install_id: InstallId,
    pub discovered: DiscoveredInstall,
    pub add_ons: Vec<DiscoveredInstall>,
    pub runtime: RuntimeResolution,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RuntimeResolution {
    Identified {
        runtime: RuntimeIdentity,
        source: String, // "executable" | "store_record"
    },
    Unidentified {
        reasons: Vec<String>,
    },
}

impl IdentifiedInstall {
    pub fn game_install(&self) -> Option<GameInstall> {
        match &self.runtime {
            RuntimeResolution::Identified { runtime, .. } => Some(GameInstall {
                id: self.install_id.clone(),
                runtime: runtime.clone(),
                kind: self.discovered.kind.clone(),
                location: self.discovered.location.to_string_lossy().to_string(),
                volume: self.discovered.volume.clone(),
                capabilities: self.discovered.capabilities.clone(),
            }),
            RuntimeResolution::Unidentified { .. } => None,
        }
    }
}

/// `{store}:{product}`, stable across runs, because bases and instances refer
/// to installs by it. A product that is not already a valid id part is
/// lowercased and sanitised, then suffixed with a hash of the original, so two
/// products that sanitise alike (`A.B` and `a-b`) still get different ids.
pub fn make_install_id(store: &StoreId, product: &str) -> InstallId {
    let mut part = String::new();
    for c in product.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-' {
            part.push(c);
        } else if !part.ends_with('-') {
            part.push('-');
        }
    }
    let part = part.trim_matches(|c| c == '-' || c == '_');
    let budget = agora_game_api::MAX_ID_LEN.saturating_sub(store.as_str().len() + 1);
    let id = if !part.is_empty() && part == product && part.len() <= budget {
        part.to_string()
    } else {
        let suffix = format!("{:08x}", fnv1a_32(product));
        let keep = budget.saturating_sub(suffix.len() + 1);
        let prefix = part[..part.len().min(keep)].trim_end_matches(['-', '_']);
        if prefix.is_empty() {
            suffix
        } else {
            format!("{prefix}-{suffix}")
        }
    };
    InstallId::new(format!("{}:{id}", store.as_str()))
        .expect("store ids are short and the product part is sanitised")
}

pub fn fnv1a_32(s: &str) -> u32 {
    let mut hash = 0x811c9dc5u32;
    for byte in s.as_bytes() {
        hash ^= *byte as u32;
        hash = hash.wrapping_mul(0x01000193);
    }
    hash
}

pub fn identify_installs(
    registry: &GameRegistry,
    report: &DiscoveryReport,
    read_version: &dyn Fn(&Path) -> Option<String>,
) -> GameInventory {
    let mut installs = Vec::new();
    let mut unsupported = Vec::new();

    let base_games: Vec<&DiscoveredInstall> = report
        .installs
        .iter()
        .filter(|i| i.kind == InstallKind::BaseGame)
        .collect();

    for bg in base_games {
        if let Some(game_id) = registry.game_for_store_product(&bg.store, &bg.product) {
            let game_def = registry
                .game(game_id)
                .expect("game definition must exist for claimed product");

            let add_ons: Vec<DiscoveredInstall> = report
                .installs
                .iter()
                .filter(|i| {
                    i.kind == InstallKind::AddOn
                        && i.store == bg.store
                        && i.parent_product.as_deref() == Some(&bg.product)
                })
                .cloned()
                .collect();

            let install_id = make_install_id(&bg.store, &bg.product);
            let runtime = resolve_runtime(game_def, bg, read_version);

            installs.push(IdentifiedInstall {
                game: game_id.clone(),
                install_id,
                discovered: bg.clone(),
                add_ons,
                runtime,
            });
        } else {
            unsupported.push(bg.clone());
        }
    }

    GameInventory {
        installs,
        unsupported,
    }
}

fn resolve_runtime(
    game_def: &GameDefinition,
    discovered: &DiscoveredInstall,
    read_version: &dyn Fn(&Path) -> Option<String>,
) -> RuntimeResolution {
    let mut reasons = Vec::new();

    for source in &game_def.version_sources {
        match source {
            agora_game_api::VersionSource::Executable { path } => {
                if !discovered.capabilities.executables_readable {
                    reasons.push("executables not readable".to_string());
                    continue;
                }
                let path_str = path.as_str();
                if path_str.is_empty()
                    || RelPath::new(path_str).is_err()
                    || path_str.split('/').any(|p| p == "..")
                    || path_str.split('\\').any(|p| p == "..")
                {
                    reasons.push(format!("executable path {path_str} escapes install"));
                    continue;
                }
                let relative = std::path::Path::new(path_str);
                if relative.is_absolute() {
                    reasons.push(format!("executable path {path_str} is absolute"));
                    continue;
                }
                let exe_path = discovered.location.join(relative);
                match read_version(&exe_path) {
                    Some(version) => {
                        return RuntimeResolution::Identified {
                            runtime: RuntimeIdentity {
                                game: game_def.id.clone(),
                                store: discovered.store.clone(),
                                version,
                                build: discovered.store_build.clone(),
                            },
                            source: "executable".to_string(),
                        };
                    }
                    None => {
                        reasons.push(format!("could not read executable version from {path_str}"));
                    }
                }
            }
            agora_game_api::VersionSource::StoreRecord => {
                let version = discovered
                    .store_version
                    .clone()
                    .or_else(|| discovered.store_build.clone());
                match version {
                    Some(version) => {
                        return RuntimeResolution::Identified {
                            runtime: RuntimeIdentity {
                                game: game_def.id.clone(),
                                store: discovered.store.clone(),
                                version,
                                build: discovered.store_build.clone(),
                            },
                            source: "store_record".to_string(),
                        };
                    }
                    None => {
                        reasons.push("store record missing version and build".to_string());
                    }
                }
            }
            agora_game_api::VersionSource::PackageBehaviour => {
                reasons.push("needs the game host, which arrives later".to_string());
            }
        }
    }

    RuntimeResolution::Unidentified { reasons }
}
