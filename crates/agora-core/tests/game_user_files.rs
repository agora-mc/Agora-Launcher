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
// Gate 1: Two instances swap and restore
// ---------------------------------------------------------------------------

#[test]
fn two_instances_swap_and_restore() {
    let harness = TestHarness::new();
    let def = test_skyrim_definition();

    // 1. Setup user original real file
    let real_plugin_file = harness
        .local_dir()
        .join("Skyrim Special Edition")
        .join("Plugins.txt");
    std::fs::create_dir_all(real_plugin_file.parent().unwrap()).unwrap();
    std::fs::write(&real_plugin_file, "User Original Plugins\n").unwrap();

    // 2. Setup instance A and instance B files
    let inst_a_dir = harness.instance_dir("instance-a");
    let inst_a_plugin = inst_a_dir.join("user").join("Plugins.txt");
    std::fs::create_dir_all(inst_a_plugin.parent().unwrap()).unwrap();
    std::fs::write(&inst_a_plugin, "Instance A Plugins\n").unwrap();

    let inst_b_dir = harness.instance_dir("instance-b");
    let inst_b_plugin = inst_b_dir.join("user").join("Plugins.txt");
    std::fs::create_dir_all(inst_b_plugin.parent().unwrap()).unwrap();
    std::fs::write(&inst_b_plugin, "Instance B Plugins\n").unwrap();

    // --- Instance A session ---
    let outcome_a = swap_in(
        &harness.ctx,
        "instance-a",
        &def,
        &StoreId::steam(),
        &harness.running_from,
    )
    .unwrap();
    assert!(outcome_a.swapped_files > 0);

    // Real file has instance A's content
    assert_eq!(
        std::fs::read_to_string(&real_plugin_file).unwrap(),
        "Instance A Plugins\n"
    );

    // Simulate game modifying the file during session
    std::fs::write(
        &real_plugin_file,
        "Instance A Plugins Modified During Session\n",
    )
    .unwrap();

    // Restore session A
    let report_a = restore(&harness.ctx, &def, &StoreId::steam()).unwrap();
    assert!(report_a.files.iter().any(|f| f.changed));

    // Real file is back to user's original byte-for-byte
    assert_eq!(
        std::fs::read_to_string(&real_plugin_file).unwrap(),
        "User Original Plugins\n"
    );

    // Instance A's copy now holds what the game wrote
    assert_eq!(
        std::fs::read_to_string(&inst_a_plugin).unwrap(),
        "Instance A Plugins Modified During Session\n"
    );

    // --- Instance B session ---
    let outcome_b = swap_in(
        &harness.ctx,
        "instance-b",
        &def,
        &StoreId::steam(),
        &harness.running_from,
    )
    .unwrap();
    assert!(outcome_b.swapped_files > 0);

    // Real file has instance B's content
    assert_eq!(
        std::fs::read_to_string(&real_plugin_file).unwrap(),
        "Instance B Plugins\n"
    );

    // Simulate game modifying file during session B
    std::fs::write(
        &real_plugin_file,
        "Instance B Plugins Modified During Session\n",
    )
    .unwrap();

    // Restore session B
    let report_b = restore(&harness.ctx, &def, &StoreId::steam()).unwrap();
    assert!(report_b.files.iter().any(|f| f.changed));

    // Real file is back to user's original byte-for-byte
    assert_eq!(
        std::fs::read_to_string(&real_plugin_file).unwrap(),
        "User Original Plugins\n"
    );

    // Instance B's copy holds what game wrote
    assert_eq!(
        std::fs::read_to_string(&inst_b_plugin).unwrap(),
        "Instance B Plugins Modified During Session\n"
    );
    // Instance A's copy remains intact
    assert_eq!(
        std::fs::read_to_string(&inst_a_plugin).unwrap(),
        "Instance A Plugins Modified During Session\n"
    );
}

// ---------------------------------------------------------------------------
// Gate 2: Two launches contend, second refused naming first
// ---------------------------------------------------------------------------

