//! Speed benchmark for deploying and launching a large instance (MASTER_SPEC §26.5).
//!
//! Run with `cargo test -p agora-core --test game_deploy_speed_bench -- --ignored --nocapture`.
//!
//! It builds a synthetic instance under a temp data root (`AGORA_BENCH_ROOT`, default
//! `D:\Agora-bench\perfdeploy-data`, which must be on the same volume as the install so links work),
//! then times: the planner; the first link deploy; an up-to-date deploy; and the launch preparation
//! up to the point where a game would start, both on a fresh instance (builds the links) and on a
//! deployed one (up to date). It counts the plans each step makes. Nothing here starts a game.
//!
//! Sizes: `AGORA_BENCH_ITEMS` content items (default 300), `AGORA_BENCH_FILES` small files per item
//! (default 400), `AGORA_BENCH_BASE_FILES` base files (default 3000).

// This bench uses only the data root of the shared fixtures.
#[allow(dead_code)]
mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use agora_core::ctx::CoreContext;
use agora_core::game_base::BaseMode;
use agora_core::game_deploy::{add_content, deploy, plan, DeployMode, DeployOutcome, PLAN_CALLS};
use agora_core::game_discovery::{DiscoveredInstall, DiscoveryReport, InstallCapabilities};
use agora_core::game_instance::{create, prepare_launch_with, GameInstanceRecord, LaunchOptions};
use agora_core::game_launch::{LaunchError, LaunchedGame, Launcher, PreparedLaunch};
use agora_core::game_registry::{
    GameRegistry, IdentifiedInstall, PackageSource, RuntimeResolution,
};
use agora_game_api::{
    DeploymentStrategy, GameDefinition, GameId, GamePackage, GamePath, InstallId, InstallKind,
    LaunchRecipe, LaunchValue, PackageDefinition, RelPath, RuntimeIdentity, StoreId,
    StoreIdentifier,
};
use std::sync::atomic::Ordering;

use common::data_root::DataRoot;

struct BenchPackage(PackageDefinition);

impl GamePackage for BenchPackage {
    fn definition(&self) -> &PackageDefinition {
        &self.0
    }
}

/// A launcher that must never be asked to start a game: the benchmark stops after preparation.
struct NoStart;

impl Launcher for NoStart {
    fn locate_vfs_dll(&self) -> Result<PathBuf, String> {
        Err("the benchmark runs on links, not the virtual file system".into())
    }

    fn launch(&self, _prepared: &PreparedLaunch) -> Result<LaunchedGame, LaunchError> {
        panic!("the benchmark must not start a game");
    }
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

fn bench_definition() -> GameDefinition {
    GameDefinition {
        mo2_game_name: None,
        id: GameId::new("bench-game").unwrap(),
        name: "Bench Game".into(),
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
                value: "-bench".into(),
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
        id: format!("bench.{}", def.id),
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
                crate_name: "bench".into(),
            },
            Arc::new(BenchPackage(pkg_def)),
        )
        .expect("register bench package");
    Arc::new(builder.build())
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
        name: "Bench Game".into(),
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

/// A game install with `base_files` small files under `Data/base`.
fn write_install(install_dir: &Path, base_files: usize) {
    std::fs::write(install_dir.join("Game.exe"), b"fake game binary").unwrap();
    std::fs::write(install_dir.join("writeable_base.txt"), b"writable").unwrap();
    let base_dir = install_dir.join("Data").join("base");
    std::fs::create_dir_all(&base_dir).unwrap();
    std::fs::write(install_dir.join("Data").join("Bench.bsa"), b"BSA DATA").unwrap();
    for k in 0..base_files {
        std::fs::write(base_dir.join(format!("b{k:05}.bin")), vec![b'b'; 200]).unwrap();
    }
}

/// One content item: `files` small unique files under `Data/mod<n>`, and one config file (a copy).
fn write_mod(dir: &Path, n: usize, files: usize) {
    let mod_dir = dir.join("Data").join(format!("mod{n:03}"));
    std::fs::create_dir_all(&mod_dir).unwrap();
    for k in 0..files {
        let text = format!("item {n} file {k} ");
        let mut body = text.into_bytes();
        body.resize(256, b'.');
        std::fs::write(mod_dir.join(format!("f{k:03}.dat")), body).unwrap();
    }
    std::fs::write(mod_dir.join("config.ini"), format!("[mod{n}]\nvalue=1\n")).unwrap();
}

fn add_item(ctx: &CoreContext, n: usize, files: usize) -> String {
    let dir = tempfile::tempdir().unwrap();
    write_mod(dir.path(), n, files);
    let outcome =
        agora_core::content_store::add_folder(ctx, dir.path(), Some(&format!("Mod{n:03}")))
            .unwrap();
    outcome.item().item_id.clone()
}

fn timed<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let start = Instant::now();
    let out = f();
    (out, start.elapsed())
}

