//! Framework files built for another game version are refused before launch (MASTER_SPEC §26.6):
//! the same check for a deployed launch, a base launch and `games instance check`.

use std::path::PathBuf;
use std::sync::Arc;

use agora_core::ctx::CoreContext;
use agora_core::game_base::BaseMode;
use agora_core::game_deploy::{add_content, deployment_dir, set_content_enabled};
use agora_core::game_discovery::{DiscoveredInstall, InstallCapabilities};
use agora_core::game_instance::{
    create, prepare_launch_with, runtime_findings, GameInstanceRecord, InstanceError,
    LaunchOptions, VfsFailure,
};
use agora_core::game_launch::{
    prepare_base_launch_with, LaunchError, LaunchedGame, Launcher, PreparedLaunch,
};
use agora_core::game_registry::{
    GameRegistry, IdentifiedInstall, PackageSource, RuntimeResolution,
};
use agora_game_api::{
    BaseReference, DeploymentStrategy, GameDefinition, GameId, GamePackage, GamePath, InstallId,
    InstallKind, LaunchRecipe, LaunchValue, PackageDefinition, RelPath, RuntimeFileProblem,
    RuntimeFileRule, RuntimeIdentity, StoreId, StoreIdentifier,
};
use tempfile::TempDir;

const VERSION: &str = "1.6.1170.0";

/// The SKSE rule Skyrim SE declares, so the tests exercise the same shape as the real game.
fn skse_rule() -> RuntimeFileRule {
    RuntimeFileRule {
        id: "skse".into(),
        name: "Skyrim Script Extender (SKSE)".into(),
        family: "skse64_*.dll".into(),
        expected: "skse64_{1}_{2}_{3}.dll".into(),
        applies_to: vec![],
        repair: "Install the SKSE build for Skyrim {version}".into(),
    }
}

