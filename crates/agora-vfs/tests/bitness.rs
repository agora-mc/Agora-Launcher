//! The 32-bit and 64-bit parent and child cases (brief Tests): a child of the other bitness is
//! injected through the helper of its own bitness; a missing 32-bit DLL ends the 32-bit child and says
//! where it looked; a helper that hangs is ended after its timeout, and so is its child.
//!
//! Run with:
//!   cargo build -p agora-vfs
//!   cargo build -p agora-vfs --target i686-pc-windows-msvc
//!   cargo test -p agora-vfs --test bitness -- --ignored
//!
//! Every file is in a temp folder. The child reads `marker.txt` by its relative name in the mount: the
//! file is only in the base (a lower), so the read succeeds only through the view, which is what shows
//! the child is under the VFS.

#![cfg(windows)]

#[path = "support/bits.rs"]
mod bits;

use std::collections::BTreeMap;
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use bits::{artifacts, Artifacts, Bits};
use serde::Deserialize;
use serde_json::json;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::Threading::*;

static TEST_MUTEX: Mutex<()> = Mutex::new(());

/// What the probe's `--child-relative` writes: the child's directory and what it read by name.
#[derive(Deserialize)]
struct ChildReport {
    cwd: String,
    read: BTreeMap<String, String>,
}

struct Scenario {
    root: PathBuf,
    mount: PathBuf,
    upper: PathBuf,
    base: PathBuf,
    results: PathBuf,
    config: PathBuf,
    ready: String,
}

fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}

/// A temp view: `mount` over `base` (a lower that holds `marker.txt`), with `dll_x86` as the 32-bit DLL
/// the configuration names.
fn scenario(name: &str, dll_x86: &Path, arts: &Artifacts) -> Scenario {
    let root = std::env::temp_dir().join(format!("agora-bitness-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let sc = Scenario {
        mount: root.join("mount"),
        upper: root.join("upper"),
        base: root.join("base"),
        results: root.join("results"),
        config: root.join("results").join("agora_vfs.json"),
        ready: format!("Local\\agora-vfs-bitness-{name}-{}", std::process::id()),
        root,
    };
    for dir in [&sc.mount, &sc.upper, &sc.base, &sc.results] {
        fs::create_dir_all(dir).unwrap();
    }
    fs::write(sc.base.join("marker.txt"), b"BASE marker").unwrap();
    let config = json!({
        "version": 2,
        "mount": sc.mount.display().to_string(),
        "upper": sc.upper.display().to_string(),
        "lowers": [sc.mount.display().to_string(), sc.base.display().to_string()],
        "dll": arts.dll(Bits::X64).display().to_string(),
        "dll_x86": dll_x86.display().to_string(),
        "log": sc.results.join("agora_vfs.log").display().to_string(),
        "verbose": false,
        "ready_event": sc.ready,
    });
    fs::write(&sc.config, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    sc
}

/// The probe's `--spawn-in` arguments: start `child` in the mount, reading `marker.txt`.
fn spawn_args(sc: &Scenario, child: &Path) -> String {
    format!(
        "--spawn-in \"{}\" --child \"{}\" --names marker.txt --out \"{}\"",
        sc.mount.display(),
        child.display(),
        sc.results.display()
    )
}

/// Start `exe` suspended under the view of `sc`, inject the DLL of its bitness `parent`, resume it,
/// and wait for it. Returns its exit code.
fn run_under_view(parent: Bits, exe: &Path, args: &str, sc: &Scenario, arts: &Artifacts) -> u32 {
    let ready = unsafe { CreateEventW(std::ptr::null(), 1, 0, wide(&sc.ready).as_ptr()) };
    assert!(!ready.is_null(), "CreateEventW failed");
    std::env::set_var("AGORA_VFS_CONFIG", &sc.config);
    let mut cmd = wide(format!("\"{}\" {args}", exe.display()));
    let cwd = wide(&sc.results);
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
            cwd.as_ptr(),
            &si,
            &mut pi,
        );
        assert!(ok != 0, "CreateProcessW failed: {}", GetLastError());
        bits::inject(pi.hProcess, pi.dwProcessId, parent, &arts.dll(parent))
            .expect("inject the parent");
        ResumeThread(pi.hThread);
        let wait = WaitForSingleObject(ready, 10_000);
        assert_eq!(
            wait, WAIT_OBJECT_0,
            "agora_vfs.dll did not confirm its hooks in the parent"
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

fn log_of(sc: &Scenario) -> String {
    fs::read_to_string(sc.results.join("agora_vfs.log")).unwrap_or_default()
}

/// The child read the base's file through the view, from the mount.
fn assert_child_read_through_the_view(sc: &Scenario) {
    let report: ChildReport = serde_json::from_slice(
        &fs::read(sc.results.join("child.json")).expect("the child wrote child.json"),
    )
    .expect("parse child.json");
    assert!(
        report
            .cwd
            .eq_ignore_ascii_case(&sc.mount.display().to_string()),
        "the child's directory is {}",
        report.cwd
    );
    assert_eq!(
        report.read.get("marker.txt").map(String::as_str),
        Some("BASE marker"),
        "the child did not read through the view: {:?}",
        report.read
    );
}

#[test]
#[ignore = "injects agora_vfs.dll into a 64-bit parent and a 32-bit child: cargo build both targets, then run with --ignored"]
fn a_64_bit_parent_injects_a_32_bit_child_through_the_32_bit_helper() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let arts = artifacts();
    arts.assert_built();
    let sc = scenario("p64-c32", &arts.dll(Bits::X86), &arts);
    let args = spawn_args(&sc, &arts.probe(Bits::X86));

    let code = run_under_view(Bits::X64, &arts.probe(Bits::X64), &args, &sc, &arts);
    let log = log_of(&sc);
    println!("the VFS log:\n{log}");
    assert_eq!(code, 0, "the 32-bit child did not run to the end");
    assert_child_read_through_the_view(&sc);
    assert!(
        log.contains("injected child") && log.contains("helper of the program's own bitness"),
        "the log does not say the child was injected by a helper:\n{log}"
    );
    let _ = fs::remove_dir_all(&sc.root);
}

#[test]
#[ignore = "injects agora_vfs.dll into a 32-bit parent and a 64-bit child: cargo build both targets, then run with --ignored"]
fn a_32_bit_parent_injects_a_64_bit_child_through_the_64_bit_helper() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let arts = artifacts();
    arts.assert_built();
    let sc = scenario("p32-c64", &arts.dll(Bits::X86), &arts);
    let args = spawn_args(&sc, &arts.probe(Bits::X64));

    let code = run_under_view(Bits::X86, &arts.probe(Bits::X86), &args, &sc, &arts);
    let log = log_of(&sc);
    println!("the VFS log:\n{log}");
    assert_eq!(code, 0, "the 64-bit child did not run to the end");
    assert_child_read_through_the_view(&sc);
    assert!(
        log.contains("injected child") && log.contains("helper of the program's own bitness"),
        "the log does not say the child was injected by a helper:\n{log}"
    );
    let _ = fs::remove_dir_all(&sc.root);
}

#[test]
#[ignore = "starts a 64-bit parent under the VFS with no 32-bit DLL: cargo build both targets, then run with --ignored"]
fn a_32_bit_child_is_ended_and_named_when_the_x86_dll_is_missing() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let arts = artifacts();
    arts.assert_built();
    let missing = std::env::temp_dir()
        .join(format!("agora-bitness-missing-{}", std::process::id()))
        .join("x86")
        .join("agora_vfs.dll");
    let sc = scenario("missing", &missing, &arts);
    let args = spawn_args(&sc, &arts.probe(Bits::X86));

    let code = run_under_view(Bits::X64, &arts.probe(Bits::X64), &args, &sc, &arts);
    let log = log_of(&sc);
    println!("the VFS log:\n{log}");
    // The probe reports a child it could not start as exit code 5.
    assert_eq!(code, 5, "the child should not have started");
    assert!(
        !sc.results.join("child.json").exists(),
        "the 32-bit child ran without the VFS"
    );
    let path = missing.display().to_string();
    assert!(
        log.contains(&format!(
            "is a 32-bit program and the 32-bit VFS files were not found at {path}"
        )),
        "the log does not name the path:\n{log}"
    );
    assert!(
        log.contains("failed, so it is ended"),
        "the child was not ended:\n{log}"
    );
    let _ = fs::remove_dir_all(&sc.root);
}