fn plans_during<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let before = PLAN_CALLS.load(Ordering::Relaxed);
    let out = f();
    (out, PLAN_CALLS.load(Ordering::Relaxed) - before)
}

fn secs(d: Duration) -> String {
    format!("{:.3} s", d.as_secs_f64())
}

fn no_discovery() -> DiscoveryReport {
    DiscoveryReport {
        installs: vec![],
        warnings: vec![],
    }
}

#[test]
#[ignore = "benchmark: run with --ignored --nocapture"]
fn deploy_and_launch_timings() {
    let items = env_usize("AGORA_BENCH_ITEMS", 300);
    let files = env_usize("AGORA_BENCH_FILES", 400);
    let base_files = env_usize("AGORA_BENCH_BASE_FILES", 3000);

    let root = std::env::var_os("AGORA_BENCH_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"D:\Agora-bench\perfdeploy-data"));
    std::fs::create_dir_all(&root).unwrap();
    // AGORA_BENCH_KEEP=1 keeps the synthetic data between runs, in a folder named for its sizes. A
    // run that finds the folder's marker reuses the imported content (the slow part) instead of
    // importing it again. Delete the folder when done.
    let keep = env_usize("AGORA_BENCH_KEEP", 0) == 1;
    // A plain run's data root is removed when the run ends, protected objects included.
    let _scratch = (!keep).then(|| DataRoot::in_dir(&root, "bench-"));
    let dir: PathBuf = match &_scratch {
        Some(scratch) => scratch.path().to_path_buf(),
        None => root.join(format!("keep-{items}x{files}-{base_files}")),
    };
    std::fs::create_dir_all(&dir).unwrap();
    let reuse = keep && dir.join("ready").exists();
    let tmp_path = dir.clone();

    let mut def = bench_definition();
    if env_usize("AGORA_BENCH_PLUGINS", 0) == 1 {
        // A game with a plugin list under the Creation Engine rules, as Skyrim has: the launch checks the
        // load order, and that check reads the installed plugins from a plan.
        def.plugin_list = Some(agora_game_api::PluginListRule {
            user_file: RelPath::new("user/Plugins.txt").unwrap(),
            plugin_folder: RelPath::new("Data").unwrap(),
            patterns: vec!["*.esp".into(), "*.esm".into()],
            active_prefix: String::new(),
            header: vec![],
            semantics: Some("creation_engine".into()),
            implicit: vec![],
            implicit_list_file: None,
        });
        def.user_files = vec![agora_game_api::UserFileMapping::new(
            GamePath::UserData {
                location: agora_game_api::UserDataLocation::LocalAppData,
                path: RelPath::new("Bench Game/Plugins.txt").unwrap(),
            },
            RelPath::new("user/Plugins.txt").unwrap(),
            agora_game_api::UserFileStrategy::JournaledSwap,
        )
        .with_stores(vec![StoreId::steam()])];
    }
    let ctx = CoreContext::for_testing(tmp_path.join("app_data"));
    agora_core::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();
    let ctx = ctx.with_games(register(def.clone()));

    let install_dir = tmp_path.join("install");
    if !reuse {
        std::fs::create_dir_all(&install_dir).unwrap();
        write_install(&install_dir, base_files);
    }
    let install = make_install(&install_dir, &def);

    let (inst_a, t_base): (GameInstanceRecord, Duration) = timed(|| {
        create(
            &ctx,
            &install,
            &def,
            "BenchA",
            None,
            BaseMode::Linked,
            &|_| {},
        )
        .expect("create instance A")
    });

    let (item_ids, t_content) = timed(|| {
        if reuse {
            std::fs::read_to_string(dir.join("items.txt"))
                .unwrap()
                .lines()
                .map(str::to_string)
                .collect::<Vec<_>>()
        } else {
            (0..items)
                .map(|n| add_item(&ctx, n, files))
                .collect::<Vec<_>>()
        }
    });
    if keep && !reuse {
        std::fs::write(
            dir.join("items.txt"),
            item_ids.join(
                "
",
            ),
        )
        .unwrap();
        std::fs::write(dir.join("ready"), "ready").unwrap();
    }
    let (_, t_layers) = timed(|| {
        for id in &item_ids {
            add_content(&ctx, &inst_a.instance_id, id, None, None).unwrap();
        }
    });

    let id_a = inst_a.instance_id.clone();
    let ((planned, t_plan), plans_plan) =
        plans_during(|| timed(|| plan(&ctx, &id_a, &def, DeployMode::Links).unwrap()));
    let planned_files = planned.files.len();

    // First link deploy: builds the farm.
    let (outcome_build, t_build) = timed(|| deploy(&ctx, &id_a, &def, DeployMode::Links).unwrap());
    assert!(matches!(outcome_build, DeployOutcome::Built { .. }));

    // Up-to-date deploy: the check that decides nothing needs building.
    let (outcome_same, t_same) = timed(|| deploy(&ctx, &id_a, &def, DeployMode::Links).unwrap());
    assert!(matches!(outcome_same, DeployOutcome::UpToDate { .. }));

    let options = LaunchOptions {
        deployment: Some(DeployMode::Links),
        ..Default::default()
    };
    let launcher = NoStart;

    // Launch preparation on the up-to-date deployment (the common case).
    let (prep_same, plans_same) = plans_during(|| {
        timed(|| {
            prepare_launch_with(&ctx, &id_a, &def, options, &no_discovery, &launcher)
                .expect("prepare launch on the deployed instance")
        })
    });
    let (_prepared_same, t_prep_same) = prep_same;

    // Launch preparation on a fresh instance: builds the farm, the first launch.
    let inst_b = create(
        &ctx,
        &install,
        &def,
        "BenchB",
        None,
        BaseMode::Linked,
        &|_| {},
    )
    .expect("create instance B");
    for id in &item_ids {
        add_content(&ctx, &inst_b.instance_id, id, None, None).unwrap();
    }
    let (prep_first, plans_first) = plans_during(|| {
        timed(|| {
            prepare_launch_with(
                &ctx,
                &inst_b.instance_id,
                &def,
                options,
                &no_discovery,
                &launcher,
            )
            .expect("first launch preparation")
        })
    });
    let (_prepared_first, t_prep_first) = prep_first;

    let (linked, copied) = match outcome_build {
        DeployOutcome::Built { linked, copied, .. } => (linked, copied),
        DeployOutcome::UpToDate { .. } => (0, 0),
    };

    println!();
    println!(
        "== bench: {items} items x {files} files, {base_files} base files, plugin list: {}; {planned_files} planned files ({linked} linked, {copied} copied)",
        def.plugin_list.is_some()
    );
    println!(
        "setup (not timed in the table): base {}, content import {}, add to instance {}",
        secs(t_base),
        secs(t_content),
        secs(t_layers)
    );
    println!("{:<34} {:>12} {:>8}", "step", "time", "plans");
    println!("{:<34} {:>12} {:>8}", "plan", secs(t_plan), plans_plan);
    println!(
        "{:<34} {:>12} {:>8}",
        "first link deploy (builds)",
        secs(t_build),
        "-"
    );
    println!(
        "{:<34} {:>12} {:>8}",
        "up-to-date deploy",
        secs(t_same),
        "-"
    );
    println!(
        "{:<34} {:>12} {:>8}",
        "launch prep, up to date",
        secs(t_prep_same),
        plans_same
    );
    println!(
        "{:<34} {:>12} {:>8}",
        "launch prep, first (builds)",
        secs(t_prep_first),
        plans_first
    );
    println!();
}
