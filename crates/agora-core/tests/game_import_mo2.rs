//! Importing a Mod Organizer 2 setup (MASTER_SPEC §26.10): a synthetic setup built in each test, the
//! hostile names an MO2 file can hold, the layer and plugin orders the import sets, the saves and INI
//! copies, refusals, interruption and re-run, and the proof that the setup itself is never written.
//!
//! The real 214,592-file setup is read by the `#[ignore]` tests in `crates/agora/tests`, never here.

#![cfg(windows)]

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use agora_core::content_store::{self, ContentSource, Mo2Provenance};
use agora_core::ctx::CoreContext;
use agora_core::game_deploy::{self, DeployMode, FileSource};
use agora_core::game_discovery::{DiscoveredInstall, InstallCapabilities};
use agora_core::game_import::{self, ImportError, Mo2Request, RunOptions};
use agora_core::game_instance;
use agora_core::game_plugins;
use agora_core::game_registry::{
    GameInventory, GameRegistry, IdentifiedInstall, PackageSource, RuntimeResolution,
};
use agora_core::game_saves;
use agora_game_api::{
    ContentLayout, GameDefinition, GameId, GamePackage, GamePath, InputFingerprint, InstallId,
    InstallKind, LaunchRecipe, LayerSource, PackageDefinition, PluginListRule, RelPath,
    RuntimeIdentity, StoreId, ToolDefinition, ToolId,
};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

const GAME: &str = "skyrim-se";
const NEMESIS_PATTERNS: [&str; 7] = [
    "Data/meshes/actors/character/**",
    "Data/meshes/animationdatasinglefile.txt",
    "Data/meshes/animationsetdatasinglefile.txt",
    "Data/Nemesis_Engine/**",
    "Data/scripts/FNIS.pex",
    "Data/scripts/FNIS_aa.pex",
    "Data/scripts/Nemesis_AA_Core.pex",
];

/// The user-data folders are redirected by `AGORA_TEST_USER_DATA_ROOT`, a process-wide setting.
static USER_DATA: Mutex<()> = Mutex::new(());

struct Fixture {
    _lock: MutexGuard<'static, ()>,
    _tmp: TempDir,
    _user_data: TempDir,
    ctx: CoreContext,
    def: GameDefinition,
    inventory: GameInventory,
    /// The MO2 setup folder: `ModOrganizer.ini`, `mods`, `overwrite`, `profiles`.
    setup: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::env::remove_var("AGORA_TEST_USER_DATA_ROOT");
    }
}

struct TestPackage(PackageDefinition);

impl GamePackage for TestPackage {
    fn definition(&self) -> &PackageDefinition {
        &self.0
    }
}

fn write(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, bytes).unwrap();
}

/// Skyrim SE as the shipped package declares it, for the parts an import reads: the MO2 name, the
/// plugin list, the content layout, the Nemesis tool with its output patterns, and the user files.
fn skyrim_like() -> GameDefinition {
    let mut def = common::skyrim_definition();
    def.mo2_game_name = Some("Skyrim Special Edition".into());
    def.content_layout = Some(ContentLayout {
        data_path: RelPath::new("Data").unwrap(),
        data_markers: vec![],
        root_markers: vec![],
        thunderstore_bepinex: false,
    });
    def.plugin_list = Some(PluginListRule {
        user_file: RelPath::new("user/Plugins.txt").unwrap(),
        plugin_folder: RelPath::new("Data").unwrap(),
        patterns: vec!["*.esm".into(), "*.esl".into(), "*.esp".into()],
        active_prefix: "*".into(),
        header: vec![
            "# This file is used by Skyrim to keep track of your downloaded content.".into(),
            "# Please do not modify this file.".into(),
        ],
        semantics: None,
        implicit: vec!["Skyrim.esm".into(), "Update.esm".into()],
        implicit_list_file: None,
    });
    def.tool_ids = vec![ToolId::new("nemesis").unwrap()];
    def.launch = Some(LaunchRecipe {
        executable: GamePath::Runtime {
            path: RelPath::new("Game.exe").unwrap(),
        },
        arguments: vec![],
        environment: Default::default(),
        working_directory: GamePath::Runtime {
            path: RelPath::default(),
        },
    });
    def
}

