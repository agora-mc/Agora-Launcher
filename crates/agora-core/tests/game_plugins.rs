//! Plugin activation at deploy, the `plugins` listing and toggle, framework loader launch
//! alternatives, and refusal of rules that could not work (MASTER_SPEC §26.3, §26.6).

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
use agora_core::game_plugins::{list, set_active, PluginListError};
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
    }
}

fn definition() -> GameDefinition {
    GameDefinition {
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
