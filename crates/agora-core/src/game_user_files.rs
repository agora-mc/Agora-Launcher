//! Per-user game files managed by journaled swap (MASTER_SPEC §26.5).
//!
//! Skyrim and Creation Engine games keep plugin lists and configuration INIs
//! in the user's profile (`LocalAppData`, `Documents`), one copy per store,
//! shared by every instance of that game.
//!
//! Each instance needs its own copies, swapped in for the launch session and
//! put back afterwards, without losing user files.
//!
//! All session state is journaled on disk at `<data>/user-files/<game>_<store>/journal.json`
//! so interruptions, crashes, or ungraceful terminations can be safely recovered.

use std::path::{Path, PathBuf};

use agora_game_api::{
    GameDefinition, GameId, GamePath, RelPath, StoreId, UserDataLocation, UserFileMapping,
    UserFileStrategy,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ctx::Ctx;
use crate::lock_manager::LockResource;
use crate::process_identity::{self, ProcessIdentity};

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum UserFilesError {
    #[error("{0}")]
    SessionRunning(String),
    #[error("cannot restore while session is running: instance '{instance}' is active")]
    CannotRestoreRunning { instance: String },
    #[error("no session found for game '{game}' ({store})")]
    NoSession { game: GameId, store: StoreId },
    #[error("user data location '{0:?}' could not be resolved")]
    UserDataNotFound(UserDataLocation),
    #[error("hash mismatch on file '{path}': expected {expected}, got {actual}")]
    HashMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    #[error("lock error: {0}")]
    Lock(#[from] crate::error::LauncherError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Other(String),
}

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// The record of a user file session on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Journal {
    pub game: GameId,
    pub store: StoreId,
    pub instance_id: String,
    pub created_at: String,
    pub running_from: PathBuf,
    pub processes: Vec<ProcessIdentity>,
    pub files: Vec<JournaledFile>,
}

/// A specific file tracked in the session journal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournaledFile {
    pub real_path: PathBuf,
    pub instance_path: RelPath,
    pub instance_copy_path: PathBuf,
    pub existed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_sha256: Option<String>,
    pub written_sha256: String,
    #[serde(default)]
    pub swapped: bool,
}

/// Status of an active or recoverable user-file session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStatus {
    pub instance: String,
    pub running: bool,
    pub files: Vec<JournaledFile>,
}

/// Status of a file restored from a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoredFile {
    pub real_path: PathBuf,
    pub instance_path: RelPath,
    pub changed: bool,
    pub original_sha256: Option<String>,
    pub final_sha256: Option<String>,
}

/// Summary report after restoring a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreReport {
    pub game: GameId,
    pub store: StoreId,
    pub instance_id: String,
    pub files: Vec<RestoredFile>,
}

impl RestoreReport {
    pub fn changed_files(&self) -> Vec<&RestoredFile> {
        self.files.iter().filter(|f| f.changed).collect()
    }
}

/// Outcome of a swap-in operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapOutcome {
    pub swapped_files: usize,
}

// ---------------------------------------------------------------------------
// UserData resolution
// ---------------------------------------------------------------------------

/// Resolving `UserData` goes through this single function core owns.
/// An environment variable `AGORA_TEST_USER_DATA_ROOT=<dir>` redirects
/// Documents, LocalAppData, RoamingAppData and Home for hermetic tests.
pub fn user_data_root(location: &UserDataLocation) -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("AGORA_TEST_USER_DATA_ROOT") {
        if !dir.is_empty() {
            let base = PathBuf::from(dir);
            return Some(match location {
                UserDataLocation::Documents => base.join("documents"),
                UserDataLocation::LocalAppData => base.join("local"),
                UserDataLocation::RoamingAppData => base.join("roaming"),
                UserDataLocation::Home => base.join("home"),
            });
        }
    }
    match location {
        UserDataLocation::Documents => dirs::document_dir(),
        UserDataLocation::RoamingAppData => dirs::data_dir(),
        UserDataLocation::LocalAppData => dirs::data_local_dir(),
        UserDataLocation::Home => dirs::home_dir(),
    }
}

/// Resolve a relative path component-by-component, matching case-insensitively
/// when an entry already exists on disk (e.g. matching `plugins.txt` to `Plugins.txt`).
pub fn resolve_rel_case_insensitive(base: &Path, rel: &str) -> PathBuf {
    let mut current = base.to_path_buf();
    for part in rel.split('/') {
        if part.is_empty() {
            continue;
        }
        let mut matched = false;
        if let Ok(entries) = std::fs::read_dir(&current) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                if name.to_string_lossy().eq_ignore_ascii_case(part) {
                    current.push(name);
                    matched = true;
                    break;
                }
            }
        }
        if !matched {
            current.push(part);
        }
    }
    current
}

