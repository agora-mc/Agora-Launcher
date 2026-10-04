use std::path::Path;

use agora_core::ctx::CoreContext;
use agora_core::game_base::{remove_base, BaseError, BaseMode, BuildOutcome};
use agora_core::game_discovery::{DiscoveredInstall, DiscoveryReport, InstallCapabilities};
use agora_core::game_instance::{
    create, delete, get, get_manifest, prepare_launch, prepare_launch_with_discovery,
    record_launch, GameInstanceManifest, InstanceError,
};
use agora_core::game_launch::LaunchError;
use agora_core::game_registry::{IdentifiedInstall, RuntimeResolution};
use agora_game_api::{
    BaseReference, DeploymentStrategy, GameDefinition, GameId, GamePath, InstallId, InstallKind,
    LaunchRecipe, LaunchValue, RelPath, RuntimeIdentity, StoreId, StoreIdentifier,
};
use tempfile::TempDir;

fn make_test_definition() -> GameDefinition {
    GameDefinition {
        id: GameId::new("skyrim-se").unwrap(),
        name: "Skyrim Special Edition".into(),
        stores: vec![
            StoreIdentifier {
                store: StoreId::new("steam").unwrap(),
                product: "489830".into(),
            },
            StoreIdentifier {
                store: StoreId::new("microsoft-store").unwrap(),
                product: "BethesdaSoftworks.SkyrimSE-PC".into(),
            },
        ],
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
        declared_writes: vec![],
    }
}

fn setup_fake_install(dir: &Path) {
    std::fs::write(dir.join("Game.exe"), b"fake game binary contents").unwrap();
    let data_dir = dir.join("Data");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::write(data_dir.join("Skyrim.bsa"), b"BSA ARCHIVE DATA 1234567890").unwrap();
}

