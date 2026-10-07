//! Running a game under `agora_vfs.dll` (MASTER_SPEC §26.5, rung 1).
//!
//! The game starts suspended, the DLL is injected with a remote `LoadLibraryW`, and the game is
//! resumed only after the DLL has confirmed its hooks through a named event. Any failure before
//! that point terminates the suspended process, so nothing has run, and is reported as
//! [`LaunchError::VfsUnavailable`]. The DLL is found at runtime and never linked, fetched or built.

use std::path::{Path, PathBuf};

use super::{LaunchError, LaunchedGame, ResolvedLaunch};

/// Everything needed to run one launch under the VFS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VfsLaunch {
    /// `agora_vfs.dll`.
    pub dll: PathBuf,
    /// The game folder the game sees (the link farm's `game/` folder).
    pub mount: PathBuf,
    /// The writable layer: every write, copy-up and whiteout lands here.
    pub upper: PathBuf,
    /// Lower layers, highest priority first.
    pub lowers: Vec<PathBuf>,
    /// Where the JSON configuration is written.
    pub config_path: PathBuf,
    /// The VFS's log.
    pub log: PathBuf,
}

/// Find `agora_vfs.dll`: `AGORA_VFS_DLL` if set, else beside the running executable.
pub fn locate_dll() -> Result<PathBuf, String> {
    locate_dll_from(
        std::env::var_os("AGORA_VFS_DLL").map(PathBuf::from),
        std::env::current_exe().ok().as_deref(),
    )
}

fn locate_dll_from(
    env_override: Option<PathBuf>,
    current_exe: Option<&Path>,
) -> Result<PathBuf, String> {
    if !cfg!(windows) {
        return Err("the virtual file system is only available on Windows".to_string());
    }
    if let Some(path) = env_override {
        return if path.is_file() {
            Ok(path)
        } else {
            Err(format!(
                "AGORA_VFS_DLL points at '{}', which is not a file",
                path.display()
            ))
        };
    }
    let beside = current_exe
        .and_then(Path::parent)
        .map(|dir| dir.join("agora_vfs.dll"))
        .ok_or_else(|| {
            "cannot tell where Agora is installed to look for agora_vfs.dll".to_string()
        })?;
    if beside.is_file() {
        Ok(beside)
    } else {
        Err(format!(
            "agora_vfs.dll was not found at '{}'",
            beside.display()
        ))
    }
}

/// The VFS's JSON configuration (`crates/agora-vfs/README.md`).
pub fn config_json(vfs: &VfsLaunch, ready_event: &str) -> serde_json::Value {
    serde_json::json!({
        "version": 1,
        "mount": vfs.mount.to_string_lossy(),
        "upper": vfs.upper.to_string_lossy(),
        "lowers": vfs.lowers.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>(),
        "dll": vfs.dll.to_string_lossy(),
        "log": vfs.log.to_string_lossy(),
        "verbose": false,
        "ready_event": ready_event,
    })
}

/// Start the game under the VFS. The returned [`LaunchedGame`] is the running game, exactly as
/// [`super::launch`] returns it.
#[cfg(not(windows))]
pub fn launch_under_vfs(
    _resolved: &ResolvedLaunch,
    _vfs: &VfsLaunch,
) -> Result<LaunchedGame, LaunchError> {
    Err(LaunchError::VfsUnavailable {
        reason: "the virtual file system is only available on Windows".to_string(),
    })
}

/// Start the game under the VFS. The returned [`LaunchedGame`] is the running game, exactly as
/// [`super::launch`] returns it.
#[cfg(windows)]
pub fn launch_under_vfs(
    resolved: &ResolvedLaunch,
    vfs: &VfsLaunch,
) -> Result<LaunchedGame, LaunchError> {
    windows::launch_under_vfs(resolved, vfs)
}

#[cfg(windows)]
mod windows {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use std::path::Path;
    use std::time::Duration;

    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::System::Diagnostics::Debug::WriteProcessMemory;
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    use windows_sys::Win32::System::Memory::{
        VirtualAllocEx, VirtualFreeEx, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE,
    };
    use windows_sys::Win32::System::Threading::{
        CreateEventW, CreateRemoteThread, GetExitCodeThread, WaitForSingleObject, CREATE_SUSPENDED,
    };

    use super::{config_json, LaunchError, LaunchedGame, ResolvedLaunch, VfsLaunch};
    use crate::process_identity;

