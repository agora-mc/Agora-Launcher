//! Reads the `FileVersion` of a Windows PE file from its version resource (MASTER_SPEC §26.8).
//!
//! Read-only and bounds-checked: every offset comes from the file, and a malformed or truncated
//! file is an error, never a panic. It does not load the file or call a Windows API, so it runs
//! the same on every platform and never starts what it reads.

use std::fmt;

/// Why a file's version could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeVersionError {
    /// The bytes are not a PE file (no `MZ` header, no `PE\0\0` signature, or an unknown
    /// optional-header magic).
    NotPe,
    /// The file ends before a structure the reader needs.
    Truncated,
    /// The PE file has no version resource.
    NoVersionResource,
    /// The version resource has no `VS_FIXEDFILEINFO` block.
    NoFixedFileInfo,
}

impl fmt::Display for PeVersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PeVersionError::NotPe => write!(f, "not a PE file"),
            PeVersionError::Truncated => write!(f, "the PE file is truncated"),
            PeVersionError::NoVersionResource => write!(f, "the PE file has no version resource"),
            PeVersionError::NoFixedFileInfo => {
                write!(f, "the version resource has no fixed file information")
            }
        }
    }
}

impl std::error::Error for PeVersionError {}

/// `RT_VERSION`, the resource type that holds version information.
const RT_VERSION: u32 = 16;
/// `VS_FIXEDFILEINFO.dwSignature`.
const FIXED_FILE_INFO_SIGNATURE: u32 = 0xFEEF_04BD;
/// The high bit of a resource entry's offset marks a subdirectory; of its name, a string name.
const HIGH_BIT: u32 = 0x8000_0000;
/// The index of the resource table in the optional header's data directories.
const RESOURCE_DIRECTORY_INDEX: usize = 2;

/// The `FileVersion` of the PE file in `bytes`, as its four 16-bit components: for `0.2.2.6`,
/// `[0, 2, 2, 6]`.
pub fn read_file_version(bytes: &[u8]) -> Result<[u16; 4], PeVersionError> {
    if bytes.get(0..2) != Some(b"MZ") {
        return Err(PeVersionError::NotPe);
    }
    let pe = u32_at(bytes, 0x3C)? as usize;
    if bytes.get(pe..pe + 4) != Some(b"PE\0\0") {
        return Err(PeVersionError::NotPe);
    }

    let coff = pe + 4;
    let section_count = u16_at(bytes, coff + 2)? as usize;
    let optional_size = u16_at(bytes, coff + 16)? as usize;
    let optional = coff + 20;
    // PE32 and PE32+ differ in where the data directories start.
    let directories = match u16_at(bytes, optional)? {
        0x010B => 96,
        0x020B => 112,
        _ => return Err(PeVersionError::NotPe),
    };

    let entry = optional + directories + RESOURCE_DIRECTORY_INDEX * 8;
    let resource_rva = u32_at(bytes, entry)?;
    if resource_rva == 0 {
        return Err(PeVersionError::NoVersionResource);
    }

    let sections_at = optional + optional_size;
    let mut sections = Vec::with_capacity(section_count.min(96));
    for index in 0..section_count {
        let header = sections_at + index * 40;
        let virtual_size = u32_at(bytes, header + 8)?;
        let virtual_address = u32_at(bytes, header + 12)?;
        let raw_size = u32_at(bytes, header + 16)?;
        let raw_pointer = u32_at(bytes, header + 20)?;
        sections.push(Section {
            virtual_address,
            span: virtual_size.max(raw_size),
            raw_pointer,
        });
    }

    let resource_base = rva_to_offset(&sections, resource_rva).ok_or(PeVersionError::Truncated)?;
    let version_block = find_version_data(bytes, &sections, resource_base)?;
    fixed_file_version(version_block)
}

struct Section {
    virtual_address: u32,
    span: u32,
    raw_pointer: u32,
}

fn rva_to_offset(sections: &[Section], rva: u32) -> Option<usize> {
    sections.iter().find_map(|section| {
        let within = rva.checked_sub(section.virtual_address)?;
        (within < section.span).then(|| section.raw_pointer as usize + within as usize)
    })
}

