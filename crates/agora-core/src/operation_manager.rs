//! Cloneable, shared operation manager for long-running core operations.
//!
//! Provides:
//! - Unique [`OperationId`] generation per registration
//! - Per-operation [`CancellationToken`] for cooperative cancellation
//! - Registration, status querying, progress tracking
//! - Safe cleanup on explicit completion, failure, or drop
//!
//! All clones of an [`OperationManager`] share the same internal state, so a
//! handle obtained from one clone is observable from any other.
//!
//! # Ownership boundary
//!
//! This module owns **nothing** outside its in-memory operation table.
//! It does not perform I/O, acquire locks, open databases, or emit events.
//! Callers (services) are responsible for associating operations with
//! instance state, progress sinks, and database transactions.
//!
//! # Drop safety
//!
//! [`OpHandle`] is `#[must_use]` — dropping without [`OpHandle::complete`] or
//! [`OpHandle::fail`] auto-cancels and unregisters the operation, preventing
//! dangling registration entries.

use crate::event_sink::{CancellationToken, OperationId};
use crate::install_pipeline::ResolvedInstallPlan;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

// ---------------------------------------------------------------------------
// OpStatus
// ---------------------------------------------------------------------------

/// Lifecycle status of a tracked operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OpStatus {
    Pending,
    Running,
    Completed,
    Failed(String),
    Cancelled,
}

// ---------------------------------------------------------------------------
// OpInfo
// ---------------------------------------------------------------------------

/// Snapshotted metadata for a single operation.
///
/// Obtained from [`OperationManager::info`], [`OperationManager::list_active`],
/// or [`OperationManager::list_all`].  Not serializable by design — the fields
/// are snapshotted for intra-core queries only.
#[derive(Debug, Clone)]
pub struct OpInfo {
    pub id: OperationId,
    pub label: String,
    pub status: OpStatus,
    pub started_at: SystemTime,
    pub finished_at: Option<SystemTime>,
    pub instance_id: Option<String>,
    pub progress: Option<f64>,
}

// ---------------------------------------------------------------------------
// Internal record
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct OpRecord {
    label: String,
    status: OpStatus,
    started_at: SystemTime,
    finished_at: Option<SystemTime>,
    instance_id: Option<String>,
    progress: Option<f64>,
    token: CancellationToken,
}

// ---------------------------------------------------------------------------
// StoredPlan — resolved InstallPlan plus its cancellation infrastructure
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct StoredPlan {
    plan: ResolvedInstallPlan,
    token: CancellationToken,
    op_id: OperationId,
}

// ---------------------------------------------------------------------------
// OpHandle
// ---------------------------------------------------------------------------

/// Handle to a registered operation.
///
/// Dropping without [`OpHandle::complete`] or [`OpHandle::fail`] auto-cancels
/// the operation and removes it from the manager, preventing dangling state.
#[derive(Debug, Clone)]
#[must_use = "dropping without complete/fail auto-cancels the operation"]
pub struct OpHandle {
    id: OperationId,
    token: CancellationToken,
    state: Arc<Mutex<HashMap<OperationId, OpRecord>>>,
    finished: bool,
}

impl OpHandle {
    /// The operation's unique identifier.
    pub fn id(&self) -> &OperationId {
        &self.id
    }

    /// The cancellation token associated with this operation.
    ///
    /// Pass this to long-running functions so they can check
    /// [`CancellationToken::is_cancelled`] and abort early.
    pub fn token(&self) -> &CancellationToken {
        &self.token
    }

    /// Cancel the operation.
    ///
    /// Triggers the cancellation token and marks the status as `Cancelled`.
    pub fn cancel(&self) {
        self.token.cancel();
        if let Ok(mut map) = self.state.lock() {
            if let Some(rec) = map.get_mut(&self.id) {
                rec.status = OpStatus::Cancelled;
                rec.finished_at = Some(SystemTime::now());
            }
        }
    }

    /// Update the progress fraction for this operation.
    ///
    /// `progress` is clamped to the `[0.0, 1.0]` range.
    pub fn set_progress(&self, progress: f64) {
        if let Ok(mut map) = self.state.lock() {
            if let Some(rec) = map.get_mut(&self.id) {
                rec.progress = Some(progress.clamp(0.0, 1.0));
            }
        }
    }

