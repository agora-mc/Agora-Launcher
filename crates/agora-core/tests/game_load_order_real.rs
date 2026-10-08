//! Real-data checks for the Creation Engine load order (MASTER_SPEC §26.6). They read a real
//! Skyrim SE install and mod folders, read-only, and print what they find. They are ignored by
//! default, and each one skips (with a note) when its paths are not set:
//!
//! - `AGORA_SALVAGE_MODS`: the MO2 `mods` folder (plugins at the top of each mod folder);
//! - `AGORA_SALVAGE_PROFILE`: the MO2 profile folder holding `plugins.txt`;
//! - `AGORA_SKYRIM_DATA`: the base game's `Data` folder (its `Skyrim.ccc` sits one level up);
//! - `AGORA_SKYRIM_PACKAGE`: the Skyrim SE package definition, `package.json` in the Creation
//!   package's `data` folder, which holds the plugin list rule.
//!
//! Run them with `cargo test -p agora-core --test game_load_order_real -- --ignored --nocapture`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use agora_core::game_load_order::{
    build_order, parse_implicit_list, read_header, sort_entries, Installed,
};
use agora_core::game_plugins::PluginEntry;
use agora_game_api::PluginListRule;

fn env_dir(name: &str) -> Option<PathBuf> {
    let value = std::env::var_os(name)?;
    let path = PathBuf::from(value);
    if path.is_dir() {
        Some(path)
    } else {
        eprintln!("{name} is not a folder: {}", path.display());
        None
    }
}

fn is_plugin(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("esm" | "esp" | "esl")
    )
}

/// The Skyrim SE plugin list rule, read from the package definition the app ships.
fn skyrim_rule() -> Option<PluginListRule> {
    let path = env_dir_or_file("AGORA_SKYRIM_PACKAGE")?;
    let text = std::fs::read_to_string(&path).expect("read the package definition");
    let package: serde_json::Value =
        serde_json::from_str(&text).expect("parse the package definition");
    let game = package["games"]
        .as_array()
        .and_then(|games| games.iter().find(|g| g["id"] == "skyrim-se"))
        .expect("skyrim-se is defined");
    Some(serde_json::from_value(game["plugin_list"].clone()).expect("plugin_list parses"))
}

fn env_dir_or_file(name: &str) -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os(name)?);
    if path.is_file() {
        Some(path)
    } else {
        eprintln!("{name} is not a file: {}", path.display());
        None
    }
}

/// The plugin files a mod folder holds at its top level, in name order.
fn mod_plugin_files(mods: &Path) -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    let mut folders: Vec<PathBuf> = std::fs::read_dir(mods)
        .expect("read mods folder")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    folders.sort();
    for folder in folders {
        let Ok(files) = std::fs::read_dir(&folder) else {
            continue;
        };
        for file in files.filter_map(Result::ok).map(|f| f.path()) {
            if file.is_file() && is_plugin(&file) {
                let name = file.file_name().unwrap().to_string_lossy().into_owned();
                found.push((name, file));
            }
        }
    }
    found
}

#[test]
#[ignore = "reads the real salvage mod folders; set AGORA_SALVAGE_MODS"]
fn every_salvage_header_parses() {
    let Some(mods) = env_dir("AGORA_SALVAGE_MODS") else {
        eprintln!("skipped: AGORA_SALVAGE_MODS is not set");
        return;
    };
    let files = mod_plugin_files(&mods);
    let (mut masters, mut light, mut failures) = (0usize, 0usize, Vec::new());
    let mut with_masters = 0usize;
    for (name, path) in &files {
        match read_header(path) {
            Ok(header) => {
                masters += usize::from(header.master);
                light += usize::from(header.light);
                with_masters += usize::from(!header.masters.is_empty());
            }
            Err(reason) => failures.push(format!("{name}: {reason}")),
        }
    }
    println!(
        "headers: {} plugin files; {} master-flagged, {} light, {} name at least one master; {} failures",
        files.len(),
        masters,
        light,
        with_masters,
        failures.len()
    );
    for failure in &failures {
        println!("  failure: {failure}");
    }
}

