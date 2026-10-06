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
        content_layout: None,
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

#[test]
fn test_priority_and_reordering_and_mount_path() {
    let tmp = TempDir::new().unwrap();
    let def = make_test_definition();
    let ctx = create_test_context(&tmp, &def);

    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(&install_dir).unwrap();
    setup_fake_install(&install_dir);
    let install = make_test_install(&install_dir, true, true);
    let inst = create_pinned_instance(&ctx, &install, &def, "PriorityTest");

    // Create item 1: conflict.txt ("item 1") and item1_only.txt
    let item1_id = add_content_folder(
        &ctx,
        "Mod1",
        &[("conflict.txt", b"item 1"), ("item1_only.txt", b"only 1")],
    );

    // Create item 2: conflict.txt ("item 2")
    let item2_id = add_content_folder(&ctx, "Mod2", &[("conflict.txt", b"item 2")]);

    // Create item 3: mount_path = "Data", file = "mounted.txt"
    let item3_id = add_content_folder(&ctx, "Mod3", &[("mounted.txt", b"in Data")]);

    // Add in order: item1, item2, item3
    let l1 = add_content(&ctx, &inst.instance_id, &item1_id, None, None).unwrap();
    let l2 = add_content(&ctx, &inst.instance_id, &item2_id, None, None).unwrap();
    let _l3 = add_content(&ctx, &inst.instance_id, &item3_id, Some("Data"), None).unwrap();

    // 1. Check plan: item 2 overrides item 1 on conflict.txt
    let p = plan(&ctx, &inst.instance_id, &def, DeployMode::Links).unwrap();
    let conflict_override = p
        .overrides
        .iter()
        .find(|o| o.path.as_str() == "conflict.txt")
        .expect("expected override for conflict.txt");
    assert_eq!(conflict_override.winner, l2.id.as_str());
    assert_eq!(conflict_override.hidden, vec![l1.id.as_str()]);

    // Check mounted file path
    let mounted_planned = p
        .files
        .iter()
        .find(|f| f.path.as_str() == "Data/mounted.txt")
        .expect("mounted.txt should be under Data/");
    assert_eq!(mounted_planned.path.as_str(), "Data/mounted.txt");

    // 2. Reordering: move item 2 to position 0 (so order becomes item2, item1, item3)
    move_content(&ctx, &inst.instance_id, &item2_id, 0).unwrap();
    let p_reordered = plan(&ctx, &inst.instance_id, &def, DeployMode::Links).unwrap();
    let conflict_override2 = p_reordered
        .overrides
        .iter()
        .find(|o| o.path.as_str() == "conflict.txt")
        .expect("expected override after reordering");
    // Now item 1 is after item 2, so item 1 wins!
    assert_eq!(conflict_override2.winner, l1.id.as_str());
    assert_eq!(conflict_override2.hidden, vec![l2.id.as_str()]);

    // 3. Disabling: disable item 1, now item 2 is unhidden and wins
    set_content_enabled(&ctx, &inst.instance_id, &item1_id, false).unwrap();
    let p_disabled = plan(&ctx, &inst.instance_id, &def, DeployMode::Links).unwrap();
    // conflict.txt now has only item 2, no override
    let file = p_disabled
        .files
        .iter()
        .find(|f| f.path.as_str() == "conflict.txt")
        .expect("conflict.txt still planned");
    match &file.source {
        FileSource::Content { item_id, .. } => assert_eq!(item_id, &item2_id),
        _ => panic!("expected content source"),
    }
    assert!(
        p_disabled
            .overrides
            .iter()
            .all(|o| o.path.as_str() != "conflict.txt"),
        "no override when lower layer is disabled"
    );
}

