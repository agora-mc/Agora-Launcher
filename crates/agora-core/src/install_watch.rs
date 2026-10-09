//! Watches a game's real install folder while a tool runs (MASTER_SPEC §26.9).
//!
//! Link capture compares the farm with its record, so a tool that writes into the real install
//! (the real Nemesis created an empty `Data\meshes` there) is invisible to it. This watch reports
//! every file or folder created, changed or deleted under the install folder while the tool runs.
//! It is a report, never a refusal: one background thread holds a recursive
//! `ReadDirectoryChangesW` from just before the tool starts until it exits, and it adds no time to
//! the run. If the kernel's buffer overflows and events are lost, the report says so.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;

use serde::{Deserialize, Serialize};

/// What changed under the real install folder during a run. Paths are relative to the install
/// folder and `/`-separated.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallChanges {
    pub created: Vec<String>,
    pub changed: Vec<String>,
    pub deleted: Vec<String>,
    /// Events were lost (the watch's buffer overflowed, or the watch stopped early), so the lists
    /// may be incomplete: "changes may have been missed".
    #[serde(default)]
    pub may_be_incomplete: bool,
    /// Set when the folder could not be watched at all, with the reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable: Option<String>,
}

impl InstallChanges {
    /// True when nothing was reported.
    pub fn is_empty(&self) -> bool {
        self.created.is_empty() && self.changed.is_empty() && self.deleted.is_empty()
    }
}

/// A running watch of an install folder. Finish it when the tool has exited.
pub struct InstallWatch {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<InstallChanges>>,
}

impl InstallWatch {
    /// Start watching `root` (recursively). Returns once the watch is armed, so a change made after
    /// this call is seen.
    pub fn start(root: &Path) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::channel::<()>();
        let root = root.to_path_buf();
        let flag = Arc::clone(&stop);
        let thread = std::thread::spawn(move || watch(&root, &flag, ready_tx));
        // The thread sends once its first request is issued, or drops its sender when it cannot
        // start; either way this returns at once.
        let _ = ready_rx.recv();
        Self {
            stop,
            thread: Some(thread),
        }
    }

    /// Stop watching and return what changed. Events already queued are collected first.
    pub fn finish(mut self) -> InstallChanges {
        self.stop.store(true, Ordering::SeqCst);
        match self.thread.take() {
            Some(handle) => handle
                .join()
                .unwrap_or_else(|_| unavailable("the watch thread stopped unexpectedly".into())),
            None => InstallChanges::default(),
        }
    }
}

impl Drop for InstallWatch {
    fn drop(&mut self) {
        if let Some(handle) = self.thread.take() {
            self.stop.store(true, Ordering::SeqCst);
            let _ = handle.join();
        }
    }
}

fn unavailable(reason: String) -> InstallChanges {
    InstallChanges {
        may_be_incomplete: true,
        unavailable: Some(reason),
        ..InstallChanges::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Added,
    Modified,
    Removed,
}

#[cfg(not(windows))]
fn watch(_root: &Path, _stop: &AtomicBool, ready: mpsc::Sender<()>) -> InstallChanges {
    drop(ready);
    unavailable("watching the real install folder needs Windows".into())
}

#[cfg(windows)]
fn watch(root: &Path, stop: &AtomicBool, ready: mpsc::Sender<()>) -> InstallChanges {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{
        CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
    use windows_sys::Win32::System::IO::{CancelIo, GetOverlappedResult};

    let wide: Vec<u16> = root
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let dir: HANDLE = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_LIST_DIRECTORY,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
            std::ptr::null_mut(),
        )
    };
    if dir == INVALID_HANDLE_VALUE || dir.is_null() {
        return unavailable(format!(
            "could not open {} for watching: {}",
            root.display(),
            std::io::Error::last_os_error()
        ));
    }
    // An automatic-reset event, not signalled: a completed read signals it.
    let event: HANDLE = unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) };
    if event.is_null() {
        unsafe { CloseHandle(dir) };
        return unavailable(format!(
            "could not create the watch event: {}",
            std::io::Error::last_os_error()
        ));
    }

    // 64 KB, the largest buffer the call accepts. Kept as u32s so it is DWORD-aligned.
    let mut state = Watch {
        dir,
        event,
        buf: vec![0u32; 16 * 1024],
        overlapped: unsafe { std::mem::zeroed() },
        returned: 0,
        events: BTreeMap::new(),
        overflow: false,
        error: None,
    };
    if !state.issue(FILTER) {
        unsafe {
            CloseHandle(event);
            CloseHandle(dir);
        }
        return unavailable(format!(
            "could not start watching {}: {}",
            root.display(),
            std::io::Error::last_os_error()
        ));
    }
    let _ = ready.send(());
    drop(ready);

    loop {
        let stopping = stop.load(Ordering::SeqCst);
        match unsafe { WaitForSingleObject(event, if stopping { 0 } else { 50 }) } {
            WAIT_OBJECT_0 => {
                if !state.collect() || !state.issue(FILTER) {
                    break;
                }
            }
            WAIT_TIMEOUT if stopping => break,
            WAIT_TIMEOUT => {}
            _ => {
                state.error = Some(format!(
                    "the watch of {} failed: {}",
                    root.display(),
                    std::io::Error::last_os_error()
                ));
                break;
            }
        }
    }

    // Cancel the outstanding request and wait for the cancellation to land before the buffer goes.
    unsafe { CancelIo(dir) };
    let mut transferred = 0u32;
    unsafe { GetOverlappedResult(dir, &state.overlapped, &mut transferred, 1) };
    unsafe {
        CloseHandle(event);
        CloseHandle(dir);
    }
    state.into_changes(root)
}

