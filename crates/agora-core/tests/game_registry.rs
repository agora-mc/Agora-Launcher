use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agora_core::game_discovery::{DiscoveredInstall, DiscoveryReport};
use agora_core::game_registry::{
    identify_installs, make_install_id, GameRegistry, GameRegistryError, PackageSource,
};
use agora_game_api::{
    DeploymentStrategy, FrameworkDefinition, FrameworkId, GameDefinition, GameId, GamePackage,
    InstallCapabilities, InstallKind, PackageDefinition, RelPath, StoreId, StoreIdentifier,
    ToolDefinition, ToolId, VersionSource,
};

struct TestPackage(PackageDefinition);

impl GamePackage for TestPackage {
    fn definition(&self) -> &PackageDefinition {
        &self.0
    }
}

fn make_package(
    id: &str,
    games: Vec<GameDefinition>,
    frameworks: Vec<FrameworkDefinition>,
    tools: Vec<ToolDefinition>,
) -> Arc<dyn GamePackage> {
    Arc::new(TestPackage(PackageDefinition {
        id: id.to_string(),
        version: semver::Version::new(0, 1, 0),
        api_range: semver::VersionReq::parse(">=0.1, <0.2").unwrap(),
        parents: vec![],
        games,
        frameworks,
        tools,
    }))
}

fn dummy_game(id: &str, stores: Vec<(&str, &str)>) -> GameDefinition {
    GameDefinition {
        id: GameId::new(id).unwrap(),
        name: id.to_string(),
        stores: stores
            .into_iter()
            .map(|(s, p)| StoreIdentifier {
                store: StoreId::new(s).unwrap(),
                product: p.to_string(),
            })
            .collect(),
        version_sources: vec![VersionSource::StoreRecord],
        deployment: DeploymentStrategy::Redirect,
        content_rules: vec![],
        native_code_patterns: vec![],
        framework_ids: vec![],
        tool_ids: vec![],
        launch: None,
        log_paths: vec![],
        crash_paths: vec![],
        user_files: vec![],
        save_paths: vec![],
        linked_archive_patterns: vec![],
        declared_writes: vec![],
        excluded_paths: vec![],
        plugin_list: None,
        runtime_files: Vec::new(),
        save_location: Vec::new(),
        launch_alternatives: Vec::new(),
        content_layout: None,
        copy_patterns: Vec::new(),
    }
}

fn dummy_launch_recipe() -> agora_game_api::LaunchRecipe {
    agora_game_api::LaunchRecipe {
        executable: agora_game_api::GamePath::Runtime {
            path: RelPath::new("tool.exe").unwrap(),
        },
        arguments: vec![],
        environment: BTreeMap::new(),
        working_directory: agora_game_api::GamePath::Runtime {
            path: RelPath::new("").unwrap(),
        },
    }
}

#[test]
fn registry_rejects_incompatible_api_range_leaving_registry_unchanged() {
    let mut builder = GameRegistry::builder();
    let pkg = Arc::new(TestPackage(PackageDefinition {
        id: "bad-api".to_string(),
        version: semver::Version::new(0, 1, 0),
        api_range: semver::VersionReq::parse(">=0.2, <0.3").unwrap(),
        parents: vec![],
        games: vec![dummy_game("game1", vec![("steam", "100")])],
        frameworks: vec![],
        tools: vec![],
    }));

    let err = builder
        .add(
            PackageSource::Compiled {
                crate_name: "test".into(),
            },
            pkg,
        )
        .unwrap_err();
    assert!(matches!(
        err,
        GameRegistryError::IncompatibleApiRange { .. }
    ));

    let reg = builder.build();
    assert_eq!(reg.games().count(), 0);
}

#[test]
fn registry_rejects_no_games_defined_leaving_registry_unchanged() {
    let mut builder = GameRegistry::builder();
    let pkg = make_package("no-games", vec![], vec![], vec![]);

    let err = builder
        .add(
            PackageSource::Compiled {
                crate_name: "test".into(),
            },
            pkg,
        )
        .unwrap_err();
    assert!(matches!(err, GameRegistryError::NoGamesDefined { .. }));

    let reg = builder.build();
    assert_eq!(reg.games().count(), 0);
}

