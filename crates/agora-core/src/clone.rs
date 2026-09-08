use crate::paths;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

const MAX_CLONE_DEPTH: usize = 256;
const MAX_CLONE_ENTRIES: usize = 1_000_000;

#[derive(Debug, Clone, Copy)]
enum EntryKind {
    Directory,
    File,
    Symlink { directory: bool },
    UnrecognizedReparse,
    Unsupported,
}

#[derive(Debug)]
struct WorkItem {
    src: PathBuf,
    dst: PathBuf,
    depth: usize,
}

struct StagingGuard {
    path: PathBuf,
    active: bool,
}

impl StagingGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, active: true }
    }

    fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for StagingGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }

        // The path was created with create_dir, so it is ours.  Re-check the
        // entry without following links before cleanup so a link at this path
        // is removed as a link rather than traversed.
        match fs::symlink_metadata(&self.path) {
            Ok(metadata) if metadata.file_type().is_dir() => {
                let _ = fs::remove_dir_all(&self.path);
            }
            Ok(_) => {
                let _ = fs::remove_file(&self.path);
            }
            Err(_) => {}
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClonePrefs {
    pub copy_saves: bool,
    pub copy_mods: bool,
    pub copy_resource_packs: bool,
    pub copy_shader_packs: bool,
    pub copy_screenshots: bool,
    pub copy_config: bool,
    pub copy_servers: bool,
    pub copy_options: bool,
    pub use_hard_links: bool,
    pub use_sym_links: bool,
}

impl Default for ClonePrefs {
    fn default() -> Self {
        ClonePrefs {
            copy_saves: true,
            copy_mods: true,
            copy_resource_packs: true,
            copy_shader_packs: true,
            copy_screenshots: true,
            copy_config: true,
            copy_servers: true,
            copy_options: true,
            use_hard_links: false,
            use_sym_links: false,
        }
    }
}

struct DirMapping {
    dir: &'static str,
    pref: fn(&ClonePrefs) -> bool,
}

const DIR_MAPPINGS: &[DirMapping] = &[
    DirMapping {
        dir: "saves",
        pref: |p| p.copy_saves,
    },
    DirMapping {
        dir: "mods",
        pref: |p| p.copy_mods,
    },
    DirMapping {
        dir: "resourcepacks",
        pref: |p| p.copy_resource_packs,
    },
    DirMapping {
        dir: "shaderpacks",
        pref: |p| p.copy_shader_packs,
    },
    DirMapping {
        dir: "screenshots",
        pref: |p| p.copy_screenshots,
    },
    DirMapping {
        dir: "config",
        pref: |p| p.copy_config,
    },
    DirMapping {
        dir: "servers",
        pref: |p| p.copy_servers,
    },
    DirMapping {
        dir: "options",
        pref: |p| p.copy_options,
    },
];

/// Clone an instance directory with the given copy preferences.
/// Returns the new instance_id (a sanitized version of the name).
pub fn clone_instance(
    src_dir: &Path,
    dest_dir: &Path,
    prefs: &ClonePrefs,
) -> Result<String, String> {
    match inspect_entry(src_dir)? {
        EntryKind::Directory => {}
        EntryKind::File => return Err(format!("Source {:?} is not a directory", src_dir)),
        EntryKind::Symlink { .. } => return Err(link_policy_error(src_dir)),
        EntryKind::UnrecognizedReparse => return Err(reparse_policy_error(src_dir)),
        EntryKind::Unsupported => return Err(unsupported_entry_error(src_dir)),
    }

    let name = src_dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let instance_id = paths::sanitize_id(&name);

    let src_resolved = fs::canonicalize(src_dir)
        .map_err(|e| format!("Cannot resolve source directory {src_dir:?}: {e}"))?;
    if path_is_present(dest_dir)? {
        return Err(format!(
            "Destination {dest_dir:?} already exists; cloning never overwrites an existing destination"
        ));
    }
    let dest_resolved = resolve_path_with_existing_parts(dest_dir)?;
    if is_same_or_descendant(&src_resolved, &dest_resolved)
        || is_same_or_descendant(&dest_resolved, &src_resolved)
    {
        return Err(format!(
            "Refusing to clone {src_dir:?} to {dest_dir:?}: the source and destination are the same path or one is an ancestor of the other"
        ));
    }

    let staging_path = create_staging_dir(dest_dir)?;
    let mut staging = StagingGuard::new(staging_path.clone());
    let mut entry_count = 0usize;

    copy_optional_manifest(
        &src_dir.join("manifest.json"),
        &staging_path.join("manifest.json"),
        prefs,
        "manifest.json",
        &mut entry_count,
    )?;
    copy_optional_manifest(
        &src_dir.join("instance_manifest.json"),
        &staging_path.join("instance_manifest.json"),
        prefs,
        "instance_manifest.json",
        &mut entry_count,
    )?;

    for mapping in DIR_MAPPINGS {
        if !(mapping.pref)(prefs) {
            continue;
        }
        let src_child = src_dir.join(mapping.dir);
        if !path_is_present(&src_child)? {
            continue;
        }
        let dst_child = staging_path.join(mapping.dir);
        copy_entry(&src_child, &dst_child, prefs, &mut entry_count)?;
    }

    publish_staging(&staging_path, dest_dir)?;
    staging.disarm();
    Ok(instance_id)
}

fn copy_entry(
    src: &Path,
    dst: &Path,
    prefs: &ClonePrefs,
    entry_count: &mut usize,
) -> Result<(), String> {
    let mut work = vec![WorkItem {
        src: src.to_path_buf(),
        dst: dst.to_path_buf(),
        depth: 0,
    }];
    while let Some(item) = work.pop() {
        *entry_count = (*entry_count)
            .checked_add(1)
            .ok_or_else(|| format!("Clone entry count overflow while processing {:?}", item.src))?;
        if *entry_count > MAX_CLONE_ENTRIES {
            return Err(format!(
                "Clone entry limit of {MAX_CLONE_ENTRIES} exceeded at {:?}",
                item.src
            ));
        }
        #[cfg(test)]
        if test_should_fail_after(*entry_count) {
            return Err(format!("Injected mid-copy failure at {:?}", item.src));
        }
        if item.depth > MAX_CLONE_DEPTH {
            return Err(format!(
                "Clone directory depth limit of {MAX_CLONE_DEPTH} exceeded at {:?}",
                item.src
            ));
        }

        let kind = inspect_entry(&item.src)?;

        if prefs.use_sym_links {
            match kind {
                EntryKind::Directory | EntryKind::File | EntryKind::Symlink { .. } => {
                    let link_target = absolute_path(&item.src)?;
                    symlink_entry(&link_target, &item.dst, kind).map_err(|error| {
                        format!(
                            "Cannot create the requested symbolic link for {:?} to {:?}: {error}; refusing to fall back to copying the source"
                        , item.src, item.dst)
                    })?;
                    continue;
                }
                EntryKind::UnrecognizedReparse => {
                    return Err(reparse_policy_error(&item.src));
                }
                EntryKind::Unsupported => {
                    return Err(unsupported_entry_error(&item.src));
                }
            }
        }

        if prefs.use_hard_links && matches!(kind, EntryKind::File | EntryKind::Symlink { .. }) {
            hardlink_entry(&item.src, &item.dst).map_err(|error| {
                format!(
                    "Cannot create the requested hard link for {:?} to {:?}: {error}; refusing to fall back to copying the source",
                    item.src, item.dst
                )
            })?;
            continue;
        }

        match kind {
            EntryKind::Directory => {
                fs::create_dir(&item.dst)
                    .map_err(|e| format!("Cannot create dir {:?}: {e}", item.dst))?;
                let entries = fs::read_dir(&item.src)
                    .map_err(|e| format!("Cannot read dir {:?}: {e}", item.src))?;
                for entry in entries {
                    let entry = entry
                        .map_err(|e| format!("Cannot read an entry under {:?}: {e}", item.src))?;
                    work.push(WorkItem {
                        src: entry.path(),
                        dst: item.dst.join(entry.file_name()),
                        depth: item.depth.saturating_add(1),
                    });
                }
            }
            EntryKind::File => {
                fs::copy(&item.src, &item.dst)
                    .map_err(|e| format!("Cannot copy {:?} to {:?}: {e}", item.src, item.dst))?;
            }
            EntryKind::Symlink { .. } => {
                return Err(link_policy_error(&item.src));
            }
            EntryKind::UnrecognizedReparse => {
                return Err(reparse_policy_error(&item.src));
            }
            EntryKind::Unsupported => {
                return Err(unsupported_entry_error(&item.src));
            }
        }
    }

    Ok(())
}

#[cfg(unix)]
fn symlink_entry(_src: &Path, _dst: &Path, _kind: EntryKind) -> io::Result<()> {
    std::os::unix::fs::symlink(_src, _dst)
}

#[cfg(windows)]
fn symlink_entry(src: &Path, dst: &Path, kind: EntryKind) -> io::Result<()> {
    if matches!(
        kind,
        EntryKind::Directory | EntryKind::Symlink { directory: true }
    ) {
        std::os::windows::fs::symlink_dir(src, dst)
    } else {
        std::os::windows::fs::symlink_file(src, dst)
    }
}

#[cfg(not(any(unix, windows)))]
fn symlink_entry(_src: &Path, _dst: &Path, _kind: EntryKind) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "symbolic links are not supported on this platform",
    ))
}