/// Follow the resource tree `RT_VERSION` → first name → first language, and return the bytes of
/// that version resource.
fn find_version_data<'a>(
    bytes: &'a [u8],
    sections: &[Section],
    resource_base: usize,
) -> Result<&'a [u8], PeVersionError> {
    let type_entry = find_entry(bytes, resource_base, |id| id == RT_VERSION)?
        .ok_or(PeVersionError::NoVersionResource)?;
    let (name_dir, is_dir) = type_entry;
    if !is_dir {
        return Err(PeVersionError::NoVersionResource);
    }
    // Type, then name, then language: only the language level holds the data entry.
    let name_base = resource_base + name_dir;
    let (language_dir, is_dir) =
        first_entry(bytes, name_base)?.ok_or(PeVersionError::NoVersionResource)?;
    if !is_dir {
        return Err(PeVersionError::NoVersionResource);
    }
    let language_base = resource_base + language_dir;
    let (lang_entry, is_dir) =
        first_entry(bytes, language_base)?.ok_or(PeVersionError::NoVersionResource)?;
    if is_dir {
        return Err(PeVersionError::NoVersionResource);
    }
    let data_entry = resource_base + lang_entry;
    let data_rva = u32_at(bytes, data_entry)?;
    let data_size = u32_at(bytes, data_entry + 4)? as usize;
    let data_at = rva_to_offset(sections, data_rva).ok_or(PeVersionError::Truncated)?;
    bytes
        .get(
            data_at
                ..data_at
                    .checked_add(data_size)
                    .ok_or(PeVersionError::Truncated)?,
        )
        .ok_or(PeVersionError::Truncated)
}

/// Find the first entry of the resource directory at `dir` whose id satisfies `wanted`. Returns
/// the entry's offset (relative to the resource base) and whether it is a subdirectory.
fn find_entry(
    bytes: &[u8],
    dir: usize,
    wanted: impl Fn(u32) -> bool,
) -> Result<Option<(usize, bool)>, PeVersionError> {
    let named = u16_at(bytes, dir + 12)? as usize;
    let ids = u16_at(bytes, dir + 14)? as usize;
    for index in 0..named + ids {
        let entry = dir + 16 + index * 8;
        let name = u32_at(bytes, entry)?;
        if name & HIGH_BIT != 0 {
            continue;
        }
        if wanted(name) {
            let offset = u32_at(bytes, entry + 4)?;
            return Ok(Some((
                (offset & !HIGH_BIT) as usize,
                offset & HIGH_BIT != 0,
            )));
        }
    }
    Ok(None)
}

/// The first entry of a resource directory, whatever its id.
fn first_entry(bytes: &[u8], dir: usize) -> Result<Option<(usize, bool)>, PeVersionError> {
    let count = u16_at(bytes, dir + 12)? as usize + u16_at(bytes, dir + 14)? as usize;
    if count == 0 {
        return Ok(None);
    }
    let entry = dir + 16;
    let offset = u32_at(bytes, entry + 4)?;
    Ok(Some((
        (offset & !HIGH_BIT) as usize,
        offset & HIGH_BIT != 0,
    )))
}

/// The `VS_FIXEDFILEINFO` block inside a version resource, found by its signature. The block is
/// 4-byte aligned, so only aligned offsets are tried.
fn fixed_file_version(block: &[u8]) -> Result<[u16; 4], PeVersionError> {
    let mut at = 0;
    while at + 16 <= block.len() {
        if u32_at(block, at)? == FIXED_FILE_INFO_SIGNATURE {
            let most = u32_at(block, at + 8)?;
            let least = u32_at(block, at + 12)?;
            return Ok([
                (most >> 16) as u16,
                (most & 0xFFFF) as u16,
                (least >> 16) as u16,
                (least & 0xFFFF) as u16,
            ]);
        }
        at += 4;
    }
    Err(PeVersionError::NoFixedFileInfo)
}

