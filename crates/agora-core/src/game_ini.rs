//! A game's INI files, as each instance keeps its own copy (MASTER_SPEC §26.5, *Per-user files*).
//!
//! The game reads its INI files from the user's profile, shared by every instance of the game.
//! An instance that needs its own settings keeps a copy at `<instance>/<instance_path>` (for
//! Skyrim, `user/Skyrim.ini`), and the copy is swapped in for each session. This module edits that
//! copy. It never edits the game's own file, except to seed a copy the first time (see [`edit_locked`]).
//!
//! The parser keeps everything it does not change: comments, blank lines, key order, the spelling
//! of keys and sections, line endings, a byte order mark, and lines it cannot parse. Names compare
//! case-insensitively.

use std::path::{Path, PathBuf};

use agora_game_api::{GameDefinition, RelPath, StoreId, UserFileMapping};
use serde::Serialize;

use crate::ctx::Ctx;
use crate::error::LauncherError;
use crate::game_plugins::PluginListError;
use crate::game_user_files::{self, Journal};
use crate::lock_manager::{LockGuard, LockResource};

const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];
const CRLF: &[u8] = b"\r\n";
const LF: &[u8] = b"\n";
const NO_ENDING: &[u8] = b"";

#[derive(Debug, thiserror::Error)]
pub enum IniError {
    #[error("instance '{0}' not found")]
    InstanceNotFound(String),
    #[error("cannot tell which store instance '{0}' runs under, so its INI files cannot be found")]
    StoreUnknown(String),
    #[error(
        "'{file}' is not a per-user file of {game} for the '{store}' store; its files are: {known}"
    )]
    NotAGameFile {
        game: String,
        store: String,
        file: String,
        known: String,
    },
    #[error("{0}")]
    Invalid(String),
    #[error(
        "{game} ({store}) is running as instance {instance}; close it before changing an instance's INI files"
    )]
    SessionRunning {
        game: String,
        store: String,
        instance: String,
    },
    #[error(
        "an earlier session of instance {instance} still has {game}'s INI files swapped in; put them back first with `agora games user-files restore {game} {store}`"
    )]
    NeedsRestore {
        game: String,
        store: String,
        instance: String,
    },
    #[error("cannot read {}: {source}", path.display())]
    Unreadable {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot write {}: {source}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Lock(#[from] LauncherError),
    #[error("{0}")]
    Other(String),
}

/// One line of an INI file that sets a key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IniEntry {
    pub section: String,
    pub key: String,
    pub value: String,
}

/// An INI file read as text, line by line, with everything it contains kept.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IniDocument {
    bom: bool,
    lines: Vec<Line>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Line {
    /// The line's bytes, without its line ending.
    text: Vec<u8>,
    /// `\r\n`, `\n`, or nothing for a last line that has no line ending.
    ending: &'static [u8],
}

impl IniDocument {
    /// Parse a file's bytes. Nothing is an error: a line that is not a section or a key is kept.
    pub fn parse(bytes: &[u8]) -> Self {
        let (bom, body) = match bytes.strip_prefix(UTF8_BOM) {
            Some(rest) => (true, rest),
            None => (false, bytes),
        };
        Self {
            bom,
            lines: split_lines(body),
        }
    }

    /// The file's bytes, exactly as they were parsed except for the changes made.
    pub fn render(&self) -> Vec<u8> {
        let mut out = Vec::new();
        if self.bom {
            out.extend_from_slice(UTF8_BOM);
        }
        for line in &self.lines {
            out.extend_from_slice(&line.text);
            out.extend_from_slice(line.ending);
        }
        out
    }

    /// The value of a key. When the key appears more than once, the last line wins.
    pub fn get(&self, section: &str, key: &str) -> Option<String> {
        self.entry_lines(section, key)
            .last()
            .and_then(|&i| value_of(&self.lines[i].text))
    }

