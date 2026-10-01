//! Curated pack install planning (locked releases and the flexible recipe).
//!
//! A curated pack in the registry has two shapes:
//!
//! - **Locked releases** (`pack_versions`): an exact Minecraft version, loader,
//!   loader version and a pinned version of every mod — the build the curator
//!   tested. This is the default whenever a pack has any.
//! - **The flexible recipe** (`pack_mods`): mod names with optional pins, which
//!   can be aimed at any Minecraft version and loader. Each mod takes its pin
//!   when that build fits, otherwise the newest compatible build.
//!
//! In both modes a mod marked `required` that cannot be resolved blocks the
//! whole install — a Create pack without Create is not a pack — while
//! `recommended` and `optional` mods that cannot be resolved are dropped and
//! reported. Planning writes nothing: the result is a list of batch-install
//! items for the normal install pipeline, which still reviews, snapshots and
//! hash-verifies every file.

use crate::ctx::Ctx;
use crate::error::{LauncherError, LauncherResult};
use crate::install_pipeline::{BatchInstallItem, SourceType};
use crate::registry::{PackModRow, PackVersionRow, RegistryService};
use crate::resolver::Resolver;
use serde::{Deserialize, Serialize};

/// Which shape of the pack to install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum CuratedPackSelection {
    /// One locked release, by its version string.
    #[serde(rename_all = "camelCase")]
    Locked { pack_version: String },
    /// The flexible recipe, aimed at any target.
    #[serde(rename_all = "camelCase")]
    Flexible {
        minecraft_version: String,
        loader: String,
    },
}

/// The Minecraft version and loader the plan was resolved against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CuratedPackTarget {
    pub minecraft_version: String,
    pub loader: String,
    /// Set for a locked release; a flexible install picks its own.
    pub loader_version: Option<String>,
}

/// A pack mod that resolved to a concrete build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedPackMod {
    pub mod_id: String,
    pub status: String,
    pub source_type: SourceType,
    /// Registry id for curated entries, Modrinth project id otherwise.
    pub item_id: String,
    /// The version handed to the install pipeline (a Modrinth version id for
    /// raw Modrinth entries, so the choice is unambiguous).
    pub version: String,
    /// Human-readable version for the review.
    pub display_version: String,
    /// True when this is the curator's pinned build rather than a fallback.
    pub pinned: bool,
}

/// A pack mod that could not be resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnresolvedPackMod {
    pub mod_id: String,
    pub status: String,
    pub reason: String,
}

/// Everything needed to review and then install a curated pack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CuratedPackPlan {
    pub pack_id: String,
    /// The locked release, or `None` for a flexible install.
    pub pack_version: Option<String>,
    pub target: CuratedPackTarget,
    pub mods: Vec<PlannedPackMod>,
    /// Recommended or optional mods left out because nothing fits.
    pub dropped: Vec<UnresolvedPackMod>,
    /// Required mods that could not be resolved. Any entry here means the pack
    /// must not be installed.
    pub blocking: Vec<UnresolvedPackMod>,
}

impl CuratedPackPlan {
    /// Whether this plan may go on to the install pipeline.
    pub fn can_install(&self) -> bool {
        self.blocking.is_empty() && !self.mods.is_empty()
    }

    /// The items to hand to a `batch-install` intent.
    pub fn batch_items(&self) -> Vec<BatchInstallItem> {
        self.mods
            .iter()
            .map(|planned| BatchInstallItem {
                source_type: planned.source_type.clone(),
                item_id: planned.item_id.clone(),
                candidate_version: Some(planned.version.clone()),
                content_type: None,
            })
            .collect()
    }
}

/// Outcome of looking one pack mod up against the target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackModLookup {
    Found {
        source_type: SourceType,
        item_id: String,
        version: String,
        display_version: String,
        pinned: bool,
    },
    /// Checked, and no build fits.
    NoBuild(String),
    /// Could not be checked (network, missing catalog entry, bad manifest).
    Unavailable(String),
}

