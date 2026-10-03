use std::collections::HashSet;
use std::fs;

use agora_core::game_discovery::discover_all;
use agora_core::game_discovery::epic::EpicAdapter;
use agora_core::game_discovery::gog::{GogAdapter, GogRegistryEntry};
use agora_core::game_discovery::microsoft_store::gaming_root::parse_gaming_root;
use agora_core::game_discovery::microsoft_store::MicrosoftStoreAdapter;
use agora_core::game_discovery::steam::vdf_binary::parse_appinfo_vdf;
use agora_core::game_discovery::steam::vdf_text::{get_object, get_str, parse_vdf_text};
use agora_core::game_discovery::steam::SteamAdapter;
use agora_core::game_discovery::volume::VolumeDetector;
use agora_game_api::InstallKind;

// ---------------------------------------------------------------------------
// 1. Empty input
// ---------------------------------------------------------------------------

#[test]
fn test_empty_input_gives_no_installs_and_no_warnings() {
    let detector = VolumeDetector::new();
    let tmp = tempfile::tempdir().unwrap();

    // Steam with non-existent root
    let steam_report = SteamAdapter::new(None).discover_with_detector(&detector);
    assert_eq!(steam_report.installs.len(), 0);
    assert_eq!(steam_report.warnings.len(), 0);

    // GOG with empty entries
    let gog_report = GogAdapter::discover_from(&[], &detector);
    assert_eq!(gog_report.installs.len(), 0);
    assert_eq!(gog_report.warnings.len(), 0);

    // Epic with empty manifests dir
    let epic_dir = tmp.path().join("epic_manifests");
    fs::create_dir_all(&epic_dir).unwrap();
    let epic_report = EpicAdapter::discover_from(&epic_dir, &detector);
    assert_eq!(epic_report.installs.len(), 0);
    assert_eq!(epic_report.warnings.len(), 0);

    // Microsoft Store with empty roots and packages
    let ms_report = MicrosoftStoreAdapter::discover_from(&[], &[], &detector);
    assert_eq!(ms_report.installs.len(), 0);
    assert_eq!(ms_report.warnings.len(), 0);
}

// ---------------------------------------------------------------------------
// 2. One broken file among good ones
// ---------------------------------------------------------------------------

#[test]
fn test_one_broken_file_among_good_ones_yields_good_and_warning() {
    let detector = VolumeDetector::new();
    let tmp = tempfile::tempdir().unwrap();

    // Epic test
    let manifests_dir = tmp.path().join("manifests");
    fs::create_dir_all(&manifests_dir).unwrap();

    // Good item
    let good_item = r#"{
        "AppName": "GoodGame",
        "DisplayName": "Good Game",
        "InstallLocation": "C:\\Games\\GoodGame",
        "AppCategories": ["games"]
    }"#;
    fs::write(manifests_dir.join("good.item"), good_item).unwrap();

    // Broken item
    let broken_item = r#"{ "AppName": "Broken", broken_json"#;
    let broken_path = manifests_dir.join("broken.item");
    fs::write(&broken_path, broken_item).unwrap();

    let report = EpicAdapter::discover_from(&manifests_dir, &detector);
    assert_eq!(report.installs.len(), 1);
    assert_eq!(report.installs[0].product, "GoodGame");
    assert_eq!(report.warnings.len(), 1);
    assert!(
        report.warnings[0].message.contains(
            &broken_path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string()
        ),
        "warning should name the broken file: {}",
        report.warnings[0].message
    );
}

// ---------------------------------------------------------------------------
// 3. GOG Cyberpunk case
// ---------------------------------------------------------------------------

