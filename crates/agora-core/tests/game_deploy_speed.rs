//! Deploy speed work (MASTER_SPEC §26.5): parallel placement must give the planned farm and record,
//! a midway failure must leave no deployment, and the up-to-date check must keep its verdicts.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agora_core::ctx::CoreContext;
use agora_core::game_base::{BaseManifest, BaseMode};
use agora_core::game_deploy::{
    add_content, deploy, deployment_dir, plan, source_path, undeploy, DeployError, DeployMode,
    DeployOutcome, DeploymentPlan, DeploymentRecord, FileSource, Placement,
};
use agora_core::game_discovery::{DiscoveredInstall, InstallCapabilities};
use agora_core::game_instance::{create, get_manifest, GameInstanceRecord};
use agora_core::game_registry::{
    GameRegistry, IdentifiedInstall, PackageSource, RuntimeResolution,
};
use agora_game_api::{
    DeploymentStrategy, GameDefinition, GameId, GamePackage, GamePath, InstallId, InstallKind,
    LaunchRecipe, LaunchValue, PackageDefinition, RelPath, RuntimeIdentity, StoreId,
    StoreIdentifier,
};
use tempfile::TempDir;

struct TestPackage(PackageDefinition);

impl GamePackage for TestPackage {
    fn definition(&self) -> &PackageDefinition {
        &self.0
    }
}

fn definition() -> GameDefinition {
    GameDefinition {
        mo2_game_name: None,
        id: GameId::new("speed-game").unwrap(),
        name: "Speed Game".into(),
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
        save_location: Vec::new(),
        launch_alternatives: Vec::new(),
        content_layout: None,
        copy_patterns: Vec::new(),
    }
}

fn register(def: GameDefinition) -> Arc<GameRegistry> {
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

struct Fixture {
    _tmp: TempDir,
    ctx: CoreContext,
    def: GameDefinition,
    install: IdentifiedInstall,
}

fn fixture() -> Fixture {
    let tmp = TempDir::new().unwrap();
    let def = definition();
    let ctx = CoreContext::for_testing(tmp.path().join("app_data"));
    agora_core::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();
    let ctx = ctx.with_games(register(def.clone()));

    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(install_dir.join("Data")).unwrap();
    std::fs::write(install_dir.join("Game.exe"), b"fake game binary").unwrap();
    std::fs::write(install_dir.join("base_file.txt"), b"initial base file").unwrap();
    std::fs::write(
        install_dir.join("writeable_base.txt"),
        b"initial writeable base",
    )
    .unwrap();
    std::fs::write(install_dir.join("Data").join("Skyrim.bsa"), b"BSA DATA").unwrap();
    for k in 0..25 {
        std::fs::write(
            install_dir.join("Data").join(format!("base{k:02}.bin")),
            format!("base {k}"),
        )
        .unwrap();
    }
    let install = make_install(&install_dir, &def);
    Fixture {
        _tmp: tmp,
        ctx,
        def,
        install,
    }
}

fn make_install(install_dir: &Path, def: &GameDefinition) -> IdentifiedInstall {
    let store_id = StoreId::new("steam").unwrap();
    let runtime = RuntimeIdentity {
        game: def.id.clone(),
        store: store_id.clone(),
        version: "1.0.0".into(),
        build: None,
    };
    let detector = agora_core::game_discovery::volume::VolumeDetector::new();
    let volume = detector.get_volume_info(install_dir);
    let discovered = DiscoveredInstall {
        store: store_id,
        product: "12345".into(),
        name: "Speed Game".into(),
        kind: InstallKind::BaseGame,
        parent_product: None,
        location: install_dir.to_path_buf(),
        store_version: Some(runtime.version.clone()),
        store_build: runtime.build.clone(),
        executables: vec!["Game.exe".into()],
        capabilities: InstallCapabilities {
            executables_readable: true,
            accepts_new_files: true,
            relocatable: true,
        },
        volume,
    };
    IdentifiedInstall {
        game: runtime.game.clone(),
        install_id: InstallId::new("steam:12345").unwrap(),
        discovered,
        add_ons: vec![],
        runtime: RuntimeResolution::Identified {
            runtime,
            source: "executable".into(),
        },
    }
}

fn new_instance(f: &Fixture, name: &str) -> GameInstanceRecord {
    create(
        &f.ctx,
        &f.install,
        &f.def,
        name,
        None,
        BaseMode::Linked,
        &|_| {},
    )
    .expect("create instance")
}

fn add_item(f: &Fixture, name: &str, files: &[(&str, &[u8])]) -> String {
    let dir = TempDir::new().unwrap();
    for (rel, body) in files {
        let path = dir.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, body).unwrap();
    }
    let outcome = agora_core::content_store::add_folder(&f.ctx, dir.path(), Some(name)).unwrap();
    outcome.item().item_id.clone()
}

