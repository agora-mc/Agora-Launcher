use agora_core::game_discovery::discover_all;
use agora_core::game_registry::{identify_installs, GameRegistry, PackageSource};
use agora_game_api::{GameId, StoreId};

#[test]
fn registers_minecraft_and_skyrim_packages_together() {
    let mut builder = GameRegistry::builder();

    builder
        .add(
            PackageSource::Compiled {
                crate_name: "agora-game-minecraft".to_string(),
            },
            agora_game_minecraft::game_package(),
        )
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
