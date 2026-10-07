use std::path::Path;
use std::sync::Arc;

use agora_core::ctx::CoreContext;
use agora_core::game_base::{get_file_identity, BaseMode};
use agora_core::game_deploy::{
    add_content, deploy, deployment_dir, move_content, plan, remove_content, set_content_enabled,
    set_content_own_copy, undeploy, DeployError, DeployMode, DeployOutcome, FileSource, Placement,
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
    assert!(matches!(outcome2, DeployOutcome::UpToDate { .. }));

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

// ---------------------------------------------------------------------------
// Virtual file system rung (MASTER_SPEC §26.5, rung 1)
// ---------------------------------------------------------------------------

fn instance_writable_dir(ctx: &CoreContext, instance_id: &str) -> std::path::PathBuf {
    ctx.paths
        .instance_dir(instance_id)
        .unwrap()
        .join("writable")
}

fn write_file(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn find<'a>(
    plan: &'a agora_core::game_deploy::DeploymentPlan,
    path: &str,
) -> Option<&'a agora_core::game_deploy::PlannedFile> {
    plan.files.iter().find(|f| f.path.as_str() == path)
}

#[test]
fn virtual_plan_copies_declared_writes_and_leaves_the_writable_layer_out() {
    let tmp = TempDir::new().unwrap();
    let def = make_test_definition();
    let ctx = create_test_context(&tmp, &def);
    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(&install_dir).unwrap();
    setup_fake_install(&install_dir);
    let install = make_test_install(&install_dir, true, true);
    let inst = create_pinned_instance(&ctx, &install, &def, "VirtualPlan");
    let id = &inst.instance_id;

    let item = add_content_folder(&ctx, "Mod", &[("mod_file.txt", b"mod")]);
    add_content(&ctx, id, &item, None, None).unwrap();

    let writable = instance_writable_dir(&ctx, id);
    write_file(&writable.join("saves/quick.dat"), b"save");
    write_file(&writable.join("base_file.txt"), b"game's own version");

    let virt = plan(&ctx, id, &def, DeployMode::Virtual).unwrap();
    // A declared write is the instance's own copy even under the VFS: DLLs loaded while the game
    // starts write before the hooks exist, and through a link that write would reach the base.
    assert_eq!(
        find(&virt, "writeable_base.txt").unwrap().placement,
        Placement::Copy
    );
    assert_eq!(
        find(&virt, "mod_file.txt").unwrap().placement,
        Placement::Link
    );
    // The writable layer is shown on top by the VFS, not placed in the farm.
    assert!(find(&virt, "saves/quick.dat").is_none());
    assert!(matches!(
        find(&virt, "base_file.txt").unwrap().source,
        FileSource::Base { .. }
    ));
    assert!(!virt
        .files
        .iter()
        .any(|f| matches!(f.source, FileSource::Writable { .. })));

    // Links and Copies still place it, and still copy a declared write.
    let links = plan(&ctx, id, &def, DeployMode::Links).unwrap();
    assert!(find(&links, "saves/quick.dat").is_some());
    assert!(matches!(
        find(&links, "base_file.txt").unwrap().source,
        FileSource::Writable { .. }
    ));
    assert_eq!(
        find(&links, "writeable_base.txt").unwrap().placement,
        Placement::Copy
    );

    // A change to the writable layer alone leaves the virtual farm's fingerprint alone,
    // and changes the link farm's.
    write_file(&writable.join("saves/another.dat"), b"more");
    write_file(&writable.join("base_file.txt"), b"rewritten again, longer");
    let virt_after = plan(&ctx, id, &def, DeployMode::Virtual).unwrap();
    let links_after = plan(&ctx, id, &def, DeployMode::Links).unwrap();
    assert_eq!(virt.fingerprint, virt_after.fingerprint);
    assert_ne!(links.fingerprint, links_after.fingerprint);
}

#[test]
fn vfs_whiteout_markers_hide_lower_files_in_every_mode() {
    let tmp = TempDir::new().unwrap();
    let def = make_test_definition();
    let ctx = create_test_context(&tmp, &def);
    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(&install_dir).unwrap();
    setup_fake_install(&install_dir);
    let install = make_test_install(&install_dir, true, true);
    let inst = create_pinned_instance(&ctx, &install, &def, "Whiteouts");
    let id = &inst.instance_id;

    let item = add_content_folder(
        &ctx,
        "Mod",
        &[
            ("mod_file.txt", b"mod"),
            ("Scripts/a.pex", b"a"),
            ("Scripts/b.pex", b"b"),
        ],
    );
    add_content(&ctx, id, &item, None, None).unwrap();

    for mode in [DeployMode::Links, DeployMode::Copies, DeployMode::Virtual] {
        let p = plan(&ctx, id, &def, mode).unwrap();
        assert!(find(&p, "base_file.txt").is_some(), "{mode}");
        assert!(find(&p, "mod_file.txt").is_some(), "{mode}");
        assert!(find(&p, "Scripts/a.pex").is_some(), "{mode}");
    }

    // What the VFS writes when a game deletes a base file, a mod file, and a whole mod folder
    // (a folder's marker hides everything under it).
    let writable = instance_writable_dir(&ctx, id);
    write_file(&writable.join(".agvfs-wh/base_file.txt.wh"), b"");
    write_file(&writable.join(".agvfs-wh/Mod_File.txt.wh"), b"");
    write_file(&writable.join(".agvfs-wh/scripts.wh"), b"");

    for mode in [DeployMode::Links, DeployMode::Copies, DeployMode::Virtual] {
        let p = plan(&ctx, id, &def, mode).unwrap();
        assert!(find(&p, "base_file.txt").is_none(), "{mode}");
        assert!(find(&p, "mod_file.txt").is_none(), "{mode}");
        assert!(find(&p, "Scripts/a.pex").is_none(), "{mode}");
        assert!(find(&p, "Scripts/b.pex").is_none(), "{mode}");
        // Everything else is still there, and the marker folder is never deployed.
        assert!(find(&p, "Game.exe").is_some(), "{mode}");
        assert!(find(&p, "Data/Skyrim.bsa").is_some(), "{mode}");
        assert!(
            !p.files
                .iter()
                .any(|f| f.path.as_str().to_ascii_lowercase().contains("agvfs-wh")),
            "{mode}: marker folder deployed"
        );
    }

    // The game recreated a deleted file: the writable file wins over the marker in the
    // modes that place the writable layer.
    write_file(&writable.join("base_file.txt"), b"recreated");
    let links = plan(&ctx, id, &def, DeployMode::Links).unwrap();
    assert!(matches!(
        find(&links, "base_file.txt").unwrap().source,
        FileSource::Writable { .. }
    ));
}

