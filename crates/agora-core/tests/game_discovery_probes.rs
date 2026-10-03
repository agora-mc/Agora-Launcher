//! Hostile and messy inputs for store discovery: corrupt caches must become
//! warnings, never an abort, a stack overflow or a duplicate install.

use std::fs;

use agora_core::game_discovery::gog::{GogAdapter, GogRegistryEntry};
use agora_core::game_discovery::microsoft_store::gaming_root::parse_gaming_root;
use agora_core::game_discovery::microsoft_store::MicrosoftStoreAdapter;
use agora_core::game_discovery::steam::vdf_binary::parse_appinfo_vdf;
use agora_core::game_discovery::steam::vdf_text::parse_vdf_text;
use agora_core::game_discovery::steam::SteamAdapter;
use agora_core::game_discovery::volume::VolumeDetector;
use agora_game_api::InstallKind;

const V29: u32 = 0x0756_4429;

fn v29_header(string_table_offset: i64) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&V29.to_le_bytes());
    b.extend_from_slice(&1u32.to_le_bytes());
    b.extend_from_slice(&string_table_offset.to_le_bytes());
    b
}

#[test]
fn appinfo_string_table_count_is_not_trusted_for_allocation() {
    let mut b = v29_header(20);
    b.extend_from_slice(&0u32.to_le_bytes()); // no apps
    b.extend_from_slice(&u32::MAX.to_le_bytes()); // "4 billion strings"
    b.extend_from_slice(b"a\0");
    assert!(parse_appinfo_vdf(&b, None).is_err());
}

#[test]
fn appinfo_deep_nesting_is_an_error_not_a_stack_overflow() {
    // String table: ["k"]
    let mut entry = Vec::new();
    entry.extend_from_slice(&[0u8; 60]);
    for _ in 0..200_000 {
        entry.push(0x00);
        entry.extend_from_slice(&0u32.to_le_bytes());
    }
    let apps_start = 16;
    let table_offset = apps_start + 8 + entry.len() + 4;
    let mut b = v29_header(table_offset as i64);
    b.extend_from_slice(&7u32.to_le_bytes());
    b.extend_from_slice(&(entry.len() as u32).to_le_bytes());
    b.extend_from_slice(&entry);
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&1u32.to_le_bytes());
    b.extend_from_slice(b"k\0");
    let wanted = [7u32].into_iter().collect();
    assert!(parse_appinfo_vdf(&b, Some(&wanted)).is_err());
}

#[test]
fn appinfo_entry_size_past_end_of_file_is_an_error() {
    let mut b = v29_header(0);
    b.extend_from_slice(&7u32.to_le_bytes());
    b.extend_from_slice(&u32::MAX.to_le_bytes());
    let len = b.len() as i64;
    b[8..16].copy_from_slice(&len.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    assert!(parse_appinfo_vdf(&b, None).is_err());
}

#[test]
fn gaming_root_count_is_not_trusted_for_allocation() {
    let mut b = b"RGBX".to_vec();
    b.extend_from_slice(&u32::MAX.to_le_bytes());
    b.extend_from_slice(&[b'X', 0, 0, 0]);
    assert!(parse_gaming_root(&b).is_err());
}

#[test]
fn text_vdf_deep_nesting_and_unterminated_input_are_errors() {
    assert!(parse_vdf_text(&"\"a\" {".repeat(200_000)).is_err());
    assert!(parse_vdf_text("\"a\" { \"b\" \"c").is_err());
    assert!(parse_vdf_text("\"a\" { \"b\" \"c\"").is_err());
}

fn acf(app_id: u32, name: &str, dir: &str, flags: &str) -> String {
    format!(
        "\"AppState\"\n{{\n\t\"appid\"\t\t\"{app_id}\"\n\t\"name\"\t\t\"{name}\"\n\t\"StateFlags\"\t\t\"{flags}\"\n\t\"installdir\"\t\t\"{dir}\"\n\t\"buildid\"\t\t\"1\"\n}}\n"
    )
}

#[test]
fn steam_library_listed_twice_with_different_spelling_gives_one_install() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("Steam");
    let steamapps = root.join("steamapps");
    fs::create_dir_all(steamapps.join("common").join("Game")).unwrap();
    fs::create_dir_all(steamapps.join("common").join("Half")).unwrap();
    fs::write(
        steamapps.join("appmanifest_10.acf"),
        acf(10, "Game", "Game", "4"),
    )
    .unwrap();
    // StateFlags without the fully-installed bit: an update queued, not installed.
    fs::write(
        steamapps.join("appmanifest_11.acf"),
        acf(11, "Half", "Half", "1026"),
    )
    .unwrap();
    let alias = root.to_string_lossy().replace('\\', "/");
    let alias = if cfg!(windows) {
        alias.to_uppercase()
    } else {
        format!("{alias}/.")
    };
    let vdf = format!(
        "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n",
        alias.replace('\\', "\\\\")
    );
    fs::write(steamapps.join("libraryfolders.vdf"), vdf).unwrap();

    let report = SteamAdapter::discover_from(&root, &VolumeDetector::new());
    let ids: Vec<_> = report.installs.iter().map(|i| i.product.as_str()).collect();
    assert_eq!(ids, ["10"], "{report:?}");
}

#[test]
fn gog_product_in_both_registry_hives_is_listed_once() {
    let tmp = tempfile::tempdir().unwrap();
    let entry = GogRegistryEntry {
        game_id: "1".into(),
        game_name: "Game".into(),
        path: tmp.path().to_path_buf(),
        ver: Some("1.0".into()),
        build_id: None,
        exe_file: Some("game.exe".into()),
        depends_on: Some(String::new()),
        dlc: None,
    };
    let report = GogAdapter::discover_from(&[entry.clone(), entry], &VolumeDetector::new());
    assert_eq!(report.installs.len(), 1, "{report:?}");
    assert_eq!(report.installs[0].kind, InstallKind::BaseGame);
}

#[test]
fn microsoft_game_config_with_a_byte_order_mark_parses() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("XboxGames");
    let content = root.join("Game").join("Content");
    fs::create_dir_all(&content).unwrap();
    let xml = "\u{feff}<?xml version=\"1.0\" encoding=\"utf-8\"?>\r\n<Game configVersion=\"1\">\r\n  <Identity Name=\"Pub.Game\" Publisher=\"CN=x\" Version=\"1.2.3.0\" />\r\n  <ExecutableList><Executable Name=\"Game.exe\" /></ExecutableList>\r\n  <ShellVisuals DefaultDisplayName=\"The Game\" />\r\n</Game>\r\n";
    fs::write(content.join("MicrosoftGame.config"), xml).unwrap();
    let report = MicrosoftStoreAdapter::discover_from(&[root], &[], &VolumeDetector::new());
    assert_eq!(report.installs.len(), 1, "{report:?}");
    assert_eq!(report.installs[0].product, "Pub.Game");
    assert_eq!(report.installs[0].name, "The Game");
    assert!(
        !report.installs[0].capabilities.executables_readable,
        "Game.exe does not exist"
    );
}
