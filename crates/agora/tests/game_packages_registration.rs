use agora_core::game_discovery::discover_all;
use agora_core::game_registry::{identify_installs, GameRegistry, PackageSource};
use agora_game_api::{GameId, StoreId};

#[test]
fn registers_minecraft_and_skyrim_packages_together() {
    let mut builder = GameRegistry::builder();

    agora_game_minecraft::register_into(&mut builder)
        .expect("Minecraft package must register successfully");

    builder
        .add(
            PackageSource::Compiled {
                crate_name: "agora-game-creation".to_string(),
            },
            agora_game_creation::game_package(),
        )
        .expect("Skyrim package must register successfully");

    let registry = builder.build();

    let games: Vec<_> = registry.games().collect();
    assert_eq!(games.len(), 2);
    // Ordered by GameId: "minecraft" < "skyrim-se"
    assert_eq!(games[0].id, GameId::minecraft());
    assert_eq!(games[0].name, "Minecraft: Java Edition");
    assert_eq!(games[1].id, GameId::new("skyrim-se").unwrap());
    assert_eq!(games[1].name, "The Elder Scrolls V: Skyrim Special Edition");

    // Lookup by GameId
    let mc_def = registry
        .game(&GameId::minecraft())
        .expect("minecraft game def");
    assert_eq!(mc_def.id, GameId::minecraft());

    let skyrim_def = registry
        .game(&GameId::new("skyrim-se").unwrap())
        .expect("skyrim game def");
    assert_eq!(skyrim_def.id, GameId::new("skyrim-se").unwrap());

    // Lookup by store product
    assert_eq!(
        registry.game_for_store_product(&StoreId::steam(), "489830"),
        Some(&GameId::new("skyrim-se").unwrap())
    );
    assert_eq!(
        registry.game_for_store_product(&StoreId::gog(), "1711230643"),
        Some(&GameId::new("skyrim-se").unwrap())
    );
    assert_eq!(
        registry.game_for_store_product(&StoreId::mojang(), "minecraft"),
        None
    );

    // Package and source lookups
    assert!(registry.package_for(&GameId::minecraft()).is_some());
    assert!(registry
        .package_for(&GameId::new("skyrim-se").unwrap())
        .is_some());

    match registry.source_for(&GameId::minecraft()) {
        Some(PackageSource::Compiled { crate_name }) => {
            assert_eq!(crate_name, "agora-game-minecraft");
        }
        other => panic!("unexpected source for minecraft: {other:?}"),
    }

    match registry.source_for(&GameId::new("skyrim-se").unwrap()) {
        Some(PackageSource::Compiled { crate_name }) => {
            assert_eq!(crate_name, "agora-game-creation");
        }
        other => panic!("unexpected source for skyrim: {other:?}"),
    }
}

#[test]
#[ignore]
fn real_machine_discovery_and_inventory() {
    let mut builder = GameRegistry::builder();
    builder
        .add(
            PackageSource::Compiled {
                crate_name: "agora-game-minecraft".to_string(),
            },
            agora_game_minecraft::game_package(),
        )
        .unwrap();
    builder
        .add(
            PackageSource::Compiled {
                crate_name: "agora-game-creation".to_string(),
            },
            agora_game_creation::game_package(),
        )
        .unwrap();
    let registry = builder.build();

    let report = discover_all();
    let inventory = identify_installs(
        &registry,
        &report,
        &agora_core::game_discovery::file_version::read_file_version,
    );

    println!("=== REAL-MACHINE DISCOVERY & INVENTORY REPORT ===");
    println!("Supported games registered: {}", registry.games().count());
    for g in registry.games() {
        println!("  - {} ({})", g.name, g.id);
    }
    println!();
    println!("Identified installs: {}", inventory.installs.len());
    for inst in &inventory.installs {
        println!(
            "  * Game: {} | Store: {} | Product: {} | Name: {}",
            inst.game, inst.discovered.store, inst.discovered.product, inst.discovered.name
        );
        println!("    Location: {}", inst.discovered.location.display());
        println!("    InstallId: {}", inst.install_id);
        println!("    Add-ons attached: {}", inst.add_ons.len());
        for addon in &inst.add_ons {
            println!("      + Add-on: {} ({})", addon.name, addon.product);
        }
        match &inst.runtime {
            agora_core::game_registry::RuntimeResolution::Identified { runtime, source } => {
                println!(
                    "    Runtime: version={}, build={:?}, source={}",
                    runtime.version, runtime.build, source
                );
            }
            agora_core::game_registry::RuntimeResolution::Unidentified { reasons } => {
                println!("    Runtime: UNIDENTIFIED: {:?}", reasons);
            }
        }
    }
    println!();
    println!(
        "Unsupported base games found: {}",
        inventory.unsupported.len()
    );
    for u in &inventory.unsupported {
        println!("  - {} ({}, {})", u.name, u.store, u.product);
    }
    println!("=================================================");
}