fn nemesis() -> ToolDefinition {
    ToolDefinition {
        output_patterns: NEMESIS_PATTERNS.iter().map(|p| p.to_string()).collect(),
        id: ToolId::new("nemesis").unwrap(),
        game: GameId::new(GAME).unwrap(),
        name: "Nemesis".into(),
        launch: LaunchRecipe {
            executable: GamePath::Runtime {
                path: RelPath::new("Game.exe").unwrap(),
            },
            arguments: vec![],
            environment: Default::default(),
            working_directory: GamePath::Runtime {
                path: RelPath::default(),
            },
        },
        relevant_settings: vec![],
        after_tools: vec![],
    }
}

fn install_of(dir: &Path) -> IdentifiedInstall {
    let store_id = StoreId::steam();
    let runtime = RuntimeIdentity {
        game: GameId::new(GAME).unwrap(),
        store: store_id.clone(),
        version: "1.6.1170".into(),
        build: None,
    };
    let volume = agora_core::game_discovery::volume::VolumeDetector::new().get_volume_info(dir);
    let discovered = DiscoveredInstall {
        store: store_id,
        product: "489830".into(),
        name: "Skyrim Special Edition".into(),
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
        install_id: InstallId::new("steam:489830").unwrap(),
        discovered,
        add_ons: vec![],
        runtime: RuntimeResolution::Identified {
            runtime,
            source: "executable".into(),
        },
    }
}

/// The MO2 `gamePath` value as MO2 writes it: backslashes doubled inside `@ByteArray(...)`.
fn game_path_value(path: &Path) -> String {
    format!(
        "@ByteArray({})",
        path.to_string_lossy().replace('\\', "\\\\")
    )
}

fn build_setup(setup: &Path, game_path: &Path) {
    write(
        &setup.join("ModOrganizer.ini"),
        format!(
            "[General]\r\ngameName=Skyrim Special Edition\r\nselected_profile=@ByteArray(Test Profile)\r\ngamePath={}\r\n[Settings]\r\nprofiles_directory=%BASE_DIR%/profiles\r\n",
            game_path_value(game_path)
        )
        .as_bytes(),
    );

    // Alpha: enabled, highest-priority mod with provenance; it has a plugin and a shared file.
    write(&setup.join("mods/Alpha/Alpha.esp"), b"ALPHA PLUGIN");
    write(&setup.join("mods/Alpha/shared.txt"), b"from alpha");
    write(
        &setup.join("mods/Alpha/meta.ini"),
        b"[General]\r\ngameName=skyrimspecialedition\r\nmodid=42\r\nversion=1.2\r\ninstallationFile=D:/downloads/Alpha-42.7z\r\nrepository=Nexus\r\n",
    );
    // Beta: disabled, no meta.ini at all.
    write(&setup.join("mods/Beta/Beta.esp"), b"BETA PLUGIN");
    write(&setup.join("mods/Beta/beta.txt"), b"beta only");
    // Gamma: enabled, lowest real mod; its meta.ini is binary garbage.
    write(&setup.join("mods/Gamma/Gamma.esp"), b"GAMMA PLUGIN");
    write(&setup.join("mods/Gamma/shared.txt"), b"from gamma");
    write(
        &setup.join("mods/Gamma/meta.ini"),
        &[0u8, 159, 146, 150, 0, 1, 2],
    );

    // A folder that a hostile name would reach: next to `mods`, so `..\outside` finds it.
    write(&setup.join("outside/secret.txt"), b"never imported");

    // The overwrite folder: Nemesis-like output, and files no tool claims.
    write(
        &setup.join("overwrite/meshes/actors/character/behaviors/idle.hkx"),
        b"HKX",
    );
    write(
        &setup.join("overwrite/Nemesis_Engine/cache/behavior_path"),
        b"CACHE",
    );
    write(&setup.join("overwrite/scripts/FNIS.pex"), b"PEX");
    write(&setup.join("overwrite/meshes/armor/helm.nif"), b"NIF");
    write(&setup.join("overwrite/scripts/custom.pex"), b"CUSTOM");
    write(&setup.join("overwrite/FNIS.esp"), b"FNIS ESP");
    write(&setup.join("overwrite/SKSE/plugin.txt"), b"SKSE");

    // The profile.
    let profile = setup.join("profiles/Test Profile");
    write(
        &profile.join("modlist.txt"),
        b"# This file was automatically generated by Mod Organizer.\r\n\
+Alpha\r\n\
+--- Textures_separator\r\n\
-Beta\r\n\
+Gamma\r\n\
*DLC: Dawnguard\r\n\
+..\\outside\r\n\
+C:\\Windows\r\n\
+bad/name\r\n\
+Missing\r\n",
    );
    write(
        &profile.join("plugins.txt"),
        b"# This file was automatically generated by Mod Organizer.\r\n*Alpha.esp\r\n*Gamma.esp\r\n*DLCX.esm\r\n",
    );
    write(
        &profile.join("loadorder.txt"),
        b"# This file was automatically generated by Mod Organizer.\r\nSkyrim.esm\r\nUpdate.esm\r\nAlpha.esp\r\nBeta.esp\r\nGamma.esp\r\nDLCX.esm\r\n",
    );
    write(
        &profile.join("lockedorder.txt"),
        b"# This file was automatically generated by Mod Organizer.\r\nGamma.esp|2\r\n",
    );
    write(
        &profile.join("settings.ini"),
        b"[General]\r\nLocalSaves=true\r\nLocalSettings=true\r\nAutomaticArchiveInvalidation=true\r\n",
    );
    write(
        &profile.join("Skyrim.ini"),
        b"[General]\r\nsLocalTest=1\r\n",
    );
    write(
        &profile.join("skyrimcustom.ini"),
        b"[Custom]\r\nsCustom=2\r\n",
    );
    write(&profile.join("saves/Save1.ess"), b"SAVE ONE");
    write(&profile.join("saves/Save1.skse"), b"SKSE CO-SAVE");
}

