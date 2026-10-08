//! The real `p3-modded` instance: a Skyrim SE with 23 mods that ran in Phase 3 (MASTER_SPEC §26.13).
//! Its load order is read in place, read-only, and the findings printed. Ignored by default.
//!
//! Run with `cargo test -p agora-cli --test game_load_order_p3 -- --ignored --nocapture`.
//! `AGORA_P3_DATA` overrides the data folder (default `D:\Agora-bench\p3-done`).

use std::path::PathBuf;

use agora_core::ctx::CoreContext;
use agora_core::game_load_order::{self, Finding};
use agora_game_api::GameDefinition;

/// The Skyrim SE definition the app ships, from the Creation Engine package.
fn skyrim() -> GameDefinition {
    agora_game_creation::game_package()
        .definition()
        .games
        .iter()
        .find(|g| g.id.as_str() == "skyrim-se")
        .expect("the package defines skyrim-se")
        .clone()
}

#[test]
#[ignore = "reads the real p3-modded instance in D:\\Agora-bench\\p3-done; read only"]
fn p3_modded_load_order_has_no_refusing_findings() {
    let root = std::env::var_os("AGORA_P3_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"D:\Agora-bench\p3-done"));
    if !root.join("instances").join("p3-modded").is_dir() {
        eprintln!("skipped: no p3-modded instance under {}", root.display());
        return;
    }

    let def = skyrim();
    // The check reads the instance's manifest, plugin list, base manifest, layers and plugin
    // headers. It takes no lock and writes nothing; `for_testing` only creates the locks folder,
    // which this data folder already has.
    let ctx = CoreContext::for_testing(root.clone());
    let findings = game_load_order::check(&ctx, "p3-modded", &def)
        .expect("the real instance's load order can be read");

    let order = game_load_order::order(&ctx, "p3-modded", &def).expect("order reads");
    println!(
        "p3-modded in {}: {} plugin lines, {} finding(s)",
        root.display(),
        order.entries.len(),
        findings.len()
    );
    for finding in &findings {
        let kind = if finding.refuses_launch() {
            "refuses"
        } else {
            "warns"
        };
        println!("  [{kind}] {}", finding.message());
    }
    // What a sort would move, worked on a copy in memory: nothing is written.
    let mut sorted = order.entries.clone();
    let (moves, blocked) =
        game_load_order::sort_entries(&mut sorted).expect("the sort dry run in memory");
    println!(
        "in-memory sort (nothing written): {} move(s), {} blocked",
        moves.len(),
        blocked.len()
    );
    for m in &moves {
        println!("  move '{}' from position {} to {}", m.plugin, m.from, m.to);
    }
    for b in &blocked {
        println!(
            "  cannot fix: '{}' loads above its master '{}'",
            b.plugin, b.master
        );
    }
    let refusing: Vec<&Finding> = findings.iter().filter(|f| f.refuses_launch()).collect();
    assert!(
        refusing.is_empty(),
        "a launch of p3-modded would be refused: {refusing:?}"
    );
}
