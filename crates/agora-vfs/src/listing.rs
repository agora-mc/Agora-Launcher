use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::Foundation::HANDLE;

use crate::config::RuntimeConfig;
use crate::hooks::hooks;
use crate::nt::*;
use crate::paths::whited_out;
use crate::util::{guarded, log};

/// (offset of FileNameLength, offset of FileName) for the listing formats we merge.
pub fn name_offsets(class: u32) -> Option<(usize, usize)> {
    Some(match class {
        1 => (60, 64),   // FileDirectoryInformation
        2 => (60, 68),   // FileFullDirectoryInformation
        3 => (60, 94),   // FileBothDirectoryInformation
        12 => (8, 12),   // FileNamesInformation
        37 => (60, 104), // FileIdBothDirectoryInformation
        38 => (60, 80),  // FileIdFullDirectoryInformation
        60 => (60, 88),  // FileIdExtdDirectoryInformation
        63 => (60, 114), // FileIdExtdBothDirectoryInformation
        _ => return None,
    })
}

pub struct Listing {
    pub class: u32,
    pub entries: Vec<Vec<u8>>, // each entry's bytes, NextEntryOffset zeroed
    pub cursor: usize,
    pub returned_any: bool,
}

static LISTINGS: OnceLock<Mutex<HashMap<usize, Listing>>> = OnceLock::new();
pub fn listings() -> &'static Mutex<HashMap<usize, Listing>> {
    LISTINGS.get_or_init(|| Mutex::new(HashMap::new()))
}

static UNMERGED_CLASSES: OnceLock<Mutex<HashSet<u32>>> = OnceLock::new();
pub fn log_unmerged_class_once(class: u32, rel: &str) {
    let mut set = UNMERGED_CLASSES
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .unwrap();
    if set.insert(class) {
        log(format!(
            "listing class {class} not merged for {rel} (passed through)"
        ));
    }
}

pub fn entry_name(entry: &[u8], class: u32) -> String {
    let (len_at, name_at) = name_offsets(class).unwrap();
    let len = u32::from_le_bytes(entry[len_at..len_at + 4].try_into().unwrap()) as usize;
    let bytes = &entry[name_at..(name_at + len).min(entry.len())];
    let wide: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&wide)
}

/// Raw entries of one real directory, in `class` format.
pub unsafe fn read_real_dir(dir: &Path, class: u32) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut wide: Vec<u16> = "\\??\\"
        .encode_utf16()
        .chain(dir.as_os_str().encode_wide())
        .collect();
    let len = (wide.len() * 2) as u16;
    let mut name = UnicodeString {
        length: len,
        maximum_length: len,
        buffer: wide.as_mut_ptr(),
    };
    let mut oa = ObjectAttributes {
        length: std::mem::size_of::<ObjectAttributes>() as u32,
        root_directory: std::ptr::null_mut(),
        object_name: &mut name,
        attributes: 0x40, // OBJ_CASE_INSENSITIVE
        security_descriptor: std::ptr::null_mut(),
        security_quality_of_service: std::ptr::null_mut(),
    };
    let mut handle: HANDLE = std::ptr::null_mut();
    let mut iosb = [0u64; 2];
    let status = hooks().open.call(
        &mut handle,
        FILE_LIST_DIRECTORY | SYNCHRONIZE,
        &mut oa,
        iosb.as_mut_ptr() as *mut c_void,
        FILE_SHARE_ALL,
        FILE_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT,
    );
    if status < 0 {
        return out;
    }
    let (len_at, name_at) = name_offsets(class).unwrap();
    let mut buf = vec![0u64; 64 * 1024 / 8];
    let mut first = 1u8;
    loop {
        let status = hooks().query_dir.call(
            handle,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            iosb.as_mut_ptr() as *mut c_void,
            buf.as_mut_ptr() as *mut c_void,
            (buf.len() * 8) as u32,
            class,
            0,
            std::ptr::null_mut(),
            first,
        );
        first = 0;
        if status < 0 || status == STATUS_NO_MORE_FILES {
            break;
        }
        let bytes = std::slice::from_raw_parts(buf.as_ptr() as *const u8, iosb[1] as usize);
        let mut offset = 0usize;
        while offset + name_at <= bytes.len() {
            let next = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
            let name_len = u32::from_le_bytes(
                bytes[offset + len_at..offset + len_at + 4]
                    .try_into()
                    .unwrap(),
            ) as usize;
            let end = (offset + name_at + name_len).min(bytes.len());
            let mut entry = bytes[offset..end].to_vec();
            entry[0..4].copy_from_slice(&0u32.to_le_bytes());
            out.push(entry);
            if next == 0 {
                break;
            }
            offset += next;
        }
    }
    hooks().close.call(handle);
    out
}

