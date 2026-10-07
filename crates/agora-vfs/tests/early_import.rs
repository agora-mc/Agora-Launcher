//! What happens when `agora_vfs.dll` is the first import of a process and cannot do its job.
//!
//! Run with:
//! cargo build -p agora-vfs -p agora-vfs-early-import && cargo test -p agora-vfs -- --ignored
//!
//! The fixture program is `agora-early-import-exe.exe` from `crates/agora-vfs/fixtures/early-import`;
//! like `agora_vfs.dll` it is found in the target folder this test runs from. The end-to-end
//! ordering test (a static import's `DllMain` writing into a deployed file before any hook could
//! exist) is `real_injection_loads_the_vfs_before_the_games_own_imports` in
//! `crates/agora-core/tests/game_deploy.rs`.

#![cfg(windows)]

use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
use windows_sys::Win32::System::Threading::CREATE_SUSPENDED;

/// What `agora_vfs.dll` ends a process it cannot protect with (`DllMain`).
const EXIT_UNPROTECTED: i32 = 0xA6F5_0001_u32 as i32;

fn artifacts() -> (PathBuf, PathBuf) {
    let mut dir = std::env::current_exe().unwrap();
    dir.pop();
    if dir.file_name() == Some(std::ffi::OsStr::new("deps")) {
        dir.pop();
    }
    let dll = dir.join("agora_vfs.dll");
    let exe = dir.join("agora-early-import-exe.exe");
    let dll_beside_exe = dir.join("agora_early_import.dll");
    for file in [&dll, &exe, &dll_beside_exe] {
        assert!(
            file.exists(),
            "{} not found. Run `cargo build -p agora-vfs -p agora-vfs-early-import` first.",
            file.display()
        );
    }
    (dll, exe)
}

unsafe fn resume(process: HANDLE) {
    type NtResumeProcess = unsafe extern "system" fn(HANDLE) -> i32;
    let ntdll = GetModuleHandleA(c"ntdll.dll".as_ptr() as *const u8);
    let proc = GetProcAddress(ntdll, c"NtResumeProcess".as_ptr() as *const u8).unwrap();
    let nt_resume_process: NtResumeProcess = std::mem::transmute(proc);
    let status = nt_resume_process(process);
    assert!(status >= 0, "NtResumeProcess {status:#x}");
}

fn wait_for_exit(child: &mut Child, limit: Duration) -> Option<i32> {
    let start = Instant::now();
    while start.elapsed() < limit {
        if let Some(status) = child.try_wait().unwrap() {
            return status.code();
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    None
}

/// Start the fixture suspended, make the DLL its first import, and let it run.
fn run_with_dll_as_first_import(config: Option<&str>) -> Option<i32> {
    let (dll, exe) = artifacts();
    let mut cmd = Command::new(exe);
    cmd.env_remove("AGORA_VFS_CONFIG");
    if let Some(config) = config {
        cmd.env("AGORA_VFS_CONFIG", config);
    }
    cmd.creation_flags(CREATE_SUSPENDED);
    let mut child = cmd.spawn().unwrap();
    // SAFETY: the handle is the live one `child` owns, and the process is still suspended.
    unsafe {
        agora_vfs_inject::inject_import_table(child.as_raw_handle() as HANDLE, &dll)
            .expect("import-table injection");
        resume(child.as_raw_handle() as HANDLE);
    }
    let code = wait_for_exit(&mut child, Duration::from_secs(10));
    if code.is_none() {
        let _ = child.kill();
    }
    code
}

/// A configuration that is named but cannot be read: the process was meant to run under the VFS
/// and cannot, so the DLL ends it before the program's own code runs, instead of letting it run
/// unprotected until a launcher's timeout.
#[test]
#[ignore = "starts a suspended process under agora_vfs.dll"]
fn an_unreadable_configuration_ends_the_process_before_it_runs() {
    let code = run_with_dll_as_first_import(Some(r"C:\no\such\agora-vfs-config.json"));
    assert_eq!(code, Some(EXIT_UNPROTECTED), "the process was not ended");
}

/// No configuration at all (a child started with its own environment block): nothing to
/// enforce, so the DLL installs nothing and the program runs normally.
#[test]
#[ignore = "starts a suspended process under agora_vfs.dll"]
fn no_configuration_installs_nothing_and_the_program_runs() {
    assert_eq!(run_with_dll_as_first_import(None), Some(0));
}
