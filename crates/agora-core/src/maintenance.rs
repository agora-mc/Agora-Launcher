//! Bounded background maintenance for launch-adjacent caches.
//!
//! Startup maintenance is intentionally conservative: it warms only the most
//! recently used instances, runs through the background scheduler lane, and
//! skips an instance if an interactive operation already owns its lock.  A
//! launch never depends on this work completing; the normal launch path still
//! validates its inputs and health state synchronously from its own worker.

use crate::ctx::Ctx;
use crate::lock_manager::LockResource;
use crate::task_scheduler::BlockingPriority;
use std::time::Duration;

/// Keep startup disk work bounded. The database already orders instances by
/// most recent launch, so this favors the instances most likely to be opened
/// next without scanning an entire library on every app start.
pub const STARTUP_PREWARM_LIMIT: usize = 2;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WarmupSummary {
    pub considered: usize,
    pub warmed: usize,
    pub skipped: usize,
    pub failed: usize,
}

/// Warm recent instance metadata and health caches on a bounded background
/// worker.  Errors are returned for scheduler/database failures; individual
/// stale or malformed instances are counted and do not prevent other entries
/// from being warmed.
pub async fn prewarm_recent_instances(ctx: Ctx) -> Result<WarmupSummary, String> {
    let scheduler = ctx.task_scheduler.clone();
    scheduler
        .run_blocking(BlockingPriority::Background, move || prewarm_blocking(&ctx))
        .await
        .map_err(|error| format!("startup maintenance worker failed: {error}"))?
}

fn prewarm_blocking(ctx: &Ctx) -> Result<WarmupSummary, String> {
    let conn = crate::db::local_state_connection(&ctx.paths.local_state_db())
        .map_err(|error| format!("could not open local state for startup maintenance: {error}"))?;
    let instances = crate::db::list_instances(&conn)
        .map_err(|error| format!("could not list instances for startup maintenance: {error}"))?;
    // Do not keep an idle SQLite handle around for the filesystem-heavy part
    // of maintenance. This also ensures startup warmup cannot extend a future
    // writer's lock lifetime through an accidental statement borrow.
    drop(conn);
    let registry_db = ctx
        .paths
        .registry_db()
        .is_file()
        .then(|| ctx.paths.registry_db());

    let mut summary = WarmupSummary::default();
    for row in instances.into_iter().take(STARTUP_PREWARM_LIMIT) {
        summary.considered += 1;
        let instance_dir = match ctx.paths.instance_dir(&row.instance_id) {
            Ok(path) => path,
            Err(_) => {
                summary.failed += 1;
                continue;
            }
        };

        // Do not wait behind a launch/install long enough to turn maintenance
        // into a hidden startup stall.  The interactive operation will warm
        // its own cache as part of the normal path.
        let lock = match ctx.lock_manager.acquire_with_timeout(
            LockResource::Instance(row.instance_id.clone()),
            "startup-maintenance",
            Duration::from_millis(25),
            None,
        ) {
            Ok(lock) => lock,
            Err(_) => {
                summary.skipped += 1;
                continue;
            }
        };
        // This is an admission probe, not a long-lived exclusive lock. Cache
        // entries are metadata-keyed and health publication verifies that the
        // state stayed stable across the scan, so a concurrent mutation only
        // turns this work into a cache miss. Releasing here prevents a launch
        // click from ever waiting behind background archive parsing.
        drop(lock);

        let manifest_path = instance_dir.join("instance_manifest.json");
        let manifest = match crate::helpers::read_manifest(&manifest_path).ok() {
            Some(manifest) => manifest,
            None => {
                summary.skipped += 1;
                continue;
            }
        };

        if crate::health::precompute_jar_metadata_cache(&instance_dir, &manifest).is_err() {
            summary.failed += 1;
            continue;
        }
        let _ =
            crate::health::cached_health(&instance_dir, &manifest, registry_db.as_deref(), None);
        summary.warmed += 1;
    }

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_limit_is_conservative() {
        assert_eq!(STARTUP_PREWARM_LIMIT, 2);
    }
}