#[test]
fn test_gog_cyberpunk_case() {
    let detector = VolumeDetector::new();
    let tmp = tempfile::tempdir().unwrap();

    let game_folder = tmp.path().join("Cyberpunk 2077");
    fs::create_dir_all(&game_folder).unwrap();

    // Create goggame-1423049311.info with playTasks
    let info_json = r#"{
        "gameId": "1423049311",
        "rootGameId": "1423049311",
        "playTasks": [
            {"category": "launcher", "isPrimary": true, "path": "REDprelauncher.exe", "type": "FileTask"},
            {"category": "game", "isHidden": true, "path": "bin\\x64\\Cyberpunk2077.exe", "type": "FileTask"}
        ]
    }"#;
    fs::write(game_folder.join("goggame-1423049311.info"), info_json).unwrap();

    let entries = vec![
        GogRegistryEntry {
            game_id: "1423049311".to_string(),
            game_name: "Cyberpunk 2077".to_string(),
            path: game_folder.clone(),
            ver: Some("2.31a".to_string()),
            build_id: Some("58989493373906337".to_string()),
            exe_file: Some("REDprelauncher.exe".to_string()),
            depends_on: None,
            dlc: Some("1256837418, 1597316373".to_string()),
        },
        GogRegistryEntry {
            game_id: "1256837418".to_string(),
            game_name: "Cyberpunk 2077: Phantom Liberty".to_string(),
            path: game_folder.clone(),
            ver: Some("2.31a".to_string()),
            build_id: Some("58989493373906337".to_string()),
            exe_file: None,
            depends_on: Some("1423049311".to_string()),
            dlc: None,
        },
        GogRegistryEntry {
            game_id: "1597316373".to_string(),
            game_name: "Cyberpunk 2077: Bonus Content".to_string(),
            path: game_folder.clone(),
            ver: Some("2.31a".to_string()),
            build_id: Some("58989493373906337".to_string()),
            exe_file: None,
            depends_on: Some("1423049311".to_string()),
            dlc: None,
        },
    ];

    let report = GogAdapter::discover_from(&entries, &detector);
    assert_eq!(report.installs.len(), 3);
    assert_eq!(report.warnings.len(), 0);

    let base = report
        .installs
        .iter()
        .find(|i| i.product == "1423049311")
        .unwrap();
    assert_eq!(base.kind, InstallKind::BaseGame);
    assert_eq!(base.parent_product, None);
    assert_eq!(base.store_version.as_deref(), Some("2.31a"));
    assert_eq!(base.store_build.as_deref(), Some("58989493373906337"));
    // Game executable before launcher
    assert_eq!(
        base.executables,
        vec!["bin\\x64\\Cyberpunk2077.exe", "REDprelauncher.exe"]
    );

    let dlc1 = report
        .installs
        .iter()
        .find(|i| i.product == "1256837418")
        .unwrap();
    assert_eq!(dlc1.kind, InstallKind::AddOn);
    assert_eq!(dlc1.parent_product.as_deref(), Some("1423049311"));

    let dlc2 = report
        .installs
        .iter()
        .find(|i| i.product == "1597316373")
        .unwrap();
    assert_eq!(dlc2.kind, InstallKind::AddOn);
    assert_eq!(dlc2.parent_product.as_deref(), Some("1423049311"));
}

// ---------------------------------------------------------------------------
// 4. Epic table classification
// ---------------------------------------------------------------------------