fn fixture() -> Fixture {
    let lock = USER_DATA.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    let user_data = TempDir::new().unwrap();
    std::env::set_var("AGORA_TEST_USER_DATA_ROOT", user_data.path());

    let def = skyrim_like();
    let tool = nemesis();
    let pkg = PackageDefinition {
        id: format!("test.{GAME}"),
        version: semver::Version::new(0, 1, 0),
        api_range: semver::VersionReq::parse(">=0.1, <0.2").unwrap(),
        parents: vec![],
        games: vec![def.clone()],
        frameworks: vec![],
        tools: vec![tool],
    };
    let mut builder = GameRegistry::builder();
    builder
        .add(
            PackageSource::Compiled {
                crate_name: "test".into(),
            },
            Arc::new(TestPackage(pkg)),
        )
        .expect("register the test package");
    let registry = Arc::new(builder.build());

    let ctx = CoreContext::for_testing(tmp.path().join("app_data"));
    agora_core::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();
    let ctx = ctx.with_games(registry);

    let game_dir = tmp.path().join("game");
    write(&game_dir.join("Data/Skyrim.esm"), b"MASTER");
    std::fs::copy(r"C:\Windows\System32\cmd.exe", game_dir.join("Game.exe")).unwrap();
    let setup = tmp.path().join("setup");
    build_setup(&setup, &game_dir);

    let inventory = GameInventory {
        installs: vec![install_of(&game_dir)],
        unsupported: vec![],
    };
    Fixture {
        _lock: lock,
        _tmp: tmp,
        _user_data: user_data,
        ctx,
        def,
        inventory,
        setup,
    }
}

fn request(f: &Fixture, name: Option<&str>, copy_saves: bool) -> Mo2Request {
    Mo2Request {
        ini_path: f.setup.join("ModOrganizer.ini"),
        profile: "Test Profile".into(),
        name: name.map(str::to_string),
        copy_saves,
    }
}

