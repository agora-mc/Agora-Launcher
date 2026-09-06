//! User-defined mod groups.
//!
//! Agora can already group installed content by pack, category, or source, but
//! all three are derived — the user cannot say "these six are my performance
//! mods". This adds the one grouping the user owns.
//!
//! # Storage
//!
//! Assignments live in the instance manifest under
//! `user_preferences.agora_mod_groups`, as `{ "<group name>": ["file.jar", …] }`.
//! Two consequences follow from that choice and are the reason for it: the
//! grouping travels with an exported instance, and no schema migration is
//! needed. It is keyed by filename because that is the identifier the installed
//! content list already carries end to end.
//!
//! An entry is in at most one group. Grouping is a way to *partition* a long
//! list, and a mod appearing under two headings would break select-all and the
//! per-group counts the UI shows.

use std::collections::BTreeMap;

use crate::models::InstanceManifest;

/// Key under `user_preferences` holding the assignments.
const GROUPS_KEY: &str = "agora_mod_groups";

/// Longest permitted group name.
///
/// Names are rendered as table headings, and the store is a JSON object in a
/// manifest that is read on every launch.
pub const MAX_GROUP_NAME_LEN: usize = 64;

/// Largest number of distinct groups per instance.
pub const MAX_GROUPS: usize = 50;

/// Group name -> the filenames assigned to it, sorted.
pub type ModGroups = BTreeMap<String, Vec<String>>;

/// Read the assignments recorded in a manifest.
///
/// Unreadable or malformed data reads as "no groups": a grouping is a view
/// preference, and failing an instance load over one would be absurd.
pub fn read_groups(manifest: &InstanceManifest) -> ModGroups {
    let Some(raw) = manifest
        .user_preferences
        .as_object()
        .and_then(|preferences| preferences.get(GROUPS_KEY))
        .and_then(|value| value.as_object())
    else {
        return ModGroups::new();
    };

    let mut groups = ModGroups::new();
    for (name, members) in raw {
        let Some(members) = members.as_array() else {
            continue;
        };
        let mut filenames: Vec<String> = members
            .iter()
            .filter_map(|entry| entry.as_str())
            .map(str::to_string)
            .collect();
        filenames.sort();
        filenames.dedup();
        if !filenames.is_empty() {
            groups.insert(name.clone(), filenames);
        }
    }
    groups
}

fn write_groups(manifest: &mut InstanceManifest, groups: &ModGroups) {
    if !manifest.user_preferences.is_object() {
        manifest.user_preferences = serde_json::json!({});
    }
    let preferences = manifest
        .user_preferences
        .as_object_mut()
        .expect("just ensured object");
    if groups.is_empty() {
        preferences.remove(GROUPS_KEY);
        return;
    }
    let encoded: serde_json::Map<String, serde_json::Value> = groups
        .iter()
        .map(|(name, filenames)| {
            (
                name.clone(),
                serde_json::Value::Array(
                    filenames
                        .iter()
                        .map(|f| serde_json::Value::String(f.clone()))
                        .collect(),
                ),
            )
        })
        .collect();
    preferences.insert(GROUPS_KEY.to_string(), serde_json::Value::Object(encoded));
}

/// Normalize and validate a group name.
pub fn normalize_group_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("A group needs a name.".to_string());
    }
    if trimmed.chars().count() > MAX_GROUP_NAME_LEN {
        return Err(format!(
            "Group names are limited to {MAX_GROUP_NAME_LEN} characters."
        ));
    }
    // Control characters would corrupt the rendered table heading and serve no
    // purpose in a label.
    if trimmed.chars().any(char::is_control) {
        return Err("Group names cannot contain control characters.".to_string());
    }
    Ok(trimmed.to_string())
}

