// Adversarial tests for deployment (fixtures shared with game_deploy.rs).
#![allow(unused_imports)]

use std::path::Path;
use std::sync::Arc;

use agora_core::ctx::CoreContext;
use agora_core::game_base::{get_file_identity, BaseMode};
use agora_core::game_deploy::{
    add_content, deploy, deployment_dir, move_content, plan, remove_content, set_content_enabled,
    undeploy, DeployError, DeployMode, DeployOutcome, FileSource, Placement,
};
use agora_core::game_discovery::{DiscoveredInstall, DiscoveryReport, InstallCapabilities};
use agora_core::game_instance::{
    create, get_manifest, prepare_launch_with_discovery, GameInstanceRecord, InstanceError,
};
use agora_core::game_registry::{
    GameRegistry, IdentifiedInstall, PackageSource, RuntimeResolution,
};
use agora_game_api::{
    BaseReference, DeploymentStrategy, GameDefinition, GameId, GamePackage, GamePath, InstallId,
    InstallKind, LaunchRecipe, LaunchValue, PackageDefinition, RelPath, RuntimeIdentity, StoreId,
    StoreIdentifier,
};
use tempfile::TempDir;

struct TestPackage(PackageDefinition);

impl GamePackage for TestPackage {
    fn definition(&self) -> &PackageDefinition {
        &self.0
    }
}

fn register_test_game_with_features(def: GameDefinition) -> Arc<GameRegistry> {
    let mut builder = GameRegistry::builder();
    let pkg_def = PackageDefinition {
        id: format!("test.{}", def.id),
        version: semver::Version::new(0, 1, 0),
        api_range: semver::VersionReq::parse(">=0.1, <0.2").unwrap(),
        parents: vec![],
        games: vec![def],
        frameworks: vec![],
        tools: vec![],
    };
    builder
        .add(
            PackageSource::Compiled {
                crate_name: "test".into(),
            },
            Arc::new(TestPackage(pkg_def)),
        )
        .expect("register test package");
    Arc::new(builder.build())
}

fn make_test_definition() -> GameDefinition {
    GameDefinition {
        id: GameId::new("test-game").unwrap(),
        name: "Test Game".into(),
        stores: vec![StoreIdentifier {
            store: StoreId::new("steam").unwrap(),
            product: "12345".into(),
        }],
        version_sources: vec![],
        deployment: DeploymentStrategy::VirtualFileSystem,
        content_rules: vec![],
        native_code_patterns: vec![],
        framework_ids: vec![],
        tool_ids: vec![],
        launch: Some(LaunchRecipe {
            executable: GamePath::Runtime {
                path: RelPath::new("Game.exe").unwrap(),
            },
            arguments: vec![LaunchValue::Literal {
                value: "-test".into(),
            }],
            environment: Default::default(),
            working_directory: GamePath::Runtime {
                path: RelPath::default(),
            },
        }),
        log_paths: vec![],
        crash_paths: vec![],
        user_files: vec![],
        save_paths: vec![],
        linked_archive_patterns: vec!["Data/*.bsa".into()],
        declared_writes: vec!["writeable_base.txt".into()],
        excluded_paths: vec![],
        plugin_list: None,
        runtime_files: Vec::new(),
        launch_alternatives: Vec::new(),
        content_layout: None,
        copy_patterns: Vec::new(),
    }
}

fn setup_fake_install(dir: &Path) {
    std::fs::write(dir.join("Game.exe"), b"fake game binary").unwrap();
    std::fs::write(dir.join("base_file.txt"), b"initial base file").unwrap();
    std::fs::write(dir.join("writeable_base.txt"), b"initial writeable base").unwrap();
    let data_dir = dir.join("Data");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::write(data_dir.join("Skyrim.bsa"), b"BSA DATA").unwrap();
}

