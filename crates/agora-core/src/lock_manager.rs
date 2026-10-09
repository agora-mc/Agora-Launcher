//! Cross-process filesystem-based locking for Agora.
//!
//! `LockManager` provides RAII lock guards backed by **exclusive file creation**.
//! Acquisition is atomic: `OpenOptions::new().create_new(true).write(true).open()`
//! on the final lock path. A competing process cannot overwrite our lock
//! because `create_new` fails atomically if the file already exists.
//!
//! Each lock carries an **ownership nonce** (a random u64). On release,
//! [`LockGuard::drop`] reads the current on-disk metadata and only removes the
//! file if the nonce still matches — an old guard cannot delete a newer owner's
//! lock after stale-recovery.
//!
//! **Stale recovery**: a lock is broken only when the owning process is
//! confirmed dead via [`sysinfo`]. An age limit is NOT used to break live
//! locks — long-running operations are protected.
//!
//! **Cancellation**: `acquire_with_timeout` and `acquire_cancellable` accept an
//! optional [`CancellationToken`]. The acquisition loop checks the token on
//! each retry and exits early on cancellation.

use crate::app_paths;
use crate::error::{LauncherError, LauncherResult};
use crate::event_sink::CancellationToken;
use agora_game_api::{GameId, StoreId};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Default timeout for acquiring a lock (30 seconds).
const DEFAULT_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(30);

/// Delay between retry attempts when acquiring a contested lock.
const RETRY_DELAY: Duration = Duration::from_secs(1);

// ---------------------------------------------------------------------------
// Lock metadata
// ---------------------------------------------------------------------------

/// Metadata written into every lock file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockMetadata {
    /// PID of the process holding the lock.
    pub pid: u32,
    /// Process start time (seconds since epoch) for stronger identity checks.
    pub pid_start_time: u64,
    /// Hostname (best-effort).
    pub hostname: String,
    /// ISO-8601 timestamp when the lock was acquired.
    pub acquired_at: String,
    /// Human-readable description of the operation holding the lock.
    pub operation: String,
    /// Random ownership nonce — prevents stale-guard deletion after replacement.
    pub nonce: u128,
}

// ---------------------------------------------------------------------------
// LockError (structured, public)
// ---------------------------------------------------------------------------

/// Structured error from a lock acquisition attempt.
#[derive(Debug)]
pub enum LockError {
    /// The lock is held by a live process. Carry the current owner's metadata.
    Contested(LockMetadata),
    /// The lock file exists but its owner process is dead. The caller may
    /// break the lock and retry.
    Stale(LockMetadata),
    /// Lock file is corrupt or empty (partial write). Recovery may proceed
    /// only after a grace period and confirmed-dead owner.
    Corrupt,
    /// I/O error interacting with the lock file.
    Io(std::io::Error),
}

impl LockError {
    /// Return the lock metadata if available (Contested or Stale).
    pub fn metadata(&self) -> Option<&LockMetadata> {
        match self {
            LockError::Contested(m) | LockError::Stale(m) => Some(m),
            LockError::Corrupt | LockError::Io(_) => None,
        }
    }

    fn into_launcher_error(self, resource: &str) -> LauncherError {
        match self {
            LockError::Contested(meta) => LauncherError::Generic {
                code: "ERR_LOCK_CONTESTED".into(),
                message: format!(
                    "Resource '{resource}' is locked by PID {} on {} (since {}, operation: {})",
                    meta.pid, meta.hostname, meta.acquired_at, meta.operation,
                ),
            },
            LockError::Stale(meta) => LauncherError::Generic {
                code: "ERR_LOCK_STALE".into(),
                message: format!(
                    "Resource '{resource}' has a stale lock from PID {} on {} (operation: {}). Retry.",
                    meta.pid, meta.hostname, meta.operation,
                ),
            },
            LockError::Corrupt => LauncherError::Generic {
                code: "ERR_LOCK_CORRUPT".into(),
                message: format!("Lock file for '{resource}' is corrupt; retrying."),
            },
            LockError::Io(e) => LauncherError::Generic {
                code: "ERR_LOCK_IO".into(),
                message: format!("Lock I/O error for '{resource}': {e}"),
            },
        }
    }
}

// ---------------------------------------------------------------------------
// LockGuard
// ---------------------------------------------------------------------------

