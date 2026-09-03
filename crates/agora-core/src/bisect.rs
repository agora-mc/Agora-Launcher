//! Guided mod bisect — binary search for the mod that breaks a launch.
//!
//! The manual version of this is the single most-repeated piece of Minecraft
//! troubleshooting advice: disable half your mods, launch, repeat. It is
//! tedious, easy to lose track of, and people give up halfway. Nothing
//! automates it.
//!
//! # Why the split has to be dependency-closed
//!
//! The naive halving is wrong, and wrong in a way that silently produces a
//! confident false answer. If mod `A` requires library `L` and a trial disables
//! `L` but leaves `A` enabled, the game crashes — on a *missing dependency*,
//! not on the bug being hunted. The bisect reads that as "the culprit is in
//! this half" and narrows toward `L`, which is innocent.
//!
//! So every held-out set is closed over its dependents: disabling `L` also
//! disables everything that needs `L`. The consequence is that the answer is a
//! *group*, not always a single jar — when a mod can only be disabled together
//! with its dependents, the bisect can narrow no further than that group, and
//! says so rather than picking one arbitrarily.
//!
//! # State
//!
//! A session lives at `<instance>/.agora_bisect.json` and survives restarts,
//! because each trial requires the user to actually launch the game. The
//! recorded history is what makes [`step_back`] possible: the user's stated
//! need is to back up and take the *other* half when a branch turns out to be a
//! dead end.
//!
//! Nothing here touches the filesystem beyond that state file. Applying a trial
//! is [`crate::loadout::apply_profile`]'s job, and reverting is
//! [`crate::snapshot`]'s; this module only decides *what* should be enabled.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::dependency_ops::{build_dependency_graph_with_aliases, AliasMap};
use crate::models::InstalledMod;

/// On-disk format version for a bisect session.
pub const BISECT_SCHEMA_VERSION: u32 = 1;

const SESSION_FILE: &str = ".agora_bisect.json";

/// What happened when the user launched with a trial applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrialOutcome {
    /// The problem still happened — it is in the enabled half.
    Reproduced,
    /// The game was fine — the problem is in the half that was held out.
    Clean,
}

/// One completed or pending trial.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BisectStep {
    /// Suspects that were enabled for this trial.
    pub enabled_suspects: Vec<String>,
    /// Suspects that were disabled for this trial, after dependency closure.
    pub disabled_suspects: Vec<String>,
    /// `None` while the user has not launched yet.
    #[serde(default)]
    pub outcome: Option<TrialOutcome>,
}

/// Where a bisect has got to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum BisectStatus {
    /// A trial is ready to be applied and launched.
    AwaitingTrial,
    /// Narrowed to a single mod.
    Culprit { filename: String },
    /// Narrowed as far as the dependency graph allows: these move together.
    CulpritGroup { filenames: Vec<String> },
    /// Every suspect was cleared. The problem is not a single mod — it may be
    /// an interaction, a config, or not mod-related at all.
    Inconclusive,
}

/// A persisted bisect session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BisectSession {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub started_at: String,
    /// Every mod that was enabled when the bisect started. Restoring this set
    /// is what "give up and put it back" means.
    pub baseline_enabled: Vec<String>,
    /// The pool still under suspicion, narrowing with each trial.
    pub suspects: Vec<String>,
    /// Completed trials, oldest first.
    pub history: Vec<BisectStep>,
    /// Set by [`step_back`]: the next trial must take the opposite half of the
    /// step that was undone, or the bisect would just repeat it.
    #[serde(default)]
    pub invert_next_split: bool,
}

fn default_schema_version() -> u32 {
    1
}

/// Path to an instance's bisect state.
pub fn session_path(instance_dir: &Path) -> PathBuf {
    instance_dir.join(SESSION_FILE)
}