/// Every file under `root` with its SHA-256, keyed by relative path.
fn tree_hashes(root: &Path) -> BTreeMap<String, String> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .to_string();
                let bytes = std::fs::read(&path).unwrap();
                out.insert(rel, format!("{:x}", Sha256::digest(&bytes)));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn item_names(ctx: &CoreContext) -> BTreeMap<String, String> {
    content_store::list_items(ctx)
        .unwrap()
        .into_iter()
        .map(|i| (i.item_id.clone(), i.name.clone()))
        .collect()
}

fn item_id_named(ctx: &CoreContext, name: &str) -> String {
    item_names(ctx)
        .into_iter()
        .find(|(_, n)| n == name)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no content item named {name}"))
}

// ---------------------------------------------------------------------------
// Hostile names: the probe, written first
// ---------------------------------------------------------------------------

/// A modlist that names a path outside `mods`, a drive, a separator, and a folder that is not there.
/// Each is reported and none is read: the `outside` folder's file never reaches the content store.
#[test]
fn hostile_modlist_names_are_reported_and_never_read() {
    let f = fixture();
    let report = game_import::plan(&f.ctx, &f.inventory, &request(&f, None, false)).unwrap();

    let mods: Vec<&str> = report.mods.iter().map(|m| m.folder.as_str()).collect();
    assert_eq!(
        mods,
        vec!["Alpha", "Beta", "Gamma"],
        "only real folders are planned, in modlist order"
    );
    let problems: Vec<&str> = report.problems.iter().map(|p| p.entry.as_str()).collect();
    for hostile in ["..\\outside", "C:\\Windows", "bad/name", "Missing"] {
        assert!(
            problems.contains(&hostile),
            "{hostile} should be reported, got {problems:?}"
        );
    }
    assert!(
        report.problems.iter().all(|p| p.line.is_some()),
        "every problem names its modlist line"
    );

    // Nothing in the plan came from outside mods: the secret file is not counted anywhere.
    assert!(report
        .mods
        .iter()
        .all(|m| m.path.starts_with(f.setup.join("mods"))));

    // Running it stores the real mods and never the secret.
    let run = game_import::run(
        &f.ctx,
        &f.inventory,
        &request(&f, None, false),
        RunOptions::default(),
        &|_| {},
    )
    .unwrap();
    assert!(!run.interrupted);
    let stored: Vec<String> = content_store::list_items(&f.ctx)
        .unwrap()
        .iter()
        .flat_map(|i| i.files.iter().map(|file| file.path.as_str().to_string()))
        .collect();
    assert!(
        stored.iter().all(|p| !p.contains("secret")),
        "the file outside mods was read: {stored:?}"
    );
}

/// A `meta.ini` that is binary garbage is recorded as unreadable, and the mod still imports.
#[test]
fn binary_meta_ini_is_recorded_and_the_mod_still_imports() {
    let f = fixture();
    let report = game_import::plan(&f.ctx, &f.inventory, &request(&f, None, false)).unwrap();
    let gamma = report.mods.iter().find(|m| m.folder == "Gamma").unwrap();
    assert!(matches!(gamma.provenance, Mo2Provenance::Unreadable { .. }));
    let beta = report.mods.iter().find(|m| m.folder == "Beta").unwrap();
    assert_eq!(beta.provenance, Mo2Provenance::Absent);
}

// ---------------------------------------------------------------------------
// The full import
// ---------------------------------------------------------------------------

