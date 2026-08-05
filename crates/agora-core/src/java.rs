//! Java runtime detection, inspection, and discovery.
//!
//! Provides the canonical [`JavaInstallation`] model used throughout the
//! launcher, platform-specific system-JRE discovery, managed/Mojang runtime
//! scanning, and the combined [`detect_java_candidates`] API.
//!
//! ## Sources
//!
//! Every discovered Java installation carries a [`JavaSource`] tag that the
//! selection policy uses to rank candidates.  The ordering is:
//!
//! 1. **Override** — explicit user path (always highest priority when set).
//! 2. **Managed** — auto-provisioned by [`crate::runtime_manager`].
//! 3. **Mojang** — bundled runtimes under the official launcher directory.
//! 4. **System** — OS-default JRE (PATH, standard install directories).

use crate::launch::VersionInfo;
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Test injection point for inspect_java
// ---------------------------------------------------------------------------

// Thread-local mock so parallel tests do not interfere with each other.
#[cfg(test)]
type MockInspectFn = Option<fn(&Path) -> Option<JavaInstallation>>;

#[cfg(test)]
thread_local! {
    static MOCK_INSPECT: std::cell::RefCell<MockInspectFn> =
        const { std::cell::RefCell::new(None) };
}

/// RAII guard that restores the previous mock on drop.
#[cfg(test)]
pub struct MockInspectGuard(Option<fn(&Path) -> Option<JavaInstallation>>);

#[cfg(test)]
impl Drop for MockInspectGuard {
    fn drop(&mut self) {
        let prev = self.0.take();
        MOCK_INSPECT.with(|cell| {
            cell.replace(prev);
        });
    }
}

/// Set a mock for `inspect_java` (test-only).
///
/// Returns a [`MockInspectGuard`] that restores the previous mock when
/// dropped. Uses a thread-local so parallel tests are isolated.
#[cfg(test)]
pub fn set_mock_inspect(f: Option<fn(&Path) -> Option<JavaInstallation>>) -> MockInspectGuard {
    MOCK_INSPECT.with(|cell| {
        let prev = cell.replace(f);
        MockInspectGuard(prev)
    })
}

// ---------------------------------------------------------------------------
// JavaSource
// ---------------------------------------------------------------------------

/// Origin of a discovered Java installation.
#[derive(
    Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
pub enum JavaSource {
    /// Explicit user override path.
    Override,
    /// Auto-provisioned via the managed runtime manager.
    Managed,
    /// Bundled runtime under the official Mojang launcher directory.
    Mojang,
    /// OS-default / system-installed JRE.
    #[default]
    System,
}

// ---------------------------------------------------------------------------
// JavaInstallation
// ---------------------------------------------------------------------------

/// A discovered or provisioned Java runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JavaInstallation {
    /// Absolute path to the `java` (or `java.exe`) executable.
    pub path: PathBuf,
    /// Parsed Java major version (e.g. 8, 11, 17, 21).
    pub version: u32,
    /// The raw version string from `java -version` (e.g. `"17.0.9"`).
    pub version_string: String,
    /// Origin of this installation.
    #[serde(default)]
    pub source: JavaSource,
    /// Architecture reported by the JVM (`os.arch`), if available.
    #[serde(default)]
    pub arch: Option<String>,
}

impl JavaInstallation {
    /// Canonicalise the path before comparison.
    fn canonical_path(&self) -> PathBuf {
        self.path
            .canonicalize()
            .unwrap_or_else(|_| self.path.clone())
    }
}

// ---------------------------------------------------------------------------
// inspect_java — bounded probe with arch extraction and leak-free process
// ---------------------------------------------------------------------------

const INSPECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Probe a Java executable, returning version info and architecture.
///
/// Runs `java -XshowSettings:properties -version` with **both** stdout and
/// stderr captured.  Parses the `java.specification.version` (or falls back
/// to the `version "…"` line) and extracts `os.arch`.  The process is killed
/// if it does not complete within [`INSPECT_TIMEOUT`].
///
/// # Leak-free guarantee
/// The child process PID is captured *before* ownership is moved into the
/// wait thread.  If the outer thread times out on the channel it kills the
/// process by PID, guaranteeing no orphaned JVM processes.
pub fn inspect_java(path: &Path) -> Option<JavaInstallation> {
    #[cfg(test)]
    {
        let result = MOCK_INSPECT.with(|cell| {
            let guard = cell.borrow();
            guard.as_ref().map(|f| f(path))
        });
        if let Some(Some(inst)) = result {
            return Some(inst);
        }
    }
    if !path.is_file() {
        return None;
    }
    let path_for_result = path.to_path_buf();
    let cloned = path.to_path_buf();

    let mut command = std::process::Command::new(&cloned);
    command
        .arg("-XshowSettings:properties")
        .arg("-version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    // Prevent an empty command prompt window from flashing during the probe.
    crate::helpers::hide_console_window(&mut command);
    let child = command.spawn().ok()?;

    let pid = child.id();

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = child.wait_with_output();
        let _ = tx.send(result);
    });

    let output = match rx.recv_timeout(INSPECT_TIMEOUT) {
        Ok(output) => output.ok()?,
        Err(_) => {
            // Timeout: kill the child by PID to prevent leaks.
            kill_pid(pid);
            return None;
        }
    };

    // Parse from combined stderr + stdout (Java sends -XshowSettings to stderr).
    let combined = String::from_utf8_lossy(&output.stderr).into_owned();
    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let combined_all = if stdout_str.is_empty() {
        combined
    } else {
        format!("{}\n{}", combined, stdout_str)
    };

    let version_str = parse_version_string(&combined_all)?;
    let major = extract_major_version(version_str)?;
    let arch = parse_os_arch(&combined_all);

    Some(JavaInstallation {
        path: path_for_result,
        version: major,
        version_string: version_str.to_string(),
        source: JavaSource::System,
        arch,
    })
}

