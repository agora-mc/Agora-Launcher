//! Shared instance folders — one real directory, linked from many instances.
//!
//! Minecraft hardcodes `<gamedir>/screenshots`, so there is no setting that
//! points several instances at one screenshot folder. Linking the directory is
//! the only way, and on Windows the obvious primitive is a trap.
//!
//! # Why junctions, not symlinks
//!
//! `std::os::windows::fs::symlink_dir` requires Developer Mode or an elevated
//! process. Code that calls it typically falls back to copying when it fails —
//! which is correct for a *clone*, and silently wrong for a *share*: the user
//! believes two instances write to one folder, and instead gets two folders
//! that diverge with no error ever surfaced.
//!
//! A directory junction is a reparse point that needs no elevation, so it works
//! for an ordinary user. It is NTFS-only and local-only, which is the whole
//! reason it is cheap; a junction to a network path is exactly the case we
//! refuse rather than half-support.
//!
//! On Unix a symlink needs no privilege, so it is used directly.
//!
//! # The rule
//!
//! Linking never destroys data. If the instance folder already has content, it
//! is moved into the shared target first (or, if that would collide, the link
//! is refused). Failing to link is always reported — never silently downgraded
//! to a copy, because a silent copy is the bug this module exists to avoid.

use std::fs;
use std::path::{Path, PathBuf};

/// What happened when linking a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkOutcome {
    /// A fresh link was created.
    Linked,
    /// The path already pointed at the requested target; nothing to do.
    AlreadyLinked,
    /// Existing files were moved into the shared target before linking.
    MigratedThenLinked { moved: usize },
}

/// Whether `path` is already a link (junction, symlink, or other reparse point).
pub fn is_link(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
}

#[cfg(windows)]
fn create_link(target: &Path, link: &Path) -> Result<(), String> {
    junction::create(target, link)
        .map_err(|error| format!("could not create a directory junction: {error}"))
}

#[cfg(unix)]
fn create_link(target: &Path, link: &Path) -> Result<(), String> {
    std::os::unix::fs::symlink(target, link)
        .map_err(|error| format!("could not create a symlink: {error}"))
}

#[cfg(not(any(windows, unix)))]
fn create_link(_target: &Path, _link: &Path) -> Result<(), String> {
    Err("linking shared folders is not supported on this platform".to_string())
}

/// Where an existing link points, if it is one.
#[cfg(windows)]
pub fn link_target(path: &Path) -> Option<PathBuf> {
    junction::get_target(path).ok()
}

#[cfg(not(windows))]
pub fn link_target(path: &Path) -> Option<PathBuf> {
    fs::read_link(path).ok()
}

/// Move every entry of `from` into `to`, refusing rather than overwriting.
fn migrate_contents(from: &Path, to: &Path) -> Result<usize, String> {
    let entries = fs::read_dir(from)
        .map_err(|error| format!("cannot read {}: {error}", from.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("cannot read {}: {error}", from.display()))?;

    // Check the whole set before moving anything: a partial migration would
    // leave the user's files split across two directories.
    for entry in &entries {
        let destination = to.join(entry.file_name());
        if destination.exists() {
            return Err(format!(
                "'{}' already exists in the shared folder; move or rename it first",
                entry.file_name().to_string_lossy()
            ));
        }
    }

    let mut moved = 0usize;
    for entry in &entries {
        let destination = to.join(entry.file_name());
        fs::rename(entry.path(), &destination).map_err(|error| {
            format!(
                "could not move '{}' into the shared folder: {error}",
                entry.file_name().to_string_lossy()
            )
        })?;
        moved += 1;
    }
    Ok(moved)
}

/// Point `instance_subdir` at `shared_target`.
///
/// Existing content is moved into the target rather than discarded. A failure
/// to link is returned as an error — callers must not fall back to copying,
/// which would silently produce the divergence this exists to prevent.
pub fn link_shared_folder(
    instance_subdir: &Path,
    shared_target: &Path,
) -> Result<LinkOutcome, String> {
    if shared_target.exists() && !shared_target.is_dir() {
        return Err(format!(
            "the shared target {} is not a directory",
            shared_target.display()
        ));
    }
    fs::create_dir_all(shared_target)
        .map_err(|error| format!("cannot create the shared folder: {error}"))?;

    if is_link(instance_subdir) {
        return match link_target(instance_subdir) {
            // Comparing canonically: the recorded target and the requested one
            // are often the same directory spelled differently.
            Some(existing)
                if fs::canonicalize(&existing).ok() == fs::canonicalize(shared_target).ok() =>
            {
                Ok(LinkOutcome::AlreadyLinked)
            }
            _ => Err(format!(
                "{} is already linked somewhere else; unlink it first",
                instance_subdir.display()
            )),
        };
    }

    let mut moved = 0usize;
    if instance_subdir.exists() {
        if !instance_subdir.is_dir() {
            return Err(format!(
                "{} exists but is not a directory",
                instance_subdir.display()
            ));
        }
        moved = migrate_contents(instance_subdir, shared_target)?;
        fs::remove_dir(instance_subdir).map_err(|error| {
            format!(
                "could not replace {} with a link: {error}",
                instance_subdir.display()
            )
        })?;
    }
    if let Some(parent) = instance_subdir.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create the instance directory: {error}"))?;
    }

    create_link(shared_target, instance_subdir)?;
    Ok(if moved > 0 {
        LinkOutcome::MigratedThenLinked { moved }
    } else {
        LinkOutcome::Linked
    })
}

