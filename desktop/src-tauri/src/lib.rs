pub mod ai_assistant;
pub mod auth;
pub mod commands;
pub mod crash_diagnostics;
pub mod crash_investigator;
pub mod dependency_ops;
pub use agora_core::{download, error, loader_manifests, models};

pub mod governance;
pub mod instances;
pub mod launcher_profiles;
pub mod mod_install;
pub mod modrinth_raw;
pub mod mojang;
pub use agora_core::override_sanitizer;
pub mod mcp;
pub mod paths;
pub mod registry;
pub mod registry_sync;
pub use agora_core::state;
pub mod technic;
pub mod version_cache;

use state::LauncherState;
use tauri::Manager;

/// Shared type alias for the managed core context.
type ManagedCoreContext = std::sync::Arc<std::sync::Mutex<agora_core::ctx::CoreContext>>;

/// Return a clone of the initialized core context for adapter commands.
pub fn core_context<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> agora_core::error::LauncherResult<agora_core::ctx::Ctx> {
    if let Some(state) = app.try_state::<ManagedCoreContext>() {
        return state.lock().map(|ctx| ctx.clone()).map_err(|_| {
            agora_core::error::LauncherError::Generic {
                code: "ERR_CORE_CONTEXT_LOCK".into(),
                message: "Core context is unavailable.".into(),
            }
        });
    }
    let paths = crate::paths::app_paths(app).map_err(|error| {
        agora_core::error::LauncherError::Generic {
            code: "ERR_APP_PATHS".into(),
            message: error.to_string(),
        }
    })?;
    agora_core::ctx::CoreContext::initialize(paths).map(|(ctx, _)| ctx)
}

/// Pull the instance id out of a `--launch <id>` / `--launch=<id>` argv.
///
/// Sanitized here rather than trusted: argv reaches this from a desktop
/// shortcut or a shell, so it is external input even though it looks internal.
/// An id that does not survive sanitizing is not a real instance and is
/// dropped rather than passed on.
fn launch_arg(args: &[String]) -> Option<String> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let candidate = if let Some(rest) = arg.strip_prefix("--launch=") {
            rest.to_string()
        } else if arg == "--launch" {
            iter.next()?.to_string()
        } else {
            continue;
        };
        let sanitized = agora_core::paths::sanitize_id(&candidate);
        if !sanitized.is_empty() && sanitized == candidate {
            return Some(sanitized);
        }
        return None;
    }
    None
}

/// A `--launch <id>` seen in this process's own argv at startup.
///
/// The single-instance path can emit an event because the frontend is already
/// listening. A cold start cannot: `setup` runs before any listener is
/// attached, so an event there goes nowhere. The id is parked here instead and
/// the frontend collects it once, on mount.
#[derive(Default)]
pub struct PendingCliLaunch(pub std::sync::Mutex<Option<String>>);