#[test]
fn import_stores_bytes_orders_layers_and_leaves_the_setup_alone() {
    let f = fixture();
    let before = tree_hashes(&f.setup);

    let run = game_import::run(
        &f.ctx,
        &f.inventory,
        &request(&f, None, false),
        RunOptions::default(),
        &|_| {},
    )
    .unwrap();
    assert!(!run.interrupted);
    assert_eq!(run.plan.mods_enabled, 2);
    assert_eq!(run.plan.mods_disabled, 1);
    assert_eq!(run.plan.separators, 1);
    assert_eq!(run.plan.unmanaged, vec!["DLC: Dawnguard".to_string()]);

    // The setup is untouched: every file hashes the same before and after.
    assert_eq!(before, tree_hashes(&f.setup), "the setup was written to");

    let instance_id = run.instance_id.clone().expect("an instance was made");
    let manifest = game_instance::get_manifest(&f.ctx, &instance_id).unwrap();
    let rec = manifest
        .imported_from
        .clone()
        .expect("the import is recorded");
    assert_eq!(rec.profile, "Test Profile");
    assert_eq!(rec.kind, "mo2");

    // Layers: Gamma lowest, then Beta (disabled), then Alpha, then the overwrite layer, then Nemesis.
    let names = item_names(&f.ctx);
    let order: Vec<String> = manifest
        .layers
        .layers()
        .iter()
        .map(|l| match &l.source {
            LayerSource::Content { content } => names.get(content).cloned().unwrap_or_default(),
            LayerSource::Generated { tool, .. } => format!("generated:{tool}"),
            other => format!("{other:?}"),
        })
        .collect();
    assert_eq!(
        order,
        vec![
            "Gamma",
            "Beta",
            "Alpha",
            "MO2 overwrite",
            "generated:nemesis"
        ]
    );
    let beta_layer = manifest
        .layers
        .layers()
        .iter()
        .find(|l| matches!(&l.source, LayerSource::Content { content } if names.get(content).map(String::as_str) == Some("Beta")))
        .unwrap();
    assert!(!beta_layer.enabled, "a disabled mod is a disabled layer");

    // Two mods with the same file: Alpha is higher, so its shared.txt is the one deployed.
    let plan = game_deploy::plan(&f.ctx, &instance_id, &f.def, DeployMode::Links).unwrap();
    let shared = plan
        .files
        .iter()
        .find(|p| p.path.as_str() == "Data/shared.txt")
        .expect("shared.txt is deployed");
    match &shared.source {
        FileSource::Content { item_id, .. } => {
            assert_eq!(
                item_id,
                &item_id_named(&f.ctx, "Alpha"),
                "the higher mod wins"
            );
        }
        other => panic!("shared.txt came from {other:?}"),
    }

    // The Nemesis output is a generated layer whose inputs are unknown; the other overwrite files
    // are in the overwrite layer.
    let generated = plan
        .files
        .iter()
        .find(|p| p.path.as_str() == "Data/meshes/actors/character/behaviors/idle.hkx")
        .expect("the Nemesis behaviour is deployed");
    assert!(matches!(generated.source, FileSource::Generated { .. }));
    let (gen_id, inputs) = match &manifest
        .layers
        .layers()
        .iter()
        .find(|l| matches!(l.source, LayerSource::Generated { .. }))
        .unwrap()
        .source
    {
        LayerSource::Generated {
            generation, inputs, ..
        } => (generation.clone(), inputs.clone()),
        _ => unreachable!(),
    };
    assert_eq!(gen_id, "1");
    assert_eq!(inputs, InputFingerprint::Unknown);
    let helm = plan
        .files
        .iter()
        .find(|p| p.path.as_str() == "Data/meshes/armor/helm.nif")
        .expect("the unclaimed mesh is deployed");
    assert!(matches!(&helm.source, FileSource::Content { .. }));
    let fnis_esp = plan
        .files
        .iter()
        .find(|p| p.path.as_str() == "Data/FNIS.esp")
        .unwrap();
    assert!(
        matches!(&fnis_esp.source, FileSource::Content { .. }),
        "FNIS.esp stays in overwrite"
    );

    // Plugins: MO2's order and activation, its lock, and Skyrim's own masters left out.
    let listed = game_plugins::list(&f.ctx, &instance_id, &f.def).unwrap();
    let entries: Vec<(&str, bool, bool, bool)> = listed
        .entries
        .iter()
        .map(|e| (e.name.as_str(), e.active, e.managed, e.locked))
        .collect();
    assert_eq!(
        entries,
        vec![
            ("Alpha.esp", true, true, false),
            ("Beta.esp", false, false, false),
            ("Gamma.esp", true, true, true),
            ("DLCX.esm", true, false, false),
        ]
    );
    assert_eq!(run.plan.plugins.locked, vec!["Gamma.esp".to_string()]);

    // Local INIs: the profile's copies are the instance's copies.
    let skyrim_ini = std::fs::read(
        f.ctx
            .paths
            .instance_dir(&instance_id)
            .unwrap()
            .join("user/Skyrim.ini"),
    )
    .unwrap();
    // The copy keeps the profile's line, and the saves choice adds its own save folder.
    let skyrim_ini = String::from_utf8(skyrim_ini).unwrap();
    assert!(skyrim_ini.contains("sLocalTest=1"), "{skyrim_ini}");
    assert!(
        skyrim_ini.contains("SLocalSavePath=Saves\\Agora\\"),
        "{skyrim_ini}"
    );
    assert_eq!(run.inis_copied.len(), 2);

    // Saves: the choice is own, and without --copy-saves nothing reached Documents.
    assert_eq!(manifest.saves, game_instance::SavesChoice::Own);
    let rule = game_saves::rule_for(&f.def, &StoreId::steam())
        .unwrap()
        .clone();
    let own = game_saves::own_folder(&rule, &instance_id).unwrap();
    assert!(
        !own.exists(),
        "saves were written to Documents without the flag"
    );
    assert_eq!(run.saves_copied, 0);
    assert!(run.next_steps.iter().any(|s| s.contains("--copy-saves")));

    // Provenance: Alpha's claim is recorded, labelled as unverified; Gamma's garbage is noted.
    let alpha = content_store::get_item(&f.ctx, &item_id_named(&f.ctx, "Alpha")).unwrap();
    let claimed = alpha.sources.iter().find_map(|s| match s {
        ContentSource::Mo2Import { provenance, .. } => Some(provenance.clone()),
        _ => None,
    });
    match claimed {
        Some(Mo2Provenance::Claimed { modid, note, .. }) => {
            assert_eq!(modid, Some(42));
            assert_eq!(note, "provenance claimed, not verified");
        }
        other => panic!("Alpha's provenance was {other:?}"),
    }
}

