#![allow(unused_imports, dead_code)]
use std::path::PathBuf;
use std::sync::Mutex;

use agora_core::ctx::CoreContext;
use agora_core::game_user_files::{
    is_session_running, record_process, restore, status, swap_in, Journal, JournaledFile,
};
use agora_core::process_identity;
use agora_game_api::{
    DeploymentStrategy, GameDefinition, GameId, GamePath, RelPath, StoreId, StoreIdentifier,
    UserDataLocation, UserFileMapping, UserFileStrategy,
};
use tempfile::TempDir;

static TEST_LOCK: Mutex<()> = Mutex::new(());

struct TestHarness {
    _lock: std::sync::MutexGuard<'static, ()>,
    _data_dir: TempDir,
    user_data_dir: TempDir,
    ctx: CoreContext,
    running_from: PathBuf,
}

impl TestHarness {
    fn new() -> Self {
        let lock = TEST_LOCK.lock().unwrap();
        let data_dir = TempDir::new().unwrap();
        let user_data_dir = TempDir::new().unwrap();

        std::env::set_var("AGORA_TEST_USER_DATA_ROOT", user_data_dir.path());

        let ctx = CoreContext::for_testing(data_dir.path().join("app_data"));
        let _ = agora_core::db::init_local_state_db(&ctx.paths.local_state_db());

        let running_from = data_dir.path().join("running_from");
        std::fs::create_dir_all(&running_from).unwrap();

        Self {
            _lock: lock,
            _data_dir: data_dir,
            user_data_dir,
            ctx,
            running_from,
        }
    }

    fn local_dir(&self) -> PathBuf {
        self.user_data_dir.path().join("local")
    }

    fn documents_dir(&self) -> PathBuf {
        self.user_data_dir.path().join("documents")
    }

    fn instance_dir(&self, id: &str) -> PathBuf {
        let dir = self.ctx.paths.instances_root().join(id);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}

impl Drop for TestHarness {
    fn drop(&mut self) {
        std::env::remove_var("AGORA_TEST_USER_DATA_ROOT");
    }
}

fn test_skyrim_definition() -> GameDefinition {
    GameDefinition {
        mo2_game_name: None,
        id: GameId::new("skyrim-se").unwrap(),
        name: "The Elder Scrolls V: Skyrim Special Edition".to_string(),
        stores: vec![
            StoreIdentifier {
                store: StoreId::steam(),
                product: "489830".into(),
            },
            StoreIdentifier {
                store: StoreId::gog(),
                product: "1711230643".into(),
            },
        ],
        version_sources: vec![],
        deployment: DeploymentStrategy::VirtualFileSystem,
        content_rules: vec![],
        native_code_patterns: vec![],
        framework_ids: vec![],
        tool_ids: vec![],
        launch: None,
        log_paths: vec![],
        crash_paths: vec![],
        user_files: vec![
            UserFileMapping {
                source: GamePath::UserData {
                    location: UserDataLocation::LocalAppData,
                    path: RelPath::new("Skyrim Special Edition/Plugins.txt").unwrap(),
                },
                instance_path: RelPath::new("user/Plugins.txt").unwrap(),
                strategy: UserFileStrategy::JournaledSwap,
                stores: vec![StoreId::steam()],
            },
            UserFileMapping {
                source: GamePath::UserData {
                    location: UserDataLocation::Documents,
                    path: RelPath::new("My Games/Skyrim Special Edition/Skyrim.ini").unwrap(),
                },
                instance_path: RelPath::new("user/Skyrim.ini").unwrap(),
                strategy: UserFileStrategy::JournaledSwap,
                stores: vec![StoreId::steam()],
            },
            UserFileMapping {
                source: GamePath::UserData {
                    location: UserDataLocation::LocalAppData,
                    path: RelPath::new("Skyrim Special Edition GOG/Plugins.txt").unwrap(),
                },
                instance_path: RelPath::new("user/Plugins.txt").unwrap(),
                strategy: UserFileStrategy::JournaledSwap,
                stores: vec![StoreId::gog()],
            },
            UserFileMapping {
                source: GamePath::UserData {
                    location: UserDataLocation::Documents,
                    path: RelPath::new("My Games/Skyrim Special Edition GOG/Skyrim.ini").unwrap(),
                },
                instance_path: RelPath::new("user/Skyrim.ini").unwrap(),
                strategy: UserFileStrategy::JournaledSwap,
                stores: vec![StoreId::gog()],
            },
        ],
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
    }
}

// ---------------------------------------------------------------------------
// Adversarial probes: a journal on disk is data, not instructions.
// ---------------------------------------------------------------------------

fn journal_path_of(h: &TestHarness, def: &GameDefinition) -> PathBuf {
    h.ctx
        .paths
        .user_files_journal_path(def.id.as_str(), StoreId::steam().as_str())
}

fn session_with_one_file(h: &TestHarness, def: &GameDefinition) -> PathBuf {
    let real = h
        .local_dir()
        .join("Skyrim Special Edition")
        .join("Plugins.txt");
    std::fs::create_dir_all(real.parent().unwrap()).unwrap();
    std::fs::write(&real, "original\n").unwrap();
    let inst = h.instance_dir("probe-a").join("user").join("Plugins.txt");
    std::fs::create_dir_all(inst.parent().unwrap()).unwrap();
    std::fs::write(&inst, "instance\n").unwrap();
    swap_in(&h.ctx, "probe-a", def, &StoreId::steam(), &h.running_from).unwrap();
    real
}

fn tamper(h: &TestHarness, def: &GameDefinition, f: impl Fn(&mut serde_json::Value)) {
    let jp = journal_path_of(h, def);
    let mut j: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&jp).unwrap()).unwrap();
    f(&mut j);
    std::fs::write(&jp, serde_json::to_vec(&j).unwrap()).unwrap();
}