#[test]
fn registry_rejects_game_already_registered_leaving_registry_unchanged() {
    let mut builder = GameRegistry::builder();
    let pkg1 = make_package(
        "pkg1",
        vec![dummy_game("dup-game", vec![("steam", "100")])],
        vec![],
        vec![],
    );
    builder
        .add(
            PackageSource::Compiled {
                crate_name: "p1".into(),
            },
            pkg1,
        )
        .unwrap();

    // Second package with same GameId
    let pkg2 = make_package(
        "pkg2",
        vec![dummy_game("dup-game", vec![("steam", "200")])],
        vec![],
        vec![],
    );
    let err = builder
        .add(
            PackageSource::Compiled {
                crate_name: "p2".into(),
            },
            pkg2,
        )
        .unwrap_err();
    assert!(matches!(
        err,
        GameRegistryError::GameAlreadyRegistered { ref game_id } if game_id.as_str() == "dup-game"
    ));

    let reg = builder.build();
    assert_eq!(reg.games().count(), 1);
    assert_eq!(
        reg.game_for_store_product(&StoreId::new("steam").unwrap(), "100"),
        Some(&GameId::new("dup-game").unwrap())
    );
    assert!(reg
        .game_for_store_product(&StoreId::new("steam").unwrap(), "200")
        .is_none());
}

#[test]
fn registry_rejects_store_product_already_claimed_leaving_registry_unchanged() {
    let mut builder = GameRegistry::builder();
    let pkg1 = make_package(
        "pkg1",
        vec![dummy_game("game1", vec![("steam", "shared-prod")])],
        vec![],
        vec![],
    );
    builder
        .add(
            PackageSource::Compiled {
                crate_name: "p1".into(),
            },
            pkg1,
        )
        .unwrap();

    let pkg2 = make_package(
        "pkg2",
        vec![dummy_game("game2", vec![("steam", "shared-prod")])],
        vec![],
        vec![],
    );
    let err = builder
        .add(
            PackageSource::Compiled {
                crate_name: "p2".into(),
            },
            pkg2,
        )
        .unwrap_err();
    assert!(matches!(
        err,
        GameRegistryError::StoreProductAlreadyClaimed { ref store, ref product, ref claimed_by }
        if store.as_str() == "steam" && product == "shared-prod" && claimed_by.as_str() == "game1"
    ));

    let reg = builder.build();
    assert_eq!(reg.games().count(), 1);
    assert!(reg.game(&GameId::new("game2").unwrap()).is_none());
}

#[test]
fn registry_rejects_undefined_game_in_framework_leaving_registry_unchanged() {
    let mut builder = GameRegistry::builder();
    let framework = FrameworkDefinition {
        id: FrameworkId::new("fw1").unwrap(),
        game: GameId::new("undefined-game").unwrap(),
        name: "Framework 1".into(),
        version: "1.0.0".into(),
        supported_runtimes: vec![],
        required_frameworks: vec![],
        content: vec![],
        launch: None,
    };
    let pkg = make_package(
        "pkg-bad-fw",
        vec![dummy_game("game1", vec![("steam", "101")])],
        vec![framework],
        vec![],
    );
    let err = builder
        .add(
            PackageSource::Compiled {
                crate_name: "p".into(),
            },
            pkg,
        )
        .unwrap_err();
    assert!(matches!(
        err,
        GameRegistryError::UndefinedGameInFramework { ref framework_id, ref game_id }
        if framework_id.as_str() == "fw1" && game_id.as_str() == "undefined-game"
    ));

    let reg = builder.build();
    assert_eq!(reg.games().count(), 0);
}

#[test]
fn registry_rejects_undefined_game_in_tool_leaving_registry_unchanged() {
    let mut builder = GameRegistry::builder();
    let tool = ToolDefinition {
        id: ToolId::new("tool1").unwrap(),
        game: GameId::new("undefined-game").unwrap(),
        name: "Tool 1".into(),
        launch: dummy_launch_recipe(),
        relevant_settings: vec![],
        after_tools: vec![],
    };
    let pkg = make_package(
        "pkg-bad-tool",
        vec![dummy_game("game1", vec![("steam", "101")])],
        vec![],
        vec![tool],
    );
    let err = builder
        .add(
            PackageSource::Compiled {
                crate_name: "p".into(),
            },
            pkg,
        )
        .unwrap_err();
    assert!(matches!(
        err,
        GameRegistryError::UndefinedGameInTool { ref tool_id, ref game_id }
        if tool_id.as_str() == "tool1" && game_id.as_str() == "undefined-game"
    ));

    let reg = builder.build();
    assert_eq!(reg.games().count(), 0);
}

