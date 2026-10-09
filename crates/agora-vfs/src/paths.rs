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

/// Which folder a path was opened through: the mount, or one of the configured aliases (by its
/// index in `aliases`). An alias shows exactly the view of the mount.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    Mount,
    Alias(usize),
}

/// A path inside the view: the mount-relative path, and the folder it was reached through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    pub rel: String,
    pub via: Via,
}

/// One folder the view is shown at (the mount, or an alias).
#[derive(Debug, Clone)]
pub struct Prefix {
    pub via: Via,
    /// The folder as configured: normalized, original case, no trailing backslash.
    pub original: String,
    /// Every NT name that names this folder, without its leading `\??\`, matched case-insensitively.
    /// The drive form (`D:\...`) comes first. The DLL adds the volume GUID, device and 8.3 forms at
    /// start-up (`add_system_forms`); the configuration alone gives only the drive form.
    pub forms: Vec<String>,
    /// `\Device\HarddiskVolumeN` of the folder's volume, when known.
    pub device: Option<String>,
}

impl Prefix {
    pub fn new(via: Via, original: String) -> Self {
        Self {
            via,
            forms: vec![original.clone()],
            original,
            device: None,
        }
    }

    /// The folder's path below its drive: `\Games\Skyrim`, or `""` for a drive root. `None` for a
    /// folder without a drive letter, which has no volume-relative names.
    pub fn tail(&self) -> Option<&str> {
        let b = self.original.as_bytes();
        if b.len() >= 2 && b[1] == b':' {
            Some(&self.original[2..])
        } else {
            None
        }
    }
}

/// `name` is `form`, or a path inside it; the rest, without its leading and trailing backslash.
fn strip_form(name: &str, form: &str) -> Option<String> {
    if name.len() < form.len() || !name.is_char_boundary(form.len()) {
        return None;
    }
    let (head, tail) = name.split_at(form.len());
    if !head.eq_ignore_ascii_case(form) {
        return None;
    }
    if tail.is_empty() {
        return Some(String::new());
    }
    tail.strip_prefix('\\')
        .map(|t| t.trim_end_matches('\\').to_string())
}

/// The path inside the view that an absolute NT name (or a Win32 name without `\\?\`) names, and
/// the folder it came through. Every form of every prefix is tried; the longest match wins.
pub fn locate(nt_name: &str, cfg: &RuntimeConfig) -> Option<Located> {
    let name = nt_name.strip_prefix("\\??\\").unwrap_or(nt_name);
    let mut best: Option<(usize, Located)> = None;
    for prefix in &cfg.prefixes {
        for form in &prefix.forms {
            if best.as_ref().is_some_and(|(len, _)| *len >= form.len()) {
                continue;
            }
            if let Some(rel) = strip_form(name, form) {
                best = Some((
                    form.len(),
                    Located {
                        rel,
                        via: prefix.via,
                    },
                ));
            }
        }
    }
    best.map(|(_, located)| located)
}

/// The path relative to the mount, for an absolute NT name inside the mount or an alias.
pub fn rel_of(nt_name: &str, cfg: &RuntimeConfig) -> Option<String> {
    locate(nt_name, cfg).map(|l| l.rel)
}

/// The name a handle opened through `via` at `rel` reports, relative to its volume: what
/// `FileNameInformation` returns, for example `\Games\Skyrim\Data\mod.esp`.
pub fn view_path(via: Via, rel: &str, cfg: &RuntimeConfig) -> Option<String> {
    let tail = cfg.prefix(via).tail()?;
    Some(match (tail.is_empty(), rel.is_empty()) {
        (true, true) => "\\".to_string(),
        (false, true) => tail.to_string(),
        (true, false) => format!("\\{rel}"),
        (false, false) => format!("{tail}\\{rel}"),
    })
}

/// The name a handle opened through `via` at `rel` reports with its device, what
/// `ObjectNameInformation` returns: `\Device\HarddiskVolumeN\Games\Skyrim\Data\mod.esp`.
pub fn device_path(via: Via, rel: &str, cfg: &RuntimeConfig) -> Option<String> {
    let device = cfg.prefix(via).device.as_ref()?;
    let view = view_path(via, rel, cfg)?;
    Some(if view == "\\" {
        device.clone()
    } else {
        format!("{device}{view}")
    })
}

