//! Getting `agora_vfs.dll` into a process that was created suspended (MASTER_SPEC §26.5).
//!
//! Two ways, best first:
//!
//! 1. [`inject_import_table`], the technique Microsoft Detours uses for
//!    `DetourCreateProcessWithDllEx`: the suspended process's import table is rewritten in memory
//!    so the DLL is its first import. The Windows loader then loads and initialises it before any
//!    of the program's own DLLs, so its hooks exist before a single line of the game's code (or of
//!    its static imports' `DllMain`s) has run.
//! 2. [`inject_remote_thread`], a remote thread running `LoadLibraryW`. Starting that thread is
//!    what makes the loader initialise the process, so the program's static imports have already run
//!    their `DllMain`s by the time the DLL loads. It is the fallback for executables the first way
//!    cannot patch.
//!
//! Both are used by the launcher (for the game) and by the DLL's `CreateProcessInternalW` hook
//! (for every process the game starts), which is why they live in a crate of their own: the
//! launcher never links the DLL.

#![cfg(windows)]
#![allow(clippy::missing_safety_doc)]

use std::ffi::{c_char, c_void};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;
use windows_sys::Win32::System::Diagnostics::Debug::WriteProcessMemory;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::Memory::{
    VirtualAllocEx, VirtualFreeEx, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE,
};
use windows_sys::Win32::System::SystemInformation::{
    IMAGE_FILE_MACHINE, IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_ARM64,
    IMAGE_FILE_MACHINE_I386, IMAGE_FILE_MACHINE_UNKNOWN,
};
use windows_sys::Win32::System::Threading::{
    CreateRemoteThread, GetExitCodeProcess, GetExitCodeThread, GetProcessId, IsWow64Process2,
    QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject, PROCESS_NAME_WIN32,
};

// Keep the Detours static library linked: the two functions used below are declared here
// (`DetourUpdateProcessWithDll` has no binding in `detours-sys2`).
use detours_sys2 as _;

// Detours declares these `WINAPI`, which is `stdcall` on x86 and the only convention on x64. An
// `extern "C"` block would ask for `cdecl` names that the i686 build of Detours does not export.
extern "system" {
    /// `DetourUpdateProcessWithDll` (Detours' `creatwth.cpp`): make `rlpDlls` the first imports of
    /// the suspended process's executable.
    fn DetourUpdateProcessWithDll(hProcess: HANDLE, rlpDlls: *mut *const c_char, nDlls: u32)
        -> i32;
    /// `DetourRestoreAfterWith` (Detours' `modules.cpp`): in the injected process, put back the
    /// headers and import table `DetourUpdateProcessWithDll` rewrote.
    fn DetourRestoreAfterWith() -> i32;
}

/// Undo, inside the injected process, what [`inject_import_table`] did to its executable's headers
/// and import table, so the program sees its own. Detours requires the injected DLL to do this from
/// `DllMain`: SteamStub-wrapped games hang on the rewritten table. Returns false when there was
/// nothing to restore, as when the DLL was loaded by a remote thread.
///
/// # Safety
/// Call only from the injected DLL's process attach.
pub unsafe fn restore_after_import_injection() -> bool {
    DetourRestoreAfterWith() != 0
}

/// The machine type `agora_vfs.dll` was built for.
#[cfg(target_arch = "x86_64")]
pub const OWN_MACHINE: IMAGE_FILE_MACHINE = IMAGE_FILE_MACHINE_AMD64;
#[cfg(target_arch = "aarch64")]
pub const OWN_MACHINE: IMAGE_FILE_MACHINE = IMAGE_FILE_MACHINE_ARM64;
#[cfg(target_arch = "x86")]
pub const OWN_MACHINE: IMAGE_FILE_MACHINE = IMAGE_FILE_MACHINE_I386;

/// The file name of the helper that injects a DLL into a process of the helper's own bitness. It
/// always sits beside the DLL of its bitness (see [`helper_beside`]).
pub const HELPER_FILE_NAME: &str = "agora-vfs-helper.exe";

/// How long a helper gets to inject a DLL into a process of another bitness before it is ended.
pub const HELPER_TIMEOUT: Duration = Duration::from_secs(15);