#[test]
#[ignore = "reads the real salvage profile and mods; set the four AGORA_SALVAGE_*, AGORA_SKYRIM_* paths"]
fn the_salvage_profile_order_findings_and_sort_dry_run() {
    let (Some(mods), Some(profile), Some(data), Some(rule)) = (
        env_dir("AGORA_SALVAGE_MODS"),
        env_dir("AGORA_SALVAGE_PROFILE"),
        env_dir("AGORA_SKYRIM_DATA"),
        skyrim_rule(),
    ) else {
        eprintln!(
            "skipped: set AGORA_SALVAGE_MODS, AGORA_SALVAGE_PROFILE, AGORA_SKYRIM_DATA and AGORA_SKYRIM_PACKAGE"
        );
        return;
    };

    // The profile's plugins.txt: MO2 writes a comment header, then `*Name` for active lines.
    let text = std::fs::read_to_string(profile.join("plugins.txt")).expect("read plugins.txt");
    let listed: Vec<PluginEntry> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| match l.strip_prefix('*') {
            Some(name) => PluginEntry {
                name: name.trim().to_string(),
                active: true,
                managed: false,
                locked: false,
            },
            None => PluginEntry {
                name: l.to_string(),
                active: false,
                managed: false,
                locked: false,
            },
        })
        .collect();
    println!("profile: {} listed plugins", listed.len());

    // What the game sees: the base Data folder first (the game's own files), then mod files that
    // the base does not already have. Where two mods have a plugin, the first by folder name wins;
    // the duplicates are counted, since the real order is MO2's priority.
    let mut installed: Installed = BTreeMap::new();
    let mut base_count = 0usize;
    for entry in std::fs::read_dir(&data).expect("read base Data").flatten() {
        let path = entry.path();
        if path.is_file() && is_plugin(&path) {
            installed.insert(
                entry.file_name().to_string_lossy().to_ascii_lowercase(),
                path,
            );
            base_count += 1;
        }
    }
    let mut mod_count = 0usize;
    let mut duplicates = 0usize;
    for (name, path) in mod_plugin_files(&mods) {
        let key = name.to_ascii_lowercase();
        match installed.entry(key) {
            std::collections::btree_map::Entry::Occupied(_) => duplicates += 1,
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert(path);
                mod_count += 1;
            }
        }
    }
    println!(
        "installed: {base_count} base plugin files, {mod_count} from mods, {duplicates} mod plugins also in an earlier folder or the base"
    );

    let ccc = data.parent().unwrap().join("Skyrim.ccc");
    let implicit_names = match std::fs::read_to_string(&ccc) {
        Ok(text) => parse_implicit_list(&text),
        Err(e) => {
            println!("Skyrim.ccc not read ({e}); no Creation Club plugins");
            Vec::new()
        }
    };
    let present_implicit = rule
        .implicit
        .iter()
        .chain(&implicit_names)
        .filter(|n| installed.contains_key(&n.to_ascii_lowercase()))
        .count();
    println!(
        "always loaded: {} named, {} present",
        rule.implicit.len() + implicit_names.len(),
        present_implicit
    );

    let order = build_order(&rule, &listed, Some(&installed), &implicit_names, true);
    let unreadable = order
        .entries
        .iter()
        .filter(|e| e.header_error.is_some())
        .count();
    let missing = order
        .entries
        .iter()
        .filter(|e| !e.present && !e.implicit)
        .count();
    println!(
        "order: {} entries ({} missing from the install, {} unreadable headers)",
        order.entries.len(),
        missing,
        unreadable
    );
    println!("findings: {}", order.findings.len());
    for finding in &order.findings {
        println!("  - {}", finding.message());
    }

    let mut entries = order.entries.clone();
    let (moves, blocked) = sort_entries(&mut entries).expect("sort settles");
    println!("sort --dry-run: {} moves", moves.len());
    for m in &moves {
        println!("  move '{}' from position {} to {}", m.plugin, m.from, m.to);
    }
    println!("blocked: {}", blocked.len());
    for b in &blocked {
        println!(
            "  '{}' loads above its master '{}' and cannot be moved past it",
            b.plugin, b.master
        );
    }
    let after = agora_core::game_load_order::findings(&entries);
    println!("findings after the dry-run sort: {}", after.len());
    for finding in &after {
        println!("  - {}", finding.message());
    }

    // Evidence that the rules ran on these headers: the same plugins in reverse order must raise
    // findings, and a sort must settle them.
    let readable = order.entries.iter().filter(|e| e.header_read).count();
    let links: usize = order.entries.iter().map(|e| e.masters.len()).sum();
    println!("headers read: {readable} entries, {links} master links");
    for e in order.entries.iter().filter(|e| !e.present && !e.implicit) {
        println!("  listed but not installed: {}", e.name);
    }
    let mut reversed = order.entries.clone();
    let head = reversed.iter().take_while(|e| e.implicit).count();
    reversed[head..].reverse();
    let reversed_findings = agora_core::game_load_order::findings(&reversed).len();
    let (reversed_moves, _) = sort_entries(&mut reversed).expect("sort settles");
    println!(
        "reversed experiment: {} findings before, {} moves, {} findings after",
        reversed_findings,
        reversed_moves.len(),
        agora_core::game_load_order::findings(&reversed).len()
    );
}