fn make_test_definition() -> GameDefinition {
    GameDefinition {
        id: GameId::new("skyrim-se").unwrap(),
        name: "Skyrim Special Edition".into(),
        stores: vec![StoreIdentifier {
            store: StoreId::new("steam").unwrap(),
            product: "489830".into(),
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
        declared_writes: vec![],
        excluded_paths: vec![],
        plugin_list: None,
        launch_alternatives: Vec::new(),
        runtime_files: vec![skse_rule()],
        content_layout: None,
        copy_patterns: Vec::new(),
    }
}

struct TestPackage(PackageDefinition);

impl GamePackage for TestPackage {
    fn definition(&self) -> &PackageDefinition {
        &self.0
    }
}

fn create_test_context(tmp: &TempDir, def: &GameDefinition) -> CoreContext {
    let ctx = CoreContext::for_testing(tmp.path().join("app_data"));
    agora_core::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();
    let mut builder = GameRegistry::builder();
    let pkg_def = PackageDefinition {
        id: format!("test.{}", def.id),
        version: semver::Version::new(0, 1, 0),
        api_range: semver::VersionReq::parse(">=0.1, <0.2").unwrap(),
        parents: vec![],
        games: vec![def.clone()],
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
    ctx.with_games(Arc::new(builder.build()))
}

/// A Skyrim-like install at [`VERSION`] holding `extra_files` (relative paths) beside its game.
fn make_install(tmp: &TempDir, extra_files: &[&str]) -> IdentifiedInstall {
    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(install_dir.join("Data")).unwrap();
    std::fs::write(install_dir.join("Game.exe"), b"fake game binary").unwrap();
    std::fs::write(install_dir.join("Data").join("Skyrim.bsa"), b"BSA DATA").unwrap();
    for extra in extra_files {
        let path = install_dir.join(extra);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"fake framework file").unwrap();
    }

    let store = StoreId::new("steam").unwrap();
    let runtime = RuntimeIdentity {
        game: GameId::new("skyrim-se").unwrap(),
        store: store.clone(),
        version: VERSION.into(),
        build: None,
    };
    let volume =
        agora_core::game_discovery::volume::VolumeDetector::new().get_volume_info(&install_dir);
    IdentifiedInstall {
        game: runtime.game.clone(),
        install_id: InstallId::new("steam:489830").unwrap(),
        discovered: DiscoveredInstall {
            store,
            product: "489830".into(),
            name: "Skyrim Special Edition".into(),
            kind: InstallKind::BaseGame,
            parent_product: None,
            location: install_dir,
            store_version: Some(runtime.version.clone()),
            store_build: None,
            executables: vec!["Game.exe".into()],
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
    }
}

struct Setup {
    _tmp: TempDir,
    ctx: CoreContext,
    def: GameDefinition,
    instance: GameInstanceRecord,
}

/// An instance at [`VERSION`] over an install holding `extra_install_files`.
fn setup(extra_install_files: &[&str]) -> Setup {
    let tmp = TempDir::new().unwrap();
    let def = make_test_definition();
    let ctx = create_test_context(&tmp, &def);
    let install = make_install(&tmp, extra_install_files);
    let instance = create(
        &ctx,
        &install,
        &def,
        "Runtime Files",
        None,
        BaseMode::Linked,
        &|_| {},
    )
    .expect("create instance");
    Setup {
        _tmp: tmp,
        ctx,
        def,
        instance,
    }
}

/// A content item holding `files` at the root of the game, added to the instance. Returns its id.
fn add_root_content(ctx: &CoreContext, instance_id: &str, name: &str, files: &[&str]) -> String {
    let mod_dir = tempfile::tempdir().unwrap();
    for file in files {
        std::fs::write(mod_dir.path().join(file), b"fake mod file").unwrap();
    }
    let outcome = agora_core::content_store::add_folder(ctx, mod_dir.path(), Some(name)).unwrap();
    let item_id = outcome.item().item_id.clone();
    add_content(ctx, instance_id, &item_id, None, None).unwrap();
    item_id
}

/// The virtual file system is not installed: a launch falls back to linked files.
struct NoVfs;

impl Launcher for NoVfs {
    fn locate_vfs_dll(&self) -> Result<PathBuf, String> {
        Err("no agora_vfs.dll in this test".into())
    }
    fn launch(&self, _prepared: &PreparedLaunch) -> Result<LaunchedGame, LaunchError> {
        Err(LaunchError::ProcessCapture(
            "tests do not start games".into(),
        ))
    }
}

/// The virtual file system is "found" (its path is never opened: preparing a launch does not
/// start anything), so a launch runs under it.
struct FakeVfs;

impl Launcher for FakeVfs {
    fn locate_vfs_dll(&self) -> Result<PathBuf, String> {
        Ok(PathBuf::from("agora_vfs_test_stub.dll"))
    }
    fn launch(&self, _prepared: &PreparedLaunch) -> Result<LaunchedGame, LaunchError> {
        Err(LaunchError::ProcessCapture(
            "tests do not start games".into(),
        ))
    }
}

fn options(launch_anyway: bool) -> LaunchOptions {
    LaunchOptions {
        launch_anyway,
        on_vfs_failure: VfsFailure::FallBack,
        ..Default::default()
    }
}

fn prepare(
    s: &Setup,
    launch_anyway: bool,
    launcher: &dyn Launcher,
) -> Result<PreparedLaunch, InstanceError> {
    prepare_launch_with(
        &s.ctx,
        &s.instance.instance_id,
        &s.def,
        options(launch_anyway),
        &agora_core::game_discovery::discover_all,
        launcher,
    )
}

fn assert_skse_mismatch(err: InstanceError, found: &str) {
    match err {
        InstanceError::LaunchError(LaunchError::RuntimeMismatch { findings }) => {
            assert_eq!(findings.len(), 1, "{findings:?}");
            let finding = &findings[0];
            assert_eq!(finding.rule_id, "skse");
            assert_eq!(finding.expected, "skse64_1_6_1170.dll");
            assert_eq!(finding.found, vec![found.to_string()]);
            assert_eq!(finding.problem, RuntimeFileProblem::WrongVersion);
        }
        other => panic!("expected RuntimeMismatch, got {other:?}"),
    }
}

#[test]
fn deployed_content_with_skse_for_another_version_refuses_the_launch() {
    let s = setup(&[]);
    add_root_content(
        &s.ctx,
        &s.instance.instance_id,
        "SKSE 1.6.1179",
        &["skse64_1_6_1179.dll"],
    );

    let err = prepare(&s, false, &NoVfs).unwrap_err();
    assert_skse_mismatch(err, "skse64_1_6_1179.dll");
}

#[test]
fn launch_anyway_carries_the_findings_on_the_prepared_launch() {
    let s = setup(&[]);
    add_root_content(
        &s.ctx,
        &s.instance.instance_id,
        "SKSE 1.6.1179",
        &["skse64_1_6_1179.dll"],
    );

    let prepared = prepare(&s, true, &NoVfs).expect("launch anyway proceeds");
    assert_eq!(prepared.runtime_findings.len(), 1);
    assert_eq!(prepared.runtime_findings[0].rule_id, "skse");
}

#[test]
fn a_file_in_the_writable_layer_counts_under_the_virtual_file_system() {
    let s = setup(&[]);
    // The game wrote this under the virtual file system: the plan leaves the writable layer out
    // in that mode, so the check has to find it there too.
    let writable = s
        .ctx
        .paths
        .instance_dir(&s.instance.instance_id)
        .unwrap()
        .join("writable");
    std::fs::create_dir_all(&writable).unwrap();
    std::fs::write(writable.join("skse64_1_6_1179.dll"), b"written by the game").unwrap();

    let err = prepare(&s, false, &FakeVfs).unwrap_err();
    assert_skse_mismatch(err, "skse64_1_6_1179.dll");
}

#[test]
fn a_clean_instance_launches_with_no_findings() {
    let s = setup(&[]);
    let prepared = prepare(&s, false, &NoVfs).expect("no framework to refuse");
    assert!(prepared.runtime_findings.is_empty());
    assert!(runtime_findings(&s.ctx, &s.instance.instance_id, &s.def)
        .unwrap()
        .is_empty());
}

#[test]
fn check_reports_what_launch_refuses_and_deploys_nothing() {
    let s = setup(&[]);
    add_root_content(
        &s.ctx,
        &s.instance.instance_id,
        "SKSE 1.6.1179",
        &["skse64_1_6_1179.dll"],
    );
    assert!(deployment_dir(&s.ctx, &s.instance.instance_id)
        .unwrap()
        .is_none());

    let findings = runtime_findings(&s.ctx, &s.instance.instance_id, &s.def).unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].rule_id, "skse");
    assert_eq!(findings[0].found, vec!["skse64_1_6_1179.dll"]);

    // Checking built nothing: the game folder is still not there.
    assert!(deployment_dir(&s.ctx, &s.instance.instance_id)
        .unwrap()
        .is_none());
}

#[test]
fn a_base_launch_with_skse_for_another_version_is_refused_and_can_be_forced() {
    // The base holds the mismatched SKSE, and nothing is deployed on top of it.
    let s = setup(&["skse64_1_6_1179.dll"]);
    let base_id = match &s.instance.base {
        BaseReference::Pinned { id, .. } => id.clone(),
        other => panic!("expected a pinned base, got {other:?}"),
    };
    let manifest_text = std::fs::read_to_string(s.ctx.paths.base_manifest_path(&base_id)).unwrap();
    let manifest: agora_core::game_base::BaseManifest =
        serde_json::from_str(&manifest_text).unwrap();
    assert_eq!(manifest.runtime.version, VERSION);

    match prepare_base_launch_with(&manifest, &s.def, false, false) {
        Err(LaunchError::RuntimeMismatch { findings }) => {
            assert_eq!(findings.len(), 1);
            assert_eq!(findings[0].found, vec!["skse64_1_6_1179.dll"]);
        }
        other => panic!("expected RuntimeMismatch, got {other:?}"),
    }

    let forced =
        prepare_base_launch_with(&manifest, &s.def, true, false).expect("launch anyway proceeds");
    assert_eq!(forced.runtime_findings.len(), 1);
}

#[test]
fn a_rule_that_cannot_be_checked_launches_with_a_warning_not_a_refusal() {
    // A definition bug: component {5} does not exist in a four-part version. The instance is
    // otherwise clean for this rule's family, so without the bug it would launch.
    let mut s = setup(&[]);
    s.def.runtime_files = vec![RuntimeFileRule {
        id: "broken".into(),
        name: "Broken rule".into(),
        family: "skse64_*.dll".into(),
        expected: "skse64_{1}_{2}_{5}.dll".into(),
        applies_to: vec![],
        repair: "Fix the game definition".into(),
    }];
    add_root_content(
        &s.ctx,
        &s.instance.instance_id,
        "SKSE 1.6.1179",
        &["skse64_1_6_1179.dll"],
    );

    let prepared =
        prepare(&s, false, &NoVfs).expect("a rule that cannot be checked does not refuse");
    assert_eq!(prepared.runtime_findings.len(), 1, "{prepared:?}");
    let finding = &prepared.runtime_findings[0];
    assert_eq!(finding.rule_id, "broken");
    assert!(matches!(
        finding.problem,
        RuntimeFileProblem::CannotCheck { .. }
    ));
    assert!(!finding.refuses_launch());
}

#[test]
fn a_disabled_content_layer_with_another_versions_skse_gives_no_finding() {
    let s = setup(&[]);
    let item_id = add_root_content(
        &s.ctx,
        &s.instance.instance_id,
        "SKSE 1.6.1179",
        &["skse64_1_6_1179.dll"],
    );
    set_content_enabled(&s.ctx, &s.instance.instance_id, &item_id, false).unwrap();

    assert!(runtime_findings(&s.ctx, &s.instance.instance_id, &s.def)
        .unwrap()
        .is_empty());
    let prepared = prepare(&s, false, &NoVfs).expect("the disabled layer is not in the game");
    assert!(prepared.runtime_findings.is_empty());
}
