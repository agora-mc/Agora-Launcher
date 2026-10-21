use agora_core::error::{LauncherError, LauncherResult};
use serde::Serialize;
use std::io::Read;
use std::path::Path;

/// Hard limits for zip extraction (§7.2.1).
pub const MAX_ZIP_SIZE: u64 = 500 * 1024 * 1024; // 500MB compressed
const MAX_EXTRACTED_SIZE: u64 = 2 * 1024 * 1024 * 1024; // 2GB total extracted
const MAX_FILE_COUNT: usize = 5000; // 5000 files max

/// Directory whitelist (§7.2.2). Only files under these prefixes are extracted.
///
/// # `kubejs/` is deliberately allowed, and is NOT inert
///
/// Every other prefix here holds data the game reads. `kubejs/` does not: the
/// KubeJS mod *executes* the `.js` files it finds there, and those scripts can
/// reach Java classes directly — they are not sandboxed the way browser
/// JavaScript is. A pack that ships `kubejs/startup_scripts/x.js` is shipping
/// code that runs on the player's machine with the game's own permissions.
///
/// This is an accepted trade-off, not an oversight: KubeJS scripting is a
/// normal part of how packs are built, and packs are reviewed by hand before
/// they enter the registry. **Curation is the control for this prefix, not
/// [`BANNED_EXTENSIONS`].** Do not reason about `kubejs/` as though the
/// extension ban made overrides safe, and do not widen either list on the
/// assumption that it did.
///
/// `scripts/` is CraftTweaker's equivalent of `kubejs/` and carries the same
/// caveat. `global_packs/`, `openloader/` and `patchouli_books/` are data the
/// game (or a data-loading mod) reads.
pub(crate) const ALLOWED_PREFIXES: &[&str] = &[
    "config/",
    "defaultconfigs/",
    "resourcepacks/",
    "shaderpacks/",
    "datapacks/",
    "kubejs/",
    "scripts/",
    "global_packs/",
    "openloader/",
    "patchouli_books/",
];

/// Banned extensions (§7.2.2). Hard-banned even inside whitelisted directories.
///
/// This list stops an overrides bundle from dropping a binary or a shell script
/// where a config file belongs. It does not, and cannot, make overrides
/// generally inert — see the `kubejs/` note on [`ALLOWED_PREFIXES`].
const BANNED_EXTENSIONS: &[&str] = &[
    ".jar", ".class", ".exe", ".bat", ".cmd", ".sh", ".ps1", ".dll", ".so", ".dylib", ".msi",
    ".dmg",
];

/// Instance files Agora owns. A pack never gets to overwrite these, in any mode.
const AGORA_OWNED_FILES: &[&str] = &["instance_manifest.json"];

/// How much of an overrides archive is accepted.
///
/// `Standard` is the whitelist above. `Permissive` is what the user opts into
/// with **Reduced security mode**: any path inside the instance, `.jar` files
/// included (plenty of packs ship mods in their overrides), because the user
/// chose to trust the pack. Some things stay refused in both modes, since no
/// pack needs them and the downside is large: path traversal, native
/// executables and scripts (Minecraft never runs them, so they are only useful
/// to an attacker), and files Agora itself keeps in the instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverridePolicy {
    #[default]
    Standard,
    Permissive,
}

impl OverridePolicy {
    /// The policy the user's settings ask for.
    pub fn from_settings(ctx: &agora_core::ctx::Ctx) -> Self {
        if agora_core::settings::reduced_security_enabled(ctx) {
            Self::Permissive
        } else {
            Self::Standard
        }
    }

    /// Whether a sanitised, instance-relative path is extracted at all.
    /// Paths it declines are skipped, not treated as an attack.
    pub fn admits(self, path: &str) -> bool {
        let path = as_created(path);
        if is_agora_owned(path) {
            return false;
        }
        match self {
            Self::Standard => is_whitelisted(path),
            Self::Permissive => true,
        }
    }

    /// Whether a path that [`admits`](Self::admits) accepted is still refused
    /// outright, failing the extraction.
    pub fn forbids(self, path: &str) -> bool {
        let path = as_created(path);
        match self {
            Self::Standard => has_banned_extension(path),
            Self::Permissive => {
                let lower = path.to_ascii_lowercase();
                agora_plugin_api::provider::BANNED_EXTENSIONS
                    .iter()
                    .any(|ext| lower.ends_with(ext))
            }
        }
    }
}

/// The name a path gets on disk. Windows drops trailing dots and spaces
/// when it creates a file, so `config/run.bat.` is written as
/// `config/run.bat`; every check has to look at that name, not the raw one.
fn as_created(path: &str) -> &str {
    path.trim_end_matches(['.', ' '])
}