/// Whether an unresolved mod with this status stops the install.
///
/// Anything other than `recommended` or `optional` counts as required, so an
/// unexpected status fails closed rather than silently dropping a mod.
pub fn blocks_install(status: &str) -> bool {
    !matches!(status, "recommended" | "optional")
}

/// Fold per-mod lookups into a plan. Pure, so the required/dropped rules are
/// testable without a network.
pub fn assemble_plan(
    pack_id: &str,
    pack_version: Option<String>,
    target: CuratedPackTarget,
    lookups: Vec<(PackModRow, PackModLookup)>,
) -> CuratedPackPlan {
    let mut plan = CuratedPackPlan {
        pack_id: pack_id.to_string(),
        pack_version,
        target,
        mods: Vec::new(),
        dropped: Vec::new(),
        blocking: Vec::new(),
    };
    for (row, lookup) in lookups {
        let reason = match lookup {
            PackModLookup::Found {
                source_type,
                item_id,
                version,
                display_version,
                pinned,
            } => {
                plan.mods.push(PlannedPackMod {
                    mod_id: row.mod_id,
                    status: row.status,
                    source_type,
                    item_id,
                    version,
                    display_version,
                    pinned,
                });
                continue;
            }
            PackModLookup::NoBuild(reason) | PackModLookup::Unavailable(reason) => reason,
        };
        let unresolved = UnresolvedPackMod {
            mod_id: row.mod_id,
            status: row.status,
            reason,
        };
        if blocks_install(&unresolved.status) {
            plan.blocking.push(unresolved);
        } else {
            plan.dropped.push(unresolved);
        }
    }
    plan
}

pub struct CuratedPackService {
    ctx: Ctx,
}

impl CuratedPackService {
    pub fn new(ctx: Ctx) -> Self {
        Self { ctx }
    }

    /// The pack's locked releases, newest first.
    pub fn versions(&self, pack_id: &str) -> LauncherResult<Vec<PackVersionRow>> {
        RegistryService::new(self.ctx.clone()).pack_versions_for_pack(pack_id)
    }

    /// Resolve every mod of the selected pack shape against its target.
    pub async fn plan(
        &self,
        pack_id: &str,
        selection: &CuratedPackSelection,
    ) -> LauncherResult<CuratedPackPlan> {
        let registry = RegistryService::new(self.ctx.clone());
        let (pack_version, target, rows, locked) = match selection {
            CuratedPackSelection::Locked { pack_version } => {
                let release = registry
                    .pack_versions_for_pack(pack_id)?
                    .into_iter()
                    .find(|release| &release.version == pack_version)
                    .ok_or_else(|| LauncherError::Generic {
                        code: "ERR_PACK_VERSION_NOT_FOUND".into(),
                        message: format!("Pack '{pack_id}' has no release '{pack_version}'."),
                    })?;
                let rows = registry.pack_version_mods(pack_id, pack_version)?;
                let target = CuratedPackTarget {
                    minecraft_version: release.minecraft_version,
                    loader: release.loader,
                    loader_version: Some(release.loader_version),
                };
                (Some(release.version), target, rows, true)
            }
            CuratedPackSelection::Flexible {
                minecraft_version,
                loader,
            } => {
                let target = CuratedPackTarget {
                    minecraft_version: minecraft_version.clone(),
                    loader: loader.clone(),
                    loader_version: None,
                };
                (None, target, registry.pack_mods_for_pack(pack_id)?, false)
            }
        };
        if rows.is_empty() {
            return Err(LauncherError::Generic {
                code: "ERR_PACK_EMPTY".into(),
                message: format!("Pack '{pack_id}' lists no mods in the catalog."),
            });
        }

        let resolver = Resolver::new(self.ctx.clone());
        let mut lookups = Vec::with_capacity(rows.len());
        for row in rows {
            let lookup = self
                .lookup(&registry, &resolver, &row, &target, locked)
                .await;
            lookups.push((row, lookup));
        }
        Ok(assemble_plan(pack_id, pack_version, target, lookups))
    }