#[test]
fn test_case_insensitivity_override() {
    let tmp = TempDir::new().unwrap();
    let def = make_test_definition();
    let ctx = create_test_context(&tmp, &def);

    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(&install_dir).unwrap();
    setup_fake_install(&install_dir);
    let install = make_test_install(&install_dir, true, true);
    let inst = create_pinned_instance(&ctx, &install, &def, "CaseTest");

    let item1 = add_content_folder(&ctx, "Layer1", &[("data/file.txt", b"lowercase")]);
    let item2 = add_content_folder(&ctx, "Layer2", &[("DATA/FILE.TXT", b"UPPERCASE")]);

    let _l1 = add_content(&ctx, &inst.instance_id, &item1, None, None).unwrap();
    let _l2 = add_content(&ctx, &inst.instance_id, &item2, None, None).unwrap();

    let p = plan(&ctx, &inst.instance_id, &def, DeployMode::Links).unwrap();
    // The winning layer's casing should be kept: DATA/FILE.TXT
    let planned = p
        .files
        .iter()
        .find(|f| f.path.as_str().eq_ignore_ascii_case("data/file.txt"))
        .expect("found planned file");
    assert_eq!(planned.path.as_str(), "DATA/FILE.TXT");

    // Deploy and verify
    let outcome = deploy(&ctx, &inst.instance_id, &def, DeployMode::Links).unwrap();
    assert!(matches!(outcome, DeployOutcome::Built { .. }));

    let game_dir = deployment_dir(&ctx, &inst.instance_id).unwrap().unwrap();
    assert!(game_dir.join("DATA").join("FILE.TXT").exists());
}

#[test]
fn test_file_folder_conflict_across_layers() {
    let tmp = TempDir::new().unwrap();
    let def = make_test_definition();
    let ctx = create_test_context(&tmp, &def);

    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(&install_dir).unwrap();
    setup_fake_install(&install_dir);
    let install = make_test_install(&install_dir, true, true);
    let inst = create_pinned_instance(&ctx, &install, &def, "ConflictTest");

    let item1 = add_content_folder(&ctx, "FileLayer", &[("Data/x", b"file content")]);
    let item2 = add_content_folder(&ctx, "FolderLayer", &[("Data/x/y", b"child content")]);

    add_content(&ctx, &inst.instance_id, &item1, None, None).unwrap();
    add_content(&ctx, &inst.instance_id, &item2, None, None).unwrap();

    let err = plan(&ctx, &inst.instance_id, &def, DeployMode::Links).unwrap_err();
    match err {
        DeployError::FileFolderConflict {
            file_path,
            file_layer,
            folder_path,
            folder_layer,
        } => {
            assert_eq!(file_path.as_str(), "Data/x");
            assert_eq!(folder_path.as_str(), "Data/x/y");
            assert!(!file_layer.is_empty());
            assert!(!folder_layer.is_empty());
        }
        other => panic!("expected FileFolderConflict, got {other:?}"),
    }
}

