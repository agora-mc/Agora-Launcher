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

/// The VFS's JSON configuration (`crates/agora-vfs/README.md`).
#[cfg_attr(not(windows), allow(dead_code))]
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
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use std::time::Duration;

    use agora_vfs_inject::{Failure, Method};
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    use windows_sys::Win32::System::Threading::{
        CreateEventW, GetExitCodeProcess, WaitForMultipleObjects, WaitForSingleObject,
        CREATE_SUSPENDED,
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
        match unsafe { inject_and_resume(process, vfs, event.0) } {
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
    ) -> Result<Method, String> {
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