/// The helper that sits beside `dll`.
pub fn helper_beside(dll: &Path) -> PathBuf {
    dll.with_file_name(HELPER_FILE_NAME)
}

/// The command line that starts the helper for `pid`: `"<helper>" --pid <pid> --dll "<dll>"`.
pub fn helper_command_line(helper: &Path, pid: u32, dll: &Path) -> String {
    format!(
        "\"{}\" --pid {pid} --dll \"{}\"",
        helper.display(),
        dll.display()
    )
}

/// Wait for a helper that was started to inject into a process, and say how it ended. A helper
/// that does not finish within `timeout` is ended, so a hung helper cannot hold its target. Its
/// exit code is 0 when the injection worked.
///
/// # Safety
/// `helper` must be a live process handle with `SYNCHRONIZE`, `PROCESS_TERMINATE` and
/// `PROCESS_QUERY_LIMITED_INFORMATION` access (the handle `CreateProcess` returns has them).
pub unsafe fn wait_for_helper(helper: HANDLE, timeout: Duration) -> Result<(), String> {
    match WaitForSingleObject(helper, timeout.as_millis() as u32) {
        WAIT_OBJECT_0 => {
            let mut code = 0u32;
            if GetExitCodeProcess(helper, &mut code) == 0 {
                return Err(format!(
                    "the helper's exit code could not be read (error {})",
                    GetLastError()
                ));
            }
            if code == 0 {
                Ok(())
            } else {
                Err(format!("the helper ended with exit code {code}"))
            }
        }
        WAIT_TIMEOUT => {
            TerminateProcess(helper, 1);
            WaitForSingleObject(helper, 5_000);
            Err(format!(
                "the helper did not finish within {} seconds and was ended",
                timeout.as_secs()
            ))
        }
        _ => Err(format!(
            "waiting for the helper failed (error {})",
            GetLastError()
        )),
    }
}

/// The full path of the program `process` runs, or `None` when Windows cannot say.
///
/// # Safety
/// `process` must be a live handle with `PROCESS_QUERY_LIMITED_INFORMATION` access.
pub unsafe fn image_path(process: HANDLE) -> Option<PathBuf> {
    let mut buf = vec![0u16; 1024];
    let mut len = buf.len() as u32;
    if QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) == 0 {
        return None;
    }
    Some(PathBuf::from(std::ffi::OsString::from_wide(
        &buf[..len as usize],
    )))
}

/// The process id of a process handle.
///
/// # Safety
/// `process` must be a live process handle.
pub unsafe fn pid_of(process: HANDLE) -> u32 {
    GetProcessId(process)
}

/// How a machine type reads in a message: "32-bit", "64-bit", or "another architecture".
pub fn bitness_word(machine: IMAGE_FILE_MACHINE) -> &'static str {
    match machine {
        IMAGE_FILE_MACHINE_I386 => "32-bit",
        IMAGE_FILE_MACHINE_AMD64 => "64-bit",
        _ => "another-architecture",
    }
}

/// Set `AGORA_VFS_INJECTION=remote-thread` to skip import-table injection and use the remote
/// thread for every process (the game and, since children inherit the setting, the processes it
/// starts). For an executable the import table breaks, and for comparing the two methods.
pub const FORCE_REMOTE_THREAD_ENV: &str = "AGORA_VFS_INJECTION";

/// Which way a DLL got into a process, for the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    ImportTable,
    RemoteThread,
    /// A helper of the program's own bitness did the injection (the program is built for another
    /// architecture than the process that started it).
    Helper,
}

impl Method {
    pub fn describe(self) -> &'static str {
        match self {
            Method::ImportTable => "import table (the DLL loads before the program's own imports)",
            Method::RemoteThread => {
                "remote thread (the program's own imports ran before the DLL loaded)"
            }
            Method::Helper => {
                "a helper of the program's own bitness (import table or remote thread)"
            }
        }
    }
}

