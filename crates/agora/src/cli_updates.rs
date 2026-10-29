//! `agora instance updates`, `agora mod pin|unpin|changelog`, and
//! `agora pack update-preview|update`.
//!
//! Each handler is an adapter. The update check and its cache, the batch plan,
//! the registry changelogs and the pack merge are `agora-core` services that the
//! desktop app calls too, so nothing here decides what counts as an update.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use agora_core::ctx::Ctx;
use agora_core::models::{InstalledMod, InstanceManifest};
use agora_core::providers::ProviderRegistry;
use agora_core::registry::RegistryService;
use agora_game_minecraft::install_pipeline::{
    BatchUpdateItem, CancellationToken, InstallAction, InstallIntent, InstallOutcome,
    PlanOverrides, RequestSource,
};
use agora_game_minecraft::install_service::InstallService;
use agora_game_minecraft::pack_merge::PlanActionKind;
use agora_game_minecraft::pack_update::{ConflictResolution, PackUpdateOutcome, PackUpdatePreview};
use agora_game_minecraft::update_cache::{ItemErrorPolicy, UpdateCheckOptions, UpdateInfo};

use crate::{print_plan, print_table, report_unresolved_plan, SilentReporter};

/// The content providers the desktop checks against: Agora's official ones plus
/// any enabled plugin's. Refreshes the plugin list first, as the desktop does.
fn provider_registry(ctx: &Ctx) -> anyhow::Result<ProviderRegistry> {
    let service = crate::plugin_service(ctx);
    service.reload()?;
    let plugins = service.is_enabled().then_some(&service);
    Ok(ProviderRegistry::new(ctx, plugins))
}

/// Every installed entry, in the same lists the core update check reads.
fn installed_entries(manifest: &InstanceManifest) -> impl Iterator<Item = &InstalledMod> {
    manifest
        .mods
        .iter()
        .chain(&manifest.resourcepacks)
        .chain(&manifest.shaders)
        .chain(&manifest.datapacks)
        .chain(&manifest.worlds)
}

fn find_installed<'a>(
    manifest: &'a InstanceManifest,
    instance: &str,
    file: &str,
) -> anyhow::Result<&'a InstalledMod> {
    installed_entries(manifest)
        .find(|entry| entry.filename == file)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "'{file}' is not installed in '{instance}'. Check the file name with `agora mod list {instance}`."
            )
        })
}

// ---------------------------------------------------------------------------
// agora instance updates
// ---------------------------------------------------------------------------

/// Check every installed item for a newer version. With `apply`, plan the
/// applicable ones as one batch and run it only with `yes`.
pub async fn instance_updates(
    ctx: &Ctx,
    instance: &str,
    apply: bool,
    yes: bool,
    json: bool,
) -> anyhow::Result<()> {
    let providers = provider_registry(ctx)?;
    let service = InstallService::new(ctx.clone()).with_providers(providers.clone());
    // Refuse an unknown instance here, before any network work.
    service.load_instance(instance)?;

    let updates = agora_game_minecraft::update_cache::check_and_cache_instance_updates(
        ctx,
        instance,
        UpdateCheckOptions {
            memory_cache: None,
            on_item_error: ItemErrorPolicy::Fail,
            providers: Some(&providers),
        },
    )
    .await?;

    if !apply {
        if json {
            println!("{}", serde_json::to_string_pretty(&updates)?);
        } else {
            print_update_rows(instance, &updates);
        }
        return Ok(());
    }
    apply_batch(&service, instance, &updates, yes, json).await
}

fn update_note(update: &UpdateInfo) -> String {
    if let Some(minecraft) = &update.needs_minecraft {
        format!("newer build is for Minecraft {minecraft}; not applied")
    } else if update.uncertain {
        "release date unknown; not applied".to_string()
    } else {
        String::new()
    }
}

fn print_update_rows(instance: &str, updates: &[UpdateInfo]) {
    if updates.is_empty() {
        println!("No updates are available for '{instance}'.");
        return;
    }
    let rows: Vec<Vec<String>> = updates
        .iter()
        .map(|u| {
            vec![
                u.filename.clone(),
                u.current_version.clone(),
                u.latest_version.clone(),
                u.target_version.clone(),
                u.source.clone(),
                update_note(u),
            ]
        })
        .collect();
    print_table(
        &["Filename", "Current", "Latest", "Target", "Source", "Note"],
        &rows,
    );
}

