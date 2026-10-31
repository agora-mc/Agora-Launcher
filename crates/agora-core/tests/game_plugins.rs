//! Plugin activation at deploy, the `plugins` listing and toggle, the Creation Engine load order
//! (sort, move, lock), framework loader launch alternatives, and refusal of rules that could not
//! work (MASTER_SPEC §26.3, §26.6).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use agora_core::ctx::CoreContext;
use agora_core::game_base::BaseMode;
use agora_core::game_deploy::{
    add_content, deploy, remove_content, set_content_enabled, DeployMode, DeployOutcome,
};
use agora_core::game_discovery::{DiscoveredInstall, DiscoveryReport, InstallCapabilities};
use agora_core::game_instance::{
    create, prepare_launch_with, GameInstanceRecord, LaunchOptions, VfsFailure,
};
use agora_core::game_launch::LaunchError;
use agora_core::game_launch::SystemLauncher;
use agora_core::game_load_order::{self, LoadOrderError, MoveTarget};
use agora_core::game_plugins::{list, set_active, set_locked, PluginListError};
use agora_core::game_registry::{
    GameRegistry, GameRegistryError, IdentifiedInstall, PackageSource, RuntimeResolution,
};
use agora_game_api::{
    DeploymentStrategy, GameDefinition, GameId, GamePackage, GamePath, InstallId, InstallKind,
    LaunchAlternative, LaunchRecipe, PackageDefinition, PluginListRule, RelPath, RuntimeIdentity,
    StoreId, StoreIdentifier, UserDataLocation, UserFileMapping, UserFileStrategy,
};
use tempfile::TempDir;

static TEST_LOCK: Mutex<()> = Mutex::new(());

struct Harness {
    _lock: std::sync::MutexGuard<'static, ()>,
    tmp: TempDir,
    user_data: TempDir,
    ctx: CoreContext,
    def: GameDefinition,
    install_dir: PathBuf,
    install: IdentifiedInstall,
}

