//! Integration tests for Thunderstore package installation (BepInEx games).

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agora_core::content_store::add_archive;
use agora_core::content_thunderstore::{install_thunderstore, ThunderstoreInstallError};
use agora_core::ctx::Ctx;
use agora_core::game_base::BaseMode;
use agora_core::game_discovery::{DiscoveredInstall, InstallCapabilities};
use agora_core::game_instance::create;
use agora_core::game_registry::{
    GameRegistry, IdentifiedInstall, PackageSource, RuntimeResolution,
};
use agora_game_api::{
    ContentLayout, DeploymentStrategy, GameDefinition, GameId, GamePackage, InstallId, InstallKind,
    LaunchRecipe, PackageDefinition, RelPath, RuntimeIdentity, StoreId, StoreIdentifier,
};
use tempfile::TempDir;

struct ValheimPackage(PackageDefinition);
impl GamePackage for ValheimPackage {
    fn definition(&self) -> &PackageDefinition {
        &self.0
    }
}

fn valheim_definition() -> GameDefinition {
    GameDefinition {
        mo2_game_name: None,
        id: GameId::new("valheim").unwrap(),
        name: "Valheim".into(),
        stores: vec![StoreIdentifier {
            store: StoreId::new("steam").unwrap(),
            product: "892970".into(),
        }],
        version_sources: vec![],
        deployment: DeploymentStrategy::Redirect,
        content_rules: vec![],
        native_code_patterns: vec![],
        framework_ids: vec![],
        tool_ids: vec![],
        launch: Some(LaunchRecipe {
            executable: agora_game_api::GamePath::Runtime {
                path: RelPath::new("valheim.exe").unwrap(),
            },
            arguments: vec![],
            environment: Default::default(),
            working_directory: agora_game_api::GamePath::Runtime {
                path: RelPath::default(),
            },
        }),
        log_paths: vec![],
        crash_paths: vec![],
        user_files: vec![],
        save_paths: vec![],
        linked_archive_patterns: vec![
            "valheim_Data/*.assets".into(),
            "valheim_Data/*.resS".into(),
            "valheim_Data/*.resource".into(),
            "valheim_Data/StreamingAssets/**/*.bundle".into(),
        ],
        declared_writes: vec!["BepInEx/config/**".into()],
        copy_patterns: vec![],
        excluded_paths: vec![],
        plugin_list: None,
        runtime_files: Vec::new(),
        save_location: Vec::new(),
        launch_alternatives: Vec::new(),
        content_layout: Some(ContentLayout {
            data_path: RelPath::default(),
            data_markers: vec![],
            root_markers: vec![],
            thunderstore_bepinex: true,
        }),
    }
}

fn create_zip(path: &Path, entries: &[(&str, &[u8])]) {
    let file = File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, content) in entries {
        zip.start_file(*name, options).unwrap();
        zip.write_all(content).unwrap();
    }
    zip.finish().unwrap();
}

fn setup_valheim_instance(tmp: &TempDir) -> (Ctx, String) {
    let def = valheim_definition();
    let ctx = agora_core::ctx::CoreContext::for_testing(tmp.path().join("app_data"));
    agora_core::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();

    let mut builder = GameRegistry::builder();
    builder
        .add(
            PackageSource::Compiled {
                crate_name: "test".into(),
            },
            Arc::new(ValheimPackage(PackageDefinition {
                id: "valheim.tracer".into(),
                version: semver::Version::new(0, 1, 0),
                api_range: semver::VersionReq::parse(">=0.1, <0.2").unwrap(),
                parents: vec![],
                games: vec![def.clone()],
                frameworks: vec![],
                tools: vec![],
            })),
        )
        .unwrap();
    let ctx = ctx.with_games(Arc::new(builder.build()));

    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(&install_dir).unwrap();
    std::fs::write(install_dir.join("valheim.exe"), b"exe").unwrap();

    let store = StoreId::new("steam").unwrap();
    let runtime = RuntimeIdentity {
        game: def.id.clone(),
        store: store.clone(),
        version: "0.218.15".into(),
        build: None,
    };
    let volume =
        agora_core::game_discovery::volume::VolumeDetector::new().get_volume_info(&install_dir);

    let install = IdentifiedInstall {
        game: def.id.clone(),
        install_id: InstallId::new("steam:892970").unwrap(),
        discovered: DiscoveredInstall {
            store,
            product: "892970".into(),
            name: "Valheim".into(),
            kind: InstallKind::BaseGame,
            parent_product: None,
            location: install_dir,
            store_version: Some(runtime.version.clone()),
            store_build: None,
            executables: vec!["valheim.exe".into()],
            capabilities: InstallCapabilities {
                executables_readable: true,
                accepts_new_files: true,
                relocatable: true,
            },
            volume,
        },
        add_ons: vec![],
        runtime: RuntimeResolution::Identified {
            runtime,
            source: "executable".into(),
        },
    };

    let inst = create(
        &ctx,
        &install,
        &def,
        "Valheim Instance",
        None,
        BaseMode::Linked,
        &|_| {},
    )
    .unwrap();
    (ctx, inst.instance_id)
}

