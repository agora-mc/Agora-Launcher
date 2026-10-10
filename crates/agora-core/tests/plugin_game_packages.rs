use std::sync::Arc;

use agora_core::app_paths::AppPaths;
use agora_core::ctx::CoreContext;
use agora_core::game_registry::{DeclarativePackage, GameRegistry, PackageSource};
use agora_core::plugins::store::{self, PluginSource};
use agora_core::plugins::PLUGINS_ENABLED_SETTING;
use agora_game_api::{DeploymentStrategy, GameDefinition, GameId, PackageDefinition};
use agora_plugin_api::capability::{Capability, CapabilitySet};
use agora_plugin_api::manifest::PluginManifest;
use semver::{Version, VersionReq};

fn dummy_package_def(game_id: &str) -> PackageDefinition {
    PackageDefinition {
        id: format!("test.pkg.{game_id}"),
        version: Version::new(0, 1, 0),
        api_range: VersionReq::parse(">=0.1, <0.2").unwrap(),
        parents: vec![],
        games: vec![GameDefinition {
            mo2_game_name: None,
            id: GameId::new(game_id).unwrap(),
            name: game_id.to_string(),
            stores: vec![],
            version_sources: vec![],
            deployment: DeploymentStrategy::Redirect,
            content_rules: vec![],
            native_code_patterns: vec![],
            framework_ids: vec![],
            tool_ids: vec![],
            launch: None,
            log_paths: vec![],
            crash_paths: vec![],
            user_files: vec![],
            save_paths: vec![],
            linked_archive_patterns: vec![],
            declared_writes: vec![],
            excluded_paths: vec![],
            plugin_list: None,
            runtime_files: Vec::new(),
            save_location: Vec::new(),
            launch_alternatives: Vec::new(),
            content_layout: None,
            copy_patterns: Vec::new(),
        }],
        frameworks: vec![],
        tools: vec![],
    }
}

fn dummy_manifest(plugin_id: &str, pkg_rel_path: &str, with_capability: bool) -> PluginManifest {
    let caps = if with_capability {
        r#"["game:define"]"#
    } else {
        "[]"
    };
    let json = format!(
        r#"{{
        "manifest": 1,
        "id": "{plugin_id}",
        "name": "{plugin_id}",
        "version": "0.1.0",
        "license": "GPL-3.0-only",
        "apiRange": ">=0.1, <0.2",
        "capabilities": {{
            "required": {caps}
        }},
        "contributions": {{
            "gamePackages": [
                {{ "path": "{pkg_rel_path}" }}
            ]
        }}
    }}"#
    );
    serde_json::from_str(&json).unwrap()
}

#[test]
fn startup_loads_enabled_granted_plugin_package() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = AppPaths::from_root(tmp.path().to_path_buf());
    paths.create_required_dirs().unwrap();
    agora_core::db::init_local_state_db(&paths.local_state_db()).unwrap();
    let conn = agora_core::db::local_state_connection(&paths.local_state_db()).unwrap();

    agora_core::db::set_setting(
        &conn,
        PLUGINS_ENABLED_SETTING,
        &serde_json::Value::Bool(true),
    )
    .unwrap();

    let plugin_dir = tmp.path().join("plugins").join("test-tracer");
    std::fs::create_dir_all(plugin_dir.join("games")).unwrap();
    let pkg_json = serde_json::to_string(&dummy_package_def("tracer-game")).unwrap();
    std::fs::write(plugin_dir.join("games/package.json"), pkg_json).unwrap();

    let manifest = dummy_manifest("test.tracer", "games/package.json", true);
    let mut granted = CapabilitySet::new();
    granted.insert(Capability::GameDefine);

    store::upsert(
        &conn,
        &manifest,
        &granted,
        &PluginSource::Package,
        &plugin_dir,
        true,
        "2026-10-03T00:00:00Z",
    )
    .unwrap();
    drop(conn);

    let (ctx, warnings) = CoreContext::initialize(paths, GameRegistry::builder()).unwrap();
    let game_id = GameId::new("tracer-game").unwrap();
    assert!(ctx.games.game(&game_id).is_some());
    assert_eq!(
        ctx.games.source_for(&game_id),
        Some(&PackageSource::Plugin {
            plugin_id: "test.tracer".into()
        })
    );
    assert!(!warnings.iter().any(|w| w.contains("test.tracer")));
}