/// Two content items, one of them overriding a path of the other, with a config file (a copy) and
/// nested folders.
fn add_standard_content(f: &Fixture, id: &str) {
    let one = add_item(
        f,
        "ModOne",
        &[
            ("Data/a/one.txt", b"one"),
            ("Data/a/two.ini", b"[two]\nvalue=2\n"),
            ("Data/b/deep/three.dat", b"three three three"),
            ("Data/shared.txt", b"from one"),
        ],
    );
    let two = add_item(
        f,
        "ModTwo",
        &[
            ("Data/c/four.txt", b"four"),
            ("Data/shared.txt", b"from two, wins"),
            ("Data/c/five.ini", b"[five]\n"),
        ],
    );
    add_content(&f.ctx, id, &one, None, None).unwrap();
    add_content(&f.ctx, id, &two, None, None).unwrap();
}

fn game_dir(f: &Fixture, id: &str) -> PathBuf {
    deployment_dir(&f.ctx, id).unwrap().expect("a deployment")
}

fn read_record(f: &Fixture, id: &str) -> DeploymentRecord {
    let dir = game_dir(f, id).parent().unwrap().to_path_buf();
    let text = std::fs::read_to_string(dir.join("deployment.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

/// Every file under `root` with its bytes, keyed by its `/`-separated relative path.
fn read_tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut folders = vec![root.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                folders.push(path);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(rel, std::fs::read(&path).unwrap());
            }
        }
    }
    out
}

/// The farm must hold exactly the planned files, with the planned bytes, and the record must list
/// them in plan order with the planned placement.
fn assert_farm_matches_plan(f: &Fixture, id: &str, plan: &DeploymentPlan, mode: DeployMode) {
    let game = game_dir(f, id);
    let farm = read_tree(&game);
    let planned: BTreeMap<String, Vec<u8>> = plan
        .files
        .iter()
        .map(|file| {
            let bytes = std::fs::read(source_path(&f.ctx, &file.source)).unwrap();
            (file.path.as_str().to_string(), bytes)
        })
        .collect();
    assert_eq!(
        farm.keys().collect::<Vec<_>>(),
        planned.keys().collect::<Vec<_>>(),
        "the farm holds the planned files and nothing else"
    );
    assert_eq!(farm, planned, "every farm file has its planned bytes");

    let record = read_record(f, id);
    assert_eq!(record.mode, mode);
    assert_eq!(record.fingerprint, plan.fingerprint);
    assert_eq!(record.files.len(), plan.files.len());
    for (recorded, planned) in record.files.iter().zip(&plan.files) {
        assert_eq!(recorded.path, planned.path, "record in plan order");
        assert_eq!(recorded.size, planned.size, "{}", planned.path);
        assert_eq!(recorded.placement, planned.placement, "{}", planned.path);
        assert_eq!(recorded.source, planned.source, "{}", planned.path);
    }
}

#[test]
fn parallel_links_deploy_matches_the_plan_file_for_file() {
    let f = fixture();
    let inst = new_instance(&f, "Parallel");
    add_standard_content(&f, &inst.instance_id);

    let planned = plan(&f.ctx, &inst.instance_id, &f.def, DeployMode::Links).unwrap();
    let outcome = deploy(&f.ctx, &inst.instance_id, &f.def, DeployMode::Links).unwrap();
    match outcome {
        DeployOutcome::Built { linked, copied, .. } => {
            let planned_links = planned
                .files
                .iter()
                .filter(|file| file.placement == Placement::Link)
                .count();
            assert_eq!(linked, planned_links, "every planned link is a link");
            assert_eq!(copied, planned.files.len() - planned_links);
        }
        other => panic!("expected a build, got {other:?}"),
    }
    assert_farm_matches_plan(&f, &inst.instance_id, &planned, DeployMode::Links);
    // The override won: the later item's text is in the farm.
    let shared = std::fs::read(game_dir(&f, &inst.instance_id).join("Data/shared.txt")).unwrap();
    assert_eq!(shared, b"from two, wins");
}

