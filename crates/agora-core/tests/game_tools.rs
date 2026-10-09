//! Tools and generated output (MASTER_SPEC §26.9): runs that promote or discard, generations,
//! rollback, remove, diff, whiteouts, the order of generated layers, the input fingerprint, the
//! staleness warnings and the refusals.
//!
//! A tool is a copy of `cmd.exe` named `Game.exe`. The one thing these tests do not do is inject the
//! virtual file system: `StagingLauncher` starts the tool with its staging folder as its working
//! folder, which is where the VFS puts a tool's writes, and a tool that deletes a file writes the
//! whiteout marker the VFS would write. The VFS itself runs only in the `#[ignore]` real-injection
//! tests at the end, which need `AGORA_VFS_DLL`.

#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use agora_core::ctx::CoreContext;
use agora_core::event_sink::CancellationToken;
use agora_core::game_base::BaseMode;
use agora_core::game_deploy::{
    add_content, deploy, move_content, plan, set_content_enabled, DeployError, DeployMode,
    DeploymentPlan, FileSource, PlannedFile,
};
use agora_core::game_discovery::{DiscoveredInstall, DiscoveryReport, InstallCapabilities};
use agora_core::game_ini;
use agora_core::game_instance::{
    create, get_manifest, launch_with, GameInstanceRecord, InstanceError, LaunchOptions,
    LaunchedInstance, VfsFailure,
};
use agora_core::game_launch::{LaunchError, LaunchedGame, Launcher, PreparedLaunch};
use agora_core::game_load_order::{self, MoveTarget};
use agora_core::game_registry::{
    GameRegistry, IdentifiedInstall, PackageSource, RuntimeResolution,
};
use agora_core::game_tools::{self, OutputStatus, RunOutcome, ToolError};
use agora_core::game_user_files::{record_process, swap_in};
use agora_core::process_identity;
use agora_game_api::{
    DeploymentStrategy, GameDefinition, GameId, GamePackage, GamePath, InputFingerprint, InstallId,
    InstallKind, LaunchRecipe, LaunchValue, LayerSource, PackageDefinition, PluginListRule,
    RelPath, RuntimeIdentity, StoreId, StoreIdentifier, ToolDefinition, ToolId, UserDataLocation,
    UserFileMapping, UserFileStrategy,
};
use tempfile::TempDir;

const GAME: &str = "test-game";

struct TestPackage(PackageDefinition);

impl GamePackage for TestPackage {
    fn definition(&self) -> &PackageDefinition {
        &self.0
    }
}

/// The user-data folders are redirected by `AGORA_TEST_USER_DATA_ROOT`, a process-wide setting, so
/// the tests that touch them run one at a time.
static USER_DATA: Mutex<()> = Mutex::new(());

struct UserData {
    _lock: MutexGuard<'static, ()>,
    _dir: TempDir,
}

impl UserData {
    fn new() -> Self {
        let lock = USER_DATA.lock().unwrap_or_else(|e| e.into_inner());
        let dir = TempDir::new().unwrap();
        std::env::set_var("AGORA_TEST_USER_DATA_ROOT", dir.path());
        Self {
            _lock: lock,
            _dir: dir,
        }
    }
}

impl Drop for UserData {
    fn drop(&mut self) {
        std::env::remove_var("AGORA_TEST_USER_DATA_ROOT");
    }
}

/// Starts a tool or a launch without the VFS. A tool's working folder is its staging folder, and
/// `env` is added to what the process sees.
struct StagingLauncher {
    dll: Result<PathBuf, String>,
    /// Whether the working folder moves to the staging folder (a tool run) or stays where the
    /// recipe puts it (a game launch).
    stage: bool,
    env: Vec<(String, String)>,
}

impl StagingLauncher {
    fn new() -> Self {
        Self {
            dll: Ok(PathBuf::from(r"C:\agora\agora_vfs.dll")),
            stage: true,
            env: Vec::new(),
        }
    }

    /// For a game launch: the same launcher, without moving the working folder.
    fn plain() -> Self {
        Self {
            stage: false,
            ..Self::new()
        }
    }

    fn with_env(mut self, key: &str, value: &str) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    fn without_vfs(reason: &str) -> Self {
        Self {
            dll: Err(reason.into()),
            stage: true,
            env: Vec::new(),
        }
    }
}

impl Launcher for StagingLauncher {
    fn locate_vfs_dll(&self) -> Result<PathBuf, String> {
        self.dll.clone()
    }

    fn launch(&self, prepared: &PreparedLaunch) -> Result<LaunchedGame, LaunchError> {
        let mut plain = prepared.clone();
        if let (true, Some(vfs)) = (self.stage, &prepared.vfs) {
            plain.resolved.cwd = vfs.upper.clone();
        }
        plain.vfs = None;
        for (key, value) in &self.env {
            plain.resolved.env.insert(key.clone(), value.clone().into());
        }
        agora_core::game_launch::launch(&plain)
    }
}

fn literal_args(args: &[&str]) -> Vec<LaunchValue> {
    args.iter()
        .map(|a| LaunchValue::Literal {
            value: a.to_string(),
        })
        .collect()
}

/// A tool that runs `args`, from the game's folder (its staging folder, under the fake launcher).
fn tool(id: &str, name: &str, args: &[&str]) -> ToolDefinition {
    ToolDefinition {
        output_patterns: Vec::new(),
        id: ToolId::new(id).unwrap(),
        game: GameId::new(GAME).unwrap(),
        name: name.into(),
        launch: LaunchRecipe {
            executable: GamePath::Runtime {
                path: RelPath::new("Game.exe").unwrap(),
            },
            arguments: literal_args(args),
            environment: Default::default(),
            working_directory: GamePath::Runtime {
                path: RelPath::default(),
            },
        },
        relevant_settings: Vec::new(),
        after_tools: Vec::new(),
    }
}

