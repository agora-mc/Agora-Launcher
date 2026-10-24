//! Read-only governance views, registry review queries, Technic and provider
//! pack installs, and plugin settings for the `agora` CLI.
//!
//! Each command here is an adapter. The work lives in `agora-core`
//! (`governance`, `registry`, `technic`, `providers`, `plugins`). Nothing in
//! this module votes, and nothing posts to GitHub.

use agora_core::ctx::Ctx;
use agora_core::governance::{self, DiagnosticStatus, GovernanceService};
use agora_core::import::{ImportResult, TechnicZipPack};
use agora_core::plugins::PluginService;
use agora_core::providers::{self, ProviderRegistry};
use agora_core::registry::RegistryService;
use agora_core::technic::{self, TechnicTier};
use anyhow::{bail, Context};
use clap::Subcommand;

use crate::{print_table, OutputFormat};

#[derive(Subcommand)]
pub enum GovernanceCmd {
    /// Show the vote counts and status for one registry item.
    Summary {
        /// Registry item ID.
        item: String,
    },
    /// List recent governance events, newest first.
    Events {
        #[arg(long, help = "Only events for this registry item")]
        item: Option<String>,
        #[arg(
            long,
            default_value_t = 100,
            value_parser = clap::value_parser!(i64).range(1..=1000),
            help = "Maximum events to list (1-1000)"
        )]
        limit: i64,
    },
    /// Show the governance repository and environment this build uses.
    Config,
    /// Check the governance setup: registry tables, flat files, and the GitHub repository. Read-only.
    Diagnostics,
}

#[derive(Subcommand)]
pub enum TechnicCmd {
    /// Search Technic packs. Each result shows its consent tier (solder or zip).
    Search {
        query: String,
        #[arg(
            long,
            default_value_t = 20,
            value_parser = clap::value_parser!(u32).range(1..=100),
            help = "Maximum results (1-100)"
        )]
        limit: u32,
    },
    /// Install a Technic pack as a new instance.
    ///
    /// Solder packs take `--build` (default: the pack's recommended build).
    /// Zip packs have no metadata, so they need `--minecraft`, `--loader` and
    /// `--loader-version`. Pass `--sha256` to pin a zip's download to a
    /// published hash; without it the zip is installed unverified and the CLI
    /// says so. Consent settings are enforced by core.
    Install {
        /// Technic pack slug, as shown by `technic search`.
        slug: String,
        #[arg(long, help = "Solder build to install (Solder packs only)")]
        build: Option<String>,
        #[arg(long, help = "Name for the new instance")]
        name: Option<String>,
        #[arg(
            long,
            value_parser = parse_sha256_arg,
            help = "SHA-256 of the zip download, 64 hex digits (zip packs only)"
        )]
        sha256: Option<String>,
        #[arg(long, help = "Minecraft version (zip packs only)")]
        minecraft: Option<String>,
        #[arg(long, help = "Mod loader (zip packs only)")]
        loader: Option<String>,
        #[arg(long, help = "Mod loader version (zip packs only)")]
        loader_version: Option<String>,
    },
}