#[test]
fn changing_the_mode_rebuilds_and_the_virtual_farm_survives_writes() {
    let tmp = TempDir::new().unwrap();
    let def = make_test_definition();
    let ctx = create_test_context(&tmp, &def);
    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(&install_dir).unwrap();
    setup_fake_install(&install_dir);
    let install = make_test_install(&install_dir, true, true);
    let inst = create_pinned_instance(&ctx, &install, &def, "Switching");
    let id = &inst.instance_id;

    let item = add_content_folder(&ctx, "Mod", &[("mod_file.txt", b"mod")]);
    add_content(&ctx, id, &item, None, None).unwrap();

    assert!(matches!(
        deploy(&ctx, id, &def, DeployMode::Links).unwrap(),
        DeployOutcome::Built { .. }
    ));
    assert!(matches!(
        deploy(&ctx, id, &def, DeployMode::Virtual).unwrap(),
        DeployOutcome::Built { .. }
    ));
    assert_eq!(
        deploy(&ctx, id, &def, DeployMode::Virtual).unwrap(),
        DeployOutcome::UpToDate { plugins: None }
    );

    // The VFS shows the writable layer on top: the game writing there, and a marker for a
    // deletion, change nothing about the farm already built (no rebuild when switching back
    // to an instance whose layers did not change).
    let game_dir = deployment_dir(&ctx, id).unwrap().unwrap();
    let writable = instance_writable_dir(&ctx, id);
    write_file(&writable.join("saves/quick.dat"), b"save");
    assert!(!game_dir.join("saves/quick.dat").exists());
    assert_eq!(
        deploy(&ctx, id, &def, DeployMode::Virtual).unwrap(),
        DeployOutcome::UpToDate { plugins: None }
    );

    // A declared write is the instance's own copy, not a link to the base: a DLL the game loads
    // at start writes before the VFS's hooks exist (Engine Fixes' d3dx9_42.dll and its log).
    let base_id = match &inst.base {
        BaseReference::Pinned { id, .. } => id.clone(),
        _ => panic!("expected pinned base"),
    };
    let base_manifest: agora_core::game_base::BaseManifest = serde_json::from_str(
        &std::fs::read_to_string(ctx.paths.base_manifest_path(&base_id)).unwrap(),
    )
    .unwrap();
    let base_copy = base_manifest.location.join("writeable_base.txt");
    assert_ne!(
        get_file_identity(&base_copy).unwrap(),
        get_file_identity(&game_dir.join("writeable_base.txt")).unwrap(),
        "under the VFS a declared write is still copied"
    );
    // That early write changes the instance's copy: the base is untouched and switching back
    // to the instance is still no rebuild.
    let base_before = std::fs::read(&base_copy).unwrap();
    std::fs::write(
        game_dir.join("writeable_base.txt"),
        b"written before the hooks existed",
    )
    .unwrap();
    assert_eq!(std::fs::read(&base_copy).unwrap(), base_before);
    assert_eq!(
        deploy(&ctx, id, &def, DeployMode::Virtual).unwrap(),
        DeployOutcome::UpToDate { plugins: None }
    );

    // Switching mode rebuilds, and the link farm now holds the writable layer.
    let outcome = deploy(&ctx, id, &def, DeployMode::Links).unwrap();
    assert!(matches!(outcome, DeployOutcome::Built { .. }));
    let game_dir = deployment_dir(&ctx, id).unwrap().unwrap();
    assert_eq!(
        std::fs::read(game_dir.join("saves/quick.dat")).unwrap(),
        b"save"
    );
    assert!(matches!(
        deploy(&ctx, id, &def, DeployMode::Copies).unwrap(),
        DeployOutcome::Built { .. }
    ));
}

// -- choosing the rung, with the DLL lookup and the launcher replaced --------

struct FakeLauncher {
    dll: Result<std::path::PathBuf, String>,
    /// When set, a launch under the VFS fails with this reason.
    vfs_error: Option<String>,
    /// `(deployment, runs under the VFS)` of every launch attempted.
    attempts: std::cell::RefCell<Vec<(Option<DeployMode>, bool)>>,
}

impl FakeLauncher {
    fn new(dll: Result<std::path::PathBuf, String>, vfs_error: Option<&str>) -> Self {
        Self {
            dll,
            vfs_error: vfs_error.map(str::to_string),
            attempts: Default::default(),
        }
    }

    fn attempts(&self) -> Vec<(Option<DeployMode>, bool)> {
        self.attempts.borrow().clone()
    }
}

impl agora_core::game_launch::Launcher for FakeLauncher {
    fn locate_vfs_dll(&self) -> Result<std::path::PathBuf, String> {
        self.dll.clone()
    }

    fn launch(
        &self,
        prepared: &agora_core::game_launch::PreparedLaunch,
    ) -> Result<agora_core::game_launch::LaunchedGame, agora_core::game_launch::LaunchError> {
        self.attempts
            .borrow_mut()
            .push((prepared.deployment, prepared.vfs.is_some()));
        if prepared.vfs.is_some() {
            if let Some(reason) = &self.vfs_error {
                return Err(agora_core::game_launch::LaunchError::VfsUnavailable {
                    reason: reason.clone(),
                });
            }
        }
        // Nothing real runs: a process that has already finished stands in for the game.
        let mut command = if cfg!(windows) {
            let mut c = std::process::Command::new("cmd");
            c.args(["/c", "exit"]);
            c
        } else {
            std::process::Command::new("true")
        };
        let child = command
            .spawn()
            .map_err(agora_core::game_launch::LaunchError::Io)?;
        let pid = child.id();
        Ok(agora_core::game_launch::LaunchedGame {
            child,
            identity: agora_core::process_identity::ProcessIdentity {
                pid,
                start_time: 0,
                expected_exe: None,
            },
            program: prepared.resolved.program.clone(),
        })
    }
}

fn rung_fixture(name: &str, def: &GameDefinition) -> (TempDir, CoreContext, GameInstanceRecord) {
    let tmp = TempDir::new().unwrap();
    let ctx = create_test_context(&tmp, def);
    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(&install_dir).unwrap();
    setup_fake_install(&install_dir);
    let install = make_test_install(&install_dir, true, true);
    let inst = create_pinned_instance(&ctx, &install, def, name);
    let item = add_content_folder(&ctx, "Mod", &[("mod_file.txt", b"mod")]);
    add_content(&ctx, &inst.instance_id, &item, None, None).unwrap();
    (tmp, ctx, inst)
}

fn launch_with_fake(
    ctx: &CoreContext,
    id: &str,
    def: &GameDefinition,
    deployment: Option<DeployMode>,
    launcher: &FakeLauncher,
) -> Result<agora_core::game_instance::LaunchedInstance, InstanceError> {
    agora_core::game_instance::launch_with(
        ctx,
        id,
        def,
        agora_core::game_instance::LaunchOptions {
            launch_anyway: false,
            plain: false,
            deployment,
        },
        &DiscoveryReport::default,
        launcher,
    )
}

fn recorded_mode(ctx: &CoreContext, id: &str) -> DeployMode {
    let game_dir = deployment_dir(ctx, id).unwrap().unwrap();
    let record: agora_core::game_deploy::DeploymentRecord = serde_json::from_str(
        &std::fs::read_to_string(game_dir.parent().unwrap().join("deployment.json")).unwrap(),
    )
    .unwrap();
    record.mode
}

#[test]
fn a_vfs_game_runs_under_the_vfs_by_default() {
    let def = make_test_definition();
    let (_tmp, ctx, inst) = rung_fixture("VfsDefault", &def);
    let launcher = FakeLauncher::new(Ok("C:/Agora/agora_vfs.dll".into()), None);

    let mut launched = launch_with_fake(&ctx, &inst.instance_id, &def, None, &launcher).unwrap();
    let _ = launched.launched.child.wait();

    assert_eq!(launcher.attempts(), vec![(Some(DeployMode::Virtual), true)]);
    assert!(launched.prepared.notice.is_none());
    assert!(!launched.prepared.deployment_chosen);
    assert_eq!(recorded_mode(&ctx, &inst.instance_id), DeployMode::Virtual);

    // The VFS is mounted over the farm, with the instance's writable folder as its upper layer.
    let vfs = launched.prepared.vfs.expect("a vfs launch");
    let game_dir = deployment_dir(&ctx, &inst.instance_id).unwrap().unwrap();
    let instance_dir = ctx.paths.instance_dir(&inst.instance_id).unwrap();
    assert_eq!(vfs.mount, game_dir);
    assert_eq!(vfs.lowers, vec![game_dir]);
    assert_eq!(vfs.upper, instance_dir.join("writable"));
    assert_eq!(
        vfs.config_path,
        instance_dir.join("vfs").join("config.json")
    );
    assert_eq!(vfs.log, instance_dir.join("logs").join("vfs.log"));
}