/// Where a process start of a path inside the view really runs from: the mount folder for the
/// folder itself, else the upper or lower file that wins. `None` when the path is not in the view
/// at all (a file that is in no layer, or whited out), which the caller refuses.
pub enum Start {
    Real(PathBuf),
    NotInView,
}

pub fn start_path(located: &Located, cfg: &RuntimeConfig) -> Start {
    if located.rel.is_empty() {
        return Start::Real(PathBuf::from(&cfg.mount_original));
    }
    match resolve(&located.rel, cfg) {
        Resolved::Upper(p) | Resolved::Lower(p) => Start::Real(p),
        Resolved::WhitedOut | Resolved::Missing => Start::NotInView,
    }
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
static ROOTS: OnceLock<Mutex<HashMap<usize, Option<Located>>>> = OnceLock::new();

#[cfg(windows)]
pub fn roots() -> &'static Mutex<HashMap<usize, Option<Located>>> {
    ROOTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The view path of a directory handle, if it is inside the view. Win32 sends every relative path
/// as a name relative to the current-directory handle, which the process opened before our hooks
/// existed, so untracked handles are asked for their path.
#[cfg(windows)]
pub unsafe fn rel_of_handle(handle: HANDLE, cfg: &RuntimeConfig) -> Option<Located> {
    if let Some(t) = handles().lock().unwrap().get(&(handle as usize)) {
        return Some(Located {
            rel: t.rel.clone(),
            via: t.via,
        });
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
    let located = if n == 0 || n as usize >= buf.len() {
        None
    } else {
        let path = String::from_utf16_lossy(&buf[..n as usize]);
        locate(path.strip_prefix(r"\\?\").unwrap_or(&path), cfg)
    };
    roots()
        .lock()
        .unwrap()
        .insert(handle as usize, located.clone());
    located
}

#[cfg(windows)]
pub unsafe fn rel_of_attrs(oa: *const ObjectAttributes, cfg: &RuntimeConfig) -> Option<Located> {
    if oa.is_null() {
        return None;
    }
    let name = unicode((*oa).object_name)?;
    if (*oa).root_directory.is_null() || name.starts_with('\\') {
        return locate(&name, cfg);
    }
    let root = rel_of_handle((*oa).root_directory, cfg)?;
    let name = name.trim_end_matches('\\');
    let rel = match (root.rel.is_empty(), name.is_empty()) {
        (_, true) => root.rel,
        (true, false) => name.to_string(),
        (false, false) => format!("{}\\{name}", root.rel),
    };
    Some(Located { rel, via: root.via })
}

/// Work out the NT names of the mount and of each alias that the DLL recognises: every form that
/// names the same folder (`add_system_forms`, at the DLL's start). A folder without a drive letter
/// keeps the forms it has.
#[cfg(windows)]
pub fn add_system_forms(cfg: &mut RuntimeConfig) {
    for prefix in &mut cfg.prefixes {
        let Some(tail) = prefix.tail().map(str::to_string) else {
            continue;
        };
        let drive = prefix.original[..2].to_string();
        let mut heads: Vec<String> = vec![drive.clone()];
        if let Some(guid) = unsafe { volume_guid(&format!("{drive}\\")) } {
            heads.push(guid);
        }
        if let Some(device) = unsafe { dos_device(&drive) } {
            heads.push(format!("GLOBALROOT{device}"));
            heads.push(device.clone());
            prefix.device = Some(device);
        }
        let mut tails = vec![tail];
        if let Some(short) = unsafe { short_path(&prefix.original) } {
            let differs = !short.eq_ignore_ascii_case(&prefix.original);
            if differs && short.len() >= 2 && short.as_bytes()[1] == b':' {
                tails.push(short[2..].to_string());
            }
        }
        let mut forms: Vec<String> = Vec::new();
        for head in &heads {
            for tail in &tails {
                let form = format!("{head}{tail}");
                if !forms.iter().any(|f| f.eq_ignore_ascii_case(&form)) {
                    forms.push(form);
                }
            }
        }
        prefix.forms = forms;
    }
}

/// `Volume{GUID}` for the volume of `root` (`D:\`), from GetVolumeNameForVolumeMountPointW.
#[cfg(windows)]
unsafe fn volume_guid(root: &str) -> Option<String> {
    use windows_sys::Win32::Storage::FileSystem::GetVolumeNameForVolumeMountPointW;
    let wide: Vec<u16> = root.encode_utf16().chain(Some(0)).collect();
    let mut buf = [0u16; 128];
    if GetVolumeNameForVolumeMountPointW(wide.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) == 0 {
        return None;
    }
    let s = wide_str(&buf);
    Some(s.strip_prefix(r"\\?\")?.trim_end_matches('\\').to_string())
}

/// `\Device\HarddiskVolumeN` for a drive letter (`D:`), from QueryDosDeviceW.
#[cfg(windows)]
unsafe fn dos_device(drive: &str) -> Option<String> {
    use windows_sys::Win32::Storage::FileSystem::QueryDosDeviceW;
    let wide: Vec<u16> = drive.encode_utf16().chain(Some(0)).collect();
    let mut buf = vec![0u16; 1024];
    let n = QueryDosDeviceW(wide.as_ptr(), buf.as_mut_ptr(), buf.len() as u32);
    if n == 0 {
        return None;
    }
    // The answer is a list of NUL-terminated names; the first is the volume's device.
    let first = wide_str(&buf);
    first.starts_with(r"\Device\").then_some(first)
}

/// The 8.3 form of a path, from GetShortPathNameW. `None` when it has none.
#[cfg(windows)]
unsafe fn short_path(path: &str) -> Option<String> {
    use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;
    let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    let mut buf = vec![0u16; 1024];
    let n = GetShortPathNameW(wide.as_ptr(), buf.as_mut_ptr(), buf.len() as u32);
    if n == 0 || n as usize >= buf.len() {
        return None;
    }
    Some(String::from_utf16_lossy(&buf[..n as usize]))
}

#[cfg(windows)]
fn wide_str(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::VfsConfig;

    /// The mount and one alias, with the forms the DLL adds at start-up, written out by hand.
    fn config() -> RuntimeConfig {
        let json = r#"{
            "version": 2,
            "mount": "D:\\Games\\Skyrim",
            "upper": "C:\\Instances\\upper",
            "lowers": ["D:\\Games\\Skyrim", "D:\\Bases\\base"],
            "dll": "C:\\Agora\\agora_vfs.dll",
            "aliases": ["E:\\Steam\\Skyrim Special Edition"]
        }"#;
        let mut cfg = VfsConfig::from_json_str(json).unwrap();
        cfg.prefixes[0].forms.extend([
            r"Volume{1111}\Games\Skyrim".to_string(),
            r"\Device\HarddiskVolume3\Games\Skyrim".to_string(),
            r"GLOBALROOT\Device\HarddiskVolume3\Games\Skyrim".to_string(),
            r"D:\GAMES~1\SKYRIM~1".to_string(),
        ]);
        cfg.prefixes[1].device = Some(r"\Device\HarddiskVolume4".to_string());
        cfg.prefixes[1].forms = vec![
            r"E:\Steam\Skyrim Special Edition".to_string(),
            r"Volume{2222}\Steam\Skyrim Special Edition".to_string(),
            r"\Device\HarddiskVolume4\Steam\Skyrim Special Edition".to_string(),
            r"GLOBALROOT\Device\HarddiskVolume4\Steam\Skyrim Special Edition".to_string(),
            r"E:\STEAM~1\SKYRIM~1".to_string(),
        ];
        cfg
    }

    #[test]
    fn an_alias_resolves_to_the_same_rel_as_the_mount_in_every_form() {
        let cfg = config();
        let alias = Via::Alias(0);
        let forms = [
            r"\??\E:\Steam\Skyrim Special Edition\Data\mod.esp",
            r"\??\Volume{2222}\Steam\Skyrim Special Edition\Data\mod.esp",
            r"\Device\HarddiskVolume4\Steam\Skyrim Special Edition\Data\mod.esp",
            r"\??\GLOBALROOT\Device\HarddiskVolume4\Steam\Skyrim Special Edition\Data\mod.esp",
            r"\??\E:\STEAM~1\SKYRIM~1\Data\mod.esp",
        ];
        for form in forms {
            assert_eq!(
                locate(form, &cfg),
                Some(Located {
                    rel: r"Data\mod.esp".to_string(),
                    via: alias
                }),
                "{form}"
            );
        }
        // Matching ignores case; the rel keeps the spelling the caller used.
        assert_eq!(
            locate(r"e:\steam\skyrim special edition\data\MOD.esp", &cfg),
            Some(Located {
                rel: r"data\MOD.esp".to_string(),
                via: alias
            })
        );
        // Every form of the mount gives the mount.
        let mount_forms = [
            r"\??\D:\Games\Skyrim\Data\mod.esp",
            r"\??\Volume{1111}\Games\Skyrim\Data\mod.esp",
            r"\Device\HarddiskVolume3\Games\Skyrim\Data\mod.esp",
            r"\??\GLOBALROOT\Device\HarddiskVolume3\Games\Skyrim\Data\mod.esp",
            r"\??\D:\GAMES~1\SKYRIM~1\Data\mod.esp",
        ];
        for form in mount_forms {
            assert_eq!(
                locate(form, &cfg),
                Some(Located {
                    rel: r"Data\mod.esp".to_string(),
                    via: Via::Mount
                }),
                "{form}"
            );
        }
    }

    #[test]
    fn a_name_that_only_starts_like_a_form_is_not_in_the_view() {
        let cfg = config();
        assert_eq!(rel_of(r"\??\D:\Games\SkyrimPrefix\x", &cfg), None);
        assert_eq!(
            rel_of(r"\??\E:\Steam\Skyrim Special Edition2\x", &cfg),
            None
        );
        assert_eq!(rel_of(r"\??\Volume{1111}\Games\Skyrimx", &cfg), None);
        assert_eq!(rel_of(r"\Device\HarddiskVolume4\Other\x", &cfg), None);
    }

    #[test]
    fn the_root_of_an_alias_is_the_empty_rel() {
        let cfg = config();
        assert_eq!(
            locate(r"\??\E:\Steam\Skyrim Special Edition", &cfg),
            Some(Located {
                rel: String::new(),
                via: Via::Alias(0)
            })
        );
    }

    #[test]
    fn view_and_device_names_start_with_the_folder_the_handle_came_through() {
        let cfg = config();
        assert_eq!(
            view_path(Via::Alias(0), r"Data\mod.esp", &cfg).as_deref(),
            Some(r"\Steam\Skyrim Special Edition\Data\mod.esp")
        );
        assert_eq!(
            device_path(Via::Alias(0), r"Data\mod.esp", &cfg).as_deref(),
            Some(r"\Device\HarddiskVolume4\Steam\Skyrim Special Edition\Data\mod.esp")
        );
        assert_eq!(
            view_path(Via::Mount, "", &cfg).as_deref(),
            Some(r"\Games\Skyrim")
        );
        // A drive root has no tail: its view is the volume root.
        let mut root = config();
        root.prefixes[0].original = "D:".to_string();
        assert_eq!(view_path(Via::Mount, "", &root).as_deref(), Some("\\"));
        assert_eq!(view_path(Via::Mount, "x", &root).as_deref(), Some("\\x"));
    }

    #[test]
    fn a_device_is_needed_for_the_device_name() {
        let cfg = VfsConfig::from_json_str(
            r#"{"version": 2, "mount": "D:\\G", "upper": "C:\\u", "dll": "C:\\d.dll",
                "aliases": ["E:\\S"]}"#,
        )
        .unwrap();
        assert_eq!(device_path(Via::Mount, "", &cfg), None);
        assert_eq!(
            view_path(Via::Alias(0), "x", &cfg).as_deref(),
            Some(r"\S\x")
        );
    }
}
