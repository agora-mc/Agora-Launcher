//! Tools: external programs (such as Nemesis or BodySlide) that write into an instance, with their
//! output tracked so it can be diffed, rolled back, removed or swapped (MASTER_SPEC §26.9).
//! Owned by the `tools` slice.
//!
//! Each command is an adapter over agora-core's `game_tools`, `game_tool_copy` and `game_tool_swap`.
//! The desktop decides nothing those services do not. Two answers that need the user are replies,
//! not errors and not runs: a virtual file system that could not start (`RunResult::VfsRefused`)
//! and a tool that works on the real install (`RunResult::NeedsSwap`). Nothing ran in either case,
//! and the screen asks before it captures from the next tier.

use std::collections::HashSet;
use std::path::Path;

use agora_core::ctx::Ctx;
use agora_core::event_sink::CancellationToken;
use agora_core::game_launch::SystemLauncher;
use agora_core::game_tool_copy::{self, CopyKind, SwapCosts};
use agora_core::game_tool_swap::{self, PendingSwap};
use agora_core::game_tools::{
    self, CaptureMethod, CaptureMode, OutputFinding, OutputStatus, RunOutcome, ToolError,
    ToolOffer, ToolState,
};
use agora_game_api::{GameDefinition, ToolId};
use serde::Serialize;

use super::error::{GamesError, CORE_FAILED, TASK_FAILED};

/// The request names a tool or a capture the tools cannot act on.
pub const INVALID_REQUEST: &str = "invalid_request";
/// The instance (or its game) is not known to this install.
pub const NOT_FOUND: &str = "not_found";
/// A full-copy swap was asked for without its costs having been shown and accepted.
pub const SWAP_NOT_CONFIRMED: &str = "swap_not_confirmed";

/// Every tool the instance's game declares, with its output here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsView {
    pub instance_id: String,
    pub tools: Vec<ToolRow>,
}

/// One declared tool and the output the instance has for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolRow {
    pub tool_id: String,
    pub name: String,
    /// Still declared by the game's package. False for output whose tool the game no longer declares.
    pub declared: bool,
    pub current: Option<String>,
    pub previous: Option<String>,
    /// `None` when the instance has no output for the tool yet.
    pub status: Option<OutputStatus>,
    /// Another tool that conflicts with this one has enabled output as well.
    pub conflicts: bool,
}

