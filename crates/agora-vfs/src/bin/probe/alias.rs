//! The alias scenario, run under `agora_vfs.dll` with the install folder as an alias (`tests/alias.rs`
//! starts it). It checks the view through the alias and writes `alias.json`: the checks, the
//! listings, and the name queries of three handles, for the test to compare with the unhooked
//! reference on the real files.

use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Storage::FileSystem::*;

use super::names::{self, Check, NtOpen, Parity};

/// One folder the test expects to open the mod through, as an NT name.
#[derive(Deserialize)]
pub struct Form {
    pub name: String,
    /// The NT name of the folder, for example `\??\C:\...\install`. `\Data\mod.esp` is appended.
    pub prefix: String,
}

#[derive(Serialize, Default)]
struct Output {
    checks: Vec<Check>,
    listing_alias: Vec<String>,
    listing_mount: Vec<String>,
    alias_lower: Parity,
    mount_lower: Parity,
    alias_upper: Parity,
}

fn check(checks: &mut Vec<Check>, name: &str, ok: bool, detail: impl Into<String>) {
    checks.push(Check {
        name: name.to_string(),
        ok,
        detail: detail.into(),
    });
}

fn describe<T: std::fmt::Debug>(r: &io::Result<T>) -> String {
    match r {
        Ok(v) => format!("{v:?}"),
        Err(e) => format!("{e} ({:?})", e.kind()),
    }
}

fn names_in(dir: &Path) -> Vec<String> {
    match fs::read_dir(dir) {
        Ok(rd) => rd
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect(),
        Err(e) => vec![format!("<read_dir failed: {e}>")],
    }
}

