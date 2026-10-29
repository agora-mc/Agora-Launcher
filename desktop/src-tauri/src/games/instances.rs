//! Game instances: the list, an instance's page, its checks, its launch, create and delete.
//! Owned by the `instances` slice (MASTER_SPEC §26.3, §26.5, §26.14).
//!
//! Each command is a thin adapter over the `agora_core::game_instance` services. The CLI's
//! `agora games instance ...` handlers call the same services, so the two cannot disagree about
//! what an instance is. Launch runs the whole sequence in `launch_with` (deploy, check, swap-in,
//! spawn, record). This file only turns the core's questions into replies the page can show, and
//! watches the game so the user's files are put back when it closes.

use std::path::PathBuf;
use std::time::Duration;

use agora_core::game_base::{BaseProblem, BuildOptions, BuildOutcome, BuildProgress, ProblemKind};
use agora_core::game_deploy::DeployMode;
use agora_core::game_instance::{
    self, GameInstanceManifest, GameInstanceRecord, InstanceError, LaunchOptions, LaunchedInstance,
    VfsFailure,
};
use agora_core::game_launch::{LaunchError, LaunchedGame};
use agora_core::game_user_files::{self, SessionStatus};
use agora_game_api::{
    BaseMode, BaseReference, GameId, RuntimeFileFinding, RuntimeFileProblem, RuntimeIdentity,
    StoreId,
};
use serde::Serialize;
use tauri::Emitter;

use super::error::{GamesError, CORE_FAILED, TASK_FAILED};

/// No instance has this id.
pub const INSTANCE_NOT_FOUND: &str = "instance_not_found";
/// The instance is running, so the change or the launch has to wait until the game closes.
pub const INSTANCE_RUNNING: &str = "instance_running";
/// The request names something that cannot exist: a blank name, an unknown mode, a bad game id.
pub const INVALID_REQUEST: &str = "invalid_request";
/// The id belongs to a Minecraft instance, which the Minecraft pages manage.
pub const MINECRAFT_INSTANCE: &str = "minecraft_instance";
/// No install with that id was found on this machine.
pub const INSTALL_NOT_FOUND: &str = "install_not_found";
/// The install's game version is not one Agora recognises, so it cannot be pinned.
pub const RUNTIME_UNKNOWN: &str = "runtime_unidentified";

/// Progress from `games_instance_create`, sent as this event.
pub const CREATE_PROGRESS_EVENT: &str = "games-instance-create-progress";
/// Sent when a launched game has closed and its user files were put back (or could not be).
pub const INSTANCE_EXITED_EVENT: &str = "games-instance-exited";

/// Progress is sent for every this-many files, and for the last one.
const PROGRESS_EVERY: u64 = 50;

// ---------------------------------------------------------------------------
// Data the frontend reads
// ---------------------------------------------------------------------------

/// The Instances tab: one card per instance of the game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceList {
    pub instances: Vec<InstanceCard>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceCard {
    pub instance_id: String,
    pub game_id: String,
    pub name: String,
    pub store: Option<String>,
    pub version: Option<String>,
    pub pinned: bool,
    /// `virtual`, `links` or `copies` when chosen; `None` means automatic (the game's default).
    pub deployment: Option<String>,
    pub last_launched_at: Option<String>,
    pub running: bool,
}

/// An instance's page header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceDetail {
    pub instance_id: String,
    pub game_id: String,
    pub game_name: String,
    pub name: String,
    pub pinned: bool,
    pub store: Option<String>,
    pub version: Option<String>,
    pub build: Option<String>,
    pub base_id: Option<String>,
    pub install_id: Option<String>,
    /// Why the instance runs from its install folder, when it is not pinned.
    pub unpinned_reason: Option<String>,
    pub deployment: Option<String>,
    /// `shared` (the game's save folder) or `own`.
    pub saves: String,
    pub created_at: String,
    pub last_launched_at: Option<String>,
    pub running: bool,
}

/// One thing a check or a launch found, in the words the page shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckFinding {
    /// `framework`, `load_order`, `tool` or `base`.
    pub kind: &'static str,
    /// True when this stops a launch unless the user launches anyway.
    pub blocks: bool,
    pub title: String,
    /// What is wrong.
    pub detail: String,
    /// Why it matters.
    pub why: String,
    /// What to do about it.
    pub repair: Option<String>,
}

/// An instance's launch checks. A pinned instance is checked; an unpinned one has nothing to check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceCheck {
    pub instance_id: String,
    pub checked: bool,
    /// `ok`, `warnings`, or `findings` (a finding that stops the launch).
    pub status: &'static str,
    pub findings: Vec<CheckFinding>,
}