fn plugin_rule() -> PluginListRule {
    PluginListRule {
        user_file: RelPath::new("user/Plugins.txt").unwrap(),
        plugin_folder: RelPath::new("Data").unwrap(),
        patterns: vec!["*.esm".into(), "*.esl".into(), "*.esp".into()],
        active_prefix: "*".into(),
        header: vec![
            "# This file is used by Skyrim to keep track of your downloaded content.".into(),
            "# Please do not modify this file.".into(),
        ],
        semantics: None,
        implicit: Vec::new(),
        implicit_list_file: None,
    }
}

/// The test game. With `user_files` it keeps a plugin list and an INI in the user-data folders
/// (a session can then run); with `plugins` it also has a plugin list.
fn definition(tools: &[ToolDefinition], user_files: bool, plugins: bool) -> GameDefinition {
    let mut mappings = Vec::new();
    if user_files {
        mappings.push(
            UserFileMapping::new(
                GamePath::UserData {
                    location: UserDataLocation::LocalAppData,
                    path: RelPath::new("Test Game/Plugins.txt").unwrap(),
                },
                RelPath::new("user/Plugins.txt").unwrap(),
                UserFileStrategy::JournaledSwap,
            )
            .with_stores(vec![StoreId::steam()]),
        );
        mappings.push(
            UserFileMapping::new(
                GamePath::UserData {
                    location: UserDataLocation::Documents,
                    path: RelPath::new("My Games/Test Game/Test.ini").unwrap(),
                },
                RelPath::new("user/Test.ini").unwrap(),
                UserFileStrategy::JournaledSwap,
            )
            .with_stores(vec![StoreId::steam()]),
        );
    }
    GameDefinition {
        mo2_game_name: None,
        id: GameId::new(GAME).unwrap(),
        name: "Test Game".into(),
        stores: vec![StoreIdentifier {
            store: StoreId::steam(),
            product: "12345".into(),
        }],
        version_sources: vec![],
        deployment: DeploymentStrategy::VirtualFileSystem,
        content_rules: vec![],
        native_code_patterns: vec![],
        framework_ids: vec![],
        tool_ids: tools.iter().map(|t| t.id.clone()).collect(),
        launch: Some(LaunchRecipe {
            executable: GamePath::Runtime {
                path: RelPath::new("Game.exe").unwrap(),
            },
            arguments: literal_args(&["/c", "exit", "0"]),
            environment: Default::default(),
            working_directory: GamePath::Runtime {
                path: RelPath::default(),
            },
        }),
        log_paths: vec![],
        crash_paths: vec![],
        user_files: mappings,
        save_paths: vec![],
        linked_archive_patterns: vec![],
        declared_writes: vec![],
        excluded_paths: vec![],
        plugin_list: plugins.then(plugin_rule),
        runtime_files: Vec::new(),
        save_location: Vec::new(),
        launch_alternatives: Vec::new(),
        content_layout: None,
        copy_patterns: Vec::new(),
    }
}

fn registry(def: &GameDefinition, tools: Vec<ToolDefinition>) -> Arc<GameRegistry> {
    let mut builder = GameRegistry::builder();
    let pkg = PackageDefinition {
        id: format!("test.{}", def.id),
        version: semver::Version::new(0, 1, 0),
        api_range: semver::VersionReq::parse(">=0.1, <0.2").unwrap(),
        parents: vec![],
        games: vec![def.clone()],
        frameworks: vec![],
        tools,
    };
    builder
        .add(
            PackageSource::Compiled {
                crate_name: "test".into(),
            },
            Arc::new(TestPackage(pkg)),
        )
        .expect("register test package");
    Arc::new(builder.build())
}

fn install_of(dir: &Path) -> IdentifiedInstall {
    let store_id = StoreId::steam();
    let runtime = RuntimeIdentity {
        game: GameId::new(GAME).unwrap(),
        store: store_id.clone(),
        version: "1.0.0".into(),
        build: None,
    };
    let volume = agora_core::game_discovery::volume::VolumeDetector::new().get_volume_info(dir);
    let discovered = DiscoveredInstall {
        store: store_id,
        product: "12345".into(),
        name: "Test Game".into(),
        kind: InstallKind::BaseGame,
        parent_product: None,
        location: dir.to_path_buf(),
        store_version: Some(runtime.version.clone()),
        store_build: None,
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

struct Fixture {
    _tmp: TempDir,
    ctx: CoreContext,
    def: GameDefinition,
    inst: GameInstanceRecord,
}

fn fixture(tools: Vec<ToolDefinition>) -> Fixture {
    fixture_with(tools, false, false)
}

fn fixture_with(tools: Vec<ToolDefinition>, user_files: bool, plugins: bool) -> Fixture {
    let tmp = TempDir::new().unwrap();
    let def = definition(&tools, user_files, plugins);
    let ctx = CoreContext::for_testing(tmp.path().join("app_data"));
    agora_core::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();
    let ctx = ctx.with_games(registry(&def, tools));
    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(install_dir.join("Data")).unwrap();
    std::fs::copy(r"C:\Windows\System32\cmd.exe", install_dir.join("Game.exe")).unwrap();
    std::fs::write(install_dir.join("Data").join("Skyrim.bsa"), b"BSA DATA").unwrap();
    std::fs::write(install_dir.join("base_file.txt"), b"initial base file").unwrap();
    let install = install_of(&install_dir);
    let inst = create(
        &ctx,
        &install,
        &def,
        "Tools",
        None,
        BaseMode::Linked,
        &|_| {},
    )
    .expect("create instance");
    Fixture {
        _tmp: tmp,
        ctx,
        def,
        inst,
    }
}

fn planned<'a>(plan: &'a DeploymentPlan, path: &str) -> Option<&'a PlannedFile> {
    plan.files.iter().find(|f| f.path.as_str() == path)
}