#[test]
fn install_id_sanitizing_and_uniqueness() {
    let store_ms = StoreId::new("microsoft-store").unwrap();
    let store_steam = StoreId::new("steam").unwrap();
    let store_epic = StoreId::new("epic").unwrap();

    // Products that are already valid id parts are kept as they are.
    assert_eq!(
        make_install_id(&store_steam, "489830").as_str(),
        "steam:489830"
    );
    assert_eq!(
        make_install_id(&StoreId::gog(), "1711230643").as_str(),
        "gog:1711230643"
    );

    // Anything else is sanitised and suffixed with a hash of the original.
    let hashed = |store: &StoreId, product: &str, prefix: &str| {
        let id = make_install_id(store, product);
        let (head, hash) = id.as_str().rsplit_once('-').unwrap();
        assert_eq!(head, prefix, "{id}");
        assert_eq!(hash.len(), 8, "{id}");
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()), "{id}");
    };
    hashed(
        &store_ms,
        "ParadoxInteractive.ProjectTitus",
        "microsoft-store:paradoxinteractive-projecttitus",
    );
    hashed(
        &store_ms,
        "19886SeavenStudio.Brotato",
        "microsoft-store:19886seavenstudio-brotato",
    );
    hashed(&store_epic, "UE_5.6", "epic:ue_5-6");
    hashed(&store_epic, "Eel", "epic:eel");

    // Two products that sanitise alike must not share an id.
    assert_ne!(
        make_install_id(&store_epic, "A.B"),
        make_install_id(&store_epic, "a-b")
    );
    assert_ne!(
        make_install_id(&store_epic, "A.B"),
        make_install_id(&store_epic, "a.b")
    );

    // Hostile products never panic and always give a valid, bounded id.
    for product in [
        "",
        "_",
        "-",
        "_x",
        "...",
        "日本語",
        "a:b",
        &"é".repeat(200),
        &"_".repeat(90),
    ] {
        let id = make_install_id(&store_ms, product);
        assert!(id.as_str().len() <= agora_game_api::MAX_ID_LEN, "{id}");
    }

    // Long product id hash & truncation test
    let long_product = "A".repeat(80);
    let id_long = make_install_id(&store_steam, &long_product);
    assert!(id_long.as_str().len() <= agora_game_api::MAX_ID_LEN);
    assert!(id_long.as_str().starts_with("steam:a"));

    // Ensure two long products with same prefix but different ends produce distinct IDs
    let long1 = format!("{}1", "a".repeat(70));
    let long2 = format!("{}2", "a".repeat(70));
    assert_ne!(
        make_install_id(&store_steam, &long1),
        make_install_id(&store_steam, &long2)
    );

    // Uniqueness across sample products
    let sample_products = [
        ("microsoft-store", "ParadoxInteractive.ProjectTitus"),
        ("microsoft-store", "19886SeavenStudio.Brotato"),
        ("epic", "UE_5.6"),
        ("steam", "489830"),
        ("gog", "1711230643"),
        ("gog", "1162721350"),
    ];
    let mut seen = std::collections::HashSet::new();
    for (store, product) in sample_products {
        let id = make_install_id(&StoreId::new(store).unwrap(), product);
        assert!(seen.insert(id), "id must be unique across installs");
    }
}

