//! Per-instance INI copies (MASTER_SPEC §26.5): seeding from the game's file, edits that touch only
//! the copy, and the refusals while a session holds the game's files. Uses the Skyrim SE definition
//! the app ships, and the test override for the user-data roots, never the real Documents folder.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use agora_core::ctx::CoreContext;
use agora_core::game_ini::{self, IniError, IniSource};
use agora_core::game_instance::GameInstanceManifest;
use agora_core::game_user_files::{record_process, restore, swap_in};
use agora_core::process_identity;
use agora_game_api::{BaseReference, GameDefinition, GameId, InstallId, RuntimeIdentity, StoreId};
use tempfile::TempDir;

static TEST_LOCK: Mutex<()> = Mutex::new(());

mod common;

use common::skyrim_definition as skyrim;

struct Harness {
    _lock: MutexGuard<'static, ()>,
    _data: TempDir,
    pub user: TempDir,
    pub ctx: CoreContext,
}

impl Harness {
    fn new() -> Self {
        let lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let data = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        std::env::set_var("AGORA_TEST_USER_DATA_ROOT", user.path());
        let ctx = CoreContext::for_testing(data.path().join("app_data"));
        Self {
            _lock: lock,
            _data: data,
            user,
            ctx,
        }
    }

    pub fn real_skyrim_ini(&self) -> PathBuf {
        self.user
            .path()
            .join("documents")
            .join("My Games")
            .join("Skyrim Special Edition")
            .join("Skyrim.ini")
    }

