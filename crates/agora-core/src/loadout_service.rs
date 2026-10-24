//! Locked, context-aware entry point for applying a loadout profile.
//!
//! [`crate::loadout`] works on an `instance_dir` and knows nothing about the
//! instance lock or a running game. Every front end goes through here, so the
//! desktop and the CLI refuse a locked or running instance the same way, and
//! the whole apply, undo point included, happens under one instance lock.

use crate::ctx::Ctx;
use crate::error::{LauncherError, LauncherResult};
use crate::install_service::InstallService;
use crate::loadout::{self, LoadoutApplyError, LoadoutUpdate, UnlistedMods};
use crate::lock_manager::LockResource;

pub struct LoadoutService {
    ctx: Ctx,
}

impl LoadoutService {
    pub fn new(ctx: Ctx) -> Self {
        Self { ctx }
    }

    /// Apply a saved profile to an instance.
    ///
    /// `unlisted` says what happens to enabled mods the profile does not list.
    /// It has no default: the caller has to decide, and the desktop and the CLI
    /// both ask the user first when there are any (see [`Self::unlisted_mods`]).
    ///
    /// Refuses a locked instance, the same check an install makes, and an
    /// instance whose game is running or starting. The undo point
    /// (`before-loadout`) is taken under the instance lock, and applying rolls
    /// back to it if it fails.
    pub fn apply(
        &self,
        instance_id: &str,
        profile_name: &str,
        unlisted: UnlistedMods,
    ) -> LauncherResult<()> {
        let instance_dir = self.ctx.paths.instance_dir(instance_id)?;
        let _guard = self.ctx.lock_manager.acquire(
            LockResource::Instance(instance_id.to_string()),
            "apply_loadout",
        )?;
        InstallService::new(self.ctx.clone()).check_not_locked(instance_id)?;
        crate::instance_runtime::check_idle(&self.ctx.paths, instance_id)?;
        loadout::apply_profile_with_undo(&instance_dir, profile_name, unlisted).map_err(apply_error)
    }

    /// The enabled mods that `profile_name` does not list, sorted. Applying the
    /// profile with [`UnlistedMods::Off`] would switch these off. Reads under
    /// the instance lock, so an install in progress cannot change the answer
    /// midway.
    pub fn unlisted_mods(
        &self,
        instance_id: &str,
        profile_name: &str,
    ) -> LauncherResult<Vec<String>> {
        let instance_dir = self.ctx.paths.instance_dir(instance_id)?;
        let _guard = self.ctx.lock_manager.acquire(
            LockResource::Instance(instance_id.to_string()),
            "list_loadout_unlisted_mods",
        )?;
        loadout::unlisted_enabled_mods(&instance_dir, profile_name).map_err(|message| {
            LauncherError::Generic {
                code: "ERR_LOADOUT".into(),
                message,
            }
        })
    }
}

impl LoadoutService {
    /// What [`Self::update`] would change in `profile_name`, without writing.
    /// Reads under the instance lock, like [`Self::unlisted_mods`].
    pub fn update_preview(
        &self,
        instance_id: &str,
        profile_name: &str,
    ) -> LauncherResult<LoadoutUpdate> {
        let instance_dir = self.ctx.paths.instance_dir(instance_id)?;
        let _guard = self.ctx.lock_manager.acquire(
            LockResource::Instance(instance_id.to_string()),
            "preview_loadout_update",
        )?;
        loadout::plan_profile_update(&instance_dir, profile_name).map_err(loadout_error)
    }

    /// Save `profile_name` again so it records the instance's current setup.
    /// Returns the changes it made, which may differ from an earlier
    /// [`Self::update_preview`] if the setup changed in between.
    ///
    /// Only the profile file is written, so a locked instance or a running
    /// game does not block it.
    pub fn update(&self, instance_id: &str, profile_name: &str) -> LauncherResult<LoadoutUpdate> {
        let instance_dir = self.ctx.paths.instance_dir(instance_id)?;
        let _guard = self.ctx.lock_manager.acquire(
            LockResource::Instance(instance_id.to_string()),
            "update_loadout",
        )?;
        loadout::update_profile(&instance_dir, profile_name).map_err(loadout_error)
    }
}

fn loadout_error(message: String) -> LauncherError {
    LauncherError::Generic {
        code: "ERR_LOADOUT".into(),
        message,
    }
}