#[test]
fn two_launches_contend_second_refused_naming_first() {
    let harness = TestHarness::new();
    let def = test_skyrim_definition();

    let real_plugin_file = harness
        .local_dir()
        .join("Skyrim Special Edition")
        .join("Plugins.txt");
    std::fs::create_dir_all(real_plugin_file.parent().unwrap()).unwrap();
    std::fs::write(&real_plugin_file, "Original Plugins\n").unwrap();

    let inst_a_dir = harness.instance_dir("instance-a");
    let inst_a_plugin = inst_a_dir.join("user").join("Plugins.txt");
    std::fs::create_dir_all(inst_a_plugin.parent().unwrap()).unwrap();
    std::fs::write(&inst_a_plugin, "Instance A\n").unwrap();

    let inst_b_dir = harness.instance_dir("instance-b");
    let inst_b_plugin = inst_b_dir.join("user").join("Plugins.txt");
    std::fs::create_dir_all(inst_b_plugin.parent().unwrap()).unwrap();
    std::fs::write(&inst_b_plugin, "Instance B\n").unwrap();

    // Launch instance A
    swap_in(
        &harness.ctx,
        "instance-a",
        &def,
        &StoreId::steam(),
        &harness.running_from,
    )
    .unwrap();

    // Simulate A's session running with a live process identity
    let live_id = process_identity::capture(std::process::id()).unwrap();
    record_process(&harness.ctx, &def.id, &StoreId::steam(), live_id).unwrap();

    // Instance B contends on the same game & store
    let err = swap_in(
        &harness.ctx,
        "instance-b",
        &def,
        &StoreId::steam(),
        &harness.running_from,
    )
    .unwrap_err();

    let msg = err.to_string();
    assert!(
        msg.contains("instance-a"),
        "error must name instance A that holds it: {msg}"
    );
    assert!(
        msg.contains("is running as instance instance-a; close it first"),
        "error must explain why: {msg}"
    );

    // Clean up journal so test completes cleanly
    let journal_path = harness
        .ctx
        .paths
        .user_files_journal_path(def.id.as_str(), StoreId::steam().as_str());
    let _ = std::fs::remove_file(journal_path);
}

// ---------------------------------------------------------------------------
// Gate 3: Failed launch restores cleanly
// ---------------------------------------------------------------------------

#[test]
fn failed_launch_restores_cleanly() {
    let harness = TestHarness::new();
    let def = test_skyrim_definition();

    let real_plugin_file = harness
        .local_dir()
        .join("Skyrim Special Edition")
        .join("Plugins.txt");
    std::fs::create_dir_all(real_plugin_file.parent().unwrap()).unwrap();
    std::fs::write(&real_plugin_file, "Original Plugins\n").unwrap();

    let inst_dir = harness.instance_dir("instance-a");
    let inst_plugin = inst_dir.join("user").join("Plugins.txt");
    std::fs::create_dir_all(inst_plugin.parent().unwrap()).unwrap();
    std::fs::write(&inst_plugin, "Instance A Plugins\n").unwrap();

    // 1. Swap in succeeds
    swap_in(
        &harness.ctx,
        "instance-a",
        &def,
        &StoreId::steam(),
        &harness.running_from,
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(&real_plugin_file).unwrap(),
        "Instance A Plugins\n"
    );

    // 2. Simulated spawn failure immediately triggers restore
    let report = restore(&harness.ctx, &def, &StoreId::steam()).unwrap();
    assert!(!report.files.is_empty());

    // 3. Real file is left exactly as before and no journal remains
    assert_eq!(
        std::fs::read_to_string(&real_plugin_file).unwrap(),
        "Original Plugins\n"
    );
    assert!(status(&harness.ctx, &def.id, &StoreId::steam()).is_none());
}

// ---------------------------------------------------------------------------
// Gate 4: Agora killed while running, then restarted
// ---------------------------------------------------------------------------

