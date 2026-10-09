//! Reverse names (brief decision 5): the names a tracked handle reports, written into the caller's
//! buffer the way NTFS writes them. The rules below were measured on Windows 11 (24H2, NTFS) against
//! unhooked calls on a real file, for every buffer length; see the tests at the end.
//!
//! Layouts (x64 and i686 alike, except where a pointer is stored):
//! - `FILE_NAME_INFORMATION` (classes 9 and 48): a ULONG byte count at 0, the name at 4. The name is
//!   volume-relative (`\Games\Skyrim\Data\mod.esp`).
//! - `FILE_ALL_INFORMATION` (class 18): the same name block at 96 (count) and 100 (name), after the
//!   fixed part.
//! - `OBJECT_NAME_INFORMATION` (class 1, NtQueryObject): a `UNICODE_STRING` (pointer-sized), then
//!   the name with its device (`\Device\HarddiskVolumeN\Games\Skyrim\Data\mod.esp`).

use std::ptr;

use crate::nt::{
    UnicodeString, NTSTATUS, STATUS_BUFFER_OVERFLOW, STATUS_INFO_LENGTH_MISMATCH,
    STATUS_OBJECT_PATH_INVALID, STATUS_SUCCESS,
};

/// The smallest buffer `FILE_NAME_INFORMATION` accepts: `sizeof` the structure, which holds one
/// character and is rounded to four bytes.
pub const FILE_NAME_MIN_LEN: usize = 8;

/// The smallest buffer `FILE_ALL_INFORMATION` accepts: the fixed part, the count, and one character,
/// rounded to eight bytes.
pub const FILE_ALL_MIN_LEN: usize = 104;

/// Where the name starts in `FILE_ALL_INFORMATION`.
pub const FILE_ALL_NAME_AT: usize = 100;

/// Fill a name block the way NtQueryInformationFile does: the byte count at `name_at - 4`, then as
/// many whole characters of `name` as fit, at `name_at`. Returns the status and the Information
/// value. A buffer shorter than `min_len` is refused with nothing written.
///
/// # Safety
/// `out` must be valid for writes of `len` bytes, and `len >= name_at`.
pub unsafe fn fill_name_block(
    out: *mut u8,
    len: u32,
    name_at: usize,
    min_len: usize,
    name: &[u16],
) -> (NTSTATUS, usize) {
    let len = len as usize;
    if len < min_len {
        return (STATUS_INFO_LENGTH_MISMATCH, 0);
    }
    let bytes = name.len() * 2;
    ptr::write_unaligned(out.add(name_at - 4) as *mut u32, bytes as u32);
    // Only whole characters are copied, so a buffer with an odd room ends on a character.
    let room = (len - name_at) & !1;
    let copy = bytes.min(room);
    for (i, unit) in name.iter().take(copy / 2).enumerate() {
        ptr::write_unaligned(out.add(name_at + 2 * i) as *mut u16, *unit);
    }
    if name_at + bytes <= len {
        (STATUS_SUCCESS, name_at + bytes)
    } else {
        (STATUS_BUFFER_OVERFLOW, name_at + copy)
    }
}

