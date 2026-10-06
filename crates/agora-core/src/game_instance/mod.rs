//! Generic game instances for non-Minecraft games (MASTER_SPEC §26.3, §26.4, §26.12, §26.13).

use std::path::PathBuf;

use rusqlite::OptionalExtension;

use agora_game_api::{
    BaseMode, BaseReference, GameDefinition, GameId, InstalledFramework, LayerStack,
    RuntimeIdentity,
};
use serde::{Deserialize, Serialize};

use crate::app_paths::AppPaths;
use crate::ctx::Ctx;
use crate::game_base::{BuildOutcome, BuildProgress};
use crate::game_discovery::DiscoveryReport;
use crate::game_launch::{LaunchError, LaunchRoots, PreparedLaunch};
use crate::game_registry::{IdentifiedInstall, RuntimeResolution};

/// A generic instance manifest stored at `<instances_root>/<id>/instance_manifest.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameInstanceManifest {
    pub manifest_version: u32,
    pub game: GameId,
    pub instance_id: String,
    pub name: String,
    pub runtime_identity: Option<RuntimeIdentity>,
    pub base: BaseReference,
    pub frameworks: Vec<InstalledFramework>,
    pub layers: LayerStack,
}

impl GameInstanceManifest {
    pub fn new(
        game: GameId,
        instance_id: impl Into<String>,
        name: impl Into<String>,
        runtime_identity: Option<RuntimeIdentity>,
        base: BaseReference,
    ) -> Self {
        Self {
            manifest_version: 3,
            game,
            instance_id: instance_id.into(),
            name: name.into(),
            runtime_identity,
            base,
            frameworks: Vec::new(),
            layers: LayerStack::default(),
        }
    }
}

/// A persistent index record from the `game_instances` database table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameInstanceRecord {
    pub instance_id: String,
    pub game: GameId,
    pub name: String,
    pub base: BaseReference,
    pub created_at: String,
    pub last_launched_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build_outcome: Option<BuildOutcome>,
}

/// A cross-game summary of an instance (Minecraft or generic).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstanceSummary {
    pub instance_id: String,
    pub game: GameId,
    pub name: String,
    pub runtime: String,
    pub pinned: Option<bool>,
    pub last_launched_at: Option<String>,
}

