use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

use agora_core::app_paths::AppPaths;
use agora_core::game_base::{build_base, BaseMode, BuildOptions};
use agora_core::game_discovery::{DiscoveredInstall, InstallCapabilities};
use agora_core::game_launch::{
    launch, prepare_base_launch, processes_running_from, resolve_recipe, wait_for_exit,
    LaunchError, LaunchRoots,
};
use agora_core::game_registry::{IdentifiedInstall, RuntimeResolution};
use agora_game_api::{
    DeploymentStrategy, GameDefinition, GameId, GamePath, InstallId, InstallKind, LaunchRecipe,
    LaunchValue, RelPath, RuntimeIdentity, StoreId, StoreIdentifier, UserDataLocation,
};
use tempfile::TempDir;

fn make_test_definition(recipe: Option<LaunchRecipe>) -> GameDefinition {
    GameDefinition {
        id: GameId::new("test-game").unwrap(),
        name: "Test Game".into(),
        stores: vec![
            StoreIdentifier {
                store: StoreId::new("steam").unwrap(),
                product: "489830".into(),
            },
            StoreIdentifier {
                store: StoreId::new("gog").unwrap(),
                product: "1711230643".into(),
            },
        ],
        version_sources: vec![],
        deployment: DeploymentStrategy::VirtualFileSystem,
        content_rules: vec![],
        native_code_patterns: vec![],
        framework_ids: vec![],
        tool_ids: vec![],
        launch: recipe,
        log_paths: vec![],
        crash_paths: vec![],
        user_files: vec![],
        save_paths: vec![],
        linked_archive_patterns: vec!["Data/*.bsa".into()],
        declared_writes: vec!["d3dx9_42.log".into()],
        excluded_paths: vec![],
        plugin_list: None,
        launch_alternatives: Vec::new(),
        content_layout: None,
    }
}

