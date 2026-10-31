//! The real-DLL alias tests (brief decisions 1 to 6): a temp install folder is an alias, the probe
//! runs under the real `agora_vfs.dll` with it, and every answer is compared with the same question
//! asked of the real files without the DLL (this test process is not injected).
//!
//! Run with:
//!   cargo build -p agora-vfs
//!   cargo test -p agora-vfs --test alias -- --ignored
//!
//! Every file is in a temp folder; the temp install's whole tree must be unchanged at the end.

#![cfg(windows)]

#[path = "support/names.rs"]
mod names;

use std::collections::BTreeMap;
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use names::{Check, Parity};
use serde::{Deserialize, Serialize};
use serde_json::json;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, GetShortPathNameW, GetVolumeNameForVolumeMountPointW, QueryDosDeviceW,
    FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Threading::*;

#[path = "support/bits.rs"]
mod bits;
use bits::Bits;

static TEST_MUTEX: Mutex<()> = Mutex::new(());

/// One folder the probe opens the mod through, as an NT name.
#[derive(Serialize)]
struct Form {
    name: String,
    prefix: String,
}

/// What the probe writes (`alias.json`).
#[derive(Deserialize)]
struct Output {
    checks: Vec<Check>,
    listing_alias: Vec<String>,
    listing_mount: Vec<String>,
    alias_lower: Parity,
    mount_lower: Parity,
    alias_upper: Parity,
}

fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}

