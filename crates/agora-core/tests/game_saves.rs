//! An instance's save choice (MASTER_SPEC §26.5): the game's setting is pointed at a folder of the
//! instance's own, put back exactly when the choice goes back, and no save file is ever moved.
//! Uses the Skyrim SE definition the app ships and the test override for the user-data roots.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use agora_core::ctx::CoreContext;
use agora_core::game_ini::{self, IniError};
use agora_core::game_instance::{self, GameInstanceManifest, SavesChoice};
use agora_core::game_saves::{self, SavesError, SettingChange};
use agora_core::game_user_files::{record_process, swap_in};
use agora_core::process_identity;
use agora_game_api::{BaseReference, GameDefinition, GameId, InstallId, RuntimeIdentity, StoreId};
use tempfile::TempDir;

static TEST_LOCK: Mutex<()> = Mutex::new(());

mod common;

use common::skyrim_definition as skyrim;

struct Harness {
    _lock: MutexGuard<'static, ()>,
    _data: TempDir,
    user: TempDir,
    ctx: CoreContext,
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

    fn my_games(&self) -> PathBuf {
        self.user
            .path()
            .join("documents")
            .join("My Games")
            .join("Skyrim Special Edition")
    }

    fn real_ini(&self) -> PathBuf {
        self.my_games().join("Skyrim.ini")
    }

    fn shared_saves(&self) -> PathBuf {
        self.my_games().join("Saves")
    }

    fn write_real(&self, bytes: &[u8]) {
        std::fs::create_dir_all(self.my_games()).unwrap();
        std::fs::write(self.real_ini(), bytes).unwrap();
    }

    fn copy_of(&self, instance_id: &str) -> PathBuf {
        self.ctx
            .paths
            .instance_dir(instance_id)
            .unwrap()
            .join("user")
            .join("Skyrim.ini")
    }

