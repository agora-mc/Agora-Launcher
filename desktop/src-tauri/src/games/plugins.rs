//! Plugins: an instance's plugin list, its load order, locks and checks (MASTER_SPEC §26.6). Owned
//! by the `plugins` slice. The INI and saves commands (§26.7) are in `ini_saves.rs`.
//!
//! Each command is an adapter over `agora_core::game_load_order` and `game_plugins`: the same
//! functions `agora games instance plugins` calls, so the desktop and the CLI cannot disagree about
//! a plugin's place.
//!
//! A core error reaches the user as its message, which core writes for people already.

use agora_core::ctx::Ctx;
use agora_core::game_load_order::{
    self as load_order, Finding, LoadOrder, LoadOrderEntry, MoveReport, MoveTarget, SortReport,
};
use agora_core::game_plugins;
use serde::Serialize;

use super::error::{GamesError, CORE_FAILED, TASK_FAILED};

pub(super) fn core_failed(error: impl std::fmt::Display) -> GamesError {
    GamesError::new(CORE_FAILED, error.to_string())
}

/// Binds the instance's game definition to `$def`, or returns the reason there is none. A macro
/// because the definition's type lives in `agora_game_api`, which this crate does not depend on:
/// each command binds it here and passes the binding on to core. `ini_saves` uses it too, so every
/// path here is absolute: a macro's items resolve where it is used, not where it is written.
macro_rules! game_definition {
    ($ctx:expr, $instance_id:expr, $def:ident) => {
        let record = agora_core::game_instance::get($ctx, $instance_id)
            .map_err(crate::games::plugins::core_failed)?
            .ok_or_else(|| {
                crate::games::error::GamesError::new(
                    crate::games::error::CORE_FAILED,
                    format!("Instance '{}' was not found.", $instance_id),
                )
            })?;
        let $def = $ctx.games.game(&record.game).ok_or_else(|| {
            crate::games::error::GamesError::new(
                crate::games::error::CORE_FAILED,
                format!(
                    "Agora does not know the game for instance '{}'.",
                    $instance_id
                ),
            )
        })?;
    };
}
pub(super) use game_definition;

/// Runs `work` on a blocking thread with the core context. Every command here reads or writes
/// files, so none of them runs on the async thread.
pub(super) async fn run_blocking<T, F>(
    app: tauri::AppHandle,
    what: &'static str,
    work: F,
) -> Result<T, GamesError>
where
    T: Send + 'static,
    F: FnOnce(Ctx) -> Result<T, GamesError> + Send + 'static,
{
    tokio::task::spawn_blocking(move || -> Result<T, GamesError> {
        let ctx = crate::core_context(&app)?;
        work(ctx)
    })
    .await
    .map_err(|_| GamesError::new(TASK_FAILED, format!("{what} did not finish.")))?
}

// ---------------------------------------------------------------------------
// Plugin list and load order
// ---------------------------------------------------------------------------

/// An instance's plugins in load order, with what the game's rules say about them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginsView {
    /// Whether the game keeps a plugin list at all. When false the tab says so, and there is
    /// nothing for Agora to order.
    pub supported: bool,
    /// Whether the game's load order rules (masters, light and full limits) apply.
    pub rules_apply: bool,
    /// Whether the instance has a copy of the plugin list yet. It is created at deploy time.
    pub list_exists: bool,
    /// Always-loaded plugins first, then the listed ones, in the order the game reads them.
    pub rows: Vec<PluginRow>,
    pub findings: Vec<PluginFinding>,
}

/// One plugin, as the load order shows it. `position` is 1-based and counts always-loaded plugins:
/// the numbering `agora games instance plugins` prints and `move --to` takes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRow {
    pub position: usize,
    pub name: String,
    /// Always loaded by the game's rule. Not in the instance's list, so it cannot be moved or locked.
    pub implicit: bool,
    pub active: bool,
    /// Whether the plugin's file is installed where the game looks for it.
    pub present: bool,
    /// Whether Agora added this line because a content layer deploys it.
    pub managed: bool,
    pub locked: bool,
    /// Whether the plugin's header was read. `master` and `light` mean nothing when it was not.
    pub header_read: bool,
    pub master: bool,
    pub light: bool,
    /// The masters the header names, in the header's order.
    pub masters: Vec<String>,
    pub header_error: Option<String>,
}

