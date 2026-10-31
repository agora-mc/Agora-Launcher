//! Fixture DLL for the import-order test (`crates/agora-vfs/README.md`, "Tests").
//!
//! The executable `agora-early-import-exe` statically imports this DLL, so the Windows loader
//! runs this `DllMain` while the process is still being initialised: before `main`, and before
//! any DLL injected the old way (a remote `LoadLibraryW` thread) has had a chance to hook
//! anything. On process attach it rewrites `early_<program>.txt` in its own folder, which must
//! already exist, exactly like Engine Fixes' preloader rewriting its log. Under agora's VFS the
//! write has to land in the writable layer, not in the file the folder links to.
//!
//! The file is named after the program that loaded the DLL (`Game.exe` writes `early_Game.txt`),
//! so a program and the child it starts each have a file of their own.

#![cfg(windows)]
#![allow(clippy::missing_safety_doc)]

use std::ffi::c_void;
use std::io::Write;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

use windows_sys::Win32::Foundation::{BOOL, HMODULE, MAX_PATH};
use windows_sys::Win32::System::LibraryLoader::GetModuleFileNameW;

/// What `DllMain` writes into `early_<program>.txt`.
pub const WRITTEN: &[u8] = b"written by the fixture DllMain";

/// Called by the executable so the linker keeps the import. Returns a constant.
#[no_mangle]
pub extern "C" fn agora_early_import_marker() -> u32 {
    0xA60A
}

fn module_path(module: HMODULE) -> Option<PathBuf> {
    let mut buf = [0u16; MAX_PATH as usize * 2];
    // SAFETY: `buf` is writable for its stated length.
    let len = unsafe { GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as u32) } as usize;
    (len > 0 && len < buf.len()).then(|| PathBuf::from(std::ffi::OsString::from_wide(&buf[..len])))
}

#[no_mangle]
pub unsafe extern "system" fn DllMain(
    module: HMODULE,
    reason: u32,
    _reserved: *mut c_void,
) -> BOOL {
    const DLL_PROCESS_ATTACH: u32 = 1;
    if reason == DLL_PROCESS_ATTACH {
        // The DLL's own folder, and the program (the null module) that loaded it.
        if let (Some(dll), Some(exe)) = (module_path(module), module_path(std::ptr::null_mut())) {
            if let (Some(dir), Some(stem)) = (dll.parent(), exe.file_stem()) {
                let name = format!("early_{}.txt", stem.to_string_lossy());
                // Only an existing file: the point is rewriting a file that is not ours.
                if let Ok(mut file) = std::fs::OpenOptions::new()
                    .write(true)
                    .truncate(true)
                    .open(dir.join(name))
                {
                    let _ = file.write_all(WRITTEN);
                }
            }
        }
    }
    1
}
