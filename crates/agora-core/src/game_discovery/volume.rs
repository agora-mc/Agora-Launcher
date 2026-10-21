use agora_game_api::VolumeInfo;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Default)]
pub struct VolumeDetector {
    // Only the Windows lookup caches.
    #[cfg_attr(not(windows), allow(dead_code))]
    cache: Mutex<HashMap<PathBuf, Option<VolumeInfo>>>,
}

impl VolumeDetector {
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn get_volume_info(&self, path: &Path) -> Option<VolumeInfo> {
        #[cfg(windows)]
        {
            self.get_windows_volume_info(path)
        }
        #[cfg(unix)]
        {
            self.get_unix_volume_info(path)
        }
        #[cfg(not(any(windows, unix)))]
        {
            let _ = path;
            None
        }
    }

    #[cfg(windows)]
    fn get_windows_volume_info(&self, path: &Path) -> Option<VolumeInfo> {
        use std::os::windows::ffi::OsStrExt;

        // 1. Get volume path name
        let volume_root = get_volume_mount_root(path)?;

        // Check cache
        if let Ok(guard) = self.cache.lock() {
            if let Some(cached) = guard.get(&volume_root) {
                return cached.clone();
            }
        }

        // 2. Query volume information
        let wide_volume_root: Vec<u16> = volume_root
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut serial_number: u32 = 0;
        let mut max_component_len: u32 = 0;
        let mut flags: u32 = 0;
        let mut fs_name_buf = [0u16; 260];

        let ok = unsafe {
            windows_sys::Win32::Storage::FileSystem::GetVolumeInformationW(
                wide_volume_root.as_ptr(),
                std::ptr::null_mut(),
                0,
                &mut serial_number,
                &mut max_component_len,
                &mut flags,
                fs_name_buf.as_mut_ptr(),
                fs_name_buf.len() as u32,
            )
        };

        let result = if ok != 0 {
            let fs_name_len = fs_name_buf
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(fs_name_buf.len());
            let fs_name = String::from_utf16_lossy(&fs_name_buf[..fs_name_len]);

            // FILE_SUPPORTS_HARD_LINKS = 0x0040_0000
            // FILE_SUPPORTS_BLOCK_REFCOUNTING = 0x0800_0000
            let supports_hardlinks = (flags & 0x0040_0000) != 0;
            let supports_file_clones = (flags & 0x0800_0000) != 0;

            Some(VolumeInfo {
                id: format!("{:08X}", serial_number),
                filesystem: fs_name,
                supports_hardlinks,
                supports_file_clones,
            })
        } else {
            None
        };

        if let Ok(mut guard) = self.cache.lock() {
            guard.insert(volume_root, result.clone());
        }

        result
    }

    #[cfg(unix)]
    fn get_unix_volume_info(&self, path: &Path) -> Option<VolumeInfo> {
        use std::os::unix::fs::MetadataExt;
        // A folder Agora has yet to create (the bases root before the first build) lives on the
        // volume of its nearest existing ancestor, as GetVolumePathNameW answers on Windows.
        let metadata = path.ancestors().find_map(|p| std::fs::metadata(p).ok())?;
        let dev = metadata.dev();

        Some(VolumeInfo {
            id: format!("{:x}", dev),
            filesystem: "unknown".to_string(),
            supports_hardlinks: true,
            supports_file_clones: false,
        })
    }
}

/// Query the volume mount root for a path (e.g. `C:\` or `D:\` on Windows via GetVolumePathNameW).
pub fn get_volume_mount_root(path: &Path) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};

        let wide_path: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let mut volume_path_buf = [0u16; 512];
        let ok = unsafe {
            windows_sys::Win32::Storage::FileSystem::GetVolumePathNameW(
                wide_path.as_ptr(),
                volume_path_buf.as_mut_ptr(),
                volume_path_buf.len() as u32,
            )
        };
        if ok == 0 {
            return None;
        }

        let volume_path_len = volume_path_buf
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(volume_path_buf.len());
        Some(PathBuf::from(std::ffi::OsString::from_wide(
            &volume_path_buf[..volume_path_len],
        )))
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        None
    }
}