#[test]
fn test_epic_table_classification() {
    let detector = VolumeDetector::new();
    let tmp = tempfile::tempdir().unwrap();
    let manifests_dir = tmp.path().join("epic_manifests");
    fs::create_dir_all(&manifests_dir).unwrap();

    let eel = r#"{
        "AppName": "Eel",
        "DisplayName": "Kingdom Come: Deliverance",
        "InstallLocation": "C:\\Games\\KCD",
        "LaunchExecutable": "bin/Win64MasterMasterEpicPGO/KingdomCome.exe",
        "MainGameAppName": "",
        "AppCategories": "public, games, applications"
    }"#;
    fs::write(manifests_dir.join("eel.item"), eel).unwrap();

    let eel_dlc = r#"{
        "AppName": "EelTexturePack",
        "DisplayName": "Kingdom Come HD Textures",
        "InstallLocation": "C:\\Games\\KCD",
        "LaunchExecutable": "",
        "MainGameAppName": "Eel",
        "AppCategories": "public, games, applications"
    }"#;
    fs::write(manifests_dir.join("eel_dlc.item"), eel_dlc).unwrap();

    let ue = r#"{
        "AppName": "UE_5.6",
        "DisplayName": "Unreal Engine 5.6",
        "InstallLocation": "C:\\UE5.6",
        "LaunchExecutable": "Engine/Binaries/Win64/UnrealEditor.exe",
        "MainGameAppName": "",
        "AppCategories": "engines/ue5, engines"
    }"#;
    fs::write(manifests_dir.join("ue.item"), ue).unwrap();

    let fab = r#"{
        "AppName": "FabPlugin_5.6",
        "DisplayName": "Fab Plugin",
        "InstallLocation": "C:\\Fab",
        "LaunchExecutable": "",
        "MainGameAppName": "",
        "AppCategories": ""
    }"#;
    fs::write(manifests_dir.join("fab.item"), fab).unwrap();

    let report = EpicAdapter::discover_from(&manifests_dir, &detector);
    assert_eq!(report.installs.len(), 4);
    assert_eq!(report.warnings.len(), 0);

    let eel_inst = report.installs.iter().find(|i| i.product == "Eel").unwrap();
    assert_eq!(eel_inst.kind, InstallKind::BaseGame);
    assert_eq!(eel_inst.parent_product, None);
    assert_eq!(
        eel_inst.executables,
        vec!["bin/Win64MasterMasterEpicPGO/KingdomCome.exe"]
    );

    let dlc_inst = report
        .installs
        .iter()
        .find(|i| i.product == "EelTexturePack")
        .unwrap();
    assert_eq!(dlc_inst.kind, InstallKind::AddOn);
    assert_eq!(dlc_inst.parent_product.as_deref(), Some("Eel"));
    assert!(dlc_inst.executables.is_empty());

    let ue_inst = report
        .installs
        .iter()
        .find(|i| i.product == "UE_5.6")
        .unwrap();
    assert_eq!(ue_inst.kind, InstallKind::Tool);
    assert_eq!(ue_inst.parent_product, None);
    assert_eq!(
        ue_inst.executables,
        vec!["Engine/Binaries/Win64/UnrealEditor.exe"]
    );

    let fab_inst = report
        .installs
        .iter()
        .find(|i| i.product == "FabPlugin_5.6")
        .unwrap();
    assert_eq!(fab_inst.kind, InstallKind::Tool);
    assert_eq!(fab_inst.parent_product, None);
    assert!(fab_inst.executables.is_empty());
}

// ---------------------------------------------------------------------------
// 5. Microsoft Store CK3-like root
// ---------------------------------------------------------------------------

#[test]
fn test_microsoft_store_ck3_like_root() {
    let detector = VolumeDetector::new();
    let tmp = tempfile::tempdir().unwrap();
    let xbox_root = tmp.path().join("XboxGames");

    // Base game
    let ck3_dir = xbox_root.join("Crusader Kings III").join("Content");
    fs::create_dir_all(&ck3_dir).unwrap();
    let base_xml = r#"<Game configVersion="1">
        <Identity Name="ParadoxInteractive.ProjectTitus" Publisher="CN=Test" Version="1.1.306.0" />
        <ExecutableList><Executable Name="launcher/bin/Paradox Launcher.exe" TargetDeviceFamily="PC"/></ExecutableList>
        <StoreId>9N7GG222GTTH</StoreId>
        <ShellVisuals DefaultDisplayName="Crusader Kings III" PublisherDisplayName="Paradox Interactive" />
    </Game>"#;
    fs::write(ck3_dir.join("MicrosoftGame.config"), base_xml).unwrap();

    // DLC
    let dlc_dir = xbox_root
        .join("Crusader Kings III - Royal Court")
        .join("Content");
    fs::create_dir_all(&dlc_dir).unwrap();
    let dlc_xml = r#"<Game configVersion="1">
        <Identity Name="ParadoxInteractive.CrusaderKingsIIIExpansion1" Publisher="CN=Test" Version="1.1.4.0"/>
        <StoreId>9PDMBMV4J906</StoreId>
        <ShellVisuals DefaultDisplayName="Crusader Kings III: Royal Court" />
        <AllowedProducts><AllowedProduct>9N7GG222GTTH</AllowedProduct></AllowedProducts>
        <DesktopRegistration><MainPackageDependency Name="ParadoxInteractive.ProjectTitus" /></DesktopRegistration>
    </Game>"#;
    fs::write(dlc_dir.join("MicrosoftGame.config"), dlc_xml).unwrap();

    // Folder without config (e.g. GameSave)
    let gamesave_dir = xbox_root.join("GameSave");
    fs::create_dir_all(&gamesave_dir).unwrap();

    let report = MicrosoftStoreAdapter::discover_from(&[xbox_root], &[], &detector);
    assert_eq!(report.installs.len(), 2);
    assert_eq!(report.warnings.len(), 0);

    let base = report
        .installs
        .iter()
        .find(|i| i.product == "ParadoxInteractive.ProjectTitus")
        .unwrap();
    assert_eq!(base.kind, InstallKind::BaseGame);
    assert_eq!(base.parent_product, None);
    assert_eq!(base.name, "Crusader Kings III");
    assert_eq!(base.store_version.as_deref(), Some("1.1.306.0"));
    assert_eq!(base.executables, vec!["launcher/bin/Paradox Launcher.exe"]);
    assert!(!base.capabilities.relocatable);
    assert!(base.capabilities.accepts_new_files);

    let dlc = report
        .installs
        .iter()
        .find(|i| i.product == "ParadoxInteractive.CrusaderKingsIIIExpansion1")
        .unwrap();
    assert_eq!(dlc.kind, InstallKind::AddOn);
    assert_eq!(
        dlc.parent_product.as_deref(),
        Some("ParadoxInteractive.ProjectTitus")
    );
    assert_eq!(dlc.name, "Crusader Kings III: Royal Court");
}

