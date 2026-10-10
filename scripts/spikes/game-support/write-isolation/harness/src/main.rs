//! Spike 2 (MASTER_SPEC §26.5, §26.13): does usvfs keep writes away from shared files?
//!
//! `spike2 run --usvfs <bin dir> --sandbox <dir> --topology mo2|full [--readonly]`
//! builds a sandbox, mounts it through usvfs, and starts this binary hooked as a probe. The probe
//! tries every kind of write on files from each lower layer, then starts a child of itself (which
//! usvfs hooks too) to try them all again. The controller then checks the real files.
//!
//! Layers: `content` (a mod folder, shared), `base_copied` (a pinned base's own copy),
//! `base_linked` (a pinned base file hardlinked to the store install, `store/`).
//! Topologies: `mo2` mounts mods and overwrite onto the base folder itself, as MO2 does with a
//! game's Data folder; `full` mounts base, mods and overwrite onto an empty folder.

use std::ffi::{c_char, c_void, CString, OsStr};
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::Threading::*;

const LAYERS: &[&str] = &["content", "base_copied", "base_linked"];
const OPS: &[&str] = &[
    "write_in_place",
    "truncate_rewrite",
    "rename_replace",
    "handle_rename_replace",
    "replace_file",
    "delete",
    "handle_delete",
    "posix_delete",
    "posix_delete_ignore_ro",
    "rename_away",
];

const LINKFLAG_CREATETARGET: u32 = 0x4;
const LINKFLAG_RECURSIVE: u32 = 0x8;

fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}

fn last_error() -> u32 {
    unsafe { GetLastError() }
}

// ---------------------------------------------------------------- probe operations

fn open(path: &Path, access: u32, disposition: u32) -> Result<HANDLE, u32> {
    let w = wide(path);
    let h = unsafe {
        CreateFileW(
            w.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            disposition,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        Err(last_error())
    } else {
        Ok(h)
    }
}

fn write_all(h: HANDLE, bytes: &[u8]) -> Result<(), u32> {
    let mut written = 0u32;
    let ok = unsafe { WriteFile(h, bytes.as_ptr(), bytes.len() as u32, &mut written, std::ptr::null_mut()) };
    if ok == 0 {
        Err(last_error())
    } else {
        Ok(())
    }
}

fn close(h: HANDLE) {
    unsafe { CloseHandle(h) };
}

fn create_with(path: &Path, bytes: &[u8], access: u32, disposition: u32) -> Result<HANDLE, u32> {
    let h = open(path, access | GENERIC_WRITE, disposition)?;
    if let Err(e) = write_all(h, bytes) {
        close(h);
        return Err(e);
    }
    Ok(h)
}

fn check(ok: BOOL) -> Result<(), u32> {
    if ok == 0 {
        Err(last_error())
    } else {
        Ok(())
    }
}

fn set_info(h: HANDLE, class: FILE_INFO_BY_HANDLE_CLASS, buf: *const c_void, len: usize) -> Result<(), u32> {
    check(unsafe { SetFileInformationByHandle(h, class, buf, len as u32) })
}

fn rename_by_handle(h: HANDLE, target: &Path) -> Result<(), u32> {
    let name: Vec<u16> = target.as_os_str().encode_wide().collect();
    let header = std::mem::size_of::<FILE_RENAME_INFO>();
    let len = header + name.len() * 2;
    let mut buf = vec![0u64; len.div_ceil(8)];
    unsafe {
        let info = buf.as_mut_ptr() as *mut FILE_RENAME_INFO;
        (*info).Anonymous.Flags = 1; // ReplaceIfExists
        (*info).RootDirectory = std::ptr::null_mut();
        (*info).FileNameLength = (name.len() * 2) as u32;
        std::ptr::copy_nonoverlapping(name.as_ptr(), (*info).FileName.as_mut_ptr(), name.len());
    }
    set_info(h, FileRenameInfo, buf.as_ptr() as *const c_void, len)
}

fn run_op(op: &str, p: &Path) -> Result<(), u32> {
    let tmp = p.with_extension("agoratmp");
    match op {
        "write_in_place" => {
            let h = open(p, GENERIC_READ | GENERIC_WRITE, OPEN_EXISTING)?;
            let r = write_all(h, b"MODIFIED");
            close(h);
            r
        }
        "truncate_rewrite" => {
            let h = create_with(p, b"REWRITTEN", 0, CREATE_ALWAYS)?;
            close(h);
            Ok(())
        }
        "rename_replace" => {
            close(create_with(&tmp, b"REPLACED-BY-MOVE", 0, CREATE_NEW)?);
            check(unsafe { MoveFileExW(wide(&tmp).as_ptr(), wide(p).as_ptr(), MOVEFILE_REPLACE_EXISTING) })
        }
        "handle_rename_replace" => {
            let h = create_with(&tmp, b"REPLACED-BY-HANDLE", DELETE, CREATE_NEW)?;
            let r = rename_by_handle(h, p);
            close(h);
            r
        }
        "replace_file" => {
            close(create_with(&tmp, b"REPLACED-BY-REPLACEFILE", 0, CREATE_NEW)?);
            check(unsafe {
                ReplaceFileW(
                    wide(p).as_ptr(),
                    wide(&tmp).as_ptr(),
                    std::ptr::null(),
                    0,
                    std::ptr::null(),
                    std::ptr::null(),
                )
            })
        }
        "delete" => check(unsafe { DeleteFileW(wide(p).as_ptr()) }),
        "handle_delete" => {
            let h = open(p, DELETE, OPEN_EXISTING)?;
            let info = FILE_DISPOSITION_INFO { DeleteFile: 1 };
            let r = set_info(h, FileDispositionInfo, &info as *const _ as *const c_void, std::mem::size_of_val(&info));
            close(h);
            r
        }
        "posix_delete" | "posix_delete_ignore_ro" => {
            let h = open(p, DELETE, OPEN_EXISTING)?;
            let mut flags = FILE_DISPOSITION_FLAG_DELETE | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS;
            if op == "posix_delete_ignore_ro" {
                flags |= FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE;
            }
            let info = FILE_DISPOSITION_INFO_EX { Flags: flags };
            let r = set_info(h, FileDispositionInfoEx, &info as *const _ as *const c_void, std::mem::size_of_val(&info));
            close(h);
            r
        }
        "rename_away" => check(unsafe { MoveFileExW(wide(p).as_ptr(), wide(moved(p)).as_ptr(), 0) }),
        _ => unreachable!(),
    }
}

fn moved(p: &Path) -> PathBuf {
    p.with_extension("moved")
}

fn visible(p: &Path) -> String {
    match fs::read(p) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(_) => "<absent>".into(),
    }
}

