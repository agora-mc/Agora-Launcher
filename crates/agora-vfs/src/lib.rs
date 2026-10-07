//! agora-vfs: Copy-on-write layered virtual file system for Windows game isolation.
//!
//! Hooks the NT file system APIs (`NtCreateFile`, `NtOpenFile`, `NtSetInformationFile`,
//! `NtQueryAttributesFile`, `NtQueryFullAttributesFile`, `NtQueryInformationByName`,
//! `NtQueryDirectoryFile`, `NtQueryDirectoryFileEx`, `NtClose`, `CreateProcessInternalW`)
//! so that operations targeting lower layers (game bases and shared mods) never alter,
//! truncate, or delete real lower files. Writes and edits are copied up to the instance's
//! writable upper layer, deletions are recorded as whiteout markers (`.agvfs-wh\<path>.wh`),
//! and folder enumerations are merged dynamically with whiteouts removed.

#![allow(
    non_snake_case,
    clippy::missing_safety_doc,
    clippy::missing_transmute_annotations,
    clippy::chunks_exact_to_as_chunks
)]

pub mod config;
pub mod paths;

#[cfg(windows)]
pub mod hooks;
#[cfg(windows)]
pub mod listing;
#[cfg(windows)]
pub mod nt;
#[cfg(windows)]
pub mod util;

#[cfg(windows)]
use std::collections::HashMap;
#[cfg(windows)]
use std::ffi::c_void;
#[cfg(windows)]
use std::sync::{Mutex, OnceLock};

#[cfg(windows)]
use windows_sys::Win32::Foundation::*;
#[cfg(windows)]
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenEventW, SetEvent, TerminateProcess, EVENT_MODIFY_STATE,
};

use crate::config::RuntimeConfig;

#[derive(Clone, Debug)]
pub struct Tracked {
    pub rel: String,
    pub lower: bool,
    pub delete_on_close: bool,
    pub dir: bool,
}

static CONFIG: OnceLock<RuntimeConfig> = OnceLock::new();

pub fn maybe_cfg() -> Option<&'static RuntimeConfig> {
    CONFIG.get()
}

pub fn cfg() -> &'static RuntimeConfig {
    CONFIG.get().expect("agora-vfs config")
}

#[cfg(windows)]
static HANDLES: OnceLock<Mutex<HashMap<usize, Tracked>>> = OnceLock::new();

#[cfg(windows)]
pub fn handles() -> &'static Mutex<HashMap<usize, Tracked>> {
    HANDLES.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(windows)]
unsafe fn signal_ready(event_name: &str) -> bool {
    let wide: Vec<u16> = event_name.encode_utf16().chain(Some(0)).collect();
    let ev = OpenEventW(EVENT_MODIFY_STATE, 0, wide.as_ptr());
    if ev.is_null() || ev == INVALID_HANDLE_VALUE {
        util::log(format!(
            "Failed to OpenEventW for ready_event '{event_name}': {}",
            GetLastError()
        ));
        return false;
    }
    let ok = SetEvent(ev) != 0;
    CloseHandle(ev);
    ok
}

/// The export at ordinal 1, which import-table injection (`crates/agora-vfs-inject`) imports by
/// number. It does nothing: importing it is what makes the loader load this DLL.
#[cfg(windows)]
#[no_mangle]
pub extern "system" fn agora_vfs_ordinal1() {}

/// Exit code of a process this DLL ended because it could not protect it.
#[cfg(windows)]
const EXIT_UNPROTECTED: u32 = 0xA6F5_0001;

/// End the process: it was set up to run under the VFS and cannot, and running it unprotected
/// would let it write to shared files. The launcher sees the process end before the ready signal
/// and steps down; for a child process the game started, the game sees it fail to start.
#[cfg(windows)]
unsafe fn end_unprotected(why: &str) -> ! {
    util::log(format!("ending the process: {why}"));
    TerminateProcess(GetCurrentProcess(), EXIT_UNPROTECTED);
    // Not reached: the process is gone.
    std::process::abort()
}

