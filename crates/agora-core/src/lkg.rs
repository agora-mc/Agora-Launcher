//! Last-known-good (LKG) and launch classification.
//!
//! Pure logic — no filesystem or process dependencies.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;

// ---------------------------------------------------------------------------
// Launch outcome
// ---------------------------------------------------------------------------

/// Result of classifying a single launch session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LaunchOutcome {
    /// Exit code 0, runtime >= threshold, no crash report or log signature.
    Success,
    /// Non-zero exit, crash report found, or signal termination.
    Crash,
    /// User initiated stop via the app (not a game crash).
    Cancelled,
    /// Process vanished without notification — conservative non-promotion.
    Unknown,
    /// Exit code 0 but runtime < threshold (opened and closed instantly).
    Abandoned,
}

// ---------------------------------------------------------------------------
// LKG state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LkgState {
    pub current_lkg_snapshot_id: Option<String>,
    pub last_promoted_at: Option<String>,
    pub last_launch_session_id: Option<String>,
    pub last_launch_outcome: Option<LaunchOutcome>,
    /// Newest-first promotion history used by retention. Legacy markers omit
    /// this field and deserialize to an empty list.
    #[serde(default)]
    pub promoted_snapshot_ids: Vec<String>,
    pub schema_version: u32,
}

impl Default for LkgState {
    fn default() -> Self {
        Self {
            current_lkg_snapshot_id: None,
            last_promoted_at: None,
            last_launch_session_id: None,
            last_launch_outcome: None,
            promoted_snapshot_ids: Vec::new(),
            schema_version: 1,
        }
    }
}

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

/// Input to the classification function.
#[derive(Debug, Clone)]
pub struct LaunchEvents {
    pub exit_code: Option<i32>,
    pub runtime_ms: u64,
    pub was_user_cancelled: bool,
    pub crash_report_found: bool,
    pub log_crash_signature_matched: bool,
}

/// Threshold below which a clean exit is considered abandoned (in ms).
pub const DEFAULT_ABANDON_THRESHOLD_MS: u64 = 60_000; // 60 seconds

/// Classify a launch session based on observed events.
///
/// Pure function — deterministic for identical inputs.
pub fn classify_launch(events: &LaunchEvents) -> LaunchOutcome {
    if events.was_user_cancelled {
        return LaunchOutcome::Cancelled;
    }

    match events.exit_code {
        Some(0) => {
            if events.runtime_ms < DEFAULT_ABANDON_THRESHOLD_MS {
                LaunchOutcome::Abandoned
            } else if events.crash_report_found || events.log_crash_signature_matched {
                LaunchOutcome::Crash
            } else {
                LaunchOutcome::Success
            }
        }
        Some(_) => LaunchOutcome::Crash,
        None => {
            if events.crash_report_found || events.log_crash_signature_matched {
                LaunchOutcome::Crash
            } else {
                LaunchOutcome::Unknown
            }
        }
    }
}

/// Whether an outcome promotes a snapshot to LKG.
pub fn promotes_to_lkg(outcome: &LaunchOutcome) -> bool {
    matches!(outcome, LaunchOutcome::Success)
}

