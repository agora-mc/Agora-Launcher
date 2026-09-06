//! Three-way pack merge planner — pure planning, no filesystem mutation.
//!
//! Every launcher that does wipe-and-replace loses three things:
//! * files the user added,
//! * edits the user made to pack files,
//! * disables the user performed (`sodium.jar` → `sodium.jar.disabled`).
//!
//! The community workaround is the manual backup-then-reapply ritual described
//! in the prompt. This module replaces it with a proper three-way merge that
//! *plans* the merge and leaves execution to the existing install transaction
//! (`install_pipeline.rs:1`).
//!
//! # The three sides
//!
//! * **BASE** — what the pack contributed at install time. Already recorded via
//!   `pack_inventory.rs:31` `collect_pack_inventory` and stored in
//!   `instance_pack_files` (`db.rs:1135` `list_instance_pack_files` /
//!   `db.rs:1164` `replace_instance_pack_files`).
//! * **THEIRS** — the same inventory shape, computed from the new version of the
//!   pack (staged somewhere, then inventoried with the same `collect_pack_inventory`).
//! * **OURS** — the instance as it actually is on disk right now, via
//!   `collect_pack_inventory`.
//!
//! Each side is a `Vec<InstancePackFile>` (`db.rs:1128`) — `(relative_path, sha256, size)`.
//! The planner is pure: it takes three slices, returns a plan that describes
//! what to do with every logical path and calls out conflicts rather than
//! resolving them.
//!
//! # Design answers (before code)
//!
//! ## 1. Disabled mods — normalise `.disabled` into the identity, but keep the
//!    suffix as an attribute.
//!
//! Agora disables a mod by renaming `sodium.jar` → `sodium.jar.disabled`
//! (`helpers.rs:242` `rename_in_content_dir`, `installed_content.rs:234`,
//! `lockfile.rs:297`). The inventory (`pack_inventory.rs:92` `collect_under`)
//! deliberately includes the `.disabled` name verbatim — it walks the filesystem
//! and records whatever it finds, including the suffix.
//!
//! A naive path-keyed merge sees:
//!
//! * BASE `mods/sodium.jar` (hash A, enabled)
//! * OURS `mods/sodium.jar.disabled` (hash A, disabled)
//! * THEIRS `mods/sodium.jar` (hash B, enabled, pack update)
//!
//! as two unrelated paths: "user deleted `sodium.jar`" plus "user added
//! `sodium.jar.disabled`" plus "pack added a different `sodium.jar`". That is
//! wrong twice: the file was not deleted, it was toggled, and the pack's update
//! should not re-enable it.
//!
//! **Normalising the `.disabled` suffix into the identity is the right move.**
//! Define
//!
//! ```text
//! logical_path = strip_suffix(path, ".disabled") // crate::pack_merge::logical_path
//! enabled      = !path.ends_with(".disabled")
//! physical( logical, enabled ) = if enabled { logical } else { format!("{logical}.disabled") }
//! ```
//!
//! Then all three sides map to the same logical key `mods/sodium.jar`:
//!
//! * BASE logical `mods/sodium.jar`  hash A enabled=true
//! * OURS logical `mods/sodium.jar`  hash A enabled=false (via physical `.disabled`)
//! * THEIRS logical `mods/sodium.jar` hash B enabled=true
//!
//! Comparison becomes:
//!
//! * `content_changed_locally = (ours.sha != base.sha)`  // false here (A==A)
//! * `enabled_changed_locally = (ours.enabled != true)`  // true here
//! * `pack_changed = (theirs.sha != base.sha)`           // true (B!=A)
//!
//! If we treated `enabled_changed` as a conflicting edit, the disabled+update
//! case would be flagged `BothModified` and force a prompt. That is not what
//! the user wants — disabling is an orthogonal intent: "keep this mod off
//! regardless of version". The correct merge is **UpdateKeepDisabled**: write
//! the new bytes from THEIRS but to the `.disabled` physical path, preserving
//! the user's toggle.
//!
//! Conversely, when both sides change *content* (`ours.sha != base.sha` and
//! `theirs.sha != base.sha` with differing hashes) we must conflict, even if
//! the enabled flag also differs. When the pack removes the file entirely
//! (`THEIRS` absent) and the user had only toggled enabled, that is still
//! `ModifiedVsRemoved` and should conflict — the user's "keep it disabled"
//! intent clashes with "pack no longer ships it".
//!
//! **Does normalising break anything?**
//!
//! * A pack that legitimately shipped a file named `something.disabled` would
//!   collapse with its enabled counterpart. Packs never ship `.disabled` files
//!   (it is purely a local toggle), so this is a non-issue; for safety both
//!   BASE and THEIRS are normalised the same way, so a weird pack would still
//!   behave deterministically (the two suffixed paths would be treated as one
//!   logical file).
//! * Config files never use `.disabled`, so they are unaffected.
//! * A user-added disabled mod (`mods/mycool.jar.disabled`, not in BASE/THEIRS)
//!   maps to logical `mods/mycool.jar` which is absent in both BASE and THEIRS,
//!   so the rule "not in base and not in theirs → keep user added" still
//!   applies and the physical `.disabled` name is preserved.
//! * The case where OURS contains *both* `foo.jar` and `foo.jar.disabled` for
//!   the same logical path (two physical files colliding) is treated as a
//!   conflict — the disk state is ambiguous and must be resolved manually.
//!
//! This matches how `lockfile.rs:224` already handles enabled/disabled as a
//! toggle on the same logical path (`strip_suffix(".disabled")` vs
//! `format!("{path}.disabled")`), so the planner reuses that established
//! pattern.
//!
//! ## 2. Path equality is the wrong identity for `mods/` jars — you need a
//!    second channel keyed on the loader-visible mod id.
//!
//! A pack update usually renames the jar (`sodium-0.5.jar` → `sodium-0.6.jar`)
//! because the filename contains the version. Under pure path comparison that
//! reads as a delete of `sodium-0.5.jar` plus an unrelated add of
//! `sodium-0.6.jar`. Every pack update would then appear to remove every mod
//! the user might have touched, and an unchanged local file would be flagged
//! `RemovedVsModified`.
//!
//! The stable identity for a mod is the **loader-visible mod id** — the `id`
//! from `fabric.mod.json`, `quilt.mod.json`, `META-INF/mods.toml` /
//! `META-INF/neoforge.mods.toml`, etc. — which the loader uses to decide
//! whether two jars are the same mod. This repo already extracts it:
//!
//! * `jar_metadata.rs:279` `collect_fabric_metadata` / `jar_metadata.rs:384`
//!   `collect_quilt_metadata` / Forge parsing — all feed
//!   `JarDeps.mod_jar_id`.
//! * `parse_jar_metadata` (`jar_metadata.rs:154`) and
//!   `parse_jar_metadata_for_loader` (`jar_metadata.rs:202`) are the entry
//!   points; they also surface `provided_mod_ids` and dependency decls.
//! * `models.rs:158` `InstalledMod.mod_jar_id` caches the same value in the
//!   manifest, populated at install time.
//!
//! That is what is available to key on. A secondary `provided_mod_ids` set
//! exists but the primary `mod_jar_id` is the correct first channel; aliases
//! are secondary and only matter for duplicate-supplied-id filtering
//! (`jar_metadata.rs:1054`).
//!
//! **How to key:**
//!
//! * For files under `mods/` with a known `mod_jar_id`, the planner should
//!   group by `mod_jar_id` (lower-cased) rather than by path. Callers that
//!   have staged THEIRS (and can parse its jars) and that have the manifest's
//!   `mod_jar_id` for BASE/OURS supply a map
//!   `logical_path → mod_jar_id` for each side. The planner partitions
//!   `mods/` entries with an id into mod-id groups and falls back to path
//!   equality for every other file (configs, resource packs, and jars with no
//!   parseable id).
//! * When a mod-id group contains a rename (`base_path != theirs_path` but same
//!   id), the merge operates on the id-level presence/hashes and produces a
//!   `RenameUpdate` / `RenameUpdateKeepDisabled` action whose `target_path` is
//!   the new filename from THEIRS (with `.disabled` preserved if OURS was
//!   disabled). Without the id channel the same situation spuriously becomes a
//!   `Remove` (for the old path) plus an `Add` (for the new path), and the
//!   disabled+rename combination becomes a false `ModifiedVsRemoved` conflict.
//! * If the id is missing (jar has no metadata, or caller did not supply a
//!   map), the planner degrades to path equality — a safe fallback that
//!   preserves the "every file has a key" invariant. Those jars will see
//!   delete+add semantics on rename, which is at least correct filesystem-wise
//!   (old removed, new added) even if not minimal.
//!
//! Other identifiers exist (`registry_id`, `modrinth_id` in `models.rs:150`)
//! but they are not present in the flat `InstancePackFile` inventory; the only
//! thing derivable from the file bytes alone is the in-jar mod id, so that is
//! the right second channel for this planner.
//!
//! ## 3. Case table holes
//!
//! The draft table in the prompt is conceptually right but misses a few rows
//! and conflates enabled-toggle with content-edit:
//!
//! | # | BASE | OURS | THEIRS | Prompt says | Correct / missing |
//! |---|------|------|--------|-------------|---------------------|
//! | 1 | Some(b) | Some(o==b) | Some(t!=b) | take theirs | correct — `Update` |
//! | 2 | Some(b) | Some(o==b) | None | remove | correct — `Remove` |
//! | 3 | Some(b) | Some(o!=b) | Some(t==b) | keep user's | correct — `Keep` (including enabled-only change) |
//! | 4 | Some(b) | Some(o!=b) | Some(t!=b, o!=t) | conflict | correct — `BothModified` |
//! | 4b | Some(b) | Some(o!=b) | Some(t!=b, o==t) | — | **Missing**: both sides converged to same new hash → no conflict, keep (often arises when user manually updated to the same version the pack now ships). |
//! | 5 | Some(b) | Some(o!=b) | None | conflict | correct — `ModifiedVsRemoved` |
//! | 6 | None | Some(o) | None | keep user added | correct — headline case, `KeepUserAdded` |
//! | 7 | None | None | Some(t) | add | correct — `Add` |
//! | 8 | None | Some(o) | Some(t, o!=t) | conflict | correct — `AddedVsAdded` |
//! | 8b | None | Some(o) | Some(t, o==t) | — | **Missing**: both sides independently added the same file (same hash) → no conflict, keep. |
//! | 9 | Some(b) | None | None | — | **Missing**: both sides deleted → no conflict, already gone. |
//! |10 | Some(b) | None | Some(t==b) | — | **Missing**: user deleted locally, pack left it alone → keep deletion (no add). Prompt's "changed locally, pack left it alone → keep user's" implies this, but the deletion variant was not listed. |
//! |11 | Some(b) | None | Some(t!=b) | — | **Missing**: user deleted, pack updated → conflict `RemovedVsModified`. Prompt lists the opposite direction (modified vs removed) but not this one. |
//! | 12 | — | disabled toggle only | pack updated | — | **Missing nuance**: `base` enabled, `ours` disabled with same hash, `theirs` new hash → **not** a conflict; merge as `UpdateKeepDisabled` (see §1). Prompt's "changed locally, pack changed it too → conflict" would over-flag this. |
//!
//! The planner below implements the full table, including 4b, 8b, 9, 10, 11, and
//! the disabled-aware merge for 12.

