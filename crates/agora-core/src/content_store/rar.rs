//! RAR archives, read through Windows' own `tar.exe` (bsdtar, libarchive).
//!
//! The only mature RAR library for Rust wraps UnRAR, whose licence is not compatible with
//! GPL-3.0-only, and Windows ships a RAR reader of its own. So a RAR is never opened by Agora
//! itself. The archive is listed (`tar -tvf`), the listing is parsed and every rule a zip or 7z
//! entry gets is applied to it before anything is written, then the archive is extracted into a
//! fresh staging folder (`tar -xf`) and the extracted tree is checked against the listing before
//! any byte of it is hashed into the store. A line of the listing that cannot be read refuses the
//! whole archive: the listing is the only thing the checks see, so a line the parser does not
//! understand is a file the checks never saw.
//!
//! `tar.exe` is run by its absolute path under the Windows system directory, never through
//! `PATH`. On any other platform, or where that file is missing, RAR is an error.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use agora_game_api::RelPath;
use sha2::{Digest, Sha256};

use super::{
    disk_free_space, is_reparse_point_or_symlink, validate_entry_path, validate_path_set,
    ContentError, StagedEntry, StagingGuard, HASH_BUFFER_SIZE, ONE_GIB,
};
use crate::ctx::Ctx;

const RAR4_MAGIC: &[u8] = b"Rar!\x1a\x07\x00";
const RAR5_MAGIC: &[u8] = b"Rar!\x1a\x07\x01\x00";

const TAR_UNAVAILABLE: &str =
    "RAR archives need Windows' built-in tar (tar.exe in the Windows system folder), which is \
     not available here";

/// Whether the first bytes of a file are a RAR4 or RAR5 signature.
pub(super) fn is_rar_magic(head: &[u8]) -> bool {
    head.starts_with(RAR4_MAGIC) || head.starts_with(RAR5_MAGIC)
}

// ---------------------------------------------------------------------------
// The listing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListedKind {
    File,
    Dir,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ListedEntry {
    kind: ListedKind,
    size: u64,
    name: String,
}

fn unparsable(line: &[u8]) -> ContentError {
    ContentError::CorruptArchive(format!(
        "cannot read this line of the archive listing, so the archive is refused: {}",
        String::from_utf8_lossy(line)
    ))
}

/// The next run of non-space bytes, skipping any spaces in front of it.
fn take_token<'a>(rest: &mut &'a [u8]) -> Option<&'a [u8]> {
    let start = rest.iter().position(|&b| b != b' ')?;
    let from = &rest[start..];
    let end = from.iter().position(|&b| b == b' ').unwrap_or(from.len());
    let (token, after) = from.split_at(end);
    *rest = after;
    Some(token)
}

fn all_digits(token: &[u8]) -> bool {
    !token.is_empty() && token.iter().all(u8::is_ascii_digit)
}

/// Names in the listing are in the Windows ANSI code page (`tar.exe` converts them for its
/// output); anything that is not valid there is refused rather than guessed at.
#[cfg(windows)]
fn decode_name(bytes: &[u8]) -> Option<String> {
    use windows_sys::Win32::Globalization::{MultiByteToWideChar, CP_ACP, MB_ERR_INVALID_CHARS};
    if bytes.is_ascii() {
        return String::from_utf8(bytes.to_vec()).ok();
    }
    let len = i32::try_from(bytes.len()).ok()?;
    // SAFETY: both calls read `len` bytes from `bytes`; the second writes at most `wide.len()`
    // u16 values into a buffer of exactly that size.
    unsafe {
        let needed = MultiByteToWideChar(
            CP_ACP,
            MB_ERR_INVALID_CHARS,
            bytes.as_ptr(),
            len,
            std::ptr::null_mut(),
            0,
        );
        if needed <= 0 {
            return None;
        }
        let mut wide = vec![0u16; needed as usize];
        let written = MultiByteToWideChar(
            CP_ACP,
            MB_ERR_INVALID_CHARS,
            bytes.as_ptr(),
            len,
            wide.as_mut_ptr(),
            needed,
        );
        if written != needed {
            return None;
        }
        String::from_utf16(&wide).ok()
    }
}

