//! Locked, context-aware entry point for the guided mod bisect.
//!
//! [`agora_core::bisect`] decides what a trial should look like and owns the session
//! file. This module is the part every front end used to repeat: looking the
//! instance up by its validated id, reading its manifest, taking the instance
//! lock around the moments that rename JARs, and restoring the baseline before a
//! cancel clears the session. The desktop app and the CLI both go through here,
//! so neither can forget one of those steps.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::install_service::InstallService;
use agora_core::bisect::{self, BisectSession, BisectTrial, TrialOutcome};
use agora_core::ctx::Ctx;
use agora_core::error::{LauncherError, LauncherResult};
use agora_core::instance_runtime;
use agora_core::lock_manager::LockResource;
use agora_core::models::InstanceManifest;

/// A session plus the trial it currently wants, in one read.
#[derive(Debug, Serialize)]
pub struct BisectView {
    pub session: Option<BisectSession>,
    pub trial: Option<BisectTrial>,
}

pub struct BisectService {
    ctx: Ctx,
}

impl BisectService {
    pub fn new(ctx: Ctx) -> Self {
        Self { ctx }
    }

    /// The session and its next trial, or an empty view when no bisect is
    /// running. Reads only; takes no lock.
    pub fn view(&self, instance_id: &str) -> LauncherResult<BisectView> {
        let instance_dir = self.instance_dir(instance_id)?;
        self.view_of(&instance_dir, instance_id)
    }

    /// Begin a bisect over the currently enabled mods. Refused while one is
    /// already running. `prime_suspects` are tested first.
    pub fn start(
        &self,
        instance_id: &str,
        prime_suspects: &[String],
    ) -> LauncherResult<BisectView> {
        let instance_dir = self.instance_dir(instance_id)?;
        self.refuse_locked(instance_id)?;
        self.refuse_running(instance_id)?;
        if bisect::read_session(&instance_dir)
            .map_err(bisect_error)?
            .is_some()
        {
            return Err(bisect_error(
                "A bisect is already in progress for this instance.".into(),
            ));
        }
        let manifest = self.load_manifest(instance_id)?;
        let session = bisect::start_session(
            &manifest.mods,
            prime_suspects,
            &chrono::Utc::now().to_rfc3339(),
        )
        .map_err(bisect_error)?;
        bisect::write_session(&instance_dir, &session).map_err(bisect_error)?;
        self.view_of(&instance_dir, instance_id)
    }

    /// Apply the current trial's enable set to the instance. Separate from
    /// recording an outcome because the user has to launch the game in between.
    /// Holds the instance lock.
    pub fn apply_trial(&self, instance_id: &str) -> LauncherResult<BisectView> {
        let _guard = self.lock(instance_id, "bisect-trial")?;
        let instance_dir = self.instance_dir(instance_id)?;
        self.refuse_locked(instance_id)?;
        self.refuse_running(instance_id)?;
        let view = self.view_of(&instance_dir, instance_id)?;
        let Some(trial) = view.trial.as_ref() else {
            return Err(bisect_error("No bisect is in progress.".into()));
        };
        if trial.status != bisect::BisectStatus::AwaitingTrial {
            return Err(bisect_error("This bisect has already finished.".into()));
        }
        bisect::apply_enabled_set(&instance_dir, &trial.enable).map_err(bisect_error)?;
        self.view_of(&instance_dir, instance_id)
    }

    /// Record what happened on the last launch and narrow the pool.
    /// `reproduced` is true when the problem still happened.
    pub fn record_outcome(
        &self,
        instance_id: &str,
        reproduced: bool,
    ) -> LauncherResult<BisectView> {
        let instance_dir = self.instance_dir(instance_id)?;
        let Some(mut session) = bisect::read_session(&instance_dir).map_err(bisect_error)? else {
            return Err(bisect_error("No bisect is in progress.".into()));
        };
        let manifest = self.load_manifest(instance_id)?;
        let outcome = if reproduced {
            TrialOutcome::Reproduced
        } else {
            TrialOutcome::Clean
        };
        bisect::record_outcome(&mut session, &manifest.mods, outcome).map_err(bisect_error)?;
        bisect::write_session(&instance_dir, &session).map_err(bisect_error)?;
        self.view_of(&instance_dir, instance_id)
    }

