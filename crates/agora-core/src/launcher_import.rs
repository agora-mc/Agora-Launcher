//! Read-only launcher discovery adapter for Prism, CurseForge, and Modrinth App.
//!
//! This module provides pure detection and metadata extraction from source
//! launchers without network access, writes, or modifications to source data.
//!
//! ## Public API
//! - [`discover_prism_launcher`] — detect Prism Launcher and enumerate instances
//! - [`discover_curseforge_launcher`] — detect CurseForge App and enumerate instances
//! - [`discover_modrinth_launcher`] — detect Modrinth App and enumerate instances
//!
//! Each function accepts an optional custom root and returns detection results
//! with detailed per-instance metadata. Unsupported instances are returned with
//! [`CandidateStatus::Unsupported`] and a list of reasons instead of panicking.
//!
//! ## Design constraints
//! - No network access
//! - No writes to disk
//! - No dependencies on Tauri, clap, or MCP crates
//! - Uses only serde, rusqlite, chrono, and std

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// The kind of source launcher detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LauncherKind {
    Prism,
    CurseForge,
    Modrinth,
}

impl LauncherKind {
    pub fn display_name(self) -> &'static str {
        match self {
            LauncherKind::Prism => "Prism Launcher",
            LauncherKind::CurseForge => "CurseForge",
            LauncherKind::Modrinth => "Modrinth App",
        }
    }
}

/// A detected launcher installation with root paths and metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedLauncher {
    /// Stable identifier for this launcher installation.
    pub installation_key: String,
    pub kind: LauncherKind,
    pub display_name: String,
    /// The canonical launcher data/config directory.
    pub config_root: PathBuf,
    /// The resolved directory containing instance subdirectories.
    pub instances_dir: PathBuf,
    pub instance_count: usize,
    pub detection_warnings: Vec<String>,
}

/// Whether an import candidate is ready, needs review, or unsupported.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateStatus {
    Ready,
    NeedsReview,
    Unsupported { reasons: Vec<String> },
}

/// The launch strategy that will be used after import.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchStrategy {
    Normal,
    Delegated,
}

/// A loader tuple identifying the mod loader, its version, and the Minecraft version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoaderTuple {
    /// Normalized loader name: vanilla, fabric, quilt, forge, neoforge.
    pub loader: String,
    pub loader_version: String,
    pub minecraft_version: String,
}

/// Preview of sanitizable launch settings from the source launcher.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchSettingsPreview {
    /// Maximum memory in MB (if detected).
    pub memory_mb: Option<i64>,
    /// Java path from the source (if detected and not launcher-owned).
    pub java_path: Option<String>,
    /// JVM argument list (excluding classpath, agents, credential-bearing flags).
    pub jvm_args: Vec<String>,
}

/// Summary of the content inside a detected instance payload root.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentInventory {
    pub payload_root: PathBuf,
    pub total_files: u64,
    pub total_bytes: u64,
    pub has_mods: bool,
    pub has_resourcepacks: bool,
    pub has_shaderpacks: bool,
    pub has_datapacks: bool,
    pub has_saves: bool,
}

/// A single import candidate instance discovered from a source launcher.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportCandidate {
    /// Stable per-launcher instance key (folder name or UUID).
    pub source_key: String,
    pub launcher: LauncherKind,
    /// Matches DetectedLauncher::installation_key.
    pub launcher_installation_key: String,
    /// Human-readable display name from source metadata.
    pub display_name: String,
    /// Absolute path to an icon file, if found.
    pub icon_path: Option<PathBuf>,
    /// The canonical game-owned payload root directory.
    pub payload_root: PathBuf,
    pub inventory: ContentInventory,
    /// The resolved loader tuple, if parseable.
    pub loader_tuple: Option<LoaderTuple>,
    /// ISO-8601 last-played timestamp from source metadata.
    pub last_played: Option<String>,
    /// The launch strategy determined for this candidate.
    pub launch_strategy: LaunchStrategy,
    pub settings_preview: LaunchSettingsPreview,
    pub status: CandidateStatus,
    pub warnings: Vec<String>,
}

/// Result of discovering instances for one launcher type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LauncherDiscovery {
    /// None when the launcher was not found at any probed path.
    pub launcher: Option<DetectedLauncher>,
    /// Candidates from this launcher (may include unsupported entries with reasons).
    pub candidates: Vec<ImportCandidate>,
}

impl LauncherDiscovery {
    pub fn empty() -> Self {
        LauncherDiscovery {
            launcher: None,
            candidates: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Parse a simple INI-style file with [section] headers and key=value lines.
///
/// Supports:
/// - Case-insensitive key lookup (keys stored in lowercase)
/// - ; and # line comments
/// - Values containing = (only the first = is the delimiter)
/// - Leading/trailing whitespace trimming on both keys and values
/// - Empty lines and whitespace-only lines
///
/// Does NOT support:
/// - Multi-line values
/// - Escape sequences
/// - Nested sections
fn parse_ini(text: &str) -> HashMap<String, HashMap<String, String>> {
    let mut result: HashMap<String, HashMap<String, String>> = HashMap::new();
    result.entry(String::new()).or_default();
    let mut current_section = String::new();

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            if let Some(end) = line.find(']') {
                let section_name = line[1..end].trim().to_string();
                current_section = section_name;
                result.entry(current_section.clone()).or_default();
            }
        } else if let Some(eq_pos) = line.find('=') {
            let key = line[..eq_pos].trim().to_ascii_lowercase();
            let value = line[eq_pos + 1..].trim().to_string();
            result
                .get_mut(&current_section)
                .map(|map| map.insert(key, value));
        }
    }

    result
}

/// Read a text file, returning None on any I/O error.
fn try_read_string(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// Read and parse a JSON file, returning None on any error.
fn try_parse_json<T>(path: &Path) -> Option<T>
where
    T: serde::de::DeserializeOwned,
{
    let text = try_read_string(path)?;
    serde_json::from_str(&text).ok()
}

/// Resolve a path relative to a base, preserving absolute paths.
fn resolve_relative(base: &Path, relative: &str) -> PathBuf {
    let candidate = PathBuf::from(relative);
    if candidate.is_absolute() {
        candidate
    } else {
        base.join(&candidate)
    }
}

/// Return Some(path) if the candidate exists and is a regular file.
fn detect_icon(candidate_path: &Path) -> Option<PathBuf> {
    if candidate_path.exists() {
        if let Ok(ft) = std::fs::metadata(candidate_path) {
            if ft.is_file() {
                return Some(candidate_path.to_path_buf());
            }
        }
    }
    None
}

fn unsafe_filesystem_entry(path: &Path, file_type: &std::fs::FileType) -> bool {
    if file_type.is_symlink() {
        return true;
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if std::fs::symlink_metadata(path)
            .map(|metadata| metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
            .unwrap_or(true)
        {
            return true;
        }
    }
    false
}

/// Known non-loader Prism component UIDs that are safe to ignore.
const PRISM_IGNORED_COMPONENTS: &[&str] = &[
    "net.minecraft",
    "net.fabricmc.intermediary",
    "org.quiltmc.intermediary",
    "org.prismlauncher.prismlauncher",
    "org.prismlauncher.prismlauncher.mmc",
    "org.prismlauncher.dev",
    "org.prismlauncher.hooks",
    "org.polymc.polymc",
    "org.polymc.polymc.mmc",
    "org.multimc.multimc",
    "org.multimc.multimc.mmc",
    "net.minecraft.liteloader",
    "net.optifine",
    "net.optifine.snapshot",
    "org.lwjgl",
    "org.lwjgl3",
];

/// Known loader UIDs and their canonical names.
fn loader_for_uid(uid: &str) -> Option<(&'static str, &'static str)> {
    match uid {
        "net.fabricmc.fabric-loader" => Some(("fabric", "")),
        "org.quiltmc.quilt-loader" => Some(("quilt", "")),
        "net.minecraftforge" => Some(("forge", "")),
        "net.neoforged" | "net.neoforged.neoforge" => Some(("neoforge", "")),
        _ => None,
    }
}

/// Check whether a component UID is a known safe/internal component.
fn is_known_safe_component(uid: &str) -> bool {
    if PRISM_IGNORED_COMPONENTS.contains(&uid) {
        return true;
    }
    if uid.starts_with("org.prismlauncher.")
        || uid.starts_with("org.polymc.")
        || uid.starts_with("org.multimc.")
    {
        return true;
    }
    false
}

// ---------------------------------------------------------------------------
// Content inventory
// ---------------------------------------------------------------------------

/// Walk a payload root and count files, bytes, and detect content subdirectories.
///
/// Never follows symlinks, junctions, or reparse points.
fn inventory_payload(payload_root: &Path) -> Result<ContentInventory, String> {
    if !payload_root.exists() {
        return Err(format!(
            "Payload root does not exist: {}",
            payload_root.display()
        ));
    }
    if !payload_root.is_dir() {
        return Err(format!(
            "Payload root is not a directory: {}",
            payload_root.display()
        ));
    }

    let mut total_files: u64 = 0;
    let mut total_bytes: u64 = 0;
    let mut has_mods = false;
    let mut has_resourcepacks = false;
    let mut has_shaderpacks = false;
    let mut has_datapacks = false;
    let mut has_saves = false;

    let mut dirs: Vec<PathBuf> = vec![payload_root.to_path_buf()];

    while let Some(dir) = dirs.pop() {
        if let Some(dirname) = dir.file_name().and_then(|n| n.to_str()) {
            match dirname {
                "mods" => has_mods = true,
                "resourcepacks" => has_resourcepacks = true,
                "shaderpacks" => has_shaderpacks = true,
                "datapacks" => has_datapacks = true,
                "saves" => has_saves = true,
                _ => {}
            }
        }

        let read_dir = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => continue,
        };

        for entry in read_dir {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            let file_type = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };

            if unsafe_filesystem_entry(&entry.path(), &file_type) {
                continue;
            }

            if file_type.is_dir() {
                dirs.push(entry.path());
            } else if file_type.is_file() {
                total_files += 1;
                if let Ok(meta) = entry.metadata() {
                    total_bytes += meta.len();
                }
            }
        }
    }

    Ok(ContentInventory {
        payload_root: payload_root.to_path_buf(),
        total_files,
        total_bytes,
        has_mods,
        has_resourcepacks,
        has_shaderpacks,
        has_datapacks,
        has_saves,
    })
}

// ---------------------------------------------------------------------------
// Platform default path helpers
// ---------------------------------------------------------------------------

/// Return the platform default root for Prism Launcher data.
fn platform_prism_root() -> Option<PathBuf> {
    dirs::data_dir().map(|directory| directory.join("PrismLauncher"))
}

/// Return the platform default root for CurseForge (Windows only).
fn platform_curseforge_root() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        dirs::data_dir().map(|d| d.join("CurseForge"))
    } else {
        None
    }
}

/// Return the platform default root for Modrinth App.
fn platform_modrinth_root() -> Option<PathBuf> {
    dirs::data_dir().map(|directory| directory.join("ModrinthApp"))
}

/// Collect default candidate roots, optionally prepending a custom root.
fn candidate_roots(
    custom_root: Option<&Path>,
    platform_fn: fn() -> Option<PathBuf>,
) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(custom) = custom_root {
        roots.push(custom.to_path_buf());
        return roots;
    }
    if let Some(platform) = platform_fn() {
        if !roots.iter().any(|r| r == &platform) {
            roots.push(platform);
        }
    }
    roots
}

fn push_unique(roots: &mut Vec<PathBuf>, path: Option<PathBuf>) {
    if let Some(path) = path {
        if !roots.iter().any(|existing| existing == &path) {
            roots.push(path);
        }
    }
}