fn make_test_install(install_dir: &Path, runtime: RuntimeIdentity) -> IdentifiedInstall {
    let install_id = InstallId::new(format!("{}:{}", runtime.store, runtime.game)).unwrap();
    let discovered = DiscoveredInstall {
        store: runtime.store.clone(),
        product: "489830".into(),
        name: "Test Game".into(),
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
        volume: None,
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

// ---------------------------------------------------------------------------
// 1. Recipe resolution tests
// ---------------------------------------------------------------------------

#[test]
fn recipe_resolution_all_supported_roots_and_values() {
    let tmp = TempDir::new().unwrap();
    let runtime_dir = tmp.path().join("runtime");
    let install_dir = tmp.path().join("install");
    let base_dir = tmp.path().join("base");

    std::fs::create_dir_all(&runtime_dir).unwrap();
    std::fs::create_dir_all(&install_dir).unwrap();
    std::fs::create_dir_all(&base_dir).unwrap();

    let exe_path = runtime_dir.join("game.exe");
    std::fs::write(&exe_path, b"fake game binary").unwrap();

    let roots = LaunchRoots {
        runtime: runtime_dir.clone(),
        install: Some(install_dir.clone()),
        base: Some(base_dir.clone()),
    };

    let mut environment = BTreeMap::new();
    environment.insert(
        "ENV_LITERAL".into(),
        LaunchValue::Literal {
            value: "hello".into(),
        },
    );
    environment.insert(
        "ENV_PATH".into(),
        LaunchValue::Path {
            path: GamePath::Install {
                install: InstallId::new("steam:test").unwrap(),
                path: RelPath::new("extra.dll").unwrap(),
            },
            prefix: "PRE_".into(),
            suffix: "_POST".into(),
        },
    );
    environment.insert(
        "ENV_PATHLIST".into(),
        LaunchValue::PathList {
            paths: vec![
                GamePath::Runtime {
                    path: RelPath::new("lib1.dll").unwrap(),
                },
                GamePath::Base {
                    base: "ignored".into(),
                    path: RelPath::new("lib2.dll").unwrap(),
                },
            ],
            prefix: "CP=".into(),
        },
    );

    let recipe = LaunchRecipe {
        executable: GamePath::Runtime {
            path: RelPath::new("game.exe").unwrap(),
        },
        arguments: vec![
            LaunchValue::Literal {
                value: "--flag".into(),
            },
            LaunchValue::Path {
                path: GamePath::UserData {
                    location: UserDataLocation::Documents,
                    path: RelPath::new("My Saves").unwrap(),
                },
                prefix: "--saves=".into(),
                suffix: "".into(),
            },
            LaunchValue::Path {
                path: GamePath::UserData {
                    location: UserDataLocation::RoamingAppData,
                    path: RelPath::new("Config").unwrap(),
                },
                prefix: "--roaming=".into(),
                suffix: "".into(),
            },
            LaunchValue::Path {
                path: GamePath::UserData {
                    location: UserDataLocation::LocalAppData,
                    path: RelPath::new("Cache").unwrap(),
                },
                prefix: "--local=".into(),
                suffix: "".into(),
            },
            LaunchValue::Path {
                path: GamePath::UserData {
                    location: UserDataLocation::Home,
                    path: RelPath::new(".test").unwrap(),
                },
                prefix: "--home=".into(),
                suffix: "".into(),
            },
        ],
        environment,
        working_directory: GamePath::Runtime {
            path: RelPath::default(),
        },
    };

    let resolved = resolve_recipe(&recipe, &roots).expect("recipe resolution should succeed");

    assert_eq!(resolved.program, exe_path);
    assert_eq!(resolved.cwd, runtime_dir);

    // Check args
    assert_eq!(resolved.args[0], OsString::from("--flag"));
    let doc_str = resolved.args[1].to_string_lossy();
    assert!(doc_str.starts_with("--saves="));
    assert!(doc_str.ends_with("My Saves"));

    let roaming_str = resolved.args[2].to_string_lossy();
    assert!(roaming_str.starts_with("--roaming="));
    assert!(roaming_str.ends_with("Config"));

    let local_str = resolved.args[3].to_string_lossy();
    assert!(local_str.starts_with("--local="));
    assert!(local_str.ends_with("Cache"));

    let home_str = resolved.args[4].to_string_lossy();
    assert!(home_str.starts_with("--home="));
    assert!(home_str.ends_with(".test"));

    // Check environment
    assert_eq!(
        resolved.env.get("ENV_LITERAL").unwrap(),
        &OsString::from("hello")
    );
    let env_path = resolved.env.get("ENV_PATH").unwrap().to_string_lossy();
    assert!(env_path.starts_with("PRE_"));
    assert!(env_path.ends_with("_POST"));
    assert!(env_path.contains("extra.dll"));

    let env_list = resolved.env.get("ENV_PATHLIST").unwrap().to_string_lossy();
    assert!(env_list.starts_with("CP="));
    #[cfg(windows)]
    assert!(env_list.contains(';'));
    #[cfg(not(windows))]
    assert!(env_list.contains(':'));
    assert!(env_list.contains("lib1.dll"));
    assert!(env_list.contains("lib2.dll"));
}

#[test]
fn recipe_resolution_unsupported_roots() {
    let tmp = TempDir::new().unwrap();
    let runtime_dir = tmp.path().join("runtime");
    std::fs::create_dir_all(&runtime_dir).unwrap();
    let exe = runtime_dir.join("game.exe");
    std::fs::write(&exe, b"bin").unwrap();

    let roots = LaunchRoots {
        runtime: runtime_dir,
        install: None,
        base: None,
    };

    let test_unsupported = |path: GamePath, expected_root_name: &str| {
        let recipe = LaunchRecipe {
            executable: GamePath::Runtime {
                path: RelPath::new("game.exe").unwrap(),
            },
            arguments: vec![LaunchValue::Path {
                path,
                prefix: "".into(),
                suffix: "".into(),
            }],
            environment: BTreeMap::new(),
            working_directory: GamePath::Runtime {
                path: RelPath::default(),
            },
        };
        let err = resolve_recipe(&recipe, &roots).unwrap_err();
        match err {
            LaunchError::UnsupportedRoot(name) => {
                assert_eq!(name, expected_root_name);
            }
            other => panic!("expected UnsupportedRoot({expected_root_name}), got: {other:?}"),
        }
    };

    test_unsupported(
        GamePath::Instance {
            path: RelPath::new("foo").unwrap(),
        },
        "instance",
    );
    test_unsupported(
        GamePath::Layer {
            layer: agora_game_api::LayerId::new("mod-a").unwrap(),
            path: RelPath::new("bar").unwrap(),
        },
        "layer",
    );
    test_unsupported(
        GamePath::RuntimeComponent {
            component: "skse".into(),
            path: RelPath::new("skse.dll").unwrap(),
        },
        "runtime_component",
    );
    test_unsupported(
        GamePath::Artifact {
            artifact: "core-art".into(),
        },
        "artifact",
    );
}

#[test]
fn recipe_resolution_missing_program() {
    let tmp = TempDir::new().unwrap();
    let runtime_dir = tmp.path().join("runtime");
    std::fs::create_dir_all(&runtime_dir).unwrap();

    let roots = LaunchRoots {
        runtime: runtime_dir.clone(),
        install: None,
        base: None,
    };

    let recipe = LaunchRecipe {
        executable: GamePath::Runtime {
            path: RelPath::new("nonexistent.exe").unwrap(),
        },
        arguments: vec![],
        environment: BTreeMap::new(),
        working_directory: GamePath::Runtime {
            path: RelPath::default(),
        },
    };

    let err = resolve_recipe(&recipe, &roots).unwrap_err();
    match err {
        LaunchError::ProgramMissing { path } => {
            assert_eq!(path, runtime_dir.join("nonexistent.exe"));
        }
        other => panic!("expected ProgramMissing, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 2. prepare_base_launch tests
// ---------------------------------------------------------------------------

#[test]
fn prepare_base_launch_no_recipe() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let install_dir = tmp.path().join("source_install");
    let base_root = tmp.path().join("bases_root");

    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&install_dir).unwrap();
    let paths = AppPaths::from_root(data_dir);
    std::fs::write(install_dir.join("Game.exe"), b"exe").unwrap();

    let def = make_test_definition(None); // no recipe

    let runtime = RuntimeIdentity {
        game: GameId::new("test-game").unwrap(),
        store: StoreId::new("steam").unwrap(),
        version: "1.0.0".into(),
        build: None,
    };
    let install = make_test_install(&install_dir, runtime);

    let outcome = build_base(
        &paths,
        &install,
        &def,
        BaseMode::Copied,
        Some(&base_root),
        BuildOptions::default(),
        &|_| {},
    )
    .unwrap();
    let manifest = outcome.manifest();

    let err = prepare_base_launch(manifest, &def, false).unwrap_err();
    assert!(matches!(err, LaunchError::NoRecipe));
}

#[test]
fn prepare_base_launch_damaged_refusal_and_launch_anyway() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let install_dir = tmp.path().join("source_install");
    let base_root = tmp.path().join("bases_root");

    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&install_dir).unwrap();
    let paths = AppPaths::from_root(data_dir);
    std::fs::write(install_dir.join("Game.exe"), b"exe binary").unwrap();
    std::fs::write(install_dir.join("data.bin"), b"original data").unwrap();

    let recipe = LaunchRecipe {
        executable: GamePath::Runtime {
            path: RelPath::new("Game.exe").unwrap(),
        },
        arguments: vec![],
        environment: BTreeMap::new(),
        working_directory: GamePath::Runtime {
            path: RelPath::default(),
        },
    };
    let def = make_test_definition(Some(recipe));

    let runtime = RuntimeIdentity {
        game: GameId::new("test-game").unwrap(),
        store: StoreId::new("steam").unwrap(),
        version: "1.0.0".into(),
        build: None,
    };
    let install = make_test_install(&install_dir, runtime);

    let outcome = build_base(
        &paths,
        &install,
        &def,
        BaseMode::Copied,
        Some(&base_root),
        BuildOptions::default(),
        &|_| {},
    )
    .unwrap();
    let manifest = outcome.manifest();

    // Corrupt data.bin in the base
    std::fs::write(manifest.location.join("data.bin"), b"corrupted data!").unwrap();

    // 1. Without launch_anyway -> Refused with BaseDamaged naming the file
    let err = prepare_base_launch(manifest, &def, false).unwrap_err();
    match err {
        LaunchError::BaseDamaged { problems } => {
            assert!(
                problems.iter().any(|p| p.path == "data.bin"),
                "must report data.bin damaged"
            );
        }
        other => panic!("expected BaseDamaged, got: {other:?}"),
    }

    // 2. With launch_anyway -> Success, carrying warnings
    let prepared = prepare_base_launch(manifest, &def, true).expect("launch_anyway should succeed");
    assert!(
        prepared.warnings.iter().any(|p| p.path == "data.bin"),
        "warnings must carry the problem"
    );
    assert_eq!(
        prepared.resolved.program,
        manifest.location.join("Game.exe")
    );
}

#[test]
fn prepare_base_launch_steam_env_only_for_steam() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let install_dir = tmp.path().join("source_install");
    let base_root = tmp.path().join("bases_root");

    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&install_dir).unwrap();
    let paths = AppPaths::from_root(data_dir);
    std::fs::write(install_dir.join("Game.exe"), b"exe").unwrap();

    let recipe = LaunchRecipe {
        executable: GamePath::Runtime {
            path: RelPath::new("Game.exe").unwrap(),
        },
        arguments: vec![],
        environment: BTreeMap::new(),
        working_directory: GamePath::Runtime {
            path: RelPath::default(),
        },
    };
    let def = make_test_definition(Some(recipe));

    // A. Steam runtime
    let runtime_steam = RuntimeIdentity {
        game: GameId::new("test-game").unwrap(),
        store: StoreId::new("steam").unwrap(),
        version: "1.0.0".into(),
        build: None,
    };
    let install_steam = make_test_install(&install_dir, runtime_steam);

    let outcome_steam = build_base(
        &paths,
        &install_steam,
        &def,
        BaseMode::Copied,
        Some(&base_root),
        BuildOptions::default(),
        &|_| {},
    )
    .unwrap();
    let prepared_steam = prepare_base_launch(outcome_steam.manifest(), &def, false).unwrap();
    assert_eq!(
        prepared_steam.resolved.env.get("SteamAppId").unwrap(),
        &OsString::from("489830")
    );
    assert_eq!(
        prepared_steam.resolved.env.get("SteamGameId").unwrap(),
        &OsString::from("489830")
    );

    // B. GOG runtime
    let runtime_gog = RuntimeIdentity {
        game: GameId::new("test-game").unwrap(),
        store: StoreId::new("gog").unwrap(),
        version: "1.0.1".into(),
        build: None,
    };
    let mut install_gog = make_test_install(&install_dir, runtime_gog);
    install_gog.discovered.store = StoreId::new("gog").unwrap();
    install_gog.discovered.product = "1711230643".into();

    let outcome_gog = build_base(
        &paths,
        &install_gog,
        &def,
        BaseMode::Copied,
        Some(&base_root),
        BuildOptions::default(),
        &|_| {},
    )
    .unwrap();
    let prepared_gog = prepare_base_launch(outcome_gog.manifest(), &def, false).unwrap();
    assert!(!prepared_gog.resolved.env.contains_key("SteamAppId"));
    assert!(!prepared_gog.resolved.env.contains_key("SteamGameId"));
}