#[cfg(any(unix, windows))]
fn hardlink_entry(src: &Path, dst: &Path) -> io::Result<()> {
    fs::hard_link(src, dst)
}

#[cfg(not(any(unix, windows)))]
fn hardlink_entry(_src: &Path, _dst: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "hard links are not supported on this platform",
    ))
}

fn inspect_entry(path: &Path) -> Result<EntryKind, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Cannot inspect source entry {:?}: {error}", path))?;
    let file_type = metadata.file_type();

    #[cfg(unix)]
    if file_type.is_symlink() {
        return Ok(EntryKind::Symlink { directory: false });
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::{FileTypeExt, MetadataExt};

        if file_type.is_symlink() || file_type.is_symlink_dir() || file_type.is_symlink_file() {
            return Ok(EntryKind::Symlink {
                directory: file_type.is_symlink_dir(),
            });
        }

        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            // Cloud providers use reparse points for placeholders, but those
            // are not path redirections.  These attributes are the standard
            // recall/pinning indicators exposed by the Windows filesystem.
            const FILE_ATTRIBUTE_OFFLINE: u32 = 0x1000;
            const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x0004_0000;
            const FILE_ATTRIBUTE_PINNED: u32 = 0x0008_0000;
            const FILE_ATTRIBUTE_UNPINNED: u32 = 0x0010_0000;
            const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;
            const CLOUD_ATTRIBUTES: u32 = FILE_ATTRIBUTE_OFFLINE
                | FILE_ATTRIBUTE_RECALL_ON_OPEN
                | FILE_ATTRIBUTE_PINNED
                | FILE_ATTRIBUTE_UNPINNED
                | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS;

            if metadata.file_attributes() & CLOUD_ATTRIBUTES != 0 {
                if file_type.is_dir() {
                    return Ok(EntryKind::Directory);
                }
                if file_type.is_file() {
                    return Ok(EntryKind::File);
                }
            }

            return Ok(EntryKind::UnrecognizedReparse);
        }
    }

    if file_type.is_dir() {
        Ok(EntryKind::Directory)
    } else if file_type.is_file() {
        Ok(EntryKind::File)
    } else {
        Ok(EntryKind::Unsupported)
    }
}

