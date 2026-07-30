//! Local-first crash evidence collector.
//!
//! Discovers a coherent set of crash evidence from an instance directory:
//! `crash-reports/*.txt`, `logs/latest.log`, `logs/debug.log`,
//! `hs_err_pid*.log`, plus explicitly user‑added files.
//!
//! All reads are bounded, lossy‑safe, and panic‑free. Serialized metadata
//! exposes basenames only — never full paths.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default max bytes per individual evidence file.
pub const DEFAULT_MAX_FILE_BYTES: u64 = 512 * 1024; // 512 KiB

/// Default max aggregate bytes across all evidence sources.
pub const DEFAULT_MAX_AGGREGATE_BYTES: u64 = 2 * 1024 * 1024; // 2 MiB

/// Bytes to keep from the head (start) of an oversized file.
pub const DEFAULT_HEAD_BYTES: u64 = 4 * 1024; // 4 KiB

/// Bytes to keep from the tail (end) of an oversized file.
pub const DEFAULT_TAIL_BYTES: u64 = 8 * 1024; // 8 KiB

/// Safe text extensions allowed for explicit evidence files.
pub const SAFE_TEXT_EXTENSIONS: &[&str] = &["txt", "log", "json", "md"];

/// Stale threshold — a file older than this (in days) is marked stale.
pub const STALE_THRESHOLD_DAYS: u64 = 30;

/// Contemporaneous window for supplementary file selection (seconds).
pub const CONTEMPORANEOUS_WINDOW_SECS: u64 = 3600; // 1 hour
const PRIMARY_RECENCY_SLOP_SECS: u64 = 10;
const LAUNCH_TIME_SLOP_SECS: u64 = 2;

// ---------------------------------------------------------------------------
// EvidenceSourceKind
// ---------------------------------------------------------------------------

/// How this evidence source was discovered or added.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EvidenceSourceKind {
    CrashReport,
    LatestLog,
    DebugLog,
    JvmFatalErrorLog,
    UserAdded,
    UserPasted,
}

// ---------------------------------------------------------------------------
// Evidence source metadata — basename only, never full path
// ---------------------------------------------------------------------------

/// Serializable metadata for a single evidence file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceSourceMeta {
    /// Basename only (e.g. `crash-2024-01-15_10.30.00-client.txt`).
    pub basename: String,
    pub kind: EvidenceSourceKind,
    pub size_bytes: u64,
    pub truncated: bool,
    pub stale: bool,
    pub supplementary: bool,
    pub modified_at: Option<String>,
    pub line_count: usize,
}

// ---------------------------------------------------------------------------
// EvidenceSource — metadata + content
// ---------------------------------------------------------------------------

/// A single evidence source with text content and metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceSource {
    pub meta: EvidenceSourceMeta,
    pub text: String,
}

// ---------------------------------------------------------------------------
// FailureCategory
// ---------------------------------------------------------------------------

/// High-level failure category inferred from the evidence set.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum FailureCategory {
    CrashReport,
    Oom,
    JvmFatal,
    NoEvidence,
}

// ---------------------------------------------------------------------------
// EvidenceConfig
// ---------------------------------------------------------------------------

/// Configuration for evidence collection bounds.
#[derive(Debug, Clone)]
pub struct EvidenceConfig {
    pub max_file_bytes: u64,
    pub max_aggregate_bytes: u64,
    pub head_bytes: u64,
    pub tail_bytes: u64,
}

impl Default for EvidenceConfig {
    fn default() -> Self {
        Self {
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            max_aggregate_bytes: DEFAULT_MAX_AGGREGATE_BYTES,
            head_bytes: DEFAULT_HEAD_BYTES,
            tail_bytes: DEFAULT_TAIL_BYTES,
        }
    }
}

// ---------------------------------------------------------------------------
// CollectedEvidence — the coherent set
// ---------------------------------------------------------------------------

/// The coherent evidence set discovered for one instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectedEvidence {
    pub sources: Vec<EvidenceSource>,
    /// Index into `sources` of the primary evidence file.
    pub primary_index: usize,
    pub aggregate_bytes: u64,
    pub any_truncated: bool,
    pub any_stale: bool,
    pub failure_category: FailureCategory,
}

// ---------------------------------------------------------------------------
// CrashEvidenceService
// ---------------------------------------------------------------------------

/// Collects local crash evidence from an instance directory.
///
/// Pure filesystem operations — no database, no network, no panics.
pub struct CrashEvidenceService {
    config: EvidenceConfig,
}

impl Default for CrashEvidenceService {
    fn default() -> Self {
        Self::new()
    }
}

impl CrashEvidenceService {
    pub fn new() -> Self {
        Self {
            config: EvidenceConfig::default(),
        }
    }

    pub fn with_config(config: EvidenceConfig) -> Self {
        Self { config }
    }

