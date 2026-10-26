//! `agora log …` (share a game log as a mclo.gs link) and `agora proxy …`
//! (how Agora's own requests reach the internet). Thin adapters over
//! `agora_core::log_share` and `agora_core::http_client`.

use agora_core::ctx::Ctx;
use agora_core::http_client::ProxySetting;
use clap::Subcommand;

use crate::OutputFormat;

#[derive(Subcommand)]
pub enum LogCmd {
    /// List the logs an instance can share: the latest game log and the
    /// newest crash reports.
    List { instance: String },
    /// Show what would be uploaded, then upload it to mclo.gs and print the
    /// link. Asks first unless --yes.
    Share {
        instance: String,
        #[arg(
            long,
            default_value = "logs/latest.log",
            help = "Which log, as `log list` names it"
        )]
        path: String,
        #[arg(long, help = "Upload without asking")]
        yes: bool,
        #[arg(long, help = "Print the text that would be uploaded and stop")]
        preview: bool,
    },
}

#[derive(Subcommand)]
pub enum ProxyCmd {
    /// Show the saved proxy choice.
    Show,
    /// Use the HTTPS_PROXY / HTTP_PROXY / ALL_PROXY environment variables (the default).
    System,
    /// Connect directly, ignoring any environment proxy.
    None,
    /// Use one proxy for everything. The password, if any, is read from the
    /// AGORA_PROXY_PASSWORD environment variable so it stays out of shell history.
    Set {
        /// http://, https://, socks5:// or socks5h:// URL.
        url: String,
        #[arg(long)]
        username: Option<String>,
    },
}

pub async fn run_log_command(
    action: LogCmd,
    ctx: &Ctx,
    output_fmt: OutputFormat,
) -> anyhow::Result<()> {
    let json = output_fmt.is_json_output();
    match action {
        LogCmd::List { instance } => {
            let logs = agora_core::log_share::shareable_logs(ctx, &instance)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&logs)?);
            } else if logs.is_empty() {
                println!("No logs yet.");
            } else {
                for log in logs {
                    println!("{}  ({} KB)", log.path, log.size_bytes.div_ceil(1024));
                }
            }
        }
        LogCmd::Share {
            instance,
            path,
            yes,
            preview,
        } => {
            let shown = agora_core::log_share::preview(ctx, &instance, &path)?;
            if preview {
                println!("{}", shown.text);
                return Ok(());
            }
            if !yes
                && !crate::cli_tools::confirm(
                    &format!(
                        "Upload {path} ({} lines{}) to mclo.gs, where anyone with the link can read it? [y/N] ",
                        shown.text.lines().count(),
                        if shown.dropped_lines > 0 {
                            format!(", oldest {} left off", shown.dropped_lines)
                        } else {
                            String::new()
                        }
                    ),
                    "Uploading a log needs confirmation; pass --yes (or --preview to see it first).",
                )?
            {
                anyhow::bail!("Cancelled.");
            }
            let shared = agora_core::log_share::upload(ctx, &instance, &path).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&shared)?);
            } else {
                println!("{}", shared.url);
            }
        }
    }
    Ok(())
}

pub fn run_proxy_command(
    action: ProxyCmd,
    ctx: &Ctx,
    output_fmt: OutputFormat,
) -> anyhow::Result<()> {
    let conn = agora_core::db::local_state_connection(&ctx.paths.local_state_db())?;
    let save = |setting: ProxySetting, password: Option<String>| -> anyhow::Result<()> {
        agora_core::http_client::save_proxy(&conn, &setting, password.as_deref())?;
        Ok(())
    };
    match action {
        ProxyCmd::Show => {}
        ProxyCmd::System => save(ProxySetting::System, Some(String::new()))?,
        ProxyCmd::None => save(ProxySetting::None, Some(String::new()))?,
        ProxyCmd::Set { url, username } => {
            let password = std::env::var("AGORA_PROXY_PASSWORD").ok();
            save(ProxySetting::Manual { url, username }, password)?;
        }
    }
    let setting = agora_core::http_client::saved_proxy(&conn);
    if output_fmt.is_json_output() {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "proxy": setting,
                "passwordStored": agora_core::http_client::proxy_password_stored(),
            }))?
        );
    } else {
        match setting {
            ProxySetting::System => println!("Proxy: system (environment variables)"),
            ProxySetting::None => println!("Proxy: none"),
            ProxySetting::Manual { url, username } => println!(
                "Proxy: {url}{}",
                username
                    .map(|name| format!(" as {name}"))
                    .unwrap_or_default()
            ),
        }
    }
    Ok(())
}

#[derive(Subcommand)]
pub enum SkinCmd {
    /// Show the active account's skin model and capes.
    Show,
    /// Upload a 64×64 PNG as the active account's skin.
    Wear {
        file: std::path::PathBuf,
        #[arg(long, help = "Slim (three-pixel) arms")]
        slim: bool,
    },
    /// Go back to the default skin.
    Reset,
    /// Show a cape by its name (see `skin show`), or `none` to hide capes.
    Cape { name: String },
}

pub async fn run_skin_command(
    action: SkinCmd,
    ctx: &Ctx,
    output_fmt: OutputFormat,
) -> anyhow::Result<()> {
    use agora_core::skins::{self, SkinVariant};
    match action {
        SkinCmd::Show => {}
        SkinCmd::Wear { file, slim } => {
            let bytes = std::fs::read(&file)?;
            let variant = if slim {
                SkinVariant::Slim
            } else {
                SkinVariant::Classic
            };
            skins::upload_skin(ctx, &bytes, variant).await?;
        }
        SkinCmd::Reset => skins::reset_skin(ctx).await?,
        SkinCmd::Cape { name } => {
            if name.eq_ignore_ascii_case("none") {
                skins::set_cape(ctx, None).await?;
            } else {
                let profile = skins::profile(ctx).await?;
                let cape = profile
                    .capes
                    .iter()
                    .find(|cape| cape.alias.eq_ignore_ascii_case(&name))
                    .ok_or_else(|| anyhow::anyhow!("This account has no cape called '{name}'."))?;
                skins::set_cape(ctx, Some(&cape.id)).await?;
            }
        }
    }
    let profile = skins::profile(ctx).await?;
    if output_fmt.is_json_output() {
        let capes: Vec<_> = profile
            .capes
            .iter()
            .map(|cape| serde_json::json!({ "name": cape.alias, "active": cape.active }))
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "username": profile.username,
                "variant": profile.variant,
                "capes": capes,
            }))?
        );
    } else {
        println!("{} · {:?} arms", profile.username, profile.variant);
        if profile.capes.is_empty() {
            println!("Capes: none owned");
        }
        for cape in &profile.capes {
            println!(
                "Cape: {}{}",
                cape.alias,
                if cape.active { " (worn)" } else { "" }
            );
        }
    }
    Ok(())
}