fn probe(root: &Path, child: bool, out_dir: &Path) {
    let prefix = if child { "child_" } else { "" };
    let data = root.join("Data");
    let mut results = Vec::new();
    for layer in LAYERS {
        for op in OPS {
            let p = data.join(format!("{prefix}{layer}__{op}.txt"));
            let r = run_op(op, &p);
            let mut entry = json!({
                "layer": layer, "op": op, "ok": r.is_ok(), "error": r.err(),
                "visible": visible(&p),
            });
            if *op == "rename_away" {
                entry["visible_moved"] = json!(visible(&moved(&p)));
            }
            results.push(entry);
        }
    }
    let created = data.join(format!("{prefix}created.txt"));
    let r = create_with(&created, b"NEW", 0, CREATE_NEW).map(close);
    results.push(json!({
        "layer": "none", "op": "create_new", "ok": r.is_ok(), "error": r.err(), "visible": visible(&created),
    }));
    let name = if child { "child.json" } else { "direct.json" };
    fs::write(out_dir.join(name), serde_json::to_vec_pretty(&results).unwrap()).unwrap();

    if !child {
        let exe = std::env::current_exe().unwrap();
        let status = Command::new(exe)
            .args(["probe", "--root"])
            .arg(root)
            .arg("--out")
            .arg(out_dir)
            .arg("--child")
            .status();
        fs::write(out_dir.join("child-status.txt"), format!("{status:?}")).unwrap();
        let mut names: Vec<String> = match fs::read_dir(&data) {
            Ok(rd) => rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect(),
            Err(e) => vec![format!("<read_dir failed: {e}>")],
        };
        names.sort();
        fs::write(out_dir.join("listing.json"), serde_json::to_vec_pretty(&names).unwrap()).unwrap();
        // GetFileAttributesW answers by name (NtQueryInformationByName on Windows 11 24H2).
        let mut found = Vec::new();
        for prefix in ["", "child_"] {
            for layer in LAYERS {
                for op in OPS {
                    for ext in ["txt", "moved"] {
                        let name = format!("{prefix}{layer}__{op}.{ext}");
                        let attrs = unsafe { GetFileAttributesW(wide(data.join(&name)).as_ptr()) };
                        if attrs != INVALID_FILE_ATTRIBUTES {
                            found.push(name);
                        }
                    }
                }
            }
            let name = format!("{prefix}created.txt");
            if unsafe { GetFileAttributesW(wide(data.join(&name)).as_ptr()) } != INVALID_FILE_ATTRIBUTES {
                found.push(name);
            }
        }
        let data_ok = unsafe { GetFileAttributesW(wide(&data).as_ptr()) } != INVALID_FILE_ATTRIBUTES;
        fs::write(out_dir.join("attrs.json"), serde_json::to_vec_pretty(&json!({"data": data_ok, "found": found})).unwrap()).unwrap();
    }
}

// ---------------------------------------------------------------- controller

fn file_name(prefix: &str, layer: &str, op: &str) -> String {
    format!("{prefix}{layer}__{op}.txt")
}