/// `agora governance ...`: read-only views over the compiled governance state.
pub async fn run_governance_command(
    action: GovernanceCmd,
    ctx: &Ctx,
    output_fmt: OutputFormat,
) -> anyhow::Result<()> {
    let json = output_fmt.is_json_output();
    let service = GovernanceService::new(RegistryService::new(ctx.clone()));
    match action {
        GovernanceCmd::Summary { item } => {
            let summary = service.get_governance_summary(&item)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&summary)?);
            } else {
                match summary {
                    Some(s) => {
                        println!("Item:               {}", s.item_id);
                        println!(
                            "Vote issue:         {}",
                            s.vote_issue_url.as_deref().unwrap_or("none")
                        );
                        println!(
                            "Raw votes:          {} up, {} down",
                            s.raw_upvotes, s.raw_downvotes
                        );
                        println!(
                            "Counted votes:      {} up, {} down",
                            s.counted_upvotes, s.counted_downvotes
                        );
                        println!(
                            "Quarantined votes:  {} up, {} down",
                            s.quarantined_upvotes, s.quarantined_downvotes
                        );
                        println!("Conflicted users:   {}", s.conflicted_users);
                        println!(
                            "Status reason:      {}",
                            s.status_reason.as_deref().unwrap_or("none")
                        );
                        println!("Compiled at:        {}", s.compiled_at);
                    }
                    None => println!("No governance summary for '{item}'."),
                }
            }
        }
        GovernanceCmd::Events { item, limit } => {
            let events = service.list_governance_events(item.as_deref(), limit)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&events)?);
            } else if events.is_empty() {
                println!("No governance events.");
            } else {
                let rows: Vec<Vec<String>> = events
                    .iter()
                    .map(|event| {
                        vec![
                            event.detected_at.clone(),
                            event.item_id.clone(),
                            event.event_type.clone(),
                            event.status.clone(),
                            event.affected_reactions.to_string(),
                        ]
                    })
                    .collect();
                print_table(&["Detected", "Item", "Type", "Status", "Reactions"], &rows);
            }
        }
        GovernanceCmd::Config => {
            let config = service.config();
            if json {
                println!("{}", serde_json::to_string_pretty(&config)?);
            } else {
                println!("Repository:           {}", config.repository);
                println!("Environment:          {:?}", config.environment);
                println!(
                    "GitHub App slug:      {}",
                    config.github_app_slug.as_deref().unwrap_or("none")
                );
                println!("Development registry: {}", config.development_registry);
                println!(
                    "Governance folder:    {}",
                    config.governance_dir.as_deref().unwrap_or("not resolved")
                );
            }
        }
        GovernanceCmd::Diagnostics => {
            let mut checks = service.run_diagnostics();
            checks.extend(governance::run_network_diagnostics(ctx).await);
            let failed = checks
                .iter()
                .filter(|check| check.status == DiagnosticStatus::Fail)
                .count();
            if json {
                println!("{}", serde_json::to_string_pretty(&checks)?);
            } else {
                for check in &checks {
                    let label = match check.status {
                        DiagnosticStatus::Pass => "PASS",
                        DiagnosticStatus::Warning => "WARN",
                        DiagnosticStatus::Fail => "FAIL",
                    };
                    println!("[{label}] {}: {}", check.id, check.message);
                }
            }
            if failed > 0 {
                bail!("{failed} governance check(s) failed");
            }
        }
    }
    Ok(())
}

/// `agora registry under-review`.
pub fn registry_under_review(ctx: &Ctx, json: bool) -> anyhow::Result<()> {
    let items = RegistryService::new(ctx.clone()).list_under_review_items()?;
    if json {
        println!("{}", serde_json::to_string_pretty(&items)?);
    } else if items.is_empty() {
        println!("No registry items are under review.");
    } else {
        let rows: Vec<Vec<String>> = items
            .iter()
            .map(|item| {
                vec![
                    item.id.clone(),
                    item.name.clone(),
                    item.content_type.clone(),
                    item.net_score.to_string(),
                ]
            })
            .collect();
        print_table(&["ID", "Name", "Type", "Net score"], &rows);
    }
    Ok(())
}

/// `agora registry reviews <item>`: curator reviews as recorded in the registry.
pub fn registry_reviews(ctx: &Ctx, item: &str, json: bool) -> anyhow::Result<()> {
    let reviews = RegistryService::new(ctx.clone()).list_mod_reviews(item.to_string())?;
    if json {
        println!("{}", serde_json::to_string_pretty(&reviews)?);
    } else if reviews.is_empty() {
        println!("No curator reviews for '{item}'.");
    } else {
        for review in &reviews {
            println!(
                "#{} by {} ({})",
                review.issue_number,
                review.author.as_deref().unwrap_or("unknown"),
                review.created_at.as_deref().unwrap_or("no date")
            );
            for line in review.text.lines() {
                println!("    {line}");
            }
            println!();
        }
    }
    Ok(())
}

/// `agora registry audit-log`: newest triage actions first.
pub fn registry_audit_log(ctx: &Ctx, limit: i64, json: bool) -> anyhow::Result<()> {
    let entries = RegistryService::new(ctx.clone()).list_audit_log(limit)?;
    print_audit_entries(&entries, json, "No audit log entries.")
}

/// `agora registry resolutions`: recent triage resolutions from the audit log.
pub fn registry_resolutions(ctx: &Ctx, limit: u32, json: bool) -> anyhow::Result<()> {
    let entries = RegistryService::new(ctx.clone()).list_recent_resolutions(limit)?;
    print_audit_entries(&entries, json, "No recent resolutions.")
}