/// Run the Tauri application.
pub fn run() {
    // Log startup so the user can verify from the log file that they are
    // actually running the freshly-compiled binary (not a stale one). When
    // diagnosing OAuth issues, the absence of this line means the running
    // app predates the latest cargo build.
    crate::auth::log_line(&format!(
        "AGORA BIN STARTED build_nonce={}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    ));
    // Read this process's own argv before Tauri takes over. A shortcut clicked
    // while Agora is closed lands here; one clicked while it is running lands
    // in the single-instance callback below.
    let pending_cli_launch = PendingCliLaunch(std::sync::Mutex::new(launch_arg(
        &std::env::args().collect::<Vec<_>>(),
    )));

    tauri::Builder::default()
        .manage(LauncherState::default())
        .manage(mcp::McpServerManager::default())
        .manage(pending_cli_launch)
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_sql::Builder::new().build())
        // Update checks are signature-verified against the `pubkey` in
        // tauri.conf.json; an unsigned or wrongly-signed bundle is rejected by
        // the plugin before anything is installed.
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Deliberately no `tauri_plugin_cli`. It requires a `plugins.cli`
        // block in tauri.conf.json and panics at startup without one, and
        // nothing here reads its parse results: `launch_arg` reads argv
        // directly, which is all a single `--launch <id>` flag needs. A
        // registered plugin whose output is never consumed is a second source
        // of truth for the argument list and a startup failure waiting to
        // happen.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // A second launch focuses the existing window instead of starting
            // a duplicate process (which previously could leave orphaned
            // windows such as the Microsoft sign-in webview behind).
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_focus();
            }
            // ...and if it carried `--launch <id>`, hand that to the running
            // app. This is what makes a desktop shortcut work: the shortcut
            // starts a second process, single-instance forwards its argv here,
            // and the already-running window does the launching.
            if let Some(instance_id) = launch_arg(&args) {
                use tauri::Emitter;
                let _ = app.emit("cli-launch", instance_id);
            }
        }))
        .invoke_handler(tauri::generate_handler![
            commands::take_pending_cli_launch,
            commands::browse_items,
            commands::for_you_items,
            commands::get_registry_item,
            commands::list_categories,
            commands::list_pack_mods,
            commands::list_audit_log,
            commands::check_registry_update,
            commands::get_registry_status,
            commands::extract_overrides,
            commands::list_instances,
            commands::get_instance_detail,
            commands::create_instance,
            commands::delete_instance,
            commands::unlock_instance,
            commands::lock_instance,
            commands::rename_instance,
            commands::revert_instance,
            commands::launch_instance,
            commands::launch_instance_direct,
            commands::launch_instance_with_recovery,
            commands::query_launch_state,
            commands::resolve_install_plan,
            commands::apply_install_plan,
            commands::cancel_install,
            commands::check_instance_updates,
            commands::get_cached_instance_updates,
            commands::get_cached_all_updates,
            commands::get_update_changelogs,
            commands::set_mod_update_pinned,
            commands::clear_cached_instance_updates,
            commands::get_lkg_marker,
            commands::export_lockfile,
            commands::import_lockfile,
            commands::detect_drift,
            commands::verify_lockfile,
            commands::repair_lockfile,
            commands::list_snapshots,
            commands::create_snapshot,
            commands::restore_snapshot,
            commands::delete_snapshot,
            commands::list_capturable_template_files,
            commands::list_instance_templates,
            commands::scan_runtime_prune,
            commands::get_migration_report,
            commands::get_launch_history,
            commands::set_instance_wrapper_command,
            commands::create_desktop_shortcut,
            commands::get_shared_screenshot_status,
            commands::link_shared_screenshots,
            commands::unlink_shared_screenshots,
            commands::preview_pack_update,
            commands::apply_pack_update,
            commands::plan_version_migration,
            commands::run_version_migration,
            commands::get_bisect_session,
            commands::start_bisect,
            commands::apply_bisect_trial,
            commands::record_bisect_outcome,
            commands::step_back_bisect,
            commands::cancel_bisect,
            commands::export_backup,
            commands::import_backup,
            commands::apply_backup_retention,
            commands::get_mod_groups,
            commands::set_mod_group,
            commands::rename_mod_group,
            commands::delete_mod_group,
            commands::run_runtime_prune,
            commands::create_instance_template,
            commands::update_instance_template,
            commands::delete_instance_template,
            commands::apply_instance_template,
            commands::list_loadout_profiles,
            commands::create_loadout_profile,
            commands::apply_loadout_profile,
            commands::delete_loadout_profile,
            commands::import_instance,
            commands::cancel_operation,
            commands::detect_launchers,
            commands::clone_instance_cmd,
            commands::check_instance_health,
            commands::check_all_instance_health,
            commands::list_loader_versions,
            commands::plan_loader_change,
            commands::change_loader_version,
            commands::list_manifest_loaders,
            commands::list_manifest_mc_versions,
            commands::get_setting,
            commands::set_setting,
            commands::evaluate_controlify_offer,
            commands::decline_controlify_offer,
            commands::reset_controlify_offer,
            commands::github_login,
            commands::github_login_poll,
            commands::github_logout,
            commands::get_auth_status,
            commands::get_github_profile,
            commands::check_instance_crash,
            commands::triage_crash_report,
            commands::list_crash_reports_cmd,
            commands::read_crash_log_cmd,
            commands::investigate_instance_evidence,
            commands::pick_and_investigate_crash_evidence,
            commands::list_mod_versions,
            commands::list_mod_versions_load_more,
            commands::check_mod_compat,
            commands::batch_check_compat,
            commands::pick_open_file,
            commands::set_custom_instance_icon,
            commands::set_custom_mod_icon,
            commands::get_custom_icon,
            commands::pick_directory,
            commands::discover_launcher_imports,
            commands::plan_launcher_imports,
            commands::execute_launcher_imports,
            commands::explain_crash,
            commands::export_instance_pack,
            commands::import_instance_pack,
            commands::is_modrinth_enabled,
            commands::search_modrinth,
            commands::list_modrinth_categories,
            commands::list_modrinth_loaders,
            commands::list_modrinth_game_versions,
            commands::list_raw_modrinth_versions,
            commands::fetch_modrinth_project,
            technic::technic_search,
            technic::technic_pack_detail,
            technic::install_technic_solder_pack,
            technic::install_technic_zip_pack,
            commands::list_under_review_items,
            commands::list_recent_resolutions,
            commands::list_mod_reviews,
            commands::fetch_triage_poll,
            commands::get_governance_config,
            commands::get_governance_summary,
            commands::get_item_vote,
            commands::set_item_vote,
            commands::list_governance_events,
            commands::run_governance_diagnostics,
            commands::list_instance_content,
            commands::enrich_instance_content,
            commands::investigate_crash,
            commands::investigate_manual,
            commands::disable_mod_for_test,
            commands::enable_mod_for_test,
            commands::disable_instance_mod,
            commands::enable_instance_mod,
            commands::confirm_crash_fix,
            commands::report_still_crashing,
            commands::get_dependency_graph,
            commands::get_orphaned_dependencies,
            commands::explain_mod_presence,
            commands::get_disable_plan,
            commands::get_removal_plan,
            commands::get_install_plan,
            commands::enable_mod_with_auto_deps,
            commands::start_mcp_server,
            commands::stop_mcp_server,
            commands::get_mcp_status,
            commands::get_mcp_token,
            commands::regenerate_mcp_token,
            commands::get_mcp_skill_content,
            commands::set_mcp_approval,
            commands::copilot_login,
            commands::copilot_try_governance_token,
            commands::copilot_login_poll,
            commands::copilot_status,
            commands::copilot_logout,
            commands::ai_chat,
            commands::msa_login,
            commands::msa_get_status,
            commands::msa_refresh,
            commands::msa_logout,
            commands::compute_gc_args,
            commands::browse_search,
            commands::browse_load_more,
            commands::browse_page,
            commands::export_server_environment,
            commands::kill_process,
            commands::install_pack,
            commands::import_modrinth_pack_by_url,
            commands::get_curated_annotation,
            commands::get_windows_accent_color,
            commands::detect_mojang_launcher,
            commands::test_launcher_path,
            commands::repair_instance_loader,
            commands::list_java_runtimes,
            commands::ensure_java_runtime,
            commands::remove_unused_java_runtimes,
            commands::inspect_java_executable,
            commands::update_instance_java,
            commands::update_instance_jvm,
            commands::recommend_instance_memory,
            commands::cancel_java_runtime,
            commands::open_instance_folder,
            commands::open_data_folder,
            commands::reveal_path,
            commands::restart_app,
            commands::open_external_url,
        ])
        .setup(|app| {
            if let Err(error) = crate::paths::migrate_legacy_data_dir(app.handle()) {
                eprintln!("Failed to migrate legacy Agora data directory: {error}");
            }
            // Initialize CoreContext from the shared core-owned data directory.
            // Keep one clone for startup maintenance after the managed state is
            // installed; maintenance is optional and must never delay setup.
            let startup_maintenance_ctx = match crate::paths::app_paths(app.handle()) {
                Ok(paths) => match agora_core::ctx::CoreContext::initialize(paths) {
                    Ok((ctx, warnings)) => {
                        for w in &warnings {
                            eprintln!("[core] {w}");
                        }
                        let maintenance_ctx = ctx.clone();
                        app.manage(ManagedCoreContext::new(std::sync::Mutex::new(ctx)));
                        Some(maintenance_ctx)
                    }
                    Err(e) => {
                        eprintln!("Failed to initialize core context: {e}");
                        None
                    }
                },
                Err(e) => {
                    eprintln!("Failed to resolve app data dir: {e}");
                    None
                }
            };

            let handle = app.handle().clone();
            tauri::async_runtime::block_on(async move {
                // Dev-only: seed registry.db from a local compiler build when
                // running `tauri:dev`. The re-seed path copies an unverified
                // local db+sig pair (acceptable in debug builds, which relax
                // signature checks) â€” must NEVER run in release binaries, where
                // it could overwrite the CI-signed registry from any
                // registry.db found in the cwd parent walk.
                #[cfg(debug_assertions)]
                if let Err(e) = crate::registry_sync::seed_from_local_build(&handle) {
                    eprintln!("Failed to seed registry: {}", e);
                }
                if let Some(ctx) = startup_maintenance_ctx {
                    // Prewarm remains bounded and launch never depends on it (maintenance.rs:1).
                    let prewarm_ctx = ctx.clone();
                    tauri::async_runtime::spawn(async move {
                        match agora_core::maintenance::prewarm_recent_instances(prewarm_ctx).await {
                            Ok(summary) if summary.warmed > 0 => eprintln!(
                                "[core] warmed {} recent instance cache(s) ({} skipped, {} failed)",
                                summary.warmed, summary.skipped, summary.failed
                            ),
                            Ok(_) => {}
                            Err(error) => {
                                eprintln!("[core] startup cache warmup unavailable: {error}")
                            }
                        }
                    });
                    // Bounded background sweep for update caches: refreshes ALL
                    // instances without blocking cold start (task_scheduler /
                    // BlockingPriority::Background). Silent offline via
                    // NetworkPolicy (network.rs), never errors.
                    let sweep_ctx = ctx.clone();
                    tauri::async_runtime::spawn(async move {
                        match agora_core::update_cache::sweep_all_updates(sweep_ctx).await {
                            Ok(summary) if summary.updated > 0 => eprintln!(
                                "[core] update sweep refreshed {} instance(s) ({} skipped, {} failed, offline={})",
                                summary.updated, summary.skipped, summary.failed, summary.offline_skipped
                            ),
                            Ok(summary) if summary.offline_skipped => {
                                eprintln!("[core] update sweep skipped (offline/lockdown)");
                            }
                            Ok(_) => {}
                            Err(error) => {
                                eprintln!("[core] update sweep unavailable: {error}")
                            }
                        }
                    });
                }
                let purge_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let _ = tokio::task::spawn_blocking(move || {
                        if let Some(core_state) = purge_handle.try_state::<ManagedCoreContext>() {
                            match core_state.lock() {
                                Ok(ctx) => {
                                    let svc =
                                        agora_core::crash_service::CrashService::new(ctx.clone());
                                    if let Err(e) = svc.purge_stale_telemetry() {
                                        eprintln!("Failed to purge stale crash telemetry: {e}",);
                                    }
                                }
                                Err(e) => {
                                    eprintln!("Core context lock failed: {e}");
                                }
                            }
                        }
                    })
                    .await;
                });
                // Start MCP server if enabled.
                if let Some(core_state) = handle.try_state::<ManagedCoreContext>() {
                    match core_state.lock() {
                        Ok(ctx) => {
                            let svc = agora_core::settings::SettingsService::new(ctx.clone());
                            match svc.get_bool("ai_mcp_enabled") {
                                Ok(true) => {
                                    let mcp_app = handle.clone();
                                    tauri::async_runtime::spawn(async move {
                                        let manager =
                                            mcp_app.state::<crate::mcp::McpServerManager>();
                                        if let Err(e) = manager.start(mcp_app.clone()).await {
                                            eprintln!("Failed to start MCP server: {e}",);
                                        }
                                    });
                                }
                                Ok(false) => {}
                                Err(e) => {
                                    eprintln!("Failed to read MCP setting: {e}");
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Core context lock failed for MCP startup: {e}");
                        }
                    }
                } else {
                    eprintln!("Core context not available, skipping MCP auto-start");
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                if let Some(manager) = window.try_state::<crate::mcp::McpServerManager>() {
                    manager.request_shutdown();
                }
                // Closing the main window must also close a pending Microsoft
                // sign-in webview; otherwise the app stays alive with an
                // orphaned login window and a relaunch starts a second process.
                if window.label() == "main" {
                    if let Some(msa_window) = window.get_webview_window("msa-login") {
                        let _ = msa_window.destroy();
                    }
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