#[test]
fn parallel_copies_deploy_matches_the_plan_file_for_file() {
    let f = fixture();
    let inst = new_instance(&f, "ParallelCopies");
    add_standard_content(&f, &inst.instance_id);

    let planned = plan(&f.ctx, &inst.instance_id, &f.def, DeployMode::Copies).unwrap();
    deploy(&f.ctx, &inst.instance_id, &f.def, DeployMode::Copies).unwrap();
    assert_farm_matches_plan(&f, &inst.instance_id, &planned, DeployMode::Copies);
    assert!(planned
        .files
        .iter()
        .all(|file| file.placement == Placement::Copy));
}

#[test]
fn a_deploy_that_fails_midway_returns_the_error_and_leaves_no_deployment() {
    let f = fixture();
    let inst = new_instance(&f, "Midway");
    let id = inst.instance_id.clone();
    add_standard_content(&f, &id);
    deploy(&f.ctx, &id, &f.def, DeployMode::Links).unwrap();
    assert!(deployment_dir(&f.ctx, &id).unwrap().is_some());

    // A third item whose one file's content object is gone: planning still works (it reads no
    // objects), but placing that file cannot.
    let broken = add_item(
        &f,
        "Broken",
        &[("Data/z/ok.txt", b"ok"), ("Data/z/gone.dat", b"gone")],
    );
    add_content(&f.ctx, &id, &broken, None, None).unwrap();
    let planned = plan(&f.ctx, &id, &f.def, DeployMode::Links).unwrap();
    let gone = planned
        .files
        .iter()
        .find(|file| file.path.as_str() == "Data/z/gone.dat")
        .unwrap();
    let FileSource::Content { sha256, .. } = &gone.source else {
        panic!("the file is a content file");
    };
    let object = f.ctx.paths.content_object_path(sha256);
    let saved = std::fs::read(&object).unwrap();
    std::fs::remove_file(&object).unwrap();

    let err = deploy(&f.ctx, &id, &f.def, DeployMode::Links).unwrap_err();
    assert!(matches!(err, DeployError::Io(_)), "{err:?}");

    // The deployment is gone (the old one was harvested first), and no staging folder is left.
    assert!(deployment_dir(&f.ctx, &id).unwrap().is_none());
    let base_id = match get_manifest(&f.ctx, &id).unwrap().base {
        agora_game_api::BaseReference::Pinned { id: base_id, .. } => base_id,
        other => panic!("the instance is pinned: {other:?}"),
    };
    let base_text = std::fs::read_to_string(f.ctx.paths.base_manifest_path(&base_id)).unwrap();
    let base: BaseManifest = serde_json::from_str(&base_text).unwrap();
    let deployments = base
        .location
        .parent()
        .unwrap()
        .join("deployments")
        .join(&id);
    if deployments.exists() {
        for entry in std::fs::read_dir(&deployments).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().to_string();
            assert!(
                !name.starts_with("game.partial-") && name != "game" && name != "deployment.json",
                "left behind: {name}"
            );
        }
    }

    // Put the object back: the next deploy builds the whole farm.
    std::fs::write(&object, saved).unwrap();
    let outcome = deploy(&f.ctx, &id, &f.def, DeployMode::Links).unwrap();
    assert!(
        matches!(outcome, DeployOutcome::Built { .. }),
        "{outcome:?}"
    );
    assert_farm_matches_plan(&f, &id, &planned, DeployMode::Links);
}