#[cfg(windows)]
const FILTER: u32 = {
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE,
        FILE_NOTIFY_CHANGE_SIZE,
    };
    FILE_NOTIFY_CHANGE_FILE_NAME
        | FILE_NOTIFY_CHANGE_DIR_NAME
        | FILE_NOTIFY_CHANGE_SIZE
        | FILE_NOTIFY_CHANGE_LAST_WRITE
};

/// The state of one watch, owned by its thread.
#[cfg(windows)]
struct Watch {
    dir: windows_sys::Win32::Foundation::HANDLE,
    event: windows_sys::Win32::Foundation::HANDLE,
    buf: Vec<u32>,
    overlapped: windows_sys::Win32::System::IO::OVERLAPPED,
    /// The bytes-returned slot an overlapped call ignores; it must still be valid memory.
    returned: u32,
    /// Each path's first and last event kind during the run.
    events: BTreeMap<String, (Kind, Kind)>,
    overflow: bool,
    error: Option<String>,
}

#[cfg(windows)]
impl Watch {
    /// Issue the next directory read. False when the kernel refused it.
    fn issue(&mut self, filter: u32) -> bool {
        self.overlapped = unsafe { std::mem::zeroed() };
        self.overlapped.hEvent = self.event;
        let ok = unsafe {
            windows_sys::Win32::Storage::FileSystem::ReadDirectoryChangesW(
                self.dir,
                self.buf.as_mut_ptr().cast(),
                (self.buf.len() * std::mem::size_of::<u32>()) as u32,
                1,
                filter,
                &mut self.returned,
                &mut self.overlapped,
                None,
            )
        };
        if ok == 0 {
            self.error = Some(format!(
                "the watch could not continue: {}",
                std::io::Error::last_os_error()
            ));
            return false;
        }
        true
    }

    /// Read the completed request. False when the watch has failed and must stop.
    fn collect(&mut self) -> bool {
        let mut transferred = 0u32;
        let ok = unsafe {
            windows_sys::Win32::System::IO::GetOverlappedResult(
                self.dir,
                &self.overlapped,
                &mut transferred,
                0,
            )
        };
        if ok == 0 {
            self.error = Some(format!(
                "the watch could not read its events: {}",
                std::io::Error::last_os_error()
            ));
            return false;
        }
        if transferred == 0 {
            // A zero-byte completion is the kernel saying its buffer overflowed.
            self.overflow = true;
            return true;
        }
        let len = (transferred as usize).min(self.buf.len() * 4);
        let bytes = unsafe { std::slice::from_raw_parts(self.buf.as_ptr().cast::<u8>(), len) };
        self.parse(bytes);
        true
    }

    /// Walk a `FILE_NOTIFY_INFORMATION` list: next offset, action, name length in bytes, UTF-16 name.
    fn parse(&mut self, bytes: &[u8]) {
        let u32_at = |at: usize| -> Option<u32> {
            bytes
                .get(at..at + 4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        let mut off = 0usize;
        while let (Some(next), Some(action), Some(name_len)) =
            (u32_at(off), u32_at(off + 4), u32_at(off + 8))
        {
            let start = off + 12;
            let Some(end) = start.checked_add(name_len as usize) else {
                break;
            };
            let Some(raw) = bytes.get(start..end) else {
                break;
            };
            let (units, _) = raw.as_chunks::<2>();
            let units: Vec<u16> = units.iter().map(|c| u16::from_le_bytes(*c)).collect();
            let name = String::from_utf16_lossy(&units).replace('\\', "/");
            let kind = match action {
                1 | 5 => Some(Kind::Added),   // ADDED, RENAMED_NEW_NAME
                2 | 4 => Some(Kind::Removed), // REMOVED, RENAMED_OLD_NAME
                3 => Some(Kind::Modified),    // MODIFIED
                _ => None,
            };
            if let (Some(kind), false) = (kind, name.is_empty()) {
                self.events
                    .entry(name)
                    .and_modify(|e| e.1 = kind)
                    .or_insert((kind, kind));
            }
            if next == 0 {
                break;
            }
            off += next as usize;
        }
    }

    fn into_changes(self, root: &Path) -> InstallChanges {
        let mut out = InstallChanges {
            may_be_incomplete: self.overflow || self.error.is_some(),
            unavailable: None,
            ..InstallChanges::default()
        };
        for (path, (first, last)) in self.events {
            match (first, last) {
                (_, Kind::Removed) => out.deleted.push(path),
                (Kind::Added, _) => out.created.push(path),
                _ => {
                    // A folder whose contents changed is not itself a change; its children are.
                    if root.join(&path).is_dir() {
                        continue;
                    }
                    out.changed.push(path);
                }
            }
        }
        if let Some(error) = self.error {
            out.unavailable = Some(error);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_report_says_so() {
        assert!(InstallChanges::default().is_empty());
        let one = InstallChanges {
            created: vec!["Data/x.nif".into()],
            ..InstallChanges::default()
        };
        assert!(!one.is_empty());
    }
}
