use crate::error::LauncherResult;

use std::path::Path;

// Re-export core crash types and scoring functions.
pub use agora_core::crash_service::{
    compute_mod_score, continue_investigation, parse_crash_log, CrashFingerprint,
    InvestigationResult, SuggestedAction, SuspectScore,
};

// ---------------------------------------------------------------------------
// 1. JAR package parsing & metadata extraction
// ---------------------------------------------------------------------------

/// Open a .jar file as a zip archive and extract Java package directories
/// from `.class` entry paths.
///
/// An entry like `me/jellysquid/nautilus/Foo.class` yields the package
/// `me.jellysquid.nautilus` — the full directory path with the filename
/// stripped, segments joined by `.`. A minimum of 2 directory segments before
/// the filename is required to avoid noise from single-segment paths like
/// `Foo.class` at the zip root.
///
/// On ANY error (not a zip, io failure, etc.), returns `vec![]`. Never panics.
pub fn parse_jar_packages(jar_path: &Path) -> Vec<String> {
    agora_game_minecraft::jar_metadata::parse_jar_metadata(jar_path).java_packages
}

// ---------------------------------------------------------------------------
// 2. Thin AppHandle -> Ctx wrappers for telemetry and mod operations
// ---------------------------------------------------------------------------

/// Record a crash event in the local state database.
pub fn record_crash_event<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    instance_id: &str,
    fingerprint: &CrashFingerprint,
    mod_ids: &[String],
    signature_name: Option<&str>,
) -> LauncherResult<i64> {
    let ctx = crate::core_context(app)?;
    agora_core::crash_service::CrashService::new(ctx).record_crash_event(
        instance_id,
        fingerprint,
        mod_ids,
        signature_name,
    )
}

/// Record that the instance survived a launch with the given mods installed.
pub fn record_survival<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    instance_id: &str,
    mod_ids: &[String],
) -> LauncherResult<()> {
    let ctx = crate::core_context(app)?;
    agora_core::crash_service::CrashService::new(ctx).record_survival(instance_id, mod_ids)
}

/// Increment the confirmation count for a mod_id matching a fingerprint.
pub fn confirm_attribution<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    fingerprint: &CrashFingerprint,
    mod_id: &str,
) -> LauncherResult<()> {
    let ctx = crate::core_context(app)?;
    agora_core::crash_service::CrashService::new(ctx).confirm_attribution(fingerprint, mod_id)
}

/// Mark a mod as ruled out for a given fingerprint.
pub fn rule_out<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    fingerprint: &CrashFingerprint,
    mod_id: &str,
) -> LauncherResult<()> {
    let ctx = crate::core_context(app)?;
    agora_core::crash_service::CrashService::new(ctx).rule_out(fingerprint, mod_id)
}

/// Disable a mod by renaming `mods/<filename>` to `mods/<filename>.disabled`.
pub fn disable_mod<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    instance_id: &str,
    filename: &str,
) -> LauncherResult<()> {
    let ctx = crate::core_context(app)?;
    agora_core::crash_service::CrashService::new(ctx).disable_mod(instance_id, filename)
}

/// Enable a previously disabled mod (rename back).
pub fn enable_mod<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    instance_id: &str,
    filename: &str,
) -> LauncherResult<()> {
    let ctx = crate::core_context(app)?;
    agora_core::crash_service::CrashService::new(ctx).enable_mod(instance_id, filename)
}

// ---------------------------------------------------------------------------
// Tests — adapter-contract tests only
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    static JAR_CTR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn build_test_jar(entries: &[&str]) -> PathBuf {
        let id = JAR_CTR.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let jar_path =
            std::env::temp_dir().join(format!("agora-test-{}-{}.jar", std::process::id(), id));
        let file = std::fs::File::create(&jar_path).expect("create temp jar");
        {
            let mut zip = zip::ZipWriter::new(file);
            let opts: zip::write::FileOptions = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            for entry in entries {
                zip.start_file(*entry, opts).expect("start zip entry");
                zip.write_all(&[]).expect("write zip entry");
            }
            zip.finish().expect("finish zip");
        }
        jar_path
    }

    fn clean_jar(path: &PathBuf) {
        let _ = std::fs::remove_file(path);
    }

    // -----------------------------------------------------------------------
    // parse_jar_packages — pure delegation to core jar_metadata
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_jar_packages_extracts_top_level_packages() {
        let jar_path = build_test_jar(&[
            "me/jellysquid/nautilus/Foo.class",
            "me/jellysquid/nautilus/Bar.class",
            "com/example/init/Baz.class",
            "assets/textures/foo.png",
            "META-INF/MANIFEST.MF",
        ]);
        let pkgs = parse_jar_packages(&jar_path);
        assert!(pkgs.contains(&"me.jellysquid.nautilus".to_string()));
        assert!(pkgs.contains(&"com.example.init".to_string()));
        assert!(!pkgs.contains(&"me.jellysquid".to_string()));
        assert!(!pkgs.contains(&"assets".to_string()));
        assert!(!pkgs.contains(&"META-INF".to_string()));
        clean_jar(&jar_path);
    }

    #[test]
    fn test_parse_jar_packages_nonexistent_file_returns_empty() {
        let pkgs = parse_jar_packages(std::path::Path::new("/nonexistent/xyz.jar"));
        assert!(pkgs.is_empty());
    }

    #[test]
    fn test_parse_jar_packages_non_zip_returns_empty() {
        let txt_path =
            std::env::temp_dir().join(format!("agora-test-txt-{}.txt", std::process::id()));
        std::fs::write(&txt_path, "not a zip file").expect("write temp txt");
        let pkgs = parse_jar_packages(&txt_path);
        assert!(pkgs.is_empty());
        let _ = std::fs::remove_file(&txt_path);
    }
}