/// The names in a folder, sorted; empty when the folder does not exist.
fn names(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect(),
        Err(_) => Vec::new(),
    };
    out.sort();
    out
}

fn launch(
    ctx: &CoreContext,
    id: &str,
    def: &GameDefinition,
    launcher: &dyn Launcher,
) -> Result<LaunchedInstance, InstanceError> {
    launch_with(
        ctx,
        id,
        def,
        LaunchOptions {
            launch_anyway: false,
            plain: false,
            deployment: None,
            on_vfs_failure: VfsFailure::Ask,
        },
        &DiscoveryReport::default,
        launcher,
    )
}

impl Fixture {
    fn id(&self) -> &str {
        &self.inst.instance_id
    }

    fn instance_dir(&self) -> PathBuf {
        self.ctx.paths.instance_dir(self.id()).unwrap()
    }

    fn writable_dir(&self) -> PathBuf {
        self.instance_dir().join("writable")
    }

    fn generation_dir(&self, tool: &str, generation: &str) -> PathBuf {
        game_tools::generation_dir(&self.instance_dir(), tool, generation).unwrap()
    }

    fn tool_dir(&self, tool: &str) -> PathBuf {
        game_tools::tool_dir(&self.instance_dir(), tool)
    }

    fn run(&self, tool: &str, launcher: &dyn Launcher) -> Result<RunOutcome, ToolError> {
        game_tools::run(
            &self.ctx,
            self.id(),
            &self.def,
            &ToolId::new(tool).unwrap(),
            launcher,
            &CancellationToken::new(),
        )
    }

    /// A run that must promote.
    fn run_ok(&self, tool: &str, launcher: &dyn Launcher) -> RunOutcome {
        let outcome = self.run(tool, launcher).expect("run");
        assert!(outcome.promoted, "{outcome:?}");
        outcome
    }

    /// The tool's generated layer: its generation and inputs.
    fn generated(&self, tool: &str) -> Option<(String, InputFingerprint)> {
        get_manifest(&self.ctx, self.id())
            .unwrap()
            .layers
            .layers()
            .iter()
            .find_map(|layer| match &layer.source {
                LayerSource::Generated {
                    tool: t,
                    generation,
                    inputs,
                } if t.as_str() == tool => Some((generation.clone(), inputs.clone())),
                _ => None,
            })
    }

    /// The tools with a generated layer, in the order the stack has them.
    fn generated_order(&self) -> Vec<String> {
        get_manifest(&self.ctx, self.id())
            .unwrap()
            .layers
            .layers()
            .iter()
            .filter_map(|layer| match &layer.source {
                LayerSource::Generated { tool, .. } => Some(tool.to_string()),
                _ => None,
            })
            .collect()
    }

    fn plan(&self, mode: DeployMode) -> DeploymentPlan {
        plan(&self.ctx, self.id(), &self.def, mode).unwrap()
    }