    /// Read one diagnostic text file through the same extension, size, and
    /// cleanup policy used by automatic evidence collection.
    pub fn read_bounded_text(&self, path: &Path) -> Option<String> {
        self.read_evidence_file(path, EvidenceSourceKind::UserAdded)
            .map(|source| source.text)
    }

    // -----------------------------------------------------------------------
    // collect — main entry point
    // -----------------------------------------------------------------------

    /// Collect a coherent evidence set from an instance directory.
    ///
    /// `instance_dir` is the root of a Minecraft instance (contains
    /// `crash-reports/`, `logs/`, etc.).
    ///
    /// `explicit_files` are user-provided absolute paths that have already been
    /// validated for safe text extensions (caller is responsible for security).
    pub fn collect(&self, instance_dir: &Path, explicit_files: &[PathBuf]) -> CollectedEvidence {
        self.collect_since(instance_dir, explicit_files, None)
    }

    /// Collect evidence, excluding automatic sources that predate the current
    /// launch. Explicit user-selected files are never filtered by this anchor.
    pub fn collect_since(
        &self,
        instance_dir: &Path,
        explicit_files: &[PathBuf],
        launch_started_at: Option<SystemTime>,
    ) -> CollectedEvidence {
        let crash_reports_dir = instance_dir.join("crash-reports");
        let logs_dir = instance_dir.join("logs");

        // 1. Discover automatic sources and select the newest useful source as
        // the primary. Older automatic files are included only when they are
        // plausibly from the same launch window.
        struct Candidate {
            path: PathBuf,
            kind: EvidenceSourceKind,
            supplementary: bool,
            modified_at: SystemTime,
        }

        let mut automatic: Vec<Candidate> = Vec::new();
        if let Some((path, modified_at)) = find_newest_file(&crash_reports_dir, "txt") {
            automatic.push(Candidate {
                path,
                kind: EvidenceSourceKind::CrashReport,
                supplementary: false,
                modified_at,
            });
        }

        let latest_log = logs_dir.join("latest.log");
        if let Some(modified_at) = file_modified_at(&latest_log) {
            automatic.push(Candidate {
                path: latest_log,
                kind: EvidenceSourceKind::LatestLog,
                supplementary: false,
                modified_at,
            });
        }

        let debug_log = logs_dir.join("debug.log");
        if let Some(modified_at) = file_modified_at(&debug_log) {
            automatic.push(Candidate {
                path: debug_log,
                kind: EvidenceSourceKind::DebugLog,
                supplementary: false,
                modified_at,
            });
        }

        if let Some((hs_path, modified_at)) = find_hs_err_file(instance_dir) {
            automatic.push(Candidate {
                path: hs_path,
                kind: EvidenceSourceKind::JvmFatalErrorLog,
                supplementary: false,
                modified_at,
            });
        }

        let newest_time = automatic
            .iter()
            .map(|candidate| candidate.modified_at)
            .max();
        automatic.retain(|candidate| {
            let belongs_to_launch = launch_started_at.is_none_or(|started| {
                candidate
                    .modified_at
                    .checked_add(Duration::from_secs(LAUNCH_TIME_SLOP_SECS))
                    .is_some_and(|modified| modified >= started)
            });
            belongs_to_launch
                && newest_time.is_none_or(|newest| {
                    absolute_duration(newest, candidate.modified_at)
                        <= Duration::from_secs(CONTEMPORANEOUS_WINDOW_SECS)
                })
        });
        automatic.sort_by(|left, right| {
            let left_current = newest_time.is_some_and(|newest| {
                absolute_duration(newest, left.modified_at)
                    <= Duration::from_secs(PRIMARY_RECENCY_SLOP_SECS)
            });
            let right_current = newest_time.is_some_and(|newest| {
                absolute_duration(newest, right.modified_at)
                    <= Duration::from_secs(PRIMARY_RECENCY_SLOP_SECS)
            });
            right_current
                .cmp(&left_current)
                .then_with(|| {
                    if left_current && right_current {
                        evidence_priority(&left.kind).cmp(&evidence_priority(&right.kind))
                    } else {
                        right.modified_at.cmp(&left.modified_at)
                    }
                })
                .then_with(|| right.modified_at.cmp(&left.modified_at))
        });
        let mut candidates: Vec<Candidate> = automatic
            .into_iter()
            .enumerate()
            .map(|(index, mut candidate)| {
                candidate.supplementary = index != 0;
                candidate
            })
            .collect();

        // Explicit user-added files
        for ef in explicit_files {
            let canon = match std::fs::canonicalize(ef) {
                Ok(c) => c,
                Err(_) => continue,
            };
            if !canon.is_file() {
                continue;
            }
            if !Self::is_safe_text_path(&canon) {
                continue;
            }
            if candidates.iter().any(|c| c.path == canon) {
                continue;
            }
            candidates.push(Candidate {
                path: canon,
                kind: EvidenceSourceKind::UserAdded,
                supplementary: !candidates.is_empty(),
                modified_at: file_modified_at(ef).unwrap_or(SystemTime::UNIX_EPOCH),
            });
        }

        // 3. Read each candidate with bounds
        let mut sources: Vec<EvidenceSource> = Vec::new();
        let mut primary_index: usize = 0;
        let mut aggregate: u64 = 0;
        let mut any_truncated = false;
        let mut any_stale = false;

        for candidate in &candidates {
            if aggregate >= self.config.max_aggregate_bytes {
                break;
            }

            if let Some(mut source) =
                self.read_evidence_file(&candidate.path, candidate.kind.clone())
            {
                let source_size = source.text.len() as u64;
                if aggregate.saturating_add(source_size) > self.config.max_aggregate_bytes {
                    if sources.is_empty() {
                        // Primary source must always fit; truncate it to the aggregate cap.
                        let cap = self
                            .config
                            .max_aggregate_bytes
                            .min(self.config.max_file_bytes);
                        let tmp_config = EvidenceConfig {
                            max_file_bytes: cap,
                            ..self.config
                        };
                        let tmp_svc = CrashEvidenceService { config: tmp_config };
                        let (t, _) = tmp_svc.truncate_text(&source.text);
                        source.text = t;
                        source.meta.truncated = true;
                    } else {
                        break;
                    }
                }
                source.meta.stale = Self::is_stale_file(&candidate.path);
                source.meta.supplementary = candidate.supplementary;
                any_stale = any_stale || source.meta.stale;
                any_truncated = any_truncated || source.meta.truncated;
                aggregate = aggregate.saturating_add(source.text.len() as u64);
                if !candidate.supplementary {
                    primary_index = sources.len();
                }
                sources.push(source);
            }
        }

        let failure_category = Self::categorize_failure(&sources);

        CollectedEvidence {
            sources,
            primary_index,
            aggregate_bytes: aggregate,
            any_truncated,
            any_stale,
            failure_category,
        }
    }