/// Replace a link with an ordinary empty directory.
///
/// The shared content is left alone — unlinking is "stop sharing", never
/// "delete what we were sharing".
pub fn unlink_shared_folder(instance_subdir: &Path) -> Result<(), String> {
    if !is_link(instance_subdir) {
        return Ok(());
    }
    // A junction is removed as a directory, a Unix symlink as a file.
    #[cfg(windows)]
    let removed = fs::remove_dir(instance_subdir);
    #[cfg(not(windows))]
    let removed = fs::remove_file(instance_subdir);
    removed.map_err(|error| format!("could not unlink the shared folder: {error}"))?;
    fs::create_dir_all(instance_subdir)
        .map_err(|error| format!("could not restore the folder: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linking_an_absent_folder_just_links_it() {
        let tmp = tempfile::tempdir().unwrap();
        let shared = tmp.path().join("shared");
        let instance = tmp.path().join("inst/screenshots");
        assert_eq!(
            link_shared_folder(&instance, &shared).unwrap(),
            LinkOutcome::Linked
        );
        assert!(is_link(&instance));
    }

    #[test]
    fn existing_files_are_moved_into_the_share_not_destroyed() {
        // The failure that matters: a user with 400 screenshots turning sharing
        // on must not lose them.
        let tmp = tempfile::tempdir().unwrap();
        let shared = tmp.path().join("shared");
        let instance = tmp.path().join("inst/screenshots");
        fs::create_dir_all(&instance).unwrap();
        fs::write(instance.join("a.png"), b"one").unwrap();
        fs::write(instance.join("b.png"), b"two").unwrap();

        let outcome = link_shared_folder(&instance, &shared).unwrap();
        assert_eq!(outcome, LinkOutcome::MigratedThenLinked { moved: 2 });
        assert_eq!(fs::read(shared.join("a.png")).unwrap(), b"one");
        // ...and they are visible through the link.
        assert_eq!(fs::read(instance.join("b.png")).unwrap(), b"two");
    }

    #[test]
    fn a_name_collision_refuses_rather_than_overwriting() {
        let tmp = tempfile::tempdir().unwrap();
        let shared = tmp.path().join("shared");
        fs::create_dir_all(&shared).unwrap();
        fs::write(shared.join("a.png"), b"the shared one").unwrap();
        let instance = tmp.path().join("inst/screenshots");
        fs::create_dir_all(&instance).unwrap();
        fs::write(instance.join("a.png"), b"the instance one").unwrap();

        let error = link_shared_folder(&instance, &shared).unwrap_err();
        assert!(error.contains("a.png"), "{error}");
        // Nothing moved, nothing clobbered, nothing linked.
        assert_eq!(fs::read(shared.join("a.png")).unwrap(), b"the shared one");
        assert_eq!(
            fs::read(instance.join("a.png")).unwrap(),
            b"the instance one"
        );
        assert!(!is_link(&instance));
    }

    #[test]
    fn writes_through_the_link_land_in_the_shared_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let shared = tmp.path().join("shared");
        let instance = tmp.path().join("inst/screenshots");
        link_shared_folder(&instance, &shared).unwrap();

        fs::write(instance.join("new.png"), b"shot").unwrap();
        assert_eq!(
            fs::read(shared.join("new.png")).unwrap(),
            b"shot",
            "a link that does not actually share is the bug this module exists to prevent"
        );
    }

    #[test]
    fn relinking_to_the_same_target_is_a_no_op_and_elsewhere_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let shared = tmp.path().join("shared");
        let other = tmp.path().join("other");
        let instance = tmp.path().join("inst/screenshots");
        link_shared_folder(&instance, &shared).unwrap();

        assert_eq!(
            link_shared_folder(&instance, &shared).unwrap(),
            LinkOutcome::AlreadyLinked
        );
        assert!(link_shared_folder(&instance, &other).is_err());
    }

    #[test]
    fn unlinking_stops_sharing_without_deleting_the_shared_content() {
        let tmp = tempfile::tempdir().unwrap();
        let shared = tmp.path().join("shared");
        let instance = tmp.path().join("inst/screenshots");
        link_shared_folder(&instance, &shared).unwrap();
        fs::write(instance.join("keep.png"), b"shot").unwrap();

        unlink_shared_folder(&instance).unwrap();
        assert!(!is_link(&instance));
        assert!(instance.is_dir(), "an ordinary folder is left behind");
        assert_eq!(
            fs::read(shared.join("keep.png")).unwrap(),
            b"shot",
            "unlinking is stop sharing, never delete"
        );
        // Idempotent.
        unlink_shared_folder(&instance).unwrap();
    }
}