/// Windows file-name matching, including the DOS wildcards FindFirstFile translates to.
pub fn wildcard(pattern: &str, name: &str) -> bool {
    fn go(p: &[char], n: &[char]) -> bool {
        match (p.first(), n.first()) {
            (None, None) => true,
            (Some('*'), _) | (Some('<'), _) => go(&p[1..], n) || (!n.is_empty() && go(p, &n[1..])),
            (Some('?'), Some(_)) | (Some('>'), Some(_)) => go(&p[1..], &n[1..]),
            (Some('"'), Some('.')) => go(&p[1..], &n[1..]),
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b) && go(&p[1..], &n[1..]),
            (Some(_), None) => p.iter().all(|c| matches!(c, '*' | '<' | '>' | '"')),
            (None, Some(_)) => false,
        }
    }
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    go(&p, &n)
}

/// The merged listing of virtual directory `rel`: upper first, then lowers in priority order;
/// a name seen in a higher layer hides the same name below, whiteouts hide it entirely.
/// If `mount == lowers[0]` or any folders point to the exact same path, duplicates are de-duped.
pub unsafe fn build_listing(
    rel: &str,
    class: u32,
    pattern: Option<String>,
    cfg: &RuntimeConfig,
) -> Vec<Vec<u8>> {
    let mut sources = Vec::new();
    let mut added_dirs: HashSet<PathBuf> = HashSet::new();

    let upper_dir = cfg.upper.join(rel);
    if added_dirs.insert(upper_dir.clone()) {
        sources.push(upper_dir);
    }

    for lower in &cfg.lowers {
        let dir = lower.join(rel);
        if added_dirs.insert(dir.clone()) {
            sources.push(dir);
        }
    }

    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for dir in sources {
        if !dir.is_dir() {
            continue;
        }
        for entry in read_real_dir(&dir, class) {
            let name = entry_name(&entry, class);
            let key = name.to_lowercase();
            if rel.is_empty() && key == ".agvfs-wh" {
                continue;
            }
            if !seen.insert(key) {
                continue;
            }
            if name != "." && name != ".." {
                let child = if rel.is_empty() {
                    name.clone()
                } else {
                    format!("{rel}\\{name}")
                };
                if whited_out(&child, &cfg.upper) {
                    continue;
                }
            }
            if let Some(p) = &pattern {
                if !wildcard(p, &name) {
                    continue;
                }
            }
            entries.push(entry);
        }
    }
    entries
}

#[allow(clippy::too_many_arguments)]
pub unsafe fn query_directory(
    handle: HANDLE,
    iosb: *mut c_void,
    out: *mut c_void,
    length: u32,
    class: u32,
    single: bool,
    pattern: *mut UnicodeString,
    restart: bool,
    cfg: &RuntimeConfig,
) -> Option<NTSTATUS> {
    if crate::util::is_inside() {
        return None;
    }
    let tracked = crate::handles()
        .lock()
        .unwrap()
        .get(&(handle as usize))
        .cloned()?;
    if !tracked.dir {
        return None;
    }
    if name_offsets(class).is_none() {
        guarded(|| log_unmerged_class_once(class, &tracked.rel));
        return None;
    }
    guarded(|| {
        log(format!(
            "list {:?} class {class} single {single} restart {restart} pattern {:?}",
            tracked.rel,
            unicode(pattern)
        ))
    });
    let mut map = listings().lock().unwrap();
    let needs_build = restart
        || map
            .get(&(handle as usize))
            .map(|l| l.class != class)
            .unwrap_or(true);
    if needs_build {
        let pattern = unicode(pattern).filter(|p| !p.is_empty() && p != "*");
        let entries = guarded(|| build_listing(&tracked.rel, class, pattern, cfg))?;
        map.insert(
            handle as usize,
            Listing {
                class,
                entries,
                cursor: 0,
                returned_any: false,
            },
        );
    }
    let listing = map.get_mut(&(handle as usize)).unwrap();
    let out_bytes = std::slice::from_raw_parts_mut(out as *mut u8, length as usize);
    let mut written = 0usize;
    let mut last_entry_at: Option<usize> = None;
    while listing.cursor < listing.entries.len() {
        let entry = &listing.entries[listing.cursor];
        let start = written.next_multiple_of(8);
        if start + entry.len() > out_bytes.len() {
            if last_entry_at.is_none() {
                let iosb = iosb as *mut u64;
                *iosb = STATUS_BUFFER_OVERFLOW as u32 as u64;
                *iosb.add(1) = 0;
                return Some(STATUS_BUFFER_OVERFLOW);
            }
            break;
        }
        if let Some(prev) = last_entry_at {
            out_bytes[prev..prev + 4].copy_from_slice(&((start - prev) as u32).to_le_bytes());
        }
        out_bytes[start..start + entry.len()].copy_from_slice(entry);
        last_entry_at = Some(start);
        written = start + entry.len();
        listing.cursor += 1;
        if single {
            break;
        }
    }
    let status = if last_entry_at.is_some() {
        listing.returned_any = true;
        STATUS_SUCCESS
    } else if listing.returned_any {
        STATUS_NO_MORE_FILES
    } else {
        listing.returned_any = true;
        STATUS_NO_SUCH_FILE
    };
    let iosb = iosb as *mut u64;
    *iosb = status as u32 as u64;
    *iosb.add(1) = written as u64;
    Some(status)
}
