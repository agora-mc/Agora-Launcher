//! Protection for content store objects (MASTER_SPEC §26.5, §26.6).
//!
//! Objects are protected from modification; folders are not (the content store
//! must stay writable to Agora).
//!
//! Note: this guards against modification, not deletion: deleting a file needs
//! `DELETE` on it or delete-child on its folder, and the data dir's folders grant
//! the user Full Control. That is the measured, intended scope.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protection {
    Protected,
    Unprotected,
    Unsupported,
}

#[cfg(windows)]
mod imp {
    use super::Protection;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE};
    use windows_sys::Win32::Security::Authorization::{
        GetNamedSecurityInfoW, SetEntriesInAclW, SetNamedSecurityInfoW, DENY_ACCESS,
        EXPLICIT_ACCESS_W, GRANT_ACCESS, SE_FILE_OBJECT, TRUSTEE_IS_SID, TRUSTEE_IS_USER,
    };
    use windows_sys::Win32::Security::{
        CopySid, DeleteAce, EqualSid, GetAce, GetLengthSid, GetTokenInformation, TokenUser,
        ACCESS_DENIED_ACE, ACE_HEADER, ACL, CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION,
        NO_INHERITANCE, PSECURITY_DESCRIPTOR, PSID, TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    pub const ACCESS_DENIED_ACE_TYPE: u8 = 1;

    /// FILE_WRITE_DATA (0x0002) | FILE_APPEND_DATA (0x0004) | FILE_WRITE_EA (0x0010) | DELETE (0x00010000)
    pub const DENY_MASK: u32 = 0x0001_0016;

    /// Get the current process user SID as an owned byte buffer.
    pub fn get_current_user_sid() -> Result<Vec<u8>, std::io::Error> {
        let mut token: HANDLE = std::ptr::null_mut();
        let ok = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }

        let mut returned_len = 0u32;
        unsafe {
            GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut returned_len);
        }

        // u64 elements: TOKEN_USER holds a pointer, so the buffer must be pointer-aligned for the
        // cast below (a Vec<u8> only guarantees byte alignment).
        let mut buf = vec![0u64; (returned_len as usize).div_ceil(8)];
        let ok = unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                buf.as_mut_ptr() as _,
                returned_len,
                &mut returned_len,
            )
        };
        unsafe { CloseHandle(token) };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }

        let token_user = unsafe { &*(buf.as_ptr() as *const TOKEN_USER) };
        let sid = token_user.User.Sid;
        let sid_len = unsafe { GetLengthSid(sid) };
        let mut sid_bytes = vec![0u8; sid_len as usize];
        let ok = unsafe { CopySid(sid_len, sid_bytes.as_mut_ptr() as _, sid) };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(sid_bytes)
    }

    fn to_wide_null(path: &Path) -> Vec<u16> {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    pub fn protect(path: &Path) -> Result<(), std::io::Error> {
        if protection(path)? == Protection::Protected {
            return Ok(());
        }

        let user_sid = get_current_user_sid()?;
        let wide_path = to_wide_null(path);

        let mut p_sec_desc: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        let mut p_dacl: *mut ACL = std::ptr::null_mut();

        let ret = unsafe {
            GetNamedSecurityInfoW(
                wide_path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut p_dacl,
                std::ptr::null_mut(),
                &mut p_sec_desc,
            )
        };
        if ret != 0 {
            return Err(std::io::Error::from_raw_os_error(ret as i32));
        }

        let mut ea: EXPLICIT_ACCESS_W = unsafe { std::mem::zeroed() };
        ea.grfAccessPermissions = DENY_MASK;
        ea.grfAccessMode = DENY_ACCESS;
        ea.grfInheritance = NO_INHERITANCE;
        ea.Trustee.pMultipleTrustee = std::ptr::null_mut();
        ea.Trustee.MultipleTrusteeOperation = 0;
        ea.Trustee.TrusteeForm = TRUSTEE_IS_SID;
        ea.Trustee.TrusteeType = TRUSTEE_IS_USER;
        ea.Trustee.ptstrName = user_sid.as_ptr() as *mut u16;

        let mut p_new_dacl: *mut ACL = std::ptr::null_mut();
        let ret = unsafe { SetEntriesInAclW(1, &ea, p_dacl, &mut p_new_dacl) };
        if ret != 0 {
            unsafe { LocalFree(p_sec_desc as _) };
            return Err(std::io::Error::from_raw_os_error(ret as i32));
        }

        let ret = unsafe {
            SetNamedSecurityInfoW(
                wide_path.as_ptr() as *mut u16,
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                p_new_dacl,
                std::ptr::null_mut(),
            )
        };

        unsafe {
            LocalFree(p_new_dacl as _);
            LocalFree(p_sec_desc as _);
        }

        if ret != 0 {
            return Err(std::io::Error::from_raw_os_error(ret as i32));
        }

        Ok(())
    }

    pub fn unprotect(path: &Path) -> Result<(), std::io::Error> {
        let user_sid = get_current_user_sid()?;
        let wide_path = to_wide_null(path);

        let mut p_sec_desc: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        let mut p_dacl: *mut ACL = std::ptr::null_mut();

        let ret = unsafe {
            GetNamedSecurityInfoW(
                wide_path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut p_dacl,
                std::ptr::null_mut(),
                &mut p_sec_desc,
            )
        };
        if ret != 0 {
            return Err(std::io::Error::from_raw_os_error(ret as i32));
        }

        if p_dacl.is_null() {
            unsafe { LocalFree(p_sec_desc as _) };
            return Ok(());
        }

        let mut modified = false;
        let count = unsafe { (*p_dacl).AceCount };
        for i in (0..count).rev() {
            let mut p_ace: *mut std::ffi::c_void = std::ptr::null_mut();
            if unsafe { GetAce(p_dacl, i as u32, &mut p_ace) } != 0 {
                let header = unsafe { &*(p_ace as *const ACE_HEADER) };
                if header.AceType == ACCESS_DENIED_ACE_TYPE {
                    let deny_ace = unsafe { &*(p_ace as *const ACCESS_DENIED_ACE) };
                    let ace_sid = &deny_ace.SidStart as *const u32 as PSID;
                    if unsafe { EqualSid(ace_sid, user_sid.as_ptr() as PSID) } != 0
                        && deny_ace.Mask == DENY_MASK
                    {
                        unsafe { DeleteAce(p_dacl, i as u32) };
                        modified = true;
                    }
                }
            }
        }

        if modified {
            let ret = unsafe {
                SetNamedSecurityInfoW(
                    wide_path.as_ptr() as *mut u16,
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    p_dacl,
                    std::ptr::null_mut(),
                )
            };
            unsafe { LocalFree(p_sec_desc as _) };
            if ret != 0 {
                return Err(std::io::Error::from_raw_os_error(ret as i32));
            }
        } else {
            unsafe { LocalFree(p_sec_desc as _) };
        }

        Ok(())
    }

    pub fn protection(path: &Path) -> Result<Protection, std::io::Error> {
        let user_sid = get_current_user_sid()?;
        let wide_path = to_wide_null(path);

        let mut p_sec_desc: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        let mut p_dacl: *mut ACL = std::ptr::null_mut();

        let ret = unsafe {
            GetNamedSecurityInfoW(
                wide_path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut p_dacl,
                std::ptr::null_mut(),
                &mut p_sec_desc,
            )
        };
        if ret != 0 {
            // ERROR_NOT_SUPPORTED (50), ERROR_INVALID_FUNCTION (1)
            if ret == 50 || ret == 1 {
                return Ok(Protection::Unsupported);
            }
            return Err(std::io::Error::from_raw_os_error(ret as i32));
        }

        if p_dacl.is_null() {
            unsafe { LocalFree(p_sec_desc as _) };
            return Ok(Protection::Unprotected);
        }

        let count = unsafe { (*p_dacl).AceCount };
        let mut is_protected = false;
        for i in 0..count {
            let mut p_ace: *mut std::ffi::c_void = std::ptr::null_mut();
            if unsafe { GetAce(p_dacl, i as u32, &mut p_ace) } != 0 {
                let header = unsafe { &*(p_ace as *const ACE_HEADER) };
                if header.AceType == ACCESS_DENIED_ACE_TYPE {
                    let deny_ace = unsafe { &*(p_ace as *const ACCESS_DENIED_ACE) };
                    let ace_sid = &deny_ace.SidStart as *const u32 as PSID;
                    if unsafe { EqualSid(ace_sid, user_sid.as_ptr() as PSID) } != 0
                        && deny_ace.Mask == DENY_MASK
                    {
                        is_protected = true;
                        break;
                    }
                }
            }
        }

        unsafe { LocalFree(p_sec_desc as _) };

        if is_protected {
            Ok(Protection::Protected)
        } else {
            Ok(Protection::Unprotected)
        }
    }

    pub fn grant_delete_child(path: &Path) -> Result<(), std::io::Error> {
        let user_sid = get_current_user_sid()?;
        let wide_path = to_wide_null(path);

        let mut p_sec_desc: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        let mut p_dacl: *mut ACL = std::ptr::null_mut();

        let ret = unsafe {
            GetNamedSecurityInfoW(
                wide_path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut p_dacl,
                std::ptr::null_mut(),
                &mut p_sec_desc,
            )
        };
        if ret != 0 {
            return Err(std::io::Error::from_raw_os_error(ret as i32));
        }

        let mut ea: EXPLICIT_ACCESS_W = unsafe { std::mem::zeroed() };
        ea.grfAccessPermissions = windows_sys::Win32::Storage::FileSystem::FILE_DELETE_CHILD;
        ea.grfAccessMode = GRANT_ACCESS;
        ea.grfInheritance = CONTAINER_INHERIT_ACE;
        ea.Trustee.pMultipleTrustee = std::ptr::null_mut();
        ea.Trustee.MultipleTrusteeOperation = 0;
        ea.Trustee.TrusteeForm = TRUSTEE_IS_SID;
        ea.Trustee.TrusteeType = TRUSTEE_IS_USER;
        ea.Trustee.ptstrName = user_sid.as_ptr() as *mut u16;

        let mut p_new_dacl: *mut ACL = std::ptr::null_mut();
        let ret = unsafe { SetEntriesInAclW(1, &ea, p_dacl, &mut p_new_dacl) };
        if ret != 0 {
            unsafe { LocalFree(p_sec_desc as _) };
            return Err(std::io::Error::from_raw_os_error(ret as i32));
        }

        let ret = unsafe {
            SetNamedSecurityInfoW(
                wide_path.as_ptr() as *mut u16,
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                p_new_dacl,
                std::ptr::null_mut(),
            )
        };

        unsafe {
            LocalFree(p_new_dacl as _);
            LocalFree(p_sec_desc as _);
        }

        if ret != 0 {
            return Err(std::io::Error::from_raw_os_error(ret as i32));
        }

        Ok(())
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Protection;
    use std::path::Path;

    pub fn protect(path: &Path) -> Result<(), std::io::Error> {
        let mut perms = std::fs::metadata(path)?.permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(path, perms)?;
        Ok(())
    }

    pub fn unprotect(path: &Path) -> Result<(), std::io::Error> {
        let mut perms = std::fs::metadata(path)?.permissions();
        perms.set_readonly(false);
        std::fs::set_permissions(path, perms)?;
        Ok(())
    }

    pub fn protection(path: &Path) -> Result<Protection, std::io::Error> {
        let perms = std::fs::metadata(path)?.permissions();
        if perms.readonly() {
            Ok(Protection::Protected)
        } else {
            Ok(Protection::Unprotected)
        }
    }

    pub fn grant_delete_child(_path: &Path) -> Result<(), std::io::Error> {
        Ok(())
    }
}

pub fn protect(path: &Path) -> Result<(), std::io::Error> {
    imp::protect(path)
}

pub fn unprotect(path: &Path) -> Result<(), std::io::Error> {
    imp::unprotect(path)
}

pub fn protection(path: &Path) -> Result<Protection, std::io::Error> {
    imp::protection(path)
}

pub fn grant_delete_child(path: &Path) -> Result<(), std::io::Error> {
    imp::grant_delete_child(path)
}
