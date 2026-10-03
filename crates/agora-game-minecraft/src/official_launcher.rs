//! Detecting and closing the official Minecraft Launcher.
//!
//! The launcher reads `launcher_profiles.json` once at startup and restores its
//! installation selection from its own UI-state file at the same moment. While
//! it is running it ignores both files and rewrites them from memory, so a
//! delegated launch into an already-open launcher cannot select anything —
//! the launcher is single-instance and a second invocation only focuses the
//! existing window.
//!
//! That makes "is it already running?" a launch-time decision rather than a
//! diagnostic, which is why it lives in core next to the profile writers.
//! Deciding what to do about it (prompt, close, hand off anyway) stays with the
//! adapter that owns the user interaction.

use std::path::Path;
use std::time::{Duration, Instant};
use sysinfo::{Pid, ProcessRefreshKind, RefreshKind, System};

/// Executable names the official launcher ships under, by platform. The
/// Microsoft Store build is `Minecraft.exe`; the standalone Windows installer
/// ships `MinecraftLauncher.exe`.
#[cfg(target_os = "windows")]
const LAUNCHER_EXE_NAMES: &[&str] = &["MinecraftLauncher.exe", "Minecraft.exe"];
#[cfg(target_os = "linux")]
const LAUNCHER_EXE_NAMES: &[&str] = &["minecraft-launcher"];
// On macOS the binary is just `launcher`, which is far too generic to match on
// its own — those processes are recognized by their bundle path instead.
#[cfg(target_os = "macos")]
const LAUNCHER_EXE_NAMES: &[&str] = &[];

/// Every PID that belongs to the official launcher, including the browser
/// subprocesses it spawns from the same executable.
///
/// `expected_exe` is the launcher path the adapter resolved. When present it is
/// matched exactly, which avoids acting on an unrelated program that happens to
/// share a name; without it the platform name list is used.
pub fn running_pids(expected_exe: Option<&Path>) -> Vec<u32> {
    let system =
        System::new_with_specifics(RefreshKind::new().with_processes(ProcessRefreshKind::new()));
    let own_pid = std::process::id();
    system
        .processes()
        .iter()
        .filter(|(pid, process)| {
            pid.as_u32() != own_pid
                && is_official_launcher(process.name(), process.exe(), expected_exe)
        })
        .map(|(pid, _)| pid.as_u32())
        .collect()
}

/// Whether the official launcher currently owns `launcher_profiles.json`.
pub fn is_running(expected_exe: Option<&Path>) -> bool {
    !running_pids(expected_exe).is_empty()
}

fn is_official_launcher(name: &str, exe: Option<&Path>, expected_exe: Option<&Path>) -> bool {
    if let (Some(exe), Some(expected)) = (exe, expected_exe) {
        if paths_match(exe, expected) {
            return true;
        }
    }
    if cfg!(target_os = "macos") {
        return exe.is_some_and(|exe| {
            exe.to_string_lossy()
                .replace('\\', "/")
                .ends_with("Minecraft.app/Contents/MacOS/launcher")
        });
    }
    LAUNCHER_EXE_NAMES
        .iter()
        .any(|candidate| name.eq_ignore_ascii_case(candidate))
}

fn paths_match(left: &Path, right: &Path) -> bool {
    let normalize = |path: &Path| path.to_string_lossy().replace('\\', "/").to_lowercase();
    normalize(left) == normalize(right)
}

/// Ask the official launcher to exit and wait for it to be gone.
///
/// Returns `true` once no launcher process remains, `false` if any survived the
/// timeout. Callers must treat a `false` as "do not write the profile files" —
/// a launcher still holding them will overwrite whatever Agora puts there.
///
/// This terminates rather than negotiates: the launcher's in-memory copy of
/// `launcher_profiles.json` is exactly what Agora needs it *not* to write back,
/// so a clean shutdown would undo the handoff it is being restarted for. The
/// adapter must confirm with the user before calling this.
pub fn close_running(expected_exe: Option<&Path>, timeout: Duration) -> bool {
    let mut system =
        System::new_with_specifics(RefreshKind::new().with_processes(ProcessRefreshKind::new()));
    let own_pid = std::process::id();
    for (pid, process) in system.processes() {
        if pid.as_u32() != own_pid
            && is_official_launcher(process.name(), process.exe(), expected_exe)
        {
            process.kill();
        }
    }

    let deadline = Instant::now() + timeout;
    loop {
        system.refresh_processes();
        let alive = system.processes().iter().any(|(pid, process)| {
            pid.as_u32() != own_pid
                && is_official_launcher(process.name(), process.exe(), expected_exe)
        });
        if !alive {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Whether `pid` is still alive — used by adapters that spawned the launcher
/// themselves and want to confirm the handoff took.
pub fn is_alive(pid: u32) -> bool {
    let mut system = System::new();
    system.refresh_process(Pid::from_u32(pid));
    system.process(Pid::from_u32(pid)).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn matches_the_resolved_launcher_path_case_and_separator_insensitively() {
        let expected = PathBuf::from("C:\\Program Files\\Minecraft Launcher\\Minecraft.exe");
        assert!(is_official_launcher(
            "Minecraft.exe",
            Some(Path::new(
                "c:/program files/minecraft launcher/minecraft.exe"
            )),
            Some(&expected),
        ));
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn matches_known_windows_executable_names_without_a_resolved_path() {
        assert!(is_official_launcher("MinecraftLauncher.exe", None, None));
        assert!(is_official_launcher("minecraft.exe", None, None));
        assert!(!is_official_launcher("javaw.exe", None, None));
        assert!(!is_official_launcher("agora-desktop.exe", None, None));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn matches_only_the_launcher_bundle_on_macos() {
        assert!(is_official_launcher(
            "launcher",
            Some(Path::new(
                "/Applications/Minecraft.app/Contents/MacOS/launcher"
            )),
            None,
        ));
        assert!(!is_official_launcher(
            "launcher",
            Some(Path::new("/usr/local/bin/launcher")),
            None,
        ));
    }

    #[test]
    fn never_reports_our_own_process() {
        // A pattern that matches this test binary must still not come back:
        // killing ourselves mid-launch would be the worst possible outcome.
        let own_exe = std::env::current_exe().unwrap();
        assert!(!running_pids(Some(&own_exe)).contains(&std::process::id()));
    }
}