#[cfg(not(windows))]
fn decode_name(bytes: &[u8]) -> Option<String> {
    String::from_utf8(bytes.to_vec()).ok()
}

/// One line of `tar -tvf`:
/// `-rw-r--r--  0 0      0    86302828 Dec 23  2025 DU01 - Textures.bsa`
/// (mode, links, owner, group, size, month, day, year or time, then one space and the name).
fn parse_listing_line(line: &[u8]) -> Result<ListedEntry, ContentError> {
    let mut rest = line;
    let mode = take_token(&mut rest).ok_or_else(|| unparsable(line))?;
    let links = take_token(&mut rest).ok_or_else(|| unparsable(line))?;
    let _owner = take_token(&mut rest).ok_or_else(|| unparsable(line))?;
    let _group = take_token(&mut rest).ok_or_else(|| unparsable(line))?;
    let size = take_token(&mut rest).ok_or_else(|| unparsable(line))?;
    let month = take_token(&mut rest).ok_or_else(|| unparsable(line))?;
    let day = take_token(&mut rest).ok_or_else(|| unparsable(line))?;
    let year_or_time = take_token(&mut rest).ok_or_else(|| unparsable(line))?;

    let mode_ok = mode.len() >= 10
        && mode[1..].iter().all(|b| b"-drwxsStTlL+@.".contains(b))
        && matches!(
            mode[0],
            b'-' | b'd' | b'l' | b'h' | b'c' | b'b' | b'p' | b's'
        );
    let month_ok = month.len() == 3 && month.iter().all(u8::is_ascii_alphabetic);
    let day_ok = all_digits(day) && day.len() <= 2;
    let year_or_time_ok = (all_digits(year_or_time) && year_or_time.len() == 4)
        || (year_or_time.len() == 5
            && year_or_time[2] == b':'
            && all_digits(&year_or_time[..2])
            && all_digits(&year_or_time[3..]));
    if !(mode_ok && all_digits(links) && month_ok && day_ok && year_or_time_ok) {
        return Err(unparsable(line));
    }
    let size: u64 = std::str::from_utf8(size)
        .ok()
        .filter(|s| all_digits(s.as_bytes()))
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| unparsable(line))?;

    // The name is everything after exactly one space; leading and trailing spaces are part of it.
    let name_bytes = rest.strip_prefix(b" ").ok_or_else(|| unparsable(line))?;
    if name_bytes.is_empty() {
        return Err(unparsable(line));
    }
    let name = decode_name(name_bytes).ok_or_else(|| unparsable(line))?;

    let kind = match mode[0] {
        b'-' => ListedKind::File,
        b'd' => ListedKind::Dir,
        other => {
            return Err(ContentError::InvalidPath {
                path: name,
                reason: format!(
                    "RAR entry of type '{}' (a link, device or other special file) is not allowed",
                    other as char
                ),
            });
        }
    };
    Ok(ListedEntry { kind, size, name })
}

/// Parse the whole output of `tar -tvf`. Lines end in `\n` or `\r\n`; an empty line anywhere
/// except after the last newline is not something tar prints, so it refuses the archive.
fn parse_tar_listing(output: &[u8]) -> Result<Vec<ListedEntry>, ContentError> {
    let mut lines: Vec<&[u8]> = output.split(|&b| b == b'\n').collect();
    if lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines
        .into_iter()
        .map(|line| parse_listing_line(line.strip_suffix(b"\r").unwrap_or(line)))
        .collect()
}

// ---------------------------------------------------------------------------
// Validation of the listing, before anything is extracted
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct RarPlan {
    files: Vec<(RelPath, u64)>,
    /// Every folder the extraction may legitimately contain (listed folders and the parents of
    /// everything), as normalised relative paths.
    dirs: HashSet<String>,
    declared_total: u64,
}

fn add_with_ancestors(dirs: &mut HashSet<String>, rel: &str) {
    let mut end = 0;
    for part in rel.split('/') {
        end += part.len();
        dirs.insert(rel[..end].to_string());
        end += 1;
    }
}