fn path_is_present(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("Cannot inspect path {:?}: {error}", path)),
    }
}

fn resolve_path_with_existing_parts(path: &Path) -> Result<PathBuf, String> {
    let mut current = path.to_path_buf();
    let mut missing_parts = Vec::new();

    loop {
        match fs::symlink_metadata(&current) {
            Ok(_) => {
                let mut resolved = fs::canonicalize(&current)
                    .map_err(|error| format!("Cannot resolve path {:?}: {error}", current))?;
                for part in missing_parts.iter().rev() {
                    resolved.push(part);
                }
                return Ok(normalize_path(&resolved));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let parent = current
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."));
                let name = current
                    .file_name()
                    .ok_or_else(|| format!("Cannot resolve path {:?}: it has no filename", path))?;
                if parent == current {
                    return Err(format!(
                        "Cannot resolve path {:?}: it has no existing parent",
                        path
                    ));
                }
                missing_parts.push(name.to_owned());
                current = parent.to_path_buf();
            }
            Err(error) => {
                return Err(format!("Cannot inspect path {:?}: {error}", current));
            }
        }
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}

fn is_same_or_descendant(ancestor: &Path, candidate: &Path) -> bool {
    let ancestor_components: Vec<_> = ancestor.components().collect();
    let candidate_components: Vec<_> = candidate.components().collect();
    ancestor_components.len() <= candidate_components.len()
        && ancestor_components
            .iter()
            .zip(candidate_components.iter())
            .all(|(left, right)| path_component_eq(left, right))
}

#[cfg(windows)]
fn path_component_eq(left: &Component<'_>, right: &Component<'_>) -> bool {
    left.as_os_str()
        .to_string_lossy()
        .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy())
}