/// Plan the applicable updates as one batch, exactly as the desktop's Update
/// All does, then run it once `yes` is given. Rows that are notes or uncertain
/// are never part of a batch.
async fn apply_batch(
    service: &InstallService,
    instance: &str,
    updates: &[UpdateInfo],
    yes: bool,
    json: bool,
) -> anyhow::Result<()> {
    let applicable: Vec<&UpdateInfo> = updates.iter().filter(|u| u.is_applicable()).collect();
    if applicable.is_empty() {
        if json {
            println!(
                "{}",
                serde_json::json!({"status": "up_to_date", "instanceId": instance})
            );
        } else {
            println!("No updates to apply to '{instance}'.");
        }
        return Ok(());
    }
    if let Some(unidentified) = applicable.iter().find(|u| u.mod_jar_id.trim().is_empty()) {
        anyhow::bail!(
            "Refusing to apply: '{}' has no item ID to update by. Nothing was changed.",
            unidentified.filename
        );
    }

    let intent = InstallIntent {
        action: InstallAction::BatchUpdate {
            items: applicable
                .iter()
                .map(|u| BatchUpdateItem {
                    item_id: u.mod_jar_id.clone(),
                    target_version: u.target_version.clone(),
                })
                .collect(),
        },
        target_instance: instance.to_string(),
        optional_deps: crate::resolve_optional_deps(None, false),
        requested_by: RequestSource::CLI,
        overrides: PlanOverrides {
            skip_health_scan: false,
            ..Default::default()
        },
    };

    let reporter = SilentReporter;
    let plan = service.resolve(intent, &reporter).await?;

    // Fail closed on an unresolved plan, as `mod update-all` does.
    if !plan.is_fully_resolved() {
        report_unresolved_plan(&plan, json);
        let has_choices = !plan.pending_choices.is_empty()
            || plan
                .conflicts
                .iter()
                .any(|c| c.blocking && c.chosen.is_none());
        if has_choices {
            return Err(agora_core::error::LauncherError::UserDecisionRequired.into());
        }
        anyhow::bail!("Update blocked: unresolved errors or conflicts. Nothing was changed.");
    }

    if !yes {
        print_plan(&plan, json)?;
        anyhow::bail!(
            "Nothing was changed. Rerun with --yes to apply {} update(s) to '{instance}' as this plan.",
            applicable.len()
        );
    }

    let cancel = CancellationToken::new();
    let outcome = service.execute(&plan, &reporter, &cancel).await;
    report_batch_outcome(outcome, json)
}

fn report_batch_outcome(outcome: InstallOutcome, json: bool) -> anyhow::Result<()> {
    match outcome {
        InstallOutcome::Success {
            warnings,
            snapshot_id,
            installed_items,
            ..
        } => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "status": "success",
                        "snapshotId": snapshot_id,
                        "installedItems": installed_items,
                        "warnings": warnings,
                    }))?
                );
            } else {
                println!(
                    "Updated {} item(s). Snapshot {} can undo this.",
                    installed_items.len(),
                    snapshot_id
                );
            }
            Ok(())
        }
        InstallOutcome::HealthRollback {
            health_report,
            snapshot_id,
            ..
        } => {
            for blocker in &health_report.blockers {
                eprintln!("  [BLOCK] {}", blocker.message);
            }
            anyhow::bail!(
                "Update has {} health blocker(s) (update kept; snapshot {} available for rollback)",
                health_report.blockers.len(),
                snapshot_id
            )
        }
        InstallOutcome::Cancelled { phase, .. } => {
            anyhow::bail!("Update was cancelled during {phase}.")
        }
        InstallOutcome::Failed {
            error,
            rollback_performed,
            ..
        } => {
            if json {
                eprintln!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "status": "failed",
                        "error": error,
                        "rollbackPerformed": rollback_performed,
                    }))?
                );
            }
            let rolled_back = if rollback_performed {
                " and was rolled back"
            } else {
                ""
            };
            anyhow::bail!("Update failed{rolled_back}: {error}")
        }
    }
}

// ---------------------------------------------------------------------------
// agora mod pin | unpin | changelog
// ---------------------------------------------------------------------------

/// Pin or unpin one installed file against update checks.
pub fn set_mod_pin(
    ctx: &Ctx,
    instance: &str,
    file: &str,
    pinned: bool,
    json: bool,
) -> anyhow::Result<()> {
    let service = InstallService::new(ctx.clone());
    let load = service.load_instance(instance)?;
    find_installed(&load.manifest, instance, file)?;
    service.check_not_locked(instance)?;

    let changed = service.set_update_pinned(instance, file, pinned)?;
    if json {
        println!(
            "{}",
            serde_json::json!({
                "instanceId": instance,
                "file": file,
                "pinned": pinned,
                "changed": changed,
            })
        );
    } else if !changed {
        let state = if pinned { "pinned" } else { "unpinned" };
        println!("{file} is already {state}.");
    } else if pinned {
        println!("Pinned {file}: update checks skip it until you unpin it.");
    } else {
        println!("Unpinned {file}: updates will be offered again.");
    }
    Ok(())
}