use crate::db::InstancePackFile;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

// ---------------------------------------------------------------------------
// Helpers: disabled normalisation
// ---------------------------------------------------------------------------

/// The suffix Agora uses to mark a disabled mod (see `helpers.rs:242`,
/// `installed_content.rs:234`, `lockfile.rs:297`).
pub const DISABLED_SUFFIX: &str = ".disabled";

/// Logical path — the identity after stripping a trailing `.disabled` if present.
///
/// Pack inventories never contain `.disabled`, but both sides are normalised
/// symmetrically so a weird pack is handled deterministically.
pub fn logical_path(path: &str) -> &str {
    if let Some(stripped) = path.strip_suffix(DISABLED_SUFFIX) {
        stripped
    } else {
        path
    }
}

/// Whether the *physical* path is the disabled form.
pub fn is_disabled(path: &str) -> bool {
    path.ends_with(DISABLED_SUFFIX)
}

/// Physical path for a given logical path and enabled state.
pub fn physical_path(logical: &str, enabled: bool) -> String {
    if enabled {
        logical.to_string()
    } else {
        format!("{logical}{DISABLED_SUFFIX}")
    }
}

// ---------------------------------------------------------------------------
// Public planner types
// ---------------------------------------------------------------------------

/// What the planner thinks should happen to a non-conflicting file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanActionKind {
    /// No filesystem change — keep OURS as is.
    Keep,
    /// A user-added file (not in BASE nor THEIRS) — keep it. Headline case.
    KeepUserAdded,
    /// A new file from THEIRS — add it.
    Add,
    /// An unmodified pack file that THEIRS removed — remove it.
    Remove,
    /// An unmodified local file that THEIRS updated — overwrite with THEIRS' bytes.
    Update,
    /// Pack updated the file but OURS was disabled with unchanged content —
    /// overwrite but preserve the `.disabled` suffix.
    UpdateKeepDisabled,
    /// A mod-id rename: same mod, different filename. `target_path` is the new
    /// name from THEIRS. Old physical file should be removed.
    RenameUpdate,
    /// Rename + disabled-preserving update.
    RenameUpdateKeepDisabled,
}

impl PlanActionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::KeepUserAdded => "keep_user_added",
            Self::Add => "add",
            Self::Remove => "remove",
            Self::Update => "update",
            Self::UpdateKeepDisabled => "update_keep_disabled",
            Self::RenameUpdate => "rename_update",
            Self::RenameUpdateKeepDisabled => "rename_update_keep_disabled",
        }
    }
}

/// One non-conflicting action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanAction {
    /// Stable key for this entry — for path-merged files it is the logical
    /// path; for mod-id-merged files it is `mod:<id>` (see §2).
    pub key: String,
    /// Logical path (without `.disabled`). For mod-id entries this is the
    /// target logical path from THEIRS when present, otherwise the surviving
    /// path.
    pub logical_path: String,
    /// Physical target path that should exist after the merge (includes
    /// `.disabled` when the result is disabled).
    pub target_path: String,
    /// When the action is a rename, the previous physical path that should be
    /// removed. `None` for non-rename actions.
    pub previous_path: Option<String>,
    pub kind: PlanActionKind,
    pub base_sha: Option<String>,
    pub ours_sha: Option<String>,
    pub theirs_sha: Option<String>,
    /// `true` if the post-merge file is enabled; `false` for `.disabled`.
    pub enabled: bool,
    /// Only set for mod-id-merged entries.
    pub mod_id: Option<String>,
}

/// Why a path/id needs manual resolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictKind {
    /// Both sides modified the same file to different contents.
    BothModified,
    /// Both sides independently added the same path with different contents.
    AddedVsAdded,
    /// There is no recorded baseline, so a user edit cannot be told apart from
    /// a pack original. Reported instead of [`ConflictKind::AddedVsAdded`],
    /// whose "both of you added this" message would simply be false.
    NoBaseline,
    /// User modified (or disabled) and pack removed.
    ModifiedVsRemoved,
    /// User removed and pack modified.
    RemovedVsModified,
    /// Same logical path has two physical files on disk (both enabled and
    /// disabled present) — ambiguous.
    AmbiguousDisabledPair,
    /// Same mod id appears in two different files on one side — duplicate id.
    DuplicateModId,
}

impl ConflictKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BothModified => "both_modified",
            Self::AddedVsAdded => "added_vs_added",
            Self::NoBaseline => "no_baseline",
            Self::ModifiedVsRemoved => "modified_vs_removed",
            Self::RemovedVsModified => "removed_vs_modified",
            Self::AmbiguousDisabledPair => "ambiguous_disabled_pair",
            Self::DuplicateModId => "duplicate_mod_id",
        }
    }
}

/// One conflict — no automatic action, caller must resolve.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanConflict {
    pub key: String,
    pub logical_path: String,
    pub kind: ConflictKind,
    pub base_path: Option<String>,
    pub ours_path: Option<String>,
    pub theirs_path: Option<String>,
    pub base_sha: Option<String>,
    pub ours_sha: Option<String>,
    pub theirs_sha: Option<String>,
    pub message: String,
    pub mod_id: Option<String>,
}

/// Full merge plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackMergePlan {
    pub actions: Vec<PlanAction>,
    pub conflicts: Vec<PlanConflict>,
    /// Keys are sorted for determinism.
    pub all_keys: Vec<String>,
    /// No baseline was recorded for this instance — it was installed before
    /// Agora tracked pack inventories.
    ///
    /// Every merge decision rests on knowing what the pack originally
    /// contributed. Without it a config the user carefully edited and a config
    /// the pack shipped untouched are literally indistinguishable, so the
    /// per-file conflicts below are all one question. A caller should ask that
    /// question once rather than dozens of times.
    pub baseline_missing: bool,
}