#[test]
fn startup_silently_ignores_when_plugin_system_disabled() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = AppPaths::from_root(tmp.path().to_path_buf());
    paths.create_required_dirs().unwrap();
    agora_core::db::init_local_state_db(&paths.local_state_db()).unwrap();
    let conn = agora_core::db::local_state_connection(&paths.local_state_db()).unwrap();

    // Plugin system disabled
    agora_core::db::set_setting(
        &conn,
        PLUGINS_ENABLED_SETTING,
        &serde_json::Value::Bool(false),
    )
    .unwrap();

    let plugin_dir = tmp.path().join("plugins").join("test-tracer");
    std::fs::create_dir_all(plugin_dir.join("games")).unwrap();
    let pkg_json = serde_json::to_string(&dummy_package_def("tracer-game")).unwrap();
    std::fs::write(plugin_dir.join("games/package.json"), pkg_json).unwrap();

    let manifest = dummy_manifest("test.tracer", "games/package.json", true);
    let mut granted = CapabilitySet::new();
    granted.insert(Capability::GameDefine);

    store::upsert(
        &conn,
        &manifest,
        &granted,
        &PluginSource::Package,
        &plugin_dir,
        true,
        "2026-10-03T00:00:00Z",
    )
    .unwrap();
    drop(conn);

    let (ctx, warnings) = CoreContext::initialize(paths, GameRegistry::builder()).unwrap();
    let game_id = GameId::new("tracer-game").unwrap();
    assert!(ctx.games.game(&game_id).is_none());
    assert!(!warnings.iter().any(|w| w.contains("test.tracer")));
}

#[test]
fn startup_silently_ignores_disabled_plugin() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = AppPaths::from_root(tmp.path().to_path_buf());
    paths.create_required_dirs().unwrap();
    agora_core::db::init_local_state_db(&paths.local_state_db()).unwrap();
    let conn = agora_core::db::local_state_connection(&paths.local_state_db()).unwrap();

    agora_core::db::set_setting(
        &conn,
        PLUGINS_ENABLED_SETTING,
        &serde_json::Value::Bool(true),
    )
    .unwrap();

    let plugin_dir = tmp.path().join("plugins").join("test-tracer");
    std::fs::create_dir_all(plugin_dir.join("games")).unwrap();
    let pkg_json = serde_json::to_string(&dummy_package_def("tracer-game")).unwrap();
    std::fs::write(plugin_dir.join("games/package.json"), pkg_json).unwrap();

    let manifest = dummy_manifest("test.tracer", "games/package.json", true);
    let mut granted = CapabilitySet::new();
    granted.insert(Capability::GameDefine);

    store::upsert(
        &conn,
        &manifest,
        &granted,
        &PluginSource::Package,
        &plugin_dir,
        false, // disabled!
        "2026-10-03T00:00:00Z",
    )
    .unwrap();
    drop(conn);

    let (ctx, warnings) = CoreContext::initialize(paths, GameRegistry::builder()).unwrap();
    let game_id = GameId::new("tracer-game").unwrap();
    assert!(ctx.games.game(&game_id).is_none());
    assert!(!warnings.iter().any(|w| w.contains("test.tracer")));
}