    async fn lookup(
        &self,
        registry: &RegistryService,
        resolver: &Resolver,
        row: &PackModRow,
        target: &CuratedPackTarget,
        locked: bool,
    ) -> PackModLookup {
        let pin = crate::resolver::normalize_requested_version(row.version.as_deref());
        if row.source == "modrinth_id" {
            let Some(project_id) = row.modrinth_id.as_deref() else {
                return PackModLookup::Unavailable(
                    "The pack lists this as a Modrinth mod without a project id.".into(),
                );
            };
            let candidates = match resolver
                .list_raw_modrinth_versions_for(
                    project_id,
                    &target.minecraft_version,
                    &target.loader,
                    "mod",
                )
                .await
            {
                Ok(candidates) => candidates,
                Err(error) => {
                    return PackModLookup::Unavailable(format!("Could not check Modrinth: {error}"))
                }
            };
            let pinned = pin.and_then(|pin| {
                candidates
                    .iter()
                    .find(|c| c.version_id == pin || c.version == pin || c.filename == pin)
            });
            let chosen = match (pinned, locked) {
                (Some(candidate), _) => Some((candidate, true)),
                (None, true) => None,
                (None, false) => candidates.first().map(|candidate| (candidate, false)),
            };
            return match chosen {
                Some((candidate, pinned)) => PackModLookup::Found {
                    source_type: SourceType::Modrinth,
                    item_id: project_id.to_string(),
                    version: candidate.version_id.clone(),
                    display_version: candidate.version.clone(),
                    pinned,
                },
                None => PackModLookup::NoBuild(no_build_reason(pin, locked, target)),
            };
        }

        let item = match registry.get_item_by_id(&row.mod_id) {
            Ok(Some(item)) => item,
            Ok(None) => {
                return PackModLookup::Unavailable("Not in the Agora catalog.".into());
            }
            Err(error) => return PackModLookup::Unavailable(error.to_string()),
        };
        let candidates = match resolver
            .list_curated_versions(&item, &target.minecraft_version, &target.loader)
            .await
        {
            Ok(candidates) => candidates,
            Err(error) => {
                return PackModLookup::Unavailable(format!("Could not list versions: {error}"))
            }
        };
        // A locked release trusts the curator's pin outright: it is the build
        // they tested on exactly this target. A flexible install only keeps the
        // pin while it still fits the chosen target.
        let pinned = pin.and_then(|pin| {
            candidates
                .iter()
                .find(|c| (c.version == pin || c.filename == pin) && (locked || c.is_compatible))
        });
        let chosen = match (pinned, locked) {
            (Some(candidate), _) => Some((candidate, true)),
            (None, true) => None,
            (None, false) => candidates
                .iter()
                .find(|c| c.is_compatible)
                .map(|candidate| (candidate, false)),
        };
        match chosen {
            Some((candidate, pinned)) => PackModLookup::Found {
                source_type: SourceType::Curated,
                item_id: item.id.clone(),
                version: candidate.version.clone(),
                display_version: candidate.version.clone(),
                pinned,
            },
            None => PackModLookup::NoBuild(no_build_reason(pin, locked, target)),
        }
    }
}