#[cfg(not(windows))]
fn path_component_eq(left: &Component<'_>, right: &Component<'_>) -> bool {
    left == right
}

fn link_policy_error(path: &Path) -> String {
    format!(
        "Refusing to clone symbolic link or filesystem redirection at {:?}; exclude that content with its copy_* preference, or use a link-based clone mode (use_sym_links/use_hard_links)",
        path
    )
}

fn reparse_policy_error(path: &Path) -> String {
    format!(
        "Refusing to clone an unrecognised Windows reparse point at {:?}; exclude that content with its copy_* preference, or use a supported link-based clone mode",
        path
    )
}

fn unsupported_entry_error(path: &Path) -> String {
    format!(
        "Unsupported filesystem entry at {:?}; exclude that content with its copy_* preference or replace the fifo, socket, device, or other unsupported entry",
        path
    )
}

fn absolute_path(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(normalize_path(path))
    } else {
        let current_dir = std::env::current_dir().map_err(|error| {
            format!(
                "Cannot resolve the current directory for {:?}: {error}",
                path
            )
        })?;
        Ok(normalize_path(&current_dir.join(path)))
    }
}

fn copy_optional_manifest(
    src: &Path,
    dst: &Path,
    prefs: &ClonePrefs,
    name: &str,
    entry_count: &mut usize,
) -> Result<(), String> {
    if !path_is_present(src)? {
        return Ok(());
    }

    match inspect_entry(src)? {
        EntryKind::File | EntryKind::Symlink { .. } => copy_entry(src, dst, prefs, entry_count),
        EntryKind::Directory => Err(format!(
            "Cannot copy {name}: expected a file but found a directory at {src:?}"
        )),
        EntryKind::UnrecognizedReparse => Err(reparse_policy_error(src)),
        EntryKind::Unsupported => Err(unsupported_entry_error(src)),
    }
}

fn create_staging_dir(dest_dir: &Path) -> Result<PathBuf, String> {
    let parent = dest_dir
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .map_err(|error| format!("Cannot create destination parent {parent:?}: {error}"))?;

    for _ in 0..100 {
        let candidate = parent.join(format!(".agora-clone-{}", uuid::Uuid::new_v4()));
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "Cannot create clone staging directory {candidate:?}: {error}"
                ));
            }
        }
    }

    Err(format!(
        "Cannot create a unique clone staging directory beside {dest_dir:?}"
    ))
}

fn publish_staging(staging: &Path, dest: &Path) -> Result<(), String> {
    rename_without_replacing(staging, dest).map_err(|error| {
        format!("Cannot publish clone to {dest:?}: {error}; the destination was not overwritten")
    })
}