/// What a launch did. `status` says which of the three answers this is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchReply {
    /// `launched`: the game started. `refused`: a check stopped it; see `findings`.
    /// `vfs_unavailable`: the virtual file system would not start; `reason` and `next` say what
    /// running from `next` means. Nothing was started; the user is asked first.
    pub status: &'static str,
    pub pid: Option<u32>,
    pub deployment: Option<String>,
    /// How this launch runs, in plain words, when it differs from what was asked.
    pub notice: Option<String>,
    /// For `launched`: what the user launched past. For `refused`: what stopped it.
    pub findings: Vec<CheckFinding>,
    pub reason: Option<String>,
    pub next: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedInstance {
    pub instance_id: String,
    pub name: String,
    /// `pinned` (a base was built or reused) or `unpinned` (it runs from the install folder).
    pub base: &'static str,
    /// True when an identical base already existed and was reused rather than built.
    pub reused_base: bool,
    pub base_id: Option<String>,
    pub unpinned_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletedInstance {
    pub instance_id: String,
    /// The base the deleted instance pinned, when nothing else pins it any more.
    pub orphaned_base: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceDeployment {
    pub instance_id: String,
    pub deployment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateProgress {
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_hashed: u64,
    pub bytes_total: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceExited {
    pub instance_id: String,
    /// The reason the user's files could not be put back, or `None` when they were.
    pub restore_error: Option<String>,
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// The generic instances of one game, for its Instances tab.
#[tauri::command]
pub async fn games_instance_list(
    app: tauri::AppHandle,
    game_id: String,
) -> Result<InstanceList, GamesError> {
    blocking(move || list_instances(&app, &game_id)).await
}

/// One instance, for its page's header.
#[tauri::command]
pub async fn games_instance_get(
    app: tauri::AppHandle,
    instance_id: String,
) -> Result<InstanceDetail, GamesError> {
    blocking(move || instance_detail(&app, &instance_id)).await
}

/// Create an instance from an install of a game. Builds the pinned base (or reuses one that
/// exists) and reports progress as `games-instance-create-progress`.
#[tauri::command]
pub async fn games_instance_create(
    app: tauri::AppHandle,
    game_id: String,
    install_id: String,
    name: String,
    base_mode: String,
) -> Result<CreatedInstance, GamesError> {
    blocking(move || create_instance(&app, &game_id, &install_id, &name, &base_mode)).await
}

/// Delete an instance and its folder. Refused while the game is running.
#[tauri::command]
pub async fn games_instance_delete(
    app: tauri::AppHandle,
    instance_id: String,
) -> Result<DeletedInstance, GamesError> {
    blocking(move || delete_instance(&app, &instance_id)).await
}

/// The launch checks for an instance, without starting anything.
#[tauri::command]
pub async fn games_instance_check(
    app: tauri::AppHandle,
    instance_id: String,
) -> Result<InstanceCheck, GamesError> {
    blocking(move || check_instance(&app, &instance_id)).await
}

/// Choose how an instance deploys and runs: `auto`, `virtual`, `links` or `copies`.
#[tauri::command]
pub async fn games_instance_set_deployment(
    app: tauri::AppHandle,
    instance_id: String,
    mode: String,
) -> Result<InstanceDeployment, GamesError> {
    blocking(move || set_deployment(&app, &instance_id, &mode)).await
}

/// Launch an instance. `deployment` runs this launch only (`None` uses the instance's choice);
/// `launch_anyway` starts the game past the checks that would stop it.
#[tauri::command]
pub async fn games_instance_launch(
    app: tauri::AppHandle,
    instance_id: String,
    launch_anyway: bool,
    deployment: Option<String>,
) -> Result<LaunchReply, GamesError> {
    let chosen = match deployment {
        Some(word) => parse_deployment(&word)?,
        None => None,
    };
    blocking(move || launch_instance(&app, &instance_id, launch_anyway, chosen)).await
}

async fn blocking<T, F>(work: F) -> Result<T, GamesError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, GamesError> + Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| GamesError::new(TASK_FAILED, "The instance work did not finish."))?
}

// ---------------------------------------------------------------------------
// The work
// ---------------------------------------------------------------------------

fn list_instances(app: &tauri::AppHandle, game_id: &str) -> Result<InstanceList, GamesError> {
    let game = parse_game_id(game_id)?;
    let ctx = crate::core_context(app)?;
    let records = game_instance::list(&ctx).map_err(map_error)?;
    let mut instances = Vec::new();
    for record in records
        .iter()
        .filter(|record| record.game.as_str() == game.as_str())
    {
        let manifest = game_instance::get_manifest(&ctx, &record.instance_id).map_err(map_error)?;
        let running = is_running(&ctx, record, &manifest);
        instances.push(card_for(record, &manifest, running));
    }
    Ok(InstanceList { instances })
}

fn instance_detail(
    app: &tauri::AppHandle,
    instance_id: &str,
) -> Result<InstanceDetail, GamesError> {
    let ctx = crate::core_context(app)?;
    let record = load_record(&ctx, instance_id)?;
    let manifest = game_instance::get_manifest(&ctx, instance_id).map_err(map_error)?;
    let definition = ctx
        .games
        .game(&record.game)
        .ok_or_else(|| game_missing(record.game.as_str()))?;
    let running = is_running(&ctx, &record, &manifest);
    let runtime = runtime_of(&record, &manifest);
    Ok(InstanceDetail {
        instance_id: record.instance_id.clone(),
        game_id: record.game.as_str().to_string(),
        game_name: definition.name.clone(),
        name: record.name.clone(),
        pinned: is_pinned(&record),
        store: runtime.as_ref().map(|r| r.store.as_str().to_string()),
        version: runtime.as_ref().map(|r| r.version.clone()),
        build: runtime.as_ref().and_then(|r| r.build.clone()),
        base_id: match &record.base {
            BaseReference::Pinned { id, .. } => Some(id.clone()),
            BaseReference::Unpinned { .. } => None,
        },
        install_id: match &record.base {
            BaseReference::Unpinned { install, .. } => Some(install.to_string()),
            BaseReference::Pinned { .. } => None,
        },
        unpinned_reason: match &record.base {
            BaseReference::Unpinned { reason, .. } => Some(reason.clone()),
            BaseReference::Pinned { .. } => None,
        },
        deployment: manifest.deployment.map(|mode| mode.as_str().to_string()),
        saves: manifest.saves.as_str().to_string(),
        created_at: record.created_at.clone(),
        last_launched_at: record.last_launched_at.clone(),
        running,
    })
}

fn create_instance(
    app: &tauri::AppHandle,
    game_id: &str,
    install_id: &str,
    name: &str,
    base_mode: &str,
) -> Result<CreatedInstance, GamesError> {
    let game = parse_game_id(game_id)?;
    let mode = parse_base_mode(base_mode)?;
    let name = clean_name(name)?;
    let ctx = crate::core_context(app)?;

    // The same discovery and identification the Setup page and `agora games list` use.
    let inventory = agora_core::game_registry::installed_games(&ctx);
    let install = inventory
        .installs
        .iter()
        .find(|install| install.install_id.as_str() == install_id)
        .ok_or_else(|| {
            GamesError::new(
                INSTALL_NOT_FOUND,
                format!("No install '{install_id}' was found. Rescan the installs in Setup and try again."),
            )
        })?;
    if install.game.as_str() != game.as_str() {
        return Err(GamesError::new(
            INVALID_REQUEST,
            format!("Install '{install_id}' is not a {game_id} install."),
        ));
    }
    let definition = ctx
        .games
        .game(&install.game)
        .ok_or_else(|| game_missing(install.game.as_str()))?;

    let progress_app = app.clone();
    let record = game_instance::create_with_options(
        &ctx,
        install,
        definition,
        &name,
        None,
        mode,
        BuildOptions::default(),
        &move |progress: BuildProgress| {
            if progress.files_done.is_multiple_of(PROGRESS_EVERY)
                || progress.files_done == progress.files_total
            {
                // Progress is cosmetic: a failed emit must not fail the create.
                let _ = progress_app.emit(
                    CREATE_PROGRESS_EVENT,
                    CreateProgress {
                        files_done: progress.files_done,
                        files_total: progress.files_total,
                        bytes_hashed: progress.bytes_hashed,
                        bytes_total: progress.bytes_total,
                    },
                );
            }
        },
    )
    .map_err(map_error)?;

    Ok(created_for(
        &record,
        name,
        reused_base(record.build_outcome.as_ref()),
    ))
}

fn delete_instance(
    app: &tauri::AppHandle,
    instance_id: &str,
) -> Result<DeletedInstance, GamesError> {
    let ctx = crate::core_context(app)?;
    let record = load_record(&ctx, instance_id)?;
    let manifest = game_instance::get_manifest(&ctx, instance_id).map_err(map_error)?;
    if is_running(&ctx, &record, &manifest) {
        return Err(running_error(instance_id));
    }
    let outcome = game_instance::delete(&ctx, instance_id).map_err(map_error)?;
    Ok(DeletedInstance {
        instance_id: outcome.instance_id,
        orphaned_base: outcome.orphaned_base,
    })
}

fn check_instance(app: &tauri::AppHandle, instance_id: &str) -> Result<InstanceCheck, GamesError> {
    let ctx = crate::core_context(app)?;
    let record = load_record(&ctx, instance_id)?;
    let definition = ctx
        .games
        .game(&record.game)
        .ok_or_else(|| game_missing(record.game.as_str()))?;
    let pinned = is_pinned(&record);

    let mut findings = Vec::new();
    if pinned {
        // The same three checks `agora games instance check` runs, in the same order.
        for finding in
            game_instance::runtime_findings(&ctx, instance_id, definition).map_err(map_error)?
        {
            findings.push(framework_finding(&finding));
        }
        if definition.plugin_list.is_some() {
            let order = agora_core::game_load_order::check(&ctx, instance_id, definition)
                .map_err(core_failed)?;
            for finding in &order {
                findings.push(load_order_finding(finding));
            }
        }
        let outputs = agora_core::game_tools::output_findings(&ctx, instance_id, definition)
            .map_err(core_failed)?;
        for output in &outputs {
            findings.push(tool_finding(output));
        }
    }
    Ok(InstanceCheck {
        instance_id: instance_id.to_string(),
        checked: pinned,
        status: status_for(&findings),
        findings,
    })
}

fn set_deployment(
    app: &tauri::AppHandle,
    instance_id: &str,
    mode: &str,
) -> Result<InstanceDeployment, GamesError> {
    let chosen = parse_deployment(mode)?;
    let ctx = crate::core_context(app)?;
    let record = load_record(&ctx, instance_id)?;
    let manifest = game_instance::get_manifest(&ctx, instance_id).map_err(map_error)?;
    if is_running(&ctx, &record, &manifest) {
        return Err(running_error(instance_id));
    }
    agora_core::game_deploy::set_deployment(&ctx, instance_id, chosen).map_err(core_failed)?;
    Ok(InstanceDeployment {
        instance_id: instance_id.to_string(),
        deployment: chosen.map(|mode| mode.as_str().to_string()),
    })
}

fn launch_instance(
    app: &tauri::AppHandle,
    instance_id: &str,
    launch_anyway: bool,
    deployment: Option<DeployMode>,
) -> Result<LaunchReply, GamesError> {
    let ctx = crate::core_context(app)?;
    let record = load_record(&ctx, instance_id)?;
    let manifest = game_instance::get_manifest(&ctx, instance_id).map_err(map_error)?;
    if is_running(&ctx, &record, &manifest) {
        return Err(running_error(instance_id));
    }
    let definition = ctx
        .games
        .game(&record.game)
        .ok_or_else(|| game_missing(record.game.as_str()))?;

    let options = LaunchOptions {
        launch_anyway,
        plain: false,
        deployment,
        // Ask, never step down on our own: the page asks the user and launches again if they say yes.
        on_vfs_failure: VfsFailure::Ask,
    };
    let launched = match game_instance::launch_with(
        &ctx,
        instance_id,
        definition,
        options,
        &|| agora_core::game_discovery::discover_with_folders(&ctx.paths),
        &agora_core::game_launch::SystemLauncher,
    ) {
        Ok(launched) => launched,
        Err(InstanceError::VfsUnavailable { reason, next, .. }) => {
            return Ok(vfs_question(reason, next));
        }
        Err(InstanceError::LaunchError(LaunchError::RuntimeMismatch { findings })) => {
            return Ok(refused(findings.iter().map(framework_finding).collect()));
        }
        Err(InstanceError::LaunchError(LaunchError::LoadOrderProblems { findings })) => {
            return Ok(refused(findings.iter().map(load_order_finding).collect()));
        }
        Err(InstanceError::LaunchError(LaunchError::BaseDamaged { problems })) => {
            return Ok(refused(
                problems.iter().map(|p| base_finding(p, true)).collect(),
            ));
        }
        Err(other) => return Err(map_error(other)),
    };

    let LaunchedInstance {
        launched: game,
        prepared,
        store,
        running_from,
    } = launched;
    let mut findings: Vec<CheckFinding> = Vec::new();
    findings.extend(prepared.warnings.iter().map(|p| base_finding(p, false)));
    findings.extend(prepared.runtime_findings.iter().map(framework_finding));
    findings.extend(prepared.load_order_findings.iter().map(load_order_finding));
    findings.extend(prepared.generated_findings.iter().map(tool_finding));
    let reply = LaunchReply {
        status: "launched",
        pid: Some(game.pid()),
        deployment: prepared.deployment.map(|mode| mode.as_str().to_string()),
        notice: prepared.notice.clone(),
        findings,
        reason: None,
        next: None,
    };

    // The user's saves and settings are swapped in for this session. They go back when the game
    // closes, the same as `agora games instance launch --wait`; the page hears about it either way.
    spawn_exit_watch(
        app.clone(),
        ctx,
        instance_id.to_string(),
        game,
        running_from,
        store,
        record.game.clone(),
    );
    Ok(reply)
}

/// Waits for the launched game to close, then puts the user's files back and tells the page.
/// A restore that fails is reported, never dropped: the files stay swapped until someone looks.
fn spawn_exit_watch(
    app: tauri::AppHandle,
    ctx: agora_core::ctx::Ctx,
    instance_id: String,
    mut game: LaunchedGame,
    running_from: PathBuf,
    store: StoreId,
    game_id: GameId,
) {
    std::thread::spawn(move || {
        let _ = agora_core::game_launch::wait_for_exit(
            &running_from,
            &mut game,
            Duration::from_millis(250),
            Duration::from_secs(5),
        );
        let restore_error = match ctx.games.game(&game_id) {
            Some(definition) => match game_user_files::restore(&ctx, definition, &store) {
                Ok(_) => None,
                Err(error) => Some(error.to_string()),
            },
            None => Some(game_missing(game_id.as_str()).message),
        };
        // The event is for the page to refresh; the restore result above is the real outcome.
        let _ = app.emit(
            INSTANCE_EXITED_EVENT,
            InstanceExited {
                instance_id,
                restore_error,
            },
        );
    });
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn load_record(
    ctx: &agora_core::ctx::Ctx,
    instance_id: &str,
) -> Result<GameInstanceRecord, GamesError> {
    game_instance::get(ctx, instance_id)
        .map_err(map_error)?
        .ok_or_else(|| not_found(instance_id))
}

fn is_pinned(record: &GameInstanceRecord) -> bool {
    matches!(record.base, BaseReference::Pinned { .. })
}

/// The runtime an instance was built for: its pinned base's, else the identity its manifest kept.
fn runtime_of(
    record: &GameInstanceRecord,
    manifest: &GameInstanceManifest,
) -> Option<RuntimeIdentity> {
    match &record.base {
        BaseReference::Pinned { runtime, .. } => Some(runtime.clone()),
        BaseReference::Unpinned { .. } => manifest.runtime_identity.clone(),
    }
}

/// Whether the game is running from this instance right now, from the user-file session.
fn is_running(
    ctx: &agora_core::ctx::Ctx,
    record: &GameInstanceRecord,
    manifest: &GameInstanceManifest,
) -> bool {
    match runtime_of(record, manifest) {
        Some(runtime) => running_from_session(
            game_user_files::status(ctx, &record.game, &runtime.store),
            &record.instance_id,
        ),
        // An instance with no recorded runtime has no store to look a session up under. It has
        // never been launched through a store this build can name, so no session can be ours.
        None => false,
    }
}

fn card_for(
    record: &GameInstanceRecord,
    manifest: &GameInstanceManifest,
    running: bool,
) -> InstanceCard {
    let runtime = runtime_of(record, manifest);
    InstanceCard {
        instance_id: record.instance_id.clone(),
        game_id: record.game.as_str().to_string(),
        name: record.name.clone(),
        store: runtime.as_ref().map(|r| r.store.as_str().to_string()),
        version: runtime.as_ref().map(|r| r.version.clone()),
        pinned: is_pinned(record),
        deployment: manifest.deployment.map(|mode| mode.as_str().to_string()),
        last_launched_at: record.last_launched_at.clone(),
        running,
    }
}

fn created_for(record: &GameInstanceRecord, name: String, reused: bool) -> CreatedInstance {
    let (base, base_id, unpinned_reason) = match &record.base {
        BaseReference::Pinned { id, .. } => ("pinned", Some(id.clone()), None),
        BaseReference::Unpinned { reason, .. } => ("unpinned", None, Some(reason.clone())),
    };
    CreatedInstance {
        instance_id: record.instance_id.clone(),
        name,
        base,
        reused_base: reused,
        base_id,
        unpinned_reason,
    }
}

/// True when the build found an identical base already on disk and reused it.
fn reused_base(outcome: Option<&BuildOutcome>) -> bool {
    matches!(outcome, Some(BuildOutcome::Existing(_)))
}

/// True only when the session belongs to this instance and the game is still running from it.
/// No session at all means nothing is running.
pub fn running_from_session(session: Option<SessionStatus>, instance_id: &str) -> bool {
    matches!(session, Some(s) if s.running && s.instance == instance_id)
}

/// The three states a check can be in: nothing found, something to review, or a finding that stops
/// the launch.
pub fn status_for(findings: &[CheckFinding]) -> &'static str {
    if findings.iter().any(|f| f.blocks) {
        "findings"
    } else if findings.is_empty() {
        "ok"
    } else {
        "warnings"
    }
}

fn vfs_question(reason: String, next: Option<DeployMode>) -> LaunchReply {
    LaunchReply {
        status: "vfs_unavailable",
        pid: None,
        deployment: None,
        notice: None,
        findings: Vec::new(),
        reason: Some(reason),
        next: next.map(|mode| mode.as_str().to_string()),
    }
}

fn refused(findings: Vec<CheckFinding>) -> LaunchReply {
    LaunchReply {
        status: "refused",
        pid: None,
        deployment: None,
        notice: None,
        findings,
        reason: None,
        next: None,
    }
}

pub fn framework_finding(finding: &RuntimeFileFinding) -> CheckFinding {
    let blocks = finding.refuses_launch();
    let (detail, why) = match &finding.problem {
        RuntimeFileProblem::WrongVersion => (
            format!(
                "It needs {}, but the game has {}.",
                finding.expected,
                if finding.found.is_empty() {
                    "no file of its family".to_string()
                } else {
                    finding.found.join(", ")
                }
            ),
            "A framework built for another game version often stops the game from starting, or makes it crash."
                .to_string(),
        ),
        RuntimeFileProblem::CannotCheck { reason } => (
            format!("This rule cannot be checked: {reason}."),
            "Agora could not compare this framework with the game, so it cannot say whether they match.".to_string(),
        ),
    };
    CheckFinding {
        kind: "framework",
        blocks,
        title: finding.rule_name.clone(),
        detail,
        why,
        repair: non_empty(&finding.repair),
    }
}

pub fn load_order_finding(finding: &agora_core::game_load_order::Finding) -> CheckFinding {
    let blocks = finding.refuses_launch();
    CheckFinding {
        kind: "load_order",
        blocks,
        title: "Plugin load order".to_string(),
        detail: finding.message(),
        why: if blocks {
            "The game refuses plugins that load before the plugins they depend on, and can crash while it starts."
                .to_string()
        } else {
            "The game may still load these plugins, but the order can cause odd behaviour."
                .to_string()
        },
        repair: Some(
            "Open this instance's Plugins tab and choose Sort, then check again.".to_string(),
        ),
    }
}

pub fn tool_finding(output: &agora_core::game_tools::OutputFinding) -> CheckFinding {
    CheckFinding {
        kind: "tool",
        blocks: false,
        title: output.tool.clone(),
        detail: output.message.clone(),
        why: "Tool output made from older files, or that conflicts with another tool, may not match the mods now installed."
            .to_string(),
        repair: Some("Run the tool again from this instance's Tools tab.".to_string()),
    }
}

fn base_finding(problem: &BaseProblem, blocks: bool) -> CheckFinding {
    let linked = if problem.linked_to_store {
        " The store install changed too."
    } else {
        ""
    };
    CheckFinding {
        kind: "base",
        blocks,
        title: "Base file changed".to_string(),
        detail: format!("{}: {}.{linked}", problem.path, problem_text(&problem.kind)),
        why: "The game's files differ from what Agora recorded when it built the base, so the game may behave oddly or crash."
            .to_string(),
        repair: Some("Verify this base under Setup, pinned bases, or launch anyway at your own risk.".to_string()),
    }
}

pub fn problem_text(kind: &ProblemKind) -> String {
    match kind {
        ProblemKind::Missing => "the file is missing".to_string(),
        ProblemKind::SizeChanged { expected, actual } => {
            format!("its size changed (expected {expected} bytes, found {actual})")
        }
        ProblemKind::ContentChanged => "its contents changed".to_string(),
        ProblemKind::Unexpected => "an unexpected file is in the base".to_string(),
    }
}

fn non_empty(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// `auto` is no choice (the game's default). Anything else must be a deployment name, in any case.
pub fn parse_deployment(word: &str) -> Result<Option<DeployMode>, GamesError> {
    if word.trim().eq_ignore_ascii_case("auto") {
        return Ok(None);
    }
    DeployMode::parse(word).map(Some).ok_or_else(|| {
        GamesError::new(
            INVALID_REQUEST,
            format!("'{word}' is not a deployment. Use automatic, virtual, links or copies."),
        )
    })
}

fn parse_base_mode(word: &str) -> Result<BaseMode, GamesError> {
    word.parse::<BaseMode>()
        .map_err(|message| GamesError::new(INVALID_REQUEST, message))
}

fn parse_game_id(game_id: &str) -> Result<GameId, GamesError> {
    if game_id.trim().is_empty() {
        return Err(GamesError::new(INVALID_REQUEST, "No game was named."));
    }
    if game_id == GameId::MINECRAFT {
        return Err(GamesError::new(
            INVALID_REQUEST,
            "Minecraft instances are listed under My Instances, not here.",
        ));
    }
    GameId::new(game_id).map_err(|error| {
        GamesError::new(
            INVALID_REQUEST,
            format!("'{game_id}' is not a game: {error}"),
        )
    })
}

/// An instance name as the user typed it, trimmed. A blank name is refused rather than replaced
/// with the game's name, so the card never shows something the user did not choose.
pub fn clean_name(name: &str) -> Result<String, GamesError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(GamesError::new(
            INVALID_REQUEST,
            "Give the instance a name.",
        ));
    }
    Ok(trimmed.to_string())
}

/// Maps a core refusal to the code the page branches on. Errors that have no page of their own
/// keep the core's message under `core_failed`.
pub fn map_error(error: InstanceError) -> GamesError {
    match error {
        InstanceError::NotFound(id) => not_found(&id),
        InstanceError::MinecraftInstance(id) => GamesError::new(
            MINECRAFT_INSTANCE,
            format!("'{id}' is a Minecraft instance. Manage it under My Instances."),
        ),
        InstanceError::RuntimeUnidentified(reasons) => GamesError::new(
            RUNTIME_UNKNOWN,
            format!(
                "Agora does not recognise this game's version, so it cannot pin it: {}.",
                reasons.join(", ")
            ),
        ),
        InstanceError::InstallNotFound(id) => GamesError::new(
            INSTALL_NOT_FOUND,
            format!("The install '{id}' is no longer on this machine."),
        ),
        other => GamesError::new(CORE_FAILED, other.to_string()),
    }
}

fn core_failed(error: impl std::fmt::Display) -> GamesError {
    GamesError::new(CORE_FAILED, error.to_string())
}

fn not_found(instance_id: &str) -> GamesError {
    GamesError::new(
        INSTANCE_NOT_FOUND,
        format!("No instance called '{instance_id}' was found. It may have been deleted."),
    )
}

fn running_error(instance_id: &str) -> GamesError {
    GamesError::new(
        INSTANCE_RUNNING,
        format!("'{instance_id}' is running. Close the game first, then try again."),
    )
}

fn game_missing(game_id: &str) -> GamesError {
    GamesError::new(
        CORE_FAILED,
        format!("Agora has no game definition for '{game_id}', so this instance cannot be used."),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use agora_core::game_load_order::Finding;
    use agora_core::game_tools::{OutputFinding, OutputStatus};

    fn runtime(version: &str) -> RuntimeIdentity {
        RuntimeIdentity {
            game: GameId::new("skyrim-se").expect("a valid game id"),
            store: StoreId::new("steam").expect("a valid store id"),
            version: version.to_string(),
            build: None,
        }
    }

    fn pinned_record(id: &str, version: &str) -> GameInstanceRecord {
        GameInstanceRecord {
            instance_id: id.to_string(),
            game: GameId::new("skyrim-se").expect("a valid game id"),
            name: "Grounded".to_string(),
            base: BaseReference::Pinned {
                id: "base-1".to_string(),
                runtime: runtime(version),
                mode: BaseMode::Linked,
            },
            created_at: "2026-10-01 10:00:00".to_string(),
            last_launched_at: None,
            build_outcome: None,
        }
    }

    fn manifest_for(
        record: &GameInstanceRecord,
        deployment: Option<DeployMode>,
    ) -> GameInstanceManifest {
        let mut manifest = GameInstanceManifest::new(
            record.game.clone(),
            record.instance_id.clone(),
            record.name.clone(),
            Some(runtime("1.6.1170")),
            record.base.clone(),
        );
        manifest.deployment = deployment;
        manifest
    }

    fn blocking_finding() -> CheckFinding {
        CheckFinding {
            kind: "framework",
            blocks: true,
            title: "t".into(),
            detail: "d".into(),
            why: "w".into(),
            repair: None,
        }
    }

    fn warning_finding() -> CheckFinding {
        CheckFinding {
            blocks: false,
            ..blocking_finding()
        }
    }

    fn wrong_version(found: Vec<&str>) -> RuntimeFileFinding {
        RuntimeFileFinding {
            rule_id: "skse".into(),
            rule_name: "Script extender".into(),
            expected: "skse64_1_6_1170.dll".into(),
            found: found.into_iter().map(str::to_string).collect(),
            repair: "Install the matching extender.".into(),
            problem: RuntimeFileProblem::WrongVersion,
        }
    }

    #[test]
    fn deployment_words_parse_in_any_case_and_auto_means_no_choice() {
        assert_eq!(parse_deployment("auto").expect("auto parses"), None);
        assert_eq!(parse_deployment(" Auto ").expect("auto parses"), None);
        assert_eq!(
            parse_deployment(" Virtual ").expect("virtual parses"),
            Some(DeployMode::Virtual)
        );
        assert_eq!(
            parse_deployment("LINKS").expect("links parses"),
            Some(DeployMode::Links)
        );
        assert_eq!(
            parse_deployment("copies").expect("copies parses"),
            Some(DeployMode::Copies)
        );
    }

    #[test]
    fn hostile_deployment_words_are_refused_with_a_reason() {
        for word in [
            "",
            "   ",
            "vfs",
            "virtual;links",
            "auto virtual",
            "copies\u{0}",
            "../links",
        ] {
            let error = parse_deployment(word).expect_err("a word that is not a mode is refused");
            assert_eq!(error.code, INVALID_REQUEST, "for {word:?}");
            assert!(
                error
                    .message
                    .contains("automatic, virtual, links or copies"),
                "for {word:?}"
            );
        }
    }

    #[test]
    fn base_mode_accepts_linked_and_copied_only() {
        assert_eq!(
            parse_base_mode("Linked").expect("linked parses"),
            BaseMode::Linked
        );
        assert_eq!(
            parse_base_mode("copied").expect("copied parses"),
            BaseMode::Copied
        );
        for word in ["", "both", "linked copied", "link"] {
            let error = parse_base_mode(word).expect_err("an unknown base mode is refused");
            assert_eq!(error.code, INVALID_REQUEST, "for {word:?}");
        }
    }

    #[test]
    fn a_name_is_trimmed_and_a_blank_one_is_refused() {
        assert_eq!(clean_name("  My Skyrim  ").expect("a name"), "My Skyrim");
        assert_eq!(
            clean_name("Skyrim ✨").expect("a name with emoji"),
            "Skyrim ✨"
        );
        for blank in ["", "   ", "\t\n", "\u{00a0}"] {
            let error = clean_name(blank).expect_err("a blank name is refused");
            assert_eq!(error.code, INVALID_REQUEST, "for {blank:?}");
        }
    }

    #[test]
    fn game_ids_refuse_blank_and_minecraft() {
        assert_eq!(
            parse_game_id("skyrim-se").expect("a game").as_str(),
            "skyrim-se"
        );
        for bad in ["", "   ", "minecraft"] {
            let error = parse_game_id(bad).expect_err("not a generic game");
            assert_eq!(error.code, INVALID_REQUEST, "for {bad:?}");
        }
    }

    #[test]
    fn core_refusals_keep_the_codes_the_page_branches_on() {
        let not_found_error = map_error(InstanceError::NotFound("grounded-1".into()));
        assert_eq!(not_found_error.code, INSTANCE_NOT_FOUND);
        assert!(not_found_error.message.contains("grounded-1"));

        let minecraft = map_error(InstanceError::MinecraftInstance("mc-1".into()));
        assert_eq!(minecraft.code, MINECRAFT_INSTANCE);

        let unknown = map_error(InstanceError::RuntimeUnidentified(vec![
            "no version file".into()
        ]));
        assert_eq!(unknown.code, RUNTIME_UNKNOWN);
        assert!(unknown.message.contains("no version file"));

        let other = map_error(InstanceError::Other("disk full".into()));
        assert_eq!(other.code, CORE_FAILED);
        assert!(other.message.contains("disk full"));
    }

    #[test]
    fn a_check_is_findings_only_when_a_finding_blocks() {
        assert_eq!(status_for(&[]), "ok");
        assert_eq!(status_for(&[warning_finding()]), "warnings");
        assert_eq!(
            status_for(&[warning_finding(), blocking_finding()]),
            "findings"
        );
    }

    #[test]
    fn a_framework_built_for_another_version_blocks_and_says_what_it_found() {
        let blocking = framework_finding(&wrong_version(vec!["skse64_1_5_97.dll"]));
        assert_eq!(blocking.kind, "framework");
        assert!(blocking.blocks);
        assert!(blocking.detail.contains("skse64_1_6_1170.dll"));
        assert!(blocking.detail.contains("skse64_1_5_97.dll"));
        assert_eq!(
            blocking.repair.as_deref(),
            Some("Install the matching extender.")
        );

        let empty = framework_finding(&wrong_version(Vec::new()));
        assert!(empty.detail.contains("no file of its family"));
    }

    #[test]
    fn a_rule_that_cannot_be_checked_only_warns() {
        let finding = RuntimeFileFinding {
            problem: RuntimeFileProblem::CannotCheck {
                reason: "bad template".into(),
            },
            repair: "   ".into(),
            ..wrong_version(Vec::new())
        };
        let shown = framework_finding(&finding);
        assert!(
            !shown.blocks,
            "a definition bug is a warning, not a refusal"
        );
        assert!(shown.detail.contains("bad template"));
        assert_eq!(shown.repair, None, "a blank repair is no repair");
    }

    #[test]
    fn only_refusing_plugin_findings_block() {
        let cycle = load_order_finding(&Finding::MasterCycle {
            plugins: vec!["a.esm".into(), "b.esm".into()],
        });
        assert!(cycle.blocks);
        let unreadable = load_order_finding(&Finding::UnreadableHeader {
            plugin: "c.esp".into(),
            reason: "truncated".into(),
        });
        assert!(
            !unreadable.blocks,
            "a header that cannot be read is a warning"
        );
        assert_eq!(unreadable.kind, "load_order");
    }

    #[test]
    fn tool_output_never_blocks_a_launch() {
        let stale = tool_finding(&OutputFinding {
            tool: "Nemesis".into(),
            generation: "gen-3".into(),
            status: OutputStatus::Stale,
            message: "Nemesis output is older than the mods".into(),
        });
        assert!(!stale.blocks);
        assert_eq!(stale.kind, "tool");
        assert_eq!(stale.title, "Nemesis");
    }

    #[test]
    fn a_session_only_counts_as_running_for_its_own_instance() {
        let session = |instance: &str, running: bool| SessionStatus {
            instance: instance.to_string(),
            running,
            files: Vec::new(),
        };
        assert!(
            !running_from_session(None, "grounded-1"),
            "no session is not running"
        );
        assert!(!running_from_session(
            Some(session("other", true)),
            "grounded-1"
        ));
        assert!(!running_from_session(
            Some(session("grounded-1", false)),
            "grounded-1"
        ));
        assert!(running_from_session(
            Some(session("grounded-1", true)),
            "grounded-1"
        ));
    }

    #[test]
    fn every_base_problem_kind_has_its_own_plain_words() {
        let texts = [
            problem_text(&ProblemKind::Missing),
            problem_text(&ProblemKind::SizeChanged {
                expected: 10,
                actual: 12,
            }),
            problem_text(&ProblemKind::ContentChanged),
            problem_text(&ProblemKind::Unexpected),
        ];
        for (i, text) in texts.iter().enumerate() {
            assert!(!text.is_empty());
            for other in &texts[i + 1..] {
                assert_ne!(text, other);
            }
        }
        assert!(problem_text(&ProblemKind::SizeChanged {
            expected: 10,
            actual: 12
        })
        .contains("expected 10"));
    }

    #[test]
    fn a_card_shows_the_pinned_runtime_and_automatic_deployment() {
        let record = pinned_record("grounded-1", "1.6.1170");
        let manifest = manifest_for(&record, None);
        let card = card_for(&record, &manifest, false);
        assert!(card.pinned);
        assert_eq!(card.store.as_deref(), Some("steam"));
        assert_eq!(card.version.as_deref(), Some("1.6.1170"));
        assert_eq!(card.deployment, None, "no choice recorded means automatic");
        assert!(!card.running);

        let chosen = card_for(
            &record,
            &manifest_for(&record, Some(DeployMode::Links)),
            true,
        );
        assert_eq!(chosen.deployment.as_deref(), Some("links"));
        assert!(chosen.running);
    }

    #[test]
    fn a_reused_base_is_told_apart_from_a_fresh_build() {
        assert!(!reused_base(None), "an unpinned instance built no base");
        let manifest: agora_core::game_base::BaseManifest = serde_json::from_value(serde_json::json!({
            "manifest_version": 1,
            "base_id": "skyrim-se_steam_1.6.1170",
            "runtime": { "game": "skyrim-se", "store": "steam", "version": "1.6.1170", "build": null },
            "mode": "linked",
            "source_install": "steam-489830",
            "source_location": "D:/Games/Skyrim Special Edition",
            "location": "D:/Agora/bases/skyrim-se",
            "created_unix_ms": 0,
            "files": [],
            "skipped": []
        }))
        .expect("a base manifest in the shape the core writes");
        assert!(reused_base(Some(&BuildOutcome::Existing(manifest.clone()))));
        assert!(!reused_base(Some(&BuildOutcome::Built {
            manifest,
            linked_bytes: 0,
            copied_bytes: 0,
        })));
    }

    #[test]
    fn the_exit_event_uses_the_page_field_names() {
        let value = serde_json::to_value(InstanceExited {
            instance_id: "grounded-1".into(),
            restore_error: Some("hash mismatch".into()),
        })
        .expect("the event serializes");
        assert_eq!(value["instanceId"], "grounded-1");
        assert_eq!(value["restoreError"], "hash mismatch");
    }

    /// The page (`features/games/instances/api.ts`) reads these names. A renamed field fails here
    /// before it becomes a page that quietly shows nothing.
    #[test]
    fn the_replies_use_the_names_the_page_reads() {
        let record = pinned_record("grounded-1", "1.6.1170");
        let card = serde_json::to_value(card_for(&record, &manifest_for(&record, None), false))
            .expect("a card serializes");
        for key in [
            "instanceId",
            "gameId",
            "name",
            "store",
            "version",
            "pinned",
            "deployment",
            "lastLaunchedAt",
            "running",
        ] {
            assert!(card.get(key).is_some(), "card is missing {key}");
        }

        let finding = serde_json::to_value(framework_finding(&wrong_version(Vec::new())))
            .expect("a finding serializes");
        for key in ["kind", "blocks", "title", "detail", "why", "repair"] {
            assert!(finding.get(key).is_some(), "finding is missing {key}");
        }

        let reply = serde_json::to_value(refused(Vec::new())).expect("a reply serializes");
        for key in [
            "status",
            "pid",
            "deployment",
            "notice",
            "findings",
            "reason",
            "next",
        ] {
            assert!(reply.get(key).is_some(), "launch reply is missing {key}");
        }
        assert_eq!(reply["status"], "refused");
    }

    #[test]
    fn a_refused_launch_names_what_stopped_it() {
        let reply = refused(vec![framework_finding(&wrong_version(Vec::new()))]);
        assert_eq!(reply.status, "refused");
        assert_eq!(reply.pid, None);
        assert_eq!(status_for(&reply.findings), "findings");

        let base = base_finding(
            &BaseProblem {
                path: "Data/Skyrim.esm".into(),
                kind: ProblemKind::ContentChanged,
                linked_to_store: true,
            },
            true,
        );
        assert!(base.blocks);
        assert!(base.detail.contains("Data/Skyrim.esm"));
        assert!(base.detail.contains("store install changed too"));
    }

    #[test]
    fn a_vfs_failure_asks_and_names_the_next_rung_without_starting_anything() {
        let reply = vfs_question("the driver is missing".into(), Some(DeployMode::Links));
        assert_eq!(reply.status, "vfs_unavailable");
        assert_eq!(reply.pid, None);
        assert_eq!(reply.next.as_deref(), Some("links"));
        assert_eq!(reply.reason.as_deref(), Some("the driver is missing"));
        assert!(reply.findings.is_empty());
    }
}