/// Compare two paths component-by-component case-insensitively.
pub fn paths_eq_case_insensitive(a: &Path, b: &Path) -> bool {
    let a_comps: Vec<_> = a
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
        .collect();
    let b_comps: Vec<_> = b
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
        .collect();
    a_comps == b_comps
}

/// Resolve a `GamePath` targeting `UserData` to its absolute host path.
pub fn resolve_user_file_source(source: &GamePath) -> Result<PathBuf, UserFilesError> {
    match source {
        GamePath::UserData { location, path } => {
            let Some(base) = user_data_root(location) else {
                return Err(UserFilesError::UserDataNotFound(location.clone()));
            };
            let rel = path.as_str();
            if rel.is_empty() {
                Ok(base)
            } else {
                Ok(resolve_rel_case_insensitive(&base, rel))
            }
        }
        other => Err(UserFilesError::Other(format!(
            "unsupported user file source root: {other:?}"
        ))),
    }
}

fn hash_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn write_journal(journal_path: &Path, journal: &Journal) -> Result<(), UserFilesError> {
    if let Some(parent) = journal_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_string_pretty(journal)?;
    let tmp_path = journal_path.with_extension("tmp");
    std::fs::write(&tmp_path, data.as_bytes())?;
    std::fs::rename(&tmp_path, journal_path)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Running session detection
// ---------------------------------------------------------------------------

/// Check whether a journal's session is still running.
/// A session is running if any recorded process identity is still alive
/// (`process_identity::verify`) or any process runs from the recorded folder
/// (`processes_running_from`).
pub fn is_session_running(journal: &Journal) -> bool {
    for proc in &journal.processes {
        if process_identity::verify(proc).is_ok() {
            return true;
        }
    }
    if journal.running_from.exists() {
        let procs = crate::game_launch::processes_running_from(&journal.running_from);
        if !procs.is_empty() {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Core Operations: swap_in, restore, status
// ---------------------------------------------------------------------------

/// Swap in per-user files for an instance launch session.
pub fn swap_in(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    store: &StoreId,
    running_from: &Path,
) -> Result<SwapOutcome, UserFilesError> {
    let _lock = ctx.lock_manager.acquire(
        LockResource::GameUserFiles(definition.id.clone(), store.clone()),
        "swap-in",
    )?;

    let journal_path = ctx
        .paths
        .user_files_journal_path(definition.id.as_str(), store.as_str());

    // 1. If a journal exists, check if running.
    if journal_path.exists() {
        let text = std::fs::read_to_string(&journal_path)?;
        let existing: Journal = serde_json::from_str(&text)?;
        if is_session_running(&existing) {
            let store_display = match store.as_str() {
                "steam" => "Steam",
                "gog" => "GOG",
                other => other,
            };
            let game_display = if definition.name.is_empty() {
                definition.id.as_str()
            } else {
                &definition.name
            };
            return Err(UserFilesError::SessionRunning(format!(
                "{} ({}) is running as instance {}; close it first",
                game_display, store_display, existing.instance_id
            )));
        }
        // Not running: recover/restore the previous session before proceeding.
        let _ = restore_journal_internal(ctx, definition, &existing)?;
    }

    // 2. Filter mappings that apply to this store
    let applicable_mappings: Vec<&UserFileMapping> = definition
        .user_files
        .iter()
        .filter(|m| m.strategy == UserFileStrategy::JournaledSwap && m.applies_to_store(store))
        .collect();

    if applicable_mappings.is_empty() {
        return Ok(SwapOutcome { swapped_files: 0 });
    }

    let instance_dir = ctx
        .paths
        .instance_dir(instance_id)
        .map_err(|e| UserFilesError::Other(e.to_string()))?;
    let backup_dir = ctx
        .paths
        .user_files_backup_dir(definition.id.as_str(), store.as_str());
    std::fs::create_dir_all(&backup_dir)?;

    // 3. Build initial journal record before touching any real file
    let mut journal_files = Vec::new();
    for m in &applicable_mappings {
        let real_path = resolve_user_file_source(&m.source)?;
        let instance_copy_path = instance_dir.join(m.instance_path.as_str());
        journal_files.push(JournaledFile {
            real_path,
            instance_path: m.instance_path.clone(),
            instance_copy_path,
            existed: false,
            backup_path: None,
            original_sha256: None,
            written_sha256: String::new(),
            swapped: false,
        });
    }

    let mut journal = Journal {
        game: definition.id.clone(),
        store: store.clone(),
        instance_id: instance_id.to_string(),
        created_at: chrono::DateTime::<chrono::Utc>::from(ctx.clock.now()).to_rfc3339(),
        running_from: running_from.to_path_buf(),
        processes: Vec::new(),
        files: journal_files,
    };

    write_journal(&journal_path, &journal)?;

    // 4. Perform the swap file-by-file, updating the journal after each file.
    let swap_result: Result<(), UserFilesError> = (|| {
        for i in 0..journal.files.len() {
            let real_path = journal.files[i].real_path.clone();
            let instance_copy_path = journal.files[i].instance_copy_path.clone();
            let backup_path = backup_dir.join(format!("{i}"));

            if real_path.exists() {
                let original_bytes = std::fs::read(&real_path)?;
                let orig_hash = hash_bytes(&original_bytes);

                // Back up real file and verify backup hash
                std::fs::copy(&real_path, &backup_path)?;
                let backup_bytes = std::fs::read(&backup_path)?;
                let b_hash = hash_bytes(&backup_bytes);
                if b_hash != orig_hash {
                    return Err(UserFilesError::HashMismatch {
                        path: backup_path,
                        expected: orig_hash,
                        actual: b_hash,
                    });
                }

                let written_hash;
                if instance_copy_path.exists() {
                    let instance_bytes = std::fs::read(&instance_copy_path)?;
                    written_hash = hash_bytes(&instance_bytes);
                    std::fs::write(&real_path, &instance_bytes)?;
                } else {
                    // First launch keeps the user's settings: write nothing and
                    // record original hash as what Agora "wrote".
                    written_hash = orig_hash.clone();
                }

                journal.files[i].existed = true;
                journal.files[i].backup_path = Some(backup_path);
                journal.files[i].original_sha256 = Some(orig_hash);
                journal.files[i].written_sha256 = written_hash;
                journal.files[i].swapped = true;
            } else {
                let written_hash;
                if instance_copy_path.exists() {
                    let instance_bytes = std::fs::read(&instance_copy_path)?;
                    written_hash = hash_bytes(&instance_bytes);
                    if let Some(parent) = real_path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::write(&real_path, &instance_bytes)?;
                } else {
                    written_hash = String::new();
                }

                journal.files[i].existed = false;
                journal.files[i].backup_path = None;
                journal.files[i].original_sha256 = None;
                journal.files[i].written_sha256 = written_hash;
                journal.files[i].swapped = true;
            }

            write_journal(&journal_path, &journal)?;
        }
        Ok(())
    })();

    if let Err(e) = swap_result {
        // Any error restores what was swapped so far and returns the error.
        let _ = restore_journal_internal(ctx, definition, &journal);
        return Err(e);
    }

    Ok(SwapOutcome {
        swapped_files: journal.files.len(),
    })
}

/// Restore user files after session ends, at next launch, or by command.
pub fn restore(
    ctx: &Ctx,
    definition: &GameDefinition,
    store: &StoreId,
) -> Result<RestoreReport, UserFilesError> {
    let _lock = ctx.lock_manager.acquire(
        LockResource::GameUserFiles(definition.id.clone(), store.clone()),
        "restore",
    )?;

    let journal_path = ctx
        .paths
        .user_files_journal_path(definition.id.as_str(), store.as_str());
    if !journal_path.exists() {
        return Err(UserFilesError::NoSession {
            game: definition.id.clone(),
            store: store.clone(),
        });
    }

    let text = std::fs::read_to_string(&journal_path)?;
    let journal: Journal = serde_json::from_str(&text)?;

    if is_session_running(&journal) {
        return Err(UserFilesError::CannotRestoreRunning {
            instance: journal.instance_id.clone(),
        });
    }

    restore_journal_internal(ctx, definition, &journal)
}

fn restore_journal_internal(
    ctx: &Ctx,
    definition: &GameDefinition,
    journal: &Journal,
) -> Result<RestoreReport, UserFilesError> {
    if journal.game != definition.id {
        return Err(UserFilesError::Other(format!(
            "journal game '{}' does not match definition '{}'",
            journal.game, definition.id
        )));
    }

    let session_dir = ctx
        .paths
        .user_files_dir(definition.id.as_str(), journal.store.as_str());
    let journal_path = ctx
        .paths
        .user_files_journal_path(definition.id.as_str(), journal.store.as_str());
    let backup_dir = ctx
        .paths
        .user_files_backup_dir(definition.id.as_str(), journal.store.as_str());

    // 1. Validate instance_id the way AppPaths does
    let instance_dir = ctx
        .paths
        .instance_dir(&journal.instance_id)
        .map_err(|e| UserFilesError::Other(e.to_string()))?;

    // 2. Applicable mappings for this store
    let applicable_mappings: Vec<&UserFileMapping> = definition
        .user_files
        .iter()
        .filter(|m| {
            m.strategy == UserFileStrategy::JournaledSwap && m.applies_to_store(&journal.store)
        })
        .collect();

    // 3. Pre-validate all journal entries BEFORE modifying or deleting any file on disk.
    // "A journal entry whose instance_path matches no mapping for that store,
    // or whose recorded real_path differs from the derived one (case-insensitive),
    // makes restore refuse, keep the journal and change nothing."
    struct ValidatedEntry<'a> {
        file: &'a JournaledFile,
        derived_real_path: PathBuf,
        derived_instance_copy: PathBuf,
        derived_backup_path: PathBuf,
        mapping: &'a UserFileMapping,
    }

    let mut validated_entries = Vec::with_capacity(journal.files.len());

    for (i, file) in journal.files.iter().enumerate() {
        let mapping = applicable_mappings
            .iter()
            .find(|m| m.instance_path == file.instance_path);
        let Some(mapping) = mapping else {
            return Err(UserFilesError::Other(format!(
                "journal entry instance_path '{}' matches no mapping for store '{}'",
                file.instance_path.as_str(),
                journal.store
            )));
        };

        let derived_real_path = resolve_user_file_source(&mapping.source)?;
        if !paths_eq_case_insensitive(&file.real_path, &derived_real_path) {
            return Err(UserFilesError::Other(format!(
                "journal entry real_path '{:?}' differs from derived real_path '{:?}'",
                file.real_path, derived_real_path
            )));
        }

        let derived_instance_copy = instance_dir.join(mapping.instance_path.as_str());
        let derived_backup_path = backup_dir.join(format!("{i}"));

        if file.swapped {
            if file.existed {
                let Some(ref bp) = file.backup_path else {
                    return Err(UserFilesError::Other(format!(
                        "missing backup path in journal for existed file: {:?}",
                        file.real_path
                    )));
                };
                if !paths_eq_case_insensitive(bp, &derived_backup_path) {
                    return Err(UserFilesError::Other(format!(
                        "journal backup path '{:?}' does not match expected backup index path '{:?}'",
                        bp, derived_backup_path
                    )));
                }
                if !derived_backup_path.exists() {
                    return Err(UserFilesError::Other(format!(
                        "backup file does not exist: {:?}",
                        derived_backup_path
                    )));
                }
            } else if file.backup_path.is_some() {
                return Err(UserFilesError::Other(format!(
                    "journal has backup path for non-existed file: {:?}",
                    file.real_path
                )));
            }
        } else if let Some(ref bp) = file.backup_path {
            if !paths_eq_case_insensitive(bp, &derived_backup_path) {
                return Err(UserFilesError::Other(format!(
                    "journal backup path '{:?}' does not match expected backup index path '{:?}'",
                    bp, derived_backup_path
                )));
            }
        }

        validated_entries.push(ValidatedEntry {
            file,
            derived_real_path,
            derived_instance_copy,
            derived_backup_path,
            mapping,
        });
    }

    // 4. Perform restore using derived paths
    let mut restored_files = Vec::new();

    for entry in &validated_entries {
        if !entry.file.swapped {
            continue;
        }

        let changed = if entry.derived_real_path.exists() {
            let current_bytes = std::fs::read(&entry.derived_real_path)?;
            let current_hash = hash_bytes(&current_bytes);
            let ch = current_hash != entry.file.written_sha256;

            // Current file contents always go to the instance copy
            if let Some(parent) = entry.derived_instance_copy.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&entry.derived_instance_copy, &current_bytes)?;
            ch
        } else {
            !entry.file.written_sha256.is_empty()
        };

        // Put the original backup back, or delete real file if it did not exist before
        if entry.file.existed {
            let backup_bytes = std::fs::read(&entry.derived_backup_path)?;
            let b_hash = hash_bytes(&backup_bytes);
            if let Some(ref orig_hash) = entry.file.original_sha256 {
                if &b_hash != orig_hash {
                    return Err(UserFilesError::HashMismatch {
                        path: entry.derived_backup_path.clone(),
                        expected: orig_hash.clone(),
                        actual: b_hash,
                    });
                }
            }
            if let Some(parent) = entry.derived_real_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&entry.derived_real_path, &backup_bytes)?;
        } else if entry.derived_real_path.exists() {
            std::fs::remove_file(&entry.derived_real_path)?;
        }

        let final_sha256 = if entry.derived_real_path.exists() {
            let bytes = std::fs::read(&entry.derived_real_path)?;
            Some(hash_bytes(&bytes))
        } else {
            None
        };

        restored_files.push(RestoredFile {
            real_path: entry.derived_real_path.clone(),
            instance_path: entry.mapping.instance_path.clone(),
            changed,
            original_sha256: entry.file.original_sha256.clone(),
            final_sha256,
        });
    }

    // Delete the journal and backups only after every file is restored.
    if backup_dir.exists() {
        let _ = std::fs::remove_dir_all(&backup_dir);
    }
    if journal_path.exists() {
        let _ = std::fs::remove_file(&journal_path);
    }
    if session_dir.exists() {
        let _ = std::fs::remove_dir(&session_dir);
    }

    Ok(RestoreReport {
        game: journal.game.clone(),
        store: journal.store.clone(),
        instance_id: journal.instance_id.clone(),
        files: restored_files,
    })
}

/// Record a launched process identity into the session journal.
pub fn record_process(
    ctx: &Ctx,
    game: &GameId,
    store: &StoreId,
    identity: ProcessIdentity,
) -> Result<(), UserFilesError> {
    let _lock = ctx.lock_manager.acquire(
        LockResource::GameUserFiles(game.clone(), store.clone()),
        "record-process",
    )?;

    let journal_path = ctx
        .paths
        .user_files_journal_path(game.as_str(), store.as_str());
    if !journal_path.exists() {
        return Ok(());
    }

    let text = std::fs::read_to_string(&journal_path)?;
    let mut journal: Journal = serde_json::from_str(&text)?;
    journal.processes.push(identity);
    write_journal(&journal_path, &journal)?;
    Ok(())
}

/// Inspect the status of a user-file session for a game and store.
pub fn status(ctx: &Ctx, game: &GameId, store: &StoreId) -> Option<SessionStatus> {
    let journal_path = ctx
        .paths
        .user_files_journal_path(game.as_str(), store.as_str());
    if !journal_path.exists() {
        return None;
    }
    let text = std::fs::read_to_string(&journal_path).ok()?;
    let journal: Journal = serde_json::from_str(&text).ok()?;
    let running = is_session_running(&journal);
    Some(SessionStatus {
        instance: journal.instance_id,
        running,
        files: journal.files,
    })
}

/// List active/recoverable user-file sessions across all games, optionally filtered.
pub fn list_statuses(
    ctx: &Ctx,
    filter_game: Option<&GameId>,
) -> Result<Vec<(GameId, StoreId, SessionStatus)>, UserFilesError> {
    let root = ctx.paths.user_files_root();
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut results = Vec::new();
    let entries = std::fs::read_dir(&root)?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let journal_path = path.join("journal.json");
        if !journal_path.exists() {
            continue;
        }
        // An unreadable journal is a session nobody can see the state of: report it rather
        // than listing "no sessions", which would invite launching over it.
        let journal = std::fs::read_to_string(&journal_path)
            .map_err(UserFilesError::from)
            .and_then(|text| serde_json::from_str::<Journal>(&text).map_err(UserFilesError::from))
            .map_err(|e| {
                UserFilesError::Other(format!(
                    "the user-files journal {} cannot be read: {e}",
                    journal_path.display()
                ))
            })?;
        if let Some(filter) = filter_game {
            if &journal.game != filter {
                continue;
            }
        }
        let running = is_session_running(&journal);
        results.push((
            journal.game,
            journal.store,
            SessionStatus {
                instance: journal.instance_id,
                running,
                files: journal.files,
            },
        ));
    }

    results.sort_by(|a, b| (a.0.as_str(), a.1.as_str()).cmp(&(b.0.as_str(), b.1.as_str())));
    Ok(results)
}
