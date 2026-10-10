//! Conformance tests for agora-vfs based on the Spike 2 write matrix.
//!
//! Run with:
//! cargo build -p agora-vfs && cargo test -p agora-vfs -- --ignored

#![cfg(windows)]

use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};
use std::sync::Mutex;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::Threading::*;

static TEST_MUTEX: Mutex<()> = Mutex::new(());

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

fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}

fn last_error() -> u32 {
    unsafe { GetLastError() }
}

fn file_name(prefix: &str, layer: &str, op: &str) -> String {
    format!("{prefix}{layer}__{op}.txt")
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

fn setup(sb: &Path, readonly: bool, acl: bool) {
    if sb.exists() {
        let _ = Command::new("icacls")
            .arg(sb)
            .args(["/reset", "/T", "/Q"])
            .status();
        clear_readonly(sb);
        fs::remove_dir_all(sb).unwrap();
    }
    for d in [
        "store/Data",
        "base/Data",
        "content/modA/Data",
        "overwrite",
        "mount",
        "results",
    ] {
        fs::create_dir_all(sb.join(d)).unwrap();
    }
    for prefix in ["", "child_"] {
        for op in OPS {
            for layer in LAYERS {
                let name = file_name(prefix, layer, op);
                let orig = format!("ORIG {layer} {op}");
                match *layer {
                    "content" => {
                        fs::write(sb.join("content/modA/Data").join(&name), &orig).unwrap()
                    }
                    "base_copied" => {
                        fs::write(sb.join("store/Data").join(&name), &orig).unwrap();
                        fs::write(sb.join("base/Data").join(&name), &orig).unwrap();
                    }
                    "base_linked" => {
                        fs::write(sb.join("store/Data").join(&name), &orig).unwrap();
                        fs::hard_link(
                            sb.join("store/Data").join(&name),
                            sb.join("base/Data").join(&name),
                        )
                        .unwrap();
                    }
                    _ => unreachable!(),
                }
            }
        }
    }
    if acl {
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
            .arg("WriteData,AppendData,WriteExtendedAttributes,Delete")
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

fn state(p: &Path, orig: &str) -> String {
    match fs::read(p) {
        Ok(b) if b == orig.as_bytes() => "orig".into(),
        Ok(b) => format!("CHANGED:{}", String::from_utf8_lossy(&b)),
        Err(_) => "GONE".into(),
    }
}

fn rel_str(sb: &Path, p: &Path) -> String {
    p.strip_prefix(sb)
        .unwrap_or(p)
        .display()
        .to_string()
        .replace('\\', "/")
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n - 1).collect::<String>() + "…"
    }
}

unsafe fn inject(process: HANDLE, dll: &Path) -> Result<(), String> {
    use windows_sys::Win32::System::Diagnostics::Debug::WriteProcessMemory;
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    use windows_sys::Win32::System::Memory::{
        VirtualAllocEx, MEM_COMMIT, MEM_RESERVE, PAGE_READWRITE,
    };
    let w = wide(dll);
    let bytes = w.len() * 2;
    let remote = VirtualAllocEx(
        process,
        std::ptr::null(),
        bytes,
        MEM_COMMIT | MEM_RESERVE,
        PAGE_READWRITE,
    );
    if remote.is_null() {
        return Err(format!("VirtualAllocEx {}", last_error()));
    }
    if WriteProcessMemory(
        process,
        remote,
        w.as_ptr() as *const _,
        bytes,
        std::ptr::null_mut(),
    ) == 0
    {
        return Err(format!("WriteProcessMemory {}", last_error()));
    }
    let load = GetProcAddress(
        GetModuleHandleW(wide("kernel32.dll").as_ptr()),
        c"LoadLibraryW".as_ptr() as *const u8,
    )
    .ok_or("no LoadLibraryW")?;
    let thread = CreateRemoteThread(
        process,
        std::ptr::null(),
        0,
        Some(std::mem::transmute::<
            unsafe extern "system" fn() -> isize,
            unsafe extern "system" fn(*mut std::ffi::c_void) -> u32,
        >(load)),
        remote,
        0,
        std::ptr::null_mut(),
    );
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

fn find_built_artifacts() -> (PathBuf, PathBuf) {
    // Current test exe is in target/debug/deps/...
    let mut test_exe = std::env::current_exe().unwrap();
    // Navigate up to target/debug or target/release
    test_exe.pop(); // pop exe
    if test_exe.file_name() == Some(std::ffi::OsStr::new("deps")) {
        test_exe.pop(); // pop deps
    }
    let target_dir = test_exe;
    let dll = target_dir.join("agora_vfs.dll");
    let probe = target_dir.join("agora-vfs-probe.exe");
    assert!(
        dll.exists(),
        "agora_vfs.dll not found at {}. Run `cargo build -p agora-vfs` first.",
        dll.display()
    );
    assert!(
        probe.exists(),
        "agora-vfs-probe.exe not found at {}. Run `cargo build -p agora-vfs` first.",
        probe.display()
    );
    (dll, probe)
}

struct MatrixResult {
    consistent: bool,
    total_ops: usize,
    leaks: usize,
    failures: usize,
    openable_count: usize,
    listed_count: usize,
    attr_count: usize,
}

fn run_matrix_case(sb: &Path, topology: &str, readonly: bool, acl: bool) -> MatrixResult {
    let _lock = TEST_MUTEX.lock().unwrap();
    let (dll, probe) = find_built_artifacts();
    setup(sb, readonly, acl);
    let results = sb.join("results");

    let (mount, lowers) = match topology {
        "full" => {
            // Empty mount over separate lower folders
            (
                sb.join("mount"),
                vec![sb.join("content/modA"), sb.join("base")],
            )
        }
        "farm" => {
            // Exactly what Agora deploys: a folder of hardlinks to the base and the mod (the mod
            // wins), mounted over itself. A write that slipped through would reach the originals
            // through their links, so the leak check below sees it.
            let farm = sb.join("farm");
            for layer in [sb.join("content/modA"), sb.join("base")] {
                for file in walk(&layer) {
                    let target = farm.join(file.strip_prefix(&layer).unwrap());
                    if !target.exists() {
                        fs::create_dir_all(target.parent().unwrap()).unwrap();
                        fs::hard_link(&file, &target).expect("hardlink into the farm");
                    }
                }
            }
            (farm.clone(), vec![farm])
        }
        "mount_is_lower" => {
            // Product topology: mount == base (lowers[0])
            (
                sb.join("base"),
                vec![sb.join("base"), sb.join("content/modA")],
            )
        }
        _ => panic!("unknown topology {topology}"),
    };

    let ready_event_name = format!("Local\\agora-vfs-test-{}", std::process::id());
    let ready_event = unsafe {
        CreateEventW(
            std::ptr::null(),
            1, // manual reset
            0, // initially non-signaled
            wide(&ready_event_name).as_ptr(),
        )
    };
    assert!(
        !ready_event.is_null() && ready_event != INVALID_HANDLE_VALUE,
        "CreateEventW failed"
    );

    let config_json = json!({
        "version": 1,
        "mount": mount.display().to_string(),
        "upper": sb.join("overwrite").display().to_string(),
        "lowers": lowers.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
        "dll": dll.display().to_string(),
        "log": results.join("agora_vfs.log").display().to_string(),
        "verbose": true,
        "ready_event": ready_event_name
    });

    let cfg_path = results.join("agora_vfs.json");
    fs::write(&cfg_path, serde_json::to_vec_pretty(&config_json).unwrap()).unwrap();
    std::env::set_var("AGORA_VFS_CONFIG", &cfg_path);

    let mut cmd = wide(format!(
        "\"{}\" --root \"{}\" --out \"{}\"",
        probe.display(),
        mount.display(),
        results.display()
    ));

    unsafe {
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
            std::ptr::null(),
            &si,
            &mut pi,
        );
        assert!(ok != 0, "CreateProcessW failed: {}", last_error());
        inject(pi.hProcess, &dll).expect("inject agora_vfs");

        // Resume and wait for ready_event handshake
        ResumeThread(pi.hThread);
        let wait_res = WaitForSingleObject(ready_event, 10_000);
        assert_eq!(
            wait_res, WAIT_OBJECT_0,
            "ready_event handshake was not signaled by agora_vfs.dll"
        );

        WaitForSingleObject(pi.hProcess, 120_000);
        let mut code = 0u32;
        GetExitCodeProcess(pi.hProcess, &mut code);
        println!("probe exit code {code}");
        CloseHandle(pi.hProcess);
        CloseHandle(pi.hThread);
        CloseHandle(ready_event);
    }
    std::env::remove_var("AGORA_VFS_CONFIG");

    // Analyze results
    println!("\n== topology {topology}, readonly {readonly}, acl {acl} ==");
    println!(
        "{:<6} {:<12} {:<24} {:<5} {:<26} {:<22} {:<10}",
        "who", "layer", "op", "ok", "game sees", "real lower file", "store"
    );

    let mut rows = Vec::new();
    let mut leak_count = 0;
    let mut failure_count = 0;

    for (who, prefix, file) in [
        ("direct", "", "direct.json"),
        ("child", "child_", "child.json"),
    ] {
        let probe_out: Vec<Value> = match fs::read(results.join(file)) {
            Ok(b) => serde_json::from_slice(&b).unwrap(),
            Err(e) => {
                println!("{who}: no results ({e})");
                continue;
            }
        };
        for r in probe_out {
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
                let store = if layer == "base_linked" {
                    state(&sb.join("store/Data").join(&name), &orig)
                } else {
                    "-".into()
                };
                (state(&lower_path, &orig), store)
            };
            let is_ok = r["ok"].as_bool().unwrap();
            if !is_ok {
                failure_count += 1;
            }
            if (lower != "orig" && lower != "-") || (store != "orig" && store != "-") {
                leak_count += 1;
            }
            let ok_str = if is_ok {
                "yes".to_string()
            } else {
                format!("e{}", r["error"])
            };
            let mut seen = r["visible"].as_str().unwrap().to_string();
            if let Some(m) = r.get("visible_moved").and_then(|v| v.as_str()) {
                seen = format!("{seen} / moved:{m}");
            }
            println!(
                "{who:<6} {layer:<12} {op:<24} {ok_str:<5} {:<26} {:<22} {:<10}",
                trunc(&seen, 26),
                trunc(&lower, 22),
                trunc(&store, 10)
            );
            rows.push(
                json!({"who": who, "layer": layer, "op": op, "ok": r["ok"], "error": r["error"],
                "game_sees": seen, "lower": lower, "store": store}),
            );
        }
    }

    let overwrite: Vec<String> = walk(&sb.join("overwrite"))
        .iter()
        .map(|p| rel_str(sb, p))
        .collect();
    println!(
        "\noverwrite/ holds {} files: {:?}",
        overwrite.len(),
        overwrite
    );
    println!(
        "child status: {}",
        fs::read_to_string(results.join("child-status.txt")).unwrap_or_default()
    );

    let mut consistent = false;
    let mut openable_count = 0;
    let mut listed_count = 0;
    let mut attr_count = 0;

    if let Ok(b) = fs::read(results.join("listing.json")) {
        let listed: Vec<String> = serde_json::from_slice(&b).unwrap();
        let mut expected = std::collections::BTreeSet::new();
        for (prefix, file) in [("", "direct.json"), ("child_", "child.json")] {
            let Ok(b) = fs::read(results.join(file)) else {
                continue;
            };
            let p_rows: Vec<Value> = serde_json::from_slice(&b).unwrap();
            for r in p_rows {
                let (layer, op) = (r["layer"].as_str().unwrap(), r["op"].as_str().unwrap());
                let name = if layer == "none" {
                    format!("{prefix}created.txt")
                } else {
                    file_name(prefix, layer, op)
                };
                if r["visible"].as_str() != Some("<absent>") {
                    expected.insert(name.clone());
                }
                if r.get("visible_moved")
                    .and_then(|v| v.as_str())
                    .is_some_and(|v| v != "<absent>")
                {
                    expected.insert(name.replace(".txt", ".moved"));
                }
            }
        }
        let listed_set: std::collections::BTreeSet<String> = listed.iter().cloned().collect();
        let missing: Vec<_> = expected.difference(&listed_set).collect();
        let extra: Vec<_> = listed_set.difference(&expected).collect();
        openable_count = expected.len();
        listed_count = listed_set.len();
        consistent = missing.is_empty() && extra.is_empty() && listed.len() == listed_set.len();
        println!(
            "listing: {} names ({} duplicates); openable but not listed: {:?}; listed but not openable: {:?}",
            listed.len(),
            listed.len() - listed_set.len(),
            missing,
            extra
        );

        if let Ok(b) = fs::read(results.join("attrs.json")) {
            let a: Value = serde_json::from_slice(&b).unwrap();
            let found: std::collections::BTreeSet<String> = a["found"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect();
            attr_count = found.len();
            consistent = consistent
                && a["data"].as_bool() == Some(true)
                && expected.difference(&found).next().is_none()
                && found.difference(&expected).next().is_none();
            println!(
                "attributes: Data folder {}; openable but no attributes: {:?}; attributes but not openable: {:?}",
                if a["data"].as_bool() == Some(true) { "found" } else { "MISSING" },
                expected.difference(&found).collect::<Vec<_>>(),
                found.difference(&expected).collect::<Vec<_>>()
            );
        }
    }

    MatrixResult {
        consistent,
        total_ops: rows.len(),
        leaks: leak_count,
        failures: failure_count,
        openable_count,
        listed_count,
        attr_count,
    }
}

#[test]
#[ignore]
fn test_matrix_topology_full_plain() {
    let sb = std::env::temp_dir().join(format!("agora-vfs-test-full-plain-{}", std::process::id()));
    let res = run_matrix_case(&sb, "full", false, false);
    assert_eq!(res.total_ops, 62);
    assert_eq!(res.leaks, 0, "Lower/store files changed!");
    assert_eq!(res.failures, 0, "Operations failed for probe!");
    assert!(res.consistent, "open, list and attributes disagree");
    assert_eq!(res.openable_count, res.listed_count);
    assert_eq!(res.openable_count, res.attr_count);
}

#[test]
#[ignore]
fn test_matrix_topology_full_acl() {
    let sb = std::env::temp_dir().join(format!("agora-vfs-test-full-acl-{}", std::process::id()));
    let res = run_matrix_case(&sb, "full", false, true);
    assert_eq!(res.total_ops, 62);
    assert_eq!(res.leaks, 0, "Lower/store files changed!");
    assert_eq!(res.failures, 0, "Operations failed for probe!");
    assert!(res.consistent, "open, list and attributes disagree");
    assert_eq!(res.openable_count, res.listed_count);
    assert_eq!(res.openable_count, res.attr_count);
}

#[test]
#[ignore]
fn test_matrix_topology_mount_is_lower_plain() {
    let sb = std::env::temp_dir().join(format!(
        "agora-vfs-test-mountlower-plain-{}",
        std::process::id()
    ));
    let res = run_matrix_case(&sb, "mount_is_lower", false, false);
    assert_eq!(res.total_ops, 62);
    assert_eq!(res.leaks, 0, "Lower/store files changed!");
    assert_eq!(res.failures, 0, "Operations failed for probe!");
    assert!(res.consistent, "open, list and attributes disagree");
    assert_eq!(res.openable_count, res.listed_count);
    assert_eq!(res.openable_count, res.attr_count);
}

#[test]
#[ignore]
fn test_matrix_topology_mount_is_lower_acl() {
    let sb = std::env::temp_dir().join(format!(
        "agora-vfs-test-mountlower-acl-{}",
        std::process::id()
    ));
    let res = run_matrix_case(&sb, "mount_is_lower", false, true);
    assert_eq!(res.total_ops, 62);
    assert_eq!(res.leaks, 0, "Lower/store files changed!");
    assert_eq!(res.failures, 0, "Operations failed for probe!");
    assert!(res.consistent, "open, list and attributes disagree");
    assert_eq!(res.openable_count, res.listed_count);
    assert_eq!(res.openable_count, res.attr_count);
}

#[test]
#[ignore]
fn test_matrix_topology_farm_plain() {
    let sb = std::env::temp_dir().join(format!("agora-vfs-test-farm-plain-{}", std::process::id()));
    let res = run_matrix_case(&sb, "farm", false, false);
    assert_eq!(res.total_ops, 62);
    assert_eq!(res.leaks, 0, "Lower/store files changed!");
    assert_eq!(res.failures, 0, "Operations failed for probe!");
    assert!(res.consistent, "open, list and attributes disagree");
}

#[test]
#[ignore]
fn test_matrix_topology_farm_acl() {
    let sb = std::env::temp_dir().join(format!("agora-vfs-test-farm-acl-{}", std::process::id()));
    let res = run_matrix_case(&sb, "farm", false, true);
    assert_eq!(res.total_ops, 62);
    assert_eq!(res.leaks, 0, "Lower/store files changed!");
    assert_eq!(res.failures, 0, "Operations failed for probe!");
    assert!(res.consistent, "open, list and attributes disagree");
}