#[test]
fn spike_machine_skyrim_steam_and_gog_identification() {
    let mut builder = GameRegistry::builder();
    let skyrim_def = GameDefinition {
        id: GameId::new("skyrim-se").unwrap(),
        name: "The Elder Scrolls V: Skyrim Special Edition".into(),
        stores: vec![
            StoreIdentifier {
                store: StoreId::steam(),
                product: "489830".into(),
            },
            StoreIdentifier {
                store: StoreId::gog(),
                product: "1711230643".into(),
            },
        ],
        version_sources: vec![
            VersionSource::Executable {
                path: RelPath::new("SkyrimSE.exe").unwrap(),
            },
            VersionSource::StoreRecord,
        ],
        deployment: DeploymentStrategy::VirtualFileSystem,
        content_rules: vec![],
        native_code_patterns: vec![],
        framework_ids: vec![],
        tool_ids: vec![],
        launch: None,
        log_paths: vec![],
        crash_paths: vec![],
        user_files: vec![],
        save_paths: vec![],
        linked_archive_patterns: vec![],
        declared_writes: vec![],
        excluded_paths: vec![],
        plugin_list: None,
        runtime_files: Vec::new(),
        save_location: Vec::new(),
        launch_alternatives: Vec::new(),
        content_layout: None,
        copy_patterns: Vec::new(),
    };

    builder
        .add(
            PackageSource::Compiled {
                crate_name: "skyrim".into(),
            },
            make_package("pkg", vec![skyrim_def], vec![], vec![]),
        )
        .unwrap();
    let registry = builder.build();

    let steam_install = DiscoveredInstall {
        store: StoreId::steam(),
        product: "489830".into(),
        name: "Skyrim Special Edition".into(),
        kind: InstallKind::BaseGame,
        parent_product: None,
        location: PathBuf::from("C:\\Games\\Steam\\Skyrim Special Edition"),
        store_version: None,
        store_build: Some("24914197".into()),
        executables: vec!["SkyrimSE.exe".into()],
        capabilities: InstallCapabilities {
            executables_readable: true,
            accepts_new_files: true,
            relocatable: true,
        },
        volume: None,
    };

    let gog_install = DiscoveredInstall {
        store: StoreId::gog(),
        product: "1711230643".into(),
        name: "Skyrim Special Edition GOG".into(),
        kind: InstallKind::BaseGame,
        parent_product: None,
        location: PathBuf::from("C:\\Games\\GOG\\Skyrim Special Edition"),
        store_version: Some("1.6.1179".into()),
        store_build: Some("57252778576965358".into()),
        executables: vec!["SkyrimSE.exe".into()],
        capabilities: InstallCapabilities {
            executables_readable: true,
            accepts_new_files: true,
            relocatable: true,
        },
        volume: None,
    };

    let gog_addon = DiscoveredInstall {
        store: StoreId::gog(),
        product: "1162721350".into(),
        name: "Anniversary Upgrade".into(),
        kind: InstallKind::AddOn,
        parent_product: Some("1711230643".into()),
        location: PathBuf::from("C:\\Games\\GOG\\Skyrim Special Edition"),
        store_version: None,
        store_build: None,
        executables: vec![],
        capabilities: InstallCapabilities {
            executables_readable: true,
            accepts_new_files: true,
            relocatable: true,
        },
        volume: None,
    };

    let report = DiscoveryReport {
        installs: vec![steam_install, gog_install, gog_addon],
        warnings: vec![],
    };

    let read_version = |path: &Path| -> Option<String> {
        let s = path.to_string_lossy();
        if s.contains("Steam") && s.ends_with("SkyrimSE.exe") {
            Some("1.6.1170.0".into())
        } else if s.contains("GOG") && s.ends_with("SkyrimSE.exe") {
            Some("1.6.1179.0".into())
        } else {
            None
        }
    };

    let inventory = identify_installs(&registry, &report, &read_version);
    assert_eq!(inventory.installs.len(), 2);
    assert!(inventory.unsupported.is_empty());

    let steam_identified = inventory
        .installs
        .iter()
        .find(|i| i.discovered.store == StoreId::steam())
        .unwrap();
    assert_eq!(steam_identified.add_ons.len(), 0);
    match &steam_identified.runtime {
        agora_core::game_registry::RuntimeResolution::Identified { runtime, source } => {
            assert_eq!(runtime.version, "1.6.1170.0");
            assert_eq!(runtime.build.as_deref(), Some("24914197"));
            assert_eq!(source, "executable");
        }
        _ => panic!("steam install should be identified"),
    }
    assert!(steam_identified.game_install().is_some());

    let gog_identified = inventory
        .installs
        .iter()
        .find(|i| i.discovered.store == StoreId::gog())
        .unwrap();
    assert_eq!(gog_identified.add_ons.len(), 1);
    assert_eq!(gog_identified.add_ons[0].product, "1162721350");
    match &gog_identified.runtime {
        agora_core::game_registry::RuntimeResolution::Identified { runtime, source } => {
            assert_eq!(runtime.version, "1.6.1179.0");
            assert_eq!(runtime.build.as_deref(), Some("57252778576965358"));
            assert_eq!(source, "executable");
        }
        _ => panic!("gog install should be identified"),
    }
    assert!(gog_identified.game_install().is_some());
}