#[test]
fn test_links_vs_copies_placement() {
    let tmp = TempDir::new().unwrap();
    let def = make_test_definition();
    let ctx = create_test_context(&tmp, &def);

    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(&install_dir).unwrap();
    setup_fake_install(&install_dir);
    let install = make_test_install(&install_dir, true, true);
    let inst = create_pinned_instance(&ctx, &install, &def, "PlacementTest");

    let item1 = add_content_folder(&ctx, "ModContent", &[("mod_file.txt", b"mod content")]);
    add_content(&ctx, &inst.instance_id, &item1, None, None).unwrap();

    // 1. Plan in Links mode
    let plan_links = plan(&ctx, &inst.instance_id, &def, DeployMode::Links).unwrap();
    let base_file = plan_links
        .files
        .iter()
        .find(|f| f.path.as_str() == "base_file.txt")
        .unwrap();
    assert_eq!(base_file.placement, Placement::Link);

    let declared_write = plan_links
        .files
        .iter()
        .find(|f| f.path.as_str() == "writeable_base.txt")
        .unwrap();
    assert_eq!(declared_write.placement, Placement::Copy);

    let mod_file = plan_links
        .files
        .iter()
        .find(|f| f.path.as_str() == "mod_file.txt")
        .unwrap();
    assert_eq!(mod_file.placement, Placement::Link);

    // 2. Deploy in Links mode
    deploy(&ctx, &inst.instance_id, &def, DeployMode::Links).unwrap();
    let game_dir = deployment_dir(&ctx, &inst.instance_id).unwrap().unwrap();

    // Base file is linked: FileIdentity matches base source
    let base_id = match &inst.base {
        BaseReference::Pinned { id, .. } => id,
        _ => panic!("expected pinned base"),
    };
    let base_manifest_content =
        std::fs::read_to_string(ctx.paths.base_manifest_path(base_id)).unwrap();
    let base_manifest: agora_core::game_base::BaseManifest =
        serde_json::from_str(&base_manifest_content).unwrap();
    let source_base_file = base_manifest.location.join("base_file.txt");
    let deployed_base_file = game_dir.join("base_file.txt");
    let id_src = get_file_identity(&source_base_file).unwrap();
    let id_deployed = get_file_identity(&deployed_base_file).unwrap();
    assert_eq!(
        id_src, id_deployed,
        "linked base file should share identity"
    );

    // Declared write file is copied: FileIdentity does NOT match
    let source_decl_file = base_manifest.location.join("writeable_base.txt");
    let deployed_decl_file = game_dir.join("writeable_base.txt");
    let id_decl_src = get_file_identity(&source_decl_file).unwrap();
    let id_decl_deployed = get_file_identity(&deployed_decl_file).unwrap();
    assert_ne!(
        id_decl_src, id_decl_deployed,
        "copied declared write should not share identity"
    );

    // Content file link
    let deployed_mod = game_dir.join("mod_file.txt");
    #[cfg(windows)]
    {
        // Deny ACL prevents writing to mod link
        let write_res = std::fs::OpenOptions::new().write(true).open(&deployed_mod);
        assert_eq!(
            write_res.unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
    }

    // Deleting the link succeeds because game folder has FILE_DELETE_CHILD
    std::fs::remove_file(&deployed_mod).expect("deleting link should succeed");
    assert!(!deployed_mod.exists());

    // 3. Plan in Copies mode copies everything
    let plan_copies = plan(&ctx, &inst.instance_id, &def, DeployMode::Copies).unwrap();
    for f in &plan_copies.files {
        assert_eq!(f.placement, Placement::Copy);
    }
}

#[test]
fn test_harvest_lifecycle() {
    let tmp = TempDir::new().unwrap();
    let def = make_test_definition();
    let ctx = create_test_context(&tmp, &def);

    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(&install_dir).unwrap();
    setup_fake_install(&install_dir);
    let install = make_test_install(&install_dir, true, true);
    let inst = create_pinned_instance(&ctx, &install, &def, "HarvestTest");

    let item1 = add_content_folder(&ctx, "ModContent", &[("mod_item.txt", b"mod 1")]);
    add_content(&ctx, &inst.instance_id, &item1, None, None).unwrap();

    deploy(&ctx, &inst.instance_id, &def, DeployMode::Links).unwrap();
    let game_dir = deployment_dir(&ctx, &inst.instance_id).unwrap().unwrap();

    // 1. Game creates a new file
    let save_dir = game_dir.join("saves");
    std::fs::create_dir_all(&save_dir).unwrap();
    std::fs::write(save_dir.join("quicksave.dat"), b"saved game data").unwrap();

    // 2. Game modifies a copied file (declared write)
    std::fs::write(
        game_dir.join("writeable_base.txt"),
        b"modified writeable base",
    )
    .unwrap();

    // 3. Game modifies a linked base file (writes through)
    std::fs::write(game_dir.join("base_file.txt"), b"modified base file!").unwrap();

    // 4. Game deletes a deployed content file
    std::fs::remove_file(game_dir.join("mod_item.txt")).unwrap();

    // Undeploy and harvest
    let harvest = undeploy(&ctx, &inst.instance_id).unwrap();

    // Verify harvest report
    assert!(
        harvest
            .copied_to_writable
            .iter()
            .any(|p| p.as_str() == "saves/quicksave.dat"),
        "new file harvested"
    );
    assert!(
        harvest
            .copied_to_writable
            .iter()
            .any(|p| p.as_str() == "writeable_base.txt"),
        "changed copied file harvested"
    );
    assert!(
        harvest
            .copied_to_writable
            .iter()
            .any(|p| p.as_str() == "base_file.txt"),
        "changed linked file harvested"
    );
    assert!(
        harvest
            .base_files_changed
            .iter()
            .any(|c| c.path.as_str() == "base_file.txt"),
        "base file reported in base_files_changed"
    );
    assert!(
        harvest
            .whiteouts_added
            .iter()
            .any(|p| p.as_str() == "mod_item.txt"),
        "deleted content file added as whiteout"
    );

    // Verify files in instance's writable directory
    let instance_dir = ctx.paths.instances_root().join(&inst.instance_id);
    let writable_dir = instance_dir.join("writable");
    assert_eq!(
        std::fs::read(writable_dir.join("saves/quicksave.dat")).unwrap(),
        b"saved game data"
    );
    assert_eq!(
        std::fs::read(writable_dir.join("writeable_base.txt")).unwrap(),
        b"modified writeable base"
    );
    assert_eq!(
        std::fs::read(writable_dir.join("base_file.txt")).unwrap(),
        b"modified base file!"
    );

    // Verify manifest has whiteout
    let manifest = get_manifest(&ctx, &inst.instance_id).unwrap();
    let writable_layer = manifest
        .layers
        .layers()
        .iter()
        .find(|l| l.id.as_str() == "writable")
        .expect("writable layer in manifest");
    assert!(writable_layer
        .whiteouts
        .iter()
        .any(|w| w.as_str() == "mod_item.txt"));

    // 5. Deploy again: writable layer should now be deployed
    deploy(&ctx, &inst.instance_id, &def, DeployMode::Links).unwrap();
    let game_dir2 = deployment_dir(&ctx, &inst.instance_id).unwrap().unwrap();
    assert!(game_dir2.join("saves/quicksave.dat").exists());
    assert!(!game_dir2.join("mod_item.txt").exists()); // Whited out

    // 6. Delete a file that came from writable: should remove it from writable, NOT add whiteout
    std::fs::remove_file(game_dir2.join("saves/quicksave.dat")).unwrap();
    let harvest2 = undeploy(&ctx, &inst.instance_id).unwrap();
    assert!(
        !writable_dir.join("saves/quicksave.dat").exists(),
        "deleting writable file should remove it from writable dir"
    );
    assert!(
        !harvest2
            .whiteouts_added
            .iter()
            .any(|w| w.as_str() == "saves/quicksave.dat"),
        "deleting writable file should not add a whiteout"
    );
}

#[test]
fn test_rebuilding_uptodate_and_delta() {
    let tmp = TempDir::new().unwrap();
    let def = make_test_definition();
    let ctx = create_test_context(&tmp, &def);

    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(&install_dir).unwrap();
    setup_fake_install(&install_dir);
    let install = make_test_install(&install_dir, true, true);
    let inst = create_pinned_instance(&ctx, &install, &def, "RebuildTest");

    let item1 = add_content_folder(&ctx, "ModContent", &[("mod_item.txt", b"mod 1")]);
    add_content(&ctx, &inst.instance_id, &item1, None, None).unwrap();

    // First deploy: Built
    let outcome1 = deploy(&ctx, &inst.instance_id, &def, DeployMode::Links).unwrap();
    assert!(matches!(outcome1, DeployOutcome::Built { .. }));

    // Second deploy without changes: UpToDate
    let outcome2 = deploy(&ctx, &inst.instance_id, &def, DeployMode::Links).unwrap();
    assert!(matches!(outcome2, DeployOutcome::UpToDate));

    // Add content: should rebuild
    let item2 = add_content_folder(&ctx, "ModContent2", &[("mod_item2.txt", b"mod 2")]);
    add_content(&ctx, &inst.instance_id, &item2, None, None).unwrap();

    let outcome3 = deploy(&ctx, &inst.instance_id, &def, DeployMode::Links).unwrap();
    assert!(matches!(outcome3, DeployOutcome::Built { .. }));
}

#[test]
fn test_undeploy_safety_checks() {
    let tmp = TempDir::new().unwrap();
    let def = make_test_definition();
    let ctx = create_test_context(&tmp, &def);

    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(&install_dir).unwrap();
    setup_fake_install(&install_dir);
    let install = make_test_install(&install_dir, true, true);
    let inst = create_pinned_instance(&ctx, &install, &def, "SafetyTest");

    let item1 = add_content_folder(&ctx, "ModContent", &[("mod_item.txt", b"mod 1")]);
    add_content(&ctx, &inst.instance_id, &item1, None, None).unwrap();

    deploy(&ctx, &inst.instance_id, &def, DeployMode::Links).unwrap();
    let game_dir = deployment_dir(&ctx, &inst.instance_id).unwrap().unwrap();
    let record_path = game_dir.parent().unwrap().join("deployment.json");
    assert!(record_path.exists());

    // Tamper with record: change instance_id
    let mut rec: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&record_path).unwrap()).unwrap();
    rec["instance_id"] = serde_json::Value::String("someone-elses-instance".into());
    std::fs::write(&record_path, serde_json::to_string_pretty(&rec).unwrap()).unwrap();

    // Undeploy must refuse
    let err = undeploy(&ctx, &inst.instance_id).unwrap_err();
    assert!(
        matches!(err, DeployError::InvalidDeploymentDir(_)),
        "expected InvalidDeploymentDir on instance_id mismatch, got {err:?}"
    );
    assert!(game_dir.exists(), "game_dir must NOT be deleted on error");
}