fn prism_candidate_roots(custom_root: Option<&Path>) -> Vec<PathBuf> {
    let mut roots = candidate_roots(custom_root, platform_prism_root);
    if custom_root.is_none() {
        push_unique(
            &mut roots,
            dirs::home_dir().map(|home| {
                home.join(".var/app/org.prismlauncher.PrismLauncher/data/PrismLauncher")
            }),
        );
    }
    roots
}

fn curseforge_candidate_roots(custom_root: Option<&Path>) -> Vec<PathBuf> {
    let mut roots = candidate_roots(custom_root, platform_curseforge_root);
    if custom_root.is_none() {
        push_unique(
            &mut roots,
            dirs::home_dir().map(|home| home.join("curseforge/minecraft")),
        );
        push_unique(
            &mut roots,
            dirs::document_dir().map(|documents| documents.join("curseforge/minecraft")),
        );
    }
    roots
}

fn modrinth_candidate_roots(custom_root: Option<&Path>) -> Vec<PathBuf> {
    let mut roots = candidate_roots(custom_root, platform_modrinth_root);
    if custom_root.is_none() {
        push_unique(
            &mut roots,
            dirs::data_dir().map(|data| data.join("com.modrinth.theseus")),
        );
        push_unique(
            &mut roots,
            dirs::home_dir()
                .map(|home| home.join(".var/app/com.modrinth.ModrinthApp/data/ModrinthApp")),
        );
    }
    roots
}

// ---------------------------------------------------------------------------
// Prism Launcher detection
// ---------------------------------------------------------------------------

/// Try to discover a Prism Launcher installation at root.
///
/// Checks for prismlauncher.cfg, parses InstanceDir, and validates the
/// resolved instances directory.
fn detect_prism_installation(root: &Path) -> Option<DetectedLauncher> {
    let cfg_path = root.join("prismlauncher.cfg");
    let cfg_text = try_read_string(&cfg_path)?;
    let cfg_map = parse_ini(&cfg_text);

    let instance_dir_raw = cfg_map
        .get("General")
        .or_else(|| cfg_map.get("general"))
        .or_else(|| cfg_map.get(""))
        .and_then(|map| map.get("instancedir"))?;

    let instances_dir = resolve_relative(root, instance_dir_raw);

    if !instances_dir.is_dir() {
        return None;
    }

    let instance_count = count_prism_subdirs(&instances_dir);

    Some(DetectedLauncher {
        installation_key: format!("prism:{}", root.to_string_lossy()),
        kind: LauncherKind::Prism,
        display_name: "Prism Launcher".to_string(),
        config_root: root.to_path_buf(),
        instances_dir,
        instance_count,
        detection_warnings: Vec::new(),
    })
}

/// Count subdirectories that have instance.cfg.
fn count_prism_subdirs(instances_dir: &Path) -> usize {
    let read_dir = match std::fs::read_dir(instances_dir) {
        Ok(rd) => rd,
        Err(_) => return 0,
    };
    let mut count = 0usize;
    for entry in read_dir {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.path().is_dir() && entry.path().join("instance.cfg").exists() {
            count += 1;
        }
    }
    count
}

/// Enumerate Prism instances from a known-good instances directory.
fn enumerate_prism_instances(launcher: &DetectedLauncher) -> Vec<ImportCandidate> {
    let read_dir = match std::fs::read_dir(&launcher.instances_dir) {
        Ok(rd) => rd,
        Err(_) => return Vec::new(),
    };

    let mut candidates: Vec<ImportCandidate> = Vec::new();
    for entry in read_dir {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let instance_dir = entry.path();
        if !instance_dir.is_dir() {
            continue;
        }
        if !instance_dir.join("instance.cfg").exists() {
            continue;
        }

        let folder_name = instance_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        let mut candidate = inspect_prism_instance(&folder_name, &instance_dir);
        candidate.launcher_installation_key = launcher.installation_key.clone();
        candidates.push(candidate);
    }
    candidates
}

/// Inspect a single Prism instance directory.
fn inspect_prism_instance(folder_name: &str, instance_dir: &Path) -> ImportCandidate {
    let cfg_path = instance_dir.join("instance.cfg");
    let cfg_text = match try_read_string(&cfg_path) {
        Some(t) => t,
        None => {
            return unsupported_candidate(
                LauncherKind::Prism,
                folder_name,
                instance_dir,
                vec!["Cannot read instance.cfg".to_string()],
            );
        }
    };
    let cfg = parse_ini(&cfg_text);

    let payload_root = if instance_dir.join("minecraft").is_dir() {
        instance_dir.join("minecraft")
    } else if instance_dir.join(".minecraft").is_dir() {
        instance_dir.join(".minecraft")
    } else {
        return unsupported_candidate(
            LauncherKind::Prism,
            folder_name,
            instance_dir,
            vec!["Neither minecraft/ nor .minecraft/ directory found".to_string()],
        );
    };

    let display_name = cfg
        .get("Minecraft")
        .and_then(|m| m.get("name"))
        .or_else(|| cfg.get("General").and_then(|g| g.get("name")))
        .or_else(|| cfg.get("").and_then(|root| root.get("name")))
        .cloned()
        .unwrap_or_else(|| folder_name.to_string());

    let icon_path = cfg
        .get("Minecraft")
        .and_then(|m| m.get("iconfile"))
        .or_else(|| cfg.get("Settings").and_then(|s| s.get("iconfile")))
        .or_else(|| cfg.get("").and_then(|root| root.get("iconfile")))
        .and_then(|icon_file| {
            let icon_path = resolve_relative(instance_dir, icon_file);
            detect_icon(&icon_path)
        });

    let pack_path = instance_dir.join("mmc-pack.json");
    let (loader_tuple, component_warnings) = parse_prism_pack(&pack_path);

    let settings_preview = parse_prism_settings(&cfg);

    let inventory = inventory_payload(&payload_root).unwrap_or_else(|_| ContentInventory {
        payload_root: payload_root.clone(),
        total_files: 0,
        total_bytes: 0,
        has_mods: false,
        has_resourcepacks: false,
        has_shaderpacks: false,
        has_datapacks: false,
        has_saves: false,
    });

    let (status, mut all_warnings) = if loader_tuple.is_some() {
        if component_warnings.is_empty() {
            (CandidateStatus::Ready, Vec::new())
        } else {
            (
                CandidateStatus::Unsupported {
                    reasons: component_warnings.clone(),
                },
                component_warnings.clone(),
            )
        }
    } else {
        (
            CandidateStatus::Unsupported {
                reasons: vec![
                    "Could not determine a valid Minecraft + loader tuple from mmc-pack.json"
                        .to_string(),
                ],
            },
            component_warnings,
        )
    };

    if inventory.total_files == 0 {
        all_warnings.push("Payload root is empty".to_string());
    }

    ImportCandidate {
        source_key: folder_name.to_string(),
        launcher: LauncherKind::Prism,
        launcher_installation_key: String::new(),
        display_name,
        icon_path,
        payload_root,
        inventory,
        loader_tuple,
        last_played: None,
        launch_strategy: LaunchStrategy::Normal,
        settings_preview,
        status,
        warnings: all_warnings,
    }
}

#[derive(Deserialize)]
struct MmcPack {
    #[serde(default)]
    components: Vec<ComponentEntry>,
}

#[derive(Deserialize)]
struct ComponentEntry {
    uid: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default, alias = "cachedName")]
    cached_name: Option<String>,
    #[serde(default, alias = "cachedImportant")]
    cached_important: Option<bool>,
    #[serde(default, alias = "cachedEnabled")]
    cached_enabled: Option<bool>,
}

/// Parse mmc-pack.json and extract Minecraft version and supported loader.
fn parse_prism_pack(pack_path: &Path) -> (Option<LoaderTuple>, Vec<String>) {
    let pack_text = match try_read_string(pack_path) {
        Some(t) => t,
        None => {
            return (
                None,
                vec!["mmc-pack.json not found or unreadable".to_string()],
            )
        }
    };

    let pack: MmcPack = match serde_json::from_str(&pack_text) {
        Ok(p) => p,
        Err(e) => return (None, vec![format!("Failed to parse mmc-pack.json: {e}")]),
    };

    let mut minecraft_version: Option<String> = None;
    let mut loader_version: Option<String> = None;
    let mut loader_name: Option<&'static str> = None;
    let mut warnings: Vec<String> = Vec::new();

    for comp in &pack.components {
        let is_important = comp.cached_important.unwrap_or(false);
        let is_enabled = comp.cached_enabled.unwrap_or(true);
        if !is_enabled {
            continue;
        }

        if comp.uid == "net.minecraft" {
            minecraft_version = comp.version.clone();
            continue;
        }

        if let Some((name, _)) = loader_for_uid(&comp.uid) {
            if let Some(existing_loader) = loader_name {
                warnings.push(format!(
                    "Multiple loaders found: {} (already have {})",
                    comp.uid, existing_loader
                ));
            } else {
                loader_name = Some(name);
                loader_version = comp.version.clone();
            }
            continue;
        }

        if is_known_safe_component(&comp.uid) {
            continue;
        }

        if is_important {
            warnings.push(format!(
                "Unknown important component '{}' (name: {}) is enabled",
                comp.uid,
                comp.cached_name.as_deref().unwrap_or("unknown")
            ));
        }
    }

    let mc_ver = match minecraft_version {
        Some(v) => v,
        None => {
            return (
                None,
                vec!["mmc-pack.json missing net.minecraft component".to_string()],
            )
        }
    };

    let tuple = LoaderTuple {
        loader: loader_name.unwrap_or("vanilla").to_string(),
        loader_version: loader_version.unwrap_or_default(),
        minecraft_version: mc_ver,
    };
    (Some(tuple), warnings)
}

