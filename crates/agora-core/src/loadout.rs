use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

use crate::models::{InstalledMod, InstanceManifest};

/// Label of the undo snapshot `apply_loadout_profile` takes before it changes
/// anything. Retention keeps only the newest one, in its own slot, so these
/// never push out install or manual restore points (see `lkg::run_retention`).
pub const UNDO_SNAPSHOT_LABEL: &str = "before-loadout";

/// How many enabled items of each kind a profile holds, counted when it was
/// saved. Apply sets every item of every kind on or off to match the profile.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadoutCoverage {
    pub mods: usize,
    pub resourcepacks: usize,
    pub shaders: usize,
    pub datapacks: usize,
    pub worlds: usize,
}

/// One item a profile records, with the state it was saved in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadoutItem {
    pub filename: String,
    pub content_type: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadoutProfile {
    pub name: String,
    /// Names of the enabled items, across every kind. Kept so older readers
    /// and profiles saved before `items` existed still mean the same thing.
    pub enabled_mods: Vec<String>,
    pub created_at: String,
    /// Recorded at save time. Profiles saved before this field existed have
    /// none, so their per-kind counts are unknown.
    #[serde(default)]
    pub covers: Option<LoadoutCoverage>,
    /// Every item the profile records, on and off, with its kind. Apply only
    /// changes items listed here (plus mods, see `apply_profile_items`).
    /// Absent on profiles saved before this existed.
    #[serde(default)]
    pub items: Option<Vec<LoadoutItem>>,
}

/// Mods are every kind that is not a pack or a world, matching `content_subdir`.
fn is_mod_kind(content_type: &str) -> bool {
    !matches!(
        content_type,
        "resourcepack" | "shader" | "datapack" | "world"
    )
}

fn loadouts_dir(instance_dir: &Path) -> std::path::PathBuf {
    instance_dir.join(".agora_loadouts")
}

fn sanitize_profile_name(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            ':' | '/' | '\\' | '<' | '>' | '"' | '|' | '?' | '*' => '_',
            _ => c,
        })
        .collect()
}

fn content_subdir(content_type: &str) -> &str {
    match content_type {
        "resourcepack" => "resourcepacks",
        "shader" => "shaderpacks",
        "datapack" => "datapacks",
        "world" => "saves",
        _ => "mods",
    }
}

/// Read through the canonical loader so a profile application heals and
/// re-stamps the manifest exactly like every other write path. A local reader
/// here used to skip both, which meant toggling a loadout could quietly undo a
/// manifest heal.
fn read_manifest(instance_dir: &Path) -> Result<InstanceManifest, String> {
    let path = instance_dir.join("instance_manifest.json");
    crate::helpers::read_manifest(&path).map_err(|e| e.to_string())
}

fn write_manifest(instance_dir: &Path, manifest: &InstanceManifest) -> Result<(), String> {
    let path = instance_dir.join("instance_manifest.json");
    crate::helpers::atomic_write_manifest(&path, manifest).map_err(|e| e.to_string())
}

fn all_content_entries(manifest: &InstanceManifest) -> impl Iterator<Item = &InstalledMod> {
    manifest
        .mods
        .iter()
        .chain(manifest.resourcepacks.iter())
        .chain(manifest.shaders.iter())
        .chain(manifest.datapacks.iter())
        .chain(manifest.worlds.iter())
}

fn all_content_entries_mut(
    manifest: &mut InstanceManifest,
) -> impl Iterator<Item = &mut InstalledMod> {
    manifest
        .mods
        .iter_mut()
        .chain(manifest.resourcepacks.iter_mut())
        .chain(manifest.shaders.iter_mut())
        .chain(manifest.datapacks.iter_mut())
        .chain(manifest.worlds.iter_mut())
}

/// The profile that records the instance's state in `manifest` as it is now.
fn profile_from_manifest(
    manifest: &InstanceManifest,
    name: &str,
    created_at: String,
) -> LoadoutProfile {
    let mut covers = LoadoutCoverage::default();
    let mut enabled_mods: Vec<String> = Vec::new();
    let mut items: Vec<LoadoutItem> = Vec::new();
    for entry in all_content_entries(manifest) {
        items.push(LoadoutItem {
            filename: entry.filename.clone(),
            content_type: entry.content_type.clone(),
            enabled: entry.enabled,
        });
        if !entry.enabled {
            continue;
        }
        enabled_mods.push(entry.filename.clone());
        match entry.content_type.as_str() {
            "resourcepack" => covers.resourcepacks += 1,
            "shader" => covers.shaders += 1,
            "datapack" => covers.datapacks += 1,
            "world" => covers.worlds += 1,
            _ => covers.mods += 1,
        }
    }

    LoadoutProfile {
        name: name.to_string(),
        enabled_mods,
        created_at,
        covers: Some(covers),
        items: Some(items),
    }
}

fn write_profile(instance_dir: &Path, profile: &LoadoutProfile) -> Result<(), String> {
    let profiles_dir = loadouts_dir(instance_dir);
    fs::create_dir_all(&profiles_dir).map_err(|e| format!("Cannot create loadouts dir: {e}"))?;

    let safe_name = sanitize_profile_name(&profile.name);
    let profile_path = profiles_dir.join(format!("{safe_name}.json"));
    let json = serde_json::to_string_pretty(profile)
        .map_err(|e| format!("Cannot serialize profile: {e}"))?;
    fs::write(&profile_path, json).map_err(|e| format!("Cannot write profile: {e}"))
}

/// Create a new loadout profile from the current enabled state in the manifest.
pub fn create_profile(instance_dir: &Path, name: &str) -> Result<LoadoutProfile, String> {
    // A blank name would save as ".json", which list and apply cannot name.
    if name.trim().is_empty() {
        return Err("A loadout profile needs a name".into());
    }
    let manifest = read_manifest(instance_dir)?;
    let profile = profile_from_manifest(&manifest, name, Utc::now().to_rfc3339());
    write_profile(instance_dir, &profile)?;
    Ok(profile)
}

/// One thing that updating a profile to the instance's current setup changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadoutChange {
    pub filename: String,
    /// Empty for a profile saved before it recorded kinds, which cannot say.
    pub content_type: String,
    pub kind: LoadoutChangeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadoutChangeKind {
    /// On the instance, and the profile did not record it. It will record it
    /// as on or off, whichever it is now.
    Added { enabled: bool },
    /// The profile recorded it as off; it is on now.
    TurnedOn,
    /// The profile recorded it as on; it is off now.
    TurnedOff,
    /// The profile recorded it, and it is gone from the instance.
    Removed { enabled: bool },
}

/// The changes that [`update_profile`] would make, in the order the instance lists them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadoutUpdate {
    pub name: String,
    pub changes: Vec<LoadoutChange>,
}