#[test]
fn agora_killed_while_running_then_restarted() {
    let harness = TestHarness::new();
    let def = test_skyrim_definition();

    let real_plugin_file = harness
        .local_dir()
        .join("Skyrim Special Edition")
        .join("Plugins.txt");
    std::fs::create_dir_all(real_plugin_file.parent().unwrap()).unwrap();
    std::fs::write(&real_plugin_file, "Original Plugins\n").unwrap();

    let inst_dir = harness.instance_dir("instance-a");
    let inst_plugin = inst_dir.join("user").join("Plugins.txt");
    std::fs::create_dir_all(inst_plugin.parent().unwrap()).unwrap();
    std::fs::write(&inst_plugin, "Instance A Plugins\n").unwrap();

    swap_in(
        &harness.ctx,
        "instance-a",
        &def,
        &StoreId::steam(),
        &harness.running_from,
    )
    .unwrap();

    // Record live process identity
    let live_id = process_identity::capture(std::process::id()).unwrap();
    record_process(&harness.ctx, &def.id, &StoreId::steam(), live_id).unwrap();

    // While process is alive, restore refuses and changes nothing
    let restore_err = restore(&harness.ctx, &def, &StoreId::steam()).unwrap_err();
    assert!(
        restore_err.to_string().contains("is active"),
        "must refuse while running: {restore_err}"
    );
    assert_eq!(
        std::fs::read_to_string(&real_plugin_file).unwrap(),
        "Instance A Plugins\n"
    );

    // Now simulate process gone: replace process in journal with a dead PID
    let journal_path = harness
        .ctx
        .paths
        .user_files_journal_path(def.id.as_str(), StoreId::steam().as_str());
    let text = std::fs::read_to_string(&journal_path).unwrap();
    let mut journal: Journal = serde_json::from_str(&text).unwrap();
    journal.processes = vec![process_identity::ProcessIdentity {
        pid: 999_999_999,
        start_time: 12345,
        expected_exe: None,
    }];
    std::fs::write(&journal_path, serde_json::to_string(&journal).unwrap()).unwrap();

    assert!(!is_session_running(&journal));

    // Once process is gone, restore succeeds
    let report = restore(&harness.ctx, &def, &StoreId::steam()).unwrap();
    assert!(!report.files.is_empty());
    assert_eq!(
        std::fs::read_to_string(&real_plugin_file).unwrap(),
        "Original Plugins\n"
    );
    assert!(status(&harness.ctx, &def.id, &StoreId::steam()).is_none());
}

// ---------------------------------------------------------------------------
// Gate 5: File edited externally before recovery is kept
// ---------------------------------------------------------------------------

#[test]
fn file_edited_externally_before_recovery_is_kept() {
    let harness = TestHarness::new();
    let def = test_skyrim_definition();

    let real_plugin_file = harness
        .local_dir()
        .join("Skyrim Special Edition")
        .join("Plugins.txt");
    std::fs::create_dir_all(real_plugin_file.parent().unwrap()).unwrap();
    std::fs::write(&real_plugin_file, "Original Plugins\n").unwrap();

    let inst_dir = harness.instance_dir("instance-a");
    let inst_plugin = inst_dir.join("user").join("Plugins.txt");
    std::fs::create_dir_all(inst_plugin.parent().unwrap()).unwrap();
    std::fs::write(&inst_plugin, "Instance A Plugins\n").unwrap();

    swap_in(
        &harness.ctx,
        "instance-a",
        &def,
        &StoreId::steam(),
        &harness.running_from,
    )
    .unwrap();

    // After session, external tool edits the real file
    std::fs::write(&real_plugin_file, "Externally Edited By Player\n").unwrap();

    let report = restore(&harness.ctx, &def, &StoreId::steam()).unwrap();
    let file_report = report
        .files
        .iter()
        .find(|f| f.instance_path == "user/Plugins.txt")
        .unwrap();
    assert!(
        file_report.changed,
        "external edit must be reported as changed"
    );

    // The edited contents are kept in the instance's copy
    assert_eq!(
        std::fs::read_to_string(&inst_plugin).unwrap(),
        "Externally Edited By Player\n"
    );

    // And the original is back in place in the user profile
    assert_eq!(
        std::fs::read_to_string(&real_plugin_file).unwrap(),
        "Original Plugins\n"
    );
}

// ---------------------------------------------------------------------------
// Gate 6: Interrupted swap restores correctly
// ---------------------------------------------------------------------------