/// Read the in-progress session, if any.
pub fn read_session(instance_dir: &Path) -> Result<Option<BisectSession>, String> {
    let path = session_path(instance_dir);
    if !path.is_file() {
        return Ok(None);
    }
    let text =
        std::fs::read_to_string(&path).map_err(|e| format!("Cannot read bisect session: {e}"))?;
    let session: BisectSession =
        serde_json::from_str(&text).map_err(|e| format!("Cannot parse bisect session: {e}"))?;
    if session.schema_version > BISECT_SCHEMA_VERSION {
        return Err(format!(
            "This bisect session was written by a newer version of Agora (format {}).",
            session.schema_version
        ));
    }
    Ok(Some(session))
}

/// Write the session atomically.
pub fn write_session(instance_dir: &Path, session: &BisectSession) -> Result<(), String> {
    let path = session_path(instance_dir);
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(session)
        .map_err(|e| format!("Cannot serialize bisect session: {e}"))?;
    std::fs::write(&tmp, json.as_bytes())
        .map_err(|e| format!("Cannot write bisect session: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("Cannot finalize bisect session: {e}"))
}

/// Abandon the bisect. The caller is responsible for restoring
/// `baseline_enabled` first.
pub fn clear_session(instance_dir: &Path) -> Result<(), String> {
    let path = session_path(instance_dir);
    if !path.exists() {
        return Ok(());
    }
    std::fs::remove_file(&path).map_err(|e| format!("Cannot clear bisect session: {e}"))
}

/// Map each mod to the mods that require it, directly or transitively.
///
/// Built from the same graph the dependency view draws, so the two can never
/// disagree about what needs what.
fn dependents_closure(installed: &[InstalledMod]) -> HashMap<String, HashSet<String>> {
    let edges = build_dependency_graph_with_aliases(installed, &AliasMap::from_pairs(&[]));
    // Direct dependents: target -> those that declare it.
    let mut direct: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in &edges {
        // Optional edges count too. A mod that merely *recommends* another
        // usually tolerates its absence, but "usually" is not good enough when
        // a wrong answer sends the user hunting an innocent mod.
        direct
            .entry(edge.to_filename.as_str())
            .or_default()
            .push(edge.from_filename.as_str());
    }

    let mut closure: HashMap<String, HashSet<String>> = HashMap::new();
    for entry in installed {
        let mut reached: HashSet<String> = HashSet::new();
        let mut queue: Vec<&str> = vec![entry.filename.as_str()];
        while let Some(current) = queue.pop() {
            for dependent in direct.get(current).into_iter().flatten() {
                if reached.insert((*dependent).to_string()) {
                    queue.push(dependent);
                }
            }
        }
        closure.insert(entry.filename.clone(), reached);
    }
    closure
}

/// Expand a set of mods to disable so it includes everything that needs them.
///
/// Without this the trial crashes on a missing dependency instead of the bug,
/// and the bisect narrows toward an innocent library.
pub fn close_over_dependents(to_disable: &[String], installed: &[InstalledMod]) -> Vec<String> {
    let closure = dependents_closure(installed);
    let mut out: BTreeSet<String> = to_disable.iter().cloned().collect();
    for filename in to_disable {
        if let Some(dependents) = closure.get(filename) {
            out.extend(dependents.iter().cloned());
        }
    }
    out.into_iter().collect()
}

/// Begin a bisect over the currently enabled mods.
///
/// `prime_suspects` — typically mod ids named in a crash log — are ordered
/// first so the opening split tests them, which is far more likely to be
/// decisive than a blind halving.
pub fn start_session(
    installed: &[InstalledMod],
    prime_suspects: &[String],
    now: &str,
) -> Result<BisectSession, String> {
    let enabled: Vec<String> = installed
        .iter()
        .filter(|entry| entry.enabled && entry.content_type == "mod")
        .map(|entry| entry.filename.clone())
        .collect();
    if enabled.len() < 2 {
        return Err("A bisect needs at least two enabled mods to split.".to_string());
    }

    // Every enabled mod is a suspect.
    //
    // An earlier version excluded mods that could not be disabled without
    // taking the whole pool with them — a library everything depends on. That
    // was exactly backwards: such a library is convicted by *elimination*, when
    // a trial disables everything else and the problem persists. Dropping it
    // from the pool made the one mod a bisect is worst at finding unfindable.
    let mut suspects: Vec<String> = enabled.clone();

    // Crash-named mods first; the rest keep their existing order so a session
    // is reproducible.
    let primes: HashSet<&str> = prime_suspects.iter().map(String::as_str).collect();
    suspects.sort_by_key(|filename| !primes.contains(filename.as_str()));

    Ok(BisectSession {
        schema_version: BISECT_SCHEMA_VERSION,
        started_at: now.to_string(),
        baseline_enabled: enabled,
        suspects,
        history: Vec::new(),
        invert_next_split: false,
    })
}

/// What the bisect wants next.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BisectTrial {
    pub status: BisectStatus,
    /// The complete set of mods that should be enabled for this trial. Empty
    /// when the bisect is finished.
    pub enable: Vec<String>,
    /// Suspects deliberately held out, after dependency closure.
    pub disable: Vec<String>,
    /// Trials completed so far, for a progress readout.
    pub completed_trials: usize,
    /// How many trials remain in the worst case.
    pub remaining_trials: usize,
}

/// Decide the next trial, or report that the bisect is finished.
pub fn next_trial(session: &BisectSession, installed: &[InstalledMod]) -> BisectTrial {
    let completed = session
        .history
        .iter()
        .filter(|s| s.outcome.is_some())
        .count();

    let finished = |status: BisectStatus| BisectTrial {
        status,
        enable: Vec::new(),
        disable: Vec::new(),
        completed_trials: completed,
        remaining_trials: 0,
    };

    match session.suspects.len() {
        0 => return finished(BisectStatus::Inconclusive),
        1 => {
            return finished(BisectStatus::Culprit {
                filename: session.suspects[0].clone(),
            })
        }
        _ => {}
    }

    let midpoint = session.suspects.len().div_ceil(2);
    let (first, second) = session.suspects.split_at(midpoint);
    // After a step back, taking the same half again would just replay the
    // branch the user rejected.
    let (kept, held) = if session.invert_next_split {
        (second, first)
    } else {
        (first, second)
    };

    #[allow(clippy::unnecessary_to_owned)]
    let disable = close_over_dependents(&held.to_vec(), installed);
    let disable_set: HashSet<&str> = disable.iter().map(String::as_str).collect();

    // The closure can pull the entire other half back in — when every suspect
    // depends on the one being held out, there is no split left to make.
    //
    // Only this direction matters. Whether disabling `kept` *would* drag in
    // `held` is a question about a trial we are not running: the trial we are
    // running disables `held`, and it is informative as long as one suspect is
    // still enabled. Testing both directions gives up on pools that are
    // perfectly separable — see
    // `a_dependent_pair_is_still_separable_from_its_library`.
    if kept.iter().all(|f| disable_set.contains(f.as_str())) {
        return finished(BisectStatus::CulpritGroup {
            filenames: session.suspects.clone(),
        });
    }

    let enable: Vec<String> = session
        .baseline_enabled
        .iter()
        .filter(|filename| !disable_set.contains(filename.as_str()))
        .cloned()
        .collect();

    BisectTrial {
        status: BisectStatus::AwaitingTrial,
        enable,
        disable,
        completed_trials: completed,
        // Binary search: each trial halves the pool.
        remaining_trials: session.suspects.len().next_power_of_two().trailing_zeros() as usize,
    }
}

/// Record what happened and narrow the suspect pool.
///
/// `Reproduced` means the problem is among the mods that were left on;
/// `Clean` means it is among those held out.
pub fn record_outcome(
    session: &mut BisectSession,
    installed: &[InstalledMod],
    outcome: TrialOutcome,
) -> Result<BisectStatus, String> {
    let trial = next_trial(session, installed);
    if trial.status != BisectStatus::AwaitingTrial {
        return Err("This bisect has already reached a conclusion.".to_string());
    }

    let disabled: HashSet<&str> = trial.disable.iter().map(String::as_str).collect();
    let enabled_suspects: Vec<String> = session
        .suspects
        .iter()
        .filter(|f| !disabled.contains(f.as_str()))
        .cloned()
        .collect();
    let disabled_suspects: Vec<String> = session
        .suspects
        .iter()
        .filter(|f| disabled.contains(f.as_str()))
        .cloned()
        .collect();

    session.history.push(BisectStep {
        enabled_suspects: enabled_suspects.clone(),
        disabled_suspects: disabled_suspects.clone(),
        outcome: Some(outcome),
    });
    session.invert_next_split = false;
    session.suspects = match outcome {
        TrialOutcome::Reproduced => enabled_suspects,
        TrialOutcome::Clean => disabled_suspects,
    };

    Ok(next_trial(session, installed).status)
}

/// Apply a trial's enabled set to the instance on disk.
///
/// Delegates to [`crate::loadout::apply_enabled_set_scoped`], which already
/// owns the `.jar` / `.jar.disabled` rename and the matching manifest update.
/// Bisect decides *what* should be enabled; it does not reimplement *how*.
///
/// Scoped to mods, because that is all a bisect ever reasons about:
/// `start_session` collects enabled entries with `content_type == "mod"`, so an
/// unscoped call would read the absence of every resource pack, shader, data
/// pack and world from the list as "turn these off" and disable the lot — and
/// the cancel path, restoring the same mod-only baseline, would never put them
/// back.
pub fn apply_enabled_set(instance_dir: &Path, enabled: &[String]) -> Result<(), String> {
    crate::loadout::apply_enabled_set_scoped(instance_dir, enabled, Some("mod"))
}

/// Undo the last trial and take the other half next time.
///
/// This is the point of persisting history: a user who follows one branch to a
/// dead end needs to back out and try the half they skipped, possibly across
/// several launches.
pub fn step_back(session: &mut BisectSession) -> Result<(), String> {
    let Some(last) = session.history.pop() else {
        return Err("There is nothing to step back to.".to_string());
    };
    // Restore the pool as it was before that trial, then force the opposite
    // branch so the next trial is not a replay.
    let mut restored = last.enabled_suspects;
    restored.extend(last.disabled_suspects);
    restored.sort();
    restored.dedup();
    session.suspects = restored;
    session.invert_next_split = true;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(filename: &str, deps: &[&str]) -> InstalledMod {
        InstalledMod {
            filename: filename.to_string(),
            registry_id: None,
            modrinth_id: None,
            source: "manual".into(),
            source_url: None,
            version: None,
            sha256: String::new(),
            installed_at: String::new(),
            java_packages: Vec::new(),
            mod_jar_id: Some(filename.trim_end_matches(".jar").to_string()),
            provided_mod_ids: Vec::new(),
            enabled: true,
            content_type: "mod".into(),
            update_pinned: false,
            pack_managed: false,
            installed_as_dependency: false,
            depends_on: deps.iter().map(|d| d.to_string()).collect(),
            optional_deps: Vec::new(),
            incompatible_deps: Vec::new(),
        }
    }

    fn plain(count: usize) -> Vec<InstalledMod> {
        (0..count).map(|i| m(&format!("mod{i}.jar"), &[])).collect()
    }

    #[test]
    fn a_split_that_disables_a_library_also_disables_what_needs_it() {
        // Otherwise the trial crashes on a missing dependency rather than the
        // bug, and the bisect convicts the library.
        let installed = vec![
            m("corelib.jar", &[]),
            m("caves.jar", &["corelib"]),
            m("towers.jar", &["caves"]),
            m("sodium.jar", &[]),
        ];
        let disable = close_over_dependents(&["corelib.jar".to_string()], &installed);
        assert_eq!(
            disable,
            vec!["caves.jar", "corelib.jar", "towers.jar"],
            "closure must be transitive"
        );
    }

    #[test]
    fn closure_leaves_unrelated_mods_alone() {
        let installed = vec![m("a.jar", &[]), m("b.jar", &[])];
        assert_eq!(
            close_over_dependents(&["a.jar".to_string()], &installed),
            vec!["a.jar"]
        );
    }

    #[test]
    fn a_bisect_needs_something_to_split() {
        assert!(start_session(&plain(1), &[], "2026-01-01T00:00:00Z").is_err());
        assert!(start_session(&plain(2), &[], "2026-01-01T00:00:00Z").is_ok());
    }

    #[test]
    fn crash_named_mods_are_tested_first() {
        let installed = plain(4);
        let session = start_session(
            &installed,
            &["mod3.jar".to_string()],
            "2026-01-01T00:00:00Z",
        )
        .unwrap();
        assert_eq!(session.suspects[0], "mod3.jar");
        let trial = next_trial(&session, &installed);
        assert!(trial.enable.contains(&"mod3.jar".to_string()));
    }

    #[test]
    fn a_reproduced_trial_narrows_to_the_half_that_was_left_on() {
        let installed = plain(4);
        let mut session = start_session(&installed, &[], "2026-01-01T00:00:00Z").unwrap();
        let before = next_trial(&session, &installed);
        assert_eq!(before.status, BisectStatus::AwaitingTrial);

        record_outcome(&mut session, &installed, TrialOutcome::Reproduced).unwrap();
        assert_eq!(session.suspects.len(), 2);
        for filename in &session.suspects {
            assert!(before.enable.contains(filename), "{filename} was disabled");
        }
    }

    #[test]
    fn a_clean_trial_narrows_to_the_half_that_was_held_out() {
        let installed = plain(4);
        let mut session = start_session(&installed, &[], "2026-01-01T00:00:00Z").unwrap();
        let before = next_trial(&session, &installed);
        record_outcome(&mut session, &installed, TrialOutcome::Clean).unwrap();
        for filename in &session.suspects {
            assert!(before.disable.contains(filename), "{filename} was enabled");
        }
    }

    #[test]
    fn a_bisect_converges_on_the_single_guilty_mod() {
        let installed = plain(8);
        let guilty = "mod5.jar".to_string();
        let mut session = start_session(&installed, &[], "2026-01-01T00:00:00Z").unwrap();
        let mut status = BisectStatus::AwaitingTrial;
        for _ in 0..8 {
            let trial = next_trial(&session, &installed);
            if trial.status != BisectStatus::AwaitingTrial {
                status = trial.status;
                break;
            }
            // The game crashes exactly when the guilty mod is loaded.
            let outcome = if trial.enable.contains(&guilty) {
                TrialOutcome::Reproduced
            } else {
                TrialOutcome::Clean
            };
            status = record_outcome(&mut session, &installed, outcome).unwrap();
        }
        assert_eq!(status, BisectStatus::Culprit { filename: guilty });
    }

    #[test]
    fn stepping_back_takes_the_other_half_instead_of_replaying() {
        let installed = plain(4);
        let mut session = start_session(&installed, &[], "2026-01-01T00:00:00Z").unwrap();
        let first = next_trial(&session, &installed);
        record_outcome(&mut session, &installed, TrialOutcome::Reproduced).unwrap();

        step_back(&mut session).unwrap();
        assert_eq!(session.suspects.len(), 4, "the pool is restored");
        let retry = next_trial(&session, &installed);
        assert_ne!(
            retry.disable, first.disable,
            "stepping back must not hand back the branch the user rejected"
        );
    }

    #[test]
    fn stepping_back_past_the_beginning_is_an_error_not_a_panic() {
        let installed = plain(4);
        let mut session = start_session(&installed, &[], "2026-01-01T00:00:00Z").unwrap();
        assert!(step_back(&mut session).is_err());
    }

    #[test]
    fn a_pool_that_cannot_be_split_reports_a_group_rather_than_guessing() {
        // Holding out `corelib` drags `caves` with it, leaving no suspect
        // enabled — the trial would tell us nothing. Narrowing further would
        // mean picking one of them arbitrarily, so report both.
        let installed = vec![m("corelib.jar", &[]), m("caves.jar", &["corelib"])];
        let mut session = start_session(&installed, &[], "2026-01-01T00:00:00Z").unwrap();
        // Order the pool so the half held out is the one everything needs.
        session.suspects = vec!["caves.jar".into(), "corelib.jar".into()];
        let trial = next_trial(&session, &installed);
        match trial.status {
            BisectStatus::CulpritGroup { filenames } => {
                assert_eq!(filenames.len(), 2);
            }
            other => panic!("expected a group, got {other:?}"),
        }
    }

    #[test]
    fn a_dependent_pair_is_still_separable_from_its_library() {
        // `caves` needs `corelib`, so disabling `corelib` would drag `caves`
        // with it — but the trial holds out `caves`, which drags nothing. That
        // is a perfectly good discriminating trial and the bisect must run it
        // rather than shrug and report both as one group.
        let installed = vec![m("corelib.jar", &[]), m("caves.jar", &["corelib"])];
        let session = start_session(&installed, &[], "2026-01-01T00:00:00Z").unwrap();
        let trial = next_trial(&session, &installed);
        assert_eq!(trial.status, BisectStatus::AwaitingTrial);
        assert_eq!(trial.disable, vec!["caves.jar"]);
        assert!(trial.enable.contains(&"corelib.jar".to_string()));
    }

    #[test]
    fn clearing_every_suspect_is_reported_as_inconclusive() {
        let installed = plain(4);
        let mut session = start_session(&installed, &[], "2026-01-01T00:00:00Z").unwrap();
        session.suspects.clear();
        assert_eq!(
            next_trial(&session, &installed).status,
            BisectStatus::Inconclusive
        );
    }

    #[test]
    fn a_trial_never_disables_a_mod_outside_the_baseline() {
        let installed = plain(6);
        let session = start_session(&installed, &[], "2026-01-01T00:00:00Z").unwrap();
        let trial = next_trial(&session, &installed);
        for filename in &trial.enable {
            assert!(session.baseline_enabled.contains(filename));
        }
    }

    #[test]
    fn a_session_round_trips_through_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let installed = plain(4);
        let session = start_session(&installed, &[], "2026-01-01T00:00:00Z").unwrap();
        assert!(read_session(tmp.path()).unwrap().is_none());
        write_session(tmp.path(), &session).unwrap();
        let reread = read_session(tmp.path()).unwrap().expect("written");
        assert_eq!(reread.suspects, session.suspects);
        clear_session(tmp.path()).unwrap();
        clear_session(tmp.path()).unwrap();
        assert!(read_session(tmp.path()).unwrap().is_none());
    }

    #[test]
    fn a_session_from_a_newer_agora_is_refused_rather_than_misread() {
        let tmp = tempfile::tempdir().unwrap();
        let installed = plain(4);
        let mut session = start_session(&installed, &[], "2026-01-01T00:00:00Z").unwrap();
        session.schema_version = BISECT_SCHEMA_VERSION + 1;
        write_session(tmp.path(), &session).unwrap();
        assert!(read_session(tmp.path()).is_err());
    }

    #[test]
    fn recording_an_outcome_after_the_answer_is_an_error() {
        let installed = plain(4);
        let mut session = start_session(&installed, &[], "2026-01-01T00:00:00Z").unwrap();
        session.suspects = vec!["mod0.jar".into()];
        assert!(record_outcome(&mut session, &installed, TrialOutcome::Reproduced).is_err());
    }
}