/// Why [`inject_import_table`] did not inject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The process is built for another architecture than the DLL. Nothing was touched, and the
    /// remote-thread method cannot work either, so callers must not fall back.
    WrongArchitecture(String),
    /// Anything else (a protected or unusual executable, a path the loader cannot read). Callers
    /// fall back to [`inject_remote_thread`].
    Other(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::WrongArchitecture(s) | Failure::Other(s) => f.write_str(s),
        }
    }
}

/// The architecture of the program a machine type names, for a message.
pub fn machine_name(machine: IMAGE_FILE_MACHINE) -> &'static str {
    match machine {
        IMAGE_FILE_MACHINE_I386 => "32-bit (x86)",
        IMAGE_FILE_MACHINE_AMD64 => "64-bit (x64)",
        IMAGE_FILE_MACHINE_ARM64 => "ARM64",
        _ => "of another architecture",
    }
}

/// The architecture a process runs as, or `None` when Windows cannot say.
pub unsafe fn process_machine(process: HANDLE) -> Option<IMAGE_FILE_MACHINE> {
    let mut machine = IMAGE_FILE_MACHINE_UNKNOWN;
    let mut native = IMAGE_FILE_MACHINE_UNKNOWN;
    if IsWow64Process2(process, &mut machine, &mut native) == 0 {
        return None;
    }
    // `machine` is UNKNOWN for a process that is not running under emulation.
    Some(if machine == IMAGE_FILE_MACHINE_UNKNOWN {
        native
    } else {
        machine
    })
}

/// Fail with [`Failure::WrongArchitecture`] unless `process` is built for the DLL's architecture.
pub unsafe fn check_architecture(process: HANDLE) -> Result<(), Failure> {
    match process_machine(process) {
        Some(machine) if machine == OWN_MACHINE => Ok(()),
        Some(machine) => Err(Failure::WrongArchitecture(format!(
            "the game is a {} program and agora_vfs.dll is built for another architecture ({})",
            machine_name(machine),
            machine_name(OWN_MACHINE),
        ))),
        None => Err(Failure::Other(format!(
            "could not tell what architecture the process is (error {})",
            GetLastError()
        ))),
    }
}

/// The DLL path as the loader will read it from an import table: an ANSI string, with backslashes
/// and no `\\?\` prefix. A path with characters outside ASCII uses its 8.3 short form; when there
/// is none, the import-table method cannot name the DLL.
fn import_name(dll: &Path) -> Result<std::ffi::CString, String> {
    let mut text = dll.to_string_lossy().replace('/', "\\");
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        if rest.starts_with("UNC\\") {
            return Err("a network path cannot be an import name".to_string());
        }
        text = rest.to_string();
    }
    if !text.is_ascii() {
        let wide: Vec<u16> = std::ffi::OsString::from(&text)
            .encode_wide()
            .chain(Some(0))
            .collect();
        // SAFETY: `wide` is NUL-terminated; a null buffer asks for the length needed.
        let mut short = vec![
            0u16;
            unsafe { GetShortPathNameW(wide.as_ptr(), std::ptr::null_mut(), 0) }
                as usize
        ];
        // SAFETY: `short` holds exactly the length the first call asked for.
        let len =
            unsafe { GetShortPathNameW(wide.as_ptr(), short.as_mut_ptr(), short.len() as u32) }
                as usize;
        if short.is_empty() || len == 0 || len >= short.len() {
            return Err("the DLL path has characters outside ASCII and no short name".to_string());
        }
        text = std::ffi::OsString::from_wide(&short[..len])
            .to_string_lossy()
            .into_owned();
        if !text.is_ascii() {
            return Err("the DLL path has characters outside ASCII".to_string());
        }
    }
    std::ffi::CString::new(text).map_err(|_| "the DLL path contains a NUL".to_string())
}