#[test]
fn a_missing_dll_steps_down_to_links_and_says_why() {
    let def = make_test_definition();
    let (_tmp, ctx, inst) = rung_fixture("NoDll", &def);
    let launcher = FakeLauncher::new(Err("agora_vfs.dll was not found at 'X'".into()), None);

    let mut launched = launch_with_fake(&ctx, &inst.instance_id, &def, None, &launcher).unwrap();
    let _ = launched.launched.child.wait();

    assert_eq!(launcher.attempts(), vec![(Some(DeployMode::Links), false)]);
    assert!(launched.prepared.vfs.is_none());
    assert!(!launched.prepared.deployment_chosen);
    assert_eq!(
        launched.prepared.notice.as_deref(),
        Some(
            "the virtual file system could not start: agora_vfs.dll was not found at 'X'; \
             running from linked files instead"
        )
    );
    assert_eq!(recorded_mode(&ctx, &inst.instance_id), DeployMode::Links);
}

#[test]
fn a_dll_that_cannot_start_steps_down_to_links_and_says_why() {
    let def = make_test_definition();
    let (_tmp, ctx, inst) = rung_fixture("VfsRefused", &def);
    let launcher = FakeLauncher::new(
        Ok("C:/Agora/agora_vfs.dll".into()),
        Some("the game process could not load agora_vfs.dll"),
    );

    let mut launched = launch_with_fake(&ctx, &inst.instance_id, &def, None, &launcher).unwrap();
    let _ = launched.launched.child.wait();

    // Tried the VFS first, then started again from links.
    assert_eq!(
        launcher.attempts(),
        vec![
            (Some(DeployMode::Virtual), true),
            (Some(DeployMode::Links), false)
        ]
    );
    assert_eq!(launched.prepared.deployment, Some(DeployMode::Links));
    assert!(launched.prepared.vfs.is_none());
    let notice = launched.prepared.notice.unwrap();
    assert!(
        notice.starts_with("the virtual file system could not start: "),
        "{notice}"
    );
    assert!(notice.contains("could not load agora_vfs.dll"), "{notice}");
    assert!(
        notice.ends_with("running from linked files instead"),
        "{notice}"
    );
    // The farm was rebuilt for the rung that ran.
    assert_eq!(recorded_mode(&ctx, &inst.instance_id), DeployMode::Links);
}

#[test]
fn a_chosen_virtual_rung_fails_with_the_reason_instead_of_falling_back() {
    let def = make_test_definition();
    let (_tmp, ctx, inst) = rung_fixture("ChosenVirtual", &def);
    let id = &inst.instance_id;
    agora_core::game_deploy::set_deployment(&ctx, id, Some(DeployMode::Virtual)).unwrap();
    assert_eq!(
        get_manifest(&ctx, id).unwrap().deployment,
        Some(DeployMode::Virtual)
    );

    // No DLL: refused before anything is deployed.
    let launcher = FakeLauncher::new(Err("agora_vfs.dll was not found at 'X'".into()), None);
    let err = launch_with_fake(&ctx, id, &def, None, &launcher)
        .err()
        .expect("must not fall back");
    match &err {
        InstanceError::VfsUnavailable {
            instance_id,
            reason,
        } => {
            assert_eq!(instance_id, id);
            assert!(reason.contains("not found"));
        }
        other => panic!("expected VfsUnavailable, got {other:?}"),
    }
    let text = err.to_string();
    assert!(text.contains("not found at 'X'"), "{text}");
    assert!(
        text.contains("set-deployment") && text.contains("links"),
        "{text}"
    );
    assert!(launcher.attempts().is_empty());
    assert!(deployment_dir(&ctx, id).unwrap().is_none());

    // A DLL that refuses at launch fails the same way, and links are not tried.
    let launcher = FakeLauncher::new(Ok("C:/Agora/agora_vfs.dll".into()), Some("blocked"));
    let err = launch_with_fake(&ctx, id, &def, None, &launcher)
        .err()
        .expect("must not fall back");
    assert!(
        matches!(err, InstanceError::VfsUnavailable { .. }),
        "{err:?}"
    );
    assert_eq!(launcher.attempts(), vec![(Some(DeployMode::Virtual), true)]);

    // The same for a one-launch override.
    set_instance_auto(&ctx, id);
    let err = launch_with_fake(&ctx, id, &def, Some(DeployMode::Virtual), &launcher)
        .err()
        .expect("must not fall back");
    assert!(
        matches!(err, InstanceError::VfsUnavailable { .. }),
        "{err:?}"
    );
}

fn set_instance_auto(ctx: &CoreContext, id: &str) {
    agora_core::game_deploy::set_deployment(ctx, id, None).unwrap();
}

#[test]
fn a_chosen_rung_is_used_and_auto_clears_the_choice() {
    let def = make_test_definition();
    let (_tmp, ctx, inst) = rung_fixture("ChosenRungs", &def);
    let id = &inst.instance_id;
    let launcher = FakeLauncher::new(Ok("C:/Agora/agora_vfs.dll".into()), None);

    // `--deployment copies` for one launch.
    let mut launched =
        launch_with_fake(&ctx, id, &def, Some(DeployMode::Copies), &launcher).unwrap();
    let _ = launched.launched.child.wait();
    assert_eq!(launched.prepared.deployment, Some(DeployMode::Copies));
    assert!(launched.prepared.deployment_chosen);
    assert!(launched.prepared.vfs.is_none());
    assert_eq!(recorded_mode(&ctx, id), DeployMode::Copies);
    assert_eq!(
        get_manifest(&ctx, id).unwrap().deployment,
        None,
        "one launch only"
    );

    // `set-deployment links` sticks, even though the VFS is available.
    agora_core::game_deploy::set_deployment(&ctx, id, Some(DeployMode::Links)).unwrap();
    let mut launched = launch_with_fake(&ctx, id, &def, None, &launcher).unwrap();
    let _ = launched.launched.child.wait();
    assert_eq!(launched.prepared.deployment, Some(DeployMode::Links));
    assert!(launched.prepared.deployment_chosen);
    assert_eq!(recorded_mode(&ctx, id), DeployMode::Links);

    // A launch override beats the instance's choice.
    let mut launched =
        launch_with_fake(&ctx, id, &def, Some(DeployMode::Virtual), &launcher).unwrap();
    let _ = launched.launched.child.wait();
    assert_eq!(launched.prepared.deployment, Some(DeployMode::Virtual));
    assert!(launched.prepared.vfs.is_some());

    // `auto` clears the choice: the default applies again, and is not "chosen".
    agora_core::game_deploy::set_deployment(&ctx, id, None).unwrap();
    assert_eq!(get_manifest(&ctx, id).unwrap().deployment, None);
    let text = std::fs::read_to_string(ctx.paths.instance_manifest(id).unwrap()).unwrap();
    assert!(
        !text.contains("deployment"),
        "an unset choice is not written: {text}"
    );
    let mut launched = launch_with_fake(&ctx, id, &def, None, &launcher).unwrap();
    let _ = launched.launched.child.wait();
    assert_eq!(launched.prepared.deployment, Some(DeployMode::Virtual));
    assert!(!launched.prepared.deployment_chosen);

    // An instance that does not exist cannot be given a choice.
    assert!(matches!(
        agora_core::game_deploy::set_deployment(&ctx, "no-such-instance", Some(DeployMode::Links)),
        Err(DeployError::InstanceNotFound(_))
    ));
}