/// Apply to a listing every rule a zip or 7z entry gets (paths, collisions, special files). The
/// folders the listing names are held to the path rules too, because tar creates them.
fn plan_from_listing(listed: Vec<ListedEntry>) -> Result<RarPlan, ContentError> {
    let mut files = Vec::new();
    let mut file_paths = Vec::new();
    let mut dir_paths = Vec::new();
    let mut declared_total: u64 = 0;

    for entry in listed {
        match entry.kind {
            ListedKind::Dir => {
                let name = entry.name.strip_suffix('/').unwrap_or(&entry.name);
                dir_paths.push(validate_entry_path(name)?);
            }
            ListedKind::File => {
                let rel = validate_entry_path(&entry.name)?;
                declared_total = declared_total.saturating_add(entry.size);
                file_paths.push(rel.clone());
                files.push((rel, entry.size));
            }
        }
    }
    validate_path_set(&file_paths)?;

    let file_set: HashSet<String> = file_paths
        .iter()
        .map(|p| p.as_str().to_ascii_lowercase())
        .collect();
    let mut dirs = HashSet::new();
    for dir in &dir_paths {
        if file_set.contains(&dir.as_str().to_ascii_lowercase()) {
            return Err(ContentError::InvalidPath {
                path: dir.to_string(),
                reason: "is both a folder and a file in the archive".into(),
            });
        }
        add_with_ancestors(&mut dirs, dir.as_str());
    }
    for file in &file_paths {
        if let Some((parent, _)) = file.as_str().rsplit_once('/') {
            add_with_ancestors(&mut dirs, parent);
        }
    }
    Ok(RarPlan {
        files,
        dirs,
        declared_total,
    })
}

// ---------------------------------------------------------------------------
// Running tar.exe
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn tar_exe_path() -> Result<PathBuf, ContentError> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;

    let mut buf = vec![0u16; 260];
    let system_dir = loop {
        // SAFETY: the buffer is `buf.len()` u16 values long, which is what is passed.
        let n = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
        if n == 0 {
            break std::env::var_os("SystemRoot").map(|root| PathBuf::from(root).join("System32"));
        }
        if n >= buf.len() {
            buf.resize(n + 1, 0);
            continue;
        }
        break Some(PathBuf::from(std::ffi::OsString::from_wide(&buf[..n])));
    };
    let tar = system_dir
        .map(|dir| dir.join("tar.exe"))
        .filter(|p| p.is_file());
    tar.ok_or_else(|| ContentError::Other(TAR_UNAVAILABLE.into()))
}

#[cfg(not(windows))]
fn tar_exe_path() -> Result<PathBuf, ContentError> {
    Err(ContentError::Other(TAR_UNAVAILABLE.into()))
}

/// What tar said when it failed. A password-protected archive and a corrupt one both end here.
fn tar_failure(stderr: &[u8]) -> ContentError {
    let text = String::from_utf8_lossy(stderr).trim().to_string();
    let lower = text.to_ascii_lowercase();
    if lower.contains("passphrase") || lower.contains("password") || lower.contains("encrypt") {
        return ContentError::PasswordProtected;
    }
    ContentError::CorruptArchive(format!(
        "Windows' tar could not read this RAR archive: {text}"
    ))
}

fn tar_command(tar: &Path) -> Command {
    let mut cmd = Command::new(tar);
    cmd.stdin(Stdio::null());
    crate::helpers::hide_console_window(&mut cmd);
    cmd
}

fn run_listing(tar: &Path, archive: &Path) -> Result<Vec<u8>, ContentError> {
    let output = tar_command(tar)
        .arg("-tvf")
        .arg(archive)
        .output()
        .map_err(|e| ContentError::Other(format!("could not run Windows' tar: {e}")))?;
    if !output.status.success() {
        return Err(tar_failure(&output.stderr));
    }
    Ok(output.stdout)
}

/// Total size of the files under `dir`, ignoring anything that cannot be read.
fn dir_bytes(dir: &Path) -> u64 {
    let mut total = 0u64;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    for entry in entries.flatten() {
        let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if meta.is_dir() {
            total = total.saturating_add(dir_bytes(&entry.path()));
        } else if meta.is_file() {
            total = total.saturating_add(meta.len());
        }
    }
    total
}

