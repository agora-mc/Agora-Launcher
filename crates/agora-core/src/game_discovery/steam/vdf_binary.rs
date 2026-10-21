use std::collections::{HashMap, HashSet};
use std::fmt;

const MAGIC_V28: u32 = 0x07564428;
const MAGIC_V29: u32 = 0x07564429;
/// Real entries nest a few levels; anything deeper is corrupt, and would
/// otherwise overflow the stack.
const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamAppMetadata {
    pub app_id: u32,
    pub app_type: Option<String>,
    pub parent: Option<String>,
    pub executables: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AppInfoParseError(pub String);

impl fmt::Display for AppInfoParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for AppInfoParseError {}

/// Parse `appcache/appinfo.vdf`.
/// If `target_apps` is Some, apps not in the set are skipped by `size`.
pub fn parse_appinfo_vdf(
    data: &[u8],
    target_apps: Option<&HashSet<u32>>,
) -> Result<HashMap<u32, SteamAppMetadata>, AppInfoParseError> {
    if data.len() < 8 {
        return Err(AppInfoParseError("file smaller than header".to_string()));
    }

    let magic = read_u32(data, 0)?;
    let _universe = read_u32(data, 4)?;

    let is_v29 = match magic {
        MAGIC_V28 => false,
        MAGIC_V29 => true,
        other => return Err(AppInfoParseError(format!("unknown magic 0x{other:08x}"))),
    };

    let (mut cursor, string_table) = if is_v29 {
        if data.len() < 16 {
            return Err(AppInfoParseError("v29 header truncated".to_string()));
        }
        let str_table_offset = read_i64(data, 8)?;
        if str_table_offset < 0 || str_table_offset as usize > data.len() {
            return Err(AppInfoParseError(format!(
                "invalid string table offset {str_table_offset}"
            )));
        }
        let table = parse_string_table(data, str_table_offset as usize)?;
        (16, Some(table))
    } else {
        (8, None)
    };

    let mut apps = HashMap::new();

    while cursor + 4 <= data.len() {
        let app_id = read_u32(data, cursor)?;
        cursor += 4;
        if app_id == 0 {
            break;
        }

        if cursor + 4 > data.len() {
            return Err(AppInfoParseError("truncated app entry size".to_string()));
        }
        let size = read_u32(data, cursor)? as usize;
        cursor += 4;

        if cursor + size > data.len() {
            return Err(AppInfoParseError(format!(
                "entry for app {app_id} size {size} exceeds file length"
            )));
        }

        let entry_end = cursor + size;

        let should_parse = match target_apps {
            Some(targets) => targets.contains(&app_id),
            None => true,
        };

        if !should_parse {
            cursor = entry_end;
            continue;
        }

        // Fixed fields: info_state (4), last_updated (4), pics_token (8),
        // sha1_1 (20), change_number (4), sha1_2 (20) = 60 bytes
        if cursor + 60 > entry_end {
            return Err(AppInfoParseError(format!(
                "entry for app {app_id} truncated before binary KV"
            )));
        }
        cursor += 60;

        // Parse binary KV for this entry
        let mut entry_cursor = cursor;
        let kv = parse_binary_kv_object(
            data,
            &mut entry_cursor,
            entry_end,
            is_v29,
            string_table.as_deref(),
            0,
        )?;

        let meta = extract_metadata(app_id, &kv);
        apps.insert(app_id, meta);

        cursor = entry_end;
    }

    Ok(apps)
}

fn parse_string_table(data: &[u8], offset: usize) -> Result<Vec<String>, AppInfoParseError> {
    if offset + 4 > data.len() {
        return Err(AppInfoParseError(
            "truncated string table count".to_string(),
        ));
    }
    let count = read_u32(data, offset)? as usize;
    let mut cursor = offset + 4;
    // Every string takes at least its NUL byte, so the file bounds the count; a
    // corrupt count must not become a huge allocation.
    let mut table = Vec::with_capacity(count.min(data.len() - cursor));

    for _ in 0..count {
        if cursor >= data.len() {
            return Err(AppInfoParseError(
                "truncated string in string table".to_string(),
            ));
        }
        let nul_pos = data[cursor..]
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| AppInfoParseError("unterminated string in string table".to_string()))?;
        let s = String::from_utf8_lossy(&data[cursor..cursor + nul_pos]).to_string();
        table.push(s);
        cursor += nul_pos + 1;
    }

    Ok(table)
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
enum BinaryKvValue {
    Object(Vec<(String, BinaryKvValue)>),
    String(String),
    Int32(i32),
    Float32(f32),
    UInt64(u64),
    Int64(i64),
}

fn parse_binary_kv_object(
    data: &[u8],
    cursor: &mut usize,
    end: usize,
    is_v29: bool,
    string_table: Option<&[String]>,
    depth: usize,
) -> Result<Vec<(String, BinaryKvValue)>, AppInfoParseError> {
    if depth > MAX_DEPTH {
        return Err(AppInfoParseError(format!(
            "nested deeper than {MAX_DEPTH} levels"
        )));
    }
    let mut entries = Vec::new();

    while *cursor < end {
        let type_byte = data[*cursor];
        *cursor += 1;
        if type_byte == 0x08 {
            // End of object
            return Ok(entries);
        }

        let key = if is_v29 {
            if *cursor + 4 > end {
                return Err(AppInfoParseError("truncated key index".to_string()));
            }
            let idx = read_u32(data, *cursor)? as usize;
            *cursor += 4;
            let strings = string_table.ok_or_else(|| {
                AppInfoParseError("missing string table in v29 appinfo".to_string())
            })?;
            strings
                .get(idx)
                .cloned()
                .unwrap_or_else(|| format!("unknown_key_{idx}"))
        } else {
            let start = *cursor;
            let nul_pos = data[start..end]
                .iter()
                .position(|&b| b == 0)
                .ok_or_else(|| AppInfoParseError("unterminated key string".to_string()))?;
            let s = String::from_utf8_lossy(&data[start..start + nul_pos]).to_string();
            *cursor = start + nul_pos + 1;
            s
        };

        let val = match type_byte {
            0x00 => {
                let nested =
                    parse_binary_kv_object(data, cursor, end, is_v29, string_table, depth + 1)?;
                BinaryKvValue::Object(nested)
            }
            0x01 => {
                let start = *cursor;
                let nul_pos = data[start..end]
                    .iter()
                    .position(|&b| b == 0)
                    .ok_or_else(|| AppInfoParseError("unterminated value string".to_string()))?;
                let s = String::from_utf8_lossy(&data[start..start + nul_pos]).to_string();
                *cursor = start + nul_pos + 1;
                BinaryKvValue::String(s)
            }
            0x02 => {
                if *cursor + 4 > end {
                    return Err(AppInfoParseError("truncated int32".to_string()));
                }
                let val = read_i32(data, *cursor)?;
                *cursor += 4;
                BinaryKvValue::Int32(val)
            }
            0x03 => {
                if *cursor + 4 > end {
                    return Err(AppInfoParseError("truncated float32".to_string()));
                }
                let bits = read_u32(data, *cursor)?;
                *cursor += 4;
                BinaryKvValue::Float32(f32::from_bits(bits))
            }
            0x07 => {
                if *cursor + 8 > end {
                    return Err(AppInfoParseError("truncated uint64".to_string()));
                }
                let val = read_u64(data, *cursor)?;
                *cursor += 8;
                BinaryKvValue::UInt64(val)
            }
            0x0A => {
                if *cursor + 8 > end {
                    return Err(AppInfoParseError("truncated int64".to_string()));
                }
                let val = read_i64(data, *cursor)?;
                *cursor += 8;
                BinaryKvValue::Int64(val)
            }
            other => {
                return Err(AppInfoParseError(format!(
                    "unsupported binary KV type byte 0x{other:02x}"
                )));
            }
        };

        entries.push((key, val));
    }

    Ok(entries)
}

fn extract_metadata(app_id: u32, entries: &[(String, BinaryKvValue)]) -> SteamAppMetadata {
    // Top level might be wrapped in "appinfo" or direct
    let root = match find_object(entries, "appinfo") {
        Some(sub) => sub,
        None => entries,
    };

    let common = find_object(root, "common");
    let app_type = common.and_then(|c| find_string(c, "type"));
    let parent = common.and_then(|c| find_string_or_int(c, "parent"));

    let mut executables = Vec::new();
    if let Some(config) = find_object(root, "config") {
        if let Some(launch) = find_object(config, "launch") {
            // launch entries are keyed by "<n>" (e.g. "0", "1", "2"...)
            // Sort by key order (numeric if possible, else string)
            let mut launch_entries: Vec<(&str, &[(String, BinaryKvValue)])> = Vec::new();
            for (k, v) in launch {
                if let BinaryKvValue::Object(obj) = v {
                    launch_entries.push((k.as_str(), obj.as_slice()));
                }
            }

            launch_entries.sort_by(|(a_key, _), (b_key, _)| {
                match (a_key.parse::<u32>(), b_key.parse::<u32>()) {
                    (Ok(a), Ok(b)) => a.cmp(&b),
                    _ => a_key.cmp(b_key),
                }
            });

            for (_key, obj) in launch_entries {
                if let Some(exe) = find_string(obj, "executable") {
                    // Steam keeps the OS list in the entry's own `config`
                    // object; entries for a beta branch only are not launch
                    // options of the installed build.
                    let config = find_object(obj, "config");
                    if config.and_then(|c| find_string(c, "betakey")).is_some() {
                        continue;
                    }
                    let oslist = find_string(obj, "oslist")
                        .or_else(|| config.and_then(|c| find_string(c, "oslist")));
                    let valid_os = match oslist {
                        None => true,
                        Some(os) => os.to_ascii_lowercase().contains("windows"),
                    };
                    if valid_os && !executables.contains(&exe) {
                        executables.push(exe);
                    }
                }
            }
        }
    }

    SteamAppMetadata {
        app_id,
        app_type,
        parent,
        executables,
    }
}

fn find_object<'a>(
    entries: &'a [(String, BinaryKvValue)],
    key: &str,
) -> Option<&'a [(String, BinaryKvValue)]> {
    for (k, v) in entries {
        if k.eq_ignore_ascii_case(key) {
            if let BinaryKvValue::Object(obj) = v {
                return Some(obj.as_slice());
            }
        }
    }
    None
}