#[test]
fn test_content_removal_refused_when_in_use() {
    let tmp = TempDir::new().unwrap();
    let def = make_test_definition();
    let ctx = create_test_context(&tmp, &def);

    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(&install_dir).unwrap();
    setup_fake_install(&install_dir);
    let install = make_test_install(&install_dir, true, true);
    let inst = create_pinned_instance(&ctx, &install, &def, "InUseTest");

    let item_id = add_content_folder(&ctx, "InUseMod", &[("mod.txt", b"data")]);
    add_content(&ctx, &inst.instance_id, &item_id, None, None).unwrap();

    // 1. Content removal refused naming instance
    let err = agora_core::content_store::remove_item(&ctx, &item_id).unwrap_err();
    match err {
        agora_core::content_store::ContentError::InUse { instances, .. } => {
            assert!(instances.contains(&inst.instance_id));
        }
        other => panic!("expected ContentError::InUse, got {other:?}"),
    }

    // 2. Remove content from instance, now content removal succeeds
    remove_content(&ctx, &inst.instance_id, &item_id).unwrap();
    agora_core::content_store::remove_item(&ctx, &item_id).expect("remove succeeds now");

    // 3. Corrupt manifest refuses removal (fail closed)
    let item_id2 = add_content_folder(&ctx, "InUseMod2", &[("mod2.txt", b"data")]);
    let manifest_path = ctx
        .paths
        .instances_root()
        .join(&inst.instance_id)
        .join("instance_manifest.json");
    std::fs::write(&manifest_path, b"not valid json").unwrap();

    let err2 = agora_core::content_store::remove_item(&ctx, &item_id2).unwrap_err();
    assert!(
        matches!(
            err2,
            agora_core::content_store::ContentError::UnreadableManifest { .. }
        ),
        "expected UnreadableManifest, got {err2:?}"
    );
}