    /// Undo the last answer and take the other half next time.
    pub fn step_back(&self, instance_id: &str) -> LauncherResult<BisectView> {
        let instance_dir = self.instance_dir(instance_id)?;
        let Some(mut session) = bisect::read_session(&instance_dir).map_err(bisect_error)? else {
            return Err(bisect_error("No bisect is in progress.".into()));
        };
        bisect::step_back(&mut session).map_err(bisect_error)?;
        bisect::write_session(&instance_dir, &session).map_err(bisect_error)?;
        self.view_of(&instance_dir, instance_id)
    }

    /// End the bisect and put every mod back the way it was. Restoring the
    /// baseline before clearing the session is the whole safety story: the
    /// baseline is the only record of what was on when the bisect started.
    /// With no bisect running this does nothing. Holds the instance lock.
    ///
    /// A locked instance can still be restored: putting the baseline back is
    /// the way out, so only a running game refuses it.
    pub fn cancel(&self, instance_id: &str) -> LauncherResult<()> {
        let _guard = self.lock(instance_id, "bisect-cancel")?;
        let instance_dir = self.instance_dir(instance_id)?;
        if let Some(session) = bisect::read_session(&instance_dir).map_err(bisect_error)? {
            self.refuse_running(instance_id)?;
            bisect::apply_enabled_set(&instance_dir, &session.baseline_enabled)
                .map_err(bisect_error)?;
        }
        bisect::clear_session(&instance_dir).map_err(bisect_error)
    }

    fn lock(
        &self,
        instance_id: &str,
        operation: &str,
    ) -> LauncherResult<agora_core::lock_manager::LockGuard> {
        self.ctx
            .lock_manager
            .acquire(LockResource::Instance(instance_id.to_string()), operation)
    }

    /// The refusals [`crate::loadout_service::LoadoutService`] makes before it
    /// changes an instance: a locked instance, and a game that is running or
    /// starting.
    fn refuse_locked(&self, instance_id: &str) -> LauncherResult<()> {
        InstallService::new(self.ctx.clone()).check_not_locked(instance_id)
    }

    fn refuse_running(&self, instance_id: &str) -> LauncherResult<()> {
        instance_runtime::check_idle(&self.ctx.paths, instance_id)
    }

    fn instance_dir(&self, instance_id: &str) -> LauncherResult<PathBuf> {
        self.ctx
            .paths
            .instance_dir(instance_id)
            .map_err(|error| LauncherError::Generic {
                code: "ERR_PATH".into(),
                message: error.to_string(),
            })
    }

    fn load_manifest(&self, instance_id: &str) -> LauncherResult<InstanceManifest> {
        let manifest_path = self
            .ctx
            .paths
            .instance_manifest(instance_id)
            .map_err(|_| LauncherError::InstanceCreateFailed)?;
        read_manifest_file(&manifest_path, instance_id)
    }

    /// The session in `instance_dir` and the trial it wants, or an empty view.
    fn view_of(&self, instance_dir: &Path, instance_id: &str) -> LauncherResult<BisectView> {
        let Some(session) = bisect::read_session(instance_dir).map_err(bisect_error)? else {
            return Ok(BisectView {
                session: None,
                trial: None,
            });
        };
        let manifest = self.load_manifest(instance_id)?;
        let trial = bisect::next_trial(&session, &manifest.mods);
        Ok(BisectView {
            session: Some(session),
            trial: Some(trial),
        })
    }
}