#[cfg(windows)]
fn rename_without_replacing(staging: &Path, dest: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(
            existing_file_name: *const u16,
            new_file_name: *const u16,
            flags: u32,
        ) -> i32;
    }

    let staging: Vec<u16> = staging.as_os_str().encode_wide().chain(Some(0)).collect();
    let dest: Vec<u16> = dest.as_os_str().encode_wide().chain(Some(0)).collect();
    const MOVEFILE_COPY_ALLOWED: u32 = 0x0000_0002;

    // Omitting MOVEFILE_REPLACE_EXISTING makes MoveFileExW fail if another
    // actor created the destination after our initial absence check.
    let result = unsafe { MoveFileExW(staging.as_ptr(), dest.as_ptr(), MOVEFILE_COPY_ALLOWED) };
    if result != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(target_os = "linux")]
fn rename_without_replacing(staging: &Path, dest: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::raw::{c_char, c_int};
    use std::os::unix::ffi::OsStrExt;

    unsafe extern "C" {
        fn renameat2(
            old_directory_fd: c_int,
            old_path: *const c_char,
            new_directory_fd: c_int,
            new_path: *const c_char,
            flags: u32,
        ) -> c_int;
    }

    let staging = CString::new(staging.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "staging path contains NUL"))?;
    let dest = CString::new(dest.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidInput, "destination path contains NUL")
    })?;
    const AT_FDCWD: c_int = -100;
    const RENAME_NOREPLACE: u32 = 1;

    let result = unsafe {
        renameat2(
            AT_FDCWD,
            staging.as_ptr(),
            AT_FDCWD,
            dest.as_ptr(),
            RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(target_os = "macos")]
fn rename_without_replacing(staging: &Path, dest: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::raw::{c_char, c_int};
    use std::os::unix::ffi::OsStrExt;

    unsafe extern "C" {
        fn renameatx_np(
            old_directory_fd: c_int,
            old_path: *const c_char,
            new_directory_fd: c_int,
            new_path: *const c_char,
            flags: u32,
        ) -> c_int;
    }

    let staging = CString::new(staging.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "staging path contains NUL"))?;
    let dest = CString::new(dest.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidInput, "destination path contains NUL")
    })?;
    const AT_FDCWD: c_int = -2;
    const RENAME_EXCL: u32 = 0x0000_0004;

    let result = unsafe {
        renameatx_np(
            AT_FDCWD,
            staging.as_ptr(),
            AT_FDCWD,
            dest.as_ptr(),
            RENAME_EXCL,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn rename_without_replacing(_staging: &Path, _dest: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "this platform has no safe no-replace directory rename primitive",
    ))
}

#[cfg(not(any(unix, windows)))]
fn rename_without_replacing(_staging: &Path, _dest: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "this platform has no safe no-replace directory rename primitive",
    ))
}