#[test]
fn two_packages_config_files_colliding_is_a_clear_error_at_add_time_naming_both() {
    let tmp = TempDir::new().unwrap();
    let (ctx, instance_id) = setup_valheim_instance(&tmp);

    // Package A
    let zip_a_path = tmp.path().join("AuthorA-ModA-1.0.0.zip");
    create_zip(
        &zip_a_path,
        &[
            (
                "manifest.json",
                br#"{"name": "ModA", "version_number": "1.0.0", "dependencies": []}"#,
            ),
            ("config/shared.cfg", b"config from mod a"),
        ],
    );
    let item_a = add_archive(&ctx, &zip_a_path, None)
        .unwrap()
        .item()
        .item_id
        .clone();

    // Package B
    let zip_b_path = tmp.path().join("AuthorB-ModB-1.0.0.zip");
    create_zip(
        &zip_b_path,
        &[
            (
                "manifest.json",
                br#"{"name": "ModB", "version_number": "1.0.0", "dependencies": []}"#,
            ),
            ("config/shared.cfg", b"config from mod b"),
        ],
    );
    let item_b = add_archive(&ctx, &zip_b_path, None)
        .unwrap()
        .item()
        .item_id
        .clone();

    // Add package A
    let outcome_a = install_thunderstore(&ctx, &instance_id, &item_a).unwrap();
    assert_eq!(outcome_a.package_id, "AuthorA-ModA");

    // Add package B -> must fail due to config/shared.cfg collision
    let err = install_thunderstore(&ctx, &instance_id, &item_b).unwrap_err();
    match &err {
        ThunderstoreInstallError::FileCollision {
            path,
            package,
            existing_package,
        } => {
            assert_eq!(path, "BepInEx/config/shared.cfg");
            assert!(package.contains("ModB"));
            assert!(existing_package.contains("ModA"));
        }
        other => panic!("expected FileCollision error, got {other:?}"),
    }

    let err_str = err.to_string();
    assert!(
        err_str.contains("AuthorB-ModB") || err_str.contains("ModB"),
        "error should name new package: {err_str}"
    );
    assert!(
        err_str.contains("AuthorA-ModA") || err_str.contains("ModA"),
        "error should name existing package: {err_str}"
    );
    assert!(
        err_str.contains("BepInEx/config/shared.cfg"),
        "error should name colliding path: {err_str}"
    );
}

#[test]
fn missing_dependencies_reported() {
    let tmp = TempDir::new().unwrap();
    let (ctx, instance_id) = setup_valheim_instance(&tmp);

    let zip_path = tmp.path().join("ValheimModding-Jotunn-2.30.2.zip");
    create_zip(
        &zip_path,
        &[
            (
                "manifest.json",
                br#"{
                    "name": "Jotunn",
                    "version_number": "2.30.2",
                    "dependencies": ["denikson-BepInExPack_Valheim-5.4.2100"]
                }"#,
            ),
            ("plugins/Jotunn.dll", b"jotunn dll content"),
        ],
    );
    let item = add_archive(&ctx, &zip_path, None)
        .unwrap()
        .item()
        .item_id
        .clone();

    let outcome = install_thunderstore(&ctx, &instance_id, &item).unwrap();
    assert_eq!(outcome.package_id, "ValheimModding-Jotunn");
    assert_eq!(outcome.version, "2.30.2");
    assert_eq!(
        outcome.missing_dependencies,
        vec!["denikson-BepInExPack_Valheim".to_string()]
    );
    assert!(outcome
        .summary
        .contains("plugins/ → BepInEx/plugins/ValheimModding-Jotunn/"));
}

fn count_stored_objects(ctx: &Ctx) -> usize {
    let objects_dir = ctx.paths.content_objects_dir();
    if !objects_dir.exists() {
        return 0;
    }
    let mut count = 0;
    for shard in std::fs::read_dir(&objects_dir).unwrap().flatten() {
        if shard.path().is_dir() {
            count += std::fs::read_dir(shard.path())
                .unwrap()
                .flatten()
                .filter(|e| e.path().is_file())
                .count();
        }
    }
    count
}