fn make_test_install(install_dir: &Path, readable: bool, relocatable: bool) -> IdentifiedInstall {
    let store_id = StoreId::new("steam").unwrap();
    let runtime = RuntimeIdentity {
        game: GameId::new("test-game").unwrap(),
        store: store_id.clone(),
        version: "1.0.0".into(),
        build: None,
    };
    let detector = agora_core::game_discovery::volume::VolumeDetector::new();
    let volume = detector.get_volume_info(install_dir);
    let install_id = InstallId::new("steam:12345").unwrap();
    let discovered = DiscoveredInstall {
        store: store_id,
        product: "12345".into(),
        name: "Test Game".into(),
        kind: InstallKind::BaseGame,
        parent_product: None,
        location: install_dir.to_path_buf(),
        store_version: Some(runtime.version.clone()),
        store_build: runtime.build.clone(),
        executables: vec!["Game.exe".into()],
        capabilities: InstallCapabilities {
            executables_readable: readable,
            accepts_new_files: true,
            relocatable,
        },
        volume,
    };
    IdentifiedInstall {
        game: runtime.game.clone(),
        install_id,
        discovered,
        add_ons: vec![],
        runtime: RuntimeResolution::Identified {
            runtime,
            source: "executable".into(),
        },
    }
}

fn create_test_context(tmp: &TempDir, def: &GameDefinition) -> CoreContext {
    let ctx = CoreContext::for_testing(tmp.path().join("app_data"));
    agora_core::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();
    let registry = register_test_game_with_features(def.clone());
    ctx.with_games(registry)
}

fn create_pinned_instance(
    ctx: &CoreContext,
    install: &IdentifiedInstall,
    def: &GameDefinition,
    name: &str,
) -> GameInstanceRecord {
    create(ctx, install, def, name, None, BaseMode::Linked, &|_| {}).expect("create instance")
}

fn add_content_folder(ctx: &CoreContext, name: &str, files: &[(&str, &[u8])]) -> String {
    let mod_dir = tempfile::tempdir().unwrap();
    for (rel, content) in files {
        let p = mod_dir.path().join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&p, content).unwrap();
    }
    let outcome = agora_core::content_store::add_folder(ctx, mod_dir.path(), Some(name)).unwrap();
    outcome.item().item_id.clone()
}

// ---------------------------------------------------------------- adversarial probes

fn setup(tmp: &TempDir) -> (CoreContext, GameDefinition, GameInstanceRecord) {
    let def = make_test_definition();
    let ctx = create_test_context(tmp, &def);
    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(&install_dir).unwrap();
    setup_fake_install(&install_dir);
    let install = make_test_install(&install_dir, true, true);
    let inst = create_pinned_instance(&ctx, &install, &def, "Probe");
    (ctx, def, inst)
}

fn record_path(ctx: &CoreContext, id: &str) -> std::path::PathBuf {
    deployment_dir(ctx, id)
        .unwrap()
        .unwrap()
        .parent()
        .unwrap()
        .join("deployment.json")
}

#[test]
fn probe_tampered_writable_source_path_cannot_delete_outside_the_instance() {
    let tmp = TempDir::new().unwrap();
    let (ctx, def, inst) = setup(&tmp);
    let item = add_content_folder(&ctx, "M", &[("a.txt", b"a")]);
    add_content(&ctx, &inst.instance_id, &item, None, None).unwrap();
    // Get a writable layer by harvesting a new file.
    deploy(&ctx, &inst.instance_id, &def, DeployMode::Links).unwrap();
    let game = deployment_dir(&ctx, &inst.instance_id).unwrap().unwrap();
    std::fs::write(game.join("new.txt"), b"game wrote").unwrap();
    undeploy(&ctx, &inst.instance_id).unwrap();
    deploy(&ctx, &inst.instance_id, &def, DeployMode::Links).unwrap();
    // Tamper: point the writable-sourced record at a victim outside the instance, then "delete" it.
    let victim = tmp.path().join("victim.txt");
    std::fs::write(&victim, b"keep me").unwrap();
    let rp = record_path(&ctx, &inst.instance_id);
    let text = std::fs::read_to_string(&rp).unwrap();
    let mut rec: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut touched = false;
    for f in rec["files"].as_array_mut().unwrap() {
        if f["path"] == "new.txt" {
            f["source"]["path"] = serde_json::json!(victim.to_string_lossy());
            touched = true;
        }
    }
    assert!(touched, "record had no new.txt: {text}");
    std::fs::write(&rp, serde_json::to_vec(&rec).unwrap()).unwrap();
    let game = deployment_dir(&ctx, &inst.instance_id).unwrap().unwrap();
    std::fs::remove_file(game.join("new.txt")).unwrap(); // the game deleted it
    let _ = undeploy(&ctx, &inst.instance_id);
    assert!(
        victim.exists(),
        "a tampered record made harvest delete a file outside the instance"
    );
}

