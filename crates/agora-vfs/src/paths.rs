use crate::config::RuntimeConfig;
use std::path::{Path, PathBuf};

#[cfg(windows)]
use std::collections::HashMap;
#[cfg(windows)]
use std::sync::{Mutex, OnceLock};
#[cfg(windows)]
use windows_sys::Win32::Foundation::HANDLE;

#[cfg(windows)]
use crate::handles;
#[cfg(windows)]
use crate::nt::{unicode, ObjectAttributes};

/// The path relative to the mount, for an absolute NT name inside it.
pub fn rel_of(nt_name: &str, cfg: &RuntimeConfig) -> Option<String> {
    let p = nt_name.strip_prefix("\\??\\").unwrap_or(nt_name);
    if p.len() < cfg.mount_len || !p.is_char_boundary(cfg.mount_len) {
        return None;
    }
    let (head, tail) = p.split_at(cfg.mount_len);
    if !head.eq_ignore_ascii_case(&cfg.mount) {
        return None;
    }
    if tail.is_empty() {
        return Some(String::new());
    }
    tail.strip_prefix('\\')
        .map(|t| t.trim_end_matches('\\').to_string())
}

/// `<upper>\.agvfs-wh\<rel>.wh`, a marker file. The suffix keeps the marker for a folder distinct
/// from the folder that holds the markers of its children.
pub fn whiteout_path(rel: &str, upper: &Path) -> PathBuf {
    let mut p = upper.join(".agvfs-wh").join(rel).into_os_string();
    p.push(".wh");
    PathBuf::from(p)
}

pub fn whited_out(rel: &str, upper: &Path) -> bool {
    whiteout_path(rel, upper).is_file()
}

pub fn add_whiteout(rel: &str, upper: &Path) {
    let p = whiteout_path(rel, upper);
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(p, b"");
}

pub fn clear_whiteout(rel: &str, upper: &Path) {
    let _ = std::fs::remove_file(whiteout_path(rel, upper));
}

pub fn in_any_lower(rel: &str, lowers: &[PathBuf]) -> Option<PathBuf> {
    lowers.iter().map(|l| l.join(rel)).find(|p| p.exists())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    Upper(PathBuf),
    Lower(PathBuf),
    WhitedOut,
    Missing,
}

pub fn resolve(rel: &str, cfg: &RuntimeConfig) -> Resolved {
    if rel.is_empty() {
        return Resolved::Missing; // the mount folder itself: the real one
    }
    if whited_out(rel, &cfg.upper) {
        return Resolved::WhitedOut;
    }
    let upper = cfg.upper.join(rel);
    if upper.exists() {
        return Resolved::Upper(upper);
    }
    match in_any_lower(rel, &cfg.lowers) {
        Some(p) => Resolved::Lower(p),
        None => Resolved::Missing,
    }
}

pub fn upper_for_write(rel: &str, upper_root: &Path) -> PathBuf {
    let p = upper_root.join(rel);
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    p
}

#[cfg(windows)]
static ROOTS: OnceLock<Mutex<HashMap<usize, Option<String>>>> = OnceLock::new();

#[cfg(windows)]
pub fn roots() -> &'static Mutex<HashMap<usize, Option<String>>> {
    ROOTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The mount-relative path of a directory handle, if it is inside the mount. Win32 sends every
/// relative path as a name relative to the current-directory handle, which the process opened
/// before our hooks existed, so untracked handles are asked for their path.
#[cfg(windows)]
pub unsafe fn rel_of_handle(handle: HANDLE, cfg: &RuntimeConfig) -> Option<String> {
    if let Some(t) = handles().lock().unwrap().get(&(handle as usize)) {
        return Some(t.rel.clone());
    }
    if let Some(cached) = roots().lock().unwrap().get(&(handle as usize)) {
        return cached.clone();
    }
    let mut buf = vec![0u16; 1024];
    let n = windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW(
        handle,
        buf.as_mut_ptr(),
        buf.len() as u32,
        0,
    );
    let rel = if n == 0 || n as usize >= buf.len() {
        None
    } else {
        let path = String::from_utf16_lossy(&buf[..n as usize]);
        rel_of(path.strip_prefix(r"\\?\").unwrap_or(&path), cfg)
    };
    roots().lock().unwrap().insert(handle as usize, rel.clone());
    rel
}

#[cfg(windows)]
pub unsafe fn rel_of_attrs(oa: *const ObjectAttributes, cfg: &RuntimeConfig) -> Option<String> {
    if oa.is_null() {
        return None;
    }
    let name = unicode((*oa).object_name)?;
    if (*oa).root_directory.is_null() || name.starts_with('\\') {
        return rel_of(&name, cfg);
    }
    let root = rel_of_handle((*oa).root_directory, cfg)?;
    let name = name.trim_end_matches('\\');
    Some(match (root.is_empty(), name.is_empty()) {
        (_, true) => root,
        (true, false) => name.to_string(),
        (false, false) => format!("{root}\\{name}"),
    })
}