fn print_audit_entries(
    entries: &[agora_core::registry::AuditLogEntry],
    json: bool,
    empty_message: &str,
) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(entries)?);
    } else if entries.is_empty() {
        println!("{empty_message}");
    } else {
        let rows: Vec<Vec<String>> = entries
            .iter()
            .map(|entry| {
                vec![
                    entry.timestamp.clone(),
                    entry.action.clone(),
                    entry.details.clone().unwrap_or_default(),
                ]
            })
            .collect();
        print_table(&["Time", "Action", "Details"], &rows);
    }
    Ok(())
}

/// `agora technic ...`. Consent is enforced by core on every call.
pub async fn run_technic_command(
    action: TechnicCmd,
    ctx: &Ctx,
    output_fmt: OutputFormat,
) -> anyhow::Result<()> {
    let json = output_fmt.is_json_output();
    match action {
        TechnicCmd::Search { query, limit } => {
            let results = technic::search_technic(ctx, &query, limit).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&results)?);
            } else if results.is_empty() {
                println!("No Technic packs match '{query}'.");
            } else {
                let rows: Vec<Vec<String>> = results
                    .iter()
                    .map(|hit| {
                        vec![
                            hit.slug.clone(),
                            hit.title.clone(),
                            hit.tier.as_str().to_string(),
                            hit.installs.to_string(),
                        ]
                    })
                    .collect();
                print_table(&["Slug", "Title", "Tier", "Installs"], &rows);
            }
        }
        TechnicCmd::Install {
            slug,
            build,
            name,
            minecraft,
            loader,
            loader_version,
            sha256,
        } => {
            let detail = technic::pack_detail(ctx, &slug).await?;
            let result = match detail.tier {
                TechnicTier::Solder => {
                    if minecraft.is_some() || loader.is_some() || loader_version.is_some() {
                        bail!(
                            "--minecraft, --loader and --loader-version apply to zip packs only; \
                             '{slug}' is a Solder pack"
                        );
                    }
                    if sha256.is_some() {
                        bail!("--sha256 applies to zip packs only; '{slug}' is a Solder pack");
                    }
                    let solder = detail
                        .solder
                        .clone()
                        .with_context(|| format!("Technic pack '{slug}' has no Solder URL"))?;
                    let build = build
                        .or_else(|| detail.recommended_build.clone())
                        .with_context(|| {
                            format!("'{slug}' has no recommended build; pass --build <BUILD>")
                        })?;
                    let pack = technic::resolve_solder_build(ctx, &solder, &slug, &build).await?;
                    technic::install_solder_pack(ctx, pack, name).await?
                }
                TechnicTier::Zip => {
                    if build.is_some() {
                        bail!("--build applies to Solder packs only; '{slug}' is a zip pack");
                    }
                    let download_url = detail
                        .download_url
                        .clone()
                        .with_context(|| format!("Technic pack '{slug}' has no download URL"))?;
                    // Said before the install starts, so it is shown even if
                    // the install then fails.
                    if sha256.is_none() {
                        eprintln!(
                            "Warning: no --sha256 given, so the zip for '{slug}' is installed \
                             unverified: its download is not checked against a published hash. \
                             Pass --sha256 <HEX> to pin it."
                        );
                    }
                    let pack = TechnicZipPack {
                        display_name: detail.title.clone(),
                        download_url,
                        sha256,
                        minecraft_version: minecraft.with_context(|| {
                            format!("zip pack '{slug}' needs --minecraft <VERSION>")
                        })?,
                        loader: loader.with_context(|| {
                            format!("zip pack '{slug}' needs --loader <LOADER>")
                        })?,
                        loader_version: loader_version.with_context(|| {
                            format!("zip pack '{slug}' needs --loader-version <VERSION>")
                        })?,
                    };
                    technic::install_zip_pack(ctx, pack, name).await?
                }
            };
            print_import_result(&result, json, &format!("Technic pack '{slug}'"))?;
        }
    }
    Ok(())
}