/// One problem with the load order, with the plugins it concerns so the row can show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginFinding {
    pub message: String,
    /// Whether a launch refuses to start for this finding. The others are warnings.
    pub refuses_launch: bool,
    /// The plugins the finding is about. Empty for a limit that applies to the whole list.
    pub plugins: Vec<String>,
}

impl PluginsView {
    fn unsupported() -> Self {
        Self {
            supported: false,
            rules_apply: false,
            list_exists: false,
            rows: Vec::new(),
            findings: Vec::new(),
        }
    }

    fn from_order(order: &LoadOrder) -> Self {
        Self {
            supported: true,
            rules_apply: order.engine,
            list_exists: order.exists,
            rows: order
                .entries
                .iter()
                .enumerate()
                .map(|(index, entry)| row_view(index + 1, entry))
                .collect(),
            findings: order.findings.iter().map(finding_view).collect(),
        }
    }
}

fn row_view(position: usize, entry: &LoadOrderEntry) -> PluginRow {
    PluginRow {
        position,
        name: entry.name.clone(),
        implicit: entry.implicit,
        active: entry.active,
        present: entry.present,
        managed: entry.managed,
        locked: entry.locked,
        header_read: entry.header_read,
        master: entry.master,
        light: entry.light,
        masters: entry.masters.clone(),
        header_error: entry.header_error.clone(),
    }
}

fn finding_view(finding: &Finding) -> PluginFinding {
    let plugins = match finding {
        Finding::MasterNotEarlier { plugin, .. }
        | Finding::UnreadableHeader { plugin, .. }
        | Finding::DuplicateListing { plugin } => vec![plugin.clone()],
        Finding::MasterCycle { plugins } => plugins.clone(),
        Finding::TooManyFullPlugins { .. } | Finding::TooManyLightPlugins { .. } => Vec::new(),
    };
    PluginFinding {
        message: finding.message(),
        refuses_launch: finding.refuses_launch(),
        plugins,
    }
}

/// A fresh check. `status` is `findings` when a launch would refuse, `warnings` when there is
/// something to read but nothing that stops the game, and `ok` when there is nothing at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckView {
    pub status: String,
    pub findings: Vec<PluginFinding>,
}

fn check_status(findings: &[PluginFinding]) -> &'static str {
    if findings.iter().any(|f| f.refuses_launch) {
        "findings"
    } else if findings.is_empty() {
        "ok"
    } else {
        "warnings"
    }
}

/// A change to one plugin's state, with the list as it is after the change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginChange {
    /// Whether the line or the lock actually changed. False when it already had that state.
    pub changed: bool,
    pub plugins: PluginsView,
}

/// A move, with the list as it is after the move.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveChange {
    pub report: MoveReport,
    pub plugins: PluginsView,
}

/// A sort, with the list as it is after the sort. A dry run changes nothing, and the report says so.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SortChange {
    pub report: SortReport,
    pub plugins: PluginsView,
}

/// The instance's plugins in load order, with the findings the game's rules give.
#[tauri::command]
pub async fn games_plugins_list(
    app: tauri::AppHandle,
    instance_id: String,
) -> Result<PluginsView, GamesError> {
    run_blocking(app, "Reading the plugin list", move |ctx| {
        game_definition!(&ctx, &instance_id, definition);
        if definition.plugin_list.is_none() {
            return Ok(PluginsView::unsupported());
        }
        let order = load_order::order(&ctx, &instance_id, definition).map_err(core_failed)?;
        Ok(PluginsView::from_order(&order))
    })
    .await
}

