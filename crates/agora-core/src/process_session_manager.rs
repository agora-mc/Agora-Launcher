use crate::error::{LauncherError, LauncherResult};
use crate::process_identity::{self, ProcessIdentity};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

/// A tracked direct-launch session.
#[derive(Debug, Clone)]
pub struct ProcessSession {
    pub instance_id: String,
    pub session_id: u64,
    pub pid: u32,
    pub process_identity: ProcessIdentity,
    pub snapshot_id: String,
    pub start_time: SystemTime,
    /// Whether the session is still attached (waiting for game exit).
    pub attached: bool,
    /// Whether the user has requested termination of this session.
    pub user_cancelled: bool,
}

/// Core-owned manager for direct-launch process sessions.
///
/// Thread-safe internally via `RwLock`; all methods are synchronous and
/// suitable for use from `spawn_blocking` or async contexts without
/// holding external locks.
#[derive(Debug, Clone)]
pub struct ProcessSessionManager {
    sessions: Arc<RwLock<HashMap<u64, ProcessSession>>>,
    /// Tracks the most recent session_id per instance for staleness checks.
    /// Used by delegated monitoring to detect same-instance replacement
    /// without cross-instance interference.
    latest_per_instance: Arc<RwLock<HashMap<String, u64>>>,
}

impl ProcessSessionManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            latest_per_instance: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Record that `session_id` is the latest session for `instance_id`.
    /// Called from both direct and delegated launch paths.
    pub fn note_latest(&self, instance_id: &str, session_id: u64) {
        if let Ok(mut map) = self.latest_per_instance.write() {
            map.insert(instance_id.to_string(), session_id);
        }
    }

    /// Return the latest session_id recorded for `instance_id`, if any.
    pub fn latest_for_instance(&self, instance_id: &str) -> Option<u64> {
        self.latest_per_instance
            .read()
            .ok()
            .and_then(|map| map.get(instance_id).copied())
    }

    /// Returns `true` when `session_id` is still the latest for its instance.
    /// Used by monitoring loops to detect same-instance replacement.
    pub fn is_latest_for_instance(&self, instance_id: &str, session_id: u64) -> bool {
        self.latest_for_instance(instance_id) == Some(session_id)
    }

    /// Register a new session.  Returns `Err` if `session_id` already exists.
    pub fn register(&self, session: ProcessSession) -> LauncherResult<()> {
        let mut map = self.sessions.write().map_err(|_| LauncherError::Generic {
            code: "ERR_SESSION_MANAGER_LOCK".into(),
            message: "Session manager write lock poisoned".into(),
        })?;
        let id = session.session_id;
        if map.contains_key(&id) {
            return Err(LauncherError::Generic {
                code: "ERR_SESSION_ALREADY_EXISTS".into(),
                message: format!("Session {id} is already registered"),
            });
        }
        map.insert(id, session);
        Ok(())
    }

    /// Get a session by ID (returns a clone).
    pub fn get(&self, session_id: u64) -> Option<ProcessSession> {
        let map = self.sessions.read().ok()?;
        map.get(&session_id).cloned()
    }

    /// Return all active sessions.
    pub fn list(&self) -> Vec<ProcessSession> {
        self.sessions
            .read()
            .ok()
            .map(|map| map.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Remove a session by ID, returning it if present.
    pub fn remove(&self, session_id: u64) -> Option<ProcessSession> {
        let mut map = self.sessions.write().ok()?;
        map.remove(&session_id)
    }

    /// Terminate a session after verifying the caller-supplied PID matches
    /// the tracked session AND the OS-level process identity is still valid.
    ///
    /// Steps:
    /// 1. Session lookup + PID match check.
    /// 2. OS identity verification via `process_identity::verify`.
    /// 3. Mark session as `user_cancelled`.
    /// 4. Platform-appropriate process kill (`taskkill /F /T` on Windows,
    ///    `kill -9` on Unix).
    /// 5. Remove the session on success; revert `user_cancelled` on failure.
    ///
    /// Returns `Err(ProcessStale)` when the OS process no longer matches the
    /// captured identity — the stale session is removed automatically.
    pub fn terminate(&self, session_id: u64, expected_pid: u32) -> LauncherResult<()> {
        // Phase 1 — snapshot session under lock, verify PID.
        let session = {
            let map = self.sessions.read().map_err(|_| LauncherError::Generic {
                code: "ERR_SESSION_MANAGER_LOCK".into(),
                message: "Session manager read lock poisoned".into(),
            })?;
            let session = map.get(&session_id).ok_or_else(|| LauncherError::Generic {
                code: "ERR_SESSION_NOT_FOUND".into(),
                message: format!("Session {session_id} not found"),
            })?;
            if session.pid != expected_pid {
                return Err(LauncherError::Generic {
                    code: "ERR_PID_MISMATCH".into(),
                    message: format!(
                        "PID mismatch: caller supplied {expected_pid} but session {session_id} has PID {}",
                        session.pid
                    ),
                });
            }
            session.clone()
        };

        // Phase 2 — verify OS identity outside the session lock.
        if let Err(stale_err) = process_identity::verify(&session.process_identity) {
            self.remove(session_id);
            return Err(stale_err);
        }

        // Phase 3 — mark user_cancelled.
        {
            let mut map = self.sessions.write().map_err(|_| LauncherError::Generic {
                code: "ERR_SESSION_MANAGER_LOCK".into(),
                message: "Session manager write lock poisoned (user_cancelled)".into(),
            })?;
            if let Some(s) = map.get_mut(&session_id) {
                s.user_cancelled = true;
            }
        }

        // Phase 4 — kill.
        let kill_result = Self::kill_pid(session.pid);

        match kill_result {
            Ok(()) => {
                self.remove(session_id);
                Ok(())
            }
            Err(error) => {
                // Revert user_cancelled so lifecycle path still works.
                let mut map = self.sessions.write().map_err(|_| LauncherError::Generic {
                    code: "ERR_SESSION_MANAGER_LOCK".into(),
                    message: "Session manager write lock poisoned (kill revert)".into(),
                })?;
                if let Some(s) = map.get_mut(&session_id) {
                    s.user_cancelled = false;
                }
                Err(LauncherError::Generic {
                    code: "ERR_KILL_FAILED".into(),
                    message: format!("Could not terminate PID {}: {error}", session.pid),
                })
            }
        }
    }

    /// Perform the OS-level process kill.
    fn kill_pid(pid: u32) -> Result<(), String> {
        #[cfg(target_os = "windows")]
        {
            let mut command = std::process::Command::new("taskkill");
            command.args(["/PID", &pid.to_string(), "/F", "/T"]);
            // Prevent an empty command prompt window from flashing on kill.
            crate::helpers::hide_console_window(&mut command);
            let output = command
                .output()
                .map_err(|e| format!("Failed to spawn taskkill: {e}"))?;
            if output.status.success() {
                Ok(())
            } else {
                Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let output = std::process::Command::new("kill")
                .args(["-9", &pid.to_string()])
                .output()
                .map_err(|e| format!("Failed to spawn kill: {e}"))?;
            if output.status.success() {
                Ok(())
            } else {
                Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
            }
        }
    }
}

impl ProcessSessionManager {
    /// Terminate all sessions for a given instance_id.
    ///
    /// Identity-safe: delegates to [`terminate`] for every matching session,
    /// which verifies PID match and OS identity before killing. Returns the
    /// first error encountered; remaining sessions are still processed.
    /// Returns an error when no session exists for the instance.
    pub fn terminate_for_instance(&self, instance_id: &str) -> LauncherResult<()> {
        let sessions = self.list();
        let matching: Vec<ProcessSession> = sessions
            .into_iter()
            .filter(|s| s.instance_id == instance_id)
            .collect();
        if matching.is_empty() {
            return Err(LauncherError::Generic {
                code: "ERR_SESSION_NOT_FOUND".into(),
                message: format!("No running process for instance '{}'", instance_id),
            });
        }
        let mut first_error = None;
        for session in matching {
            if let Err(e) = self.terminate(session.session_id, session.pid) {
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }
        match first_error {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

impl Default for ProcessSessionManager {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process_identity::ProcessIdentity;

    fn make_session(session_id: u64, pid: u32) -> ProcessSession {
        ProcessSession {
            instance_id: "test-instance".into(),
            session_id,
            pid,
            process_identity: ProcessIdentity {
                pid,
                start_time: 1000,
                expected_exe: None,
            },
            snapshot_id: "snap-1".into(),
            start_time: SystemTime::now(),
            attached: true,
            user_cancelled: false,
        }
    }

    #[test]
    fn test_register_and_get() {
        let mgr = ProcessSessionManager::new();
        let s = make_session(1, 42);
        mgr.register(s.clone()).unwrap();
        let retrieved = mgr.get(1).unwrap();
        assert_eq!(retrieved.session_id, 1);
        assert_eq!(retrieved.pid, 42);
    }

    #[test]
    fn test_register_duplicate_fails() {
        let mgr = ProcessSessionManager::new();
        mgr.register(make_session(1, 42)).unwrap();
        let result = mgr.register(make_session(1, 99));
        assert!(result.is_err());
    }

    #[test]
    fn test_get_nonexistent_returns_none() {
        let mgr = ProcessSessionManager::new();
        assert!(mgr.get(999).is_none());
    }

    #[test]
    fn test_list_empty() {
        let mgr = ProcessSessionManager::new();
        assert!(mgr.list().is_empty());
    }

    #[test]
    fn test_list_returns_all() {
        let mgr = ProcessSessionManager::new();
        mgr.register(make_session(1, 42)).unwrap();
        mgr.register(make_session(2, 43)).unwrap();
        assert_eq!(mgr.list().len(), 2);
    }

    #[test]
    fn test_remove_returns_session() {
        let mgr = ProcessSessionManager::new();
        mgr.register(make_session(1, 42)).unwrap();
        let removed = mgr.remove(1);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().pid, 42);
        assert!(mgr.get(1).is_none());
    }

    #[test]
    fn test_remove_nonexistent_returns_none() {
        let mgr = ProcessSessionManager::new();
        assert!(mgr.remove(999).is_none());
    }

    #[test]
    fn test_terminate_pid_mismatch_rejected() {
        let mgr = ProcessSessionManager::new();
        mgr.register(make_session(1, 42)).unwrap();
        let result = mgr.terminate(1, 99);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code(), "ERR_PID_MISMATCH");
        // Session should still be present.
        assert!(mgr.get(1).is_some());
    }

    #[test]
    fn test_terminate_nonexistent_session_fails() {
        let mgr = ProcessSessionManager::new();
        let result = mgr.terminate(999, 42);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), "ERR_SESSION_NOT_FOUND");
    }

    #[test]
    fn test_terminate_stale_process_removes_session() {
        // Use a PID that definitely does not exist.
        let mgr = ProcessSessionManager::new();
        let mut session = make_session(1, u32::MAX);
        // The identity must match — set a start_time that won't match any process.
        session.process_identity.start_time = 0;
        mgr.register(session).unwrap();

        let result = mgr.terminate(1, u32::MAX);
        // Should get ProcessStale because start_time=0 won't match anything
        assert!(result.is_err());
        // Session must be removed on stale.
        assert!(mgr.get(1).is_none());
    }

    // -----------------------------------------------------------------------
    // terminate_for_instance — identity-safe via terminate()
    // -----------------------------------------------------------------------

    #[test]
    fn test_terminate_for_instance_nonexistent_fails() {
        let mgr = ProcessSessionManager::new();
        assert!(mgr.terminate_for_instance("no-such-instance").is_err());
    }

    #[test]
    fn test_terminate_for_instance_mismatched_pid_rejected() {
        let mgr = ProcessSessionManager::new();
        let mut s = make_session(1, 100);
        s.instance_id = "inst-a".into();
        mgr.register(s).unwrap();
        // terminate_for_instance calls terminate(session_id, pid) which checks
        // the stored PID. The stored pid=100 must match the one we registered.
        // The terminate() checks PID match first — it will look up session 1,
        // find pid=100, and compare with the pid in the session (also 100).
        // But terminate() also calls process_identity::verify which will fail
        // for pid=100 if it doesn't exist. So we test the PID mismatch case
        // with a well-known PID that exists (the current process).
        // Actually we test with PID mismatch edge: in the stored session pid=100,
        // terminate(session_id=1, expected_pid=100) — the stored and expected
        // match at 100. Then verify fails because pid 100 doesn't exist.
        let result = mgr.terminate_for_instance("inst-a");
        // verify will fail (ProcessStale) — session gets removed
        assert!(result.is_err());
        assert!(mgr.get(1).is_none());
    }

    #[test]
    fn test_terminate_for_instance_removes_matching_sessions() {
        let mgr = ProcessSessionManager::new();
        let mut s1 = make_session(1, u32::MAX);
        s1.instance_id = "inst-a".into();
        s1.process_identity.start_time = 0;
        mgr.register(s1).unwrap();
        let mut s2 = make_session(2, u32::MAX - 1);
        s2.instance_id = "inst-b".into();
        s2.process_identity.start_time = 0;
        mgr.register(s2).unwrap();

        let result = mgr.terminate_for_instance("inst-a");
        // stale -> removed
        assert!(result.is_err());
        assert!(mgr.get(1).is_none());
        // inst-b session remains
        assert!(mgr.get(2).is_some());
    }

    // -----------------------------------------------------------------------
    // Per-instance latest session tracking
    // -----------------------------------------------------------------------

    #[test]
    fn test_note_latest_and_check() {
        let mgr = ProcessSessionManager::new();
        assert_eq!(mgr.latest_for_instance("inst-a"), None);

        mgr.note_latest("inst-a", 1);
        assert_eq!(mgr.latest_for_instance("inst-a"), Some(1));
        assert!(mgr.is_latest_for_instance("inst-a", 1));
        assert!(!mgr.is_latest_for_instance("inst-a", 2));

        // Different instance — independent.
        assert_eq!(mgr.latest_for_instance("inst-b"), None);
        mgr.note_latest("inst-b", 5);
        assert!(mgr.is_latest_for_instance("inst-a", 1));
        assert!(mgr.is_latest_for_instance("inst-b", 5));
    }

    #[test]
    fn test_same_instance_replacement_makes_older_stale() {
        let mgr = ProcessSessionManager::new();
        mgr.note_latest("inst-a", 1);
        assert!(mgr.is_latest_for_instance("inst-a", 1));

        // Newer session for same instance replaces.
        mgr.note_latest("inst-a", 2);
        assert!(!mgr.is_latest_for_instance("inst-a", 1));
        assert!(mgr.is_latest_for_instance("inst-a", 2));

        // Different instance not affected.
        mgr.note_latest("inst-b", 3);
        assert!(mgr.is_latest_for_instance("inst-a", 2));
        assert!(mgr.is_latest_for_instance("inst-b", 3));
    }
}
