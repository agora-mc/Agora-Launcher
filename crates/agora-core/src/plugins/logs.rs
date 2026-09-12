//! Per-plugin log files.
//!
//! A plugin gets its own file so that "which plugin did this?" never requires
//! reading the launcher's log, and so a chatty plugin cannot bury a launcher
//! diagnostic. Rotation is a single generation: enough to survive a plugin
//! that logs in a loop, without accumulating history nobody reads.

use agora_plugin_api::manifest::PluginId;
use agora_plugin_api::protocol::LogLevel;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Size at which the current log is rotated to `.1` and started again.
pub const ROTATE_AT_BYTES: u64 = 1024 * 1024;

/// Longest single line written. A plugin logging a megabyte-long string gets
/// truncated rather than filling the file in one call.
pub const MAX_LINE_CHARS: usize = 4_000;

pub fn log_path(logs_root: &Path, plugin_id: &PluginId) -> PathBuf {
    // `PluginId` is lowercase ASCII with a single dot, so it is already a
    // valid single filename component on every platform Agora targets.
    logs_root.join(format!("{}.log", plugin_id.as_str()))
}

/// Append one line. Failures are swallowed: a plugin that cannot be logged is
/// still a plugin that should run, and there is nowhere useful to report a
/// logging failure to.
pub fn append(logs_root: &Path, plugin_id: &PluginId, level: LogLevel, message: &str) {
    let path = log_path(logs_root, plugin_id);
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    rotate_if_needed(&path);

    let truncated = if message.chars().count() > MAX_LINE_CHARS {
        let mut text: String = message.chars().take(MAX_LINE_CHARS).collect();
        text.push_str(" … (truncated)");
        text
    } else {
        message.to_string()
    };
    // Newlines would let a plugin forge log entries that look like the
    // launcher's own, so they become a visible escape instead.
    let single_line = truncated.replace('\n', "\\n").replace('\r', "");
    let stamp = chrono::Utc::now().to_rfc3339();

    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(file, "{stamp} [{level}] {single_line}");
    }
}

fn rotate_if_needed(path: &Path) {
    let Ok(metadata) = std::fs::metadata(path) else {
        return;
    };
    if metadata.len() < ROTATE_AT_BYTES {
        return;
    }
    let rotated = path.with_extension("log.1");
    let _ = std::fs::remove_file(&rotated);
    let _ = std::fs::rename(path, &rotated);
}

/// The last `lines` lines of a plugin's log, oldest first.
pub fn tail(logs_root: &Path, plugin_id: &PluginId, lines: usize) -> Vec<String> {
    let path = log_path(logs_root, plugin_id);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let all: Vec<&str> = text.lines().collect();
    let start = all.len().saturating_sub(lines);
    all[start..].iter().map(|line| line.to_string()).collect()
}

/// Delete a plugin's logs. Part of uninstalling, not of disabling.
pub fn remove(logs_root: &Path, plugin_id: &PluginId) {
    let path = log_path(logs_root, plugin_id);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("log.1"));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id() -> PluginId {
        PluginId::parse("acme.one").unwrap()
    }

    #[test]
    fn a_line_is_written_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        append(dir.path(), &id(), LogLevel::Info, "hello");
        let lines = tail(dir.path(), &id(), 10);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("[info] hello"), "{}", lines[0]);
    }

    #[test]
    fn a_plugin_cannot_forge_extra_log_lines_with_a_newline() {
        let dir = tempfile::tempdir().unwrap();
        append(
            dir.path(),
            &id(),
            LogLevel::Info,
            "innocent\n2026-01-01T00:00:00Z [error] the launcher is broken",
        );
        assert_eq!(tail(dir.path(), &id(), 10).len(), 1);
    }

    #[test]
    fn an_enormous_line_is_truncated_rather_than_written_whole() {
        let dir = tempfile::tempdir().unwrap();
        let huge = "x".repeat(MAX_LINE_CHARS * 2);
        append(dir.path(), &id(), LogLevel::Warn, &huge);
        let lines = tail(dir.path(), &id(), 1);
        assert!(lines[0].contains("truncated"), "{}", &lines[0][..80]);
        assert!(lines[0].chars().count() < MAX_LINE_CHARS + 100);
    }

    #[test]
    fn tail_returns_the_most_recent_lines() {
        let dir = tempfile::tempdir().unwrap();
        for n in 0..10 {
            append(dir.path(), &id(), LogLevel::Debug, &format!("line {n}"));
        }
        let lines = tail(dir.path(), &id(), 3);
        assert_eq!(lines.len(), 3);
        assert!(lines[2].contains("line 9"), "{}", lines[2]);
    }

    #[test]
    fn removing_a_plugin_takes_its_logs_with_it() {
        let dir = tempfile::tempdir().unwrap();
        append(dir.path(), &id(), LogLevel::Info, "hello");
        remove(dir.path(), &id());
        assert!(tail(dir.path(), &id(), 10).is_empty());
    }

    #[test]
    fn reading_a_log_that_was_never_written_is_empty_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(tail(dir.path(), &id(), 10).is_empty());
    }
}
