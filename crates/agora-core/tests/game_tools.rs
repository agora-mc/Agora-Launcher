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

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use agora_core::ctx::CoreContext;
use agora_core::event_sink::CancellationToken;
use agora_core::game_base::BaseMode;
use agora_core::game_deploy::{
    add_content, deploy, move_content, plan, set_content_enabled, set_deployment, DeployError,
    DeployMode, DeploymentPlan, FileSource, PlannedFile,
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
use agora_core::game_tools::{
    self, CaptureMethod, CaptureMode, OutputStatus, RunOutcome, ToolError, ToolOffer,
};
use agora_core::game_user_files::{record_process, swap_in};
use agora_core::process_identity;
use agora_game_api::{
    BaseReference, DeploymentStrategy, FailureMarker, GameDefinition, GameId, GamePackage,
    GamePath, InputFingerprint, InstallId, InstallKind, LaunchRecipe, LaunchValue, LayerSource,
    PackageDefinition, PluginListRule, RelPath, RuntimeIdentity, StoreId, StoreIdentifier,
    ToolDefinition, ToolId, UserDataLocation, UserFileMapping, UserFileStrategy,
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
        uses_install_path: false,
        required_outputs: Vec::new(),
        failure_markers: Vec::new(),
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
    fixture_with_exe(tools, user_files, plugins, r"C:\Windows\System32\cmd.exe")
}

/// A fixture whose `Game.exe` is a copy of `exe`: a 64-bit `cmd.exe` by default, or the 32-bit one.
fn fixture_with_exe(
    tools: Vec<ToolDefinition>,
    user_files: bool,
    plugins: bool,
    exe: &str,
) -> Fixture {
    let tmp = TempDir::new().unwrap();
    let def = definition(&tools, user_files, plugins);
    let ctx = CoreContext::for_testing(tmp.path().join("app_data"));
    agora_core::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();
    let ctx = ctx.with_games(registry(&def, tools));
    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(install_dir.join("Data")).unwrap();
    std::fs::copy(exe, install_dir.join("Game.exe")).unwrap();
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

    /// A run with the default capture, `auto`.
    fn run(&self, tool: &str, launcher: &dyn Launcher) -> Result<RunOutcome, ToolError> {
        self.run_as(tool, launcher, CaptureMode::Auto)
    }

    fn run_as(
        &self,
        tool: &str,
        launcher: &dyn Launcher,
        capture: CaptureMode,
    ) -> Result<RunOutcome, ToolError> {
        game_tools::run(
            &self.ctx,
            self.id(),
            &self.def,
            &ToolId::new(tool).unwrap(),
            capture,
            launcher,
            &CancellationToken::new(),
        )
    }

    /// The game folder of this instance's deployment, where a link-captured run's writes land.
    fn deployed_game_dir(&self) -> PathBuf {
        let manifest = get_manifest(&self.ctx, self.id()).unwrap();
        let BaseReference::Pinned { id: base_id, .. } = manifest.base else {
            panic!("the fixture instance is pinned");
        };
        let base: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(self.ctx.paths.base_manifest_path(&base_id)).unwrap(),
        )
        .unwrap();
        let location = PathBuf::from(base["location"].as_str().unwrap());
        location
            .parent()
            .unwrap()
            .join("deployments")
            .join(self.id())
            .join("game")
    }

    /// The bytes of a content item's file, as the content store holds them.
    fn content_bytes(&self, item: &str, path: &str) -> Vec<u8> {
        let item = agora_core::content_store::get_item(&self.ctx, item).unwrap();
        let file = item
            .files
            .iter()
            .find(|f| f.path.as_str() == path)
            .unwrap_or_else(|| panic!("no file {path} in the item"));
        std::fs::read(self.ctx.paths.content_object_path(&file.sha256)).unwrap()
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
        .run_as(
            "nemesis",
            &StagingLauncher::without_vfs("agora_vfs.dll was not found"),
            CaptureMode::Vfs,
        )
        .unwrap_err();
    match err {
        ToolError::VfsUnavailable { reason, next, .. } => {
            assert!(reason.contains("agora_vfs.dll was not found"), "{reason}");
            // The tool uses its own folder, so the next tier is link capture.
            assert_eq!(next, Some(CaptureMode::Links));
        }
        other => panic!("expected VfsUnavailable, got {other:?}"),
    }
    assert!(f.generated("nemesis").is_none());
    // The refusal counts as a failure, so the tool folder holds the count and nothing else.
    assert_eq!(
        names(&f.tool_dir("nemesis")),
        vec!["failures.json".to_string()]
    );
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
// Link capture (MASTER_SPEC §26.9): a run whose writes come from the link farm, compared with the
// deployment record. These run without the VFS: `StagingLauncher` starts the tool in the game folder.
// ---------------------------------------------------------------------------------------------

/// The `whiteouts` of a tool's generated layer, as the layer stores them.
fn whiteouts_of(f: &Fixture, tool: &str) -> Vec<String> {
    get_manifest(&f.ctx, f.id())
        .unwrap()
        .layers
        .layers()
        .iter()
        .find_map(|layer| match &layer.source {
            LayerSource::Generated { tool: t, .. } if t.as_str() == tool => Some(
                layer
                    .whiteouts
                    .iter()
                    .map(|w| w.as_str().to_string())
                    .collect(),
            ),
            _ => None,
        })
        .unwrap_or_default()
}

/// Sets an environment variable for one test and removes it after.
struct EnvVar(&'static str);

impl EnvVar {
    fn set(key: &'static str, value: &str) -> Self {
        std::env::set_var(key, value);
        Self(key)
    }
}

impl Drop for EnvVar {
    fn drop(&mut self) {
        std::env::remove_var(self.0);
    }
}

/// The 32-bit DLL's path is read at every launch of a 32-bit program. The two tests that start one hold
/// this lock, and each puts back the value it found, so one cannot see the other's path.
static X86_DLL_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Sets `AGORA_VFS_DLL_X86` for one test and puts back what was there before.
struct X86Dll(Option<std::ffi::OsString>);

impl X86Dll {
    fn set(value: &str) -> Self {
        let before = std::env::var_os("AGORA_VFS_DLL_X86");
        std::env::set_var("AGORA_VFS_DLL_X86", value);
        Self(before)
    }
}

impl Drop for X86Dll {
    fn drop(&mut self) {
        match self.0.take() {
            Some(value) => std::env::set_var("AGORA_VFS_DLL_X86", value),
            None => std::env::remove_var("AGORA_VFS_DLL_X86"),
        }
    }
}

#[test]
fn a_link_capture_promotes_new_and_changed_files_and_a_whiteout_for_a_deletion() {
    let maker = tool(
        "maker",
        "Maker",
        &["/c", "echo", "one", ">", r"Data\gen.txt"],
    );
    let rewriter = tool(
        "rewriter",
        "Rewriter",
        &[
            "/c",
            "echo",
            "two",
            ">",
            r"Data\gen.txt",
            "&",
            "echo",
            "fresh",
            ">",
            r"Data\new.txt",
            "&",
            "echo",
            "value=2",
            ">",
            r"Data\Test.ini",
            "&",
            "del",
            r"Data\mod.txt",
        ],
    );
    let f = fixture(vec![maker, rewriter]);
    let item = f.add_mod(
        "Mod",
        &[
            ("Data/mod.txt", b"original"),
            ("Data/Test.ini", b"value=1\r\n"),
        ],
    );
    let mod_bytes = f.content_bytes(&item, "Data/mod.txt");
    let launcher = StagingLauncher::new();

    // The first tool makes a generated file: a copy in the farm, which the second tool rewrites.
    let first = f.run_as("maker", &launcher, CaptureMode::Links).unwrap();
    assert!(first.promoted, "{first:?}");
    assert_eq!(first.capture, CaptureMethod::Links);
    assert!(first.capture_reason.is_some());

    let out = f.run_as("rewriter", &launcher, CaptureMode::Links).unwrap();
    println!("link run outcome: {out:?}");
    assert!(out.promoted, "{out:?}");
    assert_eq!(out.capture, CaptureMethod::Links);
    assert_eq!(out.current.as_deref(), Some("1"));
    assert_eq!(out.deleted, vec!["Data/mod.txt".to_string()]);
    assert_eq!(
        out.written,
        vec![
            "Data/Test.ini".to_string(),
            "Data/gen.txt".to_string(),
            "Data/new.txt".to_string()
        ],
        "the new file, the rewritten generated file and the rewritten ini are the output"
    );

    let gen = f.generation_dir("rewriter", "1");
    assert_eq!(
        std::fs::read_to_string(gen.join("Data/gen.txt"))
            .unwrap()
            .trim(),
        "two"
    );
    assert_eq!(
        std::fs::read_to_string(gen.join("Data/new.txt"))
            .unwrap()
            .trim(),
        "fresh"
    );
    assert_eq!(
        std::fs::read_to_string(gen.join("Data/Test.ini"))
            .unwrap()
            .trim(),
        "value=2"
    );
    assert!(!gen.join("Data/mod.txt").exists());
    assert_eq!(
        whiteouts_of(&f, "rewriter"),
        vec!["Data/mod.txt".to_string()]
    );

    // The writable layer is the game's: a tool run never changes it.
    assert!(
        names(&f.writable_dir()).is_empty(),
        "{:?}",
        names(&f.writable_dir())
    );
    // The first tool's output is what it was.
    assert_eq!(
        std::fs::read_to_string(f.generation_dir("maker", "1").join("Data/gen.txt"))
            .unwrap()
            .trim(),
        "one"
    );
    // The deleted mod file's content object is untouched.
    assert_eq!(f.content_bytes(&item, "Data/mod.txt"), mod_bytes);
    // The farm the run used is taken away, so nothing it wrote is the game's.
    assert!(!f.deployed_game_dir().exists());

    // The next plan shows the new generation's bytes, and hides the deleted file.
    let p = f.plan(DeployMode::Links);
    assert!(planned(&p, "Data/mod.txt").is_none());
    let source = |rel: &str| {
        agora_core::game_deploy::visible_file_source(&f.ctx, f.id(), &f.def, DeployMode::Links, rel)
            .unwrap()
            .unwrap_or_else(|| panic!("no visible file {rel}"))
    };
    assert_eq!(
        std::fs::read_to_string(source("Data/gen.txt"))
            .unwrap()
            .trim(),
        "two"
    );
    assert_eq!(
        std::fs::read_to_string(source("Data/Test.ini"))
            .unwrap()
            .trim(),
        "value=2"
    );
}

#[test]
fn a_link_capture_that_changes_a_linked_file_is_refused_and_discarded() {
    let f = fixture(vec![tool(
        "relinker",
        "Relinker",
        &[
            "/c",
            "del",
            r"Data\mod.txt",
            "&",
            "echo",
            "changed",
            ">",
            r"Data\mod.txt",
        ],
    )]);
    let item = f.add_mod("Mod", &[("Data/mod.txt", b"original")]);
    let before = f.content_bytes(&item, "Data/mod.txt");

    let err = f
        .run_as("relinker", &StagingLauncher::new(), CaptureMode::Links)
        .unwrap_err();
    match err {
        ToolError::LinkedFileChanged {
            tool,
            paths,
            failed_folder,
        } => {
            assert_eq!(tool, "relinker");
            assert_eq!(paths, vec!["Data/mod.txt".to_string()]);
            let failed = failed_folder.expect("the discarded run is kept");
            assert!(failed.is_dir(), "{}", failed.display());
        }
        other => panic!("expected LinkedFileChanged, got {other:?}"),
    }
    assert!(f.generated("relinker").is_none());
    assert_eq!(f.content_bytes(&item, "Data/mod.txt"), before);
    assert!(names(&f.writable_dir()).is_empty());
    assert!(!f.deployed_game_dir().exists());
}

#[test]
fn a_write_through_a_linked_file_is_refused_and_the_run_is_not_promoted() {
    let f = fixture(vec![tool(
        "writer",
        "Writer",
        &["/c", "echo", "changed", ">", r"Data\mod.txt"],
    )]);
    let item = f.add_mod("Mod", &[("Data/mod.txt", b"original")]);
    let before = f.content_bytes(&item, "Data/mod.txt");

    // Whether the content store's ACL refuses the write (the tool fails) or lets it through (the
    // change is caught), the run is discarded and the content object keeps its bytes.
    match f.run_as("writer", &StagingLauncher::new(), CaptureMode::Links) {
        Ok(out) => {
            println!("write-through outcome: {out:?}");
            assert!(!out.promoted, "{out:?}");
        }
        Err(ToolError::LinkedFileChanged { paths, .. }) => {
            println!("write-through refused by the change check: {paths:?}");
            assert_eq!(paths, vec!["Data/mod.txt".to_string()]);
        }
        Err(other) => panic!("{other:?}"),
    }
    assert!(f.generated("writer").is_none());
    assert_eq!(f.content_bytes(&item, "Data/mod.txt"), before);
    assert!(names(&f.writable_dir()).is_empty());
}

#[test]
fn a_failing_link_run_is_discarded_and_the_farm_is_taken_away() {
    // Writes a new file, then exits 3: the file is kept in the discarded run only.
    let fails = tool(
        "fails",
        "Fails",
        &[
            "/c",
            "echo",
            "partial",
            ">",
            r"Data\partial.txt",
            "&",
            "exit",
            "/b",
            "3",
        ],
    );
    let f = fixture(vec![writes_or_fails("nemesis"), fails]);
    f.run_as("nemesis", &StagingLauncher::new(), CaptureMode::Links)
        .unwrap();
    assert_eq!(f.generated("nemesis").unwrap().0, "1");

    let out = f
        .run_as("fails", &StagingLauncher::new(), CaptureMode::Links)
        .expect("a failed run is an outcome, not an error");

    assert!(!out.promoted, "{out:?}");
    assert_eq!(out.exit_code, Some(3));
    assert_eq!(out.capture, CaptureMethod::Links);
    assert_eq!(out.current, None);
    let failed = out.failed_folder.expect("the discarded run is kept");
    assert!(failed.join("Data/partial.txt").is_file());
    assert!(f.generated("fails").is_none());
    assert_eq!(f.generated("nemesis").unwrap().0, "1");
    assert!(names(&f.writable_dir()).is_empty());
    assert!(!f.deployed_game_dir().exists());
    assert!(planned(&f.plan(DeployMode::Links), "Data/partial.txt").is_none());
}

#[test]
fn auto_captures_under_the_vfs_when_the_vfs_starts_and_says_so() {
    let f = fixture(vec![writes_or_fails("nemesis")]);

    let out = f.run("nemesis", &StagingLauncher::new()).unwrap();

    assert!(out.promoted, "{out:?}");
    assert_eq!(out.capture, CaptureMethod::Vfs);
    assert_eq!(out.capture_reason, None);
}

#[test]
fn auto_on_a_virtual_instance_refuses_with_links_as_the_next_tier_and_runs_nothing() {
    let _dll = EnvVar::set("AGORA_VFS_DLL", r"C:\agora-missing\agora_vfs.dll");
    let f = fixture(vec![writes_or_fails("nemesis")]);

    let err = f
        .run_as(
            "nemesis",
            &agora_core::game_launch::SystemLauncher,
            CaptureMode::Auto,
        )
        .unwrap_err();

    match &err {
        ToolError::VfsUnavailable { reason, next, .. } => {
            assert!(reason.contains("AGORA_VFS_DLL points at"), "{reason}");
            assert_eq!(*next, Some(CaptureMode::Links));
        }
        other => panic!("expected VfsUnavailable, got {other:?}"),
    }
    let text = err.to_string();
    assert!(
        text.contains("Nothing was run") && text.contains("linked files"),
        "{text}"
    );
    // Nothing was deployed, and no output was written: the user is asked before link capture runs.
    assert!(!f.deployed_game_dir().exists());
    assert!(f.generated("nemesis").is_none());
    assert_eq!(
        names(&f.tool_dir("nemesis")),
        vec!["failures.json".to_string()]
    );
}

#[test]
fn auto_on_a_virtual_instance_refuses_a_real_install_tool_with_swap_as_the_next_tier() {
    let f = fixture(vec![writes_real_data("nemesis")]);
    let before = snapshot(&real_data(&f));

    let err = f
        .run_as(
            "nemesis",
            &StagingLauncher::without_vfs("agora_vfs.dll was not found"),
            CaptureMode::Auto,
        )
        .unwrap_err();

    assert!(
        matches!(
            &err,
            ToolError::VfsUnavailable {
                next: Some(CaptureMode::Swap),
                ..
            }
        ),
        "{err:?}"
    );
    assert!(
        err.to_string().contains("copy of the instance's Data"),
        "{err}"
    );
    assert!(!f.deployed_game_dir().exists());
    assert!(!journal_file(&f).exists());
    assert_eq!(
        names(&f.tool_dir("nemesis")),
        vec!["failures.json".to_string()]
    );
    assert_eq!(snapshot(&real_data(&f)), before);
}

#[test]
fn auto_on_a_virtual_instance_runs_a_tool_that_uses_the_real_install_under_the_vfs() {
    let mut tool = writes_or_fails("nemesis");
    tool.uses_install_path = true;
    let f = fixture(vec![tool]);

    let out = f.run("nemesis", &StagingLauncher::new()).unwrap();

    assert!(out.promoted, "{out:?}");
    assert_eq!(out.capture, CaptureMethod::Vfs);
    assert_eq!(out.capture_reason, None);
}

#[test]
fn auto_on_a_linked_instance_captures_a_plain_tool_from_links_without_trying_the_vfs() {
    let f = fixture(vec![writes_or_fails("nemesis")]);
    set_deployment(&f.ctx, f.id(), Some(DeployMode::Links)).unwrap();

    // A VFS that cannot start would refuse a virtual run. A linked instance must not even try one.
    let out = f
        .run(
            "nemesis",
            &StagingLauncher::without_vfs("agora_vfs.dll was not found"),
        )
        .expect("a linked instance runs by links");

    assert!(out.promoted, "{out:?}");
    assert_eq!(out.capture, CaptureMethod::Links);
    let reason = out.capture_reason.expect("the reason is given");
    assert!(
        reason.contains("set to linked files") && !reason.contains("could not start"),
        "{reason}"
    );
    assert_eq!(out.written, vec!["Data/nemesis.txt".to_string()]);
    assert_eq!(
        std::fs::read_to_string(f.generation_dir("nemesis", "1").join("Data/nemesis.txt"))
            .unwrap()
            .trim(),
        "made"
    );
}

/// A launcher whose VFS ends a program the tool started: it writes the line the DLL writes to its log
/// during the run, then the tool fails. That is an injection failure.
struct EndsAProgram;

impl Launcher for EndsAProgram {
    fn locate_vfs_dll(&self) -> Result<PathBuf, String> {
        Ok(PathBuf::from(r"C:\agora\agora_vfs.dll"))
    }

    fn launch(&self, prepared: &PreparedLaunch) -> Result<LaunchedGame, LaunchError> {
        if let Some(vfs) = &prepared.vfs {
            std::fs::create_dir_all(vfs.log.parent().unwrap()).unwrap();
            let mut log = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&vfs.log)
                .unwrap();
            writeln!(
                log,
                "[4242] ending the process C:\\Games\\Tool\\helper.exe: anti-tamper refused the hook"
            )
            .unwrap();
        }
        StagingLauncher::new()
            .with_env("AGORA_TEST_FAIL", "1")
            .launch(prepared)
    }
}

// ---------------------------------------------------------------------------------------------
// A tool that keeps failing (MASTER_SPEC §26.9, decision 3): the offers after two discarded runs.
// ---------------------------------------------------------------------------------------------

fn failing() -> StagingLauncher {
    StagingLauncher::new().with_env("AGORA_TEST_FAIL", "1")
}

#[test]
fn a_tool_discarded_twice_in_a_row_offers_the_full_copy_swap_and_applies_nothing() {
    let f = fixture(vec![writes_or_fails("nemesis")]);

    let first = f.run("nemesis", &failing()).unwrap();
    assert!(!first.promoted);
    assert!(first.offers.is_empty(), "{:?}", first.offers);

    let second = f.run("nemesis", &failing()).unwrap();
    assert!(!second.promoted);
    match second.offers.as_slice() {
        [ToolOffer::CopySwap { costs }] => assert!(costs.data_files >= 1, "{costs:?}"),
        other => panic!("expected the copy swap offer, got {other:?}"),
    }
    // The offer changed nothing: no output, and the instance still has no deployment choice.
    assert!(f.generated("nemesis").is_none());
    assert_eq!(
        get_manifest(&f.ctx, f.id()).unwrap().deployment,
        None,
        "an offer switched the instance"
    );
}

#[test]
fn a_promoted_run_resets_the_count_of_discarded_runs() {
    let f = fixture(vec![writes_or_fails("nemesis")]);

    f.run("nemesis", &failing()).unwrap();
    f.run_ok("nemesis", &StagingLauncher::new());
    let after_promotion = f.run("nemesis", &failing()).unwrap();
    assert!(
        after_promotion.offers.is_empty(),
        "{:?}",
        after_promotion.offers
    );

    let again = f.run("nemesis", &failing()).unwrap();
    assert!(
        matches!(again.offers.as_slice(), [ToolOffer::CopySwap { .. }]),
        "{:?}",
        again.offers
    );
}

#[test]
fn a_missing_or_corrupt_count_file_counts_as_none_and_never_blocks_a_run() {
    let f = fixture(vec![writes_or_fails("nemesis")]);

    // Missing: the first discarded run is only the first, and the run goes through.
    let first = f
        .run("nemesis", &failing())
        .expect("a run with no count file");
    assert!(!first.promoted && first.offers.is_empty(), "{first:?}");

    // Corrupt: it reads as none, so the run is not blocked, and this discard is the first again.
    std::fs::write(f.tool_dir("nemesis").join("failures.json"), b"{ not json").unwrap();
    let corrupt = f
        .run("nemesis", &failing())
        .expect("a corrupt count file does not block a run");
    assert!(
        !corrupt.promoted && corrupt.offers.is_empty(),
        "{corrupt:?}"
    );

    let next = f.run("nemesis", &failing()).unwrap();
    assert!(
        matches!(next.offers.as_slice(), [ToolOffer::CopySwap { .. }]),
        "{:?}",
        next.offers
    );
}

#[test]
fn a_failure_the_vfs_injected_into_offers_switching_to_links_for_a_tool_with_its_own_folder() {
    let f = fixture(vec![writes_or_fails("nemesis")]);

    let first = f.run("nemesis", &EndsAProgram).unwrap();
    assert!(first.offers.is_empty(), "{:?}", first.offers);
    let second = f.run("nemesis", &EndsAProgram).unwrap();

    assert!(
        second.offers.contains(&ToolOffer::SwitchToLinks),
        "{:?}",
        second.offers
    );
    assert!(
        second
            .offers
            .iter()
            .any(|offer| matches!(offer, ToolOffer::CopySwap { .. })),
        "{:?}",
        second.offers
    );
    // Asked about, never done.
    assert_eq!(get_manifest(&f.ctx, f.id()).unwrap().deployment, None);
}

#[test]
fn a_tool_that_works_on_the_real_install_never_offers_switching_to_links() {
    let mut tool = writes_or_fails("nemesis");
    tool.uses_install_path = true;
    let f = fixture(vec![tool]);

    f.run("nemesis", &EndsAProgram).unwrap();
    let second = f.run("nemesis", &EndsAProgram).unwrap();

    assert!(
        !second.offers.contains(&ToolOffer::SwitchToLinks),
        "{:?}",
        second.offers
    );
    assert!(
        second
            .offers
            .iter()
            .any(|offer| matches!(offer, ToolOffer::CopySwap { .. })),
        "{:?}",
        second.offers
    );
}

#[test]
fn a_failure_without_injection_never_offers_switching_to_links() {
    let f = fixture(vec![writes_or_fails("nemesis")]);

    f.run("nemesis", &failing()).unwrap();
    let second = f.run("nemesis", &failing()).unwrap();

    assert!(
        !second.offers.contains(&ToolOffer::SwitchToLinks),
        "{:?}",
        second.offers
    );
}

#[test]
fn two_failed_swaps_do_not_offer_the_swap_again() {
    let _swap = SwapTest::same_volume();
    let f = swap_fixture(vec![writes_or_fails("nemesis")]);

    f.run_as("nemesis", &failing(), CaptureMode::Swap).unwrap();
    let second = f.run_as("nemesis", &failing(), CaptureMode::Swap).unwrap();

    assert!(!second.promoted, "{second:?}");
    assert!(second.offers.is_empty(), "{:?}", second.offers);
}

/// The offers of a refused virtual file system run.
fn refusal_offers(err: &ToolError) -> Vec<ToolOffer> {
    match err {
        ToolError::VfsUnavailable { offers, .. } => offers.clone(),
        other => panic!("expected VfsUnavailable, got {other:?}"),
    }
}

#[test]
fn two_refused_vfs_starts_offer_switching_to_links_on_the_second() {
    let f = fixture(vec![writes_or_fails("nemesis")]);
    let refuses = StagingLauncher::without_vfs("agora_vfs.dll was not found");

    let first = f.run_as("nemesis", &refuses, CaptureMode::Vfs).unwrap_err();
    assert!(refusal_offers(&first).is_empty(), "{first:?}");

    let second = f.run_as("nemesis", &refuses, CaptureMode::Vfs).unwrap_err();
    let offers = refusal_offers(&second);
    assert!(offers.contains(&ToolOffer::SwitchToLinks), "{offers:?}");
    assert!(
        offers
            .iter()
            .any(|offer| matches!(offer, ToolOffer::CopySwap { .. })),
        "{offers:?}"
    );
    // Asked about, never done: the instance keeps its deployment, and nothing ran.
    assert_eq!(get_manifest(&f.ctx, f.id()).unwrap().deployment, None);
    assert!(f.generated("nemesis").is_none());
}

#[test]
fn two_refused_vfs_starts_never_offer_switching_to_links_for_a_real_install_tool() {
    let mut tool = writes_or_fails("nemesis");
    tool.uses_install_path = true;
    let f = fixture(vec![tool]);
    let refuses = StagingLauncher::without_vfs("agora_vfs.dll was not found");

    f.run_as("nemesis", &refuses, CaptureMode::Auto)
        .unwrap_err();
    let second = f
        .run_as("nemesis", &refuses, CaptureMode::Auto)
        .unwrap_err();

    assert!(
        matches!(
            &second,
            ToolError::VfsUnavailable {
                next: Some(CaptureMode::Swap),
                ..
            }
        ),
        "{second:?}"
    );
    let offers = refusal_offers(&second);
    assert!(!offers.contains(&ToolOffer::SwitchToLinks), "{offers:?}");
}

#[test]
fn a_promoted_run_after_a_refusal_resets_the_count() {
    let f = fixture(vec![writes_or_fails("nemesis")]);
    let refuses = StagingLauncher::without_vfs("agora_vfs.dll was not found");

    f.run_as("nemesis", &refuses, CaptureMode::Vfs).unwrap_err();
    let launcher = StagingLauncher::new();
    let promoted = game_tools::run_after_vfs_refusal(
        &f.ctx,
        f.id(),
        &f.def,
        &ToolId::new("nemesis").unwrap(),
        CaptureMode::Links,
        "agora_vfs.dll was not found",
        &launcher,
        &CancellationToken::new(),
    )
    .unwrap();
    assert!(promoted.promoted, "{promoted:?}");

    // The count started again, so this refusal is the first of a new series, with no offer.
    let again = f.run_as("nemesis", &refuses, CaptureMode::Vfs).unwrap_err();
    assert!(refusal_offers(&again).is_empty(), "{again:?}");
}

#[test]
fn a_run_by_a_tier_chosen_after_a_refusal_names_the_vfs_failure_in_its_reason() {
    let f = fixture(vec![writes_or_fails("nemesis")]);
    let refusal = "agora_vfs.dll was not found";
    let err = f
        .run_as(
            "nemesis",
            &StagingLauncher::without_vfs(refusal),
            CaptureMode::Auto,
        )
        .unwrap_err();
    let ToolError::VfsUnavailable { reason, .. } = &err else {
        panic!("expected VfsUnavailable, got {err:?}");
    };

    let out = game_tools::run_after_vfs_refusal(
        &f.ctx,
        f.id(),
        &f.def,
        &ToolId::new("nemesis").unwrap(),
        CaptureMode::Links,
        reason,
        &StagingLauncher::new(),
        &CancellationToken::new(),
    )
    .unwrap();

    assert!(out.promoted, "{out:?}");
    assert_eq!(out.capture, CaptureMethod::Links);
    let why = out.capture_reason.expect("the fallback says why");
    assert!(
        why.contains("the virtual file system could not start (agora_vfs.dll was not found)")
            && why.contains("link capture was chosen"),
        "{why}"
    );
}

#[test]
fn a_swap_chosen_after_a_refusal_names_the_vfs_failure_in_its_reason() {
    let _swap = SwapTest::same_volume();
    let mut tool = writes_or_fails("nemesis");
    tool.uses_install_path = true;
    let f = swap_fixture(vec![tool]);

    let out = game_tools::run_after_vfs_refusal(
        &f.ctx,
        f.id(),
        &f.def,
        &ToolId::new("nemesis").unwrap(),
        CaptureMode::Swap,
        "agora_vfs.dll was not found",
        &StagingLauncher::new(),
        &CancellationToken::new(),
    )
    .unwrap();

    assert_eq!(out.capture, CaptureMethod::Swap);
    let why = out.capture_reason.expect("the fallback says why");
    assert!(
        why.contains("the virtual file system could not start")
            && why.contains("the full-copy swap was chosen"),
        "{why}"
    );
}

// ---------------------------------------------------------------------------------------------
// Unchanged copies (MASTER_SPEC §26.9): a VFS run's staged files the game would already read
// unchanged are dropped before the output is settled. These feed the staging folder directly.
// ---------------------------------------------------------------------------------------------

/// A staging folder for a tool, with `files` written into it.
fn stage(f: &Fixture, tool: &str, files: &[(&str, &[u8])]) -> PathBuf {
    let staging = f.tool_dir(tool).join("staging-test");
    std::fs::create_dir_all(&staging).unwrap();
    for (rel, bytes) in files {
        let path = rel.split('/').fold(staging.clone(), |p, part| p.join(part));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    staging
}

/// Runs the drop on a staging folder, as a VFS run does, and returns how many files it dropped.
fn drop_unchanged(f: &Fixture, tool: &str, staging: &Path) -> usize {
    let tool_def = f
        .ctx
        .games
        .tool(&f.def.id, &ToolId::new(tool).unwrap())
        .unwrap()
        .clone();
    game_tools::drop_unchanged_outputs(
        &f.ctx,
        f.id(),
        &f.def,
        &tool_def,
        &f.tool_dir(tool),
        staging,
        "test-run",
    )
    .unwrap()
}

#[test]
fn a_staged_file_identical_to_a_mods_file_is_dropped_and_counted() {
    let f = fixture(vec![tool("sweep", "Sweep", &["/c", "echo", "x"])]);
    f.add_mod("Mod", &[("Data/meshes/a.hkx", b"same bytes")]);
    let staging = stage(
        &f,
        "sweep",
        &[
            ("Data/meshes/a.hkx", b"same bytes"),
            ("Data/keep.txt", b"new"),
        ],
    );

    assert_eq!(drop_unchanged(&f, "sweep", &staging), 1);
    assert!(!staging.join("Data/meshes/a.hkx").exists());
    assert_eq!(
        std::fs::read(staging.join("Data/keep.txt")).unwrap(),
        b"new"
    );
}

#[test]
fn a_staged_file_that_differs_by_one_byte_at_the_same_size_is_kept() {
    let f = fixture(vec![tool("sweep", "Sweep", &["/c", "echo", "x"])]);
    f.add_mod("Mod", &[("Data/meshes/a.hkx", b"same bytes")]);
    let staging = stage(&f, "sweep", &[("Data/meshes/a.hkx", b"same bytez")]);

    assert_eq!(drop_unchanged(&f, "sweep", &staging), 0);
    assert_eq!(
        std::fs::read(staging.join("Data/meshes/a.hkx")).unwrap(),
        b"same bytez"
    );
}

#[test]
fn a_staged_file_equal_to_the_previous_generation_but_not_the_baseline_is_kept() {
    let f = fixture(vec![tool(
        "sweep",
        "Sweep",
        &[
            "/c",
            "mkdir",
            "Data",
            "&",
            "echo",
            "made>",
            r"Data\prev.txt",
        ],
    )]);
    f.run_ok("sweep", &StagingLauncher::new());
    let previous = std::fs::read(f.generation_dir("sweep", "1").join("Data/prev.txt")).unwrap();
    // Nothing below the tool holds Data/prev.txt, so the baseline does not hold it either.
    let staging = stage(&f, "sweep", &[("Data/prev.txt", &previous)]);

    assert_eq!(drop_unchanged(&f, "sweep", &staging), 0);
    assert!(staging.join("Data/prev.txt").is_file());
}

#[test]
fn a_staged_file_absent_below_is_kept() {
    let f = fixture(vec![tool("sweep", "Sweep", &["/c", "echo", "x"])]);
    f.add_mod("Mod", &[("Data/other.txt", b"mod")]);
    let staging = stage(&f, "sweep", &[("Data/new.txt", b"fresh")]);

    assert_eq!(drop_unchanged(&f, "sweep", &staging), 0);
    assert!(staging.join("Data/new.txt").is_file());
}

#[test]
fn a_whiteout_is_kept_and_stays_a_deletion() {
    let f = fixture(vec![tool("sweep", "Sweep", &["/c", "echo", "x"])]);
    f.add_mod("Mod", &[("Data/mod.txt", b"original")]);
    let staging = stage(&f, "sweep", &[]);
    let marker = staging.join(".agvfs-wh").join("Data").join("mod.txt.wh");
    std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
    std::fs::write(&marker, b"").unwrap();

    assert_eq!(drop_unchanged(&f, "sweep", &staging), 0);
    assert!(marker.is_file(), "the whiteout was removed");
}

#[test]
fn folders_emptied_by_dropped_files_are_removed_and_the_others_stay() {
    let f = fixture(vec![tool("sweep", "Sweep", &["/c", "echo", "x"])]);
    f.add_mod(
        "Mod",
        &[
            ("Data/meshes/actors/a/b.hkx", b"anim"),
            ("Data/meshes/c.hkx", b"mesh"),
        ],
    );
    let staging = stage(
        &f,
        "sweep",
        &[
            ("Data/meshes/actors/a/b.hkx", b"anim"),
            ("Data/meshes/keep.txt", b"kept"),
        ],
    );

    assert_eq!(drop_unchanged(&f, "sweep", &staging), 1);
    assert!(!staging.join("Data/meshes/actors").exists());
    assert!(staging.join("Data/meshes/keep.txt").is_file());
    assert!(staging.is_dir(), "the staging folder itself stays");

    std::fs::remove_dir_all(&staging).unwrap();
    let staging = stage(&f, "sweep", &[("Data/meshes/c.hkx", b"mesh")]);
    assert_eq!(drop_unchanged(&f, "sweep", &staging), 1);
    assert_eq!(names(&staging), Vec::<String>::new(), "nothing is left");
    assert!(staging.is_dir());
}

#[test]
fn a_required_output_equal_to_the_baseline_still_passes_and_is_not_output() {
    let mut required = tagged("sweep");
    required.required_outputs = vec!["Data/mod.txt".to_string()];
    let f = fixture(vec![required]);
    f.add_mod("Mod", &[("Data/mod.txt", b"tag 1 \r\n")]);

    // The run writes the mod's own bytes, so the file is dropped, and the check still finds it: the
    // game reads it from the mod.
    let out = f
        .run_as(
            "sweep",
            &StagingLauncher::new().with_env("AGORA_TEST_TAG", "1"),
            CaptureMode::Vfs,
        )
        .unwrap();
    assert!(out.promoted, "{out:?}");
    assert_eq!(out.unchanged_dropped, 1, "{out:?}");
    assert!(out.written.is_empty(), "{:?}", out.written);
    assert!(names(&f.generation_dir("sweep", "1")).is_empty());
}

#[test]
fn a_failure_marker_is_searched_only_in_files_the_run_changed() {
    let mut marked = tagged("sweep");
    marked.failure_markers = vec![FailureMarker {
        file: RelPath::new("Data/mod.txt").unwrap(),
        contains: "tag".into(),
    }];
    let f = fixture(vec![marked]);
    f.add_mod("Mod", &[("Data/mod.txt", b"tag 1 \r\n")]);

    // The same bytes as the mod: the file is dropped before the marker is searched, so the run passes.
    let same = f
        .run_as(
            "sweep",
            &StagingLauncher::new().with_env("AGORA_TEST_TAG", "1"),
            CaptureMode::Vfs,
        )
        .unwrap();
    assert!(same.promoted, "{same:?}");
    assert_eq!(same.unchanged_dropped, 1);

    // Changed bytes: the file is written, and the marker in it fails the run.
    let changed = f
        .run_as(
            "sweep",
            &StagingLauncher::new().with_env("AGORA_TEST_TAG", "2"),
            CaptureMode::Vfs,
        )
        .unwrap();
    assert!(!changed.promoted, "{changed:?}");
    assert_eq!(changed.unchanged_dropped, 0);
    assert_eq!(changed.written, vec!["Data/mod.txt".to_string()]);
    assert!(changed.failure.is_some(), "{changed:?}");
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

/// A tool that opens a mod's file for writing and writes nothing to it, then writes one new file. The VFS
/// copies the mod's file up on the open, so the copy is staged; it is identical to the mod's file, so it
/// is not output (MASTER_SPEC §26.9).
#[test]
#[ignore = "injects agora_vfs.dll: cargo build -p agora-vfs, then set AGORA_VFS_DLL"]
fn real_injection_a_file_opened_for_writing_and_not_changed_is_not_output() {
    let opener = tool(
        "opener",
        "Opener",
        &[
            "/c",
            "type",
            "nul",
            ">>",
            r"Data\mod.txt",
            "&",
            "echo",
            "new>",
            r"Data\new.txt",
        ],
    );
    let f = fixture(vec![opener]);
    let item = f.add_mod("Mod", &[("Data/mod.txt", b"original")]);
    let mod_bytes = f.content_bytes(&item, "Data/mod.txt");

    let out = f.run_ok("opener", &RealLauncher::new(&[]));
    println!("opener outcome: {out:?}");
    assert_eq!(out.written, vec!["Data/new.txt".to_string()], "{out:?}");
    assert_eq!(out.unchanged_dropped, 1, "{out:?}");
    let generation = f.generation_dir("opener", "1");
    assert_eq!(names(&generation), vec!["Data".to_string()]);
    assert_eq!(names(&generation.join("Data")), vec!["new.txt".to_string()]);
    assert_eq!(f.content_bytes(&item, "Data/mod.txt"), mod_bytes);
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

#[test]
#[ignore = "injects agora_vfs.dll: cargo build -p agora-vfs, then set AGORA_VFS_DLL"]
fn real_injection_a_32_bit_tool_is_captured_from_links_under_auto_when_the_x86_dll_is_missing() {
    // The game's own executable is the 32-bit cmd.exe. Without the 32-bit DLL it cannot run under the
    // VFS, so Auto captures from links and says where the 32-bit files were looked for.
    let _lock = X86_DLL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _x86 = X86Dll::set(r"C:\agora-missing\x86\agora_vfs.dll");
    let f = fixture_with_exe(
        vec![tool("wow", "Wow", &["/c", "echo", "x", ">", "out.txt"])],
        false,
        false,
        r"C:\Windows\SysWOW64\cmd.exe",
    );
    let launcher = RealLauncher::new(&[]);

    // A virtual instance does not fall back by itself: the refusal names link capture as the next tier.
    let err = f
        .run_as("wow", &launcher, CaptureMode::Auto)
        .expect_err("a 32-bit tool is refused under the VFS, and nothing runs");
    let refusal = match &err {
        ToolError::VfsUnavailable { reason, next, .. } => {
            assert!(reason.contains("32-bit"), "{reason}");
            assert_eq!(*next, Some(CaptureMode::Links));
            reason.clone()
        }
        other => panic!("expected VfsUnavailable, got {other:?}"),
    };
    // The user's answer is link capture, which runs the tool, and its reason names what failed first.
    let out = game_tools::run_after_vfs_refusal(
        &f.ctx,
        f.id(),
        &f.def,
        &ToolId::new("wow").unwrap(),
        CaptureMode::Links,
        &refusal,
        &launcher,
        &CancellationToken::new(),
    )
    .expect("a 32-bit tool runs by link capture once asked for");
    println!("run outcome: {out:?}");
    assert_eq!(out.capture, CaptureMethod::Links, "{out:?}");
    let reason = out.capture_reason.clone().unwrap_or_default();
    assert!(
        reason.contains("32-bit program and the 32-bit VFS files were not found at")
            && reason.contains(r"agora-missing"),
        "{reason}"
    );
    assert!(out.promoted, "{out:?}");
    assert_eq!(out.written, vec!["out.txt".to_string()]);
    assert_eq!(
        std::fs::read_to_string(f.generation_dir("wow", "1").join("out.txt"))
            .unwrap()
            .trim(),
        "x"
    );
    assert!(names(&f.writable_dir()).is_empty());
    assert!(!f.deployed_game_dir().exists());
}

/// A 32-bit tool runs under the VFS when the 32-bit files exist: the 32-bit probe is the tool, and its
/// own write lands in the run's promoted generation. Needs `AGORA_VFS_DLL` and `AGORA_VFS_DLL_X86`.
#[test]
#[ignore = "injects agora_vfs.dll into a 32-bit tool: cargo build -p agora-vfs, cargo build -p agora-vfs --target i686-pc-windows-msvc, set AGORA_VFS_DLL and AGORA_VFS_DLL_X86"]
fn real_injection_a_32_bit_tool_runs_under_the_vfs_and_its_write_is_promoted() {
    let _lock = X86_DLL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let x86_dll = std::env::var_os("AGORA_VFS_DLL_X86")
        .map(PathBuf::from)
        .expect("set AGORA_VFS_DLL_X86 to target/i686-pc-windows-msvc/debug/agora_vfs.dll");
    assert!(x86_dll.is_file(), "{} does not exist", x86_dll.display());
    let probe = x86_dll
        .parent()
        .expect("the 32-bit DLL's folder")
        .join("agora-vfs-probe.exe");
    assert!(probe.is_file(), "{} does not exist", probe.display());
    // The probe's results go to a folder outside the view; it writes its files into the game's Data.
    let out = std::env::temp_dir().join(format!("agora-tool32-out-{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    let f = fixture_with_exe(
        vec![tool(
            "probe32",
            "Probe",
            &["--root", ".", "--out", out.to_str().unwrap()],
        )],
        false,
        false,
        probe.to_str().unwrap(),
    );
    let launcher = RealLauncher::new(&[]);

    let run = f.run_as("probe32", &launcher, CaptureMode::Vfs);
    let _ = std::fs::remove_dir_all(&out);
    let run = run.expect("run the 32-bit tool under the VFS");
    println!("run outcome: {run:?}");
    assert_eq!(run.capture, CaptureMethod::Vfs, "{run:?}");
    assert!(run.promoted, "{run:?}");
    assert!(
        run.written.iter().any(|w| w == "Data/created.txt"),
        "{:?}",
        run.written
    );
    assert_eq!(
        std::fs::read_to_string(
            f.generation_dir("probe32", "1")
                .join("Data")
                .join("created.txt")
        )
        .unwrap(),
        "NEW"
    );
    assert!(names(&f.writable_dir()).is_empty());
}

#[test]
#[ignore = "injects agora_vfs.dll: cargo build -p agora-vfs, then set AGORA_VFS_DLL"]
fn real_injection_a_tool_that_writes_the_real_install_path_is_staged_and_the_install_is_unchanged()
{
    // The tool does not declare uses_install_path. Its install folder is an alias of the view, so
    // its write to the real install's Data lands in this run's staging folder, and is promoted.
    let f = fixture(vec![tool(
        "alias-writer",
        "Nemesis",
        &[
            "/c",
            "echo",
            "made",
            ">",
            r"%AGORA_REAL%\Data\from_alias.txt",
        ],
    )]);
    let install = install_dir(&f);
    let launcher = RealLauncher::new(&[("AGORA_REAL", install.to_str().unwrap())]);
    let before = snapshot(&install);

    let out = f
        .run_as("alias-writer", &launcher, CaptureMode::Vfs)
        .expect("run under the VFS");
    println!("run outcome: {out:?}");
    assert_eq!(out.capture, CaptureMethod::Vfs, "{out:?}");
    assert!(out.promoted, "{out:?}");
    assert_eq!(out.written, vec!["Data/from_alias.txt".to_string()]);
    assert_eq!(
        std::fs::read_to_string(
            f.generation_dir("alias-writer", "1")
                .join("Data/from_alias.txt")
        )
        .unwrap()
        .trim(),
        "made"
    );
    assert_eq!(snapshot(&install), before, "the real install changed");
    assert!(names(&f.writable_dir()).is_empty());
}

// ---------------------------------------------------------------------------
// The swap and the real install (MASTER_SPEC §26.9, slices 4c and 4e). Every test uses a fake install
// in a temp folder. The swap runs its tool on a real copy of the instance's `Data`, kept between runs
// on the same volume, or made for one run on another volume (seam).
// ---------------------------------------------------------------------------

use std::collections::{BTreeMap, BTreeSet};
use std::process::{Command, Stdio};
use std::time::Duration;

use agora_core::game_deploy::source_path;
use agora_core::game_tool_copy::{self, CopyKind};
use agora_core::game_tool_swap::{self as swap, SpaceNeed, SwapError, SwapJournal, VolumeSeam};

/// A tool that writes `Data\meshes\x.nif` in the real install, through `AGORA_REAL`.
fn writes_real_data(id: &str) -> ToolDefinition {
    swap_tool(
        id,
        "Nemesis",
        &[
            "/c",
            "mkdir",
            r"%AGORA_REAL%\Data\meshes",
            "&",
            "echo",
            "nif",
            ">",
            r"%AGORA_REAL%\Data\meshes\x.nif",
        ],
    )
}

fn install_dir(f: &Fixture) -> PathBuf {
    f._tmp.path().join("install")
}

fn real_data(f: &Fixture) -> PathBuf {
    install_dir(f).join("Data")
}

fn journal_file(f: &Fixture) -> PathBuf {
    f.ctx.paths.tool_swap_journal_path(GAME, "steam")
}

fn aside_named(f: &Fixture, name: &str) -> PathBuf {
    install_dir(f).join(format!("{}{name}", swap::ASIDE_PREFIX))
}

/// Every entry under `root` with its bytes; folders are listed with a trailing `/`, links as
/// `(link)`. A missing folder is empty. Used to prove the real folder came back unchanged.
fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if swap::is_reparse_point(&path) {
                out.insert(format!("{rel} (link)"), Vec::new());
            } else if path.is_dir() {
                out.insert(format!("{rel}/"), Vec::new());
                walk(root, &path, out);
            } else {
                out.insert(rel, std::fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    if root.is_dir() {
        walk(root, root, &mut out);
    }
    out
}

/// The volume seam is process-wide, so every swap test takes this lock, sets the seam it needs, and
/// clears it when it ends.
static SWAP_SEAM: Mutex<()> = Mutex::new(());

struct SwapTest {
    _lock: MutexGuard<'static, ()>,
}

impl SwapTest {
    fn with(seam: VolumeSeam) -> Self {
        let lock = SWAP_SEAM.lock().unwrap_or_else(|e| e.into_inner());
        swap::set_volume_seam(Some(seam));
        Self { _lock: lock }
    }

    /// The copy is kept on the install's volume: the normal case.
    fn same_volume() -> Self {
        Self::with(VolumeSeam {
            same_volume: Some(true),
            free_bytes: None,
        })
    }

    /// The copy is made for the run on the install's volume: the other-volume case, simulated.
    fn different_volume() -> Self {
        Self::with(VolumeSeam {
            same_volume: Some(false),
            free_bytes: None,
        })
    }
}

impl Drop for SwapTest {
    fn drop(&mut self) {
        swap::set_volume_seam(None);
    }
}

/// A tool that works on the real install: the same as [`tool`], with `uses_install_path` set.
fn swap_tool(id: &str, name: &str, args: &[&str]) -> ToolDefinition {
    ToolDefinition {
        uses_install_path: true,
        required_outputs: Vec::new(),
        failure_markers: Vec::new(),
        ..tool(id, name, args)
    }
}

/// A tool that writes `Data\meshes\<name>` in the real install, through `AGORA_REAL`.
fn writes_mesh(id: &str, name: &str) -> ToolDefinition {
    let target = format!(r"%AGORA_REAL%\Data\meshes\{name}");
    let args: Vec<&str> = vec![
        "/c",
        "mkdir",
        r"%AGORA_REAL%\Data\meshes",
        "&",
        "echo",
        "nif",
        ">",
        target.as_str(),
    ];
    swap_tool(id, "Nemesis", &args)
}

/// A tool that overwrites `Data\tweak.ini`, creates `Data\meshes\x.nif` and deletes `Data\delete_me.txt`.
fn writes_and_deletes(id: &str) -> ToolDefinition {
    let args: Vec<&str> = vec![
        "/c",
        "mkdir",
        r"%AGORA_REAL%\Data\meshes",
        "&",
        "echo",
        "nif",
        ">",
        r"%AGORA_REAL%\Data\meshes\x.nif",
        "&",
        "echo",
        "changed",
        ">",
        r"%AGORA_REAL%\Data\tweak.ini",
        "&",
        "del",
        r"%AGORA_REAL%\Data\delete_me.txt",
    ];
    swap_tool(id, "Nemesis", &args)
}

/// A tool that opens `Data\Skyrim.bsa` for writing and writes nothing to it.
fn opens_for_writing_only(id: &str) -> ToolDefinition {
    swap_tool(
        id,
        "Nemesis",
        &["/c", "type", "nul", ">>", r"%AGORA_REAL%\Data\Skyrim.bsa"],
    )
}

/// A tool that changes `Data` and then exits 3.
fn writes_then_fails(id: &str) -> ToolDefinition {
    swap_tool(
        id,
        "Nemesis",
        &[
            "/c",
            "echo",
            "bad",
            ">",
            r"%AGORA_REAL%\Data\bad.txt",
            "&",
            "echo",
            "x",
            ">",
            r"%AGORA_REAL%\Data\tweak.ini",
            "&",
            "del",
            r"%AGORA_REAL%\Data\delete_me.txt",
            "&",
            "exit",
            "/b",
            "3",
        ],
    )
}

/// A swap fixture: `Data` holds three files the tests keep, change and delete.
fn swap_fixture(tools: Vec<ToolDefinition>) -> Fixture {
    let tmp = TempDir::new().unwrap();
    let def = definition(&tools, false, false);
    let ctx = CoreContext::for_testing(tmp.path().join("app_data"));
    agora_core::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();
    let ctx = ctx.with_games(registry(&def, tools));
    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(install_dir.join("Data")).unwrap();
    std::fs::copy(r"C:\Windows\System32\cmd.exe", install_dir.join("Game.exe")).unwrap();
    std::fs::write(install_dir.join("Data").join("Skyrim.bsa"), b"BSA DATA").unwrap();
    std::fs::write(install_dir.join("Data").join("tweak.ini"), b"ini original").unwrap();
    std::fs::write(install_dir.join("Data").join("delete_me.txt"), b"delete me").unwrap();
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

fn swap_launcher(f: &Fixture) -> StagingLauncher {
    StagingLauncher::new().with_env("AGORA_REAL", install_dir(f).to_str().unwrap())
}

/// The swap folder of the instance: `<bases>\deployments\<instance>\swap`.
fn swap_dir(f: &Fixture) -> PathBuf {
    swap::swap_location(&f.ctx, f.id()).unwrap().swap_dir
}

/// The kept copy of the instance's `Data`.
fn kept_copy(f: &Fixture) -> PathBuf {
    swap_dir(f).join("Data")
}

/// The files the instance's plan puts under `Data`, as a swap's copy holds them.
fn planned_files(f: &Fixture) -> Vec<PlannedFile> {
    game_tool_copy::data_files(&f.ctx, f.id(), &f.def).unwrap()
}

/// The kept copy holds exactly the planned files, each with the bytes of its source, and no marker
/// output. This is what a discarded run must leave behind, and what every finished run leaves.
fn assert_copy_matches_plan(f: &Fixture) {
    let copy = kept_copy(f);
    let planned = planned_files(f);
    let on_disk: BTreeSet<String> = snapshot(&copy)
        .into_iter()
        .filter(|(k, _)| !k.ends_with('/') && k.as_str() != game_tool_copy::MARKER_FILE)
        .map(|(k, _)| k.to_ascii_lowercase())
        .collect();
    let wanted: BTreeSet<String> = planned
        .iter()
        .map(|p| p.path.as_str().to_ascii_lowercase())
        .collect();
    assert_eq!(
        on_disk, wanted,
        "the kept copy holds exactly the planned files"
    );
    for file in &planned {
        let want = std::fs::read(source_path(&f.ctx, &file.source)).unwrap();
        let got = std::fs::read(copy.join(file.path.as_str())).unwrap();
        assert_eq!(got, want, "{} matches its source", file.path.as_str());
    }
}

/// A journal for a swap a test sets up by hand, exactly as `run_by_swap` writes it.
fn journal_of(f: &Fixture, run: &str, kind: CopyKind) -> SwapJournal {
    let loc = swap::swap_location(&f.ctx, f.id()).unwrap();
    SwapJournal {
        version: swap::JOURNAL_VERSION,
        game: GAME.into(),
        store: "steam".into(),
        instance_id: f.id().into(),
        tool: "nemesis".into(),
        run: run.into(),
        started_unix_ms: 0,
        install_dir: loc.install.clone(),
        real_data: loc.install.join("Data"),
        aside: loc.install.join(format!("{}{run}", swap::ASIDE_PREFIX)),
        copy_kind: kind,
        copy_dir: game_tool_copy::copy_dir_for(kind, &loc, run),
        swap_dir: loc.swap_dir.clone(),
    }
}

/// Simulates a crash in a kept-copy swap after `step` (MASTER_SPEC §26.9, the run's steps): 4 the real
/// Data is renamed aside; 5 the copy is renamed into its place; 6 the tool has written into it; 7 the
/// copy has been renamed back home. The journal is on disk.
fn crash_after(f: &Fixture, step: u8, kind: CopyKind, run: &str) -> SwapJournal {
    let journal = journal_of(f, run, kind);
    swap::write_journal(&f.ctx, &journal).unwrap();
    game_tool_copy::prepare(&f.ctx, &journal, &planned_files(f)).unwrap();
    if step >= 4 {
        std::fs::rename(&journal.real_data, &journal.aside).unwrap();
    }
    if step >= 5 {
        std::fs::rename(&journal.copy_dir, &journal.real_data).unwrap();
    }
    if step >= 6 {
        let data = &journal.real_data;
        std::fs::write(data.join("tweak.ini"), b"tool wrote").unwrap();
        std::fs::write(data.join("new.txt"), b"tool new").unwrap();
        std::fs::remove_file(data.join("delete_me.txt")).unwrap();
    }
    if step >= 7 {
        std::fs::rename(&journal.real_data, &journal.copy_dir).unwrap();
    }
    journal
}

#[test]
fn a_swap_run_promotes_its_writes_and_the_real_data_is_byte_for_byte_unchanged() {
    let _swap = SwapTest::same_volume();
    let f = swap_fixture(vec![writes_and_deletes("nemesis")]);
    let install = install_dir(&f);
    let before = snapshot(&real_data(&f));
    let planned = planned_files(&f).len();

    let outcome = f
        .run_as("nemesis", &swap_launcher(&f), CaptureMode::Swap)
        .unwrap();

    assert!(outcome.promoted, "{outcome:?}");
    assert_eq!(outcome.capture, CaptureMethod::Swap);
    assert_eq!(outcome.copied_layer, None);
    assert!(outcome.written.contains(&"Data/meshes/x.nif".to_string()));
    assert!(outcome.written.contains(&"Data/tweak.ini".to_string()));
    assert_eq!(outcome.deleted, vec!["Data/delete_me.txt".to_string()]);
    // The real Data is unchanged, and it is the real folder again: no copy, no aside, no journal.
    assert_eq!(snapshot(&real_data(&f)), before);
    assert!(!real_data(&f).join("meshes").join("x.nif").exists());
    assert!(names(&install)
        .iter()
        .all(|n| !n.starts_with(swap::ASIDE_PREFIX)));
    assert!(!journal_file(&f).exists());
    // The output is the promoted generation.
    let (generation, _) = f.generated("nemesis").expect("a generated layer");
    let gen_dir = f.generation_dir("nemesis", &generation);
    assert!(gen_dir.join("Data").join("meshes").join("x.nif").exists());
    assert_eq!(
        std::fs::read_to_string(gen_dir.join("Data").join("tweak.ini"))
            .unwrap()
            .trim(),
        "changed"
    );
    // The kept copy now matches the plan, which has the new generation and not the deleted file.
    assert_copy_matches_plan(&f);
    let report = outcome.swap.expect("a swap report");
    assert_eq!(report.copy, CopyKind::Kept);
    assert_eq!(
        report.copied_in_files, planned,
        "the first run copies everything"
    );
}

#[test]
fn a_second_run_copies_only_what_changed() {
    let _swap = SwapTest::same_volume();
    let f = swap_fixture(vec![
        writes_mesh("first", "x.nif"),
        writes_mesh("second", "y.nif"),
    ]);
    let planned = planned_files(&f).len();

    let first = f
        .run_as("first", &swap_launcher(&f), CaptureMode::Swap)
        .unwrap();
    assert!(first.promoted, "{first:?}");
    let first_report = first.swap.expect("a swap report");
    assert_eq!(first_report.copied_in_files, planned);
    assert_eq!(first_report.captured_files, 1);
    // The first run's own output joins the plan, so the copy takes that one file after the run.
    assert_eq!(first_report.refreshed_files, 1);

    let second = f
        .run_as("second", &swap_launcher(&f), CaptureMode::Swap)
        .unwrap();
    assert!(second.promoted, "{second:?}");
    let second_report = second.swap.expect("a swap report");
    assert_eq!(
        second_report.copied_in_files, 0,
        "nothing changed since the last run, so nothing is copied in"
    );
    assert_eq!(second_report.captured_files, 1);
    assert_eq!(second_report.refreshed_files, 1);
    assert_copy_matches_plan(&f);
}

#[test]
fn a_tool_that_opens_a_file_for_writing_and_changes_nothing_produces_no_output() {
    let _swap = SwapTest::same_volume();
    let f = swap_fixture(vec![opens_for_writing_only("nemesis")]);
    let before = snapshot(&real_data(&f));

    let outcome = f
        .run_as("nemesis", &swap_launcher(&f), CaptureMode::Swap)
        .unwrap();

    assert!(outcome.promoted, "{outcome:?}");
    assert_eq!(outcome.exit_code, Some(0));
    assert!(outcome.written.is_empty(), "{:?}", outcome.written);
    assert!(outcome.deleted.is_empty(), "{:?}", outcome.deleted);
    assert_eq!(snapshot(&real_data(&f)), before);
    assert_copy_matches_plan(&f);
}

#[test]
fn a_discarded_run_leaves_the_kept_copy_matching_the_plan_again() {
    let _swap = SwapTest::same_volume();
    let f = swap_fixture(vec![writes_then_fails("nemesis")]);
    let before = snapshot(&real_data(&f));

    let outcome = f
        .run_as("nemesis", &swap_launcher(&f), CaptureMode::Swap)
        .unwrap();

    assert!(!outcome.promoted, "{outcome:?}");
    assert_eq!(outcome.exit_code, Some(3));
    assert!(outcome.failed_folder.is_some());
    assert_eq!(snapshot(&real_data(&f)), before);
    assert!(!journal_file(&f).exists());
    assert!(f.generated("nemesis").is_none());
    // The run's edits did not survive into the copy.
    assert!(!kept_copy(&f).join("bad.txt").exists());
    assert_copy_matches_plan(&f);
}

#[test]
fn on_another_volume_a_copy_is_made_for_the_run_and_deleted_after_it() {
    let _swap = SwapTest::different_volume();
    let f = swap_fixture(vec![writes_and_deletes("nemesis")]);
    let install = install_dir(&f);
    let before = snapshot(&real_data(&f));
    let planned = planned_files(&f).len();

    let outcome = f
        .run_as("nemesis", &swap_launcher(&f), CaptureMode::Swap)
        .unwrap();

    assert!(outcome.promoted, "{outcome:?}");
    let report = outcome.swap.expect("a swap report");
    assert_eq!(report.copy, CopyKind::PerRun);
    assert_eq!(
        report.copied_in_files, planned,
        "every run copies everything in"
    );
    assert!(
        !kept_copy(&f).exists(),
        "no kept copy is made on another volume"
    );
    assert!(names(&install)
        .iter()
        .all(|n| !n.starts_with(swap::COPY_PREFIX) && !n.starts_with(swap::ASIDE_PREFIX)));
    assert_eq!(snapshot(&real_data(&f)), before);
    assert!(!journal_file(&f).exists());
}

#[test]
fn a_swap_is_refused_before_anything_moves_when_the_copy_volume_lacks_space() {
    let _swap = SwapTest::with(VolumeSeam {
        same_volume: Some(true),
        free_bytes: Some(1),
    });
    let f = swap_fixture(vec![writes_and_deletes("nemesis")]);
    let before = snapshot(&real_data(&f));

    let result = f.run_as("nemesis", &swap_launcher(&f), CaptureMode::Swap);

    match result {
        Err(ToolError::Swap(SwapError::Refused(message))) => {
            assert!(message.contains("needs"), "{message}");
            assert!(message.contains("only 1 bytes is free"), "{message}");
        }
        other => panic!("expected a refusal for space, got {other:?}"),
    }
    assert_eq!(snapshot(&real_data(&f)), before);
    assert!(!journal_file(&f).exists());
    assert!(!swap_dir(&f).exists());
}

#[test]
fn a_swap_is_refused_while_a_program_runs_from_the_install() {
    let f = fixture(vec![writes_real_data("nemesis")]);
    let _swap = SwapTest::same_volume();
    let install = install_dir(&f);
    std::fs::copy(r"C:\Windows\System32\cmd.exe", install.join("busy.exe")).unwrap();
    let mut busy = Command::new(install.join("busy.exe"))
        .args(["/c", "ping", "-n", "60", "127.0.0.1"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let before = snapshot(&real_data(&f));
    let launcher = StagingLauncher::new().with_env("AGORA_REAL", install.to_str().unwrap());

    let result = f.run_as("nemesis", &launcher, CaptureMode::Swap);
    let _ = busy.kill();
    let _ = busy.wait();

    match result {
        Err(ToolError::Swap(SwapError::Refused(message))) => {
            assert!(message.contains("install folder"), "{message}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert_eq!(snapshot(&real_data(&f)), before);
    assert!(!swap::is_reparse_point(&real_data(&f)));
    assert!(!journal_file(&f).exists());
    assert!(!swap_dir(&f).exists());
}

#[test]
fn a_swap_is_refused_when_the_real_data_is_already_a_link() {
    let _swap = SwapTest::same_volume();
    let f = fixture(vec![writes_real_data("nemesis")]);
    let install = install_dir(&f);
    let kept = install.join("Data.kept");
    std::fs::rename(real_data(&f), &kept).unwrap();
    junction::create(&kept, real_data(&f)).unwrap();
    let launcher = StagingLauncher::new().with_env("AGORA_REAL", install.to_str().unwrap());

    let result = f.run_as("nemesis", &launcher, CaptureMode::Swap);

    match result {
        Err(ToolError::Swap(SwapError::Refused(message))) => {
            assert!(message.contains("already"), "{message}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert!(swap::is_reparse_point(&real_data(&f)));
    assert!(kept.is_dir());
    assert!(!journal_file(&f).exists());
}

#[test]
fn a_taken_aside_name_is_refused_before_anything_moves() {
    let f = fixture(vec![writes_real_data("nemesis")]);
    let install = install_dir(&f);
    let aside = aside_named(&f, "taken");
    std::fs::create_dir_all(&aside).unwrap();
    std::fs::write(aside.join("keep.txt"), b"keep").unwrap();
    let before = snapshot(&real_data(&f));
    let space = SpaceNeed {
        folder: install.clone(),
        bytes: 0,
    };

    let result = swap::preflight(&install, &real_data(&f), &aside, &space);

    match result {
        Err(SwapError::Refused(message)) => {
            assert!(message.contains("already exists"), "{message}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert_eq!(snapshot(&real_data(&f)), before);
    assert_eq!(std::fs::read(aside.join("keep.txt")).unwrap(), b"keep");
}

#[test]
fn auto_refuses_a_real_install_tool_on_a_linked_instance_before_anything_is_deployed() {
    let f = fixture(vec![writes_real_data("nemesis")]);
    set_deployment(&f.ctx, f.id(), Some(DeployMode::Links)).unwrap();

    let result = f.run_as("nemesis", &StagingLauncher::new(), CaptureMode::Auto);

    assert!(
        matches!(result, Err(ToolError::InstallPathNeedsSwap { .. })),
        "{result:?}"
    );
    assert!(!f.deployed_game_dir().exists());
    assert!(!journal_file(&f).exists());
}

#[test]
fn a_crash_after_step_4_is_put_back_and_the_copy_is_discarded() {
    crash_case(4, CopyKind::Kept);
}

#[test]
fn a_crash_after_step_5_is_put_back_and_the_copy_is_discarded() {
    crash_case(5, CopyKind::Kept);
}

#[test]
fn a_crash_after_step_6_is_put_back_and_the_tools_writes_are_discarded() {
    crash_case(6, CopyKind::Kept);
}

#[test]
fn a_crash_after_step_7_is_put_back_and_the_copy_is_discarded() {
    crash_case(7, CopyKind::Kept);
}

#[test]
fn a_crash_on_another_volume_is_put_back_and_the_per_run_copy_is_deleted() {
    crash_case_on_other_volume(6);
}

fn crash_case(step: u8, kind: CopyKind) {
    let _swap = SwapTest::same_volume();
    let f = swap_fixture(vec![writes_and_deletes("nemesis")]);
    let install = install_dir(&f);
    let before = snapshot(&real_data(&f));
    crash_after(&f, step, kind, "crash");

    let restored = swap::recover_game(&f.ctx, &f.def).unwrap();

    assert_eq!(restored.len(), 1, "step {step}");
    assert_eq!(snapshot(&real_data(&f)), before, "step {step}");
    assert!(!kept_copy(&f).exists(), "step {step}: the copy is deleted");
    assert!(names(&install)
        .iter()
        .all(|n| !n.starts_with(swap::ASIDE_PREFIX)));
    assert!(!journal_file(&f).exists());
}

fn crash_case_on_other_volume(step: u8) {
    let _swap = SwapTest::different_volume();
    let f = swap_fixture(vec![writes_and_deletes("nemesis")]);
    let install = install_dir(&f);
    let before = snapshot(&real_data(&f));
    crash_after(&f, step, CopyKind::PerRun, "crashpr");

    let restored = swap::recover_game(&f.ctx, &f.def).unwrap();

    assert_eq!(restored.len(), 1);
    assert_eq!(snapshot(&real_data(&f)), before);
    assert!(names(&install)
        .iter()
        .all(|n| !n.starts_with(swap::COPY_PREFIX) && !n.starts_with(swap::ASIDE_PREFIX)));
    assert!(!journal_file(&f).exists());
}

#[test]
fn a_crashed_swap_is_put_back_by_a_run_before_it_starts() {
    let _swap = SwapTest::same_volume();
    let f = swap_fixture(vec![writes_and_deletes("nemesis")]);
    let before = snapshot(&real_data(&f));
    crash_after(&f, 6, CopyKind::Kept, "crash");

    let outcome = f
        .run_as("nemesis", &swap_launcher(&f), CaptureMode::Swap)
        .unwrap();

    assert!(outcome.promoted, "{outcome:?}");
    assert_eq!(snapshot(&real_data(&f)), before);
    assert_copy_matches_plan(&f);
}

#[test]
fn a_journal_whose_copy_points_at_a_victim_folder_stops_recovery_and_touches_nothing() {
    let _swap = SwapTest::same_volume();
    let f = swap_fixture(vec![writes_and_deletes("nemesis")]);
    let victim = f._tmp.path().join("victim");
    std::fs::create_dir_all(&victim).unwrap();
    std::fs::write(victim.join("keep.txt"), b"keep").unwrap();
    let mut journal = journal_of(&f, "tamper", CopyKind::Kept);
    journal.copy_dir = victim.clone();
    swap::write_journal(&f.ctx, &journal).unwrap();
    std::fs::rename(real_data(&f), aside_named(&f, "tamper")).unwrap();

    let result = swap::recover_game(&f.ctx, &f.def);

    match result {
        Err(SwapError::Stuck(message)) => {
            assert!(message.contains("copy folder"), "{message}");
        }
        other => panic!("expected the recovery to stop, got {other:?}"),
    }
    assert_eq!(std::fs::read(victim.join("keep.txt")).unwrap(), b"keep");
    assert!(aside_named(&f, "tamper").is_dir());
    assert!(!real_data(&f).exists());
    assert!(journal_file(&f).exists());
}

#[test]
fn a_copy_without_a_matching_marker_stops_recovery_before_anything_moves() {
    let _swap = SwapTest::same_volume();
    let f = swap_fixture(vec![writes_and_deletes("nemesis")]);
    let journal = journal_of(&f, "stranger", CopyKind::Kept);
    swap::write_journal(&f.ctx, &journal).unwrap();
    std::fs::rename(real_data(&f), aside_named(&f, "stranger")).unwrap();
    std::fs::create_dir_all(kept_copy(&f)).unwrap();
    std::fs::write(kept_copy(&f).join("stranger.txt"), b"not ours").unwrap();

    let result = swap::recover_game(&f.ctx, &f.def);

    match result {
        Err(SwapError::Stuck(message)) => {
            assert!(message.contains("marker"), "{message}");
        }
        other => panic!("expected the recovery to stop, got {other:?}"),
    }
    assert!(!real_data(&f).exists(), "the real Data was not moved back");
    assert!(aside_named(&f, "stranger").is_dir());
    assert_eq!(
        std::fs::read(kept_copy(&f).join("stranger.txt")).unwrap(),
        b"not ours"
    );
    assert!(journal_file(&f).exists());
}

#[test]
fn a_version_1_junction_journal_is_refused_by_name_and_nothing_is_changed() {
    let f = fixture(vec![writes_real_data("nemesis")]);
    let before = snapshot(&real_data(&f));
    let path = journal_file(&f);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let old = serde_json::json!({
        "version": 1,
        "game": GAME,
        "store": "steam",
        "instance_id": f.id(),
        "tool": "nemesis",
        "run": "old",
        "started_unix_ms": 0,
        "install_dir": install_dir(&f),
        "real_data": real_data(&f),
        "aside": aside_named(&f, "old"),
        "junction_target": f._tmp.path().join("farm"),
        "deployment_game_dir": f._tmp.path().join("farm"),
    });
    std::fs::write(&path, serde_json::to_vec_pretty(&old).unwrap()).unwrap();

    let result = swap::recover_game(&f.ctx, &f.def);

    match result {
        Err(SwapError::Stuck(message)) => {
            assert!(message.contains("version 1"), "{message}");
            assert!(message.contains("junction"), "{message}");
        }
        other => panic!("expected the recovery to stop, got {other:?}"),
    }
    assert_eq!(snapshot(&real_data(&f)), before);
    assert!(path.exists());
}

#[test]
fn remove_copy_refuses_a_folder_without_the_marker_and_removes_a_kept_copy() {
    let _swap = SwapTest::same_volume();
    let f = swap_fixture(vec![writes_and_deletes("nemesis")]);
    std::fs::create_dir_all(kept_copy(&f)).unwrap();
    std::fs::write(kept_copy(&f).join("stranger.txt"), b"not ours").unwrap();

    let refused = game_tool_copy::remove_kept_copy(&f.ctx, &f.def, f.id());

    assert!(matches!(refused, Err(SwapError::Refused(_))), "{refused:?}");
    assert!(kept_copy(&f).join("stranger.txt").exists());

    std::fs::remove_dir_all(kept_copy(&f)).unwrap();
    let outcome = f
        .run_as("nemesis", &swap_launcher(&f), CaptureMode::Swap)
        .unwrap();
    assert!(outcome.promoted, "{outcome:?}");
    assert!(kept_copy(&f).is_dir());

    let removed = game_tool_copy::remove_kept_copy(&f.ctx, &f.def, f.id()).unwrap();

    assert_eq!(removed, game_tool_copy::RemoveCopy::Removed);
    assert!(!kept_copy(&f).exists());
    assert_eq!(
        game_tool_copy::remove_kept_copy(&f.ctx, &f.def, f.id()).unwrap(),
        game_tool_copy::RemoveCopy::NothingToRemove
    );
}

#[test]
fn deleting_an_instance_removes_its_swap_folder() {
    let _swap = SwapTest::same_volume();
    let f = swap_fixture(vec![writes_and_deletes("nemesis")]);
    let outcome = f
        .run_as("nemesis", &swap_launcher(&f), CaptureMode::Swap)
        .unwrap();
    assert!(outcome.promoted, "{outcome:?}");
    let folder = swap_dir(&f);
    assert!(folder.exists());

    agora_core::game_instance::delete(&f.ctx, f.id()).unwrap();

    assert!(!folder.exists());
}

#[test]
#[ignore = "runs the 32-bit cmd.exe from SysWOW64 on a fake install; run with --ignored"]
fn real_a_32_bit_tool_run_by_swap_promotes_its_write() {
    let _swap = SwapTest::same_volume();
    let f = fixture_with_exe(
        vec![writes_real_data("nemesis")],
        false,
        false,
        r"C:\Windows\SysWOW64\cmd.exe",
    );
    let install = install_dir(&f);
    let before = snapshot(&real_data(&f));
    let launcher = StagingLauncher::new().with_env("AGORA_REAL", install.to_str().unwrap());

    let outcome = f.run_as("nemesis", &launcher, CaptureMode::Swap).unwrap();

    println!("{outcome:#?}");
    assert!(outcome.promoted, "{outcome:?}");
    assert!(outcome.written.contains(&"Data/meshes/x.nif".to_string()));
    assert_eq!(snapshot(&real_data(&f)), before);
}

#[test]
fn a_recovery_with_nothing_pending_changes_nothing() {
    let f = fixture(vec![writes_real_data("nemesis")]);
    let before = snapshot(&real_data(&f));

    assert!(swap::recover_game(&f.ctx, &f.def).unwrap().is_empty());
    assert!(swap::recover_all(&f.ctx).is_empty());
    assert_eq!(snapshot(&real_data(&f)), before);
}

#[test]
fn a_link_run_reports_what_the_tool_wrote_into_the_real_data_folder() {
    // Under link capture the tool writes into the real install, which the farm comparison cannot see.
    let f = fixture(vec![tool(
        "nemesis",
        "Nemesis",
        &[
            "/c",
            "mkdir",
            r"%AGORA_REAL%\Data\meshes",
            "&",
            "echo",
            "nif",
            ">",
            r"%AGORA_REAL%\Data\meshes\from_links.nif",
        ],
    )]);
    let install = install_dir(&f);
    let launcher = StagingLauncher::new().with_env("AGORA_REAL", install.to_str().unwrap());

    let outcome = f.run_as("nemesis", &launcher, CaptureMode::Links).unwrap();

    assert_eq!(outcome.capture, CaptureMethod::Links);
    assert!(
        outcome
            .install_changes
            .created
            .iter()
            .any(|p| p.eq_ignore_ascii_case("Data/meshes/from_links.nif")),
        "{:?}",
        outcome.install_changes
    );
    assert!(outcome.install_changes.unavailable.is_none());
    assert!(real_data(&f).join("meshes").join("from_links.nif").exists());
}

#[test]
fn the_outside_data_watch_reports_a_file_written_at_the_install_root() {
    let f = fixture(vec![tool(
        "nemesis",
        "Nemesis",
        &["/c", "echo", "root", ">", r"%AGORA_REAL%\root_out.txt"],
    )]);
    let install = install_dir(&f);
    let launcher = StagingLauncher::new().with_env("AGORA_REAL", install.to_str().unwrap());

    let outcome = f.run_as("nemesis", &launcher, CaptureMode::Links).unwrap();

    assert!(
        outcome
            .install_changes
            .created
            .iter()
            .any(|p| p == "root_out.txt"),
        "{:?}",
        outcome.install_changes
    );
    assert!(!outcome.install_changes.may_be_incomplete);
}

// ---------------------------------------------------------------------------
// Output checks: a run that exits 0 is still discarded when it misses a required output or
// writes a failure marker (MASTER_SPEC §26.9, slice 4d).
// ---------------------------------------------------------------------------

/// A tool named `id` that runs `args`, with the output checks it declares.
fn checked(
    id: &str,
    args: &[&str],
    required: &[&str],
    markers: Vec<FailureMarker>,
) -> ToolDefinition {
    let mut t = tool(id, "Checked", args);
    t.required_outputs = required.iter().map(|r| r.to_string()).collect();
    t.failure_markers = markers;
    t
}

/// A failure marker for `file`, which must contain `text`.
fn marker(file: &str, text: &str) -> FailureMarker {
    FailureMarker {
        file: RelPath::new(file).unwrap(),
        contains: text.into(),
    }
}

#[test]
fn a_run_that_exits_zero_but_deletes_a_required_output_is_discarded() {
    let f = fixture(vec![checked(
        "checked",
        // cmd skips every `&`-chained command after a false `if`, so the conditional comes last.
        &[
            "/c",
            "echo",
            "made>",
            r"Data\required.txt",
            "&",
            "if",
            "defined",
            "AGORA_TEST_DELETE",
            "del",
            r"Data\required.txt",
        ],
        &["Data/required.txt"],
        Vec::new(),
    )]);
    let launcher = StagingLauncher::new();
    let first = f.run_as("checked", &launcher, CaptureMode::Links).unwrap();
    assert!(first.promoted, "{first:?}");
    assert!(f
        .generation_dir("checked", "1")
        .join("Data/required.txt")
        .is_file());

    let out = f
        .run_as(
            "checked",
            &launcher.with_env("AGORA_TEST_DELETE", "1"),
            CaptureMode::Links,
        )
        .expect("a discarded run is an outcome, not an error");

    assert!(!out.promoted, "{out:?}");
    assert_eq!(out.exit_code, Some(0));
    let reason = out.failure.clone().expect("the outcome says why");
    assert!(
        reason.contains("required output 'Data/required.txt'"),
        "{reason}"
    );
    assert_eq!(out.current.as_deref(), Some("1"));
    assert_eq!(f.generated("checked").unwrap().0, "1");
    assert!(f.generation_dir("checked", "1").is_dir());
    let failed = out.failed_folder.expect("the discarded run is kept");
    assert!(failed.is_dir(), "{}", failed.display());
}

#[test]
fn a_run_that_exits_zero_but_writes_a_failure_marker_is_discarded() {
    let f = fixture(vec![checked(
        "logger",
        &["/c", "echo", "%AGORA_TEST_LOG%", ">", r"Data\PatchLog.txt"],
        &[],
        vec![marker("Data/PatchLog.txt", "Failed to generate behavior")],
    )]);

    let out = f
        .run_as(
            "logger",
            &StagingLauncher::new().with_env("AGORA_TEST_LOG", "Failed to generate behavior"),
            CaptureMode::Links,
        )
        .unwrap();

    assert!(!out.promoted, "{out:?}");
    assert_eq!(out.exit_code, Some(0));
    let reason = out.failure.clone().expect("the outcome says why");
    assert!(reason.contains("Data/PatchLog.txt"), "{reason}");
    assert!(reason.contains("Failed to generate behavior"), "{reason}");
    assert!(f.generated("logger").is_none());
    let failed = out.failed_folder.expect("the discarded run is kept");
    assert!(failed.join("Data/PatchLog.txt").is_file());
}

#[test]
fn a_run_that_keeps_its_required_outputs_and_writes_no_marker_is_promoted() {
    let f = fixture(vec![checked(
        "logger",
        &[
            "/c",
            "echo",
            "made>",
            r"Data\required.txt",
            "&",
            "echo",
            "%AGORA_TEST_LOG%",
            ">",
            r"Data\PatchLog.txt",
        ],
        &["Data/required.txt"],
        vec![marker("Data/PatchLog.txt", "Failed to generate behavior")],
    )]);

    let out = f
        .run_as(
            "logger",
            &StagingLauncher::new().with_env("AGORA_TEST_LOG", "all good"),
            CaptureMode::Links,
        )
        .unwrap();

    assert!(out.promoted, "{out:?}");
    assert_eq!(out.failure, None);
    assert_eq!(out.current.as_deref(), Some("1"));
    let gen = f.generation_dir("logger", "1");
    assert!(gen.join("Data/required.txt").is_file());
    assert!(gen.join("Data/PatchLog.txt").is_file());
}

#[test]
fn a_failure_marker_is_searched_only_in_files_the_run_wrote() {
    // The marker's text is in a mod's file that the run does not write, so the run is promoted.
    let f = fixture(vec![checked(
        "quiet",
        &["/c", "echo", "fine", ">", r"Data\other.txt"],
        &[],
        vec![marker("Data/PatchLog.txt", "Failed to generate behavior")],
    )]);
    f.add_mod(
        "Old log",
        &[("Data/PatchLog.txt", b"Failed to generate behavior\r\n")],
    );

    let out = f
        .run_as("quiet", &StagingLauncher::new(), CaptureMode::Links)
        .unwrap();

    assert!(out.promoted, "{out:?}");
}

#[test]
fn a_nonzero_exit_is_discarded_and_the_outcome_says_which_code() {
    let f = fixture(vec![writes_or_fails("nemesis")]);

    let out = f
        .run_as(
            "nemesis",
            &StagingLauncher::new().with_env("AGORA_TEST_FAIL", "1"),
            CaptureMode::Links,
        )
        .unwrap();

    assert!(!out.promoted, "{out:?}");
    assert_eq!(out.failure.as_deref(), Some("the tool exited with code 3"));
}

// ---------------------------------------------------------------------------
// A tool's own mod is copied for its run (MASTER_SPEC §26.9, slice 4d).
// ---------------------------------------------------------------------------

/// The bytes of the 64-bit `cmd.exe`, which a test puts in a mod as the tool's executable.
fn cmd_exe() -> Vec<u8> {
    std::fs::read(r"C:\Windows\System32\cmd.exe").unwrap()
}

/// `tool` with its executable at `exe`, a path under the runtime root.
fn at_exe(mut tool: ToolDefinition, exe: &str) -> ToolDefinition {
    tool.launch.executable = GamePath::Runtime {
        path: RelPath::new(exe).unwrap(),
    };
    tool
}

/// The id of the layer a content item was added as.
fn content_layer_id(f: &Fixture, item: &str) -> String {
    get_manifest(&f.ctx, f.id())
        .unwrap()
        .layers
        .layers()
        .iter()
        .find_map(|layer| match &layer.source {
            LayerSource::Content { content } if content.as_str() == item => {
                Some(layer.id.as_str().to_string())
            }
            _ => None,
        })
        .expect("the content item is a layer of the instance")
}

#[test]
fn under_links_a_tool_overwrites_a_file_in_its_own_mod_and_the_mod_is_unchanged() {
    let cmd = cmd_exe();
    let f = fixture(vec![at_exe(
        tool(
            "owner",
            "Owner",
            &["/c", "echo", "changed", ">", r"Data\Tool\own.txt"],
        ),
        "Data/Tool/Tool.exe",
    )]);
    let item = f.add_mod(
        "Owner mod",
        &[
            ("Data/Tool/Tool.exe", cmd.as_slice()),
            ("Data/Tool/own.txt", b"original"),
        ],
    );
    let layer = content_layer_id(&f, &item);
    let before = f.content_bytes(&item, "Data/Tool/own.txt");

    let out = f
        .run_as("owner", &StagingLauncher::new(), CaptureMode::Links)
        .unwrap();

    assert!(out.promoted, "{out:?}");
    assert_eq!(out.copied_layer.as_deref(), Some(layer.as_str()));
    assert!(
        out.written.contains(&"Data/Tool/own.txt".to_string()),
        "{:?}",
        out.written
    );
    assert_eq!(
        std::fs::read_to_string(f.generation_dir("owner", "1").join("Data/Tool/own.txt"))
            .unwrap()
            .trim(),
        "changed"
    );
    assert_eq!(f.content_bytes(&item, "Data/Tool/own.txt"), before);
    assert!(names(&f.writable_dir()).is_empty());
    assert!(!f.deployed_game_dir().exists());
}

#[test]
fn under_swap_a_tool_overwrites_a_file_in_its_own_mod_and_the_mod_and_real_data_are_unchanged() {
    let cmd = cmd_exe();
    let f = fixture(vec![at_exe(
        swap_tool(
            "owner",
            "Owner",
            &["/c", "echo", "changed", ">", r"Data\Tool\own.txt"],
        ),
        "Data/Tool/Tool.exe",
    )]);
    let item = f.add_mod(
        "Owner mod",
        &[
            ("Data/Tool/Tool.exe", cmd.as_slice()),
            ("Data/Tool/own.txt", b"original"),
        ],
    );
    let before_mod = f.content_bytes(&item, "Data/Tool/own.txt");
    let before_real = snapshot(&real_data(&f));

    // Under a swap every file is a copy, so the tool's own mod is copied with the rest and no layer is
    // copied separately (MASTER_SPEC §26.9, decision 9).
    let _swap = SwapTest::same_volume();
    let out = f
        .run_as("owner", &StagingLauncher::new(), CaptureMode::Swap)
        .unwrap();

    assert!(out.promoted, "{out:?}");
    assert_eq!(out.capture, CaptureMethod::Swap);
    assert_eq!(out.copied_layer, None);
    assert_eq!(
        std::fs::read_to_string(f.generation_dir("owner", "1").join("Data/Tool/own.txt"))
            .unwrap()
            .trim(),
        "changed"
    );
    assert_eq!(f.content_bytes(&item, "Data/Tool/own.txt"), before_mod);
    assert_eq!(snapshot(&real_data(&f)), before_real);
    assert!(!journal_file(&f).exists());
}

#[test]
fn a_tool_that_overwrites_a_file_in_another_mod_is_still_refused_under_links() {
    let cmd = cmd_exe();
    let f = fixture(vec![at_exe(
        tool(
            "owner",
            "Owner",
            &["/c", "echo", "changed", ">", r"Data\Other\x.txt"],
        ),
        "Data/Tool/Tool.exe",
    )]);
    f.add_mod("Owner mod", &[("Data/Tool/Tool.exe", cmd.as_slice())]);
    let other = f.add_mod("Other mod", &[("Data/Other/x.txt", b"keep")]);
    let before = f.content_bytes(&other, "Data/Other/x.txt");

    // Whether the content store's ACL refuses the write (the run is not promoted) or lets it through
    // (the change check refuses it), the other mod's object keeps its bytes.
    match f.run_as("owner", &StagingLauncher::new(), CaptureMode::Links) {
        Ok(out) => assert!(!out.promoted, "{out:?}"),
        Err(ToolError::LinkedFileChanged { paths, .. }) => {
            assert_eq!(paths, vec!["Data/Other/x.txt".to_string()]);
        }
        Err(other) => panic!("{other:?}"),
    }
    assert!(f.generated("owner").is_none());
    assert_eq!(f.content_bytes(&other, "Data/Other/x.txt"), before);
}

#[test]
fn the_executable_is_copied_from_the_layer_that_wins_its_path() {
    let cmd = cmd_exe();
    let f = fixture(vec![at_exe(
        tool(
            "owner",
            "Owner",
            &["/c", "echo", "changed", ">", r"Data\Tool\own.txt"],
        ),
        "Data/Tool/Tool.exe",
    )]);
    let lower = f.add_mod(
        "Lower",
        &[
            ("Data/Tool/Tool.exe", cmd.as_slice()),
            ("Data/Tool/own.txt", b"lower"),
        ],
    );
    // Added later, so it sits above the lower mod and wins the executable's path.
    let upper = f.add_mod(
        "Upper",
        &[
            ("Data/Tool/Tool.exe", cmd.as_slice()),
            ("Data/Tool/own.txt", b"upper"),
        ],
    );
    let winner = content_layer_id(&f, &upper);
    let loser = content_layer_id(&f, &lower);
    assert_eq!(
        planned(&f.plan(DeployMode::Links), "Data/Tool/Tool.exe")
            .unwrap()
            .layer_id,
        winner
    );

    let out = f
        .run_as("owner", &StagingLauncher::new(), CaptureMode::Links)
        .unwrap();

    assert!(out.promoted, "{out:?}");
    assert_eq!(out.copied_layer.as_deref(), Some(winner.as_str()));
    assert_ne!(out.copied_layer.as_deref(), Some(loser.as_str()));
}

#[test]
fn under_the_vfs_no_content_layer_is_copied_for_a_run() {
    // The VFS copies up on write, so the fake launcher's VFS route copies nothing for the tool's mod.
    let cmd = cmd_exe();
    let f = fixture(vec![at_exe(
        writes_or_fails("nemesis"),
        "Data/Tool/Tool.exe",
    )]);
    f.add_mod("Owner mod", &[("Data/Tool/Tool.exe", cmd.as_slice())]);

    let out = f.run("nemesis", &StagingLauncher::new()).unwrap();

    assert_eq!(out.capture, CaptureMethod::Vfs);
    assert_eq!(out.copied_layer, None);
}

#[test]
fn a_run_that_drops_a_required_output_from_the_game_is_discarded_even_though_it_exits_zero() {
    // The shape of the real incident: generation 1 has the behaviour file; the next run exits 0 and
    // writes only its logs. Promotion would leave the game without the file, so the run is discarded.
    // cmd skips the `&`-chained commands after a false `if`, so the conditional comes last.
    let f = fixture(vec![checked(
        "behaviour",
        &[
            "/c",
            "echo",
            "%AGORA_TEST_LOG%",
            ">",
            r"Data\PatchLog.txt",
            "&",
            "if",
            "not",
            "defined",
            "AGORA_TEST_LOGS_ONLY",
            "echo",
            "made>",
            r"Data\0_master.hkx",
        ],
        &["Data/0_master.hkx"],
        Vec::new(),
    )]);
    let first = f
        .run_as(
            "behaviour",
            &StagingLauncher::new().with_env("AGORA_TEST_LOG", "all good"),
            CaptureMode::Links,
        )
        .unwrap();
    assert!(first.promoted, "{first:?}");
    assert!(f
        .generation_dir("behaviour", "1")
        .join("Data/0_master.hkx")
        .is_file());

    let out = f
        .run_as(
            "behaviour",
            &StagingLauncher::new()
                .with_env("AGORA_TEST_LOG", "all good")
                .with_env("AGORA_TEST_LOGS_ONLY", "1"),
            CaptureMode::Links,
        )
        .unwrap();

    assert!(!out.promoted, "{out:?}");
    assert_eq!(out.exit_code, Some(0));
    let reason = out.failure.clone().expect("the outcome says why");
    assert!(
        reason.contains("required output 'Data/0_master.hkx'"),
        "{reason}"
    );
    assert_eq!(f.generated("behaviour").unwrap().0, "1");
    assert!(f
        .generation_dir("behaviour", "1")
        .join("Data/0_master.hkx")
        .is_file());
}

#[test]
fn a_required_output_a_content_mod_provides_passes_though_the_run_does_not_write_it() {
    let f = fixture(vec![checked(
        "logger",
        &["/c", "echo", "log", ">", r"Data\log.txt"],
        &["Data/mod_provided.txt"],
        Vec::new(),
    )]);
    f.add_mod("Provider", &[("Data/mod_provided.txt", b"from the mod")]);

    let out = f
        .run_as("logger", &StagingLauncher::new(), CaptureMode::Links)
        .unwrap();

    assert!(out.promoted, "{out:?}");
    assert_eq!(out.failure, None);
    assert_eq!(out.current.as_deref(), Some("1"));
}

#[test]
fn a_run_that_deletes_a_required_output_a_mod_provided_is_discarded() {
    let f = fixture(vec![checked(
        "deleter",
        &[
            "/c",
            "echo",
            "log>",
            r"Data\log.txt",
            "&",
            "del",
            r"Data\mod_provided.txt",
        ],
        &["Data/mod_provided.txt"],
        Vec::new(),
    )]);
    f.add_mod("Provider", &[("Data/mod_provided.txt", b"from the mod")]);

    let out = f
        .run_as("deleter", &StagingLauncher::new(), CaptureMode::Links)
        .unwrap();

    assert!(!out.promoted, "{out:?}");
    let reason = out.failure.clone().expect("the outcome says why");
    assert!(
        reason.contains("required output 'Data/mod_provided.txt'"),
        "{reason}"
    );
    assert!(f.generated("deleter").is_none());
}

/// The machine type in a PE file's header (`IMAGE_FILE_HEADER.Machine`), read without running it.
fn pe_machine(path: &Path) -> std::io::Result<u16> {
    use std::io::{Error, Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    let mut dos = [0u8; 64];
    file.read_exact(&mut dos)?;
    if &dos[..2] != b"MZ" {
        return Err(Error::other("not an MZ executable"));
    }
    let header_at = u32::from_le_bytes([dos[60], dos[61], dos[62], dos[63]]);
    file.seek(SeekFrom::Start(u64::from(header_at)))?;
    let mut head = [0u8; 6];
    file.read_exact(&mut head)?;
    if &head[..4] != b"PE\0\0" {
        return Err(Error::other("no PE signature"));
    }
    Ok(u16::from_le_bytes([head[4], head[5]]))
}

/// Read only: the real Nemesis 0.84 executable is a 32-bit program, so under the VFS it needs the
/// 32-bit DLL, and the locator names the x86 folder beside the 64-bit DLL from `AGORA_VFS_DLL`. The
/// executable is never run.
#[test]
#[ignore = "reads the PE headers of the real Nemesis exe and the DLLs (never runs them): set AGORA_VFS_DLL and AGORA_VFS_DLL_X86"]
fn the_real_nemesis_exe_is_32_bit_and_the_locator_picks_the_x86_dll() {
    const I386: u16 = 0x014c;
    const AMD64: u16 = 0x8664;
    // Another test in this file sets AGORA_VFS_DLL_X86 to a missing path while it runs.
    let _lock = X86_DLL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let exe = Path::new(
        r"D:\Skyrim-MO2-Salvage\MO2\mods\090 - Nemesis Unlimited Behavior Engine 0.84\Nemesis_Engine\Nemesis Unlimited Behavior Engine.exe",
    );
    assert!(exe.is_file(), "{} does not exist", exe.display());
    let machine = pe_machine(exe).expect("read the executable's PE header");
    assert_eq!(
        machine, I386,
        "Nemesis should be 32-bit, got machine {machine:#x}"
    );

    let dll64 = PathBuf::from(
        std::env::var_os("AGORA_VFS_DLL").expect("set AGORA_VFS_DLL to the 64-bit agora_vfs.dll"),
    );
    let x86 = agora_core::game_launch::locate_dll_x86(&dll64);
    assert_eq!(
        x86,
        PathBuf::from(
            std::env::var_os("AGORA_VFS_DLL_X86")
                .expect("set AGORA_VFS_DLL_X86 to the 32-bit agora_vfs.dll")
        ),
        "the locator should pick the 32-bit DLL the environment names"
    );
    assert!(x86.is_file(), "{} does not exist", x86.display());
    assert_eq!(
        pe_machine(&x86).expect("read the 32-bit DLL's header"),
        I386
    );
    assert_eq!(
        pe_machine(&dll64).expect("read the 64-bit DLL's header"),
        AMD64
    );
}
