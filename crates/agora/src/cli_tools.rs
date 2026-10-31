//! Dependency, template, and instance-maintenance commands.
//!
//! Every handler is a thin adapter over an agora-core service that the desktop
//! app calls too, so the CLI and the app give the same answer. The subcommand
//! enums are flattened into `mod` and `instance` in main.rs; `template` is a
//! top-level command of its own.

use std::io::{IsTerminal, Write};

use agora_core::ctx::Ctx;
use agora_core::dependency_ops::Requirement;
use agora_core::error::LauncherError;
use agora_core::models::InstanceRow;
use agora_core::snapshot_service::SnapshotService;
use agora_game_minecraft::dependency_service::DependencyService;
use agora_game_minecraft::gc::GcSelection;
use agora_game_minecraft::health::{HealthReport, HealthScore};
use agora_game_minecraft::instance_service::{CaptureTemplateRequest, InstanceService};
use agora_game_minecraft::loader_service::LoaderService;
use agora_game_minecraft::template_service::TemplateJvm;
use clap::Subcommand;

use crate::{print_table, OutputFormat};

// ---------------------------------------------------------------------------
// `agora mod graph|orphans|why|disable-plan`
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum ModsToolsCmd {
    /// Show every dependency between installed mods (read-only).
    Graph { instance: String },
    /// List dependency-installed mods that nothing still needs (read-only).
    Orphans { instance: String },
    /// Explain why an installed file is present (read-only).
    Why {
        instance: String,
        /// Installed file name, as shown by `agora mod list`.
        file: String,
    },
    /// Show which installed mods depend on a file, so you can see what disabling
    /// it would affect. Changes nothing.
    #[command(name = "disable-plan")]
    DisablePlan { instance: String, file: String },
}

fn requirement_label(requirement: Requirement) -> &'static str {
    match requirement {
        Requirement::Required => "required",
        Requirement::Optional => "optional",
    }
}

pub fn run_mods_tools(
    action: ModsToolsCmd,
    ctx: &Ctx,
    output_fmt: OutputFormat,
) -> anyhow::Result<()> {
    let json = output_fmt.is_json_output();
    let service = DependencyService::new(ctx.clone());
    match action {
        ModsToolsCmd::Graph { instance } => {
            let edges = service.graph(&instance)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&edges)?);
            } else if edges.is_empty() {
                println!("No dependencies between the installed mods of '{instance}'.");
            } else {
                let rows: Vec<Vec<String>> = edges
                    .iter()
                    .map(|edge| {
                        vec![
                            edge.from_filename.clone(),
                            edge.to_filename.clone(),
                            requirement_label(edge.requirement).to_string(),
                        ]
                    })
                    .collect();
                print_table(&["From", "Needs", "Requirement"], &rows);
            }
        }
        ModsToolsCmd::Orphans { instance } => {
            let orphans = service.orphans(&instance)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&orphans)?);
            } else if orphans.is_empty() {
                println!("No orphaned dependencies in '{instance}'.");
            } else {
                let rows: Vec<Vec<String>> = orphans
                    .iter()
                    .map(|orphan| {
                        vec![
                            orphan.filename.clone(),
                            orphan.mod_jar_id.clone().unwrap_or_default(),
                            orphan.content_type.clone(),
                        ]
                    })
                    .collect();
                print_table(&["Filename", "Mod ID", "Type"], &rows);
            }
        }
        ModsToolsCmd::Why { instance, file } => {
            let Some(explanation) = service.presence(&instance, &file)? else {
                anyhow::bail!(
                    "No installed file named '{file}' in '{instance}'. Run `agora mod list {instance}` to see them."
                );
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&explanation)?);
            } else {
                println!("{file}");
                if explanation.pack_managed {
                    println!("  Comes from the modpack this instance was made from.");
                }
                if explanation.installed_as_dependency {
                    println!("  Installed to satisfy another mod.");
                } else {
                    println!("  Installed directly.");
                }
                if explanation.orphaned {
                    println!("  Nothing still needs it.");
                }
                for path in &explanation.root_paths {
                    println!("  Path: {}", path.join(" -> "));
                }
                if !explanation.dependents.is_empty() {
                    println!("  Declared as a dependency by:");
                    for dependent in &explanation.dependents {
                        println!(
                            "    - {} ({})",
                            dependent.filename,
                            requirement_label(dependent.requirement)
                        );
                    }
                }
            }
        }
        ModsToolsCmd::DisablePlan { instance, file } => {
            let plan = service.disable_plan(&instance, &file)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&plan)?);
            } else if plan.dependents.is_empty() {
                println!("No other installed mod declares a dependency on {file}.");
            } else {
                println!("Installed mods that depend on {file}:");
                for dependent in &plan.dependents {
                    println!(
                        "  - {} ({})",
                        dependent.filename,
                        requirement_label(dependent.requirement)
                    );
                }
                println!(
                    "Nothing was changed. `agora mod disable {instance} {file}` would switch it off."
                );
            }
        }
    }
    Ok(())
}

