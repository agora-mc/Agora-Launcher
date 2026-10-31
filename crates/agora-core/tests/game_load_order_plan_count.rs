//! How many times the plugin commands run the planner (MASTER_SPEC §26.6).
//!
//! The desktop's plugin list, check, move and sort each read the installed plugins from a
//! deployment plan, which costs 15-30 s on a 368-mod profile. A command that reads the order twice
//! (a move, then the order it shows) must plan once. The planner's call counter is process-wide,
//! so this file holds one test only: nothing else in the binary runs beside it.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use agora_core::ctx::CoreContext;
use agora_core::game_base::BaseMode;
use agora_core::game_deploy::{add_content, PLAN_CALLS};
use agora_core::game_discovery::{DiscoveredInstall, InstallCapabilities};
use agora_core::game_instance::{create, GameInstanceRecord};
use agora_core::game_load_order::{self as load_order, MoveTarget};
use agora_core::game_registry::{
    GameRegistry, IdentifiedInstall, PackageSource, RuntimeResolution,
};
use agora_game_api::{
    DeploymentStrategy, GameDefinition, GameId, GamePackage, GamePath, InstallId, InstallKind,
    LaunchRecipe, LaunchValue, PackageDefinition, PluginListRule, RelPath, RuntimeIdentity,
    StoreId, StoreIdentifier, UserDataLocation, UserFileMapping, UserFileStrategy,
};
use tempfile::TempDir;

struct Package(PackageDefinition);

impl GamePackage for Package {
    fn definition(&self) -> &PackageDefinition {
        &self.0
    }
}

fn definition() -> GameDefinition {
    let plugin_list = 2u8;
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

fn plans<T>(work: impl FnOnce() -> T) -> (T, usize) {
    let before = PLAN_CALLS.load(Ordering::Relaxed);
    let out = work();
    (out, PLAN_CALLS.load(Ordering::Relaxed) - before)
}

#[test]
fn each_plugin_command_plans_once() {
    let tmp = TempDir::new().unwrap();
    let def = definition();
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
        for name in ["A", "B", "C"] {
            std::fs::write(
                mod_dir.path().join("Data").join(format!("{name}{n}.esp")),
                b"not a real plugin header",
            )
            .unwrap();
        }
        let outcome =
            agora_core::content_store::add_folder(&ctx, mod_dir.path(), Some(&format!("Mod{n}")))
                .unwrap();
        add_content(&ctx, &inst.instance_id, &outcome.item().item_id, None, None).unwrap();
    }
    let id = inst.instance_id.as_str();
    let list = ctx.paths.instance_dir(id).unwrap().join("user/Plugins.txt");
    std::fs::create_dir_all(list.parent().unwrap()).unwrap();
    std::fs::write(
        &list,
        "A0.esp
B0.esp
C0.esp
A1.esp
",
    )
    .unwrap();

    let names = |order: &load_order::LoadOrder| -> Vec<String> {
        order.entries.iter().map(|e| e.name.clone()).collect()
    };

    // The list view and its findings come from one order, so one plan.
    let (order, n) = plans(|| load_order::order(&ctx, id, &def).unwrap());
    assert_eq!(n, 1, "list");
    assert!(!order.entries.is_empty());
    let findings = order.findings.clone();
    // The check alone is one plan too; a caller that wants both reads the findings off the order.
    let (checked, n) = plans(|| load_order::check(&ctx, id, &def).unwrap());
    assert_eq!(n, 1, "check on its own");
    assert_eq!(checked, findings);

    // A move and the order it returns share one plan (two before they were joined).
    let ((report, after), n) = plans(|| {
        load_order::move_plugin_and_order(&ctx, id, &def, "A1.esp", &MoveTarget::Position(1))
            .unwrap()
    });
    assert_eq!(n, 1, "move and order");
    assert!(report.written);
    assert_eq!(names(&after)[0], "A1.esp");
    let (_, separate) = plans(|| {
        load_order::move_plugin(&ctx, id, &def, "A1.esp", &MoveTarget::Position(2)).unwrap();
        load_order::order(&ctx, id, &def).unwrap()
    });
    assert_eq!(separate, 2, "the unjoined calls plan twice");

    // The result is what the separate calls give.
    let (_, joined) =
        load_order::move_plugin_and_order(&ctx, id, &def, "C0.esp", &MoveTarget::Position(1))
            .unwrap();
    let reread = load_order::order(&ctx, id, &def).unwrap();
    assert_eq!(names(&joined), names(&reread));
    assert_eq!(joined.findings, reread.findings);

    // A sort and the order it returns share one plan.
    let ((sorted, _), n) = plans(|| load_order::sort_and_order(&ctx, id, &def, true).unwrap());
    assert_eq!(n, 1, "sort and order");
    assert!(!sorted.written, "a dry run writes nothing");
}
