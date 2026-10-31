//! How many times one launch preparation runs the planner (MASTER_SPEC §26.5).
//!
//! The planner's call counter is process-wide, so this file holds one test only: nothing else in
//! the binary runs beside it.

use std::path::PathBuf;
use std::sync::Arc;

use agora_core::ctx::CoreContext;
use agora_core::game_base::BaseMode;
use agora_core::game_deploy::{add_content, DeployMode, PLAN_CALLS};
use agora_core::game_discovery::{DiscoveredInstall, DiscoveryReport, InstallCapabilities};
use agora_core::game_instance::{create, prepare_launch_with, GameInstanceRecord, LaunchOptions};
use agora_core::game_launch::{LaunchError, LaunchedGame, Launcher, PreparedLaunch};
use agora_core::game_registry::{
    GameRegistry, IdentifiedInstall, PackageSource, RuntimeResolution,
};
use agora_game_api::{
    DeploymentStrategy, GameDefinition, GameId, GamePackage, GamePath, InstallId, InstallKind,
    LaunchRecipe, LaunchValue, PackageDefinition, PluginListRule, RelPath, RuntimeIdentity,
    StoreId, StoreIdentifier, UserDataLocation, UserFileMapping, UserFileStrategy,
};
use std::sync::atomic::Ordering;
use tempfile::TempDir;

struct Package(PackageDefinition);

impl GamePackage for Package {
    fn definition(&self) -> &PackageDefinition {
        &self.0
    }
}

/// Says whether the virtual file system is available. A launch is only prepared, never started.
struct Launches {
    vfs: bool,
}

impl Launcher for Launches {
    fn locate_vfs_dll(&self) -> Result<PathBuf, String> {
        if self.vfs {
            Ok(PathBuf::from("agora_vfs.dll"))
        } else {
            Err("no virtual file system in this test".into())
        }
    }

    fn launch(&self, _prepared: &PreparedLaunch) -> Result<LaunchedGame, LaunchError> {
        panic!("the test must not start a game");
    }
}

fn definition(plugin_list: u8) -> GameDefinition {
    let mut def = GameDefinition {
        mo2_game_name: None,
        id: GameId::new("count-game").unwrap(),
        name: "Count Game".into(),
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
        linked_archive_patterns: vec![],
        declared_writes: vec!["writeable_base.txt".into()],
        excluded_paths: vec![],
        plugin_list: None,
        runtime_files: Vec::new(),
        save_location: Vec::new(),
        launch_alternatives: Vec::new(),
        content_layout: None,
        copy_patterns: Vec::new(),
    };
    if plugin_list > 0 {
        def.plugin_list = Some(PluginListRule {
            user_file: RelPath::new("user/Plugins.txt").unwrap(),
            plugin_folder: RelPath::new("Data").unwrap(),
            patterns: vec!["*.esp".into(), "*.esm".into()],
            active_prefix: String::new(),
            header: vec![],
            semantics: (plugin_list == 2).then(|| "creation_engine".to_string()),
            implicit: vec![],
            implicit_list_file: None,
        });
        def.user_files = vec![UserFileMapping::new(
            GamePath::UserData {
                location: UserDataLocation::LocalAppData,
                path: RelPath::new("Count Game/Plugins.txt").unwrap(),
            },
            RelPath::new("user/Plugins.txt").unwrap(),
            UserFileStrategy::JournaledSwap,
        )
        .with_stores(vec![StoreId::steam()])];
    }
    def
}