impl Drop for Harness {
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

fn package_of(def: &GameDefinition) -> PackageDefinition {
    PackageDefinition {
        id: format!("test.{}", def.id),
        version: semver::Version::new(0, 1, 0),
        api_range: semver::VersionReq::parse(">=0.1, <0.2").unwrap(),
        parents: vec![],
        games: vec![def.clone()],
        frameworks: vec![],
        tools: vec![],
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

fn definition() -> GameDefinition {
    GameDefinition {
        mo2_game_name: None,
        id: GameId::new("test-game").unwrap(),
        name: "Test Game".into(),
        stores: vec![StoreIdentifier {
            store: StoreId::steam(),
            product: "12345".into(),
        }],
        version_sources: vec![],
        deployment: DeploymentStrategy::VirtualFileSystem,
        content_rules: vec![],
        content_layout: None,
        native_code_patterns: vec![],
        framework_ids: vec![],
        tool_ids: vec![],
        launch: Some(LaunchRecipe {
            executable: GamePath::Runtime {
                path: RelPath::new("Game.exe").unwrap(),
            },
            arguments: vec![],
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
                path: RelPath::new("Test Game/Plugins.txt").unwrap(),
            },
            RelPath::new("user/Plugins.txt").unwrap(),
            UserFileStrategy::JournaledSwap,
        )
        .with_stores(vec![StoreId::steam()])],
        save_paths: vec![],
        linked_archive_patterns: vec![],
        declared_writes: vec![],
        excluded_paths: vec![],
        plugin_list: Some(plugin_rule()),
        runtime_files: Vec::new(),
        save_location: Vec::new(),
        launch_alternatives: vec![LaunchAlternative {
            id: "loader".into(),
            when_present: RelPath::new("loader.exe").unwrap(),
            executable: GamePath::Runtime {
                path: RelPath::new("loader.exe").unwrap(),
            },
            reason: "the loader is installed, so the game starts through it".into(),
        }],
        copy_patterns: Vec::new(),
    }
}

fn install_of(dir: &Path) -> IdentifiedInstall {
    let store_id = StoreId::steam();
    let runtime = RuntimeIdentity {
        game: GameId::new("test-game").unwrap(),
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

impl Harness {
    fn new() -> Self {
        Self::with_definition(definition())
    }

    fn with_definition(def: GameDefinition) -> Self {
        let lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = TempDir::new().unwrap();
        let user_data = TempDir::new().unwrap();
        std::env::set_var("AGORA_TEST_USER_DATA_ROOT", user_data.path());

        let install_dir = tmp.path().join("install");
        std::fs::create_dir_all(install_dir.join("Data")).unwrap();
        std::fs::write(install_dir.join("Game.exe"), b"fake game binary").unwrap();
        std::fs::write(install_dir.join("Data").join("Skyrim.esm"), b"base master").unwrap();

        let ctx = CoreContext::for_testing(tmp.path().join("app_data"));
        agora_core::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();
        let mut builder = GameRegistry::builder();
        builder
            .add(
                PackageSource::Compiled {
                    crate_name: "test".into(),
                },
                Arc::new(TestPackage(package_of(&def))),
            )
            .expect("register test package");
        let ctx = ctx.with_games(Arc::new(builder.build()));
        let install = install_of(&install_dir);
        Self {
            _lock: lock,
            tmp,
            user_data,
            ctx,
            def,
            install_dir,
            install,
        }
    }

    fn instance(&self, name: &str) -> GameInstanceRecord {
        create(
            &self.ctx,
            &self.install,
            &self.def,
            name,
            None,
            BaseMode::Linked,
            &|_| {},
        )
        .expect("create instance")
    }

    fn item(&self, name: &str, files: &[(&str, &[u8])]) -> String {
        let dir = tempfile::tempdir().unwrap();
        for (rel, content) in files {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, content).unwrap();
        }
        agora_core::content_store::add_folder(&self.ctx, dir.path(), Some(name))
            .unwrap()
            .item()
            .item_id
            .clone()
    }

    fn add(&self, inst: &GameInstanceRecord, item: &str) {
        add_content(&self.ctx, &inst.instance_id, item, None, None).unwrap();
    }

    fn deploy(&self, inst: &GameInstanceRecord) -> DeployOutcome {
        deploy(&self.ctx, &inst.instance_id, &self.def, DeployMode::Links).expect("deploy")
    }

    fn copy_path(&self, inst: &GameInstanceRecord) -> PathBuf {
        self.ctx
            .paths
            .instance_dir(&inst.instance_id)
            .unwrap()
            .join("user")
            .join("Plugins.txt")
    }

    fn copy_text(&self, inst: &GameInstanceRecord) -> String {
        std::fs::read_to_string(self.copy_path(inst)).expect("instance copy exists")
    }

    fn real_path(&self) -> PathBuf {
        self.user_data
            .path()
            .join("local")
            .join("Test Game")
            .join("Plugins.txt")
    }

    fn lines(&self, inst: &GameInstanceRecord) -> Vec<String> {
        self.copy_text(inst).lines().map(str::to_string).collect()
    }

    fn plugin_names(&self, inst: &GameInstanceRecord) -> Vec<(String, bool, bool)> {
        list(&self.ctx, &inst.instance_id, &self.def)
            .unwrap()
            .entries
            .into_iter()
            .map(|e| (e.name, e.active, e.managed))
            .collect()
    }

    fn prepare(
        &self,
        inst: &GameInstanceRecord,
        plain: bool,
    ) -> Result<agora_core::game_launch::PreparedLaunch, agora_core::game_instance::InstanceError>
    {
        let report = DiscoveryReport {
            installs: vec![self.install.discovered.clone()],
            warnings: vec![],
        };
        prepare_launch_with(
            &self.ctx,
            &inst.instance_id,
            &self.def,
            // The test machine has no agora_vfs.dll beside the test executable; these tests are
            // about the plugin list and the launch alternative, so let it step down to links.
            LaunchOptions {
                plain,
                on_vfs_failure: VfsFailure::FallBack,
                ..Default::default()
            },
            &|| report.clone(),
            &SystemLauncher,
        )
    }
}

const HEADER: [&str; 2] = [
    "# This file is used by Skyrim to keep track of your downloaded content.",
    "# Please do not modify this file.",
];

// ---------------------------------------------------------------------------
// Activation
// ---------------------------------------------------------------------------

#[test]
fn a_new_instance_gets_masters_before_plugins_after_the_header() {
    let h = Harness::new();
    let inst = h.instance("fresh");
    let item = h.item("mod", &[("Data/A.esp", b"a"), ("Data/B.esm", b"b")]);
    h.add(&inst, &item);

    let outcome = h.deploy(&inst);
    let report = outcome.plugins().expect("the game keeps a plugin list");
    assert_eq!(report.added, ["B.esm", "A.esp"]);
    assert!(report.removed.is_empty());

    assert_eq!(h.lines(&inst), [HEADER[0], HEADER[1], "*B.esm", "*A.esp"]);
    assert_eq!(
        h.plugin_names(&inst),
        [("B.esm".into(), true, true), ("A.esp".into(), true, true)]
    );
    // A new copy uses the platform's line ending for this file format.
    assert!(h.copy_text(&inst).contains("\r\n"));
}

#[test]
fn layers_append_in_layer_order_and_a_layer_sorts_masters_light_then_plain() {
    let h = Harness::new();
    let inst = h.instance("order");
    let first = h.item(
        "first",
        &[
            ("Data/Z.esp", b"z"),
            ("Data/Y.esl", b"y"),
            ("Data/X.esm", b"x"),
            ("Data/W.esp", b"w"),
        ],
    );
    let second = h.item("second", &[("Data/A.esp", b"a"), ("Data/B.esm", b"b")]);
    h.add(&inst, &first);
    h.add(&inst, &second);

    let outcome = h.deploy(&inst);
    assert_eq!(
        outcome.plugins().unwrap().added,
        ["X.esm", "Y.esl", "W.esp", "Z.esp", "B.esm", "A.esp"]
    );
}

#[test]
fn a_user_inactive_line_stays_inactive_and_in_place_after_a_redeploy() {
    let h = Harness::new();
    let inst = h.instance("inactive");
    let first = h.item("first", &[("Data/A.esp", b"a")]);
    h.add(&inst, &first);
    h.deploy(&inst);
    assert!(set_active(&h.ctx, &inst.instance_id, &h.def, "a.ESP", false).unwrap());
    assert_eq!(h.lines(&inst)[2], "A.esp");

    let second = h.item("second", &[("Data/B.esp", b"b")]);
    h.add(&inst, &second);
    let outcome = h.deploy(&inst);
    assert_eq!(outcome.plugins().unwrap().added, ["B.esp"]);
    assert_eq!(h.lines(&inst), [HEADER[0], HEADER[1], "A.esp", "*B.esp"]);

    // A redeploy with nothing to change leaves it as the user left it.
    let again = h.deploy(&inst);
    assert!(matches!(again, DeployOutcome::UpToDate { .. }));
    assert!(again.plugins().unwrap().is_empty());
    assert_eq!(h.lines(&inst)[2], "A.esp");

    // Turning it back on, and a second toggle that changes nothing.
    assert!(set_active(&h.ctx, &inst.instance_id, &h.def, "A.esp", true).unwrap());
    assert!(!set_active(&h.ctx, &inst.instance_id, &h.def, "A.esp", true).unwrap());
    assert_eq!(h.lines(&inst)[2], "*A.esp");
}

#[test]
fn disabling_a_layer_removes_its_lines_but_not_a_line_the_user_wrote() {
    let h = Harness::new();
    let inst = h.instance("disable");
    let item = h.item("mod", &[("Data/A.esp", b"a"), ("Data/Keep.esp", b"k")]);
    let other = h.item("other", &[("Data/B.esp", b"b")]);
    h.add(&inst, &item);
    h.add(&inst, &other);
    h.deploy(&inst);

    // The user's own line, written by hand after Agora's.
    let mut text = h.copy_text(&inst);
    text.push_str("*Mine.esp\r\n# my note\r\n");
    std::fs::write(h.copy_path(&inst), text).unwrap();

    set_content_enabled(&h.ctx, &inst.instance_id, &item, false).unwrap();
    let outcome = h.deploy(&inst);
    let report = outcome.plugins().unwrap();
    assert!(report.added.is_empty());
    assert_eq!(report.removed, ["A.esp", "Keep.esp"]);
    assert_eq!(
        h.lines(&inst),
        [HEADER[0], HEADER[1], "*B.esp", "*Mine.esp", "# my note"]
    );
    assert_eq!(
        h.plugin_names(&inst),
        [
            ("B.esp".into(), true, true),
            ("Mine.esp".into(), true, false)
        ]
    );
}

#[test]
fn removing_the_last_layer_still_removes_its_lines_at_launch() {
    let h = Harness::new();
    let inst = h.instance("last-layer");
    let item = h.item("mod", &[("Data/A.esp", b"a")]);
    h.add(&inst, &item);
    h.deploy(&inst);
    assert_eq!(h.lines(&inst).len(), 3);

    remove_content(&h.ctx, &inst.instance_id, &item).unwrap();
    // No content is left, so a launch does not deploy; the list must still lose the line.
    h.prepare(&inst, false).expect("vanilla launch");
    assert_eq!(h.lines(&inst), [HEADER[0], HEADER[1]]);
}

#[test]
fn base_plugins_are_never_added_even_when_a_layer_overrides_them() {
    let h = Harness::new();
    let inst = h.instance("base-plugin");
    let item = h.item(
        "mod",
        &[("Data/Skyrim.esm", b"patched master"), ("Data/A.esp", b"a")],
    );
    h.add(&inst, &item);
    let outcome = h.deploy(&inst);
    assert_eq!(outcome.plugins().unwrap().added, ["A.esp"]);
    assert!(!h.copy_text(&inst).contains("Skyrim.esm"));
}

/// The test game, with the plugins the game always loads named in its rule and in `Skyrim.ccc`.
fn implicit_definition() -> GameDefinition {
    let mut def = definition();
    let rule = def.plugin_list.as_mut().unwrap();
    rule.implicit = vec!["Skyrim.esm".into(), "Update.esm".into()];
    rule.implicit_list_file = Some(RelPath::new("Skyrim.ccc").unwrap());
    def
}

#[test]
fn a_plugin_the_game_always_loads_is_never_activated_or_removed_by_sync() {
    let h = Harness::with_definition(implicit_definition());
    std::fs::write(h.install_dir.join("Skyrim.ccc"), "ccA.esm\r\nccB.esl\r\n").unwrap();
    let inst = h.instance("creation-club");
    let cc = h.item(
        "creation-club",
        &[
            ("Data/ccA.esm", b"a"),
            ("Data/Update.esm", b"u"),
            ("Data/Mine.esp", b"m"),
        ],
    );
    h.add(&inst, &cc);
    h.deploy(&inst);
    // The rule's Update.esm is never written into the list, and neither is a plugin Skyrim.ccc names
    // until the game itself writes it.
    assert_eq!(h.lines(&inst), [HEADER[0], HEADER[1], "*Mine.esp"]);

    // The game writes its own line for ccA.esm, inactive, as Skyrim does in Plugins.txt.
    append_line(&h, &inst, "ccA.esm");

    // A later deploy changes the layers, so sync runs: it leaves the game's line exactly as it is.
    let second = h.item("second", &[("Data/Second.esp", b"s")]);
    h.add(&inst, &second);
    let outcome = h.deploy(&inst);
    let report = outcome.plugins().unwrap();
    assert_eq!(report.added, ["Second.esp"]);
    assert!(report.removed.is_empty());
    assert_eq!(
        h.lines(&inst),
        [HEADER[0], HEADER[1], "*Mine.esp", "ccA.esm", "*Second.esp"]
    );
    assert_eq!(
        h.plugin_names(&inst),
        [
            ("Mine.esp".into(), true, true),
            ("ccA.esm".into(), false, false),
            ("Second.esp".into(), true, true),
        ]
    );
}

#[test]
fn an_instance_with_no_plugins_never_gets_a_list() {
    let h = Harness::new();
    let inst = h.instance("no-plugins");
    let item = h.item("textures", &[("Data/textures/a.dds", b"a")]);
    h.add(&inst, &item);
    let outcome = h.deploy(&inst);
    assert!(outcome.plugins().unwrap().is_empty());
    assert!(!h.copy_path(&inst).exists());
    let listed = list(&h.ctx, &inst.instance_id, &h.def).unwrap();
    assert!(!listed.exists && listed.entries.is_empty());
}

#[test]
fn plugins_outside_the_plugin_folder_itself_are_not_managed() {
    let h = Harness::new();
    let inst = h.instance("subfolder");
    let item = h.item(
        "mod",
        &[
            ("Data/sub/X.esp", b"x"),
            ("Data/Y.esp.bak", b"y"),
            ("Other/Z.esp", b"z"),
            ("W.esp", b"w"),
            ("Data/A.esp", b"a"),
        ],
    );
    h.add(&inst, &item);
    let outcome = h.deploy(&inst);
    assert_eq!(outcome.plugins().unwrap().added, ["A.esp"]);
}

#[test]
fn names_match_case_insensitively_and_the_existing_spelling_stays() {
    let h = Harness::new();
    let inst = h.instance("case");
    let copy = h.copy_path(&inst);
    std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
    std::fs::write(&copy, "*a.ESP\r\n").unwrap();
    let item = h.item("mod", &[("Data/A.esp", b"a")]);
    h.add(&inst, &item);
    let outcome = h.deploy(&inst);
    assert!(outcome.plugins().unwrap().added.is_empty());
    assert_eq!(h.lines(&inst), ["*a.ESP"]);
    assert_eq!(h.plugin_names(&inst), [("a.ESP".into(), true, true)]);
}

#[test]
fn comments_blank_lines_and_other_encodings_survive_byte_for_byte() {
    let h = Harness::new();
    let inst = h.instance("bytes");
    let copy = h.copy_path(&inst);
    std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
    let original: &[u8] = b"# keep me\r\n\r\n*Caf\xe9.esp\r\n   \r\n# end\r\n";
    std::fs::write(&copy, original).unwrap();
    let item = h.item("mod", &[("Data/A.esp", b"a")]);
    h.add(&inst, &item);
    h.deploy(&inst);
    let written = std::fs::read(&copy).unwrap();
    assert!(written.starts_with(original));
    assert!(written.ends_with(b"*A.esp\r\n"));
}

#[test]
fn a_new_instance_starts_from_the_real_file_and_leaves_it_untouched() {
    let h = Harness::new();
    let real = h.real_path();
    std::fs::create_dir_all(real.parent().unwrap()).unwrap();
    let original = "# mine\r\n*SkyUI_SE.esp\r\nOldMod.esp\r\n";
    std::fs::write(&real, original).unwrap();

    let inst = h.instance("from-real");
    let item = h.item("mod", &[("Data/A.esp", b"a")]);
    h.add(&inst, &item);
    h.deploy(&inst);
    assert_eq!(
        h.lines(&inst),
        ["# mine", "*SkyUI_SE.esp", "OldMod.esp", "*A.esp"]
    );
    assert_eq!(std::fs::read_to_string(&real).unwrap(), original);
    // The existing lines are the user's, not Agora's.
    assert_eq!(
        h.plugin_names(&inst),
        [
            ("SkyUI_SE.esp".into(), true, false),
            ("OldMod.esp".into(), false, false),
            ("A.esp".into(), true, true)
        ]
    );
}

#[test]
fn a_session_left_behind_is_recovered_before_the_real_file_is_read() {
    let h = Harness::new();
    let real = h.real_path();
    std::fs::create_dir_all(real.parent().unwrap()).unwrap();
    let original = "*UserOnly.esp\r\n";
    std::fs::write(&real, original).unwrap();

    // Another instance swapped its own list in and then died without restoring.
    let other = h.instance("other");
    let other_copy = h.copy_path(&other);
    std::fs::create_dir_all(other_copy.parent().unwrap()).unwrap();
    std::fs::write(&other_copy, "*OtherInstance.esp\r\n").unwrap();
    let nowhere = h.tmp.path().join("no-such-running-from");
    agora_core::game_user_files::swap_in(
        &h.ctx,
        &other.instance_id,
        &h.def,
        &StoreId::steam(),
        &nowhere,
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(&real).unwrap(),
        "*OtherInstance.esp\r\n"
    );

    let inst = h.instance("recovering");
    let item = h.item("mod", &[("Data/A.esp", b"a")]);
    h.add(&inst, &item);
    h.deploy(&inst);

    assert_eq!(h.lines(&inst), ["*UserOnly.esp", "*A.esp"]);
    assert_eq!(std::fs::read_to_string(&real).unwrap(), original);
}

#[test]
fn an_unreadable_instance_copy_fails_the_deploy_instead_of_reading_as_empty() {
    let h = Harness::new();
    let inst = h.instance("unreadable");
    let copy = h.copy_path(&inst);
    // A folder where the file belongs cannot be read as a list.
    std::fs::create_dir_all(&copy).unwrap();
    let item = h.item("mod", &[("Data/A.esp", b"a")]);
    h.add(&inst, &item);
    let err = deploy(&h.ctx, &inst.instance_id, &h.def, DeployMode::Links).unwrap_err();
    assert!(
        err.to_string().contains("cannot be read"),
        "unexpected error: {err}"
    );
    assert!(copy.is_dir(), "nothing was replaced");
}

#[test]
fn an_unreadable_state_file_fails_closed() {
    let h = Harness::new();
    let inst = h.instance("corrupt-state");
    let dir = h.ctx.paths.instance_dir(&inst.instance_id).unwrap();
    std::fs::write(dir.join("plugin_list_state.json"), b"{ not json").unwrap();
    let item = h.item("mod", &[("Data/A.esp", b"a")]);
    h.add(&inst, &item);
    let err = deploy(&h.ctx, &inst.instance_id, &h.def, DeployMode::Links).unwrap_err();
    assert!(err.to_string().contains("plugin_list_state.json"), "{err}");
    assert!(!h.copy_path(&inst).exists());
}

#[test]
fn a_game_without_a_plugin_list_deploys_as_before() {
    let mut def = definition();
    def.plugin_list = None;
    let h = Harness::with_definition(def);
    let inst = h.instance("plain-game");
    let item = h.item("mod", &[("Data/A.esp", b"a")]);
    h.add(&inst, &item);
    let outcome = h.deploy(&inst);
    assert!(outcome.plugins().is_none());
    assert!(!h.copy_path(&inst).exists());
    assert!(matches!(
        list(&h.ctx, &inst.instance_id, &h.def),
        Err(PluginListError::NoRule(_))
    ));
}

#[test]
fn a_store_the_mapping_does_not_cover_is_told_so_not_silently_skipped() {
    let mut def = definition();
    def.user_files[0].stores = vec![StoreId::gog()];
    // The registry only requires some journaled mapping for the file.
    let h = Harness::with_definition(def);
    let inst = h.instance("wrong-store");
    let item = h.item("mod", &[("Data/A.esp", b"a")]);
    h.add(&inst, &item);
    let outcome = h.deploy(&inst);
    let report = outcome.plugins().unwrap();
    assert!(report.added.is_empty());
    assert_eq!(report.warnings.len(), 1);
    assert!(!h.copy_path(&inst).exists());
}

#[test]
fn toggling_an_unknown_plugin_or_a_missing_list_is_an_error() {
    let h = Harness::new();
    let inst = h.instance("toggle-errors");
    assert!(matches!(
        set_active(&h.ctx, &inst.instance_id, &h.def, "A.esp", false),
        Err(PluginListError::NotInList(_))
    ));
    let item = h.item("mod", &[("Data/A.esp", b"a")]);
    h.add(&inst, &item);
    h.deploy(&inst);
    assert!(matches!(
        set_active(&h.ctx, &inst.instance_id, &h.def, "Nope.esp", false),
        Err(PluginListError::NotInList(_))
    ));
    assert!(matches!(
        set_active(&h.ctx, &inst.instance_id, &h.def, "", true),
        Err(PluginListError::NotInList(_))
    ));
    assert!(matches!(
        set_active(&h.ctx, "no-such-instance", &h.def, "A.esp", true),
        Err(PluginListError::InstanceNotFound(_))
    ));
}

#[test]
fn a_list_with_no_inactive_state_cannot_disable_a_plugin() {
    let mut def = definition();
    def.plugin_list.as_mut().unwrap().active_prefix = String::new();
    let h = Harness::with_definition(def);
    let inst = h.instance("always-active");
    let item = h.item("mod", &[("Data/A.esp", b"a")]);
    h.add(&inst, &item);
    h.deploy(&inst);
    assert_eq!(h.lines(&inst).last().unwrap(), "A.esp");
    assert!(matches!(
        set_active(&h.ctx, &inst.instance_id, &h.def, "A.esp", false),
        Err(PluginListError::NoInactiveState(_))
    ));
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

fn register(def: &GameDefinition) -> Result<(), GameRegistryError> {
    let mut builder = GameRegistry::builder();
    builder.add(
        PackageSource::Compiled {
            crate_name: "test".into(),
        },
        Arc::new(TestPackage(package_of(def))),
    )
}

#[test]
fn a_plugin_list_naming_no_user_file_mapping_is_refused_at_registration() {
    assert!(register(&definition()).is_ok());

    let mut def = definition();
    def.plugin_list.as_mut().unwrap().user_file = RelPath::new("user/Other.txt").unwrap();
    assert!(matches!(
        register(&def),
        Err(GameRegistryError::PluginListUserFileNotFound { .. })
    ));

    // A mapping that is not swapped in would silently do nothing too.
    let mut def = definition();
    def.user_files[0].strategy = UserFileStrategy::Redirect;
    assert!(matches!(
        register(&def),
        Err(GameRegistryError::PluginListUserFileNotFound { .. })
    ));

    let mut def = definition();
    def.user_files.clear();
    assert!(matches!(
        register(&def),
        Err(GameRegistryError::PluginListUserFileNotFound { .. })
    ));
}

#[test]
fn a_plugin_list_that_could_match_nothing_is_refused() {
    let mut def = definition();
    def.plugin_list.as_mut().unwrap().patterns = vec![];
    assert!(matches!(
        register(&def),
        Err(GameRegistryError::InvalidPluginList { .. })
    ));
    let mut def = definition();
    def.plugin_list.as_mut().unwrap().patterns = vec!["  ".into()];
    assert!(matches!(
        register(&def),
        Err(GameRegistryError::InvalidPluginList { .. })
    ));
    let mut def = definition();
    def.plugin_list.as_mut().unwrap().active_prefix = "* ".into();
    assert!(matches!(
        register(&def),
        Err(GameRegistryError::InvalidPluginList { .. })
    ));
}

#[test]
fn malformed_launch_alternatives_are_refused_at_registration() {
    let mut def = definition();
    def.launch_alternatives[0].executable = GamePath::UserData {
        location: UserDataLocation::Documents,
        path: RelPath::new("evil.exe").unwrap(),
    };
    assert!(matches!(
        register(&def),
        Err(GameRegistryError::InvalidLaunchAlternative { .. })
    ));

    let mut def = definition();
    let dup = def.launch_alternatives[0].clone();
    def.launch_alternatives.push(dup);
    assert!(matches!(
        register(&def),
        Err(GameRegistryError::InvalidLaunchAlternative { .. })
    ));

    let mut def = definition();
    def.launch_alternatives[0].id = " ".into();
    assert!(matches!(
        register(&def),
        Err(GameRegistryError::InvalidLaunchAlternative { .. })
    ));

    let mut def = definition();
    def.launch_alternatives[0].when_present = RelPath::default();
    assert!(matches!(
        register(&def),
        Err(GameRegistryError::InvalidLaunchAlternative { .. })
    ));
}

// ---------------------------------------------------------------------------
// Launch alternatives
// ---------------------------------------------------------------------------

#[test]
fn a_deployed_loader_replaces_the_executable_and_plain_opts_out() {
    let h = Harness::new();
    let inst = h.instance("loader");
    let item = h.item(
        "framework",
        &[("loader.exe", b"loader"), ("Data/A.esp", b"a")],
    );
    h.add(&inst, &item);

    let prepared = h.prepare(&inst, false).unwrap();
    assert_eq!(prepared.resolved.program.file_name().unwrap(), "loader.exe");
    let alt = prepared.alternative.as_ref().expect("alternative reported");
    assert_eq!(alt.id, "loader");
    assert!(alt.reason.contains("loader is installed"));
    // The working directory is still the recipe's: the deployed runtime folder.
    assert_eq!(
        prepared.resolved.cwd,
        prepared.resolved.program.parent().unwrap()
    );

    let plain = h.prepare(&inst, true).unwrap();
    assert_eq!(plain.resolved.program.file_name().unwrap(), "Game.exe");
    assert!(plain.alternative.is_none());
    assert_eq!(plain.resolved.cwd, prepared.resolved.cwd);
}

#[test]
fn without_the_loader_the_recipe_runs() {
    let h = Harness::new();
    let inst = h.instance("no-loader");
    let item = h.item("mod", &[("Data/A.esp", b"a")]);
    h.add(&inst, &item);
    let prepared = h.prepare(&inst, false).unwrap();
    assert_eq!(prepared.resolved.program.file_name().unwrap(), "Game.exe");
    assert!(prepared.alternative.is_none());
}

#[test]
fn a_loader_in_the_base_is_used_by_an_instance_without_content() {
    let h = Harness::new();
    std::fs::write(h.install_dir.join("loader.exe"), b"loader").unwrap();
    // The base is built after the loader was installed.
    let inst = h.instance("base-loader");
    let prepared = h.prepare(&inst, false).unwrap();
    // A game that needs the virtual file system runs from a farm of its base even with no
    // content on top, and the loader is found in that farm.
    assert!(prepared.deploy_outcome.is_some());
    assert_eq!(prepared.resolved.program.file_name().unwrap(), "loader.exe");
    assert_eq!(prepared.alternative.as_ref().unwrap().id, "loader");
    let plain = h.prepare(&inst, true).unwrap();
    assert_eq!(plain.resolved.program.file_name().unwrap(), "Game.exe");
}

#[test]
fn an_alternative_that_matches_but_cannot_start_fails_instead_of_falling_back() {
    let mut def = definition();
    def.launch_alternatives[0].executable = GamePath::Runtime {
        path: RelPath::new("missing-loader.exe").unwrap(),
    };
    let h = Harness::with_definition(def);
    let inst = h.instance("broken-loader");
    let item = h.item("framework", &[("loader.exe", b"loader")]);
    h.add(&inst, &item);
    let err = h.prepare(&inst, false).unwrap_err();
    assert!(matches!(
        err,
        agora_core::game_instance::InstanceError::LaunchError(LaunchError::ProgramMissing { .. })
    ));
    // The user can still start the game itself.
    assert!(h.prepare(&inst, true).is_ok());
}

#[test]
fn the_first_matching_alternative_wins() {
    let mut def = definition();
    def.launch_alternatives.insert(
        0,
        LaunchAlternative {
            id: "other".into(),
            when_present: RelPath::new("other.exe").unwrap(),
            executable: GamePath::Runtime {
                path: RelPath::new("other.exe").unwrap(),
            },
            reason: "other".into(),
        },
    );
    let h = Harness::with_definition(def);
    let inst = h.instance("two-loaders");
    let item = h.item("both", &[("loader.exe", b"l"), ("other.exe", b"o")]);
    h.add(&inst, &item);
    let prepared = h.prepare(&inst, false).unwrap();
    assert_eq!(prepared.alternative.unwrap().id, "other");
}

// ---------------------------------------------------------------------------
// Creation Engine load order (MASTER_SPEC §26.6)
// ---------------------------------------------------------------------------

/// Plugin bytes: a TES4 record with flags and the given masters (see `game_load_order`'s tests).
fn plugin_file(flags: u32, masters: &[&str]) -> Vec<u8> {
    fn subrecord(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(kind);
        out.extend_from_slice(&u16::try_from(data.len()).unwrap().to_le_bytes());
        out.extend_from_slice(data);
    }
    let mut body = Vec::new();
    subrecord(&mut body, b"HEDR", &[0u8; 12]);
    for master in masters {
        let mut name = master.as_bytes().to_vec();
        name.push(0);
        subrecord(&mut body, b"MAST", &name);
        subrecord(&mut body, b"DATA", &[0u8; 8]);
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"TES4");
    out.extend_from_slice(&u32::try_from(body.len()).unwrap().to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&[0u8; 12]);
    out.extend_from_slice(&body);
    out
}

fn engine_definition(implicit: &[&str]) -> GameDefinition {
    let mut def = definition();
    let rule = def.plugin_list.as_mut().unwrap();
    rule.semantics = Some("creation_engine".into());
    rule.implicit = implicit.iter().map(|s| s.to_string()).collect();
    def
}

/// The plugin lines of the instance's copy, without the two header lines.
fn body(h: &Harness, inst: &GameInstanceRecord) -> Vec<String> {
    h.lines(inst).into_iter().skip(2).collect()
}

/// A late master: the patch needs the base, and the base's layer loads after the patch's.
fn late_master_harness(name: &str) -> (Harness, GameInstanceRecord) {
    let h = Harness::with_definition(engine_definition(&[]));
    let inst = h.instance(name);
    let patch = h.item(
        "patch",
        &[("Data/Patch.esp", &plugin_file(0, &["Base.esm"])[..])],
    );
    let base = h.item("base", &[("Data/Base.esm", &plugin_file(1, &[])[..])]);
    h.add(&inst, &patch);
    h.add(&inst, &base);
    h.deploy(&inst);
    (h, inst)
}

#[test]
fn sort_moves_a_late_master_up_once_and_a_second_sort_does_nothing() {
    let (h, inst) = late_master_harness("sorted");
    assert_eq!(body(&h, &inst), ["*Patch.esp", "*Base.esm"]);

    let dry = game_load_order::sort(&h.ctx, &inst.instance_id, &h.def, true).unwrap();
    assert_eq!(dry.moves.len(), 1);
    assert!(!dry.written);
    assert_eq!(
        body(&h, &inst),
        ["*Patch.esp", "*Base.esm"],
        "a dry run writes nothing"
    );

    let sorted = game_load_order::sort(&h.ctx, &inst.instance_id, &h.def, false).unwrap();
    assert!(sorted.written);
    assert_eq!(body(&h, &inst), ["*Base.esm", "*Patch.esp"]);
    assert_eq!(h.lines(&inst)[0], HEADER[0]);

    let again = game_load_order::sort(&h.ctx, &inst.instance_id, &h.def, false).unwrap();
    assert!(again.moves.is_empty() && !again.written);
    assert!(game_load_order::check(&h.ctx, &inst.instance_id, &h.def)
        .unwrap()
        .is_empty());
}

#[test]
fn a_locked_master_stays_put_and_its_lock_survives_a_redeploy() {
    let (h, inst) = late_master_harness("locked");
    assert!(set_locked(&h.ctx, &inst.instance_id, &h.def, "Base.esm", true).unwrap());
    assert!(!set_locked(&h.ctx, &inst.instance_id, &h.def, "Base.esm", true).unwrap());
    let listed = list(&h.ctx, &inst.instance_id, &h.def).unwrap();
    assert!(listed
        .entries
        .iter()
        .any(|e| e.name == "Base.esm" && e.locked));

    // The patch loads above its locked master, and nothing may move the master.
    let report = game_load_order::sort(&h.ctx, &inst.instance_id, &h.def, false).unwrap();
    assert!(report.moves.is_empty());
    assert_eq!(report.blocked.len(), 1);
    assert_eq!(report.blocked[0].master, "Base.esm");
    assert_eq!(body(&h, &inst), ["*Patch.esp", "*Base.esm"]);

    // A new layer changes the deployed list, which rewrites the state file: the lock stays.
    let extra = h.item("extra", &[("Data/Extra.esp", &plugin_file(0, &[])[..])]);
    h.add(&inst, &extra);
    h.deploy(&inst);
    let listed = list(&h.ctx, &inst.instance_id, &h.def).unwrap();
    assert!(listed
        .entries
        .iter()
        .any(|e| e.name == "Base.esm" && e.locked));

    assert!(set_locked(&h.ctx, &inst.instance_id, &h.def, "Base.esm", false).unwrap());
    let listed = list(&h.ctx, &inst.instance_id, &h.def).unwrap();
    assert!(!listed.entries.iter().any(|e| e.locked));
}

#[test]
fn a_state_file_from_before_locks_still_loads() {
    let h = Harness::with_definition(engine_definition(&[]));
    let inst = h.instance("old-state");
    let item = h.item("mod", &[("Data/A.esp", &plugin_file(0, &[])[..])]);
    h.add(&inst, &item);
    h.deploy(&inst);
    let state_dir = h
        .copy_path(&inst)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    std::fs::write(
        state_dir.join("plugin_list_state.json"),
        br#"{"managed": ["A.esp"]}"#,
    )
    .unwrap();
    let listed = list(&h.ctx, &inst.instance_id, &h.def).unwrap();
    let a = listed.entries.iter().find(|e| e.name == "A.esp").unwrap();
    assert!(a.managed);
    assert!(!a.locked);
}

#[test]
fn move_places_a_plugin_and_refuses_to_move_a_locked_one() {
    let (h, inst) = late_master_harness("moved");
    let extra = h.item("extra", &[("Data/Extra.esp", &plugin_file(0, &[])[..])]);
    h.add(&inst, &extra);
    h.deploy(&inst);
    assert_eq!(body(&h, &inst), ["*Patch.esp", "*Base.esm", "*Extra.esp"]);

    let moved = game_load_order::move_plugin(
        &h.ctx,
        &inst.instance_id,
        &h.def,
        "Extra.esp",
        &MoveTarget::Before("Patch.esp".into()),
    )
    .unwrap();
    assert!(moved.written);
    assert_eq!(body(&h, &inst), ["*Extra.esp", "*Patch.esp", "*Base.esm"]);

    set_locked(&h.ctx, &inst.instance_id, &h.def, "Extra.esp", true).unwrap();
    let err = game_load_order::move_plugin(
        &h.ctx,
        &inst.instance_id,
        &h.def,
        "Extra.esp",
        &MoveTarget::Position(3),
    )
    .unwrap_err();
    assert!(matches!(err, LoadOrderError::Locked(_)), "{err}");
    assert_eq!(body(&h, &inst), ["*Extra.esp", "*Patch.esp", "*Base.esm"]);
}

#[test]
fn a_plugin_the_game_always_loads_is_shown_once_and_is_not_moved() {
    let h = Harness::with_definition(engine_definition(&["Base.esm"]));
    let inst = h.instance("implicit");
    let item = h.item(
        "base",
        &[
            ("Data/Base.esm", &plugin_file(1, &[])[..]),
            ("Data/Patch.esp", &plugin_file(0, &["Base.esm"])[..]),
        ],
    );
    h.add(&inst, &item);
    h.deploy(&inst);
    // The game loads Base.esm itself, so the deployed copy never names it (game_load_order docs).
    assert_eq!(body(&h, &inst), ["*Patch.esp"]);

    let order = game_load_order::order(&h.ctx, &inst.instance_id, &h.def).unwrap();
    let names: Vec<&str> = order.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["Base.esm", "Patch.esp"]);
    assert!(order.entries[0].implicit && order.entries[0].master);
    assert!(order.findings.is_empty(), "{:?}", order.findings);

    let report = game_load_order::sort(&h.ctx, &inst.instance_id, &h.def, false).unwrap();
    assert!(report.moves.is_empty() && !report.written);
    assert_eq!(body(&h, &inst), ["*Patch.esp"]);
    let moved = game_load_order::move_plugin(
        &h.ctx,
        &inst.instance_id,
        &h.def,
        "Base.esm",
        &MoveTarget::Position(2),
    )
    .unwrap_err();
    assert!(matches!(moved, LoadOrderError::AlwaysLoaded(_)), "{moved}");
}

// ---- Review probes (Phase 4 slice 1) ----

#[test]
fn probe_empty_names_never_select_a_plugin() {
    let (h, inst) = late_master_harness("probe-empty");
    assert!(set_locked(&h.ctx, &inst.instance_id, &h.def, "", true).is_err());
    assert!(set_locked(&h.ctx, &inst.instance_id, &h.def, "   ", true).is_err());
    let to_front = game_load_order::MoveTarget::Position(1);
    assert!(
        game_load_order::move_plugin(&h.ctx, &inst.instance_id, &h.def, "", &to_front).is_err()
    );
    let before_nothing = game_load_order::MoveTarget::Before(String::new());
    assert!(game_load_order::move_plugin(
        &h.ctx,
        &inst.instance_id,
        &h.def,
        "Patch.esp",
        &before_nothing
    )
    .is_err());
    assert_eq!(
        body(&h, &inst),
        ["*Patch.esp", "*Base.esm"],
        "nothing changed"
    );
}

#[test]
fn probe_sort_keeps_inactive_and_hand_written_lines() {
    let (h, inst) = late_master_harness("probe-keep");
    let path = list(&h.ctx, &inst.instance_id, &h.def).unwrap().path;
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str("UserInactive.esp\n*UserActive.esp\n");
    std::fs::write(&path, text).unwrap();
    let before: std::collections::BTreeSet<String> = body(&h, &inst).into_iter().collect();
    game_load_order::sort(&h.ctx, &inst.instance_id, &h.def, false).unwrap();
    let after: std::collections::BTreeSet<String> = body(&h, &inst).into_iter().collect();
    assert_eq!(before, after, "sort only reorders lines");
    let lines = body(&h, &inst);
    let pos = |n: &str| {
        lines
            .iter()
            .position(|l| l.trim_start_matches('*') == n)
            .unwrap()
    };
    assert!(pos("Base.esm") < pos("Patch.esp"));
}

// ---------------------------------------------------------------------------
// The load order at launch (MASTER_SPEC §26.6)
// ---------------------------------------------------------------------------

/// Prepare a launch with `launch_anyway` as given, on the harness's machine.
fn prepare_with(
    h: &Harness,
    inst: &GameInstanceRecord,
    launch_anyway: bool,
) -> Result<agora_core::game_launch::PreparedLaunch, agora_core::game_instance::InstanceError> {
    let report = DiscoveryReport {
        installs: vec![h.install.discovered.clone()],
        warnings: vec![],
    };
    prepare_launch_with(
        &h.ctx,
        &inst.instance_id,
        &h.def,
        LaunchOptions {
            launch_anyway,
            on_vfs_failure: VfsFailure::FallBack,
            ..Default::default()
        },
        &|| report.clone(),
        &SystemLauncher,
    )
}

/// Add a line to the instance's plugin list by hand, the way a user's own line gets there.
fn append_line(h: &Harness, inst: &GameInstanceRecord, line: &str) {
    let path = h.copy_path(inst);
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str(line);
    text.push_str("\r\n");
    std::fs::write(&path, text).unwrap();
}

#[test]
fn a_plugin_whose_master_is_missing_refuses_the_launch_until_launch_anyway() {
    let h = Harness::with_definition(engine_definition(&[]));
    let inst = h.instance("missing-master");
    let patch = h.item(
        "patch",
        &[("Data/Patch.esp", &plugin_file(0, &["Base.esm"])[..])],
    );
    h.add(&inst, &patch);

    let err = prepare_with(&h, &inst, false).unwrap_err();
    let agora_core::game_instance::InstanceError::LaunchError(LaunchError::LoadOrderProblems {
        findings,
    }) = &err
    else {
        panic!("expected a load order refusal, got {err:?}");
    };
    assert!(findings.iter().any(|f| matches!(
        f,
        agora_core::game_load_order::Finding::MasterNotEarlier {
            master,
            problem: agora_core::game_load_order::MasterProblem::Missing,
            ..
        } if master == "Base.esm"
    )));
    let message = err.to_string();
    assert!(
        message.contains("Base.esm") || message.contains("master"),
        "{message}"
    );
    assert!(
        message.contains("agora games instance plugins sort <instance>"),
        "{message}"
    );

    let forced = prepare_with(&h, &inst, true).expect("launch_anyway starts the game");
    assert!(forced
        .load_order_findings
        .iter()
        .any(agora_core::game_load_order::Finding::refuses_launch));
}

#[test]
fn a_late_master_launches_with_a_warning_and_a_sort_would_fix_it() {
    let (h, inst) = late_master_harness("late-launch");

    // The game starts with a plugin before its master (the Phase 3 run of p3-modded did), so the
    // launch goes ahead and the warning says so.
    let prepared = prepare_with(&h, &inst, false).expect("a late master never refuses");
    assert!(prepared.load_order_findings.iter().any(|f| matches!(
        f,
        agora_core::game_load_order::Finding::MasterNotEarlier {
            problem: agora_core::game_load_order::MasterProblem::Later,
            ..
        }
    )));
    assert!(!prepared
        .load_order_findings
        .iter()
        .any(agora_core::game_load_order::Finding::refuses_launch));

    game_load_order::sort(&h.ctx, &inst.instance_id, &h.def, false).unwrap();
    let after = prepare_with(&h, &inst, false).expect("sorted, the launch is clean");
    assert!(after.load_order_findings.is_empty());
}

#[test]
fn a_missing_master_still_refuses_the_launch() {
    let h = Harness::with_definition(engine_definition(&[]));
    let inst = h.instance("missing-still");
    let patch = h.item(
        "patch",
        &[("Data/Patch.esp", &plugin_file(0, &["Gone.esm"])[..])],
    );
    h.add(&inst, &patch);

    let err = prepare_with(&h, &inst, false).unwrap_err();
    assert!(
        matches!(
            err,
            agora_core::game_instance::InstanceError::LaunchError(
                LaunchError::LoadOrderProblems { .. }
            )
        ),
        "{err:?}"
    );
}

#[test]
fn a_duplicate_line_only_warns_and_the_launch_goes_ahead() {
    let h = Harness::with_definition(engine_definition(&[]));
    let inst = h.instance("duplicate");
    let base = h.item("base", &[("Data/Base.esm", &plugin_file(1, &[])[..])]);
    let patch = h.item(
        "patch",
        &[("Data/Patch.esp", &plugin_file(0, &["Base.esm"])[..])],
    );
    h.add(&inst, &base);
    h.add(&inst, &patch);
    h.deploy(&inst);
    append_line(&h, &inst, "*Base.esm");

    let prepared = prepare_with(&h, &inst, false).expect("a duplicate line never refuses");
    assert!(prepared.load_order_findings.iter().any(|f| matches!(
        f,
        agora_core::game_load_order::Finding::DuplicateListing { plugin } if plugin == "Base.esm"
    )));
    assert!(!prepared
        .load_order_findings
        .iter()
        .any(agora_core::game_load_order::Finding::refuses_launch));
}

#[test]
fn the_check_lists_every_finding_and_only_the_missing_master_refuses() {
    let h = Harness::with_definition(engine_definition(&[]));
    let inst = h.instance("both-kinds");
    let base = h.item("base", &[("Data/Base.esm", &plugin_file(1, &[])[..])]);
    let patch = h.item(
        "patch",
        &[("Data/Patch.esp", &plugin_file(0, &["Base.esm"])[..])],
    );
    let orphan = h.item(
        "orphan",
        &[("Data/Orphan.esp", &plugin_file(0, &["Missing.esm"])[..])],
    );
    h.add(&inst, &base);
    h.add(&inst, &patch);
    h.add(&inst, &orphan);
    h.deploy(&inst);
    append_line(&h, &inst, "*Base.esm");

    let findings = game_load_order::check(&h.ctx, &inst.instance_id, &h.def).unwrap();
    let refusing: Vec<_> = findings.iter().filter(|f| f.refuses_launch()).collect();
    let warning: Vec<_> = findings.iter().filter(|f| !f.refuses_launch()).collect();
    assert_eq!(refusing.len(), 1, "{findings:?}");
    assert_eq!(warning.len(), 1, "{findings:?}");
    assert!(matches!(
        refusing[0],
        agora_core::game_load_order::Finding::MasterNotEarlier { .. }
    ));
    assert!(matches!(
        warning[0],
        agora_core::game_load_order::Finding::DuplicateListing { .. }
    ));

    // The launch refuses for the one that refuses, and names the other as well.
    let err = prepare_with(&h, &inst, false).unwrap_err();
    let agora_core::game_instance::InstanceError::LaunchError(LaunchError::LoadOrderProblems {
        findings: named,
    }) = err
    else {
        panic!("expected a load order refusal");
    };
    assert_eq!(named.len(), 2, "{named:?}");
    assert!(agora_core::game_load_order::describe_findings(&named).contains("more than once"));
}

#[test]
fn refuse_load_order_refuses_only_the_refusing_kinds() {
    use agora_core::game_launch::refuse_load_order;
    use agora_core::game_load_order::{Finding, MasterProblem};

    let warnings = vec![
        Finding::DuplicateListing {
            plugin: "A.esp".into(),
        },
        Finding::UnreadableHeader {
            plugin: "B.esp".into(),
            reason: "bad".into(),
        },
    ];
    assert_eq!(
        refuse_load_order(warnings.clone(), false).unwrap(),
        warnings
    );

    let cycle = vec![Finding::MasterCycle {
        plugins: vec!["A.esp".into(), "B.esp".into()],
    }];
    assert!(refuse_load_order(cycle.clone(), false).is_err());
    assert_eq!(refuse_load_order(cycle.clone(), true).unwrap(), cycle);

    // A late master only warns: the game starts with the plugin before its master.
    let late = vec![Finding::MasterNotEarlier {
        plugin: "A.esp".into(),
        master: "B.esm".into(),
        problem: MasterProblem::Later,
    }];
    assert_eq!(refuse_load_order(late.clone(), false).unwrap(), late);
    let missing = vec![Finding::MasterNotEarlier {
        plugin: "A.esp".into(),
        master: "B.esm".into(),
        problem: MasterProblem::Missing,
    }];
    assert!(refuse_load_order(missing, false).is_err());
    let full = vec![Finding::TooManyFullPlugins {
        active: 255,
        limit: 254,
    }];
    assert!(refuse_load_order(full, false).is_err());
}
