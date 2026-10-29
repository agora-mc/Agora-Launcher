//! `agora games folder …`: game folders the user adds by hand (MASTER_SPEC §26.3). A thin adapter
//! over `agora_core::game_discovery::custom_folders`, which the desktop Setup section uses too.

use agora_core::ctx::Ctx;
use agora_core::game_discovery::custom_folders;
use clap::Subcommand;

#[derive(Subcommand)]
pub enum FolderCmd {
    /// Add a folder that holds a game the store scan did not find.
    Add {
        /// The folder the game is installed in (the one with its executable).
        path: String,
    },
    /// Stop searching a folder added with `games folder add`.
    Remove {
        /// The folder, as `games folder list` shows it.
        path: String,
    },
    /// List the folders you added, and whether each is still there.
    List,
}

pub fn run_games_folder(action: FolderCmd, ctx: &Ctx, json: bool) -> anyhow::Result<()> {
    match action {
        FolderCmd::Add { path } => {
            let added = custom_folders::add_folder(&ctx.paths, &ctx.games, &path)
                .map_err(|error| anyhow::anyhow!("{error}"))?;
            if json {
                println!("{}", serde_json::to_string_pretty(&added)?);
            } else {
                println!("Added {} as {}.", added.path, added.game_name);
                println!(
                    "Run `agora games list` to see its runtime and whether Agora can manage it."
                );
            }
        }
        FolderCmd::Remove { path } => {
            let removed = custom_folders::remove_folder(&ctx.paths, &path)
                .map_err(|error| anyhow::anyhow!("{error}"))?;
            if json {
                println!("{}", serde_json::json!({ "removed": removed }));
            } else {
                println!("Removed {removed} from your game folders. Its files were not touched.");
            }
        }
        FolderCmd::List => {
            let folders = custom_folders::list_folder_status(&ctx.paths)
                .map_err(|error| anyhow::anyhow!("{error}"))?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({ "folders": folders }))?
                );
            } else if folders.is_empty() {
                println!("No game folders added. Use `agora games folder add <path>` for a copy the store scan did not find.");
            } else {
                println!("Game folders ({}):", folders.len());
                for folder in &folders {
                    let note = if folder.present {
                        ""
                    } else {
                        " (not there now, so it is not searched)"
                    };
                    println!("  - {}{note}", folder.path);
                }
            }
        }
    }
    Ok(())
}
