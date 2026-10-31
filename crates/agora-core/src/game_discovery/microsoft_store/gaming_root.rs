use std::fmt;

const GAMING_ROOT_MAGIC: [u8; 4] = [0x52, 0x47, 0x42, 0x58]; // "RGBX"

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GamingRootParseError(pub String);

impl fmt::Display for GamingRootParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for GamingRootParseError {}

/// Parse a `.GamingRoot` file.
/// Format: magic `RGBX` (4 bytes), u32 count, then `count` NUL-terminated UTF-16LE paths.
pub fn parse_gaming_root(data: &[u8]) -> Result<Vec<String>, GamingRootParseError> {
    if data.len() < 8 {
        return Err(GamingRootParseError(
            "file smaller than header (8 bytes)".to_string(),
        ));
    }

    if data[0..4] != GAMING_ROOT_MAGIC {
        return Err(GamingRootParseError(format!(
            "invalid magic: expected RGBX, found {:02X?}",
            &data[0..4]
        )));
    }

    let count = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
    let mut cursor = 8;
    // Each path takes at least its two-byte terminator; a corrupt count must not
    // become a huge allocation.
    let mut paths = Vec::with_capacity(count.min((data.len() - 8) / 2));

    for i in 0..count {
        if cursor >= data.len() {
            return Err(GamingRootParseError(format!(
                "file truncated before reading path {i} of {count}"
            )));
        }

        let mut u16_chars = Vec::new();
        let mut terminated = false;

        while cursor + 2 <= data.len() {
            let u = u16::from_le_bytes(data[cursor..cursor + 2].try_into().unwrap());
            cursor += 2;
            if u == 0 {
                terminated = true;
                break;
            }
            u16_chars.push(u);
        }

        if !terminated {
            return Err(GamingRootParseError(format!(
                "unterminated UTF-16 path at index {i}"
            )));
        }

        let s = String::from_utf16_lossy(&u16_chars);
        paths.push(s);
    }

    Ok(paths)
}