    const STEP_TIMEOUT: Duration = Duration::from_secs(10);

    fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
        s.as_ref().encode_wide().chain(Some(0)).collect()
    }

    fn unavailable(reason: impl Into<String>) -> LaunchError {
        LaunchError::VfsUnavailable {
            reason: reason.into(),
        }
    }

    /// A Win32 handle closed on drop.
    struct Handle(HANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: the handle is owned by this guard and closed exactly once.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    pub(super) fn launch_under_vfs(
        resolved: &ResolvedLaunch,
        vfs: &VfsLaunch,
    ) -> Result<LaunchedGame, LaunchError> {
        if !vfs.dll.is_file() {
            return Err(unavailable(format!(
                "agora_vfs.dll was not found at '{}'",
                vfs.dll.display()
            )));
        }

        let event_name = format!(
            "Local\\agora-vfs-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        );
        // SAFETY: a manual-reset, initially unsignalled, named event; the name is NUL-terminated.
        let event = unsafe { CreateEventW(std::ptr::null(), 1, 0, wide(&event_name).as_ptr()) };
        if event.is_null() {
            return Err(unavailable(format!(
                "could not create the ready event (error {})",
                unsafe { GetLastError() }
            )));
        }
        let event = Handle(event);

        write_config(vfs, &event_name)?;

        let mut cmd = std::process::Command::new(&resolved.program);
        cmd.args(&resolved.args);
        cmd.current_dir(&resolved.cwd);
        cmd.envs(&resolved.env);
        cmd.env("AGORA_VFS_CONFIG", &vfs.config_path);
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());
        cmd.creation_flags(CREATE_SUSPENDED);

        let mut child = cmd.spawn().map_err(LaunchError::Io)?;
        let process = child.as_raw_handle() as HANDLE;

        // From here on the process exists and is suspended: any failure must end it.
        // SAFETY: `process` is the live handle `child` owns and `event` outlives the call.
        if let Err(reason) = unsafe { inject_and_resume(process, vfs, event.0) } {
            let _ = child.kill();
            let _ = child.wait();
            return Err(unavailable(reason));
        }

        let identity = process_identity::capture(child.id())
            .map_err(|e| LaunchError::ProcessCapture(format!("{e}")))?;
        Ok(LaunchedGame {
            child,
            identity,
            program: resolved.program.clone(),
        })
    }

    fn write_config(vfs: &VfsLaunch, event_name: &str) -> Result<(), LaunchError> {
        let write = || -> std::io::Result<()> {
            for file in [&vfs.config_path, &vfs.log] {
                if let Some(parent) = file.parent() {
                    std::fs::create_dir_all(parent)?;
                }
            }
            std::fs::create_dir_all(&vfs.upper)?;
            let bytes = serde_json::to_vec_pretty(&config_json(vfs, event_name))
                .map_err(std::io::Error::other)?;
            std::fs::write(&vfs.config_path, bytes)
        };
        write().map_err(|e| {
            unavailable(format!(
                "could not write the VFS configuration '{}': {e}",
                vfs.config_path.display()
            ))
        })
    }

    /// Inject the DLL, wait for it to confirm its hooks, then resume the process.
    ///
    /// # Safety
    /// `process` must be a live handle to a suspended process and `event` a live event handle.
    unsafe fn inject_and_resume(
        process: HANDLE,
        vfs: &VfsLaunch,
        event: HANDLE,
    ) -> Result<(), String> {
        let load_result = inject(process, &vfs.dll)?;
        let timeout_ms = STEP_TIMEOUT.as_millis() as u32;
        if load_result == 0 && WaitForSingleObject(event, 0) != WAIT_OBJECT_0 {
            // The DLL signals ready from DllMain, before LoadLibraryW returns, so a NULL module
            // with no signal means it did not load.
            return Err(
                "the game process could not load agora_vfs.dll (blocked by security software, \
                 or built for another architecture)"
                    .to_string(),
            );
        }
        match WaitForSingleObject(event, timeout_ms) {
            WAIT_OBJECT_0 => {}
            WAIT_TIMEOUT => {
                return Err(format!(
                    "agora_vfs.dll loaded but did not confirm its hooks within {} seconds (see {})",
                    STEP_TIMEOUT.as_secs(),
                    vfs.log.display()
                ));
            }
            other => return Err(format!("waiting for the VFS ready signal failed ({other})")),
        }
        resume(process)
    }

    /// Load `dll` into the suspended `process` with a remote `LoadLibraryW`, and return the
    /// (truncated) module handle the call produced; zero means the load failed.
    unsafe fn inject(process: HANDLE, dll: &Path) -> Result<u32, String> {
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
        let Some(load_library) = GetProcAddress(kernel32, c"LoadLibraryW".as_ptr() as *const u8)
        else {
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
        let thread = Handle(thread);
        match WaitForSingleObject(thread.0, STEP_TIMEOUT.as_millis() as u32) {
            WAIT_OBJECT_0 => {}
            WAIT_TIMEOUT => {
                return Err(format!(
                    "loading agora_vfs.dll took longer than {} seconds",
                    STEP_TIMEOUT.as_secs()
                ));
            }
            other => return Err(format!("waiting for the injection thread failed ({other})")),
        }
        let mut code = 0u32;
        if GetExitCodeThread(thread.0, &mut code) == 0 {
            return Err(format!(
                "could not read the injection result (error {})",
                GetLastError()
            ));
        }
        // The thread is done with the path: give the page back.
        VirtualFreeEx(process, remote, 0, MEM_RELEASE);
        Ok(code)
    }

    /// Resume every thread of a process created suspended.
    unsafe fn resume(process: HANDLE) -> Result<(), String> {
        type NtResumeProcess = unsafe extern "system" fn(HANDLE) -> i32;
        let ntdll = GetModuleHandleW(wide("ntdll.dll").as_ptr());
        if ntdll.is_null() {
            return Err("ntdll.dll is not loaded".to_string());
        }
        let Some(proc) = GetProcAddress(ntdll, c"NtResumeProcess".as_ptr() as *const u8) else {
            return Err("NtResumeProcess was not found".to_string());
        };
        let nt_resume_process: NtResumeProcess = std::mem::transmute(proc);
        let status = nt_resume_process(process);
        if status < 0 {
            return Err(format!(
                "the game process could not be resumed (status {status:#x})"
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> VfsLaunch {
        VfsLaunch {
            dll: PathBuf::from("C:/Agora/agora_vfs.dll"),
            mount: PathBuf::from("D:/Bases/deployments/inst/game"),
            upper: PathBuf::from("C:/Data/instances/inst/writable"),
            lowers: vec![PathBuf::from("D:/Bases/deployments/inst/game")],
            config_path: PathBuf::from("C:/Data/instances/inst/vfs/config.json"),
            log: PathBuf::from("C:/Data/instances/inst/logs/vfs.log"),
        }
    }

    #[test]
    fn config_follows_the_dll_readme() {
        let json = config_json(&sample(), "Local\\agora-vfs-1-abc");
        assert_eq!(json["version"], 1);
        assert_eq!(json["mount"], "D:/Bases/deployments/inst/game");
        assert_eq!(json["upper"], "C:/Data/instances/inst/writable");
        assert_eq!(json["lowers"][0], json["mount"]);
        assert_eq!(json["dll"], "C:/Agora/agora_vfs.dll");
        assert_eq!(json["log"], "C:/Data/instances/inst/logs/vfs.log");
        assert_eq!(json["ready_event"], "Local\\agora-vfs-1-abc");
    }

    #[cfg(windows)]
    #[test]
    fn dll_lookup_prefers_the_override_then_the_executable_folder() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("agora.exe");
        let beside = dir.path().join("agora_vfs.dll");

        // Nothing there yet: an error naming where it looked.
        let err = locate_dll_from(None, Some(&exe)).unwrap_err();
        assert!(err.contains("agora_vfs.dll"), "{err}");

        std::fs::write(&beside, b"x").unwrap();
        assert_eq!(locate_dll_from(None, Some(&exe)).unwrap(), beside);

        // An override that points nowhere is an error, not a silent fall back to the neighbour.
        let missing = dir.path().join("elsewhere.dll");
        let err = locate_dll_from(Some(missing), Some(&exe)).unwrap_err();
        assert!(err.contains("AGORA_VFS_DLL"), "{err}");

        let custom = dir.path().join("custom.dll");
        std::fs::write(&custom, b"x").unwrap();
        assert_eq!(
            locate_dll_from(Some(custom.clone()), Some(&exe)).unwrap(),
            custom
        );
    }
}