#[test]
fn a_second_import_is_refused_unless_it_is_named() {
    let f = fixture();
    game_import::run(
        &f.ctx,
        &f.inventory,
        &request(&f, None, false),
        RunOptions::default(),
        &|_| {},
    )
    .unwrap();

    let refused = game_import::plan(&f.ctx, &f.inventory, &request(&f, None, false));
    match refused {
        Err(ImportError::AlreadyImported { instance }) => assert!(!instance.is_empty()),
        other => panic!("expected a refusal, got {:?}", other.map(|p| p.profile)),
    }

    let named = game_import::plan(
        &f.ctx,
        &f.inventory,
        &request(&f, Some("Second copy"), false),
    );
    assert!(named.is_ok(), "a new name allows the import");
}

#[test]
fn an_interrupted_import_finishes_on_rerun() {
    let f = fixture();
    let first = game_import::run(
        &f.ctx,
        &f.inventory,
        &request(&f, None, false),
        RunOptions {
            stop_after_mods: Some(1),
        },
        &|_| {},
    )
    .unwrap();
    assert!(first.interrupted);
    assert!(
        game_instance::list(&f.ctx).unwrap().is_empty(),
        "no instance before every item is stored"
    );
    let stored_after_first = content_store::list_items(&f.ctx).unwrap().len();
    assert_eq!(stored_after_first, 1);

    let second = game_import::run(
        &f.ctx,
        &f.inventory,
        &request(&f, None, false),
        RunOptions::default(),
        &|_| {},
    )
    .unwrap();
    assert!(!second.interrupted);
    assert_eq!(game_instance::list(&f.ctx).unwrap().len(), 1);
    // Alpha was stored by the first run and is reused, not stored twice.
    let items = content_store::list_items(&f.ctx).unwrap();
    let mut ids: Vec<&String> = items.iter().map(|i| &i.item_id).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), items.len());
}