#[test]
#[ignore = "starts a helper that hangs: cargo build both targets, then run with --ignored"]
fn a_helper_that_hangs_is_ended_after_its_timeout_and_its_child_with_it() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let arts = artifacts();
    arts.assert_built();
    // The sleeping program the crate builds beside its probe and helper.
    let sleep_exe = PathBuf::from(env!("CARGO_BIN_EXE_agora-vfs-sleeper"));
    let sc = scenario("hang", &arts.dll(Bits::X86), &arts);
    // A copy of the 32-bit DLL with a sleeping program beside it as its helper.
    let hang = sc.root.join("hang");
    fs::create_dir_all(&hang).unwrap();
    let dll = hang.join("agora_vfs.dll");
    fs::copy(arts.dll(Bits::X86), &dll).unwrap();
    fs::copy(&sleep_exe, hang.join("agora-vfs-helper.exe")).unwrap();
    let config_path = sc.config.clone();
    let mut config: serde_json::Value =
        serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["dll_x86"] = json!(dll.display().to_string());
    fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    let args = spawn_args(&sc, &arts.probe(Bits::X86));

    let started = Instant::now();
    let code = run_under_view(Bits::X64, &arts.probe(Bits::X64), &args, &sc, &arts);
    let elapsed = started.elapsed();
    let log = log_of(&sc);
    println!("the VFS log:\n{log}");
    assert_eq!(code, 5, "the child should not have started");
    assert!(
        elapsed >= Duration::from_secs(14) && elapsed < Duration::from_secs(120),
        "the parent took {elapsed:?}; the helper should be ended after about 15 seconds"
    );
    assert!(
        !sc.results.join("child.json").exists(),
        "the 32-bit child ran without the VFS"
    );
    assert!(
        log.contains("did not finish within 15 seconds and was ended"),
        "the log does not say the helper was ended:\n{log}"
    );
    let _ = fs::remove_dir_all(&sc.root);
}
