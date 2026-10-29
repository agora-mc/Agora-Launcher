//! The curated catalog for games other than Minecraft (`agora games catalog`, MASTER_SPEC §26.8).
//! Owned by the `catalog` slice. A thin adapter over agora-core's registry and `catalog_install`,
//! the same services the CLI calls, so both show the same hash outcome and the same refusals.

use agora_core::artifact_hash::HashOrigin;
use agora_core::catalog_install::{
    self, CatalogPlan, FrameworkCheck, HashReport, HttpTransport, InstallReport, InstallRequest,
    InstallState, PlannedSource,
};
use agora_core::game_registry::GameRegistry;
use agora_core::github_release::GitHubAuth;
use agora_core::registry::{GameCatalogItem, GameCompatibility, RegistryService};
use serde::{Deserialize, Serialize};

use super::error::{GamesError, CORE_FAILED, TASK_FAILED};

/// The game id Minecraft's catalog is browsed under, which is not this catalog.
const MINECRAFT: &str = "minecraft";
/// The request named a game this catalog does not serve.
const BAD_GAME: &str = "bad_game";
/// No catalog entry has the id the request named.
const NO_ENTRY: &str = "no_entry";

// ---------------------------------------------------------------------------
// Listing
// ---------------------------------------------------------------------------

/// The curated entries for one game, and the instances an entry can be installed into.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogListing {
    pub entries: Vec<CatalogEntryDto>,
    pub instances: Vec<InstanceChoiceDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntryDto {
    pub id: String,
    pub name: String,
    pub author: Option<String>,
    pub description: Option<String>,
    /// `github_release` or `direct_hash`: where the file comes from.
    pub download_strategy: String,
    pub status: String,
    pub page_url: Option<String>,
    pub compatibility: Vec<CompatibilityDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompatibilityDto {
    pub stores: Vec<String>,
    pub game_versions: Vec<String>,
    pub requires: Vec<RequirementDto>,
    /// The release asset this entry picks, for a GitHub release.
    pub asset: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementDto {
    pub framework: String,
    pub min_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceChoiceDto {
    pub instance_id: String,
    pub name: String,
}

impl From<&GameCatalogItem> for CatalogEntryDto {
    fn from(item: &GameCatalogItem) -> Self {
        Self {
            id: item.id.clone(),
            name: item.name.clone(),
            author: item.author.clone(),
            description: item.description.clone(),
            download_strategy: item.download_strategy.clone(),
            status: item.status.clone(),
            page_url: item.page_url.clone(),
            compatibility: item
                .game_compatibility
                .iter()
                .map(CompatibilityDto::from)
                .collect(),
        }
    }
}

impl From<&GameCompatibility> for CompatibilityDto {
    fn from(compat: &GameCompatibility) -> Self {
        Self {
            stores: compat.stores.clone(),
            game_versions: compat.game_versions.clone(),
            requires: compat
                .requires
                .iter()
                .map(|r| RequirementDto {
                    framework: r.framework.clone(),
                    min_version: r.min_version.clone(),
                })
                .collect(),
            asset: compat.asset.clone(),
        }
    }
}

/// The curated entries for one game, with the game's instances to install them into. Errors
/// (a bad game, an unreadable registry) are errors, never an empty list.
#[tauri::command]
pub async fn games_catalog_list(
    app: tauri::AppHandle,
    game_id: String,
) -> Result<CatalogListing, GamesError> {
    tokio::task::spawn_blocking(move || -> Result<CatalogListing, GamesError> {
        let ctx = crate::core_context(&app)?;
        let game = known_game(&ctx.games, &game_id)?;
        let svc = RegistryService::new(ctx.clone());
        let items = svc.list_game_items(&game)?;
        let instances = agora_core::game_instance::list(&ctx)
            .map_err(core_failed)?
            .into_iter()
            .filter(|record| record.game.as_str() == game)
            .map(|record| InstanceChoiceDto {
                instance_id: record.instance_id,
                name: record.name,
            })
            .collect();
        Ok(CatalogListing {
            entries: items.iter().map(CatalogEntryDto::from).collect(),
            instances,
        })
    })
    .await
    .map_err(|_| GamesError::new(TASK_FAILED, "Reading the catalog did not finish."))?
}

// ---------------------------------------------------------------------------
// Installing
// ---------------------------------------------------------------------------

/// One install of a catalog entry into an instance.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogInstallRequest {
    pub game_id: String,
    pub instance_id: String,
    pub entry_id: String,
    /// Accept a file that differs from a curator pin or an earlier install. Never accepts a
    /// published hash that differs, and never overrides a framework refusal.
    pub install_anyway: bool,
    /// Plan only: nothing is downloaded or stored.
    pub dry_run: bool,
}

/// What the install did, as a tagged reply the screen can read without parsing words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CatalogInstallReply {
    DryRun(PlanDto),
    AlreadyInstalled(AlreadyInstalledDto),
    Placed(PlacedDto),
    NeedsInstaller(NeedsInstallerDto),
}

/// The plan: the file that would be fetched, and the checks made before it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanDto {
    pub item_id: String,
    pub item_name: String,
    pub instance_id: String,
    /// Where the file comes from, as a person reads it.
    pub source: String,
    pub file_name: String,
    /// The release tag, for a GitHub file.
    pub release: Option<String>,
    pub size: Option<u64>,
    pub frameworks: Vec<FrameworkDto>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameworkDto {
    pub framework: String,
    pub name: String,
    pub min_version: Option<String>,
    /// `None` when the framework is present but its version cannot be read.
    pub found_version: Option<String>,
}

/// The hash outcome in words the screen can show: whether a source or a curator checked the file,
/// how, and whether a differing expectation was accepted on purpose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HashDto {
    pub sha256: String,
    pub verified: bool,
    pub basis: String,
    /// Set when a differing expectation was accepted with `install_anyway`.
    pub confirmed_past: Option<HashOrigin>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlreadyInstalledDto {
    #[serde(flatten)]
    pub plan: PlanDto,
    pub installed_release: Option<String>,
    /// Whether the release the instance has is the one this entry would install.
    pub same_release: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlacedDto {
    #[serde(flatten)]
    pub plan: PlanDto,
    pub content_item_id: String,
    pub mount_path: String,
    /// The stored archive's size. Named apart from the plan's download `size`: both are flattened
    /// into one JSON object, and a repeated key would silently replace the plan's value.
    pub stored_size: u64,
    pub hash: HashDto,
}

/// The archive has an installer that asks questions. It is in the content store, not yet in the
/// instance: the screen opens the FOMOD wizard for `content_item_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NeedsInstallerDto {
    #[serde(flatten)]
    pub plan: PlanDto,
    pub content_item_id: String,
    /// The stored archive's size (see `PlacedDto::stored_size`).
    pub stored_size: u64,
    pub hash: HashDto,
    pub reason: String,
    pub top_level: Vec<String>,
}

/// Plan, and unless it is a dry run or already installed, fetch, check, store and place the entry.
/// The download is the slow part; the screen shows a working state until it returns.
#[tauri::command]
pub async fn games_catalog_install(
    app: tauri::AppHandle,
    request: CatalogInstallRequest,
) -> Result<CatalogInstallReply, GamesError> {
    tokio::task::spawn_blocking(move || -> Result<CatalogInstallReply, GamesError> {
        let ctx = crate::core_context(&app)?;
        let game = known_game(&ctx.games, &request.game_id)?;
        let svc = RegistryService::new(ctx.clone());
        let item = svc.get_game_item(&request.entry_id)?.ok_or_else(|| {
            GamesError::new(
                NO_ENTRY,
                format!(
                    "There is no catalog entry '{}'. Reload the Library to see the entries for this game.",
                    request.entry_id
                ),
            )
        })?;
        if item.game != game {
            return Err(GamesError::new(
                BAD_GAME,
                format!("'{}' is an entry for {}, not {game}.", item.id, item.game),
            ));
        }
        let transport = HttpTransport {
            clients: &ctx.http_clients,
            auth: GitHubAuth::default(),
        };
        let core_request = InstallRequest {
            instance_id: &request.instance_id,
            item: &item,
            install_anyway: request.install_anyway,
            dry_run: request.dry_run,
        };
        // The install is async (it may download). It runs to completion on this blocking thread
        // through the runtime's handle: its future borrows a non-`Send` callback, which a command
        // future may not hold across an await.
        let report = tokio::runtime::Handle::current()
            .block_on(catalog_install::install(
                &ctx,
                &transport,
                core_request,
                &mut |_: &CatalogPlan| {},
            ))
            .map_err(GamesError::from)?;
        Ok(CatalogInstallReply::from(&report))
    })
    .await
    .map_err(|_| GamesError::new(TASK_FAILED, "Installing the catalog entry did not finish."))?
}

impl From<&InstallReport> for CatalogInstallReply {
    fn from(report: &InstallReport) -> Self {
        let plan = PlanDto::from(&report.plan);
        match &report.state {
            InstallState::DryRun => CatalogInstallReply::DryRun(plan),
            InstallState::AlreadyInstalled {
                installed_release,
                same_release,
            } => CatalogInstallReply::AlreadyInstalled(AlreadyInstalledDto {
                plan,
                installed_release: installed_release.clone(),
                same_release: *same_release,
            }),
            InstallState::Placed {
                content_item_id,
                mount_path,
                size,
                hash,
                ..
            } => CatalogInstallReply::Placed(PlacedDto {
                plan,
                content_item_id: content_item_id.clone(),
                mount_path: mount_path.clone(),
                stored_size: *size,
                hash: HashDto::from(hash),
            }),
            InstallState::NeedsInstaller {
                content_item_id,
                size,
                hash,
                reason,
                top_level,
                ..
            } => CatalogInstallReply::NeedsInstaller(NeedsInstallerDto {
                plan,
                content_item_id: content_item_id.clone(),
                stored_size: *size,
                hash: HashDto::from(hash),
                reason: reason.clone(),
                top_level: top_level.clone(),
            }),
        }
    }
}

impl From<&CatalogPlan> for PlanDto {
    fn from(plan: &CatalogPlan) -> Self {
        Self {
            item_id: plan.item_id.clone(),
            item_name: plan.item_name.clone(),
            instance_id: plan.instance_id.clone(),
            source: plan.source.describe(),
            file_name: plan.source.file_name().to_string(),
            release: plan.source.release().map(str::to_string),
            size: source_size(&plan.source),
            frameworks: plan.frameworks.iter().map(FrameworkDto::from).collect(),
            warnings: plan.warnings.clone(),
        }
    }
}

impl From<&FrameworkCheck> for FrameworkDto {
    fn from(check: &FrameworkCheck) -> Self {
        Self {
            framework: check.framework.clone(),
            name: check.name.clone(),
            min_version: check.min_version.clone(),
            found_version: check.found_version.clone(),
        }
    }
}

impl From<&HashReport> for HashDto {
    fn from(hash: &HashReport) -> Self {
        Self {
            sha256: hash.sha256.clone(),
            verified: hash.verified,
            basis: hash.basis.clone(),
            confirmed_past: hash.confirmed_past,
        }
    }
}

fn source_size(source: &PlannedSource) -> Option<u64> {
    match source {
        PlannedSource::GithubRelease { size, .. } | PlannedSource::DirectHash { size, .. } => *size,
    }
}

// ---------------------------------------------------------------------------
// Shared checks
// ---------------------------------------------------------------------------

/// The id of a game this catalog serves. Minecraft and unknown games are refused by name, so a
/// wrong id is never answered with an empty catalog.
fn known_game(games: &GameRegistry, game_id: &str) -> Result<String, GamesError> {
    let trimmed = game_id.trim();
    if trimmed == MINECRAFT {
        return Err(GamesError::new(
            BAD_GAME,
            "Minecraft's catalog is browsed in Browse, not in a game's Library.",
        ));
    }
    if !games.games().any(|game| game.id.as_str() == trimmed) {
        return Err(GamesError::new(
            BAD_GAME,
            format!("Agora does not support a game called '{trimmed}'."),
        ));
    }
    Ok(trimmed.to_string())
}

fn core_failed(error: impl std::fmt::Display) -> GamesError {
    GamesError::new(CORE_FAILED, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use agora_core::game_registry::PackageSource;

    fn registry() -> GameRegistry {
        let mut builder = GameRegistry::builder();
        builder
            .add(
                PackageSource::Compiled {
                    crate_name: "agora-game-creation".to_string(),
                },
                agora_game_creation::game_package(),
            )
            .expect("the Creation Engine package registers");
        builder.build()
    }

    #[test]
    fn a_supported_game_is_accepted_with_surrounding_space_trimmed() {
        let games = registry();
        assert_eq!(known_game(&games, "skyrim-se").unwrap(), "skyrim-se");
        assert_eq!(known_game(&games, "  skyrim-se \n").unwrap(), "skyrim-se");
    }

    #[test]
    fn minecraft_empty_unknown_and_wrong_case_games_are_refused_by_name() {
        let games = registry();
        // Minecraft gets its own sentence, pointing at Browse; the others name the game.
        let minecraft = known_game(&games, "minecraft").expect_err("minecraft");
        assert_eq!(minecraft.code, BAD_GAME);
        assert!(
            minecraft.message.contains("Browse"),
            "{}",
            minecraft.message
        );
        for bad in ["", "   ", "no-such-game", "Skyrim-SE"] {
            let error = known_game(&games, bad).expect_err(bad);
            assert_eq!(error.code, BAD_GAME, "{bad:?}");
            assert!(
                error.message.contains("does not support"),
                "{bad:?}: {}",
                error.message
            );
        }
    }

    #[test]
    fn a_placed_install_names_its_state_and_its_hash_basis() {
        let hash = HashDto {
            sha256: "ab".repeat(32),
            verified: false,
            basis: "no published hash; told and remembered".into(),
            confirmed_past: None,
        };
        let value = serde_json::to_value(CatalogInstallReply::Placed(PlacedDto {
            plan: PlanDto {
                item_id: "skse".into(),
                item_name: "SKSE".into(),
                instance_id: "inst-1".into(),
                source: "direct download https://example.invalid/skse.7z".into(),
                file_name: "skse.7z".into(),
                release: None,
                size: Some(10),
                frameworks: vec![],
                warnings: vec![],
            },
            content_item_id: "c-1".into(),
            mount_path: "Data".into(),
            stored_size: 20,
            hash,
        }))
        .unwrap();
        assert_eq!(value["state"], "placed");
        assert_eq!(value["itemName"], "SKSE");
        // The download size and the stored size are different numbers and both survive.
        assert_eq!(
            value["size"], 10,
            "the plan's download size is not replaced"
        );
        assert_eq!(value["storedSize"], 20);
        assert_eq!(value["hash"]["verified"], false);
        assert_eq!(
            value["hash"]["basis"],
            "no published hash; told and remembered"
        );
    }
}
