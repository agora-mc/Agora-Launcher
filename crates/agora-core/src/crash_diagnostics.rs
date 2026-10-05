//! Crash diagnostics — embedded signature corpus with optional DB augmentation.
//!
//! Phase 3: crash-triage works with ZERO `registry.db` dependency. The curated
//! signature set is embedded at compile time via `include_str!` of the
//! `crash-signatures/*.json` files and loaded lazily on first access.
//!
//! `triage_with_db` optionally augments the embedded set with runtime-added
//! signatures from the registry (when `registry.db` is present and the
//! `crash_signatures` table exists), but never lets a missing or corrupt DB
//! break the triage path.

use crate::error::LauncherResult;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// Per §2.4.1, crash signature regex patterns longer than this are rejected.
pub const MAX_REGEX_LEN: usize = 256;

// --- Embedded signature corpus (Phase 3 core) ---

const SIG_FABRIC_API: &str = include_str!("../../../crash-signatures/fabric-api-missing.json");
const SIG_MISSING_DEPENDENCY: &str =
    include_str!("../../../crash-signatures/fabric-missing-dependency.json");
const SIG_MIXIN_CONFLICT: &str = include_str!("../../../crash-signatures/mixin-conflict.json");
const SIG_MOD_RESOLUTION: &str = include_str!("../../../crash-signatures/mod-resolution.json");
const SIG_OUT_OF_MEMORY: &str = include_str!("../../../crash-signatures/out-of-memory.json");
const SIG_DUPLICATE_CLASSPATH: &str =
    include_str!("../../../crash-signatures/duplicate-classpath-libraries.json");

/// A curated crash signature loaded from the embedded `crash-signatures/*.json` files.
#[derive(Debug, Deserialize, Clone)]
struct CrashSignature {
    id: String,
    name: String,
    regex_pattern: String,
    solution_markdown: String,
    action_button: ActionButton,
}

/// An `action_button` entry from a crash-signature JSON file.
#[derive(Debug, Deserialize, Clone, Serialize)]
struct ActionButton {
    label: String,
    mod_id: Option<String>,
}

impl ActionButton {
    /// Serialize this action button into a JSON string suitable for the UI.
    fn to_json_string(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// Parsed crash-signature corpus, loaded once at first access.
static SIG_CORPUS: OnceLock<Vec<CrashSignature>> = OnceLock::new();

fn corpus() -> &'static [CrashSignature] {
    SIG_CORPUS.get_or_init(|| {
        [
            SIG_FABRIC_API,
            // Before the generic mod-resolution entry: a named missing mod is
            // a more useful answer than "the mod set could not be resolved".
            SIG_MISSING_DEPENDENCY,
            SIG_MIXIN_CONFLICT,
            SIG_MOD_RESOLUTION,
            SIG_OUT_OF_MEMORY,
            SIG_DUPLICATE_CLASSPATH,
        ]
        .into_iter()
        .filter_map(|raw| serde_json::from_str(raw).ok())
        .collect()
    })
}

// --- JarMetadata (used by modrinth install flow) ---

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct JarMetadata {
    pub java_packages: Vec<String>,
    pub mod_jar_id: Option<String>,
    pub depends_on: Vec<String>,
    pub optional_deps: Vec<String>,
    pub incompatible_deps: Vec<String>,
}

// --- Public types (preserved shapes) ---

/// Summary of a single crash report file on disk.
#[derive(Debug, Clone, Serialize)]
pub struct CrashReportInfo {
    pub filename: String,
    pub modified_at: String,
    pub size_bytes: u64,
}

/// Result of matching a crash log against the curated signature set.
#[derive(Debug, Clone, Serialize)]
pub struct CrashTriageResult {
    pub matched: bool,
    pub signature_name: Option<String>,
    pub solution_markdown: Option<String>,
    pub action_button_json: Option<String>,
}

impl CrashTriageResult {
    pub fn no_match() -> Self {
        Self {
            matched: false,
            signature_name: None,
            solution_markdown: None,
            action_button_json: None,
        }
    }