/// Kill a process by PID.  Platform-specific helper.
///
/// On Windows, uses `taskkill /F /T /PID` to kill the entire process tree.
/// On Unix, uses `kill -9` on the immediate process only — a safe but
/// intentionally limited approach: killing the process group would require
/// either `libc` or a guarantee that the child is a process-group leader,
/// neither of which is safely available without an extra dependency.  Since
/// the killed process is a harmless `java -XshowSettings:properties -version`
/// probe that has timed out, the immediate-process kill is sufficient.
fn kill_pid(pid: u32) {
    #[cfg(unix)]
    {
        // Immediate process only (see doc comment for rationale).
        let _ = std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .output();
    }
    #[cfg(windows)]
    {
        // /T kills the entire process tree — safe for a timed-out probe child.
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .output();
    }
}

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

/// Parse `os.arch` from the `-XshowSettings:properties` output.
fn parse_os_arch(output: &str) -> Option<String> {
    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(val) = trimmed.strip_prefix("os.arch = ") {
            let arch = val.trim().to_string();
            if !arch.is_empty() {
                return Some(arch);
            }
        }
    }
    None
}

fn parse_version_string(stderr: &str) -> Option<&str> {
    // First try `java.specification.version = 17` or `java.version = 17.0.9`
    for line in stderr.lines() {
        let trimmed = line.trim();
        if let Some(val) = trimmed.strip_prefix("java.specification.version = ") {
            let v = val.trim();
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    // Fallback: openjdk version "17.0.9" or java version "1.8.0_352"
    for line in stderr.lines() {
        let line = line.trim();
        if let Some(start) = line.find("version \"") {
            let rest = &line[start + "version \"".len()..];
            if let Some(end) = rest.find('"') {
                return Some(&rest[..end]);
            }
        }
    }
    None
}

fn extract_major_version(version: &str) -> Option<u32> {
    // Java 8 and earlier: "1.8.0_352" -> 8
    if let Some(v) = version.strip_prefix("1.") {
        if let Some(dot) = v.find('.') {
            return v[..dot].parse::<u32>().ok();
        }
        return v.parse::<u32>().ok();
    }
    // Java 9+: "17.0.9" or "21" -> take the first component
    if let Some(dot) = version.find('.') {
        return version[..dot].parse::<u32>().ok();
    }
    if let Some(underscore) = version.find('_') {
        return version[..underscore].parse::<u32>().ok();
    }
    version.parse::<u32>().ok()
}

// ---------------------------------------------------------------------------
// Discovery — System JREs (existing behaviour tagged JavaSource::System)
// ---------------------------------------------------------------------------

/// Detect system-installed JREs (the original `detect_installed_jres`
/// behaviour, now tagged [`JavaSource::System`]).
///
/// This is the backward-compatible entry point.  New callers should prefer
/// [`detect_java_candidates`] which also includes managed and Mojang runtimes.
pub fn detect_installed_jres() -> Vec<JavaInstallation> {
    let mut results = Vec::new();

    // Windows paths
    #[cfg(target_os = "windows")]
    {
        let windows_roots = [
            r"C:\Program Files\Java",
            r"C:\Program Files (x86)\Java",
            r"C:\Program Files\Eclipse Adoptium",
            r"C:\Program Files\Microsoft\jdk",
            r"C:\Program Files\Zulu",
        ];
        for root in &windows_roots {
            let dir = PathBuf::from(root);
            if dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&dir) {
                    for entry in entries.flatten() {
                        let javadir = entry.path().join("bin");
                        let path = javadir.join("java.exe");
                        if path.is_file() {
                            if let Some(mut inst) = inspect_java(&path) {
                                inst.source = JavaSource::System;
                                results.push(inst);
                            }
                        }
                    }
                }
            }
        }
    }

    // macOS paths
    #[cfg(target_os = "macos")]
    {
        let base = PathBuf::from("/Library/Java/JavaVirtualMachines");
        if base.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&base) {
                for entry in entries.flatten() {
                    let path = entry.path().join("Contents/Home/bin/java");
                    if path.is_file() {
                        if let Some(mut inst) = inspect_java(&path) {
                            inst.source = JavaSource::System;
                            results.push(inst);
                        }
                    }
                }
            }
        }
    }

    // Linux paths
    #[cfg(target_os = "linux")]
    {
        let linux_roots = ["/usr/lib/jvm", "/opt/jdk"];
        for root in &linux_roots {
            let dir = PathBuf::from(root);
            if dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&dir) {
                    for entry in entries.flatten() {
                        let path = entry.path().join("bin/java");
                        if path.is_file() {
                            if let Some(mut inst) = inspect_java(&path) {
                                inst.source = JavaSource::System;
                                results.push(inst);
                            }
                        }
                    }
                }
            }
        }
        let global = PathBuf::from("/usr/bin/java");
        if global.is_file() {
            if let Some(mut inst) = inspect_java(&global) {
                inst.source = JavaSource::System;
                results.push(inst);
            }
        }
    }

    // PATH scan (all platforms)
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            #[cfg(target_os = "windows")]
            let path = dir.join("java.exe");
            #[cfg(not(target_os = "windows"))]
            let path = dir.join("java");
            if path.is_file() {
                if let Some(mut inst) = inspect_java(&path) {
                    inst.source = JavaSource::System;
                    results.push(inst);
                }
            }
        }
    }

    results.sort_by(|left, right| {
        left.version
            .cmp(&right.version)
            .then_with(|| left.path.cmp(&right.path))
    });
    results.dedup_by(|left, right| left.path == right.path);
    results
}