#[test]
fn a_dry_run_stores_nothing() {
    let f = fixture();
    let plan = game_import::plan(&f.ctx, &f.inventory, &request(&f, None, false)).unwrap();
    assert_eq!(plan.mods.len(), 3);
    assert!(content_store::list_items(&f.ctx).unwrap().is_empty());
    assert!(game_instance::list(&f.ctx).unwrap().is_empty());
}

#[test]
fn a_gamepath_with_no_install_stops_with_a_message() {
    let f = fixture();
    let elsewhere = f._tmp.path().join("not-installed");
    build_setup(&f.setup, &elsewhere);
    let result = game_import::plan(&f.ctx, &f.inventory, &request(&f, None, false));
    match result {
        Err(e @ ImportError::NoInstall { .. }) => {
            let text = e.to_string();
            assert!(text.contains("not-installed"), "{text}");
        }
        other => panic!("expected NoInstall, got {:?}", other.map(|p| p.profile)),
    }
}

#[test]
fn saves_are_copied_only_with_the_flag_and_never_moved() {
    let f = fixture();
    // First import: no flag, so the saves stay where they are.
    let first = game_import::run(
        &f.ctx,
        &f.inventory,
        &request(&f, None, false),
        RunOptions::default(),
        &|_| {},
    )
    .unwrap();
    let rule = game_saves::rule_for(&f.def, &StoreId::steam())
        .unwrap()
        .clone();
    let own_first = game_saves::own_folder(&rule, first.instance_id.as_deref().unwrap()).unwrap();
    assert!(!own_first.exists());

    // Second import, named, with the flag: the files are copied into the instance's own folder.
    let second = game_import::run(
        &f.ctx,
        &f.inventory,
        &request(&f, Some("With saves"), true),
        RunOptions::default(),
        &|_| {},
    )
    .unwrap();
    assert_eq!(second.saves_copied, 2);
    let own = game_saves::own_folder(&rule, second.instance_id.as_deref().unwrap()).unwrap();
    assert_eq!(std::fs::read(own.join("Save1.ess")).unwrap(), b"SAVE ONE");
    assert_eq!(
        std::fs::read(f.setup.join("profiles/Test Profile/saves/Save1.ess")).unwrap(),
        b"SAVE ONE",
        "the profile's saves are copied, not moved"
    );
}

/// Two mods with identical bytes share one content item: the second is reported as a duplicate and
/// adds no layer of its own.
#[test]
fn identical_mods_share_one_layer() {
    let f = fixture();
    let gamma = f.setup.join("mods/Gamma");
    let delta = f.setup.join("mods/Delta");
    for name in ["Gamma.esp", "shared.txt", "meta.ini"] {
        write(&delta.join(name), &std::fs::read(gamma.join(name)).unwrap());
    }
    std::fs::write(
        f.setup.join("profiles/Test Profile/modlist.txt"),
        "+Delta\r\n+Gamma\r\n",
    )
    .unwrap();
    let run = game_import::run(
        &f.ctx,
        &f.inventory,
        &request(&f, None, false),
        RunOptions::default(),
        &|_| {},
    )
    .unwrap();
    assert_eq!(run.duplicates.len(), 1, "{:?}", run.duplicates);
    assert_eq!(
        run.content_layers, 2,
        "one mod layer and the overwrite layer"
    );
    let manifest =
        game_instance::get_manifest(&f.ctx, run.instance_id.as_deref().unwrap()).unwrap();
    assert_eq!(
        manifest.layers.layers().len(),
        3,
        "mod, overwrite, and Nemesis"
    );
}

