//! `agora world …` and `agora server …`: an instance's worlds and its in-game
//! server list. Thin adapters over `agora_core::worlds::WorldService`, which
//! the desktop Worlds & Servers tab uses too.

use std::path::PathBuf;

use agora_core::ctx::Ctx;
use agora_core::worlds::WorldService;
use clap::Subcommand;

use crate::{print_table, OutputFormat};

#[derive(Subcommand)]
pub enum WorldCmd {
    /// List an instance's worlds, most recently played first.
    List { instance: String },
    /// Change the name a world shows in game (its folder is kept).
    Rename {
        instance: String,
        /// The world's folder name under saves/.
        world: String,
        name: String,
    },
    /// Copy a world next to itself.
    Copy { instance: String, world: String },
    /// Move a world to the system recycle bin.
    Delete {
        instance: String,
        world: String,
        #[arg(long, help = "Do not ask for confirmation")]
        yes: bool,
    },
    /// Save a world as a zip archive.
    Export {
        instance: String,
        world: String,
        #[arg(long, help = "Where to write the .zip")]
        out: PathBuf,
    },
    /// Add a world from a zip archive.
    Import { instance: String, archive: PathBuf },
}

#[derive(Subcommand)]
pub enum ServerCmd {
    /// List the instance's multiplayer servers. Positions start at 1.
    List {
        instance: String,
        #[arg(long, help = "Also ask each server whether it is online")]
        ping: bool,
    },
    /// Add a server to the end of the list.
    Add {
        instance: String,
        /// host or host:port
        address: String,
        #[arg(long, help = "Name shown in the multiplayer list")]
        name: Option<String>,
    },
    /// Remove the server at a position shown by `server list`.
    Remove { instance: String, position: usize },
}

pub fn run_world_command(
    action: WorldCmd,
    ctx: &Ctx,
    trash: &agora_core::instance_service::TrashFn,
    output_fmt: OutputFormat,
) -> anyhow::Result<()> {
    let json = output_fmt.is_json_output();
    let service = WorldService::new(ctx.clone());
    let done = |message: String, value: serde_json::Value| -> anyhow::Result<()> {
        if json {
            println!("{}", serde_json::to_string_pretty(&value)?);
        } else {
            println!("{message}");
        }
        Ok(())
    };
    match action {
        WorldCmd::List { instance } => {
            let worlds = service.list(&instance)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&worlds)?);
            } else if worlds.is_empty() {
                println!("No worlds.");
            } else {
                let rows: Vec<Vec<String>> = worlds
                    .iter()
                    .map(|world| {
                        vec![
                            world.folder.clone(),
                            world.name.clone(),
                            world.game_mode.clone().unwrap_or_default(),
                            world.version_name.clone().unwrap_or_default(),
                            format!("{:.1} MB", world.size_bytes as f64 / 1_048_576.0),
                        ]
                    })
                    .collect();
                print_table(&["Folder", "Name", "Mode", "Version", "Size"], &rows);
            }
        }
        WorldCmd::Rename {
            instance,
            world,
            name,
        } => {
            service.rename(&instance, &world, &name)?;
            done(
                format!("Renamed '{world}' to \"{}\".", name.trim()),
                serde_json::json!({ "world": world, "name": name.trim() }),
            )?;
        }
        WorldCmd::Copy { instance, world } => {
            let folder = service.copy(&instance, &world)?;
            done(
                format!("Copied '{world}' to '{folder}'."),
                serde_json::json!({ "world": world, "copy": folder }),
            )?;
        }
        WorldCmd::Delete {
            instance,
            world,
            yes,
        } => {
            if !yes
                && !crate::cli_tools::confirm(
                    &format!("Move world '{world}' to the recycle bin? [y/N] "),
                    "Deleting a world needs confirmation; pass --yes.",
                )?
            {
                anyhow::bail!("Cancelled.");
            }
            service.delete(&instance, &world, trash)?;
            done(
                format!("Moved '{world}' to the recycle bin."),
                serde_json::json!({ "world": world, "deleted": true }),
            )?;
        }
        WorldCmd::Export {
            instance,
            world,
            out,
        } => {
            service.export(&instance, &world, &out)?;
            done(
                format!("Saved '{world}' to {}.", out.display()),
                serde_json::json!({ "world": world, "path": out }),
            )?;
        }
        WorldCmd::Import { instance, archive } => {
            let folder = service.import(&instance, &archive)?;
            done(
                format!("Imported the world as '{folder}'."),
                serde_json::json!({ "world": folder }),
            )?;
        }
    }
    Ok(())
}

pub async fn run_server_command(
    action: ServerCmd,
    ctx: &Ctx,
    output_fmt: OutputFormat,
) -> anyhow::Result<()> {
    let json = output_fmt.is_json_output();
    let service = WorldService::new(ctx.clone());
    match action {
        ServerCmd::List { instance, ping } => {
            let servers = service.list_servers(&instance)?;
            let mut statuses = Vec::new();
            if ping {
                for server in &servers {
                    statuses.push(Some(agora_core::server_ping::ping(&server.address).await?));
                }
            } else {
                statuses.resize(servers.len(), None);
            }
            if json {
                let rows: Vec<_> = servers
                    .iter()
                    .zip(&statuses)
                    .map(|(server, status)| {
                        serde_json::json!({
                            "name": server.name,
                            "address": server.address,
                            "status": status,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&rows)?);
            } else if servers.is_empty() {
                println!("No servers.");
            } else {
                let rows: Vec<Vec<String>> = servers
                    .iter()
                    .zip(&statuses)
                    .enumerate()
                    .map(|(index, (server, status))| {
                        let mut row = vec![
                            (index + 1).to_string(),
                            server.name.clone(),
                            server.address.clone(),
                        ];
                        if let Some(status) = status {
                            row.push(
                                match (status.online, status.players_online, status.players_max) {
                                    (true, Some(on), Some(max)) => format!(
                                        "online {on}/{max}{}",
                                        status
                                            .latency_ms
                                            .map(|ms| format!(", {ms} ms"))
                                            .unwrap_or_default()
                                    ),
                                    (true, _, _) => "online".into(),
                                    (false, _, _) => "offline".into(),
                                },
                            );
                        }
                        row
                    })
                    .collect();
                if ping {
                    print_table(&["#", "Name", "Address", "Status"], &rows);
                } else {
                    print_table(&["#", "Name", "Address"], &rows);
                }
            }
        }
        ServerCmd::Add {
            instance,
            address,
            name,
        } => {
            service.add_server(&instance, name.as_deref().unwrap_or(""), &address)?;
            if !json {
                println!("Added {address}.");
            }
        }
        ServerCmd::Remove { instance, position } => {
            let index = position
                .checked_sub(1)
                .ok_or_else(|| anyhow::anyhow!("Positions start at 1."))?;
            service.remove_server(&instance, index)?;
            if !json {
                println!("Removed server {position}.");
            }
        }
    }
    Ok(())
}