#[test]
fn unreadable_executable_falls_back_to_store_record_and_nothing_usable_gives_unidentified() {
    let mut builder = GameRegistry::builder();
    let game = GameDefinition {
        id: GameId::new("test-game").unwrap(),
        name: "Test Game".into(),
        stores: vec![StoreIdentifier {
            store: StoreId::microsoft_store(),
            product: "prod1".into(),
        }],
        version_sources: vec![
            VersionSource::Executable {
                path: RelPath::new("game.exe").unwrap(),
            },
            VersionSource::StoreRecord,
        ],
        deployment: DeploymentStrategy::Redirect,
        content_rules: vec![],
        native_code_patterns: vec![],
        framework_ids: vec![],
        tool_ids: vec![],
        launch: None,
        log_paths: vec![],
        crash_paths: vec![],
        user_files: vec![],
        save_paths: vec![],
        linked_archive_patterns: vec![],
        declared_writes: vec![],
        excluded_paths: vec![],
        plugin_list: None,
        runtime_files: Vec::new(),
        save_location: Vec::new(),
        launch_alternatives: Vec::new(),
        content_layout: None,
        copy_patterns: Vec::new(),
    };
    builder
        .add(
            PackageSource::Compiled {
                crate_name: "test".into(),
            },
            make_package("pkg", vec![game], vec![], vec![]),
        )
        .unwrap();
    let registry = builder.build();

    // 1. Fallback to store_record when executables unreadable
    let install_fallback = DiscoveredInstall {
        store: StoreId::microsoft_store(),
        product: "prod1".into(),
        name: "Game With Store Version".into(),
        kind: InstallKind::BaseGame,
        parent_product: None,
        location: PathBuf::from("C:\\XboxGames\\Test"),
        store_version: Some("2.0.0".into()),
        store_build: Some("build42".into()),
        executables: vec!["game.exe".into()],
        capabilities: InstallCapabilities {
            executables_readable: false, // unreadable MS Store style
            accepts_new_files: true,
            relocatable: false,
        },
        volume: None,
    };

    let report = DiscoveryReport {
        installs: vec![install_fallback],
        warnings: vec![],
    };

    let read_version = |_p: &Path| -> Option<String> { None };
    let inventory = identify_installs(&registry, &report, &read_version);
    assert_eq!(inventory.installs.len(), 1);
    match &inventory.installs[0].runtime {
        agora_core::game_registry::RuntimeResolution::Identified { runtime, source } => {
            assert_eq!(runtime.version, "2.0.0");
            assert_eq!(runtime.build.as_deref(), Some("build42"));
            assert_eq!(source, "store_record");
        }
        _ => panic!("should identify from store_record"),
    }

    // 2. Nothing usable -> Unidentified with one reason per source
    let install_empty = DiscoveredInstall {
        store: StoreId::microsoft_store(),
        product: "prod1".into(),
        name: "Game With No Version".into(),
        kind: InstallKind::BaseGame,
        parent_product: None,
        location: PathBuf::from("C:\\XboxGames\\Test"),
        store_version: None,
        store_build: None,
        executables: vec!["game.exe".into()],
        capabilities: InstallCapabilities {
            executables_readable: false,
            accepts_new_files: true,
            relocatable: false,
        },
        volume: None,
    };

    let report_empty = DiscoveryReport {
        installs: vec![install_empty],
        warnings: vec![],
    };

    let inventory_empty = identify_installs(&registry, &report_empty, &read_version);
    assert_eq!(inventory_empty.installs.len(), 1);
    match &inventory_empty.installs[0].runtime {
        agora_core::game_registry::RuntimeResolution::Unidentified { reasons } => {
            assert_eq!(reasons.len(), 2);
            assert!(reasons[0].contains("executables not readable"));
            assert!(reasons[1].contains("store record missing version"));
        }
        _ => panic!("should be unidentified"),
    }
    assert!(inventory_empty.installs[0].game_install().is_none());
}

