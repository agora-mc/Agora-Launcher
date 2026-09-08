//! Locked, context-aware entry points for snapshot creation and restore.
//!
//! [`crate::snapshot`] is a free-function module over an `instance_dir` with no
//! access to the lock manager, so it cannot serialize itself against anything.
//! Leaving that to the adapters meant the desktop command checked only its own
//! in-process launch state — a check it then released before starting the
//! blocking work — and the CLI checked nothing at all, so a concurrent
//! `agora snapshots restore` could run underneath an install.
//!
//! Core owns the invariant instead. Every adapter goes through here, and the
//! whole operation — recovery, validation, undo capture, replacement, and
//! retention — happens inside one critical section.

use crate::ctx::Ctx;
use crate::error::{LauncherError, LauncherResult};
use crate::lock_manager::LockResource;
use crate::snapshot::{self, RestoreOutcome, Snapshot};

pub struct SnapshotService {
    ctx: Ctx,
}

impl SnapshotService {
    pub fn new(ctx: Ctx) -> Self {
        Self { ctx }
    }

    /// Capture a full snapshot of an instance.
    pub fn create(&self, instance_id: &str, label: Option<&str>) -> LauncherResult<Snapshot> {
        let instance_dir = self.ctx.paths.instance_dir(instance_id)?;
        let _guard = self
            .ctx
            .lock_manager
            .acquire(LockResource::Instance(instance_id.to_string()), "snapshot")?;
        crate::instance_runtime::check_idle(&self.ctx.paths, instance_id)?;

        let snapshot = snapshot::create_snapshot(&instance_dir, label).map_err(snapshot_error)?;
        // Retention is housekeeping: a failure here must not present a
        // successful capture as a failure.
        if let Err(error) = crate::lkg::run_retention(&instance_dir) {
            eprintln!("[snapshot] retention after capture failed: {error}");
        }
        Ok(snapshot)
    }

    /// Restore an instance to a snapshot.
    ///
    /// Holds the instance lock across the whole operation, so the undo
    /// snapshot and the restore it protects cannot be separated by another
    /// mutation.
    pub fn restore(&self, instance_id: &str, snapshot_id: &str) -> LauncherResult<RestoreOutcome> {
        let instance_dir = self.ctx.paths.instance_dir(instance_id)?;
        let _guard = self.ctx.lock_manager.acquire(
            LockResource::Instance(instance_id.to_string()),
            "restore_snapshot",
        )?;
        crate::instance_runtime::check_idle(&self.ctx.paths, instance_id)?;

        // Validate the target before capturing anything: an invalid request
        // should not cost the user a snapshot.
        let scope = snapshot::restore_scope(&instance_dir, snapshot_id).map_err(snapshot_error)?;

        // The undo snapshot covers exactly what the restore will replace.
        // Matching the scope means undoing a configuration-only restore does
        // not copy gigabytes of world data it was never going to touch.
        let label = format!("pre-restore-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S"));
        let scope_refs: Vec<&str> = scope.iter().map(String::as_str).collect();
        snapshot::create_snapshot_scoped(&instance_dir, Some(&label), &scope_refs).map_err(
            |error| LauncherError::Generic {
                code: "ERR_SNAPSHOT".into(),
                message: format!("Could not create the undo snapshot: {error}"),
            },
        )?;

        let outcome =
            snapshot::restore_snapshot(&instance_dir, snapshot_id).map_err(snapshot_error)?;

        if let Err(error) = crate::lkg::run_retention(&instance_dir) {
            eprintln!("[snapshot] retention after restore failed: {error}");
        }
        Ok(outcome)
    }

    /// The roots a restore of this snapshot would cover.
    pub fn scope(&self, instance_id: &str, snapshot_id: &str) -> LauncherResult<Vec<String>> {
        let instance_dir = self.ctx.paths.instance_dir(instance_id)?;
        snapshot::restore_scope(&instance_dir, snapshot_id).map_err(snapshot_error)
    }
}