fn apply_error(error: LoadoutApplyError) -> LauncherError {
    let code = match &error {
        LoadoutApplyError::UndoPoint(_) => "ERR_SNAPSHOT_REQUIRED",
        LoadoutApplyError::Retention(_) => "ERR_RETENTION",
        LoadoutApplyError::Profile(_) | LoadoutApplyError::Apply { .. } => "ERR_LOADOUT",
    };
    LauncherError::Generic {
        code: code.into(),
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::SnapshotOrigin;
    use std::fs;
    use std::path::{Path, PathBuf};

    /// A context whose data root is a fresh temp dir, with one instance holding
    /// `mods` (`(filename, enabled)`) and its manifest.
    fn setup(tmp: &tempfile::TempDir, mods: &[(&str, bool)], locked: bool) -> (Ctx, PathBuf) {
        let ctx = Ctx::for_testing(tmp.path().to_path_buf());
        crate::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();
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
        write_manifest(&dir, mods, locked);
        (ctx, dir)
    }

    fn write_manifest(dir: &Path, mods: &[(&str, bool)], locked: bool) {
        let manifest = serde_json::json!({
            "instance_id": "demo",
            "name": "demo",
            "minecraft_version": "1.21",
            "loader": "fabric",
            "loader_version": "0.16.0",
            "is_locked": locked,
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

    /// The user's state after the profile was saved: lithium is now off.
    fn disable_lithium(dir: &Path, locked: bool) {
        fs::rename(
            dir.join("mods").join("lithium.jar"),
            dir.join("mods").join("lithium.jar.disabled"),
        )
        .unwrap();
        write_manifest(dir, &[("sodium.jar", true), ("lithium.jar", false)], locked);
    }

    #[test]
    fn a_locked_instance_is_refused_and_nothing_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, dir) = setup(&tmp, &[("sodium.jar", true), ("lithium.jar", true)], false);
        loadout::create_profile(&dir, "both").unwrap();
        disable_lithium(&dir, true);

        let err = LoadoutService::new(ctx)
            .apply("demo", "both", UnlistedMods::Off)
            .expect_err("a locked instance must not be changed");

        assert!(matches!(err, LauncherError::InstanceLocked), "got {err:?}");
        assert!(dir.join("mods").join("lithium.jar.disabled").exists());
        assert!(!dir.join("mods").join("lithium.jar").exists());
        assert!(crate::snapshot::list_snapshots(&dir).unwrap().is_empty());
    }

    #[test]
    fn a_starting_or_running_game_blocks_apply() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, dir) = setup(&tmp, &[("sodium.jar", true), ("lithium.jar", true)], false);
        loadout::create_profile(&dir, "both").unwrap();
        disable_lithium(&dir, false);
        crate::instance_runtime::publish_starting(&ctx.paths, "demo").unwrap();

        let err = LoadoutService::new(ctx)
            .apply("demo", "both", UnlistedMods::Off)
            .expect_err("a starting game must block apply");

        match err {
            LauncherError::Generic { code, .. } => {
                assert_eq!(code, "ERR_INSTANCE_LAUNCH_INDETERMINATE");
            }
            other => panic!("unexpected error: {other:?}"),
        }
        assert!(dir.join("mods").join("lithium.jar.disabled").exists());
        assert!(crate::snapshot::list_snapshots(&dir).unwrap().is_empty());
    }

    #[test]
    fn apply_takes_an_automatic_undo_point_and_releases_the_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, dir) = setup(&tmp, &[("sodium.jar", true), ("lithium.jar", true)], false);
        loadout::create_profile(&dir, "both").unwrap();
        disable_lithium(&dir, false);

        let service = LoadoutService::new(ctx);
        service.apply("demo", "both", UnlistedMods::Off).unwrap();
        assert!(dir.join("mods").join("lithium.jar").exists());

        let undo: Vec<_> = crate::snapshot::list_snapshots(&dir)
            .unwrap()
            .into_iter()
            .filter(|s| s.label.as_deref() == Some(loadout::UNDO_SNAPSHOT_LABEL))
            .collect();
        assert_eq!(undo.len(), 1, "one undo point per apply");
        assert_eq!(undo[0].effective_origin(), SnapshotOrigin::Automatic);

        // The lock is released, so a second apply goes through the same path.
        disable_lithium(&dir, false);
        service.apply("demo", "both", UnlistedMods::Off).unwrap();
        assert!(dir.join("mods").join("lithium.jar").exists());
    }

    #[test]
    fn the_caller_choice_decides_an_unlisted_mod_and_unlisted_mods_names_it() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, dir) = setup(&tmp, &[("sodium.jar", true), ("lithium.jar", true)], false);
        loadout::create_profile(&dir, "both").unwrap();
        // Installed after the profile was saved, so the profile does not list it.
        fs::write(dir.join("mods").join("newer.jar"), b"newer.jar").unwrap();
        write_manifest(
            &dir,
            &[
                ("sodium.jar", true),
                ("lithium.jar", true),
                ("newer.jar", true),
            ],
            false,
        );

        let service = LoadoutService::new(ctx);
        assert_eq!(
            service.unlisted_mods("demo", "both").unwrap(),
            vec!["newer.jar".to_string()]
        );

        service.apply("demo", "both", UnlistedMods::Keep).unwrap();
        assert!(
            dir.join("mods").join("newer.jar").exists(),
            "keep leaves the unlisted mod on"
        );

        service.apply("demo", "both", UnlistedMods::Off).unwrap();
        assert!(
            dir.join("mods").join("newer.jar.disabled").exists(),
            "off switches the unlisted mod off"
        );
        assert!(service.unlisted_mods("demo", "both").unwrap().is_empty());
    }

    #[test]
    fn update_saves_the_current_setup_through_the_instance_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, dir) = setup(&tmp, &[("sodium.jar", true), ("lithium.jar", true)], false);
        loadout::create_profile(&dir, "both").unwrap();
        // Installed after the profile was saved.
        fs::write(dir.join("mods").join("newer.jar"), b"newer.jar").unwrap();
        write_manifest(
            &dir,
            &[
                ("sodium.jar", true),
                ("lithium.jar", true),
                ("newer.jar", true),
            ],
            false,
        );

        let service = LoadoutService::new(ctx.clone());
        let preview = service.update_preview("demo", "both").unwrap();
        assert_eq!(preview.changes.len(), 1);
        assert_eq!(
            service.unlisted_mods("demo", "both").unwrap(),
            vec!["newer.jar".to_string()],
            "a preview writes nothing, so Apply still asks"
        );

        let done = service.update("demo", "both").unwrap();
        assert_eq!(done.changes, preview.changes);
        assert!(service.unlisted_mods("demo", "both").unwrap().is_empty());
    }
}