fn setup(sb: &Path, readonly: bool, acl: bool) {
    if sb.exists() {
        let _ = Command::new("icacls").arg(sb).args(["/reset", "/T", "/Q"]).status();
        clear_readonly(sb);
        fs::remove_dir_all(sb).unwrap();
    }
    for d in ["store/Data", "base/Data", "content/modA/Data", "overwrite", "mount", "results"] {
        fs::create_dir_all(sb.join(d)).unwrap();
    }
    for prefix in ["", "child_"] {
        for op in OPS {
            for layer in LAYERS {
                let name = file_name(prefix, layer, op);
                let orig = format!("ORIG {layer} {op}");
                match *layer {
                    "content" => fs::write(sb.join("content/modA/Data").join(&name), &orig).unwrap(),
                    "base_copied" => {
                        fs::write(sb.join("store/Data").join(&name), &orig).unwrap();
                        fs::write(sb.join("base/Data").join(&name), &orig).unwrap();
                    }
                    "base_linked" => {
                        fs::write(sb.join("store/Data").join(&name), &orig).unwrap();
                        fs::hard_link(sb.join("store/Data").join(&name), sb.join("base/Data").join(&name)).unwrap();
                    }
                    _ => unreachable!(),
                }
            }
        }
    }
    if acl {
        // Deny the current user every right that changes a file: delete, write data, append,
        // write attributes and extended attributes; on the folder, adding and deleting children.
        // Not icacls: it widens these to its `W`, which includes SYNCHRONIZE and so denies reads.
        let script = r#"
param($sb, $fileRights)
function Deny($path, $rights) {
  $acl = Get-Acl -LiteralPath $path
  $acl.AddAccessRule((New-Object System.Security.AccessControl.FileSystemAccessRule($env:USERNAME, $rights, 'Deny')))
  Set-Acl -LiteralPath $path $acl
}
foreach ($d in @('content\modA\Data', 'base\Data')) {
  $dir = Join-Path $sb $d
  Get-ChildItem -LiteralPath $dir -File | ForEach-Object { Deny $_.FullName $fileRights }
  Deny $dir 'CreateFiles,CreateDirectories,DeleteSubdirectoriesAndFiles'
}
"#;
        let ps1 = sb.join("results/deny.ps1");
        fs::write(&ps1, script).unwrap();
        let s = Command::new("powershell")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(&ps1)
            .arg(sb)
            // Link deployment needs WriteAttributes left open: creating a hardlink requires it.
            .arg(std::env::var("SPIKE2_FILE_DENY").unwrap_or_else(|_| {
                "WriteData,AppendData,WriteExtendedAttributes,WriteAttributes,Delete".into()
            }))
            .status()
            .unwrap();
        assert!(s.success());
    }
    if readonly {
        for dir in ["content/modA/Data", "base/Data"] {
            for e in fs::read_dir(sb.join(dir)).unwrap() {
                let p = e.unwrap().path();
                let mut perm = fs::metadata(&p).unwrap().permissions();
                perm.set_readonly(true);
                fs::set_permissions(&p, perm).unwrap();
            }
        }
    }
}

fn clear_readonly(dir: &Path) {
    for e in walk(dir) {
        if let Ok(m) = fs::metadata(&e) {
            let mut perm = m.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            perm.set_readonly(false);
            let _ = fs::set_permissions(&e, perm);
        }
    }
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
    }
    out
}

fn state(p: &Path, orig: &str) -> String {
    match fs::read(p) {
        Ok(b) if b == orig.as_bytes() => "orig".into(),
        Ok(b) => format!("CHANGED:{}", String::from_utf8_lossy(&b)),
        Err(_) => "GONE".into(),
    }
}

struct Usvfs {
    lib: libloading::Library,
}

impl Usvfs {
    unsafe fn sym<T: Copy>(&self, name: &[u8]) -> T {
        *self.lib.get::<T>(name).unwrap_or_else(|e| panic!("{}: {e}", String::from_utf8_lossy(name)))
    }
}