#[test]
fn a_redirect_game_defaults_to_virtual_and_steps_down_to_links_when_dll_missing() {
    let mut def = make_test_definition();
    def.deployment = DeploymentStrategy::Redirect;
    let (tmp, ctx, inst) = rung_fixture("RedirectGame", &def);

    // Found DLL: deploys and launches as Virtual
    let launcher = FakeLauncher::new(Ok("C:/Agora/agora_vfs.dll".into()), None);
    let mut launched = launch_with_fake(&ctx, &inst.instance_id, &def, None, &launcher).unwrap();
    let _ = launched.launched.child.wait();
    assert_eq!(launcher.attempts(), vec![(Some(DeployMode::Virtual), true)]);
    assert_eq!(launched.prepared.deployment, Some(DeployMode::Virtual));
    assert!(launched.prepared.notice.is_none());

    // Missing DLL: steps down to Links with notice
    let launcher = FakeLauncher::new(Err("no dll".into()), None);
    let mut launched = launch_with_fake(&ctx, &inst.instance_id, &def, None, &launcher).unwrap();
    let _ = launched.launched.child.wait();
    assert_eq!(launcher.attempts(), vec![(Some(DeployMode::Links), false)]);
    assert_eq!(launched.prepared.deployment, Some(DeployMode::Links));
    assert!(
        launched.prepared.notice.is_some(),
        "notice when stepped down from virtual to links"
    );

    // Unpinned instance is unchanged
    let install_dir = tmp.path().join("install");
    let unpinned_install = make_test_install(&install_dir, false, true);
    let unpinned_inst = create(
        &ctx,
        &unpinned_install,
        &def,
        "UnpinnedRedirect",
        None,
        BaseMode::Linked,
        &|_| {},
    )
    .unwrap();
    let unpinned_report = DiscoveryReport {
        installs: vec![unpinned_install.discovered.clone()],
        ..Default::default()
    };
    let unpinned_prep =
        prepare_launch_with_discovery(&ctx, &unpinned_inst.instance_id, &def, false, &|| {
            unpinned_report.clone()
        })
        .unwrap();
    assert!(unpinned_prep.deploy_outcome.is_none());
    assert_eq!(unpinned_prep.deployment, None);
}

fn bare_fixture(name: &str, def: &GameDefinition) -> (TempDir, CoreContext, GameInstanceRecord) {
    let tmp = TempDir::new().unwrap();
    let ctx = create_test_context(&tmp, def);
    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(&install_dir).unwrap();
    setup_fake_install(&install_dir);
    let install = make_test_install(&install_dir, true, true);
    let inst = create_pinned_instance(&ctx, &install, def, name);
    (tmp, ctx, inst)
}

#[test]
fn a_vfs_game_with_no_content_still_runs_under_the_vfs_and_steps_down_the_same_way() {
    let def = make_test_definition();
    let (_tmp, ctx, inst) = bare_fixture("BareVfs", &def);
    let id = &inst.instance_id;

    // On a linked base only the VFS keeps the game's writes out of the store install.
    let launcher = FakeLauncher::new(Ok("C:/Agora/agora_vfs.dll".into()), None);
    let mut launched = launch_with_fake(&ctx, id, &def, None, &launcher).unwrap();
    let _ = launched.launched.child.wait();
    assert!(launched.prepared.deploy_outcome.is_some());
    assert!(launched.prepared.vfs.is_some());
    assert_eq!(launched.prepared.deployment, Some(DeployMode::Virtual));

    // Without the DLL it runs from a farm of links, and says so.
    let launcher = FakeLauncher::new(Err("agora_vfs.dll was not found".into()), None);
    let mut launched = launch_with_fake(&ctx, id, &def, None, &launcher).unwrap();
    let _ = launched.launched.child.wait();
    assert_eq!(launched.prepared.deployment, Some(DeployMode::Links));
    assert!(launched.prepared.notice.is_some());

    // `deploy` builds what a launch would run: the instance's choice, else the default,
    // stepping down only when the DLL is missing.
    use agora_core::game_instance::deploy_mode_for;
    let present = FakeLauncher::new(Ok("C:/Agora/agora_vfs.dll".into()), None);
    let missing = FakeLauncher::new(Err("gone".into()), None);
    assert_eq!(
        deploy_mode_for(&ctx, id, &def, None, &present).unwrap(),
        DeployMode::Virtual
    );
    assert_eq!(
        deploy_mode_for(&ctx, id, &def, None, &missing).unwrap(),
        DeployMode::Links
    );
    assert_eq!(
        deploy_mode_for(&ctx, id, &def, Some(DeployMode::Copies), &present).unwrap(),
        DeployMode::Copies
    );
    agora_core::game_deploy::set_deployment(&ctx, id, Some(DeployMode::Virtual)).unwrap();
    assert_eq!(
        deploy_mode_for(&ctx, id, &def, None, &missing).unwrap(),
        DeployMode::Virtual,
        "a chosen rung is returned as chosen, whatever the machine has"
    );
}

#[test]
fn a_redirect_game_with_no_content_runs_under_the_vfs_and_steps_down_the_same_way() {
    let mut def = make_test_definition();
    def.deployment = DeploymentStrategy::Redirect;
    let (_tmp, ctx, inst) = bare_fixture("BareRedirect", &def);
    let id = &inst.instance_id;
    let launcher = FakeLauncher::new(Ok("C:/Agora/agora_vfs.dll".into()), None);

    let mut launched = launch_with_fake(&ctx, id, &def, None, &launcher).unwrap();
    let _ = launched.launched.child.wait();
    assert!(launched.prepared.deploy_outcome.is_some());
    assert!(launched.prepared.vfs.is_some());
    assert_eq!(launched.prepared.deployment, Some(DeployMode::Virtual));

    // Without the DLL it steps down to links and says so
    let launcher = FakeLauncher::new(Err("agora_vfs.dll was not found".into()), None);
    let mut launched = launch_with_fake(&ctx, id, &def, None, &launcher).unwrap();
    let _ = launched.launched.child.wait();
    assert_eq!(launched.prepared.deployment, Some(DeployMode::Links));
    assert!(launched.prepared.notice.is_some());
}

// -- real injection ----------------------------------------------------------

/// Finds the DLL the way the tests are documented to be run: `AGORA_VFS_DLL`.
#[cfg(windows)]
struct RealDllLauncher(std::path::PathBuf);

#[cfg(windows)]
impl agora_core::game_launch::Launcher for RealDllLauncher {
    fn locate_vfs_dll(&self) -> Result<std::path::PathBuf, String> {
        Ok(self.0.clone())
    }

    fn launch(
        &self,
        prepared: &agora_core::game_launch::PreparedLaunch,
    ) -> Result<agora_core::game_launch::LaunchedGame, agora_core::game_launch::LaunchError> {
        agora_core::game_launch::launch(prepared)
    }
}

#[cfg(windows)]
fn built_dll() -> std::path::PathBuf {
    let dll = std::env::var_os("AGORA_VFS_DLL")
        .map(std::path::PathBuf::from)
        .expect("set AGORA_VFS_DLL to target/debug/agora_vfs.dll (cargo build -p agora-vfs)");
    assert!(dll.is_file(), "{} does not exist", dll.display());
    dll
}

/// A game whose executable is a copy of cmd.exe, running `cmd_args` in its game folder.
#[cfg(windows)]
fn cmd_game(cmd_args: &[&str]) -> GameDefinition {
    let mut def = make_test_definition();
    let launch = def.launch.as_mut().unwrap();
    launch.arguments = cmd_args
        .iter()
        .map(|a| LaunchValue::Literal {
            value: a.to_string(),
        })
        .collect();
    def
}