/// Print the changelog between an installed file and its pending update.
pub async fn mod_changelog(
    ctx: &Ctx,
    instance: &str,
    file: &str,
    json: bool,
) -> anyhow::Result<()> {
    let providers = provider_registry(ctx)?;
    let service = InstallService::new(ctx.clone()).with_providers(providers.clone());
    let load = service.load_instance(instance)?;
    let installed = find_installed(&load.manifest, instance, file)?;
    if installed.update_pinned {
        anyhow::bail!("{file} is pinned, so update checks skip it. Unpin it to see its changelog.");
    }

    let updates = agora_game_minecraft::update_cache::check_single_instance_updates_with(
        ctx,
        instance,
        UpdateCheckOptions {
            memory_cache: None,
            on_item_error: ItemErrorPolicy::Fail,
            providers: Some(&providers),
        },
    )
    .await?;

    let Some(update) = updates
        .iter()
        .find(|u| u.filename == file && u.is_applicable())
    else {
        if json {
            println!("[]");
        } else {
            println!("{file} has no newer version, so there is no changelog to show.");
        }
        return Ok(());
    };

    let registry = RegistryService::new(ctx.clone());
    let changelogs = registry.get_changelogs_between(
        &update.mod_jar_id,
        &update.current_version,
        &update.latest_version,
    )?;

    if json {
        println!("{}", serde_json::to_string_pretty(&changelogs)?);
        return Ok(());
    }
    if changelogs.is_empty() {
        println!(
            "No changelog is stored for {file} from {} to {}. Its source may publish none.",
            update.current_version, update.latest_version
        );
        return Ok(());
    }
    for entry in &changelogs {
        match &entry.published_at {
            Some(date) => println!("== {} ({date}) ==", entry.version),
            None => println!("== {} ==", entry.version),
        }
        println!("{}\n", entry.changelog.trim_end());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// agora pack update-preview | update
// ---------------------------------------------------------------------------

/// Load the instance and build the offline preview of an .mrpack update.
fn load_pack_preview(
    ctx: &Ctx,
    instance: &str,
    mrpack: &Path,
) -> anyhow::Result<(PathBuf, PackUpdatePreview)> {
    let load = InstallService::new(ctx.clone()).load_instance(instance)?;
    let conn = agora_core::db::local_state_connection(&ctx.paths.local_state_db())?;
    let sanitized = agora_core::paths::sanitize_id(instance);
    let preview = agora_game_minecraft::pack_update::preview_pack_update(
        &conn,
        &sanitized,
        &load.instance_dir,
        mrpack,
    )
    .map_err(anyhow::Error::msg)?;
    Ok((load.instance_dir, preview))
}

fn print_pack_preview(instance: &str, preview: &PackUpdatePreview) {
    match &preview.pack_version_id {
        Some(version) => println!(
            "Pack update for '{instance}': {} ({version})",
            preview.pack_name
        ),
        None => println!("Pack update for '{instance}': {}", preview.pack_name),
    }
    if preview.plan.baseline_missing {
        println!(
            "Warning: this instance has no record of what its pack installed, so every edited file shows as a conflict."
        );
    }

    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for action in &preview.plan.actions {
        if !matches!(action.kind, PlanActionKind::Keep) {
            *counts.entry(action.kind.as_str()).or_default() += 1;
        }
    }
    if counts.is_empty() {
        println!("No file changes.");
    } else {
        println!("Changes:");
        for (kind, count) in &counts {
            println!("  {kind}: {count}");
        }
    }

    println!(
        "Downloads: {} file(s), {} bytes{}",
        preview.files_needing_download,
        preview.download_bytes,
        if preview.size_unknown_count > 0 {
            " (some sizes unknown)"
        } else {
            ""
        }
    );
    if preview.content_uncertain() > 0 {
        println!(
            "Estimated: {} mod jar(s) are not fetched yet, so their content is checked when you apply.",
            preview.content_uncertain()
        );
    }

    if preview.plan.conflicts.is_empty() {
        println!("Conflicts: none.");
    } else {
        println!("Conflicts (answer each with --keep-ours <KEY> or --take-theirs <KEY>):");
        for conflict in &preview.plan.conflicts {
            println!("  {}: {}", conflict.key, conflict.message);
        }
    }
}

/// Preview what a newer .mrpack would change. Changes nothing.
pub fn pack_update_preview(
    ctx: &Ctx,
    instance: &str,
    mrpack: &Path,
    json: bool,
) -> anyhow::Result<()> {
    let (_, preview) = load_pack_preview(ctx, instance, mrpack)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&preview)?);
    } else {
        print_pack_preview(instance, &preview);
    }
    Ok(())
}