/// `tar -xf archive -C dest`, stopped if it writes more than the listing declared in total.
fn run_extraction(
    tar: &Path,
    archive: &Path,
    dest: &Path,
    declared_total: u64,
) -> Result<(), ContentError> {
    let mut child = tar_command(tar)
        .arg("-xf")
        .arg(archive)
        .arg("-C")
        .arg(dest)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| ContentError::Other(format!("could not run Windows' tar: {e}")))?;

    let mut stderr = child.stderr.take();
    let stderr_reader = std::thread::spawn(move || {
        let mut text = Vec::new();
        if let Some(pipe) = stderr.as_mut() {
            let mut buf = [0u8; 4096];
            while let Ok(n) = pipe.read(&mut buf) {
                if n == 0 {
                    break;
                }
                // Keep draining so tar never blocks on a full pipe, but keep only the start.
                let room = 64 * 1024 - text.len().min(64 * 1024);
                text.extend_from_slice(&buf[..n.min(room)]);
            }
        }
        text
    });

    let mut delay = Duration::from_millis(5);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e.into());
            }
        }
        if dir_bytes(dest) > declared_total {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ContentError::EntryExceedsDeclaredSize {
                path: "(the archive as a whole)".into(),
                declared: declared_total,
            });
        }
        std::thread::sleep(delay);
        delay = (delay * 2).min(Duration::from_millis(200));
    };
    let stderr_text = stderr_reader.join().unwrap_or_default();
    if !status.success() {
        return Err(tar_failure(&stderr_text));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Checking what tar wrote
// ---------------------------------------------------------------------------

/// Exactly the listed files must exist, each with its listed size; no folder may be there that
/// the listing does not account for; nothing may be a reparse point or anything but a file or
/// folder.
fn check_extracted_tree(root: &Path, plan: &RarPlan) -> Result<(), ContentError> {
    let mut expected: HashMap<&str, u64> = plan
        .files
        .iter()
        .map(|(rel, size)| (rel.as_str(), *size))
        .collect();

    fn walk(
        root: &Path,
        rel: &str,
        plan: &RarPlan,
        expected: &mut HashMap<&str, u64>,
    ) -> Result<(), ContentError> {
        let dir = if rel.is_empty() {
            root.to_path_buf()
        } else {
            root.join(rel)
        };
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                return Err(ContentError::CorruptArchive(
                    "tar extracted a file whose name is not valid text".into(),
                ));
            };
            let child_rel = if rel.is_empty() {
                name.to_string()
            } else {
                format!("{rel}/{name}")
            };
            let meta = std::fs::symlink_metadata(entry.path())?;
            if is_reparse_point_or_symlink(&meta) {
                return Err(ContentError::InvalidPath {
                    path: child_rel,
                    reason: "extracted as a symlink or reparse point, which is not allowed".into(),
                });
            }
            if meta.is_dir() {
                if !plan.dirs.contains(&child_rel) {
                    return Err(ContentError::CorruptArchive(format!(
                        "tar extracted a folder the archive listing does not have: {child_rel}"
                    )));
                }
                walk(root, &child_rel, plan, expected)?;
            } else if meta.is_file() {
                match expected.remove(child_rel.as_str()) {
                    None => {
                        return Err(ContentError::CorruptArchive(format!(
                            "tar extracted a file the archive listing does not have: {child_rel}"
                        )));
                    }
                    Some(size) if size != meta.len() => {
                        return Err(ContentError::CorruptArchive(format!(
                            "{child_rel} is {} bytes after extraction but the archive lists {size}",
                            meta.len()
                        )));
                    }
                    Some(_) => {}
                }
            } else {
                return Err(ContentError::InvalidPath {
                    path: child_rel,
                    reason: "extracted as something other than a file or folder".into(),
                });
            }
        }
        Ok(())
    }

    walk(root, "", plan, &mut expected)?;
    if let Some(missing) = expected.keys().min() {
        return Err(ContentError::CorruptArchive(format!(
            "tar did not extract a file the archive lists: {missing}"
        )));
    }
    Ok(())
}

fn hash_file_counting(path: &Path) -> Result<(String, u64), std::io::Error> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; HASH_BUFFER_SIZE];
    let mut total = 0u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        hasher.update(&buf[..n]);
    }
    Ok((format!("{:x}", hasher.finalize()), total))
}

// ---------------------------------------------------------------------------
// The importer
// ---------------------------------------------------------------------------