/// Work out what updating `profile_name` to the instance's current setup would
/// change. Writes nothing.
fn diff_against_current(old: &LoadoutProfile, manifest: &InstanceManifest) -> Vec<LoadoutChange> {
    let mut changes = Vec::new();
    for entry in all_content_entries(manifest) {
        let kind = match recorded_state(old, entry) {
            None => LoadoutChangeKind::Added {
                enabled: entry.enabled,
            },
            Some(was) if was == entry.enabled => continue,
            Some(true) => LoadoutChangeKind::TurnedOff,
            Some(false) => LoadoutChangeKind::TurnedOn,
        };
        changes.push(LoadoutChange {
            filename: entry.filename.clone(),
            content_type: entry.content_type.clone(),
            kind,
        });
    }

    let on_instance = |filename: &str, content_type: Option<&str>| {
        all_content_entries(manifest)
            .any(|e| e.filename == filename && content_type.is_none_or(|t| e.content_type == t))
    };
    match &old.items {
        Some(items) => {
            for item in items {
                if !on_instance(&item.filename, Some(&item.content_type)) {
                    changes.push(LoadoutChange {
                        filename: item.filename.clone(),
                        content_type: item.content_type.clone(),
                        kind: LoadoutChangeKind::Removed {
                            enabled: item.enabled,
                        },
                    });
                }
            }
        }
        None => {
            for name in &old.enabled_mods {
                if !on_instance(name, None) {
                    changes.push(LoadoutChange {
                        filename: name.clone(),
                        content_type: String::new(),
                        kind: LoadoutChangeKind::Removed { enabled: true },
                    });
                }
            }
        }
    }
    changes
}

/// What [`update_profile`] would change, without writing anything. The
/// desktop shows this before it asks the user to confirm.
pub fn plan_profile_update(
    instance_dir: &Path,
    profile_name: &str,
) -> Result<LoadoutUpdate, String> {
    let old = read_profile(instance_dir, profile_name)?;
    let manifest = read_manifest(instance_dir)?;
    Ok(LoadoutUpdate {
        name: old.name.clone(),
        changes: diff_against_current(&old, &manifest),
    })
}

/// Save `profile_name` again so it records the instance's current setup: every
/// mod and pack on it, on or off, and nothing that is gone. The profile's
/// `created_at` stays as it was. Returns the changes it made.
pub fn update_profile(instance_dir: &Path, profile_name: &str) -> Result<LoadoutUpdate, String> {
    let old = read_profile(instance_dir, profile_name)?;
    let manifest = read_manifest(instance_dir)?;
    let changes = diff_against_current(&old, &manifest);
    let profile = profile_from_manifest(&manifest, &old.name, old.created_at.clone());
    write_profile(instance_dir, &profile)?;
    Ok(LoadoutUpdate {
        name: old.name,
        changes,
    })
}

/// List all loadout profiles for an instance.
pub fn list_profiles(instance_dir: &Path) -> Result<Vec<LoadoutProfile>, String> {
    let profiles_dir = loadouts_dir(instance_dir);
    if !profiles_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut profiles = Vec::new();
    let entries =
        fs::read_dir(&profiles_dir).map_err(|e| format!("Cannot read loadouts dir: {e}"))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("Cannot read entry: {e}"))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let content =
            fs::read_to_string(&path).map_err(|e| format!("Cannot read {:?}: {e}", path))?;
        let profile: LoadoutProfile =
            serde_json::from_str(&content).map_err(|e| format!("Invalid profile JSON: {e}"))?;
        profiles.push(profile);
    }

    profiles.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(profiles)
}

fn read_profile(instance_dir: &Path, profile_name: &str) -> Result<LoadoutProfile, String> {
    let safe_name = sanitize_profile_name(profile_name);
    let profile_path = loadouts_dir(instance_dir).join(format!("{safe_name}.json"));
    let content =
        fs::read_to_string(&profile_path).map_err(|e| format!("Cannot read profile: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("Invalid profile JSON: {e}"))
}

/// What applying a profile does with an enabled mod the profile does not list
/// (one installed after the profile was saved, say). Every caller passes one
/// explicitly: a silent default here would decide for the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UnlistedMods {
    /// Leave the unlisted mod on.
    Keep,
    /// Switch the unlisted mod off, as a profile is the whole mod set.
    Off,
}

/// The state a profile records for `entry`, or `None` when it does not list it.
fn recorded_state(profile: &LoadoutProfile, entry: &InstalledMod) -> Option<bool> {
    match &profile.items {
        Some(items) => items
            .iter()
            .find(|item| item.filename == entry.filename && item.content_type == entry.content_type)
            .map(|item| item.enabled),
        None => profile
            .enabled_mods
            .contains(&entry.filename)
            .then_some(true),
    }
}

/// Mod filenames that are on now but that the profile does not list. Applying
/// with [`UnlistedMods::Off`] would switch these off. Sorted, without duplicates.
pub fn unlisted_enabled_mods(
    instance_dir: &Path,
    profile_name: &str,
) -> Result<Vec<String>, String> {
    let profile = read_profile(instance_dir, profile_name)?;
    let manifest = read_manifest(instance_dir)?;
    let mut names: Vec<String> = all_content_entries(&manifest)
        .filter(|entry| {
            entry.enabled
                && is_mod_kind(&entry.content_type)
                && recorded_state(&profile, entry).is_none()
        })
        .map(|entry| entry.filename.clone())
        .collect();
    names.sort();
    names.dedup();
    Ok(names)
}

/// Apply a loadout profile: set each item it records to the state it records,
/// updating the manifest's `enabled` field and renaming files on disk
/// (`.ext` ↔ `.ext.disabled`) so the game does not load disabled content.
pub fn apply_profile(
    instance_dir: &Path,
    profile_name: &str,
    unlisted: UnlistedMods,
) -> Result<(), String> {
    let profile = read_profile(instance_dir, profile_name)?;
    apply_profile_items(instance_dir, &profile, unlisted)
}

/// Why applying a profile with its undo point failed. Each variant says what
/// state the instance is left in.
#[derive(Debug)]
pub enum LoadoutApplyError {
    /// The profile could not be read. Nothing was touched and no undo point was taken.
    Profile(String),
    /// The undo point could not be taken. Nothing was changed.
    UndoPoint(String),
    /// Applying failed. `rollback` says whether the undo point put the instance back.
    Apply {
        error: String,
        rollback: Result<(), String>,
    },
    /// The profile applied, but trimming old undo points failed.
    Retention(String),
}

impl std::fmt::Display for LoadoutApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Profile(error) => write!(f, "{error}"),
            Self::UndoPoint(error) => write!(
                f,
                "Could not create the required recovery snapshot, so nothing was changed: {error}"
            ),
            Self::Apply {
                error,
                rollback: Ok(()),
            } => write!(f, "Loadout application failed and was rolled back: {error}"),
            Self::Apply {
                error,
                rollback: Err(restore),
            } => write!(
                f,
                "Loadout application failed and rollback also failed: {error}; {restore}"
            ),
            Self::Retention(error) => write!(
                f,
                "Loadout applied, but old snapshots could not be trimmed: {error}"
            ),
        }
    }
}

