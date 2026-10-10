use std::path::Path;

/// Read a Windows executable or DLL's file version from its `VS_FIXEDFILEINFO` resource.
///
/// Returns `Some("major.minor.build.revision")` on Windows when the binary has a version resource,
/// or `None` on other platforms or if reading fails.
#[cfg(windows)]
pub fn read_file_version(path: &Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW, VS_FIXEDFILEINFO,
    };

    let wide_path: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut dummy: u32 = 0;
    let size = unsafe { GetFileVersionInfoSizeW(wide_path.as_ptr(), &mut dummy) };
    if size == 0 || size > 16 * 1024 * 1024 {
        return None;
    }

    let mut buffer = vec![0u8; size as usize];
    let ok =
        unsafe { GetFileVersionInfoW(wide_path.as_ptr(), 0, size, buffer.as_mut_ptr() as *mut _) };
    if ok == 0 {
        return None;
    }

    let sub_block: Vec<u16> = "\\\0".encode_utf16().collect();
    let mut lp_buffer: *mut core::ffi::c_void = std::ptr::null_mut();
    let mut len: u32 = 0;

    let ok = unsafe {
        VerQueryValueW(
            buffer.as_ptr() as *const _,
            sub_block.as_ptr(),
            &mut lp_buffer,
            &mut len,
        )
    };
    if ok == 0 || lp_buffer.is_null() || (len as usize) < std::mem::size_of::<VS_FIXEDFILEINFO>() {
        return None;
    }

    // The pointer is into `buffer`, which a byte vector does not align.
    let info = unsafe { std::ptr::read_unaligned(lp_buffer as *const VS_FIXEDFILEINFO) };
    if info.dwSignature != 0xFEEF_04BD {
        return None;
    }
    let major = (info.dwFileVersionMS >> 16) & 0xffff;
    let minor = info.dwFileVersionMS & 0xffff;
    let build = (info.dwFileVersionLS >> 16) & 0xffff;
    let revision = info.dwFileVersionLS & 0xffff;

    Some(format!("{major}.{minor}.{build}.{revision}"))
}

#[cfg(not(windows))]
pub fn read_file_version(_path: &Path) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_file_version_system_kernel32() {
        #[cfg(windows)]
        {
            let path = Path::new(r"C:\Windows\System32\kernel32.dll");
            if path.exists() {
                let ver = read_file_version(path);
                assert!(ver.is_some(), "expected file version for kernel32.dll");
                let ver = ver.unwrap();
                let parts: Vec<&str> = ver.split('.').collect();
                assert_eq!(
                    parts.len(),
                    4,
                    "version should have 4 dot-separated parts: {ver}"
                );
                for part in parts {
                    assert!(
                        part.parse::<u32>().is_ok(),
                        "part {part} should be a number"
                    );
                }
            }
        }
        #[cfg(not(windows))]
        {
            assert!(read_file_version(Path::new("/bin/ls")).is_none());
        }
    }
}