#[test]
fn interrupted_swap_restores_correctly() {
    let harness = TestHarness::new();
    let def = test_skyrim_definition();

    let real_plugin_file = harness
        .local_dir()
        .join("Skyrim Special Edition")
        .join("Plugins.txt");
    std::fs::create_dir_all(real_plugin_file.parent().unwrap()).unwrap();
    std::fs::write(&real_plugin_file, "Original Plugins\n").unwrap();

    let real_ini_file = harness
        .documents_dir()
        .join("My Games")
        .join("Skyrim Special Edition")
        .join("Skyrim.ini");
    std::fs::create_dir_all(real_ini_file.parent().unwrap()).unwrap();
    std::fs::write(&real_ini_file, "Original Skyrim.ini\n").unwrap();

    let inst_dir = harness.instance_dir("instance-a");
    let inst_plugin = inst_dir.join("user").join("Plugins.txt");
    std::fs::create_dir_all(inst_plugin.parent().unwrap()).unwrap();
    std::fs::write(&inst_plugin, "Instance A Plugins\n").unwrap();

    let inst_ini = inst_dir.join("user").join("Skyrim.ini");
    std::fs::create_dir_all(inst_ini.parent().unwrap()).unwrap();
    std::fs::write(&inst_ini, "Instance A Skyrim.ini\n").unwrap();

    // Setup an interrupted session on disk:
    // File 0 (Plugins.txt) was swapped (swapped: true, backup written, real file updated)
    // File 1 (Skyrim.ini) was NOT swapped (swapped: false, real file untouched)
    let backup_dir = harness
        .ctx
        .paths
        .user_files_backup_dir(def.id.as_str(), StoreId::steam().as_str());
    std::fs::create_dir_all(&backup_dir).unwrap();
    let backup_0 = backup_dir.join("0");
    std::fs::copy(&real_plugin_file, &backup_0).unwrap();
    std::fs::write(&real_plugin_file, "Swapped Plugins Content\n").unwrap();

    let journal = Journal {
        game: def.id.clone(),
        store: StoreId::steam(),
        instance_id: "instance-a".to_string(),
        created_at: "2026-10-05T00:00:00Z".to_string(),
        running_from: harness.running_from.clone(),
        processes: vec![],
        files: vec![
            JournaledFile {
                real_path: real_plugin_file.clone(),
                instance_path: RelPath::new("user/Plugins.txt").unwrap(),
                instance_copy_path: inst_plugin.clone(),
                existed: true,
                backup_path: Some(backup_0),
                original_sha256: Some(sha256_hex("Original Plugins\n".as_bytes())),
                written_sha256: sha256_hex("Swapped Plugins Content\n".as_bytes()),
                swapped: true,
            },
            JournaledFile {
                real_path: real_ini_file.clone(),
                instance_path: RelPath::new("user/Skyrim.ini").unwrap(),
                instance_copy_path: inst_ini.clone(),
                existed: true,
                backup_path: None,
                original_sha256: None,
                written_sha256: String::new(),
                swapped: false,
            },
        ],
    };

    let journal_path = harness
        .ctx
        .paths
        .user_files_journal_path(def.id.as_str(), StoreId::steam().as_str());
    std::fs::write(&journal_path, serde_json::to_string(&journal).unwrap()).unwrap();

    // Restore should correctly restore file 0 and skip file 1
    let report = restore(&harness.ctx, &def, &StoreId::steam()).unwrap();
    assert_eq!(report.files.len(), 1);

    // File 0 is restored to its original
    assert_eq!(
        std::fs::read_to_string(&real_plugin_file).unwrap(),
        "Original Plugins\n"
    );

    // File 1 is untouched
    assert_eq!(
        std::fs::read_to_string(&real_ini_file).unwrap(),
        "Original Skyrim.ini\n"
    );

    // Journal and backups deleted
    assert!(!journal_path.exists());
    assert!(!backup_dir.exists());
}

// ---------------------------------------------------------------------------
// Gate 7: First launch with no instance copy keeps user file unchanged
// ---------------------------------------------------------------------------

#[test]
fn first_launch_no_instance_copy_keeps_user_file_unchanged() {
    let harness = TestHarness::new();
    let def = test_skyrim_definition();

    let real_ini_file = harness
        .documents_dir()
        .join("My Games")
        .join("Skyrim Special Edition")
        .join("Skyrim.ini");
    std::fs::create_dir_all(real_ini_file.parent().unwrap()).unwrap();
    std::fs::write(&real_ini_file, "User Original Settings INI\n").unwrap();

    // Instance has NO copy yet
    let inst_dir = harness.instance_dir("instance-a");
    let inst_ini = inst_dir.join("user").join("Skyrim.ini");
    assert!(!inst_ini.exists());

    // Swap in
    swap_in(
        &harness.ctx,
        "instance-a",
        &def,
        &StoreId::steam(),
        &harness.running_from,
    )
    .unwrap();

    // First launch keeps user's file unchanged!
    assert_eq!(
        std::fs::read_to_string(&real_ini_file).unwrap(),
        "User Original Settings INI\n"
    );

    // Game writes new settings during session
    std::fs::write(&real_ini_file, "Modified Settings INI From Game\n").unwrap();

    // Restore
    restore(&harness.ctx, &def, &StoreId::steam()).unwrap();

    // Real file has user's original back
    assert_eq!(
        std::fs::read_to_string(&real_ini_file).unwrap(),
        "User Original Settings INI\n"
    );

    // Instance's copy now exists and holds the game's settings
    assert_eq!(
        std::fs::read_to_string(&inst_ini).unwrap(),
        "Modified Settings INI From Game\n"
    );
}