/// Assign filenames to a group, or clear their assignment when `group` is
/// `None`.
///
/// Each filename is removed from every other group first, so an entry is never
/// in two places. Groups left empty are dropped — an empty heading is noise.
/// Returns the resulting assignments.
pub fn assign(
    manifest: &mut InstanceManifest,
    filenames: &[String],
    group: Option<&str>,
) -> Result<ModGroups, String> {
    let target = match group {
        Some(name) => Some(normalize_group_name(name)?),
        None => None,
    };
    let mut groups = read_groups(manifest);

    if let Some(target) = target.as_deref() {
        if !groups.contains_key(target) && groups.len() >= MAX_GROUPS {
            return Err(format!("An instance can have at most {MAX_GROUPS} groups."));
        }
    }

    for members in groups.values_mut() {
        members.retain(|existing| !filenames.contains(existing));
    }
    if let Some(target) = target {
        let members = groups.entry(target).or_default();
        for filename in filenames {
            if !members.contains(filename) {
                members.push(filename.clone());
            }
        }
        members.sort();
    }
    groups.retain(|_, members| !members.is_empty());

    write_groups(manifest, &groups);
    Ok(groups)
}

/// Rename a group, merging into the destination when one already exists.
pub fn rename(manifest: &mut InstanceManifest, from: &str, to: &str) -> Result<ModGroups, String> {
    let to = normalize_group_name(to)?;
    let mut groups = read_groups(manifest);
    let Some(members) = groups.remove(from) else {
        return Err(format!("No group named '{from}'."));
    };
    let destination = groups.entry(to).or_default();
    for filename in members {
        if !destination.contains(&filename) {
            destination.push(filename);
        }
    }
    destination.sort();
    write_groups(manifest, &groups);
    Ok(groups)
}

/// Delete a group. Its members become ungrouped rather than being removed.
pub fn delete(manifest: &mut InstanceManifest, name: &str) -> ModGroups {
    let mut groups = read_groups(manifest);
    groups.remove(name);
    write_groups(manifest, &groups);
    groups
}