// ---------------------------------------------------------------------------
// 6. .GamingRoot parsing
// ---------------------------------------------------------------------------

#[test]
fn test_gaming_root_parsing() {
    // Valid file: RGBX, count 1, "XboxGames\0"
    let mut data = vec![0x52, 0x47, 0x42, 0x58]; // RGBX
    data.extend_from_slice(&1u32.to_le_bytes()); // count = 1
    for u in "XboxGames".encode_utf16() {
        data.extend_from_slice(&u.to_le_bytes());
    }
    data.extend_from_slice(&0u16.to_le_bytes()); // NUL terminator

    let paths = parse_gaming_root(&data).expect("should parse valid .GamingRoot");
    assert_eq!(paths, vec!["XboxGames"]);

    // Truncated file (fewer than 8 bytes)
    let short_data = vec![0x52, 0x47, 0x42];
    assert!(parse_gaming_root(&short_data).is_err());

    // Truncated file in string data
    let mut truncated_data = vec![0x52, 0x47, 0x42, 0x58];
    truncated_data.extend_from_slice(&2u32.to_le_bytes()); // expects 2 strings
    for u in "XboxGames".encode_utf16() {
        truncated_data.extend_from_slice(&u.to_le_bytes());
    }
    truncated_data.extend_from_slice(&0u16.to_le_bytes()); // only 1 string provided
    assert!(parse_gaming_root(&truncated_data).is_err());
}

// ---------------------------------------------------------------------------
// 7. Text VDF: nesting, escapes, libraryfolders with 2 libraries
// ---------------------------------------------------------------------------

#[test]
fn test_vdf_text_parsing() {
    let input = r#"
    // Root libraryfolders configuration
    "libraryfolders"
    {
        "0"
        {
            "path"		"C:\\Program Files (x86)\\Steam"
            "label"		"Main \"Library\""
            "apps"
            {
                "228980"		"12345"
            }
        }
        "1"
        {
            "path"		"D:\\SteamLibrary"
            "apps"
            {
                "2379780"		"67890"
            }
        }
    }
    "#;

    let parsed = parse_vdf_text(input).expect("should parse valid VDF");
    let lib_folders = get_object(&parsed, "libraryfolders").expect("libraryfolders exists");

    let lib0 = get_object(lib_folders, "0").expect("lib 0 exists");
    assert_eq!(
        get_str(lib0, "path"),
        Some("C:\\Program Files (x86)\\Steam")
    );
    assert_eq!(get_str(lib0, "label"), Some("Main \"Library\""));

    let lib1 = get_object(lib_folders, "1").expect("lib 1 exists");
    assert_eq!(get_str(lib1, "path"), Some("D:\\SteamLibrary"));
}

// ---------------------------------------------------------------------------
// 8. Binary appinfo.vdf: v28 and v29, classification, skipping, truncation
// ---------------------------------------------------------------------------