fn run(usvfs_bin: &Path, sb: &Path, topology: &str, readonly: bool, acl: bool) {
    setup(sb, readonly, acl);
    let lib = unsafe { libloading::Library::new(usvfs_bin.join("usvfs_x64.dll")) }.expect("load usvfs_x64.dll");
    let u = Usvfs { lib };
    let results = sb.join("results");
    let mount = match topology {
        "mo2" => sb.join("base"),
        "full" => sb.join("mount"),
        t => panic!("unknown topology {t}"),
    };
    unsafe {
        let version: unsafe extern "system" fn() -> *const c_char = u.sym(b"usvfsVersionString");
        println!("usvfs {}", std::ffi::CStr::from_ptr(version()).to_string_lossy());
        let create_params: unsafe extern "C" fn() -> *mut c_void = u.sym(b"usvfsCreateParameters");
        let set_name: unsafe extern "C" fn(*mut c_void, *const c_char) = u.sym(b"usvfsSetInstanceName");
        let set_debug: unsafe extern "C" fn(*mut c_void, BOOL) = u.sym(b"usvfsSetDebugMode");
        let set_level: unsafe extern "C" fn(*mut c_void, u8) = u.sym(b"usvfsSetLogLevel");
        let set_dump: unsafe extern "C" fn(*mut c_void, u8) = u.sym(b"usvfsSetCrashDumpType");
        let free_params: unsafe extern "C" fn(*mut c_void) = u.sym(b"usvfsFreeParameters");
        let init_logging: unsafe extern "system" fn(bool) = u.sym(b"usvfsInitLogging");
        let create_vfs: unsafe extern "system" fn(*const c_void) -> BOOL = u.sym(b"usvfsCreateVFS");
        let link_dir: unsafe extern "system" fn(*const u16, *const u16, u32) -> BOOL =
            u.sym(b"usvfsVirtualLinkDirectoryStatic");
        let create_process: unsafe extern "system" fn(
            *const u16,
            *mut u16,
            *const c_void,
            *const c_void,
            BOOL,
            u32,
            *const c_void,
            *const u16,
            *mut STARTUPINFOW,
            *mut PROCESS_INFORMATION,
        ) -> BOOL = u.sym(b"usvfsCreateProcessHooked");
        let get_log: unsafe extern "system" fn(*mut u8, usize, bool) -> bool = u.sym(b"usvfsGetLogMessages");
        let disconnect: unsafe extern "system" fn() = u.sym(b"usvfsDisconnectVFS");

        init_logging(false);
        let params = create_params();
        let name = CString::new(format!("agora_spike2_{}", std::process::id())).unwrap();
        set_name(params, name.as_ptr());
        set_debug(params, 0);
        set_level(params, 0); // Debug
        set_dump(params, 0); // None
        assert!(create_vfs(params) != 0, "usvfsCreateVFS failed: {}", last_error());
        free_params(params);

        let link = |src: &Path, dst: &Path, flags: u32| {
            let ok = link_dir(wide(src).as_ptr(), wide(dst).as_ptr(), flags);
            assert!(ok != 0, "link {} -> {} failed: {}", src.display(), dst.display(), last_error());
        };
        if topology == "full" {
            link(&sb.join("base"), &mount, LINKFLAG_RECURSIVE);
        }
        link(&sb.join("content/modA"), &mount, LINKFLAG_RECURSIVE);
        link(&sb.join("overwrite"), &mount, LINKFLAG_CREATETARGET | LINKFLAG_RECURSIVE);

        let exe = std::env::current_exe().unwrap();
        let mut cmd = wide(format!(
            "\"{}\" probe --root \"{}\" --out \"{}\"",
            exe.display(),
            mount.display(),
            results.display()
        ));
        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let ok = create_process(
            std::ptr::null(),
            cmd.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            0,
            std::ptr::null(),
            std::ptr::null(),
            &mut si,
            &mut pi,
        );
        assert!(ok != 0, "usvfsCreateProcessHooked failed: {}", last_error());
        WaitForSingleObject(pi.hProcess, 120_000);
        let mut code = 0u32;
        GetExitCodeProcess(pi.hProcess, &mut code);
        println!("probe exit code {code}");
        CloseHandle(pi.hProcess);
        CloseHandle(pi.hThread);

        let mut log = String::new();
        let mut buf = vec![0u8; 16 * 1024];
        while get_log(buf.as_mut_ptr(), buf.len(), false) {
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            log.push_str(&String::from_utf8_lossy(&buf[..end]));
            log.push('\n');
        }
        fs::write(results.join("usvfs.log"), log).unwrap();
        disconnect();
    }
    report(sb, topology, readonly);
}