fn u16_at(bytes: &[u8], at: usize) -> Result<u16, PeVersionError> {
    let raw = bytes.get(at..at + 2).ok_or(PeVersionError::Truncated)?;
    Ok(u16::from_le_bytes([raw[0], raw[1]]))
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32, PeVersionError> {
    let raw = bytes.get(at..at + 4).ok_or(PeVersionError::Truncated)?;
    Ok(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal PE file with one `.rsrc` section holding a version resource whose
    /// `FileVersion` is `components`. `magic` picks PE32 (0x10B) or PE32+ (0x20B).
    fn synthetic_pe(magic: u16, components: [u16; 4]) -> Vec<u8> {
        let mut file = vec![0u8; 0x400];
        file[0..2].copy_from_slice(b"MZ");
        put_u32(&mut file, 0x3C, 0x40);
        file[0x40..0x44].copy_from_slice(b"PE\0\0");
        let coff = 0x44;
        put_u16(&mut file, coff + 2, 1); // one section
        let optional_size: usize = if magic == 0x020B { 240 } else { 224 };
        put_u16(&mut file, coff + 16, optional_size as u16);
        let optional = coff + 20;
        put_u16(&mut file, optional, magic);
        let directories = if magic == 0x020B { 112 } else { 96 };
        // Resource table: RVA 0x1000, size 0x200.
        put_u32(&mut file, optional + directories + 16, 0x1000);
        put_u32(&mut file, optional + directories + 20, 0x200);

        let section = optional + optional_size;
        file[section..section + 6].copy_from_slice(b".rsrc\0");
        put_u32(&mut file, section + 8, 0x200); // virtual size
        put_u32(&mut file, section + 12, 0x1000); // virtual address
        put_u32(&mut file, section + 16, 0x200); // raw size
        put_u32(&mut file, section + 20, 0x200); // raw pointer

        // The resource tree, relative to the section start at file offset 0x200.
        let rsrc = 0x200;
        put_u16(&mut file, rsrc + 14, 1); // level 1: one id entry
        put_u32(&mut file, rsrc + 16, RT_VERSION);
        put_u32(&mut file, rsrc + 20, HIGH_BIT | 0x18);
        put_u16(&mut file, rsrc + 0x18 + 14, 1); // level 2: one id entry
        put_u32(&mut file, rsrc + 0x18 + 16, 1);
        put_u32(&mut file, rsrc + 0x18 + 20, HIGH_BIT | 0x28);
        put_u16(&mut file, rsrc + 0x28 + 14, 1); // level 3: one language
        put_u32(&mut file, rsrc + 0x28 + 16, 0x409);
        put_u32(&mut file, rsrc + 0x28 + 20, 0x48);
        // Data entry: the version blob sits at RVA 0x1060.
        put_u32(&mut file, rsrc + 0x48, 0x1060);
        put_u32(&mut file, rsrc + 0x48 + 4, 188);

        // VS_VERSIONINFO header, then the fixed block 40 bytes in (after the key, aligned).
        let blob = rsrc + 0x60;
        put_u16(&mut file, blob, 188);
        put_u16(&mut file, blob + 2, 52);
        let fixed = blob + 136;
        put_u32(&mut file, fixed, FIXED_FILE_INFO_SIGNATURE);
        put_u32(&mut file, fixed + 4, 0x0001_0000);
        put_u32(
            &mut file,
            fixed + 8,
            ((components[0] as u32) << 16) | components[1] as u32,
        );
        put_u32(
            &mut file,
            fixed + 12,
            ((components[2] as u32) << 16) | components[3] as u32,
        );
        file
    }

    fn put_u16(file: &mut [u8], at: usize, value: u16) {
        file[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u32(file: &mut [u8], at: usize, value: u32) {
        file[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    #[test]
    fn reads_the_file_version_of_a_pe32_file() {
        let file = synthetic_pe(0x010B, [0, 2, 2, 6]);
        assert_eq!(read_file_version(&file), Ok([0, 2, 2, 6]));
    }

    #[test]
    fn reads_the_file_version_of_a_pe32_plus_file() {
        let file = synthetic_pe(0x020B, [1, 6, 1170, 0]);
        assert_eq!(read_file_version(&file), Ok([1, 6, 1170, 0]));
    }

    #[test]
    fn refuses_bytes_that_are_not_a_pe_file() {
        assert_eq!(read_file_version(b"plain text"), Err(PeVersionError::NotPe));
        let mut file = synthetic_pe(0x010B, [0, 2, 2, 6]);
        file[0x40..0x44].copy_from_slice(b"XX\0\0");
        assert_eq!(read_file_version(&file), Err(PeVersionError::NotPe));
    }

    #[test]
    fn a_truncated_file_is_an_error_not_a_panic() {
        let file = synthetic_pe(0x010B, [0, 2, 2, 6]);
        for cut in [0, 0x30, 0x80, 0x180, 0x2A0, 0x300] {
            assert!(read_file_version(&file[..cut]).is_err(), "cut at {cut:#x}");
        }
    }

    #[test]
    fn a_file_without_a_version_resource_says_so() {
        let mut file = synthetic_pe(0x010B, [0, 2, 2, 6]);
        put_u32(&mut file, 0x44 + 20 + 96 + 16, 0);
        assert_eq!(
            read_file_version(&file),
            Err(PeVersionError::NoVersionResource)
        );
    }

    #[test]
    fn a_version_resource_without_fixed_info_says_so() {
        let mut file = synthetic_pe(0x010B, [0, 2, 2, 6]);
        put_u32(&mut file, 0x200 + 0x60 + 136, 0);
        assert_eq!(
            read_file_version(&file),
            Err(PeVersionError::NoFixedFileInfo)
        );
    }

    /// The real SKSE loader, read-only. Its FileVersion is `0.2.2.6`, which is what the manifest
    /// calls `2.2.6` once the leading zero is dropped.
    #[test]
    #[ignore = "reads the real SKSE loader in the p3 bases; run with --ignored"]
    fn reads_the_real_skse_loader_version() {
        let path = std::path::Path::new(
            r"D:\Agora-bench\p3-done\bases\deployments\p3-modded\game\skse64_loader.exe",
        );
        let bytes = std::fs::read(path).expect("the real SKSE loader is readable");
        let version = read_file_version(&bytes).expect("a version resource");
        println!("real skse64_loader.exe FileVersion components: {version:?}");
        assert_eq!(version, [0, 2, 2, 6]);
    }
}