#[test]
fn list_all_includes_minecraft_and_generic_game_instances() {
    let mut builder = GameRegistry::builder();
    agora_game_minecraft::register_into(&mut builder)
        .expect("Minecraft package must register successfully");
    builder
        .add(
            PackageSource::Compiled {
                crate_name: "agora-game-creation".to_string(),
            },
            agora_game_creation::game_package(),
        )
        .expect("Skyrim package must register successfully");
    let registry = std::sync::Arc::new(builder.build());

    let tmp = tempfile::tempdir().unwrap();
    let paths = agora_core::app_paths::AppPaths::from_root(tmp.path().to_path_buf());
    paths.create_required_dirs().unwrap();
    agora_core::db::init_local_state_db(&paths.local_state_db()).unwrap();

    let ctx = agora_core::ctx::Ctx::for_testing(tmp.path().to_path_buf()).with_games(registry);

    // 1. Insert Minecraft instance directly into user_instances
    let conn = rusqlite::Connection::open(paths.local_state_db()).unwrap();
    conn.execute(
        "INSERT INTO user_instances (instance_id, name, minecraft_version, loader, loader_version)
         VALUES ('mc-inst-1', 'My Minecraft', '1.20.1', 'fabric', '0.14.21')",
        [],
    )
    .unwrap();

    // 2. Create generic Skyrim instance
    let skyrim_def = ctx
        .games
        .game(&GameId::new("skyrim-se").unwrap())
        .expect("skyrim def");
    let install_dir = tmp.path().join("Skyrim");
    std::fs::create_dir_all(&install_dir).unwrap();
    let exe = install_dir.join("SkyrimSE.exe");
    std::fs::write(&exe, b"skyrim exe").unwrap();

    let install = agora_core::game_registry::IdentifiedInstall {
        install_id: agora_game_api::InstallId::new("steam:489830").unwrap(),
        game: GameId::new("skyrim-se").unwrap(),
        discovered: agora_core::game_discovery::DiscoveredInstall {
            store: StoreId::steam(),
            product: "489830".to_string(),
            name: "Skyrim".to_string(),
            kind: agora_core::game_discovery::InstallKind::BaseGame,
            parent_product: None,
            location: install_dir.clone(),
            store_version: Some("1.6.1170.0".to_string()),
            store_build: None,
            executables: vec!["SkyrimSE.exe".to_string()],
            capabilities: agora_core::game_discovery::InstallCapabilities {
                executables_readable: true,
                accepts_new_files: true,
                relocatable: true,
            },
            volume: None,
        },
        runtime: agora_core::game_registry::RuntimeResolution::Identified {
            runtime: agora_game_api::RuntimeIdentity {
                game: GameId::new("skyrim-se").unwrap(),
                store: StoreId::steam(),
                version: "1.6.1170.0".to_string(),
                build: None,
            },
            source: "exe".to_string(),
        },
        add_ons: vec![],
    };

    agora_core::game_instance::create(
        &ctx,
        &install,
        skyrim_def,
        "My Skyrim",
        Some("skyrim-inst-1".to_string()),
        agora_game_api::BaseMode::Copied,
        &|_| {},
    )
    .unwrap();

    // 3. list_all sees both instances
    let (all, warnings) = agora_core::game_instance::list_all(&ctx);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(all.len(), 2);

    let mc_summary = all
        .iter()
        .find(|i| i.instance_id == "mc-inst-1")
        .expect("mc-inst-1 found");
    assert_eq!(mc_summary.game, "minecraft");
    assert_eq!(mc_summary.name, "My Minecraft");
    assert_eq!(mc_summary.runtime, "1.20.1 fabric");
    assert_eq!(mc_summary.pinned, None);

    let skyrim_summary = all
        .iter()
        .find(|i| i.instance_id == "skyrim-inst-1")
        .expect("skyrim-inst-1 found");
    assert_eq!(skyrim_summary.game, "skyrim-se");
    assert_eq!(skyrim_summary.name, "My Skyrim");
    assert_eq!(skyrim_summary.runtime, "steam 1.6.1170.0");
    assert_eq!(skyrim_summary.pinned, Some(true));
}