fn text(path: &Path) -> io::Result<String> {
    fs::read(path).map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// Open a file read-only and collect its name queries.
fn handle_parity(path: &Path) -> Parity {
    let wide = names::wide(&path.display().to_string());
    let h = unsafe {
        CreateFileW(
            wide.as_ptr(),
            0x8000_0000,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return Parity::default();
    }
    let p = unsafe { names::parity(h) };
    unsafe { CloseHandle(h) };
    p
}

/// Write the name answers of a real file, asked of this process's own bitness and unhooked, to `out`
/// as a `Parity`: the reference the test compares the probe's answers with, for a 32-bit probe.
pub fn parity_of(file: &Path, out: &Path) -> i32 {
    let parity = handle_parity(file);
    match fs::write(out, serde_json::to_vec(&parity).unwrap()) {
        Ok(()) => 0,
        Err(_) => 2,
    }
}

/// Read a whole file opened by NT name, through the same hooks as every other open.
fn read_nt(name: &str) -> io::Result<String> {
    let file = NtOpen::file(None, name, false).map_err(|s| io::Error::other(format!("{s:#x}")))?;
    read_handle(file.handle())
}

/// Write `alias.json` into `out` and return the checks' verdict in the process exit code.
pub fn run(install: &Path, mount: &Path, upper: &Path, out: &Path, forms: &[Form]) -> i32 {
    let mut o = Output::default();
    let c = &mut o.checks;
    let install_data = install.join("Data");
    let mount_data = mount.join("Data");
    let upper_data = upper.join("Data");

    // The lower's bytes come through the alias, not the install's own copy.
    let r = text(&install_data.join("mod.esp"));
    check(
        c,
        "alias reads a lower's bytes for Data\\mod.esp",
        matches!(&r, Ok(s) if s == "LOWER mod.esp"),
        describe(&r),
    );
    // A file that is in the install and no layer is not found.
    let r = fs::read(install_data.join("real_only.txt"));
    check(
        c,
        "a real file in no layer is not found through the alias",
        matches!(&r, Err(e) if e.kind() == io::ErrorKind::NotFound),
        describe(&r),
    );
    // The listings: the same names in the same order, and no real-only name.
    let alias_list = names_in(&install_data);
    let mount_list = names_in(&mount_data);
    check(
        c,
        "the listing of Data through the alias equals the mount's, in order",
        alias_list == mount_list,
        format!("alias {alias_list:?} mount {mount_list:?}"),
    );
    check(
        c,
        "the real-only file is absent from the alias listing",
        !alias_list.iter().any(|n| n == "real_only.txt"),
        format!("{alias_list:?}"),
    );
    o.listing_alias = alias_list;
    o.listing_mount = mount_list;

    // Name queries on the lower's file, through the alias and through the mount.
    o.alias_lower = handle_parity(&install_data.join("mod.esp"));
    o.mount_lower = handle_parity(&mount_data.join("mod.esp"));

    // Every path form of the alias and the mount opens the same file.
    for form in forms {
        let r = read_nt(&format!("{}\\Data\\mod.esp", form.prefix));
        check(
            c,
            &format!("path form {}: reads the lower's bytes", form.name),
            matches!(&r, Ok(s) if s == "LOWER mod.esp"),
            describe(&r),
        );
    }

    // A relative open: the current directory is the alias's Data folder (the test sets it).
    let r = text(Path::new("lower_only.txt"));
    check(
        c,
        "a relative open from the alias's Data folder reads the lower-only file",
        matches!(&r, Ok(s) if s == "BASE lower_only"),
        describe(&r),
    );
    // A relative open from a directory handle opened through the alias.
    let relative = NtOpen::file(None, &format!("\\??\\{}", install_data.display()), true)
        .map_err(|s| format!("opening the folder: {s:#x}"))
        .and_then(|dir| {
            NtOpen::file(Some(dir.handle()), "lower_only.txt", false)
                .map_err(|s| format!("opening the file relative to it: {s:#x}"))
                .and_then(|file| read_handle(file.handle()).map_err(|e| e.to_string()))
        });
    check(
        c,
        "a relative NT open from a directory handle opened through the alias reads the lower-only file",
        matches!(&relative, Ok(s) if s == "BASE lower_only"),
        format!("{relative:?}"),
    );

    // Copy-up: an append lands in the upper layer; the lower and the install are unchanged.
    let append = fs::OpenOptions::new()
        .append(true)
        .open(install_data.join("mod.esp"))
        .and_then(|mut f| f.write_all(b"+APPEND"));
    check(
        c,
        "an append through the alias succeeds",
        append.is_ok(),
        describe(&append.map(|_| ())),
    );
    let up = text(&upper_data.join("mod.esp"));
    check(
        c,
        "the append copied the file up to the upper layer",
        matches!(&up, Ok(s) if s == "LOWER mod.esp+APPEND"),
        describe(&up),
    );
    // The lower and the install are checked by the test on the real files: through the view the
    // mount now shows the upper's copy, which is the point of copy-up.
    // The same queries once the handle resolves to the upper layer.
    o.alias_upper = handle_parity(&install_data.join("mod.esp"));

    // A new file lands in the upper layer, not the install.
    let created = fs::write(install_data.join("new.txt"), b"NEW");
    check(
        c,
        "creating a file through the alias succeeds",
        created.is_ok(),
        describe(&created),
    );
    let up = text(&upper_data.join("new.txt"));
    check(
        c,
        "the new file is in the upper layer",
        matches!(&up, Ok(s) if s == "NEW"),
        describe(&up),
    );

    // Rename within the alias, then from the alias to the mount.
    let renamed = fs::rename(install_data.join("new.txt"), install_data.join("new2.txt"));
    check(
        c,
        "a rename within the alias succeeds",
        renamed.is_ok()
            && upper_data.join("new2.txt").is_file()
            && !upper_data.join("new.txt").exists(),
        describe(&renamed),
    );
    let moved = fs::rename(install_data.join("new2.txt"), mount_data.join("moved.txt"));
    check(
        c,
        "a rename from an alias path to a mount path succeeds",
        moved.is_ok()
            && upper_data.join("moved.txt").is_file()
            && !upper_data.join("new2.txt").exists(),
        describe(&moved),
    );

    // Deleting a file the lower also provides writes a whiteout.
    let deleted = fs::remove_file(install_data.join("mod.esp"));
    check(
        c,
        "deleting a lower's file through the alias succeeds",
        deleted.is_ok(),
        describe(&deleted),
    );
    check(
        c,
        "the delete writes a whiteout in the upper layer",
        upper
            .join(".agvfs-wh")
            .join("Data")
            .join("mod.esp.wh")
            .is_file(),
        "",
    );
    let r = fs::read(install_data.join("mod.esp"));
    check(
        c,
        "the deleted file is then not found through the alias",
        matches!(&r, Err(e) if e.kind() == io::ErrorKind::NotFound),
        describe(&r),
    );
    let alias_after = names_in(&install_data);
    let mount_after = names_in(&mount_data);
    check(
        c,
        "after the changes, the alias listing still equals the mount's",
        alias_after == mount_after,
        format!("alias {alias_after:?} mount {mount_after:?}"),
    );

    // A child exe that exists only in a lower, started by its alias path. The child reads a
    // lower-only file by its alias path: it is itself under the VFS.
    let child_exe = install.join("Tools").join("child.exe");
    let child_out = out.join("child");
    fs::create_dir_all(&child_out).ok();
    let spawned = Command::new(&child_exe)
        .arg("--child-read")
        .arg(install_data.join("lower_only.txt"))
        .arg("--out")
        .arg(&child_out)
        .status();
    let child_ok = matches!(&spawned, Ok(s) if s.success());
    let child_read = fs::read(child_out.join("child.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
    let child_text = child_read
        .as_ref()
        .and_then(|v| v["read"].as_str().map(str::to_string));
    check(
        c,
        "a child started by its alias path runs and reads a lower-only file through the alias",
        child_ok && child_text.as_deref() == Some("BASE lower_only"),
        format!("status {spawned:?}, child said {child_text:?}"),
    );

    let bytes = serde_json::to_vec_pretty(&o).expect("serialize alias.json");
    let written = fs::write(out.join("alias.json"), bytes);
    if written.is_err() {
        return 3;
    }
    if o.checks.iter().all(|c| c.ok) {
        0
    } else {
        1
    }
}

/// Read all of an open handle as text.
fn read_handle(h: HANDLE) -> io::Result<String> {
    let mut out = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let mut got = 0u32;
        let ok = unsafe {
            ReadFile(
                h,
                chunk.as_mut_ptr(),
                chunk.len() as u32,
                &mut got,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 || got == 0 {
            break;
        }
        out.extend_from_slice(&chunk[..got as usize]);
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// The child's side: read the file it was given by path, and write what it read to `child.json`.
pub fn child_read(file: &Path, out: &Path) -> i32 {
    let read = fs::read(file).map(|b| String::from_utf8_lossy(&b).into_owned());
    let body = match &read {
        Ok(s) => serde_json::json!({ "read": s }),
        Err(e) => serde_json::json!({ "error": format!("{e} ({:?})", e.kind()) }),
    };
    let _ = fs::write(out.join("child.json"), body.to_string());
    if read.is_ok() {
        0
    } else {
        2
    }
}

/// A child started in a folder of the view: read each name relative to its current directory, and
/// report those reads and the directory it actually runs in, to `child.json`.
pub fn child_relative(names: &[String], out: &Path) -> i32 {
    let mut read = serde_json::Map::new();
    let mut code = 0;
    for name in names {
        match fs::read(name) {
            Ok(b) => {
                read.insert(
                    name.clone(),
                    serde_json::json!(String::from_utf8_lossy(&b).into_owned()),
                );
            }
            Err(e) => {
                read.insert(
                    name.clone(),
                    serde_json::json!(format!("{e} ({:?})", e.kind())),
                );
                code = 2;
            }
        }
    }
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let body = serde_json::json!({ "cwd": cwd, "read": read });
    let _ = fs::write(out.join("child.json"), body.to_string());
    code
}

/// Start `child` with `dir` as its current directory, running `--child-relative` with `names`, and
/// return its exit code. The CreateProcess of the parent is the call the DLL must translate.
pub fn spawn_child_in(child: &Path, dir: &Path, names: &[String], out: &Path) -> i32 {
    let status = Command::new(child)
        .current_dir(dir)
        .arg("--child-relative")
        .args(names)
        .arg("--out")
        .arg(out)
        .status();
    match status {
        Ok(s) => s.code().unwrap_or(4),
        Err(_) => 5,
    }
}