/// Turn the `--keep-ours` / `--take-theirs` answers into one resolution per
/// conflict. Every conflict must be answered, and an answer must name a real
/// conflict, so a typo cannot silently leave a file for the merge to decide.
fn pack_resolutions(
    preview: &PackUpdatePreview,
    keep_ours: &[String],
    take_theirs: &[String],
) -> anyhow::Result<BTreeMap<String, ConflictResolution>> {
    let answers = keep_ours
        .iter()
        .map(|key| (key, ConflictResolution::KeepOurs))
        .chain(
            take_theirs
                .iter()
                .map(|key| (key, ConflictResolution::TakeTheirs)),
        );
    let mut resolutions = BTreeMap::new();
    for (key, choice) in answers {
        if key.trim().is_empty() {
            anyhow::bail!("A conflict key cannot be empty.");
        }
        if resolutions.insert(key.clone(), choice).is_some() {
            anyhow::bail!(
                "'{key}' was answered more than once. Give each conflict one of --keep-ours or --take-theirs."
            );
        }
    }

    let conflict_keys: BTreeSet<&str> = preview
        .plan
        .conflicts
        .iter()
        .map(|conflict| conflict.key.as_str())
        .collect();
    if let Some(unknown) = resolutions
        .keys()
        .find(|key| !conflict_keys.contains(key.as_str()))
    {
        anyhow::bail!(
            "'{unknown}' is not a conflict in this pack update. Check the keys with `agora pack update-preview`."
        );
    }

    let unanswered: Vec<&str> = conflict_keys
        .iter()
        .copied()
        .filter(|key| !resolutions.contains_key(*key))
        .collect();
    if !unanswered.is_empty() {
        anyhow::bail!(
            "{} conflict(s) need an answer before anything is changed: {}. Answer each with --keep-ours <KEY> or --take-theirs <KEY>.",
            unanswered.len(),
            unanswered.join(", ")
        );
    }
    Ok(resolutions)
}

/// Apply a newer .mrpack. Refuses unless every conflict is answered and `yes`
/// is given.
pub async fn pack_update(
    ctx: &Ctx,
    instance: &str,
    mrpack: &Path,
    keep_ours: &[String],
    take_theirs: &[String],
    yes: bool,
    json: bool,
) -> anyhow::Result<()> {
    let (instance_dir, preview) = load_pack_preview(ctx, instance, mrpack)?;
    let resolutions = pack_resolutions(&preview, keep_ours, take_theirs)?;

    if !yes {
        if json {
            println!("{}", serde_json::to_string_pretty(&preview)?);
        } else {
            print_pack_preview(instance, &preview);
        }
        anyhow::bail!(
            "Nothing was changed. Rerun with --yes to apply this pack update to '{instance}'."
        );
    }

    // The merge hashes and downloads jars with blocking HTTP, so it must not run
    // on the async runtime's thread.
    let task_ctx = ctx.clone();
    let instance_id = instance.to_string();
    let mrpack = mrpack.to_path_buf();
    let outcome = tokio::task::spawn_blocking(move || {
        agora_game_minecraft::pack_update::apply_pack_update(
            &task_ctx,
            &instance_id,
            &instance_dir,
            &mrpack,
            &resolutions,
        )
    })
    .await
    .map_err(|e| anyhow::anyhow!("Pack update task failed: {e}"))?
    .map_err(anyhow::Error::msg)?;
    report_pack_outcome(instance, outcome, json)
}

fn report_pack_outcome(
    instance: &str,
    outcome: PackUpdateOutcome,
    json: bool,
) -> anyhow::Result<()> {
    match outcome {
        PackUpdateOutcome::Updated {
            snapshot_id,
            changed,
            kept,
            health,
        } => {
            let blockers = health.as_ref().map_or(0, |report| report.blockers.len());
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "status": "updated",
                        "instanceId": instance,
                        "snapshotId": snapshot_id,
                        "changed": changed,
                        "kept": kept,
                        "healthBlockers": blockers,
                    }))?
                );
            } else {
                println!(
                    "Updated '{instance}' to the new pack: {changed} file(s) changed, {kept} kept as they were. Snapshot {snapshot_id} can undo this."
                );
                if blockers > 0 {
                    println!("The health check reports {blockers} blocker(s); run `agora health {instance}`.");
                }
            }
            Ok(())
        }
        PackUpdateOutcome::HealthBlocked {
            snapshot_id,
            health,
        } => {
            for blocker in &health.blockers {
                eprintln!("  [BLOCK] {}", blocker.message);
            }
            anyhow::bail!(
                "Pack update kept with {} health blocker(s); snapshot {} can undo it.",
                health.blockers.len(),
                snapshot_id
            )
        }
        PackUpdateOutcome::Failed {
            phase,
            error,
            rolled_back,
            ..
        } => {
            let rolled_back = if rolled_back {
                " (restored to the snapshot)"
            } else {
                ""
            };
            anyhow::bail!("Pack update failed during {phase}{rolled_back}: {error}")
        }
    }
}
