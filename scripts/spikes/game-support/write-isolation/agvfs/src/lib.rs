//! Spike 2 prototype: a copy-on-write layered VFS for Windows, injected into a game process.
//!
//! Configuration comes from the file named by `AGVFS_CONFIG` (inherited by child processes):
//!
//! ```text
//! mount=D:\...\mount        the folder the game sees
//! upper=D:\...\overwrite    the instance's writable layer
//! lower=D:\...\modA         lower layers, highest priority first (repeatable)
//! lower=D:\...\base
//! dll=D:\...\agvfs.dll      injected into child processes
//! log=D:\...\agvfs.log      optional
//! ```
//!
//! Rules: a lower file is never opened with write or delete rights. Writing to it copies it up
//! first (a whole-file rewrite skips the copy). Deleting it records a whiteout in the upper layer
//! (`<upper>\.agvfs-wh\<path>.wh`), and renaming it copies it to the new name and whites out the old.
//! Hooks sit on the NT calls every Win32 file API funnels through, so `DeleteFileW`, `MoveFileExW`,
//! `ReplaceFileW` and handle-based rename/delete are all covered.
//!
//! Not yet: directory enumeration merging, `RootDirectory`-relative names, 32-bit processes.

#![allow(non_snake_case, clippy::missing_safety_doc)]

use std::cell::Cell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::io::Write;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use retour::GenericDetour;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::Diagnostics::Debug::WriteProcessMemory;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::Memory::{VirtualAllocEx, MEM_COMMIT, MEM_RESERVE, PAGE_READWRITE};
use windows_sys::Win32::System::Threading::*;

type NTSTATUS = i32;
const STATUS_SUCCESS: NTSTATUS = 0;
const STATUS_OBJECT_NAME_NOT_FOUND: NTSTATUS = 0xC000_0034_u32 as i32;
const STATUS_OBJECT_NAME_COLLISION: NTSTATUS = 0xC000_0035_u32 as i32;
const STATUS_ACCESS_DENIED: NTSTATUS = 0xC000_0022_u32 as i32;

// Access rights
const FILE_WRITE_DATA: u32 = 0x2;
const FILE_APPEND_DATA: u32 = 0x4;
const FILE_WRITE_EA: u32 = 0x10;
const FILE_WRITE_ATTRIBUTES: u32 = 0x100;
const DELETE_ACCESS: u32 = 0x1_0000;
const WRITE_DAC: u32 = 0x4_0000;
const WRITE_OWNER: u32 = 0x8_0000;
const MAXIMUM_ALLOWED: u32 = 0x0200_0000;
const GENERIC_ALL: u32 = 0x1000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const GENERIC_READ: u32 = 0x8000_0000;
const SYNCHRONIZE: u32 = 0x10_0000;
const WRITE_RIGHTS: u32 = FILE_WRITE_DATA
    | FILE_APPEND_DATA
    | FILE_WRITE_EA
    | FILE_WRITE_ATTRIBUTES
    | WRITE_DAC
    | WRITE_OWNER
    | GENERIC_ALL
    | GENERIC_WRITE;

// Create dispositions
const FILE_SUPERSEDE: u32 = 0;
const FILE_OPEN: u32 = 1;
const FILE_CREATE: u32 = 2;
const FILE_OPEN_IF: u32 = 3;
const FILE_OVERWRITE: u32 = 4;
const FILE_OVERWRITE_IF: u32 = 5;

// Create options
const FILE_DIRECTORY_FILE: u32 = 0x1;
const FILE_DELETE_ON_CLOSE: u32 = 0x1000;

// Information classes
const FILE_RENAME_INFORMATION: u32 = 10;
const FILE_DISPOSITION_INFORMATION: u32 = 13;
const FILE_DISPOSITION_INFORMATION_EX: u32 = 64;
const FILE_RENAME_INFORMATION_EX: u32 = 65;
const FILE_RENAME_REPLACE_IF_EXISTS: u32 = 0x1;
const FILE_DISPOSITION_DELETE: u32 = 0x1;

#[repr(C)]
pub struct UnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *mut u16,
}

#[repr(C)]
pub struct ObjectAttributes {
    length: u32,
    root_directory: HANDLE,
    object_name: *mut UnicodeString,
    attributes: u32,
    security_descriptor: *mut c_void,
    security_quality_of_service: *mut c_void,
}

type NtCreateFileFn = unsafe extern "system" fn(
    *mut HANDLE,
    u32,
    *mut ObjectAttributes,
    *mut c_void,
    *mut i64,
    u32,
    u32,
    u32,
    u32,
    *mut c_void,
    u32,
) -> NTSTATUS;
type NtOpenFileFn =
    unsafe extern "system" fn(*mut HANDLE, u32, *mut ObjectAttributes, *mut c_void, u32, u32) -> NTSTATUS;
type NtSetInformationFileFn = unsafe extern "system" fn(HANDLE, *mut c_void, *mut c_void, u32, u32) -> NTSTATUS;
type NtQueryAttributesFileFn = unsafe extern "system" fn(*mut ObjectAttributes, *mut c_void) -> NTSTATUS;
type NtCloseFn = unsafe extern "system" fn(HANDLE) -> NTSTATUS;
type NtQueryInformationByNameFn =
    unsafe extern "system" fn(*mut ObjectAttributes, *mut c_void, *mut c_void, u32, u32) -> NTSTATUS;
