//! Agora VFS Probe Test Binary
//!
//! Ported from Spike 2 harness. Used to perform operations on the virtual file system
//! and write output reports for conformance testing.

#![cfg(windows)]

use std::ffi::{c_void, OsStr};
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::json;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Storage::FileSystem::*;

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

fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}

fn last_error() -> u32 {
    unsafe { GetLastError() }
}

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
    let ok = unsafe {
        WriteFile(
            h,
            bytes.as_ptr(),
            bytes.len() as u32,
            &mut written,
            std::ptr::null_mut(),
        )
    };
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

fn set_info(
    h: HANDLE,
    class: FILE_INFO_BY_HANDLE_CLASS,
    buf: *const c_void,
    len: usize,
) -> Result<(), u32> {
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

fn moved(p: &Path) -> PathBuf {
    p.with_extension("moved")
}

fn visible(p: &Path) -> String {
    match fs::read(p) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(_) => "<absent>".into(),
    }
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
            check(unsafe {
                MoveFileExW(
                    wide(&tmp).as_ptr(),
                    wide(p).as_ptr(),
                    MOVEFILE_REPLACE_EXISTING,
                )
            })
        }
        "handle_rename_replace" => {
            let h = create_with(&tmp, b"REPLACED-BY-HANDLE", DELETE, CREATE_NEW)?;
            let r = rename_by_handle(h, p);
            close(h);
            r
        }
        "replace_file" => {
            close(create_with(
                &tmp,
                b"REPLACED-BY-REPLACEFILE",
                0,
                CREATE_NEW,
            )?);
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
            let r = set_info(
                h,
                FileDispositionInfo,
                &info as *const _ as *const c_void,
                std::mem::size_of_val(&info),
            );
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
            let r = set_info(
                h,
                FileDispositionInfoEx,
                &info as *const _ as *const c_void,
                std::mem::size_of_val(&info),
            );
            close(h);
            r
        }
        "rename_away" => {
            check(unsafe { MoveFileExW(wide(p).as_ptr(), wide(moved(p)).as_ptr(), 0) })
        }
        _ => unreachable!(),
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
    fs::write(
        out_dir.join(name),
        serde_json::to_vec_pretty(&results).unwrap(),
    )
    .unwrap();

    if !child {
        let exe = std::env::current_exe().unwrap();
        let status = Command::new(exe)
            .args(["--root"])
            .arg(root)
            .arg("--out")
            .arg(out_dir)
            .arg("--child")
            .status();
        fs::write(out_dir.join("child-status.txt"), format!("{status:?}")).unwrap();
        let mut names: Vec<String> = match fs::read_dir(&data) {
            Ok(rd) => rd
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect(),
            Err(e) => vec![format!("<read_dir failed: {e}>")],
        };
        names.sort();
        fs::write(
            out_dir.join("listing.json"),
            serde_json::to_vec_pretty(&names).unwrap(),
        )
        .unwrap();

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
            if unsafe { GetFileAttributesW(wide(data.join(&name)).as_ptr()) }
                != INVALID_FILE_ATTRIBUTES
            {
                found.push(name);
            }
        }
        let data_ok =
            unsafe { GetFileAttributesW(wide(&data).as_ptr()) } != INVALID_FILE_ATTRIBUTES;
        fs::write(
            out_dir.join("attrs.json"),
            serde_json::to_vec_pretty(&json!({"data": data_ok, "found": found})).unwrap(),
        )
        .unwrap();
    }
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = arg(&args, "--root").expect("--root required");
    let out = arg(&args, "--out").expect("--out required");
    let child = args.iter().any(|a| a == "--child");
    probe(Path::new(&root), child, Path::new(&out));
}