/// Read an instance's LKG pointer. Missing files produce the default state;
/// malformed state is surfaced so callers never overwrite recovery metadata
/// they could not understand.
pub fn read_lkg_state(instance_dir: &Path) -> Result<LkgState, String> {
    let path = instance_dir.join("lkg.json");
    if !path.is_file() {
        return Ok(LkgState::default());
    }
    let text =
        std::fs::read_to_string(&path).map_err(|e| format!("failed to read LKG state: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("failed to parse LKG state: {e}"))
}

/// Atomically record a classified launch and promote the exact pre-launch
/// snapshot only for a genuine success.
pub fn record_launch_outcome(
    instance_dir: &Path,
    pre_launch_snapshot_id: Option<&str>,
    launch_session_id: &str,
    outcome: LaunchOutcome,
) -> Result<LkgState, String> {
    if promotes_to_lkg(&outcome) && pre_launch_snapshot_id.is_none() {
        return Err("successful launch cannot be promoted without a pre-launch snapshot id".into());
    }
    if let (LaunchOutcome::Success, Some(snapshot_id)) = (&outcome, pre_launch_snapshot_id) {
        crate::snapshot::snapshot_file_index(instance_dir, snapshot_id).map_err(|error| {
            format!("successful launch snapshot {snapshot_id} is unavailable or invalid: {error}")
        })?;
    }

    let mut state = read_lkg_state(instance_dir)?;
    state.last_launch_session_id = Some(launch_session_id.to_string());
    state.last_launch_outcome = Some(outcome.clone());
    if let (LaunchOutcome::Success, Some(snapshot_id)) = (&outcome, pre_launch_snapshot_id) {
        state.current_lkg_snapshot_id = Some(snapshot_id.to_string());
        state.last_promoted_at = Some(chrono::Utc::now().to_rfc3339());
        state
            .promoted_snapshot_ids
            .retain(|existing| existing != snapshot_id);
        state
            .promoted_snapshot_ids
            .insert(0, snapshot_id.to_string());
    }

    std::fs::create_dir_all(instance_dir)
        .map_err(|e| format!("failed to ensure instance directory: {e}"))?;
    let path = instance_dir.join("lkg.json");
    let temp = instance_dir.join("lkg.json.tmp");
    let bytes = serde_json::to_vec_pretty(&state)
        .map_err(|e| format!("failed to serialize LKG state: {e}"))?;
    let mut file = std::fs::File::create(&temp)
        .map_err(|e| format!("failed to create temporary LKG state: {e}"))?;
    file.write_all(&bytes)
        .map_err(|e| format!("failed to write temporary LKG state: {e}"))?;
    file.sync_all()
        .map_err(|e| format!("failed to sync temporary LKG state: {e}"))?;
    std::fs::rename(&temp, &path).map_err(|e| format!("failed to commit LKG state: {e}"))?;

    let audit = serde_json::json!({
        "recordedAt": chrono::Utc::now().to_rfc3339(),
        "launchSessionId": launch_session_id,
        "preLaunchSnapshotId": pre_launch_snapshot_id,
        "outcome": outcome,
    });
    let mut launches = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(instance_dir.join("launches.jsonl"))
        .map_err(|e| format!("failed to open launch audit: {e}"))?;
    writeln!(launches, "{}", audit).map_err(|e| format!("failed to append launch audit: {e}"))?;
    launches
        .sync_all()
        .map_err(|e| format!("failed to sync launch audit: {e}"))?;

    Ok(state)
}

// ---------------------------------------------------------------------------
// Diff
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffEntry {
    pub path: String,
    pub old_sha256: Option<String>,
    pub new_sha256: Option<String>,
    pub old_size: Option<u64>,
    pub new_size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Diff {
    pub from_snapshot_id: Option<String>,
    pub to_snapshot_id: Option<String>,
    pub added: Vec<DiffEntry>,
    pub removed: Vec<DiffEntry>,
    pub modified: Vec<DiffEntry>,
    pub unchanged_count: usize,
    pub total_files_a: usize,
    pub total_files_b: usize,
}

/// A simplified file index for diff computation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub path: String,
    pub sha256: String,
    pub size: u64,
}

/// Compute a diff between two file indexes.
pub fn compute_diff(
    from: &[FileEntry],
    to: &[FileEntry],
    from_id: Option<String>,
    to_id: Option<String>,
) -> Diff {
    use std::collections::HashMap;

    let from_map: HashMap<&str, &FileEntry> = from.iter().map(|f| (f.path.as_str(), f)).collect();
    let to_map: HashMap<&str, &FileEntry> = to.iter().map(|f| (f.path.as_str(), f)).collect();

    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut modified = Vec::new();
    let mut unchanged_count = 0;

    for (path, from_entry) in &from_map {
        match to_map.get(path) {
            None => {
                removed.push(DiffEntry {
                    path: path.to_string(),
                    old_sha256: Some(from_entry.sha256.clone()),
                    new_sha256: None,
                    old_size: Some(from_entry.size),
                    new_size: None,
                });
            }
            Some(to_entry) if from_entry.sha256 != to_entry.sha256 => {
                modified.push(DiffEntry {
                    path: path.to_string(),
                    old_sha256: Some(from_entry.sha256.clone()),
                    new_sha256: Some(to_entry.sha256.clone()),
                    old_size: Some(from_entry.size),
                    new_size: Some(to_entry.size),
                });
            }
            Some(_) => {
                unchanged_count += 1;
            }
        }
    }

    for (path, to_entry) in &to_map {
        if !from_map.contains_key(path) {
            added.push(DiffEntry {
                path: path.to_string(),
                old_sha256: None,
                new_sha256: Some(to_entry.sha256.clone()),
                old_size: None,
                new_size: Some(to_entry.size),
            });
        }
    }

    Diff {
        from_snapshot_id: from_id,
        to_snapshot_id: to_id,
        added,
        removed,
        modified,
        unchanged_count,
        total_files_a: from.len(),
        total_files_b: to.len(),
    }
}

// ---------------------------------------------------------------------------
// Retention
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetentionPolicy {
    /// Number of LKG snapshots to keep (including current).
    pub keep_lkg_count: u32,
    /// Number of non-LKG snapshots to keep (most recent).
    pub keep_non_lkg_count: u32,
    /// Number of pre-restore snapshots to keep.
    pub keep_pre_restore_count: u32,
    /// Maximum total snapshot storage per instance in bytes.
    pub size_cap_bytes: u64,
}

/// Snapshot metadata needed to enforce both category counts and the storage
/// cap. Entries must be supplied newest-first.
#[derive(Debug, Clone)]
pub struct RetentionEntry {
    pub id: String,
    pub size_bytes: u64,
    pub is_lkg: bool,
    pub is_current_lkg: bool,
    pub is_pre_restore: bool,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            keep_lkg_count: 3,
            keep_non_lkg_count: 1,
            keep_pre_restore_count: 1,
            size_cap_bytes: 2_000_000_000, // 2 GB
        }
    }
}