/// Apply a profile with the safety net every adapter needs: take the undo
/// point first, roll back to it if applying fails, then trim old undo points.
/// The desktop and the CLI both call this.
pub fn apply_profile_with_undo(
    instance_dir: &Path,
    profile_name: &str,
    unlisted: UnlistedMods,
) -> Result<(), LoadoutApplyError> {
    let profile = read_profile(instance_dir, profile_name).map_err(LoadoutApplyError::Profile)?;
    let undo = crate::snapshot::create_snapshot(instance_dir, Some(UNDO_SNAPSHOT_LABEL))
        .map_err(LoadoutApplyError::UndoPoint)?;
    if let Err(error) = apply_profile_items(instance_dir, &profile, unlisted) {
        let rollback = crate::snapshot::restore_snapshot(instance_dir, &undo.id).map(|_| ());
        return Err(LoadoutApplyError::Apply { error, rollback });
    }
    crate::lkg::run_retention(instance_dir).map_err(LoadoutApplyError::Retention)
}

/// Set each item the profile records to the state it records.
///
/// Anything the profile does not record is left as it is, with one choice the
/// caller makes: a mod the profile does not list is either kept as it is or
/// switched off ([`UnlistedMods`]). Packs and worlds the profile does not list
/// always keep their current state. A profile saved before items were recorded
/// only knows which names were on, so those names are matched on their own.
fn apply_profile_items(
    instance_dir: &Path,
    profile: &LoadoutProfile,
    unlisted: UnlistedMods,
) -> Result<(), String> {
    let mut manifest = read_manifest(instance_dir)?;
    apply_enabled_flags(instance_dir, &mut manifest, |entry| {
        match recorded_state(profile, entry) {
            Some(enabled) => Some(enabled),
            None if is_mod_kind(&entry.content_type) && unlisted == UnlistedMods::Off => {
                Some(false)
            }
            None => None,
        }
    })
}

/// The renames a change has made so far, as `(from, to)`, in the order made.
/// Kept so a change that fails part-way can put each file back.
type RenameJournal = Vec<(std::path::PathBuf, std::path::PathBuf)>;

/// Enable or disable one entry: rename its file to match and update its flag.
/// A file that is missing on disk only has its flag changed. Each rename is
/// recorded in `journal` before the next step, so the caller can undo it.
fn set_entry_enabled(
    instance_dir: &Path,
    entry: &mut InstalledMod,
    should_enable: bool,
    journal: &mut RenameJournal,
) -> Result<(), String> {
    if entry.enabled == should_enable {
        return Ok(());
    }

    let subdir = content_subdir(&entry.content_type);
    let dir = instance_dir.join(subdir);
    let original = dir.join(&entry.filename);
    let disabled = dir.join(format!("{}.disabled", entry.filename));

    if should_enable {
        // Currently disabled → enable
        if disabled.exists() {
            fs::rename(&disabled, &original)
                .map_err(|e| format!("Cannot enable {}: {e}", entry.filename))?;
            journal.push((disabled, original));
        }
        entry.enabled = true;
    } else {
        // Currently enabled → disable
        if original.exists() {
            fs::rename(&original, &disabled)
                .map_err(|e| format!("Cannot disable {}: {e}", entry.filename))?;
            journal.push((original, disabled));
        }
        entry.enabled = false;
    }
    Ok(())
}

/// Set the enabled flag of every entry `decide` names (`None` leaves it alone),
/// renaming its file to match, and write the manifest.
///
/// All or nothing. If any rename or the manifest write fails, the renames
/// already made are undone in reverse, and `manifest` is left as it was, so the
/// files and the manifest still agree. Both the loadout and the bisect go
/// through here.
fn apply_enabled_flags(
    instance_dir: &Path,
    manifest: &mut InstanceManifest,
    decide: impl Fn(&InstalledMod) -> Option<bool>,
) -> Result<(), String> {
    let mut updated = manifest.clone();
    let mut journal = RenameJournal::new();
    let attempt = set_flags_recording_renames(instance_dir, &mut updated, &decide, &mut journal)
        .and_then(|()| write_manifest(instance_dir, &updated));
    match attempt {
        Ok(()) => {
            *manifest = updated;
            Ok(())
        }
        Err(error) => match undo_renames(&journal) {
            Ok(()) => Err(error),
            Err(undo) => Err(format!(
                "{error}. Putting back the files already renamed also failed, so some files may not match the manifest: {undo}"
            )),
        },
    }
}

fn set_flags_recording_renames(
    instance_dir: &Path,
    manifest: &mut InstanceManifest,
    decide: &impl Fn(&InstalledMod) -> Option<bool>,
    journal: &mut RenameJournal,
) -> Result<(), String> {
    for entry in all_content_entries_mut(manifest) {
        if let Some(should_enable) = decide(entry) {
            set_entry_enabled(instance_dir, entry, should_enable, journal)?;
        }
    }
    Ok(())
}

/// Rename every journalled file back, newest first. Every rename is attempted
/// even after one fails; the first failure is what is reported.
fn undo_renames(journal: &RenameJournal) -> Result<(), String> {
    let mut first_failure = None;
    for (from, to) in journal.iter().rev() {
        if let Err(error) = fs::rename(to, from) {
            first_failure.get_or_insert(format!(
                "cannot move {} back to {}: {error}",
                to.display(),
                from.display()
            ));
        }
    }
    match first_failure {
        None => Ok(()),
        Some(failure) => Err(failure),
    }
}

/// Make exactly `enabled` the enabled set, renaming jars and updating the
/// manifest to match.
///
/// Split out of [`apply_profile`] because a loadout profile is only one source
/// of an enabled set — the guided bisect drives the same mechanism from a set
/// it computes rather than one the user saved.
pub fn apply_enabled_set(instance_dir: &Path, enabled: &[String]) -> Result<(), String> {
    apply_enabled_set_scoped(instance_dir, enabled, None)
}

/// [`apply_enabled_set`] restricted to a single content type.
///
/// The unscoped form treats `enabled` as the complete picture of what should be
/// on, across mods, resource packs, shaders, data packs and worlds — so
/// anything absent from the list is turned *off*. That is right for a loadout
/// profile, which is captured from every content type at once, and wrong for
/// any caller that only knows about one of them: a mod-only list would silently
/// disable every resource pack in the instance.
///
/// Pass `Some("mod")` to leave the other content types exactly as they are.
///
/// All or nothing: see [`apply_enabled_flags`].
pub fn apply_enabled_set_scoped(
    instance_dir: &Path,
    enabled: &[String],
    only_content_type: Option<&str>,
) -> Result<(), String> {
    let mut manifest = read_manifest(instance_dir)?;
    let enabled_set: std::collections::HashSet<String> = enabled.iter().cloned().collect();

    apply_enabled_flags(instance_dir, &mut manifest, |entry| {
        if only_content_type.is_some_and(|wanted| entry.content_type != wanted) {
            None
        } else {
            Some(enabled_set.contains(&entry.filename))
        }
    })
}