fn no_build_reason(pin: Option<&str>, locked: bool, target: &CuratedPackTarget) -> String {
    match (pin, locked) {
        (Some(pin), true) => format!("The pinned build {pin} is no longer available."),
        _ => format!(
            "No build for Minecraft {} with {}.",
            target.minecraft_version, target.loader
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(mod_id: &str, status: &str) -> PackModRow {
        PackModRow {
            pack_id: "p".into(),
            mod_id: mod_id.into(),
            source: "manifest".into(),
            version: None,
            status: status.into(),
            description: None,
            modrinth_id: None,
        }
    }

    fn found(item_id: &str) -> PackModLookup {
        PackModLookup::Found {
            source_type: SourceType::Curated,
            item_id: item_id.into(),
            version: "1.0.0".into(),
            display_version: "1.0.0".into(),
            pinned: false,
        }
    }

    fn target() -> CuratedPackTarget {
        CuratedPackTarget {
            minecraft_version: "1.21".into(),
            loader: "fabric".into(),
            loader_version: None,
        }
    }

    #[test]
    fn a_missing_required_mod_blocks_the_install() {
        let plan = assemble_plan(
            "p",
            None,
            target(),
            vec![
                (
                    row("create", "required"),
                    PackModLookup::NoBuild("none".into()),
                ),
                (row("sodium", "required"), found("sodium")),
            ],
        );
        assert_eq!(plan.blocking.len(), 1);
        assert_eq!(plan.blocking[0].mod_id, "create");
        assert!(!plan.can_install());
    }

    #[test]
    fn missing_recommended_and_optional_mods_are_dropped_not_blocking() {
        let plan = assemble_plan(
            "p",
            None,
            target(),
            vec![
                (
                    row("iris", "recommended"),
                    PackModLookup::NoBuild("none".into()),
                ),
                (
                    row("minimap", "optional"),
                    PackModLookup::Unavailable("offline".into()),
                ),
                (row("sodium", "required"), found("sodium")),
            ],
        );
        assert!(plan.blocking.is_empty());
        assert_eq!(plan.dropped.len(), 2);
        assert!(plan.can_install());
        assert_eq!(plan.batch_items().len(), 1);
    }

    #[test]
    fn a_check_that_could_not_run_still_blocks_a_required_mod() {
        let plan = assemble_plan(
            "p",
            None,
            target(),
            vec![(
                row("create", "required"),
                PackModLookup::Unavailable("offline".into()),
            )],
        );
        assert!(!plan.can_install());
    }

    #[test]
    fn an_unknown_status_fails_closed() {
        assert!(blocks_install("required"));
        assert!(blocks_install("must-have"));
        assert!(!blocks_install("recommended"));
        assert!(!blocks_install("optional"));
    }

    #[test]
    fn an_empty_plan_cannot_be_installed() {
        let plan = assemble_plan(
            "p",
            None,
            target(),
            vec![(
                row("iris", "optional"),
                PackModLookup::NoBuild("none".into()),
            )],
        );
        assert!(!plan.can_install());
    }

    #[test]
    fn batch_items_carry_the_resolved_version() {
        let plan = assemble_plan(
            "p",
            Some("1.0.0".into()),
            target(),
            vec![(
                row("xaero", "optional"),
                PackModLookup::Found {
                    source_type: SourceType::Modrinth,
                    item_id: "1bokaNcj".into(),
                    version: "abc123".into(),
                    display_version: "25.3.2".into(),
                    pinned: true,
                },
            )],
        );
        let items = plan.batch_items();
        assert_eq!(items[0].source_type, SourceType::Modrinth);
        assert_eq!(items[0].item_id, "1bokaNcj");
        assert_eq!(items[0].candidate_version.as_deref(), Some("abc123"));
    }

    #[test]
    fn selection_serializes_with_a_mode_tag() {
        let locked = CuratedPackSelection::Locked {
            pack_version: "1.0.0".into(),
        };
        assert_eq!(
            serde_json::to_value(&locked).unwrap(),
            serde_json::json!({ "mode": "locked", "packVersion": "1.0.0" })
        );
        let flexible: CuratedPackSelection = serde_json::from_value(serde_json::json!({
            "mode": "flexible", "minecraftVersion": "1.20.1", "loader": "forge"
        }))
        .unwrap();
        assert_eq!(
            flexible,
            CuratedPackSelection::Flexible {
                minecraft_version: "1.20.1".into(),
                loader: "forge".into()
            }
        );
    }
}
