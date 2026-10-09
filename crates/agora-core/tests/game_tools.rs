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
use agora_core::game_tools::{
    self, CaptureMethod, CaptureMode, OutputStatus, RunOutcome, ToolError,
};
use agora_core::game_user_files::{record_process, swap_in};
use agora_core::process_identity;
use agora_game_api::{
    BaseReference, DeploymentStrategy, GameDefinition, GameId, GamePackage, GamePath,
    InputFingerprint, InstallId, InstallKind, LaunchRecipe, LaunchValue, LayerSource,
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
fn auto_captures_from_links_when_the_vfs_cannot_start_and_says_why() {
    let _dll = EnvVar::set("AGORA_VFS_DLL", r"C:\agora-missing\agora_vfs.dll");
    let f = fixture(vec![writes_or_fails("nemesis")]);

    let out = f
        .run_as(
            "nemesis",
            &agora_core::game_launch::SystemLauncher,
            CaptureMode::Auto,
        )
        .expect("auto falls back to links");

    assert!(out.promoted, "{out:?}");
    assert_eq!(out.capture, CaptureMethod::Links);
    let reason = out.capture_reason.expect("the fallback says why");
    assert!(
        reason.contains("the virtual file system could not start")
            && reason.contains("AGORA_VFS_DLL points at"),
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

#[test]
#[ignore = "injects agora_vfs.dll: cargo build -p agora-vfs, then set AGORA_VFS_DLL"]
fn real_injection_a_32_bit_tool_is_captured_from_links_under_auto() {
    // The game's own executable is the 32-bit cmd.exe, so agora_vfs.dll (64-bit) cannot load into it.
    let f = fixture_with_exe(
        vec![tool("wow", "Wow", &["/c", "echo", "x", ">", "out.txt"])],
        false,
        false,
        r"C:\Windows\SysWOW64\cmd.exe",
    );
    let launcher = RealLauncher::new(&[]);

    let out = f
        .run_as("wow", &launcher, CaptureMode::Auto)
        .expect("a 32-bit tool runs by link capture");
    println!("run outcome: {out:?}");
    assert_eq!(out.capture, CaptureMethod::Links, "{out:?}");
    let reason = out.capture_reason.clone().unwrap_or_default();
    assert!(reason.contains("32-bit"), "{reason}");
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

// ---------------------------------------------------------------------------
// The swap and the real install (MASTER_SPEC §26.9, slice 4c). Every test uses a fake install in a
// temp folder. `AGORA_REAL` names that folder for the tool, so it can write where Nemesis writes.
// ---------------------------------------------------------------------------

use std::collections::BTreeMap;
use std::process::{Command, Stdio};
use std::time::Duration;

use agora_core::game_tool_swap::{self as swap, SwapError, SwapJournal};

/// A tool that works on the real install: the same as [`tool`], with `uses_install_path` set.
fn swap_tool(id: &str, name: &str, args: &[&str]) -> ToolDefinition {
    ToolDefinition {
        uses_install_path: true,
        ..tool(id, name, args)
    }
}

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

/// A journal for a swap that a test sets up by hand, pointing at the instance's farm.
fn journal_for(f: &Fixture, run: &str, aside: &Path, farm_data: &Path) -> SwapJournal {
    SwapJournal {
        version: swap::JOURNAL_VERSION,
        game: GAME.into(),
        store: "steam".into(),
        instance_id: f.id().into(),
        tool: "nemesis".into(),
        run: run.into(),
        started_unix_ms: 0,
        install_dir: install_dir(f),
        real_data: real_data(f),
        aside: aside.to_path_buf(),
        junction_target: farm_data.to_path_buf(),
        deployment_game_dir: f.deployed_game_dir(),
    }
}

/// Simulates a crash in the middle of a swap: the real `Data` is renamed aside, a junction to the
/// farm stands in its place, and the journal is on disk.
fn simulate_crashed_swap(f: &Fixture) -> SwapJournal {
    // A real deployment, as a crashed run leaves behind: the farm has its record and its links.
    agora_core::game_deploy::deploy(&f.ctx, f.id(), &f.def, DeployMode::Links).unwrap();
    let farm_data = f.deployed_game_dir().join("Data");
    std::fs::create_dir_all(&farm_data).unwrap();
    let aside = aside_named(f, "crash");
    let journal = journal_for(f, "crash", &aside, &farm_data);
    swap::write_journal(&f.ctx, &journal).unwrap();
    std::fs::rename(real_data(f), &aside).unwrap();
    junction::create(&farm_data, real_data(f)).unwrap();
    journal
}

#[test]
fn a_swap_run_restores_the_real_data_and_its_writes_through_the_link_are_captured() {
    let f = fixture(vec![writes_real_data("nemesis")]);
    let install = install_dir(&f);
    let before = snapshot(&real_data(&f));
    let launcher = StagingLauncher::new().with_env("AGORA_REAL", install.to_str().unwrap());

    let outcome = f.run_as("nemesis", &launcher, CaptureMode::Swap).unwrap();

    assert!(outcome.promoted, "{outcome:?}");
    assert_eq!(outcome.capture, CaptureMethod::Swap);
    assert!(outcome.written.contains(&"Data/meshes/x.nif".to_string()));
    // The real Data is byte-identical, has no link, and never received the file.
    assert_eq!(snapshot(&real_data(&f)), before);
    assert!(!swap::is_reparse_point(&real_data(&f)));
    assert!(real_data(&f).is_dir());
    assert!(!real_data(&f).join("meshes").join("x.nif").exists());
    assert!(names(&install)
        .iter()
        .all(|n| !n.starts_with(swap::ASIDE_PREFIX)));
    assert!(!journal_file(&f).exists());
    // The output is the promoted generation.
    let (generation, _) = f.generated("nemesis").expect("a generated layer");
    assert!(f
        .generation_dir("nemesis", &generation)
        .join("Data")
        .join("meshes")
        .join("x.nif")
        .exists());
}

#[test]
fn a_failing_swap_run_is_restored_and_discarded() {
    let f = fixture(vec![swap_tool(
        "nemesis",
        "Nemesis",
        &[
            "/c",
            "echo",
            "bad",
            ">",
            r"%AGORA_REAL%\Data\bad.txt",
            "&",
            "exit",
            "/b",
            "3",
        ],
    )]);
    let install = install_dir(&f);
    let before = snapshot(&real_data(&f));
    let launcher = StagingLauncher::new().with_env("AGORA_REAL", install.to_str().unwrap());

    let outcome = f.run_as("nemesis", &launcher, CaptureMode::Swap).unwrap();

    assert!(!outcome.promoted, "{outcome:?}");
    assert_eq!(outcome.exit_code, Some(3));
    assert!(outcome.failed_folder.is_some());
    assert_eq!(snapshot(&real_data(&f)), before);
    assert!(!swap::is_reparse_point(&real_data(&f)));
    assert!(!journal_file(&f).exists());
    assert!(f.generated("nemesis").is_none());
}

#[test]
fn a_swap_is_refused_while_a_program_runs_from_the_install() {
    let f = fixture(vec![writes_real_data("nemesis")]);
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
    assert!(!f.deployed_game_dir().exists());
}

#[test]
fn a_swap_is_refused_when_the_real_data_is_already_a_link() {
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

    let result = swap::preflight(&install, &f.deployed_game_dir(), &real_data(&f), &aside);

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
fn auto_refuses_a_tool_that_works_on_the_real_install_before_anything_is_deployed() {
    let f = fixture(vec![writes_real_data("nemesis")]);

    let result = f.run_as("nemesis", &StagingLauncher::new(), CaptureMode::Auto);

    assert!(
        matches!(result, Err(ToolError::InstallPathNeedsSwap { .. })),
        "{result:?}"
    );
    assert!(!f.deployed_game_dir().exists());
    assert!(!journal_file(&f).exists());
}

#[test]
fn a_crashed_swap_is_put_back_by_the_next_check() {
    let f = fixture(vec![writes_real_data("nemesis")]);
    let install = install_dir(&f);
    let before = snapshot(&real_data(&f));
    simulate_crashed_swap(&f);
    assert!(swap::is_reparse_point(&real_data(&f)));

    let restored = swap::recover_game(&f.ctx, &f.def).unwrap();

    assert_eq!(restored.len(), 1);
    assert!(!swap::is_reparse_point(&real_data(&f)));
    assert!(real_data(&f).is_dir());
    assert_eq!(snapshot(&real_data(&f)), before);
    assert!(names(&install)
        .iter()
        .all(|n| !n.starts_with(swap::ASIDE_PREFIX)));
    assert!(!journal_file(&f).exists());
}

#[test]
fn a_crashed_swap_is_put_back_by_a_run_before_it_starts() {
    let f = fixture(vec![writes_real_data("nemesis")]);
    let install = install_dir(&f);
    let before = snapshot(&real_data(&f));
    simulate_crashed_swap(&f);
    let launcher = StagingLauncher::new().with_env("AGORA_REAL", install.to_str().unwrap());

    let outcome = f.run_as("nemesis", &launcher, CaptureMode::Swap).unwrap();

    assert!(outcome.promoted, "{outcome:?}");
    assert_eq!(snapshot(&real_data(&f)), before);
    assert!(!swap::is_reparse_point(&real_data(&f)));
}

#[test]
fn a_journal_with_a_real_data_folder_and_an_aside_stops_and_touches_neither() {
    let f = fixture(vec![writes_real_data("nemesis")]);
    let aside = aside_named(&f, "both");
    std::fs::create_dir_all(&aside).unwrap();
    std::fs::write(aside.join("aside.txt"), b"aside").unwrap();
    let farm_data = f.deployed_game_dir().join("Data");
    swap::write_journal(&f.ctx, &journal_for(&f, "both", &aside, &farm_data)).unwrap();
    let real_before = snapshot(&real_data(&f));
    let aside_before = snapshot(&aside);

    let result = swap::recover_game(&f.ctx, &f.def);

    match result {
        Err(SwapError::Stuck(message)) => {
            assert!(message.contains("both"), "{message}");
            assert!(message.contains("nothing was merged"), "{message}");
        }
        other => panic!("expected the recovery to stop, got {other:?}"),
    }
    assert_eq!(snapshot(&real_data(&f)), real_before);
    assert_eq!(snapshot(&aside), aside_before);
    assert!(journal_file(&f).exists());
}

#[test]
fn a_junction_pointing_somewhere_else_stops_recovery_and_is_left_alone() {
    let f = fixture(vec![writes_real_data("nemesis")]);
    let install = install_dir(&f);
    let elsewhere = f._tmp.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let aside = aside_named(&f, "elsewhere");
    let farm_data = f.deployed_game_dir().join("Data");
    std::fs::create_dir_all(&farm_data).unwrap();
    swap::write_journal(&f.ctx, &journal_for(&f, "elsewhere", &aside, &farm_data)).unwrap();
    std::fs::rename(real_data(&f), &aside).unwrap();
    junction::create(&elsewhere, real_data(&f)).unwrap();
    let aside_before = snapshot(&aside);

    let result = swap::recover_game(&f.ctx, &f.def);

    match result {
        Err(SwapError::Stuck(message)) => {
            assert!(message.contains("not the instance's farm"), "{message}");
        }
        other => panic!("expected the recovery to stop, got {other:?}"),
    }
    assert!(swap::is_reparse_point(&real_data(&f)));
    assert!(swap::same_path(
        &junction::get_target(real_data(&f)).unwrap(),
        &elsewhere
    ));
    assert_eq!(snapshot(&aside), aside_before);
    assert!(aside.is_dir());
    assert!(names(&install)
        .iter()
        .any(|n| n.starts_with(swap::ASIDE_PREFIX)));
    assert!(journal_file(&f).exists());
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

/// A 32-bit tool, run by swap, promotes its write: the 32-bit cmd cannot load the 64-bit VFS DLL, so
/// this is the path a real 32-bit tool takes. A real program on a fake install, so it is ignored by
/// default.
#[test]
#[ignore = "runs the 32-bit cmd.exe from SysWOW64 on a fake install; run with --ignored"]
fn real_a_32_bit_tool_run_by_swap_promotes_its_write() {
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
    assert!(!swap::is_reparse_point(&real_data(&f)));
}

#[test]
fn a_recovery_with_nothing_pending_changes_nothing() {
    let f = fixture(vec![writes_real_data("nemesis")]);
    let before = snapshot(&real_data(&f));

    assert!(swap::recover_game(&f.ctx, &f.def).unwrap().is_empty());
    assert!(swap::recover_all(&f.ctx).is_empty());
    assert_eq!(snapshot(&real_data(&f)), before);
}