#[test]
fn escaping_executable_path_cannot_reach_a_definition() {
    // `RelPath` is checked on construction and on deserialization, so neither a
    // compiled package nor a JSON one can name a version source outside the
    // install; `identify_installs` only ever joins a checked relative path.
    for path in [
        "../x.exe",
        "game/../../x.exe",
        "C:/Windows/x.exe",
        "/bin/x",
        r"\\server\x.exe",
    ] {
        let json = serde_json::json!({"kind": "executable", "path": path});
        assert!(
            serde_json::from_value::<VersionSource>(json).is_err(),
            "{path} must not deserialize"
        );
    }
}

#[test]
fn unsupported_base_games_are_listed_addons_and_tools_are_not() {
    let mut builder = GameRegistry::builder();
    let game = dummy_game("known-game", vec![("steam", "100")]);
    builder
        .add(
            PackageSource::Compiled {
                crate_name: "p".into(),
            },
            make_package("pkg", vec![game], vec![], vec![]),
        )
        .unwrap();
    let registry = builder.build();

    let known_base = DiscoveredInstall {
        store: StoreId::steam(),
        product: "100".into(),
        name: "Known Game".into(),
        kind: InstallKind::BaseGame,
        parent_product: None,
        location: PathBuf::from("C:\\Games\\Known"),
        store_version: Some("1.0".into()),
        store_build: None,
        executables: vec![],
        capabilities: InstallCapabilities {
            executables_readable: true,
            accepts_new_files: true,
            relocatable: true,
        },
        volume: None,
    };

    let unknown_base = DiscoveredInstall {
        store: StoreId::steam(),
        product: "999".into(),
        name: "Unknown Base Game".into(),
        kind: InstallKind::BaseGame,
        parent_product: None,
        location: PathBuf::from("C:\\Games\\Unknown"),
        store_version: Some("1.0".into()),
        store_build: None,
        executables: vec![],
        capabilities: InstallCapabilities {
            executables_readable: true,
            accepts_new_files: true,
            relocatable: true,
        },
        volume: None,
    };

    let unknown_addon = DiscoveredInstall {
        store: StoreId::steam(),
        product: "998".into(),
        name: "Unknown DLC".into(),
        kind: InstallKind::AddOn,
        parent_product: Some("999".into()),
        location: PathBuf::from("C:\\Games\\Unknown\\DLC"),
        store_version: None,
        store_build: None,
        executables: vec![],
        capabilities: InstallCapabilities {
            executables_readable: true,
            accepts_new_files: true,
            relocatable: true,
        },
        volume: None,
    };

    let tool = DiscoveredInstall {
        store: StoreId::steam(),
        product: "997".into(),
        name: "Proton".into(),
        kind: InstallKind::Tool,
        parent_product: None,
        location: PathBuf::from("C:\\Tools\\Proton"),
        store_version: None,
        store_build: None,
        executables: vec![],
        capabilities: InstallCapabilities {
            executables_readable: true,
            accepts_new_files: true,
            relocatable: true,
        },
        volume: None,
    };

    let report = DiscoveryReport {
        installs: vec![known_base, unknown_base, unknown_addon, tool],
        warnings: vec![],
    };

    let read_version = |_p: &Path| -> Option<String> { None };
    let inventory = identify_installs(&registry, &report, &read_version);

    assert_eq!(inventory.installs.len(), 1);
    assert_eq!(inventory.installs[0].discovered.product, "100");

    assert_eq!(inventory.unsupported.len(), 1);
    assert_eq!(inventory.unsupported[0].product, "999");
    assert_eq!(inventory.unsupported[0].name, "Unknown Base Game");
}