    fn from_signature(sig: &CrashSignature) -> Self {
        Self {
            matched: true,
            signature_name: Some(sig.name.clone()),
            solution_markdown: Some(sig.solution_markdown.clone()),
            action_button_json: Some(sig.action_button.to_json_string()),
        }
    }
}

// --- Pure triage (zero DB dependency) ---

/// Triage a crash log against the embedded signature corpus.
///
/// Returns the first signature whose `regex_pattern` matches the log text.
/// Patterns exceeding `MAX_REGEX_LEN` characters are skipped.
///
/// This function has **zero** `registry.db` dependency — it works on any
/// instance including Modrinth-only setups.
pub fn triage(log: &str) -> CrashTriageResult {
    for sig in corpus() {
        if sig.regex_pattern.chars().count() > MAX_REGEX_LEN {
            continue;
        }
        if let Ok(re) = regex::Regex::new(&sig.regex_pattern) {
            if re.is_match(log) {
                let mut result = CrashTriageResult::from_signature(sig);
                if sig.id == MISSING_DEPENDENCY_SIGNATURE_ID {
                    describe_missing_dependencies(&mut result, log);
                }
                return result;
            }
        }
    }
    CrashTriageResult::no_match()
}

const MISSING_DEPENDENCY_SIGNATURE_ID: &str = "fabric-missing-dependency";

/// One "mod A needs mod B, which is not installed" finding read from a Fabric
/// or Quilt loader failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingDependency {
    /// Loader id of the mod that is missing (for example `fabric-api`).
    pub missing_id: String,
    /// Human name for the missing mod when the loader printed one.
    pub missing_name: Option<String>,
    /// Version requirement as the loader phrased it ("any version",
    /// "version 0.43.1 or later").
    pub requirement: String,
    /// Display name and loader id of the installed mod that needs it.
    pub required_by_name: String,
    pub required_by_id: String,
}

/// Read the "Unmet dependency listing" of a Fabric/Quilt resolution failure:
/// `Mod 'Entity Culling' (entityculling) 1.11.2 requires any version of
/// fabric-api, which is missing!`. Each (dependent, missing) pair is reported
/// once, in log order.
pub fn missing_dependencies(log: &str) -> Vec<MissingDependency> {
    static LINE: OnceLock<regex::Regex> = OnceLock::new();
    let line = LINE.get_or_init(|| {
        regex::Regex::new(
            r"Mod '(?P<name>[^\r\n]+?)' \((?P<id>[^)\s]+)\) \S+ requires (?P<need>[^\r\n]+?), which is missing!",
        )
        .expect("missing-dependency line pattern is valid")
    });
    let mut found: Vec<MissingDependency> = Vec::new();
    for caps in line.captures_iter(log) {
        let need = &caps["need"];
        // "any version of fabric-api" / "version 0.43.1 or later of Fabric API (fabric-api)"
        let (requirement, target) = match need.split_once(" of ") {
            Some((requirement, target)) => (requirement.trim(), target.trim()),
            None => ("any version", need.trim()),
        };
        let (missing_name, missing_id) = match target.rsplit_once(" (") {
            Some((name, id)) if id.ends_with(')') => (
                Some(name.trim_matches('\'').trim().to_string()),
                id.trim_end_matches(')').trim().to_string(),
            ),
            _ => (None, target.trim_matches('\'').to_string()),
        };
        if missing_id.is_empty() {
            continue;
        }
        let item = MissingDependency {
            missing_id,
            missing_name: missing_name.filter(|name| !name.is_empty()),
            requirement: requirement.to_string(),
            required_by_name: caps["name"].to_string(),
            required_by_id: caps["id"].to_string(),
        };
        if !found.iter().any(|seen| {
            seen.missing_id == item.missing_id && seen.required_by_id == item.required_by_id
        }) {
            found.push(item);
        }
    }
    found
}

/// Keep text lifted from a log inside a single inline-code span.
fn markdown_code(text: &str) -> String {
    format!("`{}`", text.replace(['`', '\r', '\n'], ""))
}

