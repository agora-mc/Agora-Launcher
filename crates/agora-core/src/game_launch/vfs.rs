//! Running a game under `agora_vfs.dll` (MASTER_SPEC §26.5, rung 1).
//!
//! The game starts suspended and its import table is rewritten in memory so `agora_vfs.dll` is its
//! first import (`agora-vfs-inject`, the technique Microsoft Detours uses). Once resumed, Windows'
//! own loader loads the DLL before any of the game's other DLLs, and the DLL confirms its hooks
//! through a named event. Any failure ends the process and is reported as
//! [`LaunchError::VfsUnavailable`]. For an executable whose import table cannot be rewritten the
//! older way is the fallback: the DLL is loaded by a remote `LoadLibraryW` thread and the game is
//! resumed only after the DLL has confirmed its hooks. By then the game's own static imports have
//! already run their `DllMain`s, which is why it is only the fallback. The VFS's log says which
//! was used. The DLL is found at runtime and never linked, fetched or built.

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::{LaunchError, LaunchedGame, ResolvedLaunch};

/// Everything needed to run one launch under the VFS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VfsLaunch {
    /// `agora_vfs.dll`.
    pub dll: PathBuf,
    /// The 32-bit `agora_vfs.dll` (see [`locate_dll_x86`]). Its helper sits beside it. Needed only when
    /// a 32-bit program starts; when its files are missing, that start says where it looked.
    pub dll_x86: Option<PathBuf>,
    /// The game folder the game sees (the link farm's `game/` folder).
    pub mount: PathBuf,
    /// The writable layer: every write, copy-up and whiteout lands here.
    pub upper: PathBuf,
    /// Lower layers, highest priority first.
    pub lowers: Vec<PathBuf>,
    /// Folders that show the same view as `mount`: a tool run's real install folder, so the tool
    /// sees the mod at the path it reads. Empty for a game launch.
    pub aliases: Vec<PathBuf>,
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

/// Where the 32-bit `agora_vfs.dll` is: `AGORA_VFS_DLL_X86` if set, else `x86\agora_vfs.dll` in the
/// folder of the 64-bit DLL `dll`. The path is not checked here: a 32-bit program that starts while
/// the file is missing is refused with this path in its reason.
pub fn locate_dll_x86(dll: &Path) -> PathBuf {
    locate_dll_x86_from(
        std::env::var_os("AGORA_VFS_DLL_X86").map(PathBuf::from),
        dll,
    )
}

fn locate_dll_x86_from(env_override: Option<PathBuf>, dll: &Path) -> PathBuf {
    env_override.unwrap_or_else(|| {
        dll.parent()
            .unwrap_or_else(|| Path::new(""))
            .join("x86")
            .join("agora_vfs.dll")
    })
}

/// The files a 32-bit program needs, given the 32-bit DLL's path: that DLL and its helper beside
/// it. Returns the ones that are not there, so none when both are. A file that cannot be checked
/// counts as missing: a 32-bit program is refused rather than started without its DLL.
#[cfg(windows)]
pub fn missing_x86_files(dll_x86: &Path) -> Vec<PathBuf> {
    [
        dll_x86.to_path_buf(),
        agora_vfs_inject::helper_beside(dll_x86),
    ]
    .into_iter()
    .filter(|path| !path.is_file())
    .collect()
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

/// A program the VFS's DLL ended because it was meant to run protected and could not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EndedProcess {
    pub pid: u32,
    /// The program's path.
    pub exe: String,
    /// Why the DLL could not protect it, in the DLL's words.
    pub reason: String,
}

/// The length of the VFS's log now, to read what a session appends afterwards. A log that does
/// not exist yet is empty.
pub fn log_len(log: &Path) -> u64 {
    std::fs::metadata(log).map(|m| m.len()).unwrap_or(0)
}

/// The programs the DLL ended during whatever was appended to `log` after its first `from` bytes
/// (a [`log_len`] taken before the session). A missing or unreadable log, or one with nothing new,
/// has none; a log shorter than `from` was replaced, so all of it is new.
pub fn processes_ended_since(log: &Path, from: u64) -> Vec<EndedProcess> {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut file) = std::fs::File::open(log) else {
        return Vec::new();
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let start = if len < from { 0 } else { from };
    let mut bytes = Vec::new();
    if file.seek(SeekFrom::Start(start)).is_err() || file.read_to_end(&mut bytes).is_err() {
        return Vec::new();
    }
    processes_ended_by_vfs(&String::from_utf8_lossy(&bytes))
}

