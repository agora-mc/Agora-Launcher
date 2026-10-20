//! Tells one Agora program when another has changed an instance.
//!
//! The desktop app and the CLI share a data root, and with an AI agent driving
//! the CLI both are often running at once. Each loads data into its own memory,
//! so without this the app keeps showing a mod the agent just disabled until
//! something makes it reload.
//!
//! A change is recorded as one small file per instance under
//! `<locks_root>/changes/`, replaced on every change. Releasing an instance
//! lock records one automatically, which covers every operation that changes
//! instance files; the few database-only edits that take no lock (rename,
//! lock/unlock, JVM settings) call [`record_instance_change`] themselves.
//! [`ChangeFeed::poll`] reports records written by *other* processes since the
//! last poll. Recording is best-effort: a missed record costs a stale screen,
//! never data.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const CHANGES_DIR: &str = "changes";

/// One recorded change, as stored on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChangeRecord {
    pid: u32,
    #[serde(default)]
    program: String,
    operation: String,
    at: String,
    /// Distinguishes two changes by the same process within one poll interval.
    nonce: u128,
}

/// A change another process made to an instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstanceChange {
    pub instance_id: String,
    /// The other program's name, e.g. "Agora CLI". Empty if it did not say.
    pub program: String,
    /// What it did, e.g. "disable_mod".
    pub operation: String,
}

fn changes_dir(locks_root: &Path) -> PathBuf {
    locks_root.join(CHANGES_DIR)
}

/// Record that this process changed `instance_id`.
pub fn record_instance_change(locks_root: &Path, instance_id: &str, operation: &str) {
    let dir = changes_dir(locks_root);
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let record = ChangeRecord {
        pid: std::process::id(),
        program: crate::lock_manager::program_name(),
        operation: operation.to_string(),
        at: chrono::Utc::now().to_rfc3339(),
        nonce: uuid::Uuid::new_v4().as_u128(),
    };
    let Ok(json) = serde_json::to_string(&record) else {
        return;
    };
    // Written beside the target and renamed over it, so a reader never sees a
    // half-written record. The temporary name is unique per write because two
    // threads of one process can finish with the same instance at once.
    let target = dir.join(format!("{instance_id}.json"));
    let tmp = dir.join(format!(".{instance_id}.{:x}.tmp", record.nonce));
    if std::fs::write(&tmp, json).is_err() {
        return;
    }
    // On Windows the rename fails if a reader has the target open at that
    // instant. One retry is enough: readers hold it for a single read.
    if std::fs::rename(&tmp, &target).is_err() {
        std::thread::sleep(std::time::Duration::from_millis(20));
        if std::fs::rename(&tmp, &target).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

/// Watches for instance changes made by other processes.
#[derive(Debug)]
pub struct ChangeFeed {
    dir: PathBuf,
    seen: HashMap<String, u128>,
}

impl ChangeFeed {
    /// Start watching. Changes recorded before this call are not reported.
    pub fn new(locks_root: &Path) -> Self {
        let mut feed = Self {
            dir: changes_dir(locks_root),
            seen: HashMap::new(),
        };
        feed.poll();
        feed
    }

    /// Changes other processes recorded since the previous call.
    pub fn poll(&mut self) -> Vec<InstanceChange> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let own_pid = std::process::id();
        let mut changes = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(instance_id) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(record) = serde_json::from_str::<ChangeRecord>(&text) else {
                continue;
            };
            if self.seen.get(instance_id) == Some(&record.nonce) {
                continue;
            }
            self.seen.insert(instance_id.to_string(), record.nonce);
            if record.pid != own_pid {
                changes.push(InstanceChange {
                    instance_id: instance_id.to_string(),
                    program: record.program,
                    operation: record.operation,
                });
            }
        }
        changes.sort_by(|a, b| a.instance_id.cmp(&b.instance_id));
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_foreign(locks_root: &Path, instance_id: &str, nonce: u128) {
        let dir = changes_dir(locks_root);
        std::fs::create_dir_all(&dir).unwrap();
        let record = ChangeRecord {
            pid: std::process::id().wrapping_add(1),
            program: "Agora CLI".into(),
            operation: "disable_mod".into(),
            at: "2026-01-01T00:00:00Z".into(),
            nonce,
        };
        std::fs::write(
            dir.join(format!("{instance_id}.json")),
            serde_json::to_string(&record).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn reports_another_process_change_once() {
        let root = tempfile::tempdir().unwrap();
        let mut feed = ChangeFeed::new(root.path());
        assert!(feed.poll().is_empty());

        write_foreign(root.path(), "survival", 1);
        assert_eq!(
            feed.poll(),
            vec![InstanceChange {
                instance_id: "survival".into(),
                program: "Agora CLI".into(),
                operation: "disable_mod".into(),
            }]
        );
        assert!(
            feed.poll().is_empty(),
            "the same record is not reported twice"
        );

        write_foreign(root.path(), "survival", 2);
        assert_eq!(feed.poll().len(), 1, "a newer change is reported");
    }

    #[test]
    fn ignores_this_process_and_older_changes() {
        let root = tempfile::tempdir().unwrap();
        write_foreign(root.path(), "old", 1);
        let mut feed = ChangeFeed::new(root.path());
        assert!(
            feed.poll().is_empty(),
            "changes before watching are skipped"
        );

        record_instance_change(root.path(), "mine", "rename");
        assert!(feed.poll().is_empty(), "own changes are not reported");
    }
}