    // -----------------------------------------------------------------------
    // read_evidence_file — safe single-file read
    // -----------------------------------------------------------------------

    /// Read a single evidence file with bounds, truncation, and text cleanup.
    fn read_evidence_file(&self, path: &Path, kind: EvidenceSourceKind) -> Option<EvidenceSource> {
        if !path.is_file() {
            return None;
        }
        if !Self::is_safe_text_path(path) {
            return None;
        }

        let meta = std::fs::metadata(path).ok()?;
        let file_size = meta.len();
        let modified_at = meta.modified().ok();
        let mtime_str = modified_at.map(system_time_to_rfc3339);

        let raw = read_bounded(path, self.config.max_file_bytes, self.config.tail_bytes);
        let cleaned = Self::clean_text(&raw);
        let line_count = cleaned.lines().count();

        let (text, truncated) = self.truncate_text(&cleaned);

        let basename = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        Some(EvidenceSource {
            meta: EvidenceSourceMeta {
                basename,
                kind,
                size_bytes: file_size,
                truncated,
                stale: false, // filled in by caller
                supplementary: false,
                modified_at: mtime_str,
                line_count,
            },
            text,
        })
    }

    // -----------------------------------------------------------------------
    // Text cleaning
    // -----------------------------------------------------------------------

    /// Decode lossy UTF-8, strip NUL bytes and control characters.
    ///
    /// Keeps `\n`, `\r`, and `\t`. Replaces all other control characters and
    /// NUL bytes with spaces. Uses `U+FFFD` replacement for invalid UTF-8.
    pub fn clean_text(raw: &[u8]) -> String {
        let lossy = String::from_utf8_lossy(raw);
        let mut out = String::with_capacity(lossy.len());
        for c in lossy.chars() {
            match c {
                '\x00' => out.push(' '),
                '\n' | '\r' | '\t' => out.push(c),
                c if c.is_control() => out.push(' '),
                c => out.push(c),
            }
        }
        out
    }

    // -----------------------------------------------------------------------
    // Text truncation — head + tail with marker
    // -----------------------------------------------------------------------

    /// Truncate text to head + tail with a `--- [TRUNCATED] ---` separator.
    ///
    /// Avoids splitting multi-byte UTF-8 characters at truncation boundaries.
    /// Returns `(truncated_text, was_truncated)`.
    fn truncate_text(&self, text: &str) -> (String, bool) {
        let len = text.len();
        if len as u64 <= self.config.max_file_bytes {
            return (text.to_string(), false);
        }

        let head_end = (self.config.head_bytes as usize).min(len);
        let head_end = if text.is_char_boundary(head_end) {
            head_end
        } else {
            (0..=head_end)
                .rev()
                .find(|&i| text.is_char_boundary(i))
                .unwrap_or(0)
        };

        let tail_start = len.saturating_sub(self.config.tail_bytes as usize);
        let tail_start = if text.is_char_boundary(tail_start) {
            tail_start
        } else {
            (tail_start..len)
                .find(|&i| text.is_char_boundary(i))
                .unwrap_or(len)
        };

        let head = &text[..head_end];
        let tail = &text[tail_start..];
        let truncated = format!("{}\n--- [TRUNCATED] ---\n{}", head, tail);
        (truncated, true)
    }