fn snapshot_error(message: String) -> LauncherError {
    LauncherError::Generic {
        code: "ERR_SNAPSHOT".into(),
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn service() -> (SnapshotService, Ctx, String) {
        let root = std::env::temp_dir().join(format!(
            "agora-snapshot-service-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let ctx = Ctx::for_testing(root);
        crate::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();
        let instance_id = "demo".to_string();
        let dir = ctx.paths.instance_dir(&instance_id).unwrap();
        fs::create_dir_all(dir.join("mods")).unwrap();
        fs::write(dir.join("mods").join("a.jar"), b"original").unwrap();
        fs::create_dir_all(dir.join("saves").join("world")).unwrap();
        fs::write(dir.join("saves").join("world").join("level.dat"), b"world").unwrap();
        (SnapshotService::new(ctx.clone()), ctx, instance_id)
    }

    #[test]
    fn restore_takes_an_undo_snapshot_scoped_to_what_it_replaces() {
        let (service, ctx, id) = service();
        let dir = ctx.paths.instance_dir(&id).unwrap();
        let scoped = snapshot::create_snapshot_scoped(
            &dir,
            Some("pre-launch"),
            snapshot::prelaunch_tracked_entries(),
        )
        .unwrap();

        fs::write(dir.join("mods").join("a.jar"), b"broken").unwrap();
        let outcome = service.restore(&id, &scoped.id).unwrap();

        assert_eq!(
            fs::read(dir.join("mods").join("a.jar")).unwrap(),
            b"original"
        );
        assert!(outcome.preserved_roots.iter().any(|r| r == "saves"));

        // The undo snapshot exists and covers the same roots, not the worlds.
        let undo = snapshot::list_snapshots(&dir)
            .unwrap()
            .into_iter()
            .find(|s| {
                s.label
                    .as_deref()
                    .is_some_and(|l| l.starts_with("pre-restore-"))
            })
            .expect("undo snapshot was not created");
        let undo_scope = service.scope(&id, &undo.id).unwrap();
        assert!(!undo_scope.iter().any(|r| r == "saves"));
    }

    /// A restore must not proceed while a game may be writing to the instance.
    #[test]
    fn restore_refuses_while_a_launch_is_unaccounted_for() {
        let (service, ctx, id) = service();
        let dir = ctx.paths.instance_dir(&id).unwrap();
        let snap = snapshot::create_snapshot(&dir, None).unwrap();
        fs::write(dir.join("mods").join("a.jar"), b"changed").unwrap();

        crate::instance_runtime::publish_starting(&ctx.paths, &id).unwrap();
        let error = service.restore(&id, &snap.id).unwrap_err();
        assert!(format!("{error:?}").contains("ERR_INSTANCE_LAUNCH_INDETERMINATE"));
        // Nothing was touched, including no undo snapshot.
        assert_eq!(
            fs::read(dir.join("mods").join("a.jar")).unwrap(),
            b"changed"
        );
        assert_eq!(snapshot::list_snapshots(&dir).unwrap().len(), 1);
    }

    /// An unknown snapshot must be rejected before an undo snapshot is taken.
    #[test]
    fn an_invalid_target_does_not_cost_a_snapshot() {
        let (service, ctx, id) = service();
        let dir = ctx.paths.instance_dir(&id).unwrap();
        assert!(service.restore(&id, "does-not-exist").is_err());
        assert!(snapshot::list_snapshots(&dir).unwrap().is_empty());
    }

    /// Restore must not run while another process holds the instance lock.
    #[test]
    fn restore_is_serialized_against_other_instance_mutations() {
        let (service, ctx, id) = service();
        let dir = ctx.paths.instance_dir(&id).unwrap();
        let snap = snapshot::create_snapshot(&dir, None).unwrap();

        let held = ctx
            .lock_manager
            .acquire(LockResource::Instance(id.clone()), "install")
            .unwrap();
        let error = ctx
            .lock_manager
            .acquire_with_timeout(
                LockResource::Instance(id.clone()),
                "restore_snapshot",
                std::time::Duration::from_millis(200),
                None,
            )
            .unwrap_err();
        assert!(format!("{error:?}").contains("Lock") || format!("{error:?}").contains("lock"));
        drop(held);

        // With the lock free it proceeds.
        service.restore(&id, &snap.id).unwrap();
    }
}