    /// Add a content item (a mod) to the instance, and return its id.
    fn add_mod(&self, name: &str, files: &[(&str, &[u8])]) -> String {
        let mod_dir = TempDir::new().unwrap();
        for (rel, bytes) in files {
            let path = mod_dir.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        let outcome =
            agora_core::content_store::add_folder(&self.ctx, mod_dir.path(), Some(name)).unwrap();
        let item = outcome.item().item_id.clone();
        add_content(&self.ctx, self.id(), &item, None, None).unwrap();
        item
    }
}

/// A tool that writes `tag <AGORA_TEST_TAG>` into `Data\mod.txt`. The space before `>` matters: cmd
/// reads a digit joined to `>` as a handle number, so `echo 1>file` writes nothing. Under the fake
/// launcher the staging folder starts empty, so the tool makes `Data` first; under the VFS `Data` is
/// already there, so the real tests use [`tagged_in_place`].
fn tagged(id: &str) -> ToolDefinition {
    tool(
        id,
        "Nemesis",
        &[
            "/c",
            "mkdir",
            "Data",
            "&",
            "echo",
            "tag",
            "%AGORA_TEST_TAG%",
            ">",
            r"Data\mod.txt",
        ],
    )
}

fn tagged_in_place(id: &str) -> ToolDefinition {
    tool(
        id,
        "Nemesis",
        &[
            "/c",
            "echo",
            "tag",
            "%AGORA_TEST_TAG%",
            ">",
            r"Data\mod.txt",
        ],
    )
}

/// A tool that writes `Data\nemesis.txt`, and exits 3 when `AGORA_TEST_FAIL` is set.
fn writes_or_fails(id: &str) -> ToolDefinition {
    tool(
        id,
        "Nemesis",
        &[
            "/c",
            "mkdir",
            "Data",
            "&",
            "echo",
            "made>",
            r"Data\nemesis.txt",
            "&",
            "if",
            "defined",
            "AGORA_TEST_FAIL",
            "exit",
            "/b",
            "3",
        ],
    )
}

#[test]
fn a_successful_run_is_promoted_and_the_writable_layer_is_untouched() {
    let f = fixture(vec![writes_or_fails("nemesis")]);

    let out = f.run_ok("nemesis", &StagingLauncher::new());

    assert_eq!(out.current.as_deref(), Some("1"));
    assert_eq!(out.previous, None);
    assert_eq!(out.written, vec!["Data/nemesis.txt".to_string()]);
    assert_eq!(out.exit_code, Some(0));
    assert_eq!(
        std::fs::read_to_string(f.generation_dir("nemesis", "1").join("Data/nemesis.txt")).unwrap(),
        // cmd writes the space before `>` too.
        "made \r\n"
    );
    assert!(
        names(&f.writable_dir()).is_empty(),
        "the game's writable layer changed: {:?}",
        names(&f.writable_dir())
    );
    assert!(
        names(&f.tool_dir("nemesis"))
            .iter()
            .all(|n| !n.starts_with("staging-")),
        "a staging folder was left behind"
    );
    let (generation, inputs) = f.generated("nemesis").expect("a generated layer");
    assert_eq!(generation, "1");
    assert!(matches!(inputs, InputFingerprint::Known(_)));
}

#[test]
fn a_failing_run_is_discarded_and_the_previous_generation_stays_in_effect() {
    let f = fixture(vec![writes_or_fails("nemesis")]);
    f.run_ok("nemesis", &StagingLauncher::new());

    let out = f
        .run(
            "nemesis",
            &StagingLauncher::new().with_env("AGORA_TEST_FAIL", "1"),
        )
        .expect("a failed run is an outcome, not an error");

    assert!(!out.promoted, "{out:?}");
    assert_eq!(out.exit_code, Some(3));
    assert!(!out.cancelled);
    assert_eq!(out.current.as_deref(), Some("1"));
    assert_eq!(out.previous, None);
    let failed = out.failed_folder.expect("the discarded run is kept");
    assert!(failed.is_dir(), "{}", failed.display());
    assert!(failed.join("Data/nemesis.txt").is_file());
    assert_eq!(f.generated("nemesis").unwrap().0, "1");
    assert!(f.generation_dir("nemesis", "1").is_dir());
}

#[test]
fn a_second_run_keeps_one_a_third_deletes_it() {
    let f = fixture(vec![tagged("nemesis")]);

    f.run_ok(
        "nemesis",
        &StagingLauncher::new().with_env("AGORA_TEST_TAG", "1"),
    );
    let second = f.run_ok(
        "nemesis",
        &StagingLauncher::new().with_env("AGORA_TEST_TAG", "2"),
    );
    assert_eq!(second.current.as_deref(), Some("2"));
    assert_eq!(second.previous.as_deref(), Some("1"));
    assert!(f.generation_dir("nemesis", "1").is_dir());
    assert!(f.generation_dir("nemesis", "2").is_dir());

    let third = f.run_ok(
        "nemesis",
        &StagingLauncher::new().with_env("AGORA_TEST_TAG", "3"),
    );
    assert_eq!(third.current.as_deref(), Some("3"));
    assert_eq!(third.previous.as_deref(), Some("2"));
    assert!(
        !f.generation_dir("nemesis", "1").exists(),
        "generation 1 is kept"
    );
    assert!(f.generation_dir("nemesis", "2").is_dir());
    assert!(f.generation_dir("nemesis", "3").is_dir());
    assert_eq!(f.generated("nemesis").unwrap().0, "3");
}

#[test]
fn rollback_swaps_the_current_and_previous_generations() {
    let f = fixture(vec![tagged("nemesis")]);
    f.run_ok(
        "nemesis",
        &StagingLauncher::new().with_env("AGORA_TEST_TAG", "1"),
    );
    f.run_ok(
        "nemesis",
        &StagingLauncher::new().with_env("AGORA_TEST_TAG", "2"),
    );

    let state =
        game_tools::rollback(&f.ctx, f.id(), &f.def, &ToolId::new("nemesis").unwrap()).unwrap();
    assert_eq!(state.current.as_deref(), Some("1"));
    assert_eq!(state.previous.as_deref(), Some("2"));
    assert_eq!(f.generated("nemesis").unwrap().0, "1");
    assert!(f.generation_dir("nemesis", "1").is_dir());
    assert!(
        f.generation_dir("nemesis", "2").is_dir(),
        "nothing is deleted"
    );

    // Rolling back again swaps back.
    let again =
        game_tools::rollback(&f.ctx, f.id(), &f.def, &ToolId::new("nemesis").unwrap()).unwrap();
    assert_eq!(again.current.as_deref(), Some("2"));
    assert_eq!(again.previous.as_deref(), Some("1"));
}

#[test]
fn rollback_needs_a_previous_generation() {
    let f = fixture(vec![tagged("nemesis")]);
    f.run_ok(
        "nemesis",
        &StagingLauncher::new().with_env("AGORA_TEST_TAG", "1"),
    );

    let err =
        game_tools::rollback(&f.ctx, f.id(), &f.def, &ToolId::new("nemesis").unwrap()).unwrap_err();
    assert!(matches!(err, ToolError::NoPrevious(_)), "{err:?}");
}

#[test]
fn remove_drops_the_layer_and_every_generation_and_leaves_the_mods() {
    let f = fixture(vec![tagged("nemesis")]);
    f.add_mod("Mod", &[("Data/other.txt", b"mod")]);
    f.run_ok(
        "nemesis",
        &StagingLauncher::new().with_env("AGORA_TEST_TAG", "1"),
    );
    f.run_ok(
        "nemesis",
        &StagingLauncher::new().with_env("AGORA_TEST_TAG", "2"),
    );

    let mut removed =
        game_tools::remove(&f.ctx, f.id(), &f.def, &ToolId::new("nemesis").unwrap()).unwrap();
    removed.sort();
    assert_eq!(removed, vec!["1".to_string(), "2".to_string()]);
    assert!(f.generated("nemesis").is_none());
    assert!(!f.tool_dir("nemesis").exists());
    assert!(planned(&f.plan(DeployMode::Virtual), "Data/other.txt").is_some());

    let err =
        game_tools::remove(&f.ctx, f.id(), &f.def, &ToolId::new("nemesis").unwrap()).unwrap_err();
    assert!(matches!(err, ToolError::NoOutput(_)), "{err:?}");
}

#[test]
fn diff_lists_files_added_changed_and_removed_between_the_generations() {
    // cmd's `if` takes the rest of the line with it, so the two branches are one `if ... else`.
    let f = fixture(vec![tool(
        "nemesis",
        "Nemesis",
        &[
            "/c",
            "mkdir",
            "Data",
            "&",
            "echo",
            "tag",
            "%AGORA_TEST_TAG%",
            ">",
            r"Data\a.txt",
            "&",
            "if",
            "%AGORA_TEST_TAG%==1",
            "(",
            "echo",
            "x>",
            r"Data\gone.txt",
            ")",
            "else",
            "(",
            "echo",
            "y>",
            r"Data\new.txt",
            ")",
        ],
    )]);
    let tool_id = ToolId::new("nemesis").unwrap();
    f.run_ok(
        "nemesis",
        &StagingLauncher::new().with_env("AGORA_TEST_TAG", "1"),
    );
    let err = game_tools::diff(&f.ctx, f.id(), &tool_id).unwrap_err();
    assert!(matches!(err, ToolError::NoPrevious(_)), "{err:?}");

    f.run_ok(
        "nemesis",
        &StagingLauncher::new().with_env("AGORA_TEST_TAG", "2"),
    );
    let report = game_tools::diff(&f.ctx, f.id(), &tool_id).unwrap();
    assert_eq!(report.previous, "1");
    assert_eq!(report.current, "2");
    assert_eq!(report.added, vec!["Data/new.txt".to_string()]);
    assert_eq!(report.changed, vec!["Data/a.txt".to_string()]);
    assert_eq!(report.removed, vec!["Data/gone.txt".to_string()]);
}

#[test]
fn a_tool_that_deletes_a_mods_file_hides_it_in_the_plan() {
    let deleter = tool(
        "deleter",
        "Deleter",
        &[
            "/c",
            "mkdir",
            r".agvfs-wh\Data",
            "&",
            "type",
            "nul",
            ">",
            r".agvfs-wh\Data\mod.txt.wh",
        ],
    );
    let f = fixture(vec![deleter]);
    f.add_mod("Mod", &[("Data/mod.txt", b"original")]);
    assert!(planned(&f.plan(DeployMode::Virtual), "Data/mod.txt").is_some());

    let out = f.run_ok("deleter", &StagingLauncher::new());
    assert_eq!(out.deleted, vec!["Data/mod.txt".to_string()]);

    for mode in [DeployMode::Virtual, DeployMode::Links, DeployMode::Copies] {
        let p = f.plan(mode);
        assert!(
            planned(&p, "Data/mod.txt").is_none(),
            "{mode}: the whiteout did not hide the mod's file"
        );
    }
}

#[test]
fn a_tool_that_runs_after_another_sits_above_it() {
    let mut alpha = tool(
        "alpha",
        "Alpha",
        &[
            "/c",
            "mkdir",
            "Data",
            "&",
            "echo",
            "alpha>",
            r"Data\shared.txt",
        ],
    );
    alpha.after_tools = vec![ToolId::new("beta").unwrap()];
    let beta = tool(
        "beta",
        "Beta",
        &[
            "/c",
            "mkdir",
            "Data",
            "&",
            "echo",
            "beta>",
            r"Data\shared.txt",
        ],
    );
    let f = fixture(vec![alpha, beta]);

    f.run_ok("alpha", &StagingLauncher::new());
    f.run_ok("beta", &StagingLauncher::new());

    assert_eq!(
        f.generated_order(),
        vec!["beta".to_string(), "alpha".to_string()]
    );
    let p = f.plan(DeployMode::Virtual);
    let shared = planned(&p, "Data/shared.txt").expect("the shared file is planned");
    assert!(
        matches!(&shared.source, FileSource::Generated { tool, .. } if tool == "alpha"),
        "alpha runs after beta, so its file wins: {:?}",
        shared.source
    );
}

#[test]
fn the_fingerprint_changes_with_the_enabled_layers_and_their_order_and_is_stable_otherwise() {
    let f = fixture(vec![writes_or_fails("nemesis")]);
    let a = f.add_mod("A", &[("Data/a.txt", b"a")]);
    let b = f.add_mod("B", &[("Data/b.txt", b"b")]);
    let tool_def = f
        .ctx
        .games
        .tool(&f.def.id, &ToolId::new("nemesis").unwrap())
        .unwrap()
        .clone();
    let fingerprint = || game_tools::input_fingerprint(&f.ctx, f.id(), &f.def, &tool_def).unwrap();

    let first = fingerprint();
    assert_eq!(
        first,
        fingerprint(),
        "the same inputs give the same fingerprint"
    );

    set_content_enabled(&f.ctx, f.id(), &a, false).unwrap();
    let without_a = fingerprint();
    assert_ne!(first, without_a, "disabling a layer changes it");
    set_content_enabled(&f.ctx, f.id(), &a, true).unwrap();
    assert_eq!(first, fingerprint(), "enabling it again restores it");

    move_content(&f.ctx, f.id(), &b, 0).unwrap();
    assert_ne!(first, fingerprint(), "the stack's order changes it");
}

#[test]
fn the_fingerprint_changes_with_a_relevant_ini_value() {
    let _user_data = UserData::new();
    let mut settings_tool = writes_or_fails("nemesis");
    settings_tool.relevant_settings = vec!["user/Test.ini:General:Gamma".into()];
    let f = fixture_with(vec![settings_tool], true, false);
    let tool_def = f
        .ctx
        .games
        .tool(&f.def.id, &ToolId::new("nemesis").unwrap())
        .unwrap()
        .clone();
    let fingerprint = || game_tools::input_fingerprint(&f.ctx, f.id(), &f.def, &tool_def).unwrap();

    let first = fingerprint();
    game_ini::set_value(
        &f.ctx,
        f.id(),
        &f.def,
        "user/Test.ini",
        "General",
        "Gamma",
        "2",
    )
    .unwrap();
    let changed = fingerprint();
    assert_ne!(first, changed, "a changed relevant setting changes it");
    game_ini::set_value(
        &f.ctx,
        f.id(),
        &f.def,
        "user/Test.ini",
        "General",
        "Gamma",
        "2",
    )
    .unwrap();
    assert_eq!(
        changed,
        fingerprint(),
        "writing the same value again changes nothing"
    );
}

#[test]
fn the_fingerprint_changes_with_the_plugin_load_order() {
    let _user_data = UserData::new();
    let f = fixture_with(vec![writes_or_fails("nemesis")], true, true);
    f.add_mod("Plugins", &[("Data/A.esp", b"A"), ("Data/B.esp", b"B")]);
    deploy(&f.ctx, f.id(), &f.def, DeployMode::Virtual).unwrap();
    let tool_def = f
        .ctx
        .games
        .tool(&f.def.id, &ToolId::new("nemesis").unwrap())
        .unwrap()
        .clone();
    let fingerprint = || game_tools::input_fingerprint(&f.ctx, f.id(), &f.def, &tool_def).unwrap();
    let order = || -> Vec<String> {
        game_load_order::order(&f.ctx, f.id(), &f.def)
            .unwrap()
            .entries
            .iter()
            .filter(|e| e.active)
            .map(|e| e.name.clone())
            .collect()
    };

    let first = fingerprint();
    let before = order();
    game_load_order::move_plugin(
        &f.ctx,
        f.id(),
        &f.def,
        &before[0],
        &MoveTarget::After(before[1].clone()),
    )
    .unwrap();
    assert_ne!(before, order(), "the move changed the order");
    assert_ne!(first, fingerprint(), "a changed plugin order changes it");
}

#[test]
fn a_stale_output_and_an_unknown_one_warn_and_never_refuse() {
    let f = fixture(vec![writes_or_fails("nemesis")]);
    let a = f.add_mod("A", &[("Data/a.txt", b"a")]);
    f.run_ok("nemesis", &StagingLauncher::new());

    // Current: no finding.
    assert_eq!(
        game_tools::list(&f.ctx, f.id(), &f.def).unwrap()[0].status,
        Some(OutputStatus::Current)
    );
    assert!(game_tools::output_findings(&f.ctx, f.id(), &f.def)
        .unwrap()
        .is_empty());

    // Stale once the enabled content changes.
    set_content_enabled(&f.ctx, f.id(), &a, false).unwrap();
    assert_eq!(
        game_tools::list(&f.ctx, f.id(), &f.def).unwrap()[0].status,
        Some(OutputStatus::Stale)
    );
    let findings = game_tools::output_findings(&f.ctx, f.id(), &f.def).unwrap();
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].status, OutputStatus::Stale);
    assert_eq!(
        findings[0].message,
        "Nemesis output is out of date: rebuild with `agora games instance tools run ".to_string()
            + f.id()
            + " nemesis`"
    );
    let launched = launch(&f.ctx, f.id(), &f.def, &StagingLauncher::plain())
        .expect("a stale output does not refuse a launch");
    assert_eq!(launched.prepared.generated_findings.len(), 1);
    let mut child = launched.launched;
    child.child.wait().unwrap();

    // Unknown: the manifest says nobody recorded the inputs (an imported output).
    set_content_enabled(&f.ctx, f.id(), &a, true).unwrap();
    let path = f.instance_dir().join("instance_manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    for layer in manifest["layers"].as_array_mut().unwrap() {
        if layer["source"]["kind"] == "generated" {
            layer["source"]["inputs"] = serde_json::json!({ "kind": "unknown" });
        }
    }
    std::fs::write(&path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();

    assert_eq!(
        game_tools::list(&f.ctx, f.id(), &f.def).unwrap()[0].status,
        Some(OutputStatus::Unknown)
    );
    let findings = game_tools::output_findings(&f.ctx, f.id(), &f.def).unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].status, OutputStatus::Unknown);
    assert!(
        findings[0].message.contains("rebuild with"),
        "{}",
        findings[0].message
    );
    let launched = launch(&f.ctx, f.id(), &f.def, &StagingLauncher::plain())
        .expect("an unknown output does not refuse a launch");
    assert_eq!(launched.prepared.generated_findings.len(), 1);
    let mut child = launched.launched;
    child.child.wait().unwrap();

    // Neither warning changed what the layer points at.
    assert_eq!(f.generated("nemesis").unwrap().0, "1");
}

