use std::ffi::c_void;
use windows_sys::Win32::Foundation::HANDLE;

pub type NTSTATUS = i32;
pub const STATUS_SUCCESS: NTSTATUS = 0;
pub const STATUS_BUFFER_OVERFLOW: NTSTATUS = 0x8000_0005_u32 as i32;
pub const STATUS_NO_MORE_FILES: NTSTATUS = 0x8000_0006_u32 as i32;
pub const STATUS_NO_SUCH_FILE: NTSTATUS = 0xC000_000F_u32 as i32;
pub const STATUS_ACCESS_DENIED: NTSTATUS = 0xC000_0022_u32 as i32;
pub const STATUS_OBJECT_NAME_NOT_FOUND: NTSTATUS = 0xC000_0034_u32 as i32;
pub const STATUS_OBJECT_NAME_COLLISION: NTSTATUS = 0xC000_0035_u32 as i32;

// Access rights
pub const FILE_WRITE_DATA: u32 = 0x2;
pub const FILE_APPEND_DATA: u32 = 0x4;
pub const FILE_WRITE_EA: u32 = 0x10;
pub const FILE_WRITE_ATTRIBUTES: u32 = 0x100;
pub const DELETE_ACCESS: u32 = 0x1_0000;
pub const WRITE_DAC: u32 = 0x4_0000;
pub const WRITE_OWNER: u32 = 0x8_0000;
pub const MAXIMUM_ALLOWED: u32 = 0x0200_0000;
pub const GENERIC_ALL: u32 = 0x1000_0000;
pub const GENERIC_WRITE: u32 = 0x4000_0000;
pub const GENERIC_READ: u32 = 0x8000_0000;
pub const SYNCHRONIZE: u32 = 0x10_0000;
pub const WRITE_RIGHTS: u32 = FILE_WRITE_DATA
    | FILE_APPEND_DATA
    | FILE_WRITE_EA
    | FILE_WRITE_ATTRIBUTES
    | WRITE_DAC
    | WRITE_OWNER
    | GENERIC_ALL
    | GENERIC_WRITE;

// Create dispositions
pub const FILE_SUPERSEDE: u32 = 0;
pub const FILE_OPEN: u32 = 1;
pub const FILE_CREATE: u32 = 2;
pub const FILE_OPEN_IF: u32 = 3;
pub const FILE_OVERWRITE: u32 = 4;
pub const FILE_OVERWRITE_IF: u32 = 5;

// Create options
pub const FILE_DIRECTORY_FILE: u32 = 0x1;
pub const FILE_DELETE_ON_CLOSE: u32 = 0x1000;
pub const FILE_OPEN_BY_FILE_ID: u32 = 0x2000;

// Information classes
pub const FILE_RENAME_INFORMATION: u32 = 10;
pub const FILE_DISPOSITION_INFORMATION: u32 = 13;
pub const FILE_DISPOSITION_INFORMATION_EX: u32 = 64;
pub const FILE_RENAME_INFORMATION_EX: u32 = 65;
pub const FILE_RENAME_REPLACE_IF_EXISTS: u32 = 0x1;
pub const FILE_DISPOSITION_DELETE: u32 = 0x1;

// Directory query flags
pub const SL_RESTART_SCAN: u32 = 0x1;
pub const SL_RETURN_SINGLE_ENTRY: u32 = 0x2;
pub const FILE_LIST_DIRECTORY: u32 = 0x1;
pub const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x20;
pub const FILE_SHARE_ALL: u32 = 0x7;

#[repr(C)]
pub struct UnicodeString {
    pub length: u16,
    pub maximum_length: u16,
    pub buffer: *mut u16,
}

#[repr(C)]
pub struct ObjectAttributes {
    pub length: u32,
    pub root_directory: HANDLE,
    pub object_name: *mut UnicodeString,
    pub attributes: u32,
    pub security_descriptor: *mut c_void,
    pub security_quality_of_service: *mut c_void,
}

pub unsafe fn unicode(u: *const UnicodeString) -> Option<String> {
    if u.is_null() || (*u).buffer.is_null() {
        return None;
    }
    let s = std::slice::from_raw_parts((*u).buffer, (*u).length as usize / 2);
    Some(String::from_utf16_lossy(s))
}