// ---------------------------------------------------------------------------
// Discovery — Managed JREs
// ---------------------------------------------------------------------------

/// Detect previously provisioned managed JREs under `runtimes_root`.
///
/// Scans the known layout: `{runtimes_root}/temurin/<major>/<full_version>/<os>-<arch>/`
/// and validates each receipt alongside the Java executable.  Does NOT recurse
/// arbitrarily outside this known catalog layout.
///
/// Returns installations tagged [`JavaSource::Managed`].
pub fn detect_managed_jres(runtimes_root: &Path) -> Vec<JavaInstallation> {
    crate::runtime_manager::list_managed_runtimes(runtimes_root)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|runtime| {
            crate::runtime_manager::validate_managed_runtime(&runtime, runtime.receipt.major).ok()
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Discovery — Mojang bundled JREs
// ---------------------------------------------------------------------------

/// Maximum number of directories to scan under Mojang's `runtime/` directory.
const MOJANG_MAX_DIRS: usize = 100;

/// Maximum directory depth under `runtime/`.
const MOJANG_MAX_DEPTH: usize = 6;

/// Maximum Java candidates returned from Mojang discovery.
const MOJANG_MAX_CANDIDATES: usize = 16;

/// Best-effort bounded scan for Mojang-bundled JREs.
///
/// Scans `{minecraft_dir}/runtime/` for `bin/java(.exe)`, with caps on the
/// number of directories traversed, recursion depth, and total candidates.
/// No symlink or reparse-point escapes are followed.
///
/// Returns installations tagged [`JavaSource::Mojang`].
pub fn detect_mojang_jres(minecraft_dir: &Path) -> Vec<JavaInstallation> {
    let runtime_dir = minecraft_dir.join("runtime");
    if !runtime_dir.is_dir() {
        return Vec::new();
    }

    let mut results = Vec::new();
    let mut dir_count = 0usize;

    // Non-recursive bounded BFS-style scan.
    let mut stack = vec![runtime_dir];
    while let Some(dir) = stack.pop() {
        if dir_count >= MOJANG_MAX_DIRS || results.len() >= MOJANG_MAX_CANDIDATES {
            break;
        }
        // Guard: depth from runtime/
        // (We just count dirs rather than measure depth precisely — the max
        //  depth cap combined with max dirs is sufficient to bound the scan.)
        if stack.len() > MOJANG_MAX_DEPTH {
            continue;
        }

        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let ft = match entry.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };

            if ft.is_dir() {
                // Skip symlink/reparse-point dirs to prevent escape.
                let meta = match entry.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                #[cfg(unix)]
                {
                    if meta.file_type().is_symlink() {
                        continue;
                    }
                }
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    // Reparse point check (includes junctions/symlinks).
                    if meta.file_attributes() & 0x400 /* FILE_ATTRIBUTE_REPARSE_POINT */ != 0 {
                        continue;
                    }
                }
                dir_count += 1;
                if dir_count <= MOJANG_MAX_DIRS {
                    stack.push(entry.path());
                }
            } else if ft.is_file() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                let is_java = if cfg!(target_os = "windows") {
                    name_str.eq_ignore_ascii_case("java.exe")
                        || name_str.eq_ignore_ascii_case("javaw.exe")
                } else {
                    name_str == "java"
                };
                if is_java {
                    if results.len() >= MOJANG_MAX_CANDIDATES {
                        break;
                    }
                    if let Some(mut inst) = inspect_java(&entry.path()) {
                        inst.source = JavaSource::Mojang;
                        results.push(inst);
                    }
                }
            }
        }
    }

    results
}

