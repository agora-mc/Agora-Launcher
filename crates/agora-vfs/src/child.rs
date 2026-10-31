//! Getting a child the game started into the VFS, by the child's own bitness (brief decision 4).
//!
//! A child of the same bitness as this DLL is injected directly, as it always was. A child of the
//! other bitness cannot be injected from here: its import table is rewritten, and its DLL loaded,
//! by a helper of the child's own bitness, `agora-vfs-helper.exe`, which sits beside the DLL of
//! that bitness. The helper is started through the original `CreateProcessInternalW`, so this hook
//! does not try to inject it. The helper gets 15 seconds and is ended when it takes longer.
//!
//! Any failure ends the child, as every other injection failure does: a child that is not under the
//! VFS must not run, because it could write to the layers.

#![cfg(windows)]

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::{null, null_mut};

use agora_vfs_inject::{Failure, Method, OWN_MACHINE};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, FALSE, HANDLE, TRUE};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::ReadFile;
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::SystemInformation::{
    IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_I386,
};
use windows_sys::Win32::System::Threading::{
    DeleteProcThreadAttributeList, InitializeProcThreadAttributeList, UpdateProcThreadAttribute,
    CREATE_NO_WINDOW, EXTENDED_STARTUPINFO_PRESENT, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

use crate::config::RuntimeConfig;
use crate::hooks::hooks;
use crate::util::{is_inside, set_inside};

/// The most of the helper's stderr that is kept for a reason.
const REASON_LIMIT: usize = 4096;

/// Get `agora_vfs.dll` into the suspended child `process` (process id `pid`), by the child's own
/// bitness. Returns how it was done, and the reason a remote thread was used when the import table
/// could not be (as [`agora_vfs_inject::inject_child`] does).
///
/// # Safety
/// `process` must be a live handle to a suspended child that has not been resumed.
pub(crate) unsafe fn inject(
    process: HANDLE,
    pid: u32,
    c: &RuntimeConfig,
) -> Result<(Method, Option<Failure>), String> {
    let machine = agora_vfs_inject::process_machine(process).ok_or_else(|| {
        format!(
            "could not tell what architecture the child {pid} is (error {})",
            GetLastError()
        )
    })?;
    let dll = match machine {
        IMAGE_FILE_MACHINE_AMD64 => c.dll.clone(),
        IMAGE_FILE_MACHINE_I386 => match &c.dll_x86 {
            Some(dll) => dll.clone(),
            None => {
                return Err(not_found(
                    process,
                    machine,
                    "(no path is configured for them)",
                ))
            }
        },
        // Another architecture: the injection refuses it, and nothing is touched.
        _ => return agora_vfs_inject::inject_child(process, &c.dll),
    };
    if !dll.is_file() {
        return Err(not_found(process, machine, &dll.display().to_string()));
    }
    if machine == OWN_MACHINE {
        return agora_vfs_inject::inject_child(process, &dll);
    }
    let helper = agora_vfs_inject::helper_beside(&dll);
    if !helper.is_file() {
        return Err(not_found(process, machine, &helper.display().to_string()));
    }
    run_helper(&helper, pid, &dll)?;
    Ok((Method::Helper, None))
}

/// The reason a 32-bit or 64-bit child could not be protected because its VFS files are missing.
unsafe fn not_found(process: HANDLE, machine: u16, path: &str) -> String {
    let program = agora_vfs_inject::image_path(process)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "the child".to_string());
    let bits = agora_vfs_inject::bitness_word(machine);
    format!("{program} is a {bits} program and the {bits} VFS files were not found at {path}")
}

fn wide(text: &str) -> Vec<u16> {
    std::ffi::OsStr::new(text)
        .encode_wide()
        .chain(Some(0))
        .collect()
}