/// Make `dll` the first import of the executable in `process`, which must have been created
/// suspended and not yet resumed (nor have had a remote thread started in it).
///
/// Nothing runs yet: the DLL is loaded and initialised by the process's own loader after the
/// caller resumes it, before the program's other imports. A process of another architecture is
/// refused up front, untouched.
///
/// # Safety
/// `process` must be a live handle with the access rights `DetourUpdateProcessWithDll` needs
/// (the handle `CreateProcess` returns has them).
pub unsafe fn inject_import_table(process: HANDLE, dll: &Path) -> Result<(), Failure> {
    check_architecture(process)?;
    if std::env::var_os(FORCE_REMOTE_THREAD_ENV).is_some_and(|v| v == "remote-thread") {
        return Err(Failure::Other(format!(
            "{FORCE_REMOTE_THREAD_ENV}=remote-thread is set"
        )));
    }
    let name = import_name(dll).map_err(Failure::Other)?;
    let mut names = [name.as_ptr()];
    if DetourUpdateProcessWithDll(process, names.as_mut_ptr(), 1) == 0 {
        return Err(Failure::Other(format!(
            "rewriting the process's import table failed (error {})",
            GetLastError()
        )));
    }
    Ok(())
}

fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}

/// Load `dll` into the suspended `process` with a remote `LoadLibraryW`, and return the
/// (truncated) module handle the call produced; zero means the load failed.
///
/// # Safety
/// `process` must be a live handle to a process that is still suspended.
pub unsafe fn inject_remote_thread(
    process: HANDLE,
    dll: &Path,
    timeout: Duration,
) -> Result<u32, String> {
    let path = wide(dll);
    let bytes = path.len() * 2;
    let remote = VirtualAllocEx(
        process,
        std::ptr::null(),
        bytes,
        MEM_COMMIT | MEM_RESERVE,
        PAGE_READWRITE,
    );
    if remote.is_null() {
        return Err(format!(
            "could not allocate memory in the game process (error {})",
            GetLastError()
        ));
    }
    let mut written = 0usize;
    if WriteProcessMemory(
        process,
        remote,
        path.as_ptr() as *const c_void,
        bytes,
        &mut written,
    ) == 0
        || written != bytes
    {
        return Err(format!(
            "could not write to the game process (error {})",
            GetLastError()
        ));
    }
    let kernel32 = GetModuleHandleW(wide("kernel32.dll").as_ptr());
    if kernel32.is_null() {
        return Err("kernel32.dll is not loaded".to_string());
    }
    let Some(load_library) = GetProcAddress(kernel32, c"LoadLibraryW".as_ptr() as *const u8) else {
        return Err("LoadLibraryW was not found".to_string());
    };
    let thread = CreateRemoteThread(
        process,
        std::ptr::null(),
        0,
        Some(std::mem::transmute::<
            unsafe extern "system" fn() -> isize,
            unsafe extern "system" fn(*mut c_void) -> u32,
        >(load_library)),
        remote,
        0,
        std::ptr::null_mut(),
    );
    if thread.is_null() {
        return Err(format!(
            "the game process refused the injection thread (error {})",
            GetLastError()
        ));
    }
    let wait = WaitForSingleObject(thread, timeout.as_millis() as u32);
    let result = match wait {
        WAIT_OBJECT_0 => {
            let mut code = 0u32;
            if GetExitCodeThread(thread, &mut code) == 0 {
                Err(format!(
                    "could not read the injection result (error {})",
                    GetLastError()
                ))
            } else {
                // The thread is done with the path: give the page back.
                VirtualFreeEx(process, remote, 0, MEM_RELEASE);
                Ok(code)
            }
        }
        WAIT_TIMEOUT => Err(format!(
            "loading agora_vfs.dll took longer than {} seconds",
            timeout.as_secs()
        )),
        other => Err(format!("waiting for the injection thread failed ({other})")),
    };
    CloseHandle(thread);
    result
}

/// Inject for a process whose caller does not wait on a ready signal (a child the game started):
/// the import table first, the remote thread when that is not possible. Returns the method used.
///
/// # Safety
/// `process` must be a live handle to a suspended process.
pub unsafe fn inject_child(
    process: HANDLE,
    dll: &Path,
) -> Result<(Method, Option<Failure>), String> {
    match inject_import_table(process, dll) {
        Ok(()) => Ok((Method::ImportTable, None)),
        Err(Failure::WrongArchitecture(why)) => Err(why),
        Err(other) => match inject_remote_thread(process, dll, Duration::from_secs(30))? {
            0 => Err("LoadLibraryW returned NULL in the target".to_string()),
            _ => Ok((Method::RemoteThread, Some(other))),
        },
    }
}