    /// Every key the file sets, in file order.
    pub fn entries(&self) -> Vec<IniEntry> {
        let mut out = Vec::new();
        let mut current: Option<String> = None;
        for line in &self.lines {
            if let Some(name) = header_name(&line.text) {
                current = Some(name);
                continue;
            }
            if let (Some(section), Some(key)) = (&current, entry_key(&line.text)) {
                out.push(IniEntry {
                    section: section.clone(),
                    key,
                    value: value_of(&line.text).unwrap_or_default(),
                });
            }
        }
        out
    }

    /// Set a key to a value. Every line that already sets the key changes; without one, the key is
    /// added to the section's last occurrence, and the section is added at the end when missing.
    /// Returns whether the file changed.
    pub fn set(&mut self, section: &str, key: &str, value: &str) -> Result<bool, IniError> {
        check_names(section, key)?;
        check_value(value)?;
        let found = self.entry_lines(section, key);
        if !found.is_empty() {
            let mut changed = false;
            for i in found {
                changed |= set_value_in(&mut self.lines[i].text, value);
            }
            return Ok(changed);
        }
        let entry = format!("{key}={value}").into_bytes();
        match self.last_header(section) {
            Some(header) => {
                let end = self.section_end(header);
                let at = (header + 1..end)
                    .rev()
                    .find(|&i| !is_blank(&self.lines[i].text))
                    .map_or(header + 1, |i| i + 1);
                self.insert_line(at, entry);
            }
            None => {
                if self.lines.last().is_some_and(|l| !is_blank(&l.text)) {
                    self.insert_line(self.lines.len(), Vec::new());
                }
                self.insert_line(self.lines.len(), format!("[{section}]").into_bytes());
                self.insert_line(self.lines.len(), entry);
            }
        }
        Ok(true)
    }

    /// Remove every line that sets the key. Returns whether the file changed.
    pub fn unset(&mut self, section: &str, key: &str) -> Result<bool, IniError> {
        check_names(section, key)?;
        let found = self.entry_lines(section, key);
        if found.is_empty() {
            return Ok(false);
        }
        // Each remaining line keeps its own ending. A file that ended without one may end with the
        // line before the removed one, which keeps its newline: the rest of the file is untouched.
        for &i in found.iter().rev() {
            self.lines.remove(i);
        }
        Ok(true)
    }

    /// Indexes of the lines that set `key` in `section` (names compare case-insensitively).
    fn entry_lines(&self, section: &str, key: &str) -> Vec<usize> {
        let mut current: Option<String> = None;
        let mut found = Vec::new();
        for (i, line) in self.lines.iter().enumerate() {
            if let Some(name) = header_name(&line.text) {
                current = Some(name);
                continue;
            }
            if let (Some(name), Some(k)) = (&current, entry_key(&line.text)) {
                if name.eq_ignore_ascii_case(section) && k.eq_ignore_ascii_case(key) {
                    found.push(i);
                }
            }
        }
        found
    }

    fn last_header(&self, section: &str) -> Option<usize> {
        (0..self.lines.len()).rev().find(|&i| {
            header_name(&self.lines[i].text).is_some_and(|n| n.eq_ignore_ascii_case(section))
        })
    }

    /// The index of the line after `header`'s section: the next section header, or the end.
    fn section_end(&self, header: usize) -> usize {
        (header + 1..self.lines.len())
            .find(|&i| header_name(&self.lines[i].text).is_some())
            .unwrap_or(self.lines.len())
    }

    /// The line ending new lines use: the one the file uses more of, CRLF for a file without any.
    fn newline(&self) -> &'static [u8] {
        let crlf = self.lines.iter().filter(|l| l.ending == CRLF).count();
        let lf = self.lines.iter().filter(|l| l.ending == LF).count();
        if lf > crlf {
            LF
        } else {
            CRLF
        }
    }

    fn insert_line(&mut self, at: usize, text: Vec<u8>) {
        let newline = self.newline();
        let appends_after_bare_last =
            at == self.lines.len() && self.lines.last().is_some_and(|l| l.ending.is_empty());
        if appends_after_bare_last {
            // The file had no line ending at its end: the line before gets one, and the new last
            // line keeps the file without one.
            if let Some(last) = self.lines.last_mut() {
                last.ending = newline;
            }
            self.lines.push(Line {
                text,
                ending: NO_ENDING,
            });
        } else {
            self.lines.insert(
                at,
                Line {
                    text,
                    ending: newline,
                },
            );
        }
    }
}