type CreateProcessInternalWFn = unsafe extern "system" fn(
    HANDLE,
    *const u16,
    *mut u16,
    *const c_void,
    *const c_void,
    BOOL,
    u32,
    *const c_void,
    *const u16,
    *const c_void,
    *mut PROCESS_INFORMATION,
    *mut HANDLE,
) -> BOOL;

struct Hooks {
    create: GenericDetour<NtCreateFileFn>,
    open: GenericDetour<NtOpenFileFn>,
    set_info: GenericDetour<NtSetInformationFileFn>,
    query_attr: GenericDetour<NtQueryAttributesFileFn>,
    query_full_attr: GenericDetour<NtQueryAttributesFileFn>,
    close: GenericDetour<NtCloseFn>,
    create_process: GenericDetour<CreateProcessInternalWFn>,
    query_dir: GenericDetour<NtQueryDirectoryFileFn>,
    query_dir_ex: GenericDetour<NtQueryDirectoryFileExFn>,
    query_by_name: GenericDetour<NtQueryInformationByNameFn>,
}
unsafe impl Sync for Hooks {}
unsafe impl Send for Hooks {}

struct Config {
    mount: String, // no trailing backslash; compared case-insensitively
    mount_original: String,
    mount_len: usize,
    upper: PathBuf,
    lowers: Vec<PathBuf>,
    dll: PathBuf,
    log: Option<PathBuf>,
    verbose: bool,
}

#[derive(Clone)]
struct Tracked {
    rel: String,
    lower: bool,
    delete_on_close: bool,
    dir: bool,
}

static CONFIG: OnceLock<Config> = OnceLock::new();
static HOOKS: OnceLock<Hooks> = OnceLock::new();
static HANDLES: OnceLock<Mutex<HashMap<usize, Tracked>>> = OnceLock::new();

thread_local! {
    static INSIDE: Cell<bool> = const { Cell::new(false) };
}

/// Runs `f` with hooks passing straight through on this thread (our own file work must not be
/// redirected). Returns `None` when already inside, so the caller calls the original.
fn guarded<T>(f: impl FnOnce() -> T) -> Option<T> {
    if INSIDE.with(|i| i.replace(true)) {
        return None;
    }
    let r = f();
    INSIDE.with(|i| i.set(false));
    Some(r)
}

fn cfg() -> &'static Config {
    CONFIG.get().expect("agvfs config")
}
fn hooks() -> &'static Hooks {
    HOOKS.get().expect("agvfs hooks")
}
fn handles() -> &'static Mutex<HashMap<usize, Tracked>> {
    HANDLES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn log(msg: impl AsRef<str>) {
    if let Some(path) = &cfg().log {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "[{}] {}", std::process::id(), msg.as_ref());
        }
    }
}

// ---------------------------------------------------------------- paths

unsafe fn unicode(u: *const UnicodeString) -> Option<String> {
    if u.is_null() || (*u).buffer.is_null() {
        return None;
    }
    let s = std::slice::from_raw_parts((*u).buffer, (*u).length as usize / 2);
    Some(String::from_utf16_lossy(s))
}

/// The path relative to the mount, for an absolute NT name inside it.
fn rel_of(nt_name: &str) -> Option<String> {
    let c = cfg();
    let p = nt_name.strip_prefix("\\??\\").unwrap_or(nt_name);
    if p.len() < c.mount_len || !p.is_char_boundary(c.mount_len) {
        return None;
    }
    let (head, tail) = p.split_at(c.mount_len);
    if !head.eq_ignore_ascii_case(&c.mount) {
        return None;
    }
    if tail.is_empty() {
        return Some(String::new());
    }
    tail.strip_prefix('\\').map(|t| t.trim_end_matches('\\').to_string())
}

static ROOTS: OnceLock<Mutex<HashMap<usize, Option<String>>>> = OnceLock::new();
fn roots() -> &'static Mutex<HashMap<usize, Option<String>>> {
    ROOTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The mount-relative path of a directory handle, if it is inside the mount. Win32 sends every