#[test]
fn a_run_while_a_session_is_active_is_refused_and_leaves_nothing_behind() {
    let _user_data = UserData::new();
    let f = fixture_with(vec![writes_or_fails("nemesis")], true, false);
    let running_from = f._tmp.path().join("running_from");
    std::fs::create_dir_all(&running_from).unwrap();
    swap_in(&f.ctx, f.id(), &f.def, &StoreId::steam(), &running_from).unwrap();
    let live = process_identity::capture(std::process::id()).unwrap();
    record_process(&f.ctx, &f.def.id, &StoreId::steam(), live).unwrap();

    let err = f.run("nemesis", &StagingLauncher::new()).unwrap_err();
    assert!(matches!(err, ToolError::SessionRunning(_)), "{err:?}");
    assert!(f.generated("nemesis").is_none());
    assert!(!f.tool_dir("nemesis").exists());
}

#[test]
fn a_generated_layer_whose_folder_is_missing_is_an_error_not_an_empty_layer() {
    let f = fixture(vec![writes_or_fails("nemesis")]);
    f.run_ok("nemesis", &StagingLauncher::new());
    std::fs::remove_dir_all(f.generation_dir("nemesis", "1")).unwrap();

    let err = plan(&f.ctx, f.id(), &f.def, DeployMode::Virtual).unwrap_err();
    match err {
        DeployError::GeneratedLayerMissing {
            tool, generation, ..
        } => {
            assert_eq!(tool, "nemesis");
            assert_eq!(generation, "1");
        }
        other => panic!("expected GeneratedLayerMissing, got {other:?}"),
    }
    assert!(launch(&f.ctx, f.id(), &f.def, &StagingLauncher::plain()).is_err());
}