// ---------------------------------------------------------------------------
// Gate 8: Mapping for another store not touched; Plugins.txt matches plugins.txt
// ---------------------------------------------------------------------------

#[test]
fn mapping_for_another_store_not_touched_and_case_insensitive_matching() {
    let harness = TestHarness::new();

    // Create a definition where Steam mapping uses lowercase "plugins.txt"
    let mut def = test_skyrim_definition();
    def.user_files[0].source = GamePath::UserData {
        location: UserDataLocation::LocalAppData,
        path: RelPath::new("Skyrim Special Edition/plugins.txt").unwrap(),
    };

    // On disk, create "Plugins.txt" (TitleCase) for Steam, and GOG file
    let steam_real_file = harness
        .local_dir()
        .join("Skyrim Special Edition")
        .join("Plugins.txt");
    std::fs::create_dir_all(steam_real_file.parent().unwrap()).unwrap();
    std::fs::write(&steam_real_file, "Steam Original Plugins\n").unwrap();

    let gog_real_file = harness
        .local_dir()
        .join("Skyrim Special Edition GOG")
        .join("Plugins.txt");
    std::fs::create_dir_all(gog_real_file.parent().unwrap()).unwrap();
    std::fs::write(&gog_real_file, "GOG Original Plugins\n").unwrap();

    let inst_dir = harness.instance_dir("instance-a");
    let inst_plugin = inst_dir.join("user").join("Plugins.txt");
    std::fs::create_dir_all(inst_plugin.parent().unwrap()).unwrap();
    std::fs::write(&inst_plugin, "Instance A Swapped Plugins\n").unwrap();

    // Swap in Steam
    swap_in(
        &harness.ctx,
        "instance-a",
        &def,
        &StoreId::steam(),
        &harness.running_from,
    )
    .unwrap();

    // GOG mapping for another store was completely untouched
    assert_eq!(
        std::fs::read_to_string(&gog_real_file).unwrap(),
        "GOG Original Plugins\n"
    );

    // Plugins.txt matched the mapping that said plugins.txt
    assert_eq!(
        std::fs::read_to_string(&steam_real_file).unwrap(),
        "Instance A Swapped Plugins\n"
    );

    // Restore
    restore(&harness.ctx, &def, &StoreId::steam()).unwrap();
    assert_eq!(
        std::fs::read_to_string(&steam_real_file).unwrap(),
        "Steam Original Plugins\n"
    );
    assert_eq!(
        std::fs::read_to_string(&gog_real_file).unwrap(),
        "GOG Original Plugins\n"
    );
}

// ---------------------------------------------------------------------------
// Gate 9: Real file not existing before session deleted on restore
// ---------------------------------------------------------------------------

#[test]
fn real_file_not_existing_before_session_deleted_on_restore() {
    let harness = TestHarness::new();
    let def = test_skyrim_definition();

    let real_plugin_file = harness
        .local_dir()
        .join("Skyrim Special Edition")
        .join("Plugins.txt");
    assert!(!real_plugin_file.exists());

    let inst_dir = harness.instance_dir("instance-a");
    let inst_plugin = inst_dir.join("user").join("Plugins.txt");
    std::fs::create_dir_all(inst_plugin.parent().unwrap()).unwrap();
    std::fs::write(&inst_plugin, "Instance Initial Plugins\n").unwrap();

    // Swap in
    swap_in(
        &harness.ctx,
        "instance-a",
        &def,
        &StoreId::steam(),
        &harness.running_from,
    )
    .unwrap();

    // Real file was created with instance contents
    assert_eq!(
        std::fs::read_to_string(&real_plugin_file).unwrap(),
        "Instance Initial Plugins\n"
    );

    // Game writes new content
    std::fs::write(&real_plugin_file, "Game Created Content\n").unwrap();

    // Restore
    restore(&harness.ctx, &def, &StoreId::steam()).unwrap();

    // Real file that did not exist before the session is deleted on restore
    assert!(
        !real_plugin_file.exists(),
        "file must be deleted on restore if it did not exist before"
    );

    // And its session contents go to the instance
    assert_eq!(
        std::fs::read_to_string(&inst_plugin).unwrap(),
        "Game Created Content\n"
    );
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}