/// `agora mod enable <instance> <file> --with-deps`: enable the file, then
/// re-enable any of its required dependencies that are switched off.
pub fn enable_with_deps(
    ctx: &Ctx,
    instance: &str,
    file: &str,
    output_fmt: OutputFormat,
) -> anyhow::Result<()> {
    let json = output_fmt.is_json_output();
    let outcome = DependencyService::new(ctx.clone()).enable_with_auto_deps(instance, file)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "status": "enabled",
                "instanceId": instance,
                "file": file,
                "autoEnabled": outcome.enabled_dependencies,
                "failedDependencies": outcome.failed_dependencies,
            }))?
        );
    } else {
        println!("Enabled {file} in '{instance}'");
        for dependency in &outcome.enabled_dependencies {
            println!("  Also enabled required dependency {dependency}");
        }
        // Best-effort, like the desktop app: the target is on, so this is a
        // warning, not a failure. The exit code stays 0.
        for failure in &outcome.failed_dependencies {
            eprintln!(
                "[WARN] Could not enable dependency {}: {}",
                failure.filename, failure.message
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// `agora instance health|revert|loader-version|java|wrapper`
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum InstanceToolsCmd {
    /// Run the health check for one instance, or for every instance with --all.
    /// Exits non-zero when the result is red or an instance cannot be read.
    Health {
        /// Instance ID. Leave out with --all.
        #[arg(required_unless_present = "all")]
        instance: Option<String>,
        #[arg(long, conflicts_with = "instance", help = "Check every instance")]
        all: bool,
    },
    /// Put an unlocked instance back to its lock snapshot.
    Revert { instance: String },
    /// Switch an instance to another pinned loader version. Shows the plan
    /// first and asks; --yes skips the question.
    #[command(name = "loader-version")]
    LoaderVersion {
        instance: String,
        /// Pinned loader version to switch to.
        version: String,
        #[arg(long, help = "Switch without asking")]
        yes: bool,
    },
    /// Show, or change, an instance's Java path, memory, and JVM arguments.
    /// With no options, shows the current values.
    Java {
        instance: String,
        #[arg(
            long,
            conflicts_with = "clear_path",
            help = "Java executable this instance should use"
        )]
        path: Option<String>,
        #[arg(long = "clear-path", help = "Use the global Java setting again")]
        clear_path: bool,
        #[arg(long, help = "Heap size in MB")]
        memory: Option<i64>,
        #[arg(
            long = "jvm-args",
            allow_hyphen_values = true,
            help = "Extra JVM arguments, as one string"
        )]
        jvm_args: Option<String>,
    },
    /// Show, or set, the command the game is launched under. With no command,
    /// shows the current one. An empty command or --clear removes it.
    Wrapper {
        instance: String,
        #[arg(
            allow_hyphen_values = true,
            conflicts_with = "clear",
            help = "Command to launch the game under, e.g. \"mangohud\""
        )]
        command: Option<String>,
        #[arg(long, help = "Remove the wrapper command")]
        clear: bool,
    },
    /// Show, or set, the commands run before and after a direct launch and
    /// the game's extra environment variables. Options replace what is set;
    /// --clear removes all of them.
    Hooks {
        instance: String,
        #[arg(
            long,
            allow_hyphen_values = true,
            help = "Command run before the game starts; empty to remove"
        )]
        pre: Option<String>,
        #[arg(
            long,
            allow_hyphen_values = true,
            help = "Command run after the game exits; empty to remove"
        )]
        post: Option<String>,
        #[arg(
            long = "env",
            value_name = "NAME=VALUE",
            help = "Environment variable for the game; repeat for more"
        )]
        env: Vec<String>,
        #[arg(long, conflicts_with_all = ["pre", "post", "env"], help = "Remove all launch commands and variables")]
        clear: bool,
    },
    /// Show, or set, whether this instance shares its game settings
    /// (options.txt) and server list with other instances that also share.
    Share {
        instance: String,
        #[arg(long, value_name = "on|off")]
        options: Option<String>,
        #[arg(long, value_name = "on|off")]
        servers: Option<String>,
    },
    /// Show, or set, an instance's notes. --clear removes them.
    Notes {
        instance: String,
        #[arg(conflicts_with = "clear", help = "New notes, replacing the old")]
        text: Option<String>,
        #[arg(long)]
        clear: bool,
    },
    /// Show, or set, the group an instance is filed under in My Instances.
    /// --clear ungroups it.
    Group {
        instance: String,
        #[arg(conflicts_with = "clear")]
        name: Option<String>,
        #[arg(long)]
        clear: bool,
    },
    /// Show, or set, the size the game window opens at (direct launch). With
    /// no size, shows the current one; --clear falls back to the global default.
    Window {
        instance: String,
        #[arg(conflicts_with = "clear", help = "WIDTHxHEIGHT, e.g. 1280x720")]
        size: Option<String>,
        #[arg(long, help = "Use the global default instead")]
        clear: bool,
    },
}

