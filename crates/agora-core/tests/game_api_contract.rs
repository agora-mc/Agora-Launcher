//! Serialization fixtures for the compiled/script contract. JSON lives in this
//! consumer's tests so agora-game-api keeps its three-dependency boundary.
use agora_game_api::{GamePackage, GamePath, LaunchValue, PackageDefinition};
use serde_json::json;

struct DeclarativePackage(PackageDefinition);

impl GamePackage for DeclarativePackage {
    fn definition(&self) -> &PackageDefinition {
        &self.0
    }
}

#[test]
fn declarative_skyrim_and_valheim_packages_need_no_discovered_install_ids() {
    for (game, strategy, executable, framework, version) in [
        (
            "skyrim",
            "virtual_file_system",
            "skse64_loader.exe",
            "skse",
            "1.6.1170.0",
        ),
        ("valheim", "redirect", "valheim.exe", "bepinex", "0.218.15"),
    ] {
        let launch = json!({
            "executable": {"root": "runtime", "path": executable},
            "arguments": [],
            "environment": {
                "MOD_DIRECTORY": {"kind": "path", "path": {"root": "instance", "path": "mods"},
                    "prefix": "", "suffix": ""}
            },
            "working_directory": {"root": "runtime", "path": ""}
        });
        let definition = json!({
            "id": format!("community.{game}"), "version": "0.1.0", "api_range": ">=0.1, <0.2",
            "parents": [],
            "games": [{
                "id": game, "name": game, "stores": [{"store": "steam", "product": "store-id"}],
                "version_sources": [{"kind": "executable", "path": executable}],
                "deployment": strategy,
                "content_rules": [{"source_pattern": "**/*", "destination": "mods", "content_kind": "mod"}],
                "native_code_patterns": ["mods/**/*.dll"], "framework_ids": [framework],
                "tool_ids": ["patcher"], "launch": launch,
                "log_paths": [{"root": "user_data", "location": "local_app_data", "path": "game/logs"}],
                "crash_paths": [], "user_files": [], "save_paths": [], "linked_archive_patterns": ["*.bsa"]
            }],
            "frameworks": [{
                "id": framework, "game": game, "name": framework, "version": "2.2.6",
                "supported_runtimes": [{"game": game, "stores": ["steam"], "builds": [],
                    "versions": {"kind": "exact", "value": [version]}}],
                "required_frameworks": [], "content": ["managed-framework-content"], "launch": null
            }],
            "tools": [{
                "id": "patcher", "game": game, "name": "Patcher", "launch": launch,
                "input_layers": ["mod-1"], "relevant_settings": ["animation"],
                "output_layer": "patcher-output", "after_tools": ["body-builder"]
            }]
        });
        let package = DeclarativePackage(serde_json::from_value(definition.clone()).unwrap());
        let registered: &dyn GamePackage = &package;
        assert_eq!(
            serde_json::to_value(registered.definition()).unwrap(),
            definition
        );
        assert!(matches!(
            registered.definition().games[0]
                .launch
                .as_ref()
                .unwrap()
                .executable,
            GamePath::Runtime { .. }
        ));
        assert_eq!(
            registered.definition().games[0].deployment,
            if game == "skyrim" {
                agora_game_api::DeploymentStrategy::VirtualFileSystem
            } else {
                agora_game_api::DeploymentStrategy::Redirect
            }
        );
    }
}

#[test]
fn minecraft_classpath_can_reference_downloads_without_cache_paths() {
    let value = json!({"kind": "path_list", "prefix": "", "paths": [
        {"root": "artifact", "artifact": "downloaded-library"},
        {"root": "instance", "path": "versions/client.jar"}
    ]});
    let argument: LaunchValue = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(&argument).unwrap(), value);
    assert!(matches!(argument, LaunchValue::PathList { paths, .. }
        if matches!(paths[0], GamePath::Artifact { .. })));
}

#[test]
fn test_validated_identifiers_and_rel_path_serde() {
    use agora_game_api::{GameId, LayerId, RelPath};

    assert!(serde_json::from_str::<GameId>("\"../bad\"").is_err());
    assert_eq!(
        serde_json::from_str::<GameId>("\"minecraft\"").unwrap(),
        GameId::minecraft()
    );
    assert_eq!(
        serde_json::to_string(&GameId::minecraft()).unwrap(),
        "\"minecraft\""
    );

    assert_eq!(
        serde_json::from_str::<LayerId>("\"minecraft:mod\"")
            .unwrap()
            .as_str(),
        "minecraft:mod"
    );
    assert_eq!(
        serde_json::to_string(&LayerId::new("minecraft:mod").unwrap()).unwrap(),
        "\"minecraft:mod\""
    );

    assert!(serde_json::from_str::<RelPath>("\"../escape\"").is_err());
    assert_eq!(
        serde_json::from_str::<RelPath>(r#""mods\\sub""#)
            .unwrap()
            .as_str(),
        "mods/sub"
    );
    assert_eq!(
        serde_json::to_string(&RelPath::new("mods/sub").unwrap()).unwrap(),
        "\"mods/sub\""
    );
}

#[test]
fn test_layer_stack_serde_validation() {
    use agora_game_api::{Layer, LayerId, LayerSource, LayerStack, RelPath};

    let invalid_json = r#"[
        {"id":"w","enabled":true,"mount_path":"","source":{"kind":"writable","path":""}},
        {"id":"c","enabled":true,"mount_path":"","source":{"kind":"content","content":"h"}}
    ]"#;
    assert!(serde_json::from_str::<LayerStack>(invalid_json).is_err());

    let stack = LayerStack::new(vec![
        Layer {
            id: LayerId::new("c").unwrap(),
            enabled: true,
            mount_path: RelPath::default(),
            source_path: RelPath::default(),
            source: LayerSource::Content {
                content: "h".into(),
            },
            whiteouts: Vec::new(),
        },
        Layer {
            id: LayerId::new("w").unwrap(),
            enabled: true,
            mount_path: RelPath::default(),
            source_path: RelPath::default(),
            source: LayerSource::Writable {
                path: RelPath::default(),
            },
            whiteouts: Vec::new(),
        },
    ])
    .unwrap();

    let json_str = serde_json::to_string(&stack).unwrap();
    let deserialized: LayerStack = serde_json::from_str(&json_str).unwrap();
    assert_eq!(deserialized, stack);
}