impl PackMergePlan {
    pub fn has_conflicts(&self) -> bool {
        !self.conflicts.is_empty()
    }
    pub fn is_clean(&self) -> bool {
        self.conflicts.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Pure three-way merge keyed by (normalised) path.
/// Handles `.disabled` normalisation (see §1) but not mod renames.
pub fn plan_pack_update(
    base: &[InstancePackFile],
    theirs: &[InstancePackFile],
    ours: &[InstancePackFile],
) -> PackMergePlan {
    plan_pack_update_with_mod_ids(base, theirs, ours, None, None, None)
}

/// Three-way merge with an optional second identity channel for `mods/`.
/// When `*_mod_ids` are supplied, entries under `mods/` with a known loader
/// mod id (`jar_metadata.rs:154`) are grouped by `mod_jar_id` (case-insensitive)
/// rather than by filename, so a rename like `sodium-0.5.jar` → `sodium-0.6.jar`
/// is recognised as an update, not a delete+add.
///
/// Each map is `logical_path → mod_id` (lowercasing is normalised internally).
/// Pass `None` to fall back to pure path equality for that side.
pub fn plan_pack_update_with_mod_ids(
    base: &[InstancePackFile],
    theirs: &[InstancePackFile],
    ours: &[InstancePackFile],
    base_mod_ids: Option<&HashMap<String, String>>,
    theirs_mod_ids: Option<&HashMap<String, String>>,
    ours_mod_ids: Option<&HashMap<String, String>>,
) -> PackMergePlan {
    // An instance that predates inventory tracking has nothing to diff against.
    // `ours` being non-empty is what distinguishes it from a genuinely empty
    // instance, where an absent baseline is simply correct.
    let baseline_missing = base.is_empty() && !ours.is_empty();

    // Partition each side into path-grouped vs mod-id-grouped entries.
    let base_parts = partition_side(base, base_mod_ids);
    let theirs_parts = partition_side(theirs, theirs_mod_ids);
    let ours_parts = partition_side_ours(ours, ours_mod_ids);

    let mut actions = Vec::new();
    let mut conflicts = Vec::new();

    // Collect all mod ids.
    let mut all_mod_ids = BTreeSet::new();
    for id in base_parts.mod_groups.keys() {
        all_mod_ids.insert(id.clone());
    }
    for id in theirs_parts.mod_groups.keys() {
        all_mod_ids.insert(id.clone());
    }
    for id in ours_parts.mod_groups.keys() {
        all_mod_ids.insert(id.clone());
    }

    // Collect all path keys.
    let mut all_paths = BTreeSet::new();
    for k in base_parts.path_groups.keys() {
        all_paths.insert(k.clone());
    }
    for k in theirs_parts.path_groups.keys() {
        all_paths.insert(k.clone());
    }
    for k in ours_parts.path_groups.keys() {
        all_paths.insert(k.clone());
    }

    // Process mod-id groups.
    for mod_id in &all_mod_ids {
        let base_entry = base_parts.mod_groups.get(mod_id);
        let theirs_entry = theirs_parts.mod_groups.get(mod_id);
        let ours_entry = ours_parts.mod_groups.get(mod_id);

        // Detect duplicate mod ids on a single side (should be at most one
        // file per mod id). If a side has two files claiming same id, flag.
        // `partition_side` already collapses duplicates; we detect via
        // auxiliary duplicate set.
        if base_parts.duplicate_mod_ids.contains(mod_id)
            || theirs_parts.duplicate_mod_ids.contains(mod_id)
            || ours_parts.duplicate_mod_ids.contains(mod_id)
        {
            conflicts.push(PlanConflict {
                key: format!("mod:{mod_id}"),
                logical_path: theirs_entry
                    .map(|e| e.logical.clone())
                    .or_else(|| base_entry.map(|e| e.logical.clone()))
                    .or_else(|| ours_entry.map(|e| e.logical.clone()))
                    .unwrap_or_else(|| format!("mod:{mod_id}")),
                kind: ConflictKind::DuplicateModId,
                base_path: base_entry.map(|e| e.logical.clone()),
                ours_path: ours_entry.map(|e| e.logical.clone()),
                theirs_path: theirs_entry.map(|e| e.logical.clone()),
                base_sha: base_entry.map(|e| e.file.sha256.clone()),
                ours_sha: ours_entry.map(|e| e.file.sha256.clone()),
                theirs_sha: theirs_entry.map(|e| e.file.sha256.clone()),
                message: format!("Multiple files claim the same mod id '{mod_id}' on one side"),
                mod_id: Some(mod_id.clone()),
            });
            continue;
        }

        let key = format!("mod:{mod_id}");
        let base_logical = base_entry.map(|e| e.logical.as_str());
        let theirs_logical = theirs_entry.map(|e| e.logical.as_str());
        let ours_logical = ours_entry.map(|e| e.logical.as_str());

        // For display, the logical_path is the target from THEIRS if present,
        // otherwise the surviving side's logical.
        let display_logical = theirs_logical
            .or(ours_logical)
            .or(base_logical)
            .unwrap_or(mod_id.as_str())
            .to_string();

        let base_sha = base_entry.map(|e| e.file.sha256.as_str());
        let theirs_sha = theirs_entry.map(|e| e.file.sha256.as_str());
        let ours_sha = ours_entry.map(|e| e.file.sha256.as_str());

        // Ambiguous disabled pair on OURS side for this mod id?
        if let Some(ours) = ours_entry {
            if ours.ambiguous {
                conflicts.push(PlanConflict {
                    key: key.clone(),
                    logical_path: display_logical.clone(),
                    kind: ConflictKind::AmbiguousDisabledPair,
                    base_path: base_entry.map(|e| e.logical.clone()),
                    ours_path: Some(ours.physical.clone()),
                    theirs_path: theirs_entry.map(|e| e.logical.clone()),
                    base_sha: base_sha.map(str::to_string),
                    ours_sha: ours_sha.map(str::to_string),
                    theirs_sha: theirs_sha.map(str::to_string),
                    message: format!(
                        "OURS has both enabled and disabled files for mod '{mod_id}' ({})",
                        ours.physical
                    ),
                    mod_id: Some(mod_id.clone()),
                });
                continue;
            }
        }

        let base_present = base_entry.is_some();
        let theirs_present = theirs_entry.is_some();
        let ours_present = ours_entry.is_some();

        match (base_present, ours_present, theirs_present) {
            (false, false, false) => unreachable!("empty mod id group"),
            (false, false, true) => {
                // Added in THEIRS, not locally. Add.
                let theirs_entry = theirs_entry.unwrap();
                actions.push(PlanAction {
                    key: key.clone(),
                    logical_path: display_logical.clone(),
                    target_path: theirs_entry.logical.clone(),
                    previous_path: None,
                    kind: PlanActionKind::Add,
                    base_sha: None,
                    ours_sha: None,
                    theirs_sha: Some(theirs_entry.file.sha256.clone()),
                    enabled: true,
                    mod_id: Some(mod_id.clone()),
                });
            }
            (false, true, false) => {
                // User added, not in pack. Headline case — keep.
                let ours_entry = ours_entry.unwrap();
                actions.push(PlanAction {
                    key: key.clone(),
                    logical_path: display_logical.clone(),
                    target_path: ours_entry.physical.clone(),
                    previous_path: None,
                    kind: PlanActionKind::KeepUserAdded,
                    base_sha: None,
                    ours_sha: Some(ours_entry.file.sha256.clone()),
                    theirs_sha: None,
                    enabled: ours_entry.enabled,
                    mod_id: Some(mod_id.clone()),
                });
            }
            (false, true, true) => {
                // Both added independently, same mod id, different paths possibly.
                let ours_entry = ours_entry.unwrap();
                let theirs_entry = theirs_entry.unwrap();
                if ours_entry
                    .file
                    .sha256
                    .eq_ignore_ascii_case(&theirs_entry.file.sha256)
                    && ours_entry.enabled
                {
                    // Same content, same enabled — already present, no conflict.
                    actions.push(PlanAction {
                        key: key.clone(),
                        logical_path: display_logical.clone(),
                        target_path: ours_entry.physical.clone(),
                        previous_path: None,
                        kind: PlanActionKind::Keep,
                        base_sha: None,
                        ours_sha: Some(ours_entry.file.sha256.clone()),
                        theirs_sha: Some(theirs_entry.file.sha256.clone()),
                        enabled: true,
                        mod_id: Some(mod_id.clone()),
                    });
                } else if ours_entry
                    .file
                    .sha256
                    .eq_ignore_ascii_case(&theirs_entry.file.sha256)
                    && !ours_entry.enabled
                {
                    // Same content but ours is disabled vs theirs enabled — keep disabled.
                    actions.push(PlanAction {
                        key: key.clone(),
                        logical_path: display_logical.clone(),
                        target_path: ours_entry.physical.clone(),
                        previous_path: None,
                        kind: PlanActionKind::Keep,
                        base_sha: None,
                        ours_sha: Some(ours_entry.file.sha256.clone()),
                        theirs_sha: Some(theirs_entry.file.sha256.clone()),
                        enabled: false,
                        mod_id: Some(mod_id.clone()),
                    });
                } else {
                    conflicts.push(PlanConflict {
                        key: key.clone(),
                        logical_path: display_logical.clone(),
                        kind: if baseline_missing {
                            ConflictKind::NoBaseline
                        } else {
                            ConflictKind::AddedVsAdded
                        },
                        base_path: None,
                        ours_path: Some(ours_entry.physical.clone()),
                        theirs_path: Some(theirs_entry.logical.clone()),
                        base_sha: None,
                        ours_sha: Some(ours_entry.file.sha256.clone()),
                        theirs_sha: Some(theirs_entry.file.sha256.clone()),
                        message: if baseline_missing {
                            format!(
                                "Agora has no record of what this pack originally installed, so it cannot tell whether your '{mod_id}' is an edit or the pack's own file"
                            )
                        } else {
                            format!(
                                "Both you and the pack added mod '{mod_id}' as different files ({} vs {})",
                                ours_entry.physical, theirs_entry.logical
                            )
                        },
                        mod_id: Some(mod_id.clone()),
                    });
                }
            }
            (true, false, false) => {
                // Both removed — already gone.
                // No action needed.
            }
            (true, false, true) => {
                // User removed, pack still ships (maybe renamed/updated).
                let base_entry = base_entry.unwrap();
                let theirs_entry = theirs_entry.unwrap();
                if base_entry
                    .file
                    .sha256
                    .eq_ignore_ascii_case(&theirs_entry.file.sha256)
                {
                    // Pack unchanged, user deletion respected — keep absent.
                } else {
                    conflicts.push(PlanConflict {
                        key: key.clone(),
                        logical_path: display_logical.clone(),
                        kind: ConflictKind::RemovedVsModified,
                        base_path: Some(base_entry.logical.clone()),
                        ours_path: None,
                        theirs_path: Some(theirs_entry.logical.clone()),
                        base_sha: Some(base_entry.file.sha256.clone()),
                        ours_sha: None,
                        theirs_sha: Some(theirs_entry.file.sha256.clone()),
                        message: format!(
                            "You removed mod '{mod_id}' but the pack updated it ({} → {})",
                            base_entry.logical, theirs_entry.logical
                        ),
                        mod_id: Some(mod_id.clone()),
                    });
                }
            }
            (true, true, false) => {
                // Pack removed, user still has it (maybe disabled/edited).
                let base_entry = base_entry.unwrap();
                let ours_entry = ours_entry.unwrap();
                let content_changed = !base_entry
                    .file
                    .sha256
                    .eq_ignore_ascii_case(&ours_entry.file.sha256);
                let enabled_changed = !ours_entry.enabled;
                if !content_changed && !enabled_changed {
                    // Unchanged locally, pack removed → remove.
                    actions.push(PlanAction {
                        key: key.clone(),
                        logical_path: display_logical.clone(),
                        target_path: ours_entry.physical.clone(),
                        previous_path: None,
                        kind: PlanActionKind::Remove,
                        base_sha: Some(base_entry.file.sha256.clone()),
                        ours_sha: Some(ours_entry.file.sha256.clone()),
                        theirs_sha: None,
                        enabled: ours_entry.enabled,
                        mod_id: Some(mod_id.clone()),
                    });
                } else {
                    conflicts.push(PlanConflict {
                        key: key.clone(),
                        logical_path: display_logical.clone(),
                        kind: ConflictKind::ModifiedVsRemoved,
                        base_path: Some(base_entry.logical.clone()),
                        ours_path: Some(ours_entry.physical.clone()),
                        theirs_path: None,
                        base_sha: Some(base_entry.file.sha256.clone()),
                        ours_sha: Some(ours_entry.file.sha256.clone()),
                        theirs_sha: None,
                        message: format!(
                            "You modified mod '{mod_id}' ({}) but the pack removed it",
                            ours_entry.physical
                        ),
                        mod_id: Some(mod_id.clone()),
                    });
                }
            }
            (true, true, true) => {
                // All three present (maybe with renames).
                let base_entry = base_entry.unwrap();
                let ours_entry = ours_entry.unwrap();
                let theirs_entry = theirs_entry.unwrap();

                let local_content_changed = !base_entry
                    .file
                    .sha256
                    .eq_ignore_ascii_case(&ours_entry.file.sha256);
                let local_enabled_changed = !ours_entry.enabled;
                let pack_changed = !base_entry
                    .file
                    .sha256
                    .eq_ignore_ascii_case(&theirs_entry.file.sha256);
                let renamed = base_entry.logical != theirs_entry.logical;

                if !local_content_changed && !local_enabled_changed && !pack_changed {
                    // All identical.
                    actions.push(PlanAction {
                        key: key.clone(),
                        logical_path: display_logical.clone(),
                        target_path: ours_entry.physical.clone(),
                        previous_path: None,
                        kind: PlanActionKind::Keep,
                        base_sha: Some(base_entry.file.sha256.clone()),
                        ours_sha: Some(ours_entry.file.sha256.clone()),
                        theirs_sha: Some(theirs_entry.file.sha256.clone()),
                        enabled: true,
                        mod_id: Some(mod_id.clone()),
                    });
                } else if !local_content_changed && !local_enabled_changed && pack_changed {
                    // Unchanged locally, pack updated → take theirs (with rename if needed).
                    let kind = if renamed {
                        PlanActionKind::RenameUpdate
                    } else {
                        PlanActionKind::Update
                    };
                    let previous = if renamed {
                        Some(ours_entry.physical.clone())
                    } else {
                        None
                    };
                    actions.push(PlanAction {
                        key: key.clone(),
                        logical_path: theirs_entry.logical.clone(),
                        target_path: theirs_entry.logical.clone(),
                        previous_path: previous,
                        kind,
                        base_sha: Some(base_entry.file.sha256.clone()),
                        ours_sha: Some(ours_entry.file.sha256.clone()),
                        theirs_sha: Some(theirs_entry.file.sha256.clone()),
                        enabled: true,
                        mod_id: Some(mod_id.clone()),
                    });
                } else if (local_content_changed || local_enabled_changed) && !pack_changed {
                    // Locally modified (content or disabled), pack left alone → keep user's.
                    actions.push(PlanAction {
                        key: key.clone(),
                        logical_path: display_logical.clone(),
                        target_path: ours_entry.physical.clone(),
                        previous_path: None,
                        kind: PlanActionKind::Keep,
                        base_sha: Some(base_entry.file.sha256.clone()),
                        ours_sha: Some(ours_entry.file.sha256.clone()),
                        theirs_sha: Some(theirs_entry.file.sha256.clone()),
                        enabled: ours_entry.enabled,
                        mod_id: Some(mod_id.clone()),
                    });
                } else if local_content_changed && pack_changed {
                    // Both changed content.
                    if ours_entry
                        .file
                        .sha256
                        .eq_ignore_ascii_case(&theirs_entry.file.sha256)
                    {
                        // Converged to same new hash.
                        actions.push(PlanAction {
                            key: key.clone(),
                            logical_path: display_logical.clone(),
                            target_path: if ours_entry.enabled {
                                theirs_entry.logical.clone()
                            } else {
                                format!("{}{}", theirs_entry.logical, DISABLED_SUFFIX)
                            },
                            previous_path: if renamed {
                                Some(ours_entry.physical.clone())
                            } else {
                                None
                            },
                            kind: if renamed {
                                PlanActionKind::RenameUpdate
                            } else {
                                PlanActionKind::Keep
                            },
                            base_sha: Some(base_entry.file.sha256.clone()),
                            ours_sha: Some(ours_entry.file.sha256.clone()),
                            theirs_sha: Some(theirs_entry.file.sha256.clone()),
                            enabled: ours_entry.enabled,
                            mod_id: Some(mod_id.clone()),
                        });
                    } else {
                        conflicts.push(PlanConflict {
                            key: key.clone(),
                            logical_path: display_logical.clone(),
                            kind: ConflictKind::BothModified,
                            base_path: Some(base_entry.logical.clone()),
                            ours_path: Some(ours_entry.physical.clone()),
                            theirs_path: Some(theirs_entry.logical.clone()),
                            base_sha: Some(base_entry.file.sha256.clone()),
                            ours_sha: Some(ours_entry.file.sha256.clone()),
                            theirs_sha: Some(theirs_entry.file.sha256.clone()),
                            message: format!(
                                "You and the pack both modified mod '{mod_id}' differently"
                            ),
                            mod_id: Some(mod_id.clone()),
                        });
                    }
                } else if !local_content_changed && local_enabled_changed && pack_changed {
                    // Only enabled toggled locally, pack updated content → UpdateKeepDisabled (or rename variant).
                    let kind = if renamed {
                        PlanActionKind::RenameUpdateKeepDisabled
                    } else {
                        PlanActionKind::UpdateKeepDisabled
                    };
                    let target = format!("{}{}", theirs_entry.logical, DISABLED_SUFFIX);
                    let previous = if renamed {
                        Some(ours_entry.physical.clone())
                    } else {
                        None
                    };
                    // Even with rename, the previous physical is the disabled old file.
                    let prev = previous.or_else(|| Some(ours_entry.physical.clone()));
                    actions.push(PlanAction {
                        key: key.clone(),
                        logical_path: theirs_entry.logical.clone(),
                        target_path: target,
                        previous_path: prev,
                        kind,
                        base_sha: Some(base_entry.file.sha256.clone()),
                        ours_sha: Some(ours_entry.file.sha256.clone()),
                        theirs_sha: Some(theirs_entry.file.sha256.clone()),
                        enabled: false,
                        mod_id: Some(mod_id.clone()),
                    });
                } else {
                    unreachable!("unhandled mod-id case for {mod_id}");
                }
            }
        }
    }

    // Process path groups (non-mod-id files).
    for logical in &all_paths {
        let base_entry = base_parts.path_groups.get(logical);
        let theirs_entry = theirs_parts.path_groups.get(logical);
        let ours_entry = ours_parts.path_groups.get(logical);

        let key = logical.clone();
        let base_sha = base_entry.map(|e| e.file.sha256.as_str());
        let theirs_sha = theirs_entry.map(|e| e.file.sha256.as_str());
        let ours_sha = ours_entry.map(|e| e.file.sha256.as_str());

        if let Some(ours) = ours_entry {
            if ours.ambiguous {
                conflicts.push(PlanConflict {
                    key: key.clone(),
                    logical_path: logical.clone(),
                    kind: ConflictKind::AmbiguousDisabledPair,
                    base_path: base_entry.map(|e| e.logical.clone()),
                    ours_path: Some(ours.physical.clone()),
                    theirs_path: theirs_entry.map(|e| e.logical.clone()),
                    base_sha: base_sha.map(str::to_string),
                    ours_sha: ours_sha.map(str::to_string),
                    theirs_sha: theirs_sha.map(str::to_string),
                    message: format!("OURS has both enabled and disabled files for '{logical}'"),
                    mod_id: None,
                });
                continue;
            }
        }

        let base_present = base_entry.is_some();
        let theirs_present = theirs_entry.is_some();
        let ours_present = ours_entry.is_some();

        match (base_present, ours_present, theirs_present) {
            (false, false, false) => unreachable!(),
            (false, false, true) => {
                let theirs_entry = theirs_entry.unwrap();
                actions.push(PlanAction {
                    key: key.clone(),
                    logical_path: logical.clone(),
                    target_path: theirs_entry.logical.clone(),
                    previous_path: None,
                    kind: PlanActionKind::Add,
                    base_sha: None,
                    ours_sha: None,
                    theirs_sha: Some(theirs_entry.file.sha256.clone()),
                    enabled: true,
                    mod_id: None,
                });
            }
            (false, true, false) => {
                let ours_entry = ours_entry.unwrap();
                actions.push(PlanAction {
                    key: key.clone(),
                    logical_path: logical.clone(),
                    target_path: ours_entry.physical.clone(),
                    previous_path: None,
                    kind: PlanActionKind::KeepUserAdded,
                    base_sha: None,
                    ours_sha: Some(ours_entry.file.sha256.clone()),
                    theirs_sha: None,
                    enabled: ours_entry.enabled,
                    mod_id: None,
                });
            }
            (false, true, true) => {
                let ours_entry = ours_entry.unwrap();
                let theirs_entry = theirs_entry.unwrap();
                if ours_entry
                    .file
                    .sha256
                    .eq_ignore_ascii_case(&theirs_entry.file.sha256)
                    && ours_entry.enabled
                {
                    actions.push(PlanAction {
                        key: key.clone(),
                        logical_path: logical.clone(),
                        target_path: ours_entry.physical.clone(),
                        previous_path: None,
                        kind: PlanActionKind::Keep,
                        base_sha: None,
                        ours_sha: Some(ours_entry.file.sha256.clone()),
                        theirs_sha: Some(theirs_entry.file.sha256.clone()),
                        enabled: true,
                        mod_id: None,
                    });
                } else if ours_entry
                    .file
                    .sha256
                    .eq_ignore_ascii_case(&theirs_entry.file.sha256)
                    && !ours_entry.enabled
                {
                    actions.push(PlanAction {
                        key: key.clone(),
                        logical_path: logical.clone(),
                        target_path: ours_entry.physical.clone(),
                        previous_path: None,
                        kind: PlanActionKind::Keep,
                        base_sha: None,
                        ours_sha: Some(ours_entry.file.sha256.clone()),
                        theirs_sha: Some(theirs_entry.file.sha256.clone()),
                        enabled: false,
                        mod_id: None,
                    });
                } else {
                    conflicts.push(PlanConflict {
                        key: key.clone(),
                        logical_path: logical.clone(),
                        kind: if baseline_missing {
                            ConflictKind::NoBaseline
                        } else {
                            ConflictKind::AddedVsAdded
                        },
                        base_path: None,
                        ours_path: Some(ours_entry.physical.clone()),
                        theirs_path: Some(theirs_entry.logical.clone()),
                        base_sha: None,
                        ours_sha: Some(ours_entry.file.sha256.clone()),
                        theirs_sha: Some(theirs_entry.file.sha256.clone()),
                        message: if baseline_missing {
                            format!(
                                "Agora has no record of what this pack originally installed, so it cannot tell whether your '{logical}' is an edit or the pack's own file"
                            )
                        } else {
                            format!("Both you and the pack added '{logical}' with different content")
                        },
                        mod_id: None,
                    });
                }
            }
            (true, false, false) => {
                // Both removed.
            }
            (true, false, true) => {
                let base_entry = base_entry.unwrap();
                let theirs_entry = theirs_entry.unwrap();
                if base_entry
                    .file
                    .sha256
                    .eq_ignore_ascii_case(&theirs_entry.file.sha256)
                {
                    // Pack unchanged, user deletion kept.
                } else {
                    conflicts.push(PlanConflict {
                        key: key.clone(),
                        logical_path: logical.clone(),
                        kind: ConflictKind::RemovedVsModified,
                        base_path: Some(base_entry.logical.clone()),
                        ours_path: None,
                        theirs_path: Some(theirs_entry.logical.clone()),
                        base_sha: Some(base_entry.file.sha256.clone()),
                        ours_sha: None,
                        theirs_sha: Some(theirs_entry.file.sha256.clone()),
                        message: format!("You removed '{logical}' but the pack updated it"),
                        mod_id: None,
                    });
                }
            }
            (true, true, false) => {
                let base_entry = base_entry.unwrap();
                let ours_entry = ours_entry.unwrap();
                let content_changed = !base_entry
                    .file
                    .sha256
                    .eq_ignore_ascii_case(&ours_entry.file.sha256);
                let enabled_changed = !ours_entry.enabled;
                if !content_changed && !enabled_changed {
                    actions.push(PlanAction {
                        key: key.clone(),
                        logical_path: logical.clone(),
                        target_path: ours_entry.physical.clone(),
                        previous_path: None,
                        kind: PlanActionKind::Remove,
                        base_sha: Some(base_entry.file.sha256.clone()),
                        ours_sha: Some(ours_entry.file.sha256.clone()),
                        theirs_sha: None,
                        enabled: true,
                        mod_id: None,
                    });
                } else {
                    conflicts.push(PlanConflict {
                        key: key.clone(),
                        logical_path: logical.clone(),
                        kind: ConflictKind::ModifiedVsRemoved,
                        base_path: Some(base_entry.logical.clone()),
                        ours_path: Some(ours_entry.physical.clone()),
                        theirs_path: None,
                        base_sha: Some(base_entry.file.sha256.clone()),
                        ours_sha: Some(ours_entry.file.sha256.clone()),
                        theirs_sha: None,
                        message: format!("You modified '{logical}' but the pack removed it"),
                        mod_id: None,
                    });
                }
            }
            (true, true, true) => {
                let base_entry = base_entry.unwrap();
                let ours_entry = ours_entry.unwrap();
                let theirs_entry = theirs_entry.unwrap();

                let local_content_changed = !base_entry
                    .file
                    .sha256
                    .eq_ignore_ascii_case(&ours_entry.file.sha256);
                let local_enabled_changed = !ours_entry.enabled;
                let pack_changed = !base_entry
                    .file
                    .sha256
                    .eq_ignore_ascii_case(&theirs_entry.file.sha256);

                if !local_content_changed && !local_enabled_changed && !pack_changed {
                    actions.push(PlanAction {
                        key: key.clone(),
                        logical_path: logical.clone(),
                        target_path: ours_entry.physical.clone(),
                        previous_path: None,
                        kind: PlanActionKind::Keep,
                        base_sha: Some(base_entry.file.sha256.clone()),
                        ours_sha: Some(ours_entry.file.sha256.clone()),
                        theirs_sha: Some(theirs_entry.file.sha256.clone()),
                        enabled: true,
                        mod_id: None,
                    });
                } else if !local_content_changed && !local_enabled_changed && pack_changed {
                    actions.push(PlanAction {
                        key: key.clone(),
                        logical_path: logical.clone(),
                        target_path: theirs_entry.logical.clone(),
                        previous_path: None,
                        kind: PlanActionKind::Update,
                        base_sha: Some(base_entry.file.sha256.clone()),
                        ours_sha: Some(ours_entry.file.sha256.clone()),
                        theirs_sha: Some(theirs_entry.file.sha256.clone()),
                        enabled: true,
                        mod_id: None,
                    });
                } else if (local_content_changed || local_enabled_changed) && !pack_changed {
                    actions.push(PlanAction {
                        key: key.clone(),
                        logical_path: logical.clone(),
                        target_path: ours_entry.physical.clone(),
                        previous_path: None,
                        kind: PlanActionKind::Keep,
                        base_sha: Some(base_entry.file.sha256.clone()),
                        ours_sha: Some(ours_entry.file.sha256.clone()),
                        theirs_sha: Some(theirs_entry.file.sha256.clone()),
                        enabled: ours_entry.enabled,
                        mod_id: None,
                    });
                } else if local_content_changed && pack_changed {
                    if ours_entry
                        .file
                        .sha256
                        .eq_ignore_ascii_case(&theirs_entry.file.sha256)
                    {
                        actions.push(PlanAction {
                            key: key.clone(),
                            logical_path: logical.clone(),
                            target_path: ours_entry.physical.clone(),
                            previous_path: None,
                            kind: PlanActionKind::Keep,
                            base_sha: Some(base_entry.file.sha256.clone()),
                            ours_sha: Some(ours_entry.file.sha256.clone()),
                            theirs_sha: Some(theirs_entry.file.sha256.clone()),
                            enabled: ours_entry.enabled,
                            mod_id: None,
                        });
                    } else {
                        conflicts.push(PlanConflict {
                            key: key.clone(),
                            logical_path: logical.clone(),
                            kind: ConflictKind::BothModified,
                            base_path: Some(base_entry.logical.clone()),
                            ours_path: Some(ours_entry.physical.clone()),
                            theirs_path: Some(theirs_entry.logical.clone()),
                            base_sha: Some(base_entry.file.sha256.clone()),
                            ours_sha: Some(ours_entry.file.sha256.clone()),
                            theirs_sha: Some(theirs_entry.file.sha256.clone()),
                            message: format!(
                                "You and the pack both modified '{logical}' differently"
                            ),
                            mod_id: None,
                        });
                    }
                } else if !local_content_changed && local_enabled_changed && pack_changed {
                    // Enabled-only change + pack content update → UpdateKeepDisabled.
                    let target = format!("{}{}", logical, DISABLED_SUFFIX);
                    actions.push(PlanAction {
                        key: key.clone(),
                        logical_path: logical.clone(),
                        target_path: target,
                        previous_path: Some(ours_entry.physical.clone()),
                        kind: PlanActionKind::UpdateKeepDisabled,
                        base_sha: Some(base_entry.file.sha256.clone()),
                        ours_sha: Some(ours_entry.file.sha256.clone()),
                        theirs_sha: Some(theirs_entry.file.sha256.clone()),
                        enabled: false,
                        mod_id: None,
                    });
                } else {
                    unreachable!("unhandled path case for {logical}");
                }
            }
        }
    }

    actions.sort_by(|a, b| a.key.cmp(&b.key));
    conflicts.sort_by(|a, b| a.key.cmp(&b.key));
    let mut all_keys = BTreeSet::new();
    for a in &actions {
        all_keys.insert(a.key.clone());
    }
    for c in &conflicts {
        all_keys.insert(c.key.clone());
    }
    PackMergePlan {
        actions,
        conflicts,
        all_keys: all_keys.into_iter().collect(),
        baseline_missing,
    }
}

// ---------------------------------------------------------------------------
// Partitioning helpers
// ---------------------------------------------------------------------------

struct SideEntry {
    logical: String,
    file: InstancePackFile,
}

struct OursSideEntry {
    logical: String,
    physical: String,
    file: InstancePackFile,
    enabled: bool,
    ambiguous: bool,
}

struct PartitionedSide {
    path_groups: BTreeMap<String, SideEntry>,
    mod_groups: BTreeMap<String, SideEntry>,
    duplicate_mod_ids: BTreeSet<String>,
}

struct PartitionedOursSide {
    path_groups: BTreeMap<String, OursSideEntry>,
    mod_groups: BTreeMap<String, OursSideEntry>,
    duplicate_mod_ids: BTreeSet<String>,
}

fn is_mods_path(logical: &str) -> bool {
    logical.starts_with("mods/")
}

fn partition_side(
    files: &[InstancePackFile],
    mod_ids: Option<&HashMap<String, String>>,
) -> PartitionedSide {
    let mut path_groups: BTreeMap<String, SideEntry> = BTreeMap::new();
    let mut mod_groups: BTreeMap<String, SideEntry> = BTreeMap::new();
    let mut duplicate_mod_ids = BTreeSet::new();
    let mut seen_logical: BTreeSet<String> = BTreeSet::new();

    for file in files {
        let logical = logical_path(&file.relative_path).to_string();
        // Detect duplicate logical paths on same side (should not happen).
        // For BASE/THEIRS which are normalised, duplicates mean same logical
        // path appears twice — treat as last wins but it is not a planner
        // conflict; the caller should ensure inputs are deduplicated.
        if !seen_logical.insert(logical.clone()) {
            continue;
        }
        let mod_id_opt = mod_ids
            .and_then(|m| m.get(&logical))
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty());

        if let Some(mod_id) = mod_id_opt {
            if is_mods_path(&logical) {
                if mod_groups.contains_key(&mod_id) {
                    duplicate_mod_ids.insert(mod_id.clone());
                } else {
                    mod_groups.insert(
                        mod_id.clone(),
                        SideEntry {
                            logical: logical.clone(),
                            file: file.clone(),
                        },
                    );
                }
                continue;
            }
        }
        path_groups.insert(
            logical.clone(),
            SideEntry {
                logical,
                file: file.clone(),
            },
        );
    }
    PartitionedSide {
        path_groups,
        mod_groups,
        duplicate_mod_ids,
    }
}

fn partition_side_ours(
    files: &[InstancePackFile],
    mod_ids: Option<&HashMap<String, String>>,
) -> PartitionedOursSide {
    // OURS may have both `foo.jar` and `foo.jar.disabled` for same logical.
    // Group by logical and detect ambiguity.
    let mut by_logical: BTreeMap<String, Vec<InstancePackFile>> = BTreeMap::new();
    for file in files {
        let logical = logical_path(&file.relative_path).to_string();
        by_logical.entry(logical).or_default().push(file.clone());
    }

    let mut path_groups: BTreeMap<String, OursSideEntry> = BTreeMap::new();
    let mut mod_groups: BTreeMap<String, OursSideEntry> = BTreeMap::new();
    let mut duplicate_mod_ids = BTreeSet::new();

    for (logical, mut group) in by_logical {
        group.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
        let ambiguous = group.len() > 1;
        // Pick the disabled variant as representative if present? For
        // disabled logic, the enabled state is determined by whether the
        // logical's disabled physical exists. If both exist, it's ambiguous.
        // For non-ambiguous single entry, derive enabled correctly.
        let (physical, enabled, representative) = if ambiguous {
            // Take first as representative but mark ambiguous.
            let rep = group[0].clone();
            let phys = rep.relative_path.clone();
            let en = !is_disabled(&phys);
            (phys, en, rep)
        } else {
            let rep = group[0].clone();
            let phys = rep.relative_path.clone();
            let en = !is_disabled(&phys);
            (phys, en, rep)
        };

        let mod_id_opt = mod_ids
            .and_then(|m| m.get(&logical))
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty());

        let entry = OursSideEntry {
            logical: logical.clone(),
            physical,
            file: representative,
            enabled,
            ambiguous,
        };

        if let Some(mod_id) = mod_id_opt {
            if is_mods_path(&logical) {
                if let std::collections::btree_map::Entry::Occupied(_) =
                    mod_groups.entry(mod_id.clone())
                {
                    duplicate_mod_ids.insert(mod_id.clone());
                } else {
                    mod_groups.insert(mod_id, entry);
                }
                continue;
            }
        }
        path_groups.insert(logical, entry);
    }