/// Delete a loadout profile.
pub fn delete_profile(instance_dir: &Path, profile_name: &str) -> Result<(), String> {
    let profiles_dir = loadouts_dir(instance_dir);
    let safe_name = sanitize_profile_name(profile_name);
    let profile_path = profiles_dir.join(format!("{safe_name}.json"));

    if !profile_path.exists() {
        return Err(format!("Profile '{profile_name}' not found"));
    }

    fs::remove_file(&profile_path).map_err(|e| format!("Cannot delete profile: {e}"))?;

    if profiles_dir.exists() {
        let remaining: Vec<_> = fs::read_dir(&profiles_dir)
            .map_err(|e| format!("Cannot read loadouts dir: {e}"))?
            .filter_map(|e| e.ok())
            .collect();
        if remaining.is_empty() {
            let _ = fs::remove_dir(&profiles_dir);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::InstalledMod;

    fn make_manifest(_instance_dir: &Path, mod_files: &[&str]) -> InstanceManifest {
        let mods: Vec<InstalledMod> = mod_files
            .iter()
            .map(|f| InstalledMod {
                provider: None,
                update_pinned: false,
                pack_managed: false,
                installed_as_dependency: false,
                filename: f.to_string(),
                registry_id: None,
                modrinth_id: None,
                source: "test".to_string(),
                source_url: None,
                version: None,
                sha256: String::new(),
                hash_verified: true,
                installed_at: String::new(),
                java_packages: Vec::new(),
                mod_jar_id: None,
                depends_on: Vec::new(),
                optional_deps: Vec::new(),
                incompatible_deps: Vec::new(),
                provided_mod_ids: Vec::new(),
                enabled: true,
                content_type: "mod".to_string(),
            })
            .collect();
        InstanceManifest {
            manifest_version: crate::models::CURRENT_MANIFEST_VERSION,
            game_data: Default::default(),
            pack_origin: None,
            instance_id: "test".to_string(),
            name: "Test".to_string(),
            created_from_pack: None,
            minecraft_version: "1.21".to_string(),
            loader: "fabric".to_string(),
            loader_version: "0.16".to_string(),
            is_locked: false,
            mods,
            resourcepacks: Vec::new(),
            shaders: Vec::new(),
            datapacks: Vec::new(),
            worlds: Vec::new(),
            user_preferences: serde_json::json!({}),
        }
    }

    fn setup_instance(tmp: &tempfile::TempDir, mod_files: &[&str]) -> std::path::PathBuf {
        let dir = tmp.path().join("test-instance");
        let mods = dir.join("mods");
        fs::create_dir_all(&mods).unwrap();
        for f in mod_files {
            fs::write(mods.join(f), f).unwrap();
        }
        let manifest = make_manifest(&dir, mod_files);
        write_manifest(&dir, &manifest).unwrap();
        dir
    }

    #[test]
    fn test_create_profile_records_enabled_from_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = setup_instance(&tmp, &["sodium.jar", "lithium.jar", "phosphor.jar"]);

        let profile = create_profile(&dir, "test-profile").unwrap();
        assert_eq!(profile.name, "test-profile");
        assert_eq!(profile.enabled_mods.len(), 3);
        assert!(profile.enabled_mods.contains(&"sodium.jar".to_string()));
    }

    #[test]
    fn test_create_profile_skips_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = setup_instance(&tmp, &["sodium.jar", "lithium.jar", "phosphor.jar"]);

        // Manually disable one in the manifest
        let mut manifest = read_manifest(&dir).unwrap();
        manifest.mods[1].enabled = false;
        write_manifest(&dir, &manifest).unwrap();
        // Also rename file on disk to match disabled state
        fs::rename(
            dir.join("mods").join("lithium.jar"),
            dir.join("mods").join("lithium.jar.disabled"),
        )
        .unwrap();

        let profile = create_profile(&dir, "partial").unwrap();
        assert!(profile.enabled_mods.contains(&"sodium.jar".to_string()));
        assert!(!profile.enabled_mods.contains(&"lithium.jar".to_string()));
        assert!(profile.enabled_mods.contains(&"phosphor.jar".to_string()));
    }

    #[test]
    fn test_apply_profile_disables_and_updates_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = setup_instance(&tmp, &["sodium.jar", "lithium.jar", "phosphor.jar"]);

        create_profile(&dir, "full").unwrap();

        let minimal = LoadoutProfile {
            name: "minimal".to_string(),
            enabled_mods: vec!["sodium.jar".to_string()],
            created_at: Utc::now().to_rfc3339(),
            covers: None,
            items: None,
        };
        let profiles_dir = loadouts_dir(&dir);
        fs::create_dir_all(&profiles_dir).unwrap();
        fs::write(
            profiles_dir.join("minimal.json"),
            serde_json::to_string_pretty(&minimal).unwrap(),
        )
        .unwrap();

        apply_profile(&dir, "minimal", UnlistedMods::Off).unwrap();

        // Files on disk
        let mods = dir.join("mods");
        let entries: Vec<String> = fs::read_dir(&mods)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert!(entries.contains(&"sodium.jar".to_string()));
        assert!(entries.contains(&"lithium.jar.disabled".to_string()));
        assert!(entries.contains(&"phosphor.jar.disabled".to_string()));

        // Manifest updated
        let manifest = read_manifest(&dir).unwrap();
        assert!(manifest.mods[0].enabled); // sodium
        assert!(!manifest.mods[1].enabled); // lithium
        assert!(!manifest.mods[2].enabled); // phosphor
    }

    #[test]
    fn test_apply_profile_enables_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = setup_instance(&tmp, &["sodium.jar", "lithium.jar", "phosphor.jar"]);

        // Start with all disabled
        let mut manifest = read_manifest(&dir).unwrap();
        for m in &mut manifest.mods {
            m.enabled = false;
        }
        write_manifest(&dir, &manifest).unwrap();
        let mods_dir = dir.join("mods");
        fs::rename(
            mods_dir.join("sodium.jar"),
            mods_dir.join("sodium.jar.disabled"),
        )
        .unwrap();
        fs::rename(
            mods_dir.join("lithium.jar"),
            mods_dir.join("lithium.jar.disabled"),
        )
        .unwrap();
        fs::rename(
            mods_dir.join("phosphor.jar"),
            mods_dir.join("phosphor.jar.disabled"),
        )
        .unwrap();

        let profile = LoadoutProfile {
            name: "all-on".to_string(),
            enabled_mods: vec![
                "sodium.jar".to_string(),
                "lithium.jar".to_string(),
                "phosphor.jar".to_string(),
            ],
            created_at: Utc::now().to_rfc3339(),
            covers: None,
            items: None,
        };
        let profiles_dir = loadouts_dir(&dir);
        fs::create_dir_all(&profiles_dir).unwrap();
        fs::write(
            profiles_dir.join("all-on.json"),
            serde_json::to_string_pretty(&profile).unwrap(),
        )
        .unwrap();

        apply_profile(&dir, "all-on", UnlistedMods::Off).unwrap();

        let mods = dir.join("mods");
        let entries: Vec<String> = fs::read_dir(&mods)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert!(entries.contains(&"sodium.jar".to_string()));
        assert!(entries.contains(&"lithium.jar".to_string()));
        assert!(entries.contains(&"phosphor.jar".to_string()));
        assert!(!entries.iter().any(|n| n.ends_with(".disabled")));

        let manifest = read_manifest(&dir).unwrap();
        assert!(manifest.mods.iter().all(|m| m.enabled));
    }

    #[test]
    fn scoped_apply_leaves_other_content_types_alone() {
        // A bisect only ever knows about mods, so its enabled list says nothing
        // about shaders. Unscoped, the absence of "bsl.zip" reads as "turn it
        // off" and the shader is disabled as collateral — and the cancel path,
        // restoring the same mod-only baseline, never puts it back.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("test-instance");
        fs::create_dir_all(dir.join("mods")).unwrap();
        fs::create_dir_all(dir.join("shaderpacks")).unwrap();
        fs::write(dir.join("mods").join("sodium.jar"), b"sodium").unwrap();
        fs::write(dir.join("mods").join("iris.jar"), b"iris").unwrap();
        fs::write(dir.join("shaderpacks").join("bsl.zip"), b"bsl").unwrap();

        let mut manifest = make_manifest(&dir, &["sodium.jar", "iris.jar"]);
        let mut shader = manifest.mods[0].clone();
        shader.filename = "bsl.zip".to_string();
        shader.content_type = "shader".to_string();
        manifest.shaders.push(shader);
        write_manifest(&dir, &manifest).unwrap();

        // Disable iris, the way a bisect trial would.
        apply_enabled_set_scoped(&dir, &["sodium.jar".to_string()], Some("mod")).unwrap();

        let after = read_manifest(&dir).unwrap();
        assert!(
            !after
                .mods
                .iter()
                .find(|m| m.filename == "iris.jar")
                .unwrap()
                .enabled
        );
        assert!(
            after.shaders[0].enabled,
            "the shader was not the bisect's business"
        );
        assert!(
            dir.join("shaderpacks").join("bsl.zip").exists(),
            "shader file must not have been renamed to .disabled"
        );

        // And the unscoped form still behaves as a loadout needs it to.
        apply_enabled_set(&dir, &["sodium.jar".to_string()]).unwrap();
        let after_unscoped = read_manifest(&dir).unwrap();
        assert!(!after_unscoped.shaders[0].enabled);
    }

    #[test]
    fn test_profile_applies_content_type_aware() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("test-instance");
        fs::create_dir_all(dir.join("mods")).unwrap();
        fs::create_dir_all(dir.join("shaderpacks")).unwrap();
        fs::write(dir.join("mods").join("sodium.jar"), b"sodium").unwrap();
        fs::write(dir.join("shaderpacks").join("bsl.zip"), b"bsl").unwrap();

        let mut manifest = make_manifest(&dir, &["sodium.jar"]);
        manifest.shaders.push(InstalledMod {
            provider: None,
            update_pinned: false,
            pack_managed: false,
            installed_as_dependency: false,
            filename: "bsl.zip".to_string(),
            registry_id: None,
            modrinth_id: None,
            source: "test".to_string(),
            source_url: None,
            version: None,
            sha256: String::new(),
            hash_verified: true,
            installed_at: String::new(),
            java_packages: Vec::new(),
            mod_jar_id: None,
            depends_on: Vec::new(),
            optional_deps: Vec::new(),
            incompatible_deps: Vec::new(),
            provided_mod_ids: Vec::new(),
            enabled: true,
            content_type: "shader".to_string(),
        });
        write_manifest(&dir, &manifest).unwrap();

        let profile = LoadoutProfile {
            name: "no-shaders".to_string(),
            enabled_mods: vec!["sodium.jar".to_string()],
            created_at: Utc::now().to_rfc3339(),
            covers: None,
            items: None,
        };
        let profiles_dir = loadouts_dir(&dir);
        fs::create_dir_all(&profiles_dir).unwrap();
        fs::write(
            profiles_dir.join("no-shaders.json"),
            serde_json::to_string_pretty(&profile).unwrap(),
        )
        .unwrap();

        apply_profile(&dir, "no-shaders", UnlistedMods::Off).unwrap();

        // A pack the profile does not name is not the profile's business: the
        // shader stays on disk and enabled.
        let shader_dir = dir.join("shaderpacks");
        let entries: Vec<String> = fs::read_dir(&shader_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert!(entries.contains(&"bsl.zip".to_string()));
        assert!(!entries.contains(&"bsl.zip.disabled".to_string()));

        let manifest = read_manifest(&dir).unwrap();
        assert!(manifest.mods[0].enabled); // sodium still on
        assert!(manifest.shaders[0].enabled); // bsl left alone
    }

    /// A minimal entry of the given kind, enabled, for hand-built manifests.
    fn item(filename: &str, content_type: &str) -> InstalledMod {
        let mut entry = make_manifest(Path::new("."), &["x"]).mods.remove(0);
        entry.filename = filename.to_string();
        entry.content_type = content_type.to_string();
        entry.enabled = true;
        entry
    }

    #[test]
    fn apply_leaves_worlds_and_packs_added_after_the_profile_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = setup_instance(&tmp, &["sodium.jar"]);
        create_profile(&dir, "base").unwrap();

        // Added after the profile was saved: a world, a shader and a mod.
        fs::create_dir_all(dir.join("saves")).unwrap();
        fs::create_dir_all(dir.join("shaderpacks")).unwrap();
        fs::write(dir.join("saves").join("Survival"), b"world").unwrap();
        fs::write(dir.join("shaderpacks").join("bsl.zip"), b"bsl").unwrap();
        fs::write(dir.join("mods").join("added.jar"), b"added").unwrap();
        let mut manifest = read_manifest(&dir).unwrap();
        manifest.worlds.push(item("Survival", "world"));
        manifest.shaders.push(item("bsl.zip", "shader"));
        manifest.mods.push(item("added.jar", "mod"));
        write_manifest(&dir, &manifest).unwrap();

        apply_profile(&dir, "base", UnlistedMods::Off).unwrap();

        let after = read_manifest(&dir).unwrap();
        assert!(
            after.worlds[0].enabled,
            "a world the profile did not record stays on"
        );
        assert!(dir.join("saves").join("Survival").exists());
        assert!(
            after.shaders[0].enabled,
            "a pack the profile did not record stays on"
        );
        assert!(dir.join("shaderpacks").join("bsl.zip").exists());
        // The one deliberate exception: an unlisted mod is switched off.
        let added = after
            .mods
            .iter()
            .find(|m| m.filename == "added.jar")
            .unwrap();
        assert!(
            !added.enabled,
            "unlisted mods are switched off, as loadouts always did"
        );
        assert!(dir.join("mods").join("added.jar.disabled").exists());
    }

    #[test]
    fn apply_switches_off_a_pack_the_profile_recorded_off() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("test-instance");
        fs::create_dir_all(dir.join("mods")).unwrap();
        fs::create_dir_all(dir.join("shaderpacks")).unwrap();
        fs::write(dir.join("mods").join("sodium.jar"), b"sodium").unwrap();
        fs::write(dir.join("shaderpacks").join("bsl.zip.disabled"), b"bsl").unwrap();
        let mut manifest = make_manifest(&dir, &["sodium.jar"]);
        let mut shader = item("bsl.zip", "shader");
        shader.enabled = false;
        manifest.shaders.push(shader);
        write_manifest(&dir, &manifest).unwrap();
        // Recorded with the shader off.
        create_profile(&dir, "no-shader").unwrap();

        // The user later turns the shader on.
        fs::rename(
            dir.join("shaderpacks").join("bsl.zip.disabled"),
            dir.join("shaderpacks").join("bsl.zip"),
        )
        .unwrap();
        let mut manifest = read_manifest(&dir).unwrap();
        manifest.shaders[0].enabled = true;
        write_manifest(&dir, &manifest).unwrap();

        apply_profile(&dir, "no-shader", UnlistedMods::Off).unwrap();

        assert!(dir.join("shaderpacks").join("bsl.zip.disabled").exists());
        assert!(!read_manifest(&dir).unwrap().shaders[0].enabled);
    }

    #[test]
    fn apply_matches_items_by_kind_as_well_as_name() {
        // A mod and a shader can share a filename. Matching by name alone would
        // give the shader the mod's recorded state.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("test-instance");
        fs::create_dir_all(dir.join("mods")).unwrap();
        fs::create_dir_all(dir.join("shaderpacks")).unwrap();
        fs::write(dir.join("mods").join("dup.zip"), b"mod").unwrap();
        fs::write(dir.join("shaderpacks").join("dup.zip.disabled"), b"shader").unwrap();
        let mut manifest = make_manifest(&dir, &["dup.zip"]);
        let mut shader = item("dup.zip", "shader");
        shader.enabled = false;
        manifest.shaders.push(shader);
        write_manifest(&dir, &manifest).unwrap();
        create_profile(&dir, "kinds").unwrap();

        // Turn the shader on, then apply: the shader must go back off.
        fs::rename(
            dir.join("shaderpacks").join("dup.zip.disabled"),
            dir.join("shaderpacks").join("dup.zip"),
        )
        .unwrap();
        let mut manifest = read_manifest(&dir).unwrap();
        manifest.shaders[0].enabled = true;
        write_manifest(&dir, &manifest).unwrap();

        apply_profile(&dir, "kinds", UnlistedMods::Off).unwrap();

        let after = read_manifest(&dir).unwrap();
        assert!(after.mods[0].enabled, "the mod recorded on stays on");
        assert!(
            !after.shaders[0].enabled,
            "the shader recorded off goes off"
        );
        assert!(dir.join("shaderpacks").join("dup.zip.disabled").exists());
    }

    #[test]
    fn apply_with_undo_takes_an_undo_point_that_restores_the_pre_apply_state() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = setup_instance(&tmp, &["sodium.jar", "lithium.jar"]);
        create_profile(&dir, "all-on").unwrap();

        // Turn lithium off by hand, then apply the all-on profile.
        fs::rename(
            dir.join("mods").join("lithium.jar"),
            dir.join("mods").join("lithium.jar.disabled"),
        )
        .unwrap();
        let mut manifest = read_manifest(&dir).unwrap();
        manifest.mods[1].enabled = false;
        write_manifest(&dir, &manifest).unwrap();

        apply_profile_with_undo(&dir, "all-on", UnlistedMods::Off).unwrap();
        assert!(dir.join("mods").join("lithium.jar").exists());

        let snapshots = crate::snapshot::list_snapshots(&dir).unwrap();
        let undo = snapshots
            .iter()
            .find(|s| s.label.as_deref() == Some(UNDO_SNAPSHOT_LABEL))
            .expect("apply must save an undo point labelled before-loadout");
        crate::snapshot::restore_snapshot(&dir, &undo.id).unwrap();

        assert!(
            dir.join("mods").join("lithium.jar.disabled").exists(),
            "the undo point brings back the state before the apply"
        );
        assert!(!read_manifest(&dir).unwrap().mods[1].enabled);
    }

    #[test]
    fn apply_with_undo_refuses_an_unknown_profile_before_taking_an_undo_point() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = setup_instance(&tmp, &["sodium.jar"]);

        let err = apply_profile_with_undo(&dir, "missing", UnlistedMods::Off)
            .expect_err("unknown profile");
        assert!(matches!(err, LoadoutApplyError::Profile(_)), "got {err:?}");
        assert!(crate::snapshot::list_snapshots(&dir).unwrap().is_empty());
    }

    #[test]
    fn test_list_profiles_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("empty");
        let profiles = list_profiles(&dir).unwrap();
        assert!(profiles.is_empty());
    }

    #[test]
    fn test_create_list_delete_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = setup_instance(&tmp, &["sodium.jar"]);

        create_profile(&dir, "alpha").unwrap();
        create_profile(&dir, "beta").unwrap();

        let profiles = list_profiles(&dir).unwrap();
        assert_eq!(profiles.len(), 2);

        delete_profile(&dir, "alpha").unwrap();
        let profiles = list_profiles(&dir).unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].name, "beta");

        delete_profile(&dir, "beta").unwrap();
        let profiles = list_profiles(&dir).unwrap();
        assert!(profiles.is_empty());
    }

    #[test]
    fn create_profile_records_enabled_items_per_kind() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("test-instance");
        fs::create_dir_all(dir.join("mods")).unwrap();
        fs::create_dir_all(dir.join("shaderpacks")).unwrap();
        fs::create_dir_all(dir.join("resourcepacks")).unwrap();

        let mut manifest = make_manifest(&dir, &["sodium.jar", "iris.jar"]);
        manifest.mods[1].enabled = false;
        let extra = |filename: &str, content_type: &str, enabled: bool| {
            let mut item = manifest.mods[0].clone();
            item.filename = filename.to_string();
            item.content_type = content_type.to_string();
            item.enabled = enabled;
            item
        };
        manifest.shaders.push(extra("bsl.zip", "shader", true));
        manifest.shaders.push(extra("off.zip", "shader", false));
        manifest
            .resourcepacks
            .push(extra("faithful.zip", "resourcepack", true));
        manifest.worlds.push(extra("Survival", "world", true));
        write_manifest(&dir, &manifest).unwrap();

        let profile = create_profile(&dir, "kinds").unwrap();

        let covers = profile.covers.expect("new profiles record coverage");
        assert_eq!(
            covers,
            LoadoutCoverage {
                mods: 1,
                resourcepacks: 1,
                shaders: 1,
                datapacks: 0,
                worlds: 1,
            }
        );
        assert_eq!(
            profile.enabled_mods.len(),
            4,
            "disabled items are not stored"
        );
        assert!(!profile.enabled_mods.contains(&"iris.jar".to_string()));
        assert!(!profile.enabled_mods.contains(&"off.zip".to_string()));
    }

    #[test]
    fn profile_saved_before_coverage_loads_with_none() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("legacy");
        let profiles_dir = loadouts_dir(&dir);
        fs::create_dir_all(&profiles_dir).unwrap();
        fs::write(
            profiles_dir.join("old.json"),
            r#"{"name":"old","enabled_mods":["a.jar"],"created_at":"2026-01-01T00:00:00Z"}"#,
        )
        .unwrap();

        let profiles = list_profiles(&dir).unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].enabled_mods, vec!["a.jar".to_string()]);
        assert!(profiles[0].covers.is_none(), "unknown counts stay unknown");
    }

    #[test]
    fn create_profile_rejects_blank_names() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = setup_instance(&tmp, &["sodium.jar"]);

        for blank in ["", "   ", "\t"] {
            let err = create_profile(&dir, blank).expect_err("blank name must be refused");
            assert!(err.contains("needs a name"), "unexpected error: {err}");
        }
        assert!(list_profiles(&dir).unwrap().is_empty());
        assert!(
            !loadouts_dir(&dir).exists(),
            "a refused profile must not leave a loadouts folder behind"
        );
    }

    #[test]
    fn test_delete_profile_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("no-profiles");
        let result = delete_profile(&dir, "nonexistent");
        assert!(result.is_err());
    }

    /// A mod installed after the profile was saved: its jar is in `mods/`
    /// (or parked as `.disabled`), and the manifest records it.
    fn install_after_profile(dir: &Path, filename: &str, enabled: bool) {
        let mut manifest = read_manifest(dir).unwrap();
        let mut item = manifest.mods[0].clone();
        item.filename = filename.to_string();
        item.enabled = enabled;
        manifest.mods.push(item);
        write_manifest(dir, &manifest).unwrap();
        let on_disk = if enabled {
            filename.to_string()
        } else {
            format!("{filename}.disabled")
        };
        fs::write(dir.join("mods").join(on_disk), filename).unwrap();
    }

    fn save_legacy_profile(dir: &Path, name: &str, enabled_mods: &[&str]) {
        let profile = LoadoutProfile {
            name: name.to_string(),
            enabled_mods: enabled_mods.iter().map(|m| m.to_string()).collect(),
            created_at: Utc::now().to_rfc3339(),
            covers: None,
            items: None,
        };
        let profiles_dir = loadouts_dir(dir);
        fs::create_dir_all(&profiles_dir).unwrap();
        fs::write(
            profiles_dir.join(format!("{name}.json")),
            serde_json::to_string_pretty(&profile).unwrap(),
        )
        .unwrap();
    }

    fn is_on(dir: &Path, filename: &str) -> bool {
        dir.join("mods").join(filename).exists()
    }

    #[test]
    fn a_mod_the_profile_does_not_list_follows_the_caller_choice() {
        for (choice, expect_on) in [(UnlistedMods::Keep, true), (UnlistedMods::Off, false)] {
            let tmp = tempfile::tempdir().unwrap();
            let dir = setup_instance(&tmp, &["sodium.jar", "lithium.jar"]);
            create_profile(&dir, "both").unwrap();
            install_after_profile(&dir, "newer.jar", true);

            apply_profile(&dir, "both", choice).unwrap();

            assert_eq!(is_on(&dir, "newer.jar"), expect_on, "{choice:?}");
            let entry = read_manifest(&dir)
                .unwrap()
                .mods
                .into_iter()
                .find(|m| m.filename == "newer.jar")
                .unwrap();
            assert_eq!(
                entry.enabled, expect_on,
                "manifest must match disk for {choice:?}"
            );
            assert!(
                is_on(&dir, "lithium.jar"),
                "a listed mod follows the profile"
            );
        }
    }

    #[test]
    fn a_legacy_profile_follows_the_caller_choice_for_unlisted_mods() {
        for (choice, expect_on) in [(UnlistedMods::Keep, true), (UnlistedMods::Off, false)] {
            let tmp = tempfile::tempdir().unwrap();
            let dir = setup_instance(&tmp, &["sodium.jar", "lithium.jar"]);
            save_legacy_profile(&dir, "sodium-only", &["sodium.jar"]);

            apply_profile(&dir, "sodium-only", choice).unwrap();

            assert!(is_on(&dir, "sodium.jar"));
            assert_eq!(is_on(&dir, "lithium.jar"), expect_on, "{choice:?}");
        }
    }

    #[test]
    fn unlisted_enabled_mods_names_only_what_off_would_switch_off() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = setup_instance(&tmp, &["sodium.jar", "lithium.jar"]);
        create_profile(&dir, "both").unwrap();
        install_after_profile(&dir, "newer.jar", true);
        install_after_profile(&dir, "parked.jar", false);

        assert_eq!(
            unlisted_enabled_mods(&dir, "both").unwrap(),
            vec!["newer.jar".to_string()],
            "a parked mod is off already, and a listed mod is not unlisted"
        );
        assert!(
            unlisted_enabled_mods(&dir, "no-such-profile").is_err(),
            "an unknown profile is an error, not an empty list"
        );
    }

    #[test]
    fn unlisted_enabled_mods_is_empty_when_the_profile_lists_everything_on() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = setup_instance(&tmp, &["sodium.jar"]);
        create_profile(&dir, "all").unwrap();

        assert!(unlisted_enabled_mods(&dir, "all").unwrap().is_empty());
    }

    /// A profile saved, then the instance changed: one mod turned off, one
    /// installed since, one removed. Updating records the setup as it is now.
    #[test]
    fn update_records_the_current_setup_and_reports_each_change() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = setup_instance(&tmp, &["sodium.jar", "lithium.jar"]);
        let saved = create_profile(&dir, "both").unwrap();

        // The user turns lithium off, installs newer.jar, and removes sodium.
        fs::rename(
            dir.join("mods").join("lithium.jar"),
            dir.join("mods").join("lithium.jar.disabled"),
        )
        .unwrap();
        fs::remove_file(dir.join("mods").join("sodium.jar")).unwrap();
        fs::write(dir.join("mods").join("newer.jar"), b"newer").unwrap();
        let mut manifest = read_manifest(&dir).unwrap();
        manifest.mods[1].enabled = false;
        manifest.mods.remove(0);
        let mut newer = manifest.mods[0].clone();
        newer.filename = "newer.jar".to_string();
        newer.enabled = true;
        manifest.mods.push(newer);
        write_manifest(&dir, &manifest).unwrap();

        // Before the update, Apply would ask about newer.jar every time.
        assert_eq!(
            unlisted_enabled_mods(&dir, "both").unwrap(),
            vec!["newer.jar".to_string()]
        );

        let planned = plan_profile_update(&dir, "both").unwrap();
        let expected = vec![
            LoadoutChange {
                filename: "lithium.jar".into(),
                content_type: "mod".into(),
                kind: LoadoutChangeKind::TurnedOff,
            },
            LoadoutChange {
                filename: "newer.jar".into(),
                content_type: "mod".into(),
                kind: LoadoutChangeKind::Added { enabled: true },
            },
            LoadoutChange {
                filename: "sodium.jar".into(),
                content_type: "mod".into(),
                kind: LoadoutChangeKind::Removed { enabled: true },
            },
        ];
        assert_eq!(planned.changes, expected);
        assert_eq!(planned.name, "both");

        // Planning writes nothing.
        let still = list_profiles(&dir).unwrap();
        assert_eq!(still[0].enabled_mods, saved.enabled_mods);

        let done = update_profile(&dir, "both").unwrap();
        assert_eq!(done.changes, expected);

        let after = list_profiles(&dir).unwrap();
        assert_eq!(
            after.len(),
            1,
            "updating replaces the profile, it does not add one"
        );
        let after = &after[0];
        assert_eq!(after.created_at, saved.created_at, "created_at is kept");
        assert_eq!(after.enabled_mods, vec!["newer.jar".to_string()]);
        let items = after.items.as_ref().unwrap();
        assert_eq!(
            items,
            &vec![
                LoadoutItem {
                    filename: "lithium.jar".into(),
                    content_type: "mod".into(),
                    enabled: false,
                },
                LoadoutItem {
                    filename: "newer.jar".into(),
                    content_type: "mod".into(),
                    enabled: true,
                },
            ]
        );
        assert!(
            unlisted_enabled_mods(&dir, "both").unwrap().is_empty(),
            "after an update Apply has nothing new to ask about"
        );
        // And a second update with nothing changed reports nothing.
        assert!(update_profile(&dir, "both").unwrap().changes.is_empty());
    }

    #[test]
    fn update_of_an_unknown_profile_is_an_error_and_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = setup_instance(&tmp, &["sodium.jar"]);

        assert!(plan_profile_update(&dir, "missing").is_err());
        assert!(update_profile(&dir, "missing").is_err());
        assert!(!loadouts_dir(&dir).exists(), "no profile file created");
    }

    /// A profile saved before it recorded items knows only the names that were
    /// on. Updating upgrades it, and says what it now records and what is gone.
    #[test]
    fn update_upgrades_a_legacy_profile_and_names_what_is_gone() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = setup_instance(&tmp, &["sodium.jar", "lithium.jar"]);
        manifest_with_lithium_off(&dir);
        save_legacy_profile(&dir, "old", &["sodium.jar", "gone.jar"]);

        let done = update_profile(&dir, "old").unwrap();
        assert_eq!(
            done.changes,
            vec![
                LoadoutChange {
                    filename: "lithium.jar".into(),
                    content_type: "mod".into(),
                    kind: LoadoutChangeKind::Added { enabled: false },
                },
                LoadoutChange {
                    filename: "gone.jar".into(),
                    content_type: String::new(),
                    kind: LoadoutChangeKind::Removed { enabled: true },
                },
            ]
        );
        let upgraded = &list_profiles(&dir).unwrap()[0];
        assert!(upgraded.items.is_some(), "the profile now records items");
        assert!(upgraded.covers.is_some());
    }

    /// Turns lithium off in the manifest and on disk, the way a user would.
    fn manifest_with_lithium_off(dir: &Path) {
        fs::rename(
            dir.join("mods").join("lithium.jar"),
            dir.join("mods").join("lithium.jar.disabled"),
        )
        .unwrap();
        let mut manifest = read_manifest(dir).unwrap();
        manifest.mods[1].enabled = false;
        write_manifest(dir, &manifest).unwrap();
    }

    /// A name with a character the file name cannot hold is stored sanitized,
    /// and must be found and kept under the same name by update.
    #[test]
    fn update_finds_a_profile_whose_name_needed_sanitizing() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = setup_instance(&tmp, &["sodium.jar"]);
        create_profile(&dir, "Pvp: fast").unwrap();
        fs::write(dir.join("mods").join("extra.jar"), b"extra").unwrap();
        let mut manifest = read_manifest(&dir).unwrap();
        let mut extra = manifest.mods[0].clone();
        extra.filename = "extra.jar".into();
        manifest.mods.push(extra);
        write_manifest(&dir, &manifest).unwrap();

        let done = update_profile(&dir, "Pvp: fast").unwrap();
        assert_eq!(done.name, "Pvp: fast");
        assert_eq!(done.changes.len(), 1);
        let profiles = list_profiles(&dir).unwrap();
        assert_eq!(profiles.len(), 1, "same file, not a second profile");
        assert_eq!(profiles[0].name, "Pvp: fast");
    }

    // --- A rename that fails part-way through a change ---------------------

    #[test]
    fn a_shared_enable_set_that_fails_midway_puts_back_the_renamed_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = setup_instance(&tmp, &["a.jar", "b.jar", "c.jar", "d.jar"]);
        let manifest_before = fs::read(dir.join("instance_manifest.json")).unwrap();
        // Disabling b and d renames b first. A directory where d's disabled name
        // should go makes the second rename fail after b has moved.
        fs::create_dir_all(dir.join("mods").join("d.jar.disabled")).unwrap();

        let error = apply_enabled_set(&dir, &["a.jar".to_string(), "c.jar".to_string()])
            .expect_err("the second rename must fail");
        assert!(error.contains("d.jar"), "{error}");
        assert!(
            dir.join("mods").join("b.jar").exists(),
            "b.jar is back under its name"
        );
        assert!(!dir.join("mods").join("b.jar.disabled").exists());
        assert_eq!(
            fs::read(dir.join("instance_manifest.json")).unwrap(),
            manifest_before,
            "the manifest is not written when a rename fails"
        );
    }

    #[test]
    fn a_shared_enable_set_whose_manifest_write_fails_puts_back_every_rename() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = setup_instance(&tmp, &["a.jar", "b.jar", "c.jar", "d.jar"]);
        let manifest_before = fs::read(dir.join("instance_manifest.json")).unwrap();
        // Every rename succeeds; then the manifest cannot be written, so the
        // write fails after the renames are done.
        let _fail = crate::helpers::fail_manifest_writes_on_this_thread();

        assert!(apply_enabled_set(&dir, &["a.jar".to_string(), "c.jar".to_string()]).is_err());
        for name in ["a.jar", "b.jar", "c.jar", "d.jar"] {
            assert!(
                dir.join("mods").join(name).exists(),
                "{name} is back under its name"
            );
        }
        let disabled: Vec<_> = fs::read_dir(dir.join("mods"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".disabled"))
            .collect();
        assert!(
            disabled.is_empty(),
            "no file is left disabled: {disabled:?}"
        );
        assert_eq!(
            fs::read(dir.join("instance_manifest.json")).unwrap(),
            manifest_before
        );
    }

    #[test]
    fn a_loadout_that_fails_midway_puts_back_the_files_it_renamed() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = setup_instance(&tmp, &["a.jar", "b.jar", "c.jar", "d.jar"]);
        create_profile(&dir, "all").unwrap();
        // Switch b and d off, so that applying "all" has to switch both back on.
        apply_enabled_set(&dir, &["a.jar".to_string(), "c.jar".to_string()]).unwrap();
        let manifest_before = fs::read(dir.join("instance_manifest.json")).unwrap();
        // Enabling d renames d.jar.disabled to d.jar; a directory at d.jar makes
        // that fail after b has been switched back on.
        fs::create_dir_all(dir.join("mods").join("d.jar")).unwrap();

        let error =
            apply_profile(&dir, "all", UnlistedMods::Keep).expect_err("enabling d must fail");
        assert!(error.contains("d.jar"), "{error}");
        assert!(
            dir.join("mods").join("b.jar.disabled").exists(),
            "b.jar is off again"
        );
        assert!(!dir.join("mods").join("b.jar").exists());
        assert_eq!(
            fs::read(dir.join("instance_manifest.json")).unwrap(),
            manifest_before,
            "the manifest still records b and d as off"
        );
    }
}
