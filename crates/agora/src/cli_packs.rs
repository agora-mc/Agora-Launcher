//! `agora pack export|review|import`, `agora import launchers|plan|run` and
//! `agora backup ...`: thin adapters over the same core services the desktop uses.

use crate::{print_table, wait_for_initial_snapshot, OutputFormat};
use agora_core::agora_pack_import::{PackImportPlan, PlannedSource};
use agora_core::ctx::Ctx;
use agora_core::launcher_import::{CandidateStatus, ImportCandidate, LauncherKind};
use agora_core::launcher_import_service::{
    ImportSelection, LauncherImportPlan, LauncherImportService, PerItemOutcome,
};
use clap::{Args, Subcommand, ValueEnum};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// pack export
// ---------------------------------------------------------------------------

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackExportFormat {
    /// Modrinth modpack (.mrpack); bundles files Modrinth does not host.
    Mrpack,
    /// Agora pack (.agora-pack.json); lists mods only.
    Agora,
    /// Reproduction lockfile (.json).
    Lockfile,
}

#[derive(Args)]
pub struct PackExportArgs {
    /// Instance to export.
    pub instance: String,
    #[arg(long, value_enum, help = "Export format")]
    pub format: PackExportFormat,
    #[arg(long = "out", short = 'o', help = "File to write")]
    pub out: PathBuf,
    #[arg(
        long,
        help = "Export even when some mods would be left out of the pack, without asking"
    )]
    pub yes: bool,
}

#[derive(Args)]
pub struct PackReviewArgs {
    /// A .mrpack or .agora-pack.json file.
    pub file: PathBuf,
}

#[derive(Args)]
pub struct PackImportArgs {
    /// The review id `agora pack review` printed.
    pub review_id: String,
    #[arg(
        long,
        help = "Name for the new instance (default: the pack's own name)"
    )]
    pub name: Option<String>,
    #[arg(
        long,
        help = "Import even though some files cannot be fetched; they are left out"
    )]
    pub import_anyway: bool,
}