#[cfg(windows)]
#[no_mangle]
pub unsafe extern "system" fn DllMain(
    _module: HANDLE,
    reason: u32,
    _reserved: *mut c_void,
) -> BOOL {
    const DLL_PROCESS_ATTACH: u32 = 1;
    if reason == DLL_PROCESS_ATTACH {
        // Put back the import table that import-table injection rewrote, before the program
        // reads it. A no-op when the DLL was loaded any other way.
        agora_vfs_inject::restore_after_import_injection();
        let Some(config) = config::load_config_from_env() else {
            // No `AGORA_VFS_CONFIG` at all: this process was never meant to run under the VFS
            // (a child started with its own environment block), so install nothing.
            if std::env::var_os("AGORA_VFS_CONFIG").is_none() {
                return 1;
            }
            // A configuration that is named but unreadable or invalid: the process was meant to
            // be protected and cannot be.
            eprintln!("[agora-vfs] the configuration could not be loaded");
            end_unprotected("the configuration could not be loaded");
        };
        let ready_event = config.ready_event.clone();
        let _ = CONFIG.set(config);
        util::set_inside(true);
        let result = hooks::install_hooks();
        util::set_inside(false);
        match result {
            Ok(()) => {
                util::log(format!(
                    "hooks installed in {}",
                    std::env::current_exe()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                ));
                if let Some(event_name) = &ready_event {
                    if signal_ready(event_name) {
                        util::log(format!("Signaled ready event '{event_name}'"));
                    }
                }
            }
            Err(e) => {
                util::log(format!("hook install failed: {e}"));
                end_unprotected("its hooks could not be installed");
            }
        }
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn test_config_parsing_valid() {
        let json = r#"{
            "version": 1,
            "mount": "D:\\Game\\mount\\",
            "upper": "C:\\Instances\\upper",
            "lowers": ["D:\\Game\\mount\\", "D:\\Game\\base"],
            "dll": "C:\\Agora\\agora_vfs.dll",
            "log": "C:\\Instances\\vfs.log",
            "verbose": true,
            "ready_event": "Local\\agora-vfs-test"
        }"#;

        let cfg = config::VfsConfig::from_json_str(json).expect("valid config");
        assert_eq!(cfg.version, 1);
        assert_eq!(cfg.mount, "D:\\Game\\mount");
        assert_eq!(cfg.mount_original, "D:\\Game\\mount");
        assert_eq!(cfg.upper, PathBuf::from("C:\\Instances\\upper"));
        assert_eq!(cfg.lowers.len(), 2);
        assert_eq!(cfg.lowers[0], PathBuf::from("D:\\Game\\mount"));
        assert_eq!(cfg.lowers[1], PathBuf::from("D:\\Game\\base"));
        assert!(cfg.verbose);
        assert_eq!(cfg.ready_event.as_deref(), Some("Local\\agora-vfs-test"));
    }

    #[test]
    fn test_config_slash_normalization() {
        let json = r#"{
            "version": 1,
            "mount": "D:/Game/mount/",
            "upper": "C:/Instances/upper/",
            "lowers": ["D:/Game/mount/", "D:/Game/base/modA/"],
            "dll": "C:/Agora/agora_vfs.dll/",
            "log": "C:/Instances/vfs.log/"
        }"#;

        let cfg = config::VfsConfig::from_json_str(json).expect("valid config");
        assert_eq!(cfg.mount, "D:\\Game\\mount");
        assert_eq!(cfg.upper, PathBuf::from("C:\\Instances\\upper"));
        assert_eq!(cfg.lowers[0], PathBuf::from("D:\\Game\\mount"));
        assert_eq!(cfg.lowers[1], PathBuf::from("D:\\Game\\base\\modA"));
        assert_eq!(cfg.dll, PathBuf::from("C:\\Agora\\agora_vfs.dll"));
        assert_eq!(cfg.log, Some(PathBuf::from("C:\\Instances\\vfs.log")));
    }

    #[test]
    fn test_config_parsing_rejects_version_2() {
        let json = r#"{
            "version": 2,
            "mount": "D:\\Game\\mount",
            "upper": "C:\\Instances\\upper",
            "dll": "C:\\Agora\\agora_vfs.dll"
        }"#;
        let err = config::VfsConfig::from_json_str(json).unwrap_err();
        assert!(err.contains("Unsupported config version"));
    }

    #[test]
    fn test_config_parsing_rejects_empty_mount() {
        let json = r#"{
            "version": 1,
            "mount": "   ",
            "upper": "C:\\Instances\\upper",
            "dll": "C:\\Agora\\agora_vfs.dll"
        }"#;
        let err = config::VfsConfig::from_json_str(json).unwrap_err();
        assert!(err.contains("Mount path cannot be empty"));
    }

    #[test]
    fn test_path_rel_of() {
        let cfg = config::RuntimeConfig {
            version: 1,
            mount: "D:\\Games\\Skyrim".to_string(),
            mount_original: "D:\\Games\\Skyrim".to_string(),
            mount_len: "D:\\Games\\Skyrim".len(),
            upper: PathBuf::from("C:\\upper"),
            lowers: vec![],
            dll: PathBuf::from("C:\\agora.dll"),
            log: None,
            verbose: false,
            ready_event: None,
        };

        // Exact mount match
        assert_eq!(
            paths::rel_of("D:\\Games\\Skyrim", &cfg),
            Some(String::new())
        );
        assert_eq!(
            paths::rel_of("\\??\\D:\\Games\\Skyrim", &cfg),
            Some(String::new())
        );
        assert_eq!(
            paths::rel_of("d:\\games\\skyrim", &cfg),
            Some(String::new())
        );
        assert_eq!(
            paths::rel_of("d:\\games\\skyrim\\", &cfg),
            Some(String::new())
        );

        // Children
        assert_eq!(
            paths::rel_of("D:\\Games\\Skyrim\\Data\\test.esp", &cfg),
            Some("Data\\test.esp".to_string())
        );
        assert_eq!(
            paths::rel_of("\\??\\D:\\Games\\Skyrim\\Data\\test.esp", &cfg),
            Some("Data\\test.esp".to_string())
        );
        assert_eq!(
            paths::rel_of("d:\\games\\skyrim\\Data\\Folder\\", &cfg),
            Some("Data\\Folder".to_string())
        );

        // Outside mount
        assert_eq!(paths::rel_of("D:\\Games\\Other\\file.txt", &cfg), None);
        assert_eq!(paths::rel_of("C:\\Windows\\system32", &cfg), None);
        assert_eq!(
            paths::rel_of("D:\\Games\\SkyrimPrefix\\file.txt", &cfg),
            None
        );
    }

    #[test]
    fn test_whiteout_path() {
        let upper = Path::new("C:\\Upper");
        let wh = paths::whiteout_path("Data\\foo.txt", upper);
        assert_eq!(wh, PathBuf::from("C:\\Upper\\.agvfs-wh\\Data\\foo.txt.wh"));
    }

    #[test]
    fn test_wildcard_matching() {
        #[cfg(windows)]
        use listing::wildcard;

        #[cfg(not(windows))]
        fn wildcard(pattern: &str, name: &str) -> bool {
            fn go(p: &[char], n: &[char]) -> bool {
                match (p.first(), n.first()) {
                    (None, None) => true,
                    (Some('*'), _) | (Some('<'), _) => {
                        go(&p[1..], n) || (!n.is_empty() && go(p, &n[1..]))
                    }
                    (Some('?'), Some(_)) | (Some('>'), Some(_)) => go(&p[1..], &n[1..]),
                    (Some('"'), Some('.')) => go(&p[1..], &n[1..]),
                    (Some(a), Some(b)) => a.eq_ignore_ascii_case(b) && go(&p[1..], &n[1..]),
                    (Some(_), None) => p.iter().all(|c| matches!(c, '*' | '<' | '>' | '"')),
                    (None, Some(_)) => false,
                }
            }
            let p: Vec<char> = pattern.chars().collect();
            let n: Vec<char> = name.chars().collect();
            go(&p, &n)
        }

        assert!(wildcard("*", "anything.txt"));
        assert!(wildcard("*.txt", "foo.TXT"));
        assert!(wildcard("foo?.txt", "foo1.txt"));
        assert!(!wildcard("foo?.txt", "foo12.txt"));
        assert!(wildcard("foo<txt", "foo.txt")); // DOS wildcard < matches . or *
        assert!(wildcard("foo\".txt", "foo..txt"));
        assert!(!wildcard("*.esp", "foo.esm"));
    }
}