#[test]
fn test_launch_deployment_integration() {
    let tmp = TempDir::new().unwrap();
    let def = make_test_definition();
    let ctx = create_test_context(&tmp, &def);

    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(&install_dir).unwrap();
    setup_fake_install(&install_dir);
    let install = make_test_install(&install_dir, true, true);
    let inst = create_pinned_instance(&ctx, &install, &def, "LaunchTest");

    let item_id = add_content_folder(&ctx, "LaunchMod", &[("mod.txt", b"mod data")]);
    add_content(&ctx, &inst.instance_id, &item_id, None, None).unwrap();

    let report = DiscoveryReport {
        installs: vec![install.discovered.clone()],
        warnings: vec![],
    };

    // 1. Pinned instance with content auto-deploys
    let prepared =
        prepare_launch_with_discovery(&ctx, &inst.instance_id, &def, false, &|| report.clone())
            .expect("prepare launch should succeed");
    assert!(
        prepared.deploy_outcome.is_some(),
        "deploy_outcome must be returned"
    );
    let expected_game_dir = deployment_dir(&ctx, &inst.instance_id).unwrap().unwrap();
    assert_eq!(prepared.resolved.cwd, expected_game_dir);

    // 2. Unpinned instance: create with readable: false
    let unpinned_install = make_test_install(&install_dir, false, true);
    let unpinned_inst = create(
        &ctx,
        &unpinned_install,
        &def,
        "UnpinnedStore",
        None,
        BaseMode::Linked,
        &|_| {},
    )
    .unwrap();
    assert!(matches!(unpinned_inst.base, BaseReference::Unpinned { .. }));

    // Unpinned instance without content launches vanilla from store install
    let unpinned_report = DiscoveryReport {
        installs: vec![unpinned_install.discovered.clone()],
        warnings: vec![],
    };
    let unpinned_prep =
        prepare_launch_with_discovery(&ctx, &unpinned_inst.instance_id, &def, false, &|| {
            unpinned_report.clone()
        })
        .expect("unpinned vanilla launch should succeed");
    assert!(unpinned_prep.deploy_outcome.is_none());
    assert_eq!(unpinned_prep.resolved.cwd, install_dir);

    // Unpinned instance with content returns deploy error
    add_content(&ctx, &unpinned_inst.instance_id, &item_id, None, None).unwrap();
    let err = prepare_launch_with_discovery(&ctx, &unpinned_inst.instance_id, &def, false, &|| {
        unpinned_report.clone()
    })
    .unwrap_err();
    match err {
        InstanceError::Deploy(DeployError::UnpinnedInstance(id)) => {
            assert_eq!(id, unpinned_inst.instance_id);
        }
        other => panic!("expected DeployError::UnpinnedInstance, got {other:?}"),
    }

    // Direct plan call on unpinned instance with content also returns UnpinnedInstance
    let plan_err = plan(&ctx, &unpinned_inst.instance_id, &def, DeployMode::Links).unwrap_err();
    assert!(matches!(plan_err, DeployError::UnpinnedInstance(_)));
}