    PartitionedOursSide {
        path_groups,
        mod_groups,
        duplicate_mod_ids,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::InstancePackFile;

    fn file(path: &str, sha: &str) -> InstancePackFile {
        InstancePackFile {
            relative_path: path.to_string(),
            sha256: sha.to_string(),
            size: 1,
        }
    }

    fn sha(c: char) -> String {
        std::iter::repeat_n(c, 64).collect()
    }

    #[test]
    fn user_added_mod_survives_pack_update() {
        // BASE has pack file A, THEIRS updates A and adds no new file for user-mod.
        // OURS has A (unchanged) plus user-added B.
        let base = vec![file("mods/packmod.jar", &sha('a'))];
        let theirs = vec![file("mods/packmod.jar", &sha('b'))]; // pack updated
        let ours = vec![
            file("mods/packmod.jar", &sha('a')),
            file("mods/usermod.jar", &sha('u')),
        ];
        let plan = plan_pack_update(&base, &theirs, &ours);
        assert!(
            plan.conflicts.is_empty(),
            "unexpected conflicts: {:?}",
            plan.conflicts
        );
        // packmod should be Update, usermod should be KeepUserAdded
        let pack_action = plan
            .actions
            .iter()
            .find(|a| a.logical_path == "mods/packmod.jar")
            .unwrap();
        assert_eq!(pack_action.kind, PlanActionKind::Update);
        let user_action = plan
            .actions
            .iter()
            .find(|a| a.logical_path == "mods/usermod.jar")
            .unwrap();
        assert_eq!(user_action.kind, PlanActionKind::KeepUserAdded);
        assert_eq!(user_action.target_path, "mods/usermod.jar");
    }

    #[test]
    fn user_added_disabled_mod_survives() {
        let base = vec![file("mods/packmod.jar", &sha('a'))];
        let theirs = vec![file("mods/packmod.jar", &sha('b'))];
        let ours = vec![
            file("mods/packmod.jar", &sha('a')),
            file("mods/usermod.jar.disabled", &sha('u')),
        ];
        let plan = plan_pack_update(&base, &theirs, &ours);
        assert!(plan.conflicts.is_empty());
        let user_action = plan
            .actions
            .iter()
            .find(|a| a.logical_path == "mods/usermod.jar")
            .unwrap();
        assert_eq!(user_action.kind, PlanActionKind::KeepUserAdded);
        assert!(!user_action.enabled);
        assert_eq!(user_action.target_path, "mods/usermod.jar.disabled");
    }

    #[test]
    fn disabled_pack_mod_keeps_disabled_on_update() {
        // BASE packmod A enabled, OURS disabled same hash, THEIRS B enabled → UpdateKeepDisabled
        let base = vec![file("mods/sodium.jar", &sha('a'))];
        let theirs = vec![file("mods/sodium.jar", &sha('b'))];
        let ours = vec![file("mods/sodium.jar.disabled", &sha('a'))];
        let plan = plan_pack_update(&base, &theirs, &ours);
        assert!(
            plan.conflicts.is_empty(),
            "should not conflict, got {:?}",
            plan.conflicts
        );
        let action = plan
            .actions
            .iter()
            .find(|a| a.logical_path == "mods/sodium.jar")
            .unwrap();
        assert_eq!(action.kind, PlanActionKind::UpdateKeepDisabled);
        assert!(!action.enabled);
        assert_eq!(action.target_path, "mods/sodium.jar.disabled");
        assert_eq!(action.theirs_sha.as_deref(), Some(sha('b').as_str()));
    }

    #[test]
    fn disabled_pack_mod_keeps_disabled_when_pack_unchanged() {
        let base = vec![file("mods/sodium.jar", &sha('a'))];
        let theirs = vec![file("mods/sodium.jar", &sha('a'))];
        let ours = vec![file("mods/sodium.jar.disabled", &sha('a'))];
        let plan = plan_pack_update(&base, &theirs, &ours);
        assert!(plan.conflicts.is_empty());
        let action = plan
            .actions
            .iter()
            .find(|a| a.logical_path == "mods/sodium.jar")
            .unwrap();
        assert_eq!(action.kind, PlanActionKind::Keep);
        assert!(!action.enabled);
    }

    #[test]
    fn unchanged_locally_pack_changed_takes_theirs() {
        let base = vec![file("config/foo.toml", &sha('a'))];
        let theirs = vec![file("config/foo.toml", &sha('b'))];
        let ours = vec![file("config/foo.toml", &sha('a'))];
        let plan = plan_pack_update(&base, &theirs, &ours);
        assert!(plan.conflicts.is_empty());
        let a = plan
            .actions
            .iter()
            .find(|a| a.logical_path == "config/foo.toml")
            .unwrap();
        assert_eq!(a.kind, PlanActionKind::Update);
    }

    #[test]
    fn unchanged_locally_pack_removed_removes() {
        let base = vec![file("config/foo.toml", &sha('a'))];
        let theirs = vec![];
        let ours = vec![file("config/foo.toml", &sha('a'))];
        let plan = plan_pack_update(&base, &theirs, &ours);
        assert!(plan.conflicts.is_empty());
        let a = plan
            .actions
            .iter()
            .find(|a| a.logical_path == "config/foo.toml")
            .unwrap();
        assert_eq!(a.kind, PlanActionKind::Remove);
    }

    #[test]
    fn changed_locally_pack_left_alone_keeps_user() {
        let base = vec![file("config/foo.toml", &sha('a'))];
        let theirs = vec![file("config/foo.toml", &sha('a'))];
        let ours = vec![file("config/foo.toml", &sha('x'))];
        let plan = plan_pack_update(&base, &theirs, &ours);
        assert!(plan.conflicts.is_empty());
        let a = plan
            .actions
            .iter()
            .find(|a| a.logical_path == "config/foo.toml")
            .unwrap();
        assert_eq!(a.kind, PlanActionKind::Keep);
        assert_eq!(a.ours_sha.as_deref(), Some(sha('x').as_str()));
    }

    #[test]
    fn both_modified_conflicts() {
        let base = vec![file("config/foo.toml", &sha('a'))];
        let theirs = vec![file("config/foo.toml", &sha('b'))];
        let ours = vec![file("config/foo.toml", &sha('x'))];
        let plan = plan_pack_update(&base, &theirs, &ours);
        assert_eq!(plan.conflicts.len(), 1);
        assert_eq!(plan.conflicts[0].kind, ConflictKind::BothModified);
        assert!(
            plan.actions.is_empty()
                || !plan
                    .actions
                    .iter()
                    .any(|a| a.logical_path == "config/foo.toml")
        );
    }

    #[test]
    fn both_modified_converged_no_conflict() {
        let base = vec![file("config/foo.toml", &sha('a'))];
        let theirs = vec![file("config/foo.toml", &sha('b'))];
        let ours = vec![file("config/foo.toml", &sha('b'))];
        let plan = plan_pack_update(&base, &theirs, &ours);
        assert!(plan.conflicts.is_empty());
        let a = plan
            .actions
            .iter()
            .find(|a| a.logical_path == "config/foo.toml")
            .unwrap();
        assert_eq!(a.kind, PlanActionKind::Keep);
    }

    #[test]
    fn modified_vs_removed_conflicts() {
        let base = vec![file("config/foo.toml", &sha('a'))];
        let theirs = vec![];
        let ours = vec![file("config/foo.toml", &sha('x'))];
        let plan = plan_pack_update(&base, &theirs, &ours);
        assert_eq!(plan.conflicts.len(), 1);
        assert_eq!(plan.conflicts[0].kind, ConflictKind::ModifiedVsRemoved);
    }

    #[test]
    fn removed_vs_modified_conflicts() {
        let base = vec![file("config/foo.toml", &sha('a'))];
        let theirs = vec![file("config/foo.toml", &sha('b'))];
        let ours: Vec<InstancePackFile> = vec![];
        let plan = plan_pack_update(&base, &theirs, &ours);
        assert_eq!(plan.conflicts.len(), 1);
        assert_eq!(plan.conflicts[0].kind, ConflictKind::RemovedVsModified);
    }

    #[test]
    fn added_vs_added_conflicts_when_different() {
        // A real baseline exists; `config/new.toml` is simply not in it.
        let base = vec![file("config/existing.toml", &sha('z'))];
        let theirs = vec![
            file("config/existing.toml", &sha('z')),
            file("config/new.toml", &sha('b')),
        ];
        let ours = vec![
            file("config/existing.toml", &sha('z')),
            file("config/new.toml", &sha('x')),
        ];
        let plan = plan_pack_update(&base, &theirs, &ours);
        assert_eq!(plan.conflicts.len(), 1);
        assert_eq!(plan.conflicts[0].kind, ConflictKind::AddedVsAdded);
    }

    #[test]
    fn added_vs_added_no_conflict_when_same_hash() {
        let base: Vec<InstancePackFile> = vec![];
        let theirs = vec![file("config/new.toml", &sha('b'))];
        let ours = vec![file("config/new.toml", &sha('b'))];
        let plan = plan_pack_update(&base, &theirs, &ours);
        assert!(plan.conflicts.is_empty());
    }

    #[test]
    fn both_removed_no_conflict() {
        let base = vec![file("config/foo.toml", &sha('a'))];
        let theirs: Vec<InstancePackFile> = vec![];
        let ours: Vec<InstancePackFile> = vec![];
        let plan = plan_pack_update(&base, &theirs, &ours);
        assert!(plan.conflicts.is_empty());
        assert!(plan.actions.is_empty());
    }

    #[test]
    fn user_deleted_pack_unchanged_keeps_absent() {
        let base = vec![file("config/foo.toml", &sha('a'))];
        let theirs = vec![file("config/foo.toml", &sha('a'))];
        let ours: Vec<InstancePackFile> = vec![];
        let plan = plan_pack_update(&base, &theirs, &ours);
        assert!(plan.conflicts.is_empty());
        assert!(plan.actions.is_empty()); // keep absent
    }

    #[test]
    fn in_theirs_not_base_absent_locally_adds() {
        let base: Vec<InstancePackFile> = vec![];
        let theirs = vec![file("config/new.toml", &sha('b'))];
        let ours: Vec<InstancePackFile> = vec![];
        let plan = plan_pack_update(&base, &theirs, &ours);
        assert!(plan.conflicts.is_empty());
        let a = plan
            .actions
            .iter()
            .find(|a| a.logical_path == "config/new.toml")
            .unwrap();
        assert_eq!(a.kind, PlanActionKind::Add);
    }

    #[test]
    fn renamed_jar_is_update_not_delete_plus_add_when_mod_ids_supplied() {
        // BASE sodium-0.5.jar (id sodium, hash A)
        // THEIRS sodium-0.6.jar (id sodium, hash B) — rename
        // OURS unchanged sodium-0.5.jar hash A
        let base = vec![file("mods/sodium-0.5.jar", &sha('a'))];
        let theirs = vec![file("mods/sodium-0.6.jar", &sha('b'))];
        let ours = vec![file("mods/sodium-0.5.jar", &sha('a'))];

        // Without mod ids, path-keyed sees delete+add.
        let plan_path = plan_pack_update(&base, &theirs, &ours);
        assert!(
            plan_path
                .actions
                .iter()
                .any(|a| a.kind == PlanActionKind::Remove),
            "without mod ids old path should be Remove"
        );
        assert!(
            plan_path
                .actions
                .iter()
                .any(|a| a.kind == PlanActionKind::Add),
            "without mod ids new path should be Add"
        );

        // With mod ids, it is a single rename update.
        let mut base_ids = HashMap::new();
        base_ids.insert("mods/sodium-0.5.jar".to_string(), "sodium".to_string());
        let mut theirs_ids = HashMap::new();
        theirs_ids.insert("mods/sodium-0.6.jar".to_string(), "sodium".to_string());
        let mut ours_ids = HashMap::new();
        ours_ids.insert("mods/sodium-0.5.jar".to_string(), "sodium".to_string());

        let plan_mod = plan_pack_update_with_mod_ids(
            &base,
            &theirs,
            &ours,
            Some(&base_ids),
            Some(&theirs_ids),
            Some(&ours_ids),
        );
        assert!(plan_mod.conflicts.is_empty());
        assert_eq!(plan_mod.actions.len(), 1);
        assert_eq!(plan_mod.actions[0].kind, PlanActionKind::RenameUpdate);
        assert_eq!(plan_mod.actions[0].target_path, "mods/sodium-0.6.jar");
        assert_eq!(
            plan_mod.actions[0].previous_path.as_deref(),
            Some("mods/sodium-0.5.jar")
        );
        assert_eq!(plan_mod.actions[0].mod_id.as_deref(), Some("sodium"));
    }

    #[test]
    fn renamed_jar_disabled_keeps_disabled_with_new_name() {
        let base = vec![file("mods/sodium-0.5.jar", &sha('a'))];
        let theirs = vec![file("mods/sodium-0.6.jar", &sha('b'))];
        let ours = vec![file("mods/sodium-0.5.jar.disabled", &sha('a'))];

        let mut base_ids = HashMap::new();
        base_ids.insert("mods/sodium-0.5.jar".to_string(), "sodium".to_string());
        let mut theirs_ids = HashMap::new();
        theirs_ids.insert("mods/sodium-0.6.jar".to_string(), "sodium".to_string());
        let mut ours_ids = HashMap::new();
        ours_ids.insert("mods/sodium-0.5.jar".to_string(), "sodium".to_string());

        let plan = plan_pack_update_with_mod_ids(
            &base,
            &theirs,
            &ours,
            Some(&base_ids),
            Some(&theirs_ids),
            Some(&ours_ids),
        );
        assert!(
            plan.conflicts.is_empty(),
            "got conflicts: {:?}",
            plan.conflicts
        );
        assert_eq!(plan.actions.len(), 1);
        assert_eq!(
            plan.actions[0].kind,
            PlanActionKind::RenameUpdateKeepDisabled
        );
        assert_eq!(plan.actions[0].target_path, "mods/sodium-0.6.jar.disabled");
        assert!(!plan.actions[0].enabled);
    }

    #[test]
    fn user_added_mod_with_same_mod_id_as_pack_conflicts() {
        // User added sodium-0.5.jar with id sodium, pack adds sodium-0.6.jar same id different hash.
        // The baseline is real but contains neither, so this is a true
        // added-vs-added rather than the baseline-missing case.
        let base = vec![file("config/pack.toml", &sha('z'))];
        let theirs = vec![
            file("config/pack.toml", &sha('z')),
            file("mods/sodium-0.6.jar", &sha('b')),
        ];
        let ours = vec![
            file("config/pack.toml", &sha('z')),
            file("mods/sodium-0.5.jar", &sha('a')),
        ];

        let mut theirs_ids = HashMap::new();
        theirs_ids.insert("mods/sodium-0.6.jar".to_string(), "sodium".to_string());
        let mut ours_ids = HashMap::new();
        ours_ids.insert("mods/sodium-0.5.jar".to_string(), "sodium".to_string());

        let plan = plan_pack_update_with_mod_ids(
            &base,
            &theirs,
            &ours,
            None,
            Some(&theirs_ids),
            Some(&ours_ids),
        );
        assert_eq!(plan.conflicts.len(), 1);
        assert_eq!(plan.conflicts[0].kind, ConflictKind::AddedVsAdded);
        assert_eq!(plan.conflicts[0].mod_id.as_deref(), Some("sodium"));
    }

    #[test]
    fn ambiguous_disabled_pair_conflicts() {
        // OURS has both enabled and disabled for same logical path.
        let base = vec![file("mods/foo.jar", &sha('a'))];
        let theirs = vec![file("mods/foo.jar", &sha('a'))];
        let ours = vec![
            file("mods/foo.jar", &sha('a')),
            file("mods/foo.jar.disabled", &sha('a')),
        ];
        let plan = plan_pack_update(&base, &theirs, &ours);
        assert_eq!(plan.conflicts.len(), 1);
        assert_eq!(plan.conflicts[0].kind, ConflictKind::AmbiguousDisabledPair);
    }

    #[test]
    fn an_empty_baseline_does_not_turn_the_whole_pack_into_conflicts() {
        // A pack installed before Agora recorded inventories has no BASE. Every
        // pack file is then "not in base but present locally", which the
        // ordinary rules read as two independent additions — turning a routine
        // update into a wall of conflicts the user cannot meaningfully answer.
        let ours = vec![
            file("mods/sodium-0.5.jar", &sha('a')),
            file("config/sodium.json", &sha('b')),
            file("mods/user-mod.jar", &sha('c')),
        ];
        let theirs = vec![
            file("mods/sodium-0.6.jar", &sha('d')),
            file("config/sodium.json", &sha('e')),
        ];
        let plan = plan_pack_update(&[], &theirs, &ours);
        assert!(plan.baseline_missing, "the caller has to be told why");
        // Still reported per file, but as one honest kind the UI can collapse
        // into a single question — never as "both of you added this", which
        // would be a plain untruth about a file the user never touched.
        assert!(
            plan.conflicts
                .iter()
                .all(|c| c.kind == ConflictKind::NoBaseline),
            "{:?}",
            plan.conflicts
        );
        // The user's own mod is still recognised and kept.
        assert!(plan
            .actions
            .iter()
            .any(|a| a.logical_path == "mods/user-mod.jar"));
    }

    #[test]
    fn a_present_baseline_is_never_reported_as_missing() {
        let base = vec![file("config/a.toml", &sha('a'))];
        let plan = plan_pack_update(&base, &base, &base);
        assert!(!plan.baseline_missing);
        assert!(plan.conflicts.is_empty());
    }

    #[test]
    fn a_brand_new_empty_instance_is_not_a_missing_baseline() {
        // Nothing installed yet, so an absent baseline is simply the truth.
        let theirs = vec![file("config/a.toml", &sha('a'))];
        let plan = plan_pack_update(&[], &theirs, &[]);
        assert!(!plan.baseline_missing);
        assert!(plan.conflicts.is_empty());
    }

    #[test]
    fn plan_is_deterministic_sorted() {
        let base = vec![file("mods/b.jar", &sha('a')), file("mods/a.jar", &sha('a'))];
        let theirs = vec![file("mods/b.jar", &sha('b')), file("mods/a.jar", &sha('a'))];
        let ours = vec![file("mods/b.jar", &sha('a')), file("mods/a.jar", &sha('a'))];
        let plan = plan_pack_update(&base, &theirs, &ours);
        let keys: Vec<&String> = plan.actions.iter().map(|a| &a.key).collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
    }
}