/// Split bytes into lines, each keeping its own line ending.
fn split_lines(bytes: &[u8]) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if b != b'\n' {
            continue;
        }
        let (end, ending) = if i > start && bytes[i - 1] == b'\r' {
            (i - 1, CRLF)
        } else {
            (i, LF)
        };
        lines.push(Line {
            text: bytes[start..end].to_vec(),
            ending,
        });
        start = i + 1;
    }
    if start < bytes.len() {
        lines.push(Line {
            text: bytes[start..].to_vec(),
            ending: NO_ENDING,
        });
    }
    lines
}

fn is_ws(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

fn trim_ws(bytes: &[u8]) -> &[u8] {
    let start = bytes.iter().position(|&b| !is_ws(b)).unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|&b| !is_ws(b))
        .map_or(start, |i| i + 1);
    &bytes[start..end.max(start)]
}

fn is_blank(text: &[u8]) -> bool {
    trim_ws(text).is_empty()
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// The section a `[name]` line opens, or `None` when the line is not a section header.
fn header_name(text: &[u8]) -> Option<String> {
    let trimmed = trim_ws(text);
    if trimmed.first() != Some(&b'[') {
        return None;
    }
    let rest = &trimmed[1..];
    let close = rest.iter().position(|&b| b == b']')?;
    let name = trim_ws(&rest[..close]);
    if name.is_empty() {
        None
    } else {
        Some(lossy(name))
    }
}

/// The key a `key = value` line sets, or `None` for a comment, a header, or any other line.
fn entry_key(text: &[u8]) -> Option<String> {
    let trimmed = trim_ws(text);
    match trimmed.first() {
        None | Some(b';') | Some(b'#') | Some(b'[') => return None,
        _ => {}
    }
    let eq = text.iter().position(|&b| b == b'=')?;
    let key = trim_ws(&text[..eq]);
    if key.is_empty() {
        None
    } else {
        Some(lossy(key))
    }
}

/// Where the value of a `key = value` line starts and ends, leaving the spacing around it alone.
fn value_span(text: &[u8]) -> Option<(usize, usize)> {
    let eq = text.iter().position(|&b| b == b'=')?;
    let mut start = eq + 1;
    while start < text.len() && is_ws(text[start]) {
        start += 1;
    }
    let mut end = text.len();
    while end > start && is_ws(text[end - 1]) {
        end -= 1;
    }
    Some((start, end))
}

fn value_of(text: &[u8]) -> Option<String> {
    value_span(text).map(|(start, end)| lossy(&text[start..end]))
}

/// Replace the value on a key line. Returns whether the line changed.
fn set_value_in(text: &mut Vec<u8>, value: &str) -> bool {
    let Some((start, end)) = value_span(text) else {
        return false;
    };
    if text[start..end] == *value.as_bytes() {
        return false;
    }
    let mut replaced = Vec::with_capacity(text.len() + value.len());
    replaced.extend_from_slice(&text[..start]);
    replaced.extend_from_slice(value.as_bytes());
    replaced.extend_from_slice(&text[end..]);
    *text = replaced;
    true
}

/// Names must be one line, not padded, and must not break the file's own syntax.
pub(crate) fn check_names(section: &str, key: &str) -> Result<(), IniError> {
    if section.is_empty() || section.trim() != section || section.contains(['[', ']', '\r', '\n']) {
        return Err(IniError::Invalid(format!(
            "'{section}' is not a section name: one line, no brackets, no spaces at the ends"
        )));
    }
    if key.is_empty()
        || key.trim() != key
        || key.starts_with([';', '#'])
        || key.contains(['=', '[', ']', '\r', '\n'])
    {
        return Err(IniError::Invalid(format!(
            "'{key}' is not a key name: one line, no '=' or brackets, no spaces at the ends, not a comment"
        )));
    }
    Ok(())
}

/// A value is one line with no padding, so reading it back gives the same text.
pub(crate) fn check_value(value: &str) -> Result<(), IniError> {
    if value.contains(['\r', '\n']) || value.trim() != value {
        return Err(IniError::Invalid(format!(
            "'{value}' is not a value: one line, with no spaces at the ends"
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The instance's copies
// ---------------------------------------------------------------------------

/// A per-user file the game keeps for an instance's store, and where its copy and the game's own
/// file are.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IniFile {
    pub instance_path: String,
    pub copy_exists: bool,
    pub game_file: PathBuf,
    pub game_file_exists: bool,
}

/// Where a file's contents came from when it was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IniSource {
    /// The instance's own copy.
    Copy,
    /// The game's file, because the instance has no copy yet.
    GameFile,
    /// Neither exists: the file reads as empty.
    Nothing,
}

/// A file as the instance sees it.
#[derive(Debug)]
pub struct IniRead {
    pub instance_path: String,
    pub source: IniSource,
    pub document: IniDocument,
}

/// The store an instance runs under. Its per-user files are the game's for that store.
pub fn store_of(ctx: &Ctx, instance_id: &str) -> Result<StoreId, IniError> {
    let store = crate::game_plugins::instance_store(ctx, instance_id).map_err(|e| match e {
        PluginListError::InstanceNotFound(id) => IniError::InstanceNotFound(id),
        other => IniError::Other(other.to_string()),
    })?;
    store.ok_or_else(|| IniError::StoreUnknown(instance_id.to_string()))
}

fn instance_dir(ctx: &Ctx, instance_id: &str) -> Result<PathBuf, IniError> {
    ctx.paths
        .instance_dir(instance_id)
        .map_err(|e| IniError::Other(e.to_string()))
}

/// `dir` joined with a `/`-separated relative path, one component at a time.
pub(crate) fn join_rel(dir: &Path, rel: &str) -> PathBuf {
    rel.split('/')
        .filter(|part| !part.is_empty())
        .fold(dir.to_path_buf(), |path, part| path.join(part))
}

fn game_file_of(mapping: &UserFileMapping) -> Result<PathBuf, IniError> {
    game_user_files::resolve_user_file_source(&mapping.source)
        .map_err(|e| IniError::Other(e.to_string()))
}

fn mapping_for<'a>(
    definition: &'a GameDefinition,
    store: &StoreId,
    file: &RelPath,
) -> Option<&'a UserFileMapping> {
    definition.user_files.iter().find(|m| {
        m.applies_to_store(store) && m.instance_path.as_str().eq_ignore_ascii_case(file.as_str())
    })
}

/// The file a name refers to for this store: a valid relative path that the game keeps.
fn resolve_file<'a>(
    definition: &'a GameDefinition,
    store: &StoreId,
    file: &str,
) -> Result<(RelPath, &'a UserFileMapping), IniError> {
    let rel = RelPath::new(file).map_err(|e| IniError::Invalid(e.to_string()))?;
    match mapping_for(definition, store, &rel) {
        Some(mapping) => Ok((rel, mapping)),
        None => {
            let known = definition
                .user_files
                .iter()
                .filter(|m| m.applies_to_store(store))
                .map(|m| m.instance_path.as_str().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            Err(IniError::NotAGameFile {
                game: definition.id.as_str().to_string(),
                store: store.as_str().to_string(),
                file: file.to_string(),
                known,
            })
        }
    }
}

/// The game's per-user files for an instance, and whether each has a copy yet.
pub fn list_files(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
) -> Result<Vec<IniFile>, IniError> {
    let store = store_of(ctx, instance_id)?;
    let dir = instance_dir(ctx, instance_id)?;
    let mut out = Vec::new();
    for mapping in definition
        .user_files
        .iter()
        .filter(|m| m.applies_to_store(&store))
    {
        let game_file = game_file_of(mapping)?;
        out.push(IniFile {
            instance_path: mapping.instance_path.as_str().to_string(),
            copy_exists: join_rel(&dir, mapping.instance_path.as_str()).exists(),
            game_file_exists: game_file.exists(),
            game_file,
        });
    }
    Ok(out)
}

/// Read a file as the instance sees it. Reading never seeds a copy.
pub fn read(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    file: &str,
) -> Result<IniRead, IniError> {
    let store = store_of(ctx, instance_id)?;
    let (rel, mapping) = resolve_file(definition, &store, file)?;
    let copy = join_rel(&instance_dir(ctx, instance_id)?, rel.as_str());
    let game_file = game_file_of(mapping)?;
    let (document, source) = if copy.exists() {
        (load(&copy)?, IniSource::Copy)
    } else if game_file.exists() {
        (load(&game_file)?, IniSource::GameFile)
    } else {
        (IniDocument::default(), IniSource::Nothing)
    };
    Ok(IniRead {
        instance_path: rel.as_str().to_string(),
        source,
        document,
    })
}

fn load(path: &Path) -> Result<IniDocument, IniError> {
    let bytes = std::fs::read(path).map_err(|source| IniError::Unreadable {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(IniDocument::parse(&bytes))
}

/// Set a key in one of the instance's files. Returns whether the file changed.
pub fn set_value(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    file: &str,
    section: &str,
    key: &str,
    value: &str,
) -> Result<bool, IniError> {
    check_names(section, key)?;
    check_value(value)?;
    let store = store_of(ctx, instance_id)?;
    let _locks = lock_for_edit(ctx, instance_id, definition, &store)?;
    edit_locked(ctx, instance_id, definition, &store, file, |doc| {
        let changed = doc.set(section, key, value)?;
        Ok((changed, changed))
    })
}

/// Remove a key from one of the instance's files. Returns whether the file changed.
pub fn unset_value(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    file: &str,
    section: &str,
    key: &str,
) -> Result<bool, IniError> {
    check_names(section, key)?;
    let store = store_of(ctx, instance_id)?;
    let _locks = lock_for_edit(ctx, instance_id, definition, &store)?;
    edit_locked(ctx, instance_id, definition, &store, file, |doc| {
        let changed = doc.unset(section, key)?;
        Ok((changed, changed))
    })
}

/// The two locks an edit holds: the instance's, and the game's per-user files for the store, the
/// lock a launch's swap takes. An edit takes the instance's first, as deployment does.
pub(crate) fn lock_for_edit(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    store: &StoreId,
) -> Result<(LockGuard, LockGuard), IniError> {
    let instance = ctx
        .lock_manager
        .acquire(LockResource::Instance(instance_id.to_string()), "ini-edit")?;
    let user_files = ctx.lock_manager.acquire(
        LockResource::GameUserFiles(definition.id.clone(), store.clone()),
        "ini-edit",
    )?;
    Ok((instance, user_files))
}

/// Edit the instance's copy of `file` with `f`, which returns its own result and whether it changed
/// the document. The copy is written only when the document changed.
///
/// The caller holds the locks from [`lock_for_edit`]. Refused while the game's session is running,
/// and while an earlier session's swap has not been put back: either way the game's file is not the
/// instance's to read or write. A copy that does not exist yet is seeded from the game's file, and
/// that file must exist or the copy starts empty.
pub(crate) fn edit_locked<T>(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    store: &StoreId,
    file: &str,
    f: impl FnOnce(&mut IniDocument) -> Result<(T, bool), IniError>,
) -> Result<T, IniError> {
    let (rel, mapping) = resolve_file(definition, store, file)?;
    refuse_during_session(ctx, definition, store)?;
    let copy = join_rel(&instance_dir(ctx, instance_id)?, rel.as_str());
    let mut document = if copy.exists() {
        load(&copy)?
    } else {
        let game_file = game_file_of(mapping)?;
        if game_file.exists() {
            load(&game_file)?
        } else {
            IniDocument::default()
        }
    };
    let (value, changed) = f(&mut document)?;
    if changed {
        write_atomic(&copy, &document.render())?;
    }
    Ok(value)
}

/// Refuse an edit while a session of this game and store is running, or has not been put back.
fn refuse_during_session(
    ctx: &Ctx,
    definition: &GameDefinition,
    store: &StoreId,
) -> Result<(), IniError> {
    let journal_path = ctx
        .paths
        .user_files_journal_path(definition.id.as_str(), store.as_str());
    if !journal_path.exists() {
        return Ok(());
    }
    let text = std::fs::read_to_string(&journal_path).map_err(|e| {
        IniError::Other(format!(
            "cannot read the session journal {}: {e}",
            journal_path.display()
        ))
    })?;
    let journal: Journal = serde_json::from_str(&text).map_err(|e| {
        IniError::Other(format!(
            "the session journal {} cannot be read: {e}",
            journal_path.display()
        ))
    })?;
    let game = definition.id.as_str().to_string();
    let store_name = store.as_str().to_string();
    if game_user_files::is_session_running(&journal) {
        Err(IniError::SessionRunning {
            game,
            store: store_name,
            instance: journal.instance_id,
        })
    } else {
        Err(IniError::NeedsRestore {
            game,
            store: store_name,
            instance: journal.instance_id,
        })
    }
}

/// Write a file through a temporary file beside it, so an interrupted write leaves the old file.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), IniError> {
    let write_err = |source| IniError::Write {
        path: path.to_path_buf(),
        source,
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(write_err)?;
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    let tmp = path.with_file_name(format!("{name}.agora-tmp"));
    std::fs::write(&tmp, bytes).map_err(write_err)?;
    std::fs::rename(&tmp, path).map_err(write_err)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(doc: &IniDocument) -> String {
        String::from_utf8(doc.render()).unwrap()
    }

    #[test]
    fn an_untouched_file_renders_exactly_as_read() {
        let original =
            "\u{feff}; comment\r\n\r\n[General]\r\n  sFoo =  bar  \r\n junk line\r\n[Other]\nx=1";
        let doc = IniDocument::parse(original.as_bytes());
        assert_eq!(doc.render(), original.as_bytes());
    }

    #[test]
    fn setting_one_key_changes_only_that_line() {
        let original = "\u{feff}; header\r\n\r\n[General]\r\nsFoo=1\r\n  iBar   =  2  \r\n\r\n[Dup]\r\na=1\r\n[General]\r\n# note\r\nsBaz = q\r\njunk here\r\n";
        let mut doc = IniDocument::parse(original.as_bytes());
        assert!(doc.set("General", "iBar", "9").unwrap());
        // The spacing around the value, including the trailing spaces, is kept.
        assert_eq!(
            text(&doc),
            original.replace("  iBar   =  2  ", "  iBar   =  9  ")
        );
    }

    #[test]
    fn setting_the_same_value_changes_nothing() {
        let original = "[General]\r\nsFoo=bar\r\n";
        let mut doc = IniDocument::parse(original.as_bytes());
        assert!(!doc.set("General", "sFoo", "bar").unwrap());
        assert_eq!(doc.render(), original.as_bytes());
    }

    #[test]
    fn names_compare_case_insensitively_and_keep_their_spelling() {
        let original = "[general]\r\nSLocalSavePath = old\r\n";
        let mut doc = IniDocument::parse(original.as_bytes());
        assert_eq!(doc.get("General", "slocalsavepath").as_deref(), Some("old"));
        assert!(doc.set("GENERAL", "SLOCALSAVEPATH", "new").unwrap());
        assert_eq!(text(&doc), "[general]\r\nSLocalSavePath = new\r\n");
    }

    #[test]
    fn a_missing_section_is_added_at_the_end_with_the_files_line_endings() {
        let original = "[Other]\r\na=1\r\n";
        let mut doc = IniDocument::parse(original.as_bytes());
        assert!(doc.set("General", "k", "v").unwrap());
        assert_eq!(text(&doc), "[Other]\r\na=1\r\n\r\n[General]\r\nk=v\r\n");
    }

    #[test]
    fn a_missing_key_is_added_after_its_sections_last_key() {
        let original = "[General]\na=1\n\n[Next]\nb=2\n";
        let mut doc = IniDocument::parse(original.as_bytes());
        assert!(doc.set("General", "k", "v").unwrap());
        assert_eq!(text(&doc), "[General]\na=1\nk=v\n\n[Next]\nb=2\n");
    }

    #[test]
    fn a_key_is_added_to_the_last_of_duplicate_sections() {
        let original = "[General]\na=1\n[Other]\nb=2\n[General]\nc=3\n";
        let mut doc = IniDocument::parse(original.as_bytes());
        assert!(doc.set("General", "k", "v").unwrap());
        assert_eq!(
            text(&doc),
            "[General]\na=1\n[Other]\nb=2\n[General]\nc=3\nk=v\n"
        );
    }

    #[test]
    fn a_file_without_a_final_newline_stays_without_one() {
        let original = "[General]\na=1";
        let mut doc = IniDocument::parse(original.as_bytes());
        assert!(doc.set("General", "b", "2").unwrap());
        assert_eq!(text(&doc), "[General]\na=1\nb=2");
        assert!(doc.set("Other", "c", "3").unwrap());
        assert_eq!(text(&doc), "[General]\na=1\nb=2\n\n[Other]\nc=3");
    }

    #[test]
    fn duplicate_keys_all_change_and_get_returns_the_last() {
        let original = "[General]\na=1\nb=2\na=3\n";
        let mut doc = IniDocument::parse(original.as_bytes());
        assert_eq!(doc.get("General", "a").as_deref(), Some("3"));
        assert!(doc.set("General", "a", "7").unwrap());
        assert_eq!(text(&doc), "[General]\na=7\nb=2\na=7\n");
    }

    #[test]
    fn unset_removes_only_the_key_lines() {
        let original = "[General]\r\n; keep\r\na=1\r\n\r\nb=2\r\na = 3\r\n[Other]\r\na=4\r\n";
        let mut doc = IniDocument::parse(original.as_bytes());
        assert!(doc.unset("General", "A").unwrap());
        assert_eq!(
            text(&doc),
            "[General]\r\n; keep\r\n\r\nb=2\r\n[Other]\r\na=4\r\n"
        );
        assert!(!doc.unset("General", "a").unwrap());
    }

    #[test]
    fn unset_of_the_last_line_leaves_the_line_before_its_ending() {
        let mut doc = IniDocument::parse(b"[General]\na=1\nb=2");
        assert!(doc.unset("General", "b").unwrap());
        assert_eq!(doc.render(), b"[General]\na=1\n".to_vec());
    }

    #[test]
    fn a_file_with_only_comments_round_trips() {
        let original = "; one\r\n# two\r\n";
        assert_eq!(
            IniDocument::parse(original.as_bytes()).render(),
            original.as_bytes()
        );
    }

    #[test]
    fn entries_list_every_key_in_order_with_its_section() {
        let doc = IniDocument::parse(b"[A]\r\nx=1\r\n; no\r\njunk\r\n[B]\r\ny = two words\r\n");
        let entries = doc.entries();
        assert_eq!(
            entries,
            vec![
                IniEntry {
                    section: "A".into(),
                    key: "x".into(),
                    value: "1".into()
                },
                IniEntry {
                    section: "B".into(),
                    key: "y".into(),
                    value: "two words".into()
                },
            ]
        );
    }

    #[test]
    fn names_and_values_that_would_break_the_file_are_refused() {
        let mut doc = IniDocument::default();
        assert!(doc.set("General", "a\nb", "1").is_err());
        assert!(doc.set("General", "a=b", "1").is_err());
        assert!(doc.set("General", "", "1").is_err());
        assert!(doc.set("Gen]eral", "a", "1").is_err());
        assert!(doc.set("General", "a", "x\r\ny").is_err());
        assert!(doc.set("General", "a", " padded").is_err());
        assert!(doc.set("General", ";a", "1").is_err());
        assert_eq!(doc.render(), Vec::<u8>::new());
    }

    #[test]
    fn an_empty_file_gets_crlf_endings() {
        let mut doc = IniDocument::parse(b"");
        assert!(doc.set("General", "a", "1").unwrap());
        assert_eq!(doc.render(), b"[General]\r\na=1\r\n".to_vec());
    }
}