/// Start a real game through usvfs: `base` mounted (with `overwrite` as the create target) on the
/// empty folder `mount`, the game started from its virtual path. Drains usvfs's log while the game
/// runs (its buffer is fixed-size) and waits until every process in the VFS has exited.
fn launch(usvfs_bin: &Path, base: &Path, mount: &Path, overwrite: &Path, exe: &str, log_path: &Path) {
    fs::create_dir_all(mount).unwrap();
    fs::create_dir_all(overwrite).unwrap();
    assert!(fs::read_dir(mount).unwrap().next().is_none(), "mount folder must be empty");
    let lib = unsafe { libloading::Library::new(usvfs_bin.join("usvfs_x64.dll")) }.expect("load usvfs_x64.dll");
    let u = Usvfs { lib };
    unsafe {
        let create_params: unsafe extern "C" fn() -> *mut c_void = u.sym(b"usvfsCreateParameters");
        let set_name: unsafe extern "C" fn(*mut c_void, *const c_char) = u.sym(b"usvfsSetInstanceName");
        let set_level: unsafe extern "C" fn(*mut c_void, u8) = u.sym(b"usvfsSetLogLevel");
        let init_logging: unsafe extern "system" fn(bool) = u.sym(b"usvfsInitLogging");
        let create_vfs: unsafe extern "system" fn(*const c_void) -> BOOL = u.sym(b"usvfsCreateVFS");
        let link_dir: unsafe extern "system" fn(*const u16, *const u16, u32) -> BOOL =
            u.sym(b"usvfsVirtualLinkDirectoryStatic");
        let create_process: unsafe extern "system" fn(
            *const u16, *mut u16, *const c_void, *const c_void, BOOL, u32, *const c_void, *const u16,
            *mut STARTUPINFOW, *mut PROCESS_INFORMATION,
        ) -> BOOL = u.sym(b"usvfsCreateProcessHooked");
        let get_log: unsafe extern "system" fn(*mut u8, usize, bool) -> bool = u.sym(b"usvfsGetLogMessages");
        let process_list: unsafe extern "system" fn(*mut usize, *mut u32) -> BOOL = u.sym(b"usvfsGetVFSProcessList");
        let disconnect: unsafe extern "system" fn() = u.sym(b"usvfsDisconnectVFS");

        init_logging(false);
        let params = create_params();
        let name = CString::new(format!("agora_spike2_{}", std::process::id())).unwrap();
        set_name(params, name.as_ptr());
        set_level(params, 0);
        assert!(create_vfs(params) != 0, "usvfsCreateVFS failed");

        let started = std::time::Instant::now();
        let ok = link_dir(wide(base).as_ptr(), wide(mount).as_ptr(), LINKFLAG_RECURSIVE);
        assert!(ok != 0, "link base failed: {}", last_error());
        let ok = link_dir(wide(overwrite).as_ptr(), wide(mount).as_ptr(), LINKFLAG_CREATETARGET | LINKFLAG_RECURSIVE);
        assert!(ok != 0, "link overwrite failed: {}", last_error());
        let files = walk(base).len();
        println!("mounted {files} files in {:?}", started.elapsed());

        let exe_path = mount.join(exe);
        let cwd = exe_path.parent().unwrap().to_path_buf();
        // The controller is not hooked, so CreateProcess needs the working directory and the
        // executable on disk. The exe is hardlinked from the base (a copied file there, never the
        // store's), so the game still sees its own path inside the mount and resolves its data
        // folders through the VFS.
        // The loader resolves the exe's static imports before usvfs's hooks are live, so every
        // .exe and .dll beside it must be physical too (measured: Skyrim exits 0xC0000135 otherwise).
        fs::create_dir_all(&cwd).unwrap();
        let exe_dir_in_base = base.join(exe).parent().unwrap().to_path_buf();
        for entry in fs::read_dir(&exe_dir_in_base).unwrap().flatten() {
            let path = entry.path();
            let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase());
            if path.is_file() && matches!(ext.as_deref(), Some("exe") | Some("dll")) {
                let target = cwd.join(entry.file_name());
                if !target.exists() {
                    fs::hard_link(&path, &target).expect("hardlink binary into mount");
                }
            }
        }
        let mut cmd = wide(format!("\"{}\"", exe_path.display()));
        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let ok = create_process(
            wide(&exe_path).as_ptr(), cmd.as_mut_ptr(), std::ptr::null(), std::ptr::null(), 0, 0,
            std::ptr::null(), wide(&cwd).as_ptr(), &mut si, &mut pi,
        );
        assert!(ok != 0, "usvfsCreateProcessHooked failed: {}", last_error());
        println!("started {} (PID {})", exe_path.display(), pi.dwProcessId);

        let mut log = std::io::BufWriter::new(fs::File::create(log_path).unwrap());
        let mut buf = vec![0u8; 16 * 1024];
        let mut drain = |log: &mut std::io::BufWriter<fs::File>| {
            use std::io::Write;
            while get_log(buf.as_mut_ptr(), buf.len(), false) {
                let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                log.write_all(&buf[..end]).unwrap();
                log.write_all(b"
").unwrap();
            }
        };
        let mut pids = vec![0u32; 256];
        loop {
            drain(&mut log);
            let main_done = WaitForSingleObject(pi.hProcess, 100) == WAIT_OBJECT_0;
            let mut count = pids.len();
            let listed = process_list(&mut count, pids.as_mut_ptr()) != 0;
            let others = if listed { pids[..count].iter().filter(|p| **p != std::process::id()).count() } else { 0 };
            if main_done && others == 0 {
                break;
            }
        }
        drain(&mut log);
        let mut code = 0u32;
        GetExitCodeProcess(pi.hProcess, &mut code);
        println!("game exited with code {code} after {:?}", started.elapsed());
        CloseHandle(pi.hProcess);
        CloseHandle(pi.hThread);
        disconnect();
    }
    let created: Vec<String> = walk(overwrite).iter().map(|p| rel(overwrite, p)).collect();
    println!("overwrite/ holds {} new files: {:?}", created.len(), created);
    println!("mount/ physically holds {} files (folders are expected)", walk(mount).len());
}