/// Parse instance settings from a Prism instance.cfg INI map.
fn parse_prism_settings(cfg: &HashMap<String, HashMap<String, String>>) -> LaunchSettingsPreview {
    let mut settings = cfg.get("").cloned().unwrap_or_default();
    if let Some(section) = cfg.get("Settings") {
        settings.extend(section.clone());
    }

    let memory_mb = settings
        .get("maxmem")
        .or_else(|| settings.get("maxmemalloc"))
        .and_then(|v| v.parse::<i64>().ok())
        .or_else(|| {
            settings
                .get("jvmargs")
                .or_else(|| settings.get("JvmArgs"))
                .and_then(|args| {
                    for part in args.split_whitespace() {
                        if let Some(rest) = part.strip_prefix("-Xmx") {
                            if let Some(mb) = parse_memory_value(rest) {
                                return Some(mb);
                            }
                        }
                    }
                    None
                })
        });

    let java_path = settings
        .get("javapath")
        .filter(|p| !p.trim().is_empty())
        .cloned();

    let jvm_args: Vec<String> = settings
        .get("jvmargs")
        .map(|args| {
            args.split_whitespace()
                .filter(|arg| {
                    !arg.starts_with("-Xmx")
                        && !arg.starts_with("-Xms")
                        && !arg.starts_with("-Xss")
                        && !arg.starts_with("-cp")
                        && !arg.starts_with("-classpath")
                        && !arg.starts_with("-agentpath")
                        && !arg.starts_with("-javaagent:")
                        && !arg.starts_with("-Djava.library.path")
                })
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();

    LaunchSettingsPreview {
        memory_mb,
        java_path,
        jvm_args,
    }
}

/// Parse a memory value like 2048M, 2G, 4096 into MB.
fn parse_memory_value(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (num_str, multiplier) =
        if let Some(rest) = s.strip_suffix(|c: char| c.eq_ignore_ascii_case(&'g')) {
            (rest, 1024i64)
        } else if let Some(rest) = s.strip_suffix(|c: char| c.eq_ignore_ascii_case(&'m')) {
            (rest, 1i64)
        } else {
            (s, 1i64)
        };
    let num: i64 = num_str.trim().parse().ok()?;
    Some(num * multiplier)
}

// ---------------------------------------------------------------------------
// CurseForge detection
// ---------------------------------------------------------------------------

/// Detect a CurseForge installation at the given root.
fn detect_curseforge_installation(root: &Path) -> Option<DetectedLauncher> {
    let minecraft_root = resolve_curseforge_minecraft_root(root)?;
    let instances_dir = minecraft_root.join("Instances");
    if !instances_dir.is_dir() {
        return None;
    }

    let instance_count = count_cf_subdirs(&instances_dir);
    Some(DetectedLauncher {
        installation_key: format!("curseforge:{}", root.to_string_lossy()),
        kind: LauncherKind::CurseForge,
        display_name: "CurseForge".to_string(),
        config_root: root.to_path_buf(),
        instances_dir,
        instance_count,
        detection_warnings: Vec::new(),
    })
}

fn count_cf_subdirs(instances_dir: &Path) -> usize {
    let read_dir = match std::fs::read_dir(instances_dir) {
        Ok(rd) => rd,
        Err(_) => return 0,
    };
    let mut count = 0usize;
    for entry in read_dir {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.path().is_dir() && entry.path().join("minecraftinstance.json").exists() {
            count += 1;
        }
    }
    count
}

/// Try to resolve the CurseForge Minecraft root from a candidate root.
fn resolve_curseforge_minecraft_root(root: &Path) -> Option<PathBuf> {
    let storage_path = root.join("storage.json");
    if storage_path.is_file() {
        if let Some(mc_root) = parse_storage_json_minecraft_root(&storage_path) {
            let resolved = resolve_relative(root, &mc_root);
            if resolved.is_dir() {
                return Some(resolved);
            }
        }
    }
    if root.join("Instances").is_dir() {
        return Some(root.to_path_buf());
    }
    None
}

/// Parse the nested minecraft-settings JSON from storage.json.
fn parse_storage_json_minecraft_root(storage_path: &Path) -> Option<String> {
    #[derive(Deserialize)]
    struct StorageJson {
        #[serde(rename = "minecraft-settings")]
        minecraft_settings: Option<serde_json::Value>,
    }

    let text = try_read_string(storage_path)?;
    let storage: StorageJson = serde_json::from_str(&text).ok()?;

    match storage.minecraft_settings? {
        serde_json::Value::String(encoded) => {
            let inner: serde_json::Value = serde_json::from_str(&encoded).ok()?;
            inner.get("minecraftRoot")?.as_str().map(|s| s.to_string())
        }
        serde_json::Value::Object(map) => map.get("minecraftRoot")?.as_str().map(|s| s.to_string()),
        _ => None,
    }
}

fn enumerate_curseforge_instances(launcher: &DetectedLauncher) -> Vec<ImportCandidate> {
    let read_dir = match std::fs::read_dir(&launcher.instances_dir) {
        Ok(rd) => rd,
        Err(_) => return Vec::new(),
    };

    let mut candidates: Vec<ImportCandidate> = Vec::new();
    for entry in read_dir {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let instance_dir = entry.path();
        if !instance_dir.is_dir() {
            continue;
        }
        let meta_path = instance_dir.join("minecraftinstance.json");
        if !meta_path.exists() {
            continue;
        }

        let folder_name = instance_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        let mut candidate = inspect_curseforge_instance(&folder_name, &instance_dir);
        candidate.launcher_installation_key = launcher.installation_key.clone();
        candidates.push(candidate);
    }
    candidates
}

fn inspect_curseforge_instance(folder_name: &str, instance_dir: &Path) -> ImportCandidate {
    let meta_path = instance_dir.join("minecraftinstance.json");
    let meta_text = match try_read_string(&meta_path) {
        Some(t) => t,
        None => {
            return unsupported_candidate(
                LauncherKind::CurseForge,
                folder_name,
                instance_dir,
                vec!["Cannot read minecraftinstance.json".to_string()],
            )
        }
    };

    let meta: serde_json::Value = match serde_json::from_str(&meta_text) {
        Ok(v) => v,
        Err(e) => {
            return unsupported_candidate(
                LauncherKind::CurseForge,
                folder_name,
                instance_dir,
                vec![format!("Failed to parse minecraftinstance.json: {e}")],
            )
        }
    };

    let display_name = meta
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or(folder_name)
        .to_string();

    let icon_path = meta
        .get("profileImagePath")
        .and_then(|v| v.as_str())
        .and_then(|icon_rel| {
            let icon_path = instance_dir.join(icon_rel);
            detect_icon(&icon_path)
        });

    let minecraft_version = meta
        .get("gameVersion")
        .and_then(|v| v.as_str())
        .or_else(|| {
            meta.get("minecraft")
                .and_then(|mc| mc.get("version"))
                .and_then(|v| v.as_str())
        })
        .map(|s| s.to_string());

    let (loader, loader_version) = parse_curseforge_loader(&meta);

    let loader_tuple = minecraft_version.as_ref().map(|mc_ver| LoaderTuple {
        loader: loader.clone().unwrap_or_else(|| "vanilla".to_string()),
        loader_version: loader_version.unwrap_or_default(),
        minecraft_version: mc_ver.clone(),
    });

    let payload_root = instance_dir.to_path_buf();
    let inventory = inventory_payload(&payload_root).unwrap_or_else(|_| ContentInventory {
        payload_root: payload_root.clone(),
        total_files: 0,
        total_bytes: 0,
        has_mods: false,
        has_resourcepacks: false,
        has_shaderpacks: false,
        has_datapacks: false,
        has_saves: false,
    });

    let (status, mut all_warnings) = if minecraft_version.is_some() {
        (CandidateStatus::Ready, Vec::new())
    } else {
        (
            CandidateStatus::Unsupported {
                reasons: vec!["Could not determine Minecraft version".to_string()],
            },
            Vec::new(),
        )
    };

    if inventory.total_files == 0 {
        all_warnings.push("Payload root is empty".to_string());
    }

    ImportCandidate {
        source_key: folder_name.to_string(),
        launcher: LauncherKind::CurseForge,
        launcher_installation_key: String::new(),
        display_name,
        icon_path,
        payload_root,
        inventory,
        loader_tuple,
        last_played: None,
        launch_strategy: LaunchStrategy::Normal,
        settings_preview: parse_curseforge_settings(&meta),
        status,
        warnings: all_warnings,
    }
}

fn parse_curseforge_settings(meta: &serde_json::Value) -> LaunchSettingsPreview {
    let memory_mb = [
        "/memorySettings/maximum",
        "/memorySettings/maxMemory",
        "/memorySettings/maximumMemory",
        "/allocatedMemory",
    ]
    .iter()
    .find_map(|pointer| meta.pointer(pointer).and_then(serde_json::Value::as_i64))
    .map(|value| {
        if value > 0 && value < 128 {
            value * 1024
        } else {
            value
        }
    });
    let java_path = meta
        .get("javaPath")
        .or_else(|| meta.pointer("/javaSettings/path"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let raw_arguments = meta.get("javaArgs").or_else(|| meta.get("jvmArgs"));
    let arguments: Vec<String> = match raw_arguments {
        Some(serde_json::Value::String(value)) => value
            .split_whitespace()
            .filter(|argument| safe_jvm_argument(argument))
            .map(str::to_owned)
            .collect(),
        Some(serde_json::Value::Array(values)) => values
            .iter()
            .filter_map(serde_json::Value::as_str)
            .filter(|argument| safe_jvm_argument(argument))
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    };
    LaunchSettingsPreview {
        memory_mb,
        java_path,
        jvm_args: arguments,
    }
}

/// Parse loader info from a CurseForge minecraftinstance.json.
fn parse_curseforge_loader(meta: &serde_json::Value) -> (Option<String>, Option<String>) {
    if let Some(bml) = meta.get("baseModLoader") {
        if let Some(name) = bml.get("name").and_then(|v| v.as_str()) {
            let version = bml
                .get("version")
                .or_else(|| bml.get("forgeVersion"))
                .and_then(|v| v.as_str())
                .map(str::to_owned);
            let (parsed_loader, parsed_version) = parse_cf_loader_string(name);
            return (parsed_loader, version.or(parsed_version));
        }
        if let Some(text) = bml.as_str() {
            return parse_cf_loader_string(text);
        }
    }

    if let Some(loaders) = meta
        .get("minecraft")
        .and_then(|mc| mc.get("modLoaders"))
        .and_then(|v| v.as_array())
    {
        for le in loaders {
            if let Some(lid) = le.get("id").and_then(|v| v.as_str()) {
                let primary = le.get("primary").and_then(|v| v.as_bool()).unwrap_or(true);
                if primary {
                    return parse_cf_modloader_id(lid);
                }
            }
        }
        if let Some(first) = loaders.first() {
            if let Some(lid) = first.get("id").and_then(|v| v.as_str()) {
                return parse_cf_modloader_id(lid);
            }
        }
    }

    if let Some(loader_str) = meta.get("loader").and_then(|v| v.as_str()) {
        return parse_cf_loader_string(loader_str);
    }
    (None, None)
}

fn normalize_cf_loader_name(name: &str) -> String {
    match name.to_ascii_lowercase().as_str() {
        "fabric" => "fabric".to_string(),
        "forge" => "forge".to_string(),
        "neoforge" => "neoforge".to_string(),
        "quilt" => "quilt".to_string(),
        _ => name.to_ascii_lowercase(),
    }
}

fn parse_cf_loader_string(text: &str) -> (Option<String>, Option<String>) {
    let text = text.trim();
    if text.is_empty() {
        return (None, None);
    }
    if let Some(dash_pos) = text.find('-') {
        let name = &text[..dash_pos];
        let version = &text[dash_pos + 1..];
        (
            Some(normalize_cf_loader_name(name)),
            Some(version.to_string()),
        )
    } else {
        (Some(normalize_cf_loader_name(text)), None)
    }
}

fn parse_cf_modloader_id(id: &str) -> (Option<String>, Option<String>) {
    if let Some(dash_pos) = id.find('-') {
        let name = &id[..dash_pos];
        let version = &id[dash_pos + 1..];
        (
            Some(normalize_cf_loader_name(name)),
            Some(version.to_string()),
        )
    } else {
        (Some(normalize_cf_loader_name(id)), None)
    }
}

// ---------------------------------------------------------------------------
// Modrinth App detection
// ---------------------------------------------------------------------------

fn detect_modrinth_installation(root: &Path) -> Option<DetectedLauncher> {
    let app_db_path = root.join("app.db");
    let profiles_dir = root.join("profiles");
    let has_current = app_db_path.is_file();
    let has_legacy = profiles_dir.is_dir();
    if !has_current && !has_legacy {
        return None;
    }

    let instance_count = if has_current {
        count_modrinth_db_instances(&app_db_path).unwrap_or(0)
    } else {
        count_modrinth_legacy_profiles(&profiles_dir)
    };

    Some(DetectedLauncher {
        installation_key: format!("modrinth:{}", root.to_string_lossy()),
        kind: LauncherKind::Modrinth,
        display_name: "Modrinth App".to_string(),
        config_root: root.to_path_buf(),
        instances_dir: profiles_dir,
        instance_count,
        detection_warnings: Vec::new(),
    })
}

fn count_modrinth_db_instances(db_path: &Path) -> Option<usize> {
    let conn =
        rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .ok()?;
    conn.execute_batch("PRAGMA query_only = 1").ok()?;
    conn.execute_batch("PRAGMA busy_timeout = 100").ok()?;

    let has_instances: bool = conn
        .prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1")
        .ok()
        .and_then(|mut s| s.query_row(["instances"], |_| Ok(true)).ok())
        .unwrap_or(false);
    if !has_instances {
        return None;
    }

    conn.query_row("SELECT COUNT(*) FROM instances", [], |row| row.get(0))
        .ok()
}

fn count_modrinth_legacy_profiles(profiles_dir: &Path) -> usize {
    let read_dir = match std::fs::read_dir(profiles_dir) {
        Ok(rd) => rd,
        Err(_) => return 0,
    };
    let mut count = 0usize;
    for entry in read_dir {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.path().is_dir() && entry.path().join("profile.json").exists() {
            count += 1;
        }
    }
    count
}

fn enumerate_modrinth_instances(launcher: &DetectedLauncher) -> Vec<ImportCandidate> {
    let app_db_path = launcher.config_root.join("app.db");
    let profiles_dir = launcher.config_root.join("profiles");
    let mut candidates: Vec<ImportCandidate> = Vec::new();

    if app_db_path.is_file() {
        if let Ok(conn) = rusqlite::Connection::open_with_flags(
            &app_db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) {
            let _ = conn.execute_batch("PRAGMA query_only = 1");
            let _ = conn.execute_batch("PRAGMA busy_timeout = 100");
            if let Ok(db_candidates) = enumerate_modrinth_db_instances(&conn, launcher) {
                candidates = db_candidates;
            }
        }
    }

    if profiles_dir.is_dir() {
        let db_keys: std::collections::HashSet<String> =
            candidates.iter().map(|c| c.source_key.clone()).collect();
        if let Ok(legacy) = enumerate_modrinth_legacy_profiles(&profiles_dir, launcher) {
            for cand in legacy {
                if !db_keys.contains(&cand.source_key) {
                    candidates.push(cand);
                }
            }
        }
    }
    candidates
}

struct ModrinthDbRow {
    id: String,
    name: String,
    inst_path: Option<String>,
    icon: Option<String>,
    game_version: Option<String>,
    loader: Option<String>,
    loader_version: Option<String>,
    modified: Option<String>,
    install_stage: Option<String>,
}

fn enumerate_modrinth_db_instances(
    conn: &rusqlite::Connection,
    launcher: &DetectedLauncher,
) -> Result<Vec<ImportCandidate>, String> {
    let tables: Vec<String> = {
        let mut stmt = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type='table' AND name IN (?1, ?2, ?3, ?4)",
            )
            .map_err(|e| format!("SQL error: {e}"))?;
        let rows = stmt
            .query_map(
                [
                    "instances",
                    "settings",
                    "instance_launch_overrides",
                    "instance_content_sets",
                ],
                |row| row.get::<_, String>(0),
            )
            .map_err(|e| format!("SQL error: {e}"))?;
        let mut names = Vec::new();
        for row in rows {
            names.push(row.map_err(|e| format!("SQL error: {e}"))?);
        }
        names
    };

    if !tables.contains(&"instances".to_string()) {
        return Err("app.db missing 'instances' table".to_string());
    }

    let columns: Vec<String> = {
        let mut stmt = conn
            .prepare("PRAGMA table_info(instances)")
            .map_err(|e| format!("PRAGMA error: {e}"))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| format!("PRAGMA error: {e}"))?;
        let mut names = Vec::new();
        for row in rows {
            names.push(row.map_err(|e| format!("PRAGMA error: {e}"))?);
        }
        names
    };

    let has_path = columns.contains(&"path".to_string());
    let has_current_content_sets = columns.contains(&"applied_content_set_id".to_string())
        && tables.contains(&"instance_content_sets".to_string());
    let has_icon =
        columns.contains(&"icon".to_string()) || columns.contains(&"icon_path".to_string());
    let has_game_version =
        columns.contains(&"game_version".to_string()) || has_current_content_sets;
    let has_loader = columns.contains(&"loader".to_string()) || has_current_content_sets;
    let has_loader_version =
        columns.contains(&"loader_version".to_string()) || has_current_content_sets;
    let has_modified =
        columns.contains(&"modified".to_string()) || columns.contains(&"last_played".to_string());
    let has_install_stage = columns.contains(&"install_stage".to_string());

    let custom_dir: Option<String> = if tables.contains(&"settings".to_string()) {
        let settings_columns: Vec<String> = conn
            .prepare("PRAGMA table_info(settings)")
            .ok()
            .and_then(|mut statement| {
                statement
                    .query_map([], |row| row.get::<_, String>(1))
                    .ok()
                    .map(|rows| rows.filter_map(Result::ok).collect())
            })
            .unwrap_or_default();
        if settings_columns.contains(&"custom_dir".to_string()) {
            conn.query_row("SELECT custom_dir FROM settings WHERE id = 0", [], |row| {
                row.get(0)
            })
            .ok()
        } else {
            conn.query_row(
                "SELECT value FROM settings WHERE key = ?1",
                ["custom_dir"],
                |row| row.get(0),
            )
            .ok()
        }
    } else {
        None
    };

    let profiles_base = custom_dir
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| launcher.config_root.clone());

    let sql = if has_current_content_sets {
        "SELECT i.id, i.name, i.path, i.icon_path,
                cs.game_version, cs.loader, cs.loader_version,
                CAST(i.last_played AS TEXT), i.install_stage
         FROM instances i
         LEFT JOIN instance_content_sets cs ON cs.id = i.applied_content_set_id
         ORDER BY i.name"
            .to_string()
    } else {
        let mut select_cols: Vec<&str> = vec!["id", "name"];
        if has_path {
            select_cols.push("path");
        }
        if has_icon {
            select_cols.push(if columns.contains(&"icon_path".to_string()) {
                "icon_path"
            } else {
                "icon"
            });
        }
        if has_game_version {
            select_cols.push("game_version");
        }
        if has_loader {
            select_cols.push("loader");
        }
        if has_loader_version {
            select_cols.push("loader_version");
        }
        if has_modified {
            select_cols.push(if columns.contains(&"last_played".to_string()) {
                "CAST(last_played AS TEXT)"
            } else {
                "modified"
            });
        }
        if has_install_stage {
            select_cols.push("install_stage");
        }
        format!(
            "SELECT {} FROM instances ORDER BY name",
            select_cols.join(", ")
        )
    };
    let mut stmt = conn.prepare(&sql).map_err(|e| format!("SQL error: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            let id: String = row.get(0)?;
            let name: String = row.get(1)?;
            let mut col = 2usize;
            let inst_path: Option<String> = if has_path {
                let v: Option<String> = row.get(col)?;
                col += 1;
                v
            } else {
                None
            };
            let icon: Option<String> = if has_icon {
                let v: Option<String> = row.get(col)?;
                col += 1;
                v
            } else {
                None
            };
            let game_version: Option<String> = if has_game_version {
                let v: Option<String> = row.get(col)?;
                col += 1;
                v
            } else {
                None
            };
            let loader: Option<String> = if has_loader {
                let v: Option<String> = row.get(col)?;
                col += 1;
                v
            } else {
                None
            };
            let loader_version: Option<String> = if has_loader_version {
                let v: Option<String> = row.get(col)?;
                col += 1;
                v
            } else {
                None
            };
            let modified: Option<String> = if has_modified {
                let value = row.get(col)?;
                col += 1;
                value
            } else {
                None
            };
            let install_stage: Option<String> = if has_install_stage {
                row.get(col)?
            } else {
                None
            };
            Ok(ModrinthDbRow {
                id,
                name,
                inst_path,
                icon,
                game_version,
                loader,
                loader_version,
                modified,
                install_stage,
            })
        })
        .map_err(|e| format!("SQL error: {e}"))?;

    let mut candidates: Vec<ImportCandidate> = Vec::new();
    for row_res in rows {
        let row = match row_res {
            Ok(r) => r,
            Err(_) => continue,
        };

        let payload_base = if let Some(ref rel_path) = row.inst_path {
            profiles_base.join("profiles").join(rel_path)
        } else {
            launcher.config_root.join("profiles").join(&row.id)
        };

        let payload_root = if payload_base.is_dir() {
            payload_base
        } else {
            let alt = launcher.config_root.join("profiles").join(&row.id);
            if alt.is_dir() {
                alt
            } else {
                candidates.push(unsupported_candidate(
                    LauncherKind::Modrinth,
                    &row.id,
                    &payload_base,
                    vec![format!("Profile directory not found")],
                ));
                continue;
            }
        };

        let icon_path = row.icon.as_ref().and_then(|icon_val| {
            let icon_path = PathBuf::from(icon_val);
            if icon_path.is_absolute() {
                detect_icon(&icon_path)
            } else {
                detect_icon(&payload_root.join(&icon_path))
            }
        });

        let loader_tuple = row.game_version.as_ref().map(|mc_ver| LoaderTuple {
            loader: normalize_cf_loader_name(
                &row.loader.clone().unwrap_or_else(|| "vanilla".to_string()),
            ),
            loader_version: row.loader_version.clone().unwrap_or_default(),
            minecraft_version: mc_ver.clone(),
        });

        let inventory = inventory_payload(&payload_root).unwrap_or_else(|_| ContentInventory {
            payload_root: payload_root.clone(),
            total_files: 0,
            total_bytes: 0,
            has_mods: false,
            has_resourcepacks: false,
            has_shaderpacks: false,
            has_datapacks: false,
            has_saves: false,
        });

        let settings_preview = if tables.contains(&"instance_launch_overrides".to_string()) {
            read_modrinth_launch_overrides(conn, &row.id)
                .unwrap_or_else(|| read_modrinth_global_launch_settings(conn))
        } else {
            read_modrinth_global_launch_settings(conn)
        };

        let (status, mut all_warnings) = if row
            .install_stage
            .as_deref()
            .is_some_and(|stage| stage != "installed")
        {
            (
                CandidateStatus::Unsupported {
                    reasons: vec![format!(
                        "Modrinth profile is not fully installed ({})",
                        row.install_stage.as_deref().unwrap_or("unknown")
                    )],
                },
                Vec::new(),
            )
        } else if loader_tuple.is_some() {
            (CandidateStatus::Ready, Vec::new())
        } else {
            (
                CandidateStatus::NeedsReview,
                vec!["No Minecraft version or loader detected".to_string()],
            )
        };

        if inventory.total_files == 0 {
            all_warnings.push("Payload root is empty".to_string());
        }

        candidates.push(ImportCandidate {
            source_key: row.id,
            launcher: LauncherKind::Modrinth,
            launcher_installation_key: launcher.installation_key.clone(),
            display_name: row.name,
            icon_path,
            payload_root,
            inventory,
            loader_tuple,
            last_played: row.modified,
            launch_strategy: LaunchStrategy::Normal,
            settings_preview,
            status,
            warnings: all_warnings,
        });
    }
    Ok(candidates)
}

fn read_modrinth_launch_overrides(
    conn: &rusqlite::Connection,
    instance_id: &str,
) -> Option<LaunchSettingsPreview> {
    let columns: Vec<String> = {
        let mut stmt = conn
            .prepare("PRAGMA table_info(instance_launch_overrides)")
            .ok()?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(1)).ok()?;
        let mut names = Vec::new();
        for row in rows {
            names.push(row.ok()?);
        }
        names
    };

    if columns.contains(&"overrides".to_string()) {
        let mut preview = read_modrinth_global_launch_settings(conn);
        let json: Option<String> = conn
            .query_row(
                "SELECT CAST(overrides AS TEXT) FROM instance_launch_overrides WHERE instance_id = ?1",
                [instance_id],
                |row| row.get(0),
            )
            .ok();
        if let Some(json) = json {
            let value: serde_json::Value = serde_json::from_str(&json).ok()?;
            if let Some(memory) = value
                .get("memory")
                .and_then(|memory| memory.get("maximum"))
                .and_then(serde_json::Value::as_i64)
            {
                preview.memory_mb = Some(memory);
            }
            if let Some(java_path) = value.get("java_path").and_then(serde_json::Value::as_str) {
                preview.java_path = Some(java_path.to_owned());
            }
            if let Some(arguments) = value
                .get("extra_launch_args")
                .and_then(serde_json::Value::as_array)
            {
                preview.jvm_args = arguments
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .filter(|argument| safe_jvm_argument(argument))
                    .map(str::to_owned)
                    .collect();
            }
        }
        return Some(preview);
    }

    let memory_col = if columns.contains(&"memory".to_string()) {
        "memory"
    } else if columns.contains(&"max_memory".to_string()) {
        "max_memory"
    } else {
        ""
    };
    let has_memory = !memory_col.is_empty();
    let has_java = columns.contains(&"java_path".to_string());

    if !has_memory && !has_java {
        return None;
    }

    let mut select = Vec::new();
    if has_memory {
        select.push(memory_col);
    }
    if has_java {
        select.push("java_path");
    }

    let sql = format!(
        "SELECT {} FROM instance_launch_overrides WHERE instance_id = ?1 LIMIT 1",
        select.join(", ")
    );
    let mut stmt = conn.prepare(&sql).ok()?;

    let mut s = stmt.query([instance_id]).ok()?;
    let r = s.next().ok()??;

    let memory_mb: Option<i64> = if has_memory { r.get(0).ok() } else { None };
    let java_path: Option<String> = if has_java {
        r.get(if has_memory { 1 } else { 0 }).ok()
    } else {
        None
    };

    Some(LaunchSettingsPreview {
        memory_mb,
        java_path,
        jvm_args: Vec::new(),
    })
}