    fn make_instance(&self, instance_id: &str) {
        let dir = self.ctx.paths.instance_dir(instance_id).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        let game = GameId::new("skyrim-se").unwrap();
        let runtime = RuntimeIdentity {
            game: game.clone(),
            store: StoreId::steam(),
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

    fn choice(
        &self,
        instance_id: &str,
        def: &GameDefinition,
        choice: SavesChoice,
    ) -> SettingChange {
        game_saves::set_choice(&self.ctx, instance_id, def, choice)
            .unwrap()
            .setting
    }

    fn saves_of(&self, instance_id: &str) -> SavesChoice {
        game_instance::get_manifest(&self.ctx, instance_id)
            .unwrap()
            .saves
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        std::env::remove_var("AGORA_TEST_USER_DATA_ROOT");
    }
}

/// A game file with a [General] section and a setting that the save choice will use.
const REAL: &[u8] = b"[General]\r\nsLanguage=ENGLISH\r\n[Display]\r\nfGamma=1.0\r\n";

#[test]
fn own_writes_the_key_with_the_instance_id_and_creates_its_folder() {
    let h = Harness::new();
    let def = skyrim();
    h.make_instance("survival-run");
    h.write_real(REAL);

    let setting = h.choice("survival-run", &def, SavesChoice::Own);

    assert_eq!(
        setting,
        SettingChange::Set {
            value: r"Saves\Agora\survival-run\".into()
        }
    );
    assert_eq!(
        std::fs::read(h.copy_of("survival-run")).unwrap(),
        b"[General]\r\nsLanguage=ENGLISH\r\nSLocalSavePath=Saves\\Agora\\survival-run\\\r\n[Display]\r\nfGamma=1.0\r\n"
    );
    assert!(h
        .my_games()
        .join("Saves")
        .join("Agora")
        .join("survival-run")
        .is_dir());
    assert_eq!(h.saves_of("survival-run"), SavesChoice::Own);
    assert_eq!(
        std::fs::read(h.real_ini()).unwrap(),
        REAL,
        "the game's own file is not edited"
    );
}

#[test]
fn shared_restores_a_pre_existing_value_exactly() {
    let h = Harness::new();
    let def = skyrim();
    h.make_instance("run-two");
    let original: &[u8] =
        b"[General]\r\nSLocalSavePath = Saves\\Custom\\  \r\n[Display]\r\nfGamma=1.0\r\n";
    h.write_real(original);

    h.choice("run-two", &def, SavesChoice::Own);
    let setting = h.choice("run-two", &def, SavesChoice::Shared);

    assert_eq!(
        setting,
        SettingChange::Restored {
            value: r"Saves\Custom\".into()
        }
    );
    assert_eq!(
        std::fs::read(h.copy_of("run-two")).unwrap(),
        original,
        "the line is back byte for byte, spacing and all"
    );
    assert_eq!(h.saves_of("run-two"), SavesChoice::Shared);
}

#[test]
fn shared_removes_a_key_that_was_absent() {
    let h = Harness::new();
    let def = skyrim();
    h.make_instance("run-three");
    h.write_real(REAL);

    h.choice("run-three", &def, SavesChoice::Own);
    let setting = h.choice("run-three", &def, SavesChoice::Shared);

    assert_eq!(setting, SettingChange::Removed);
    assert_eq!(std::fs::read(h.copy_of("run-three")).unwrap(), REAL);
}

#[test]
fn shared_after_a_hand_edit_leaves_the_setting_and_says_so() {
    let h = Harness::new();
    let def = skyrim();
    h.make_instance("run-four");
    h.write_real(REAL);

    h.choice("run-four", &def, SavesChoice::Own);
    game_ini::set_value(
        &h.ctx,
        "run-four",
        &def,
        "user/Skyrim.ini",
        "General",
        "SLocalSavePath",
        r"Saves\Mine\",
    )
    .unwrap();
    let setting = h.choice("run-four", &def, SavesChoice::Shared);

    assert_eq!(
        setting,
        SettingChange::LeftAlone {
            now: Some(r"Saves\Mine\".into())
        }
    );
    let read = game_ini::read(&h.ctx, "run-four", &def, "user/Skyrim.ini").unwrap();
    assert_eq!(
        read.document.get("General", "SLocalSavePath").as_deref(),
        Some(r"Saves\Mine\"),
        "the hand edit is kept"
    );
    assert_eq!(h.saves_of("run-four"), SavesChoice::Shared);
}

#[test]
fn toggling_twice_is_stable_and_a_second_own_keeps_the_first_record() {
    let h = Harness::new();
    let def = skyrim();
    h.make_instance("run-five");
    h.write_real(REAL);

    h.choice("run-five", &def, SavesChoice::Own);
    let state = h
        .ctx
        .paths
        .instance_dir("run-five")
        .unwrap()
        .join("saves_state.json");
    let first_record = std::fs::read(&state).unwrap();
    assert_eq!(
        h.choice("run-five", &def, SavesChoice::Own),
        SettingChange::Unchanged
    );
    assert_eq!(std::fs::read(&state).unwrap(), first_record);

    h.choice("run-five", &def, SavesChoice::Shared);
    h.choice("run-five", &def, SavesChoice::Own);
    h.choice("run-five", &def, SavesChoice::Shared);

    assert_eq!(std::fs::read(h.copy_of("run-five")).unwrap(), REAL);
    assert!(
        !state.exists(),
        "the record is spent when the choice goes back"
    );
    assert_eq!(h.saves_of("run-five"), SavesChoice::Shared);
}

#[test]
fn a_manifest_without_saves_reads_as_shared_and_a_new_one_leaves_the_key_out() {
    let h = Harness::new();
    h.make_instance("older");

    let path = h
        .ctx
        .paths
        .instance_dir("older")
        .unwrap()
        .join("instance_manifest.json");
    let mut value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert!(
        value.get("saves").is_none(),
        "a shared manifest does not write the key: {value}"
    );
    value.as_object_mut().unwrap().remove("saves");
    std::fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();

    assert_eq!(h.saves_of("older"), SavesChoice::Shared);
}

#[test]
fn an_instance_id_with_characters_that_need_care_gets_a_safe_folder_and_a_readable_setting() {
    let h = Harness::new();
    let def = skyrim();
    // Spaces, `#`, `;`, `=`, parentheses: all legal instance ids, all unsafe as an INI value or as
    // a folder name. It is the worst id a person can give, short of a reserved name.
    let id = "Run #2; x=y (ok)";
    h.make_instance(id);
    h.write_real(REAL);

    let setting = h.choice(id, &def, SavesChoice::Own);
    let SettingChange::Set { value } = setting else {
        panic!("expected the setting to be set");
    };
    assert!(value.starts_with(r"Saves\Agora\"), "{value}");
    let folder = value
        .trim_start_matches(r"Saves\Agora\")
        .trim_end_matches('\\');
    assert!(!folder.is_empty());
    assert!(
        folder
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        "{folder:?}"
    );
    assert!(h
        .my_games()
        .join("Saves")
        .join("Agora")
        .join(folder)
        .is_dir());

    let read = game_ini::read(&h.ctx, id, &def, "user/Skyrim.ini").unwrap();
    assert_eq!(
        read.document.get("General", "SLocalSavePath").as_deref(),
        Some(value.as_str()),
        "the value reads back whole"
    );
    assert_eq!(
        h.choice(id, &def, SavesChoice::Shared),
        SettingChange::Removed
    );
}

#[test]
fn status_counts_the_saves_in_the_folder_in_use_and_in_the_other() {
    let h = Harness::new();
    let def = skyrim();
    h.make_instance("counted");
    h.write_real(REAL);
    std::fs::create_dir_all(h.shared_saves()).unwrap();
    std::fs::write(h.shared_saves().join("Save1.ess"), b"one").unwrap();
    std::fs::write(h.shared_saves().join("Save2.ESS"), b"two").unwrap();
    std::fs::write(h.shared_saves().join("notes.txt"), b"not a save").unwrap();

    let shared = game_saves::status(&h.ctx, "counted", &def).unwrap();
    assert_eq!(shared.choice, SavesChoice::Shared);
    assert_eq!(shared.in_use.saves, 2);
    assert!(shared.in_use.newest.is_some());
    assert_eq!(shared.other.saves, 0);
    assert!(!shared.other.exists);

    h.choice("counted", &def, SavesChoice::Own);
    let own = game_saves::status(&h.ctx, "counted", &def).unwrap();
    assert_eq!(own.choice, SavesChoice::Own);
    assert_eq!(own.in_use.saves, 0);
    assert_eq!(own.other.saves, 2);
    assert_eq!(own.setting_value.as_deref(), Some(r"Saves\Agora\counted\"));
}

#[test]
fn switching_never_moves_copies_or_deletes_a_save() {
    let h = Harness::new();
    let def = skyrim();
    h.make_instance("keeper");
    h.write_real(REAL);
    std::fs::create_dir_all(h.shared_saves()).unwrap();
    std::fs::write(h.shared_saves().join("Quicksave.ess"), b"shared bytes").unwrap();

    h.choice("keeper", &def, SavesChoice::Own);
    assert_eq!(
        std::fs::read(h.shared_saves().join("Quicksave.ess")).unwrap(),
        b"shared bytes",
        "the shared save stays where it was, unchanged"
    );
    let own_folder = h.my_games().join("Saves").join("Agora").join("keeper");
    assert!(
        !own_folder.join("Quicksave.ess").exists(),
        "and nothing was copied to the instance's folder"
    );

    h.choice("keeper", &def, SavesChoice::Shared);
    assert_eq!(
        std::fs::read(h.shared_saves().join("Quicksave.ess")).unwrap(),
        b"shared bytes"
    );
}

#[test]
fn a_running_session_refuses_the_choice_and_the_manifest_stays_as_it_was() {
    let h = Harness::new();
    let def = skyrim();
    h.make_instance("busy");
    h.write_real(REAL);
    let running_from = h.user.path().join("running_from");
    std::fs::create_dir_all(&running_from).unwrap();
    swap_in(&h.ctx, "busy", &def, &StoreId::steam(), &running_from).unwrap();
    let live = process_identity::capture(std::process::id()).unwrap();
    record_process(&h.ctx, &def.id, &StoreId::steam(), live).unwrap();

    let err = game_saves::set_choice(&h.ctx, "busy", &def, SavesChoice::Own).unwrap_err();

    assert!(
        matches!(err, SavesError::Ini(IniError::SessionRunning { .. })),
        "{err:?}"
    );
    assert_eq!(h.saves_of("busy"), SavesChoice::Shared);
}

#[test]
fn a_game_with_no_save_location_refuses_an_own_choice() {
    let h = Harness::new();
    let mut def = skyrim();
    def.save_location.clear();
    h.make_instance("plain");

    let err = game_saves::set_choice(&h.ctx, "plain", &def, SavesChoice::Own).unwrap_err();
    assert!(matches!(err, SavesError::NoSaveLocation { .. }), "{err:?}");
}