/// Fill an OBJECT_NAME_INFORMATION the way NtQueryObject does. `device_bytes` is the byte length of
/// the device part (`\Device\HarddiskVolumeN`); the kernel's partial answers change at that length.
/// Writes the required length to `ret` (when not null) and returns the status.
///
/// # Safety
/// `out` must be valid for writes of `len` bytes, and `ret` null or valid for a `u32` write.
pub unsafe fn fill_object_name(
    out: *mut u8,
    len: u32,
    name: &[u16],
    device_bytes: usize,
    ret: *mut u32,
) -> NTSTATUS {
    let header = std::mem::size_of::<UnicodeString>();
    let len = len as usize;
    let bytes = name.len() * 2;
    let need = header + bytes + 2;
    let write_ret = |value: usize| {
        if !ret.is_null() {
            ptr::write_unaligned(ret, value as u32);
        }
    };
    if len < header {
        write_ret(need);
        return STATUS_INFO_LENGTH_MISMATCH;
    }
    // Whole character slots in the room after the header; the last one is the terminator.
    let slots = (len - header) / 2;
    let device_slots = device_bytes / 2 + 1;
    let (status, length, required) = if len >= need {
        (STATUS_SUCCESS, bytes, need)
    } else if slots < device_slots {
        // Not even the device and its terminator fit: nothing is written.
        write_ret(need);
        return STATUS_BUFFER_OVERFLOW;
    } else if slots == device_slots {
        // Exactly the device and its terminator fit: the kernel fills the device and reports a
        // path it cannot return in full, with a zero required length.
        (STATUS_OBJECT_PATH_INVALID, device_bytes, 0)
    } else {
        (STATUS_BUFFER_OVERFLOW, (slots - 1) * 2, need)
    };
    let chars = length / 2;
    for (i, unit) in name.iter().take(chars).enumerate() {
        ptr::write_unaligned(out.add(header + 2 * i) as *mut u16, *unit);
    }
    ptr::write_unaligned(out.add(header + length) as *mut u16, 0);
    let buffer_at = std::mem::offset_of!(UnicodeString, buffer);
    if status == STATUS_SUCCESS {
        // A complete answer is copied in whole, so the padding between the lengths and the pointer
        // is zero. A partial answer writes only the three fields, and the padding stays as it was.
        ptr::write_bytes(out.add(4), 0, buffer_at - 4);
    }
    ptr::write_unaligned(out as *mut u16, length as u16);
    ptr::write_unaligned(out.add(2) as *mut u16, (length + 2) as u16);
    ptr::write_unaligned(
        out.add(buffer_at) as *mut *mut u16,
        out.add(header) as *mut u16,
    );
    write_ret(required);
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The measured volume-relative name, 130 bytes (65 characters).
    fn sample_name() -> Vec<u16> {
        let name: Vec<u16> = r"\Users\jarja\AppData\Local\Temp\agora-scratch-nt\Data\sub\abc.txt"
            .encode_utf16()
            .collect();
        assert_eq!(name.len() * 2, 130, "fixture length");
        name
    }

    fn read_u32(buf: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(buf[at..at + 4].try_into().unwrap())
    }

    /// Measured on the real file (FileNameInformation, buffer length -> status and Information).
    #[test]
    fn file_name_block_matches_the_measured_windows_answers() {
        let name = sample_name();
        let bytes = name.len() * 2;
        let table: &[(u32, NTSTATUS, usize)] = &[
            (0, STATUS_INFO_LENGTH_MISMATCH, 0),
            (7, STATUS_INFO_LENGTH_MISMATCH, 0),
            (8, STATUS_BUFFER_OVERFLOW, 8),
            (9, STATUS_BUFFER_OVERFLOW, 8),
            (10, STATUS_BUFFER_OVERFLOW, 10),
            (99, STATUS_BUFFER_OVERFLOW, 98),
            (100, STATUS_BUFFER_OVERFLOW, 100),
            (101, STATUS_BUFFER_OVERFLOW, 100),
            (102, STATUS_BUFFER_OVERFLOW, 102),
            (120, STATUS_BUFFER_OVERFLOW, 120),
            (4 + bytes as u32 - 1, STATUS_BUFFER_OVERFLOW, 4 + bytes - 2),
            (4 + bytes as u32, STATUS_SUCCESS, 4 + bytes),
            (400, STATUS_SUCCESS, 4 + bytes),
        ];
        for &(len, status, info) in table {
            let mut buf = vec![0xAAu8; 512];
            let (got_status, got_info) =
                unsafe { fill_name_block(buf.as_mut_ptr(), len, 4, FILE_NAME_MIN_LEN, &name) };
            assert_eq!((got_status, got_info), (status, info), "len {len}");
            if len < FILE_NAME_MIN_LEN as u32 {
                assert_eq!(
                    read_u32(&buf, 0),
                    0xAAAA_AAAA,
                    "nothing written at len {len}"
                );
            } else {
                assert_eq!(read_u32(&buf, 0), bytes as u32, "count at len {len}");
            }
        }
    }

    #[test]
    fn file_all_name_block_refuses_a_short_buffer_and_fills_from_100() {
        let name = sample_name();
        let bytes = name.len() * 2;
        for (len, status, info) in [
            (102u32, STATUS_INFO_LENGTH_MISMATCH, 0usize),
            (104, STATUS_BUFFER_OVERFLOW, 104),
            (105, STATUS_BUFFER_OVERFLOW, 104),
            (200, STATUS_BUFFER_OVERFLOW, 200),
            (100 + bytes as u32, STATUS_SUCCESS, 100 + bytes),
            (400, STATUS_SUCCESS, 100 + bytes),
        ] {
            let mut buf = vec![0xAAu8; 512];
            let got = unsafe {
                fill_name_block(
                    buf.as_mut_ptr(),
                    len,
                    FILE_ALL_NAME_AT,
                    FILE_ALL_MIN_LEN,
                    &name,
                )
            };
            assert_eq!(got, (status, info), "len {len}");
        }
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn object_name_matches_the_measured_windows_answers_on_x64() {
        // `\Device\HarddiskVolume5` is 23 characters: 46 bytes.
        let mut name: Vec<u16> = r"\Device\HarddiskVolume5".encode_utf16().collect();
        name.extend(
            r"\Users\jarja\AppData\Local\Temp\agora-scratch-nt\Data\sub\abc.txt".encode_utf16(),
        );
        assert_eq!(name.len() * 2, 176);
        let device_bytes = 46;
        // (length, status, returned length, Length field)
        let table: &[(u32, NTSTATUS, u32, u16)] = &[
            (0, STATUS_INFO_LENGTH_MISMATCH, 194, 0xAAAA),
            (15, STATUS_INFO_LENGTH_MISMATCH, 194, 0xAAAA),
            (16, STATUS_BUFFER_OVERFLOW, 194, 0xAAAA),
            (62, STATUS_BUFFER_OVERFLOW, 194, 0xAAAA),
            (63, STATUS_BUFFER_OVERFLOW, 194, 0xAAAA),
            (64, STATUS_OBJECT_PATH_INVALID, 0, 46),
            (65, STATUS_OBJECT_PATH_INVALID, 0, 46),
            (66, STATUS_BUFFER_OVERFLOW, 194, 48),
            (80, STATUS_BUFFER_OVERFLOW, 194, 62),
            (192, STATUS_BUFFER_OVERFLOW, 194, 174),
            (194, STATUS_SUCCESS, 194, 176),
            (300, STATUS_SUCCESS, 194, 176),
        ];
        for &(len, status, required, length) in table {
            let mut buf = vec![0xAAu8; 1024];
            let mut ret = 0xDEADu32;
            let got =
                unsafe { fill_object_name(buf.as_mut_ptr(), len, &name, device_bytes, &mut ret) };
            assert_eq!(got, status, "status at len {len}");
            assert_eq!(ret, required, "required length at len {len}");
            let written_length = u16::from_le_bytes([buf[0], buf[1]]);
            if length == 0xAAAA {
                assert_eq!(written_length, 0xAAAA, "nothing written at len {len}");
            } else {
                assert_eq!(written_length, length, "Length at len {len}");
                let ptr = usize::from_le_bytes(buf[8..16].try_into().unwrap());
                assert_eq!(
                    ptr,
                    buf.as_ptr() as usize + 16,
                    "Buffer points past the header"
                );
            }
        }
    }
}
