//! Pinned bases: the verified copies of a game that instances are built from (MASTER_SPEC §26.4).
//! Owned by the `bases` slice.
//!
//! A thin adapter over `agora_core::game_base`: the CLI's `agora games base ...` does the same
//! work. This file maps core results to DTOs and core errors to `GamesError` codes, and nothing
//! else. The desktop crate does not depend on `agora-game-api`, so the types it would name are
//! reached through `agora_core` re-exports, and test fixtures are written as the JSON a manifest
//! is stored as.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use agora_core::ctx::Ctx;
use agora_core::game_base::{
    self, BaseError, BaseManifest, BaseMode, BaseProblem, BaseVerification, BuildOptions,
    BuildOutcome, BuildProgress, ProblemKind, VerifyDepth,
};
use agora_core::game_instance::{GameInstanceRecord, InstanceError};
use agora_core::game_registry::IdentifiedInstall;
use serde::Serialize;
use tauri::Emitter;

use super::error::{GamesError, TASK_FAILED};

/// Emitted while a base is being built, about several times a second at most. The payload is
/// `BuildProgressEvent`; the frontend keeps only the events for the install it started.
pub const BUILD_PROGRESS_EVENT: &str = "games-base-progress";

/// The request was not something a base can be made from (an unknown mode, a malformed id).
pub const INVALID_INPUT: &str = "invalid_input";
/// No base with that id has a readable manifest.
pub const BASE_NOT_FOUND: &str = "base_not_found";
/// Instances still use the base, so it is kept.
pub const BASE_IN_USE: &str = "base_in_use";
/// The install is not in the current discovery, so there is nothing to build from.
pub const INSTALL_NOT_FOUND: &str = "install_not_found";
/// Agora cannot tell which exact version the install is.
pub const RUNTIME_UNKNOWN: &str = "runtime_unknown";
/// A Linked base would hardlink a file the game writes to, which would change the store install.
pub const DECLARED_WRITE_LINKED: &str = "declared_write_linked";
/// Hardlinks cannot be used for this base. Only returned where a choice is not possible.
pub const LINK_UNAVAILABLE: &str = "link_unavailable";
/// The game's definition is not loaded, so the base cannot be checked against it.
pub const GAME_DEFINITION_MISSING: &str = "game_definition_missing";
/// Reading, writing or deleting base files failed, or the answer could not be known.
pub const BASE_FAILED: &str = "base_failed";

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BasesList {
    pub bases: Vec<BaseSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseSummary {
    pub base_id: String,
    pub game_id: String,
    pub store: String,
    pub version: String,
    pub build: Option<String>,
    /// `linked` or `copied`.
    pub mode: String,
    pub location: String,
    /// False when the base folder is gone. Instances on it cannot launch until it is rebuilt.
    pub present: bool,
    pub file_count: usize,
    /// Files that are hardlinks into the store install. They take no extra disk space.
    pub linked_file_count: usize,
    pub linked_bytes: u64,
    /// Bytes this base holds itself. This is the disk the base costs.
    pub copied_bytes: u64,
    /// Source entries the build left out (symlinks, junctions, excluded files), with reasons.
    pub skipped_count: usize,
    pub created_unix_ms: i64,
    /// The instances pinned to this base. Removing the base is refused while this is not empty.
    pub used_by: Vec<BaseUser>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseUser {
    pub instance_id: String,
    pub name: String,
}

/// The bases built for one game. A base whose manifest cannot be read is not listed (core does
/// not report it), so a missing base here is not proof that none exists.
#[tauri::command]
pub async fn games_base_list(app: tauri::AppHandle, game_id: String) -> Result<BasesList, GamesError> {
    tokio::task::spawn_blocking(move || -> Result<BasesList, GamesError> {
        let ctx = crate::core_context(&app)?;
        let names = instance_names(&ctx)?;
        let mut bases = Vec::new();
        for listing in game_base::list_bases(&ctx.paths) {
            if listing.manifest.runtime.game.as_str() != game_id {
                continue;
            }
            let pinning = game_base::instances_pinning_base(&ctx.paths, &listing.manifest.base_id)
                .map_err(base_error)?;
            let used_by = named_users(&pinning, &names);
            bases.push(summarize(&listing.manifest, listing.present, used_by));
        }
        Ok(BasesList { bases })
    })
    .await
    .map_err(task_failed)?
}

/// Name every instance in the table once. A table that cannot be read is an error, not "no
/// instances", because the list would then claim a base is unused when it may be in use.
fn instance_names(ctx: &Ctx) -> Result<HashMap<String, String>, GamesError> {
    let records = agora_core::game_instance::list(ctx).map_err(instance_error)?;
    Ok(name_map(&records))
}

fn name_map(records: &[GameInstanceRecord]) -> HashMap<String, String> {
    records
        .iter()
        .map(|record| (record.instance_id.clone(), record.name.clone()))
        .collect()
}

/// The users of a base, by name. An id missing from the name table keeps its id as the name:
/// the user is still a user, so the base is still in use.
fn named_users(instance_ids: &[String], names: &HashMap<String, String>) -> Vec<BaseUser> {
    instance_ids
        .iter()
        .map(|id| BaseUser {
            instance_id: id.clone(),
            name: names.get(id).cloned().unwrap_or_else(|| id.clone()),
        })
        .collect()
}

fn summarize(manifest: &BaseManifest, present: bool, used_by: Vec<BaseUser>) -> BaseSummary {
    let (linked, copied): (Vec<_>, Vec<_>) = manifest.files.iter().partition(|file| file.linked);
    BaseSummary {
        base_id: manifest.base_id.clone(),
        game_id: manifest.runtime.game.to_string(),
        store: manifest.runtime.store.to_string(),
        version: manifest.runtime.version.clone(),
        build: manifest.runtime.build.clone(),
        mode: manifest.mode.to_string(),
        location: manifest.location.to_string_lossy().into_owned(),
        present,
        file_count: manifest.files.len(),
        linked_file_count: linked.len(),
        linked_bytes: linked.iter().map(|file| file.size).sum(),
        copied_bytes: copied.iter().map(|file| file.size).sum(),
        skipped_count: manifest.skipped.len(),
        created_unix_ms: manifest.created_unix_ms,
        used_by,
    }
}

// ---------------------------------------------------------------------------
// Build
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum BuildReply {
    /// The base was made now.
    Built {
        base_id: String,
        mode: String,
        location: String,
        file_count: usize,
        linked_bytes: u64,
        copied_bytes: u64,
    },
    /// A base for this exact version already existed, so nothing was copied.
    Existing {
        base_id: String,
        mode: String,
        location: String,
        file_count: usize,
    },
    /// Hardlinks cannot be made here (another volume, or a filesystem without them). Nothing was
    /// built. This is a choice for the user, not a failure: a Copied base is the way forward.
    LinkUnavailable {
        reason: String,
        /// The disk a Copied base of this install would take.
        copied_bytes_needed: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildProgressEvent {
    pub install_id: String,
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_hashed: u64,
    pub bytes_total: u64,
}

/// Build a base from a discovered install. `mode` is `linked` or `copied`.
#[tauri::command]
pub async fn games_base_build(
    app: tauri::AppHandle,
    install_id: String,
    mode: String,
    include_excluded: bool,
) -> Result<BuildReply, GamesError> {
    let base_mode: BaseMode = mode.parse().map_err(|_| {
        GamesError::new(
            INVALID_INPUT,
            format!("'{mode}' is not a base type. Choose Linked or Copied."),
        )
    })?;
    tokio::task::spawn_blocking(move || -> Result<BuildReply, GamesError> {
        let ctx = crate::core_context(&app)?;
        let install = find_install(&ctx, &install_id)?;
        let definition = ctx.games.game(&install.game).ok_or_else(|| {
            GamesError::new(
                GAME_DEFINITION_MISSING,
                "Agora does not have this game's definition loaded, so it cannot build a base for it.",
            )
        })?;
        let progress = progress_emitter(app.clone(), install_id.clone());
        let result = game_base::build_base(
            &ctx.paths,
            &install,
            definition,
            base_mode,
            None,
            BuildOptions { include_excluded },
            &progress,
        );
        build_reply(result)
    })
    .await
    .map_err(task_failed)?
}

/// Look the install up the same way `games list` does, so an id from the Installs section
/// always resolves to the same install the CLI would build from.
fn find_install(ctx: &Ctx, install_id: &str) -> Result<IdentifiedInstall, GamesError> {
    let report = agora_core::game_discovery::discover_all();
    let inventory = agora_core::game_registry::identify_installs(
        &ctx.games,
        &report,
        &agora_core::game_discovery::file_version::read_file_version,
    );
    inventory
        .installs
        .into_iter()
        .find(|install| install.install_id.as_str() == install_id)
        .ok_or_else(|| {
            GamesError::new(
                INSTALL_NOT_FOUND,
                format!(
                    "The install {install_id} was not found. It may have been uninstalled or moved; rescan installs and try again."
                ),
            )
        })
}

/// A progress callback that emits Tauri events. Hashing emits per file, so it is throttled to
/// one event every 250 ms, plus the last file always.
fn progress_emitter(app: tauri::AppHandle, install_id: String) -> impl Fn(BuildProgress) + Send + Sync {
    let started = Instant::now();
    let last_emit_ms = AtomicU64::new(0);
    move |progress: BuildProgress| {
        let now_ms = started.elapsed().as_millis() as u64;
        let last = last_emit_ms.load(Ordering::Relaxed);
        let finished = progress.files_done >= progress.files_total;
        let due = finished || now_ms.saturating_sub(last) >= 250;
        if due && last_emit_ms.compare_exchange(last, now_ms, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
            // A missed progress event costs nothing; the build's result is what matters.
            let _ = app.emit(
                BUILD_PROGRESS_EVENT,
                BuildProgressEvent {
                    install_id: install_id.clone(),
                    files_done: progress.files_done,
                    files_total: progress.files_total,
                    bytes_hashed: progress.bytes_hashed,
                    bytes_total: progress.bytes_total,
                },
            );
        }
    }
}

/// Map core's build result to the reply. A Linked build that cannot link is a reply the user
/// chooses from; every other failure is an error.
fn build_reply(result: Result<BuildOutcome, BaseError>) -> Result<BuildReply, GamesError> {
    match result {
        Ok(BuildOutcome::Built {
            manifest,
            linked_bytes,
            copied_bytes,
        }) => Ok(BuildReply::Built {
            base_id: manifest.base_id,
            mode: manifest.mode.to_string(),
            location: manifest.location.to_string_lossy().into_owned(),
            file_count: manifest.files.len(),
            linked_bytes,
            copied_bytes,
        }),
        Ok(BuildOutcome::Existing(manifest)) => Ok(BuildReply::Existing {
            base_id: manifest.base_id,
            mode: manifest.mode.to_string(),
            location: manifest.location.to_string_lossy().into_owned(),
            file_count: manifest.files.len(),
        }),
        Err(BaseError::LinkUnavailable {
            reason,
            copied_bytes,
        }) => Ok(BuildReply::LinkUnavailable {
            reason,
            copied_bytes_needed: copied_bytes,
        }),
        Err(error) => Err(base_error(error)),
    }
}

// ---------------------------------------------------------------------------
// Verify
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyReply {
    pub base_id: String,
    /// `quick` (sizes, times and identities) or `full` (every file hashed again).
    pub depth: String,
    pub checked: usize,
    pub hashed: usize,
    pub clean: bool,
    /// Files the game wrote into the base. Those are kept in each instance, not a damage.
    pub game_writes: Vec<String>,
    pub problems: Vec<BaseFileProblem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseFileProblem {
    pub path: String,
    /// `missing`, `size_changed`, `content_changed` or `unexpected`.
    pub kind: String,
    pub expected_bytes: Option<u64>,
    pub actual_bytes: Option<u64>,
    /// The file is a hardlink to the store install, so the change reached the store install too.
    pub linked_to_store: bool,
}

/// Check a base against its manifest. A damaged base is a successful reply that lists the
/// problems; only a base that cannot be checked at all is an error.
#[tauri::command]
pub async fn games_base_verify(
    app: tauri::AppHandle,
    base_id: String,
    full: bool,
) -> Result<VerifyReply, GamesError> {
    validate_base_id(&base_id)?;
    tokio::task::spawn_blocking(move || -> Result<VerifyReply, GamesError> {
        let ctx = crate::core_context(&app)?;
        let manifest = read_manifest(&ctx, &base_id)?;
        let definition = ctx.games.game(&manifest.runtime.game).ok_or_else(|| {
            GamesError::new(
                GAME_DEFINITION_MISSING,
                "Agora does not have this game's definition loaded, so it cannot check the base.",
            )
        })?;
        let depth = if full { VerifyDepth::Full } else { VerifyDepth::Quick };
        let verification = game_base::verify_base(
            &manifest,
            depth,
            &|path| definition.is_declared_write(path),
            &|path| definition.is_excluded(path),
        );
        Ok(verify_reply(&base_id, depth, verification))
    })
    .await
    .map_err(task_failed)?
}

fn verify_reply(base_id: &str, depth: VerifyDepth, verification: BaseVerification) -> VerifyReply {
    let clean = verification.problems.is_empty();
    VerifyReply {
        base_id: base_id.to_string(),
        depth: match depth {
            VerifyDepth::Quick => "quick".to_string(),
            VerifyDepth::Full => "full".to_string(),
        },
        checked: verification.checked,
        hashed: verification.hashed,
        clean,
        game_writes: verification.game_writes,
        problems: verification.problems.into_iter().map(file_problem).collect(),
    }
}

fn file_problem(problem: BaseProblem) -> BaseFileProblem {
    let (kind, expected_bytes, actual_bytes) = match problem.kind {
        ProblemKind::Missing => ("missing", None, None),
        ProblemKind::SizeChanged { expected, actual } => ("size_changed", Some(expected), Some(actual)),
        ProblemKind::ContentChanged => ("content_changed", None, None),
        ProblemKind::Unexpected => ("unexpected", None, None),
    };
    BaseFileProblem {
        path: problem.path,
        kind: kind.to_string(),
        expected_bytes,
        actual_bytes,
        linked_to_store: problem.linked_to_store,
    }
}

// ---------------------------------------------------------------------------
// Remove
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveReply {
    pub base_id: String,
    pub status: String,
}

/// Delete a base no instance uses. Core refuses while any instance is pinned to it and names
/// them; the UI asks before calling this.
#[tauri::command]
pub async fn games_base_remove(app: tauri::AppHandle, base_id: String) -> Result<RemoveReply, GamesError> {
    validate_base_id(&base_id)?;
    tokio::task::spawn_blocking(move || -> Result<RemoveReply, GamesError> {
        let ctx = crate::core_context(&app)?;
        game_base::remove_base(&ctx.paths, &base_id).map_err(base_error)?;
        Ok(RemoveReply {
            base_id,
            status: "removed".to_string(),
        })
    })
    .await
    .map_err(task_failed)?
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// A base id names a manifest file and a folder, so it must be a plain name. Core's ids are
/// lowercase letters, digits, `.`, `-` and `_`; anything else is refused before it touches a path.
pub fn validate_base_id(base_id: &str) -> Result<(), GamesError> {
    let plain = !base_id.is_empty()
        && base_id.len() <= 200
        && base_id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '-' | '_'));
    if plain {
        Ok(())
    } else {
        Err(GamesError::new(
            INVALID_INPUT,
            format!("'{base_id}' is not a base id. Refresh the list and pick a base from it."),
        ))
    }
}

fn read_manifest(ctx: &Ctx, base_id: &str) -> Result<BaseManifest, GamesError> {
    let path = ctx.paths.base_manifest_path(base_id);
    let text = std::fs::read_to_string(&path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            GamesError::new(BASE_NOT_FOUND, format!("Base {base_id} was not found. Refresh the list."))
        } else {
            GamesError::new(
                BASE_FAILED,
                format!("Could not read the record of base {base_id}: {error}"),
            )
        }
    })?;
    serde_json::from_str(&text).map_err(|error| {
        GamesError::new(
            BASE_FAILED,
            format!("The record of base {base_id} is damaged and cannot be read: {error}"),
        )
    })
}

/// Core's base errors, in words a modder can act on.
pub fn base_error(error: BaseError) -> GamesError {
    match error {
        BaseError::InUse { instances } => GamesError::new(
            BASE_IN_USE,
            format!(
                "This base is still used by {}. Move or remove those instances first, then remove the base.",
                instances.join(", ")
            ),
        ),
        BaseError::NotFound(id) => {
            GamesError::new(BASE_NOT_FOUND, format!("Base {id} was not found. Refresh the list."))
        }
        BaseError::RuntimeUnknown => GamesError::new(
            RUNTIME_UNKNOWN,
            "Agora cannot tell which exact version this install is, so it cannot build a base from it. Rescan installs, then try again.",
        ),
        BaseError::DeclaredWriteLinked { path } => GamesError::new(
            DECLARED_WRITE_LINKED,
            format!(
                "The game writes to {path}, and a Linked base would share that file with the store install. Build a Copied base instead."
            ),
        ),
        BaseError::LinkUnavailable {
            reason,
            copied_bytes,
        } => GamesError::new(
            LINK_UNAVAILABLE,
            format!(
                "Hardlinks are not available for this install: {reason}. A Copied base would need {} GB.",
                copied_bytes / (1024 * 1024 * 1024)
            ),
        ),
        BaseError::Io(error) => GamesError::new(
            BASE_FAILED,
            format!("Reading or writing the base failed on disk: {error}"),
        ),
        BaseError::Json(error) => GamesError::new(
            BASE_FAILED,
            format!("The base's record could not be written or read: {error}"),
        ),
        BaseError::Other(message) => GamesError::new(BASE_FAILED, message),
    }
}

fn instance_error(error: InstanceError) -> GamesError {
    GamesError::new(
        BASE_FAILED,
        format!("Agora could not read the list of instances, so it cannot tell which bases are in use: {error}"),
    )
}

fn task_failed(_: tokio::task::JoinError) -> GamesError {
    GamesError::new(TASK_FAILED, "Working on the base did not finish.")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A manifest as it is stored on disk. Built from JSON so the test does not need the
    /// `agora-game-api` types the desktop crate does not depend on.
    fn manifest_json(files: serde_json::Value) -> BaseManifest {
        serde_json::from_value(serde_json::json!({
            "manifest_version": 1,
            "base_id": "skyrim-se_steam_1.6.1170_nobuild",
            "runtime": { "game": "skyrim-se", "store": "steam", "version": "1.6.1170", "build": null },
            "mode": "linked",
            "source_install": "steam:489830",
            "source_product": "489830",
            "source_location": "D:/Games/Skyrim Special Edition",
            "location": "D:/AgoraBases/skyrim-se_steam_1.6.1170_nobuild",
            "created_unix_ms": 1700000000000i64,
            "files": files,
            "skipped": ["Data/link.bsa: symlink or junction"],
        }))
        .expect("the fixture is a valid manifest")
    }

    fn file(path: &str, size: u64, linked: bool) -> serde_json::Value {
        serde_json::json!({
            "path": path,
            "size": size,
            "sha256": "00",
            "linked": linked,
            "identity": null,
            "modified_unix_ms": 0,
        })
    }

    #[test]
    fn summary_counts_linked_and_copied_bytes_separately() {
        let manifest = manifest_json(serde_json::json!([
            file("Data/Skyrim - Meshes0.bsa", 1_000, true),
            file("Data/Skyrim.esm", 200, false),
            file("SkyrimSE.exe", 50, false),
        ]));
        let summary = summarize(&manifest, true, Vec::new());
        assert_eq!(summary.file_count, 3);
        assert_eq!(summary.linked_file_count, 1);
        assert_eq!(summary.linked_bytes, 1_000);
        assert_eq!(summary.copied_bytes, 250);
        assert_eq!(summary.mode, "linked");
        assert_eq!(summary.game_id, "skyrim-se");
        assert_eq!(summary.skipped_count, 1);
        assert!(summary.present);
    }

    #[test]
    fn summary_of_a_base_with_no_files_is_zero_and_missing_is_reported() {
        let summary = summarize(&manifest_json(serde_json::json!([])), false, Vec::new());
        assert_eq!(summary.file_count, 0);
        assert_eq!(summary.copied_bytes, 0);
        assert!(!summary.present);
    }

    #[test]
    fn users_are_named_and_an_unknown_id_stays_as_its_own_name() {
        let mut names = HashMap::new();
        names.insert("nordic-run".to_string(), "Nordic Run".to_string());
        let users = named_users(&["nordic-run".to_string(), "gone".to_string()], &names);
        assert_eq!(users[0].name, "Nordic Run");
        assert_eq!(users[1].name, "gone");
        assert_eq!(users.len(), 2, "an instance missing from the table is still a user");
    }

    #[test]
    fn name_map_covers_every_instance_record() {
        let record: GameInstanceRecord = serde_json::from_value(serde_json::json!({
            "instance_id": "nordic-run",
            "game": "skyrim-se",
            "name": "Nordic Run",
            "base": { "kind": "unpinned", "install": "steam:489830", "reason": "test" },
            "created_at": "2026-10-01T00:00:00Z",
            "last_launched_at": null,
        }))
        .expect("the fixture is a valid record");
        assert_eq!(name_map(&[record]).get("nordic-run").map(String::as_str), Some("Nordic Run"));
    }

    #[test]
    fn in_use_error_names_every_instance() {
        let error = base_error(BaseError::InUse {
            instances: vec!["Nordic Run".to_string(), "Vanilla Plus".to_string()],
        });
        assert_eq!(error.code, BASE_IN_USE);
        assert!(error.message.contains("Nordic Run, Vanilla Plus"), "{}", error.message);
    }

    #[test]
    fn not_found_runtime_unknown_and_declared_write_map_to_their_own_codes() {
        assert_eq!(base_error(BaseError::NotFound("x".into())).code, BASE_NOT_FOUND);
        assert_eq!(base_error(BaseError::RuntimeUnknown).code, RUNTIME_UNKNOWN);
        assert_eq!(
            base_error(BaseError::DeclaredWriteLinked { path: "Data/a.bsa".into() }).code,
            DECLARED_WRITE_LINKED
        );
    }

    #[test]
    fn io_and_other_failures_keep_their_message() {
        let io = base_error(BaseError::Io(std::io::Error::other("disk full")));
        assert_eq!(io.code, BASE_FAILED);
        assert!(io.message.contains("disk full"));
        let other = base_error(BaseError::Other("refusing to remove D:/x".into()));
        assert_eq!(other.code, BASE_FAILED);
        assert_eq!(other.message, "refusing to remove D:/x");
    }

    #[test]
    fn a_link_that_cannot_be_made_is_a_choice_not_an_error() {
        let reply = build_reply(Err(BaseError::LinkUnavailable {
            reason: "base is on another volume".into(),
            copied_bytes: 16_000_000_000,
        }))
        .expect("the user is offered the choice, so this is not an error");
        assert_eq!(
            reply,
            BuildReply::LinkUnavailable {
                reason: "base is on another volume".into(),
                copied_bytes_needed: 16_000_000_000,
            }
        );
    }

    #[test]
    fn a_built_base_maps_its_byte_counts() {
        let manifest = manifest_json(serde_json::json!([file("Data/a.bsa", 10, true), file("b.exe", 4, false)]));
        let reply = build_reply(Ok(BuildOutcome::Built {
            manifest,
            linked_bytes: 10,
            copied_bytes: 4,
        }))
        .expect("a built base is a reply");
        match reply {
            BuildReply::Built { linked_bytes, copied_bytes, file_count, .. } => {
                assert_eq!((linked_bytes, copied_bytes, file_count), (10, 4, 2));
            }
            other => panic!("expected a built reply, got {other:?}"),
        }
    }

    #[test]
    fn verify_maps_every_problem_kind() {
        let verification = BaseVerification {
            checked: 4,
            hashed: 1,
            game_writes: vec!["Data/Saves.ini".into()],
            problems: vec![
                BaseProblem { path: "a".into(), kind: ProblemKind::Missing, linked_to_store: true },
                BaseProblem {
                    path: "b".into(),
                    kind: ProblemKind::SizeChanged { expected: 10, actual: 7 },
                    linked_to_store: false,
                },
                BaseProblem { path: "c".into(), kind: ProblemKind::ContentChanged, linked_to_store: false },
                BaseProblem { path: "d".into(), kind: ProblemKind::Unexpected, linked_to_store: false },
            ],
        };
        let reply = verify_reply("skyrim-se_steam_1.6.1170_nobuild", VerifyDepth::Full, verification);
        assert!(!reply.clean);
        assert_eq!(reply.depth, "full");
        assert_eq!(reply.game_writes, vec!["Data/Saves.ini".to_string()]);
        let kinds: Vec<&str> = reply.problems.iter().map(|p| p.kind.as_str()).collect();
        assert_eq!(kinds, ["missing", "size_changed", "content_changed", "unexpected"]);
        assert_eq!(reply.problems[1].expected_bytes, Some(10));
        assert_eq!(reply.problems[1].actual_bytes, Some(7));
        assert!(reply.problems[0].linked_to_store);
    }

    #[test]
    fn a_clean_verification_says_so() {
        let reply = verify_reply(
            "x",
            VerifyDepth::Quick,
            BaseVerification { checked: 2, hashed: 0, game_writes: vec![], problems: vec![] },
        );
        assert!(reply.clean);
        assert_eq!(reply.depth, "quick");
    }

    #[test]
    fn base_ids_must_be_plain_names() {
        assert!(validate_base_id("skyrim-se_steam_1.6.1170.0_24914197").is_ok());
        assert!(validate_base_id("skyrim-se_gog_1.6.1179_nobuild_copied").is_ok());
        for bad in ["", "../x", "a/b", "a\\b", "Upper", "with space", "a:b"] {
            assert_eq!(validate_base_id(bad).map_err(|e| e.code), Err(INVALID_INPUT.to_string()), "{bad:?}");
        }
    }

    /// Reads the real bases in the bench data root. Read-only: it lists and quick-verifies, and
    /// writes nothing. Run with `cargo test -p agora-desktop -- --ignored real_bench_bases`.
    #[test]
    #[ignore]
    fn real_bench_bases_list_and_quick_verify() {
        let root = std::path::PathBuf::from("D:/Agora-bench/p3-done");
        let paths = agora_core::app_paths::AppPaths::from_root(root);
        let mut builder = agora_core::game_registry::GameRegistry::builder();
        builder
            .add(
                agora_core::game_registry::PackageSource::Compiled {
                    crate_name: "agora-game-creation".to_string(),
                },
                agora_game_creation::game_package(),
            )
            .expect("the Creation Engine package registers");
        let games = builder.build();

        let listings = game_base::list_bases(&paths);
        assert!(!listings.is_empty(), "the bench root has base manifests");
        for listing in listings {
            let definition = games
                .game(&listing.manifest.runtime.game)
                .expect("the bench bases are for a registered game");
            let verification = game_base::verify_base(
                &listing.manifest,
                VerifyDepth::Quick,
                &|path| definition.is_declared_write(path),
                &|path| definition.is_excluded(path),
            );
            let reply = verify_reply(&listing.manifest.base_id, VerifyDepth::Quick, verification);
            eprintln!(
                "{}: present={} files={} clean={} problems={}",
                reply.base_id,
                listing.present,
                listing.manifest.files.len(),
                reply.clean,
                reply.problems.len()
            );
        }
    }
}