    // -----------------------------------------------------------------------
    // Safe path check
    // -----------------------------------------------------------------------

    /// Check that a file path has an allowed diagnostic text extension.
    pub fn is_safe_text_path(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| {
                SAFE_TEXT_EXTENSIONS
                    .iter()
                    .any(|safe| e.eq_ignore_ascii_case(safe))
            })
            .unwrap_or(false)
    }

    // -----------------------------------------------------------------------
    // Staleness check
    // -----------------------------------------------------------------------

    /// Check whether a file is stale based on modification time.
    fn is_stale_file(path: &Path) -> bool {
        std::fs::metadata(path)
            .ok()
            .and_then(|m| m.modified().ok())
            .map(|mtime| {
                SystemTime::now()
                    .duration_since(mtime)
                    .map(|d| d > Duration::from_secs(STALE_THRESHOLD_DAYS * 86400))
                    .unwrap_or(false)
            })
            .unwrap_or(false)
    }

    // -----------------------------------------------------------------------
    // Failure categorization
    // -----------------------------------------------------------------------

    /// Infer a failure category from the collected evidence text.
    fn categorize_failure(sources: &[EvidenceSource]) -> FailureCategory {
        if sources.is_empty() {
            return FailureCategory::NoEvidence;
        }

        let all_text: String = sources.iter().map(|s| s.text.as_str()).collect();

        if all_text.contains("java.lang.OutOfMemoryError") || all_text.contains("OutOfMemoryError")
        {
            return FailureCategory::Oom;
        }

        if sources
            .iter()
            .any(|s| s.meta.kind == EvidenceSourceKind::JvmFatalErrorLog)
        {
            return FailureCategory::JvmFatal;
        }

        if all_text.contains("Exception") || all_text.contains("Mixin apply failed") {
            return FailureCategory::CrashReport;
        }

        for s in sources {
            if !s.text.trim().is_empty() {
                return FailureCategory::CrashReport;
            }
        }

        FailureCategory::NoEvidence
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Read up to `limit` bytes from a file, keeping `tail_bytes` from the end if
/// the file is larger than `limit + tail_bytes`. Returns `Vec<u8>` — never
/// panics; returns empty vec on error.
fn read_bounded(path: &Path, limit: u64, tail_bytes: u64) -> Vec<u8> {
    let file_size = match std::fs::metadata(path).map(|m| m.len()) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    if file_size <= limit + tail_bytes {
        return std::fs::read(path).unwrap_or_default();
    }

    let mut buf = Vec::with_capacity((limit + tail_bytes) as usize);
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    use std::io::Read;

    // Read head
    let head_size = limit.min(file_size);
    let mut head_buf = vec![0u8; head_size as usize];
    let n = file.read(&mut head_buf).unwrap_or(0);
    buf.extend_from_slice(&head_buf[..n]);

    // Truncation marker
    let marker = b"\n--- [TRUNCATED] ---\n";
    buf.extend_from_slice(marker);

    // Read tail
    use std::io::Seek;
    if file
        .seek(std::io::SeekFrom::End(-(tail_bytes as i64)))
        .is_ok()
    {
        let mut tail_buf = vec![0u8; tail_bytes as usize];
        let n = file.read(&mut tail_buf).unwrap_or(0);
        buf.extend_from_slice(&tail_buf[..n]);
    }

    buf
}

/// Find the newest file with a given extension in a directory.
fn find_newest_file(dir: &Path, extension: &str) -> Option<(PathBuf, SystemTime)> {
    let entries = std::fs::read_dir(dir).ok()?;
    entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case(extension))
                && e.file_type().map(|t| t.is_file()).unwrap_or(false)
        })
        .filter_map(|e| {
            let mtime = e.metadata().ok().and_then(|m| m.modified().ok())?;
            Some((e.path(), mtime))
        })
        .max_by_key(|(_, mtime)| *mtime)
}

/// Find the newest `hs_err_pid*.log` file in a directory.
fn find_hs_err_file(dir: &Path) -> Option<(PathBuf, SystemTime)> {
    let entries = std::fs::read_dir(dir).ok()?;
    entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name();
            let name_str = name.to_string_lossy();
            name_str.starts_with("hs_err_pid")
                && name_str.ends_with(".log")
                && e.file_type().map(|t| t.is_file()).unwrap_or(false)
        })
        .filter_map(|e| {
            let mtime = e.metadata().ok().and_then(|m| m.modified().ok())?;
            Some((e.path(), mtime))
        })
        .max_by_key(|(_, mtime)| *mtime)
}

