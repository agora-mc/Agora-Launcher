//! Creation Engine (Skyrim SE) support for Agora (MASTER_SPEC §26.11-§26.12).

use agora_game_api::{GamePackage, PackageDefinition};
use std::sync::{Arc, OnceLock};

struct CreationEnginePackage(PackageDefinition);

impl GamePackage for CreationEnginePackage {
    fn definition(&self) -> &PackageDefinition {
        &self.0
    }
}

/// The compiled game package for Creation Engine games (Skyrim Special Edition).
pub fn game_package() -> Arc<dyn GamePackage> {
    static PACKAGE: OnceLock<Arc<dyn GamePackage>> = OnceLock::new();
    PACKAGE
        .get_or_init(|| {
            let json_str = include_str!("../data/package.json");
            let def: PackageDefinition = serde_json::from_str(json_str)
                .expect("failed to deserialize agora-game-creation package.json");
            Arc::new(CreationEnginePackage(def))
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use agora_game_api::{DeploymentStrategy, GameId, GamePath, RelPath, StoreId, VersionSource};

    #[test]
    fn parses_embedded_package_json() {
        let pkg = game_package();
        let def = pkg.definition();

        assert_eq!(def.id, "agora.creation-engine");
        assert_eq!(def.version.major, 0);
        assert_eq!(def.version.minor, 1);
        assert_eq!(def.version.patch, 0);
        assert!(def.api_range.matches(&agora_game_api::GAME_API_VERSION));
        assert_eq!(def.games.len(), 1);

        let game = &def.games[0];
        assert_eq!(game.id, GameId::new("skyrim-se").unwrap());
        assert_eq!(game.name, "The Elder Scrolls V: Skyrim Special Edition");
        assert_eq!(game.deployment, DeploymentStrategy::VirtualFileSystem);

        // Stores
        assert_eq!(game.stores.len(), 2);
        assert_eq!(game.stores[0].store, StoreId::steam());
        assert_eq!(game.stores[0].product, "489830");
        assert_eq!(game.stores[1].store, StoreId::gog());
        assert_eq!(game.stores[1].product, "1711230643");

        // Version sources
        assert_eq!(
            game.version_sources,
            vec![
                VersionSource::Executable {
                    path: RelPath::new("SkyrimSE.exe").unwrap()
                },
                VersionSource::StoreRecord,
            ]
        );

        // Native code & archives
        assert_eq!(
            game.native_code_patterns,
            vec![RelPath::new("Data/SKSE/Plugins/*.dll").unwrap()]
        );
        assert_eq!(
            game.linked_archive_patterns,
            vec!["Data/*.bsa", "Data/*.esm", "Data/*.esl", "Data/*.bik",]
        );
        assert_eq!(game.declared_writes, vec!["d3dx9_42.log"]);
        assert!(game.is_declared_write("d3dx9_42.log"));
        assert!(game.is_declared_write("D3DX9_42.LOG"));

        assert_eq!(game.excluded_paths, vec!["Data/SSEEdit Backups/**"]);
        assert!(game.is_excluded("Data/SSEEdit Backups/x.esm.backup"));
        assert!(game.is_excluded("Data/SSEEdit Backups/sub/x.esm.backup"));
        assert!(!game.is_excluded("Data/Skyrim.esm"));

        // Launch recipe
        let launch = game.launch.as_ref().expect("launch recipe should be Some");
        assert_eq!(
            launch.executable,
            GamePath::Runtime {
                path: RelPath::new("SkyrimSE.exe").unwrap()
            }
        );
        assert_eq!(
            launch.working_directory,
            GamePath::Runtime {
                path: RelPath::default()
            }
        );
        assert!(launch.arguments.is_empty());
        assert!(launch.environment.is_empty());

        // User files
        assert_eq!(game.user_files.len(), 8);
        assert_eq!(
            game.user_files[0].source,
            GamePath::UserData {
                location: agora_game_api::UserDataLocation::LocalAppData,
                path: RelPath::new("Skyrim Special Edition/Plugins.txt").unwrap(),
            }
        );
        assert_eq!(game.user_files[0].instance_path, "user/Plugins.txt");
        assert_eq!(
            game.user_files[0].strategy,
            agora_game_api::UserFileStrategy::JournaledSwap
        );
        assert_eq!(game.user_files[0].stores, vec![StoreId::steam()]);

        assert_eq!(
            game.user_files[4].source,
            GamePath::UserData {
                location: agora_game_api::UserDataLocation::LocalAppData,
                path: RelPath::new("Skyrim Special Edition GOG/Plugins.txt").unwrap(),
            }
        );
        assert_eq!(game.user_files[4].instance_path, "user/Plugins.txt");
        assert_eq!(
            game.user_files[4].strategy,
            agora_game_api::UserFileStrategy::JournaledSwap
        );
        assert_eq!(game.user_files[4].stores, vec![StoreId::gog()]);

        // Plugin activation: masters, then light plugins, then plain plugins
        let rule = game
            .plugin_list
            .as_ref()
            .expect("Skyrim keeps a plugin list");
        assert_eq!(rule.user_file, "user/Plugins.txt");
        assert_eq!(rule.plugin_folder, "Data");
        assert_eq!(rule.patterns, vec!["*.esm", "*.esl", "*.esp"]);
        assert_eq!(rule.active_prefix, "*");
        assert_eq!(rule.header.len(), 2);
        assert!(rule.header.iter().all(|l| l.starts_with('#')));

        // Framework loaders
        assert_eq!(game.launch_alternatives.len(), 1);
        let skse = &game.launch_alternatives[0];
        assert_eq!(skse.id, "skse");
        assert_eq!(skse.when_present, "skse64_loader.exe");
        assert_eq!(
            skse.executable,
            GamePath::Runtime {
                path: RelPath::new("skse64_loader.exe").unwrap()
            }
        );
        assert!(!skse.reason.is_empty());

        // Empty lists
        assert!(game.content_rules.is_empty());
        // SKSE is the one framework Skyrim SE declares (MASTER_SPEC §26.11).
        assert_eq!(
            game.framework_ids,
            vec![agora_game_api::FrameworkId::new("skse").unwrap()]
        );
        assert_eq!(def.frameworks.len(), 1);
        assert_eq!(
            def.frameworks[0].id,
            agora_game_api::FrameworkId::new("skse").unwrap()
        );
        assert_eq!(def.frameworks[0].game, GameId::new("skyrim-se").unwrap());
        assert_eq!(def.frameworks[0].name, "Skyrim Script Extender");
        // Nemesis and BodySlide are the tools Skyrim SE declares (MASTER_SPEC §26.9): Nemesis's engine
        // sits in the deployed Nemesis mod folder and runs from there. Both read the install path,
        // so both run by swap (MASTER_SPEC §26.9, slice 4c).
        assert_eq!(
            game.tool_ids,
            vec![
                agora_game_api::ToolId::new("nemesis").unwrap(),
                agora_game_api::ToolId::new("bodyslide").unwrap(),
            ]
        );
        assert_eq!(def.tools.len(), 2);
        let nemesis = &def.tools[0];
        assert!(nemesis.uses_install_path);
        let bodyslide = &def.tools[1];
        assert_eq!(
            bodyslide.id,
            agora_game_api::ToolId::new("bodyslide").unwrap()
        );
        assert!(bodyslide.uses_install_path);
        assert_eq!(
            bodyslide.launch.executable,
            GamePath::Runtime {
                path: RelPath::new("Data/CalienteTools/BodySlide/BodySlide x64.exe").unwrap()
            }
        );
        assert_eq!(nemesis.id, agora_game_api::ToolId::new("nemesis").unwrap());
        assert_eq!(nemesis.game, GameId::new("skyrim-se").unwrap());
        assert_eq!(
            nemesis.launch.executable,
            GamePath::Runtime {
                path: RelPath::new("Data/Nemesis_Engine/Nemesis Unlimited Behavior Engine.exe")
                    .unwrap()
            }
        );
        assert_eq!(
            nemesis.launch.working_directory,
            GamePath::Runtime {
                path: RelPath::new("Data/Nemesis_Engine").unwrap()
            }
        );
        assert!(nemesis.relevant_settings.is_empty());
        assert!(nemesis.after_tools.is_empty());
        // Nemesis's output checks (MASTER_SPEC §26.9, slice 4d): the three behaviour files a run must
        // leave, and the line its PatchLog gets when it fails although it exits 0. BodySlide declares
        // neither: it builds what the user picks.
        assert_eq!(
            nemesis.required_outputs,
            vec![
                "Data/meshes/actors/character/behaviors/0_master.hkx".to_string(),
                "Data/meshes/actors/character/characters/defaultmale.hkx".to_string(),
                "Data/meshes/actors/character/characters female/defaultfemale.hkx".to_string(),
            ]
        );
        assert_eq!(nemesis.failure_markers.len(), 1);
        assert_eq!(
            nemesis.failure_markers[0].file.as_str(),
            "Data/Nemesis_Engine/PatchLog.txt"
        );
        assert_eq!(
            nemesis.failure_markers[0].contains,
            "Failed to generate behavior"
        );
        assert!(bodyslide.required_outputs.is_empty());
        assert!(bodyslide.failure_markers.is_empty());
        assert!(game.log_paths.is_empty());
        assert!(game.crash_paths.is_empty());
        assert!(game.save_paths.is_empty());
    }

    /// The three runtime-file rules the embedded Skyrim SE definition declares.
    fn skyrim_runtime_rules() -> Vec<agora_game_api::RuntimeFileRule> {
        game_package().definition().games[0].runtime_files.clone()
    }

    /// Paths, `/`-separated and relative to `root`, of every file under `root`. Read-only.
    fn files_under(root: &std::path::Path) -> Vec<String> {
        fn visit(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).expect("read the folder") {
                let path = entry.expect("read a directory entry").path();
                if path.is_dir() {
                    visit(root, &path, out);
                } else {
                    let rel = path.strip_prefix(root).expect("path under root");
                    let parts: Vec<String> = rel
                        .components()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .collect();
                    out.push(parts.join("/"));
                }
            }
        }
        let mut out = Vec::new();
        visit(root, root, &mut out);
        out.sort();
        out
    }

    #[test]
    fn skyrim_runtime_rules_match_the_skse_and_address_library_files_of_1_6_1170() {
        let rules = skyrim_runtime_rules();
        assert_eq!(
            rules.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["skse", "address-library", "address-library-se"]
        );

        // The working Skyrim SE 1.6.1170.0 instance, with names as they are on disk.
        let files = [
            "SkyrimSE.exe",
            "skse64_1_6_1170.dll",
            "skse64_loader.exe",
            "skse64_readme.txt",
            "Data/SKSE/Plugins/versionlib-1-6-1170-0.bin",
            "Data/SKSE/Plugins/versionlib-1-6-1170-0-1.bin",
            "Data/Scripts/PO3_SKSEFunctions.pex",
        ];
        assert!(agora_game_api::check_runtime_files(&rules, "1.6.1170.0", &files).is_empty());

        // The same files against a newer runtime: SKSE and the 1.6 library are both missing.
        let findings = agora_game_api::check_runtime_files(&rules, "1.6.1179.0", &files);
        let ids: Vec<&str> = findings.iter().map(|f| f.rule_id.as_str()).collect();
        assert_eq!(ids, vec!["skse", "address-library"]);

        // Against 1.5.97.0: SKSE and the 1.5 library are missing.
        let findings = agora_game_api::check_runtime_files(&rules, "1.5.97.0", &files);
        let ids: Vec<&str> = findings.iter().map(|f| f.rule_id.as_str()).collect();
        assert_eq!(ids, vec!["skse", "address-library-se"]);
    }

    /// Read-only check of the real Skyrim SE deployment folder from the P3 benchmark. It prints what
    /// the rules say; run it with `cargo test -p agora-game-creation -- --ignored --nocapture`.
    #[test]
    #[ignore = "reads the real deployment folder under D:\\Agora-bench\\p3-done"]
    fn real_skyrim_deployment_folder_checks_out_against_its_runtime() {
        let root = std::path::Path::new(r"D:\Agora-bench\p3-done\bases\deployments\p3-modded\game");
        assert!(root.is_dir(), "missing {}", root.display());
        let files = files_under(root);
        println!("{} files under {}", files.len(), root.display());
        let rules = skyrim_runtime_rules();

        let at_1_6_1170 = agora_game_api::check_runtime_files(&rules, "1.6.1170.0", &files);
        println!("1.6.1170.0: {} finding(s)", at_1_6_1170.len());
        for finding in &at_1_6_1170 {
            println!("  {}", finding.summary());
        }
        assert!(at_1_6_1170.is_empty(), "{at_1_6_1170:?}");

        let at_1_6_1179 = agora_game_api::check_runtime_files(&rules, "1.6.1179.0", &files);
        println!("1.6.1179.0: {} finding(s)", at_1_6_1179.len());
        for finding in &at_1_6_1179 {
            println!("  {}", finding.summary());
        }
        let ids: Vec<&str> = at_1_6_1179.iter().map(|f| f.rule_id.as_str()).collect();
        assert_eq!(ids, vec!["skse", "address-library"]);
    }

    /// Read-only check of the Nemesis run that failed on 2026-10-09 (MASTER_SPEC §26.9, slice 4d). The
    /// behaviour files the package requires are in generation 1, under the names and casing on disk;
    /// the failure marker is in generation 2, the run that exited 0 and failed. Run with
    /// `cargo test -p agora-game-creation -- --ignored --nocapture`.
    #[test]
    #[ignore = "reads the real Nemesis output under D:\\Agora-bench\\p3-done"]
    fn real_nemesis_generation_one_has_the_declared_outputs_and_generation_two_the_marker() {
        let package = game_package();
        let nemesis = package
            .definition()
            .tools
            .iter()
            .find(|t| t.id.as_str() == "nemesis")
            .expect("the package declares nemesis");
        let root = std::path::Path::new(
            r"D:\Agora-bench\p3-done\instances\grounded-apocalypse-mo2-import-6d97f2\generated\nemesis",
        );
        assert!(root.is_dir(), "missing {}", root.display());

        let generation_one = files_under(&root.join("1"));
        println!("{} files in generation 1", generation_one.len());
        for pattern in &nemesis.required_outputs {
            let wanted = pattern.to_ascii_lowercase();
            let found = generation_one
                .iter()
                .any(|file| agora_game_api::glob_match(&wanted, &file.to_ascii_lowercase()));
            println!(
                "required {pattern}: {}",
                if found { "found" } else { "MISSING" }
            );
            assert!(found, "{pattern} is not in generation 1");
        }

        let marker = &nemesis.failure_markers[0];
        let text = std::fs::read_to_string(root.join("2").join(marker.file.as_str()))
            .expect("generation 2 has the marker's file");
        assert!(
            text.contains(&marker.contains),
            "{} does not contain '{}'",
            marker.file.as_str(),
            marker.contains
        );
    }
}