/// Run the probe under the agvfs prototype: lowers `content/modA` then `base`, upper `overwrite`,
/// mounted at the empty `mount`.
fn run_agvfs(dll: &Path, sb: &Path, readonly: bool, acl: bool) {
    setup(sb, readonly, acl);
    let results = sb.join("results");
    let mount = sb.join("mount");
    let cfg = results.join("agvfs.cfg");
    fs::write(
        &cfg,
        format!(
            "mount={}
upper={}
lower={}
lower={}
dll={}
log={}
",
            mount.display(),
            sb.join("overwrite").display(),
            sb.join("content").join("modA").display(),
            sb.join("base").display(),
            dll.display(),
            results.join("agvfs.log").display()
        ),
    )
    .unwrap();
    std::env::set_var("AGVFS_CONFIG", &cfg);
    let exe = std::env::current_exe().unwrap();
    let mut cmd = wide(format!(
        "\"{}\" probe --root \"{}\" --out \"{}\"",
        exe.display(),
        mount.display(),
        results.display()
    ));
    unsafe {
        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let ok = CreateProcessW(std::ptr::null(), cmd.as_mut_ptr(), std::ptr::null(), std::ptr::null(), 0,
            CREATE_SUSPENDED, std::ptr::null(), std::ptr::null(), &si, &mut pi);
        assert!(ok != 0, "CreateProcessW failed: {}", last_error());
        inject(pi.hProcess, dll).expect("inject agvfs");
        ResumeThread(pi.hThread);
        WaitForSingleObject(pi.hProcess, 120_000);
        let mut code = 0u32;
        GetExitCodeProcess(pi.hProcess, &mut code);
        println!("probe exit code {code}");
        CloseHandle(pi.hProcess);
        CloseHandle(pi.hThread);
    }
    std::env::remove_var("AGVFS_CONFIG");
    report(sb, "agvfs", readonly);
}