/// Drop assignments naming content that is no longer installed.
///
/// Called on read rather than on every removal: a stale name is harmless until
/// someone renders it, and this keeps the removal path from having to know
/// about grouping at all.
pub fn prune_missing(groups: &mut ModGroups, installed: &[String]) {
    for members in groups.values_mut() {
        members.retain(|filename| installed.contains(filename));
    }
    groups.retain(|_, members| !members.is_empty());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> InstanceManifest {
        InstanceManifest {
            manifest_version: crate::models::CURRENT_MANIFEST_VERSION,
            pack_origin: None,
            instance_id: "test".into(),
            name: "Test".into(),
            created_from_pack: None,
            minecraft_version: "1.21".into(),
            loader: "fabric".into(),
            loader_version: "0.16.0".into(),
            is_locked: false,
            mods: vec![],
            resourcepacks: vec![],
            shaders: vec![],
            datapacks: vec![],
            worlds: vec![],
            user_preferences: serde_json::json!({}),
        }
    }

    #[test]
    fn assignment_round_trips_through_the_manifest() {
        let mut m = manifest();
        assign(&mut m, &["sodium.jar".into()], Some("Performance")).unwrap();
        let reread = read_groups(&m);
        assert_eq!(reread.get("Performance").unwrap(), &vec!["sodium.jar"]);
    }

    #[test]
    fn an_entry_lands_in_exactly_one_group() {
        // Two headings for one row would break select-all and the per-group
        // counts, so a reassignment has to move rather than copy.
        let mut m = manifest();
        assign(&mut m, &["sodium.jar".into()], Some("Performance")).unwrap();
        let groups = assign(&mut m, &["sodium.jar".into()], Some("Visual")).unwrap();
        assert!(!groups.contains_key("Performance"), "{groups:?}");
        assert_eq!(groups.get("Visual").unwrap(), &vec!["sodium.jar"]);
    }

    #[test]
    fn clearing_an_assignment_drops_the_group_when_it_empties() {
        let mut m = manifest();
        assign(&mut m, &["sodium.jar".into()], Some("Performance")).unwrap();
        let groups = assign(&mut m, &["sodium.jar".into()], None).unwrap();
        assert!(groups.is_empty());
        // The key is removed outright rather than left as an empty object.
        assert!(m.user_preferences.get("agora_mod_groups").is_none());
    }

    #[test]
    fn other_preferences_survive_a_group_write() {
        let mut m = manifest();
        m.user_preferences =
            serde_json::json!({ "agora_pack_icon_url": "https://example.com/i.png" });
        assign(&mut m, &["sodium.jar".into()], Some("Performance")).unwrap();
        assert_eq!(
            m.user_preferences.get("agora_pack_icon_url").unwrap(),
            "https://example.com/i.png"
        );
    }

    #[test]
    fn names_are_trimmed_and_junk_is_rejected() {
        assert_eq!(
            normalize_group_name("  Performance  ").unwrap(),
            "Performance"
        );
        assert!(normalize_group_name("   ").is_err());
        assert!(normalize_group_name("bad\nname").is_err());
        assert!(normalize_group_name(&"x".repeat(MAX_GROUP_NAME_LEN + 1)).is_err());
    }

    #[test]
    fn the_group_count_is_bounded_but_refilling_an_existing_group_still_works() {
        let mut m = manifest();
        for index in 0..MAX_GROUPS {
            assign(
                &mut m,
                &[format!("mod{index}.jar")],
                Some(&format!("g{index}")),
            )
            .unwrap();
        }
        assert!(assign(&mut m, &["extra.jar".into()], Some("one-too-many")).is_err());
        // Adding to a group that already exists is not a new group.
        assert!(assign(&mut m, &["extra.jar".into()], Some("g0")).is_ok());
    }

    #[test]
    fn renaming_into_an_existing_group_merges_without_duplicating() {
        let mut m = manifest();
        assign(&mut m, &["a.jar".into()], Some("Old")).unwrap();
        assign(&mut m, &["b.jar".into()], Some("New")).unwrap();
        let groups = rename(&mut m, "Old", "New").unwrap();
        assert_eq!(groups.get("New").unwrap(), &vec!["a.jar", "b.jar"]);
        assert!(!groups.contains_key("Old"));
        assert!(rename(&mut m, "Missing", "Whatever").is_err());
    }

    #[test]
    fn deleting_a_group_ungroups_its_members_rather_than_losing_them() {
        let mut m = manifest();
        assign(&mut m, &["a.jar".into(), "b.jar".into()], Some("Old")).unwrap();
        let groups = delete(&mut m, "Old");
        assert!(groups.is_empty());
        // The mods themselves are untouched — grouping is only a view.
        assert!(m.mods.is_empty());
    }

    #[test]
    fn malformed_stored_groups_read_as_no_groups() {
        let mut m = manifest();
        m.user_preferences = serde_json::json!({ "agora_mod_groups": "not an object" });
        assert!(read_groups(&m).is_empty());
        m.user_preferences = serde_json::json!({ "agora_mod_groups": { "G": "not an array" } });
        assert!(read_groups(&m).is_empty());
        m.user_preferences = serde_json::json!({ "agora_mod_groups": { "G": [1, 2, 3] } });
        assert!(read_groups(&m).is_empty(), "non-string members are skipped");
    }

    #[test]
    fn pruning_drops_names_of_content_that_is_gone() {
        let mut groups = ModGroups::new();
        groups.insert(
            "Performance".into(),
            vec!["gone.jar".into(), "here.jar".into()],
        );
        groups.insert("Empty".into(), vec!["also-gone.jar".into()]);
        prune_missing(&mut groups, &["here.jar".to_string()]);
        assert_eq!(groups.get("Performance").unwrap(), &vec!["here.jar"]);
        assert!(!groups.contains_key("Empty"));
    }
}