fn file_modified_at(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

fn absolute_duration(left: SystemTime, right: SystemTime) -> Duration {
    left.duration_since(right)
        .or_else(|_| right.duration_since(left))
        .unwrap_or_default()
}

fn evidence_priority(kind: &EvidenceSourceKind) -> u8 {
    match kind {
        EvidenceSourceKind::CrashReport => 0,
        EvidenceSourceKind::JvmFatalErrorLog => 1,
        EvidenceSourceKind::LatestLog => 2,
        EvidenceSourceKind::DebugLog => 3,
        EvidenceSourceKind::UserAdded => 4,
        EvidenceSourceKind::UserPasted => 5,
    }
}

fn system_time_to_rfc3339(t: SystemTime) -> String {
    let dt: chrono::DateTime<chrono::Utc> = t.into();
    dt.to_rfc3339()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    struct TestFixture {
        _dir: tempfile::TempDir,
        path: PathBuf,
    }

    impl TestFixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("tempdir");
            let path = dir.path().to_path_buf();
            Self { _dir: dir, path }
        }

        fn crash_reports_dir(&self) -> PathBuf {
            let d = self.path.join("crash-reports");
            let _ = std::fs::create_dir_all(&d);
            d
        }

        fn logs_dir(&self) -> PathBuf {
            let d = self.path.join("logs");
            let _ = std::fs::create_dir_all(&d);
            d
        }

        fn write_file(&self, path: &Path, content: &str) {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let mut f = std::fs::File::create(path).unwrap();
            f.write_all(content.as_bytes()).unwrap();
        }

        fn write_bytes(&self, path: &Path, content: &[u8]) {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let mut f = std::fs::File::create(path).unwrap();
            f.write_all(content).unwrap();
        }
    }

    // ------------------------------------------------------------------
    // No evidence
    // ------------------------------------------------------------------

    #[test]
    fn test_collect_no_evidence_returns_empty() {
        let fix = TestFixture::new();
        let svc = CrashEvidenceService::new();
        let result = svc.collect(&fix.path, &[]);
        assert!(result.sources.is_empty());
        assert_eq!(result.failure_category, FailureCategory::NoEvidence);
        assert_eq!(result.aggregate_bytes, 0);
    }

    #[test]
    fn test_launch_anchor_excludes_old_automatic_evidence_but_not_explicit_files() {
        let fixture = TestFixture::new();
        let crash_dir = fixture.crash_reports_dir();
        let old_report = crash_dir.join("crash-old.txt");
        fixture.write_file(&old_report, "old automatic crash");
        let future_launch = SystemTime::now() + Duration::from_secs(30);
        let service = CrashEvidenceService::new();

        let automatic = service.collect_since(&fixture.path, &[], Some(future_launch));
        assert!(automatic.sources.is_empty());

        let explicit = service.collect_since(
            &fixture.path,
            std::slice::from_ref(&old_report),
            Some(future_launch),
        );
        assert_eq!(explicit.sources.len(), 1);
        assert_eq!(explicit.sources[0].meta.kind, EvidenceSourceKind::UserAdded);
    }

    // ------------------------------------------------------------------
    // Crash report only
    // ------------------------------------------------------------------

    #[test]
    fn test_collect_crash_report_only() {
        let fix = TestFixture::new();
        let text = "Exception in thread \"main\" java.lang.RuntimeException: Test";
        fix.write_file(&fix.crash_reports_dir().join("crash-2024-01-15.txt"), text);

        let svc = CrashEvidenceService::new();
        let result = svc.collect(&fix.path, &[]);

        assert_eq!(result.sources.len(), 1);
        let src = &result.sources[0];
        assert_eq!(src.meta.kind, EvidenceSourceKind::CrashReport);
        assert!(!src.meta.supplementary);
        assert_eq!(src.meta.line_count, 1);
        assert_eq!(result.primary_index, 0);
        assert_eq!(result.failure_category, FailureCategory::CrashReport);
    }

    // ------------------------------------------------------------------
    // Crash report with supplementary logs
    // ------------------------------------------------------------------

    #[test]
    fn test_collect_crash_report_with_supplementary_logs() {
        let fix = TestFixture::new();
        fix.write_file(
            &fix.crash_reports_dir().join("crash.txt"),
            "Exception: test",
        );
        fix.write_file(
            &fix.logs_dir().join("latest.log"),
            "[INFO] Game started\n[ERROR] Something broke",
        );
        fix.write_file(
            &fix.logs_dir().join("debug.log"),
            "[DEBUG] Loader initialized",
        );

        let svc = CrashEvidenceService::new();
        let result = svc.collect(&fix.path, &[]);

        assert_eq!(result.sources.len(), 3);
        assert_eq!(result.sources[0].meta.kind, EvidenceSourceKind::CrashReport);
        assert!(!result.sources[0].meta.supplementary);

        assert_eq!(result.sources[1].meta.kind, EvidenceSourceKind::LatestLog);
        assert!(result.sources[1].meta.supplementary);

        assert_eq!(result.sources[2].meta.kind, EvidenceSourceKind::DebugLog);
        assert!(result.sources[2].meta.supplementary);
    }

    // ------------------------------------------------------------------
    // OOM in latest.log, no crash report
    // ------------------------------------------------------------------

    #[test]
    fn test_collect_oom_in_latest_log_no_crash_report() {
        let fix = TestFixture::new();
        fix.write_file(
            &fix.logs_dir().join("latest.log"),
            "[INFO] Loading\n[ERROR] java.lang.OutOfMemoryError: Java heap space",
        );

        let svc = CrashEvidenceService::new();
        let result = svc.collect(&fix.path, &[]);

        assert_eq!(result.failure_category, FailureCategory::Oom);
    }

    // ------------------------------------------------------------------
    // hs_err_pid fatal JVM log
    // ------------------------------------------------------------------

    #[test]
    fn test_collect_hs_err_fatal() {
        let fix = TestFixture::new();
        fix.write_file(
            &fix.path.join("hs_err_pid12345.log"),
            "# A fatal error has been detected by the Java Runtime Environment",
        );

        let svc = CrashEvidenceService::new();
        let result = svc.collect(&fix.path, &[]);

        assert_eq!(result.failure_category, FailureCategory::JvmFatal);
        assert_eq!(
            result.sources[0].meta.kind,
            EvidenceSourceKind::JvmFatalErrorLog
        );
    }

    // ------------------------------------------------------------------
    // Explicit user-added files
    // ------------------------------------------------------------------

    #[test]
    fn test_collect_explicit_files() {
        let fix = TestFixture::new();
        let extra = fix.path.join("extra_log.txt");
        fix.write_file(&extra, "additional evidence");

        let svc = CrashEvidenceService::new();
        let result = svc.collect(&fix.path, &[extra]);

        assert_eq!(result.sources.len(), 1);
        assert_eq!(result.sources[0].meta.kind, EvidenceSourceKind::UserAdded);
    }

    #[test]
    fn test_collect_explicit_file_outside_instance() {
        let fix = TestFixture::new();
        let outside = fix.path.join("outside.txt");
        fix.write_file(&outside, "outside evidence");

        let svc = CrashEvidenceService::new();
        let result = svc.collect(&fix.path, &[outside]);

        assert_eq!(result.sources.len(), 1);
        assert_eq!(result.sources[0].meta.kind, EvidenceSourceKind::UserAdded);
    }

    // ------------------------------------------------------------------
    // Unsafe extension rejection
    // ------------------------------------------------------------------

    #[test]
    fn test_collect_rejects_unsafe_extension() {
        let fix = TestFixture::new();
        let unsafe_file = fix.path.join("evil.exe");
        fix.write_file(&unsafe_file, "binary content");

        let svc = CrashEvidenceService::new();
        let result = svc.collect(&fix.path, &[unsafe_file]);
        assert!(result.sources.is_empty());
    }

    #[test]
    fn test_collect_rejects_exe_in_crash_reports() {
        let fix = TestFixture::new();
        fix.write_bytes(
            &fix.crash_reports_dir().join("crash.exe"),
            b"not a text file",
        );

        let svc = CrashEvidenceService::new();
        let result = svc.collect(&fix.path, &[]);
        assert!(result.sources.is_empty());
    }

    // ------------------------------------------------------------------
    // Clean text — lossy UTF-8 and control chars
    // ------------------------------------------------------------------

    #[test]
    fn test_clean_text_lossy_utf8() {
        let raw = b"hello \xFF world \x00 null";
        let cleaned = CrashEvidenceService::clean_text(raw);
        assert!(cleaned.contains('\u{FFFD}'));
        assert!(!cleaned.contains('\x00'));
    }

    #[test]
    fn test_clean_text_controls_stripped() {
        let raw = b"line1\nline2\x01\x02\r\tline3";
        let cleaned = CrashEvidenceService::clean_text(raw);
        assert!(cleaned.contains("line1\n"));
        assert!(!cleaned.contains('\x01'));
        assert!(!cleaned.contains('\x02'));
    }

    #[test]
    fn test_clean_text_preserves_newlines() {
        let raw = b"line1\nline2\nline3\n";
        let cleaned = CrashEvidenceService::clean_text(raw);
        assert_eq!(cleaned, "line1\nline2\nline3\n");
    }

    #[test]
    fn test_clean_text_empty() {
        let cleaned = CrashEvidenceService::clean_text(b"");
        assert_eq!(cleaned, "");
    }

    // ------------------------------------------------------------------
    // Truncation
    // ------------------------------------------------------------------

    #[test]
    fn test_truncation_head_tail() {
        let config = EvidenceConfig {
            max_file_bytes: 50,
            head_bytes: 20,
            tail_bytes: 20,
            ..EvidenceConfig::default()
        };
        let svc = CrashEvidenceService::with_config(config);
        let long_text = "A".repeat(100);
        let (truncated, was_truncated) = svc.truncate_text(&long_text);
        assert!(was_truncated);
        assert!(truncated.starts_with("AAAAAAAAAAAAAAAAAAAA"));
        assert!(truncated.ends_with("AAAAAAAAAAAAAAAAAAAA"));
        assert!(truncated.contains("TRUNCATED"));
    }

    #[test]
    fn test_truncation_no_truncate_small() {
        let config = EvidenceConfig {
            max_file_bytes: 1000,
            ..EvidenceConfig::default()
        };
        let svc = CrashEvidenceService::with_config(config);
        let short = "Hello, world!";
        let (_, was) = svc.truncate_text(short);
        assert!(!was);
    }

    #[test]
    fn test_truncation_utf8_boundary_safe() {
        let config = EvidenceConfig {
            max_file_bytes: 5,
            head_bytes: 3,
            tail_bytes: 2,
            ..EvidenceConfig::default()
        };
        let svc = CrashEvidenceService::with_config(config);
        // Multi-byte UTF-8: é is 2 bytes
        let text = "abédefghij";
        let (truncated, was) = svc.truncate_text(text);
        assert!(was);
        assert!(truncated.starts_with("ab"));
        assert!(truncated.ends_with("ij"));
        assert!(truncated.contains("TRUNCATED"));
    }

    // ------------------------------------------------------------------
    // Binary/invalid input — no panic
    // ------------------------------------------------------------------

    #[test]
    fn test_collect_no_panic_on_binary_input() {
        let fix = TestFixture::new();
        fix.write_bytes(
            &fix.crash_reports_dir().join("crash.txt"),
            &[0x00, 0xFF, 0xFE, 0x01, 0x00],
        );

        let svc = CrashEvidenceService::new();
        let result = svc.collect(&fix.path, &[]);
        assert!(!result.sources.is_empty());
        assert!(!result.sources[0].text.contains('\x00'));
    }

    // ------------------------------------------------------------------
    // Aggregate bound
    // ------------------------------------------------------------------

    #[test]
    fn test_collect_aggregate_bound() {
        let fix = TestFixture::new();
        fix.write_file(&fix.crash_reports_dir().join("crash.txt"), &"A".repeat(100));
        fix.write_file(&fix.logs_dir().join("latest.log"), &"B".repeat(100));

        let limit: u64 = 50;
        let svc = CrashEvidenceService::with_config(EvidenceConfig {
            max_aggregate_bytes: limit,
            max_file_bytes: 100,
            ..EvidenceConfig::default()
        });
        let result = svc.collect(&fix.path, &[]);
        assert_eq!(
            result.sources.len(),
            1,
            "expected 1 source within {limit} byte cap"
        );
        assert_eq!(result.sources[0].meta.kind, EvidenceSourceKind::CrashReport);
    }

    // ------------------------------------------------------------------
    // Basename only — no full path leakage
    // ------------------------------------------------------------------

    #[test]
    fn test_basename_no_full_path() {
        let fix = TestFixture::new();
        fix.write_file(
            &fix.crash_reports_dir().join("crash-2024-01-15.txt"),
            "crash",
        );

        let svc = CrashEvidenceService::new();
        let result = svc.collect(&fix.path, &[]);

        let json = serde_json::to_string(&result.sources[0].meta).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["basename"], "crash-2024-01-15.txt");
        // Verify no path components
        assert!(
            !json.contains("crash-reports"),
            "must not contain directory: {json}"
        );
    }

    // ------------------------------------------------------------------
    // Fabric missing dependency crash
    // ------------------------------------------------------------------

    #[test]
    fn test_collect_fabric_missing_dependency_crash() {
        let fix = TestFixture::new();
        let crash = r#"---- Minecraft Crash Report ----
Description: Initializing game

java.lang.RuntimeException: Could not execute entrypoint stage 'main' due to missing required dependency: 'fabric-api'!
	at net.fabricmc.loader.impl.ModResolutionImpl.resolve(ModResolutionImpl.java:123)
	at net.minecraft.client.Minecraft.main(Minecraft.java:800)
"#;
        fix.write_file(&fix.crash_reports_dir().join("crash.txt"), crash);

        let svc = CrashEvidenceService::new();
        let result = svc.collect(&fix.path, &[]);
        assert_eq!(result.sources.len(), 1);
        assert!(result.sources[0].text.contains("fabric-api"));
    }

    // ------------------------------------------------------------------
    // Mixin crash
    // ------------------------------------------------------------------

    #[test]
    fn test_collect_mixin_crash() {
        let fix = TestFixture::new();
        let crash = r#"Mixin apply failed mixins.mod.json:core.mixins.json:CoreMixin -> net.minecraft.class_123
java.lang.RuntimeException: @Overwrite sentinel mismatch
	at org.spongepowered.asm.mixin.injection.callback.CallbackInjector.inject(CallbackInjector.java:123)
"#;
        fix.write_file(&fix.crash_reports_dir().join("crash.txt"), crash);

        let svc = CrashEvidenceService::new();
        let result = svc.collect(&fix.path, &[]);
        assert_eq!(result.failure_category, FailureCategory::CrashReport);
    }

    // ------------------------------------------------------------------
    // Serialization round-trip
    // ------------------------------------------------------------------

    #[test]
    fn test_serialize_evidence_source_roundtrip() {
        let source = EvidenceSource {
            meta: EvidenceSourceMeta {
                basename: "crash.txt".into(),
                kind: EvidenceSourceKind::CrashReport,
                size_bytes: 100,
                truncated: false,
                stale: false,
                supplementary: false,
                modified_at: Some("2024-01-15T10:30:00Z".into()),
                line_count: 5,
            },
            text: "line1\nline2\nline3\n".into(),
        };
        let json = serde_json::to_string(&source).unwrap();
        let parsed: EvidenceSource = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.meta.basename, "crash.txt");
        assert_eq!(parsed.text, "line1\nline2\nline3\n");
    }

    #[test]
    fn test_serialize_collected_evidence_roundtrip() {
        let evidence = CollectedEvidence {
            sources: vec![],
            primary_index: 0,
            aggregate_bytes: 0,
            any_truncated: false,
            any_stale: false,
            failure_category: FailureCategory::NoEvidence,
        };
        let json = serde_json::to_string(&evidence).unwrap();
        let parsed: CollectedEvidence = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.failure_category, FailureCategory::NoEvidence);
    }

    // ------------------------------------------------------------------
    // Oversized file
    // ------------------------------------------------------------------

    #[test]
    fn test_collect_oversized_file() {
        let fix = TestFixture::new();
        let content = "X".repeat(10_000);
        fix.write_file(&fix.crash_reports_dir().join("big-crash.txt"), &content);

        let config = EvidenceConfig {
            max_file_bytes: 100,
            head_bytes: 30,
            tail_bytes: 30,
            max_aggregate_bytes: 10_000,
        };
        let svc = CrashEvidenceService::with_config(config);
        let result = svc.collect(&fix.path, &[]);

        assert_eq!(result.sources.len(), 1);
        assert!(result.sources[0].meta.truncated);
        assert!(result.any_truncated);
    }

    // ------------------------------------------------------------------
    // Exe excluded from crash-reports
    // ------------------------------------------------------------------

    #[test]
    fn test_crash_reports_skips_non_txt() {
        let fix = TestFixture::new();
        fix.write_file(&fix.crash_reports_dir().join("crash.txt"), "real crash");
        fix.write_bytes(
            &fix.crash_reports_dir().join("crash.exe"),
            b"not a crash report",
        );

        let svc = CrashEvidenceService::new();
        let result = svc.collect(&fix.path, &[]);
        assert_eq!(result.sources.len(), 1);
        assert_eq!(result.sources[0].meta.basename, "crash.txt");
    }

    // ------------------------------------------------------------------
    // hs_err only picked if starts with hs_err_pid
    // ------------------------------------------------------------------

    #[test]
    fn test_hs_err_only_picks_correct_prefix() {
        let fix = TestFixture::new();
        fix.write_file(&fix.path.join("hs_err_pid999.log"), "JVM fatal error");
        fix.write_file(&fix.path.join("other.log"), "not a hs_err");

        let svc = CrashEvidenceService::new();
        let result = svc.collect(&fix.path, &[]);
        assert_eq!(result.sources.len(), 1);
        assert_eq!(
            result.sources[0].meta.kind,
            EvidenceSourceKind::JvmFatalErrorLog
        );
        assert_eq!(result.sources[0].meta.basename, "hs_err_pid999.log");
    }

    // ------------------------------------------------------------------
    // Multiple crash reports — discovers at least one txt
    // ------------------------------------------------------------------

    #[test]
    fn test_collect_finds_crash_reports_with_txt_extension() {
        let fix = TestFixture::new();
        let dir = fix.crash_reports_dir();

        fix.write_file(&dir.join("crash-old.txt"), "old crash");
        fix.write_file(&dir.join("crash-new.txt"), "new crash");

        let svc = CrashEvidenceService::new();
        let result = svc.collect(&fix.path, &[]);
        assert_eq!(result.sources.len(), 1);
        let name = &result.sources[0].meta.basename;
        assert!(
            name.starts_with("crash-"),
            "expected crash-* name, got {name}"
        );
        assert!(name.ends_with(".txt"), "expected .txt, got {name}");
    }
}