fn is_agora_owned(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    AGORA_OWNED_FILES.contains(&lower.as_str())
        || lower
            .split('/')
            .any(|segment| segment.starts_with(".agora"))
}

/// Result of an override extraction.
#[derive(Debug, Clone, Serialize)]
pub struct ExtractionResult {
    pub extracted: Vec<String>,
    pub skipped: Vec<String>,
    pub total_bytes_written: u64,
}

/// Extract a zip file into an instance directory with full sanitization.
///
/// This is the main entry point for §7.2. It:
/// 1. Checks compressed size against MAX_ZIP_SIZE.
/// 2. Pre-scans all entries for total uncompressed size and file count.
/// 3. Validates each entry path against the directory whitelist.
/// 4. Rejects banned extensions even within whitelisted directories.
/// 5. Prevents Zip Slip (path traversal) by rejecting `..` and absolute paths.
/// 6. Tracks actual bytes written and aborts mid-stream if limits are exceeded.
/// 7. On any security violation, deletes partially extracted files.
pub fn extract_overrides(zip_path: &Path, dest_dir: &Path) -> LauncherResult<ExtractionResult> {
    extract_overrides_with(zip_path, dest_dir, OverridePolicy::Standard)
}

/// [`extract_overrides`] under an explicit [`OverridePolicy`].
pub fn extract_overrides_with(
    zip_path: &Path,
    dest_dir: &Path,
    policy: OverridePolicy,
) -> LauncherResult<ExtractionResult> {
    // Pre-check: compressed file size.
    let zip_size = zip_path
        .metadata()
        .map_err(|_| LauncherError::Generic {
            code: "ERR_OVERRIDE_FAILED".to_string(),
            message: "Could not read zip file metadata.".to_string(),
        })?
        .len();

    if zip_size > MAX_ZIP_SIZE {
        return Err(LauncherError::Generic {
            code: "ERR_ZIP_TOO_LARGE".to_string(),
            message: format!(
                "Zip file is {}MB, exceeds the {}MB limit.",
                zip_size / (1024 * 1024),
                MAX_ZIP_SIZE / (1024 * 1024)
            ),
        });
    }

    let file = std::fs::File::open(zip_path).map_err(|_| LauncherError::Generic {
        code: "ERR_OVERRIDE_FAILED".to_string(),
        message: "Could not open zip file.".to_string(),
    })?;

    let mut archive = zip::ZipArchive::new(file).map_err(|_| LauncherError::Generic {
        code: "ERR_OVERRIDE_FAILED".to_string(),
        message: "Invalid or corrupt zip file.".to_string(),
    })?;

    // Phase 1: Pre-scan all entries for size and count limits.
    let mut total_uncompressed: u64 = 0;
    let mut entry_count: usize = 0;

    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|_| LauncherError::Generic {
            code: "ERR_OVERRIDE_FAILED".to_string(),
            message: "Could not read zip entry.".to_string(),
        })?;

        total_uncompressed = total_uncompressed.saturating_add(entry.size());
        entry_count += 1;

        if total_uncompressed > MAX_EXTRACTED_SIZE {
            return Err(LauncherError::Generic {
                code: "ERR_ZIP_BOMB".to_string(),
                message: format!(
                    "Total uncompressed size exceeds the {}GB limit. Possible zip bomb.",
                    MAX_EXTRACTED_SIZE / (1024 * 1024 * 1024)
                ),
            });
        }

        if entry_count > MAX_FILE_COUNT {
            return Err(LauncherError::Generic {
                code: "ERR_TOO_MANY_FILES".to_string(),
                message: format!(
                    "Zip contains more than {} files. Limit exceeded.",
                    MAX_FILE_COUNT
                ),
            });
        }
    }

    // Phase 2: Extract with path validation.
    let mut extracted: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut bytes_written: u64 = 0;

    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|_| LauncherError::Generic {
            code: "ERR_OVERRIDE_FAILED".to_string(),
            message: "Could not read zip entry during extraction.".to_string(),
        })?;

        let raw_name = entry.name().to_string();

        // Sanitize the path: reject absolute paths and parent traversal.
        let safe_name = match sanitize_path(&raw_name) {
            Some(name) => name,
            None => {
                // Path traversal attempt — abort entire extraction.
                cleanup_partial(dest_dir, &extracted);
                return Err(LauncherError::Generic {
                    code: "ERR_ZIP_SLIP".to_string(),
                    message: format!(
                        "Path traversal detected in zip entry: '{}'. Extraction aborted.",
                        raw_name
                    ),
                });
            }
        };

        // Skip directories (they'll be created by their files).
        if entry.is_dir() {
            continue;
        }

        // Check directory whitelist.
        if !policy.admits(&safe_name) {
            skipped.push(safe_name.clone());
            continue;
        }

        // Check banned extensions.
        if policy.forbids(&safe_name) {
            cleanup_partial(dest_dir, &extracted);
            return Err(LauncherError::Generic {
                code: "ERR_SECURITY_VIOLATION".to_string(),
                message: format!(
                    "Security Violation: Pack overrides cannot contain executable files or mods. \
                     Banned file type detected: '{}'. \
                     All mods must be routed through the platform manifest.",
                    safe_name
                ),
            });
        }

        // Build the destination path and verify it's inside the sandbox.
        let dest_path = dest_dir.join(&safe_name);
        if !dest_path.starts_with(dest_dir) {
            cleanup_partial(dest_dir, &extracted);
            return Err(LauncherError::Generic {
                code: "ERR_ZIP_SLIP".to_string(),
                message: format!(
                    "Resolved path escapes the instance directory: '{}'. Extraction aborted.",
                    raw_name
                ),
            });
        }

        // Create parent directories.
        if let Some(parent) = dest_path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| LauncherError::Generic {
                code: "ERR_OVERRIDE_FAILED".to_string(),
                message: "Could not create directory for extracted file.".to_string(),
            })?;
        }

        // Write the file, tracking actual bytes.
        //
        // The Phase 1 prescan sums `entry.size()`, which is the *declared*
        // uncompressed size from the central directory — attacker-controlled
        // metadata. A single entry can declare a few KB and inflate to
        // gigabytes, so an unbounded `read_to_end` here would exhaust memory
        // before the running-total check below ever runs. Cap the read at the
        // remaining budget (plus one byte, so an over-long stream is detected
        // rather than silently truncated).
        let remaining_budget = MAX_EXTRACTED_SIZE.saturating_sub(bytes_written);
        let mut file_data = Vec::new();
        entry
            .take(remaining_budget.saturating_add(1))
            .read_to_end(&mut file_data)
            .map_err(|_| LauncherError::Generic {
                code: "ERR_OVERRIDE_FAILED".to_string(),
                message: "Could not read file data from zip.".to_string(),
            })?;

        let file_len = file_data.len() as u64;

        // Mid-stream check: abort if actual bytes exceed limit.
        if file_len > remaining_budget {
            cleanup_partial(dest_dir, &extracted);
            return Err(LauncherError::Generic {
                code: "ERR_ZIP_BOMB".to_string(),
                message: "Actual extracted size exceeds the 2GB limit. Aborting.".to_string(),
            });
        }
        bytes_written = bytes_written.saturating_add(file_len);

        std::fs::write(&dest_path, &file_data).map_err(|_| LauncherError::Generic {
            code: "ERR_OVERRIDE_FAILED".to_string(),
            message: format!("Could not write extracted file: '{}'.", safe_name),
        })?;

        extracted.push(safe_name);
    }

    Ok(ExtractionResult {
        extracted,
        skipped,
        total_bytes_written: bytes_written,
    })
}