fn build_v28_synthetic_appinfo() -> Vec<u8> {
    let mut data = Vec::new();
    // Magic v28
    data.extend_from_slice(&0x07564428u32.to_le_bytes());
    // Universe
    data.extend_from_slice(&1u32.to_le_bytes());

    // App 100: game
    add_v28_entry(&mut data, 100, "game", None, &[("0", "game.exe", None)]);
    // App 200: dlc
    add_v28_entry(&mut data, 200, "dlc", Some("100"), &[]);
    // App 300: application / tool
    add_v28_entry(
        &mut data,
        300,
        "application",
        None,
        &[("0", "tool.exe", Some("windows"))],
    );

    // End sentinel
    data.extend_from_slice(&0u32.to_le_bytes());
    data
}

fn add_v28_entry(
    data: &mut Vec<u8>,
    app_id: u32,
    app_type: &str,
    parent: Option<&str>,
    launch: &[(&str, &str, Option<&str>)],
) {
    data.extend_from_slice(&app_id.to_le_bytes());

    // Build entry payload
    let mut entry_payload = Vec::new();
    // 60 fixed bytes: info_state(4), last_updated(4), pics_token(8), sha1(20), change(4), sha2(20)
    entry_payload.extend_from_slice(&[0u8; 60]);

    // Binary KV for entry
    // "appinfo" root object
    entry_payload.push(0x00); // object
    entry_payload.extend_from_slice(b"appinfo\0");

    // "common" object
    entry_payload.push(0x00);
    entry_payload.extend_from_slice(b"common\0");

    // "type" string
    entry_payload.push(0x01);
    entry_payload.extend_from_slice(b"type\0");
    entry_payload.extend_from_slice(app_type.as_bytes());
    entry_payload.push(0x00);

    if let Some(p) = parent {
        entry_payload.push(0x01);
        entry_payload.extend_from_slice(b"parent\0");
        entry_payload.extend_from_slice(p.as_bytes());
        entry_payload.push(0x00);
    }
    entry_payload.push(0x08); // end of common

    // "config" object
    if !launch.is_empty() {
        entry_payload.push(0x00);
        entry_payload.extend_from_slice(b"config\0");

        entry_payload.push(0x00);
        entry_payload.extend_from_slice(b"launch\0");

        for (idx, exe, oslist) in launch {
            entry_payload.push(0x00);
            entry_payload.extend_from_slice(idx.as_bytes());
            entry_payload.push(0x00);

            entry_payload.push(0x01);
            entry_payload.extend_from_slice(b"executable\0");
            entry_payload.extend_from_slice(exe.as_bytes());
            entry_payload.push(0x00);

            if let Some(os) = oslist {
                entry_payload.push(0x01);
                entry_payload.extend_from_slice(b"oslist\0");
                entry_payload.extend_from_slice(os.as_bytes());
                entry_payload.push(0x00);
            }

            entry_payload.push(0x08); // end of launch entry
        }

        entry_payload.push(0x08); // end of launch
        entry_payload.push(0x08); // end of config
    }

    entry_payload.push(0x08); // end of appinfo
    entry_payload.push(0x08); // end of root

    // Write size of entry payload
    let size = entry_payload.len() as u32;
    data.extend_from_slice(&size.to_le_bytes());
    data.extend_from_slice(&entry_payload);
}

fn build_v29_synthetic_appinfo() -> Vec<u8> {
    let mut data = Vec::new();
    // Magic v29
    data.extend_from_slice(&0x07564429u32.to_le_bytes());
    // Universe
    data.extend_from_slice(&1u32.to_le_bytes());
    // String table offset placeholder (at offset 8)
    data.extend_from_slice(&0i64.to_le_bytes());

    // Strings table to construct
    let strings = vec![
        "appinfo".to_string(),    // 0
        "common".to_string(),     // 1
        "type".to_string(),       // 2
        "parent".to_string(),     // 3
        "config".to_string(),     // 4
        "launch".to_string(),     // 5
        "0".to_string(),          // 6
        "executable".to_string(), // 7
        "oslist".to_string(),     // 8
    ];

    // App 100: game
    add_v29_entry(&mut data, 100, "game", None, &[("0", "game.exe", None)]);
    // App 200: dlc
    add_v29_entry(&mut data, 200, "dlc", Some("100"), &[]);
    // App 300: application / tool
    add_v29_entry(
        &mut data,
        300,
        "application",
        None,
        &[("0", "tool.exe", Some("windows"))],
    );

    // End sentinel
    data.extend_from_slice(&0u32.to_le_bytes());

    // Write string table
    let str_offset = data.len() as i64;
    // Patch string table offset
    data[8..16].copy_from_slice(&str_offset.to_le_bytes());

    data.extend_from_slice(&(strings.len() as u32).to_le_bytes());
    for s in &strings {
        data.extend_from_slice(s.as_bytes());
        data.push(0x00);
    }

    data
}