/// Read a RAR through Windows' tar into a staging folder: the same result the zip and 7z
/// importers hand to `finish_adding`. On any error the staging folder is gone.
pub(super) fn stage_rar(
    ctx: &Ctx,
    archive_path: &Path,
) -> Result<(Vec<StagedEntry>, StagingGuard, PathBuf), ContentError> {
    let tar = tar_exe_path()?;

    // 1. List, then apply every rule to the listing before anything is extracted.
    let listing = run_listing(&tar, archive_path)?;
    let plan = plan_from_listing(parse_tar_listing(&listing)?)?;

    // 2. Untrusted sizes: check free disk space.
    let content_root = ctx.paths.content_root();
    std::fs::create_dir_all(&content_root)?;
    let required = plan.declared_total.saturating_add(ONE_GIB);
    if let Some(available) = disk_free_space(&content_root) {
        if available < required {
            return Err(ContentError::InsufficientSpace {
                required,
                available,
            });
        }
    }

    // 3. Staging directory setup.
    let staging_root = ctx.paths.content_staging_dir();
    std::fs::create_dir_all(&staging_root)?;
    let unique = format!("{}-{}", std::process::id(), uuid::Uuid::new_v4().simple());
    let staging_dir = staging_root.join(&unique);
    std::fs::create_dir_all(&staging_dir)?;
    let guard = StagingGuard::new(&staging_dir);

    // 4. Extract, then check the extracted tree against the listing.
    let extract_dir = staging_dir.join("extract");
    std::fs::create_dir_all(&extract_dir)?;
    run_extraction(&tar, archive_path, &extract_dir, plan.declared_total)?;
    check_extracted_tree(&extract_dir, &plan)?;

    // 5. Hash each file and move it to where the other importers leave theirs.
    let mut staged_entries = Vec::with_capacity(plan.files.len());
    for (idx, (rel_path, size)) in plan.files.iter().enumerate() {
        let extracted = extract_dir.join(rel_path.as_str());
        let (sha256, bytes) = hash_file_counting(&extracted)?;
        if bytes != *size {
            return Err(ContentError::CorruptArchive(format!(
                "{rel_path} changed size while it was being read"
            )));
        }
        let staged_path = staging_dir.join(format!("obj_{idx}"));
        std::fs::rename(&extracted, &staged_path)?;
        staged_entries.push(StagedEntry {
            rel_path: rel_path.clone(),
            staged_path,
            size: bytes,
            sha256,
        });
    }

    Ok((staged_entries, guard, staging_dir))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Captured from `C:\Windows\System32\tar.exe -tvf` (bsdtar 3.8.8) on real RAR5 archives.
    const CAPTURED: &[u8] = b"\
-rw-r--r--  0 0      0    86302828 Dec 23  2025 DU01DagothUrFollower - Textures.bsa\r\n\
-rw-r--r--  0 0      0        1153 Nov 30  2021 EmbersXD-Campfire Patch.esp\r\n\
drwxr-xr-x  0 0      0           0 Mar 31  2023 [ELLE] Bifrost/Main\r\n\
-rw-r--r--  0 0      0       89120 Mar 31 10:51 [ELLE] Bifrost/Images/Screenshot 2023-04-01 105001.jpg\r\n";

    fn entries(text: &[u8]) -> Vec<ListedEntry> {
        parse_tar_listing(text).expect("listing parses")
    }

    #[test]
    fn parses_captured_lines() {
        let got = entries(CAPTURED);
        assert_eq!(got.len(), 4);
        assert_eq!(
            got[0],
            ListedEntry {
                kind: ListedKind::File,
                size: 86302828,
                name: "DU01DagothUrFollower - Textures.bsa".into()
            }
        );
        assert_eq!(got[1].name, "EmbersXD-Campfire Patch.esp");
        assert_eq!(got[1].size, 1153);
        assert_eq!(got[2].kind, ListedKind::Dir);
        assert_eq!(got[2].name, "[ELLE] Bifrost/Main");
        assert_eq!(
            got[3].name,
            "[ELLE] Bifrost/Images/Screenshot 2023-04-01 105001.jpg"
        );
    }

    #[test]
    fn accepts_unix_line_endings_and_a_missing_final_newline() {
        let text = b"-rw-r--r--  0 0      0           5 Jan  2  2020 a.txt\n\
-rw-r--r--  0 0      0           6 Jan  2  2020 b.txt";
        let got = entries(text);
        assert_eq!(got.len(), 2);
        assert_eq!(got[1].name, "b.txt");
        assert!(entries(b"").is_empty());
    }

    #[test]
    fn keeps_spaces_in_names_exactly() {
        let text = b"-rw-r--r--  0 0      0           5 Jan  2  2020 two  spaces  here.txt\r\n\
-rw-r--r--  0 0      0           5 Jan  2  2020  leading.txt\r\n\
-rw-r--r--  0 0      0           5 Jan  2  2020 trailing.txt \r\n\
-rw-r--r--  0 0      0           5 Jan  2 12:34 12 Jan 2020 not a date.txt\r\n";
        let got = entries(text);
        assert_eq!(got[0].name, "two  spaces  here.txt");
        assert_eq!(got[1].name, " leading.txt");
        assert_eq!(got[2].name, "trailing.txt ");
        assert_eq!(got[3].name, "12 Jan 2020 not a date.txt");
    }

    #[test]
    fn a_name_that_ends_in_a_space_is_refused_by_the_path_rules() {
        let text = b"-rw-r--r--  0 0      0           5 Jan  2  2020 trailing.txt \r\n";
        let err = plan_from_listing(entries(text)).expect_err("refused");
        assert!(matches!(err, ContentError::InvalidPath { .. }), "{err:?}");
    }

    #[test]
    fn a_name_with_a_leading_space_is_kept_and_validated_as_is() {
        let text = b"-rw-r--r--  0 0      0           5 Jan  2  2020  leading.txt\r\n";
        let plan = plan_from_listing(entries(text)).expect("accepted");
        assert_eq!(plan.files[0].0.as_str(), " leading.txt");
    }

    #[test]
    fn directories_are_not_files_but_are_expected_folders() {
        let text = b"drwxr-xr-x  0 0      0           0 Jan  2  2020 mods/empty\r\n\
-rw-r--r--  0 0      0           5 Jan  2  2020 meshes/a/b.nif\r\n";
        let plan = plan_from_listing(entries(text)).expect("accepted");
        assert_eq!(plan.files.len(), 1);
        assert_eq!(plan.declared_total, 5);
        for dir in ["mods", "mods/empty", "meshes", "meshes/a"] {
            assert!(plan.dirs.contains(dir), "{dir}");
        }
        assert!(!plan.dirs.contains("meshes/a/b.nif"));
    }

    #[test]
    fn links_devices_and_other_special_entries_are_refused() {
        for line in [
            &b"lrwxrwxrwx  0 0      0           0 Jan  2  2020 evil -> /etc/passwd\r\n"[..],
            b"hrw-r--r--  0 0      0           0 Jan  2  2020 copy link to original\r\n",
            b"crw-r--r--  0 0      0           0 Jan  2  2020 device\r\n",
            b"prw-r--r--  0 0      0           0 Jan  2  2020 fifo\r\n",
        ] {
            let err = parse_tar_listing(line).expect_err("refused");
            assert!(matches!(err, ContentError::InvalidPath { .. }), "{err:?}");
        }
    }

    #[test]
    fn a_line_that_cannot_be_read_refuses_the_whole_archive() {
        let good = "-rw-r--r--  0 0      0           5 Jan  2  2020 a.txt\r\n";
        for bad in [
            "tar.exe: Archive entry has empty or unreadable filename ... skipping\r\n",
            "garbage\r\n",
            "-rw-r--r--  0 0      0           5 Jan  2  2020\r\n",
            "-rw-r--r--  0 0      0           5 Jan  2  2020 \r\n",
            "-rw-r--r--  0 0      0         five Jan  2  2020 a.txt\r\n",
            "-rw-r--r--  0 0      0          -5 Jan  2  2020 a.txt\r\n",
            "-rw-r--r--  0 0      0           5 January 2  2020 a.txt\r\n",
            "-rw-r--r--  0 0      0           5 Jan  2  20 a.txt\r\n",
            "-rw-r--r--  0 0      0           5 Jan  2  2020a.txt\r\n",
            "-rw-r--r--  x 0      0           5 Jan  2  2020 a.txt\r\n",
            "?rw-r--r--  0 0      0           5 Jan  2  2020 a.txt\r\n",
            "-rw  0 0      0           5 Jan  2  2020 a.txt\r\n",
            "\r\n",
        ] {
            let text = format!("{good}{bad}{good}");
            let err = parse_tar_listing(text.as_bytes())
                .err()
                .unwrap_or_else(|| panic!("accepted {bad:?}"));
            assert!(
                matches!(err, ContentError::CorruptArchive(_)),
                "{bad:?}: {err:?}"
            );
        }
        // A blank line in the middle is not something tar prints either.
        let text = format!("{good}\r\n{good}");
        assert!(parse_tar_listing(text.as_bytes()).is_err());
    }

    #[test]
    fn the_listing_goes_through_the_same_path_and_collision_rules() {
        let line =
            |name: &str| format!("-rw-r--r--  0 0      0           5 Jan  2  2020 {name}\r\n");
        let plan = |names: &[&str]| {
            let text: String = names.iter().map(|n| line(n)).collect();
            plan_from_listing(parse_tar_listing(text.as_bytes()).unwrap())
        };
        assert!(plan(&["../escape.txt"]).is_err());
        assert!(plan(&["/abs.txt"]).is_err());
        assert!(plan(&["a/./b.txt"]).is_err());
        assert!(plan(&["CON"]).is_err());
        assert!(plan(&["File.txt", "file.txt"]).is_err());
        assert!(plan(&["a", "a/b.txt"]).is_err());
        assert!(matches!(plan(&[]), Err(ContentError::EmptyItem)));
        assert!(plan(&["a/b.txt", "a/c.txt"]).is_ok());

        // A folder that shares a file's name, and a folder with a bad name, are refused too.
        let text = format!(
            "{}drwxr-xr-x  0 0      0           0 Jan  2  2020 A.TXT\r\n",
            line("a.txt")
        );
        assert!(plan_from_listing(parse_tar_listing(text.as_bytes()).unwrap()).is_err());
        let text = format!(
            "{}drwxr-xr-x  0 0      0           0 Jan  2  2020 ../up\r\n",
            line("a.txt")
        );
        assert!(plan_from_listing(parse_tar_listing(text.as_bytes()).unwrap()).is_err());
    }

    #[test]
    fn declared_sizes_are_summed() {
        let text = b"-rw-r--r--  0 0      0   100 Jan  2  2020 a.txt\r\n\
-rw-r--r--  0 0      0   250 Jan  2  2020 b/c.txt\r\n\
-rw-r--r--  0 0      0 18446744073709551615 Jan  2  2020 huge.bin\r\n";
        let plan = plan_from_listing(entries(text)).unwrap();
        assert_eq!(
            plan.declared_total,
            u64::MAX,
            "saturates instead of wrapping"
        );
    }

    #[test]
    fn tar_failures_are_reported_with_tars_own_words() {
        let err = tar_failure(b"tar.exe: Declared dictionary size is not supported\r\n");
        match err {
            ContentError::CorruptArchive(text) => {
                assert!(
                    text.contains("Declared dictionary size is not supported"),
                    "{text}"
                );
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            tar_failure(b"tar.exe: Passphrase required for this entry\r\n"),
            ContentError::PasswordProtected
        ));
    }

    #[test]
    fn rar_signatures_are_recognised() {
        assert!(is_rar_magic(b"Rar!\x1a\x07\x00\xcf"));
        assert!(is_rar_magic(b"Rar!\x1a\x07\x01\x00"));
        assert!(!is_rar_magic(b"Rar!\x1a\x07\x02\x00"));
        assert!(!is_rar_magic(b"Rar!\x1a\x07"));
        assert!(!is_rar_magic(b"PK\x03\x04"));
    }

    fn plan_of(files: &[(&str, u64)], dirs: &[&str]) -> RarPlan {
        let mut text = String::new();
        for (name, size) in files {
            text.push_str(&format!(
                "-rw-r--r--  0 0      0 {size} Jan  2  2020 {name}\r\n"
            ));
        }
        for name in dirs {
            text.push_str(&format!("drwxr-xr-x  0 0      0 0 Jan  2  2020 {name}\r\n"));
        }
        plan_from_listing(parse_tar_listing(text.as_bytes()).unwrap()).unwrap()
    }

    fn write(root: &Path, rel: &str, bytes: &[u8]) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn the_extracted_tree_must_match_the_listing_exactly() {
        let plan = plan_of(&[("a/one.txt", 3), ("two.txt", 2)], &["empty"]);

        let ok = tempfile::tempdir().unwrap();
        write(ok.path(), "a/one.txt", b"abc");
        write(ok.path(), "two.txt", b"xy");
        std::fs::create_dir_all(ok.path().join("empty")).unwrap();
        check_extracted_tree(ok.path(), &plan).expect("matches");

        let missing = tempfile::tempdir().unwrap();
        write(missing.path(), "a/one.txt", b"abc");
        assert!(check_extracted_tree(missing.path(), &plan).is_err());

        let wrong_size = tempfile::tempdir().unwrap();
        write(wrong_size.path(), "a/one.txt", b"abcd");
        write(wrong_size.path(), "two.txt", b"xy");
        assert!(check_extracted_tree(wrong_size.path(), &plan).is_err());

        let extra_file = tempfile::tempdir().unwrap();
        write(extra_file.path(), "a/one.txt", b"abc");
        write(extra_file.path(), "two.txt", b"xy");
        write(extra_file.path(), "a/extra.txt", b"!");
        assert!(check_extracted_tree(extra_file.path(), &plan).is_err());

        let extra_dir = tempfile::tempdir().unwrap();
        write(extra_dir.path(), "a/one.txt", b"abc");
        write(extra_dir.path(), "two.txt", b"xy");
        std::fs::create_dir_all(extra_dir.path().join("surprise")).unwrap();
        assert!(check_extracted_tree(extra_dir.path(), &plan).is_err());

        let case = tempfile::tempdir().unwrap();
        write(case.path(), "A/one.txt", b"abc");
        write(case.path(), "two.txt", b"xy");
        assert!(check_extracted_tree(case.path(), &plan).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn a_junction_in_the_extracted_tree_is_refused() {
        let plan = plan_of(&[("two.txt", 2)], &["link"]);
        let root = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        write(root.path(), "two.txt", b"xy");
        let status = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(root.path().join("link"))
            .arg(target.path())
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        let err = check_extracted_tree(root.path(), &plan).expect_err("refused");
        assert!(matches!(err, ContentError::InvalidPath { .. }), "{err:?}");
    }

    #[cfg(windows)]
    #[test]
    fn tar_is_found_in_the_windows_system_folder() {
        let tar = tar_exe_path().expect("tar.exe ships with Windows 10 and later");
        assert!(tar.is_absolute());
        assert!(tar.ends_with("tar.exe"));
    }

    #[cfg(not(windows))]
    #[test]
    fn rar_is_an_error_where_there_is_no_windows_tar() {
        let err = tar_exe_path().expect_err("no tar.exe");
        assert!(err.to_string().contains("Windows' built-in tar"));
    }

    // ---- real archives (run with `cargo test -p agora-core -- --ignored rar`) ----

    fn vortex_rars() -> Vec<PathBuf> {
        let dir = std::env::var_os("AGORA_TEST_RAR_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("APPDATA").expect("APPDATA"))
                    .join("Vortex/downloads/skyrimse")
            });
        let mut found: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("rar")))
            .collect();
        found.sort();
        found
    }

    /// Every real RAR either lists, parses and passes the path rules, or is refused with a reason
    /// (tar itself cannot read it, or a name breaks a rule). None may crash or be misread.
    #[test]
    #[ignore = "reads the real Vortex RAR downloads"]
    fn real_listings_parse_or_are_refused_with_a_reason() {
        let tar = tar_exe_path().unwrap();
        let (mut accepted, mut refused) = (0, 0);
        for rar in vortex_rars() {
            let result = run_listing(&tar, &rar)
                .and_then(|text| parse_tar_listing(&text))
                .and_then(plan_from_listing);
            match result {
                Ok(plan) => {
                    assert!(!plan.files.is_empty());
                    accepted += 1;
                }
                Err(e) => {
                    refused += 1;
                    println!("refused {}: {e}", rar.display());
                }
            }
        }
        println!("accepted {accepted}, refused {refused}");
        assert!(accepted > 100, "only {accepted} accepted");
    }
}
