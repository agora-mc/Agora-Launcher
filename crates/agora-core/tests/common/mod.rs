//! Shared test fixtures for the INI and save tests. The Skyrim SE definition is written out here
//! as the app's own package declares it (`agora-game-creation/data/package.json`), so core's tests
//! do not reach into a game package. The CLI tests check the shipped file itself.

use agora_game_api::{
    DeploymentStrategy, GameDefinition, GameId, GamePath, RelPath, SaveLocationRule, StoreId,
    StoreIdentifier, UserDataLocation, UserFileMapping, UserFileStrategy,
};

fn user(location: UserDataLocation, path: &str) -> GamePath {
    GamePath::UserData {
        location,
        path: RelPath::new(path).unwrap(),
    }
}

fn mapping(source: GamePath, instance_path: &str, store: StoreId) -> UserFileMapping {
    UserFileMapping::new(
        source,
        RelPath::new(instance_path).unwrap(),
        UserFileStrategy::JournaledSwap,
    )
    .with_stores(vec![store])
}

/// Skyrim SE as the shipped package declares it: four per-user files per store, and the save
/// location for each store.
pub fn skyrim_definition() -> GameDefinition {
    let documents = UserDataLocation::Documents;
    let local = UserDataLocation::LocalAppData;
    let mut user_files = Vec::new();
    for (store, folder) in [
        (StoreId::steam(), "Skyrim Special Edition"),
        (StoreId::gog(), "Skyrim Special Edition GOG"),
    ] {
        user_files.push(mapping(
            user(local.clone(), &format!("{folder}/Plugins.txt")),
            "user/Plugins.txt",
            store.clone(),
        ));
        for name in ["Skyrim.ini", "SkyrimPrefs.ini", "SkyrimCustom.ini"] {
            user_files.push(mapping(
                user(documents.clone(), &format!("My Games/{folder}/{name}")),
                &format!("user/{name}"),
                store.clone(),
            ));
        }
    }
    let save_location = [
        (StoreId::steam(), "Skyrim Special Edition"),
        (StoreId::gog(), "Skyrim Special Edition GOG"),
    ]
    .into_iter()
    .map(|(store, folder)| SaveLocationRule {
        ini: RelPath::new("user/Skyrim.ini").unwrap(),
        section: "General".into(),
        key: "SLocalSavePath".into(),
        own_value: r"Saves\Agora\{instance}\".into(),
        shared_dir: user(documents.clone(), &format!("My Games/{folder}/Saves")),
        relative_to: user(documents.clone(), &format!("My Games/{folder}")),
        stores: vec![store],
    })
    .collect();

    GameDefinition {
        id: GameId::new("skyrim-se").unwrap(),
        name: "The Elder Scrolls V: Skyrim Special Edition".to_string(),
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
        version_sources: vec![],
        deployment: DeploymentStrategy::VirtualFileSystem,
        content_rules: vec![],
        native_code_patterns: vec![],
        framework_ids: vec![],
        tool_ids: vec![],
        launch: None,
        log_paths: vec![],
        crash_paths: vec![],
        user_files,
        save_paths: vec![],
        linked_archive_patterns: vec![],
        declared_writes: vec![],
        excluded_paths: vec![],
        plugin_list: None,
        runtime_files: Vec::new(),
        launch_alternatives: Vec::new(),
        content_layout: None,
        copy_patterns: Vec::new(),
        save_location,
    }
}