#[test]
fn test_deploy_source_path_filtering_and_boundary() {
    let tmp = TempDir::new().unwrap();
    let def = make_test_definition();
    let ctx = create_test_context(&tmp, &def);

    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(&install_dir).unwrap();
    setup_fake_install(&install_dir);
    let install = make_test_install(&install_dir, true, true);
    let inst = create_pinned_instance(&ctx, &install, &def, "SourcePathTest");

    // Item with files under MyMod, MyModX, and top-level
    let item_id = add_content_folder(
        &ctx,
        "ModWithSubfolder",
        &[
            ("MyMod/textures/a.dds", b"dds bytes"),
            ("MyMod/meshes/b.nif", b"nif bytes"),
            ("MyModX/c.dds", b"other bytes"),
            ("readme.txt", b"docs"),
        ],
    );

    // 1. Layer with source_path: "MyMod", mount_path: "Data"
    let _layer = add_content(
        &ctx,
        &inst.instance_id,
        &item_id,
        Some("Data"),
        Some("MyMod"),
    )
    .unwrap();

    let p = plan(&ctx, &inst.instance_id, &def, DeployMode::Links).unwrap();
    // Only MyMod files should be deployed, stripped under mount_path "Data"
    assert!(p
        .files
        .iter()
        .any(|f| f.path.as_str() == "Data/textures/a.dds"));
    assert!(p
        .files
        .iter()
        .any(|f| f.path.as_str() == "Data/meshes/b.nif"));
    // MyModX/c.dds must NOT match MyMod
    assert!(!p.files.iter().any(|f| f.path.as_str().contains("c.dds")));
    assert!(!p
        .files
        .iter()
        .any(|f| f.path.as_str().contains("readme.txt")));

    // 2. Layer with source_path matching nothing deploys nothing and records a warning
    let item_id2 = add_content_folder(&ctx, "ModEmptyMatch", &[("other/file.txt", b"data")]);
    let layer2 = add_content(
        &ctx,
        &inst.instance_id,
        &item_id2,
        Some("Data"),
        Some("NonExistentFolder"),
    )
    .unwrap();

    let p2 = plan(&ctx, &inst.instance_id, &def, DeployMode::Links).unwrap();
    assert!(
        p2.warnings.iter().any(|w| w.contains(layer2.id.as_str())),
        "warnings must name the layer: {:?}",
        p2.warnings
    );
}