#[cfg(test)]
thread_local! {
    static TEST_FAIL_AFTER_ENTRIES: std::cell::Cell<Option<usize>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn test_should_fail_after(entries: usize) -> bool {
    TEST_FAIL_AFTER_ENTRIES.with(|fail_after| match fail_after.get() {
        Some(limit) => entries >= limit,
        None => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    use std::process::Command;

    #[cfg(any(unix, windows))]
    fn create_directory_symlink(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link).is_ok()
        }

        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_dir(target, link).is_ok()
        }
    }

    #[cfg(not(any(unix, windows)))]
    fn create_directory_symlink(_target: &Path, _link: &Path) -> bool {
        false
    }

    #[cfg(any(unix, windows))]
    fn create_file_symlink(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link).is_ok()
        }

        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_file(target, link).is_ok()
        }
    }

    #[cfg(not(any(unix, windows)))]
    fn create_file_symlink(_target: &Path, _link: &Path) -> bool {
        false
    }

    #[test]
    fn test_clone_all_dirs_exist() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("original");

        for dir in [
            "mods",
            "saves",
            "config",
            "resourcepacks",
            "shaderpacks",
            "screenshots",
            "servers",
            "options",
        ] {
            fs::create_dir_all(src.join(dir)).unwrap();
            fs::write(src.join(dir).join("placeholder.txt"), b"data").unwrap();
        }
        fs::write(src.join("instance_manifest.json"), b"{}").unwrap();

        let dst = tmp.path().join("clone");
        let prefs = ClonePrefs::default();
        let id = clone_instance(&src, &dst, &prefs).unwrap();
        assert!(!id.is_empty());

        for dir in [
            "mods",
            "saves",
            "config",
            "resourcepacks",
            "shaderpacks",
            "screenshots",
            "servers",
            "options",
        ] {
            assert!(dst.join(dir).exists(), "missing {dir}");
        }
        assert!(dst.join("instance_manifest.json").exists());
    }

    #[test]
    fn test_clone_no_mods() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("original");

        fs::create_dir_all(src.join("mods")).unwrap();
        fs::write(src.join("mods").join("some-mod.jar"), b"mod").unwrap();
        fs::create_dir_all(src.join("saves")).unwrap();
        fs::write(src.join("saves").join("world.dat"), b"world").unwrap();

        let dst = tmp.path().join("clone");
        let prefs = ClonePrefs {
            copy_mods: false,
            ..Default::default()
        };
        clone_instance(&src, &dst, &prefs).unwrap();

        assert!(!dst.join("mods").exists());
        assert!(dst.join("saves").exists());
    }

    #[test]
    fn test_clone_hardlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("original");

        fs::create_dir_all(src.join("mods")).unwrap();
        fs::write(src.join("mods").join("test.jar"), b"hardlink test").unwrap();

        let dst = tmp.path().join("clone");
        let prefs = ClonePrefs {
            use_hard_links: true,
            ..Default::default()
        };
        clone_instance(&src, &dst, &prefs).unwrap();

        assert!(dst.join("mods").join("test.jar").exists());
        assert_eq!(
            fs::read(src.join("mods").join("test.jar")).unwrap(),
            fs::read(dst.join("mods").join("test.jar")).unwrap()
        );
    }

    #[test]
    fn test_clone_source_not_a_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let not_dir = tmp.path().join("nonexistent");
        let dst = tmp.path().join("dest");
        let prefs = ClonePrefs::default();
        let result = clone_instance(&not_dir, &dst, &prefs);
        assert!(result.is_err());
    }

    #[test]
    fn test_clone_default_prefs_all_true() {
        let prefs = ClonePrefs::default();
        assert!(prefs.copy_saves);
        assert!(prefs.copy_mods);
        assert!(prefs.copy_resource_packs);
        assert!(prefs.copy_shader_packs);
        assert!(prefs.copy_screenshots);
        assert!(prefs.copy_config);
        assert!(prefs.copy_servers);
        assert!(prefs.copy_options);
        assert!(!prefs.use_hard_links);
        assert!(!prefs.use_sym_links);
    }

    #[test]
    fn test_clone_refuses_symlinked_directory_and_cleans_destination() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("original");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(src.join("mods")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("private.txt"), b"private").unwrap();

        let link = src.join("mods").join("outside-mods");
        if !create_directory_symlink(&outside, &link) {
            eprintln!("SKIPPED: creating a directory symlink requires elevation here");
            return;
        }

        let dst = tmp.path().join("clone");
        let error = clone_instance(&src, &dst, &ClonePrefs::default()).unwrap_err();
        assert!(error.contains(link.to_string_lossy().as_ref()), "{error}");
        assert!(!path_is_present(&dst).unwrap());
    }

    #[test]
    fn test_clone_refuses_symlinked_file_without_copying_outside_content() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("original");
        let outside = tmp.path().join("outside.txt");
        fs::create_dir_all(src.join("config")).unwrap();
        fs::write(&outside, b"must not appear in clone").unwrap();

        let link = src.join("config").join("outside.txt");
        if !create_file_symlink(&outside, &link) {
            eprintln!("SKIPPED: creating a file symlink requires elevation here");
            return;
        }

        let dst = tmp.path().join("clone");
        let error = clone_instance(&src, &dst, &ClonePrefs::default()).unwrap_err();
        assert!(error.contains(link.to_string_lossy().as_ref()), "{error}");
        assert!(!path_is_present(&dst).unwrap());
    }

    #[test]
    fn test_clone_refuses_symlink_cycle_in_bounded_time() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("original");
        fs::create_dir_all(src.join("mods")).unwrap();

        let link = src.join("mods").join("ancestor");
        if !create_directory_symlink(&src, &link) {
            eprintln!("SKIPPED: creating a directory symlink requires elevation here");
            return;
        }

        let dst = tmp.path().join("clone");
        let started = std::time::Instant::now();
        let result = clone_instance(&src, &dst, &ClonePrefs::default());
        assert!(result.is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        assert!(!path_is_present(&dst).unwrap());
    }

    #[test]
    fn test_clone_refuses_dangling_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("original");
        fs::create_dir_all(src.join("saves")).unwrap();

        let link = src.join("saves").join("missing-world");
        if !create_directory_symlink(&tmp.path().join("does-not-exist"), &link) {
            eprintln!("SKIPPED: creating a directory symlink requires elevation here");
            return;
        }

        let dst = tmp.path().join("clone");
        let error = clone_instance(&src, &dst, &ClonePrefs::default()).unwrap_err();
        assert!(error.contains(link.to_string_lossy().as_ref()), "{error}");
        assert!(!path_is_present(&dst).unwrap());
    }

    #[test]
    fn test_clone_existing_destination_is_preserved() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("original");
        let dst = tmp.path().join("clone");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dst).unwrap();
        let protected = dst.join("keep.txt");
        fs::write(&protected, b"do not delete").unwrap();

        assert!(clone_instance(&src, &dst, &ClonePrefs::default()).is_err());
        assert_eq!(fs::read(&protected).unwrap(), b"do not delete");
    }

    #[test]
    fn test_clone_destination_equal_to_source_preserves_source() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("original");
        fs::create_dir_all(src.join("mods")).unwrap();
        let protected = src.join("mods").join("keep.jar");
        fs::write(&protected, b"source data").unwrap();

        assert!(clone_instance(&src, &src, &ClonePrefs::default()).is_err());
        assert_eq!(fs::read(&protected).unwrap(), b"source data");
    }

    #[test]
    fn test_clone_destination_inside_source_preserves_source() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("original");
        fs::create_dir_all(src.join("mods")).unwrap();
        let protected = src.join("mods").join("keep.jar");
        fs::write(&protected, b"source data").unwrap();
        let dst = src.join("clone");

        assert!(clone_instance(&src, &dst, &ClonePrefs::default()).is_err());
        assert_eq!(fs::read(&protected).unwrap(), b"source data");
        assert!(!path_is_present(&dst).unwrap());
    }

    #[test]
    fn test_clone_mid_copy_failure_removes_only_owned_staging() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("original");
        let dst = tmp.path().join("clone");
        fs::create_dir_all(src.join("mods")).unwrap();
        fs::write(src.join("mods").join("first.jar"), b"first").unwrap();
        fs::write(src.join("mods").join("second.jar"), b"second").unwrap();

        TEST_FAIL_AFTER_ENTRIES.with(|fail_after| fail_after.set(Some(2)));
        let result = clone_instance(&src, &dst, &ClonePrefs::default());
        TEST_FAIL_AFTER_ENTRIES.with(|fail_after| fail_after.set(None));

        assert!(result.is_err());
        assert!(!path_is_present(&dst).unwrap());
        let staging_left = fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".agora-clone-")
            });
        assert!(!staging_left);
    }

    #[cfg(windows)]
    #[test]
    fn test_clone_refuses_windows_junction() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("original");
        let outside = tmp.path().join("outside");
        let junction = src.join("resourcepacks").join("outside-resourcepacks");
        fs::create_dir_all(src.join("resourcepacks")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("private.txt"), b"private").unwrap();

        let status = Command::new("cmd")
            .args([
                "/C",
                "mklink",
                "/J",
                junction.to_string_lossy().as_ref(),
                outside.to_string_lossy().as_ref(),
            ])
            .status()
            .unwrap();
        assert!(status.success(), "mklink /J failed with {status}");

        let dst = tmp.path().join("clone");
        let error = clone_instance(&src, &dst, &ClonePrefs::default()).unwrap_err();
        // The message formats the path with `{:?}`, which escapes Windows
        // separators, so compare against the same rendering rather than the
        // Display form.
        assert!(error.contains(&format!("{junction:?}")), "{error}");
        assert!(!path_is_present(&dst).unwrap());
        // Nothing outside the instance was copied.
        assert!(!dst.exists());
    }
}