#[test]
fn a_run_is_refused_before_anything_is_deployed_when_the_vfs_cannot_start() {
    let f = fixture(vec![writes_or_fails("nemesis")]);

    let err = f
        .run(
            "nemesis",
            &StagingLauncher::without_vfs("agora_vfs.dll was not found"),
        )
        .unwrap_err();
    match err {
        ToolError::VfsUnavailable { reason, next } => {
            assert!(reason.contains("agora_vfs.dll was not found"), "{reason}");
            assert!(next.is_none());
        }
        other => panic!("expected VfsUnavailable, got {other:?}"),
    }
    assert!(f.generated("nemesis").is_none());
    assert!(!f.tool_dir("nemesis").exists());
}

#[test]
fn only_the_last_three_failed_runs_are_kept() {
    let f = fixture(vec![writes_or_fails("nemesis")]);
    for _ in 0..4 {
        let out = f
            .run(
                "nemesis",
                &StagingLauncher::new().with_env("AGORA_TEST_FAIL", "1"),
            )
            .unwrap();
        assert!(!out.promoted);
    }
    let failed = names(&f.tool_dir("nemesis"))
        .into_iter()
        .filter(|n| n.starts_with("failed-"))
        .count();
    assert_eq!(failed, game_tools::FAILED_RUNS_KEPT);
}