#[cfg(windows)]
fn cmd_fixture(def: &GameDefinition) -> (TempDir, CoreContext, GameInstanceRecord, String) {
    let tmp = TempDir::new().unwrap();
    let ctx = create_test_context(&tmp, def);
    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(&install_dir).unwrap();
    setup_fake_install(&install_dir);
    std::fs::copy(r"C:\Windows\System32\cmd.exe", install_dir.join("Game.exe")).unwrap();
    let install = make_test_install(&install_dir, true, true);
    let inst = create_pinned_instance(&ctx, &install, def, "Injected");
    let item = add_content_folder(&ctx, "Mod", &[("Data/mod.txt", b"original")]);
    add_content(&ctx, &inst.instance_id, &item, None, None).unwrap();
    let sha = agora_core::content_store::get_item(&ctx, &item)
        .unwrap()
        .files[0]
        .sha256
        .clone();
    (tmp, ctx, inst, sha)
}

#[cfg(windows)]
#[test]
#[ignore = "injects agora_vfs.dll: cargo build -p agora-vfs, then set AGORA_VFS_DLL"]
fn real_injection_a_write_to_a_deployed_file_lands_in_the_writable_layer() {
    let def = cmd_game(&["/c", "echo", "changed>", r"Data\mod.txt"]);
    let (_tmp, ctx, inst, sha) = cmd_fixture(&def);
    let id = &inst.instance_id;
    let launcher = RealDllLauncher(built_dll());

    let mut launched = launch_with_fake_real(&ctx, id, &def, &launcher);
    assert_eq!(launched.prepared.deployment, Some(DeployMode::Virtual));
    assert!(
        launched.prepared.vfs.is_some(),
        "{:?}",
        launched.prepared.notice
    );
    assert!(
        launched.prepared.notice.is_none(),
        "{:?}",
        launched.prepared.notice
    );
    let status = launched.launched.child.wait().unwrap();
    assert!(status.success(), "cmd exited with {status}");

    let object = ctx.paths.content_object_path(&sha);
    assert_eq!(
        std::fs::read(&object).unwrap(),
        b"original",
        "content object changed"
    );
    let game_dir = deployment_dir(&ctx, id).unwrap().unwrap();
    assert_eq!(
        std::fs::read(game_dir.join("Data/mod.txt")).unwrap(),
        b"original",
        "the farm's link changed"
    );
    let written = std::fs::read_to_string(instance_writable_dir(&ctx, id).join("Data/mod.txt"))
        .expect("the write must land in the writable layer");
    assert_eq!(written.trim(), "changed");
    println!("writable layer holds {written:?}; content object still holds \"original\"");
    println!("vfs log:\n{}", read_vfs_log(&ctx, id));

    // Nothing needs harvesting: the plan now shows the game's file on top, and a rebuild for
    // the same mode is a no-op.
    assert_eq!(
        deploy(&ctx, id, &def, DeployMode::Virtual).unwrap(),
        DeployOutcome::UpToDate { plugins: None }
    );
}

#[cfg(windows)]
#[test]
#[ignore = "injects agora_vfs.dll: cargo build -p agora-vfs, then set AGORA_VFS_DLL"]
fn real_injection_a_delete_of_a_deployed_file_leaves_a_whiteout() {
    let def = cmd_game(&["/c", "del", r"Data\mod.txt"]);
    let (_tmp, ctx, inst, sha) = cmd_fixture(&def);
    let id = &inst.instance_id;
    let launcher = RealDllLauncher(built_dll());

    let mut launched = launch_with_fake_real(&ctx, id, &def, &launcher);
    assert!(
        launched.prepared.vfs.is_some(),
        "{:?}",
        launched.prepared.notice
    );
    let status = launched.launched.child.wait().unwrap();
    assert!(status.success(), "cmd exited with {status}");

    let object = ctx.paths.content_object_path(&sha);
    assert_eq!(
        std::fs::read(&object).unwrap(),
        b"original",
        "content object changed"
    );
    let marker = instance_writable_dir(&ctx, id).join(".agvfs-wh/Data/mod.txt.wh");
    assert!(
        marker.is_file(),
        "no whiteout marker at {}",
        marker.display()
    );
    println!("whiteout marker present: {}", marker.display());
    println!("vfs log:\n{}", read_vfs_log(&ctx, id));

    // The next deployment leaves the deleted file out, in every mode.
    for mode in [DeployMode::Virtual, DeployMode::Links, DeployMode::Copies] {
        let p = plan(&ctx, id, &def, mode).unwrap();
        assert!(find(&p, "Data/mod.txt").is_none(), "{mode}");
    }
}

/// The fixture program and DLL of `crates/agora-vfs/fixtures/early-import`, which sit beside
/// `agora_vfs.dll` in the target folder: `cargo build -p agora-vfs -p agora-vfs-early-import`.
#[cfg(windows)]
fn early_import_fixture() -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = built_dll().parent().unwrap().to_path_buf();
    let exe = dir.join("agora-early-import-exe.exe");
    let dll = dir.join("agora_early_import.dll");
    for file in [&exe, &dll] {
        assert!(
            file.is_file(),
            "{} does not exist: run `cargo build -p agora-vfs -p agora-vfs-early-import`",
            file.display()
        );
    }
    (exe, dll)
}

/// A game whose executable statically imports a DLL whose `DllMain` rewrites `early_<program>.txt`
/// beside it, the way Engine Fixes' preloader rewrites its log, and which starts a child that
/// does the same (the way SKSE's loader starts the game). Both writes must reach the writable
/// layer: the DLL has to be loaded, and its hooks installed, before the program's own imports in
/// the game and in its child.
#[cfg(windows)]
#[test]
#[ignore = "injects agora_vfs.dll: cargo build -p agora-vfs -p agora-vfs-early-import, then set AGORA_VFS_DLL"]
fn real_injection_loads_the_vfs_before_the_games_own_imports() {
    let (exe, dll) = early_import_fixture();
    let def = cmd_game(&["spawn"]);
    let tmp = TempDir::new().unwrap();
    let ctx = create_test_context(&tmp, &def);
    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(&install_dir).unwrap();
    setup_fake_install(&install_dir);
    std::fs::copy(&exe, install_dir.join("Game.exe")).unwrap();
    std::fs::copy(&exe, install_dir.join("Child.exe")).unwrap();
    std::fs::copy(&dll, install_dir.join("agora_early_import.dll")).unwrap();
    let install = make_test_install(&install_dir, true, true);
    let inst = create_pinned_instance(&ctx, &install, &def, "Early");
    let id = &inst.instance_id;
    // The files the DLL rewrites are a mod's: links into the content store.
    let item = add_content_folder(
        &ctx,
        "Mod",
        &[
            ("early_Game.txt", b"original"),
            ("early_Child.txt", b"original"),
        ],
    );
    add_content(&ctx, id, &item, None, None).unwrap();
    let objects: Vec<_> = agora_core::content_store::get_item(&ctx, &item)
        .unwrap()
        .files
        .iter()
        .map(|f| ctx.paths.content_object_path(&f.sha256))
        .collect();
    let launcher = RealDllLauncher(built_dll());

    let mut launched = launch_with_fake_real(&ctx, id, &def, &launcher);
    assert_eq!(launched.prepared.deployment, Some(DeployMode::Virtual));
    assert!(
        launched.prepared.vfs.is_some() && launched.prepared.notice.is_none(),
        "{:?}",
        launched.prepared.notice
    );
    let status = launched.launched.child.wait().unwrap();
    assert!(status.success(), "the fixture exited with {status}");

    let log = read_vfs_log(&ctx, id);
    println!("vfs log:\n{log}");
    for object in &objects {
        assert_eq!(
            std::fs::read(object).unwrap(),
            b"original",
            "a DllMain ran before the hooks and wrote through the link into the content store"
        );
    }
    let game_dir = deployment_dir(&ctx, id).unwrap().unwrap();
    for program in ["Game", "Child"] {
        let name = format!("early_{program}.txt");
        assert_eq!(
            std::fs::read(game_dir.join(&name)).unwrap(),
            b"original",
            "the farm's link {name} changed"
        );
        let written =
            std::fs::read(instance_writable_dir(&ctx, id).join(&name)).unwrap_or_else(|e| {
                panic!("{program}'s DllMain write must land in the writable layer: {e}")
            });
        assert_eq!(written, b"written by the fixture DllMain", "{program}");
        println!(
            "{program}'s DllMain write landed in the writable layer ({:?}); the content object still holds \"original\"",
            String::from_utf8_lossy(&written)
        );
    }
    assert!(
        log.contains("[agora] injected by import table"),
        "the game was not injected by its import table"
    );
    assert!(
        log.contains("injected child") && !log.contains("remote thread"),
        "the child was not injected by its import table"
    );
}