    pub fn write_real(&self, bytes: &[u8]) {
        let path = self.real_skyrim_ini();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    pub fn copy_of(&self, instance_id: &str) -> PathBuf {
        self.ctx
            .paths
            .instance_dir(instance_id)
            .unwrap()
            .join("user")
            .join("Skyrim.ini")
    }

    /// An instance whose manifest says it runs under `store`. Its base is not needed to find the
    /// store, so none is written.
    pub fn make_instance(&self, instance_id: &str, store: StoreId) {
        let dir = self.ctx.paths.instance_dir(instance_id).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        let game = GameId::new("skyrim-se").unwrap();
        let runtime = RuntimeIdentity {
            game: game.clone(),
            store,
            version: "1.6.1170.0".into(),
            build: None,
        };
        let manifest = GameInstanceManifest::new(
            game,
            instance_id,
            instance_id,
            Some(runtime),
            BaseReference::Unpinned {
                install: InstallId::new("steam:489830").unwrap(),
                reason: "test".into(),
            },
        );
        std::fs::write(
            dir.join("instance_manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
    }

    /// Swap the game's files in for `instance_id`, as a launch does, and return the running folder.
    pub fn start_session(&self, instance_id: &str, def: &GameDefinition) -> PathBuf {
        let running_from = self.user.path().join("running_from");
        std::fs::create_dir_all(&running_from).unwrap();
        swap_in(
            &self.ctx,
            instance_id,
            def,
            &StoreId::steam(),
            &running_from,
        )
        .unwrap();
        running_from
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        std::env::remove_var("AGORA_TEST_USER_DATA_ROOT");
    }
}

/// The game's file as a player has it: a byte order mark, comments, CRLF endings, odd spacing.
const REAL: &str = "\u{feff}; Skyrim settings\r\n[General]\r\nsLanguage=ENGLISH\r\n  iMaxArrowsWanted =  9  \r\n\r\n[Display]\r\nfGamma=1.0\r\njunk line\r\n";

#[test]
fn an_instance_without_a_copy_is_seeded_from_the_game_file_and_only_the_key_changes() {
    let h = Harness::new();
    let def = skyrim();
    h.make_instance("inst-a", StoreId::steam());
    h.write_real(REAL.as_bytes());

    let changed = game_ini::set_value(
        &h.ctx,
        "inst-a",
        &def,
        "user/Skyrim.ini",
        "Display",
        "fGamma",
        "2.2",
    )
    .unwrap();

    assert!(changed);
    assert_eq!(
        std::fs::read(h.copy_of("inst-a")).unwrap(),
        REAL.replace("fGamma=1.0", "fGamma=2.2").as_bytes()
    );
    assert_eq!(
        std::fs::read(h.real_skyrim_ini()).unwrap(),
        REAL.as_bytes(),
        "the game's own file is never edited"
    );
}

#[test]
fn reading_shows_the_game_file_and_seeds_nothing() {
    let h = Harness::new();
    let def = skyrim();
    h.make_instance("inst-a", StoreId::steam());
    h.write_real(REAL.as_bytes());

    let read = game_ini::read(&h.ctx, "inst-a", &def, "user/Skyrim.ini").unwrap();

    assert_eq!(read.source, IniSource::GameFile);
    assert_eq!(
        read.document.get("Display", "fGamma").as_deref(),
        Some("1.0")
    );
    assert!(!h.copy_of("inst-a").exists(), "a read creates no copy");
}

#[test]
fn an_instance_with_no_game_file_starts_its_copy_empty_and_creates_no_game_file() {
    let h = Harness::new();
    let def = skyrim();
    h.make_instance("inst-b", StoreId::steam());

    assert!(game_ini::set_value(
        &h.ctx,
        "inst-b",
        &def,
        "user/Skyrim.ini",
        "General",
        "k",
        "v"
    )
    .unwrap());

    assert_eq!(
        std::fs::read(h.copy_of("inst-b")).unwrap(),
        b"[General]\r\nk=v\r\n"
    );
    assert!(!h.real_skyrim_ini().exists());
    let read = game_ini::read(&h.ctx, "inst-b", &def, "user/Skyrim.ini").unwrap();
    assert_eq!(read.source, IniSource::Copy);
}

#[test]
fn a_change_that_changes_nothing_writes_no_copy() {
    let h = Harness::new();
    let def = skyrim();
    h.make_instance("inst-c", StoreId::steam());
    h.write_real(REAL.as_bytes());

    assert!(
        !game_ini::unset_value(&h.ctx, "inst-c", &def, "user/Skyrim.ini", "General", "nope")
            .unwrap()
    );
    assert!(!h.copy_of("inst-c").exists());
}

#[test]
fn names_compare_case_insensitively_and_the_spelling_in_the_file_is_kept() {
    let h = Harness::new();
    let def = skyrim();
    h.make_instance("inst-d", StoreId::steam());
    h.write_real(REAL.as_bytes());

    assert!(game_ini::set_value(
        &h.ctx,
        "inst-d",
        &def,
        "user/Skyrim.ini",
        "DISPLAY",
        "FGAMMA",
        "3"
    )
    .unwrap());

    let text = std::fs::read_to_string(h.copy_of("inst-d")).unwrap();
    assert!(text.contains("fGamma=3\r\n"), "{text:?}");
    assert!(!text.contains("fGamma=1.0"));
}

#[test]
fn unset_removes_the_key_from_the_copy_only() {
    let h = Harness::new();
    let def = skyrim();
    h.make_instance("inst-e", StoreId::steam());
    h.write_real(REAL.as_bytes());

    assert!(game_ini::unset_value(
        &h.ctx,
        "inst-e",
        &def,
        "user/Skyrim.ini",
        "Display",
        "fGamma"
    )
    .unwrap());

    let read = game_ini::read(&h.ctx, "inst-e", &def, "user/Skyrim.ini").unwrap();
    assert_eq!(read.document.get("Display", "fGamma"), None);
    assert_eq!(
        read.document.get("General", "iMaxArrowsWanted").as_deref(),
        Some("9")
    );
    assert!(h.real_skyrim_ini().exists());
}

#[test]
fn an_edit_is_refused_while_the_games_session_runs_and_the_copy_is_left_alone() {
    let h = Harness::new();
    let def = skyrim();
    h.make_instance("inst-f", StoreId::steam());
    h.write_real(REAL.as_bytes());
    h.start_session("inst-f", &def);
    let live = process_identity::capture(std::process::id()).unwrap();
    record_process(&h.ctx, &def.id, &StoreId::steam(), live).unwrap();

    let err = game_ini::set_value(
        &h.ctx,
        "inst-f",
        &def,
        "user/Skyrim.ini",
        "Display",
        "fGamma",
        "2",
    )
    .unwrap_err();

    assert!(matches!(err, IniError::SessionRunning { .. }), "{err:?}");
    assert!(!h.copy_of("inst-f").exists());
}

#[test]
fn an_edit_is_refused_while_an_earlier_session_is_not_put_back() {
    let h = Harness::new();
    let def = skyrim();
    h.make_instance("inst-g", StoreId::steam());
    h.write_real(REAL.as_bytes());
    h.start_session("inst-g", &def);

    let err = game_ini::set_value(
        &h.ctx,
        "inst-g",
        &def,
        "user/Skyrim.ini",
        "Display",
        "fGamma",
        "2",
    )
    .unwrap_err();
    assert!(matches!(err, IniError::NeedsRestore { .. }), "{err:?}");

    restore(&h.ctx, &def, &StoreId::steam()).unwrap();
    assert!(game_ini::set_value(
        &h.ctx,
        "inst-g",
        &def,
        "user/Skyrim.ini",
        "Display",
        "fGamma",
        "2"
    )
    .unwrap());
}

#[test]
fn an_unreadable_game_file_is_an_error_and_not_an_empty_file() {
    let h = Harness::new();
    let def = skyrim();
    h.make_instance("inst-h", StoreId::steam());
    // A folder where the game's file should be: it exists, and it cannot be read as a file.
    std::fs::create_dir_all(h.real_skyrim_ini()).unwrap();

    let read_err = game_ini::read(&h.ctx, "inst-h", &def, "user/Skyrim.ini").unwrap_err();
    assert!(
        matches!(read_err, IniError::Unreadable { .. }),
        "{read_err:?}"
    );

    let set_err = game_ini::set_value(
        &h.ctx,
        "inst-h",
        &def,
        "user/Skyrim.ini",
        "General",
        "k",
        "v",
    )
    .unwrap_err();
    assert!(
        matches!(set_err, IniError::Unreadable { .. }),
        "{set_err:?}"
    );
    assert!(
        !h.copy_of("inst-h").exists(),
        "no copy is made from a file that cannot be read"
    );
}

#[test]
fn an_unreadable_copy_is_an_error_too() {
    let h = Harness::new();
    let def = skyrim();
    h.make_instance("inst-i", StoreId::steam());
    std::fs::create_dir_all(h.copy_of("inst-i")).unwrap();

    let err = game_ini::read(&h.ctx, "inst-i", &def, "user/Skyrim.ini").unwrap_err();
    assert!(matches!(err, IniError::Unreadable { .. }), "{err:?}");
}

#[test]
fn names_that_would_break_the_file_are_refused_before_anything_is_written() {
    let h = Harness::new();
    let def = skyrim();
    h.make_instance("inst-j", StoreId::steam());
    h.write_real(REAL.as_bytes());

    for (section, key, value) in [
        ("General", "a=b", "1"),
        ("General", "a", "x\r\ny"),
        ("General", "", "1"),
        ("Gen]eral", "a", "1"),
    ] {
        let err = game_ini::set_value(
            &h.ctx,
            "inst-j",
            &def,
            "user/Skyrim.ini",
            section,
            key,
            value,
        )
        .unwrap_err();
        assert!(matches!(err, IniError::Invalid(_)), "{err:?}");
    }
    assert!(!h.copy_of("inst-j").exists());
}

#[test]
fn a_file_the_game_does_not_keep_is_refused_and_a_path_out_of_the_instance_is_invalid() {
    let h = Harness::new();
    let def = skyrim();
    h.make_instance("inst-k", StoreId::steam());

    let not_kept = game_ini::read(&h.ctx, "inst-k", &def, "user/Other.ini").unwrap_err();
    assert!(
        matches!(not_kept, IniError::NotAGameFile { .. }),
        "{not_kept:?}"
    );

    let escape = game_ini::read(&h.ctx, "inst-k", &def, "../Skyrim.ini").unwrap_err();
    assert!(matches!(escape, IniError::Invalid(_)), "{escape:?}");
}

#[test]
fn listing_shows_each_file_the_store_keeps_with_its_copy_and_game_file() {
    let h = Harness::new();
    let def = skyrim();
    h.make_instance("inst-l", StoreId::steam());
    h.write_real(REAL.as_bytes());

    let files = game_ini::list_files(&h.ctx, "inst-l", &def).unwrap();

    let paths: Vec<&str> = files.iter().map(|f| f.instance_path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "user/Plugins.txt",
            "user/Skyrim.ini",
            "user/SkyrimPrefs.ini",
            "user/SkyrimCustom.ini"
        ]
    );
    let skyrim_ini = files
        .iter()
        .find(|f| f.instance_path == "user/Skyrim.ini")
        .unwrap();
    assert!(skyrim_ini.game_file_exists);
    assert!(!skyrim_ini.copy_exists);
}