/// Compute which snapshot ids should be evicted under a given policy.
pub fn retention_plan(
    snapshot_ids: &[String],
    lkg_ids: &[String],
    pre_restore_ids: &[String],
    policy: &RetentionPolicy,
) -> Vec<String> {
    let current = lkg_ids.first();
    let entries: Vec<RetentionEntry> = snapshot_ids
        .iter()
        .rev() // the legacy API historically receives oldest-first ids
        .map(|id| RetentionEntry {
            id: id.clone(),
            size_bytes: 0,
            is_lkg: lkg_ids.contains(id),
            is_current_lkg: current == Some(id),
            is_pre_restore: pre_restore_ids.contains(id),
        })
        .collect();
    retention_plan_with_sizes(&entries, policy)
}

/// Compute snapshot eviction with configurable category counts and a hard
/// size cap. The current LKG is never removed merely to satisfy the cap; if it
/// alone exceeds the cap it remains available and all other snapshots are
/// evicted.
pub fn retention_plan_with_sizes(
    entries_newest_first: &[RetentionEntry],
    policy: &RetentionPolicy,
) -> Vec<String> {
    use std::collections::HashSet;

    let mut keep = HashSet::new();
    if let Some(current) = entries_newest_first
        .iter()
        .find(|entry| entry.is_current_lkg)
    {
        keep.insert(current.id.clone());
    }

    let mut kept_lkg = usize::from(!keep.is_empty());
    let mut kept_non_lkg = 0usize;
    let mut kept_pre_restore = 0usize;
    for entry in entries_newest_first {
        if entry.is_current_lkg {
            continue;
        }
        if entry.is_lkg {
            if kept_lkg < policy.keep_lkg_count as usize {
                keep.insert(entry.id.clone());
                kept_lkg += 1;
            }
        } else if entry.is_pre_restore {
            if kept_pre_restore < policy.keep_pre_restore_count as usize {
                keep.insert(entry.id.clone());
                kept_pre_restore += 1;
            }
        } else if kept_non_lkg < policy.keep_non_lkg_count as usize {
            keep.insert(entry.id.clone());
            kept_non_lkg += 1;
        }
    }

    let mut retained_size: u64 = entries_newest_first
        .iter()
        .filter(|entry| keep.contains(&entry.id))
        .map(|entry| entry.size_bytes)
        .sum();
    if retained_size > policy.size_cap_bytes {
        let mut candidates: Vec<&RetentionEntry> = entries_newest_first
            .iter()
            .rev() // oldest first within each safety tier
            .filter(|entry| keep.contains(&entry.id) && !entry.is_current_lkg)
            .collect();
        candidates.sort_by_key(|entry| {
            if !entry.is_lkg && !entry.is_pre_restore {
                0u8
            } else if entry.is_pre_restore {
                1
            } else {
                2
            }
        });
        for entry in candidates {
            if retained_size <= policy.size_cap_bytes {
                break;
            }
            if keep.remove(&entry.id) {
                retained_size = retained_size.saturating_sub(entry.size_bytes);
            }
        }
    }

    entries_newest_first
        .iter()
        .filter(|entry| !keep.contains(&entry.id))
        .map(|entry| entry.id.clone())
        .collect()
}

