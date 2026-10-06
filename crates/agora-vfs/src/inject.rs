use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::Diagnostics::Debug::WriteProcessMemory;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::Memory::{VirtualAllocEx, MEM_COMMIT, MEM_RESERVE, PAGE_READWRITE};
use windows_sys::Win32::System::Threading::*;

/// Load `dll` into `process` (created suspended) with a remote `LoadLibraryW`.
pub unsafe fn inject(process: HANDLE, dll: &Path) -> Result<(), String> {
    let wide: Vec<u16> = dll.as_os_str().encode_wide().chain(Some(0)).collect();
    let bytes = wide.len() * 2;
    let remote = VirtualAllocEx(
        process,
        std::ptr::null(),
        bytes,
        MEM_COMMIT | MEM_RESERVE,
        PAGE_READWRITE,
    );
    if remote.is_null() {
        return Err(format!("VirtualAllocEx {}", GetLastError()));
    }
    if WriteProcessMemory(
        process,
        remote,
        wide.as_ptr() as *const c_void,
        bytes,
        std::ptr::null_mut(),
    ) == 0
    {
        return Err(format!("WriteProcessMemory {}", GetLastError()));
    }
    let k32: Vec<u16> = "kernel32.dll\0".encode_utf16().collect();
    let load = GetProcAddress(
        GetModuleHandleW(k32.as_ptr()),
        c"LoadLibraryW".as_ptr() as *const u8,
    )
    .ok_or("no LoadLibraryW")?;
    let thread = CreateRemoteThread(
        process,
        std::ptr::null(),
        0,
        Some(std::mem::transmute::<
            _,
            unsafe extern "system" fn(*mut c_void) -> u32,
        >(load)),
        remote,
        0,
        std::ptr::null_mut(),
    );
    if thread.is_null() {
        return Err(format!("CreateRemoteThread {}", GetLastError()));
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
