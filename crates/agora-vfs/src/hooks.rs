use std::collections::HashSet;
use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use retour::GenericDetour;
use windows_sys::Win32::Foundation::{BOOL, HANDLE};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::Threading::*;

use crate::cfg;
use crate::config::RuntimeConfig;
use crate::handles;
use crate::listing::*;
use crate::nt::*;
use crate::paths::*;
use crate::util::{catch_hook_panic, guarded, is_inside, log};
use crate::Tracked;

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
type NtOpenFileFn = unsafe extern "system" fn(
    *mut HANDLE,
    u32,
    *mut ObjectAttributes,
    *mut c_void,
    u32,
    u32,
) -> NTSTATUS;
type NtSetInformationFileFn =
    unsafe extern "system" fn(HANDLE, *mut c_void, *mut c_void, u32, u32) -> NTSTATUS;
type NtQueryAttributesFileFn =
    unsafe extern "system" fn(*mut ObjectAttributes, *mut c_void) -> NTSTATUS;
type NtCloseFn = unsafe extern "system" fn(HANDLE) -> NTSTATUS;
type NtQueryInformationByNameFn = unsafe extern "system" fn(
    *mut ObjectAttributes,
    *mut c_void,
    *mut c_void,
    u32,
    u32,
) -> NTSTATUS;
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
pub type NtQueryDirectoryFileFn = unsafe extern "system" fn(
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
pub type NtQueryDirectoryFileExFn = unsafe extern "system" fn(
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

pub struct Hooks {
    pub create: GenericDetour<NtCreateFileFn>,
    pub open: GenericDetour<NtOpenFileFn>,
    pub set_info: GenericDetour<NtSetInformationFileFn>,
    pub query_attr: GenericDetour<NtQueryAttributesFileFn>,
    pub query_full_attr: GenericDetour<NtQueryAttributesFileFn>,
    pub close: GenericDetour<NtCloseFn>,
    pub create_process: GenericDetour<CreateProcessInternalWFn>,
    pub query_dir: GenericDetour<NtQueryDirectoryFileFn>,
    pub query_dir_ex: GenericDetour<NtQueryDirectoryFileExFn>,
    pub query_by_name: GenericDetour<NtQueryInformationByNameFn>,
}
unsafe impl Sync for Hooks {}
unsafe impl Send for Hooks {}

static HOOKS: OnceLock<Hooks> = OnceLock::new();
pub fn hooks() -> &'static Hooks {
    HOOKS.get().expect("agora-vfs hooks")
}

/// A heap-held OBJECT_ATTRIBUTES naming `path`, kept alive for the duration of the call.
pub struct Redirect {
    pub _wide: Vec<u16>,
    pub _name: Box<UnicodeString>,
    pub attrs: Box<ObjectAttributes>,
}

pub unsafe fn redirect(original: *const ObjectAttributes, path: &Path) -> Redirect {
    let mut wide: Vec<u16> = "\\??\\"
        .encode_utf16()
        .chain(path.as_os_str().encode_wide())
        .collect();
    let len = (wide.len() * 2) as u16;
    let mut name = Box::new(UnicodeString {
        length: len,
        maximum_length: len,
        buffer: wide.as_mut_ptr(),
    });
    let attrs = Box::new(ObjectAttributes {
        length: std::mem::size_of::<ObjectAttributes>() as u32,
        root_directory: std::ptr::null_mut(),
        object_name: &mut *name,
        attributes: (*original).attributes,
        security_descriptor: (*original).security_descriptor,
        security_quality_of_service: (*original).security_quality_of_service,
    });
    Redirect {
        _wide: wide,
        _name: name,
        attrs,
    }
}

pub enum Plan {
    Passthrough,
    Fail(NTSTATUS),
    Open {
        path: PathBuf,
        access: u32,
        disposition: u32,
        options: u32,
        track: Tracked,
    },
}

pub fn plan_open(
    rel: String,
    access: u32,
    disposition: u32,
    options: u32,
    cfg: &RuntimeConfig,
) -> Plan {
    let wants_write = access & WRITE_RIGHTS != 0;
    let truncates = matches!(
        disposition,
        FILE_SUPERSEDE | FILE_OVERWRITE | FILE_OVERWRITE_IF
    );
    let creates = matches!(
        disposition,
        FILE_CREATE | FILE_OPEN_IF | FILE_OVERWRITE_IF | FILE_SUPERSEDE
    );
    let delete_on_close = options & FILE_DELETE_ON_CLOSE != 0;
    let upper_track = |rel: &str, dir: bool| Tracked {
        rel: rel.to_string(),
        lower: false,
        delete_on_close: false,
        dir,
    };
    match resolve(&rel, cfg) {
        Resolved::Missing if rel.is_empty() => {
            // The mount folder itself: open the real one, list it merged.
            let path = PathBuf::from(&cfg.mount_original);
            Plan::Open {
                path,
                access,
                disposition,
                options,
                track: upper_track(&rel, true),
            }
        }
        Resolved::Upper(path) => {
            let dir = path.is_dir();
            Plan::Open {
                path,
                access,
                disposition,
                options,
                track: upper_track(&rel, dir),
            }
        }
        Resolved::Lower(lower) => {
            if lower.is_dir() {
                if disposition == FILE_CREATE {
                    return Plan::Fail(STATUS_OBJECT_NAME_COLLISION);
                }
                // Directories are only ever read from a lower layer.
                let access = (access & !(WRITE_RIGHTS | DELETE_ACCESS | MAXIMUM_ALLOWED))
                    | GENERIC_READ
                    | SYNCHRONIZE;
                let track = Tracked {
                    rel,
                    lower: true,
                    delete_on_close,
                    dir: true,
                };
                return Plan::Open {
                    path: lower,
                    access,
                    disposition: FILE_OPEN,
                    options: options & !FILE_DELETE_ON_CLOSE,
                    track,
                };
            }
            if disposition == FILE_CREATE {
                return Plan::Fail(STATUS_OBJECT_NAME_COLLISION);
            }
            if wants_write || truncates {
                // Copy up. A whole-file rewrite needs no copy of the old bytes.
                let upper = upper_for_write(&rel, &cfg.upper);
                if !truncates {
                    if let Err(e) = std::fs::copy(&lower, &upper) {
                        log(format!("copy-up {rel} failed: {e}"));
                        return Plan::Fail(STATUS_ACCESS_DENIED);
                    }
                    log(format!(
                        "copy-up {rel} ({} bytes)",
                        std::fs::metadata(&upper).map(|m| m.len()).unwrap_or(0)
                    ));
                } else {
                    log(format!("redirect rewrite {rel}"));
                }
                let disposition = if disposition == FILE_OVERWRITE {
                    FILE_OVERWRITE_IF
                } else {
                    disposition
                };
                let track = Tracked {
                    rel,
                    lower: false,
                    delete_on_close,
                    dir: false,
                };
                return Plan::Open {
                    path: upper,
                    access,
                    disposition,
                    options,
                    track,
                };
            }
            // Read-only use of a lower file: never with delete rights, never deleted on close.
            let mut access = access & !DELETE_ACCESS;
            if access & MAXIMUM_ALLOWED != 0 {
                access = (access & !MAXIMUM_ALLOWED) | GENERIC_READ | SYNCHRONIZE;
            }
            let track = Tracked {
                rel,
                lower: true,
                delete_on_close,
                dir: false,
            };
            Plan::Open {
                path: lower,
                access,
                disposition: FILE_OPEN,
                options: options & !FILE_DELETE_ON_CLOSE,
                track,
            }
        }
        Resolved::WhitedOut | Resolved::Missing => {
            if !creates {
                return Plan::Fail(STATUS_OBJECT_NAME_NOT_FOUND);
            }
            clear_whiteout(&rel, &cfg.upper);
            let path = upper_for_write(&rel, &cfg.upper);
            Plan::Open {
                path,
                access,
                disposition,
                options,
                track: upper_track(&rel, options & FILE_DIRECTORY_FILE != 0),
            }
        }
    }
}

static OPENED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

/// With `verbose=true`, log each file the game opens through the mount, once.
fn note_open(t: &Tracked, path: &Path, cfg: &RuntimeConfig) {
    if !cfg.verbose || t.dir {
        return;
    }
    let first = OPENED
        .get_or_init(|| Mutex::new(HashSet::new()))
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

// ---------------------------------------------------------------- Hook Implementations

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
    catch_hook_panic("NtCreateFile", || {
        let original = |h, a, o, d, op| {
            hooks()
                .create
                .call(h, a, o, iosb, alloc, attrs, share, d, op, ea, ea_len)
        };
        let c = cfg();
        // FILE_OPEN_BY_FILE_ID: the "name" is a binary file id, usually relative to a handle on
        // the whole volume, so it cannot be resolved against the layers. A write or delete by id
        // could reach a lower file, so it is refused wherever it points (the §26.5 rule: degrade
        // by refusing, never by passing a write through); a read by id passes through unchanged.
        if options & FILE_OPEN_BY_FILE_ID != 0 {
            if (access & WRITE_RIGHTS != 0) || (access & DELETE_ACCESS != 0) {
                guarded(|| log("FILE_OPEN_BY_FILE_ID with write or delete rights refused"));
                return STATUS_ACCESS_DENIED;
            }
            return original(handle, access, oa, disposition, options);
        }

        let plan = guarded(|| match rel_of_attrs(oa, c) {
            Some(rel) => plan_open(rel, access, disposition, options, c),
            None => Plan::Passthrough,
        });
        if let Some(Plan::Fail(status)) = &plan {
            let name = guarded(|| unicode((*oa).object_name)).flatten();
            guarded(|| log(format!("NtCreateFile {name:?} refused {status:#x}")));
        }
        match plan {
            None | Some(Plan::Passthrough) => original(handle, access, oa, disposition, options),
            Some(Plan::Fail(status)) => status,
            Some(Plan::Open {
                path,
                access,
                disposition,
                options,
                track: t,
            }) => {
                let r = redirect(oa, &path);
                let status = original(
                    handle,
                    access,
                    &*r.attrs as *const _ as *mut _,
                    disposition,
                    options,
                );
                if status < 0 {
                    guarded(|| {
                        log(format!(
                            "NtCreateFile {} -> {} failed {status:#x}",
                            t.rel,
                            path.display()
                        ))
                    });
                }
                if status >= 0 {
                    note_open(&t, &path, c);
                    track(*handle, t);
                }
                status
            }
        }
    })
}

unsafe extern "system" fn nt_open_file(
    handle: *mut HANDLE,
    access: u32,
    oa: *mut ObjectAttributes,
    iosb: *mut c_void,
    share: u32,
    options: u32,
) -> NTSTATUS {
    catch_hook_panic("NtOpenFile", || {
        let c = cfg();
        // FILE_OPEN_BY_FILE_ID: see the NtCreateFile hook.
        if options & FILE_OPEN_BY_FILE_ID != 0 {
            if (access & WRITE_RIGHTS != 0) || (access & DELETE_ACCESS != 0) {
                guarded(|| log("FILE_OPEN_BY_FILE_ID with write or delete rights refused"));
                return STATUS_ACCESS_DENIED;
            }
            return hooks().open.call(handle, access, oa, iosb, share, options);
        }

        let plan = guarded(|| match rel_of_attrs(oa, c) {
            Some(rel) => plan_open(rel, access, FILE_OPEN, options, c),
            None => Plan::Passthrough,
        });
        if let Some(Plan::Fail(status)) = &plan {
            let name = guarded(|| unicode((*oa).object_name)).flatten();
            guarded(|| log(format!("NtOpenFile {name:?} refused {status:#x}")));
        }
        match plan {
            None | Some(Plan::Passthrough) => {
                hooks().open.call(handle, access, oa, iosb, share, options)
            }
            Some(Plan::Fail(status)) => status,
            Some(Plan::Open {
                path,
                access,
                options,
                track: t,
                ..
            }) => {
                let r = redirect(oa, &path);
                let status = hooks().open.call(
                    handle,
                    access,
                    &*r.attrs as *const _ as *mut _,
                    iosb,
                    share,
                    options,
                );
                if status < 0 {
                    guarded(|| {
                        log(format!(
                            "NtOpenFile {} -> {} failed {status:#x}",
                            t.rel,
                            path.display()
                        ))
                    });
                }
                if status >= 0 {
                    note_open(&t, &path, c);
                    track(*handle, t);
                }
                status
            }
        }
    })
}

unsafe extern "system" fn nt_set_information_file(
    handle: HANDLE,
    iosb: *mut c_void,
    info: *mut c_void,
    len: u32,
    class: u32,
) -> NTSTATUS {
    catch_hook_panic("NtSetInformationFile", || {
        let tracked = handles().lock().unwrap().get(&(handle as usize)).cloned();
        let (Some(t), false) = (tracked, is_inside()) else {
            return hooks().set_info.call(handle, iosb, info, len, class);
        };
        let c = cfg();
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
                        add_whiteout(&t.rel, &c.upper);
                        log(format!("whiteout {} (delete of a lower file)", t.rel));
                    });
                    return STATUS_SUCCESS;
                }
                let status = hooks().set_info.call(handle, iosb, info, len, class);
                if status >= 0 {
                    guarded(|| {
                        if in_any_lower(&t.rel, &c.lowers).is_some() {
                            add_whiteout(&t.rel, &c.upper);
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
                let name = String::from_utf16_lossy(std::slice::from_raw_parts(
                    base.add(20) as *const u16,
                    name_len,
                ));
                let target_rel = if root.is_null() {
                    guarded(|| rel_of(&name, c)).flatten()
                } else {
                    None
                };
                let Some(target_rel) = target_rel else {
                    if t.lower {
                        // Moving a lower file out of the mount would take it from its layer.
                        return STATUS_ACCESS_DENIED;
                    }
                    return hooks().set_info.call(handle, iosb, info, len, class);
                };
                let outcome = guarded(|| -> Result<Option<PathBuf>, NTSTATUS> {
                    let exists = !matches!(
                        resolve(&target_rel, c),
                        Resolved::Missing | Resolved::WhitedOut
                    );
                    if exists && !replace {
                        return Err(STATUS_OBJECT_NAME_COLLISION);
                    }
                    let target = upper_for_write(&target_rel, &c.upper);
                    if t.lower {
                        let source =
                            in_any_lower(&t.rel, &c.lowers).ok_or(STATUS_OBJECT_NAME_NOT_FOUND)?;
                        std::fs::copy(&source, &target).map_err(|_| STATUS_ACCESS_DENIED)?;
                        add_whiteout(&t.rel, &c.upper);
                        clear_whiteout(&target_rel, &c.upper);
                        log(format!(
                            "rename {} -> {} (copied from a lower layer)",
                            t.rel, target_rel
                        ));
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
                        let wide: Vec<u16> = "\\??\\"
                            .encode_utf16()
                            .chain(target.as_os_str().encode_wide())
                            .collect();
                        let new_len = 20 + wide.len() * 2;
                        let mut buf = vec![0u64; new_len.div_ceil(8)];
                        let p = buf.as_mut_ptr() as *mut u8;
                        std::ptr::copy_nonoverlapping(base, p, 8);
                        *(p.add(8) as *mut HANDLE) = std::ptr::null_mut();
                        *(p.add(16) as *mut u32) = (wide.len() * 2) as u32;
                        std::ptr::copy_nonoverlapping(
                            wide.as_ptr(),
                            p.add(20) as *mut u16,
                            wide.len(),
                        );
                        let status = hooks().set_info.call(
                            handle,
                            iosb,
                            p as *mut c_void,
                            new_len as u32,
                            class,
                        );
                        if status >= 0 {
                            guarded(|| {
                                if in_any_lower(&t.rel, &c.lowers).is_some() {
                                    add_whiteout(&t.rel, &c.upper);
                                }
                                clear_whiteout(&target_rel, &c.upper);
                            });
                            if let Some(entry) =
                                handles().lock().unwrap().get_mut(&(handle as usize))
                            {
                                entry.rel = target_rel;
                            }
                        }
                        status
                    }
                }
            }
            _ => hooks().set_info.call(handle, iosb, info, len, class),
        }
    })
}

unsafe fn query_attributes(oa: *mut ObjectAttributes, out: *mut c_void, full: bool) -> NTSTATUS {
    let call = |o| {
        if full {
            hooks().query_full_attr.call(o, out)
        } else {
            hooks().query_attr.call(o, out)
        }
    };
    let c = cfg();
    let resolved = guarded(|| rel_of_attrs(oa, c).map(|rel| resolve(&rel, c)));
    match resolved {
        Some(Some(Resolved::Upper(p))) | Some(Some(Resolved::Lower(p))) => {
            let r = redirect(oa, &p);
            call(&*r.attrs as *const _ as *mut _)
        }
        Some(Some(Resolved::WhitedOut)) => STATUS_OBJECT_NAME_NOT_FOUND,
        Some(Some(Resolved::Missing)) => {
            let rel = guarded(|| rel_of_attrs(oa, c))
                .flatten()
                .unwrap_or_default();
            if rel.is_empty() {
                call(oa)
            } else {
                STATUS_OBJECT_NAME_NOT_FOUND
            }
        }
        _ => call(oa),
    }
}

unsafe extern "system" fn nt_query_attributes_file(
    oa: *mut ObjectAttributes,
    out: *mut c_void,
) -> NTSTATUS {
    catch_hook_panic("NtQueryAttributesFile", || query_attributes(oa, out, false))
}

unsafe extern "system" fn nt_query_full_attributes_file(
    oa: *mut ObjectAttributes,
    out: *mut c_void,
) -> NTSTATUS {
    catch_hook_panic("NtQueryFullAttributesFile", || {
        query_attributes(oa, out, true)
    })
}

unsafe extern "system" fn nt_query_information_by_name(
    oa: *mut ObjectAttributes,
    iosb: *mut c_void,
    out: *mut c_void,
    len: u32,
    class: u32,
) -> NTSTATUS {
    catch_hook_panic("NtQueryInformationByName", || {
        let call = |o| hooks().query_by_name.call(o, iosb, out, len, class);
        let c = cfg();
        let resolved =
            guarded(|| rel_of_attrs(oa, c).map(|rel| (rel.is_empty(), resolve(&rel, c))));
        match resolved {
            Some(Some((_, Resolved::Upper(p)))) | Some(Some((_, Resolved::Lower(p)))) => {
                let r = redirect(oa, &p);
                call(&*r.attrs as *const _ as *mut _)
            }
            Some(Some((_, Resolved::WhitedOut))) | Some(Some((false, Resolved::Missing))) => {
                STATUS_OBJECT_NAME_NOT_FOUND
            }
            _ => call(oa),
        }
    })
}

unsafe extern "system" fn nt_close(handle: HANDLE) -> NTSTATUS {
    catch_hook_panic("NtClose", || {
        // Our own file work closes handles while holding the listing lock; those handles were never
        // tracked, and taking the lock again on this thread would deadlock.
        if is_inside() {
            return hooks().close.call(handle);
        }
        if let Ok(mut m) = listings().lock() {
            m.remove(&(handle as usize));
        }
        if let Ok(mut m) = roots().lock() {
            m.remove(&(handle as usize));
        }
        let removed = handles()
            .lock()
            .ok()
            .and_then(|mut m| m.remove(&(handle as usize)));
        if let Some(t) = removed {
            if t.lower && t.delete_on_close {
                let c = cfg();
                guarded(|| add_whiteout(&t.rel, &c.upper));
            }
        }
        hooks().close.call(handle)
    })
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
    let ok = hooks().create_process.call(
        token,
        app,
        cmd,
        pa,
        ta,
        inherit,
        flags | CREATE_SUSPENDED,
        env,
        cwd,
        si,
        pi,
        new_token,
    );
    if ok != 0 {
        // Our own file work (the short path name, the log) must not be redirected.
        let was_inside = is_inside();
        crate::util::set_inside(true);
        match agora_vfs_inject::inject_child((*pi).hProcess, &cfg().dll) {
            Ok((method, fallback_from)) => log(format!(
                "injected child {} by {}{}",
                (*pi).dwProcessId,
                method.describe(),
                fallback_from
                    .map(|why| format!(" (import-table injection was not possible: {why})"))
                    .unwrap_or_default()
            )),
            Err(e) => log(format!(
                "inject into child {} failed: {e}",
                (*pi).dwProcessId
            )),
        }
        crate::util::set_inside(was_inside);
        if flags & CREATE_SUSPENDED == 0 {
            ResumeThread((*pi).hThread);
        }
    }
    ok
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
    catch_hook_panic("NtQueryDirectoryFile", || {
        let c = cfg();
        match query_directory(
            handle,
            iosb,
            out,
            length,
            class,
            single != 0,
            pattern,
            restart != 0,
            c,
        ) {
            Some(status) => status,
            None => hooks().query_dir.call(
                handle, event, apc, apc_ctx, iosb, out, length, class, single, pattern, restart,
            ),
        }
    })
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
    catch_hook_panic("NtQueryDirectoryFileEx", || {
        let single = flags & SL_RETURN_SINGLE_ENTRY != 0;
        let restart = flags & SL_RESTART_SCAN != 0;
        let c = cfg();
        match query_directory(
            handle, iosb, out, length, class, single, pattern, restart, c,
        ) {
            Some(status) => status,
            None => hooks().query_dir_ex.call(
                handle, event, apc, apc_ctx, iosb, out, length, class, flags, pattern,
            ),
        }
    })
}

unsafe fn proc_addr<T>(module: &str, name: &str) -> T {
    let m: Vec<u16> = module.encode_utf16().chain(Some(0)).collect();
    let n = std::ffi::CString::new(name).unwrap();
    let f = GetProcAddress(GetModuleHandleW(m.as_ptr()), n.as_ptr() as *const u8).expect(name);
    std::mem::transmute_copy(&f)
}

pub unsafe fn install_hooks() -> Result<(), retour::Error> {
    let mut installed: Vec<&'static dyn HookEnable> = Vec::new();

    let create = GenericDetour::new(
        proc_addr::<NtCreateFileFn>("ntdll.dll", "NtCreateFile"),
        nt_create_file,
    )?;
    let open = GenericDetour::new(
        proc_addr::<NtOpenFileFn>("ntdll.dll", "NtOpenFile"),
        nt_open_file,
    )?;
    let set_info = GenericDetour::new(
        proc_addr::<NtSetInformationFileFn>("ntdll.dll", "NtSetInformationFile"),
        nt_set_information_file,
    )?;
    let query_attr = GenericDetour::new(
        proc_addr::<NtQueryAttributesFileFn>("ntdll.dll", "NtQueryAttributesFile"),
        nt_query_attributes_file,
    )?;
    let query_full_attr = GenericDetour::new(
        proc_addr::<NtQueryAttributesFileFn>("ntdll.dll", "NtQueryFullAttributesFile"),
        nt_query_full_attributes_file,
    )?;
    let close = GenericDetour::new(proc_addr::<NtCloseFn>("ntdll.dll", "NtClose"), nt_close)?;
    let create_process = GenericDetour::new(
        proc_addr::<CreateProcessInternalWFn>("kernelbase.dll", "CreateProcessInternalW"),
        create_process_internal_w,
    )?;
    let query_dir = GenericDetour::new(
        proc_addr::<NtQueryDirectoryFileFn>("ntdll.dll", "NtQueryDirectoryFile"),
        nt_query_directory_file,
    )?;
    let query_dir_ex = GenericDetour::new(
        proc_addr::<NtQueryDirectoryFileExFn>("ntdll.dll", "NtQueryDirectoryFileEx"),
        nt_query_directory_file_ex,
    )?;
    let query_by_name = GenericDetour::new(
        proc_addr::<NtQueryInformationByNameFn>("ntdll.dll", "NtQueryInformationByName"),
        nt_query_information_by_name,
    )?;

    let h = Hooks {
        create,
        open,
        set_info,
        query_attr,
        query_full_attr,
        close,
        create_process,
        query_dir,
        query_dir_ex,
        query_by_name,
    };
    let h = HOOKS.get_or_init(|| h);

    trait HookEnable {
        unsafe fn enable_hook(&self) -> Result<(), retour::Error>;
        unsafe fn disable_hook(&self) -> Result<(), retour::Error>;
    }
    impl<T: Copy + retour::Function> HookEnable for GenericDetour<T> {
        unsafe fn enable_hook(&self) -> Result<(), retour::Error> {
            self.enable()
        }
        unsafe fn disable_hook(&self) -> Result<(), retour::Error> {
            self.disable()
        }
    }

    let all_hooks: [&dyn HookEnable; 10] = [
        &h.create,
        &h.open,
        &h.set_info,
        &h.query_attr,
        &h.query_full_attr,
        &h.close,
        &h.create_process,
        &h.query_dir,
        &h.query_dir_ex,
        &h.query_by_name,
    ];

    for hook in all_hooks {
        if let Err(e) = hook.enable_hook() {
            log(format!("Failed to enable hook, rolling back: {e}"));
            for prev in installed {
                let _ = prev.disable_hook();
            }
            return Err(e);
        }
        // transmute reference to static lifetime since `h` is stored in static OnceLock HOOKS
        let static_ref: &'static dyn HookEnable = std::mem::transmute(hook);
        installed.push(static_ref);
    }

    Ok(())
}