/// Name the missing mod and the mods that need it in the answer, and point the
/// action at the missing mod when there is exactly one.
fn describe_missing_dependencies(result: &mut CrashTriageResult, log: &str) {
    let missing = missing_dependencies(log);
    if missing.is_empty() {
        return;
    }
    let mut ids: Vec<&str> = Vec::new();
    for item in &missing {
        if !ids.contains(&item.missing_id.as_str()) {
            ids.push(item.missing_id.as_str());
        }
    }
    let mut details = Vec::new();
    for id in &ids {
        let group: Vec<&MissingDependency> = missing
            .iter()
            .filter(|item| item.missing_id == *id)
            .collect();
        let label = match group[0].missing_name.as_deref() {
            Some(name) => format!("{} ({})", markdown_code(name), markdown_code(id)),
            None => markdown_code(id),
        };
        let needed_by = group
            .iter()
            .map(|item| {
                format!(
                    "{} ({}, {})",
                    markdown_code(&item.required_by_name),
                    markdown_code(&item.required_by_id),
                    item.requirement
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        details.push(format!("- **Missing:** {label}, needed by {needed_by}"));
    }
    if let Some(markdown) = result.solution_markdown.as_mut() {
        markdown.push_str("\n\n");
        markdown.push_str(&details.join("\n"));
    }
    if let [only] = ids.as_slice() {
        let name = missing
            .iter()
            .find_map(|item| item.missing_name.clone())
            .unwrap_or_else(|| (*only).to_string());
        result.action_button_json = Some(
            ActionButton {
                label: format!("Install {name}"),
                mod_id: Some((*only).to_string()),
            }
            .to_json_string(),
        );
    }
}

// --- Augmented triage (optional DB) ---

/// Triage with optional database augmentation.
///
/// First runs `triage()` against the embedded corpus. If `conn` is `Some`
/// and the `crash_signatures` table exists, also queries the database for
/// any signatures NOT already in the embedded set (by `id`) and runs those
/// too. Returns the first hit from either source.
///
/// If the table is missing or the query errors, the embedded result is
/// returned unchanged — the DB path never fails the triage.
pub fn triage_with_db(log: &str, conn: Option<&rusqlite::Connection>) -> CrashTriageResult {
    // 1. Check embedded corpus first.
    let embedded_result = triage(log);
    if embedded_result.matched {
        return embedded_result;
    }

    // 2. Optionally check DB-augmented signatures.
    if let Some(c) = conn {
        if let Some(db_sigs) = load_db_signatures(c) {
            let embedded_ids: std::collections::HashSet<&str> =
                corpus().iter().map(|s| s.id.as_str()).collect();

            for sig in db_sigs {
                if embedded_ids.contains(sig.id.as_str()) {
                    continue;
                }
                if sig.regex_pattern.chars().count() > MAX_REGEX_LEN {
                    continue;
                }
                if let Ok(re) = regex::Regex::new(&sig.regex_pattern) {
                    if re.is_match(log) {
                        return CrashTriageResult {
                            matched: true,
                            signature_name: Some(sig.name),
                            solution_markdown: sig.solution_markdown,
                            action_button_json: sig.action_button_json,
                        };
                    }
                }
            }
        }
    }

    embedded_result
}

/// A signature row loaded from the `crash_signatures` database table.
#[derive(Debug, Deserialize)]
struct CrashSignatureRow {
    id: String,
    name: String,
    regex_pattern: String,
    solution_markdown: Option<String>,
    action_button_json: Option<String>,
}

/// Load crash signatures from the `crash_signatures` table.
///
/// Returns `None` if the table doesn't exist or the query fails.
fn load_db_signatures(conn: &rusqlite::Connection) -> Option<Vec<CrashSignatureRow>> {
    // Check if the table exists.
    let mut stmt = conn
        .prepare(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='crash_signatures'",
        )
        .ok()?;
    let exists: i32 = stmt.query_row([], |row| row.get(0)).ok()?;
    if exists == 0 {
        return None;
    }
    drop(stmt);

    // Query all signatures.
    let mut stmt = conn
        .prepare(
            "SELECT id, name, regex_pattern, solution_markdown, action_button_json \
         FROM crash_signatures",
        )
        .ok()?;
    let rows = stmt
        .query_map([], |row| {
            Ok(CrashSignatureRow {
                id: row.get(0).unwrap_or_default(),
                name: row.get(1).unwrap_or_default(),
                regex_pattern: row.get(2).unwrap_or_default(),
                solution_markdown: row.get(3).ok(),
                action_button_json: row.get(4).ok(),
            })
        })
        .ok()?;

    let mut out = Vec::new();
    for sig in rows.flatten() {
        out.push(sig);
    }
    Some(out)
}

// --- Path-based helpers (for shim delegation) ---

/// List crash report files from a directory path, returning sorted
/// (newest first) `[CrashReportInfo]`. Returns an empty vec when the
/// directory does not exist or cannot be read.
pub fn list_reports_from_dir(dir: &std::path::Path) -> Vec<CrashReportInfo> {
    if !dir.exists() {
        return Vec::new();
    }
    let mut out: Vec<(CrashReportInfo, std::time::SystemTime)> = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    for entry in entries.flatten() {
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !meta.is_file() {
            continue;
        }
        let mtime = match entry.metadata().and_then(|m| m.modified()) {
            Ok(t) => t,
            Err(_) => continue,
        };
        out.push((
            CrashReportInfo {
                filename: entry.file_name().to_string_lossy().to_string(),
                modified_at: system_time_to_rfc3339(mtime),
                size_bytes: meta.len(),
            },
            mtime,
        ));
    }
    out.sort_by_key(|b| std::cmp::Reverse(b.1));
    out.into_iter().map(|(info, _)| info).collect()
}

/// Read the content of a specific crash report file at the given path.
pub fn read_crash_log_from_path(path: &std::path::Path) -> LauncherResult<String> {
    crate::crash_evidence::CrashEvidenceService::new()
        .read_bounded_text(path)
        .ok_or_else(|| crate::error::LauncherError::Generic {
            code: "ERR_CRASH_LOG_READ".to_string(),
            message: "Could not read the crash log file.".to_string(),
        })
}

/// Check whether a fresh crash report appeared after the given timestamp.
///
/// Reads the instance's `last_launched_at` (provided by the caller), lists
/// files in the crash-reports directory, and returns the newest file whose
/// mtime is strictly newer than `last_launched_at`.
pub fn check_for_crash_from_path(
    dir: &std::path::Path,
    last_launched_at: &str,
) -> LauncherResult<Option<CrashReportInfo>> {
    let last_launched = parse_rfc3339(last_launched_at);

    if !dir.exists() {
        return Ok(None);
    }

    let mut newest: Option<(CrashReportInfo, std::time::SystemTime)> = None;
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(None),
    };
    for entry in entries.flatten() {
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !meta.is_file() {
            continue;
        }
        let mtime = match meta.modified() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if let Some(ref last) = last_launched {
            if mtime <= *last {
                continue;
            }
        }
        let filename = entry.file_name().to_string_lossy().to_string();
        let info = CrashReportInfo {
            filename: filename.clone(),
            modified_at: system_time_to_rfc3339(mtime),
            size_bytes: meta.len(),
        };
        match &newest {
            Some((_, best_mtime)) if mtime <= *best_mtime => {}
            _ => newest = Some((info, mtime)),
        }
    }

    Ok(newest.map(|(info, _)| info))
}

// --- Regex helpers ---

/// Pure regex matching helper — compiles a pattern and checks if it matches
/// the given text. Returns `false` for invalid patterns or non-matches.
pub fn match_signature(pattern: &str, crash_text: &str) -> bool {
    if pattern.is_empty() {
        return false;
    }
    regex::Regex::new(pattern)
        .map(|re| re.is_match(crash_text))
        .unwrap_or(false)
}

/// Check whether a regex pattern exceeds the MAX_REGEX_LEN guard.
pub fn is_regex_too_long(pattern: &str) -> bool {
    pattern.chars().count() > MAX_REGEX_LEN
}

// --- Internal helpers ---

fn parse_rfc3339(ts: &str) -> Option<std::time::SystemTime> {
    chrono::DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|dt| std::time::SystemTime::from(dt.with_timezone(&chrono::Utc)))
}

fn system_time_to_rfc3339(t: std::time::SystemTime) -> String {
    let dt: chrono::DateTime<chrono::Utc> = t.into();
    dt.to_rfc3339()
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;

    // --- Regex matching ---

    /// test_regex_matches_known_crash: a real crash-signature regex from
    /// `crash-signatures/mixin-conflict.json` should match a matching snippet.
    #[test]
    fn test_regex_matches_known_crash() {
        let pattern = "Mixin apply failed";
        let crash_text =
            "[06:12:33] [Worker-3/FABRIC]: Mixin apply failed mixins.fabric.json:debug.mixins.json:DebugMixin -> org.example.Mod: java/lang/RuntimeException";
        assert!(match_signature(pattern, crash_text));
    }

    /// test_regex_no_match_unrelated: same regex against unrelated text.
    #[test]
    fn test_regex_no_match_unrelated() {
        let pattern = "Mixin apply failed";
        let unrelated = "Game loaded successfully with 42 mods active.";
        assert!(!match_signature(pattern, unrelated));
    }

    /// test_regex_no_match_empty: empty input should not panic and returns false.
    #[test]
    fn test_regex_no_match_empty() {
        let pattern = "java\\.lang\\.OutOfMemoryError";
        assert!(!match_signature(pattern, ""));
    }

    /// test_regex_no_match_malformed: garbage bytes should not panic.
    #[test]
    fn test_regex_no_match_malformed() {
        let pattern = "java\\.lang\\.OutOfMemoryError";
        let garbage = "x\x00y\x01z\x02garbage";
        assert!(!match_signature(pattern, garbage));
    }

    // --- Crash report discovery (path-based) ---

    /// test_list_crash_reports_finds_txt: temp dir with .txt files finds them.
    #[test]
    fn test_list_crash_reports_finds_txt() {
        let tmp =
            std::env::temp_dir().join(format!("agora_test_crash_reports_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("crash-1.txt"), "crash data 1").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::write(tmp.join("crash-2.txt"), "crash data 2").unwrap();

        let reports = list_reports_from_dir(&tmp);
        assert_eq!(reports.len(), 2);
        let names: Vec<&str> = reports.iter().map(|r| r.filename.as_str()).collect();
        assert!(names.contains(&"crash-2.txt"));
        assert!(names.contains(&"crash-1.txt"));

        std::fs::remove_dir_all(&tmp).ok();
    }

    /// test_list_crash_reports_empty_dir: empty directory returns empty vec.
    #[test]
    fn test_list_crash_reports_empty_dir() {
        let tmp =
            std::env::temp_dir().join(format!("agora_test_crash_empty_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();

        let reports = list_reports_from_dir(&tmp);
        assert!(reports.is_empty());

        std::fs::remove_dir_all(&tmp).ok();
    }

    /// test_list_crash_reports_nonexistent_dir: missing dir returns empty.
    #[test]
    fn test_list_crash_reports_nonexistent_dir() {
        let tmp = std::env::temp_dir().join(format!(
            "agora_test_crash_missing_{}_nonexistent",
            std::process::id()
        ));
        let reports = list_reports_from_dir(&tmp);
        assert!(reports.is_empty());
    }

    // --- MAX_REGEX_LEN guard ---

    /// test_max_regex_len_rejects_long: pattern >256 chars is rejected.
    #[test]
    fn test_max_regex_len_rejects_long() {
        let long_pattern = "a".repeat(257);
        assert!(is_regex_too_long(&long_pattern));
    }

    /// test_max_regex_len_accepts_short: normal pattern is fine.
    #[test]
    fn test_max_regex_len_accepts_short() {
        let short_pattern = "java\\.lang\\.OutOfMemoryError";
        assert!(!is_regex_too_long(short_pattern));
    }

    // --- Struct serialization ---

    /// test_crash_report_info_serializes: construct a CrashReportInfo, serialize,
    /// verify fields round-trip.
    #[test]
    fn test_crash_report_info_serializes() {
        let info = CrashReportInfo {
            filename: "crash-1.txt".to_string(),
            modified_at: "2024-01-15T10:30:00Z".to_string(),
            size_bytes: 4096,
        };
        let json = serde_json::to_string(&info).expect("should serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("should parse");
        assert_eq!(parsed["filename"], "crash-1.txt");
        assert_eq!(parsed["modified_at"], "2024-01-15T10:30:00Z");
        assert_eq!(parsed["size_bytes"], 4096);
    }

    // --- Phase 3: Embedded corpus triage (zero DB dependency) ---

    /// test_triage_mixin_conflict: embedded mixin-conflict signature matches.
    #[test]
    fn test_triage_mixin_conflict() {
        let log =
            "[06:12:33] [Worker-3/FABRIC]: Mixin apply failed mixins.fabric.json:debug.mixins.json";
        let result = triage(log);
        assert!(result.matched);
        assert_eq!(
            result.signature_name.as_deref(),
            Some("Mixin Injection Conflict")
        );
        assert!(result.solution_markdown.is_some());
        assert!(result.action_button_json.is_some());
    }

    /// test_triage_out_of_memory: embedded OOM signature matches.
    #[test]
    fn test_triage_out_of_memory() {
        let log = "Caused by: java.lang.OutOfMemoryError: Java heap space";
        let result = triage(log);
        assert!(result.matched);
        assert_eq!(result.signature_name.as_deref(), Some("Out of Memory"));
    }

    /// test_triage_no_match: unrelated log returns no match.
    #[test]
    fn test_triage_no_match() {
        let log = "Game loaded successfully with 42 mods active.";
        let result = triage(log);
        assert!(!result.matched);
        assert!(result.signature_name.is_none());
        assert!(result.solution_markdown.is_none());
        assert!(result.action_button_json.is_none());
    }

    /// test_triage_empty_log: empty string returns no match, never panics.
    #[test]
    fn test_triage_empty_log() {
        let result = triage("");
        assert!(!result.matched);
    }

    /// test_triage_with_db_none: triage_with_db(None) behaves identically to triage().
    #[test]
    fn test_triage_with_db_none() {
        let log = "Caused by: java.lang.OutOfMemoryError: Java heap space";
        let r1 = triage(log);
        let r2 = triage_with_db(log, None);
        assert_eq!(r1.matched, r2.matched);
        assert_eq!(r1.signature_name, r2.signature_name);
    }

    #[test]
    fn test_duplicate_asm_classpath_matches() {
        let log = "[stderr] java.lang.RuntimeException: Found duplicate ASM classes found on classpath: asm-9.3.jar, asm-9.10.1.jar";
        let r = triage("duplicate ASM classes found on classpath");
        assert!(r.matched, "{log}");
        assert_eq!(
            r.signature_name.as_deref(),
            Some("Duplicate Libraries on the Classpath")
        );
    }

    /// Fabric Loader 0.16+ output as captured from the game's stdout/stderr.
    const FABRIC_MISSING_API_LOG: &str = concat!(
        "[stderr] net.fabricmc.loader.impl.FormattedException: net.fabricmc.loader.impl.discovery.ModResolutionException: Mod resolution encountered an incompatible mod set!\n",
        "[stderr] A potential solution has been determined, this may resolve your problem:\n",
        "[stderr] \t - Install fabric-api, any version.\n",
        "[stderr] Unmet dependency listing:\n",
        "[stderr] \t - Mod 'Entity Culling' (entityculling) 1.11.2 requires any version of fabric-api, which is missing!\n",
        "[stderr] \t - Mod 'Xaero's Minimap' (xaerominimap) 26.5.0 requires version 0.43.1 or later of fabric-api, which is missing!\n",
        "[stderr] More details: For further details, see the log.\n",
    );

    #[test]
    fn test_triage_missing_fabric_api_names_the_missing_mod_and_who_needs_it() {
        let r = triage(FABRIC_MISSING_API_LOG);
        assert!(r.matched);
        assert_eq!(r.signature_name.as_deref(), Some("Missing Required Mod"));
        let markdown = r.solution_markdown.unwrap();
        assert!(markdown.contains("`fabric-api`"), "{markdown}");
        assert!(markdown.contains("`Entity Culling`"), "{markdown}");
        assert!(markdown.contains("`entityculling`"), "{markdown}");
        assert!(markdown.contains("`Xaero's Minimap`"), "{markdown}");
        assert!(markdown.contains("version 0.43.1 or later"), "{markdown}");
        let action: serde_json::Value =
            serde_json::from_str(&r.action_button_json.unwrap()).unwrap();
        assert_eq!(action["mod_id"], "fabric-api");
        assert_eq!(action["label"], "Install fabric-api");
    }

    #[test]
    fn test_missing_dependencies_reads_names_ids_and_versions() {
        let found = missing_dependencies(FABRIC_MISSING_API_LOG);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].missing_id, "fabric-api");
        assert_eq!(found[0].required_by_id, "entityculling");
        assert_eq!(found[0].required_by_name, "Entity Culling");
        assert_eq!(found[0].requirement, "any version");
        assert_eq!(found[1].required_by_id, "xaerominimap");
        assert_eq!(found[1].requirement, "version 0.43.1 or later");
    }

    #[test]
    fn test_missing_dependencies_handles_named_targets_and_log4j_wrapping() {
        let log = "<log4j:Message><![CDATA[Mod resolution encountered an incompatible mod set!\n\
\t - Mod 'Mod Menu' (modmenu) 18.0.2 requires version 0.130.0 or later of Fabric API (fabric-api), which is missing!\n\
\t - Mod 'Iris' (iris) 1.11.4 requires version 0.9.2 or later of 'Sodium' (sodium), which is missing!\n\
\t - Mod 'Mod Menu' (modmenu) 18.0.2 requires version 0.130.0 or later of Fabric API (fabric-api), which is missing!\n\
]]></log4j:Message>";
        let r = triage(log);
        assert_eq!(r.signature_name.as_deref(), Some("Missing Required Mod"));
        let found = missing_dependencies(log);
        assert_eq!(found.len(), 2, "duplicate lines collapse: {found:?}");
        assert_eq!(found[0].missing_id, "fabric-api");
        assert_eq!(found[0].missing_name.as_deref(), Some("Fabric API"));
        assert_eq!(found[1].missing_id, "sodium");
        assert_eq!(found[1].missing_name.as_deref(), Some("Sodium"));
        // Two different mods are missing, so no single install action is named.
        let action: serde_json::Value =
            serde_json::from_str(&r.action_button_json.unwrap()).unwrap();
        assert!(action["mod_id"].is_null());
    }

    #[test]
    fn test_missing_dependency_signature_wins_over_generic_resolution() {
        // Without a parsable listing the generic answer would still apply;
        // with one, the specific signature must come first.
        assert_eq!(
            triage("ModResolutionException: Some of your mods are incompatible with the game or each other!")
                .signature_name
                .as_deref(),
            Some("Mod Resolution Failure")
        );
        assert_eq!(
            triage(FABRIC_MISSING_API_LOG).signature_name.as_deref(),
            Some("Missing Required Mod")
        );
    }

    /// test_corpus_nonempty: the embedded corpus should have all 6 signatures.
    #[test]
    fn test_corpus_nonempty() {
        let c = corpus();
        assert_eq!(c.len(), 6);
        assert!(c.iter().any(|s| s.id == "duplicate-classpath-libraries"));
        let ids: Vec<&str> = c.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"fabric-api-missing"));
        assert!(ids.contains(&"fabric-missing-dependency"));
        assert!(ids.contains(&"mixin-conflict"));
        assert!(ids.contains(&"mod-resolution"));
        assert!(ids.contains(&"out-of-memory"));
    }
}