fn make_test_install(
    install_dir: &Path,
    store: &str,
    product: &str,
    readable: bool,
    relocatable: bool,
) -> IdentifiedInstall {
    let store_id = StoreId::new(store).unwrap();
    let runtime = RuntimeIdentity {
        game: GameId::new("skyrim-se").unwrap(),
        store: store_id.clone(),
        version: "1.6.1170.0".into(),
        build: None,
    };
    let detector = agora_core::game_discovery::volume::VolumeDetector::new();
    let volume = detector.get_volume_info(install_dir);
    let install_id = InstallId::new(format!("{store}:{product}")).unwrap();
    let discovered = DiscoveredInstall {
        store: store_id,
        product: product.into(),
        name: "Skyrim Special Edition".into(),
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

#[test]
fn two_instances_from_one_install_one_base_built_once() {
    let tmp = TempDir::new().unwrap();
    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(&install_dir).unwrap();
    setup_fake_install(&install_dir);

    let ctx = CoreContext::for_testing(tmp.path().join("app_data"));
    agora_core::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();

    let def = make_test_definition();
    let install = make_test_install(&install_dir, "steam", "489830", true, true);

    // Instance 1
    let rec1 = create(
        &ctx,
        &install,
        &def,
        "Playthrough 1",
        None,
        BaseMode::Linked,
        &|_| {},
    )
    .unwrap();

    assert!(matches!(
        rec1.build_outcome,
        Some(BuildOutcome::Built { .. })
    ));
    let base_id1 = match &rec1.base {
        BaseReference::Pinned { id, .. } => id.clone(),
        _ => panic!("expected pinned base"),
    };

    // Instance 2 from the same install
    let rec2 = create(
        &ctx,
        &install,
        &def,
        "Playthrough 2",
        None,
        BaseMode::Linked,
        &|_| {},
    )
    .unwrap();

    assert!(matches!(
        rec2.build_outcome,
        Some(BuildOutcome::Existing(_))
    ));
    let base_id2 = match &rec2.base {
        BaseReference::Pinned { id, .. } => id.clone(),
        _ => panic!("expected pinned base"),
    };

    // Both pin the same base
    assert_eq!(base_id1, base_id2);

    // IDs are unique and valid
    assert_ne!(rec1.instance_id, rec2.instance_id);
    assert!(ctx.paths.instance_dir(&rec1.instance_id).is_ok());
    assert!(ctx.paths.instance_dir(&rec2.instance_id).is_ok());

    // Manifests round-trip correctly
    let m1: GameInstanceManifest = get_manifest(&ctx, &rec1.instance_id).unwrap();
    assert_eq!(m1.instance_id, rec1.instance_id);
    assert_eq!(m1.name, "Playthrough 1");
    assert_eq!(m1.manifest_version, 3);
    assert_eq!(m1.base, rec1.base);

    let m2: GameInstanceManifest = get_manifest(&ctx, &rec2.instance_id).unwrap();
    assert_eq!(m2.instance_id, rec2.instance_id);
    assert_eq!(m2.name, "Playthrough 2");
    assert_eq!(m2.manifest_version, 3);
    assert_eq!(m2.base, rec2.base);
}

#[test]
fn unpinned_install_creates_unpinned_instance_with_reason_and_launch_never_verifies() {
    let tmp = TempDir::new().unwrap();
    let install_dir = tmp.path().join("ms_store_install");
    std::fs::create_dir_all(&install_dir).unwrap();
    setup_fake_install(&install_dir);

    let ctx = CoreContext::for_testing(tmp.path().join("app_data"));
    agora_core::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();

    let def = make_test_definition();
    let install = make_test_install(
        &install_dir,
        "microsoft-store",
        "bethesda-softworks-skyrim-se-pc",
        false, // executables not readable
        true,
    );

    let rec = create(
        &ctx,
        &install,
        &def,
        "Store Skyrim",
        None,
        BaseMode::Linked,
        &|_| {},
    )
    .unwrap();

    let reason_str = match &rec.base {
        BaseReference::Unpinned { reason, .. } => reason.clone(),
        _ => panic!("expected unpinned base"),
    };
    assert!(
        reason_str.contains("executables are not readable"),
        "reason should explain missing capability: {reason_str}"
    );

    // Launch resolves against the install folder and never verifies
    let report = DiscoveryReport {
        installs: vec![install.discovered.clone()],
        warnings: vec![],
    };
    let prepared =
        prepare_launch_with_discovery(&ctx, &rec.instance_id, &def, false, &|| report.clone())
            .unwrap();

    assert_eq!(prepared.resolved.program, install_dir.join("Game.exe"));
    assert_eq!(prepared.resolved.cwd, install_dir);
    assert!(prepared.warnings.is_empty());
}

#[test]
fn remove_base_refuses_pinned_base_naming_instances_and_succeeds_after_both_deleted() {
    let tmp = TempDir::new().unwrap();
    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(&install_dir).unwrap();
    setup_fake_install(&install_dir);

    let ctx = CoreContext::for_testing(tmp.path().join("app_data"));
    agora_core::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();

    let def = make_test_definition();
    let install = make_test_install(&install_dir, "steam", "489830", true, true);

    let rec1 = create(
        &ctx,
        &install,
        &def,
        "First",
        Some("skyrim-first".into()),
        BaseMode::Linked,
        &|_| {},
    )
    .unwrap();

    let rec2 = create(
        &ctx,
        &install,
        &def,
        "Second",
        Some("skyrim-second".into()),
        BaseMode::Linked,
        &|_| {},
    )
    .unwrap();

    let base_id = match &rec1.base {
        BaseReference::Pinned { id, .. } => id.clone(),
        _ => panic!("expected pinned base"),
    };

    // remove_base refuses naming both instances
    match remove_base(&ctx.paths, &base_id) {
        Err(BaseError::InUse { instances }) => {
            assert!(instances.contains(&rec1.instance_id));
            assert!(instances.contains(&rec2.instance_id));
        }
        other => panic!("expected BaseError::InUse, got: {other:?}"),
    }

    // Delete first instance
    let outcome1 = delete(&ctx, &rec1.instance_id).unwrap();
    assert_eq!(outcome1.orphaned_base, None);

    // remove_base still refuses naming the second instance
    match remove_base(&ctx.paths, &base_id) {
        Err(BaseError::InUse { instances }) => {
            assert_eq!(instances, vec![rec2.instance_id.clone()]);
        }
        other => panic!("expected BaseError::InUse naming second, got: {other:?}"),
    }

    // Delete second instance
    let outcome2 = delete(&ctx, &rec2.instance_id).unwrap();
    assert_eq!(outcome2.orphaned_base, Some(base_id.clone()));

    // Now remove_base succeeds!
    assert!(remove_base(&ctx.paths, &base_id).is_ok());
    assert!(!ctx.paths.base_manifest_path(&base_id).exists());
}

#[test]
fn delete_refuses_id_that_only_exists_in_user_instances_and_leaves_folder_alone() {
    let tmp = TempDir::new().unwrap();
    let ctx = CoreContext::for_testing(tmp.path().join("app_data"));
    agora_core::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();

    let conn = agora_core::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
    conn.execute(
        "INSERT INTO user_instances (instance_id, name, minecraft_version, loader, loader_version)
         VALUES ('mc-vanilla', 'Vanilla MC', '1.21', 'vanilla', '')",
        [],
    )
    .unwrap();

    let mc_dir = ctx.paths.instance_dir("mc-vanilla").unwrap();
    std::fs::create_dir_all(&mc_dir).unwrap();
    let marker_file = mc_dir.join("marker.txt");
    std::fs::write(&marker_file, b"important minecraft data").unwrap();

    // delete through game_instance should refuse
    let err = delete(&ctx, "mc-vanilla").unwrap_err();
    assert!(matches!(err, InstanceError::MinecraftInstance(_)));

    // Folder and marker file remain intact
    assert!(mc_dir.exists());
    assert!(marker_file.exists());
}

#[test]
fn tampered_pinned_instance_launch_is_refused_naming_file_and_record_launch_sets_time() {
    let tmp = TempDir::new().unwrap();
    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(&install_dir).unwrap();
    setup_fake_install(&install_dir);

    let ctx = CoreContext::for_testing(tmp.path().join("app_data"));
    agora_core::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();

    let def = make_test_definition();
    let install = make_test_install(&install_dir, "steam", "489830", true, true);

    let rec = create(
        &ctx,
        &install,
        &def,
        "Tamper Test",
        None,
        BaseMode::Linked,
        &|_| {},
    )
    .unwrap();

    let base_id = match &rec.base {
        BaseReference::Pinned { id, .. } => id.clone(),
        _ => panic!("expected pinned base"),
    };

    let manifest_path = ctx.paths.base_manifest_path(&base_id);
    let manifest_content = std::fs::read_to_string(&manifest_path).unwrap();
    let manifest: agora_core::game_base::BaseManifest =
        serde_json::from_str(&manifest_content).unwrap();

    // Tamper with a file in the base
    let base_file = manifest.location.join("Data/Skyrim.bsa");
    std::fs::write(&base_file, b"corrupted bytes different length").unwrap();

    // Launch should be refused naming the file
    let err = prepare_launch(&ctx, &rec.instance_id, &def, false).unwrap_err();
    match err {
        InstanceError::LaunchError(LaunchError::BaseDamaged { problems }) => {
            assert!(
                problems.iter().any(|p| p.path == "Data/Skyrim.bsa"),
                "problems should name the tampered file: {problems:?}"
            );
        }
        other => panic!("expected BaseDamaged error, got {other:?}"),
    }

    // Launch anyway should succeed with warning
    let prepared = prepare_launch(&ctx, &rec.instance_id, &def, true).unwrap();
    assert!(!prepared.warnings.is_empty());

    // Record launch sets last_launched_at
    assert_eq!(rec.last_launched_at, None);
    record_launch(&ctx, &rec.instance_id).unwrap();

    let updated = get(&ctx, &rec.instance_id).unwrap().unwrap();
    assert!(updated.last_launched_at.is_some());
}

fn fresh(tmp: &TempDir) -> (CoreContext, IdentifiedInstall, GameDefinition) {
    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(&install_dir).unwrap();
    setup_fake_install(&install_dir);
    let ctx = CoreContext::for_testing(tmp.path().join("app_data"));
    agora_core::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();
    let install = make_test_install(&install_dir, "steam", "489830", true, true);
    (ctx, install, make_test_definition())
}

/// "Cannot tell which instances use this base" must keep the base, never
/// read as "nothing uses it".
#[test]
fn an_unreadable_instance_row_keeps_its_base() {
    let tmp = TempDir::new().unwrap();
    let (ctx, install, def) = fresh(&tmp);
    let rec = create(
        &ctx,
        &install,
        &def,
        "Keep",
        None,
        BaseMode::Linked,
        &|_| {},
    )
    .unwrap();
    let BaseReference::Pinned { id: base_id, .. } = rec.base else {
        panic!("pinned")
    };
    let base_dir = rec.build_outcome.unwrap().manifest().location.clone();

    let conn = rusqlite::Connection::open(ctx.paths.local_state_db()).unwrap();
    conn.execute(
        "UPDATE game_instances SET base_json = 'not json' WHERE instance_id = ?1",
        [&rec.instance_id],
    )
    .unwrap();
    assert!(remove_base(&ctx.paths, &base_id).is_err());
    assert!(
        base_dir.exists(),
        "a base whose users cannot be read is kept"
    );

    conn.execute("DROP TABLE game_instances", []).unwrap();
    assert!(remove_base(&ctx.paths, &base_id).is_err());
    assert!(base_dir.exists());
}

/// A taken or invalid --id is refused before a base is built.
#[test]
fn a_bad_chosen_id_is_refused_before_any_base_is_built() {
    let tmp = TempDir::new().unwrap();
    let (ctx, install, def) = fresh(&tmp);
    let first = create(
        &ctx,
        &install,
        &def,
        "A",
        Some("taken".into()),
        BaseMode::Linked,
        &|_| {},
    )
    .unwrap();
    let base_id = match first.base {
        BaseReference::Pinned { id, .. } => id,
        _ => unreachable!(),
    };
    // With the only instance gone and its base removed, nothing exists to reuse.
    delete(&ctx, "taken").unwrap();
    remove_base(&ctx.paths, &base_id).unwrap();
    let conn = rusqlite::Connection::open(ctx.paths.local_state_db()).unwrap();
    conn.execute(
        "INSERT INTO user_instances (instance_id, name, minecraft_version, loader, loader_version) VALUES ('taken', 'mc', '1.21', 'vanilla', '')",
        [],
    )
    .unwrap();

    for bad in ["taken", "../escape", ""] {
        let built = std::sync::atomic::AtomicBool::new(false);
        let result = create(
            &ctx,
            &install,
            &def,
            "B",
            Some(bad.into()),
            BaseMode::Linked,
            &|_| built.store(true, std::sync::atomic::Ordering::SeqCst),
        );
        assert!(result.is_err(), "{bad:?} must be refused");
        assert!(
            !built.load(std::sync::atomic::Ordering::SeqCst),
            "{bad:?} started a base build first"
        );
        assert!(!ctx.paths.base_manifest_path(&base_id).exists());
    }
}
