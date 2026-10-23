//! Locked, context-aware entry point for applying a loadout profile.
//!
//! [`crate::loadout`] works on an `instance_dir` and knows nothing about the
//! instance lock or a running game. Every front end goes through here, so the
//! desktop and the CLI refuse a locked or running instance the same way, and
//! the whole apply, undo point included, happens under one instance lock.

use crate::ctx::Ctx;
use crate::error::{LauncherError, LauncherResult};
use crate::install_service::InstallService;
use crate::loadout::{self, LoadoutApplyError};
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
    /// Refuses a locked instance, the same check an install makes, and an
    /// instance whose game is running or starting. The undo point
    /// (`before-loadout`) is taken under the instance lock, and applying rolls
    /// back to it if it fails.
    pub fn apply(&self, instance_id: &str, profile_name: &str) -> LauncherResult<()> {
        let instance_dir = self.ctx.paths.instance_dir(instance_id)?;
        let _guard = self.ctx.lock_manager.acquire(
            LockResource::Instance(instance_id.to_string()),
            "apply_loadout",
        )?;
        InstallService::new(self.ctx.clone()).check_not_locked(instance_id)?;
        crate::instance_runtime::check_idle(&self.ctx.paths, instance_id)?;
        loadout::apply_profile_with_undo(&instance_dir, profile_name).map_err(apply_error)
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
            .apply("demo", "both")
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
            .apply("demo", "both")
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
        service.apply("demo", "both").unwrap();
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
        service.apply("demo", "both").unwrap();
        assert!(dir.join("mods").join("lithium.jar").exists());
    }
}