/// Turn a plugin on or off in the instance's list. Refused while the game is running.
#[tauri::command]
pub async fn games_plugins_set_active(
    app: tauri::AppHandle,
    instance_id: String,
    plugin: String,
    active: bool,
) -> Result<PluginChange, GamesError> {
    run_blocking(app, "Changing the plugin", move |ctx| {
        game_definition!(&ctx, &instance_id, definition);
        let changed = game_plugins::set_active(&ctx, &instance_id, definition, &plugin, active)
            .map_err(core_failed)?;
        let order = load_order::order(&ctx, &instance_id, definition).map_err(core_failed)?;
        Ok(PluginChange {
            changed,
            plugins: PluginsView::from_order(&order),
        })
    })
    .await
}

/// Lock or unlock a plugin's place, so sort and move leave it where it is.
#[tauri::command]
pub async fn games_plugins_set_locked(
    app: tauri::AppHandle,
    instance_id: String,
    plugin: String,
    locked: bool,
) -> Result<PluginChange, GamesError> {
    run_blocking(app, "Changing the lock", move |ctx| {
        game_definition!(&ctx, &instance_id, definition);
        let changed = game_plugins::set_locked(&ctx, &instance_id, definition, &plugin, locked)
            .map_err(core_failed)?;
        let order = load_order::order(&ctx, &instance_id, definition).map_err(core_failed)?;
        Ok(PluginChange {
            changed,
            plugins: PluginsView::from_order(&order),
        })
    })
    .await
}

/// Move a plugin to a 1-based position that counts always-loaded plugins, as `move --to` does.
#[tauri::command]
pub async fn games_plugins_move(
    app: tauri::AppHandle,
    instance_id: String,
    plugin: String,
    to: usize,
) -> Result<MoveChange, GamesError> {
    run_blocking(app, "Moving the plugin", move |ctx| {
        game_definition!(&ctx, &instance_id, definition);
        let report = load_order::move_plugin(
            &ctx,
            &instance_id,
            definition,
            &plugin,
            &MoveTarget::Position(to),
        )
        .map_err(core_failed)?;
        let order = load_order::order(&ctx, &instance_id, definition).map_err(core_failed)?;
        Ok(MoveChange {
            report,
            plugins: PluginsView::from_order(&order),
        })
    })
    .await
}

/// Sort the list so every master comes before the plugins that need it, by the game's master
/// rules. `dry_run` reports the moves and writes nothing, so the user can read them first.
#[tauri::command]
pub async fn games_plugins_sort(
    app: tauri::AppHandle,
    instance_id: String,
    dry_run: bool,
) -> Result<SortChange, GamesError> {
    run_blocking(app, "Sorting the plugins", move |ctx| {
        game_definition!(&ctx, &instance_id, definition);
        let report =
            load_order::sort(&ctx, &instance_id, definition, dry_run).map_err(core_failed)?;
        let order = load_order::order(&ctx, &instance_id, definition).map_err(core_failed)?;
        Ok(SortChange {
            report,
            plugins: PluginsView::from_order(&order),
        })
    })
    .await
}