/// Import-table injection rewrites the suspended process's headers and import table; the DLL must
/// put them back when it loads (Detours' `DetourRestoreAfterWith`), because DRM such as SteamStub
/// reads them (real Skyrim hung at start-up without it). The fixture compares its own in-memory
/// headers with its file and exits non-zero if they differ.
#[cfg(windows)]
#[test]
#[ignore = "injects agora_vfs.dll: cargo build -p agora-vfs -p agora-vfs-early-import, then set AGORA_VFS_DLL"]
fn real_injection_leaves_the_games_headers_as_they_were_on_disk() {
    let (exe, dll) = early_import_fixture();
    let def = cmd_game(&["headers"]);
    let tmp = TempDir::new().unwrap();
    let ctx = create_test_context(&tmp, &def);
    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(&install_dir).unwrap();
    setup_fake_install(&install_dir);
    std::fs::copy(&exe, install_dir.join("Game.exe")).unwrap();
    std::fs::copy(&dll, install_dir.join("agora_early_import.dll")).unwrap();
    let install = make_test_install(&install_dir, true, true);
    let inst = create_pinned_instance(&ctx, &install, &def, "Headers");
    let id = &inst.instance_id;
    let launcher = RealDllLauncher(built_dll());

    let mut launched = launch_with_fake_real(&ctx, id, &def, &launcher);
    assert!(
        launched.prepared.vfs.is_some() && launched.prepared.notice.is_none(),
        "{:?}",
        launched.prepared.notice
    );
    let status = launched.launched.child.wait().unwrap();
    let log = read_vfs_log(&ctx, id);
    println!(
        "vfs log:
{log}"
    );
    assert!(
        log.contains("[agora] injected by import table"),
        "the game was not injected by its import table"
    );
    assert!(
        status.success(),
        "the game's in-memory headers differ from its file ({status}): the DLL did not restore them"
    );
    println!("the game's headers matched its file under import-table injection");
}

