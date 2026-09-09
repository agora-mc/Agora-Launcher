pub use agora_core::crash_export::CrashReportContext;

/// Build a shareable crash report for `context`, including the instance's mod
/// list when the instance is known.
pub fn build_crash_report(app: &tauri::AppHandle, context: &CrashReportContext) -> String {
    let manifest_path = context
        .instance_id
        .as_ref()
        .and_then(|id| crate::paths::instance_manifest_path(app, id).ok());
    agora_core::crash_export::build_crash_report(manifest_path, context)
}
