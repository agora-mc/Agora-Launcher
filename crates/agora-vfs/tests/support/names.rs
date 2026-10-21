//! Name queries on an open handle, shared by the probe (`src/bin/probe/alias.rs`, run under the VFS)
//! and the real-DLL test (`tests/alias.rs`, run unhooked as the reference). The same code on both
//! sides makes the results comparable byte for byte.

// Included by both the probe and the test, and each uses only part of it.
#![allow(dead_code)]

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;

use serde::{Deserialize, Serialize};
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Storage::FileSystem::{
    FileNameInfo, GetFileInformationByHandleEx, GetFinalPathNameByHandleW,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

/// The `GetFinalPathNameByHandleW` flags the test compares: DOS and volume GUID (normalized),
/// NT device, and the opened name in DOS and NT form.
pub const GFPN_FLAGS: &[u32] = &[0x0, 0x1, 0x2, 0x8, 0xA];

/// The buffer lengths every name query is tried with, from zero past every boundary measured.
pub const LENGTHS: std::ops::RangeInclusive<u32> = 0..=260;

const BUF: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gfpn {
    pub flags: u32,
    pub path: String,
    pub error: u32,
}

/// One query with a buffer of `len` bytes. `status` is the NTSTATUS (or 0), `info` the IO status
/// Information, the ReturnLength, or whether the call succeeded, and `error` the Win32 error.
/// `bytes` is the buffer after the call, normalised (see `queries`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Query {
    pub kind: String,
    pub len: u32,
    pub status: u32,
    pub info: u64,
    pub error: u32,
    pub bytes: String,
}

/// Everything the test compares for one handle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Parity {
    pub gfpn: Vec<Gfpn>,
    pub queries: Vec<Query>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

/// The UNICODE_STRING layout NtQueryObject writes (pointer-sized, as in the kernel's structure).
#[repr(C)]
pub struct Unicode {
    pub length: u16,
    pub maximum_length: u16,
    pub buffer: *mut u16,
}

#[repr(C)]
struct ObjectAttributes {
    length: u32,
    root_directory: HANDLE,
    object_name: *mut Unicode,
    attributes: u32,
    security_descriptor: *mut c_void,
    security_qos: *mut c_void,
}

type NtCreateFile = unsafe extern "system" fn(
    *mut HANDLE,
    u32,
    *const ObjectAttributes,
    *mut [usize; 2],
    *const i64,
    u32,
    u32,
    u32,
    u32,
    *mut c_void,
    u32,
) -> i32;

/// A file or folder opened by NT name (`\??\...`, `\Device\...`, or relative to `root`), through
/// ntdll's NtCreateFile, so the name reaches the hooks exactly as the game's own would. Closed on drop.
pub struct NtOpen(HANDLE);

impl NtOpen {
    pub fn file(root: Option<HANDLE>, name: &str, directory: bool) -> Result<NtOpen, i32> {
        let mut wide: Vec<u16> = name.encode_utf16().collect();
        let mut uni = Unicode {
            length: (wide.len() * 2) as u16,
            maximum_length: (wide.len() * 2) as u16,
            buffer: wide.as_mut_ptr(),
        };
        let oa = ObjectAttributes {
            length: std::mem::size_of::<ObjectAttributes>() as u32,
            root_directory: root.unwrap_or(std::ptr::null_mut()),
            object_name: &mut uni,
            attributes: 0x40, // OBJ_CASE_INSENSITIVE
            security_descriptor: std::ptr::null_mut(),
            security_qos: std::ptr::null_mut(),
        };
        let mut handle: HANDLE = std::ptr::null_mut();
        let mut iosb = [0usize; 2];
        let create: NtCreateFile = unsafe { ntdll("NtCreateFile") };
        let options = 0x20 | if directory { 0x1 } else { 0x40 }; // synchronous; directory or not
        let status = unsafe {
            create(
                &mut handle,
                0x8000_0000 | 0x0010_0000, // GENERIC_READ | SYNCHRONIZE
                &oa,
                &mut iosb,
                std::ptr::null(),
                0,
                7, // share read, write, delete
                1, // FILE_OPEN
                options,
                std::ptr::null_mut(),
                0,
            )
        };
        if status < 0 {
            Err(status)
        } else {
            Ok(NtOpen(handle))
        }
    }

    pub fn handle(&self) -> HANDLE {
        self.0
    }
}