/// Outcome of deleting an instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteOutcome {
    pub instance_id: String,
    pub orphaned_base: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum InstanceError {
    #[error("runtime is unknown or unidentified: {}", .0.join(", "))]
    RuntimeUnidentified(Vec<String>),
    #[error("instance '{0}' already exists")]
    AlreadyExists(String),
    #[error("instance '{0}' not found")]
    NotFound(String),
    #[error("instance '{0}' is a Minecraft instance; use 'agora launch {0}' to launch or 'agora instance delete {0}' to delete")]
    MinecraftInstance(String),
    #[error("invalid instance ID '{0}': {1}")]
    InvalidId(String, String),
    #[error("instance directory '{0}' is invalid: must be directly inside instances root")]
    InvalidInstanceDir(PathBuf),
    #[error("game install '{0}' is no longer installed")]
    InstallNotFound(String),
    #[error("game install directory '{0}' does not exist")]
    InstallDirMissing(PathBuf),
    #[error("pinned base '{0}' not found")]
    BaseNotFound(String),
    #[error(transparent)]
    BaseError(#[from] crate::game_base::BaseError),
    #[error(transparent)]
    LaunchError(#[from] LaunchError),
    #[error(transparent)]
    Deploy(#[from] crate::game_deploy::DeployError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("{0}")]
    Other(String),
}

/// Generate a valid instance id from a display name:
/// lowercase, `[a-z0-9-]`, runs of `-` collapsed, at most 40 characters,
/// then `-` and 6 random hex characters.
pub fn generate_instance_id_from_name(name: &str) -> String {
    let mut slug = String::new();
    for c in name.chars() {
        let lc = c.to_ascii_lowercase();
        if lc.is_ascii_alphanumeric() {
            slug.push(lc);
        } else if lc == '-' || !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    let slug = if slug.is_empty() {
        "instance"
    } else if slug.len() > 40 {
        slug[..40].trim_end_matches('-')
    } else {
        slug
    };
    let hex = &uuid::Uuid::new_v4().simple().to_string()[..6];
    format!("{slug}-{hex}")
}

fn check_id_exists(
    paths: &AppPaths,
    conn: &rusqlite::Connection,
    candidate: &str,
) -> Result<bool, InstanceError> {
    // Only "no row" means free: a query that fails must not read as "unused".
    for query in [
        "SELECT 1 FROM user_instances WHERE instance_id = ?1",
        "SELECT 1 FROM game_instances WHERE instance_id = ?1",
    ] {
        if conn
            .query_row(query, [candidate], |_| Ok(()))
            .optional()?
            .is_some()
        {
            return Ok(true);
        }
    }
    let dir = paths
        .instance_dir(candidate)
        .map_err(|e| InstanceError::InvalidId(candidate.to_string(), e.to_string()))?;
    if dir.exists() {
        return Ok(true);
    }
    Ok(false)
}

/// Create a generic game instance from an identified install.
pub fn create(
    ctx: &Ctx,
    install: &IdentifiedInstall,
    definition: &GameDefinition,
    name: &str,
    id: Option<String>,
    mode: BaseMode,
    progress: &(dyn Fn(BuildProgress) + Send + Sync),
) -> Result<GameInstanceRecord, InstanceError> {
    create_with_options(
        ctx,
        install,
        definition,
        name,
        id,
        mode,
        crate::game_base::BuildOptions::default(),
        progress,
    )
}

/// Create a generic game instance from an identified install with build options.
#[allow(clippy::too_many_arguments)]
pub fn create_with_options(
    ctx: &Ctx,
    install: &IdentifiedInstall,
    definition: &GameDefinition,
    name: &str,
    id: Option<String>,
    mode: BaseMode,
    options: crate::game_base::BuildOptions,
    progress: &(dyn Fn(BuildProgress) + Send + Sync),
) -> Result<GameInstanceRecord, InstanceError> {
    let runtime = match &install.runtime {
        RuntimeResolution::Identified { runtime, .. } => runtime.clone(),
        RuntimeResolution::Unidentified { reasons } => {
            return Err(InstanceError::RuntimeUnidentified(reasons.clone()));
        }
    };

    let instance_name = if name.trim().is_empty() {
        definition.name.as_str()
    } else {
        name.trim()
    };

    let conn = crate::db::local_state_connection(&ctx.paths.local_state_db())
        .map_err(|e| InstanceError::Other(e.to_string()))?;

    let instance_id = if let Some(chosen_id) = id {
        ctx.paths
            .instance_dir(&chosen_id)
            .map_err(|e| InstanceError::InvalidId(chosen_id.clone(), e.to_string()))?;
        if check_id_exists(&ctx.paths, &conn, &chosen_id)? {
            return Err(InstanceError::AlreadyExists(chosen_id));
        }
        chosen_id
    } else {
        let mut candidate = generate_instance_id_from_name(instance_name);
        for _ in 0..100 {
            if !check_id_exists(&ctx.paths, &conn, &candidate)? {
                break;
            }
            candidate = generate_instance_id_from_name(instance_name);
        }
        if check_id_exists(&ctx.paths, &conn, &candidate)? {
            return Err(InstanceError::AlreadyExists(candidate));
        }
        candidate
    };

    // The id is settled before the base is built: a duplicate or invalid id
    // must not cost a full build and hash first.
    let (base_ref, build_outcome) = if install.discovered.capabilities.executables_readable
        && install.discovered.capabilities.relocatable
    {
        let outcome = crate::game_base::build_base(
            &ctx.paths, install, definition, mode, None, options, progress,
        )?;
        let manifest = outcome.manifest();
        let b_ref = BaseReference::Pinned {
            id: manifest.base_id.clone(),
            runtime: manifest.runtime.clone(),
            mode: manifest.mode,
        };
        (b_ref, Some(outcome))
    } else {
        let reason = if !install.discovered.capabilities.executables_readable
            && !install.discovered.capabilities.relocatable
        {
            "executables are not readable and game is not relocatable".to_string()
        } else if !install.discovered.capabilities.executables_readable {
            if install.discovered.store.as_str() == "microsoft-store" {
                "executables are not readable (Microsoft Store)".to_string()
            } else {
                "executables are not readable".to_string()
            }
        } else {
            "game cannot run from outside its store folder".to_string()
        };
        let b_ref = BaseReference::Unpinned {
            install: install.install_id.clone(),
            reason,
        };
        (b_ref, None)
    };

    let instance_dir = ctx
        .paths
        .instance_dir(&instance_id)
        .map_err(|e| InstanceError::InvalidId(instance_id.clone(), e.to_string()))?;
    std::fs::create_dir_all(&instance_dir)?;

    let manifest = GameInstanceManifest {
        manifest_version: 3,
        game: definition.id.clone(),
        instance_id: instance_id.clone(),
        name: instance_name.to_string(),
        runtime_identity: Some(runtime),
        base: base_ref.clone(),
        frameworks: Vec::new(),
        layers: LayerStack::default(),
    };

    let manifest_path = instance_dir.join("instance_manifest.json");
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    let tmp_path = instance_dir.join(format!(
        "instance_manifest.json.{}.tmp",
        uuid::Uuid::new_v4()
    ));

    let write_result: Result<(), InstanceError> = (|| {
        std::fs::write(&tmp_path, &manifest_bytes)?;
        std::fs::rename(&tmp_path, &manifest_path)?;
        let base_json = serde_json::to_string(&base_ref)?;
        conn.execute(
            "INSERT INTO game_instances (instance_id, game, name, base_json, created_at, last_launched_at)
             VALUES (?1, ?2, ?3, ?4, datetime('now'), NULL)",
            rusqlite::params![
                &instance_id,
                definition.id.as_str(),
                instance_name,
                &base_json
            ],
        )?;
        Ok(())
    })();

    if let Err(e) = write_result {
        let _ = std::fs::remove_dir_all(&instance_dir);
        return Err(e);
    }

    let created_at: String = conn.query_row(
        "SELECT created_at FROM game_instances WHERE instance_id = ?1",
        [&instance_id],
        |row| row.get(0),
    )?;

    Ok(GameInstanceRecord {
        instance_id,
        game: definition.id.clone(),
        name: instance_name.to_string(),
        base: base_ref,
        created_at,
        last_launched_at: None,
        build_outcome,
    })
}

/// Retrieve a generic instance record by ID.
pub fn get(ctx: &Ctx, id: &str) -> Result<Option<GameInstanceRecord>, InstanceError> {
    let db_path = ctx.paths.local_state_db();
    if !db_path.exists() {
        return Ok(None);
    }
    let conn = crate::db::local_state_connection(&db_path)
        .map_err(|e| InstanceError::Other(e.to_string()))?;
    let mut stmt = conn.prepare(
        "SELECT instance_id, game, name, base_json, created_at, last_launched_at
         FROM game_instances WHERE instance_id = ?1",
    )?;
    let mut rows = stmt.query([id])?;
    if let Some(row) = rows.next()? {
        let instance_id: String = row.get(0)?;
        let game_str: String = row.get(1)?;
        let name: String = row.get(2)?;
        let base_json: String = row.get(3)?;
        let created_at: String = row.get(4)?;
        let last_launched_at: Option<String> = row.get(5)?;
        let base: BaseReference = serde_json::from_str(&base_json)?;
        let game = GameId::new(&game_str)
            .map_err(|e| InstanceError::Other(format!("invalid game id in db: {e}")))?;
        Ok(Some(GameInstanceRecord {
            instance_id,
            game,
            name,
            base,
            created_at,
            last_launched_at,
            build_outcome: None,
        }))
    } else {
        Ok(None)
    }
}

/// Read the generic instance manifest from disk for an instance ID.
pub fn get_manifest(ctx: &Ctx, id: &str) -> Result<GameInstanceManifest, InstanceError> {
    let manifest_path = ctx
        .paths
        .instance_dir(id)
        .map_err(|e| InstanceError::InvalidId(id.to_string(), e.to_string()))?
        .join("instance_manifest.json");
    if !manifest_path.exists() {
        return Err(InstanceError::NotFound(id.to_string()));
    }
    let content = std::fs::read_to_string(&manifest_path)?;
    let manifest: GameInstanceManifest = serde_json::from_str(&content)?;
    Ok(manifest)
}

/// List all generic game instances ordered by last launched, then created.
pub fn list(ctx: &Ctx) -> Result<Vec<GameInstanceRecord>, InstanceError> {
    let db_path = ctx.paths.local_state_db();
    if !db_path.exists() {
        return Ok(Vec::new());
    }
    let conn = crate::db::local_state_connection(&db_path)
        .map_err(|e| InstanceError::Other(e.to_string()))?;
    let mut stmt = match conn.prepare(
        "SELECT instance_id, game, name, base_json, created_at, last_launched_at
         FROM game_instances
         ORDER BY last_launched_at DESC NULLS LAST, created_at DESC",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return Ok(Vec::new()),
    };
    let rows = stmt.query_map([], |row| {
        let instance_id: String = row.get(0)?;
        let game_str: String = row.get(1)?;
        let name: String = row.get(2)?;
        let base_json: String = row.get(3)?;
        let created_at: String = row.get(4)?;
        let last_launched_at: Option<String> = row.get(5)?;
        Ok((
            instance_id,
            game_str,
            name,
            base_json,
            created_at,
            last_launched_at,
        ))
    })?;
    let mut records = Vec::new();
    for row in rows {
        let (instance_id, game_str, name, base_json, created_at, last_launched_at) = row?;
        let base: BaseReference = serde_json::from_str(&base_json)?;
        let game = GameId::new(&game_str)
            .map_err(|e| InstanceError::Other(format!("invalid game id in db: {e}")))?;
        records.push(GameInstanceRecord {
            instance_id,
            game,
            name,
            base,
            created_at,
            last_launched_at,
            build_outcome: None,
        });
    }
    Ok(records)
}

/// Whether an instance ID belongs to a Minecraft instance in `user_instances`.
pub fn is_minecraft_instance(ctx: &Ctx, id: &str) -> bool {
    let db_path = ctx.paths.local_state_db();
    if !db_path.exists() {
        return false;
    }
    let Ok(conn) = crate::db::local_state_connection(&db_path) else {
        return false;
    };
    conn.query_row(
        "SELECT 1 FROM user_instances WHERE instance_id = ?1",
        [id],
        |_| Ok(()),
    )
    .is_ok()
}

/// Delete a generic game instance and its folder. Refuses Minecraft instances and folders
/// outside `<instances_root>`.
pub fn delete(ctx: &Ctx, id: &str) -> Result<DeleteOutcome, InstanceError> {
    let db_path = ctx.paths.local_state_db();
    let conn = crate::db::local_state_connection(&db_path)
        .map_err(|e| InstanceError::Other(e.to_string()))?;

    // Refuse an id that is in user_instances (Minecraft instance)
    let in_mc = conn
        .query_row(
            "SELECT 1 FROM user_instances WHERE instance_id = ?1",
            [id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if in_mc {
        return Err(InstanceError::MinecraftInstance(id.to_string()));
    }

    // Check if it exists in game_instances
    let base_json: String = match conn.query_row(
        "SELECT base_json FROM game_instances WHERE instance_id = ?1",
        [id],
        |row| row.get(0),
    ) {
        Ok(json) => json,
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            return Err(InstanceError::NotFound(id.to_string()));
        }
        Err(e) => return Err(e.into()),
    };
    let base_ref: BaseReference = serde_json::from_str(&base_json)?;

    let instance_dir = ctx
        .paths
        .instance_dir(id)
        .map_err(|e| InstanceError::InvalidId(id.to_string(), e.to_string()))?;
    if instance_dir.parent() != Some(&ctx.paths.instances_root()) {
        return Err(InstanceError::InvalidInstanceDir(instance_dir));
    }

    // The row goes first: a folder left behind by a failed removal is inert,
    // while a row whose folder is gone is a broken instance.
    conn.execute("DELETE FROM game_instances WHERE instance_id = ?1", [id])?;
    if instance_dir.exists() {
        std::fs::remove_dir_all(&instance_dir)?;
    }

    let orphaned_base = if let BaseReference::Pinned { id: base_id, .. } = base_ref {
        // Only suggest removing the base when we can tell nothing pins it.
        let unused = crate::game_base::instances_pinning_base(&ctx.paths, &base_id)
            .map(|pinning| pinning.is_empty())
            .unwrap_or(false);
        if unused {
            Some(base_id)
        } else {
            None
        }
    } else {
        None
    };

    Ok(DeleteOutcome {
        instance_id: id.to_string(),
        orphaned_base,
    })
}

/// Update `last_launched_at` timestamp for a game instance.
pub fn record_launch(ctx: &Ctx, id: &str) -> Result<(), InstanceError> {
    let db_path = ctx.paths.local_state_db();
    let conn = crate::db::local_state_connection(&db_path)
        .map_err(|e| InstanceError::Other(e.to_string()))?;
    let count = conn.execute(
        "UPDATE game_instances SET last_launched_at = datetime('now') WHERE instance_id = ?1",
        [id],
    )?;
    if count == 0 {
        return Err(InstanceError::NotFound(id.to_string()));
    }
    Ok(())
}

/// Prepare launch for a generic game instance.
pub fn prepare_launch(
    ctx: &Ctx,
    id: &str,
    definition: &GameDefinition,
    launch_anyway: bool,
) -> Result<PreparedLaunch, InstanceError> {
    prepare_launch_with_discovery(
        ctx,
        id,
        definition,
        launch_anyway,
        &crate::game_discovery::discover_all,
    )
}

/// Prepare launch with a custom discovery function (used for testing unpinned launches).
pub fn prepare_launch_with_discovery(
    ctx: &Ctx,
    id: &str,
    definition: &GameDefinition,
    launch_anyway: bool,
    discover_fn: &dyn Fn() -> DiscoveryReport,
) -> Result<PreparedLaunch, InstanceError> {
    let record = get(ctx, id)?.ok_or_else(|| InstanceError::NotFound(id.to_string()))?;
    let manifest = get_manifest(ctx, id)?;
    match record.base {
        BaseReference::Pinned { id: base_id, .. } => {
            let has_deployment_layers = manifest.layers.iter().any(|l| {
                (matches!(l.source, agora_game_api::LayerSource::Content { .. }) && l.enabled)
                    || matches!(l.source, agora_game_api::LayerSource::Writable { .. })
            });

            if has_deployment_layers {
                let outcome = crate::game_deploy::deploy(
                    ctx,
                    id,
                    definition,
                    crate::game_deploy::DeployMode::Links,
                )?;

                let game_dir = crate::game_deploy::deployment_dir(ctx, id)?.ok_or_else(|| {
                    InstanceError::Other("deployed game directory not found".into())
                })?;

                let manifest_path = ctx.paths.base_manifest_path(&base_id);
                if !manifest_path.exists() {
                    return Err(InstanceError::BaseNotFound(base_id));
                }
                let content = std::fs::read_to_string(&manifest_path)?;
                let base_manifest: crate::game_base::BaseManifest = serde_json::from_str(&content)?;

                let ver = crate::game_base::verify_base(
                    &base_manifest,
                    crate::game_base::VerifyDepth::Quick,
                    &|p| definition.is_declared_write(p),
                    &|p| definition.is_excluded(p),
                );
                if !ver.problems.is_empty() && !launch_anyway {
                    return Err(crate::game_launch::LaunchError::BaseDamaged {
                        problems: ver.problems,
                    }
                    .into());
                }
                let warnings = ver.problems;

                let roots = LaunchRoots {
                    runtime: game_dir,
                    install: Some(base_manifest.source_location.clone()),
                    base: Some(base_manifest.location.clone()),
                };
                let Some(recipe) = &definition.launch else {
                    return Err(LaunchError::NoRecipe.into());
                };
                let mut resolved = crate::game_launch::resolve_recipe(recipe, &roots)?;
                if base_manifest.runtime.store.as_str() == "steam" {
                    let product = base_manifest.source_product.as_deref().or_else(|| {
                        definition
                            .stores
                            .iter()
                            .find(|s| s.store == base_manifest.runtime.store)
                            .map(|s| s.product.as_str())
                    });
                    if let Some(prod) = product {
                        resolved
                            .env
                            .insert("SteamAppId".to_string(), std::ffi::OsString::from(prod));
                        resolved
                            .env
                            .insert("SteamGameId".to_string(), std::ffi::OsString::from(prod));
                    }
                }
                Ok(PreparedLaunch {
                    resolved,
                    warnings,
                    deploy_outcome: Some(outcome),
                })
            } else {
                let manifest_path = ctx.paths.base_manifest_path(&base_id);
                if !manifest_path.exists() {
                    return Err(InstanceError::BaseNotFound(base_id));
                }
                let content = std::fs::read_to_string(&manifest_path)?;
                let base_manifest: crate::game_base::BaseManifest = serde_json::from_str(&content)?;
                let prepared = crate::game_launch::prepare_base_launch(
                    &base_manifest,
                    definition,
                    launch_anyway,
                )?;
                Ok(prepared)
            }
        }
        BaseReference::Unpinned { install, .. } => {
            let has_content = manifest
                .layers
                .iter()
                .any(|l| matches!(l.source, agora_game_api::LayerSource::Content { .. }));
            if has_content {
                return Err(
                    crate::game_deploy::DeployError::UnpinnedInstance(id.to_string()).into(),
                );
            }
            let report = discover_fn();
            let matching = report.installs.iter().find(|discovered| {
                crate::game_registry::make_install_id(&discovered.store, &discovered.product)
                    == install
            });
            let Some(discovered) = matching else {
                return Err(InstanceError::InstallNotFound(install.to_string()));
            };
            if !discovered.location.is_dir() {
                return Err(InstanceError::InstallDirMissing(
                    discovered.location.clone(),
                ));
            }
            let roots = LaunchRoots {
                runtime: discovered.location.clone(),
                install: Some(discovered.location.clone()),
                base: None,
            };
            let Some(recipe) = &definition.launch else {
                return Err(LaunchError::NoRecipe.into());
            };
            let mut resolved = crate::game_launch::resolve_recipe(recipe, &roots)?;
            if discovered.store.as_str() == "steam" {
                let product = definition
                    .stores
                    .iter()
                    .find(|s| s.store == discovered.store)
                    .map(|s| s.product.as_str())
                    .unwrap_or(discovered.product.as_str());
                resolved
                    .env
                    .insert("SteamAppId".to_string(), std::ffi::OsString::from(product));
                resolved
                    .env
                    .insert("SteamGameId".to_string(), std::ffi::OsString::from(product));
            }
            Ok(PreparedLaunch {
                resolved,
                warnings: Vec::new(),
                deploy_outcome: None,
            })
        }
    }
}

/// List all instances across all games, including Minecraft.
/// Every game's instances, and a warning for each source that could not be
/// read, so an unreadable table never looks like "no instances".
pub fn list_all(ctx: &Ctx) -> (Vec<InstanceSummary>, Vec<String>) {
    let mut summaries = Vec::new();
    let mut warnings = Vec::new();

    // 1. Generic game instances
    let records = list(ctx).unwrap_or_else(|e| {
        warnings.push(format!("cannot read game instances: {e}"));
        Vec::new()
    });
    {
        for rec in records {
            let (pinned, runtime) = match &rec.base {
                BaseReference::Pinned { runtime, .. } => {
                    (Some(true), format!("{} {}", runtime.store, runtime.version))
                }
                BaseReference::Unpinned { install, .. } => {
                    let rt = ctx
                        .paths
                        .instance_dir(&rec.instance_id)
                        .ok()
                        .map(|d| d.join("instance_manifest.json"))
                        .and_then(|p| std::fs::read_to_string(p).ok())
                        .and_then(|c| serde_json::from_str::<GameInstanceManifest>(&c).ok())
                        .and_then(|m| m.runtime_identity)
                        .map(|rt| format!("{} {}", rt.store, rt.version))
                        .unwrap_or_else(|| install.to_string());
                    (Some(false), rt)
                }
            };
            summaries.push(InstanceSummary {
                instance_id: rec.instance_id,
                game: rec.game,
                name: rec.name,
                runtime,
                pinned,
                last_launched_at: rec.last_launched_at,
            });
        }
    }

    // 2. Minecraft instances through ctx.games.instance_backend()
    if let Ok(backend) = ctx.games.instance_backend() {
        let mc_rows = backend.list(ctx).unwrap_or_else(|e| {
            warnings.push(format!("cannot read Minecraft instances: {e}"));
            Vec::new()
        });
        {
            for row in mc_rows {
                let runtime = if row.loader.is_empty() {
                    row.minecraft_version
                } else {
                    format!("{} {}", row.minecraft_version, row.loader)
                };
                summaries.push(InstanceSummary {
                    instance_id: row.instance_id,
                    game: GameId::minecraft(),
                    name: row.name,
                    runtime,
                    pinned: None,
                    last_launched_at: row.last_launched_at,
                });
            }
        }
    }

    summaries.sort_by(|a, b| {
        b.last_launched_at
            .cmp(&a.last_launched_at)
            .then_with(|| a.game.as_str().cmp(b.game.as_str()))
            .then_with(|| a.name.cmp(&b.name))
    });

    (summaries, warnings)
}