fn confirm(question: &str, refusal: &str) -> anyhow::Result<bool> {
    if !std::io::stdin().is_terminal() {
        anyhow::bail!("{refusal}");
    }
    eprint!("{question} [y/N]: ");
    std::io::stderr().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(matches!(
        input.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn require_instance(data_dir: &Path, instance: &str) -> anyhow::Result<PathBuf> {
    let dir = agora_core::paths::instance_dir(data_dir, instance)?;
    if !dir.exists() {
        anyhow::bail!("Instance '{}' not found", instance);
    }
    Ok(dir)
}

pub async fn pack_export(
    args: PackExportArgs,
    data_dir: &Path,
    ctx: &Ctx,
    output_fmt: OutputFormat,
) -> anyhow::Result<()> {
    let json = output_fmt.is_json_output();
    let instance_dir = require_instance(data_dir, &args.instance)?;
    let mut left_out: Vec<String> = Vec::new();
    let format_name = match args.format {
        PackExportFormat::Lockfile => {
            let lockfile = agora_core::lockfile::build_from_instance(&instance_dir)
                .map_err(|e| anyhow::anyhow!("Failed to build lockfile: {e}"))?;
            let text = lockfile
                .to_pretty_json()
                .map_err(|e| anyhow::anyhow!("Failed to serialize lockfile: {e}"))?;
            std::fs::write(&args.out, text)
                .map_err(|e| anyhow::anyhow!("Failed to write lockfile: {e}"))?;
            "lockfile"
        }
        PackExportFormat::Mrpack | PackExportFormat::Agora => {
            let core_format = if args.format == PackExportFormat::Agora {
                "json"
            } else {
                "mrpack"
            };
            left_out = agora_core::export_service::sourceless_mods_of_instance(
                ctx,
                &args.instance,
                core_format,
            )
            .await?;
            if !left_out.is_empty() && !args.yes {
                eprintln!(
                    "These mods have no catalog or Modrinth source, so a friend importing this pack cannot get them:"
                );
                for name in &left_out {
                    eprintln!("  {name}");
                }
                let refusal = format!(
                    "{} mod(s) would be missing for anyone importing this pack. Use --format mrpack to bundle them, or rerun with --yes to export anyway.",
                    left_out.len()
                );
                if !confirm("Export anyway?", &refusal)? {
                    anyhow::bail!("Export cancelled; nothing was written.");
                }
            }
            agora_core::export_service::export_pack_of_instance(
                ctx,
                &args.instance,
                core_format,
                &args.out,
            )
            .await?;
            core_format
        }
    };
    if json {
        println!(
            "{}",
            serde_json::json!({
                "status": "exported",
                "format": format_name,
                "path": args.out.display().to_string(),
                "modsWithoutSource": left_out,
            })
        );
    } else {
        println!("Exported {} to {}", format_name, args.out.display());
        if !left_out.is_empty() {
            println!(
                "Warning: {} mod(s) have no source and are missing from the pack: {}",
                left_out.len(),
                left_out.join(", ")
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// pack review / import
// ---------------------------------------------------------------------------

fn source_word(source: PlannedSource) -> &'static str {
    match source {
        PlannedSource::Curated => "catalog",
        PlannedSource::Modrinth => "modrinth",
        PlannedSource::CannotGet => "unavailable",
        PlannedSource::Download => "download",
        PlannedSource::Bundled => "bundled",
    }
}

fn print_plan(plan: &PackImportPlan) {
    println!("Pack: {}", plan.pack_name);
    println!(
        "  Minecraft {} on {} {}",
        plan.instance.minecraft_version, plan.instance.loader, plan.instance.loader_version
    );
    if plan.name_taken {
        println!(
            "  An instance named '{}' already exists. Suggested name: {}",
            plan.pack_name, plan.suggested_name
        );
    } else {
        println!("  Suggested name: {}", plan.suggested_name);
    }
    let rows: Vec<Vec<String>> = plan
        .entries
        .iter()
        .map(|entry| {
            vec![
                entry
                    .display_name
                    .clone()
                    .unwrap_or_else(|| entry.name.clone()),
                entry.content_type.clone(),
                source_word(entry.source).to_string(),
                entry.reason.clone().unwrap_or_default(),
            ]
        })
        .collect();
    print_table(&["Name", "Type", "Source", "Note"], &rows);
    for (title, lines) in [
        ("Left out", &plan.left_out),
        ("Problems (the import will refuse)", &plan.problems),
        (
            "Files that cannot be fetched (--import-anyway leaves them out)",
            &plan.soft_problems,
        ),
        ("Notes", &plan.notes),
    ] {
        if !lines.is_empty() {
            println!("{title}:");
            for line in lines {
                println!("  - {line}");
            }
        }
    }
}

pub async fn pack_review(
    args: PackReviewArgs,
    ctx: &Ctx,
    output_fmt: OutputFormat,
) -> anyhow::Result<()> {
    let json = output_fmt.is_json_output();
    if !args.file.is_file() {
        anyhow::bail!("Pack file '{}' does not exist", args.file.display());
    }
    let path = args.file.to_string_lossy().into_owned();
    let plan = agora_core::pack_file_import::plan_pack_file(ctx, &path)?;
    if !path.to_ascii_lowercase().ends_with(".mrpack") {
        // A .mrpack is staged by the review itself. Stage the Agora pack the
        // same way so the import runs on exactly the bytes reviewed.
        let (_, staged_sha, _) = agora_core::import_service::ImportService::new(ctx.clone())
            .stage_for_review(&args.file)?;
        if !staged_sha.eq_ignore_ascii_case(&plan.file_sha256) {
            anyhow::bail!("The pack file changed while it was being reviewed. Review it again.");
        }
    }
    if json {
        let mut value = serde_json::to_value(&plan)?;
        value["reviewId"] = serde_json::Value::String(plan.file_sha256.clone());
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        print_plan(&plan);
        println!("Review id: {}", plan.file_sha256);
        if plan.problems.is_empty() {
            println!("Import it with: agora pack import {}", plan.file_sha256);
        } else {
            println!("This pack cannot be imported until the problems above are fixed.");
        }
    }
    Ok(())
}

pub async fn pack_import(
    args: PackImportArgs,
    ctx: &Ctx,
    output_fmt: OutputFormat,
) -> anyhow::Result<()> {
    let json = output_fmt.is_json_output();
    let svc = agora_core::import_service::ImportService::new(ctx.clone());
    let review_id = args.review_id.trim().to_ascii_lowercase();
    let staged = svc.take_reviewed_copy(Some(&review_id))?;
    let staged_path = staged.to_string_lossy().into_owned();
    // Re-plan the staged copy: hard problems always refuse, whatever the flags.
    let plan = agora_core::pack_file_import::plan_pack_file(ctx, &staged_path)?;
    if !plan.problems.is_empty() {
        anyhow::bail!(
            "This pack cannot be imported:\n  - {}",
            plan.problems.join("\n  - ")
        );
    }
    if !plan.soft_problems.is_empty() && !args.import_anyway {
        anyhow::bail!(
            "Some files cannot be fetched:\n  - {}\nRerun with --import-anyway to import without them.",
            plan.soft_problems.join("\n  - ")
        );
    }
    let outcome = agora_core::pack_file_import::import_pack_file(
        ctx,
        &staged_path,
        Some(&review_id),
        args.name.as_deref(),
        args.import_anyway,
        &|_instance_id: &str| ctx.progress_sink.clone(),
    )
    .await
    .map_err(|error| {
        if error.code() == "ERR_INSTANCE_EXISTS" {
            anyhow::anyhow!("{error} Re-run with --name \"<new name>\" to import it as a copy.")
        } else {
            anyhow::Error::from(error)
        }
    })?;
    svc.discard_reviewed_copy(&review_id);
    wait_for_initial_snapshot(ctx, &outcome.instance_id)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&outcome)?);
    } else {
        println!("Imported: {}", outcome.instance_id);
        if let Some(report) = &outcome.report {
            println!(
                "  {} of {} mod(s) installed",
                report.installed, report.total
            );
            for warning in &report.warnings {
                println!("  Note: {warning}");
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// import launchers | plan | run
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum ImportCmd {
    /// Detect Prism, CurseForge and Modrinth instances that can be imported.
    Launchers {
        #[arg(long, help = "Also look in this launcher data folder")]
        root: Option<PathBuf>,
    },
    /// Show what importing launcher instances would do. Writes nothing.
    Plan(LauncherSelectArgs),
    /// Import launcher instances. Items commit independently.
    Run {
        #[command(flatten)]
        select: LauncherSelectArgs,
        #[arg(long, help = "Only run if the plan still has this fingerprint")]
        fingerprint: Option<String>,
    },
}

#[derive(Args)]
pub struct LauncherSelectArgs {
    /// Instance keys from `agora import launchers`, optionally `prism:KEY`,
    /// `curseforge:KEY` or `modrinth:KEY` when the key alone is ambiguous.
    #[arg(required = true)]
    pub sources: Vec<String>,
    #[arg(long, help = "Also look in this launcher data folder")]
    pub root: Option<PathBuf>,
    #[arg(long, help = "Name for the imported instance (one source only)")]
    pub name: Option<String>,
    #[arg(long, help = "Do not copy the source's memory/JVM/Java settings")]
    pub no_settings: bool,
}

fn kind_word(kind: LauncherKind) -> &'static str {
    match kind {
        LauncherKind::Prism => "prism",
        LauncherKind::CurseForge => "curseforge",
        LauncherKind::Modrinth => "modrinth",
    }
}

fn status_word(status: &CandidateStatus) -> String {
    match status {
        CandidateStatus::Ready => "ready".into(),
        CandidateStatus::NeedsReview => "needs review".into(),
        CandidateStatus::Unsupported { reasons } => format!("unsupported: {}", reasons.join("; ")),
    }
}

pub async fn run_import(
    action: ImportCmd,
    ctx: &Ctx,
    output_fmt: OutputFormat,
) -> anyhow::Result<()> {
    let json = output_fmt.is_json_output();
    match action {
        ImportCmd::Launchers { root } => {
            let discovery = LauncherImportService::new(ctx.clone()).discover(root);
            if json {
                println!("{}", serde_json::to_string_pretty(&discovery)?);
                return Ok(());
            }
            let candidates = discovery.all_candidates();
            if candidates.is_empty() {
                println!("No importable launcher instances found.");
                return Ok(());
            }
            let rows: Vec<Vec<String>> = candidates
                .iter()
                .map(|c| {
                    vec![
                        format!("{}:{}", kind_word(c.launcher), c.source_key),
                        c.display_name.clone(),
                        status_word(&c.status),
                    ]
                })
                .collect();
            print_table(&["Key", "Name", "Status"], &rows);
        }
        ImportCmd::Plan(select) => {
            let (_, plan) = build_plan(ctx, &select)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&plan)?);
            } else {
                print_launcher_plan(&plan);
            }
        }
        ImportCmd::Run {
            select,
            fingerprint,
        } => {
            let (service, plan) = build_plan(ctx, &select)?;
            if let Some(expected) = fingerprint {
                if !plan.batch_fingerprint.eq_ignore_ascii_case(expected.trim()) {
                    anyhow::bail!(
                        "The plan changed since it was reviewed (now {}). Run `agora import plan` again.",
                        plan.batch_fingerprint
                    );
                }
            }
            let mut blockers: Vec<String> = plan.batch_blockers.clone();
            for item in &plan.items {
                for blocker in &item.blockers {
                    blockers.push(format!("{}: {blocker}", item.destination_name));
                }
            }
            if !blockers.is_empty() {
                anyhow::bail!("Cannot import:\n  - {}", blockers.join("\n  - "));
            }
            let result = service.execute(plan).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            let mut failed = 0;
            for outcome in &result.outcomes {
                match outcome {
                    PerItemOutcome::Imported { instance_id, .. } if !json => {
                        println!("Imported: {instance_id}")
                    }
                    PerItemOutcome::Updated { instance_id, .. } if !json => {
                        println!("Updated: {instance_id}")
                    }
                    PerItemOutcome::Skipped { reason } if !json => println!("Skipped: {reason}"),
                    PerItemOutcome::Cancelled { reason } if !json => {
                        println!("Cancelled: {reason}")
                    }
                    PerItemOutcome::Failed { error, .. } => {
                        failed += 1;
                        if !json {
                            println!("Failed: {error}");
                        }
                    }
                    _ => {}
                }
            }
            if failed > 0 {
                anyhow::bail!("{failed} item(s) failed to import");
            }
        }
    }
    Ok(())
}

fn find_candidate<'a>(
    candidates: &[&'a ImportCandidate],
    source: &str,
) -> anyhow::Result<&'a ImportCandidate> {
    let (kind, key) = match source.split_once(':') {
        Some((prefix, key))
            if matches!(
                prefix.to_ascii_lowercase().as_str(),
                "prism" | "curseforge" | "modrinth"
            ) =>
        {
            (Some(prefix.to_ascii_lowercase()), key)
        }
        _ => (None, source),
    };
    let matches: Vec<&&ImportCandidate> = candidates
        .iter()
        .filter(|c| {
            c.source_key == key && kind.as_deref().is_none_or(|k| k == kind_word(c.launcher))
        })
        .collect();
    match matches.as_slice() {
        [one] => Ok(**one),
        [] => anyhow::bail!("No launcher instance '{source}'. Run `agora import launchers`."),
        many => anyhow::bail!(
            "'{source}' matches {} launchers; use one of: {}",
            many.len(),
            many.iter()
                .map(|c| format!("{}:{}", kind_word(c.launcher), c.source_key))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn build_plan(
    ctx: &Ctx,
    select: &LauncherSelectArgs,
) -> anyhow::Result<(LauncherImportService, LauncherImportPlan)> {
    if select.name.is_some() && select.sources.len() != 1 {
        anyhow::bail!("--name can only be used with one source");
    }
    let service = LauncherImportService::new(ctx.clone());
    let discovery = service.discover(select.root.clone());
    let candidates = discovery.all_candidates();
    let mut selections = Vec::new();
    for source in &select.sources {
        let candidate = find_candidate(&candidates, source)?;
        selections.push(ImportSelection {
            source_key: candidate.source_key.clone(),
            launcher_kind: candidate.launcher,
            installation_key: candidate.launcher_installation_key.clone(),
            destination_name: select.name.clone(),
            preserve_settings: !select.no_settings,
        });
    }
    let plan = service.plan(selections)?;
    Ok((service, plan))
}

fn print_launcher_plan(plan: &LauncherImportPlan) {
    let rows: Vec<Vec<String>> = plan
        .items
        .iter()
        .map(|item| {
            vec![
                item.destination_name.clone(),
                format!("{:?}", item.action).to_ascii_lowercase(),
                item.total_files.to_string(),
                item.total_bytes.to_string(),
            ]
        })
        .collect();
    print_table(&["Becomes", "Action", "Files", "Bytes"], &rows);
    for item in &plan.items {
        for warning in &item.warnings {
            println!("Warning ({}): {warning}", item.destination_name);
        }
        for blocker in &item.blockers {
            println!("Blocked ({}): {blocker}", item.destination_name);
        }
    }
    for blocker in &plan.batch_blockers {
        println!("Blocked: {blocker}");
    }
    println!("Plan fingerprint: {}", plan.batch_fingerprint);
}

// ---------------------------------------------------------------------------
// backup
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum BackupCmd {
    /// Write a snapshot of an instance to a folder as one self-contained zip.
    Export {
        instance: String,
        #[arg(long = "out", short = 'o', help = "Folder to write the backup into")]
        out: PathBuf,
        #[arg(long, help = "Snapshot to export (default: take a new one now)")]
        snapshot: Option<String>,
    },
    /// Read a backup zip into an instance as a restorable snapshot. The live
    /// instance is not changed; restore it with `agora snapshots restore`.
    Import {
        file: PathBuf,
        #[arg(
            long,
            help = "Instance to add the snapshot to (default: the backup's own)"
        )]
        instance: Option<String>,
    },
    /// Delete old snapshots beyond a retention limit.
    Prune {
        instance: String,
        #[arg(long, default_value_t = 10, help = "Newest snapshots to keep")]
        keep: u32,
        #[arg(long, help = "Delete without asking")]
        yes: bool,
    },
}

pub async fn run_backup(
    action: BackupCmd,
    data_dir: &Path,
    ctx: &Ctx,
    output_fmt: OutputFormat,
) -> anyhow::Result<()> {
    let json = output_fmt.is_json_output();
    match action {
        BackupCmd::Export {
            instance,
            out,
            snapshot,
        } => {
            let instance_dir = require_instance(data_dir, &instance)?;
            let snapshot_id = match snapshot {
                Some(id) => id,
                None => {
                    agora_core::snapshot::create_snapshot_with_origin(
                        &instance_dir,
                        Some("backup export"),
                        agora_core::snapshot::SnapshotOrigin::User,
                    )
                    .map_err(|e| anyhow::anyhow!("{e}"))?
                    .id
                }
            };
            std::fs::create_dir_all(&out)?;
            let path =
                agora_core::backup::export_instance_backup(ctx, &instance, &snapshot_id, &out)
                    .await?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({"status": "exported", "instanceId": instance, "snapshotId": snapshot_id, "path": path})
                );
            } else {
                println!("Backed up '{instance}' (snapshot {snapshot_id}) to {path}");
            }
        }
        BackupCmd::Import { file, instance } => {
            let instance = match instance {
                Some(id) => id,
                None => agora_core::backup::backup_artifact_instance_id(&file)?,
            };
            let snapshot =
                agora_core::backup::import_instance_backup(ctx, &instance, &file).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&snapshot)?);
            } else {
                println!(
                    "Imported backup into '{instance}' as snapshot {}. Restore it with: agora snapshots restore {instance} {}",
                    snapshot.id, snapshot.id
                );
            }
        }
        BackupCmd::Prune {
            instance,
            keep,
            yes,
        } => {
            require_instance(data_dir, &instance)?;
            let policy = agora_core::backup::BackupRetentionPolicy::keep_last(keep);
            let doomed = agora_core::backup::preview_instance_retention(ctx, &instance, &policy)?;
            let removed = if doomed.is_empty() {
                Vec::new()
            } else {
                if !yes {
                    eprintln!("These snapshots would be deleted:");
                    for id in &doomed {
                        eprintln!("  {id}");
                    }
                    let refusal = format!(
                        "Pruning {} snapshot(s) of '{instance}' needs confirmation. Rerun with --yes once the person using this computer has agreed.",
                        doomed.len()
                    );
                    if !confirm("Delete them?", &refusal)? {
                        anyhow::bail!("Prune cancelled; nothing was deleted.");
                    }
                }
                agora_core::backup::apply_instance_retention(ctx, &instance, policy).await?
            };
            if json {
                println!(
                    "{}",
                    serde_json::json!({"status": "pruned", "instanceId": instance, "removed": removed})
                );
            } else {
                println!("Removed {} snapshot(s) from '{instance}'", removed.len());
            }
        }
    }
    Ok(())
}
