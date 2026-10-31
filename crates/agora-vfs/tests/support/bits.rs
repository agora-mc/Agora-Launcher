//! The 64-bit and 32-bit artifacts of this crate, and how a suspended process of each bitness gets
//! `agora_vfs.dll` in these tests. The harness is always the 64-bit test binary: a 32-bit process is
//! injected through the 32-bit helper beside the 32-bit DLL, as the launcher does (brief decision 4).
//!
//! Build both before running the tests that use this module:
//!   cargo build -p agora-vfs
//!   cargo build -p agora-vfs --target i686-pc-windows-msvc

#![cfg(windows)]
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use windows_sys::Win32::Foundation::HANDLE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bits {
    X64,
    X86,
}

/// The two target folders: `target\debug` for 64-bit, `target\i686-pc-windows-msvc\debug` for 32-bit.
/// Found from this test executable, which is in `target\debug\deps`.
pub struct Artifacts {
    x64_dir: PathBuf,
    x86_dir: PathBuf,
}

pub fn artifacts() -> Artifacts {
    let mut dir = std::env::current_exe().unwrap();
    dir.pop();
    if dir.file_name() == Some(std::ffi::OsStr::new("deps")) {
        dir.pop();
    }
    let target = dir.parent().unwrap().to_path_buf();
    Artifacts {
        x64_dir: dir,
        x86_dir: target.join("i686-pc-windows-msvc").join("debug"),
    }
}

impl Artifacts {
    fn dir(&self, bits: Bits) -> &Path {
        match bits {
            Bits::X64 => &self.x64_dir,
            Bits::X86 => &self.x86_dir,
        }
    }

    /// `agora_vfs.dll` of `bits`.
    pub fn dll(&self, bits: Bits) -> PathBuf {
        self.dir(bits).join("agora_vfs.dll")
    }

    /// `agora-vfs-probe.exe` of `bits`.
    pub fn probe(&self, bits: Bits) -> PathBuf {
        self.dir(bits).join("agora-vfs-probe.exe")
    }

    /// The helper beside the DLL of `bits`.
    pub fn helper(&self, bits: Bits) -> PathBuf {
        self.dir(bits).join("agora-vfs-helper.exe")
    }

    /// Panics, naming what to build, unless the DLL, probe and helper of both bitnesses exist.
    pub fn assert_built(&self) {
        for bits in [Bits::X64, Bits::X86] {
            for file in [self.dll(bits), self.probe(bits), self.helper(bits)] {
                assert!(
                    file.exists(),
                    "{} missing: cargo build -p agora-vfs, then cargo build -p agora-vfs --target i686-pc-windows-msvc",
                    file.display()
                );
            }
        }
    }
}

/// Inject `dll` (of `bits`) into the suspended process `process` (process id `pid`). A 64-bit process
/// gets it from a remote `LoadLibraryW` in this process. A 32-bit one is handed to the 32-bit helper,
/// which must succeed. Returns the reason on failure.
///
/// # Safety
/// `process` must be a live handle to a suspended process of the bitness `bits`.
pub unsafe fn inject(process: HANDLE, pid: u32, bits: Bits, dll: &Path) -> Result<(), String> {
    match bits {
        Bits::X64 => {
            let loaded =
                agora_vfs_inject::inject_remote_thread(process, dll, Duration::from_secs(30))?;
            if loaded == 0 {
                return Err("LoadLibraryW returned NULL in the target".to_string());
            }
            Ok(())
        }
        Bits::X86 => inject_by_helper(pid, dll),
    }
}

/// Run the helper beside `dll` to inject it into process `pid`, and wait for it.
pub fn inject_by_helper(pid: u32, dll: &Path) -> Result<(), String> {
    let helper = agora_vfs_inject::helper_beside(dll);
    let output = Command::new(&helper)
        .args(["--pid", &pid.to_string(), "--dll"])
        .arg(dll)
        .output()
        .map_err(|e| format!("could not run {}: {e}", helper.display()))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{} failed: {}",
            helper.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}