/// A filesystem-based lock guard.
///
/// On [`Drop`], reads the on-disk lock metadata and only removes the file if
/// the ownership nonce still matches. This prevents an old guard from deleting
/// a lock that was replaced via stale recovery by another thread/process.
#[derive(Debug)]
pub struct LockGuard {
    lock_path: PathBuf,
    nonce: u128,
    released: Arc<AtomicBool>,
}

impl LockGuard {
    /// The path to the lock file on disk.
    pub fn path(&self) -> &Path {
        &self.lock_path
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        if self.released.load(Ordering::Relaxed) {
            return;
        }
        // Read the current on-disk metadata. Only remove if our nonce still
        // matches (no replacement happened).
        if let Ok(current_meta) = read_lock_metadata(&self.lock_path) {
            if current_meta.nonce == self.nonce {
                let _ = std::fs::remove_file(&self.lock_path);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// LockResource
// ---------------------------------------------------------------------------

/// Named locks that the manager knows about.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LockResource {
    /// Exclusive access to registry update/download.
    RegistryUpdate,
    /// Exclusive access to loader installation.
    LoaderInstall,
    /// Per-major-version Java runtime provisioning (e.g., `JavaMajor(21)`).
    JavaMajor(u32),
    /// Exclusive access to launch materialization.
    Materialization,
    /// Per-instance lock for install/remove/update operations.
    Instance(String),
    /// Exclusive access to the content store for adding or removing items.
    ContentStore,
    /// Exclusive access for per-user game files journaled swap.
    GameUserFiles(GameId, StoreId),
}

impl LockResource {
    /// Return the filesystem-safe lock name, validating all inputs.
    ///
    /// Returns `Err` if:
    /// - An instance ID contains path separators, traversal, or is empty.
    /// - A Java major version is 0.
    pub fn lock_name(&self) -> LauncherResult<String> {
        match self {
            LockResource::RegistryUpdate => Ok("registry-update".into()),
            LockResource::LoaderInstall => Ok("loader-install".into()),
            LockResource::JavaMajor(major) => {
                if *major == 0 {
                    return Err(LauncherError::Generic {
                        code: "ERR_INVALID_LOCK".into(),
                        message: "Java major version must be >= 1 for lock acquisition.".into(),
                    });
                }
                Ok(format!("java-major-{major}"))
            }
            LockResource::Materialization => Ok("materialization".into()),
            LockResource::Instance(id) => {
                app_paths::validate_path_component(id)?;
                Ok(format!("instance-{id}"))
            }
            LockResource::ContentStore => Ok("content-store".into()),
            LockResource::GameUserFiles(game, store) => {
                app_paths::validate_path_component(game.as_str())?;
                app_paths::validate_path_component(store.as_str())?;
                Ok(format!(
                    "game-user-files-{}-{}",
                    game.as_str(),
                    store.as_str()
                ))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// LockManager
// ---------------------------------------------------------------------------

/// Cross-process lock manager backed by atomic file creation.
///
/// All locks live under the configured `locks_root` directory.
#[derive(Debug, Clone)]
pub struct LockManager {
    locks_root: PathBuf,
}

impl LockManager {
    /// Create a new lock manager that stores lock files under `locks_root`.
    pub fn new(locks_root: PathBuf) -> Self {
        Self { locks_root }
    }

    /// The root directory for lock files.
    pub fn locks_root(&self) -> &Path {
        &self.locks_root
    }

    // ------------------------------------------------------------------
    // Public acquire helpers
    // ------------------------------------------------------------------

    /// Attempt to acquire a lock with the default timeout (30s) and no
    /// cancellation token.
    pub fn acquire(&self, resource: LockResource, operation: &str) -> LauncherResult<LockGuard> {
        self.acquire_with_timeout(resource, operation, DEFAULT_ACQUIRE_TIMEOUT, None)
    }

    /// Attempt to acquire a lock with a custom timeout.
    pub fn acquire_with_timeout(
        &self,
        resource: LockResource,
        operation: &str,
        timeout: Duration,
        cancel: Option<&CancellationToken>,
    ) -> LauncherResult<LockGuard> {
        let lock_name = resource.lock_name()?;
        let lock_path = self.locks_root.join(format!("{lock_name}.lock"));
        let deadline = Instant::now() + timeout;

        loop {
            // Check cancellation before each attempt.
            if let Some(tok) = &cancel {
                if tok.is_cancelled() {
                    return Err(LauncherError::Generic {
                        code: "ERR_LOCK_CANCELLED".into(),
                        message: format!("Lock acquisition cancelled for '{lock_name}'"),
                    });
                }
            }

            match try_acquire(&lock_path, operation) {
                Ok(guard) => return Ok(guard),
                Err(LockError::Contested(meta)) => {
                    if Instant::now() >= deadline {
                        return Err(LauncherError::Generic {
                            code: "ERR_LOCK_TIMEOUT".into(),
                            message: format!(
                                "Could not acquire lock '{lock_name}' within {}s. \
                                 Held by PID {} on {} since {} for: {}",
                                timeout.as_secs(),
                                meta.pid,
                                meta.hostname,
                                meta.acquired_at,
                                meta.operation,
                            ),
                        });
                    }
                    std::thread::sleep(RETRY_DELAY);
                }
                Err(LockError::Stale(meta)) => {
                    // Owner is dead — break the lock and retry.
                    eprintln!(
                        "[lock_manager] Breaking stale lock '{lock_name}' \
                         (PID {} on {}, operation: {}). Owner no longer alive.",
                        meta.pid, meta.hostname, meta.operation,
                    );
                    let _ = std::fs::remove_file(&lock_path);
                }
                Err(LockError::Corrupt) => {
                    // Lock file is present but unreadable. Wait for a grace
                    // interval to let the owner finish writing metadata.
                    // If still corrupt after grace, only a PID that is
                    // confirmed dead permits recovery. Unknown ownership is
                    // preserved rather than risking interruption of a live
                    // writer whose metadata is not yet readable.
                    let corrupt_start = std::time::Instant::now();
                    const CORRUPT_GRACE: Duration = Duration::from_secs(3);
                    let mut recovered = false;
                    while corrupt_start.elapsed() < CORRUPT_GRACE && !recovered {
                        std::thread::sleep(Duration::from_millis(500));
                        // The owner finished writing, or already released the lock: the next
                        // attempt sees which, and waits, breaks a stale lock or takes it.
                        if !lock_path.exists() || read_lock_metadata(&lock_path).is_ok() {
                            recovered = true;
                        }
                    }
                    if !recovered {
                        // Grace expired — still corrupt. Check for a readable PID.
                        let isolated_pid = extract_owner_pid_from_corrupt_lock(&lock_path);
                        if let Some(pid) = isolated_pid {
                            let owner_alive = matches!(probe_process(pid), Some((true, _)));
                            if !owner_alive {
                                eprintln!(
                                    "[lock_manager] Breaking corrupt lock '{}' after grace; PID {} is dead.",
                                    lock_name,
                                    pid,
                                );
                                let _ = std::fs::remove_file(&lock_path);
                            } else {
                                return Err(LauncherError::Generic {
                                    code: "ERR_LOCK_CORRUPT".into(),
                                    message: format!(
                                        "Resource '{lock_name}' has a corrupt lock file; owner PID {} appears alive. \
                                         Try again later or remove the lock manually.",
                                        pid,
                                    ),
                                });
                            }
                        } else {
                            eprintln!(
                                "[lock_manager] Preserving corrupt lock '{}' after grace; owner is unknown.",
                                lock_name
                            );
                            return Err(LauncherError::Generic {
                                code: "ERR_LOCK_CORRUPT".into(),
                                message: format!(
                                    "Resource '{lock_name}' has a corrupt lock file with unknown owner; \
                                     try again later or remove the lock manually.",
                                ),
                            });
                        }
                    }
                }
                Err(LockError::Io(e)) => {
                    return Err(LockError::Io(e).into_launcher_error(&lock_name));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Internal: try_acquire (stateless, reusable)
// ---------------------------------------------------------------------------

/// Attempt to acquire the lock at `lock_path` once. Does not retry.
///
/// Uses `OpenOptions::new().create_new(true).write(true).open(lock_path)` for
/// atomic creation — this is the cross-platform primitive that fails with
/// `AlreadyExists` when the file already exists. No check-then-rename race.
///
/// If the lock file exists but contains partial/corrupt metadata (e.g. from
/// a crash during write), the contender returns `Corrupt`. The caller applies
/// a grace period and only removes the file when a partial PID is confirmed
/// dead; unknown ownership remains fail-closed.
fn try_acquire(lock_path: &Path, operation: &str) -> Result<LockGuard, LockError> {
    let metadata = LockMetadata {
        pid: std::process::id(),
        pid_start_time: get_own_start_time(),
        hostname: hostname(),
        acquired_at: chrono::Utc::now().to_rfc3339(),
        operation: operation.to_string(),
        nonce: uuid::Uuid::new_v4().as_u128(),
    };

    // Serialise metadata to JSON.
    let json =
        serde_json::to_string(&metadata).map_err(|e| LockError::Io(std::io::Error::other(e)))?;

    // Atomically create the lock file. create_new(true) fails with
    // AlreadyExists if the file already exists — no race possible.
    match OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(lock_path)
    {
        Ok(mut file) => {
            file.write_all(json.as_bytes())
                .and_then(|_| file.sync_all())
                .map_err(LockError::Io)?;
            Ok(LockGuard {
                lock_path: lock_path.to_path_buf(),
                nonce: metadata.nonce,
                released: Arc::new(AtomicBool::new(false)),
            })
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // Lock file exists. Try to read metadata.
            match read_lock_metadata(lock_path) {
                Ok(meta) => {
                    if is_stale_lock(&meta) {
                        Err(LockError::Stale(meta))
                    } else {
                        Err(LockError::Contested(meta))
                    }
                }
                Err(read_err) => {
                    // File exists but metadata is unreadable (partial write or
                    // corrupt). We DON'T immediately delete — the owner may be
                    // between create_new and metadata write. Return Corrupt so
                    // the caller retries. Only after a grace interval and
                    // confirmed dead owner do we break.
                    eprintln!(
                        "[lock_manager] Lock file exists but metadata is corrupt: {}. \
                         Will retry; not breaking yet.",
                        read_err
                    );
                    Err(LockError::Corrupt)
                }
            }
        }
        Err(e) => Err(LockError::Io(e)),
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn hostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "unknown".into())
}

fn read_lock_metadata(lock_path: &Path) -> Result<LockMetadata, std::io::Error> {
    let mut file = File::open(lock_path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    // Reject empty files (partial writes, crash residue).
    if contents.trim().is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "lock file is empty",
        ));
    }
    serde_json::from_str(&contents)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Get our own process start time (seconds since epoch) via sysinfo.
fn get_own_start_time() -> u64 {
    let mut system = sysinfo::System::new();
    let own_pid = sysinfo::Pid::from_u32(std::process::id());
    system.refresh_process(own_pid);
    system.process(own_pid).map(|p| p.start_time()).unwrap_or(0)
}

/// A lock is stale only when the owning process is confirmed dead,
/// or its start time has changed (PID reuse).
///
/// We do NOT use an age-based threshold to break locks held by live
/// processes — long-running operations must not be interrupted.
fn is_stale_lock(metadata: &LockMetadata) -> bool {
    match probe_process(metadata.pid) {
        Some((alive, start_time))
            if alive
                && metadata.pid_start_time != 0
                && start_time != 0
                && metadata.pid_start_time != start_time =>
        {
            true // PID reused
        }
        Some((true, _)) => false, // alive and start-time matches
        _ => true,                // dead or not found
    }
}

/// Try to extract the PID from a lock file that has partial content.
/// Looks for `"pid":<number>` or `"pid": <number>` via simple text scan.
fn extract_owner_pid_from_corrupt_lock(lock_path: &Path) -> Option<u32> {
    let data = std::fs::read(lock_path).ok()?;
    let text = String::from_utf8_lossy(&data);
    // Look for the standard JSON key pattern: "pid":<spaces><digits>
    let needle = "\"pid\":";
    if let Some(pos) = text.find(needle) {
        let after = &text[pos + needle.len()..];
        // Skip whitespace
        let digits_start = after.find(|c: char| !c.is_whitespace())?;
        let after_ws = &after[digits_start..];
        // Collect digits
        let digits: String = after_ws
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if !digits.is_empty() {
            return digits.parse::<u32>().ok();
        }
    }
    None
}

/// Probe a process by PID. Returns `Some((is_alive, start_time))` or `None`
/// if the process was not found in the system table.
fn probe_process(pid: u32) -> Option<(bool, u64)> {
    let mut system = sysinfo::System::new();
    system.refresh_process(sysinfo::Pid::from_u32(pid));
    system
        .process(sysinfo::Pid::from_u32(pid))
        .map(|p| (true, p.start_time()))
        .or(Some((false, 0)))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    static DIR_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn test_manager() -> (LockManager, PathBuf) {
        let n = DIR_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("agora-lock-test-{}-{}", std::process::id(), n));
        let _ = std::fs::create_dir_all(&dir);
        (LockManager::new(dir.clone()), dir)
    }

    fn cleanup(dir: &Path) {
        if dir.exists() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn test_acquire_and_release() {
        let (mgr, dir) = test_manager();
        let lock = mgr.acquire(LockResource::RegistryUpdate, "test");
        assert!(lock.is_ok(), "should acquire: {:?}", lock.err());
        drop(lock);
        cleanup(&dir);
    }

    #[test]
    fn test_same_process_contention() {
        let (mgr, dir) = test_manager();
        let lock1 = mgr.acquire(LockResource::RegistryUpdate, "first").unwrap();
        let result = mgr.acquire_with_timeout(
            LockResource::RegistryUpdate,
            "second",
            Duration::from_millis(100),
            None,
        );
        assert!(result.is_err(), "same-process contention should time out");
        assert_eq!(result.unwrap_err().code(), "ERR_LOCK_TIMEOUT");
        drop(lock1);
        cleanup(&dir);
    }

    #[test]
    fn test_serial_acquire_after_release() {
        let (mgr, dir) = test_manager();
        let lock1 = mgr.acquire(LockResource::Materialization, "first").unwrap();
        drop(lock1);
        let lock2 = mgr.acquire(LockResource::Materialization, "second");
        assert!(lock2.is_ok(), "should re-acquire after release");
        drop(lock2);
        cleanup(&dir);
    }

    #[test]
    fn test_guard_does_not_delete_replaced_lock() {
        // Simulate: acquire, write different metadata (nonce mismatch),
        // then drop — the guard must NOT remove the file.
        let (mgr, dir) = test_manager();
        let lock_path;
        {
            let lock = mgr
                .acquire(LockResource::LoaderInstall, "original")
                .unwrap();
            lock_path = lock.path().to_path_buf();
            // Overwrite lock metadata with a different nonce (simulates replacement).
            let fake = LockMetadata {
                pid: 99999,
                pid_start_time: 0,
                hostname: "replacer".into(),
                acquired_at: "2026-01-01T00:00:00Z".into(),
                operation: "replaced".into(),
                nonce: lock.nonce.wrapping_add(1u128), // different nonce
            };
            let json = serde_json::to_string(&fake).unwrap();
            std::fs::write(&lock_path, json).unwrap();
            // When `lock` drops here, nonce won't match, so it should NOT
            // delete the file.
        }
        assert!(lock_path.exists(), "guard must not delete replaced lock");
        let _ = std::fs::remove_file(&lock_path);
        cleanup(&dir);
    }

    #[test]
    fn test_guard_release_on_drop() {
        let (mgr, dir) = test_manager();
        let lock_path;
        {
            let lock = mgr
                .acquire(LockResource::LoaderInstall, "drop-test")
                .unwrap();
            lock_path = lock.path().to_path_buf();
            assert!(lock_path.exists(), "lock file should exist");
        }
        assert!(!lock_path.exists(), "lock file should be removed on drop");
        cleanup(&dir);
    }

    #[test]
    fn test_different_lock_names_do_not_conflict() {
        let (mgr, dir) = test_manager();
        let lock1 = mgr.acquire(LockResource::RegistryUpdate, "reg");
        let lock2 = mgr.acquire(LockResource::LoaderInstall, "loader");
        assert!(lock1.is_ok());
        assert!(lock2.is_ok(), "different locks should not conflict");
        drop(lock2);
        drop(lock1);
        cleanup(&dir);
    }

    #[test]
    fn test_java_major_locks_are_independent() {
        let (mgr, dir) = test_manager();
        let lock17 = mgr.acquire(LockResource::JavaMajor(17), "java17");
        let lock21 = mgr.acquire(LockResource::JavaMajor(21), "java21");
        assert!(lock17.is_ok());
        assert!(
            lock21.is_ok(),
            "different Java major locks should be independent"
        );
        drop(lock21);
        drop(lock17);
        cleanup(&dir);
    }

    #[test]
    fn test_instance_lock_contention() {
        let (mgr, dir) = test_manager();
        let lock = mgr
            .acquire(LockResource::Instance("my-instance".into()), "install")
            .unwrap();
        let lock2 = mgr.acquire_with_timeout(
            LockResource::Instance("my-instance".into()),
            "second",
            Duration::from_millis(100),
            None,
        );
        assert!(lock2.is_err(), "same instance lock should be contested");
        drop(lock);
        cleanup(&dir);
    }

    #[test]
    fn test_cancellation() {
        let (mgr, dir) = test_manager();
        let cancel = CancellationToken::new();

        // Acquire a lock to create contention.
        let _lock1 = mgr.acquire(LockResource::RegistryUpdate, "holder").unwrap();

        // Cancel the token, then try to acquire with short timeout + cancellation.
        cancel.cancel();
        let result = mgr.acquire_with_timeout(
            LockResource::RegistryUpdate,
            "cancelled",
            Duration::from_secs(30), // long timeout, but cancelled
            Some(&cancel),
        );
        assert!(result.is_err(), "cancelled acquisition should fail");
        assert_eq!(result.unwrap_err().code(), "ERR_LOCK_CANCELLED");

        drop(_lock1);
        cleanup(&dir);
    }

    #[test]
    fn test_lock_metadata_serialization() {
        use std::str::FromStr;
        let meta = LockMetadata {
            pid: 12345,
            pid_start_time: 1000000,
            hostname: "test-host".into(),
            acquired_at: "2026-01-01T00:00:00Z".into(),
            operation: "test".into(),
            nonce: uuid::Uuid::from_str("550e8400-e29b-41d4-a716-446655440000")
                .unwrap()
                .as_u128(),
        };
        let json = serde_json::to_string(&meta).unwrap();
        let back: LockMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(back.pid, 12345);
        assert_eq!(back.pid_start_time, 1000000);
        assert_eq!(back.hostname, "test-host");
    }

    #[test]
    fn test_lock_name_rejects_empty_instance_id() {
        let r = LockResource::Instance("".into());
        assert!(r.lock_name().is_err());
    }

    #[test]
    fn test_lock_name_rejects_traversal_instance_id() {
        let r = LockResource::Instance("../evil".into());
        assert!(r.lock_name().is_err());
    }

    #[test]
    fn test_lock_name_rejects_separator_instance_id() {
        let r = LockResource::Instance("a/b".into());
        assert!(r.lock_name().is_err());
    }

    #[test]
    fn test_lock_name_rejects_zero_java_major() {
        let r = LockResource::JavaMajor(0);
        assert!(r.lock_name().is_err());
    }

    #[test]
    fn test_lock_name_accepts_valid_java_major() {
        let r = LockResource::JavaMajor(17);
        assert_eq!(r.lock_name().unwrap(), "java-major-17");
    }

    #[test]
    fn test_lock_name_accepts_valid_instance() {
        let r = LockResource::Instance("my-instance".into());
        assert_eq!(r.lock_name().unwrap(), "instance-my-instance");
    }

    #[test]
    fn test_lock_name_accepts_content_store() {
        let r = LockResource::ContentStore;
        assert_eq!(r.lock_name().unwrap(), "content-store");
    }

    #[test]
    fn test_lock_name_accepts_game_user_files() {
        let r = LockResource::GameUserFiles(
            GameId::new("skyrim-se").unwrap(),
            StoreId::new("steam").unwrap(),
        );
        assert_eq!(r.lock_name().unwrap(), "game-user-files-skyrim-se-steam");
    }

    #[test]
    fn test_unique_nonce() {
        // Acquire two locks in different directories — their nonces should differ.
        let (mgr1, dir1) = test_manager();
        let (mgr2, dir2) = test_manager();
        let lock1 = mgr1.acquire(LockResource::RegistryUpdate, "test1").unwrap();
        let lock2 = mgr2.acquire(LockResource::RegistryUpdate, "test2").unwrap();
        assert_ne!(lock1.nonce, lock2.nonce, "nonces should be unique");
        drop(lock2);
        drop(lock1);
        cleanup(&dir2);
        cleanup(&dir1);
    }

    #[test]
    fn test_corrupt_lock_is_handled_safely() {
        let (mgr, dir) = test_manager();
        // Write a partial/corrupt lock file.
        let lock_path = dir.join("registry-update.lock");
        std::fs::write(&lock_path, b"not-json").unwrap();

        // Unknown ownership must not be broken after the grace period.
        let result = mgr.acquire_with_timeout(
            LockResource::RegistryUpdate,
            "test",
            Duration::from_secs(5),
            None,
        );
        assert!(
            result.is_err(),
            "unknown corrupt ownership must fail closed"
        );
        assert_eq!(result.unwrap_err().code(), "ERR_LOCK_CORRUPT");
        assert!(lock_path.exists(), "unknown corrupt lock must be preserved");
        cleanup(&dir);
    }

    #[test]
    fn test_dead_partial_lock_is_recovered() {
        let (mgr, dir) = test_manager();
        let lock_path = dir.join("registry-update.lock");
        std::fs::write(&lock_path, br#"{"pid":4294967294"#).unwrap();

        let result = mgr.acquire_with_timeout(
            LockResource::RegistryUpdate,
            "test",
            Duration::from_secs(5),
            None,
        );
        assert!(
            result.is_ok(),
            "dead partial lock should be recoverable: {:?}",
            result.err()
        );
        cleanup(&dir);
    }

    // -----------------------------------------------------------------------
    // Cross-process contention tests: independent LockManager instances
    // test that the filesystem-based locking works across logical "processes."
    // -----------------------------------------------------------------------

    #[test]
    fn test_independent_managers_contend_on_registry() {
        let dir = std::env::temp_dir().join(format!("agora-lock-cross-reg-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);

        let mgr1 = LockManager::new(dir.clone());
        let mgr2 = LockManager::new(dir.clone());

        let _lock1 = mgr1
            .acquire(LockResource::RegistryUpdate, "process-a")
            .expect("mgr1 should acquire RegistryUpdate");
        let result = mgr2.acquire_with_timeout(
            LockResource::RegistryUpdate,
            "process-b",
            Duration::from_millis(100),
            None,
        );
        assert!(
            result.is_err(),
            "independent managers must contend on RegistryUpdate"
        );
        assert_eq!(result.unwrap_err().code(), "ERR_LOCK_TIMEOUT");

        drop(_lock1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_independent_managers_contend_on_materialization() {
        let dir = std::env::temp_dir().join(format!("agora-lock-cross-mat-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);

        let mgr1 = LockManager::new(dir.clone());
        let mgr2 = LockManager::new(dir.clone());

        let _lock1 = mgr1
            .acquire(LockResource::Materialization, "process-a")
            .expect("mgr1 should acquire Materialization");
        let result = mgr2.acquire_with_timeout(
            LockResource::Materialization,
            "process-b",
            Duration::from_millis(100),
            None,
        );
        assert!(
            result.is_err(),
            "independent managers must contend on Materialization"
        );
        assert_eq!(result.unwrap_err().code(), "ERR_LOCK_TIMEOUT");

        drop(_lock1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_independent_managers_different_resources_no_conflict() {
        let dir =
            std::env::temp_dir().join(format!("agora-lock-cross-diff-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);

        let mgr1 = LockManager::new(dir.clone());
        let mgr2 = LockManager::new(dir.clone());

        let _lock_reg = mgr1
            .acquire(LockResource::RegistryUpdate, "process-a-reg")
            .expect("mgr1 should acquire RegistryUpdate");
        let _lock_mat = mgr2
            .acquire(LockResource::Materialization, "process-b-mat")
            .expect("mgr2 should acquire Materialization (different resource)");

        // Both held simultaneously without conflict.
        drop(_lock_reg);
        drop(_lock_mat);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_lock_released_on_drop_after_scope() {
        let (mgr, dir) = test_manager();

        // Scope simulating a service operation that acquires the lock then
        // exits early (simulating a failure mid-transaction).
        {
            let _guard = mgr
                .acquire(LockResource::RegistryUpdate, "test-op")
                .unwrap();
            // Guard drops here as scope exits — lock file removed.
        }
        assert!(
            !dir.join("registry-update.lock").exists(),
            "lock file must be removed after guard drops"
        );

        // Re-acquire succeeds after the simulated failure.
        let reacquire = mgr.acquire_with_timeout(
            LockResource::RegistryUpdate,
            "retry",
            Duration::from_millis(100),
            None,
        );
        assert!(
            reacquire.is_ok(),
            "should reacquire after failure-scope release"
        );

        cleanup(&dir);
    }
}