/// What a run answered. `Finished` covers both a promoted run and a discarded one; `outcome.promoted`
/// says which. The other two are questions for the user, and nothing ran for either.
#[derive(Debug, Serialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum RunResult {
    /// Boxed: a finished run's report is much larger than the two questions, and clippy holds the enum to that.
    Finished {
        outcome: Box<RunReport>,
    },
    VfsRefused {
        reason: String,
        /// The tier that can run instead: `links` or `swap`. `None` when there is no such tier.
        next: Option<CaptureMode>,
        offers: Vec<OfferDto>,
    },
    NeedsSwap {
        tool: String,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunReport {
    pub tool_id: String,
    pub name: String,
    pub run: String,
    pub capture: CaptureMethod,
    pub capture_reason: Option<String>,
    pub promoted: bool,
    pub current: Option<String>,
    pub previous: Option<String>,
    pub written: Vec<String>,
    pub deleted: Vec<String>,
    pub exit_code: Option<i32>,
    pub cancelled: bool,
    pub failed_folder: Option<String>,
    pub failure: Option<String>,
    pub copied_layer: Option<String>,
    pub install_changes: InstallChangesDto,
    pub swap: Option<SwapReportDto>,
    pub offers: Vec<OfferDto>,
    pub unchanged_dropped: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallChangesDto {
    pub created: Vec<String>,
    pub changed: Vec<String>,
    pub deleted: Vec<String>,
    pub may_be_incomplete: bool,
    pub unavailable: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwapReportDto {
    pub copy: CopyKind,
    pub copied_in_files: usize,
    pub copied_in_bytes: u64,
    pub captured_files: usize,
    pub deleted_files: usize,
    pub refreshed_files: usize,
    pub refreshed_bytes: u64,
}

/// Something a tool that keeps failing can try next. Shown and asked about, never applied on its own.
#[derive(Debug, Serialize)]
#[serde(
    tag = "offer",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum OfferDto {
    CopySwap { costs: SwapCostsDto },
    SwitchToLinks,
}

/// What the full-copy swap would cost, before it runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwapCostsDto {
    pub install_dir: String,
    pub data_files: usize,
    pub data_bytes: u64,
    pub same_volume: bool,
    pub copy_kind: CopyKind,
    pub copy_folder: String,
    pub kept_copy_exists: bool,
    pub copy_files: usize,
    pub copy_bytes: u64,
    /// `None` when the free space on the copy's volume cannot be read.
    pub free_bytes: Option<u64>,
    pub steam_running: bool,
}

/// Whether a swap was interrupted and is waiting to be put back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwapStatus {
    pub interrupted: Option<InterruptedSwap>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InterruptedSwap {
    /// The tool whose swap was interrupted. `None` when its journal cannot be read, so the tool is unknown.
    pub tool: Option<String>,
    /// A process is still running from the install folder, so the swap cannot be put back yet.
    pub running: bool,
    /// Why the journal cannot be read, when it cannot.
    pub unreadable: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffDto {
    pub tool_id: String,
    pub previous: String,
    pub current: String,
    pub added: Vec<String>,
    pub changed: Vec<String>,
    pub removed: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveReply {
    pub removed_generations: Vec<String>,
    pub view: ToolsView,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreReply {
    /// The tools whose interrupted swap was put back.
    pub restored_tools: Vec<String>,
}

fn core_error(error: impl std::fmt::Display) -> GamesError {
    GamesError::new(CORE_FAILED, error.to_string())
}

/// Run `work` on a blocking thread with the core context. Tool runs and swaps are slow file work.
async fn with_ctx<T, F>(app: tauri::AppHandle, work: F) -> Result<T, GamesError>
where
    F: FnOnce(Ctx) -> Result<T, GamesError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let ctx = crate::core_context(&app)?;
        work(ctx)
    })
    .await
    .map_err(|_| GamesError::new(TASK_FAILED, "The tools work did not finish."))?
}

/// The game an instance belongs to, read from the instance. An unknown instance is an error, never
/// an empty tool list.
fn definition_of(ctx: &Ctx, instance_id: &str) -> Result<GameDefinition, GamesError> {
    let record = agora_core::game_instance::get(ctx, instance_id)
        .map_err(core_error)?
        .ok_or_else(|| {
            GamesError::new(
                NOT_FOUND,
                format!("Instance '{instance_id}' was not found."),
            )
        })?;
    ctx.games
        .game(&record.game)
        .cloned()
        .ok_or_else(|| core_error(format!("the game '{}' is not known here", record.game)))
}

fn tool_id_of(tool: &str) -> Result<ToolId, GamesError> {
    ToolId::new(tool).map_err(|e| GamesError::new(INVALID_REQUEST, e.to_string()))
}

/// The capture a run uses, by the name the screen sends (`auto`, `vfs`, `links`, `swap`).
pub fn parse_capture(capture: &str) -> Result<CaptureMode, GamesError> {
    CaptureMode::parse(capture).ok_or_else(|| {
        GamesError::new(
            INVALID_REQUEST,
            format!("capture must be auto, vfs, links or swap, not '{capture}'."),
        )
    })
}

/// A swap runs only after its costs were shown and the user accepted them. Refused here too, so the
/// screen is not the only thing standing between a click and a copy of the whole of Data.
pub fn require_swap_confirmed(capture: CaptureMode, confirmed: bool) -> Result<(), GamesError> {
    if capture == CaptureMode::Swap && !confirmed {
        return Err(GamesError::new(
            SWAP_NOT_CONFIRMED,
            "The full-copy swap was not confirmed. Its costs have to be shown and accepted before it runs.",
        ));
    }
    Ok(())
}

/// A run that follows a refusal has to say what was refused. An empty or blank reason is refused here,
/// because the screen's own check is not the only thing standing between a click and the run.
pub fn require_named_refusal(refusal: Option<&str>) -> Result<(), GamesError> {
    match refusal {
        Some(reason) if reason.trim().is_empty() => Err(GamesError::new(
            INVALID_REQUEST,
            "A run that follows a refusal has to say what was refused.",
        )),
        _ => Ok(()),
    }
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// The rows of a tool list, with the conflict flags the output findings carry.
pub fn tools_view(
    instance_id: String,
    states: Vec<ToolState>,
    findings: &[OutputFinding],
) -> ToolsView {
    let conflicting: HashSet<&str> = findings
        .iter()
        .filter(|finding| finding.status == OutputStatus::Conflict)
        .map(|finding| finding.tool.as_str())
        .collect();
    let tools = states
        .into_iter()
        .map(|state| ToolRow {
            conflicts: conflicting.contains(state.tool.as_str()),
            tool_id: state.tool,
            name: state.name,
            declared: state.declared,
            current: state.current,
            previous: state.previous,
            status: state.status,
        })
        .collect();
    ToolsView { instance_id, tools }
}

fn view_of(ctx: &Ctx, instance_id: &str, game: &GameDefinition) -> Result<ToolsView, GamesError> {
    let states = game_tools::list(ctx, instance_id, game).map_err(core_error)?;
    let findings = game_tools::output_findings(ctx, instance_id, game).map_err(core_error)?;
    Ok(tools_view(instance_id.to_string(), states, &findings))
}

impl From<&SwapCosts> for SwapCostsDto {
    fn from(costs: &SwapCosts) -> Self {
        Self {
            install_dir: display_path(&costs.install_dir),
            data_files: costs.data_files,
            data_bytes: costs.data_bytes,
            same_volume: costs.same_volume,
            copy_kind: costs.copy_kind,
            copy_folder: display_path(&costs.copy_folder),
            kept_copy_exists: costs.kept_copy_exists,
            copy_files: costs.copy_files,
            copy_bytes: costs.copy_bytes,
            free_bytes: costs.free_bytes,
            steam_running: costs.steam_running,
        }
    }
}

impl From<&ToolOffer> for OfferDto {
    fn from(offer: &ToolOffer) -> Self {
        match offer {
            ToolOffer::CopySwap { costs } => OfferDto::CopySwap {
                costs: SwapCostsDto::from(costs),
            },
            ToolOffer::SwitchToLinks => OfferDto::SwitchToLinks,
        }
    }
}

fn run_report(outcome: &RunOutcome) -> RunReport {
    RunReport {
        tool_id: outcome.tool.clone(),
        name: outcome.name.clone(),
        run: outcome.run.clone(),
        capture: outcome.capture,
        capture_reason: outcome.capture_reason.clone(),
        promoted: outcome.promoted,
        current: outcome.current.clone(),
        previous: outcome.previous.clone(),
        written: outcome.written.clone(),
        deleted: outcome.deleted.clone(),
        exit_code: outcome.exit_code,
        cancelled: outcome.cancelled,
        failed_folder: outcome.failed_folder.as_deref().map(display_path),
        failure: outcome.failure.clone(),
        copied_layer: outcome.copied_layer.clone(),
        install_changes: InstallChangesDto {
            created: outcome.install_changes.created.clone(),
            changed: outcome.install_changes.changed.clone(),
            deleted: outcome.install_changes.deleted.clone(),
            may_be_incomplete: outcome.install_changes.may_be_incomplete,
            unavailable: outcome.install_changes.unavailable.clone(),
        },
        swap: outcome.swap.as_ref().map(|swap| SwapReportDto {
            copy: swap.copy,
            copied_in_files: swap.copied_in_files,
            copied_in_bytes: swap.copied_in_bytes,
            captured_files: swap.captured_files,
            deleted_files: swap.deleted_files,
            refreshed_files: swap.refreshed_files,
            refreshed_bytes: swap.refreshed_bytes,
        }),
        offers: outcome.offers.iter().map(OfferDto::from).collect(),
        unchanged_dropped: outcome.unchanged_dropped,
    }
}

/// Map a run's result to the reply the screen acts on. Only the two questions become replies; every
/// other failure is an error that names what went wrong.
pub fn run_reply(result: Result<RunOutcome, ToolError>) -> Result<RunResult, GamesError> {
    match result {
        Ok(outcome) => Ok(RunResult::Finished {
            outcome: Box::new(run_report(&outcome)),
        }),
        Err(ToolError::VfsUnavailable {
            reason,
            next,
            offers,
        }) => Ok(RunResult::VfsRefused {
            reason,
            next,
            offers: offers.iter().map(OfferDto::from).collect(),
        }),
        Err(ToolError::InstallPathNeedsSwap { tool, .. }) => Ok(RunResult::NeedsSwap { tool }),
        Err(other) => Err(core_error(other)),
    }
}

/// The interrupted swap that concerns an instance. A journal that cannot be read may belong to any
/// instance of the game, so it is reported for the game rather than dropped.
pub fn interrupted_for(
    pending: &[PendingSwap],
    game: &str,
    instance_id: &str,
) -> Option<InterruptedSwap> {
    pending
        .iter()
        .filter(|swap| swap.game == game)
        .find_map(|swap| match (&swap.journal, &swap.read_error) {
            (Some(journal), _) if journal.instance_id == instance_id => Some(InterruptedSwap {
                tool: Some(journal.tool.clone()),
                running: swap.running,
                unreadable: None,
            }),
            (None, Some(error)) => Some(InterruptedSwap {
                tool: None,
                running: swap.running,
                unreadable: Some(error.clone()),
            }),
            _ => None,
        })
}

/// The tools the instance's game declares, with the output each has in this instance.
#[tauri::command]
pub async fn games_tools_list(
    app: tauri::AppHandle,
    instance_id: String,
) -> Result<ToolsView, GamesError> {
    with_ctx(app, move |ctx| {
        let game = definition_of(&ctx, &instance_id)?;
        view_of(&ctx, &instance_id, &game)
    })
    .await
}

/// Run a tool in the instance. `refusal` is the reason the virtual file system gave, when the user has
/// chosen the tier that runs instead of it; `confirm_swap` says the swap's costs were shown and accepted.
#[tauri::command]
pub async fn games_tools_run(
    app: tauri::AppHandle,
    instance_id: String,
    tool_id: String,
    capture: String,
    refusal: Option<String>,
    confirm_swap: bool,
) -> Result<RunResult, GamesError> {
    let tool = tool_id_of(&tool_id)?;
    let capture = parse_capture(&capture)?;
    require_swap_confirmed(capture, confirm_swap)?;
    require_named_refusal(refusal.as_deref())?;
    with_ctx(app, move |ctx| {
        let game = definition_of(&ctx, &instance_id)?;
        let cancel = CancellationToken::new();
        let result = match refusal {
            Some(reason) => game_tools::run_after_vfs_refusal(
                &ctx,
                &instance_id,
                &game,
                &tool,
                capture,
                &reason,
                &SystemLauncher,
                &cancel,
            ),
            None => game_tools::run(
                &ctx,
                &instance_id,
                &game,
                &tool,
                capture,
                &SystemLauncher,
                &cancel,
            ),
        };
        run_reply(result)
    })
    .await
}

/// Roll the tool's output back to its previous generation. Nothing is deleted.
#[tauri::command]
pub async fn games_tools_rollback(
    app: tauri::AppHandle,
    instance_id: String,
    tool_id: String,
) -> Result<ToolsView, GamesError> {
    let tool = tool_id_of(&tool_id)?;
    with_ctx(app, move |ctx| {
        let game = definition_of(&ctx, &instance_id)?;
        game_tools::rollback(&ctx, &instance_id, &game, &tool).map_err(core_error)?;
        view_of(&ctx, &instance_id, &game)
    })
    .await
}

/// Remove the tool's output from the instance, with its generations.
#[tauri::command]
pub async fn games_tools_remove(
    app: tauri::AppHandle,
    instance_id: String,
    tool_id: String,
) -> Result<RemoveReply, GamesError> {
    let tool = tool_id_of(&tool_id)?;
    with_ctx(app, move |ctx| {
        let game = definition_of(&ctx, &instance_id)?;
        let removed_generations =
            game_tools::remove(&ctx, &instance_id, &game, &tool).map_err(core_error)?;
        let view = view_of(&ctx, &instance_id, &game)?;
        Ok(RemoveReply {
            removed_generations,
            view,
        })
    })
    .await
}

/// The files a tool's latest output added, changed and removed, against the generation before it.
#[tauri::command]
pub async fn games_tools_diff(
    app: tauri::AppHandle,
    instance_id: String,
    tool_id: String,
) -> Result<DiffDto, GamesError> {
    let tool = tool_id_of(&tool_id)?;
    with_ctx(app, move |ctx| {
        let report = game_tools::diff(&ctx, &instance_id, &tool).map_err(core_error)?;
        Ok(DiffDto {
            tool_id: report.tool,
            previous: report.previous,
            current: report.current,
            added: report.added,
            changed: report.changed,
            removed: report.removed,
        })
    })
    .await
}

/// What a full-copy swap of the instance would cost. Changes nothing.
#[tauri::command]
pub async fn games_tools_swap_costs(
    app: tauri::AppHandle,
    instance_id: String,
) -> Result<SwapCostsDto, GamesError> {
    with_ctx(app, move |ctx| {
        let game = definition_of(&ctx, &instance_id)?;
        let costs = game_tool_copy::swap_costs(&ctx, &instance_id, &game).map_err(core_error)?;
        Ok(SwapCostsDto::from(&costs))
    })
    .await
}

/// Whether a swap of this instance was interrupted and is waiting to be put back.
#[tauri::command]
pub async fn games_tools_swap_status(
    app: tauri::AppHandle,
    instance_id: String,
) -> Result<SwapStatus, GamesError> {
    with_ctx(app, move |ctx| {
        let game = definition_of(&ctx, &instance_id)?;
        let pending = game_tool_swap::pending(&ctx);
        Ok(SwapStatus {
            interrupted: interrupted_for(&pending, game.id.as_str(), &instance_id),
        })
    })
    .await
}

/// Put back the real Data folder of an interrupted swap. Refused while a process still runs from the
/// install folder, as core refuses it.
#[tauri::command]
pub async fn games_tools_swap_restore(
    app: tauri::AppHandle,
    instance_id: String,
) -> Result<RestoreReply, GamesError> {
    with_ctx(app, move |ctx| {
        let game = definition_of(&ctx, &instance_id)?;
        let restored = game_tool_swap::recover_game(&ctx, &game).map_err(core_error)?;
        Ok(RestoreReply {
            restored_tools: restored.into_iter().map(|journal| journal.tool).collect(),
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use agora_core::game_tool_swap::SwapJournal;
    use agora_core::install_watch::InstallChanges;
    use std::path::PathBuf;

    fn outcome(promoted: bool) -> RunOutcome {
        RunOutcome {
            tool: "nemesis".into(),
            name: "Nemesis".into(),
            run: "0000000000001-abcd1234".into(),
            capture: CaptureMethod::Vfs,
            capture_reason: None,
            promoted,
            current: Some("2".into()),
            previous: Some("1".into()),
            written: vec!["Data/meshes/a.hkx".into()],
            deleted: vec![],
            exit_code: Some(0),
            cancelled: false,
            failed_folder: None,
            failure: None,
            copied_layer: None,
            install_changes: InstallChanges::default(),
            swap: None,
            offers: vec![],
            unchanged_dropped: 0,
        }
    }

    fn costs() -> SwapCosts {
        SwapCosts {
            install_dir: PathBuf::from(r"D:\Games\Skyrim"),
            data_files: 10,
            data_bytes: 1024,
            same_volume: true,
            copy_kind: CopyKind::Kept,
            copy_folder: PathBuf::from(r"D:\Agora\swap"),
            kept_copy_exists: false,
            copy_files: 10,
            copy_bytes: 1024,
            free_bytes: None,
            steam_running: true,
        }
    }

    fn state(tool: &str, status: Option<OutputStatus>) -> ToolState {
        ToolState {
            tool: tool.into(),
            name: tool.into(),
            declared: true,
            current: status.map(|_| "1".to_string()),
            previous: None,
            status,
        }
    }

    fn finding(tool: &str, status: OutputStatus) -> OutputFinding {
        OutputFinding {
            tool: tool.into(),
            generation: "1".into(),
            status,
            message: String::new(),
        }
    }

    #[test]
    fn capture_names_parse_and_hostile_ones_are_refused() {
        assert_eq!(parse_capture("auto").unwrap(), CaptureMode::Auto);
        assert_eq!(parse_capture(" Links ").unwrap(), CaptureMode::Links);
        assert_eq!(parse_capture("SWAP").unwrap(), CaptureMode::Swap);
        // Surrounding whitespace is trimmed by core's parser, so a newline around a name is not a hostile name.
        assert_eq!(parse_capture("swap\n").unwrap(), CaptureMode::Swap);
        for hostile in ["", "   ", "nemesis", "swap;rm", "swap extra", "vfs\0"] {
            let error = parse_capture(hostile).expect_err(hostile);
            assert_eq!(error.code, INVALID_REQUEST, "{hostile:?}");
        }
    }

    #[test]
    fn tool_ids_are_validated_before_any_work() {
        assert!(tool_id_of("nemesis").is_ok());
        for hostile in ["", "   ", "../evil", "Nemesis", "nemesis\\x", "a b"] {
            let error = tool_id_of(hostile).expect_err(hostile);
            assert_eq!(error.code, INVALID_REQUEST, "{hostile:?}");
        }
    }

    #[test]
    fn a_swap_needs_its_costs_confirmed_and_other_captures_do_not() {
        let refused =
            require_swap_confirmed(CaptureMode::Swap, false).expect_err("unconfirmed swap");
        assert_eq!(refused.code, SWAP_NOT_CONFIRMED);
        assert!(require_swap_confirmed(CaptureMode::Swap, true).is_ok());
        assert!(require_swap_confirmed(CaptureMode::Auto, false).is_ok());
        assert!(require_swap_confirmed(CaptureMode::Links, false).is_ok());
        assert!(require_swap_confirmed(CaptureMode::Vfs, false).is_ok());
    }

    #[test]
    fn a_run_after_a_refusal_must_name_the_refusal() {
        assert!(require_named_refusal(None).is_ok());
        assert!(require_named_refusal(Some("injection failed")).is_ok());
        for hostile in ["", "   ", "\n\t"] {
            let error = require_named_refusal(Some(hostile)).expect_err(hostile);
            assert_eq!(error.code, INVALID_REQUEST, "{hostile:?}");
        }
    }

    #[test]
    fn a_promoted_run_is_finished_and_a_discarded_one_says_why() {
        let mut discarded = outcome(false);
        discarded.failure = Some("exit code 3".into());
        discarded.failed_folder = Some(PathBuf::from(r"D:\x\failed-1"));
        match run_reply(Ok(discarded)).expect("a discarded run is an answer") {
            RunResult::Finished { outcome } => {
                assert!(!outcome.promoted);
                assert_eq!(outcome.failure.as_deref(), Some("exit code 3"));
                assert_eq!(outcome.failed_folder.as_deref(), Some(r"D:\x\failed-1"));
            }
            other => panic!("expected a finished run, got {other:?}"),
        }
    }

    #[test]
    fn a_refused_virtual_file_system_is_a_question_with_its_next_tier() {
        let error = ToolError::VfsUnavailable {
            reason: "injection failed".into(),
            next: Some(CaptureMode::Links),
            offers: vec![ToolOffer::SwitchToLinks],
        };
        match run_reply(Err(error)).expect("a refusal is a reply, not an error") {
            RunResult::VfsRefused {
                reason,
                next,
                offers,
            } => {
                assert_eq!(reason, "injection failed");
                assert_eq!(next, Some(CaptureMode::Links));
                assert!(matches!(offers.as_slice(), [OfferDto::SwitchToLinks]));
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_real_install_tool_asks_for_a_swap() {
        let error = ToolError::InstallPathNeedsSwap {
            tool: "bodyslide".into(),
            instance_id: "sk-1".into(),
        };
        match run_reply(Err(error)).expect("needs-swap is a reply") {
            RunResult::NeedsSwap { tool } => assert_eq!(tool, "bodyslide"),
            other => panic!("expected needs-swap, got {other:?}"),
        }
    }

    #[test]
    fn any_other_failure_is_an_error_that_says_what_failed() {
        let error = run_reply(Err(ToolError::Other("the game is running".into())))
            .expect_err("a real failure is an error");
        assert_eq!(error.code, CORE_FAILED);
        assert!(
            error.message.contains("the game is running"),
            "{}",
            error.message
        );
    }

    #[test]
    fn the_reply_is_camel_case_and_names_the_tier_and_offers() {
        let refusal = RunResult::VfsRefused {
            reason: "x".into(),
            next: Some(CaptureMode::Swap),
            offers: vec![OfferDto::CopySwap {
                costs: SwapCostsDto::from(&costs()),
            }],
        };
        let json = serde_json::to_value(&refusal).unwrap();
        assert_eq!(json["status"], "vfsRefused");
        assert_eq!(json["next"], "swap");
        assert_eq!(json["offers"][0]["offer"], "copySwap");
        assert_eq!(json["offers"][0]["costs"]["copyBytes"], 1024);
        assert_eq!(json["offers"][0]["costs"]["steamRunning"], true);
        assert_eq!(json["offers"][0]["costs"]["copyKind"], "kept");

        let finished = serde_json::to_value(RunResult::Finished {
            outcome: Box::new(run_report(&outcome(true))),
        })
        .unwrap();
        assert_eq!(finished["status"], "finished");
        assert_eq!(finished["outcome"]["capture"], "vfs");
        assert_eq!(finished["outcome"]["toolId"], "nemesis");
        assert_eq!(
            finished["outcome"]["installChanges"]["mayBeIncomplete"],
            false
        );
        assert!(finished["outcome"]["captureReason"].is_null());
    }

    #[test]
    fn a_conflict_marks_both_tools_and_a_stale_finding_marks_neither() {
        let states = vec![
            state("nemesis", Some(OutputStatus::Current)),
            state("pandora", Some(OutputStatus::Current)),
            state("bodyslide", Some(OutputStatus::Stale)),
        ];
        let findings = vec![
            finding("nemesis", OutputStatus::Conflict),
            finding("pandora", OutputStatus::Conflict),
            finding("bodyslide", OutputStatus::Stale),
        ];
        let view = tools_view("sk-1".into(), states, &findings);
        let by_id = |id: &str| {
            view.tools
                .iter()
                .find(|row| row.tool_id == id)
                .unwrap()
                .clone()
        };
        assert!(by_id("nemesis").conflicts);
        assert!(by_id("pandora").conflicts);
        assert!(!by_id("bodyslide").conflicts);
        assert_eq!(by_id("bodyslide").status, Some(OutputStatus::Stale));
    }

    #[test]
    fn a_tool_with_no_output_is_listed_as_not_built() {
        let view = tools_view("sk-1".into(), vec![state("nemesis", None)], &[]);
        assert_eq!(view.tools[0].status, None);
        assert_eq!(view.tools[0].current, None);
        assert!(!view.tools[0].conflicts);
    }

    fn journal(instance_id: &str, tool: &str) -> SwapJournal {
        SwapJournal {
            version: game_tool_swap::JOURNAL_VERSION,
            game: "skyrim-se".into(),
            store: "steam".into(),
            instance_id: instance_id.into(),
            tool: tool.into(),
            run: "run-1".into(),
            started_unix_ms: 0,
            install_dir: PathBuf::from(r"D:\Games\Skyrim"),
            real_data: PathBuf::from(r"D:\Games\Skyrim\Data"),
            aside: PathBuf::from(r"D:\Games\Skyrim\Data.agora-aside-run-1"),
            copy_kind: CopyKind::Kept,
            copy_dir: PathBuf::from(r"D:\Agora\swap\Data"),
            swap_dir: PathBuf::from(r"D:\Agora\swap"),
        }
    }

    fn pending(journal: Option<SwapJournal>, read_error: Option<&str>) -> PendingSwap {
        PendingSwap {
            game: "skyrim-se".into(),
            store: "steam".into(),
            path: PathBuf::from(r"D:\Agora\journal.json"),
            journal,
            read_error: read_error.map(str::to_string),
            running: false,
        }
    }

    #[test]
    fn an_interrupted_swap_shows_for_its_own_instance_only() {
        let pending = vec![pending(Some(journal("sk-1", "bodyslide")), None)];
        let mine = interrupted_for(&pending, "skyrim-se", "sk-1").expect("its own swap shows");
        assert_eq!(mine.tool.as_deref(), Some("bodyslide"));
        assert_eq!(interrupted_for(&pending, "skyrim-se", "sk-2"), None);
        assert_eq!(interrupted_for(&pending, "fallout-4", "sk-1"), None);
    }

    #[test]
    fn an_unreadable_journal_is_reported_for_its_game_not_dropped() {
        let pending = vec![pending(None, Some("bad json"))];
        let shown = interrupted_for(&pending, "skyrim-se", "sk-9").expect("unreadable is shown");
        assert_eq!(shown.tool, None);
        assert_eq!(shown.unreadable.as_deref(), Some("bad json"));
    }
}