// ---------------------------------------------------------------------------
// Retention (moved from desktop commands.rs — core-owned)
// ---------------------------------------------------------------------------

/// Run snapshot retention for an instance directory using default policy.
/// Lists snapshots, cross-references LKG state, evicts those exceeding the
/// configured category counts and size cap.
pub fn run_retention(instance_dir: &Path) -> Result<(), String> {
    let snapshots = crate::snapshot::list_snapshots(instance_dir)
        .map_err(|e| format!("list_snapshots: {e}"))?;
    if snapshots.is_empty() {
        return Ok(());
    }

    let lkg = read_lkg_state(instance_dir)?;
    let entries: Vec<RetentionEntry> = snapshots
        .iter()
        .map(|snapshot| {
            let measured = crate::snapshot::snapshot_storage_size(instance_dir, &snapshot.id);
            let archive_size = if measured == 0 {
                snapshot.size_estimate
            } else {
                measured
            };
            RetentionEntry {
                id: snapshot.id.clone(),
                size_bytes: archive_size,
                is_lkg: lkg.promoted_snapshot_ids.contains(&snapshot.id)
                    || lkg.current_lkg_snapshot_id.as_ref() == Some(&snapshot.id),
                is_current_lkg: lkg.current_lkg_snapshot_id.as_ref() == Some(&snapshot.id),
                is_pre_restore: snapshot
                    .label
                    .as_deref()
                    .is_some_and(|label| label.starts_with("pre-restore-")),
            }
        })
        .collect();
    let policy = RetentionPolicy::default();
    let to_evict = retention_plan_with_sizes(&entries, &policy);

    let mut errors = Vec::new();
    for id in &to_evict {
        if let Err(error) = crate::snapshot::delete_snapshot(instance_dir, id) {
            errors.push(format!("{id}: {error}"));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "snapshot retention could not remove: {}",
            errors.join("; ")
        ))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_snapshot(instance_dir: &Path, label: &str) -> String {
        std::fs::write(instance_dir.join("instance_manifest.json"), b"{}\n").unwrap();
        crate::snapshot::create_snapshot(instance_dir, Some(label))
            .unwrap()
            .id
    }

    #[test]
    fn test_classify_success() {
        assert_eq!(
            classify_launch(&LaunchEvents {
                exit_code: Some(0),
                runtime_ms: 120_000,
                was_user_cancelled: false,
                crash_report_found: false,
                log_crash_signature_matched: false,
            }),
            LaunchOutcome::Success,
        );
    }

    #[test]
    fn test_classify_crash_nonzero_exit() {
        assert_eq!(
            classify_launch(&LaunchEvents {
                exit_code: Some(1),
                runtime_ms: 300_000,
                was_user_cancelled: false,
                crash_report_found: false,
                log_crash_signature_matched: false,
            }),
            LaunchOutcome::Crash,
        );
    }

    #[test]
    fn test_classify_crash_report() {
        assert_eq!(
            classify_launch(&LaunchEvents {
                exit_code: Some(0),
                runtime_ms: 300_000,
                was_user_cancelled: false,
                crash_report_found: true,
                log_crash_signature_matched: false,
            }),
            LaunchOutcome::Crash,
        );
    }

    #[test]
    fn test_classify_abandoned() {
        assert_eq!(
            classify_launch(&LaunchEvents {
                exit_code: Some(0),
                runtime_ms: 5_000,
                was_user_cancelled: false,
                crash_report_found: false,
                log_crash_signature_matched: false,
            }),
            LaunchOutcome::Abandoned,
        );
    }

    #[test]
    fn test_classify_cancelled() {
        assert_eq!(
            classify_launch(&LaunchEvents {
                exit_code: None,
                runtime_ms: 60_000,
                was_user_cancelled: true,
                crash_report_found: false,
                log_crash_signature_matched: false,
            }),
            LaunchOutcome::Cancelled,
        );
    }

    #[test]
    fn test_classify_unknown() {
        assert_eq!(
            classify_launch(&LaunchEvents {
                exit_code: None,
                runtime_ms: 120_000,
                was_user_cancelled: false,
                crash_report_found: false,
                log_crash_signature_matched: false,
            }),
            LaunchOutcome::Unknown,
        );
    }

    #[test]
    fn test_promotes_to_lkg() {
        assert!(promotes_to_lkg(&LaunchOutcome::Success));
        assert!(!promotes_to_lkg(&LaunchOutcome::Crash));
        assert!(!promotes_to_lkg(&LaunchOutcome::Cancelled));
        assert!(!promotes_to_lkg(&LaunchOutcome::Unknown));
        assert!(!promotes_to_lkg(&LaunchOutcome::Abandoned));
    }

    #[test]
    fn test_success_records_exact_snapshot_as_lkg() {
        let tmp = TempDir::new().unwrap();
        let snapshot_id = create_test_snapshot(tmp.path(), "known-good");
        let state = record_launch_outcome(
            tmp.path(),
            Some(&snapshot_id),
            "launch-1",
            LaunchOutcome::Success,
        )
        .unwrap();
        assert_eq!(
            state.current_lkg_snapshot_id.as_deref(),
            Some(snapshot_id.as_str())
        );
        assert_eq!(state.promoted_snapshot_ids, vec![snapshot_id]);
        assert_eq!(
            read_lkg_state(tmp.path()).unwrap().last_launch_outcome,
            Some(LaunchOutcome::Success)
        );
        assert!(tmp.path().join("launches.jsonl").is_file());
    }

    #[test]
    fn test_failed_launch_does_not_replace_current_lkg() {
        let tmp = TempDir::new().unwrap();
        let known_good = create_test_snapshot(tmp.path(), "known-good");
        record_launch_outcome(
            tmp.path(),
            Some(&known_good),
            "launch-1",
            LaunchOutcome::Success,
        )
        .unwrap();
        let state = record_launch_outcome(
            tmp.path(),
            Some("failed-candidate"),
            "launch-2",
            LaunchOutcome::Crash,
        )
        .unwrap();
        assert_eq!(
            state.current_lkg_snapshot_id.as_deref(),
            Some(known_good.as_str())
        );
        assert_eq!(state.last_launch_outcome, Some(LaunchOutcome::Crash));
    }

    #[test]
    fn test_cancelled_launch_preserves_current_lkg() {
        let tmp = TempDir::new().unwrap();
        let known_good = create_test_snapshot(tmp.path(), "known-good");
        record_launch_outcome(
            tmp.path(),
            Some(&known_good),
            "launch-1",
            LaunchOutcome::Success,
        )
        .unwrap();
        let state = record_launch_outcome(
            tmp.path(),
            Some(&known_good),
            "launch-2",
            LaunchOutcome::Cancelled,
        )
        .unwrap();
        assert_eq!(
            state.current_lkg_snapshot_id.as_deref(),
            Some(known_good.as_str())
        );
        assert_eq!(state.last_launch_outcome, Some(LaunchOutcome::Cancelled));
    }

    #[test]
    fn test_read_missing_lkg_state_returns_default() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(read_lkg_state(tmp.path()).unwrap(), LkgState::default());
    }

    #[test]
    fn test_success_without_snapshot_fails_closed() {
        let tmp = TempDir::new().unwrap();
        let error = record_launch_outcome(tmp.path(), None, "launch-1", LaunchOutcome::Success)
            .unwrap_err();
        assert!(error.contains("snapshot id"));
        assert!(!tmp.path().join("lkg.json").exists());
    }

    #[test]
    fn test_success_with_missing_snapshot_fails_closed() {
        let tmp = TempDir::new().unwrap();
        let error = record_launch_outcome(
            tmp.path(),
            Some("missing"),
            "launch-1",
            LaunchOutcome::Success,
        )
        .unwrap_err();
        assert!(error.contains("unavailable or invalid"));
        assert!(!tmp.path().join("lkg.json").exists());
    }

    #[test]
    fn test_compute_diff_empty() {
        let diff = compute_diff(&[], &[], None, None);
        assert_eq!(diff.added.len(), 0);
        assert_eq!(diff.removed.len(), 0);
        assert_eq!(diff.modified.len(), 0);
    }

    #[test]
    fn test_compute_diff_added_removed_modified() {
        let from = vec![
            FileEntry {
                path: "mod-a.jar".into(),
                sha256: "aaa".into(),
                size: 100,
            },
            FileEntry {
                path: "mod-b.jar".into(),
                sha256: "bbb".into(),
                size: 200,
            },
        ];
        let to = vec![
            FileEntry {
                path: "mod-b.jar".into(),
                sha256: "bbb-changed".into(),
                size: 210,
            },
            FileEntry {
                path: "mod-c.jar".into(),
                sha256: "ccc".into(),
                size: 300,
            },
        ];
        let diff = compute_diff(&from, &to, None, None);
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.added[0].path, "mod-c.jar");
        assert_eq!(diff.removed.len(), 1);
        assert_eq!(diff.removed[0].path, "mod-a.jar");
        assert_eq!(diff.modified.len(), 1);
        assert_eq!(diff.modified[0].path, "mod-b.jar");
        assert_eq!(diff.unchanged_count, 0);
    }

    #[test]
    fn test_compute_diff_identical_inputs_count_unchanged_files() {
        let files = vec![FileEntry {
            path: "mods/example.jar".into(),
            sha256: "abc".into(),
            size: 42,
        }];
        let diff = compute_diff(&files, &files, Some("a".into()), Some("b".into()));
        assert!(diff.added.is_empty());
        assert!(diff.removed.is_empty());
        assert!(diff.modified.is_empty());
        assert_eq!(diff.unchanged_count, 1);
    }

    #[test]
    fn test_retention_plan_keeps_current_lkg() {
        let ids: Vec<String> = (0..5).map(|i| format!("snap-{i}")).collect();
        let lkg: Vec<String> = vec!["snap-4".into(), "snap-2".into()];
        let policy = RetentionPolicy::default();
        let evict = retention_plan(&ids, &lkg, &[], &policy);
        assert!(!evict.contains(&"snap-4".to_string()));
    }

    #[test]
    fn test_retention_plan_evicts_oldest() {
        let ids: Vec<String> = (0..10).map(|i| format!("snap-{i}")).collect();
        let lkg: Vec<String> = vec!["snap-9".into(), "snap-8".into(), "snap-7".into()];
        let policy = RetentionPolicy {
            keep_lkg_count: 2,
            keep_non_lkg_count: 1,
            keep_pre_restore_count: 1,
            size_cap_bytes: 2_000_000_000,
        };
        let evict = retention_plan(&ids, &lkg, &[], &policy);
        // snap-9 and snap-8 kept (current LKG + 1 more), snap-7 evicted
        assert!(evict.contains(&"snap-7".to_string()));
        assert!(!evict.contains(&"snap-9".to_string()));
        assert!(!evict.contains(&"snap-8".to_string()));
    }

    #[test]
    fn test_retention_plan_honors_all_configured_counts() {
        let entries = vec![
            retention("current", 10, true, true, false),
            retention("non-1", 10, false, false, false),
            retention("non-2", 10, false, false, false),
            retention("pre-1", 10, false, false, true),
            retention("pre-2", 10, false, false, true),
            retention("lkg-old", 10, true, false, false),
        ];
        let policy = RetentionPolicy {
            keep_lkg_count: 2,
            keep_non_lkg_count: 2,
            keep_pre_restore_count: 2,
            size_cap_bytes: 1_000,
        };
        assert!(retention_plan_with_sizes(&entries, &policy).is_empty());
    }

    #[test]
    fn test_retention_size_cap_evicts_lower_value_snapshots_first() {
        let entries = vec![
            retention("current", 60, true, true, false),
            retention("new-regular", 30, false, false, false),
            retention("pre", 30, false, false, true),
            retention("old-lkg", 30, true, false, false),
        ];
        let policy = RetentionPolicy {
            keep_lkg_count: 2,
            keep_non_lkg_count: 1,
            keep_pre_restore_count: 1,
            size_cap_bytes: 90,
        };
        let evicted = retention_plan_with_sizes(&entries, &policy);
        assert!(evicted.contains(&"new-regular".to_string()));
        assert!(evicted.contains(&"pre".to_string()));
        assert!(!evicted.contains(&"current".to_string()));
        assert!(!evicted.contains(&"old-lkg".to_string()));
    }

    #[test]
    fn test_retention_keeps_oversized_current_lkg_as_last_recovery_point() {
        let entries = vec![
            retention("current", 200, true, true, false),
            retention("regular", 10, false, false, false),
        ];
        let policy = RetentionPolicy {
            keep_lkg_count: 1,
            keep_non_lkg_count: 1,
            keep_pre_restore_count: 0,
            size_cap_bytes: 100,
        };
        let evicted = retention_plan_with_sizes(&entries, &policy);
        assert_eq!(evicted, vec!["regular".to_string()]);
    }

    #[test]
    fn test_retention_evicts_pre_restore_before_known_good() {
        let entries = vec![
            retention("current", 50, true, true, false),
            retention("old-lkg", 30, true, false, false),
            retention("pre-restore", 30, false, false, true),
        ];
        let policy = RetentionPolicy {
            keep_lkg_count: 2,
            keep_non_lkg_count: 0,
            keep_pre_restore_count: 1,
            size_cap_bytes: 80,
        };
        let evicted = retention_plan_with_sizes(&entries, &policy);
        assert_eq!(evicted, vec!["pre-restore".to_string()]);
    }

    fn retention(
        id: &str,
        size_bytes: u64,
        is_lkg: bool,
        is_current_lkg: bool,
        is_pre_restore: bool,
    ) -> RetentionEntry {
        RetentionEntry {
            id: id.into(),
            size_bytes,
            is_lkg,
            is_current_lkg,
            is_pre_restore,
        }
    }
}