fn read_manifest_file(manifest_path: &Path, instance_id: &str) -> LauncherResult<InstanceManifest> {
    let text = std::fs::read_to_string(manifest_path).map_err(|_| LauncherError::Generic {
        code: "ERR_MANIFEST_MISSING".to_string(),
        message: format!("Instance manifest not found for '{}'.", instance_id),
    })?;
    serde_json::from_str(&text).map_err(|_| LauncherError::Generic {
        code: "ERR_MANIFEST_PARSE".to_string(),
        message: "Failed to parse instance manifest.".to_string(),
    })
}

fn bisect_error(message: String) -> LauncherError {
    LauncherError::Generic {
        code: "ERR_BISECT".into(),
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A context rooted in a fresh temp dir, with one instance `demo` whose mod
    /// files and manifest match `mods` (`(filename, enabled)`).
    fn setup(tmp: &tempfile::TempDir, mods: &[(&str, bool)]) -> (Ctx, PathBuf) {
        let ctx = Ctx::for_testing(tmp.path().to_path_buf());
        agora_core::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();
        fs::create_dir_all(ctx.paths.locks_root()).unwrap();
        let dir = ctx.paths.instance_dir("demo").unwrap();
        fs::create_dir_all(dir.join("mods")).unwrap();
        for (filename, enabled) in mods {
            let name = if *enabled {
                filename.to_string()
            } else {
                format!("{filename}.disabled")
            };
            fs::write(dir.join("mods").join(name), filename.as_bytes()).unwrap();
        }
        write_manifest(&dir, mods);
        (ctx, dir)
    }

    fn write_manifest(dir: &Path, mods: &[(&str, bool)]) {
        let manifest = serde_json::json!({
            "instance_id": "demo",
            "name": "demo",
            "minecraft_version": "1.21",
            "loader": "fabric",
            "loader_version": "0.16.0",
            "is_locked": false,
            "mods": mods.iter().map(|(filename, enabled)| serde_json::json!({
                "filename": filename,
                "source": "manual",
                "version": null,
                "sha256": "0".repeat(64),
                "installed_at": "2024-01-01T00:00:00Z",
                "java_packages": [],
                "modrinth_id": null,
                "registry_id": null,
                "mod_jar_id": null,
                "provided_mod_ids": [],
                "enabled": enabled,
                "content_type": "mod",
                "depends_on": [],
                "optional_deps": [],
                "incompatible_deps": [],
                "source_url": null,
            })).collect::<Vec<_>>(),
            "resourcepacks": [],
            "shaders": [],
            "datapacks": [],
            "worlds": [],
            "user_preferences": {}
        });
        fs::write(
            dir.join("instance_manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
    }

    fn four_mods() -> Vec<(&'static str, bool)> {
        vec![
            ("a.jar", true),
            ("b.jar", true),
            ("c.jar", true),
            ("d.jar", true),
        ]
    }

    /// Which mod files are on in the mods folder, sorted.
    fn enabled_files(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir.join("mods"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| !n.ends_with(".disabled"))
            .collect();
        names.sort();
        names
    }

    fn code_of(error: &LauncherError) -> String {
        error.code()
    }

    // --- Empty, hostile and missing inputs fail closed --------------------

    #[test]
    fn an_empty_or_traversal_instance_id_is_refused_by_every_entry_point() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, _dir) = setup(&tmp, &four_mods());
        let service = BisectService::new(ctx);
        for bad in ["", "   ", "../demo", "demo/..", "a:b"] {
            assert!(service.view(bad).is_err(), "view accepted {bad:?}");
            assert!(service.start(bad, &[]).is_err(), "start accepted {bad:?}");
            assert!(service.apply_trial(bad).is_err(), "apply accepted {bad:?}");
            assert!(
                service.record_outcome(bad, true).is_err(),
                "record accepted {bad:?}"
            );
            assert!(
                service.step_back(bad).is_err(),
                "step_back accepted {bad:?}"
            );
            assert!(service.cancel(bad).is_err(), "cancel accepted {bad:?}");
        }
    }

    #[test]
    fn an_unknown_instance_is_refused_by_the_commands_that_change_state() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, _dir) = setup(&tmp, &four_mods());
        let service = BisectService::new(ctx);
        assert!(service.start("ghost", &[]).is_err());
        assert!(service.apply_trial("ghost").is_err());
        assert!(service.record_outcome("ghost", false).is_err());
    }

    #[test]
    fn viewing_an_instance_with_no_bisect_is_an_empty_view_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, _dir) = setup(&tmp, &four_mods());
        let view = BisectService::new(ctx).view("demo").unwrap();
        assert!(view.session.is_none());
        assert!(view.trial.is_none());
    }

    #[test]
    fn a_bisect_needs_two_enabled_mods_to_start() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, dir) = setup(&tmp, &[("only.jar", true)]);
        let service = BisectService::new(ctx);
        let error = service.start("demo", &[]).unwrap_err();
        assert_eq!(code_of(&error), "ERR_BISECT");
        assert!(bisect::read_session(&dir).unwrap().is_none());

        let tmp2 = tempfile::tempdir().unwrap();
        let (ctx2, _dir2) = setup(&tmp2, &[]);
        assert!(BisectService::new(ctx2).start("demo", &[]).is_err());
    }

    #[test]
    fn starting_while_a_bisect_is_running_is_refused_and_keeps_the_session() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, dir) = setup(&tmp, &four_mods());
        let service = BisectService::new(ctx);
        service.start("demo", &[]).unwrap();
        let before = fs::read_to_string(bisect::session_path(&dir)).unwrap();
        assert!(service.start("demo", &[]).is_err());
        let after = fs::read_to_string(bisect::session_path(&dir)).unwrap();
        assert_eq!(
            before, after,
            "a refused start must not rewrite the session"
        );
    }

    #[test]
    fn answering_or_stepping_back_with_no_bisect_running_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, dir) = setup(&tmp, &four_mods());
        let service = BisectService::new(ctx);
        assert!(service.record_outcome("demo", true).is_err());
        assert!(service.record_outcome("demo", false).is_err());
        assert!(service.step_back("demo").is_err());
        assert!(service.apply_trial("demo").is_err());
        assert!(!bisect::session_path(&dir).exists());
        assert_eq!(enabled_files(&dir).len(), 4, "no mod may move");
    }

    #[test]
    fn cancel_with_no_bisect_running_is_a_no_op() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, dir) = setup(&tmp, &four_mods());
        BisectService::new(ctx).cancel("demo").unwrap();
        assert_eq!(enabled_files(&dir).len(), 4);
    }

    #[test]
    fn applying_a_finished_bisect_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, dir) = setup(&tmp, &four_mods());
        let service = BisectService::new(ctx);
        service.start("demo", &[]).unwrap();
        // Force a conclusion: one suspect left.
        let mut session = bisect::read_session(&dir).unwrap().unwrap();
        session.suspects = vec!["a.jar".into()];
        bisect::write_session(&dir, &session).unwrap();
        let before = enabled_files(&dir);
        assert!(service.apply_trial("demo").is_err());
        assert_eq!(enabled_files(&dir), before, "a refused apply moves nothing");
    }

    // --- Happy paths -------------------------------------------------------

    #[test]
    fn a_trial_turns_the_held_out_half_off_and_cancel_puts_every_mod_back() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, dir) = setup(&tmp, &four_mods());
        let service = BisectService::new(ctx);

        let started = service.start("demo", &[]).unwrap();
        assert_eq!(started.trial.as_ref().unwrap().disable.len(), 2);

        let applied = service.apply_trial("demo").unwrap();
        let trial = applied.trial.unwrap();
        assert_eq!(enabled_files(&dir).len(), 4 - trial.disable.len());

        service.cancel("demo").unwrap();
        assert_eq!(
            enabled_files(&dir),
            vec!["a.jar", "b.jar", "c.jar", "d.jar"],
            "cancel must restore every mod the bisect touched"
        );
        assert!(!bisect::session_path(&dir).exists());
    }

    #[test]
    fn a_reproduced_answer_narrows_the_pool_and_is_persisted() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, dir) = setup(&tmp, &four_mods());
        let service = BisectService::new(ctx);
        service.start("demo", &[]).unwrap();
        service.apply_trial("demo").unwrap();
        let view = service.record_outcome("demo", true).unwrap();
        assert_eq!(view.session.as_ref().unwrap().suspects.len(), 2);
        let reread = bisect::read_session(&dir).unwrap().unwrap();
        assert_eq!(reread.history.len(), 1);
        assert_eq!(reread.suspects.len(), 2);
    }

    #[test]
    fn stepping_back_restores_the_pool_and_takes_the_other_branch() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, _dir) = setup(&tmp, &four_mods());
        let service = BisectService::new(ctx);
        let first = service.start("demo", &[]).unwrap().trial.unwrap();
        service.record_outcome("demo", true).unwrap();
        let back = service.step_back("demo").unwrap();
        assert_eq!(back.session.as_ref().unwrap().suspects.len(), 4);
        let retry = back.trial.unwrap();
        assert_ne!(retry.disable, first.disable);
    }

    #[test]
    fn crash_named_mods_are_tested_first_and_duplicates_or_unknown_names_do_not_break_start() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, _dir) = setup(&tmp, &four_mods());
        let service = BisectService::new(ctx);
        let suspects = vec![
            "c.jar".to_string(),
            "c.jar".to_string(),
            "not-installed.jar".to_string(),
            String::new(),
        ];
        let view = service.start("demo", &suspects).unwrap();
        let session = view.session.unwrap();
        assert_eq!(session.suspects[0], "c.jar");
        assert_eq!(
            session.suspects.len(),
            4,
            "pool keeps each enabled mod once"
        );
    }

    #[test]
    fn a_disabled_mod_is_never_a_suspect() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, _dir) = setup(&tmp, &[("a.jar", true), ("b.jar", false), ("c.jar", true)]);
        let view = BisectService::new(ctx).start("demo", &[]).unwrap();
        let session = view.session.unwrap();
        assert!(!session.suspects.contains(&"b.jar".to_string()));
        assert_eq!(session.suspects.len(), 2);
    }

    // --- Locked instances and running games --------------------------------

    /// Mark the instance's manifest locked (or not), as the install lock does.
    fn set_locked(dir: &Path, locked: bool) {
        let path = dir.join("instance_manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        manifest["is_locked"] = serde_json::Value::Bool(locked);
        fs::write(path, serde_json::to_string_pretty(&manifest).unwrap()).unwrap();
    }

    fn assert_locked(error: LauncherError) {
        assert!(
            matches!(error, LauncherError::InstanceLocked),
            "expected InstanceLocked, got {error:?}"
        );
    }

    #[test]
    fn a_locked_instance_refuses_start_and_apply_and_changes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, dir) = setup(&tmp, &four_mods());
        set_locked(&dir, true);
        let service = BisectService::new(ctx);

        assert_locked(service.start("demo", &[]).unwrap_err());
        assert!(
            !bisect::session_path(&dir).exists(),
            "a refused start writes nothing"
        );

        // A session started before the lock: apply is refused and renames nothing.
        set_locked(&dir, false);
        service.start("demo", &[]).unwrap();
        set_locked(&dir, true);
        assert_locked(service.apply_trial("demo").unwrap_err());
        assert_eq!(
            enabled_files(&dir).len(),
            4,
            "a refused apply moves nothing"
        );
    }

    #[test]
    fn a_locked_instance_can_still_be_cancelled_back_to_its_baseline() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, dir) = setup(&tmp, &four_mods());
        let service = BisectService::new(ctx);
        service.start("demo", &[]).unwrap();
        service.apply_trial("demo").unwrap();
        assert_eq!(enabled_files(&dir).len(), 2);

        set_locked(&dir, true);
        service.cancel("demo").unwrap();
        assert_eq!(enabled_files(&dir).len(), 4, "cancel restores every mod");
        assert!(!bisect::session_path(&dir).exists());
    }

    #[test]
    fn a_running_game_refuses_start_apply_and_cancel_and_changes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, dir) = setup(&tmp, &four_mods());
        let paths = ctx.paths.clone();
        let service = BisectService::new(ctx);
        service.start("demo", &[]).unwrap();
        service.apply_trial("demo").unwrap();
        let session_before = fs::read_to_string(bisect::session_path(&dir)).unwrap();
        let on_before = enabled_files(&dir);

        // A launch that has not finished starting counts as running.
        agora_core::instance_runtime::publish_starting(&paths, "demo").unwrap();
        assert!(
            service.cancel("demo").is_err(),
            "cancel must refuse a starting game"
        );
        assert!(
            service.apply_trial("demo").is_err(),
            "apply must refuse a starting game"
        );
        assert_eq!(enabled_files(&dir), on_before);
        assert_eq!(
            fs::read_to_string(bisect::session_path(&dir)).unwrap(),
            session_before,
            "a refused cancel keeps the session"
        );
        agora_core::instance_runtime::clear(&paths, "demo", None).unwrap();

        // A launch whose process is alive counts as running too.
        let identity = agora_core::process_identity::capture(std::process::id()).unwrap();
        agora_core::instance_runtime::publish_running(&paths, "demo", "gen-test", identity)
            .unwrap();
        let cancel = service.cancel("demo").unwrap_err();
        assert_eq!(cancel.code(), "ERR_INSTANCE_RUNNING");
        assert_eq!(enabled_files(&dir), on_before);
        agora_core::instance_runtime::clear(&paths, "demo", None).unwrap();

        // Once the game is gone, the same cancel goes through.
        service.cancel("demo").unwrap();
        assert_eq!(enabled_files(&dir).len(), 4);
    }

    #[test]
    fn a_running_game_refuses_start() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, dir) = setup(&tmp, &four_mods());
        agora_core::instance_runtime::publish_starting(&ctx.paths, "demo").unwrap();
        assert!(BisectService::new(ctx).start("demo", &[]).is_err());
        assert!(!bisect::session_path(&dir).exists());
    }

    // --- A rename that fails part-way through a trial ----------------------

    #[test]
    fn a_trial_that_fails_midway_puts_back_the_files_it_renamed_and_keeps_the_session() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, dir) = setup(&tmp, &four_mods());
        let service = BisectService::new(ctx);
        service.start("demo", &[]).unwrap();
        let session_before = fs::read_to_string(bisect::session_path(&dir)).unwrap();
        let manifest_before = fs::read(dir.join("instance_manifest.json")).unwrap();
        // The trial turns c.jar and then d.jar off. A directory where d.jar's
        // disabled name should go makes the second rename fail after c.jar moved.
        fs::create_dir_all(dir.join("mods").join("d.jar.disabled")).unwrap();

        assert!(service.apply_trial("demo").is_err());
        assert!(
            dir.join("mods").join("c.jar").exists(),
            "c.jar is back under its name"
        );
        assert!(!dir.join("mods").join("c.jar.disabled").exists());
        assert!(dir.join("mods").join("d.jar").exists());
        assert_eq!(
            fs::read(dir.join("instance_manifest.json")).unwrap(),
            manifest_before,
            "the manifest is not written"
        );
        assert_eq!(
            fs::read_to_string(bisect::session_path(&dir)).unwrap(),
            session_before,
            "the session still describes the trial, so it can be retried"
        );
    }
}