#[test]
fn an_undeclared_tool_is_refused() {
    let f = fixture(vec![writes_or_fails("nemesis")]);
    let err = f.run("pandora", &StagingLauncher::new()).unwrap_err();
    assert!(matches!(err, ToolError::UnknownTool { .. }), "{err:?}");
}

// ---------------------------------------------------------------------------------------------
// Real injection: the same tools under agora_vfs.dll. Run with
//   cargo build -p agora-vfs
//   AGORA_VFS_DLL=<worktree>\target\debug\agora_vfs.dll cargo test -p agora-core --test game_tools -- --ignored
// ---------------------------------------------------------------------------------------------

/// Injects the DLL, and sets `env` on the process it starts.
struct RealLauncher {
    dll: PathBuf,
    env: Vec<(String, String)>,
}

impl RealLauncher {
    fn new(env: &[(&str, &str)]) -> Self {
        Self {
            dll: built_dll(),
            env: env
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }
}

impl Launcher for RealLauncher {
    fn locate_vfs_dll(&self) -> Result<PathBuf, String> {
        Ok(self.dll.clone())
    }

    fn launch(&self, prepared: &PreparedLaunch) -> Result<LaunchedGame, LaunchError> {
        let mut with_env = prepared.clone();
        for (key, value) in &self.env {
            with_env
                .resolved
                .env
                .insert(key.clone(), value.clone().into());
        }
        agora_core::game_launch::launch(&with_env)
    }
}

fn built_dll() -> PathBuf {
    let dll = std::env::var_os("AGORA_VFS_DLL")
        .map(PathBuf::from)
        .expect("set AGORA_VFS_DLL to target/debug/agora_vfs.dll (cargo build -p agora-vfs)");
    assert!(dll.is_file(), "{} does not exist", dll.display());
    dll
}

#[test]
#[ignore = "injects agora_vfs.dll: cargo build -p agora-vfs, then set AGORA_VFS_DLL"]
fn real_injection_a_tool_run_writes_into_staging_and_is_promoted() {
    let f = fixture(vec![writes_or_fails("nemesis")]);
    let launcher = RealLauncher::new(&[]);

    let out = f.run("nemesis", &launcher).expect("run under the VFS");
    println!("run outcome: {out:?}");
    assert!(out.promoted, "{out:?}");
    assert_eq!(out.written, vec!["Data/nemesis.txt".to_string()]);
    assert_eq!(
        std::fs::read_to_string(f.generation_dir("nemesis", "1").join("Data/nemesis.txt")).unwrap(),
        "made \r\n"
    );
    assert!(
        names(&f.writable_dir()).is_empty(),
        "the game's writable layer changed: {:?}",
        names(&f.writable_dir())
    );
    assert!(names(&f.tool_dir("nemesis"))
        .iter()
        .all(|n| !n.starts_with("staging-")));
}

#[test]
#[ignore = "injects agora_vfs.dll: cargo build -p agora-vfs, then set AGORA_VFS_DLL"]
fn real_injection_rollback_changes_the_bytes_the_game_reads() {
    let f = fixture(vec![tagged_in_place("nemesis")]);
    // The game reads Data\mod.txt through the stack and writes what it read into Data\seen.txt,
    // which lands in the writable layer, so the test can read what the game saw.
    let mut read_def = f.def.clone();
    read_def.launch.as_mut().unwrap().arguments =
        literal_args(&["/c", "type", r"Data\mod.txt", ">", r"Data\seen.txt"]);
    let seen = f.writable_dir().join("Data").join("seen.txt");
    // Read the game's copy of the file, after a launch that reads it through the stack.
    let read_back = || std::fs::read_to_string(&seen).unwrap_or_default();

    f.run_ok("nemesis", &RealLauncher::new(&[("AGORA_TEST_TAG", "1")]));
    f.run_ok("nemesis", &RealLauncher::new(&[("AGORA_TEST_TAG", "2")]));
    let launched = launch(&f.ctx, f.id(), &read_def, &RealLauncher::new(&[])).expect("launch");
    let mut child = launched.launched;
    assert!(child.child.wait().unwrap().success());
    assert_eq!(
        read_back().trim(),
        "tag 2",
        "generation 2 is what the game reads"
    );

    game_tools::rollback(&f.ctx, f.id(), &f.def, &ToolId::new("nemesis").unwrap()).unwrap();
    let launched = launch(&f.ctx, f.id(), &read_def, &RealLauncher::new(&[])).expect("launch");
    let mut child = launched.launched;
    assert!(child.child.wait().unwrap().success());
    assert_eq!(
        read_back().trim(),
        "tag 1",
        "after rollback the game reads generation 1"
    );
}

/// What a tool sees of a file the game deleted: the game's whiteout lives in its writable layer,
/// which is a lower layer of the tool's run. Checked by the tool's own `if exist`.
#[test]
#[ignore = "injects agora_vfs.dll: cargo build -p agora-vfs, then set AGORA_VFS_DLL"]
fn real_injection_a_game_deletion_is_seen_by_a_tool_run() {
    let probe = tool(
        "probe",
        "Probe",
        &[
            "/c",
            "if",
            "exist",
            r"Data\mod.txt",
            "(",
            "echo",
            "present",
            ">",
            r"Data\state.txt",
            ")",
            "else",
            "(",
            "echo",
            "gone",
            ">",
            r"Data\state.txt",
            ")",
        ],
    );
    let f = fixture(vec![probe]);
    f.add_mod("Mod", &[("Data/mod.txt", b"original")]);
    let mut delete_def = f.def.clone();
    delete_def.launch.as_mut().unwrap().arguments = literal_args(&["/c", "del", r"Data\mod.txt"]);
    let launched = launch(&f.ctx, f.id(), &delete_def, &RealLauncher::new(&[])).expect("launch");
    let mut child = launched.launched;
    assert!(child.child.wait().unwrap().success());
    assert!(
        f.writable_dir().join(".agvfs-wh/Data/mod.txt.wh").is_file(),
        "the game's deletion left no whiteout in its writable layer"
    );

    let out = f.run("probe", &RealLauncher::new(&[])).expect("probe run");
    println!("probe outcome: {out:?}");
    let state = std::fs::read_to_string(f.generation_dir("probe", "1").join("Data/state.txt"))
        .unwrap_or_default();
    assert_eq!(
        state.trim(),
        "gone",
        "the tool still sees a file the game deleted"
    );
}

#[test]
#[ignore = "injects agora_vfs.dll: cargo build -p agora-vfs, then set AGORA_VFS_DLL"]
fn real_injection_a_tool_that_deletes_a_mods_file_hides_it() {
    let deleter = tool("deleter", "Deleter", &["/c", "del", r"Data\mod.txt"]);
    let f = fixture(vec![deleter]);
    let item = f.add_mod("Mod", &[("Data/mod.txt", b"original")]);
    let sha = agora_core::content_store::get_item(&f.ctx, &item)
        .unwrap()
        .files[0]
        .sha256
        .clone();

    let out = f
        .run("deleter", &RealLauncher::new(&[]))
        .expect("run under the VFS");
    println!("run outcome: {out:?}");
    assert!(out.promoted, "{out:?}");
    assert_eq!(out.deleted, vec!["Data/mod.txt".to_string()]);
    for mode in [DeployMode::Virtual, DeployMode::Links, DeployMode::Copies] {
        assert!(planned(&f.plan(mode), "Data/mod.txt").is_none(), "{mode}");
    }
    assert_eq!(
        std::fs::read(f.ctx.paths.content_object_path(&sha)).unwrap(),
        b"original",
        "the mod's stored file changed"
    );
}