// ---------------------------------------------------------------------------
// 3. Launch and watch tests (stand-in process)
// ---------------------------------------------------------------------------

#[test]
#[cfg(windows)]
fn launch_and_watch_standin_process() {
    let ping_sys_path = Path::new(r"C:\Windows\System32\PING.EXE");
    if !ping_sys_path.exists() {
        eprintln!("Skipping test: C:\\Windows\\System32\\PING.EXE not found");
        return;
    }

    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let install_dir = tmp.path().join("source_install");
    let base_root = tmp.path().join("bases_root");

    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&install_dir).unwrap();
    let paths = AppPaths::from_root(data_dir);

    // Copy PING.EXE into the fake install as PingGame.exe
    let fake_game_exe = install_dir.join("PingGame.exe");
    std::fs::copy(ping_sys_path, &fake_game_exe).unwrap();

    let recipe = LaunchRecipe {
        executable: GamePath::Runtime {
            path: RelPath::new("PingGame.exe").unwrap(),
        },
        arguments: vec![
            LaunchValue::Literal { value: "-n".into() },
            LaunchValue::Literal { value: "3".into() },
            LaunchValue::Literal {
                value: "127.0.0.1".into(),
            },
        ],
        environment: BTreeMap::new(),
        working_directory: GamePath::Runtime {
            path: RelPath::default(),
        },
    };
    let def = make_test_definition(Some(recipe));

    let runtime = RuntimeIdentity {
        game: GameId::new("test-game").unwrap(),
        store: StoreId::new("steam").unwrap(),
        version: "1.0.0".into(),
        build: None,
    };
    let install = make_test_install(&install_dir, runtime);

    let outcome = build_base(
        &paths,
        &install,
        &def,
        BaseMode::Copied,
        Some(&base_root),
        BuildOptions::default(),
        &|_| {},
    )
    .unwrap();
    let manifest = outcome.manifest();

    let prepared = prepare_base_launch(manifest, &def, false).unwrap();
    let mut launched = launch(&prepared).expect("launch must succeed");
    let pid = launched.pid();
    assert!(pid > 0);

    // Check that processes_running_from detects the process in the base folder
    let running = processes_running_from(&manifest.location);
    assert!(
        running.iter().any(|p| p.pid == pid),
        "processes_running_from must detect the launched process"
    );

    // Check that a process from outside the base is NOT counted in processes_running_from
    let outside_running = processes_running_from(&install_dir);
    assert!(
        !outside_running.iter().any(|p| p.pid == pid),
        "outside directory must not count the base process"
    );

    // Wait for exit
    let exit_report = wait_for_exit(
        &manifest.location,
        &mut launched,
        Duration::from_millis(50),
        Duration::from_millis(300),
    );

    assert!(
        exit_report.processes.iter().any(|p| p.pid == pid),
        "exit report must list the process that ran from the base"
    );
    assert!(
        !exit_report.relaunched_outside,
        "relaunched_outside must be false for normal base execution"
    );
}

