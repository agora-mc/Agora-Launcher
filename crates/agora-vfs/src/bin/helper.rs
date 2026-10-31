//! `agora-vfs-helper --pid <pid> --dll <path>`: injects `agora_vfs.dll` into a suspended process of
//! its own bitness (MASTER_SPEC §26.5, brief decision 2).
//!
//! The launcher or a hooked game starts this helper when the target is of the other bitness, because
//! a process cannot inject a DLL into a process of another bitness. The injection is the same one the
//! DLL does for a same-bitness child (`agora_vfs_inject::inject_child`). The helper never resumes the
//! target: whoever started the target does that.
//!
//! Exit code 0 means the DLL is in the target's import table or was loaded by a remote thread. On
//! failure it exits 1 and writes one line to stderr saying why.

#![cfg(windows)]

use std::path::PathBuf;

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError};
use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_ALL_ACCESS};

const USAGE: &str = "usage: agora-vfs-helper --pid <pid> --dll <path>";

fn main() {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => std::process::exit(0),
        Err(why) => {
            eprintln!("agora-vfs-helper: {why}");
            std::process::exit(1);
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let mut pid = None;
    let mut dll = None;
    let mut rest = args.into_iter();
    while let Some(flag) = rest.next() {
        let value = rest.next();
        match (flag.as_str(), value) {
            ("--pid", Some(value)) => {
                pid = Some(
                    value
                        .parse::<u32>()
                        .map_err(|_| format!("'{value}' is not a process id"))?,
                )
            }
            ("--dll", Some(value)) => dll = Some(PathBuf::from(value)),
            _ => return Err(USAGE.to_string()),
        }
    }
    let (Some(pid), Some(dll)) = (pid, dll) else {
        return Err(USAGE.to_string());
    };
    if !dll.is_file() {
        return Err(format!("agora_vfs.dll was not found at {}", dll.display()));
    }
    // SAFETY: the handle is closed below, once, and the injection uses only that handle.
    unsafe {
        let process = OpenProcess(PROCESS_ALL_ACCESS, 0, pid);
        if process.is_null() {
            return Err(format!(
                "could not open process {pid} (error {})",
                GetLastError()
            ));
        }
        let result = agora_vfs_inject::inject_child(process, &dll);
        CloseHandle(process);
        result.map(|_| ())
    }
}