/// `agora pack install --provider <id> --project <id>`: install a content
/// provider's modpack as a new instance, through the same registry the
/// desktop uses (plugin providers included when plugins are on).
pub async fn install_provider_pack(
    ctx: &Ctx,
    provider: &str,
    project: &str,
    version: Option<&str>,
    name: Option<&str>,
    json: bool,
) -> anyhow::Result<()> {
    let service = crate::plugin_service(ctx);
    service.reload()?;
    let plugins = service.is_enabled().then_some(&service);
    let registry = ProviderRegistry::new(ctx, plugins);
    let item = providers::item_id(provider, project);
    let result = providers::install::install_pack(ctx, &registry, &item, version, name).await?;
    print_import_result(&result, json, &format!("'{item}'"))
}

fn print_import_result(result: &ImportResult, json: bool, what: &str) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(result)?);
    } else {
        println!(
            "Installed {what} as '{}' (instance {}): {} mod(s), Minecraft {} with {} {}.",
            result.name,
            result.instance_id,
            result.imported_mods,
            result.minecraft_version,
            result.loader,
            result.loader_version
        );
    }
    Ok(())
}

/// `agora plugin settings <id>`: declared settings with their current values.
pub fn plugin_settings(service: &PluginService, id: &str, json: bool) -> anyhow::Result<()> {
    service.reload()?;
    let plugin_id = crate::parse_plugin_id(id)?;
    let view = service.settings(&plugin_id)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&view)?);
        return Ok(());
    }
    if view.definitions.is_empty() {
        println!("'{id}' declares no settings.");
        return Ok(());
    }
    let mut rows = Vec::with_capacity(view.definitions.len());
    for definition in &view.definitions {
        let kind = serde_json::to_value(definition)?
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let value = view
            .values
            .get(&definition.key)
            .map(serde_json::Value::to_string)
            .unwrap_or_else(|| "(not set)".to_string());
        rows.push(vec![definition.key.clone(), kind, value]);
    }
    print_table(&["Key", "Type", "Value"], &rows);
    Ok(())
}

/// `agora plugin set <id> <key> <value>`. The value is read as JSON
/// (`true`, `42`, `"text"`); text that is not valid JSON is taken as a plain
/// string. Core checks the result against the type the plugin declared, so a
/// wrong type is refused rather than stored.
pub fn plugin_set(
    service: &PluginService,
    id: &str,
    key: &str,
    raw_value: &str,
    json: bool,
) -> anyhow::Result<()> {
    service.reload()?;
    let plugin_id = crate::parse_plugin_id(id)?;
    let value = serde_json::from_str::<serde_json::Value>(raw_value)
        .unwrap_or_else(|_| serde_json::Value::String(raw_value.to_string()));
    service.set_setting(&plugin_id, key, &value)?;
    if json {
        println!(
            "{}",
            serde_json::json!({"status": "set", "plugin": id, "key": key, "value": value})
        );
    } else {
        println!("Set {key} = {value} for {id}.");
    }
    Ok(())
}

/// `--sha256`: exactly 64 hex digits, lower-cased so it compares the same way
/// as a published hash. Anything else is refused: a pin that can never match
/// is worse than no pin, because it would look like protection.
fn parse_sha256_arg(raw: &str) -> Result<String, String> {
    let hash = raw.trim();
    if hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(hash.to_ascii_lowercase())
    } else {
        Err("expected exactly 64 hexadecimal characters (a SHA-256 digest)".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::parse_sha256_arg;

    const HASH: &str = "0123456789abcdef0123456789ABCDEF0123456789abcdef0123456789abcdef";

    #[test]
    fn accepts_a_64_digit_hash_and_lower_cases_it() {
        assert_eq!(parse_sha256_arg(HASH).unwrap(), HASH.to_ascii_lowercase());
    }

    #[test]
    fn trims_surrounding_whitespace_before_checking() {
        assert_eq!(
            parse_sha256_arg(&format!("  {HASH}\n")).unwrap(),
            HASH.to_ascii_lowercase()
        );
    }

    #[test]
    fn refuses_a_hash_of_the_wrong_length() {
        assert!(parse_sha256_arg(&HASH[..63]).is_err());
        assert!(parse_sha256_arg(&format!("{HASH}0")).is_err());
        assert!(parse_sha256_arg("").is_err());
        assert!(parse_sha256_arg("   ").is_err());
    }

    #[test]
    fn refuses_non_hex_characters_at_the_right_length() {
        let bad = format!("{}g", &HASH[..63]);
        assert!(parse_sha256_arg(&bad).is_err());
        assert!(parse_sha256_arg(&"z".repeat(64)).is_err());
    }
}