/// Parse `WIDTHxHEIGHT`.
fn parse_size(size: &str) -> anyhow::Result<(u32, u32)> {
    let (width, height) = size
        .to_ascii_lowercase()
        .split_once('x')
        .map(|(w, h)| (w.trim().parse::<u32>(), h.trim().parse::<u32>()))
        .ok_or_else(|| anyhow::anyhow!("Give the size as WIDTHxHEIGHT, e.g. 1280x720"))?;
    Ok((width?, height?))
}

/// Ask a yes/no question on the terminal. Without a terminal there is nobody
/// to ask, so the refusal says to pass `--yes` instead.
pub(crate) fn confirm(question: &str, refusal: &str) -> anyhow::Result<bool> {
    if !std::io::stdin().is_terminal() {
        anyhow::bail!("{refusal}");
    }
    eprint!("{question}");
    std::io::stderr().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(matches!(
        input.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn instance_row(service: &InstanceService, instance: &str) -> anyhow::Result<InstanceRow> {
    service
        .get(instance)?
        .map(|detail| detail.row)
        .ok_or_else(|| anyhow::anyhow!("Instance '{instance}' not found"))
}

fn print_health(report: &HealthReport) {
    println!("Health score: {:?}", report.score);
    for warning in &report.warnings {
        println!("  [WARN] {}", warning.message);
    }
    for blocker in &report.blockers {
        println!("  [BLOCK] {}", blocker.message);
    }
    for recommendation in &report.recommendations {
        println!("  [RECOMMEND] {}", recommendation.message);
    }
}

pub async fn run_instance_tools(
    action: InstanceToolsCmd,
    ctx: &Ctx,
    output_fmt: OutputFormat,
) -> anyhow::Result<()> {
    let json = output_fmt.is_json_output();
    let service = InstanceService::new(ctx.clone());
    match action {
        InstanceToolsCmd::Health { instance, all } => {
            if all {
                let results = service.health_scan_all()?;
                let needs_attention: Vec<String> = results
                    .iter()
                    .filter(|result| {
                        result.error.is_some()
                            || result
                                .report
                                .as_ref()
                                .is_some_and(|report| report.score == HealthScore::Red)
                    })
                    .map(|result| result.instance_id.clone())
                    .collect();
                if json {
                    println!("{}", serde_json::to_string_pretty(&results)?);
                } else if results.is_empty() {
                    println!("No instances to check.");
                } else {
                    let rows: Vec<Vec<String>> = results
                        .iter()
                        .map(|result| match &result.report {
                            Some(report) => vec![
                                result.instance_id.clone(),
                                format!("{:?}", report.score),
                                report.blockers.len().to_string(),
                                report.warnings.len().to_string(),
                                String::new(),
                            ],
                            None => vec![
                                result.instance_id.clone(),
                                "unreadable".to_string(),
                                String::new(),
                                String::new(),
                                result
                                    .error
                                    .clone()
                                    .unwrap_or_else(|| "unknown error".to_string()),
                            ],
                        })
                        .collect();
                    print_table(
                        &["Instance", "Health", "Blockers", "Warnings", "Error"],
                        &rows,
                    );
                }
                if !needs_attention.is_empty() {
                    anyhow::bail!(
                        "{} instance(s) need attention: {}",
                        needs_attention.len(),
                        needs_attention.join(", ")
                    );
                }
            } else {
                // clap requires an instance unless --all is given.
                let Some(instance) = instance else {
                    anyhow::bail!("Name an instance, or pass --all.");
                };
                let report = service.health_report(&instance)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&report)?);
                } else {
                    print_health(&report);
                }
                if report.score == HealthScore::Red {
                    anyhow::bail!("Health score is {:?} (see report above)", report.score);
                }
            }
        }
        InstanceToolsCmd::Revert { instance } => {
            let outcome = SnapshotService::new(ctx.clone()).revert_to_lock(&instance)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&outcome)?);
            } else {
                println!(
                    "Reverted '{instance}' to its lock snapshot ({}).",
                    outcome.snapshot_id
                );
                if let Some(warning) = &outcome.cleanup_warning {
                    eprintln!("[WARN] {warning}");
                }
            }
        }
        InstanceToolsCmd::LoaderVersion {
            instance,
            version,
            yes,
        } => {
            let loader = LoaderService::new(ctx.clone());
            let plan = loader.plan_loader_change(&instance)?;
            let compatible: Vec<&str> = plan
                .current_report
                .compatible_versions
                .iter()
                .map(|candidate| candidate.loader_version.as_str())
                .collect();
            if !json {
                println!(
                    "'{instance}' runs {} {} on Minecraft {}.",
                    plan.loader, plan.current_loader_version, plan.minecraft_version
                );
                println!(
                    "Compatible {} versions right now: {}",
                    plan.loader,
                    if compatible.is_empty() {
                        "none".to_string()
                    } else {
                        compatible.join(", ")
                    }
                );
                if let Some(recommended) = &plan.recommended_loader_version {
                    println!("Recommended: {recommended}");
                }
            }
            if !compatible.contains(&version.as_str()) {
                anyhow::bail!(
                    "{} {version} is not compatible with the enabled mods of '{instance}' right now. Pick one of the compatible versions above.",
                    plan.loader
                );
            }
            if !yes {
                let question = format!(
                    "Switch '{instance}' from {} to {version}? [y/N]: ",
                    plan.current_loader_version
                );
                if !confirm(
                    &question,
                    "Changing the loader version needs confirmation. Rerun with --yes once the person using this computer has agreed.",
                )? {
                    println!("Left as it is.");
                    return Ok(());
                }
            }
            let result = loader
                .change_loader_version_with_confirmation(&instance, &version, false)
                .await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!(
                    "Switched '{instance}' from {} to {}.",
                    result.previous_loader_version, result.loader_version
                );
                print_health(&result.health);
            }
        }
        InstanceToolsCmd::Java {
            instance,
            path,
            clear_path,
            memory,
            jvm_args,
        } => {
            if path.as_deref().is_some_and(|p| p.trim().is_empty()) {
                anyhow::bail!("--path needs a file path. Use --clear-path to go back to the global Java setting.");
            }
            if let Some(path) = path.as_deref() {
                if !std::path::Path::new(path).is_file() {
                    anyhow::bail!("No Java executable at '{path}'.");
                }
            }
            let row = instance_row(&service, &instance)?;
            if path.is_some() || clear_path {
                // The incompatible-Java override is kept as it is; this only moves the path.
                service.update_java(
                    &instance,
                    path.as_deref(),
                    row.java_incompatible_override,
                    None,
                )?;
            }
            if memory.is_some() || jvm_args.is_some() {
                let row = instance_row(&service, &instance)?;
                // Setting a heap size makes it a manual choice, as creation does.
                let memory_mb = memory.unwrap_or(row.jvm_memory_mb);
                let memory_mode = if memory.is_some() {
                    "manual"
                } else {
                    row.jvm_memory_mode.as_str()
                };
                let custom_args = jvm_args
                    .clone()
                    .unwrap_or_else(|| row.jvm_custom_args.clone());
                service.update_jvm(
                    &instance,
                    memory_mb,
                    &row.jvm_gc,
                    row.jvm_always_pre_touch,
                    &custom_args,
                    memory_mode,
                )?;
            }
            let row = instance_row(&service, &instance)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "instanceId": row.instance_id,
                        "javaPath": row.java_path,
                        "allowIncompatibleJava": row.java_incompatible_override,
                        "memoryMb": row.jvm_memory_mb,
                        "memoryMode": row.jvm_memory_mode,
                        "gc": row.jvm_gc,
                        "jvmArgs": row.jvm_custom_args,
                    }))?
                );
            } else {
                println!(
                    "Java path: {}",
                    row.java_path.as_deref().unwrap_or("global setting")
                );
                println!(
                    "Allow incompatible Java: {}",
                    if row.java_incompatible_override {
                        "yes"
                    } else {
                        "no"
                    }
                );
                println!("Memory: {} MB ({})", row.jvm_memory_mb, row.jvm_memory_mode);
                println!("GC: {}", row.jvm_gc);
                println!(
                    "JVM args: {}",
                    if row.jvm_custom_args.is_empty() {
                        "none"
                    } else {
                        row.jvm_custom_args.as_str()
                    }
                );
                if let Some(requested) = memory {
                    if requested != row.jvm_memory_mb {
                        eprintln!(
                            "[WARN] Memory was set to {} MB. Agora keeps heap sizes between 2048 and 32768 MB.",
                            row.jvm_memory_mb
                        );
                    }
                }
            }
        }
        InstanceToolsCmd::Wrapper {
            instance,
            command,
            clear,
        } => {
            if clear || command.is_some() {
                let value = if clear {
                    String::new()
                } else {
                    command.unwrap_or_default()
                };
                service.set_wrapper_command(&instance, &value)?;
            }
            let detail = service
                .get(&instance)?
                .ok_or_else(|| anyhow::anyhow!("Instance '{instance}' not found"))?;
            let current = detail
                .manifest
                .as_ref()
                .and_then(|manifest| manifest.user_preferences.get("agora_wrapper_command"))
                .and_then(|value| value.as_str())
                .map(str::to_string);
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "instanceId": instance,
                        "wrapper": current,
                    }))?
                );
            } else {
                match current {
                    Some(command) => println!("Wrapper: {command}"),
                    None => println!("Wrapper: none"),
                }
            }
        }
        InstanceToolsCmd::Hooks {
            instance,
            pre,
            post,
            env,
            clear,
        } => {
            let current = |service: &InstanceService| -> anyhow::Result<agora_game_minecraft::launch_service::LaunchHooks> {
                let detail = service
                    .get(&instance)?
                    .ok_or_else(|| anyhow::anyhow!("Instance '{instance}' not found"))?;
                Ok(detail
                    .manifest
                    .as_ref()
                    .map(agora_game_minecraft::launch_service::LaunchHooks::from_manifest)
                    .unwrap_or_default())
            };
            if clear {
                service.set_launch_hooks(&instance, "", "", Default::default())?;
            } else if pre.is_some() || post.is_some() || !env.is_empty() {
                let existing = current(&service)?;
                let env = if env.is_empty() {
                    existing.env
                } else {
                    env.iter()
                        .map(|pair| {
                            pair.split_once('=')
                                .map(|(name, value)| (name.to_string(), value.to_string()))
                                .ok_or_else(|| {
                                    anyhow::anyhow!("Give variables as NAME=VALUE, not '{pair}'")
                                })
                        })
                        .collect::<anyhow::Result<_>>()?
                };
                service.set_launch_hooks(
                    &instance,
                    &pre.unwrap_or_else(|| existing.pre_launch.unwrap_or_default()),
                    &post.unwrap_or_else(|| existing.post_exit.unwrap_or_default()),
                    env,
                )?;
            }
            let hooks = current(&service)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "instanceId": instance,
                        "preLaunch": hooks.pre_launch,
                        "postExit": hooks.post_exit,
                        "env": hooks.env,
                    }))?
                );
            } else {
                println!(
                    "Pre-launch: {}",
                    hooks.pre_launch.as_deref().unwrap_or("none")
                );
                println!(
                    "Post-exit:  {}",
                    hooks.post_exit.as_deref().unwrap_or("none")
                );
                if hooks.env.is_empty() {
                    println!("Environment: none");
                } else {
                    for (name, value) in &hooks.env {
                        println!("Environment: {name}={value}");
                    }
                }
            }
        }
        InstanceToolsCmd::Share {
            instance,
            options,
            servers,
        } => {
            let switch = |value: &str| match value.to_ascii_lowercase().as_str() {
                "on" | "true" | "yes" => Ok(true),
                "off" | "false" | "no" => Ok(false),
                other => Err(anyhow::anyhow!("Use on or off, not '{other}'")),
            };
            let current = || -> anyhow::Result<(bool, bool)> {
                let detail = service
                    .get(&instance)?
                    .ok_or_else(|| anyhow::anyhow!("Instance '{instance}' not found"))?;
                Ok(detail
                    .manifest
                    .as_ref()
                    .map(agora_core::shared_settings::sharing)
                    .unwrap_or_default())
            };
            if options.is_some() || servers.is_some() {
                let (now_options, now_servers) = current()?;
                let options = options
                    .as_deref()
                    .map(switch)
                    .transpose()?
                    .unwrap_or(now_options);
                let servers = servers
                    .as_deref()
                    .map(switch)
                    .transpose()?
                    .unwrap_or(now_servers);
                service.set_sharing(&instance, options, servers)?;
            }
            let (options, servers) = current()?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "instanceId": instance, "options": options, "servers": servers,
                    }))?
                );
            } else {
                let word = |on: bool| if on { "shared" } else { "not shared" };
                println!("Game settings: {}", word(options));
                println!("Server list:   {}", word(servers));
            }
        }
        InstanceToolsCmd::Notes {
            instance,
            text,
            clear,
        } => {
            if clear {
                service.set_notes(&instance, "")?;
            } else if let Some(text) = text {
                service.set_notes(&instance, &text)?;
            }
            let detail = service
                .get(&instance)?
                .ok_or_else(|| anyhow::anyhow!("Instance '{instance}' not found"))?;
            let notes = detail
                .manifest
                .as_ref()
                .and_then(|manifest| {
                    manifest
                        .user_preferences
                        .get(agora_game_minecraft::instance_service::NOTES_KEY)
                })
                .and_then(|value| value.as_str())
                .map(str::to_string);
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &serde_json::json!({ "instanceId": instance, "notes": notes })
                    )?
                );
            } else {
                println!("{}", notes.as_deref().unwrap_or("No notes."));
            }
        }
        InstanceToolsCmd::Group {
            instance,
            name,
            clear,
        } => {
            if clear {
                service.set_group(&instance, None)?;
            } else if let Some(name) = &name {
                service.set_group(&instance, Some(name))?;
            }
            let group = service.groups()?.get(&instance).cloned();
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &serde_json::json!({ "instanceId": instance, "group": group })
                    )?
                );
            } else {
                println!("Group: {}", group.as_deref().unwrap_or("none"));
            }
        }
        InstanceToolsCmd::Window {
            instance,
            size,
            clear,
        } => {
            if clear {
                service.set_window_size(&instance, None)?;
            } else if let Some(size) = size {
                service.set_window_size(&instance, Some(parse_size(&size)?))?;
            }
            let detail = service
                .get(&instance)?
                .ok_or_else(|| anyhow::anyhow!("Instance '{instance}' not found"))?;
            let current = detail.manifest.as_ref().and_then(|manifest| {
                manifest
                    .user_preferences
                    .get(agora_game_minecraft::launch_service::WINDOW_SIZE_KEY)
                    .and_then(agora_game_minecraft::launch_service::parse_window_size)
            });
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "instanceId": instance,
                        "window": current.map(|(width, height)| serde_json::json!({
                            "width": width,
                            "height": height,
                        })),
                    }))?
                );
            } else {
                match current {
                    Some((width, height)) => println!("Window: {width}x{height}"),
                    None => println!("Window: global default"),
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// `agora template list|create|delete|apply`
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum TemplateCmd {
    /// List saved templates.
    List,
    /// Save a template from an instance's chosen files and/or JVM settings.
    Create {
        /// Display name for the template.
        name: String,
        #[arg(long, help = "What the template is for")]
        description: Option<String>,
        #[arg(long, help = "Instance to copy the chosen files from")]
        from: Option<String>,
        #[arg(
            long = "include",
            requires = "from",
            help = "File to keep, relative to the instance, e.g. config/sodium-options.json (repeatable)"
        )]
        include: Vec<String>,
        #[arg(long = "java-path", help = "Java executable the template sets")]
        java_path: Option<String>,
        #[arg(long, help = "Heap size in MB the template sets")]
        memory: Option<i64>,
        #[arg(
            long,
            help = "GC the template sets: auto, low_latency, high_efficiency, or manual"
        )]
        gc: Option<String>,
        #[arg(
            long = "jvm-args",
            allow_hyphen_values = true,
            help = "Extra JVM arguments the template sets"
        )]
        jvm_args: Option<String>,
    },
    /// Delete a template and everything it captured.
    Delete {
        /// Template ID, from `agora template list`.
        template: String,
        #[arg(long, help = "Delete without asking")]
        yes: bool,
    },
    /// Copy a template's files and JVM settings onto an existing instance.
    /// Agora takes an undo snapshot first.
    Apply {
        /// Template ID, from `agora template list`.
        template: String,
        /// Target instance ID.
        instance: String,
    },
}