/// relative path as a name relative to the current-directory handle, which the process opened
/// before our hooks existed, so untracked handles are asked for their path.
unsafe fn rel_of_handle(handle: HANDLE) -> Option<String> {
    if let Some(t) = handles().lock().unwrap().get(&(handle as usize)) {
        return Some(t.rel.clone());
    }
    if let Some(cached) = roots().lock().unwrap().get(&(handle as usize)) {
        return cached.clone();
    }
    let mut buf = vec![0u16; 1024];
    let n = windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW(handle, buf.as_mut_ptr(), buf.len() as u32, 0);
    let rel = if n == 0 || n as usize >= buf.len() {
        None
    } else {
        let path = String::from_utf16_lossy(&buf[..n as usize]);
        rel_of(path.strip_prefix(r"\\?\").unwrap_or(&path))
    };
    roots().lock().unwrap().insert(handle as usize, rel.clone());
    rel
}

unsafe fn rel_of_attrs(oa: *const ObjectAttributes) -> Option<String> {
    if oa.is_null() {
        return None;
    }
    let name = unicode((*oa).object_name)?;
    if (*oa).root_directory.is_null() || name.starts_with('\\') {
        return rel_of(&name);
    }
    let root = rel_of_handle((*oa).root_directory)?;
    let name = name.trim_end_matches('\\');
    Some(match (root.is_empty(), name.is_empty()) {
        (_, true) => root,
        (true, false) => name.to_string(),
        (false, false) => format!("{root}\\{name}"),
    })
}

/// `<upper>\.agvfs-wh\<rel>.wh`, a marker file. The suffix keeps the marker for a folder distinct
/// from the folder that holds the markers of its children.
fn whiteout_path(rel: &str) -> PathBuf {
    let mut p = cfg().upper.join(".agvfs-wh").join(rel).into_os_string();
    p.push(".wh");
    PathBuf::from(p)
}

fn whited_out(rel: &str) -> bool {
    whiteout_path(rel).is_file()
}

fn add_whiteout(rel: &str) {
    let p = whiteout_path(rel);
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(p, b"");
}

fn clear_whiteout(rel: &str) {
    let _ = std::fs::remove_file(whiteout_path(rel));
}

fn in_any_lower(rel: &str) -> Option<PathBuf> {
    cfg().lowers.iter().map(|l| l.join(rel)).find(|p| p.exists())
}

enum Resolved {
    Upper(PathBuf),
    Lower(PathBuf),
    WhitedOut,
    Missing,
}

fn resolve(rel: &str) -> Resolved {
    if rel.is_empty() {
        return Resolved::Missing; // the mount folder itself: the real one
    }
    if whited_out(rel) {
        return Resolved::WhitedOut;
    }
    let upper = cfg().upper.join(rel);
    if upper.exists() {
        return Resolved::Upper(upper);
    }
    match in_any_lower(rel) {
        Some(p) => Resolved::Lower(p),
        None => Resolved::Missing,
    }
}

fn upper_for_write(rel: &str) -> PathBuf {
    let p = cfg().upper.join(rel);
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    p
}

/// A heap-held OBJECT_ATTRIBUTES naming `path`, kept alive for the duration of the call.
struct Redirect {
    _wide: Vec<u16>,
    _name: Box<UnicodeString>,
    attrs: Box<ObjectAttributes>,
}

unsafe fn redirect(original: *const ObjectAttributes, path: &Path) -> Redirect {
    let mut wide: Vec<u16> = "\\??\\".encode_utf16().chain(path.as_os_str().encode_wide()).collect();
    let len = (wide.len() * 2) as u16;
    let mut name = Box::new(UnicodeString { length: len, maximum_length: len, buffer: wide.as_mut_ptr() });
    let attrs = Box::new(ObjectAttributes {
        length: std::mem::size_of::<ObjectAttributes>() as u32,
        root_directory: std::ptr::null_mut(),
        object_name: &mut *name,
        attributes: (*original).attributes,
        security_descriptor: (*original).security_descriptor,
        security_quality_of_service: (*original).security_quality_of_service,
    });
    Redirect { _wide: wide, _name: name, attrs }
}

// ---------------------------------------------------------------- the open decision

enum Plan {
    Passthrough,
    Fail(NTSTATUS),
    Open { path: PathBuf, access: u32, disposition: u32, options: u32, track: Tracked },
}

fn plan_open(rel: String, access: u32, disposition: u32, options: u32) -> Plan {
    let wants_write = access & WRITE_RIGHTS != 0;
    let truncates = matches!(disposition, FILE_SUPERSEDE | FILE_OVERWRITE | FILE_OVERWRITE_IF);
    let creates = matches!(disposition, FILE_CREATE | FILE_OPEN_IF | FILE_OVERWRITE_IF | FILE_SUPERSEDE);
    let delete_on_close = options & FILE_DELETE_ON_CLOSE != 0;
    let upper_track = |rel: &str, dir: bool| Tracked { rel: rel.to_string(), lower: false, delete_on_close: false, dir };
    match resolve(&rel) {
        Resolved::Missing if rel.is_empty() => {
            // The mount folder itself: open the real one, list it merged.
            let path = PathBuf::from(&cfg().mount_original);
            Plan::Open { path, access, disposition, options, track: upper_track(&rel, true) }
        }
        Resolved::Upper(path) => {
            let dir = path.is_dir();
            Plan::Open { path, access, disposition, options, track: upper_track(&rel, dir) }
        }
        Resolved::Lower(lower) => {
            if lower.is_dir() {
                if disposition == FILE_CREATE {
                    return Plan::Fail(STATUS_OBJECT_NAME_COLLISION);
                }
                // Directories are only ever read from a lower layer.
                let access = (access & !(WRITE_RIGHTS | DELETE_ACCESS | MAXIMUM_ALLOWED)) | GENERIC_READ | SYNCHRONIZE;
                let track = Tracked { rel, lower: true, delete_on_close, dir: true };
                return Plan::Open { path: lower, access, disposition: FILE_OPEN, options: options & !FILE_DELETE_ON_CLOSE, track };
            }
            if disposition == FILE_CREATE {
                return Plan::Fail(STATUS_OBJECT_NAME_COLLISION);
            }
            if wants_write || truncates {
                // Copy up. A whole-file rewrite needs no copy of the old bytes.
                let upper = upper_for_write(&rel);
                if !truncates {
                    if let Err(e) = std::fs::copy(&lower, &upper) {
                        log(format!("copy-up {rel} failed: {e}"));
                        return Plan::Fail(STATUS_ACCESS_DENIED);
                    }
                    log(format!("copy-up {rel} ({} bytes)", std::fs::metadata(&upper).map(|m| m.len()).unwrap_or(0)));
                } else {
                    log(format!("redirect rewrite {rel}"));
                }
                let disposition = if disposition == FILE_OVERWRITE { FILE_OVERWRITE_IF } else { disposition };
                let track = Tracked { rel, lower: false, delete_on_close, dir: false };
                return Plan::Open { path: upper, access, disposition, options, track };
            }
            // Read-only use of a lower file: never with delete rights, never deleted on close.
            let mut access = access & !DELETE_ACCESS;
            if access & MAXIMUM_ALLOWED != 0 {
                access = (access & !MAXIMUM_ALLOWED) | GENERIC_READ | SYNCHRONIZE;
            }
            let track = Tracked { rel, lower: true, delete_on_close, dir: false };
            Plan::Open { path: lower, access, disposition: FILE_OPEN, options: options & !FILE_DELETE_ON_CLOSE, track }
        }
        Resolved::WhitedOut | Resolved::Missing => {
            if !creates {
                return Plan::Fail(STATUS_OBJECT_NAME_NOT_FOUND);
            }
            clear_whiteout(&rel);
            let path = upper_for_write(&rel);
            Plan::Open { path, access, disposition, options, track: upper_track(&rel, options & FILE_DIRECTORY_FILE != 0) }
        }
    }
}

static OPENED: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();

/// With `verbose=1`, log each file the game opens through the mount, once.
fn note_open(t: &Tracked, path: &Path) {
    if !cfg().verbose || t.dir {
        return;
    }
    let first = OPENED
        .get_or_init(|| Mutex::new(std::collections::HashSet::new()))
        .lock()
        .unwrap()
        .insert(t.rel.to_lowercase());
    if first {
        guarded(|| log(format!("opened {} -> {}", t.rel, path.display())));
    }
}

fn track(handle: HANDLE, t: Tracked) {
    handles().lock().unwrap().insert(handle as usize, t);
}

// ---------------------------------------------------------------- hooks

unsafe extern "system" fn nt_create_file(
    handle: *mut HANDLE,
    access: u32,
    oa: *mut ObjectAttributes,
    iosb: *mut c_void,
    alloc: *mut i64,
    attrs: u32,
    share: u32,
    disposition: u32,
    options: u32,
    ea: *mut c_void,
    ea_len: u32,
) -> NTSTATUS {
    let original = |h, a, o, d, op| hooks().create.call(h, a, o, iosb, alloc, attrs, share, d, op, ea, ea_len);
    let plan = guarded(|| match rel_of_attrs(oa) {
        Some(rel) => plan_open(rel, access, disposition, options),
        None => Plan::Passthrough,
    });
    if let Some(Plan::Fail(status)) = &plan {
        let name = guarded(|| unicode((*oa).object_name)).flatten();
        guarded(|| log(format!("NtCreateFile {name:?} refused {status:#x}")));
    }
    match plan {
        None | Some(Plan::Passthrough) => original(handle, access, oa, disposition, options),
        Some(Plan::Fail(status)) => status,
        Some(Plan::Open { path, access, disposition, options, track: t }) => {
            let r = redirect(oa, &path);
            let status = original(handle, access, &*r.attrs as *const _ as *mut _, disposition, options);
            if status < 0 {
                guarded(|| log(format!("NtCreateFile {} -> {} failed {status:#x}", t.rel, path.display())));
            }
            if status >= 0 {
                note_open(&t, &path);
                track(*handle, t);
            }
            status
        }
    }
}

unsafe extern "system" fn nt_open_file(
    handle: *mut HANDLE,
    access: u32,
    oa: *mut ObjectAttributes,
    iosb: *mut c_void,
    share: u32,
    options: u32,
) -> NTSTATUS {
    let plan = guarded(|| match rel_of_attrs(oa) {
        Some(rel) => plan_open(rel, access, FILE_OPEN, options),
        None => Plan::Passthrough,
    });
    if let Some(Plan::Fail(status)) = &plan {
        let name = guarded(|| unicode((*oa).object_name)).flatten();
        guarded(|| log(format!("NtOpenFile {name:?} refused {status:#x}")));
    }
    match plan {
        None | Some(Plan::Passthrough) => hooks().open.call(handle, access, oa, iosb, share, options),
        Some(Plan::Fail(status)) => status,
        Some(Plan::Open { path, access, options, track: t, .. }) => {
            let r = redirect(oa, &path);
            let status = hooks().open.call(handle, access, &*r.attrs as *const _ as *mut _, iosb, share, options);
            if status < 0 {
                guarded(|| log(format!("NtOpenFile {} -> {} failed {status:#x}", t.rel, path.display())));
            }
            if status >= 0 {
                note_open(&t, &path);
                track(*handle, t);
            }
            status
        }
    }
}

unsafe extern "system" fn nt_set_information_file(
    handle: HANDLE,
    iosb: *mut c_void,
    info: *mut c_void,
    len: u32,
    class: u32,
) -> NTSTATUS {
    let tracked = handles().lock().unwrap().get(&(handle as usize)).cloned();
    let (Some(t), false) = (tracked, INSIDE.with(|i| i.get())) else {
        return hooks().set_info.call(handle, iosb, info, len, class);
    };
    match class {
        FILE_DISPOSITION_INFORMATION | FILE_DISPOSITION_INFORMATION_EX => {
            let delete = if class == FILE_DISPOSITION_INFORMATION {
                *(info as *const u8) != 0
            } else {
                *(info as *const u32) & FILE_DISPOSITION_DELETE != 0
            };
            if !delete {
                return hooks().set_info.call(handle, iosb, info, len, class);
            }
            if t.lower {
                guarded(|| {
                    add_whiteout(&t.rel);
                    log(format!("whiteout {} (delete of a lower file)", t.rel));
                });
                return STATUS_SUCCESS;
            }
            let status = hooks().set_info.call(handle, iosb, info, len, class);
            if status >= 0 {
                guarded(|| {
                    if in_any_lower(&t.rel).is_some() {
                        add_whiteout(&t.rel);
                    }
                });
            }
            status
        }
        FILE_RENAME_INFORMATION | FILE_RENAME_INFORMATION_EX => {
            // x64 layout: flags/ReplaceIfExists @0, RootDirectory @8, FileNameLength @16, FileName @20
            let base = info as *const u8;
            let replace = if class == FILE_RENAME_INFORMATION {
                *base != 0
            } else {
                *(base as *const u32) & FILE_RENAME_REPLACE_IF_EXISTS != 0
            };
            let root = *(base.add(8) as *const HANDLE);
            let name_len = *(base.add(16) as *const u32) as usize / 2;
            let name = String::from_utf16_lossy(std::slice::from_raw_parts(base.add(20) as *const u16, name_len));
            let target_rel = if root.is_null() { guarded(|| rel_of(&name)).flatten() } else { None };
            let Some(target_rel) = target_rel else {
                if t.lower {
                    // Moving a lower file out of the mount would take it from its layer.
                    return STATUS_ACCESS_DENIED;
                }
                return hooks().set_info.call(handle, iosb, info, len, class);
            };
            let outcome = guarded(|| -> Result<Option<PathBuf>, NTSTATUS> {
                let exists = !matches!(resolve(&target_rel), Resolved::Missing | Resolved::WhitedOut);
                if exists && !replace {
                    return Err(STATUS_OBJECT_NAME_COLLISION);
                }
                let target = upper_for_write(&target_rel);
                if t.lower {
                    let source = in_any_lower(&t.rel).ok_or(STATUS_OBJECT_NAME_NOT_FOUND)?;
                    std::fs::copy(&source, &target).map_err(|_| STATUS_ACCESS_DENIED)?;
                    add_whiteout(&t.rel);
                    clear_whiteout(&target_rel);
                    log(format!("rename {} -> {} (copied from a lower layer)", t.rel, target_rel));
                    Ok(None)
                } else {
                    Ok(Some(target))
                }
            });
            match outcome {
                None => hooks().set_info.call(handle, iosb, info, len, class),
                Some(Err(status)) => status,
                Some(Ok(None)) => STATUS_SUCCESS,
                Some(Ok(Some(target))) => {
                    // Upper-to-upper: the real rename, pointed at the upper path.
                    let wide: Vec<u16> = "\\??\\".encode_utf16().chain(target.as_os_str().encode_wide()).collect();
                    let new_len = 20 + wide.len() * 2;
                    let mut buf = vec![0u64; new_len.div_ceil(8)];
                    let p = buf.as_mut_ptr() as *mut u8;
                    std::ptr::copy_nonoverlapping(base, p, 8);
                    *(p.add(8) as *mut HANDLE) = std::ptr::null_mut();
                    *(p.add(16) as *mut u32) = (wide.len() * 2) as u32;
                    std::ptr::copy_nonoverlapping(wide.as_ptr(), p.add(20) as *mut u16, wide.len());
                    let status = hooks().set_info.call(handle, iosb, p as *mut c_void, new_len as u32, class);
                    if status >= 0 {
                        guarded(|| {
                            if in_any_lower(&t.rel).is_some() {
                                add_whiteout(&t.rel);
                            }
                            clear_whiteout(&target_rel);
                        });
                        if let Some(entry) = handles().lock().unwrap().get_mut(&(handle as usize)) {
                            entry.rel = target_rel;
                        }
                    }
                    status
                }
            }
        }
        _ => hooks().set_info.call(handle, iosb, info, len, class),
    }
}

unsafe fn query_attributes(oa: *mut ObjectAttributes, out: *mut c_void, full: bool) -> NTSTATUS {
    let call = |o| if full { hooks().query_full_attr.call(o, out) } else { hooks().query_attr.call(o, out) };
    let resolved = guarded(|| rel_of_attrs(oa).map(|rel| resolve(&rel)));
    match resolved {
        Some(Some(Resolved::Upper(p))) | Some(Some(Resolved::Lower(p))) => {
            let r = redirect(oa, &p);
            call(&*r.attrs as *const _ as *mut _)
        }
        Some(Some(Resolved::WhitedOut)) => STATUS_OBJECT_NAME_NOT_FOUND,
        Some(Some(Resolved::Missing)) => {
            let rel = guarded(|| rel_of_attrs(oa)).flatten().unwrap_or_default();
            if rel.is_empty() {
                call(oa)
            } else {
                STATUS_OBJECT_NAME_NOT_FOUND
            }
        }
        _ => call(oa),
    }
}

unsafe extern "system" fn nt_query_attributes_file(oa: *mut ObjectAttributes, out: *mut c_void) -> NTSTATUS {
    query_attributes(oa, out, false)
}

unsafe extern "system" fn nt_query_full_attributes_file(oa: *mut ObjectAttributes, out: *mut c_void) -> NTSTATUS {
    query_attributes(oa, out, true)
}

unsafe extern "system" fn nt_query_information_by_name(
    oa: *mut ObjectAttributes,
    iosb: *mut c_void,
    out: *mut c_void,
    len: u32,
    class: u32,
) -> NTSTATUS {
    let call = |o| hooks().query_by_name.call(o, iosb, out, len, class);
    let resolved = guarded(|| rel_of_attrs(oa).map(|rel| (rel.is_empty(), resolve(&rel))));
    match resolved {
        Some(Some((_, Resolved::Upper(p)))) | Some(Some((_, Resolved::Lower(p)))) => {
            let r = redirect(oa, &p);
            call(&*r.attrs as *const _ as *mut _)
        }
        Some(Some((_, Resolved::WhitedOut))) | Some(Some((false, Resolved::Missing))) => STATUS_OBJECT_NAME_NOT_FOUND,
        _ => call(oa),
    }
}

unsafe extern "system" fn nt_close(handle: HANDLE) -> NTSTATUS {
    // Our own file work closes handles while holding the listing lock; those handles were never
    // tracked, and taking the lock again on this thread would deadlock.
    if INSIDE.with(|i| i.get()) {
        return hooks().close.call(handle);
    }
    if let Ok(mut m) = listings().lock() {
        m.remove(&(handle as usize));
    }
    if let Ok(mut m) = roots().lock() {
        m.remove(&(handle as usize));
    }
    let removed = handles().lock().ok().and_then(|mut m| m.remove(&(handle as usize)));
    if let Some(t) = removed {
        if t.lower && t.delete_on_close {
            guarded(|| add_whiteout(&t.rel));
        }
    }
    hooks().close.call(handle)
}

unsafe extern "system" fn create_process_internal_w(
    token: HANDLE,
    app: *const u16,
    cmd: *mut u16,
    pa: *const c_void,
    ta: *const c_void,
    inherit: BOOL,
    flags: u32,
    env: *const c_void,
    cwd: *const u16,
    si: *const c_void,
    pi: *mut PROCESS_INFORMATION,
    new_token: *mut HANDLE,
) -> BOOL {
    let ok = hooks().create_process.call(token, app, cmd, pa, ta, inherit, flags | CREATE_SUSPENDED, env, cwd, si, pi, new_token);
    if ok != 0 {
        if let Err(e) = inject((*pi).hProcess, &cfg().dll) {
            guarded(|| log(format!("inject into child {} failed: {e}", (*pi).dwProcessId)));
        }
        if flags & CREATE_SUSPENDED == 0 {
            ResumeThread((*pi).hThread);
        }
    }
    ok
}

// ---------------------------------------------------------------- merged directory listings

type NtQueryDirectoryFileFn = unsafe extern "system" fn(
    HANDLE,
    HANDLE,
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut c_void,
    u32,
    u32,
    u8,
    *mut UnicodeString,
    u8,
) -> NTSTATUS;
type NtQueryDirectoryFileExFn = unsafe extern "system" fn(
    HANDLE,
    HANDLE,
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut c_void,
    u32,
    u32,
    u32,
    *mut UnicodeString,
) -> NTSTATUS;

const STATUS_NO_MORE_FILES: NTSTATUS = 0x8000_0006_u32 as i32;
const STATUS_NO_SUCH_FILE: NTSTATUS = 0xC000_000F_u32 as i32;
const STATUS_BUFFER_OVERFLOW: NTSTATUS = 0x8000_0005_u32 as i32;
const SL_RESTART_SCAN: u32 = 0x1;
const SL_RETURN_SINGLE_ENTRY: u32 = 0x2;
const FILE_LIST_DIRECTORY: u32 = 0x1;
const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x20;
const FILE_SHARE_ALL: u32 = 0x7;

/// (offset of FileNameLength, offset of FileName) for the listing formats we merge.
fn name_offsets(class: u32) -> Option<(usize, usize)> {
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

struct Listing {
    class: u32,
    entries: Vec<Vec<u8>>, // each entry's bytes, NextEntryOffset zeroed
    cursor: usize,
    returned_any: bool,
}

static LISTINGS: OnceLock<Mutex<HashMap<usize, Listing>>> = OnceLock::new();
fn listings() -> &'static Mutex<HashMap<usize, Listing>> {
    LISTINGS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn entry_name(entry: &[u8], class: u32) -> String {
    let (len_at, name_at) = name_offsets(class).unwrap();
    let len = u32::from_le_bytes(entry[len_at..len_at + 4].try_into().unwrap()) as usize;
    let bytes = &entry[name_at..(name_at + len).min(entry.len())];
    let wide: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    String::from_utf16_lossy(&wide)
}

/// Raw entries of one real directory, in `class` format.
unsafe fn read_real_dir(dir: &Path, class: u32) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut wide: Vec<u16> = "\\??\\".encode_utf16().chain(dir.as_os_str().encode_wide()).collect();
    let len = (wide.len() * 2) as u16;
    let mut name = UnicodeString { length: len, maximum_length: len, buffer: wide.as_mut_ptr() };
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
            let name_len = u32::from_le_bytes(bytes[offset + len_at..offset + len_at + 4].try_into().unwrap()) as usize;
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
fn wildcard(pattern: &str, name: &str) -> bool {
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
unsafe fn build_listing(rel: &str, class: u32, pattern: Option<String>) -> Vec<Vec<u8>> {
    let c = cfg();
    let mut sources = vec![c.upper.join(rel)];
    sources.extend(c.lowers.iter().map(|l| l.join(rel)));
    let mut seen = std::collections::HashSet::new();
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
                let child = if rel.is_empty() { name.clone() } else { format!("{rel}\\{name}") };
                if whited_out(&child) {
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
unsafe fn query_directory(
    handle: HANDLE,
    iosb: *mut c_void,
    out: *mut c_void,
    length: u32,
    class: u32,
    single: bool,
    pattern: *mut UnicodeString,
    restart: bool,
) -> Option<NTSTATUS> {
    if INSIDE.with(|i| i.get()) {
        return None;
    }
    let tracked = handles().lock().unwrap().get(&(handle as usize)).cloned()?;
    if !tracked.dir {
        return None;
    }
    if name_offsets(class).is_none() {
        guarded(|| log(format!("listing class {class} not merged for {}", tracked.rel)));
        return None;
    }
    guarded(|| log(format!("list {:?} class {class} single {single} restart {restart} pattern {:?}", tracked.rel, unicode(pattern))));
    let mut map = listings().lock().unwrap();
    let needs_build = restart || map.get(&(handle as usize)).map(|l| l.class != class).unwrap_or(true);
    if needs_build {
        let pattern = unicode(pattern).filter(|p| !p.is_empty() && p != "*");
        let entries = guarded(|| build_listing(&tracked.rel, class, pattern))?;
        map.insert(handle as usize, Listing { class, entries, cursor: 0, returned_any: false });
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

unsafe extern "system" fn nt_query_directory_file(
    handle: HANDLE,
    event: HANDLE,
    apc: *mut c_void,
    apc_ctx: *mut c_void,
    iosb: *mut c_void,
    out: *mut c_void,
    length: u32,
    class: u32,
    single: u8,
    pattern: *mut UnicodeString,
    restart: u8,
) -> NTSTATUS {
    match query_directory(handle, iosb, out, length, class, single != 0, pattern, restart != 0) {
        Some(status) => status,
        None => hooks().query_dir.call(handle, event, apc, apc_ctx, iosb, out, length, class, single, pattern, restart),
    }
}

unsafe extern "system" fn nt_query_directory_file_ex(
    handle: HANDLE,
    event: HANDLE,
    apc: *mut c_void,
    apc_ctx: *mut c_void,
    iosb: *mut c_void,
    out: *mut c_void,
    length: u32,
    class: u32,
    flags: u32,
    pattern: *mut UnicodeString,
) -> NTSTATUS {
    let single = flags & SL_RETURN_SINGLE_ENTRY != 0;
    let restart = flags & SL_RESTART_SCAN != 0;
    match query_directory(handle, iosb, out, length, class, single, pattern, restart) {
        Some(status) => status,
        None => hooks().query_dir_ex.call(handle, event, apc, apc_ctx, iosb, out, length, class, flags, pattern),
    }
}

// ---------------------------------------------------------------- injection (also used by the launcher)

/// Load `dll` into `process` (created suspended) with a remote `LoadLibraryW`.
pub unsafe fn inject(process: HANDLE, dll: &Path) -> Result<(), String> {
    let wide: Vec<u16> = dll.as_os_str().encode_wide().chain(Some(0)).collect();
    let bytes = wide.len() * 2;
    let remote = VirtualAllocEx(process, std::ptr::null(), bytes, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
    if remote.is_null() {
        return Err(format!("VirtualAllocEx {}", GetLastError()));
    }
    if WriteProcessMemory(process, remote, wide.as_ptr() as *const c_void, bytes, std::ptr::null_mut()) == 0 {
        return Err(format!("WriteProcessMemory {}", GetLastError()));
    }
    let k32: Vec<u16> = "kernel32.dll\0".encode_utf16().collect();
    let load = GetProcAddress(GetModuleHandleW(k32.as_ptr()), c"LoadLibraryW".as_ptr() as *const u8)
        .ok_or("no LoadLibraryW")?;
    let thread = CreateRemoteThread(
        process,
        std::ptr::null(),
        0,
        Some(std::mem::transmute::<_, unsafe extern "system" fn(*mut c_void) -> u32>(load)),
        remote,
        0,
        std::ptr::null_mut(),
    );
    if thread.is_null() {
        return Err(format!("CreateRemoteThread {}", GetLastError()));
    }
    WaitForSingleObject(thread, 30_000);
    let mut code = 0u32;
    GetExitCodeThread(thread, &mut code);
    CloseHandle(thread);
    if code == 0 {
        return Err("LoadLibraryW returned NULL in the target".into());
    }
    Ok(())
}

/// Exported for the launcher: create `cmd` suspended, inject this DLL, resume. Returns the PID.
#[no_mangle]
pub unsafe extern "system" fn agvfs_launch(cmd: *mut u16, cwd: *const u16, dll: *const u16, pi_out: *mut PROCESS_INFORMATION) -> BOOL {
    let mut si: STARTUPINFOW = std::mem::zeroed();
    si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    if CreateProcessW(std::ptr::null(), cmd, std::ptr::null(), std::ptr::null(), 0, CREATE_SUSPENDED, std::ptr::null(), cwd, &si, pi_out) == 0 {
        return 0;
    }
    let len = (0..).take_while(|&i| *dll.add(i) != 0).count();
    let dll = PathBuf::from(String::from_utf16_lossy(std::slice::from_raw_parts(dll, len)));
    if inject((*pi_out).hProcess, &dll).is_err() {
        TerminateProcess((*pi_out).hProcess, 1);
        return 0;
    }
    ResumeThread((*pi_out).hThread);
    1
}

// ---------------------------------------------------------------- setup

fn load_config() -> Option<Config> {
    let path = std::env::var_os("AGVFS_CONFIG")?;
    let text = std::fs::read_to_string(path).ok()?;
    let (mut mount, mut upper, mut lowers, mut dll, mut log, mut verbose) = (None, None, Vec::new(), None, None, false);
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        let v = v.trim().trim_end_matches('\\');
        match k.trim() {
            "mount" => mount = Some(v.to_string()),
            "upper" => upper = Some(PathBuf::from(v)),
            "lower" => lowers.push(PathBuf::from(v)),
            "dll" => dll = Some(PathBuf::from(v)),
            "log" => log = Some(PathBuf::from(v)),
            "verbose" => verbose = v == "1",
            _ => {}
        }
    }
    let mount = mount?;
    Some(Config { mount_len: mount.len(), mount_original: mount.clone(), mount, upper: upper?, lowers, dll: dll?, log, verbose })
}

unsafe fn proc_addr<T>(module: &str, name: &str) -> T {
    let m: Vec<u16> = module.encode_utf16().chain(Some(0)).collect();
    let n = std::ffi::CString::new(name).unwrap();
    let f = GetProcAddress(GetModuleHandleW(m.as_ptr()), n.as_ptr() as *const u8).expect(name);
    std::mem::transmute_copy(&f)
}

unsafe fn install() -> Result<(), retour::Error> {
    let h = Hooks {
        create: GenericDetour::new(proc_addr::<NtCreateFileFn>("ntdll.dll", "NtCreateFile"), nt_create_file)?,
        open: GenericDetour::new(proc_addr::<NtOpenFileFn>("ntdll.dll", "NtOpenFile"), nt_open_file)?,
        set_info: GenericDetour::new(
            proc_addr::<NtSetInformationFileFn>("ntdll.dll", "NtSetInformationFile"),
            nt_set_information_file,
        )?,
        query_attr: GenericDetour::new(
            proc_addr::<NtQueryAttributesFileFn>("ntdll.dll", "NtQueryAttributesFile"),
            nt_query_attributes_file,
        )?,
        query_full_attr: GenericDetour::new(
            proc_addr::<NtQueryAttributesFileFn>("ntdll.dll", "NtQueryFullAttributesFile"),
            nt_query_full_attributes_file,
        )?,
        close: GenericDetour::new(proc_addr::<NtCloseFn>("ntdll.dll", "NtClose"), nt_close)?,
        create_process: GenericDetour::new(
            proc_addr::<CreateProcessInternalWFn>("kernelbase.dll", "CreateProcessInternalW"),
            create_process_internal_w,
        )?,
        query_dir: GenericDetour::new(
            proc_addr::<NtQueryDirectoryFileFn>("ntdll.dll", "NtQueryDirectoryFile"),
            nt_query_directory_file,
        )?,
        query_dir_ex: GenericDetour::new(
            proc_addr::<NtQueryDirectoryFileExFn>("ntdll.dll", "NtQueryDirectoryFileEx"),
            nt_query_directory_file_ex,
        )?,
        // Windows 11 24H2's GetFileAttributes(Ex)W asks by name, without opening the file.
        query_by_name: GenericDetour::new(
            proc_addr::<NtQueryInformationByNameFn>("ntdll.dll", "NtQueryInformationByName"),
            nt_query_information_by_name,
        )?,
    };
    let h = HOOKS.get_or_init(|| h);
    h.create.enable()?;
    h.open.enable()?;
    h.set_info.enable()?;
    h.query_attr.enable()?;
    h.query_full_attr.enable()?;
    h.close.enable()?;
    h.create_process.enable()?;
    h.query_dir.enable()?;
    h.query_dir_ex.enable()?;
    h.query_by_name.enable()?;
    Ok(())
}

#[no_mangle]
pub unsafe extern "system" fn DllMain(_module: HANDLE, reason: u32, _reserved: *mut c_void) -> BOOL {
    const DLL_PROCESS_ATTACH: u32 = 1;
    if reason == DLL_PROCESS_ATTACH {
        let Some(config) = load_config() else { return 1 };
        let _ = CONFIG.set(config);
        INSIDE.with(|i| i.set(true));
        let result = install();
        INSIDE.with(|i| i.set(false));
        guarded(|| match result {
            Ok(()) => log(format!("hooks installed in {}", std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default())),
            Err(e) => log(format!("hook install failed: {e}")),
        });
    }
    1
}