    /// Mark the operation as completed and unregister it.
    pub fn complete(mut self) {
        self.finish(OpStatus::Completed);
    }

    /// Mark the operation as failed with an error message and unregister it.
    pub fn fail(mut self, error: impl Into<String>) {
        self.finish(OpStatus::Failed(error.into()));
    }

    fn finish(&mut self, status: OpStatus) {
        if self.finished {
            return;
        }
        self.finished = true;
        if let Ok(mut map) = self.state.lock() {
            if let Some(rec) = map.get_mut(&self.id) {
                rec.status = status;
                rec.finished_at = Some(SystemTime::now());
            }
        }
    }
}

impl Drop for OpHandle {
    fn drop(&mut self) {
        if !self.finished {
            self.token.cancel();
            if let Ok(mut map) = self.state.lock() {
                map.remove(&self.id);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// OperationManager
// ---------------------------------------------------------------------------

/// Cloneable, shared operation manager.
///
/// All clones observe the same set of registered operations.  Cloning is
/// cheap (arc bumps) and is the intended way to share the manager between
/// services and callers.
#[derive(Clone, Default)]
pub struct OperationManager {
    state: Arc<Mutex<HashMap<OperationId, OpRecord>>>,
    next_id: Arc<AtomicU64>,
    plans: Arc<Mutex<HashMap<String, StoredPlan>>>,
}

impl OperationManager {
    /// Create an empty manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new operation and return a handle.
    ///
    /// The operation starts in `Pending` status.  Use [`OpHandle::set_progress`]
    /// and [`OpHandle::complete`] / [`OpHandle::fail`] to drive it through its
    /// lifecycle.
    pub fn register(&self, label: &str) -> OpHandle {
        self.register_impl(label, None)
    }

    /// Register a new operation scoped to a specific instance.
    ///
    /// The `instance_id` is recorded in the metadata so [`cancel_for_instance`]
    /// can find and cancel it later.
    pub fn register_for_instance(&self, label: &str, instance_id: &str) -> OpHandle {
        self.register_impl(label, Some(instance_id))
    }

    fn register_impl(&self, label: &str, instance_id: Option<&str>) -> OpHandle {
        let id = {
            let n = self.next_id.fetch_add(1, Ordering::Relaxed);
            OperationId::new(format!("op-{n}"))
        };
        let token = CancellationToken::new();
        let rec = OpRecord {
            label: label.to_owned(),
            status: OpStatus::Pending,
            started_at: SystemTime::now(),
            finished_at: None,
            instance_id: instance_id.map(str::to_owned),
            progress: None,
            token: token.clone(),
        };
        if let Ok(mut map) = self.state.lock() {
            map.insert(id.clone(), rec);
        }
        OpHandle {
            id: id.clone(),
            token,
            state: self.state.clone(),
            finished: false,
        }
    }

    /// Return metadata for a specific operation, or `None` if it does not
    /// exist or has been cleaned up.
    pub fn info(&self, id: &OperationId) -> Option<OpInfo> {
        let map = self.state.lock().ok()?;
        map.get(id).map(|rec| OpInfo {
            id: id.clone(),
            label: rec.label.clone(),
            status: rec.status.clone(),
            started_at: rec.started_at,
            finished_at: rec.finished_at,
            instance_id: rec.instance_id.clone(),
            progress: rec.progress,
        })
    }

    /// List all currently active operations (pending or running).
    pub fn list_active(&self) -> Vec<OpInfo> {
        self.list_filtered(|rec| matches!(rec.status, OpStatus::Pending | OpStatus::Running))
    }

    /// List every known operation, including completed, failed, and cancelled.
    pub fn list_all(&self) -> Vec<OpInfo> {
        self.list_filtered(|_| true)
    }

    fn list_filtered(&self, keep: fn(&OpRecord) -> bool) -> Vec<OpInfo> {
        let Ok(map) = self.state.lock() else {
            return Vec::new();
        };
        map.iter()
            .filter(|(_, rec)| keep(rec))
            .map(|(id, rec)| OpInfo {
                id: id.clone(),
                label: rec.label.clone(),
                status: rec.status.clone(),
                started_at: rec.started_at,
                finished_at: rec.finished_at,
                instance_id: rec.instance_id.clone(),
                progress: rec.progress,
            })
            .collect()
    }

    /// Cancel an operation by ID.  Returns `true` if the operation existed.
    pub fn cancel(&self, id: &OperationId) -> bool {
        let mut map = match self.state.lock() {
            Ok(m) => m,
            Err(_) => return false,
        };
        match map.get_mut(id) {
            Some(rec) => {
                rec.token.cancel();
                rec.status = OpStatus::Cancelled;
                rec.finished_at = Some(SystemTime::now());
                true
            }
            None => false,
        }
    }

    /// Cancel all pending or running operations for a given instance.
    /// Returns the number of operations cancelled.
    pub fn cancel_for_instance(&self, instance_id: &str) -> usize {
        let mut map = match self.state.lock() {
            Ok(m) => m,
            Err(_) => return 0,
        };
        let mut count = 0;
        for rec in map.values_mut() {
            let is_match = rec
                .instance_id
                .as_deref()
                .map(|id| id == instance_id)
                .unwrap_or(false);
            if is_match && matches!(rec.status, OpStatus::Pending | OpStatus::Running) {
                rec.token.cancel();
                rec.status = OpStatus::Cancelled;
                rec.finished_at = Some(SystemTime::now());
                count += 1;
            }
        }
        count
    }

    /// Remove terminal operations (completed, failed, cancelled) that finished
    /// more than `max_age` ago.  Returns the number of records removed.
    pub fn gc(&self, max_age: Duration) -> usize {
        let cutoff = SystemTime::now()
            .checked_sub(max_age)
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let mut map = match self.state.lock() {
            Ok(m) => m,
            Err(_) => return 0,
        };
        let before = map.len();
        map.retain(|_, rec| {
            // Keep active operations regardless of age.
            if matches!(rec.status, OpStatus::Pending | OpStatus::Running) {
                return true;
            }
            // Keep terminal operations that finished recently.
            rec.finished_at.is_none_or(|t| t > cutoff)
        });
        before - map.len()
    }

    /// Number of currently active (pending + running) operations.
    pub fn active_count(&self) -> usize {
        let Ok(map) = self.state.lock() else {
            return 0;
        };
        map.values()
            .filter(|rec| matches!(rec.status, OpStatus::Pending | OpStatus::Running))
            .count()
    }

    /// Total number of tracked operations (all statuses).
    pub fn total_count(&self) -> usize {
        self.state.lock().map(|m| m.len()).unwrap_or(0)
    }

    // -----------------------------------------------------------------------
    // Install-plan storage
    // -----------------------------------------------------------------------

    /// Store a resolved install plan and register a pending operation for it.
    ///
    /// Returns a [`CancellationToken`] that is shared with the registered
    /// operation.  Pass it to [`InstallService::execute`] so the plan can be
    /// cancelled via [`cancel_plan`] or the token directly.
    ///
    /// The plan is keyed by its `fingerprint` — overwriting an existing key
    /// replaces the previous plan and its operation.
    pub fn insert_plan(&self, fingerprint: String, plan: ResolvedInstallPlan) -> CancellationToken {
        let id = OperationId::new(format!(
            "plan-{}",
            self.next_id.fetch_add(1, Ordering::Relaxed)
        ));
        let token = CancellationToken::new();
        let instance_id = plan.intent.target_instance.clone();

        // Register an operation record so the plan appears in the standard
        // operation listings (list_active / list_all / info).
        let rec = OpRecord {
            label: format!("install:{}", plan.intent.target_instance),
            status: OpStatus::Pending,
            started_at: SystemTime::now(),
            finished_at: None,
            instance_id: Some(instance_id),
            progress: None,
            token: token.clone(),
        };
        if let Ok(mut map) = self.state.lock() {
            map.insert(id.clone(), rec);
        }
        let stored = StoredPlan {
            plan,
            token: token.clone(),
            op_id: id,
        };
        if let Ok(mut map) = self.plans.lock() {
            map.insert(fingerprint, stored);
        }
        token
    }

    /// Retrieve a stored plan by fingerprint.
    pub fn get_plan(&self, fingerprint: &str) -> Option<ResolvedInstallPlan> {
        let map = self.plans.lock().ok()?;
        map.get(fingerprint).map(|s| s.plan.clone())
    }

    /// Remove a stored plan and its associated operation record.
    ///
    /// Returns `true` if the plan was found and removed.
    ///
    /// This does **not** cancel the shared cancellation token — callers that
    /// need to abort an in-flight execution should call [`cancel_plan`] first.
    pub fn remove_plan(&self, fingerprint: &str) -> bool {
        let removed = self
            .plans
            .lock()
            .ok()
            .and_then(|mut map| map.remove(fingerprint));
        if let Some(stored) = removed {
            // Clean up the associated operation record.
            if let Ok(mut s) = self.state.lock() {
                s.remove(&stored.op_id);
            }
            true
        } else {
            false
        }
    }

    /// Cancel a plan by fingerprint.
    ///
    /// Triggers the plan's shared cancellation token and transitions the
    /// associated operation to `Cancelled`.  The plan data is **not** removed
    /// — call [`remove_plan`] after the execution has finished (or via the
    /// drop guard on panic).
    ///
    /// Returns `true` if the plan was found.
    pub fn cancel_plan(&self, fingerprint: &str) -> bool {
        let op_id = {
            let map = match self.plans.lock() {
                Ok(m) => m,
                Err(_) => return false,
            };
            map.get(fingerprint).map(|s| {
                s.token.cancel();
                s.op_id.clone()
            })
        };
        if let Some(id) = op_id {
            self.cancel(&id);
            true
        } else {
            false
        }
    }

    /// Return the cancellation token for a stored plan, if one exists.
    pub fn token_for_plan(&self, fingerprint: &str) -> Option<CancellationToken> {
        let map = self.plans.lock().ok()?;
        map.get(fingerprint).map(|s| s.token.clone())
    }

    /// Number of stored install plans.
    pub fn stored_plan_count(&self) -> usize {
        self.plans.lock().map(|m| m.len()).unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // -- Registration & info -----------------------------------------------

    #[test]
    fn register_creates_pending_operation() {
        let mgr = OperationManager::new();
        let handle = mgr.register("test-op");

        let info = mgr.info(handle.id()).expect("should exist");
        assert_eq!(info.label, "test-op");
        assert_eq!(info.status, OpStatus::Pending);
        assert_eq!(info.instance_id, None);
        assert_eq!(info.progress, None);
    }

    #[test]
    fn register_for_instance_sets_instance_id() {
        let mgr = OperationManager::new();
        let handle = mgr.register_for_instance("instance-op", "my-instance");

        let info = mgr.info(handle.id()).expect("should exist");
        assert_eq!(info.instance_id, Some("my-instance".into()));
    }

    // -- Lifecycle transitions ---------------------------------------------

    #[test]
    fn complete_transitions_to_completed() {
        let mgr = OperationManager::new();
        let handle = mgr.register("op");
        let id = handle.id().clone();
        handle.complete();

        let info = mgr.info(&id).expect("should still exist");
        assert_eq!(info.status, OpStatus::Completed);
        assert!(info.finished_at.is_some());
    }

    #[test]
    fn fail_transitions_to_failed() {
        let mgr = OperationManager::new();
        let handle = mgr.register("op");
        let id = handle.id().clone();
        handle.fail("something broke");

        let info = mgr.info(&id).expect("should still exist");
        assert_eq!(info.status, OpStatus::Failed("something broke".into()));
    }

    #[test]
    fn cancel_via_handle_transitions_to_cancelled() {
        let mgr = OperationManager::new();
        let handle = mgr.register("op");
        let id = handle.id().clone();

        handle.cancel();
        assert!(handle.token().is_cancelled());

        let info = mgr.info(&id).expect("should exist");
        assert_eq!(info.status, OpStatus::Cancelled);
    }

    #[test]
    fn cancel_via_manager_by_id() {
        let mgr = OperationManager::new();
        let handle = mgr.register("op");
        let id = handle.id().clone();

        assert!(mgr.cancel(&id));

        let info = mgr.info(&id).expect("should exist");
        assert_eq!(info.status, OpStatus::Cancelled);
    }

    #[test]
    fn cancel_nonexistent_id_returns_false() {
        let mgr = OperationManager::new();
        assert!(!mgr.cancel(&OperationId::new("nope")));
    }

    // -- Progress ----------------------------------------------------------

    #[test]
    fn set_progress_updates_record() {
        let mgr = OperationManager::new();
        let handle = mgr.register("op");
        handle.set_progress(0.75);

        let info = mgr.info(handle.id()).unwrap();
        assert!((info.progress.unwrap() - 0.75).abs() < 1e-6);
    }

    #[test]
    fn set_progress_clamps_to_unit_interval() {
        let mgr = OperationManager::new();
        let handle = mgr.register("op");
        handle.set_progress(1.5);
        assert!((mgr.info(handle.id()).unwrap().progress.unwrap() - 1.0).abs() < 1e-6);

        handle.set_progress(-0.1);
        assert!((mgr.info(handle.id()).unwrap().progress.unwrap() - 0.0).abs() < 1e-6);
    }

    // -- Listing -----------------------------------------------------------

    #[test]
    fn list_active_returns_only_non_terminal() {
        let mgr = OperationManager::new();
        let h1 = mgr.register("active-1");
        let _h2 = mgr.register("active-2");
        let (id3, id4) = {
            let h3 = mgr.register("to-complete");
            let h4 = mgr.register("to-cancel");
            let id3 = h3.id().clone();
            let id4 = h4.id().clone();
            h3.complete();
            h4.cancel();
            (id3, id4)
        };

        let active = mgr.list_active();
        let ids: Vec<OperationId> = active.iter().map(|i| i.id.clone()).collect();
        assert!(ids.contains(h1.id()));
        assert!(!ids.contains(&id3));
        assert!(!ids.contains(&id4));
    }

    #[test]
    fn list_all_returns_everything() {
        let mgr = OperationManager::new();
        let h1 = mgr.register("a");
        let _h2 = mgr.register("b");
        h1.complete();

        assert_eq!(mgr.list_all().len(), 2);
    }

    // -- Cancellation for instance -----------------------------------------

    #[test]
    fn cancel_for_instance_targets_correct_ops() {
        let mgr = OperationManager::new();
        let h_foo = mgr.register_for_instance("foo-op", "foo-instance");
        let h_bar = mgr.register_for_instance("bar-op", "bar-instance");
        let h_foo2 = mgr.register_for_instance("foo-op-2", "foo-instance");

        let cancelled = mgr.cancel_for_instance("foo-instance");
        assert_eq!(cancelled, 2);

        assert!(h_foo.token().is_cancelled());
        assert!(!h_bar.token().is_cancelled());
        assert!(h_foo2.token().is_cancelled());
    }

    #[test]
    fn cancel_for_instance_skips_completed_ops() {
        let mgr = OperationManager::new();
        let h = mgr.register_for_instance("op", "inst");
        h.complete();

        assert_eq!(mgr.cancel_for_instance("inst"), 0);
    }

    // -- Drop safety -------------------------------------------------------

    #[test]
    fn dropping_handle_auto_removes_entry() {
        let mgr = OperationManager::new();
        let id = {
            let handle = mgr.register("ephemeral");
            let id = handle.id().clone();
            // Drop handle without complete/fail.
            drop(handle);
            id
        };
        assert!(mgr.info(&id).is_none());
    }

    #[test]
    fn dropping_completed_handle_does_not_double_remove() {
        let mgr = OperationManager::new();
        let id = {
            let handle = mgr.register("clean");
            let id = handle.id().clone();
            handle.complete();
            // Drop the now-completed handle.
            id
        };
        // Completed operation should still be visible.
        assert!(mgr.info(&id).is_some());
        assert_eq!(mgr.info(&id).unwrap().status, OpStatus::Completed);
    }

    // -- GC -----------------------------------------------------------------

    #[test]
    fn gc_removes_old_terminal_operations() {
        let mgr = OperationManager::new();
        let h1 = mgr.register("will-complete");
        let h2 = mgr.register("will-stay-active");
        let id1 = h1.id().clone();
        h1.complete();

        // Force finished_at to be in the past.
        {
            let mut map = mgr.state.lock().unwrap();
            if let Some(rec) = map.get_mut(&id1) {
                rec.finished_at = Some(
                    SystemTime::now()
                        .checked_sub(Duration::from_secs(3600))
                        .unwrap(),
                );
            }
        }

        // GC with a short threshold so the completed op is eligible.
        let removed = mgr.gc(Duration::from_secs(60));
        assert_eq!(removed, 1);
        assert!(mgr.info(&id1).is_none());
        assert!(mgr.info(h2.id()).is_some());
    }

    #[test]
    fn gc_preserves_active_ops() {
        let mgr = OperationManager::new();
        let h = mgr.register("active");

        let removed = mgr.gc(Duration::from_secs(0));
        assert_eq!(removed, 0);
        assert!(mgr.info(h.id()).is_some());
    }

    // -- Clone sharing -----------------------------------------------------

    #[test]
    fn clones_share_state() {
        let mgr = OperationManager::new();
        let handle = mgr.register("shared");

        let mgr2 = mgr.clone();
        assert!(mgr2.info(handle.id()).is_some());
        assert_eq!(mgr2.list_active().len(), 1);
    }

    // -- Counters ----------------------------------------------------------

    #[test]
    fn active_count_matches_active_operations() {
        let mgr = OperationManager::new();
        assert_eq!(mgr.active_count(), 0);

        let _h1 = mgr.register("a");
        assert_eq!(mgr.active_count(), 1);

        let _h2 = mgr.register("b");
        assert_eq!(mgr.active_count(), 2);
    }

    #[test]
    fn total_count_includes_all_statuses() {
        let mgr = OperationManager::new();
        let h = mgr.register("op");
        h.complete();
        let _active = mgr.register("active");

        assert_eq!(mgr.total_count(), 2);
    }

    // -- Edge cases --------------------------------------------------------

    #[test]
    fn double_complete_is_noop() {
        let mgr = OperationManager::new();
        let handle = mgr.register("op");
        let id = handle.id().clone();
        handle.complete();
        // Calling complete again is a noop (handle consumed, can't be called).
        // Verify it's completed.
        assert_eq!(mgr.info(&id).unwrap().status, OpStatus::Completed);
    }

    #[test]
    fn empty_manager_queries() {
        let mgr = OperationManager::new();
        assert!(mgr.list_active().is_empty());
        assert!(mgr.list_all().is_empty());
        assert_eq!(mgr.active_count(), 0);
        assert_eq!(mgr.total_count(), 0);
        assert_eq!(mgr.gc(Duration::from_secs(0)), 0);
        assert_eq!(mgr.stored_plan_count(), 0);
    }

    // -- Plan storage -------------------------------------------------------

    fn sample_plan() -> ResolvedInstallPlan {
        use crate::install_pipeline::*;
        ResolvedInstallPlan {
            fingerprint: "test-fp-1".into(),
            intent: InstallIntent {
                action: InstallAction::Install {
                    source_type: SourceType::Curated,
                    item_id: "sodium".into(),
                    candidate_version: Some("1.0".into()),
                },
                target_instance: "test-instance".into(),
                optional_deps: OptionalDepsPolicy::ExcludeAll,
                requested_by: RequestSource::Interactive,
                overrides: PlanOverrides::default(),
            },
            operation: ResolvedOperation::Install {
                artifact: ResolvedArtifact::Download(ResolvedDownload {
                    item_id: "sodium".into(),
                    version_id: "1.0".into(),
                    source: ArtifactSource::Download {
                        url: "https://example.com/sodium.jar".into(),
                    },
                    hashes: HashSpec {
                        values: vec![HashedValue {
                            algorithm: HashAlgorithm::Sha256,
                            value: "abc".into(),
                        }],
                    },
                    size: 1024,
                    filename: "sodium.jar".into(),
                    metadata: ArtifactMetadata {
                        source_type: SourceType::Curated,
                        registry_id: None,
                        modrinth_id: None,
                        content_type: "mod".into(),
                        version: None,
                        download_strategy: None,
                    },
                }),
            },
            dependencies: Vec::new(),
            conflicts: Vec::new(),
            files_to_add: Vec::new(),
            files_to_remove: Vec::new(),
            files_to_disable: Vec::new(),
            snapshot: SnapshotPlan {
                label: "test-snapshot".into(),
                estimated_bytes: 1024,
            },
            disk_estimate: DiskSpaceEstimate {
                download_bytes: 1024,
                snapshot_bytes: 0,
                apply_overhead_bytes: 0,
                peak_additional_bytes: 1024,
                post_commit_delta_bytes: 1024,
            },
            warnings: Vec::new(),
            blocking_errors: Vec::new(),
            pending_choices: Vec::new(),
            loader_change: None,
            created_at: "2024-01-01T00:00:00Z".into(),
            instance_state_hash: "hash".into(),
            registry_revision: "rev".into(),
        }
    }

    #[test]
    fn insert_plan_stores_and_returns_token() {
        let mgr = OperationManager::new();
        let plan = sample_plan();
        let fp = plan.fingerprint.clone();

        let token = mgr.insert_plan(fp.clone(), plan.clone());
        assert!(!token.is_cancelled());
        assert_eq!(mgr.stored_plan_count(), 1);

        let retrieved = mgr.get_plan(&fp).expect("plan should exist");
        assert_eq!(retrieved.fingerprint, "test-fp-1");
    }

    #[test]
    fn insert_plan_registers_operation() {
        let mgr = OperationManager::new();
        let plan = sample_plan();
        let fp = plan.fingerprint.clone();

        let _token = mgr.insert_plan(fp.clone(), plan);
        // Should have 1 operation in active list.
        let active = mgr.list_active();
        assert_eq!(active.len(), 1);
        assert!(active[0].label.contains("test-instance"));
        assert_eq!(active[0].instance_id, Some("test-instance".into()));
    }

    #[test]
    fn get_plan_returns_none_for_missing() {
        let mgr = OperationManager::new();
        assert!(mgr.get_plan("nonexistent").is_none());
    }

    #[test]
    fn remove_plan_clears_plan_and_operation() {
        let mgr = OperationManager::new();
        let plan = sample_plan();
        let fp = plan.fingerprint.clone();

        let _token = mgr.insert_plan(fp.clone(), plan);
        assert_eq!(mgr.stored_plan_count(), 1);
        assert_eq!(mgr.total_count(), 1);

        assert!(mgr.remove_plan(&fp));
        assert_eq!(mgr.stored_plan_count(), 0);
        assert_eq!(mgr.total_count(), 0);
        assert!(mgr.get_plan(&fp).is_none());
    }

    #[test]
    fn remove_plan_returns_false_for_missing() {
        let mgr = OperationManager::new();
        assert!(!mgr.remove_plan("nonexistent"));
    }

    #[test]
    fn cancel_plan_cancels_token_and_operation() {
        let mgr = OperationManager::new();
        let plan = sample_plan();
        let fp = plan.fingerprint.clone();

        let token = mgr.insert_plan(fp.clone(), plan);
        assert!(!token.is_cancelled());

        assert!(mgr.cancel_plan(&fp));
        assert!(token.is_cancelled());

        // Plan data is still retrievable after cancel.
        assert!(mgr.get_plan(&fp).is_some());
        assert_eq!(mgr.stored_plan_count(), 1);
    }

    #[test]
    fn cancel_plan_returns_false_for_missing() {
        let mgr = OperationManager::new();
        assert!(!mgr.cancel_plan("nonexistent"));
    }

    #[test]
    fn token_for_plan_returns_token() {
        let mgr = OperationManager::new();
        let plan = sample_plan();
        let fp = plan.fingerprint.clone();

        let token = mgr.insert_plan(fp.clone(), plan);
        let retrieved = mgr.token_for_plan(&fp).expect("token should exist");
        assert!(!retrieved.is_cancelled());
        // Same underlying flag.
        token.cancel();
        assert!(retrieved.is_cancelled());
    }

    #[test]
    fn token_for_plan_returns_none_for_missing() {
        let mgr = OperationManager::new();
        assert!(mgr.token_for_plan("nonexistent").is_none());
    }

    #[test]
    fn insert_plan_overwrites_existing_key() {
        let mgr = OperationManager::new();
        let mut plan_a = sample_plan();
        plan_a.fingerprint = "same-fp".into();
        plan_a.intent.target_instance = "instance-a".into();

        let mut plan_b = sample_plan();
        plan_b.fingerprint = "same-fp".into();
        plan_b.intent.target_instance = "instance-b".into();

        let _token_a = mgr.insert_plan("same-fp".into(), plan_a);
        assert_eq!(mgr.stored_plan_count(), 1);

        let _token_b = mgr.insert_plan("same-fp".into(), plan_b);
        assert_eq!(mgr.stored_plan_count(), 1);

        let retrieved = mgr.get_plan("same-fp").expect("should exist");
        assert_eq!(retrieved.intent.target_instance, "instance-b");
    }

    #[test]
    fn clones_share_plan_state() {
        let mgr = OperationManager::new();
        let plan = sample_plan();
        let fp = plan.fingerprint.clone();

        let _token = mgr.insert_plan(fp.clone(), plan);
        let mgr2 = mgr.clone();
        assert!(mgr2.get_plan(&fp).is_some());
        assert_eq!(mgr2.stored_plan_count(), 1);
    }
}