fn template_error(message: String) -> anyhow::Error {
    LauncherError::Generic {
        code: "ERR_TEMPLATE".into(),
        message,
    }
    .into()
}

pub fn run_template_command(
    action: TemplateCmd,
    ctx: &Ctx,
    output_fmt: OutputFormat,
) -> anyhow::Result<()> {
    let json = output_fmt.is_json_output();
    match action {
        TemplateCmd::List => {
            let templates =
                agora_game_minecraft::template_service::list_templates(&ctx.paths.templates_root())
                    .map_err(template_error)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&templates)?);
            } else if templates.is_empty() {
                println!("No templates saved.");
            } else {
                let rows: Vec<Vec<String>> = templates
                    .iter()
                    .map(|template| {
                        vec![
                            template.id.clone(),
                            template.name.clone(),
                            template.files.len().to_string(),
                            if template.jvm.is_empty() { "no" } else { "yes" }.to_string(),
                            template.created_at.clone(),
                        ]
                    })
                    .collect();
                print_table(&["ID", "Name", "Files", "JVM", "Created"], &rows);
            }
        }
        TemplateCmd::Create {
            name,
            description,
            from,
            include,
            java_path,
            memory,
            gc,
            jvm_args,
        } => {
            let gc = match gc {
                Some(value) => Some(
                    GcSelection::from_input(&value)
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "Unsupported GC '{}'. Expected auto, low_latency, high_efficiency, or manual.",
                                value.trim()
                            )
                        })?
                        .as_str()
                        .to_string(),
                ),
                None => None,
            };
            let jvm = TemplateJvm {
                java_path,
                jvm_memory_mb: memory,
                jvm_memory_mode: memory.map(|_| "manual".to_string()),
                jvm_gc: gc,
                jvm_custom_args: jvm_args,
                jvm_always_pre_touch: None,
            };
            if from.is_none() && include.is_empty() && jvm.is_empty() {
                anyhow::bail!(
                    "Nothing to capture. Pass --from <instance> with --include <file>, or a JVM setting such as --memory."
                );
            }
            let template =
                InstanceService::new(ctx.clone()).capture_template(CaptureTemplateRequest {
                    name,
                    description,
                    jvm,
                    source_instance_id: from,
                    selected_paths: include,
                })?;
            if json {
                println!("{}", serde_json::to_string_pretty(&template)?);
            } else {
                println!(
                    "Saved template '{}' ({}): {} file(s), JVM settings {}",
                    template.name,
                    template.id,
                    template.files.len(),
                    if template.jvm.is_empty() {
                        "none"
                    } else {
                        "included"
                    }
                );
            }
        }
        TemplateCmd::Delete { template, yes } => {
            if !yes {
                let question =
                    format!("Delete template '{template}' and everything it captured? [y/N]: ");
                if !confirm(
                    &question,
                    "Deleting a template needs confirmation. Rerun with --yes once the person using this computer has agreed.",
                )? {
                    println!("Left as it is.");
                    return Ok(());
                }
            }
            InstanceService::new(ctx.clone()).delete_template(&template)?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({"status": "deleted", "templateId": template})
                );
            } else {
                println!("Deleted template '{template}'.");
            }
        }
        TemplateCmd::Apply { template, instance } => {
            let outcome = InstanceService::new(ctx.clone()).apply_template(
                &instance,
                &ctx.paths.templates_root(),
                &template,
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&outcome)?);
            } else {
                println!(
                    "Applied template '{template}' to '{instance}': {} file(s), JVM settings {}.",
                    outcome.files_applied,
                    if outcome.jvm_applied {
                        "changed"
                    } else {
                        "unchanged"
                    }
                );
                if outcome.files_missing > 0 {
                    eprintln!(
                        "[WARN] {} file(s) listed in the template are missing from it and were skipped.",
                        outcome.files_missing
                    );
                }
                if let Some(snapshot) = &outcome.undo_snapshot_id {
                    println!("Undo point: snapshot {snapshot}");
                }
            }
        }
    }
    Ok(())
}