/// A copy of the game the user already had running elsewhere is not a
/// relaunch; one started after the launch, outside the base, is.
#[test]
#[cfg(windows)]
fn only_a_process_started_after_launch_counts_as_relaunched_outside() {
    let ping = Path::new(r"C:\Windows\System32\PING.EXE");
    if !ping.exists() {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let install_dir = tmp.path().join("source_install");
    let elsewhere = tmp.path().join("store_folder");
    for dir in [&data_dir, &install_dir, &elsewhere] {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::copy(ping, install_dir.join("PingRelaunch.exe")).unwrap();
    std::fs::copy(ping, elsewhere.join("PingRelaunch.exe")).unwrap();
    let run_elsewhere = |count: &str| {
        std::process::Command::new(elsewhere.join("PingRelaunch.exe"))
            .args(["-n", count, "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap()
    };

    let recipe = LaunchRecipe {
        executable: GamePath::Runtime {
            path: RelPath::new("PingRelaunch.exe").unwrap(),
        },
        arguments: ["-n", "3", "127.0.0.1"]
            .into_iter()
            .map(|v| LaunchValue::Literal { value: v.into() })
            .collect(),
        environment: BTreeMap::new(),
        working_directory: GamePath::Runtime {
            path: RelPath::default(),
        },
    };
    let def = make_test_definition(Some(recipe));
    let runtime = RuntimeIdentity {
        game: GameId::new("test-game").unwrap(),
        store: StoreId::new("gog").unwrap(),
        version: "1.0.0".into(),
        build: None,
    };
    let install = make_test_install(&install_dir, runtime);
    let paths = AppPaths::from_root(data_dir);
    let outcome = build_base(
        &paths,
        &install,
        &def,
        BaseMode::Copied,
        Some(&tmp.path().join("AgoraBases")),
        BuildOptions::default(),
        &|_| {},
    )
    .unwrap();
    let manifest = outcome.manifest();

    // Already running before the launch: not a relaunch.
    let mut before = run_elsewhere("8");
    std::thread::sleep(Duration::from_millis(1100));
    let mut launched = launch(&prepare_base_launch(manifest, &def, false).unwrap()).unwrap();
    let report = wait_for_exit(
        &manifest.location,
        &mut launched,
        Duration::from_millis(50),
        Duration::from_millis(300),
    );
    assert!(
        !report.relaunched_outside,
        "a pre-existing copy is not a relaunch"
    );

    // Started after the launch, outside the base: a relaunch.
    let mut launched = launch(&prepare_base_launch(manifest, &def, false).unwrap()).unwrap();
    std::thread::sleep(Duration::from_millis(1100));
    let mut after = run_elsewhere("2");
    let report = wait_for_exit(
        &manifest.location,
        &mut launched,
        Duration::from_millis(50),
        Duration::from_millis(300),
    );
    let _ = before.kill();
    let _ = before.wait();
    let _ = after.wait();
    assert!(
        report.relaunched_outside,
        "a copy started after launch, elsewhere, is a relaunch"
    );
}

/// Defining a game is not consent to run any program in the user's folders.
#[test]
fn a_recipe_executable_outside_the_game_is_refused() {
    let tmp = TempDir::new().unwrap();
    let roots = agora_core::game_launch::LaunchRoots {
        runtime: tmp.path().to_path_buf(),
        install: None,
        base: None,
    };
    for location in [
        UserDataLocation::Documents,
        UserDataLocation::RoamingAppData,
    ] {
        let recipe = LaunchRecipe {
            executable: GamePath::UserData {
                location,
                path: RelPath::new("evil.exe").unwrap(),
            },
            arguments: vec![],
            environment: BTreeMap::new(),
            working_directory: GamePath::Runtime {
                path: RelPath::default(),
            },
        };
        assert!(matches!(
            agora_core::game_launch::resolve_recipe(&recipe, &roots),
            Err(LaunchError::ExecutableOutsideGame)
        ));
    }
}