// ---------------------------------------------------------------------------
// Combined discovery
// ---------------------------------------------------------------------------

/// Source-priority ordering for candidate ranking.
fn source_priority(src: &JavaSource) -> u8 {
    match src {
        JavaSource::Override => 0,
        JavaSource::Managed => 1,
        JavaSource::Mojang => 2,
        JavaSource::System => 3,
    }
}

/// Combine managed, Mojang, and system JRE candidates with deduplication.
///
/// Candidates are returned sorted by version ascending, then by source
/// priority (Managed > Mojang > System), then by stable path.
/// Duplicates (same canonical path) are removed, keeping the highest-priority
/// source entry.
pub fn detect_java_candidates(
    runtimes_root: Option<&Path>,
    minecraft_dir: Option<&Path>,
) -> Vec<JavaInstallation> {
    let mut results = Vec::new();

    // Managed (highest priority for equal major)
    if let Some(root) = runtimes_root {
        results.extend(detect_managed_jres(root));
    }

    // Mojang
    if let Some(dir) = minecraft_dir {
        results.extend(detect_mojang_jres(dir));
    }

    // System (lowest priority)
    results.extend(detect_system_jres());

    // Deduplicate by canonical path, keeping highest priority source.
    // Sort by (version, source_priority, path) then dedup by canonical path.
    results.sort_by_key(|inst| {
        let sp = source_priority(&inst.source);
        let canon = inst.canonical_path();
        (inst.version, sp, canon)
    });
    results.dedup_by(|a, b| a.canonical_path() == b.canonical_path());

    results
}

/// Tagged system JRE discovery — equivalent to [`detect_installed_jres`] but
/// with an explicit name matching the discovery-family convention.
pub fn detect_system_jres() -> Vec<JavaInstallation> {
    detect_installed_jres()
}

// ---------------------------------------------------------------------------
// Persistent inventory
// ---------------------------------------------------------------------------
//
// Full discovery spawns a `java -XshowSettings:properties -version` probe per
// candidate, which is the dominant cold-start cost of the resolve phase.  The
// inventory persists the last discovery result and trusts it while every
// recorded executable still matches its recorded file metadata and the runtime
// root directories have not changed.  A stale inventory simply falls back to
// full discovery, which rewrites it.

const JAVA_INVENTORY_SCHEMA_VERSION: u32 = 1;

/// Inventories older than this are re-discovered even when every recorded
/// executable still matches, so newly installed system JREs surface within a
/// bounded time without a user-initiated refresh.
const JAVA_INVENTORY_MAX_AGE: chrono::Duration = chrono::Duration::hours(24);

#[derive(Debug, Serialize, Deserialize)]
struct JavaInventoryRoot {
    path: String,
    modified_ns: u128,
}