/// Read the lines `agora_vfs.dll` writes when it ends a process, `[pid] ending the process
/// <exe>: <why>`, out of log text. Every other line, the DLL's or the launcher's, is ignored. The
/// reason holds no `": "`, so the last one splits it from a path that may.
pub fn processes_ended_by_vfs(appended: &str) -> Vec<EndedProcess> {
    appended
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix('[')?;
            let (pid, rest) = rest.split_once("] ")?;
            let pid = pid.parse().ok()?;
            let rest = rest.strip_prefix("ending the process ")?;
            let (exe, reason) = rest.trim_end().rsplit_once(": ")?;
            if exe.is_empty() || reason.is_empty() {
                return None;
            }
            Some(EndedProcess {
                pid,
                exe: exe.to_string(),
                reason: reason.to_string(),
            })
        })
        .collect()
}

/// The VFS's JSON configuration (`crates/agora-vfs/README.md`). Version 2 when there are aliases or
/// a 32-bit DLL, version 1 otherwise.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn config_json(vfs: &VfsLaunch, ready_event: &str) -> serde_json::Value {
    let mut json = serde_json::json!({
        "version": 1,
        "mount": vfs.mount.to_string_lossy(),
        "upper": vfs.upper.to_string_lossy(),
        "lowers": vfs.lowers.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>(),
        "dll": vfs.dll.to_string_lossy(),
        "log": vfs.log.to_string_lossy(),
        "verbose": false,
        "ready_event": ready_event,
    });
    if let Some(dll_x86) = &vfs.dll_x86 {
        json["dll_x86"] = serde_json::json!(dll_x86.to_string_lossy());
    }
    if !vfs.aliases.is_empty() || vfs.dll_x86.is_some() {
        json["version"] = serde_json::json!(2);
        json["aliases"] = serde_json::json!(vfs
            .aliases
            .iter()
            .map(|p| p.to_string_lossy())
            .collect::<Vec<_>>());
    }
    json
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
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use std::time::Duration;

    use std::io::Read;

    use agora_vfs_inject::{Failure, Method, HELPER_TIMEOUT, OWN_MACHINE};
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    use windows_sys::Win32::System::SystemInformation::IMAGE_FILE_MACHINE_I386;
    use windows_sys::Win32::System::Threading::{
        CreateEventW, GetExitCodeProcess, GetProcessId, WaitForMultipleObjects,
        WaitForSingleObject, CREATE_NO_WINDOW, CREATE_SUSPENDED,
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
        // For a process that cannot read its configuration and so does not know where to log.
        cmd.env("AGORA_VFS_LOG", &vfs.log);
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());
        cmd.creation_flags(CREATE_SUSPENDED);

        let mut child = cmd.spawn().map_err(LaunchError::Io)?;
        let process = child.as_raw_handle() as HANDLE;

        // From here on the process exists: any failure must end it.
        // SAFETY: `process` is the live handle `child` owns and `event` outlives the call.
        match unsafe { inject_and_resume(process, vfs, event.0, &resolved.program) } {
            Ok(method) => append_log(vfs, &format!("[agora] injected by {}", method.describe())),
            Err(reason) => {
                let _ = child.kill();
                let _ = child.wait();
                append_log(vfs, &format!("[agora] injection failed: {reason}"));
                return Err(unavailable(reason));
            }
        }

        let identity = process_identity::capture(child.id())
            .map_err(|e| LaunchError::ProcessCapture(format!("{e}")))?;
        Ok(LaunchedGame {
            child,
            identity,
            program: resolved.program.clone(),
        })
    }

    /// One line in the VFS's log, beside the DLL's own lines. Best effort: the log is a diagnostic.
    fn append_log(vfs: &VfsLaunch, line: &str) {
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&vfs.log)
        {
            let _ = file.write_all(format!("{line}\n").as_bytes());
        }
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

    /// Get the DLL into the suspended process, make sure it confirmed its hooks, and leave the
    /// process running. Returns how the DLL was injected.
    ///
    /// # Safety
    /// `process` must be a live handle to a suspended process and `event` a live event handle.
    unsafe fn inject_and_resume(
        process: HANDLE,
        vfs: &VfsLaunch,
        event: HANDLE,
        program: &std::path::Path,
    ) -> Result<Method, String> {
        // A 32-bit program cannot be injected from this 64-bit process: its DLL comes through the
        // 32-bit helper, as a 32-bit child's does (brief decision 6).
        if agora_vfs_inject::process_machine(process) == Some(IMAGE_FILE_MACHINE_I386)
            && OWN_MACHINE != IMAGE_FILE_MACHINE_I386
        {
            inject_by_helper(process, vfs, program)?;
            resume(process)?;
            wait_for_ready(process, event, vfs)?;
            return Ok(Method::Helper);
        }
        match agora_vfs_inject::inject_import_table(process, &vfs.dll) {
            Ok(()) => {
                // The DLL loads during the process's own start-up, so the process must run for
                // it to confirm. It is first in the import table, so the loader initialises it
                // before the game's other DLLs; a DLL that cannot load or hook ends the process
                // (the loader, or the DLL itself).
                resume(process)?;
                wait_for_ready(process, event, vfs)?;
                Ok(Method::ImportTable)
            }
            // Nothing was touched, and a remote thread would fail the same way.
            Err(Failure::WrongArchitecture(why)) => Err(format!(
                "the game process could not load agora_vfs.dll: {why}"
            )),
            Err(Failure::Other(why)) => {
                append_log(
                    vfs,
                    &format!(
                        "[agora] import-table injection not possible ({why}); using a remote thread"
                    ),
                );
                inject_remote_and_resume(process, vfs, event)?;
                Ok(Method::RemoteThread)
            }
        }
    }

    /// The older way: load the DLL with a remote `LoadLibraryW`, wait for its hooks, then resume.
    unsafe fn inject_remote_and_resume(
        process: HANDLE,
        vfs: &VfsLaunch,
        event: HANDLE,
    ) -> Result<(), String> {
        let load_result = agora_vfs_inject::inject_remote_thread(process, &vfs.dll, STEP_TIMEOUT)?;
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

    /// Wait for the DLL's ready signal in a process that is running, or for the process to end.
    unsafe fn wait_for_ready(
        process: HANDLE,
        event: HANDLE,
        vfs: &VfsLaunch,
    ) -> Result<(), String> {
        // The event comes first: when both are signalled, the lowest index wins, so a game that
        // ran to its end straight after the DLL's confirmation still counts as confirmed.
        let handles = [event, process];
        match WaitForMultipleObjects(
            handles.len() as u32,
            handles.as_ptr(),
            0,
            STEP_TIMEOUT.as_millis() as u32,
        ) {
            WAIT_OBJECT_0 => Ok(()),
            x if x == WAIT_OBJECT_0 + 1 => {
                let mut code = 0u32;
                GetExitCodeProcess(process, &mut code);
                Err(format!(
                    "the game process ended (exit code {code:#x}) before agora_vfs.dll confirmed \
                     its hooks (blocked by security software, or its hooks could not be \
                     installed; see {})",
                    vfs.log.display()
                ))
            }
            WAIT_TIMEOUT => Err(format!(
                "agora_vfs.dll did not confirm its hooks within {} seconds (see {})",
                STEP_TIMEOUT.as_secs(),
                vfs.log.display()
            )),
            other => Err(format!("waiting for the VFS ready signal failed ({other})")),
        }
    }

    /// Why a 32-bit program is refused: which of its VFS files are missing, and both paths they are
    /// expected at, so the user can see where to put them.
    fn x86_files_missing(
        program: &std::path::Path,
        dll: &std::path::Path,
        missing: &[std::path::PathBuf],
    ) -> String {
        let listed: Vec<String> = missing
            .iter()
            .map(|path| format!("'{}'", path.display()))
            .collect();
        format!(
            "{} is a 32-bit program and the 32-bit VFS files were not found: missing {}. \
             A 32-bit program needs agora_vfs.dll and agora-vfs-helper.exe together, expected at '{}' and '{}'.",
            program.display(),
            listed.join(" and "),
            dll.display(),
            agora_vfs_inject::helper_beside(dll).display()
        )
    }

    /// Have the 32-bit helper beside the 32-bit DLL inject it into the suspended `process`. The helper
    /// is waited for, and ended after 15 seconds. Nothing is run unless the injection worked.
    unsafe fn inject_by_helper(
        process: HANDLE,
        vfs: &VfsLaunch,
        program: &std::path::Path,
    ) -> Result<(), String> {
        let Some(dll) = vfs.dll_x86.as_deref() else {
            return Err(format!(
                "{} is a 32-bit program and the 32-bit VFS files were not found: no path is configured for them",
                program.display()
            ));
        };
        let missing = super::missing_x86_files(dll);
        if !missing.is_empty() {
            return Err(x86_files_missing(program, dll, &missing));
        }
        let helper = agora_vfs_inject::helper_beside(dll);
        let pid = GetProcessId(process);
        let mut child = std::process::Command::new(&helper)
            .args(["--pid", &pid.to_string(), "--dll"])
            .arg(dll)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("could not start {}: {e}", helper.display()))?;
        let waited =
            agora_vfs_inject::wait_for_helper(child.as_raw_handle() as HANDLE, HELPER_TIMEOUT);
        let mut said = String::new();
        if let Some(mut stderr) = child.stderr.take() {
            let _ = stderr.read_to_string(&mut said);
        }
        let _ = child.wait();
        waited.map_err(|why| {
            let said = said.trim();
            if said.is_empty() {
                why
            } else {
                format!("{said} ({why})")
            }
        })
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
            dll_x86: None,
            mount: PathBuf::from("D:/Bases/deployments/inst/game"),
            upper: PathBuf::from("C:/Data/instances/inst/writable"),
            lowers: vec![PathBuf::from("D:/Bases/deployments/inst/game")],
            aliases: Vec::new(),
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

    #[test]
    fn a_configuration_without_aliases_is_version_one_with_no_aliases_key() {
        let json = config_json(&sample(), "Local\\agora-vfs-1-abc");
        assert_eq!(json["version"], 1);
        assert!(json.get("aliases").is_none(), "{json}");
    }

    #[test]
    fn a_32_bit_dll_is_named_in_the_configuration_and_makes_it_version_two() {
        let mut vfs = sample();
        vfs.dll_x86 = Some(PathBuf::from("C:/Agora/x86/agora_vfs.dll"));
        let json = config_json(&vfs, "Local\\agora-vfs-1-abc");
        assert_eq!(json["version"], 2);
        assert_eq!(json["dll_x86"], "C:/Agora/x86/agora_vfs.dll");
        assert_eq!(json["dll"], "C:/Agora/agora_vfs.dll");
    }

    #[test]
    fn the_32_bit_dll_is_the_override_or_the_x86_folder_beside_the_64_bit_one() {
        let dll = Path::new("C:/Agora/agora_vfs.dll");
        assert_eq!(
            locate_dll_x86_from(None, dll),
            PathBuf::from("C:/Agora/x86/agora_vfs.dll")
        );
        assert_eq!(
            locate_dll_x86_from(Some(PathBuf::from("D:/elsewhere/x.dll")), dll),
            PathBuf::from("D:/elsewhere/x.dll")
        );
    }

    #[cfg(windows)]
    #[test]
    fn the_32_bit_files_are_reported_missing_by_path_until_both_are_present() {
        let dir = tempfile::tempdir().unwrap();
        let dll = dir.path().join("x86").join("agora_vfs.dll");
        let helper = dir.path().join("x86").join("agora-vfs-helper.exe");

        // Missing: both expected paths are named, the DLL first.
        assert_eq!(missing_x86_files(&dll), vec![dll.clone(), helper.clone()]);

        // The DLL without its helper: only the helper is missing.
        std::fs::create_dir_all(dll.parent().unwrap()).unwrap();
        std::fs::write(&dll, b"x").unwrap();
        assert_eq!(missing_x86_files(&dll), vec![helper.clone()]);

        // Present: nothing is missing.
        std::fs::write(&helper, b"x").unwrap();
        assert!(missing_x86_files(&dll).is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn an_override_is_used_as_is_and_its_helper_is_checked_beside_it() {
        let dir = tempfile::tempdir().unwrap();
        let dll64 = dir.path().join("agora_vfs.dll");
        let custom = dir.path().join("custom").join("vfs32.dll");

        // The override wins over the x86 folder, even where the override's files do not exist.
        let located = locate_dll_x86_from(Some(custom.clone()), &dll64);
        assert_eq!(located, custom);
        assert_eq!(
            missing_x86_files(&located),
            vec![
                custom.clone(),
                dir.path().join("custom").join("agora-vfs-helper.exe")
            ]
        );

        // The default folder is not consulted once an override is set.
        std::fs::create_dir_all(dir.path().join("x86")).unwrap();
        std::fs::write(dir.path().join("x86").join("agora_vfs.dll"), b"x").unwrap();
        assert_eq!(locate_dll_x86_from(Some(custom.clone()), &dll64), custom);
    }

    #[test]
    fn a_configuration_with_aliases_is_version_two_and_names_them() {
        let mut vfs = sample();
        vfs.aliases = vec![PathBuf::from(
            "D:/SteamLibrary/common/Skyrim Special Edition",
        )];
        let json = config_json(&vfs, "Local\\agora-vfs-1-abc");
        assert_eq!(json["version"], 2);
        assert_eq!(
            json["aliases"],
            serde_json::json!(["D:/SteamLibrary/common/Skyrim Special Edition"])
        );
        // The rest is what version 1 carries, unchanged.
        assert_eq!(json["mount"], "D:/Bases/deployments/inst/game");
        assert_eq!(json["lowers"][0], json["mount"]);
    }

    #[test]
    fn the_log_parser_finds_the_program_and_the_reason() {
        let text = [
            r"[10] hooks installed in C:\Games\Game.exe",
            "[agora] injected by the import table",
            r"[42] ending the process C:\Games\Tool: v2\Helper.exe: its hooks could not be installed",
            r"[43] ending the process D:\x\y.exe: the configuration could not be loaded",
            "",
        ]
        .join("\n");
        assert_eq!(
            processes_ended_by_vfs(&text),
            vec![
                EndedProcess {
                    pid: 42,
                    exe: r"C:\Games\Tool: v2\Helper.exe".into(),
                    reason: "its hooks could not be installed".into(),
                },
                EndedProcess {
                    pid: 43,
                    exe: r"D:\x\y.exe".into(),
                    reason: "the configuration could not be loaded".into(),
                },
            ]
        );
    }

    #[test]
    fn the_log_parser_ignores_ordinary_lines_that_mention_ending() {
        let text = [
            "[7] ending the session",
            r"[7] pending the process C:\a.exe: nope",
            r"[agora] ending the process C:\a.exe: nope",
            r"[x] ending the process C:\a.exe: nope",
            r"ending the process C:\a.exe: nope",
            r"[7] hooks installed in a path ending the process C:\a.exe: no",
            "[7] ending the process without a reason",
            "[7] ending the process : ",
            r"[7] ending the process C:\a.exe: ",
            "",
        ]
        .join("\n");
        assert_eq!(processes_ended_by_vfs(&text), Vec::new());
        assert_eq!(processes_ended_by_vfs(""), Vec::new());
    }

    #[test]
    fn only_what_was_appended_since_the_recorded_length_is_read() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("logs").join("vfs.log");
        // No log yet: nothing, and a length of zero.
        assert_eq!(log_len(&log), 0);
        assert!(processes_ended_since(&log, 0).is_empty());

        std::fs::create_dir_all(log.parent().unwrap()).unwrap();
        std::fs::write(
            &log,
            "[1] ending the process C:\\old.exe: its hooks could not be installed\n",
        )
        .unwrap();
        let before = log_len(&log);
        assert!(processes_ended_since(&log, before).is_empty());
        // From the start, the old line is found.
        assert_eq!(processes_ended_since(&log, 0).len(), 1);

        let mut f = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
        f.write_all(
            b"[2] hooks installed in C:\\g.exe\n\
              [3] ending the process C:\\new.exe: the configuration could not be loaded\n",
        )
        .unwrap();
        let ended = processes_ended_since(&log, before);
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].pid, 3);
        assert_eq!(ended[0].exe, r"C:\new.exe");

        // A log that was replaced by a shorter one is all new.
        std::fs::write(&log, "[9] ending the process C:\\z.exe: why\n").unwrap();
        assert_eq!(processes_ended_since(&log, 10_000).len(), 1);
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