/// A 32-bit game from the 64-bit DLL: refused up front with the reason, nothing left running,
/// and the executable untouched.
#[cfg(windows)]
#[test]
#[ignore = "starts and ends a suspended process: needs a 32-bit cmd.exe (SysWOW64) and AGORA_VFS_DLL"]
fn real_injection_a_32_bit_game_is_refused_cleanly() {
    let wow64_cmd = std::path::Path::new(r"C:\Windows\SysWOW64\cmd.exe");
    assert!(wow64_cmd.is_file(), "no 32-bit cmd.exe on this machine");
    let def = cmd_game(&["/c", "exit", "0"]);
    let tmp = TempDir::new().unwrap();
    let ctx = create_test_context(&tmp, &def);
    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(&install_dir).unwrap();
    setup_fake_install(&install_dir);
    std::fs::copy(wow64_cmd, install_dir.join("Game.exe")).unwrap();
    let before = std::fs::read(install_dir.join("Game.exe")).unwrap();
    let install = make_test_install(&install_dir, true, true);
    let inst = create_pinned_instance(&ctx, &install, &def, "Wow64");
    let id = &inst.instance_id;
    let launcher = RealDllLauncher(built_dll());

    let err = agora_core::game_instance::launch_with(
        &ctx,
        id,
        &def,
        agora_core::game_instance::LaunchOptions {
            launch_anyway: false,
            plain: false,
            deployment: Some(DeployMode::Virtual),
        },
        &DiscoveryReport::default,
        &launcher,
    )
    .err()
    .expect("a chosen rung must fail");
    println!("chosen virtual, 32-bit game: {err}");
    assert!(
        matches!(err, InstanceError::VfsUnavailable { .. }),
        "{err:?}"
    );
    assert!(
        err.to_string().contains("built for another architecture"),
        "{err}"
    );
    let game_dir = deployment_dir(&ctx, id).unwrap().unwrap();
    for _ in 0..40 {
        if agora_core::game_launch::processes_running_from(&game_dir).is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(
        agora_core::game_launch::processes_running_from(&game_dir).is_empty(),
        "a suspended 32-bit game was left behind"
    );
    assert_eq!(
        std::fs::read(install_dir.join("Game.exe")).unwrap(),
        before,
        "the executable changed"
    );
}

#[cfg(windows)]
fn read_vfs_log(ctx: &CoreContext, id: &str) -> String {
    std::fs::read_to_string(ctx.paths.instance_dir(id).unwrap().join("logs/vfs.log"))
        .unwrap_or_else(|e| format!("(no log: {e})"))
}

#[cfg(windows)]
fn launch_with_fake_real(
    ctx: &CoreContext,
    id: &str,
    def: &GameDefinition,
    launcher: &RealDllLauncher,
) -> agora_core::game_instance::LaunchedInstance {
    agora_core::game_instance::launch_with(
        ctx,
        id,
        def,
        agora_core::game_instance::LaunchOptions {
            launch_anyway: false,
            plain: false,
            deployment: None,
        },
        &DiscoveryReport::default,
        launcher,
    )
    .expect("launch under the VFS")
}

/// A "DLL" that is not one: the injection is refused, the suspended game is ended, and nothing
/// of it keeps running.
#[cfg(windows)]
#[test]
#[ignore = "starts and ends a suspended process"]
fn real_injection_a_bad_dll_ends_the_suspended_game_and_steps_down_or_fails() {
    let def = cmd_game(&["/c", "exit", "0"]);
    let (tmp, ctx, inst, _sha) = cmd_fixture(&def);
    let id = &inst.instance_id;
    let bogus = tmp.path().join("agora_vfs.dll");
    std::fs::write(&bogus, b"this is not a dll").unwrap();
    let launcher = RealDllLauncher(bogus);

    // Chosen: a failure naming the reason, and no process left behind.
    let err = agora_core::game_instance::launch_with(
        &ctx,
        id,
        &def,
        agora_core::game_instance::LaunchOptions {
            launch_anyway: false,
            plain: false,
            deployment: Some(DeployMode::Virtual),
        },
        &DiscoveryReport::default,
        &launcher,
    )
    .err()
    .expect("a chosen rung must fail");
    println!("chosen virtual, bad dll: {err}");
    assert!(
        matches!(err, InstanceError::VfsUnavailable { .. }),
        "{err:?}"
    );
    let game_dir = deployment_dir(&ctx, id).unwrap().unwrap();
    for _ in 0..40 {
        if agora_core::game_launch::processes_running_from(&game_dir).is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(
        agora_core::game_launch::processes_running_from(&game_dir).is_empty(),
        "a suspended game was left behind"
    );

    // Not chosen: steps down to links, announced, and the game runs from there.
    let mut launched = launch_with_fake_real(&ctx, id, &def, &launcher);
    println!("default, bad dll: {:?}", launched.prepared.notice);
    assert_eq!(launched.prepared.deployment, Some(DeployMode::Links));
    assert!(launched
        .prepared
        .notice
        .as_deref()
        .unwrap()
        .starts_with("the virtual file system could not start: "));
    let status = launched.launched.child.wait().unwrap();
    assert!(status.success());
}

#[test]
fn harvesting_a_recreated_file_clears_its_whiteout_marker() {
    let tmp = TempDir::new().unwrap();
    let def = make_test_definition();
    let ctx = create_test_context(&tmp, &def);
    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(&install_dir).unwrap();
    setup_fake_install(&install_dir);
    let install = make_test_install(&install_dir, true, true);
    let inst = create_pinned_instance(&ctx, &install, &def, "Recreated");
    let id = &inst.instance_id;

    // The VFS deleted a base file in an earlier session...
    let writable = instance_writable_dir(&ctx, id);
    write_file(&writable.join(".agvfs-wh/base_file.txt.wh"), b"");
    let item = add_content_folder(&ctx, "Mod", &[("mod_file.txt", b"mod")]);
    add_content(&ctx, id, &item, None, None).unwrap();
    deploy(&ctx, id, &def, DeployMode::Links).unwrap();
    let game_dir = deployment_dir(&ctx, id).unwrap().unwrap();
    assert!(!game_dir.join("base_file.txt").exists());

    // ...and a game running from the link farm makes the file again.
    std::fs::write(game_dir.join("base_file.txt"), b"made again").unwrap();
    let report = undeploy(&ctx, id).unwrap();
    assert!(report
        .copied_to_writable
        .iter()
        .any(|p| p.as_str() == "base_file.txt"));

    // A stale marker would hide the new file under the VFS.
    assert!(!writable.join(".agvfs-wh/base_file.txt.wh").exists());
    assert_eq!(
        std::fs::read(writable.join("base_file.txt")).unwrap(),
        b"made again"
    );
}

#[test]
fn copy_patterns_in_links_mode() {
    let mut def = make_test_definition();
    def.copy_patterns = vec!["**/*.dat".into()];
    let tmp = TempDir::new().unwrap();
    let ctx = create_test_context(&tmp, &def);
    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(&install_dir).unwrap();
    setup_fake_install(&install_dir);
    let install = make_test_install(&install_dir, true, true);
    let inst = create_pinned_instance(&ctx, &install, &def, "CopyPatternsInstance");
    let id = &inst.instance_id;

    // Small settings.ini (copy)
    let small_ini = b"[Settings]\nfoo = bar\n";
    // .esp plugin (link)
    let esp_data = b"ESP BINARY DATA";
    // 2 MiB json (stays link)
    let large_json = vec![b' '; 2 * 1024 * 1024];
    // custom .dat (copy)
    let dat_data = b"custom binary dat";
    // case-insensitive SETTINGS.INI (copy)
    let upper_ini = b"[UPPER]\nx=1\n";

    let item = add_content_folder(
        &ctx,
        "ModWithConfigs",
        &[
            ("Mod/settings.ini", small_ini),
            ("Mod/plugin.esp", esp_data),
            ("Mod/large.json", &large_json),
            ("Mod/data.dat", dat_data),
            ("Mod2/SETTINGS.INI", upper_ini),
        ],
    );
    add_content(&ctx, id, &item, None, None).unwrap();

    let p = plan(&ctx, id, &def, DeployMode::Links).unwrap();
    assert_eq!(
        find(&p, "Mod/settings.ini").unwrap().placement,
        Placement::Copy
    );
    assert_eq!(
        find(&p, "Mod/plugin.esp").unwrap().placement,
        Placement::Link
    );
    assert_eq!(
        find(&p, "Mod/large.json").unwrap().placement,
        Placement::Link
    );
    assert_eq!(find(&p, "Mod/data.dat").unwrap().placement, Placement::Copy);
    assert_eq!(
        find(&p, "Mod2/SETTINGS.INI").unwrap().placement,
        Placement::Copy
    );

    let outcome = deploy(&ctx, id, &def, DeployMode::Links).unwrap();
    match outcome {
        DeployOutcome::Built { config_copied, .. } => {
            assert_eq!(
                config_copied, 3,
                "settings.ini, data.dat, SETTINGS.INI are small config copies"
            );
        }
        _ => panic!("expected DeployOutcome::Built"),
    }

    let game_dir = deployment_dir(&ctx, id).unwrap().unwrap();
    // Changing the copied file does not make the next deploy rebuild
    std::fs::write(
        game_dir.join("Mod/settings.ini"),
        b"[Settings]\nfoo = changed\n",
    )
    .unwrap();
    let second_deploy = deploy(&ctx, id, &def, DeployMode::Links).unwrap();
    assert!(
        matches!(second_deploy, DeployOutcome::UpToDate { .. }),
        "deploy should be up to date even after config copy changed"
    );

    // Undeploy harvests the change into the writable layer
    let report = undeploy(&ctx, id).unwrap();
    assert!(
        report
            .copied_to_writable
            .iter()
            .any(|p| p.as_str() == "Mod/settings.ini"),
        "changed settings.ini should be harvested into writable layer"
    );
    let writable = instance_writable_dir(&ctx, id);
    assert_eq!(
        std::fs::read(writable.join("Mod/settings.ini")).unwrap(),
        b"[Settings]\nfoo = changed\n"
    );
}

#[test]
fn own_copy_behaviour() {
    let def = make_test_definition();
    let tmp = TempDir::new().unwrap();
    let ctx = create_test_context(&tmp, &def);
    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(&install_dir).unwrap();
    setup_fake_install(&install_dir);
    let install = make_test_install(&install_dir, true, true);
    let inst = create_pinned_instance(&ctx, &install, &def, "OwnCopyInstance");
    let id = &inst.instance_id;

    let item1 = add_content_folder(
        &ctx,
        "ModOwn",
        &[("Mod/file1.bin", b"file 1"), ("Mod/file2.bin", b"file 2")],
    );
    let item2 = add_content_folder(&ctx, "ModOther", &[("Mod/other.bin", b"other")]);

    add_content(&ctx, id, &item1, None, None).unwrap();
    add_content(&ctx, id, &item2, None, None).unwrap();

    // Turn own_copy on for ModOwn
    set_content_own_copy(&ctx, id, &item1, true).unwrap();

    // In Links mode, every file of ModOwn is a copy, others stay links
    let p_links = plan(&ctx, id, &def, DeployMode::Links).unwrap();
    assert_eq!(
        find(&p_links, "Mod/file1.bin").unwrap().placement,
        Placement::Copy
    );
    assert_eq!(
        find(&p_links, "Mod/file2.bin").unwrap().placement,
        Placement::Copy
    );
    assert_eq!(
        find(&p_links, "Mod/other.bin").unwrap().placement,
        Placement::Link
    );

    // In Virtual mode, every file of ModOwn is also a copy, others stay links
    let p_virt = plan(&ctx, id, &def, DeployMode::Virtual).unwrap();
    assert_eq!(
        find(&p_virt, "Mod/file1.bin").unwrap().placement,
        Placement::Copy
    );
    assert_eq!(
        find(&p_virt, "Mod/file2.bin").unwrap().placement,
        Placement::Copy
    );
    assert_eq!(
        find(&p_virt, "Mod/other.bin").unwrap().placement,
        Placement::Link
    );

    // Deploy in Links mode
    deploy(&ctx, id, &def, DeployMode::Links).unwrap();
    let game_dir = deployment_dir(&ctx, id).unwrap().unwrap();

    // Edit a file in the game folder
    std::fs::write(game_dir.join("Mod/file1.bin"), b"file 1 edited by game").unwrap();

    // Turn own_copy off and redeploy: keeps the change in the writable layer
    set_content_own_copy(&ctx, id, &item1, false).unwrap();
    let outcome = deploy(&ctx, id, &def, DeployMode::Links).unwrap();
    assert!(matches!(outcome, DeployOutcome::Built { .. }));

    let writable = instance_writable_dir(&ctx, id);
    assert_eq!(
        std::fs::read(writable.join("Mod/file1.bin")).unwrap(),
        b"file 1 edited by game"
    );
}

#[test]
fn bepinex_case_end_to_end() {
    let def = make_test_definition();
    let tmp = TempDir::new().unwrap();
    let ctx = create_test_context(&tmp, &def);
    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(&install_dir).unwrap();
    setup_fake_install(&install_dir);
    let install = make_test_install(&install_dir, true, true);
    let inst = create_pinned_instance(&ctx, &install, &def, "BepInExInstance");
    let id = &inst.instance_id;

    let original_cfg = b"[Logging]\nUnityLog = true\n";
    let bepinex_item = add_content_folder(
        &ctx,
        "BepInExPack",
        &[("BepInEx/config/BepInEx.cfg", original_cfg)],
    );
    add_content(&ctx, id, &bepinex_item, None, None).unwrap();

    // Deploy in Links mode
    let outcome = deploy(&ctx, id, &def, DeployMode::Links).unwrap();
    match outcome {
        DeployOutcome::Built { config_copied, .. } => {
            assert_eq!(
                config_copied, 1,
                "BepInEx.cfg matched default config copy pattern"
            );
        }
        _ => panic!("expected DeployOutcome::Built"),
    }

    let game_dir = deployment_dir(&ctx, id).unwrap().unwrap();
    let cfg_path = game_dir.join("BepInEx/config/BepInEx.cfg");

    // File is writable in the game folder (open for write succeeds)
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(&cfg_path)
        .expect("open for write succeeds on small config copy in link mode");
    file.write_all(b"[Logging]\nUnityLog = false\n").unwrap();
    drop(file);

    // Content object in content store stays protected and unchanged
    let item_info = agora_core::content_store::get_item(&ctx, &bepinex_item).unwrap();
    let sha256 = &item_info.files[0].sha256;
    let obj_path = ctx.paths.content_object_path(sha256);
    let obj_bytes = std::fs::read(&obj_path).unwrap();
    assert_eq!(
        obj_bytes, original_cfg,
        "content store object remains unchanged"
    );
}

// ---- Review probes (slice 10) ----

fn probe_fixture(name: &str) -> (TempDir, CoreContext, GameDefinition, String) {
    let mut def = make_test_definition();
    def.declared_writes = vec!["Logs/**".into()];
    let tmp = TempDir::new().unwrap();
    let ctx = create_test_context(&tmp, &def);
    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(&install_dir).unwrap();
    setup_fake_install(&install_dir);
    let install = make_test_install(&install_dir, true, true);
    let inst = create_pinned_instance(&ctx, &install, &def, name);
    let id = inst.instance_id.clone();
    (tmp, ctx, def, id)
}

#[test]
fn probe_size_boundary_root_level_and_big_declared_write() {
    let (_tmp, ctx, def, id) = probe_fixture("ProbeBoundary");
    let exactly = vec![b'a'; 1024 * 1024];
    let over = vec![b'a'; 1024 * 1024 + 1];
    let big_log = vec![b'l'; 3 * 1024 * 1024];
    let item = add_content_folder(
        &ctx,
        "ProbeMod",
        &[
            ("exact.json", &exactly),
            ("over.json", &over),
            ("doorstop_config.ini", b"[General]\n"),
            ("Logs/big.cfg", &big_log),
        ],
    );
    add_content(&ctx, &id, &item, None, None).unwrap();
    let p = plan(&ctx, &id, &def, DeployMode::Links).unwrap();
    assert_eq!(find(&p, "exact.json").unwrap().placement, Placement::Copy);
    assert_eq!(find(&p, "over.json").unwrap().placement, Placement::Link);
    assert_eq!(
        find(&p, "doorstop_config.ini").unwrap().placement,
        Placement::Copy
    );
    assert_eq!(find(&p, "Logs/big.cfg").unwrap().placement, Placement::Copy);
    // Virtual is unaffected by config copies.
    let v = plan(&ctx, &id, &def, DeployMode::Virtual).unwrap();
    assert_eq!(
        find(&v, "doorstop_config.ini").unwrap().placement,
        Placement::Link
    );
}

#[test]
fn probe_own_copy_and_enable_refuse_an_empty_prefix() {
    let (_tmp, ctx, def, id) = probe_fixture("ProbeEmptyPrefix");
    let a = add_content_folder(&ctx, "ProbeA", &[("A/a.bin", b"a")]);
    let b = add_content_folder(&ctx, "ProbeB", &[("B/b.bin", b"b")]);
    add_content(&ctx, &id, &a, None, None).unwrap();
    add_content(&ctx, &id, &b, None, None).unwrap();
    assert!(
        set_content_own_copy(&ctx, &id, "", true).is_err(),
        "empty prefix must not match every layer"
    );
    assert!(
        set_content_enabled(&ctx, &id, "", false).is_err(),
        "empty prefix must not pick a layer"
    );
    let p = plan(&ctx, &id, &def, DeployMode::Links).unwrap();
    assert_eq!(find(&p, "A/a.bin").unwrap().placement, Placement::Link);
    assert_eq!(find(&p, "B/b.bin").unwrap().placement, Placement::Link);
    // A full id affects only its own layer.
    set_content_own_copy(&ctx, &id, &a, true).unwrap();
    let p = plan(&ctx, &id, &def, DeployMode::Links).unwrap();
    assert_eq!(find(&p, "A/a.bin").unwrap().placement, Placement::Copy);
    assert_eq!(find(&p, "B/b.bin").unwrap().placement, Placement::Link);
}

#[test]
fn probe_own_copy_on_a_disabled_layer_changes_nothing() {
    let (_tmp, ctx, def, id) = probe_fixture("ProbeDisabledOwn");
    let a = add_content_folder(&ctx, "ProbeDis", &[("A/a.bin", b"a")]);
    add_content(&ctx, &id, &a, None, None).unwrap();
    set_content_own_copy(&ctx, &id, &a, true).unwrap();
    set_content_enabled(&ctx, &id, &a, false).unwrap();
    let p = plan(&ctx, &id, &def, DeployMode::Links).unwrap();
    assert!(find(&p, "A/a.bin").is_none());
}

#[test]
fn probe_an_edited_config_copy_survives_redeploy_and_is_harvested() {
    let (_tmp, ctx, def, id) = probe_fixture("ProbeEditCfg");
    let item = add_content_folder(&ctx, "ProbeCfg", &[("Mod/x.cfg", b"original")]);
    add_content(&ctx, &id, &item, None, None).unwrap();
    deploy(&ctx, &id, &def, DeployMode::Links).unwrap();
    let game = deployment_dir(&ctx, &id).unwrap().unwrap();
    std::fs::write(game.join("Mod/x.cfg"), b"edited by the game, longer").unwrap();
    let again = deploy(&ctx, &id, &def, DeployMode::Links).unwrap();
    assert!(
        matches!(again, DeployOutcome::UpToDate { .. }),
        "got {again:?}"
    );
    assert_eq!(
        std::fs::read(game.join("Mod/x.cfg")).unwrap(),
        b"edited by the game, longer"
    );
    undeploy(&ctx, &id).unwrap();
    assert_eq!(
        std::fs::read(instance_writable_dir(&ctx, &id).join("Mod/x.cfg")).unwrap(),
        b"edited by the game, longer"
    );
}