/// Strip absolute paths and `../` sequences. Returns None if the path is
/// purely traversal (no valid path remains).
fn sanitize_path(raw: &str) -> Option<String> {
    // Replace backslashes with forward slashes for Windows compatibility.
    let normalized = raw.replace('\\', "/");

    // Reject absolute paths (Unix and Windows drive letters).
    if normalized.starts_with('/') || normalized.matches(':').count() > 0 {
        return None;
    }

    // Split on '/' and rebuild, rejecting any '..' component.
    let mut parts: Vec<&str> = Vec::new();
    for part in normalized.split('/') {
        if part == ".." {
            return None; // Zip Slip attempt
        }
        if part == "." || part.is_empty() {
            continue;
        }
        parts.push(part);
    }

    if parts.is_empty() {
        return None;
    }

    Some(parts.join("/"))
}

/// Check if a path starts with one of the whitelisted directory prefixes.
fn is_whitelisted(path: &str) -> bool {
    ALLOWED_PREFIXES
        .iter()
        .any(|prefix| path.starts_with(prefix))
}

/// Check if a filename has a banned extension.
fn has_banned_extension(path: &str) -> bool {
    let lower = as_created(path).to_lowercase();
    BANNED_EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
}

/// Delete partially extracted files on security violation or error.
fn cleanup_partial(dest_dir: &Path, extracted: &[String]) {
    for file in extracted {
        let path = dest_dir.join(file);
        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_path_rejects_traversal() {
        assert!(sanitize_path("../../evil.exe").is_none());
        assert!(sanitize_path("config/../../evil.exe").is_none());
        assert!(sanitize_path("/etc/passwd").is_none());
        assert!(sanitize_path("C:/windows/system32/evil.dll").is_none());
    }

    #[test]
    fn test_sanitize_path_normalizes_backslashes() {
        assert_eq!(
            sanitize_path("config\\mod\\settings.toml").unwrap(),
            "config/mod/settings.toml"
        );
    }

    #[test]
    fn test_sanitize_path_strips_dot_segments() {
        assert_eq!(
            sanitize_path("./config/./mod.toml").unwrap(),
            "config/mod.toml"
        );
    }

    #[test]
    fn test_whitelist_allows_config() {
        assert!(is_whitelisted("config/mod.toml"));
        assert!(is_whitelisted("defaultconfigs/server.toml"));
        assert!(is_whitelisted("resourcepacks/mypack.zip"));
        assert!(is_whitelisted("kubejs/server_scripts/script.js"));
    }

    #[test]
    fn test_whitelist_rejects_mods() {
        assert!(!is_whitelisted("mods/evil.jar"));
        assert!(!is_whitelisted("saves/world/level.dat"));
        assert!(!is_whitelisted("README.txt"));
    }

    #[test]
    fn test_banned_extensions() {
        assert!(has_banned_extension("config/setup.exe"));
        assert!(has_banned_extension("config/lib.dll"));
        assert!(has_banned_extension("kubejs/evil.sh"));
        assert!(!has_banned_extension("config/mod.toml"));
        assert!(!has_banned_extension("kubejs/script.js"));
    }

    #[test]
    fn test_whitelist_allows_shaderpacks() {
        assert!(is_whitelisted("shaderpacks/ComplementaryShaders.zip"));
        assert!(is_whitelisted("datapacks/custom_loot.zip"));
    }

    #[test]
    fn permissive_overrides_take_jars_and_any_folder_but_never_executables() {
        let permissive = OverridePolicy::Permissive;
        assert!(permissive.admits("mods/extra.jar"));
        assert!(!permissive.forbids("mods/extra.jar"));
        assert!(permissive.admits("options.txt"));
        assert!(permissive.admits("saves/Tutorial/level.dat"));
        assert!(permissive.forbids("bin/run.sh"));
        assert!(permissive.forbids("natives/lib.dll"));
        assert!(!permissive.admits("instance_manifest.json"));
        assert!(!permissive.admits(".agora/state.json"));
        // Windows would create these without the trailing dot or space.
        assert!(permissive.forbids("config/run.bat."));
        assert!(permissive.forbids("config/run.bat. ."));
        assert!(!permissive.admits("instance_manifest.json."));
        assert!(OverridePolicy::Standard.forbids("config/x.jar."));

        let standard = OverridePolicy::Standard;
        assert!(!standard.admits("mods/extra.jar"));
        assert!(standard.forbids("config/extra.jar"));
        assert!(standard.admits("scripts/recipes.zs"));
        assert!(standard.admits("global_packs/required_data/pack.zip"));
    }

    #[test]
    fn permissive_extraction_writes_mods_and_still_refuses_executables() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("o.zip");
        let write_zip = |entries: &[&str]| {
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
            for name in entries {
                zip.start_file(*name, zip::write::FileOptions::default())
                    .unwrap();
                zip.write_all(b"x").unwrap();
            }
            zip.finish().unwrap();
        };
        let dest = dir.path().join("instance");
        std::fs::create_dir_all(&dest).unwrap();

        write_zip(&["mods/a.jar", "options.txt", "instance_manifest.json"]);
        let standard = extract_overrides(&zip_path, &dest).unwrap();
        assert!(standard.extracted.is_empty());
        let permissive =
            extract_overrides_with(&zip_path, &dest, OverridePolicy::Permissive).unwrap();
        assert_eq!(permissive.extracted, vec!["mods/a.jar", "options.txt"]);
        assert_eq!(permissive.skipped, vec!["instance_manifest.json"]);

        write_zip(&["tools/setup.exe"]);
        let error =
            extract_overrides_with(&zip_path, &dest, OverridePolicy::Permissive).unwrap_err();
        assert!(error.to_string().contains("setup.exe"), "{error}");
    }

    #[test]
    fn test_whitelist_rejects_shaderpacks_jar() {
        // .jar in shaderpacks is still banned — it should go through mods/
        assert!(has_banned_extension("shaderpacks/evil.jar"));
        // .zip is fine
        assert!(!has_banned_extension(
            "shaderpacks/ComplementaryShaders.zip"
        ));
    }
}