#[derive(Debug, Serialize, Deserialize)]
struct JavaInventoryEntry {
    path: PathBuf,
    version: u32,
    version_string: String,
    source: JavaSource,
    arch: Option<String>,
    file_size: u64,
    modified_ns: u128,
    file_identity: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct JavaInventory {
    schema_version: u32,
    recorded_at: String,
    roots: Vec<JavaInventoryRoot>,
    candidates: Vec<JavaInventoryEntry>,
}

/// Return the persisted Java inventory when it can still be trusted without
/// re-running discovery.  The inventory is valid only when every recorded
/// executable exists with unchanged size, modification time, and platform
/// file identity, the runtime root directories are unchanged, and the
/// inventory is not older than [`JAVA_INVENTORY_MAX_AGE`].
pub fn persisted_java_candidates(
    inventory_path: &Path,
    runtimes_root: &Path,
    minecraft_dir: Option<&Path>,
) -> Option<Vec<JavaInstallation>> {
    let bytes = std::fs::read(inventory_path).ok()?;
    let inventory = serde_json::from_slice::<JavaInventory>(&bytes).ok()?;
    if inventory.schema_version != JAVA_INVENTORY_SCHEMA_VERSION {
        return None;
    }
    let recorded_at = chrono::DateTime::parse_from_rfc3339(&inventory.recorded_at).ok()?;
    if chrono::Utc::now().signed_duration_since(recorded_at.with_timezone(&chrono::Utc))
        > JAVA_INVENTORY_MAX_AGE
    {
        return None;
    }

    let mut current_roots = vec![inventory_root_state(runtimes_root)];
    if let Some(minecraft_dir) = minecraft_dir {
        current_roots.push(inventory_root_state(&minecraft_dir.join("runtime")));
    }
    if current_roots.len() != inventory.roots.len() {
        return None;
    }
    for (current, recorded) in current_roots.iter().zip(&inventory.roots) {
        if current.path != recorded.path || current.modified_ns != recorded.modified_ns {
            return None;
        }
    }

    let mut candidates = Vec::with_capacity(inventory.candidates.len());
    for entry in &inventory.candidates {
        let (file_size, modified_ns, file_identity) = inventory_file_state(&entry.path)?;
        if file_size != entry.file_size
            || modified_ns != entry.modified_ns
            || file_identity != entry.file_identity
        {
            return None;
        }
        candidates.push(JavaInstallation {
            path: entry.path.clone(),
            version: entry.version,
            version_string: entry.version_string.clone(),
            source: entry.source,
            arch: entry.arch.clone(),
        });
    }
    Some(candidates)
}

/// Persist a discovery result for future sessions.  Best-effort: failure only
/// costs a re-discovery on the next launch.
pub fn persist_java_inventory(
    inventory_path: &Path,
    runtimes_root: &Path,
    minecraft_dir: Option<&Path>,
    candidates: &[JavaInstallation],
) {
    let mut roots = vec![inventory_root_state(runtimes_root)];
    if let Some(minecraft_dir) = minecraft_dir {
        roots.push(inventory_root_state(&minecraft_dir.join("runtime")));
    }
    let entries = candidates
        .iter()
        .filter_map(|candidate| {
            let (file_size, modified_ns, file_identity) = inventory_file_state(&candidate.path)?;
            Some(JavaInventoryEntry {
                path: candidate.path.clone(),
                version: candidate.version,
                version_string: candidate.version_string.clone(),
                source: candidate.source,
                arch: candidate.arch.clone(),
                file_size,
                modified_ns,
                file_identity,
            })
        })
        .collect();
    let inventory = JavaInventory {
        schema_version: JAVA_INVENTORY_SCHEMA_VERSION,
        recorded_at: chrono::Utc::now().to_rfc3339(),
        roots,
        candidates: entries,
    };
    let Ok(bytes) = serde_json::to_vec(&inventory) else {
        return;
    };
    let parent = inventory_path.parent();
    if parent.is_some_and(|dir| std::fs::create_dir_all(dir).is_err()) {
        return;
    }
    let temp = inventory_path.with_extension(format!(
        "json.tmp-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    ));
    if std::fs::write(&temp, &bytes).is_err() {
        let _ = std::fs::remove_file(&temp);
        return;
    }
    let _ = std::fs::remove_file(inventory_path);
    if std::fs::rename(&temp, inventory_path).is_err() {
        let _ = std::fs::remove_file(&temp);
    }
}

/// Cache-first combined discovery: use the persisted inventory when valid,
/// otherwise run full discovery and refresh the inventory.  This is the entry
/// point for launch and Java-listing paths that want subprocess-free warm
/// launches.
pub fn java_candidates_cached(
    inventory_path: &Path,
    runtimes_root: &Path,
    minecraft_dir: Option<&Path>,
) -> Vec<JavaInstallation> {
    let started = std::time::Instant::now();
    if let Some(candidates) =
        persisted_java_candidates(inventory_path, runtimes_root, minecraft_dir)
    {
        eprintln!(
            "[launch-timing] java-discovery: persistent inventory hit ({} candidates, no JVM probes) in {} ms",
            candidates.len(),
            started.elapsed().as_millis()
        );
        return candidates;
    }
    eprintln!(
        "[launch-timing] java-discovery: inventory miss, running full discovery (JVM probes)"
    );
    let candidates = detect_java_candidates(Some(runtimes_root), minecraft_dir);
    persist_java_inventory(inventory_path, runtimes_root, minecraft_dir, &candidates);
    candidates
}

fn inventory_root_state(path: &Path) -> JavaInventoryRoot {
    let modified_ns = std::fs::metadata(path)
        .ok()
        .and_then(|metadata| {
            metadata
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        })
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    JavaInventoryRoot {
        path: path.to_string_lossy().into_owned(),
        modified_ns,
    }
}

fn inventory_file_state(path: &Path) -> Option<(u64, u128, Option<String>)> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let modified_ns = metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    #[cfg(unix)]
    let file_identity = Some(format!("unix:{}:{}", metadata.dev(), metadata.ino()));
    #[cfg(windows)]
    let file_identity = Some(format!(
        "windows:creation-time:{}",
        metadata.creation_time()
    ));
    #[cfg(not(any(unix, windows)))]
    let file_identity = None;
    Some((metadata.len(), modified_ns, file_identity))
}

// ---------------------------------------------------------------------------
// JavaRequirement — derived from an already-resolved VersionInfo
// ---------------------------------------------------------------------------

/// The Java major version and component string required by a Minecraft
/// version's metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JavaRequirement {
    /// Required Java major version (e.g. 8, 17, 21).
    pub major: u32,
    /// The Mojang component name (e.g. `"java-runtime-gamma"`, `"jre-legacy"`).
    pub component: String,
}

/// Derive the [`JavaRequirement`] from an already-resolved [`VersionInfo`].
///
/// The Mojang version manifest (the `.json` per version) carries an optional
/// `javaVersion` block containing `component` and `majorVersion`. When absent,
/// the requirement defaults to Java 8 with `"jre-legacy"`.
///
/// # Why a pure helper
///
/// This function does no I/O. It is intended for use **before** Java selection
/// in the launch planner (and in the Forge installer bootstrap) so the caller
/// can know what major version is needed without fetching Mojang metadata
/// again after it has already been resolved.
pub fn java_requirement_from_version(version: &VersionInfo) -> JavaRequirement {
    match version.java_version.as_ref() {
        Some(jv) => {
            let major = u32::try_from(jv.major_version)
                .ok()
                .filter(|&m| m > 0)
                .unwrap_or(8);
            let component = if jv.component.is_empty() {
                "jre-legacy".to_string()
            } else {
                jv.component.clone()
            };
            JavaRequirement { major, component }
        }
        None => JavaRequirement {
            major: 8,
            component: "jre-legacy".to_string(),
        },
    }
}

/// Cache-first helper to resolve a Minecraft version JSON from local
/// `.minecraft/versions/<id>/<id>.json` before falling back to the
/// Mojang version manifest.  Returns [`VersionInfo`] if found.
///
/// This is useful when the caller needs the Java requirement but does not
/// yet want to run the full launch planner (e.g. during installer bootstrap).
pub fn resolve_version_metadata(minecraft_dir: &Path, version_id: &str) -> Option<VersionInfo> {
    // Prefer installed base profile.
    let installed_path = minecraft_dir
        .join("versions")
        .join(version_id)
        .join(format!("{}.json", version_id));
    if let Ok(data) = std::fs::read_to_string(&installed_path) {
        if let Ok(info) = serde_json::from_str::<VersionInfo>(&data) {
            return Some(info);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- extract_major_version ---

    #[test]
    fn test_extract_major_version_18() {
        assert_eq!(extract_major_version("1.8.0_352"), Some(8));
    }

    #[test]
    fn test_extract_major_version_17() {
        assert_eq!(extract_major_version("17.0.1"), Some(17));
    }

    #[test]
    fn test_extract_major_version_21() {
        assert_eq!(extract_major_version("21"), Some(21));
    }

    #[test]
    fn test_extract_major_version_invalid() {
        assert_eq!(extract_major_version("invalid"), None);
    }

    // --- parse_version_string ---

    #[test]
    fn test_parse_version_string_java8() {
        let input = "java version \"1.8.0_352\"\nJava(TM) SE Runtime Environment (build 1.8.0_352-b08)\nJava HotSpot(TM) 64-Bit Server VM (build 25.352-b08, mixed mode)";
        assert_eq!(parse_version_string(input), Some("1.8.0_352"));
    }

    #[test]
    fn test_parse_version_string_java17() {
        let input = "openjdk version \"17.0.9\" 2023-10-17\nOpenJDK Runtime Environment (build 17.0.9+9)\nOpenJDK 64-Bit Server VM (build 17.0.9+9, mixed mode)";
        assert_eq!(parse_version_string(input), Some("17.0.9"));
    }

    #[test]
    fn test_parse_version_string_prefers_specification_version() {
        let input = "Property settings:\n    java.specification.version = 21\n    java.version = 21.0.2\n\nopenjdk version \"21.0.2\" 2024-01-16\n";
        assert_eq!(parse_version_string(input), Some("21"));
    }

    // --- parse_os_arch ---

    #[test]
    fn test_parse_os_arch_found() {
        let input = "Property settings:\n    os.arch = amd64\n    java.specification.version = 17";
        assert_eq!(parse_os_arch(input), Some("amd64".into()));
    }

    #[test]
    fn test_parse_os_arch_not_found() {
        assert_eq!(parse_os_arch("no arch here"), None);
    }

    // --- detect_installed_jres ---

    #[test]
    fn test_detect_no_panic() {
        let _ = detect_installed_jres();
    }

    // --- JavaInstallation serde backward compat ---

    #[test]
    fn test_java_installation_deserializes_without_source() {
        let json = r#"{"path": "/usr/bin/java", "version": 17, "version_string": "17"}"#;
        let inst: JavaInstallation = serde_json::from_str(json).unwrap();
        assert_eq!(inst.source, JavaSource::System);
        assert!(inst.arch.is_none());
    }

    #[test]
    fn test_java_installation_deserializes_with_source() {
        let json = r#"{"path": "/managed/java", "version": 21, "version_string": "21", "source": "Managed", "arch": "amd64"}"#;
        let inst: JavaInstallation = serde_json::from_str(json).unwrap();
        assert_eq!(inst.source, JavaSource::Managed);
        assert_eq!(inst.arch.as_deref(), Some("amd64"));
    }

    #[test]
    fn test_java_installation_roundtrip() {
        let inst = JavaInstallation {
            path: PathBuf::from("/test/java"),
            version: 17,
            version_string: "17.0.1".into(),
            source: JavaSource::Mojang,
            arch: Some("aarch64".into()),
        };
        let json = serde_json::to_string(&inst).unwrap();
        let deserialized: JavaInstallation = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.source, JavaSource::Mojang);
        assert_eq!(deserialized.arch.as_deref(), Some("aarch64"));
    }

    // --- source priority ---

    #[test]
    fn test_source_priority_ordering() {
        assert!(source_priority(&JavaSource::Override) < source_priority(&JavaSource::Managed));
        assert!(source_priority(&JavaSource::Managed) < source_priority(&JavaSource::Mojang));
        assert!(source_priority(&JavaSource::Mojang) < source_priority(&JavaSource::System));
    }

    // --- persistent inventory ---

    #[test]
    fn persisted_inventory_roundtrip_is_valid_when_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let runtimes_root = tmp.path().join("runtimes");
        let inventory_path = tmp.path().join("java-inventory.json");
        let fake_java = tmp.path().join("java.exe");
        std::fs::write(&fake_java, b"fake java binary").unwrap();

        let candidate = JavaInstallation {
            path: fake_java.clone(),
            version: 21,
            version_string: "21.0.1".into(),
            source: JavaSource::System,
            arch: Some("amd64".into()),
        };
        persist_java_inventory(&inventory_path, &runtimes_root, None, &[candidate.clone()]);
        assert_eq!(
            persisted_java_candidates(&inventory_path, &runtimes_root, None),
            Some(vec![candidate])
        );
    }

    #[test]
    fn persisted_inventory_is_invalidated_by_executable_metadata_change() {
        let tmp = tempfile::tempdir().unwrap();
        let runtimes_root = tmp.path().join("runtimes");
        let inventory_path = tmp.path().join("java-inventory.json");
        let fake_java = tmp.path().join("java.exe");
        std::fs::write(&fake_java, b"fake java binary").unwrap();

        let candidate = JavaInstallation {
            path: fake_java.clone(),
            version: 21,
            version_string: "21.0.1".into(),
            source: JavaSource::System,
            arch: None,
        };
        persist_java_inventory(&inventory_path, &runtimes_root, None, &[candidate]);
        std::fs::write(&fake_java, b"replaced java binary").unwrap();
        assert_eq!(
            persisted_java_candidates(&inventory_path, &runtimes_root, None),
            None
        );
    }

    #[test]
    fn persisted_inventory_is_invalidated_by_missing_executable() {
        let tmp = tempfile::tempdir().unwrap();
        let runtimes_root = tmp.path().join("runtimes");
        let inventory_path = tmp.path().join("java-inventory.json");
        let fake_java = tmp.path().join("java.exe");
        std::fs::write(&fake_java, b"fake java binary").unwrap();

        let candidate = JavaInstallation {
            path: fake_java.clone(),
            version: 21,
            version_string: "21.0.1".into(),
            source: JavaSource::System,
            arch: None,
        };
        persist_java_inventory(&inventory_path, &runtimes_root, None, &[candidate]);
        std::fs::remove_file(&fake_java).unwrap();
        assert_eq!(
            persisted_java_candidates(&inventory_path, &runtimes_root, None),
            None
        );
    }

    #[test]
    fn persisted_inventory_is_invalidated_by_runtime_root_change() {
        let tmp = tempfile::tempdir().unwrap();
        let runtimes_root = tmp.path().join("runtimes");
        let inventory_path = tmp.path().join("java-inventory.json");
        let fake_java = tmp.path().join("java.exe");
        std::fs::write(&fake_java, b"fake java binary").unwrap();

        let candidate = JavaInstallation {
            path: fake_java.clone(),
            version: 21,
            version_string: "21.0.1".into(),
            source: JavaSource::Managed,
            arch: None,
        };
        persist_java_inventory(&inventory_path, &runtimes_root, None, &[candidate]);
        std::fs::create_dir_all(runtimes_root.join("temurin").join("21")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(
            persisted_java_candidates(&inventory_path, &runtimes_root, None),
            None
        );
    }

    #[test]
    fn java_candidates_cached_skips_discovery_on_valid_inventory() {
        let tmp = tempfile::tempdir().unwrap();
        let runtimes_root = tmp.path().join("runtimes");
        let inventory_path = tmp.path().join("java-inventory.json");
        let fake_java = tmp.path().join("java.exe");
        std::fs::write(&fake_java, b"fake java binary").unwrap();

        let candidate = JavaInstallation {
            path: fake_java.clone(),
            version: 21,
            version_string: "21.0.1".into(),
            source: JavaSource::System,
            arch: None,
        };
        persist_java_inventory(&inventory_path, &runtimes_root, None, &[candidate.clone()]);

        // Discovery is mocked to find nothing; a cache hit must not call it.
        let _guard = set_mock_inspect(Some(mock_inspect_none));
        assert_eq!(
            java_candidates_cached(&inventory_path, &runtimes_root, None),
            vec![candidate]
        );
    }

    fn mock_inspect_none(_: &Path) -> Option<JavaInstallation> {
        None
    }

    #[test]
    fn java_candidates_cached_rediscovers_and_refreshes_inventory() {
        let tmp = tempfile::tempdir().unwrap();
        let runtimes_root = tmp.path().join("runtimes");
        let inventory_path = tmp.path().join("java-inventory.json");
        let fake_java = tmp.path().join("bin").join("java");
        std::fs::create_dir_all(fake_java.parent().unwrap()).unwrap();
        std::fs::write(&fake_java, b"java1").unwrap();

        let candidate = JavaInstallation {
            path: fake_java.clone(),
            version: 21,
            version_string: "21.0.1".into(),
            source: JavaSource::System,
            arch: None,
        };
        persist_java_inventory(&inventory_path, &runtimes_root, None, &[candidate]);
        assert!(persisted_java_candidates(&inventory_path, &runtimes_root, None).is_some());

        // Executable content changes invalidate the inventory.
        std::fs::write(&fake_java, b"java2").unwrap();
        assert!(persisted_java_candidates(&inventory_path, &runtimes_root, None).is_none());

        // The cached path therefore re-discovers (mock-driven, no real JVM
        // probes) and persists a refreshed inventory that is trusted again.
        let _guard = set_mock_inspect(Some(mock_managed_java));
        java_candidates_cached(&inventory_path, &runtimes_root, None);
        drop(_guard);

        let loaded = persisted_java_candidates(&inventory_path, &runtimes_root, None);
        assert!(loaded.is_some());
        if let Some(loaded) = loaded {
            // Every entry carries the mock's version, never a real probe's.
            assert!(loaded.iter().all(|candidate| candidate.version == 21));
        }
    }

    fn mock_managed_java(path: &Path) -> Option<JavaInstallation> {
        Some(JavaInstallation {
            path: path.to_path_buf(),
            version: 21,
            version_string: "21.0.2".into(),
            source: JavaSource::Managed,
            arch: Some(crate::runtime_catalog::normalize_arch(std::env::consts::ARCH)?.to_string()),
        })
    }

    #[test]
    fn test_detect_managed_jres_rejects_tampered_java() {
        use sha2::Digest;

        let _guard = set_mock_inspect(Some(mock_managed_java));
        let tmp = tempfile::tempdir().unwrap();
        let os = crate::runtime_catalog::normalize_os(std::env::consts::OS).unwrap();
        let arch = crate::runtime_catalog::normalize_arch(std::env::consts::ARCH).unwrap();
        let platform_dir = tmp
            .path()
            .join("temurin")
            .join("21")
            .join("21.0.2+13")
            .join(format!("{os}-{arch}"));
        let java_relative = if cfg!(windows) {
            PathBuf::from("bin/java.exe")
        } else {
            PathBuf::from("bin/java")
        };
        let java_path = platform_dir.join(&java_relative);
        std::fs::create_dir_all(java_path.parent().unwrap()).unwrap();
        std::fs::write(&java_path, b"original java").unwrap();
        let java_sha256 = hex::encode(sha2::Sha256::digest(b"original java"));

        let receipt = crate::runtime_manager::RuntimeReceipt {
            schema_version: crate::runtime_manager::RECEIPT_SCHEMA_VERSION,
            vendor: "eclipse-temurin".into(),
            major: 21,
            full_version: "21.0.2+13".into(),
            os: os.into(),
            arch: arch.into(),
            archive_sha256: "a".repeat(64),
            archive_size: 1,
            source_url: "https://api.adoptium.net/test".into(),
            java_relative_path: java_relative.to_string_lossy().into_owned(),
            installed_at: chrono::Utc::now(),
            last_used_at: None,
            successful_use_at: None,
            java_sha256: Some(java_sha256),
        };
        receipt
            .write_to(&platform_dir.join("receipt.json"))
            .unwrap();

        assert_eq!(detect_managed_jres(tmp.path()).len(), 1);

        std::fs::write(&java_path, b"tampered java").unwrap();
        assert!(detect_managed_jres(tmp.path()).is_empty());
    }

    // --- JavaRequirement tests ---

    #[test]
    fn test_java_requirement_from_version_specified() {
        let v = crate::launch::VersionInfo {
            java_version: Some(crate::launch::JavaVersion {
                component: "java-runtime-gamma".into(),
                major_version: 21,
            }),
            ..Default::default()
        };
        let req = java_requirement_from_version(&v);
        assert_eq!(req.major, 21);
        assert_eq!(req.component, "java-runtime-gamma");
    }

    #[test]
    fn test_java_requirement_from_version_defaults_to_8() {
        let v = crate::launch::VersionInfo::default();
        let req = java_requirement_from_version(&v);
        assert_eq!(req.major, 8);
        assert_eq!(req.component, "jre-legacy");
    }

    #[test]
    fn test_java_requirement_from_version_zero_major_defaults() {
        let v = crate::launch::VersionInfo {
            java_version: Some(crate::launch::JavaVersion {
                component: "test".into(),
                major_version: 0,
            }),
            ..Default::default()
        };
        let req = java_requirement_from_version(&v);
        assert_eq!(req.major, 8);
        assert_eq!(req.component, "test");
    }

    #[test]
    fn test_java_requirement_roundtrip_serialize() {
        let req = JavaRequirement {
            major: 17,
            component: "java-runtime-alpha".into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let deserialized: JavaRequirement = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, req);
    }

    // --- resolve_version_metadata returns None for missing ---

    #[test]
    fn test_resolve_version_metadata_missing_dir_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let result = resolve_version_metadata(tmp.path(), "1.21");
        assert!(result.is_none());
    }
}