#[test]
fn probe_tampered_record_path_cannot_escape() {
    let tmp = TempDir::new().unwrap();
    let (ctx, def, inst) = setup(&tmp);
    let item = add_content_folder(&ctx, "M", &[("a.txt", b"a")]);
    add_content(&ctx, &inst.instance_id, &item, None, None).unwrap();
    deploy(&ctx, &inst.instance_id, &def, DeployMode::Links).unwrap();
    let victim = tmp.path().join("victim2.txt");
    std::fs::write(&victim, b"keep me").unwrap();
    let rp = record_path(&ctx, &inst.instance_id);
    let mut rec: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&rp).unwrap()).unwrap();
    rec["files"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "path": "../../victim2.txt",
            "source": {"kind": "writable", "path": victim.to_string_lossy()},
            "placement": "copy", "size": 7, "modified_unix_ms": 0
        }));
    std::fs::write(&rp, serde_json::to_vec(&rec).unwrap()).unwrap();
    let _ = undeploy(&ctx, &inst.instance_id);
    assert!(victim.exists());
}

#[test]
fn probe_failed_harvest_keeps_the_games_writes() {
    let tmp = TempDir::new().unwrap();
    let (ctx, def, inst) = setup(&tmp);
    let item = add_content_folder(&ctx, "M", &[("a.txt", b"a")]);
    add_content(&ctx, &inst.instance_id, &item, None, None).unwrap();
    deploy(&ctx, &inst.instance_id, &def, DeployMode::Links).unwrap();
    let game = deployment_dir(&ctx, &inst.instance_id).unwrap().unwrap();
    std::fs::write(game.join("save-like.txt"), b"precious").unwrap();
    // Make the writable layer impossible to create: a FILE where the folder must go.
    let inst_dir = ctx.paths.instance_dir(&inst.instance_id).unwrap();
    std::fs::write(inst_dir.join("writable"), b"in the way").unwrap();
    assert!(
        undeploy(&ctx, &inst.instance_id).is_err(),
        "harvest should fail"
    );
    assert_eq!(
        std::fs::read(game.join("save-like.txt")).unwrap(),
        b"precious",
        "the write was lost"
    );
    assert!(record_path(&ctx, &inst.instance_id).exists());
}

#[test]
fn probe_record_for_another_instance_is_refused() {
    let tmp = TempDir::new().unwrap();
    let (ctx, def, inst) = setup(&tmp);
    let item = add_content_folder(&ctx, "M", &[("a.txt", b"a")]);
    add_content(&ctx, &inst.instance_id, &item, None, None).unwrap();
    deploy(&ctx, &inst.instance_id, &def, DeployMode::Links).unwrap();
    let game = deployment_dir(&ctx, &inst.instance_id).unwrap().unwrap();
    let rp = record_path(&ctx, &inst.instance_id);
    let mut rec: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&rp).unwrap()).unwrap();
    rec["instance_id"] = serde_json::json!("someone-else");
    std::fs::write(&rp, serde_json::to_vec(&rec).unwrap()).unwrap();
    assert!(undeploy(&ctx, &inst.instance_id).is_err());
    assert!(game.join("a.txt").exists());
}

#[test]
fn probe_changed_copy_survives_a_redeploy() {
    let tmp = TempDir::new().unwrap();
    let (ctx, def, inst) = setup(&tmp);
    let item = add_content_folder(&ctx, "M", &[("a.txt", b"a")]);
    add_content(&ctx, &inst.instance_id, &item, None, None).unwrap();
    deploy(&ctx, &inst.instance_id, &def, DeployMode::Links).unwrap();
    let game = deployment_dir(&ctx, &inst.instance_id).unwrap().unwrap();
    std::fs::write(
        game.join("writeable_base.txt"),
        b"game changed this declared write",
    )
    .unwrap();
    let _ = deploy(&ctx, &inst.instance_id, &def, DeployMode::Links).unwrap();
    let game = deployment_dir(&ctx, &inst.instance_id).unwrap().unwrap();
    assert_eq!(
        std::fs::read(game.join("writeable_base.txt")).unwrap(),
        b"game changed this declared write"
    );
}