/// Check the load order again and report its findings. Read only.
#[tauri::command]
pub async fn games_plugins_check(
    app: tauri::AppHandle,
    instance_id: String,
) -> Result<CheckView, GamesError> {
    run_blocking(app, "Checking the plugins", move |ctx| {
        game_definition!(&ctx, &instance_id, definition);
        if definition.plugin_list.is_none() {
            return Ok(CheckView {
                status: "ok".to_string(),
                findings: Vec::new(),
            });
        }
        let findings: Vec<PluginFinding> = load_order::check(&ctx, &instance_id, definition)
            .map_err(core_failed)?
            .iter()
            .map(finding_view)
            .collect();
        Ok(CheckView {
            status: check_status(&findings).to_string(),
            findings,
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use agora_core::game_load_order::MasterProblem;
    use agora_core::game_plugins::PluginListError;

    fn entry(name: &str) -> LoadOrderEntry {
        LoadOrderEntry {
            name: name.to_string(),
            implicit: false,
            active: true,
            present: true,
            managed: false,
            locked: false,
            header_read: true,
            master: false,
            light: false,
            masters: Vec::new(),
            header_error: None,
        }
    }

    fn order(entries: Vec<LoadOrderEntry>, findings: Vec<Finding>) -> LoadOrder {
        LoadOrder {
            engine: true,
            exists: true,
            entries,
            findings,
        }
    }

    #[test]
    fn rows_are_numbered_from_one_and_keep_the_order_given() {
        let mut skyrim = entry("Skyrim.esm");
        skyrim.master = true;
        let mut implicit = entry("Update.esm");
        implicit.implicit = true;
        let view = PluginsView::from_order(&order(
            vec![implicit, skyrim, entry("SkyUI.esp")],
            Vec::new(),
        ));

        assert!(view.supported && view.rules_apply && view.list_exists);
        let numbered: Vec<(usize, &str)> = view
            .rows
            .iter()
            .map(|r| (r.position, r.name.as_str()))
            .collect();
        assert_eq!(
            numbered,
            vec![(1, "Update.esm"), (2, "Skyrim.esm"), (3, "SkyUI.esp")]
        );
        assert!(view.rows[0].implicit);
        assert!(view.rows[1].master);
    }

    #[test]
    fn a_late_master_warns_and_names_the_plugin_that_needs_it() {
        let finding = Finding::MasterNotEarlier {
            plugin: "SkyUI.esp".to_string(),
            master: "Skyrim.esm".to_string(),
            problem: MasterProblem::Later,
        };
        let view = PluginsView::from_order(&order(vec![entry("SkyUI.esp")], vec![finding]));

        assert_eq!(view.findings.len(), 1);
        assert!(!view.findings[0].refuses_launch);
        assert_eq!(view.findings[0].plugins, vec!["SkyUI.esp".to_string()]);
        assert!(view.findings[0]
            .message
            .contains("loads before its master 'Skyrim.esm'"));
        assert_eq!(check_status(&view.findings), "warnings");
    }

    #[test]
    fn a_missing_master_refuses_and_a_limit_names_no_plugin() {
        let missing = Finding::MasterNotEarlier {
            plugin: "SkyUI.esp".to_string(),
            master: "Skyrim.esm".to_string(),
            problem: MasterProblem::Missing,
        };
        let limit = Finding::TooManyFullPlugins {
            active: 300,
            limit: 254,
        };
        let view = PluginsView::from_order(&order(Vec::new(), vec![missing, limit]));

        assert!(view.findings[0].refuses_launch);
        assert!(view.findings[1].refuses_launch);
        assert!(view.findings[1].plugins.is_empty());
        assert_eq!(check_status(&view.findings), "findings");
    }

    #[test]
    fn a_cycle_names_every_plugin_in_it() {
        let cycle = Finding::MasterCycle {
            plugins: vec!["A.esp".to_string(), "B.esp".to_string()],
        };
        let finding = finding_view(&cycle);
        assert_eq!(
            finding.plugins,
            vec!["A.esp".to_string(), "B.esp".to_string()]
        );
        assert!(finding.refuses_launch);
    }

    #[test]
    fn a_game_without_a_plugin_list_is_unsupported_not_an_empty_success() {
        let view = PluginsView::unsupported();
        assert!(!view.supported);
        assert!(view.rows.is_empty());
        assert!(view.findings.is_empty());
    }

    #[test]
    fn an_empty_check_is_ok() {
        assert_eq!(check_status(&[]), "ok");
    }

    #[test]
    fn a_missing_list_reads_as_no_list_yet_not_as_an_error() {
        let mut no_list = order(Vec::new(), Vec::new());
        no_list.exists = false;
        let view = PluginsView::from_order(&no_list);
        assert!(view.supported);
        assert!(!view.list_exists);
    }

    #[test]
    fn a_core_error_reaches_the_user_as_its_message() {
        let error = core_failed(PluginListError::NotInList("SkyUI.esp".to_string()));
        assert_eq!(error.code, CORE_FAILED);
        assert!(error
            .message
            .contains("'SkyUI.esp' is not in the instance's plugin list"));
        assert_eq!(error.core_code, None);
    }

    #[test]
    fn a_running_game_is_refused_with_the_reason_core_gives() {
        let error = core_failed(PluginListError::InstanceRunning("abc".to_string()));
        assert!(error.message.contains("close the game"));
    }

    /// Read-only check against a real MO2 import. Run by hand:
    /// `cargo test -p agora-desktop --lib real_profile_read_only -- --ignored --nocapture`.
    ///
    /// Runs the same core calls as `games_plugins_list` and `games_plugins_check`, and writes
    /// nothing: no sort, move, lock or set. The data root is opened with `CoreContext::for_testing`
    /// rather than `initialize`, because `initialize` creates the required folders and migrates
    /// `local_state.db`, which would write to the real data root.
    #[test]
    #[ignore = "reads the p3-done data root; run by hand, read only"]
    fn real_profile_read_only() {
        let root = std::path::PathBuf::from(r"D:\Agora-bench\p3-done");
        let instance_id = "grounded-apocalypse-mo2-import-6d97f2";
        let ctx = agora_core::ctx::CoreContext::for_testing(root)
            .with_games(std::sync::Arc::new(crate::build_game_registry().build()));
        let record = agora_core::game_instance::get(&ctx, instance_id)
            .expect("the instance record reads")
            .expect("the instance exists");
        let definition = ctx.games.game(&record.game).expect("the game is known");

        let started = std::time::Instant::now();
        agora_core::game_deploy::plan(
            &ctx,
            instance_id,
            definition,
            agora_core::game_deploy::DeployMode::Links,
        )
        .expect("the deployment plan reads");
        let plan_time = started.elapsed();

        let started = std::time::Instant::now();
        let order = load_order::order(&ctx, instance_id, definition).expect("the load order reads");
        let view = PluginsView::from_order(&order);
        let order_time = started.elapsed();

        let started = std::time::Instant::now();
        let findings: Vec<PluginFinding> = load_order::check(&ctx, instance_id, definition)
            .expect("the check reads")
            .iter()
            .map(finding_view)
            .collect();
        let check_time = started.elapsed();

        let rows = &view.rows;
        let count = |f: fn(&PluginRow) -> bool| rows.iter().filter(|r| f(r)).count();
        println!(
            "supported={} rules_apply={} list_exists={}",
            view.supported, view.rules_apply, view.list_exists
        );
        println!(
            "rows={} implicit={} active={} present={} managed={} locked={} masters={} lights={} header_read={} header_error={}",
            rows.len(),
            count(|r| r.implicit),
            count(|r| r.active),
            count(|r| r.present),
            count(|r| r.managed),
            count(|r| r.locked),
            count(|r| r.master),
            count(|r| r.light),
            count(|r| r.header_read),
            count(|r| r.header_error.is_some()),
        );
        println!(
            "plan: {:?}  order: {:?}  check: {:?}",
            plan_time, order_time, check_time
        );
        println!("check status: {}", check_status(&findings));
        let refusing = findings.iter().filter(|f| f.refuses_launch).count();
        println!(
            "findings: {} total, {} refuse launch",
            findings.len(),
            refusing
        );
        for finding in findings.iter().take(15) {
            println!(
                "  [{}] {} {:?}",
                if finding.refuses_launch {
                    "refuses"
                } else {
                    "warns"
                },
                finding.message,
                finding.plugins
            );
        }
        for row in rows.iter().filter(|r| r.header_error.is_some()).take(10) {
            println!(
                "  header error {}: {}",
                row.name,
                row.header_error.as_deref().unwrap_or("")
            );
        }
        assert!(
            view.supported && view.list_exists,
            "the real profile has a plugin list"
        );
        assert!(!rows.is_empty(), "the real profile lists plugins");
    }
}
