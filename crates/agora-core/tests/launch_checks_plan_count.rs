//! How many deployment plans the launch checks make (`games instance check` and the instance
//! Overview, both through `launch_checks`).
//!
//! The framework check plans in the instance's own mode, and the plugin load order and every tool's
//! output judge one Links plan. That plan is made once for the whole check: before, each generated
//! layer planned again, so an instance with N tool outputs planned 2 + N times. The planner's
//! call counter is process-wide, so this file holds one test only: nothing else in the binary runs
//! beside it.

#![cfg(windows)]

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use agora_core::ctx::CoreContext;
use agora_core::event_sink::CancellationToken;
use agora_core::game_base::BaseMode;
use agora_core::game_deploy::PLAN_CALLS;
use agora_core::game_discovery::{DiscoveredInstall, InstallCapabilities};
use agora_core::game_instance::{create, launch_checks, GameInstanceRecord};
use agora_core::game_launch::{LaunchError, LaunchedGame, Launcher, PreparedLaunch};
use agora_core::game_load_order;
use agora_core::game_registry::{
    GameRegistry, IdentifiedInstall, PackageSource, RuntimeResolution,
};
use agora_core::game_tools::{self, CaptureMode};
use agora_game_api::{
    DeploymentStrategy, GameDefinition, GameId, GamePackage, GamePath, InstallId, InstallKind,
    LaunchRecipe, LaunchValue, PackageDefinition, PluginListRule, RelPath, RuntimeIdentity,
    StoreId, StoreIdentifier, ToolDefinition, ToolId, UserDataLocation, UserFileMapping,
    UserFileStrategy,
};
use tempfile::TempDir;

const GAME: &str = "count-game";

struct TestPackage(PackageDefinition);

impl GamePackage for TestPackage {
    fn definition(&self) -> &PackageDefinition {
        &self.0
    }
}

/// Starts a tool with its staging folder as its working folder, as the fake VFS would.
struct StagingLauncher;

impl Launcher for StagingLauncher {
    fn locate_vfs_dll(&self) -> Result<PathBuf, String> {
        Ok(PathBuf::from(r"C:\agora\agora_vfs.dll"))
    }

    fn launch(&self, prepared: &PreparedLaunch) -> Result<LaunchedGame, LaunchError> {
        let mut plain = prepared.clone();
        if let Some(vfs) = &prepared.vfs {
            plain.resolved.cwd = vfs.upper.clone();
        }
        plain.vfs = None;
        agora_core::game_launch::launch(&plain)
    }
}

fn literal_args(args: &[&str]) -> Vec<LaunchValue> {
    args.iter()
        .map(|a| LaunchValue::Literal { value: (*a).into() })
        .collect()
}

/// A tool that writes `Data\<name>.txt` into its staging folder.
fn writes_a_file(id: &str, file: &str) -> ToolDefinition {
    ToolDefinition {
        output_patterns: Vec::new(),
        id: ToolId::new(id).unwrap(),
        game: GameId::new(GAME).unwrap(),
        name: id.into(),
        launch: LaunchRecipe {
            executable: GamePath::Runtime {
                path: RelPath::new("Game.exe").unwrap(),
            },
            arguments: literal_args(&[
                "/c",
                "mkdir",
                "Data",
                "&",
                "echo",
                "made>",
                &format!(r"Data\{file}.txt"),
            ]),
            environment: Default::default(),
            working_directory: GamePath::Runtime {
                path: RelPath::default(),
            },
        },
        relevant_settings: Vec::new(),
        after_tools: Vec::new(),
        conflicts_with: Vec::new(),
        uses_install_path: false,
        required_outputs: Vec::new(),
        failure_markers: Vec::new(),
        required_magic: Vec::new(),
    }
}

/// The test game: a plugin list (so the load order is checked), and the given tools.
fn definition(tools: &[ToolDefinition]) -> GameDefinition {
    GameDefinition {
        mo2_game_name: None,
        id: GameId::new(GAME).unwrap(),
        name: "Count Game".into(),
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
        user_files: vec![UserFileMapping::new(
            GamePath::UserData {
                location: UserDataLocation::LocalAppData,
                path: RelPath::new("Count Game/Plugins.txt").unwrap(),
            },
            RelPath::new("user/Plugins.txt").unwrap(),
            UserFileStrategy::JournaledSwap,
        )
        .with_stores(vec![StoreId::steam()])],
        save_paths: vec![],
        linked_archive_patterns: vec![],
        declared_writes: vec![],
        excluded_paths: vec![],
        plugin_list: Some(PluginListRule {
            user_file: RelPath::new("user/Plugins.txt").unwrap(),
            plugin_folder: RelPath::new("Data").unwrap(),
            patterns: vec!["*.esm".into(), "*.esl".into(), "*.esp".into()],
            active_prefix: "*".into(),
            header: vec![],
            // The Creation Engine rules: only an engine game's load order reads a deployment plan.
            semantics: Some("creation_engine".to_string()),
            implicit: Vec::new(),
            implicit_list_file: None,
        }),
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

fn install_of(dir: &std::path::Path) -> IdentifiedInstall {
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
        name: "Count Game".into(),
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

#[test]
fn launch_checks_plan_the_instance_twice_however_many_tool_outputs_it_has() {
    let tools = vec![
        writes_a_file("nemesis", "nemesis"),
        writes_a_file("bodyslide", "bodyslide"),
    ];
    let tmp = TempDir::new().unwrap();
    let def = definition(&tools);
    let ctx = CoreContext::for_testing(tmp.path().join("app_data"));
    agora_core::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();
    let ctx = ctx.with_games(registry(&def, tools.clone()));
    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(install_dir.join("Data")).unwrap();
    std::fs::copy(r"C:\Windows\System32\cmd.exe", install_dir.join("Game.exe")).unwrap();
    std::fs::write(install_dir.join("Data").join("Base.esm"), b"BASE").unwrap();
    let inst: GameInstanceRecord = create(
        &ctx,
        &install_of(&install_dir),
        &def,
        "Count",
        None,
        BaseMode::Linked,
        &|_| {},
    )
    .expect("create instance");
    let id = inst.instance_id.clone();

    // Two tool outputs, so two generated layers, each with recorded inputs.
    for tool in &tools {
        let outcome = game_tools::run(
            &ctx,
            &id,
            &def,
            &tool.id,
            CaptureMode::Auto,
            false,
            &StagingLauncher,
            &CancellationToken::new(),
        )
        .expect("the tool runs");
        assert!(outcome.promoted, "{outcome:?}");
    }

    let before = PLAN_CALLS.load(Ordering::Relaxed);
    let checks = launch_checks(&ctx, &inst, &def).expect("the launch checks run");
    let planned = PLAN_CALLS.load(Ordering::Relaxed) - before;
    // One plan for the framework check (the instance's own mode), and one Links plan shared by the
    // load order and both tool outputs.
    assert_eq!(planned, 2, "launch_checks planned {planned} times");

    // The same findings as the separate calls make.
    let order = game_load_order::check(&ctx, &id, &def).expect("the load order checks");
    assert_eq!(checks.load_order.len(), order.len());
    let outputs = game_tools::output_findings(&ctx, &id, &def).expect("the outputs check");
    assert_eq!(checks.outputs.len(), outputs.len());
}