#[test]
fn startup_silently_ignores_plugin_without_granted_game_define() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = AppPaths::from_root(tmp.path().to_path_buf());
    paths.create_required_dirs().unwrap();
    agora_core::db::init_local_state_db(&paths.local_state_db()).unwrap();
    let conn = agora_core::db::local_state_connection(&paths.local_state_db()).unwrap();

    agora_core::db::set_setting(
        &conn,
        PLUGINS_ENABLED_SETTING,
        &serde_json::Value::Bool(true),
    )
    .unwrap();

    let plugin_dir = tmp.path().join("plugins").join("test-tracer");
    std::fs::create_dir_all(plugin_dir.join("games")).unwrap();
    let pkg_json = serde_json::to_string(&dummy_package_def("tracer-game")).unwrap();
    std::fs::write(plugin_dir.join("games/package.json"), pkg_json).unwrap();

    let manifest = dummy_manifest("test.tracer", "games/package.json", true);
    let granted = CapabilitySet::new(); // Not granted game:define

    store::upsert(
        &conn,
        &manifest,
        &granted,
        &PluginSource::Package,
        &plugin_dir,
        true,
        "2026-10-03T00:00:00Z",
    )
    .unwrap();
    drop(conn);

    let (ctx, warnings) = CoreContext::initialize(paths, GameRegistry::builder()).unwrap();
    let game_id = GameId::new("tracer-game").unwrap();
    assert!(ctx.games.game(&game_id).is_none());
    assert!(!warnings.iter().any(|w| w.contains("test.tracer")));
}

#[test]
fn startup_warns_on_missing_file_and_succeeds() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = AppPaths::from_root(tmp.path().to_path_buf());
    paths.create_required_dirs().unwrap();
    agora_core::db::init_local_state_db(&paths.local_state_db()).unwrap();
    let conn = agora_core::db::local_state_connection(&paths.local_state_db()).unwrap();

    agora_core::db::set_setting(
        &conn,
        PLUGINS_ENABLED_SETTING,
        &serde_json::Value::Bool(true),
    )
    .unwrap();

    let plugin_dir = tmp.path().join("plugins").join("test-tracer");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    // No package file written

    let manifest = dummy_manifest("test.tracer", "games/package.json", true);
    let mut granted = CapabilitySet::new();
    granted.insert(Capability::GameDefine);

    store::upsert(
        &conn,
        &manifest,
        &granted,
        &PluginSource::Package,
        &plugin_dir,
        true,
        "2026-10-03T00:00:00Z",
    )
    .unwrap();
    drop(conn);

    let (ctx, warnings) = CoreContext::initialize(paths, GameRegistry::builder()).unwrap();
    let game_id = GameId::new("tracer-game").unwrap();
    assert!(ctx.games.game(&game_id).is_none());
    assert!(warnings
        .iter()
        .any(|w| w.contains("test.tracer") && w.contains("cannot read game package")));
}

#[test]
fn startup_warns_on_bad_json_and_succeeds() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = AppPaths::from_root(tmp.path().to_path_buf());
    paths.create_required_dirs().unwrap();
    agora_core::db::init_local_state_db(&paths.local_state_db()).unwrap();
    let conn = agora_core::db::local_state_connection(&paths.local_state_db()).unwrap();

    agora_core::db::set_setting(
        &conn,
        PLUGINS_ENABLED_SETTING,
        &serde_json::Value::Bool(true),
    )
    .unwrap();

    let plugin_dir = tmp.path().join("plugins").join("test-tracer");
    std::fs::create_dir_all(plugin_dir.join("games")).unwrap();
    std::fs::write(plugin_dir.join("games/package.json"), "{ bad json").unwrap();

    let manifest = dummy_manifest("test.tracer", "games/package.json", true);
    let mut granted = CapabilitySet::new();
    granted.insert(Capability::GameDefine);

    store::upsert(
        &conn,
        &manifest,
        &granted,
        &PluginSource::Package,
        &plugin_dir,
        true,
        "2026-10-03T00:00:00Z",
    )
    .unwrap();
    drop(conn);

    let (ctx, warnings) = CoreContext::initialize(paths, GameRegistry::builder()).unwrap();
    let game_id = GameId::new("tracer-game").unwrap();
    assert!(ctx.games.game(&game_id).is_none());
    assert!(warnings
        .iter()
        .any(|w| w.contains("test.tracer") && w.contains("failed to parse game package")));
}