#[test]
#[ignore]
fn test_real_packages_import_and_placement_without_storing_new_objects() {
    let mods_dir = PathBuf::from(r"D:\Agora-bench\valheim-mods");
    if !mods_dir.exists() {
        eprintln!("Skipping: {mods_dir:?} not found");
        return;
    }

    let tmp = TempDir::new().unwrap();
    let (ctx, instance_id) = setup_valheim_instance(&tmp);

    let zips = [
        "denikson-BepInExPack_Valheim-5.4.2351.zip",
        "ValheimModding-Jotunn-2.30.2.zip",
        "Advize-PlantEverything-1.21.3.zip",
        "Azumatt-AzuClock-1.1.0.zip",
    ];

    let mut imported_items = Vec::new();
    for zip_name in &zips {
        let zip_path = mods_dir.join(zip_name);
        assert!(zip_path.exists(), "zip file {zip_path:?} must exist");
        let outcome = add_archive(&ctx, &zip_path, None).unwrap();
        imported_items.push((zip_name, outcome.item().item_id.clone()));
    }

    // Count stored objects in content store after archives are imported
    let initial_objects_count = count_stored_objects(&ctx);
    assert!(initial_objects_count > 0, "must have stored objects");

    // Install each package into the Valheim instance
    for (zip_name, item_id) in &imported_items {
        let outcome = install_thunderstore(&ctx, &instance_id, item_id)
            .unwrap_or_else(|e| panic!("failed to install {zip_name}: {e}"));

        let derived = &outcome.derived_item;
        let dest_paths: Vec<&str> = derived.files.iter().map(|f| f.path.as_str()).collect();

        if **zip_name == "denikson-BepInExPack_Valheim-5.4.2351.zip" {
            // pack goes to root without its top-level readme
            assert!(dest_paths.contains(&"winhttp.dll"));
            assert!(dest_paths.contains(&"doorstop_config.ini"));
            assert!(dest_paths.iter().any(|p| p.starts_with("BepInEx/core/")));
            assert!(!dest_paths.contains(&"README.md"));
            assert!(!dest_paths.contains(&"manifest.json"));
            assert!(!dest_paths
                .iter()
                .any(|p| p.starts_with("BepInExPack_Valheim")));
        } else if **zip_name == "ValheimModding-Jotunn-2.30.2.zip" {
            assert!(dest_paths.contains(&"BepInEx/plugins/ValheimModding-Jotunn/Jotunn.dll"));
        } else if **zip_name == "Azumatt-AzuClock-1.1.0.zip" {
            assert!(dest_paths.contains(&"BepInEx/plugins/Azumatt-AzuClock/AzuClock.dll"));
        } else if **zip_name == "Advize-PlantEverything-1.21.3.zip" {
            assert!(dest_paths
                .contains(&"BepInEx/plugins/Advize-PlantEverything/Advize_PlantEverything.dll"));
        }
    }

    // Verify: NO NEW OBJECTS ARE STORED! Every object is reused by derive_item.
    let final_objects_count = count_stored_objects(&ctx);
    assert_eq!(
        initial_objects_count, final_objects_count,
        "no new objects should be stored when deriving items"
    );
}

#[test]
fn manifest_text_is_a_package_only_when_well_formed() {
    use agora_core::content_thunderstore::parse_manifest_text;
    let not = |t: &str| assert!(parse_manifest_text(t.as_bytes()).is_none(), "{t}");
    not("not json at all");
    not(r#"{"version_number": "1.0.0", "dependencies": []}"#);
    not(r#"{"name": "Mod", "dependencies": []}"#);
    not(r#"{"name": "Mod", "version_number": "1.0.0"}"#);
    not(r#"{"name": "Mod", "version_number": "1.0.0", "dependencies": "none"}"#);
    not(r#"{"name": "Mod", "version_number": "1.0.0", "dependencies": [1]}"#);
    not(r#"["name", "Mod"]"#);
    // The name becomes a folder name: only Thunderstore's own characters.
    for name in ["../../evil", "a/b", "a\\b", "C:", "", "..", "x y"] {
        not(&format!(
            r#"{{"name": "{name}", "version_number": "1.0.0", "dependencies": []}}"#
        ));
    }
    // A hostile, deeply nested file is not a stack overflow: in a field placement ignores it is
    // skipped, in one it reads it is refused.
    let open = "[".repeat(200_000);
    let close = "]".repeat(200_000);
    let ignored =
        format!(r#"{{"name":"x","version_number":"1.0.0","dependencies":[],"z":{open}{close}}}"#);
    assert!(parse_manifest_text(ignored.as_bytes()).is_some());
    not(&format!(
        r#"{{"name":"x","version_number":"1.0.0","dependencies":{open}{close}}}"#
    ));

    let valid = "\u{feff}{\"name\": \"Jotunn\", \"version_number\": \"2.30.2\", \"website_url\": 5, \"dependencies\": [\"denikson-BepInExPack_Valheim-5.4.2100\"]}";
    let parsed = parse_manifest_text(valid.as_bytes()).expect("a BOM and unknown fields are fine");
    assert_eq!(parsed.name, "Jotunn");
    assert_eq!(parsed.version_number, "2.30.2");
    assert_eq!(
        parsed.dependencies,
        vec!["denikson-BepInExPack_Valheim-5.4.2100"]
    );
}