fn wide_until_nul(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

fn walk(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn go(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                go(root, &p, out);
            } else {
                let rel = p.strip_prefix(root).unwrap().display().to_string();
                out.insert(rel, fs::read(&p).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    go(root, root, &mut out);
    out
}

/// The path forms that name `folder`, computed here with the Windows APIs and not by the DLL:
/// the drive form, the volume GUID, the device, GLOBALROOT, and the 8.3 form of the folder when it
/// has one. A form that is not available is printed and skipped.
fn forms_for(label: &str, folder: &Path) -> Vec<Form> {
    let full = folder.display().to_string();
    let drive = &full[..2]; // "C:"
    let tail = &full[2..]; // "\...\folder"
    let mut forms = vec![Form {
        name: format!("{label} drive"),
        prefix: format!(r"\??\{full}"),
    }];
    let root = wide(format!("{drive}\\"));
    let mut guid = [0u16; 128];
    let ok = unsafe { GetVolumeNameForVolumeMountPointW(root.as_ptr(), guid.as_mut_ptr(), 128) };
    if ok != 0 {
        let s = wide_until_nul(&guid);
        let volume = s.trim_start_matches(r"\\?\").trim_end_matches('\\');
        forms.push(Form {
            name: format!("{label} volume"),
            prefix: format!(r"\??\{volume}{tail}"),
        });
    }
    let drive_wide = wide(drive);
    let mut device = vec![0u16; 1024];
    let n = unsafe { QueryDosDeviceW(drive_wide.as_ptr(), device.as_mut_ptr(), 1024) };
    let device_name = if n == 0 {
        None
    } else {
        Some(wide_until_nul(&device))
    };
    if let Some(dev) = &device_name {
        forms.push(Form {
            name: format!("{label} device"),
            prefix: format!(r"{dev}{tail}"),
        });
        forms.push(Form {
            name: format!("{label} globalroot"),
            prefix: format!(r"\??\GLOBALROOT{dev}{tail}"),
        });
    }
    let long = wide(&full);
    let mut short = vec![0u16; 1024];
    let n = unsafe { GetShortPathNameW(long.as_ptr(), short.as_mut_ptr(), 1024) };
    if n > 0 {
        let s = String::from_utf16_lossy(&short[..n as usize]);
        if !s.eq_ignore_ascii_case(&full) {
            forms.push(Form {
                name: format!("{label} 8.3"),
                prefix: format!(r"\??\{s}"),
            });
        } else {
            println!("{label}: 8.3 names are disabled here; the 8.3 form is skipped");
        }
    } else {
        println!("{label}: GetShortPathNameW failed; the 8.3 form is skipped");
    }
    forms
}

fn open_for_reference(path: &Path) -> HANDLE {
    let w = wide(path);
    let h = unsafe {
        CreateFileW(
            w.as_ptr(),
            0x8000_0000,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    assert!(h != INVALID_HANDLE_VALUE, "open {}", path.display());
    h
}

/// The unhooked answers for a real file, as a process of `bits` gets them: this process for 64 bits,
/// the 32-bit probe run unhooked for 32 bits (WOW64 answers in its own terms, so a 64-bit query is
/// not the reference for a 32-bit one).
fn reference(bits: Bits, probe: &Path, path: &Path) -> Parity {
    if bits == Bits::X64 {
        return reference_here(path);
    }
    let out =
        std::env::temp_dir().join(format!("agora-alias-reference-{}.json", std::process::id()));
    let status = std::process::Command::new(probe)
        .arg("--parity-of")
        .arg(path)
        .arg("--out")
        .arg(&out)
        .status()
        .expect("run the 32-bit probe for its reference");
    assert!(
        status.success(),
        "the 32-bit probe's reference failed: {status}"
    );
    let parity: Parity = serde_json::from_slice(&fs::read(&out).expect("the reference file"))
        .expect("parse reference");
    let _ = fs::remove_file(&out);
    parity
}

/// The unhooked answers for a real file in this process.
fn reference_here(path: &Path) -> Parity {
    let h = open_for_reference(path);
    let p = unsafe { names::parity(h) };
    unsafe { CloseHandle(h) };
    p
}

fn artifacts(bits: Bits) -> (PathBuf, PathBuf) {
    // The DLL and the probe of `bits`: in target/debug, or target/i686-pc-windows-msvc/debug.
    let arts = bits::artifacts();
    let dll = arts.dll(bits);
    let probe = arts.probe(bits);
    assert!(
        dll.exists(),
        "{} missing: cargo build -p agora-vfs",
        dll.display()
    );
    assert!(
        probe.exists(),
        "{} missing: cargo build -p agora-vfs",
        probe.display()
    );
    (dll, probe)
}

fn alias_scenario_shows_the_mount_and_leaves_the_install_unchanged(bits: Bits) {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let (dll, probe) = artifacts(bits);

    let sb = std::env::temp_dir().join(format!("agora-alias-{}", std::process::id()));
    let _ = fs::remove_dir_all(&sb);
    let install = sb.join("install");
    let mount = sb.join("mount");
    let upper = sb.join("upper");
    let base = sb.join("base");
    let results = sb.join("results");
    for d in [
        install.join("Data"),
        mount.join("Data"),
        upper.clone(),
        base.join("Data"),
        base.join("Tools"),
        results.clone(),
    ] {
        fs::create_dir_all(d).unwrap();
    }
    // The install's own Data: a real file no layer has, and its own copy of a file a lower provides.
    fs::write(install.join("Data").join("real_only.txt"), b"REAL ONLY").unwrap();
    fs::write(install.join("Data").join("mod.esp"), b"INSTALL mod.esp").unwrap();
    // The deployment (the mount) and a base below it.
    fs::write(mount.join("Data").join("mod.esp"), b"LOWER mod.esp").unwrap();
    fs::write(base.join("Data").join("lower_only.txt"), b"BASE lower_only").unwrap();
    // A program that exists only in the base. It is the probe itself, run as a child.
    fs::copy(&probe, base.join("Tools").join("child.exe")).unwrap();

    let forms: Vec<Form> = forms_for("alias", &install)
        .into_iter()
        .chain(forms_for("mount", &mount))
        .collect();
    let forms_file = results.join("forms.json");
    fs::write(&forms_file, serde_json::to_vec_pretty(&forms).unwrap()).unwrap();

    let before_install = walk(&install);
    let before_mount = walk(&mount);
    let before_base = walk(&base);

    let config = json!({
        "version": 2,
        "mount": mount.display().to_string(),
        "upper": upper.display().to_string(),
        "lowers": [mount.display().to_string(), base.display().to_string()],
        "dll": bits::artifacts().dll(Bits::X64).display().to_string(),
        "dll_x86": bits::artifacts().dll(Bits::X86).display().to_string(),
        "log": results.join("agora_vfs.log").display().to_string(),
        "verbose": false,
        "ready_event": format!("Local\\agora-vfs-alias-{}", std::process::id()),
        "aliases": [install.display().to_string()],
    });
    let config_path = results.join("agora_vfs.json");
    fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    let ready_name = config["ready_event"].as_str().unwrap().to_string();

    let ready = unsafe { CreateEventW(std::ptr::null(), 1, 0, wide(&ready_name).as_ptr()) };
    assert!(!ready.is_null(), "CreateEventW failed");

    std::env::set_var("AGORA_VFS_CONFIG", &config_path);
    let mut cmd = wide(format!(
        "\"{}\" --alias-scenario --install \"{}\" --mount \"{}\" --upper \"{}\" --out \"{}\" --forms \"{}\"",
        probe.display(),
        install.display(),
        mount.display(),
        upper.display(),
        results.display(),
        forms_file.display()
    ));
    let cwd = wide(install.join("Data"));
    let exit_code = unsafe {
        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let ok = CreateProcessW(
            std::ptr::null(),
            cmd.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            CREATE_SUSPENDED,
            std::ptr::null(),
            cwd.as_ptr(),
            &si,
            &mut pi,
        );
        assert!(ok != 0, "CreateProcessW failed: {}", GetLastError());
        bits::inject(pi.hProcess, pi.dwProcessId, bits, &dll).expect("inject agora_vfs.dll");
        ResumeThread(pi.hThread);
        let wait = WaitForSingleObject(ready, 10_000);
        assert_eq!(
            wait, WAIT_OBJECT_0,
            "agora_vfs.dll did not confirm its hooks"
        );
        WaitForSingleObject(pi.hProcess, 120_000);
        let mut code = 0u32;
        GetExitCodeProcess(pi.hProcess, &mut code);
        CloseHandle(pi.hProcess);
        CloseHandle(pi.hThread);
        CloseHandle(ready);
        code
    };
    std::env::remove_var("AGORA_VFS_CONFIG");

    let out: Output =
        serde_json::from_slice(&fs::read(results.join("alias.json")).unwrap_or_else(|e| {
            panic!("the probe wrote no alias.json (exit code {exit_code:#x}): {e}")
        }))
        .expect("parse alias.json");
    let failed: Vec<&Check> = out.checks.iter().filter(|c| !c.ok).collect();
    for c in &out.checks {
        println!(
            "{} {}: {}",
            if c.ok { "ok  " } else { "FAIL" },
            c.name,
            c.detail
        );
    }
    assert_eq!(exit_code, 0, "the probe's checks failed: {failed:?}");
    assert!(failed.is_empty(), "checks failed: {failed:?}");
    assert_eq!(
        out.listing_alias, out.listing_mount,
        "the listing through the alias differs from the mount's"
    );

    // Against the real files, unhooked: the same names, the same buffers, the same statuses.
    let real_install = reference(bits, &probe, &install.join("Data").join("mod.esp"));
    let real_mount = reference(bits, &probe, &mount.join("Data").join("mod.esp"));
    assert_parity("alias, the lower's file", &out.alias_lower, &real_install);
    assert_parity("alias, after copy-up", &out.alias_upper, &real_install);
    assert_parity("mount, the lower's file", &out.mount_lower, &real_mount);

    // The evidence: what both sides answered, for the report.
    println!("GetFinalPathNameByHandleW through the alias, after copy-up:");
    for g in &out.alias_upper.gfpn {
        println!("  flags {:#x}: {:?} (error {})", g.flags, g.path, g.error);
    }
    println!("GetFinalPathNameByHandleW of the real file, unhooked, for the same flags:");
    for g in &real_install.gfpn {
        println!("  flags {:#x}: {:?} (error {})", g.flags, g.path, g.error);
    }
    for (kind, lens) in [
        ("nq9", vec![0u32, 7, 8, 99, 100, 133, 134]),
        ("nq18", vec![103, 104, 105, 229, 230]),
        ("obj1", vec![15, 16, 63, 64, 65, 66, 193, 194]),
        ("gfie", vec![7, 8, 133, 134]),
    ] {
        for len in lens {
            let q = out
                .alias_upper
                .queries
                .iter()
                .find(|q| q.kind == kind && q.len == len)
                .expect("query recorded");
            println!(
                "  {kind} len {len:>3}: status {:#010x} info {:>3} error {:>3} (matches the real file)",
                q.status, q.info, q.error
            );
        }
    }

    // On the real files, unhooked: the lower still holds its own bytes after the copy-up, and the
    // install still holds its own copy after the delete (the delete was a whiteout, not a delete).
    assert_eq!(
        fs::read(mount.join("Data").join("mod.esp")).unwrap(),
        b"LOWER mod.esp",
        "the lower changed"
    );
    assert_eq!(
        fs::read(install.join("Data").join("mod.esp")).unwrap(),
        b"INSTALL mod.esp",
        "the install's own mod.esp changed"
    );
    assert!(
        !install.join("Data").join("new.txt").exists()
            && !install.join("Data").join("new2.txt").exists(),
        "a created file reached the install"
    );
    assert!(
        !mount.join("Data").join("moved.txt").exists(),
        "a file moved to the mount reached the lower"
    );

    // The temp install (and the mount and the base) are byte for byte as they were.
    assert_eq!(walk(&install), before_install, "the install changed");
    assert_eq!(walk(&mount), before_mount, "the mount changed");
    assert_eq!(walk(&base), before_base, "the base changed");

    let _ = fs::remove_dir_all(&sb);
}

/// Every answer the probe gave must equal the real file's. All differences are printed, so a
/// failure shows the whole pattern, not only its first case.
fn assert_parity(label: &str, probe: &Parity, real: &Parity) {
    let mut differences = Vec::new();
    for (got, want) in probe.gfpn.iter().zip(&real.gfpn) {
        if got != want {
            differences.push(format!(
                "{label}: GetFinalPathNameByHandleW {got:?} != {want:?}"
            ));
        }
    }
    if probe.gfpn.len() != real.gfpn.len() {
        differences.push(format!(
            "{label}: {} flags, not {}",
            probe.gfpn.len(),
            real.gfpn.len()
        ));
    }
    if probe.queries.len() != real.queries.len() {
        differences.push(format!(
            "{label}: {} queries, not {}",
            probe.queries.len(),
            real.queries.len()
        ));
    }
    for (got, want) in probe.queries.iter().zip(&real.queries) {
        if got != want {
            differences.push(format!(
                "{label}: {} len {}\n  probe {got:?}\n  real  {want:?}",
                want.kind, want.len
            ));
        }
    }
    let total = differences.len();
    for line in differences.iter().take(12) {
        println!("{line}");
    }
    assert!(
        total == 0,
        "{label}: {total} answers differ from the real file"
    );
}

/// What the relative-names child writes (`child.json`).
#[derive(Deserialize)]
struct ChildReport {
    cwd: String,
    read: BTreeMap<String, String>,
}

/// Start `probe` suspended with `args`, in folder `cwd`, under the DLL configured by `config_path`,
/// and wait for it. Returns the probe's exit code.
fn run_probe_under_view(
    bits: Bits,
    dll: &Path,
    probe: &Path,
    args: &str,
    cwd: &Path,
    config_path: &Path,
    ready_name: &str,
) -> u32 {
    let ready = unsafe { CreateEventW(std::ptr::null(), 1, 0, wide(ready_name).as_ptr()) };
    assert!(!ready.is_null(), "CreateEventW failed");
    std::env::set_var("AGORA_VFS_CONFIG", config_path);
    let mut cmd = wide(format!("\"{}\" {args}", probe.display()));
    let cwd_wide = wide(cwd);
    let code = unsafe {
        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let ok = CreateProcessW(
            std::ptr::null(),
            cmd.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            CREATE_SUSPENDED,
            std::ptr::null(),
            cwd_wide.as_ptr(),
            &si,
            &mut pi,
        );
        assert!(ok != 0, "CreateProcessW failed: {}", GetLastError());
        bits::inject(pi.hProcess, pi.dwProcessId, bits, dll).expect("inject agora_vfs.dll");
        ResumeThread(pi.hThread);
        let wait = WaitForSingleObject(ready, 10_000);
        assert_eq!(
            wait, WAIT_OBJECT_0,
            "agora_vfs.dll did not confirm its hooks"
        );
        WaitForSingleObject(pi.hProcess, 120_000);
        let mut code = 0u32;
        GetExitCodeProcess(pi.hProcess, &mut code);
        CloseHandle(pi.hProcess);
        CloseHandle(pi.hThread);
        CloseHandle(ready);
        code
    };
    std::env::remove_var("AGORA_VFS_CONFIG");
    code
}

fn cwd_scenario_reads_both_layers_by_relative_names(bits: Bits) {
    // The upper holds one file of Data\sub, the mount (a lower) holds another. A child started with
    // current directory alias\Data\sub must read both by relative name: its directory is translated
    // to the mount's own copy of the folder, so its relative opens go through the view.
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let (dll, probe) = artifacts(bits);

    let sb = std::env::temp_dir().join(format!("agora-cwd-{}", std::process::id()));
    let _ = fs::remove_dir_all(&sb);
    let install = sb.join("install");
    let mount = sb.join("mount");
    let upper = sb.join("upper");
    let base = sb.join("base");
    let results = sb.join("results");
    for d in [
        install.join("Data"),
        mount.join("Data").join("sub"),
        upper.join("Data").join("sub"),
        base.join("Tools"),
        results.clone(),
    ] {
        fs::create_dir_all(d).unwrap();
    }
    fs::write(
        mount.join("Data").join("sub").join("from_mount.txt"),
        b"FROM MOUNT",
    )
    .unwrap();
    fs::write(
        upper.join("Data").join("sub").join("from_upper.txt"),
        b"FROM UPPER",
    )
    .unwrap();
    fs::copy(&probe, base.join("Tools").join("child.exe")).unwrap();

    let config = json!({
        "version": 2,
        "mount": mount.display().to_string(),
        "upper": upper.display().to_string(),
        "lowers": [mount.display().to_string(), base.display().to_string()],
        "dll": bits::artifacts().dll(Bits::X64).display().to_string(),
        "dll_x86": bits::artifacts().dll(Bits::X86).display().to_string(),
        "log": results.join("agora_vfs.log").display().to_string(),
        "verbose": false,
        "ready_event": format!("Local\\agora-vfs-cwd-{}", std::process::id()),
        "aliases": [install.display().to_string()],
    });
    let config_path = results.join("agora_vfs.json");
    fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    let ready_name = config["ready_event"].as_str().unwrap().to_string();

    let folder = install.join("Data").join("sub");
    let child = install.join("Tools").join("child.exe");
    let args = format!(
        "--spawn-in \"{}\" --child \"{}\" --names from_mount.txt from_upper.txt --out \"{}\"",
        folder.display(),
        child.display(),
        results.display()
    );
    let code = run_probe_under_view(
        bits,
        &dll,
        &probe,
        &args,
        &results,
        &config_path,
        &ready_name,
    );
    let log = fs::read_to_string(results.join("agora_vfs.log")).unwrap_or_default();
    println!("the VFS log:\n{log}");
    assert_eq!(code, 0, "the child did not run and read both files");

    let report: ChildReport = serde_json::from_slice(
        &fs::read(results.join("child.json")).expect("the child wrote child.json"),
    )
    .expect("parse child.json");
    println!("the child's directory: {}", report.cwd);
    let mount_folder = mount.join("Data").join("sub");
    assert!(
        report
            .cwd
            .eq_ignore_ascii_case(&mount_folder.display().to_string()),
        "the child's directory is {}, not the mount's copy {}",
        report.cwd,
        mount_folder.display()
    );
    assert_eq!(
        report.read.get("from_mount.txt").map(String::as_str),
        Some("FROM MOUNT"),
        "{:?}",
        report.read
    );
    assert_eq!(
        report.read.get("from_upper.txt").map(String::as_str),
        Some("FROM UPPER"),
        "{:?}",
        report.read
    );

    let _ = fs::remove_dir_all(&sb);
}

#[test]
#[ignore = "injects agora_vfs.dll: cargo build -p agora-vfs, then run with --ignored"]
fn an_alias_shows_the_mount_and_leaves_the_real_install_unchanged() {
    alias_scenario_shows_the_mount_and_leaves_the_install_unchanged(Bits::X64);
}

#[test]
#[ignore = "injects agora_vfs.dll: cargo build -p agora-vfs --target i686-pc-windows-msvc, then run with --ignored"]
fn an_alias_shows_the_mount_and_leaves_the_real_install_unchanged_32_bit() {
    alias_scenario_shows_the_mount_and_leaves_the_install_unchanged(Bits::X86);
}

#[test]
#[ignore = "injects agora_vfs.dll: cargo build -p agora-vfs, then run with --ignored"]
fn a_child_started_in_an_alias_folder_reads_both_layers_by_relative_names() {
    cwd_scenario_reads_both_layers_by_relative_names(Bits::X64);
}

#[test]
#[ignore = "injects agora_vfs.dll: cargo build -p agora-vfs --target i686-pc-windows-msvc, then run with --ignored"]
fn a_child_started_in_an_alias_folder_reads_both_layers_by_relative_names_32_bit() {
    cwd_scenario_reads_both_layers_by_relative_names(Bits::X86);
}