#[test]
fn the_up_to_date_check_rebuilds_after_a_deleted_file() {
    let f = fixture();
    let inst = new_instance(&f, "Deleted");
    let id = inst.instance_id.clone();
    add_standard_content(&f, &id);
    deploy(&f.ctx, &id, &f.def, DeployMode::Links).unwrap();

    // A file the game no longer has is a deletion: the rebuild records it as a whiteout, so the
    // file stays gone, and the rebuild is what the check decided.
    std::fs::remove_file(game_dir(&f, &id).join("Data/a/one.txt")).unwrap();
    let outcome = deploy(&f.ctx, &id, &f.def, DeployMode::Links).unwrap();
    match outcome {
        DeployOutcome::Built {
            harvest: Some(report),
            ..
        } => assert!(
            report
                .whiteouts_added
                .iter()
                .any(|p| p.as_str() == "Data/a/one.txt"),
            "{report:?}"
        ),
        other => panic!("expected a rebuild with a harvest, got {other:?}"),
    }
    assert!(!game_dir(&f, &id).join("Data/a/one.txt").exists());
    assert!(game_dir(&f, &id).join("Data/a/two.ini").is_file());
}

/// Today's up-to-date check compares, for each recorded file that is not an expected rewrite, that
/// the farm has a file of the recorded size at that path. It does not look at which file it is (a
/// link or a copy) or at extra files, so those three edits keep the deployment "up to date". The
/// change that decides them is the harvest's comparison, which `undeploy` runs: it must still name
/// each of them.
#[test]
fn the_up_to_date_check_keeps_its_verdicts_and_harvest_still_sees_the_changes() {
    // 1. A linked base file replaced by a copy of the same bytes.
    {
        let f = fixture();
        let inst = new_instance(&f, "LinkToCopy");
        let id = inst.instance_id.clone();
        add_standard_content(&f, &id);
        deploy(&f.ctx, &id, &f.def, DeployMode::Links).unwrap();
        let file = game_dir(&f, &id).join("base_file.txt");
        let bytes = std::fs::read(&file).unwrap();
        std::fs::remove_file(&file).unwrap();
        std::fs::write(&file, bytes).unwrap();

        let outcome = deploy(&f.ctx, &id, &f.def, DeployMode::Links).unwrap();
        assert!(
            matches!(outcome, DeployOutcome::UpToDate { .. }),
            "{outcome:?}"
        );
        let report = undeploy(&f.ctx, &id).unwrap();
        assert!(
            report
                .copied_to_writable
                .iter()
                .any(|p| p.as_str() == "base_file.txt"),
            "harvest sees the replaced link: {report:?}"
        );
    }
    // 2. A copied config file edited in place, to the same length.
    {
        let f = fixture();
        let inst = new_instance(&f, "EditedCopy");
        let id = inst.instance_id.clone();
        add_standard_content(&f, &id);
        deploy(&f.ctx, &id, &f.def, DeployMode::Links).unwrap();
        let file = game_dir(&f, &id).join("Data/a/two.ini");
        let edited = b"[two]\nvalue=9\n";
        assert_eq!(std::fs::read(&file).unwrap().len(), edited.len());
        // On Linux a copy takes the time it was made, not its source's, so an edit in the same
        // millisecond would look unchanged by size and time; no person edits that fast.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&file, edited).unwrap();

        let outcome = deploy(&f.ctx, &id, &f.def, DeployMode::Links).unwrap();
        assert!(
            matches!(outcome, DeployOutcome::UpToDate { .. }),
            "{outcome:?}"
        );
        let report = undeploy(&f.ctx, &id).unwrap();
        assert!(
            report
                .copied_to_writable
                .iter()
                .any(|p| p.as_str() == "Data/a/two.ini"),
            "harvest sees the edited copy: {report:?}"
        );
    }
    // 3. An extra file in the farm.
    {
        let f = fixture();
        let inst = new_instance(&f, "Extra");
        let id = inst.instance_id.clone();
        add_standard_content(&f, &id);
        deploy(&f.ctx, &id, &f.def, DeployMode::Links).unwrap();
        std::fs::write(game_dir(&f, &id).join("Data/extra.txt"), b"extra").unwrap();

        let outcome = deploy(&f.ctx, &id, &f.def, DeployMode::Links).unwrap();
        assert!(
            matches!(outcome, DeployOutcome::UpToDate { .. }),
            "{outcome:?}"
        );
        let report = undeploy(&f.ctx, &id).unwrap();
        assert!(
            report
                .copied_to_writable
                .iter()
                .any(|p| p.as_str() == "Data/extra.txt"),
            "harvest sees the extra file: {report:?}"
        );
    }
}