/// A top-level `meta.ini` is MO2's metadata: it is left out of the stored content, so it never deploys
/// as `Data\meta.ini`, but its claims are still read. A `meta.ini` deeper in a mod is content and stays.
#[test]
fn top_level_meta_ini_is_provenance_not_content() {
    let f = fixture();
    write(
        &f.setup.join("mods/Alpha/sub/meta.ini"),
        b"[General]\r\nmodid=7\r\n",
    );
    // Beta has no meta.ini: give it one in a different case, which is still top-level metadata.
    write(
        &f.setup.join("mods/Beta/Meta.INI"),
        b"[General]\r\nmodid=9\r\n",
    );

    let plan = game_import::plan(&f.ctx, &f.inventory, &request(&f, None, false)).unwrap();
    // Alpha's and Gamma's top-level files, and Beta's differently-cased one.
    assert_eq!(plan.meta_ini_excluded, 3);
    let beta = plan.mods.iter().find(|m| m.folder == "Beta").unwrap();
    assert!(beta.meta_ini_excluded);
    assert!(matches!(
        beta.provenance,
        Mo2Provenance::Claimed { modid: Some(9), .. }
    ));

    let run = game_import::run(
        &f.ctx,
        &f.inventory,
        &request(&f, None, false),
        RunOptions::default(),
        &|_| {},
    )
    .unwrap();
    let alpha = content_store::get_item(&f.ctx, &item_id_named(&f.ctx, "Alpha")).unwrap();
    let paths: Vec<&str> = alpha.files.iter().map(|file| file.path.as_str()).collect();
    assert!(paths.contains(&"sub/meta.ini"), "{paths:?}");
    assert!(
        !paths.iter().any(|p| p.eq_ignore_ascii_case("meta.ini")),
        "{paths:?}"
    );
    let claimed = alpha.sources.iter().find_map(|s| match s {
        ContentSource::Mo2Import { provenance, .. } => Some(provenance.clone()),
        _ => None,
    });
    assert!(
        matches!(
            claimed,
            Some(Mo2Provenance::Claimed {
                modid: Some(42),
                ..
            })
        ),
        "Alpha's top-level meta.ini is still read as provenance: {claimed:?}"
    );

    let instance_id = run.instance_id.clone().unwrap();
    let deployed = game_deploy::plan(&f.ctx, &instance_id, &f.def, DeployMode::Links).unwrap();
    assert!(
        !deployed
            .files
            .iter()
            .any(|p| p.path.as_str().eq_ignore_ascii_case("Data/meta.ini")),
        "a top-level meta.ini was deployed"
    );
    assert!(deployed
        .files
        .iter()
        .any(|p| p.path.as_str() == "Data/sub/meta.ini"));
}

#[test]
fn a_plan_names_the_sources_it_would_use() {
    let f = fixture();
    let plan = game_import::plan(&f.ctx, &f.inventory, &request(&f, None, false)).unwrap();
    assert_eq!(plan.overwrite.generated.len(), 1);
    assert_eq!(plan.overwrite.generated[0].tool, "nemesis");
    assert_eq!(plan.overwrite.generated[0].files, 3);
    assert_eq!(plan.overwrite.rest_files, 4);
    assert_eq!(plan.saves.files, 2);
    assert!(plan.plugins.kept);
}

#[test]
fn the_setup_is_refused_when_it_is_not_there() {
    let f = fixture();
    let mut req = request(&f, None, false);
    req.ini_path = f.setup.join("nope/ModOrganizer.ini");
    assert!(matches!(
        game_import::plan(&f.ctx, &f.inventory, &req),
        Err(ImportError::NoSetup(_))
    ));
}

#[test]
fn an_unknown_game_is_refused() {
    let f = fixture();
    let text = std::fs::read_to_string(f.setup.join("ModOrganizer.ini")).unwrap();
    std::fs::write(
        f.setup.join("ModOrganizer.ini"),
        text.replace("gameName=Skyrim Special Edition", "gameName=Rimworld"),
    )
    .unwrap();
    assert!(matches!(
        game_import::plan(&f.ctx, &f.inventory, &request(&f, None, false)),
        Err(ImportError::UnknownGame(_))
    ));
}

#[test]
fn a_profile_that_is_not_there_is_refused() {
    let f = fixture();
    let mut req = request(&f, None, false);
    req.profile = "Nope".into();
    assert!(matches!(
        game_import::plan(&f.ctx, &f.inventory, &req),
        Err(ImportError::NoProfile { .. })
    ));
}
