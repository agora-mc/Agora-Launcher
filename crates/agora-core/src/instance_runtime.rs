//! Durable record of whether a game is currently using an instance.
//!
//! # Why this is not just the instance lock
//!
//! [`LockResource::Instance`](crate::lock_manager::LockResource::Instance) is a
//! short-lived mutex held by whichever process is *mutating* an instance. It
//! cannot express "a game is running", because the game outlives the critical
//! section — and, on the delegated launch path, can outlive the launcher
//! itself. A lock owned by the launcher gets released (or correctly detected as
//! stale, its owner being genuinely dead) while Minecraft is still writing to
//! `saves/`, at which point another process is free to restore a snapshot out
//! from under a live game.
//!
//! So ownership has to track the *game*, not the launcher. The lease records
//! the game's OS-level [`ProcessIdentity`] — pid plus start time plus
//! executable — which
//! [`process_identity::verify`](crate::process_identity::verify) checks
//! fail-closed against pid reuse.
//!
//! # The starting window
//!
//! Between spawning the game and learning its identity there is a window where
//! a crash leaves a `Starting` lease with no identity to verify. That state is
//! deliberately **not** self-clearing: a launcher that died proves nothing
//! about whether a game exists, and neither does a timeout. It blocks mutation
//! until [`clear`] is called explicitly, which is a decision for the user to
//! make once they can see no game is running.
//!
//! Lease files live under the locks root, never inside the instance directory —
//! coordination state must not be something a restore can roll back.

use crate::app_paths::AppPaths;
use crate::error::{LauncherError, LauncherResult};
use crate::process_identity::{self, ProcessIdentity};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const RUNTIME_LEASE_SCHEMA_VERSION: u32 = 1;

/// How far along a launch is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeState {
    /// Spawned, or about to be, but the game's identity is not yet recorded.
    Starting,
    /// Running, with a verifiable process identity.
    Running,
}

/// The durable "this instance is in use" record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeLease {
    pub schema_version: u32,
    pub instance_id: String,
    /// Identifies this particular launch. Only the holder of a matching
    /// generation may clear the lease, so a late exit handler from a previous
    /// launch cannot release the current one.
    pub generation: String,
    pub state: RuntimeState,
    /// Present once the game has been identified.
    pub identity: Option<ProcessIdentity>,
    pub launcher_pid: u32,
    pub recorded_at: String,
}

fn lease_path(paths: &AppPaths, instance_id: &str) -> LauncherResult<PathBuf> {
    // Reuses the lock path validation, then swaps the extension, so an
    // instance id that is unsafe as a lock name is unsafe here too.
    let lock = paths.instance_lock(instance_id)?;
    Ok(lock.with_extension("runtime.json"))
}

fn write_lease(paths: &AppPaths, lease: &RuntimeLease) -> LauncherResult<()> {
    let path = lease_path(paths, &lease.instance_id)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| LauncherError::Generic {
            code: "ERR_RUNTIME_LEASE".into(),
            message: format!("Could not create the lock directory: {e}"),
        })?;
    }
    let bytes = serde_json::to_vec_pretty(lease).map_err(|e| LauncherError::Generic {
        code: "ERR_RUNTIME_LEASE".into(),
        message: format!("Could not serialize the runtime lease: {e}"),
    })?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, &bytes).map_err(|e| LauncherError::Generic {
        code: "ERR_RUNTIME_LEASE".into(),
        message: format!("Could not write the runtime lease: {e}"),
    })?;
    fs::rename(&tmp, &path).map_err(|e| LauncherError::Generic {
        code: "ERR_RUNTIME_LEASE".into(),
        message: format!("Could not publish the runtime lease: {e}"),
    })
}

/// Read the current lease, if any. A corrupt lease is treated as present and
/// unreadable rather than absent — see [`check_idle`].
pub fn read(paths: &AppPaths, instance_id: &str) -> LauncherResult<Option<RuntimeLease>> {
    let path = lease_path(paths, instance_id)?;
    let Ok(bytes) = fs::read(&path) else {
        return Ok(None);
    };
    match serde_json::from_slice::<RuntimeLease>(&bytes) {
        Ok(lease) if lease.schema_version == RUNTIME_LEASE_SCHEMA_VERSION => Ok(Some(lease)),
        // Unreadable or from a future version: report it as occupied. Deleting
        // it would be assuming the instance is idle on the strength of a file
        // we could not parse.
        _ => Err(LauncherError::Generic {
            code: "ERR_RUNTIME_LEASE_UNREADABLE".into(),
            message: format!(
                "The runtime record for '{instance_id}' at {} cannot be read, so it is not \
                 possible to tell whether a game is using it. Close any running game and remove \
                 that file to continue.",
                path.display()
            ),
        }),
    }
}

/// Publish a `Starting` lease and return its generation.
///
/// Call this *before* spawning, while holding the instance lock.
pub fn publish_starting(paths: &AppPaths, instance_id: &str) -> LauncherResult<String> {
    let generation = uuid::Uuid::new_v4().to_string();
    write_lease(
        paths,
        &RuntimeLease {
            schema_version: RUNTIME_LEASE_SCHEMA_VERSION,
            instance_id: instance_id.to_string(),
            generation: generation.clone(),
            state: RuntimeState::Starting,
            identity: None,
            launcher_pid: std::process::id(),
            recorded_at: chrono::Utc::now().to_rfc3339(),
        },
    )?;
    Ok(generation)
}