#[test]
fn probe_tampered_real_path_cannot_overwrite_another_file() {
    let h = TestHarness::new();
    let def = test_skyrim_definition();
    session_with_one_file(&h, &def);
    let victim = h._data_dir.path().join("victim.txt");
    std::fs::write(&victim, "keep me\n").unwrap();
    tamper(&h, &def, |j| {
        for f in j["files"].as_array_mut().unwrap() {
            f["real_path"] = serde_json::json!(victim.to_string_lossy());
        }
    });
    let _ = restore(&h.ctx, &def, &StoreId::steam());
    assert_eq!(
        std::fs::read_to_string(&victim).unwrap(),
        "keep me\n",
        "restore overwrote a file the journal named"
    );
}

#[test]
fn probe_tampered_existed_false_cannot_delete_another_file() {
    let h = TestHarness::new();
    let def = test_skyrim_definition();
    session_with_one_file(&h, &def);
    let victim = h._data_dir.path().join("victim2.txt");
    std::fs::write(&victim, "keep me\n").unwrap();
    tamper(&h, &def, |j| {
        for f in j["files"].as_array_mut().unwrap() {
            f["real_path"] = serde_json::json!(victim.to_string_lossy());
            f["existed"] = serde_json::json!(false);
            f["backup_path"] = serde_json::Value::Null;
        }
    });
    let _ = restore(&h.ctx, &def, &StoreId::steam());
    assert!(victim.exists(), "restore deleted a file the journal named");
}

#[test]
fn probe_tampered_instance_copy_path_cannot_write_elsewhere() {
    let h = TestHarness::new();
    let def = test_skyrim_definition();
    session_with_one_file(&h, &def);
    let victim = h._data_dir.path().join("victim3.txt");
    std::fs::write(&victim, "keep me\n").unwrap();
    tamper(&h, &def, |j| {
        for f in j["files"].as_array_mut().unwrap() {
            f["instance_copy_path"] = serde_json::json!(victim.to_string_lossy());
        }
    });
    let _ = restore(&h.ctx, &def, &StoreId::steam());
    assert_eq!(
        std::fs::read_to_string(&victim).unwrap(),
        "keep me\n",
        "restore wrote the session into a file the journal named"
    );
}

#[test]
fn probe_missing_backup_keeps_journal_and_real_file() {
    let h = TestHarness::new();
    let def = test_skyrim_definition();
    let real = session_with_one_file(&h, &def);
    let jp = journal_path_of(&h, &def);
    let j: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&jp).unwrap()).unwrap();
    let backup = PathBuf::from(j["files"][0]["backup_path"].as_str().expect("a backup"));
    std::fs::remove_file(&backup).unwrap();
    assert!(
        restore(&h.ctx, &def, &StoreId::steam()).is_err(),
        "restore without its backup succeeded"
    );
    assert!(jp.exists(), "journal deleted");
    assert!(real.exists(), "real file deleted");
}

#[test]
fn probe_garbage_journal_refuses_swap() {
    let h = TestHarness::new();
    let def = test_skyrim_definition();
    let real = h
        .local_dir()
        .join("Skyrim Special Edition")
        .join("Plugins.txt");
    std::fs::create_dir_all(real.parent().unwrap()).unwrap();
    std::fs::write(&real, "original\n").unwrap();
    let jp = journal_path_of(&h, &def);
    std::fs::create_dir_all(jp.parent().unwrap()).unwrap();
    std::fs::write(&jp, b"{ not json").unwrap();
    assert!(swap_in(&h.ctx, "probe-b", &def, &StoreId::steam(), &h.running_from).is_err());
    assert_eq!(std::fs::read_to_string(&real).unwrap(), "original\n");
}

#[test]
fn probe_restore_without_a_session_is_harmless() {
    let h = TestHarness::new();
    let def = test_skyrim_definition();
    let r = restore(&h.ctx, &def, &StoreId::steam());
    // Either Ok with nothing restored or a clear "nothing to restore"; never a panic.
    let _ = r;
}