fn add_v29_entry(
    data: &mut Vec<u8>,
    app_id: u32,
    app_type: &str,
    parent: Option<&str>,
    launch: &[(&str, &str, Option<&str>)],
) {
    data.extend_from_slice(&app_id.to_le_bytes());

    let mut entry_payload = Vec::new();
    entry_payload.extend_from_slice(&[0u8; 60]);

    // "appinfo" root
    entry_payload.push(0x00);
    entry_payload.extend_from_slice(&0u32.to_le_bytes()); // index 0: "appinfo"

    // "common"
    entry_payload.push(0x00);
    entry_payload.extend_from_slice(&1u32.to_le_bytes()); // index 1: "common"

    // "type"
    entry_payload.push(0x01);
    entry_payload.extend_from_slice(&2u32.to_le_bytes()); // index 2: "type"
    entry_payload.extend_from_slice(app_type.as_bytes());
    entry_payload.push(0x00);

    if let Some(p) = parent {
        entry_payload.push(0x01);
        entry_payload.extend_from_slice(&3u32.to_le_bytes()); // index 3: "parent"
        entry_payload.extend_from_slice(p.as_bytes());
        entry_payload.push(0x00);
    }
    entry_payload.push(0x08); // end common

    if !launch.is_empty() {
        entry_payload.push(0x00);
        entry_payload.extend_from_slice(&4u32.to_le_bytes()); // index 4: "config"

        entry_payload.push(0x00);
        entry_payload.extend_from_slice(&5u32.to_le_bytes()); // index 5: "launch"

        for (_idx, exe, oslist) in launch {
            entry_payload.push(0x00);
            entry_payload.extend_from_slice(&6u32.to_le_bytes()); // index 6: "0"

            entry_payload.push(0x01);
            entry_payload.extend_from_slice(&7u32.to_le_bytes()); // index 7: "executable"
            entry_payload.extend_from_slice(exe.as_bytes());
            entry_payload.push(0x00);

            if let Some(os) = oslist {
                entry_payload.push(0x01);
                entry_payload.extend_from_slice(&8u32.to_le_bytes()); // index 8: "oslist"
                entry_payload.extend_from_slice(os.as_bytes());
                entry_payload.push(0x00);
            }
            entry_payload.push(0x08);
        }

        entry_payload.push(0x08);
        entry_payload.push(0x08);
    }

    entry_payload.push(0x08); // end appinfo
    entry_payload.push(0x08); // end root

    let size = entry_payload.len() as u32;
    data.extend_from_slice(&size.to_le_bytes());
    data.extend_from_slice(&entry_payload);
}

#[test]
fn test_appinfo_v28_parsing_and_classification() {
    let bytes = build_v28_synthetic_appinfo();
    let apps = parse_appinfo_vdf(&bytes, None).expect("v28 should parse");
    assert_eq!(apps.len(), 3);

    let app100 = apps.get(&100).unwrap();
    assert_eq!(app100.app_type.as_deref(), Some("game"));
    assert_eq!(app100.executables, vec!["game.exe"]);

    let app200 = apps.get(&200).unwrap();
    assert_eq!(app200.app_type.as_deref(), Some("dlc"));
    assert_eq!(app200.parent.as_deref(), Some("100"));

    let app300 = apps.get(&300).unwrap();
    assert_eq!(app300.app_type.as_deref(), Some("application"));
    assert_eq!(app300.executables, vec!["tool.exe"]);
}

#[test]
fn test_appinfo_v29_parsing_and_classification() {
    let bytes = build_v29_synthetic_appinfo();
    let apps = parse_appinfo_vdf(&bytes, None).expect("v29 should parse");
    assert_eq!(apps.len(), 3);

    let app100 = apps.get(&100).unwrap();
    assert_eq!(app100.app_type.as_deref(), Some("game"));
    assert_eq!(app100.executables, vec!["game.exe"]);

    let app200 = apps.get(&200).unwrap();
    assert_eq!(app200.app_type.as_deref(), Some("dlc"));
    assert_eq!(app200.parent.as_deref(), Some("100"));

    let app300 = apps.get(&300).unwrap();
    assert_eq!(app300.app_type.as_deref(), Some("application"));
    assert_eq!(app300.executables, vec!["tool.exe"]);
}