#[test]
fn probe_base_write_through_link_is_reported_and_kept() {
    let tmp = TempDir::new().unwrap();
    let (ctx, def, inst) = setup(&tmp);
    let item = add_content_folder(&ctx, "M", &[("a.txt", b"a")]);
    add_content(&ctx, &inst.instance_id, &item, None, None).unwrap();
    deploy(&ctx, &inst.instance_id, &def, DeployMode::Links).unwrap();
    let game = deployment_dir(&ctx, &inst.instance_id).unwrap().unwrap();
    std::fs::write(
        game.join("base_file.txt"),
        b"written through the base link!",
    )
    .unwrap();
    let report = undeploy(&ctx, &inst.instance_id).unwrap();
    assert!(
        report
            .base_files_changed
            .iter()
            .any(|c| c.path.as_str() == "base_file.txt"),
        "{report:?}"
    );
    deploy(&ctx, &inst.instance_id, &def, DeployMode::Links).unwrap();
    let game = deployment_dir(&ctx, &inst.instance_id).unwrap().unwrap();
    assert_eq!(
        std::fs::read(game.join("base_file.txt")).unwrap(),
        b"written through the base link!"
    );
}

#[cfg(windows)]
#[test]
fn probe_deleting_a_deployed_mod_file_in_a_subfolder_keeps_the_object() {
    let tmp = TempDir::new().unwrap();
    let (ctx, def, inst) = setup(&tmp);
    let item = add_content_folder(&ctx, "M", &[("textures/deep/x.dds", b"pixels")]);
    add_content(&ctx, &inst.instance_id, &item, Some("Data"), None).unwrap();
    deploy(&ctx, &inst.instance_id, &def, DeployMode::Links).unwrap();
    let game = deployment_dir(&ctx, &inst.instance_id).unwrap().unwrap();
    let f = game.join("Data/textures/deep/x.dds");
    assert!(
        std::fs::OpenOptions::new().write(true).open(&f).is_err(),
        "in-place write must fail"
    );
    std::fs::remove_file(&f).expect("deleting a deployed link in a subfolder must work");
    let stored = agora_core::content_store::get_item(&ctx, &item).unwrap();
    let object = ctx.paths.content_object_path(&stored.files[0].sha256);
    assert_eq!(std::fs::read(object).unwrap(), b"pixels");
    // And the deletion becomes a whiteout that sticks.
    undeploy(&ctx, &inst.instance_id).unwrap();
    deploy(&ctx, &inst.instance_id, &def, DeployMode::Links).unwrap();
    deploy(&ctx, &inst.instance_id, &def, DeployMode::Links).unwrap();
    let game = deployment_dir(&ctx, &inst.instance_id).unwrap().unwrap();
    assert!(!game.join("Data/textures/deep/x.dds").exists());
}

#[test]
fn probe_empty_item_prefix_is_refused() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _def, inst) = setup(&tmp);
    add_content_folder(&ctx, "M", &[("a.txt", b"a")]);
    for p in ["", " ", "*"] {
        assert!(
            add_content(&ctx, &inst.instance_id, p, None, None).is_err(),
            "prefix {p:?} accepted"
        );
    }
}

#[test]
fn probe_hostile_instance_id_creates_nothing() {
    let tmp = TempDir::new().unwrap();
    let (ctx, def, _inst) = setup(&tmp);
    for id in ["../escape", "..\\escape", "", "a/b"] {
        assert!(deploy(&ctx, id, &def, DeployMode::Links).is_err());
        assert!(undeploy(&ctx, id).is_err());
    }
    assert!(!tmp.path().join("escape").exists());
}

#[test]
fn probe_a_minecraft_instance_does_not_block_content_removal() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _def, _inst) = setup(&tmp);
    let item = add_content_folder(&ctx, "M", &[("a.txt", b"a")]);
    // A Minecraft instance shares the instances root and the manifest file name, in its own format.
    let mc = ctx.paths.instances_root().join("Vanilla-MC");
    std::fs::create_dir_all(&mc).unwrap();
    std::fs::write(
        mc.join("instance_manifest.json"),
        br#"{"manifest_version":3,"instance_id":"Vanilla-MC","name":"Vanilla","minecraft_version":"1.21.1","loader":"vanilla","loader_version":"","is_locked":false,"mods":[]}"#,
    )
    .unwrap();
    agora_core::content_store::remove_item(&ctx, &item)
        .expect("a Minecraft instance blocked removal");
}