impl Drop for NtOpen {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

pub type QueryInformationFile =
    unsafe extern "system" fn(HANDLE, *mut [usize; 2], *mut c_void, u32, u32) -> i32;
pub type QueryObject = unsafe extern "system" fn(HANDLE, u32, *mut c_void, u32, *mut u32) -> i32;

pub unsafe fn ntdll<T: Copy>(name: &str) -> T {
    let module: Vec<u16> = "ntdll.dll".encode_utf16().chain(Some(0)).collect();
    let symbol = std::ffi::CString::new(name).unwrap();
    let f = GetProcAddress(
        GetModuleHandleW(module.as_ptr()),
        symbol.as_ptr() as *const u8,
    )
    .unwrap_or_else(|| panic!("ntdll has no {name}"));
    std::mem::transmute_copy(&f)
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// GetFinalPathNameByHandleW for each flag in `GFPN_FLAGS`.
pub unsafe fn gfpn(handle: HANDLE) -> Vec<Gfpn> {
    GFPN_FLAGS
        .iter()
        .map(|&flags| {
            let mut buf = vec![0u16; 2048];
            let n = GetFinalPathNameByHandleW(handle, buf.as_mut_ptr(), buf.len() as u32, flags);
            let (path, error) = if n == 0 || n as usize >= buf.len() {
                (
                    String::new(),
                    windows_sys::Win32::Foundation::GetLastError(),
                )
            } else {
                (String::from_utf16_lossy(&buf[..n as usize]), 0)
            };
            Gfpn { flags, path, error }
        })
        .collect()
}

/// Name queries on `handle` for every length in `LENGTHS`:
/// - NtQueryInformationFile FileNameInformation (9), FileNormalizedNameInformation (48) and
///   FileAllInformation (18); the fixed part of class 18 is the real file's and is zeroed, so only
///   the name part is compared;
/// - NtQueryObject ObjectNameInformation (1); the Buffer pointer is written as an offset from the
///   buffer, since the two sides allocate at different addresses;
/// - GetFileInformationByHandleEx FileNameInfo.
pub unsafe fn queries(handle: HANDLE) -> Vec<Query> {
    let qif: QueryInformationFile = ntdll("NtQueryInformationFile");
    let qo: QueryObject = ntdll("NtQueryObject");
    let header_size = std::mem::size_of::<Unicode>();
    let ptr_at = std::mem::offset_of!(Unicode, buffer);
    let mut out = Vec::new();
    for len in LENGTHS {
        for (kind, class) in [("nq9", 9u32), ("nq48", 48), ("nq18", 18)] {
            let mut buf = vec![0xAAu8; BUF];
            let mut iosb = [0usize; 2];
            let status = qif(
                handle,
                &mut iosb,
                buf.as_mut_ptr() as *mut c_void,
                len,
                class,
            );
            if class == 18 {
                let fixed = (len as usize).min(96);
                buf[..fixed].fill(0);
            }
            out.push(Query {
                kind: kind.to_string(),
                len,
                status: status as u32,
                info: iosb[1] as u64,
                error: 0,
                bytes: hex(&buf[..len as usize]),
            });
        }

        let mut buf = vec![0xAAu8; BUF];
        let base = buf.as_ptr() as usize;
        let mut ret = 0xDEAD_BEEFu32;
        let status = qo(handle, 1, buf.as_mut_ptr() as *mut c_void, len, &mut ret);
        if len as usize >= header_size {
            let slot = ptr_at..ptr_at + std::mem::size_of::<usize>();
            let pointer = usize::from_le_bytes(buf[slot.clone()].try_into().unwrap());
            if pointer >= base && pointer < base + BUF {
                buf[slot].copy_from_slice(&(pointer - base).to_le_bytes());
            }
        }
        out.push(Query {
            kind: "obj1".to_string(),
            len,
            status: status as u32,
            info: ret as u64,
            error: 0,
            bytes: hex(&buf[..len as usize]),
        });

        let mut buf = vec![0xAAu8; BUF];
        let ok = GetFileInformationByHandleEx(
            handle,
            FileNameInfo,
            buf.as_mut_ptr() as *mut c_void,
            len,
        );
        let error = if ok == 0 {
            windows_sys::Win32::Foundation::GetLastError()
        } else {
            0
        };
        out.push(Query {
            kind: "gfie".to_string(),
            len,
            status: 0,
            info: ok as u64,
            error,
            bytes: hex(&buf[..len as usize]),
        });
    }
    out
}

/// Both halves of the comparison for one handle.
pub unsafe fn parity(handle: HANDLE) -> Parity {
    Parity {
        gfpn: gfpn(handle),
        queries: queries(handle),
    }
}

/// A wide, NUL-terminated copy of `s`.
pub fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(Some(0))
        .collect()
}