#[test]
fn startup_warns_on_game_id_collision_with_compiled_and_succeeds() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = AppPaths::from_root(tmp.path().to_path_buf());
    paths.create_required_dirs().unwrap();
    agora_core::db::init_local_state_db(&paths.local_state_db()).unwrap();
    let conn = agora_core::db::local_state_connection(&paths.local_state_db()).unwrap();

    agora_core::db::set_setting(
        &conn,
        PLUGINS_ENABLED_SETTING,
        &serde_json::Value::Bool(true),
    )
    .unwrap();

    let plugin_dir = tmp.path().join("plugins").join("test-tracer");
    std::fs::create_dir_all(plugin_dir.join("games")).unwrap();
    let pkg_json = serde_json::to_string(&dummy_package_def("compiled-game")).unwrap();
    std::fs::write(plugin_dir.join("games/package.json"), pkg_json).unwrap();

    let manifest = dummy_manifest("test.tracer", "games/package.json", true);
    let mut granted = CapabilitySet::new();
    granted.insert(Capability::GameDefine);

    store::upsert(
        &conn,
        &manifest,
        &granted,
        &PluginSource::Package,
        &plugin_dir,
        true,
        "2026-10-03T00:00:00Z",
    )
    .unwrap();
    drop(conn);

    let mut builder = GameRegistry::builder();
    builder
        .add(
            PackageSource::Compiled {
                crate_name: "test-compiled".into(),
            },
            Arc::new(DeclarativePackage(dummy_package_def("compiled-game"))),
        )
        .unwrap();

    let (ctx, warnings) = CoreContext::initialize(paths, builder).unwrap();
    let game_id = GameId::new("compiled-game").unwrap();
    assert!(ctx.games.game(&game_id).is_some());
    assert_eq!(
        ctx.games.source_for(&game_id),
        Some(&PackageSource::Compiled {
            crate_name: "test-compiled".into()
        })
    );
    assert!(warnings
        .iter()
        .any(|w| w.contains("test.tracer") && w.contains("rejected game package")));
}

/// The manifest stored at install was validated, but the database copy is
/// user-writable: a package path that leaves the plugin folder is refused at
/// load with a warning, and nothing outside the plugin is read.
#[test]
fn a_tampered_package_path_is_refused_at_load() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = AppPaths::from_root(tmp.path().to_path_buf());
    paths.create_required_dirs().unwrap();
    agora_core::db::init_local_state_db(&paths.local_state_db()).unwrap();
    let conn = agora_core::db::local_state_connection(&paths.local_state_db()).unwrap();
    agora_core::db::set_setting(
        &conn,
        PLUGINS_ENABLED_SETTING,
        &serde_json::Value::Bool(true),
    )
    .unwrap();
    let plugin_dir = tmp.path().join("plugins").join("sly");
    std::fs::create_dir_all(plugin_dir.join("games")).unwrap();
    // A valid package sits outside the plugin, where a tampered path points.
    let outside = serde_json::to_string(&dummy_package_def("outside-game")).unwrap();
    std::fs::write(tmp.path().join("plugins").join("outside.json"), outside).unwrap();
    let mut granted = CapabilitySet::new();
    granted.insert(Capability::GameDefine);
    store::upsert(
        &conn,
        &dummy_manifest("test.sly", "games/package.json", true),
        &granted,
        &PluginSource::Package,
        &plugin_dir,
        true,
        "2026-10-04T00:00:00Z",
    )
    .unwrap();
    let changed = conn
        .execute(
            "UPDATE plugin_installs SET manifest_json = replace(manifest_json, 'games/package.json', '../outside.json')",
            [],
        )
        .unwrap();
    assert_eq!(changed, 1);
    drop(conn);

    let (ctx, warnings) = CoreContext::initialize(paths, GameRegistry::builder()).unwrap();
    assert!(ctx
        .games
        .game(&GameId::new("outside-game").unwrap())
        .is_none());
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("test.sly") && w.contains("leaves the plugin folder")),
        "{warnings:?}"
    );
}