fn read_modrinth_global_launch_settings(conn: &rusqlite::Connection) -> LaunchSettingsPreview {
    let columns: Vec<String> = conn
        .prepare("PRAGMA table_info(settings)")
        .ok()
        .and_then(|mut statement| {
            statement
                .query_map([], |row| row.get::<_, String>(1))
                .ok()
                .map(|rows| rows.filter_map(Result::ok).collect())
        })
        .unwrap_or_default();
    let memory_mb = if columns.contains(&"mc_memory_max".to_string()) {
        conn.query_row(
            "SELECT mc_memory_max FROM settings WHERE id = 0",
            [],
            |row| row.get(0),
        )
        .ok()
    } else {
        None
    };
    let jvm_args = if columns.contains(&"extra_launch_args".to_string()) {
        conn.query_row(
            "SELECT CAST(extra_launch_args AS TEXT) FROM settings WHERE id = 0",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .and_then(|json| serde_json::from_str::<Vec<String>>(&json).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|argument| safe_jvm_argument(argument))
        .collect()
    } else {
        Vec::new()
    };
    LaunchSettingsPreview {
        memory_mb,
        java_path: None,
        jvm_args,
    }
}

fn safe_jvm_argument(argument: &str) -> bool {
    !argument.starts_with("-Xmx")
        && !argument.starts_with("-Xms")
        && !argument.starts_with("-cp")
        && !argument.starts_with("-classpath")
        && !argument.starts_with("-agentpath")
        && !argument.starts_with("-agentlib")
        && !argument.starts_with("-javaagent")
        && !argument.starts_with("-Djava.library.path")
}

#[derive(Deserialize)]
struct LegacyProfile {
    #[serde(default)]
    name: Option<String>,
    #[serde(default, alias = "gameVersion")]
    game_version: Option<String>,
    #[serde(default)]
    loader: Option<LegacyLoaderField>,
    #[serde(default)]
    loader_version: Option<LegacyLoaderVersion>,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    created: Option<String>,
    #[serde(default)]
    #[serde(alias = "date_modified")]
    modified: Option<String>,
    #[serde(default)]
    last_played: Option<String>,
}

#[derive(Deserialize)]
struct LegacyLoader {
    #[serde(rename = "type")]
    loader_type: Option<String>,
    #[serde(default)]
    version: Option<String>,
}

#[derive(Deserialize)]
struct LegacyLoaderVersion {
    id: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum LegacyLoaderField {
    Object(LegacyLoader),
    Name(String),
}

fn enumerate_modrinth_legacy_profiles(
    profiles_dir: &Path,
    launcher: &DetectedLauncher,
) -> Result<Vec<ImportCandidate>, String> {
    let read_dir =
        std::fs::read_dir(profiles_dir).map_err(|e| format!("Cannot read profiles dir: {e}"))?;
    let mut candidates: Vec<ImportCandidate> = Vec::new();

    for entry in read_dir {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            continue;
        }

        let profile_path = entry.path().join("profile.json");
        if !profile_path.is_file() {
            continue;
        }

        let profile: LegacyProfile = match try_parse_json(&profile_path) {
            Some(p) => p,
            None => continue,
        };

        let folder_name = entry
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        let payload_root = entry.path();
        let display_name = profile.name.unwrap_or_else(|| folder_name.clone());

        let icon_path = profile.icon.as_ref().and_then(|icon_val| {
            let p = PathBuf::from(icon_val);
            if p.is_absolute() {
                detect_icon(&p)
            } else {
                detect_icon(&payload_root.join(&p))
            }
        });

        let (loader_name, loader_ver) = if let Some(loader) = profile.loader {
            match loader {
                LegacyLoaderField::Object(loader) => (
                    Some(
                        loader
                            .loader_type
                            .map(|value| normalize_cf_loader_name(&value))
                            .unwrap_or_else(|| "unknown".to_string()),
                    ),
                    loader.version.unwrap_or_default(),
                ),
                LegacyLoaderField::Name(loader) => (
                    Some(normalize_cf_loader_name(&loader)),
                    profile
                        .loader_version
                        .as_ref()
                        .map(|version| version.id.clone())
                        .unwrap_or_default(),
                ),
            }
        } else {
            (None, String::new())
        };

        let loader_tuple = profile.game_version.as_ref().map(|mc_ver| LoaderTuple {
            loader: loader_name.clone().unwrap_or_else(|| "vanilla".to_string()),
            loader_version: loader_ver.clone(),
            minecraft_version: mc_ver.clone(),
        });

        let inventory = inventory_payload(&payload_root).unwrap_or_else(|_| ContentInventory {
            payload_root: payload_root.clone(),
            total_files: 0,
            total_bytes: 0,
            has_mods: false,
            has_resourcepacks: false,
            has_shaderpacks: false,
            has_datapacks: false,
            has_saves: false,
        });

        let (status, mut all_warnings) = if loader_tuple.is_some() {
            (CandidateStatus::Ready, Vec::new())
        } else {
            (
                CandidateStatus::NeedsReview,
                vec!["Incomplete loader metadata in legacy profile.json".to_string()],
            )
        };
        if inventory.total_files == 0 {
            all_warnings.push("Payload root is empty".to_string());
        }

        candidates.push(ImportCandidate {
            source_key: folder_name,
            launcher: LauncherKind::Modrinth,
            launcher_installation_key: launcher.installation_key.clone(),
            display_name,
            icon_path,
            payload_root,
            inventory,
            loader_tuple,
            last_played: profile.last_played.or(profile.modified).or(profile.created),
            launch_strategy: LaunchStrategy::Normal,
            settings_preview: LaunchSettingsPreview {
                memory_mb: None,
                java_path: None,
                jvm_args: Vec::new(),
            },
            status,
            warnings: all_warnings,
        });
    }
    Ok(candidates)
}

fn unsupported_candidate(
    launcher: LauncherKind,
    folder_name: &str,
    instance_dir: &Path,
    reasons: Vec<String>,
) -> ImportCandidate {
    ImportCandidate {
        source_key: folder_name.to_string(),
        launcher,
        launcher_installation_key: String::new(),
        display_name: folder_name.to_string(),
        icon_path: None,
        payload_root: instance_dir.to_path_buf(),
        inventory: ContentInventory {
            payload_root: instance_dir.to_path_buf(),
            total_files: 0,
            total_bytes: 0,
            has_mods: false,
            has_resourcepacks: false,
            has_shaderpacks: false,
            has_datapacks: false,
            has_saves: false,
        },
        loader_tuple: None,
        last_played: None,
        launch_strategy: LaunchStrategy::Normal,
        settings_preview: LaunchSettingsPreview {
            memory_mb: None,
            java_path: None,
            jvm_args: Vec::new(),
        },
        status: CandidateStatus::Unsupported { reasons },
        warnings: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Public discovery functions
// ---------------------------------------------------------------------------

/// Discover Prism Launcher instances.
pub fn discover_prism_launcher(custom_root: Option<&Path>) -> LauncherDiscovery {
    for root in prism_candidate_roots(custom_root) {
        if !root.join("prismlauncher.cfg").is_file() {
            continue;
        }
        let mut launcher = match detect_prism_installation(&root) {
            Some(l) => l,
            None => continue,
        };
        let mut candidates = enumerate_prism_instances(&launcher);
        launcher.instance_count = candidates.len();
        for c in &mut candidates {
            c.launcher_installation_key = launcher.installation_key.clone();
        }
        return LauncherDiscovery {
            launcher: Some(launcher),
            candidates,
        };
    }
    LauncherDiscovery::empty()
}

/// Discover CurseForge instances.
pub fn discover_curseforge_launcher(custom_root: Option<&Path>) -> LauncherDiscovery {
    for root in curseforge_candidate_roots(custom_root) {
        let mut launcher = match detect_curseforge_installation(&root) {
            Some(l) => l,
            None => continue,
        };
        let mut candidates = enumerate_curseforge_instances(&launcher);
        launcher.instance_count = candidates.len();
        for c in &mut candidates {
            c.launcher_installation_key = launcher.installation_key.clone();
        }
        return LauncherDiscovery {
            launcher: Some(launcher),
            candidates,
        };
    }
    LauncherDiscovery::empty()
}

/// Discover Modrinth App instances.
pub fn discover_modrinth_launcher(custom_root: Option<&Path>) -> LauncherDiscovery {
    for root in modrinth_candidate_roots(custom_root) {
        let mut launcher = match detect_modrinth_installation(&root) {
            Some(l) => l,
            None => continue,
        };
        let mut candidates = enumerate_modrinth_instances(&launcher);
        launcher.instance_count = candidates.len();
        for c in &mut candidates {
            c.launcher_installation_key = launcher.installation_key.clone();
        }
        return LauncherDiscovery {
            launcher: Some(launcher),
            candidates,
        };
    }
    LauncherDiscovery::empty()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --------------------------------------------------------------
    // INI parser
    // --------------------------------------------------------------

    #[test]
    fn test_parse_ini_basic() {
        let ini =
            "[General]\nInstanceDir=instances\n[Settings]\nMaxMem=4096\nJavaPath=/usr/bin/java\n";
        let parsed = parse_ini(ini);
        assert_eq!(
            parsed.get("General").unwrap().get("instancedir").unwrap(),
            "instances"
        );
        assert_eq!(
            parsed.get("Settings").unwrap().get("maxmem").unwrap(),
            "4096"
        );
    }

    #[test]
    fn test_parse_ini_case_insensitive() {
        let ini = "[Minecraft]\nName=My Instance\nIconFile=icon.png\n";
        let parsed = parse_ini(ini);
        assert_eq!(
            parsed.get("Minecraft").unwrap().get("name").unwrap(),
            "My Instance"
        );
        assert_eq!(
            parsed.get("Minecraft").unwrap().get("iconfile").unwrap(),
            "icon.png"
        );
    }

    #[test]
    fn test_parse_ini_comments() {
        let ini = "; comment\n# another\n[Sec]\nkey=val\n";
        let parsed = parse_ini(ini);
        assert_eq!(parsed.get("Sec").unwrap().get("key").unwrap(), "val");
    }

    #[test]
    fn test_parse_ini_value_with_equals() {
        let ini = "[Minecraft]\nargs=-Xmx2G -Dfoo=bar\n";
        let parsed = parse_ini(ini);
        assert_eq!(
            parsed.get("Minecraft").unwrap().get("args").unwrap(),
            "-Xmx2G -Dfoo=bar"
        );
    }

    // --------------------------------------------------------------
    // Content inventory
    // --------------------------------------------------------------

    #[test]
    fn test_inventory_empty_directory() {
        let dir = tempfile::tempdir().unwrap();
        let result = inventory_payload(dir.path()).unwrap();
        assert_eq!(result.total_files, 0);
        assert!(!result.has_mods);
    }

    #[test]
    fn test_inventory_counts_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("mod.jar"), "data").unwrap();
        std::fs::create_dir_all(dir.path().join("mods")).unwrap();
        std::fs::write(dir.path().join("mods").join("test.jar"), "data").unwrap();
        let result = inventory_payload(dir.path()).unwrap();
        assert_eq!(result.total_files, 2);
        assert!(result.has_mods);
    }

    #[test]
    fn test_inventory_detects_content_subdirs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("resourcepacks")).unwrap();
        std::fs::create_dir_all(dir.path().join("shaderpacks")).unwrap();
        std::fs::create_dir_all(dir.path().join("datapacks")).unwrap();
        std::fs::create_dir_all(dir.path().join("saves")).unwrap();
        let result = inventory_payload(dir.path()).unwrap();
        assert!(result.has_resourcepacks);
        assert!(result.has_shaderpacks);
        assert!(result.has_datapacks);
        assert!(result.has_saves);
    }

    #[test]
    fn test_inventory_missing_root_fails() {
        let dir = tempfile::tempdir().unwrap();
        assert!(inventory_payload(&dir.path().join("nope")).is_err());
    }

    // --------------------------------------------------------------
    // Memory value parser
    // --------------------------------------------------------------

    #[test]
    fn test_parse_memory_value_mb() {
        assert_eq!(parse_memory_value("2048M"), Some(2048));
        assert_eq!(parse_memory_value("2048"), Some(2048));
    }

    #[test]
    fn test_parse_memory_value_gb() {
        assert_eq!(parse_memory_value("2G"), Some(2048));
        assert_eq!(parse_memory_value("4g"), Some(4096));
    }

    #[test]
    fn test_parse_memory_value_invalid() {
        assert_eq!(parse_memory_value(""), None);
        assert_eq!(parse_memory_value("abc"), None);
    }

    // --------------------------------------------------------------
    // Prism fixture tests
    // --------------------------------------------------------------

    fn create_prism_cfg(data_dir: &Path, instance_dir: &str) {
        let cfg = format!("[General]\nInstanceDir={}\n", instance_dir);
        std::fs::write(data_dir.join("prismlauncher.cfg"), cfg).unwrap();
    }

    fn create_prism_instance(
        instances_dir: &Path,
        name: &str,
        instance_cfg: &str,
        pack_json: &str,
        payload_sub: Option<&str>,
    ) -> PathBuf {
        let inst_dir = instances_dir.join(name);
        std::fs::create_dir_all(&inst_dir).unwrap();
        std::fs::write(inst_dir.join("instance.cfg"), instance_cfg).unwrap();
        std::fs::write(inst_dir.join("mmc-pack.json"), pack_json).unwrap();
        if let Some(sub) = payload_sub {
            let p = inst_dir.join(sub);
            std::fs::create_dir_all(p.join("mods")).unwrap();
            std::fs::write(p.join("mods").join("test.jar"), "test").unwrap();
        }
        inst_dir
    }

    #[test]
    fn test_discover_prism_custom_root() {
        let root = tempfile::tempdir().unwrap();
        let pd = root.path().join("PrismLauncher");
        std::fs::create_dir_all(&pd).unwrap();
        create_prism_cfg(&pd, "instances");

        let inst_dir = pd.join("instances");
        create_prism_instance(
            &inst_dir,
            "MyInstance",
            "[Minecraft]\nname=Test Instance\niconFile=icon.png\n",
            r#"{"components":[{"uid":"net.minecraft","version":"1.21"},{"uid":"net.fabricmc.fabric-loader","version":"0.16.0"}]}"#,
            Some("minecraft"),
        );
        // Create an icon file in the instance dir
        std::fs::write(inst_dir.join("MyInstance").join("icon.png"), "PNG").unwrap();

        let result = discover_prism_launcher(Some(&pd));
        assert!(result.launcher.is_some());
        assert_eq!(result.launcher.unwrap().kind, LauncherKind::Prism);
        assert_eq!(result.candidates.len(), 1);
        let c = &result.candidates[0];
        assert_eq!(c.display_name, "Test Instance");
        assert!(c.icon_path.is_some());
        let lt = c.loader_tuple.as_ref().unwrap();
        assert_eq!(lt.loader, "fabric");
        assert_eq!(lt.loader_version, "0.16.0");
        assert_eq!(lt.minecraft_version, "1.21");
    }

    #[test]
    fn test_prism_legacy_dot_minecraft() {
        let root = tempfile::tempdir().unwrap();
        let pd = root.path().join("PrismLauncher");
        std::fs::create_dir_all(&pd).unwrap();
        create_prism_cfg(&pd, "instances");

        let inst_dir = pd.join("instances");
        let _inst = create_prism_instance(
            &inst_dir,
            "Legacy",
            "[Minecraft]\nname=Legacy Instance\n",
            r#"{"components":[{"uid":"net.minecraft","version":"1.12.2"},{"uid":"net.minecraftforge","version":"14.23.5.2860"}]}"#,
            Some(".minecraft"),
        );
        // Create .minecraft payload (the helper creates it inside .minecraft)
        // .minecraft is already created by create_prism_instance with Some(".minecraft")

        let result = discover_prism_launcher(Some(&pd));
        assert_eq!(result.candidates.len(), 1);
        let c = &result.candidates[0];
        assert!(c.payload_root.ends_with(".minecraft"));
        let lt = c.loader_tuple.as_ref().unwrap();
        assert_eq!(lt.loader, "forge");
        assert_eq!(lt.minecraft_version, "1.12.2");
    }

    #[test]
    fn test_prism_no_payload_root() {
        let root = tempfile::tempdir().unwrap();
        let pd = root.path().join("PrismLauncher");
        std::fs::create_dir_all(&pd).unwrap();
        create_prism_cfg(&pd, "instances");
        create_prism_instance(
            &pd.join("instances"),
            "Broken",
            "[Minecraft]\nname=Broken\n",
            r#"{"components":[{"uid":"net.minecraft","version":"1.21"}]}"#,
            None,
        );
        let result = discover_prism_launcher(Some(&pd));
        assert_eq!(result.candidates.len(), 1);
        assert!(matches!(
            result.candidates[0].status,
            CandidateStatus::Unsupported { .. }
        ));
    }

    #[test]
    fn test_prism_unsupported_important_component() {
        let root = tempfile::tempdir().unwrap();
        let pd = root.path().join("PrismLauncher");
        std::fs::create_dir_all(&pd).unwrap();
        create_prism_cfg(&pd, "instances");
        create_prism_instance(
            &pd.join("instances"),
            "Custom",
            "[Minecraft]\nname=Custom\n",
            r#"{"components":[{"uid":"net.minecraft","version":"1.21"},{"uid":"net.fabricmc.fabric-loader","version":"0.16.0"},{"uid":"com.example.custompatch","version":"1.0","cachedName":"Custom Patch","cachedImportant":true}]}"#,
            Some("minecraft"),
        );
        let result = discover_prism_launcher(Some(&pd));
        assert_eq!(result.candidates.len(), 1);
        let c = &result.candidates[0];
        assert!(matches!(c.status, CandidateStatus::Unsupported { .. }));
        assert!(c.warnings.iter().any(|w| w.contains("important")));
    }

    #[test]
    fn test_prism_quilt_loader() {
        let root = tempfile::tempdir().unwrap();
        let pd = root.path().join("PrismLauncher");
        std::fs::create_dir_all(&pd).unwrap();
        create_prism_cfg(&pd, "instances");
        create_prism_instance(
            &pd.join("instances"),
            "Quilt",
            "[Minecraft]\nname=Quilt\n",
            r#"{"components":[{"uid":"net.minecraft","version":"1.20.1"},{"uid":"org.quiltmc.quilt-loader","version":"0.25.0"}]}"#,
            Some("minecraft"),
        );
        let result = discover_prism_launcher(Some(&pd));
        let lt = result.candidates[0].loader_tuple.as_ref().unwrap();
        assert_eq!(lt.loader, "quilt");
    }

    #[test]
    fn test_prism_neoforge_loader() {
        let root = tempfile::tempdir().unwrap();
        let pd = root.path().join("PrismLauncher");
        std::fs::create_dir_all(&pd).unwrap();
        create_prism_cfg(&pd, "instances");
        create_prism_instance(
            &pd.join("instances"),
            "Neo",
            "[Minecraft]\nname=Neo\n",
            r#"{"components":[{"uid":"net.minecraft","version":"1.21"},{"uid":"net.neoforged","version":"21.0.0-beta"}]}"#,
            Some("minecraft"),
        );
        let result = discover_prism_launcher(Some(&pd));
        let lt = result.candidates[0].loader_tuple.as_ref().unwrap();
        assert_eq!(lt.loader, "neoforge");
    }

    #[test]
    fn test_prism_vanilla_no_loader() {
        let root = tempfile::tempdir().unwrap();
        let pd = root.path().join("PrismLauncher");
        std::fs::create_dir_all(&pd).unwrap();
        create_prism_cfg(&pd, "instances");
        create_prism_instance(
            &pd.join("instances"),
            "Vanilla",
            "[Minecraft]\nname=Vanilla\n",
            r#"{"components":[{"uid":"net.minecraft","version":"1.21"}]}"#,
            Some("minecraft"),
        );
        let result = discover_prism_launcher(Some(&pd));
        let lt = result.candidates[0].loader_tuple.as_ref().unwrap();
        assert_eq!(lt.loader, "vanilla");
    }

    #[test]
    fn test_prism_no_instance_cfg_skipped() {
        let root = tempfile::tempdir().unwrap();
        let pd = root.path().join("PrismLauncher");
        std::fs::create_dir_all(&pd).unwrap();
        create_prism_cfg(&pd, "instances");
        let skip_dir = pd.join("instances").join("NoCfg");
        std::fs::create_dir_all(&skip_dir).unwrap();
        // No instance.cfg
        let result = discover_prism_launcher(Some(&pd));
        assert_eq!(result.candidates.len(), 0);
    }

    #[test]
    fn test_prism_no_launcher_found() {
        let result =
            discover_prism_launcher(Some(Path::new("C:\\nonexistent_agora_test_path_xyz")));
        assert!(result.launcher.is_none());
        assert!(result.candidates.is_empty());
    }

    #[test]
    fn test_prism_disabled_component_ignored() {
        let root = tempfile::tempdir().unwrap();
        let pd = root.path().join("PrismLauncher");
        std::fs::create_dir_all(&pd).unwrap();
        create_prism_cfg(&pd, "instances");
        create_prism_instance(
            &pd.join("instances"),
            "Disabled",
            "[Minecraft]\nname=Disabled\n",
            r#"{"components":[{"uid":"net.minecraft","version":"1.21"},{"uid":"net.fabricmc.fabric-loader","version":"0.16.0"},{"uid":"net.minecraftforge","version":"52.0.0","cachedEnabled":false}]}"#,
            Some("minecraft"),
        );
        let result = discover_prism_launcher(Some(&pd));
        let lt = result.candidates[0].loader_tuple.as_ref().unwrap();
        assert_eq!(lt.loader, "fabric");
    }

    // --------------------------------------------------------------
    // Prism settings tests
    // --------------------------------------------------------------

    #[test]
    fn test_prism_settings_memory_from_maxmem() {
        let parsed = parse_ini("[Settings]\nMaxMem=4096\n");
        let settings = parse_prism_settings(&parsed);
        assert_eq!(settings.memory_mb, Some(4096));
    }

    #[test]
    fn test_prism_flat_instance_settings() {
        let parsed = parse_ini(
            "name=Flat Instance\nMaxMemAlloc=6144\nJavaPath=C:/Java/bin/java.exe\nJvmArgs=-Xmx6G -XX:+UseG1GC\n",
        );
        let settings = parse_prism_settings(&parsed);
        assert_eq!(settings.memory_mb, Some(6144));
        assert_eq!(settings.java_path.as_deref(), Some("C:/Java/bin/java.exe"));
        assert_eq!(settings.jvm_args, vec!["-XX:+UseG1GC"]);
    }

    #[test]
    fn test_prism_settings_memory_from_jvmargs() {
        let parsed = parse_ini("[Settings]\nJvmArgs=-Xmx4G -XX:+UseG1GC\n");
        let settings = parse_prism_settings(&parsed);
        assert_eq!(settings.memory_mb, Some(4096));
    }

    #[test]
    fn test_prism_settings_jvmargs_filtered() {
        let parsed = parse_ini("[Settings]\nJvmArgs=-Xmx2G -Xms1G -XX:+UseG1GC -agentpath:/bad -cp:foo -javaagent:bad.jar\n");
        let settings = parse_prism_settings(&parsed);
        assert!(!settings.jvm_args.iter().any(|a| a.starts_with("-Xmx")));
        assert!(!settings.jvm_args.iter().any(|a| a.starts_with("-Xms")));
        assert!(!settings
            .jvm_args
            .iter()
            .any(|a| a.starts_with("-agentpath")));
        assert!(!settings.jvm_args.iter().any(|a| a.starts_with("-cp")));
        assert!(!settings
            .jvm_args
            .iter()
            .any(|a| a.starts_with("-javaagent:")));
        assert!(settings.jvm_args.iter().any(|a| a == "-XX:+UseG1GC"));
    }

    // --------------------------------------------------------------
    // CurseForge fixture tests
    // --------------------------------------------------------------

    fn setup_cf_fixture(root: &Path, storage_mc_root: &str) -> PathBuf {
        let cf_root = root.join("CurseForge");
        std::fs::create_dir_all(&cf_root).unwrap();
        let nested = serde_json::json!({"minecraftRoot": storage_mc_root});
        let storage = serde_json::json!({"minecraft-settings": nested.to_string()});
        std::fs::write(
            cf_root.join("storage.json"),
            serde_json::to_string_pretty(&storage).unwrap(),
        )
        .unwrap();
        cf_root
    }

    fn setup_cf_instance(instances_dir: &Path, name: &str, meta: &serde_json::Value) {
        let inst_dir = instances_dir.join(name);
        std::fs::create_dir_all(inst_dir.join("mods")).unwrap();
        std::fs::write(
            inst_dir.join("minecraftinstance.json"),
            serde_json::to_string_pretty(meta).unwrap(),
        )
        .unwrap();
        std::fs::write(inst_dir.join("mods").join("mod.jar"), "mod").unwrap();
    }

    #[test]
    fn test_discover_curseforge_custom_root() {
        let root = tempfile::tempdir().unwrap();
        let mc_root = root.path().join("MC");
        let cf_root = setup_cf_fixture(root.path(), &mc_root.to_string_lossy());
        std::fs::create_dir_all(mc_root.join("Instances")).unwrap();

        let meta = serde_json::json!({
            "name": "My Pack",
            "gameVersion": "1.20.1",
            "baseModLoader": {"name": "Forge", "version": "47.2.0"},
            "profileImagePath": "icon.png",
        });
        setup_cf_instance(&mc_root.join("Instances"), "MyPack", &meta);
        std::fs::write(
            mc_root.join("Instances").join("MyPack").join("icon.png"),
            "PNG",
        )
        .unwrap();

        let result = discover_curseforge_launcher(Some(&cf_root));
        assert!(result.launcher.is_some());
        assert_eq!(result.candidates.len(), 1);
        let c = &result.candidates[0];
        assert_eq!(c.display_name, "My Pack");
        assert!(c.icon_path.is_some());
        let lt = c.loader_tuple.as_ref().unwrap();
        assert_eq!(lt.loader, "forge");
        assert_eq!(lt.loader_version, "47.2.0");
        assert_eq!(lt.minecraft_version, "1.20.1");
    }

    #[test]
    fn test_curseforge_fabric() {
        let root = tempfile::tempdir().unwrap();
        let mc_root = root.path().join("MC");
        let cf_root = setup_cf_fixture(root.path(), &mc_root.to_string_lossy());
        std::fs::create_dir_all(mc_root.join("Instances")).unwrap();

        let meta = serde_json::json!({
            "name": "Fabric Pack",
            "gameVersion": "1.21",
            "baseModLoader": {"name": "Fabric", "version": "0.16.0"},
        });
        setup_cf_instance(&mc_root.join("Instances"), "FabricPack", &meta);

        let result = discover_curseforge_launcher(Some(&cf_root));
        let lt = result.candidates[0].loader_tuple.as_ref().unwrap();
        assert_eq!(lt.loader, "fabric");
        assert_eq!(lt.loader_version, "0.16.0");
    }

    #[test]
    fn test_curseforge_realistic_base_loader_shape() {
        let metadata = serde_json::json!({
            "gameVersion": "26.2",
            "baseModLoader": {
                "name": "fabric-0.18.6-26.2",
                "forgeVersion": "0.18.6"
            }
        });
        let (loader, version) = parse_curseforge_loader(&metadata);
        assert_eq!(loader.as_deref(), Some("fabric"));
        assert_eq!(version.as_deref(), Some("0.18.6"));
    }

    #[test]
    fn test_curseforge_neoforge_string_loader() {
        let root = tempfile::tempdir().unwrap();
        let mc_root = root.path().join("MC");
        let cf_root = setup_cf_fixture(root.path(), &mc_root.to_string_lossy());
        std::fs::create_dir_all(mc_root.join("Instances")).unwrap();

        let meta = serde_json::json!({
            "name": "Neo Pack",
            "gameVersion": "1.21",
            "baseModLoader": "neoforge-21.0.0",
        });
        setup_cf_instance(&mc_root.join("Instances"), "NeoPack", &meta);

        let result = discover_curseforge_launcher(Some(&cf_root));
        let lt = result.candidates[0].loader_tuple.as_ref().unwrap();
        assert_eq!(lt.loader, "neoforge");
    }

    #[test]
    fn test_curseforge_vanilla() {
        let root = tempfile::tempdir().unwrap();
        let mc_root = root.path().join("MC");
        let cf_root = setup_cf_fixture(root.path(), &mc_root.to_string_lossy());
        std::fs::create_dir_all(mc_root.join("Instances")).unwrap();

        let meta = serde_json::json!({
            "name": "Vanilla",
            "gameVersion": "1.21",
        });
        setup_cf_instance(&mc_root.join("Instances"), "Vanilla", &meta);

        let result = discover_curseforge_launcher(Some(&cf_root));
        let c = &result.candidates[0];
        assert!(matches!(c.status, CandidateStatus::Ready));
        let lt = c.loader_tuple.as_ref().unwrap();
        assert_eq!(lt.loader, "vanilla");
    }

    #[test]
    fn test_curseforge_no_game_version() {
        let root = tempfile::tempdir().unwrap();
        let mc_root = root.path().join("MC");
        let cf_root = setup_cf_fixture(root.path(), &mc_root.to_string_lossy());
        std::fs::create_dir_all(mc_root.join("Instances")).unwrap();

        let meta = serde_json::json!({"name": "Broken"});
        setup_cf_instance(&mc_root.join("Instances"), "Broken", &meta);

        let result = discover_curseforge_launcher(Some(&cf_root));
        let c = &result.candidates[0];
        assert!(matches!(c.status, CandidateStatus::Unsupported { .. }));
    }

    #[test]
    fn test_curseforge_no_launcher_found() {
        let result =
            discover_curseforge_launcher(Some(Path::new("C:\\nonexistent_cf_test_path_xyz")));
        assert!(result.launcher.is_none());
    }

    // --------------------------------------------------------------
    // Modrinth fixture tests
    // --------------------------------------------------------------

    #[test]
    fn test_modrinth_legacy_profile() {
        let root = tempfile::tempdir().unwrap();
        let mr_root = root.path().join("ModrinthApp");
        let profiles_dir = mr_root.join("profiles").join("MyProfile");
        std::fs::create_dir_all(&profiles_dir).unwrap();

        let profile = serde_json::json!({
            "name": "My Modrinth Profile",
            "gameVersion": "1.21",
            "loader": {"type": "fabric", "version": "0.16.0"},
            "icon": "icon.png",
            "modified": "2026-07-24T12:00:00Z",
        });
        std::fs::write(
            profiles_dir.join("profile.json"),
            serde_json::to_string_pretty(&profile).unwrap(),
        )
        .unwrap();
        std::fs::write(profiles_dir.join("icon.png"), "PNG").unwrap();
        std::fs::create_dir_all(profiles_dir.join("mods")).unwrap();
        std::fs::write(profiles_dir.join("mods").join("test.jar"), "test").unwrap();

        let result = discover_modrinth_launcher(Some(&mr_root));
        assert!(result.launcher.is_some());
        assert_eq!(result.candidates.len(), 1);
        let c = &result.candidates[0];
        assert_eq!(c.display_name, "My Modrinth Profile");
        assert_eq!(c.source_key, "MyProfile");
        assert!(c.icon_path.is_some());
        let lt = c.loader_tuple.as_ref().unwrap();
        assert_eq!(lt.loader, "fabric");
        assert_eq!(lt.minecraft_version, "1.21");
        assert_eq!(c.last_played.as_deref(), Some("2026-07-24T12:00:00Z"));
    }

    #[test]
    fn test_modrinth_legacy_no_loader_still_detected() {
        let root = tempfile::tempdir().unwrap();
        let mr_root = root.path().join("ModrinthApp");
        let profiles_dir = mr_root.join("profiles").join("Vanilla");
        std::fs::create_dir_all(&profiles_dir).unwrap();

        let profile = serde_json::json!({
            "name": "Vanilla Profile",
            "gameVersion": "1.21",
        });
        std::fs::write(
            profiles_dir.join("profile.json"),
            serde_json::to_string_pretty(&profile).unwrap(),
        )
        .unwrap();
        std::fs::create_dir_all(profiles_dir.join("mods")).unwrap();
        std::fs::write(profiles_dir.join("mods").join("test.jar"), "test").unwrap();

        let result = discover_modrinth_launcher(Some(&mr_root));
        let c = &result.candidates[0];
        let lt = c.loader_tuple.as_ref().unwrap();
        assert_eq!(lt.loader, "vanilla");
    }

    #[test]
    fn test_modrinth_legacy_no_game_version() {
        let root = tempfile::tempdir().unwrap();
        let mr_root = root.path().join("ModrinthApp");
        let profiles_dir = mr_root.join("profiles").join("Incomplete");
        std::fs::create_dir_all(&profiles_dir).unwrap();

        let profile = serde_json::json!({"name": "Incomplete Profile"});
        std::fs::write(
            profiles_dir.join("profile.json"),
            serde_json::to_string_pretty(&profile).unwrap(),
        )
        .unwrap();

        let result = discover_modrinth_launcher(Some(&mr_root));
        let c = &result.candidates[0];
        assert!(matches!(c.status, CandidateStatus::NeedsReview));
    }

    #[test]
    fn test_modrinth_db_based() {
        let root = tempfile::tempdir().unwrap();
        let mr_root = root.path().join("ModrinthApp");
        std::fs::create_dir_all(&mr_root).unwrap();

        // Create an in-memory SQLite database and save it.
        let conn = rusqlite::Connection::open(mr_root.join("app.db")).unwrap();
        conn.execute_batch(
            "
            CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT);
            INSERT INTO settings VALUES ('custom_dir', '');
            CREATE TABLE instances (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                path TEXT,
                icon TEXT,
                game_version TEXT,
                loader TEXT,
                loader_version TEXT,
                modified TEXT
            );
            INSERT INTO instances VALUES (
                'uuid-1234', 'DB Instance', 'dbprofile', 'icon.png',
                '1.21', 'fabric', '0.16.0', '2026-07-24T12:00:00Z'
            );
        ",
        )
        .unwrap();

        let profiles_dir = mr_root.join("profiles").join("dbprofile");
        std::fs::create_dir_all(profiles_dir.join("mods")).unwrap();
        std::fs::write(profiles_dir.join("mods").join("mod.jar"), "mod").unwrap();
        std::fs::write(
            mr_root.join("profiles").join("dbprofile").join("icon.png"),
            "PNG",
        )
        .unwrap();

        let result = discover_modrinth_launcher(Some(&mr_root));
        assert!(result.launcher.is_some());
        assert_eq!(result.candidates.len(), 1);
        let c = &result.candidates[0];
        assert_eq!(c.display_name, "DB Instance");
        assert!(c.icon_path.is_some());
        let lt = c.loader_tuple.as_ref().unwrap();
        assert_eq!(lt.loader, "fabric");
        assert_eq!(lt.loader_version, "0.16.0");
        assert_eq!(lt.minecraft_version, "1.21");
        assert_eq!(c.last_played.as_deref(), Some("2026-07-24T12:00:00Z"));
    }

    #[test]
    fn test_modrinth_db_with_launch_overrides() {
        let root = tempfile::tempdir().unwrap();
        let mr_root = root.path().join("ModrinthApp");
        std::fs::create_dir_all(&mr_root).unwrap();

        let conn = rusqlite::Connection::open(mr_root.join("app.db")).unwrap();
        conn.execute_batch("
            CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT);
            CREATE TABLE instances (
                id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT,
                game_version TEXT, loader TEXT, loader_version TEXT
            );
            INSERT INTO instances VALUES ('uuid-1', 'Overrides', 'prof', '1.21', 'neoforge', '21.0.0');
            CREATE TABLE instance_launch_overrides (
                instance_id TEXT PRIMARY KEY, memory INTEGER, java_path TEXT
            );
            INSERT INTO instance_launch_overrides VALUES ('uuid-1', 8192, '/custom/java');
        ").unwrap();

        let profiles_dir = mr_root.join("profiles").join("prof");
        std::fs::create_dir_all(profiles_dir.join("mods")).unwrap();
        std::fs::write(profiles_dir.join("mods").join("mod.jar"), "mod").unwrap();

        let result = discover_modrinth_launcher(Some(&mr_root));
        assert_eq!(result.candidates.len(), 1);
        let c = &result.candidates[0];
        assert_eq!(c.settings_preview.memory_mb, Some(8192));
        assert_eq!(
            c.settings_preview.java_path.as_deref(),
            Some("/custom/java")
        );
        let lt = c.loader_tuple.as_ref().unwrap();
        assert_eq!(lt.loader, "neoforge");
    }

    #[test]
    fn test_modrinth_db_custom_dir() {
        let root = tempfile::tempdir().unwrap();
        let mr_root = root.path().join("ModrinthApp");
        std::fs::create_dir_all(&mr_root).unwrap();

        let custom_base = root.path().join("CustomProfiles");
        std::fs::create_dir_all(custom_base.join("profiles").join("myinst").join("mods")).unwrap();
        std::fs::write(
            custom_base
                .join("profiles")
                .join("myinst")
                .join("mods")
                .join("mod.jar"),
            "mod",
        )
        .unwrap();

        let conn = rusqlite::Connection::open(mr_root.join("app.db")).unwrap();
        conn.execute_batch(&format!("
            CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT);
            INSERT INTO settings VALUES ('custom_dir', '{}');
            CREATE TABLE instances (
                id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT,
                game_version TEXT, loader TEXT, loader_version TEXT
            );
            INSERT INTO instances VALUES ('inst-1', 'Custom Dir Inst', 'myinst', '1.21', 'forge', '52.0.0');
        ", custom_base.to_string_lossy().replace("\\", "\\\\"))).unwrap();

        let result = discover_modrinth_launcher(Some(&mr_root));
        assert_eq!(result.candidates.len(), 1);
        let c = &result.candidates[0];
        assert_eq!(c.display_name, "Custom Dir Inst");
        let lt = c.loader_tuple.as_ref().unwrap();
        assert_eq!(lt.loader, "forge");
    }

    #[test]
    fn test_modrinth_current_content_set_schema() {
        let root = tempfile::tempdir().unwrap();
        let mr_root = root.path().join("ModrinthApp");
        let profile = mr_root.join("profiles").join("current-profile");
        std::fs::create_dir_all(profile.join("mods")).unwrap();
        std::fs::write(profile.join("mods").join("current.jar"), b"mod").unwrap();

        let conn = rusqlite::Connection::open(mr_root.join("app.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE settings (id INTEGER PRIMARY KEY, custom_dir TEXT);
             INSERT INTO settings VALUES (0, NULL);
             CREATE TABLE instances (
                 id TEXT PRIMARY KEY, path TEXT NOT NULL, applied_content_set_id TEXT,
                 install_stage TEXT NOT NULL, name TEXT NOT NULL, icon_path TEXT,
                 created INTEGER NOT NULL, modified INTEGER NOT NULL, last_played INTEGER
             );
             CREATE TABLE instance_content_sets (
                 id TEXT PRIMARY KEY, instance_id TEXT NOT NULL, game_version TEXT NOT NULL,
                 loader TEXT NOT NULL, loader_version TEXT
             );
             CREATE TABLE instance_launch_overrides (
                 instance_id TEXT PRIMARY KEY, overrides TEXT NOT NULL
             );
             INSERT INTO instances VALUES (
                 'instance-1', 'current-profile', 'set-1', 'installed',
                 'Current Profile', NULL, 1, 2, 3
             );
             INSERT INTO instance_content_sets VALUES (
                 'set-1', 'instance-1', '1.21.1', 'fabric', '0.16.9'
             );
             INSERT INTO instance_launch_overrides VALUES (
                 'instance-1', '{\"memory\":{\"maximum\":7168},\"extra_launch_args\":[\"-XX:+UseG1GC\",\"-javaagent:bad.jar\"]}'
             );",
        )
        .unwrap();
        drop(conn);

        let discovery = discover_modrinth_launcher(Some(&mr_root));
        assert_eq!(discovery.candidates.len(), 1);
        let candidate = &discovery.candidates[0];
        assert_eq!(candidate.display_name, "Current Profile");
        let loader = candidate.loader_tuple.as_ref().unwrap();
        assert_eq!(loader.minecraft_version, "1.21.1");
        assert_eq!(loader.loader, "fabric");
        assert_eq!(loader.loader_version, "0.16.9");
        assert_eq!(candidate.settings_preview.memory_mb, Some(7168));
        assert_eq!(candidate.settings_preview.jvm_args, vec!["-XX:+UseG1GC"]);
        assert!(matches!(candidate.status, CandidateStatus::Ready));
    }

    #[test]
    fn test_modrinth_no_launcher_found() {
        let result =
            discover_modrinth_launcher(Some(Path::new("C:\\nonexistent_mr_test_path_xyz")));
        assert!(result.launcher.is_none());
    }

    // --------------------------------------------------------------
    // Serialization roundtrip tests
    // --------------------------------------------------------------

    #[test]
    fn test_types_serialize() {
        let launcher = DetectedLauncher {
            installation_key: "test".into(),
            kind: LauncherKind::Prism,
            display_name: "Prism".into(),
            config_root: PathBuf::from("/tmp"),
            instances_dir: PathBuf::from("/tmp/instances"),
            instance_count: 1,
            detection_warnings: vec![],
        };
        let json = serde_json::to_string(&launcher).unwrap();
        assert!(json.contains("prism"));
        assert!(json.contains("installation_key"));

        let candidate = ImportCandidate {
            source_key: "key".into(),
            launcher: LauncherKind::CurseForge,
            launcher_installation_key: "test".into(),
            display_name: "CF Instance".into(),
            icon_path: None,
            payload_root: PathBuf::from("/tmp/payload"),
            inventory: ContentInventory {
                payload_root: PathBuf::from("/tmp/payload"),
                total_files: 42,
                total_bytes: 1000,
                has_mods: true,
                has_resourcepacks: false,
                has_shaderpacks: false,
                has_datapacks: false,
                has_saves: false,
            },
            loader_tuple: Some(LoaderTuple {
                loader: "forge".into(),
                loader_version: "52.0.0".into(),
                minecraft_version: "1.21".into(),
            }),
            last_played: Some("2026-01-01T00:00:00Z".into()),
            launch_strategy: LaunchStrategy::Normal,
            settings_preview: LaunchSettingsPreview {
                memory_mb: Some(4096),
                java_path: None,
                jvm_args: vec!["-XX:+UseG1GC".into()],
            },
            status: CandidateStatus::Ready,
            warnings: vec![],
        };
        let json = serde_json::to_string(&candidate).unwrap();
        assert!(json.contains("forge"));
        assert!(json.contains("CF Instance"));

        let deserialized: ImportCandidate = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.source_key, "key");
        assert_eq!(deserialized.loader_tuple.unwrap().loader, "forge");
    }

    #[test]
    fn test_unsupported_serialize() {
        let cand = unsupported_candidate(
            LauncherKind::Prism,
            "broken",
            Path::new("/tmp/broken"),
            vec!["Missing cfg".into()],
        );
        let json = serde_json::to_string(&cand).unwrap();
        assert!(json.contains("unsupported"));
        assert!(json.contains("Missing cfg"));
    }

    #[test]
    fn test_loader_for_uid() {
        assert_eq!(
            loader_for_uid("net.fabricmc.fabric-loader").unwrap().0,
            "fabric"
        );
        assert_eq!(
            loader_for_uid("org.quiltmc.quilt-loader").unwrap().0,
            "quilt"
        );
        assert_eq!(loader_for_uid("net.minecraftforge").unwrap().0, "forge");
        assert_eq!(loader_for_uid("net.neoforged").unwrap().0, "neoforge");
        assert_eq!(
            loader_for_uid("net.neoforged.neoforge").unwrap().0,
            "neoforge"
        );
        assert!(loader_for_uid("com.example.unknown").is_none());
    }

    #[test]
    fn test_parse_storage_json_nested() {
        let dir = tempfile::tempdir().unwrap();
        let nested = serde_json::json!({"minecraftRoot": "C:\\MC\\Root"});
        let storage = serde_json::json!({"minecraft-settings": nested.to_string()});
        let path = dir.path().join("storage.json");
        std::fs::write(&path, serde_json::to_string_pretty(&storage).unwrap()).unwrap();
        let result = parse_storage_json_minecraft_root(&path);
        assert_eq!(result, Some("C:\\MC\\Root".to_string()));
    }

    #[test]
    fn test_parse_storage_json_direct_object() {
        let dir = tempfile::tempdir().unwrap();
        let storage = serde_json::json!({"minecraft-settings": {"minecraftRoot": "/mc/root"}});
        let path = dir.path().join("storage.json");
        std::fs::write(&path, serde_json::to_string_pretty(&storage).unwrap()).unwrap();
        let result = parse_storage_json_minecraft_root(&path);
        assert_eq!(result, Some("/mc/root".to_string()));
    }
}