#[test]
fn test_appinfo_skip_unrequested_apps_by_size() {
    let bytes = build_v29_synthetic_appinfo();
    let mut targets = HashSet::new();
    targets.insert(200);

    let apps = parse_appinfo_vdf(&bytes, Some(&targets)).expect("should parse target app");
    assert_eq!(apps.len(), 1);
    assert!(apps.contains_key(&200));
    assert!(!apps.contains_key(&100));
    assert!(!apps.contains_key(&300));
}

#[test]
fn test_appinfo_truncated_file_returns_error_not_panic() {
    let bytes = build_v29_synthetic_appinfo();
    // Truncate halfway through
    let truncated = &bytes[..bytes.len() - 30];
    let result = parse_appinfo_vdf(truncated, None);
    assert!(result.is_err(), "truncated file must return Err");
}

// ---------------------------------------------------------------------------
// 9. Real-machine tests (#[ignore])
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn test_real_machine_discovery() {
    println!("\n=== RUNNING REAL MACHINE DISCOVERY ===");
    let report = discover_all();

    println!("Total installs discovered: {}", report.installs.len());
    println!("Total warnings: {}", report.warnings.len());

    let mut by_store = std::collections::BTreeMap::new();
    for install in &report.installs {
        by_store
            .entry(install.store.to_string())
            .or_insert_with(Vec::new)
            .push(install);
    }

    for (store, list) in by_store {
        println!("\nStore: {} ({} installs)", store, list.len());
        let base_games: Vec<_> = list
            .iter()
            .filter(|i| i.kind == InstallKind::BaseGame)
            .collect();
        let add_ons: Vec<_> = list
            .iter()
            .filter(|i| i.kind == InstallKind::AddOn)
            .collect();
        let tools: Vec<_> = list
            .iter()
            .filter(|i| i.kind == InstallKind::Tool)
            .collect();

        println!("  Base games: {}", base_games.len());
        for bg in base_games {
            let addon_count = add_ons
                .iter()
                .filter(|a| a.parent_product.as_deref() == Some(&bg.product))
                .count();
            println!(
                "    - {} (product: {}, add-ons: {}, volume: {:?})",
                bg.name,
                bg.product,
                addon_count,
                bg.volume.as_ref().map(|v| &v.id)
            );
        }

        println!("  Add-ons: {}", add_ons.len());
        for ao in add_ons {
            println!("    - {} (parent: {:?})", ao.name, ao.parent_product);
        }

        println!("  Tools: {}", tools.len());
        for t in tools {
            println!("    - {}", t.name);
        }
    }

    if !report.warnings.is_empty() {
        println!("\nWarnings:");
        for w in &report.warnings {
            println!("  [{}] {}", w.store, w.message);
        }
    }
    println!("=== END REAL MACHINE DISCOVERY ===\n");
}

#[test]
#[ignore]
fn test_real_machine_steam_appinfo() {
    let steam_root = agora_core::game_discovery::platform::find_steam_root();
    println!("Steam root: {:?}", steam_root);
    let Some(root) = steam_root else {
        println!("Steam is not installed on this machine.");
        return;
    };

    let appinfo_path = root.join("appcache").join("appinfo.vdf");
    if !appinfo_path.exists() {
        println!("appinfo.vdf not found at {}", appinfo_path.display());
        return;
    }

    let bytes = fs::read(&appinfo_path).expect("failed to read appinfo.vdf");
    println!("Read appinfo.vdf ({} bytes)", bytes.len());

    let apps = parse_appinfo_vdf(&bytes, None).expect("failed to parse real appinfo.vdf");
    println!("Parsed {} apps from appinfo.vdf", apps.len());

    for (app_id, meta) in apps.iter().take(20) {
        println!(
            "App {}: type={:?}, parent={:?}, executables={:?}",
            app_id, meta.app_type, meta.parent, meta.executables
        );
    }
}