fn find_string(entries: &[(String, BinaryKvValue)], key: &str) -> Option<String> {
    for (k, v) in entries {
        if k.eq_ignore_ascii_case(key) {
            if let BinaryKvValue::String(s) = v {
                return Some(s.clone());
            }
        }
    }
    None
}

fn find_string_or_int(entries: &[(String, BinaryKvValue)], key: &str) -> Option<String> {
    for (k, v) in entries {
        if k.eq_ignore_ascii_case(key) {
            match v {
                BinaryKvValue::String(s) => return Some(s.clone()),
                BinaryKvValue::Int32(i) => return Some(i.to_string()),
                BinaryKvValue::UInt64(u) => return Some(u.to_string()),
                BinaryKvValue::Int64(i) => return Some(i.to_string()),
                _ => {}
            }
        }
    }
    None
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32, AppInfoParseError> {
    if offset + 4 > data.len() {
        return Err(AppInfoParseError("unexpected EOF reading u32".to_string()));
    }
    Ok(u32::from_le_bytes(
        data[offset..offset + 4].try_into().unwrap(),
    ))
}

fn read_i32(data: &[u8], offset: usize) -> Result<i32, AppInfoParseError> {
    if offset + 4 > data.len() {
        return Err(AppInfoParseError("unexpected EOF reading i32".to_string()));
    }
    Ok(i32::from_le_bytes(
        data[offset..offset + 4].try_into().unwrap(),
    ))
}

fn read_u64(data: &[u8], offset: usize) -> Result<u64, AppInfoParseError> {
    if offset + 8 > data.len() {
        return Err(AppInfoParseError("unexpected EOF reading u64".to_string()));
    }
    Ok(u64::from_le_bytes(
        data[offset..offset + 8].try_into().unwrap(),
    ))
}

fn read_i64(data: &[u8], offset: usize) -> Result<i64, AppInfoParseError> {
    if offset + 8 > data.len() {
        return Err(AppInfoParseError("unexpected EOF reading i64".to_string()));
    }
    Ok(i64::from_le_bytes(
        data[offset..offset + 8].try_into().unwrap(),
    ))
}