/// Prepare a launch twice (the first builds the deployment, the second finds it up to date) and
/// return how many plans each preparation made.
fn plans_per_preparation(plugin_list: u8, links: bool, vfs: bool) -> (usize, usize) {
    let tmp = TempDir::new().unwrap();
    let def = definition(plugin_list);
    let registry = {
        let mut builder = GameRegistry::builder();
        let pkg = PackageDefinition {
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
                Arc::new(Package(pkg)),
            )
            .expect("register test package");
        Arc::new(builder.build())
    };
    let ctx = CoreContext::for_testing(tmp.path().join("app_data"));
    agora_core::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();
    let ctx = ctx.with_games(registry);

    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(install_dir.join("Data")).unwrap();
    std::fs::write(install_dir.join("Game.exe"), b"fake game binary").unwrap();
    std::fs::write(install_dir.join("writeable_base.txt"), b"writable").unwrap();
    std::fs::write(install_dir.join("Data").join("base.bin"), b"base").unwrap();
    let detector = agora_core::game_discovery::volume::VolumeDetector::new();
    let install = IdentifiedInstall {
        game: def.id.clone(),
        install_id: InstallId::new("steam:12345").unwrap(),
        discovered: DiscoveredInstall {
            store: StoreId::new("steam").unwrap(),
            product: "12345".into(),
            name: "Count Game".into(),
            kind: InstallKind::BaseGame,
            parent_product: None,
            location: install_dir.clone(),
            store_version: Some("1.0.0".into()),
            store_build: None,
            executables: vec!["Game.exe".into()],
            capabilities: InstallCapabilities {
                executables_readable: true,
                accepts_new_files: true,
                relocatable: true,
            },
            volume: detector.get_volume_info(&install_dir),
        },
        add_ons: vec![],
        runtime: RuntimeResolution::Identified {
            runtime: RuntimeIdentity {
                game: def.id.clone(),
                store: StoreId::new("steam").unwrap(),
                version: "1.0.0".into(),
                build: None,
            },
            source: "executable".into(),
        },
    };
    let inst: GameInstanceRecord = create(
        &ctx,
        &install,
        &def,
        "Count",
        None,
        BaseMode::Linked,
        &|_| {},
    )
    .expect("create instance");

    for n in 0..2 {
        let mod_dir = TempDir::new().unwrap();
        std::fs::create_dir_all(mod_dir.path().join("Data")).unwrap();
        std::fs::write(
            mod_dir.path().join("Data").join(format!("mod{n}.txt")),
            format!("mod {n}"),
        )
        .unwrap();
        let outcome =
            agora_core::content_store::add_folder(&ctx, mod_dir.path(), Some(&format!("Mod{n}")))
                .unwrap();
        add_content(&ctx, &inst.instance_id, &outcome.item().item_id, None, None).unwrap();
    }

    let launcher = Launches { vfs };
    let discover = || DiscoveryReport {
        installs: vec![],
        warnings: vec![],
    };
    let options = LaunchOptions {
        deployment: links.then_some(DeployMode::Links),
        ..Default::default()
    };
    let mut counts = Vec::new();
    for _ in 0..2 {
        let before = PLAN_CALLS.load(Ordering::Relaxed);
        prepare_launch_with(&ctx, &inst.instance_id, &def, options, &discover, &launcher)
            .expect("prepare launch");
        counts.push(PLAN_CALLS.load(Ordering::Relaxed) - before);
    }
    (counts[0], counts[1])
}

#[test]
fn one_launch_preparation_plans_once_except_where_the_load_order_needs_the_links_view() {
    // Linked deployment, no plugin list: the deployment's plan is the only plan.
    assert_eq!(
        plans_per_preparation(0, true, false),
        (1, 1),
        "links, no plugin list"
    );
    // A plugin list without the Creation Engine rules reads no plan for its load order.
    assert_eq!(
        plans_per_preparation(1, true, false),
        (1, 1),
        "links, plain plugin list"
    );
    // Creation Engine rules read the installed plugins from a plan: the deployment's own, when linked.
    assert_eq!(
        plans_per_preparation(2, true, false),
        (1, 1),
        "links, creation engine"
    );
    // Virtual deployment without a plugin list: the one plan.
    assert_eq!(
        plans_per_preparation(0, false, true),
        (1, 1),
        "virtual, no plugin list"
    );
    // Virtual deployment with the Creation Engine rules: the deployment leaves the writable layer
    // out (the virtual file system shows it on top), but the load order reads the Links view, which
    // includes it. That is one more plan, the only one beyond the deployment's.
    assert_eq!(
        plans_per_preparation(2, false, true),
        (2, 2),
        "virtual, creation engine"
    );
}