/// Load `dll` into a suspended `process` with a remote `LoadLibraryW`.
unsafe fn inject(process: HANDLE, dll: &Path) -> Result<(), String> {
    use windows_sys::Win32::System::Diagnostics::Debug::WriteProcessMemory;
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    use windows_sys::Win32::System::Memory::{VirtualAllocEx, MEM_COMMIT, MEM_RESERVE, PAGE_READWRITE};
    let w = wide(dll);
    let bytes = w.len() * 2;
    let remote = VirtualAllocEx(process, std::ptr::null(), bytes, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
    if remote.is_null() {
        return Err(format!("VirtualAllocEx {}", last_error()));
    }
    if WriteProcessMemory(process, remote, w.as_ptr() as *const c_void, bytes, std::ptr::null_mut()) == 0 {
        return Err(format!("WriteProcessMemory {}", last_error()));
    }
    let load = GetProcAddress(GetModuleHandleW(wide("kernel32.dll").as_ptr()), c"LoadLibraryW".as_ptr() as *const u8)
        .ok_or("no LoadLibraryW")?;
    let thread = CreateRemoteThread(process, std::ptr::null(), 0,
        Some(std::mem::transmute::<unsafe extern "system" fn() -> isize, unsafe extern "system" fn(*mut c_void) -> u32>(load)),
        remote, 0, std::ptr::null_mut());
    if thread.is_null() {
        return Err(format!("CreateRemoteThread {}", last_error()));
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

/// Start a real game under the agvfs prototype: `base` as the only lower layer, `overwrite` as the
/// upper, mounted on the empty `mount`; binaries beside the exe placed physically (see `launch`).
fn launch_agvfs(dll: &Path, base: &Path, mount: &Path, overwrite: &Path, exe: &str, log_path: &Path) {
    fs::create_dir_all(mount).unwrap();
    fs::create_dir_all(overwrite).unwrap();
    let exe_path = mount.join(exe);
    let cwd = exe_path.parent().unwrap().to_path_buf();
    fs::create_dir_all(&cwd).unwrap();
    // Every .exe and .dll in the base is placed physically at its path in the mount: Windows maps a
    // new process's image and its static imports in the kernel or before hooks exist (Skyrim's DLLs,
    // Satisfactory's launcher starting Engine\...\Shipping.exe).
    for path in walk(base) {
        let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase());
        if matches!(ext.as_deref(), Some("exe") | Some("dll")) {
            let target = mount.join(path.strip_prefix(base).unwrap());
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            if !target.exists() {
                // A store install is never linked into: its binaries are copied.
                if std::env::var_os("SPIKE2_COPY_BINARIES").is_some() {
                    fs::copy(&path, &target).expect("copy binary into mount");
                } else {
                    fs::hard_link(&path, &target).expect("hardlink binary into mount");
                }
            }
        }
    }
    let cfg = log_path.with_extension("cfg");
    fs::write(
        &cfg,
        format!(
            "mount={}
upper={}
lower={}
dll={}
log={}
verbose=1
",
            mount.display(),
            overwrite.display(),
            base.display(),
            dll.display(),
            log_path.display()
        ),
    )
    .unwrap();
    std::env::set_var("AGVFS_CONFIG", &cfg);
    let started = std::time::Instant::now();
    let mut cmd = wide(format!("\"{}\"", exe_path.display()));
    unsafe {
        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let ok = CreateProcessW(std::ptr::null(), cmd.as_mut_ptr(), std::ptr::null(), std::ptr::null(), 0,
            CREATE_SUSPENDED, std::ptr::null(), wide(&cwd).as_ptr(), &si, &mut pi);
        assert!(ok != 0, "CreateProcessW failed: {}", last_error());
        inject(pi.hProcess, dll).expect("inject agvfs");
        ResumeThread(pi.hThread);
        println!("started {} (PID {}) under agvfs", exe_path.display(), pi.dwProcessId);
        WaitForSingleObject(pi.hProcess, INFINITE);
        let mut code = 0u32;
        GetExitCodeProcess(pi.hProcess, &mut code);
        println!("game exited with code {code:#x} after {:?}", started.elapsed());
        CloseHandle(pi.hProcess);
        CloseHandle(pi.hThread);
    }
    std::env::remove_var("AGVFS_CONFIG");
    let created: Vec<String> = walk(overwrite).iter().map(|p| rel(overwrite, p)).collect();
    println!("overwrite/ holds {} files: {:?}", created.len(), created);
}

/// Link deployment, the no-injection fallback: `mount` becomes a real folder of hardlinks (the mod
/// over the base), and the probe runs on it directly. With `--acl` the linked files carry the
/// deny entries of the files they link to.
fn run_links(sb: &Path, readonly: bool, acl: bool) {
    setup(sb, readonly, acl);
    let mount = sb.join("mount");
    for layer in [sb.join("content").join("modA"), sb.join("base")] {
        for file in walk(&layer) {
            let rel = file.strip_prefix(&layer).unwrap();
            let target = mount.join(rel);
            if !target.exists() {
                fs::create_dir_all(target.parent().unwrap()).unwrap();
                fs::hard_link(&file, &target).expect("hardlink into deployment");
            }
        }
    }
    let results = sb.join("results");
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["probe", "--root"])
        .arg(&mount)
        .arg("--out")
        .arg(&results)
        .status()
        .unwrap();
    println!("probe exit {status:?}");
    report(sb, "links", readonly);
}

fn report(sb: &Path, topology: &str, readonly: bool) {
    let results = sb.join("results");
    let mut rows = Vec::new();
    println!("\n== topology {topology}, readonly {readonly} ==");
    println!("{:<6} {:<12} {:<24} {:<5} {:<26} {:<22} {:<10}", "who", "layer", "op", "ok", "game sees", "real lower file", "store");
    for (who, prefix, file) in [("direct", "", "direct.json"), ("child", "child_", "child.json")] {
        let probe: Vec<Value> = match fs::read(results.join(file)) {
            Ok(b) => serde_json::from_slice(&b).unwrap(),
            Err(e) => {
                println!("{who}: no results ({e})");
                continue;
            }
        };
        for r in probe {
            let layer = r["layer"].as_str().unwrap();
            let op = r["op"].as_str().unwrap();
            let (lower, store) = if layer == "none" {
                ("-".to_string(), "-".to_string())
            } else {
                let name = file_name(prefix, layer, op);
                let orig = format!("ORIG {layer} {op}");
                let lower_path = match layer {
                    "content" => sb.join("content/modA/Data").join(&name),
                    _ => sb.join("base/Data").join(&name),
                };
                let store = if layer == "base_linked" { state(&sb.join("store/Data").join(&name), &orig) } else { "-".into() };
                (state(&lower_path, &orig), store)
            };
            let ok = if r["ok"].as_bool().unwrap() { "yes".to_string() } else { format!("e{}", r["error"]) };
            let mut seen = r["visible"].as_str().unwrap().to_string();
            if let Some(m) = r.get("visible_moved").and_then(|v| v.as_str()) {
                seen = format!("{seen} / moved:{m}");
            }
            println!("{who:<6} {layer:<12} {op:<24} {ok:<5} {:<26} {:<22} {:<10}", trunc(&seen, 26), trunc(&lower, 22), trunc(&store, 10));
            rows.push(json!({"who": who, "layer": layer, "op": op, "ok": r["ok"], "error": r["error"],
                "game_sees": seen, "lower": lower, "store": store}));
        }
    }
    let overwrite: Vec<String> = walk(&sb.join("overwrite")).iter().map(|p| rel(sb, p)).collect();
    let mount_leaks: Vec<String> = walk(&sb.join("mount")).iter().map(|p| rel(sb, p)).collect();
    let fixture_count = OPS.len() * 2 * 2; // base_copied + base_linked, direct + child
    let base_extra: Vec<String> = walk(&sb.join("base"))
        .iter()
        .map(|p| rel(sb, p))
        .filter(|p| {
            let f = p.rsplit('/').next().unwrap();
            !(f.ends_with(".txt") && (f.contains("base_copied__") || f.contains("base_linked__")))
        })
        .collect();
    println!("\noverwrite/ holds {} files: {:?}", overwrite.len(), overwrite);
    println!("mount/ physically holds {} files: {:?}", mount_leaks.len(), mount_leaks);
    println!("base/ holds {} files that are not fixtures (of {fixture_count}): {:?}", base_extra.len(), base_extra);
    println!("child status: {}", fs::read_to_string(results.join("child-status.txt")).unwrap_or_default());
    if let Ok(b) = fs::read(results.join("listing.json")) {
        // What the game can open must be exactly what it can list.
        let listed: Vec<String> = serde_json::from_slice(&b).unwrap();
        let mut expected = std::collections::BTreeSet::new();
        for (prefix, file) in [("", "direct.json"), ("child_", "child.json")] {
            let Ok(b) = fs::read(results.join(file)) else { continue };
            let probe: Vec<Value> = serde_json::from_slice(&b).unwrap();
            for r in probe {
                let (layer, op) = (r["layer"].as_str().unwrap(), r["op"].as_str().unwrap());
                let name = if layer == "none" { format!("{prefix}created.txt") } else { file_name(prefix, layer, op) };
                if r["visible"].as_str() != Some("<absent>") {
                    expected.insert(name.clone());
                }
                if r.get("visible_moved").and_then(|v| v.as_str()).is_some_and(|v| v != "<absent>") {
                    expected.insert(name.replace(".txt", ".moved"));
                }
            }
        }
        let listed_set: std::collections::BTreeSet<String> = listed.iter().cloned().collect();
        let missing: Vec<_> = expected.difference(&listed_set).collect();
        let extra: Vec<_> = listed_set.difference(&expected).collect();
        println!(
            "listing: {} names ({} duplicates); openable but not listed: {:?}; listed but not openable: {:?}",
            listed.len(),
            listed.len() - listed_set.len(),
            missing,
            extra
        );
        if let Ok(b) = fs::read(results.join("attrs.json")) {
            let a: Value = serde_json::from_slice(&b).unwrap();
            let found: std::collections::BTreeSet<String> =
                a["found"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
            println!(
                "attributes: Data folder {}; openable but no attributes: {:?}; attributes but not openable: {:?}",
                if a["data"].as_bool() == Some(true) { "found" } else { "MISSING" },
                expected.difference(&found).collect::<Vec<_>>(),
                found.difference(&expected).collect::<Vec<_>>()
            );
        }
    }
    let summary = json!({"topology": topology, "readonly": readonly, "rows": rows,
        "overwrite": overwrite, "mount_physical": mount_leaks, "base_extra": base_extra});
    fs::write(results.join("summary.json"), serde_json::to_vec_pretty(&summary).unwrap()).unwrap();
}

fn rel(sb: &Path, p: &Path) -> String {
    p.strip_prefix(sb).unwrap_or(p).display().to_string().replace('\\', "/")
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n - 1).collect::<String>() + "…"
    }
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("probe") => probe(
            Path::new(&arg(&args, "--root").unwrap()),
            args.iter().any(|a| a == "--child"),
            Path::new(&arg(&args, "--out").unwrap()),
        ),
        Some("run") => run(
            Path::new(&arg(&args, "--usvfs").expect("--usvfs")),
            Path::new(&arg(&args, "--sandbox").expect("--sandbox")),
            &arg(&args, "--topology").unwrap_or_else(|| "full".into()),
            args.iter().any(|a| a == "--readonly"),
            args.iter().any(|a| a == "--acl"),
        ),
        Some("agvfs") => run_agvfs(
            Path::new(&arg(&args, "--dll").expect("--dll")),
            Path::new(&arg(&args, "--sandbox").expect("--sandbox")),
            args.iter().any(|a| a == "--readonly"),
            args.iter().any(|a| a == "--acl"),
        ),
        Some("links") => run_links(
            Path::new(&arg(&args, "--sandbox").expect("--sandbox")),
            args.iter().any(|a| a == "--readonly"),
            args.iter().any(|a| a == "--acl"),
        ),
        Some("launch-agvfs") => launch_agvfs(
            Path::new(&arg(&args, "--dll").expect("--dll")),
            Path::new(&arg(&args, "--base").expect("--base")),
            Path::new(&arg(&args, "--mount").expect("--mount")),
            Path::new(&arg(&args, "--overwrite").expect("--overwrite")),
            &arg(&args, "--exe").expect("--exe"),
            Path::new(&arg(&args, "--log").expect("--log")),
        ),
        Some("launch") => launch(
            Path::new(&arg(&args, "--usvfs").expect("--usvfs")),
            Path::new(&arg(&args, "--base").expect("--base")),
            Path::new(&arg(&args, "--mount").expect("--mount")),
            Path::new(&arg(&args, "--overwrite").expect("--overwrite")),
            &arg(&args, "--exe").expect("--exe"),
            Path::new(&arg(&args, "--log").expect("--log")),
        ),
        Some("report") => report(
            Path::new(&arg(&args, "--sandbox").expect("--sandbox")),
            &arg(&args, "--topology").unwrap_or_else(|| "full".into()),
            args.iter().any(|a| a == "--readonly"),
        ),
        _ => eprintln!("usage: spike2 run --usvfs <bin> --sandbox <dir> --topology mo2|full [--readonly]"),
    }
}