#[test]
fn a_package_whose_second_game_conflicts_registers_none_of_its_games() {
    let mut builder = GameRegistry::builder();
    builder
        .add(
            PackageSource::Compiled {
                crate_name: "first".into(),
            },
            make_package(
                "first",
                vec![dummy_game("taken", vec![("steam", "1")])],
                vec![],
                vec![],
            ),
        )
        .unwrap();
    let err = builder
        .add(
            PackageSource::Plugin {
                plugin_id: "community".into(),
            },
            make_package(
                "community",
                vec![
                    dummy_game("fresh", vec![("gog", "9")]),
                    dummy_game("squatter", vec![("steam", "1")]),
                ],
                vec![],
                vec![],
            ),
        )
        .unwrap_err();
    assert!(matches!(
        err,
        GameRegistryError::StoreProductAlreadyClaimed { ref claimed_by, .. } if claimed_by.as_str() == "taken"
    ));
    let reg = builder.build();
    let ids: Vec<_> = reg.games().map(|g| g.id.as_str().to_string()).collect();
    assert_eq!(ids, ["taken"]);
    assert!(reg.game_for_store_product(&StoreId::gog(), "9").is_none());
}

#[test]
fn an_add_on_from_another_store_does_not_attach() {
    let mut builder = GameRegistry::builder();
    builder
        .add(
            PackageSource::Compiled {
                crate_name: "p".into(),
            },
            make_package(
                "p",
                vec![dummy_game("g", vec![("steam", "42")])],
                vec![],
                vec![],
            ),
        )
        .unwrap();
    let registry = builder.build();
    let install = |store: StoreId, product: &str, kind: InstallKind, parent: Option<&str>| {
        DiscoveredInstall {
            store,
            product: product.into(),
            name: product.into(),
            kind,
            parent_product: parent.map(Into::into),
            location: PathBuf::from("games"),
            store_version: Some("1".into()),
            store_build: None,
            executables: vec![],
            capabilities: InstallCapabilities {
                executables_readable: true,
                accepts_new_files: true,
                relocatable: true,
            },
            volume: None,
        }
    };
    let report = DiscoveryReport {
        installs: vec![
            install(StoreId::steam(), "42", InstallKind::BaseGame, None),
            install(StoreId::steam(), "43", InstallKind::AddOn, Some("42")),
            install(StoreId::gog(), "44", InstallKind::AddOn, Some("42")),
        ],
        warnings: vec![],
    };
    let inventory = identify_installs(&registry, &report, &|_| None);
    assert_eq!(inventory.installs.len(), 1);
    let add_ons: Vec<_> = inventory.installs[0]
        .add_ons
        .iter()
        .map(|a| a.product.as_str())
        .collect();
    assert_eq!(add_ons, ["43"]);
    assert!(inventory.installs[0]
        .game_install()
        .unwrap()
        .volume
        .is_none());
}

#[test]
fn a_registry_without_instance_services_says_so() {
    let error = match GameRegistry::empty().instance_backend() {
        Ok(_) => panic!("no package provides instances"),
        Err(error) => error,
    };
    assert_eq!(error.code(), "ERR_NO_GAME_PACKAGE");
}

#[test]
fn a_refused_compiled_package_attaches_no_services() {
    struct Loud;
    impl agora_core::game_hooks::CompiledServices for Loud {
        fn recover_at_startup(&self, _: &agora_core::app_paths::AppPaths) -> Vec<String> {
            vec!["ran".into()]
        }
    }
    let mut builder = GameRegistry::builder();
    builder
        .add_compiled(
            "first",
            make_package("first", vec![dummy_game("same", vec![])], vec![], vec![]),
            Arc::new(Loud),
        )
        .unwrap();
    assert!(builder
        .add_compiled(
            "second",
            make_package("second", vec![dummy_game("same", vec![])], vec![], vec![]),
            Arc::new(Loud),
        )
        .is_err());
    let registry = builder.build();
    let tmp = tempfile::tempdir().unwrap();
    let paths = agora_core::app_paths::AppPaths::from_root(tmp.path().to_path_buf());
    assert_eq!(registry.recover_at_startup(&paths), ["ran"]);
}
