//! Game settings shared between instances.
//!
//! An instance can opt in to sharing its `options.txt` settings (video,
//! controls, sound, key bindings) and its multiplayer server list with every
//! other instance that has opted in. Before a launch the shared copy is
//! written into the instance; after the game closes the instance's copy is
//! taken back as the new shared one. Last game closed wins.
//!
//! `options.txt` is merged key by key, so a setting one instance does not know
//! (a mod's key binding, say) survives in the shared copy for the instances
//! that do. A few keys that only make sense for one instance are never shared.

use crate::ctx::Ctx;
use crate::models::InstanceManifest;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// `user_preferences` switches for the two kinds of sharing.
pub const SHARE_OPTIONS_KEY: &str = "agora_share_options";
pub const SHARE_SERVERS_KEY: &str = "agora_share_servers";

/// Keys that describe this instance rather than the player's preferences.
const NOT_SHARED: &[&str] = &[
    "version",
    "resourcePacks",
    "incompatibleResourcePacks",
    "lastServer",
];

fn shared_dir(ctx: &Ctx) -> PathBuf {
    ctx.paths.root().join("shared-game-settings")
}

fn flag(manifest: &InstanceManifest, key: &str) -> bool {
    manifest
        .user_preferences
        .get(key)
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

/// Which kinds of sharing an instance has switched on.
pub fn sharing(manifest: &InstanceManifest) -> (bool, bool) {
    (
        flag(manifest, SHARE_OPTIONS_KEY),
        flag(manifest, SHARE_SERVERS_KEY),
    )
}

fn parse_options(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn read_options(path: &Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(path)
        .map(|text| parse_options(&text))
        .unwrap_or_default()
}

/// Write `key:value` lines, keeping the order of an existing file and adding
/// new keys at the end, as the game does.
fn write_options(path: &Path, values: &BTreeMap<String, String>) -> std::io::Result<()> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let mut written = std::collections::HashSet::new();
    let mut lines = Vec::new();
    for line in existing.lines() {
        if let Some((key, _)) = line.split_once(':') {
            if let Some(value) = values.get(key) {
                lines.push(format!("{key}:{value}"));
                written.insert(key.to_string());
                continue;
            }
        }
        lines.push(line.to_string());
    }
    for (key, value) in values {
        if !written.contains(key) {
            lines.push(format!("{key}:{value}"));
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let partial = path.with_extension("txt.agora-partial");
    std::fs::write(&partial, lines.join("\n") + "\n")?;
    std::fs::rename(&partial, path)
}

fn copy_atomically(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let partial = to.with_extension("agora-partial");
    std::fs::copy(from, &partial)?;
    std::fs::rename(&partial, to)
}

/// Bring the shared settings into an instance before it launches. Best
/// effort: a failure here must never stop a launch, so problems are returned
/// as messages for the console rather than errors.
pub fn apply_before_launch(ctx: &Ctx, game_dir: &Path, manifest: &InstanceManifest) -> Vec<String> {
    let (options, servers) = sharing(manifest);
    let dir = shared_dir(ctx);
    let mut problems = Vec::new();
    if options {
        let shared = read_options(&dir.join("options.txt"));
        if !shared.is_empty() {
            let mut mine = read_options(&game_dir.join("options.txt"));
            for (key, value) in shared {
                if !NOT_SHARED.contains(&key.as_str()) {
                    mine.insert(key, value);
                }
            }
            if let Err(error) = write_options(&game_dir.join("options.txt"), &mine) {
                problems.push(format!("Could not apply shared game settings: {error}"));
            }
        }
    }
    if servers && dir.join("servers.dat").is_file() {
        if let Err(error) = copy_atomically(&dir.join("servers.dat"), &game_dir.join("servers.dat"))
        {
            problems.push(format!("Could not apply the shared server list: {error}"));
        }
    }
    problems
}

/// Take an instance's settings back as the shared copy after the game closes,
/// or when sharing is first switched on.
pub fn capture(ctx: &Ctx, game_dir: &Path, manifest: &InstanceManifest) -> Vec<String> {
    let (options, servers) = sharing(manifest);
    let dir = shared_dir(ctx);
    let mut problems = Vec::new();
    if options {
        let mine = read_options(&game_dir.join("options.txt"));
        if !mine.is_empty() {
            let mut shared = read_options(&dir.join("options.txt"));
            for (key, value) in mine {
                if !NOT_SHARED.contains(&key.as_str()) {
                    shared.insert(key, value);
                }
            }
            if let Err(error) = write_options(&dir.join("options.txt"), &shared) {
                problems.push(format!("Could not save shared game settings: {error}"));
            }
        }
    }
    if servers && game_dir.join("servers.dat").is_file() {
        if let Err(error) = copy_atomically(&game_dir.join("servers.dat"), &dir.join("servers.dat"))
        {
            problems.push(format!("Could not save the shared server list: {error}"));
        }
    }
    problems
}

/// Whether a shared copy exists yet, so switching sharing on can say whether
/// this instance's settings will be replaced or will become the shared ones.
pub fn shared_copy_exists(ctx: &Ctx) -> (bool, bool) {
    let dir = shared_dir(ctx);
    (
        dir.join("options.txt").is_file(),
        dir.join("servers.dat").is_file(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(options: bool, servers: bool) -> InstanceManifest {
        serde_json::from_value(serde_json::json!({
            "instance_id": "a", "name": "A", "minecraft_version": "1.21",
            "loader": "vanilla", "loader_version": "", "is_locked": false,
            "mods": [], "user_preferences": {
                SHARE_OPTIONS_KEY: options, SHARE_SERVERS_KEY: servers
            }
        }))
        .unwrap()
    }

    #[test]
    fn options_merge_by_key_and_keep_instance_only_keys() {
        let root = tempfile::tempdir().unwrap();
        let ctx = Ctx::for_testing(root.path().to_path_buf());
        let a = root.path().join("a");
        let b = root.path().join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(
            a.join("options.txt"),
            "version:3955\nfov:0.5\nkey_key.mymod.zoom:key.keyboard.c\nresourcePacks:[\"a\"]\n",
        )
        .unwrap();
        std::fs::write(
            b.join("options.txt"),
            "version:3700\nfov:0.0\nresourcePacks:[\"b\"]\n",
        )
        .unwrap();
        let on = manifest(true, false);

        assert!(capture(&ctx, &a, &on).is_empty());
        assert!(apply_before_launch(&ctx, &b, &on).is_empty());
        let b_options = read_options(&b.join("options.txt"));
        assert_eq!(b_options["fov"], "0.5", "shared setting applied");
        assert_eq!(b_options["key_key.mymod.zoom"], "key.keyboard.c");
        assert_eq!(b_options["version"], "3700", "instance-only key untouched");
        assert_eq!(b_options["resourcePacks"], "[\"b\"]");
        // The file keeps its own order, with new keys at the end.
        let text = std::fs::read_to_string(b.join("options.txt")).unwrap();
        assert!(text.starts_with("version:3700\nfov:0.5\n"), "{text}");
    }

    #[test]
    fn nothing_happens_without_opting_in() {
        let root = tempfile::tempdir().unwrap();
        let ctx = Ctx::for_testing(root.path().to_path_buf());
        let a = root.path().join("a");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::write(a.join("options.txt"), "fov:0.5\n").unwrap();
        std::fs::write(a.join("servers.dat"), b"nbt").unwrap();
        capture(&ctx, &a, &manifest(false, false));
        assert_eq!(shared_copy_exists(&ctx), (false, false));
        capture(&ctx, &a, &manifest(false, true));
        assert_eq!(shared_copy_exists(&ctx), (false, true));
    }
}