/// Upgrade the lease to `Running` once the game's identity is known.
pub fn publish_running(
    paths: &AppPaths,
    instance_id: &str,
    generation: &str,
    identity: ProcessIdentity,
) -> LauncherResult<()> {
    write_lease(
        paths,
        &RuntimeLease {
            schema_version: RUNTIME_LEASE_SCHEMA_VERSION,
            instance_id: instance_id.to_string(),
            generation: generation.to_string(),
            state: RuntimeState::Running,
            identity: Some(identity),
            launcher_pid: std::process::id(),
            recorded_at: chrono::Utc::now().to_rfc3339(),
        },
    )
}

/// Release the lease.
///
/// A `generation` clears only a matching lease, so a stale exit handler cannot
/// release a newer launch. `None` forces the release — the explicit escape
/// hatch for a `Starting` lease the user has confirmed is dead.
pub fn clear(paths: &AppPaths, instance_id: &str, generation: Option<&str>) -> LauncherResult<()> {
    let path = lease_path(paths, instance_id)?;
    if let Some(generation) = generation {
        match read(paths, instance_id) {
            Ok(Some(lease)) if lease.generation != generation => return Ok(()),
            // An unreadable lease is not ours to interpret, so leave it.
            Err(_) => return Ok(()),
            _ => {}
        }
    }
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(LauncherError::Generic {
            code: "ERR_RUNTIME_LEASE".into(),
            message: format!("Could not clear the runtime lease: {error}"),
        }),
    }
}

/// Establish that no game is using this instance.
///
/// Call while holding the instance lock, before any mutation.
pub fn check_idle(paths: &AppPaths, instance_id: &str) -> LauncherResult<()> {
    let Some(lease) = read(paths, instance_id)? else {
        return Ok(());
    };
    match (lease.state, &lease.identity) {
        (RuntimeState::Running, Some(identity)) => {
            if process_identity::verify(identity).is_ok() {
                return Err(LauncherError::Generic {
                    code: "ERR_INSTANCE_RUNNING".into(),
                    message: format!(
                        "Minecraft is still running for '{instance_id}' (pid {}). Close the game \
                         before changing this instance.",
                        identity.pid
                    ),
                });
            }
            // The recorded process is provably gone — pid, start time, and
            // executable no longer match — so the lease is stale.
            clear(paths, instance_id, Some(&lease.generation))?;
            Ok(())
        }
        // Recorded as running but with nothing to verify: the same
        // indeterminate case as Starting.
        (RuntimeState::Running, None) | (RuntimeState::Starting, _) => {
            Err(LauncherError::Generic {
                code: "ERR_INSTANCE_LAUNCH_INDETERMINATE".into(),
                message: format!(
                    "A launch of '{instance_id}' was recorded but never finished starting, so it \
                     is not possible to tell whether Minecraft is running. Make sure the game is \
                     closed, then clear the launch state for this instance and try again."
                ),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> (AppPaths, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = AppPaths::from_root(tmp.path().to_path_buf());
        fs::create_dir_all(paths.locks_root()).unwrap();
        (paths, tmp)
    }

    #[test]
    fn no_lease_means_idle() {
        let (paths, _tmp) = paths();
        check_idle(&paths, "demo").unwrap();
    }

    #[test]
    fn a_live_game_blocks_mutation() {
        let (paths, _tmp) = paths();
        let mut child = if cfg!(windows) {
            std::process::Command::new("cmd.exe")
                .args(["/c", "ping", "-n", "60", "127.0.0.1"])
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap()
        } else {
            std::process::Command::new("sleep")
                .arg("60")
                .spawn()
                .unwrap()
        };
        let identity = process_identity::capture(child.id()).unwrap();
        let generation = publish_starting(&paths, "demo").unwrap();
        publish_running(&paths, "demo", &generation, identity).unwrap();

        let error = check_idle(&paths, "demo").unwrap_err();
        assert!(format!("{error:?}").contains("ERR_INSTANCE_RUNNING"));

        let _ = child.kill();
        let _ = child.wait();
    }

    /// The launcher dying does not prove the game did, so a half-published
    /// launch must not silently become "idle".
    #[test]
    fn an_unfinished_launch_is_indeterminate_not_idle() {
        let (paths, _tmp) = paths();
        publish_starting(&paths, "demo").unwrap();
        let error = check_idle(&paths, "demo").unwrap_err();
        assert!(format!("{error:?}").contains("ERR_INSTANCE_LAUNCH_INDETERMINATE"));

        // ...and it clears only when something explicitly says so.
        clear(&paths, "demo", None).unwrap();
        check_idle(&paths, "demo").unwrap();
    }

    #[test]
    fn a_stale_generation_cannot_release_a_newer_launch() {
        let (paths, _tmp) = paths();
        let old = publish_starting(&paths, "demo").unwrap();
        let new = publish_starting(&paths, "demo").unwrap();
        clear(&paths, "demo", Some(&old)).unwrap();
        assert!(
            read(&paths, "demo").unwrap().is_some(),
            "the newer lease was released by a stale generation"
        );
        clear(&paths, "demo", Some(&new)).unwrap();
        assert!(read(&paths, "demo").unwrap().is_none());
    }

    #[test]
    fn an_unreadable_lease_is_not_treated_as_idle() {
        let (paths, _tmp) = paths();
        let path = lease_path(&paths, "demo").unwrap();
        fs::write(&path, b"{ this is not a lease").unwrap();
        assert!(check_idle(&paths, "demo").is_err());
    }
}