/// Start `helper` for `pid` and wait for it. Its stderr, when it wrote any, is the reason.
unsafe fn run_helper(helper: &Path, pid: u32, dll: &Path) -> Result<(), String> {
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: null_mut(),
        bInheritHandle: TRUE,
    };
    let (mut read, mut write): (HANDLE, HANDLE) = (null_mut(), null_mut());
    if CreatePipe(&mut read, &mut write, &attributes, 0) == FALSE {
        return Err(format!(
            "could not make a pipe for the helper (error {})",
            GetLastError()
        ));
    }
    let started = spawn_helper(helper, pid, dll, write);
    // Only the helper may hold the write end now, so the pipe ends when the helper does.
    CloseHandle(write);
    let (process, thread) = match started {
        Ok(started) => started,
        Err(why) => {
            CloseHandle(read);
            return Err(why);
        }
    };
    let waited = agora_vfs_inject::wait_for_helper(process, agora_vfs_inject::HELPER_TIMEOUT);
    let said = read_text(read);
    CloseHandle(read);
    CloseHandle(thread);
    CloseHandle(process);
    waited.map_err(|why| {
        if said.is_empty() {
            why
        } else {
            format!("{said} ({why})")
        }
    })
}

/// Start the helper with only the pipe's write end inherited, as its standard error, through the
/// original `CreateProcessInternalW`: this hook would otherwise try to inject the helper.
unsafe fn spawn_helper(
    helper: &Path,
    pid: u32,
    dll: &Path,
    stderr: HANDLE,
) -> Result<(HANDLE, HANDLE), String> {
    let mut size = 0usize;
    InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut size);
    let mut storage = vec![0usize; size.div_ceil(std::mem::size_of::<usize>())];
    let list = storage.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
    if InitializeProcThreadAttributeList(list, 1, 0, &mut size) == FALSE {
        return Err(format!(
            "could not prepare the helper's handles (error {})",
            GetLastError()
        ));
    }
    let _list = AttributeList(list);
    let mut inherited = [stderr];
    if UpdateProcThreadAttribute(
        list,
        0,
        PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
        inherited.as_mut_ptr() as *const c_void,
        std::mem::size_of_val(&inherited),
        null_mut(),
        null(),
    ) == FALSE
    {
        return Err(format!(
            "could not prepare the helper's handles (error {})",
            GetLastError()
        ));
    }
    let mut startup: STARTUPINFOEXW = std::mem::zeroed();
    startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdError = stderr;
    startup.lpAttributeList = list;
    let mut command = wide(&agora_vfs_inject::helper_command_line(helper, pid, dll));
    let mut info: PROCESS_INFORMATION = std::mem::zeroed();
    let was_inside = is_inside();
    set_inside(true);
    let ok = hooks().create_process.call(
        null_mut(),
        null(),
        command.as_mut_ptr(),
        null(),
        null(),
        TRUE,
        CREATE_NO_WINDOW | EXTENDED_STARTUPINFO_PRESENT,
        null(),
        null(),
        &startup as *const STARTUPINFOEXW as *const c_void,
        &mut info,
        null_mut(),
    );
    set_inside(was_inside);
    if ok == FALSE {
        return Err(format!(
            "could not start {} (error {})",
            helper.display(),
            GetLastError()
        ));
    }
    Ok((info.hProcess, info.hThread))
}

/// Closes the attribute list it owns.
struct AttributeList(LPPROC_THREAD_ATTRIBUTE_LIST);

impl Drop for AttributeList {
    fn drop(&mut self) {
        // SAFETY: the list was initialised by `spawn_helper` and is deleted once.
        unsafe { DeleteProcThreadAttributeList(self.0) };
    }
}

/// Everything the helper wrote to `read`, up to [`REASON_LIMIT`]. Read after the helper ended, so
/// the pipe is closed by then and the read stops.
unsafe fn read_text(read: HANDLE) -> String {
    let mut out = Vec::new();
    let mut chunk = [0u8; 512];
    while out.len() < REASON_LIMIT {
        let mut n = 0u32;
        if ReadFile(
            read,
            chunk.as_mut_ptr(),
            chunk.len() as u32,
            &mut n,
            null_mut(),
        ) == FALSE
            || n == 0
        {
            break;
        }
        out.extend_from_slice(&chunk[..n as usize]);
    }
    String::from_utf8_lossy(&out).trim().to_string()
}
