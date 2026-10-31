//! Desktop shim for dependency resolution.
//!
//! Re-exports all types and functions from `agora_core::dependency_ops`. The
//! JAR metadata parser has been deduplicated to `agora_game_minecraft::jar_metadata`;
//! callers use `agora_game_minecraft::jar_metadata::parse_jar_metadata` directly.

use agora_core::dependency_ops::JarDeps;

// ---------------------------------------------------------------------------
// 1. Re-export all public types from core
// ---------------------------------------------------------------------------

pub use agora_core::dependency_ops::{
    AliasMap, DepCandidate, DepConflict, DepSource, DependencyEdge, DependentInfo, DisablePlan,
    IncompatibilityDecl, IncompatibilitySource, InstallPlan, RemovalPlan, Requirement,
    ResolvedInstallDeps,
};

// ---------------------------------------------------------------------------
// 2. Re-export core functions that callers reference
// ---------------------------------------------------------------------------

pub use agora_core::dependency_ops::{
    build_dependency_graph, build_dependency_graph_with_aliases, build_disable_plan_with_aliases,
    build_install_plan_with_aliases, build_removal_plan_with_aliases, detect_source_disagreement,
    find_dependents, find_dependents_with_aliases, resolve_install_deps,
    resolve_install_deps_with_aliases,
};

// ---------------------------------------------------------------------------
// 3. Desktop-specific wrappers preserving original signatures
// ---------------------------------------------------------------------------

/// Build a disable plan for a target mod.
///
/// Preserves the original signature used by `commands::get_disable_plan`.
pub fn build_disable_plan(
    installed: &[crate::models::InstalledMod],
    target: &crate::models::InstalledMod,
) -> DisablePlan {
    agora_core::dependency_ops::build_disable_plan(installed, target)
}

/// Build a removal plan for a target mod.
///
/// Preserves the original signature used by `commands::get_removal_plan`.
pub fn build_removal_plan(
    installed: &[crate::models::InstalledMod],
    target: &crate::models::InstalledMod,
) -> RemovalPlan {
    agora_core::dependency_ops::build_removal_plan(installed, target)
}

/// Build an install plan for a target mod.
///
/// Delegates directly to `agora_core::dependency_ops::build_install_plan`.
/// Callers pass `agora_core::dependency_ops::JarDeps` (from
/// `agora_game_minecraft::jar_metadata::parse_jar_metadata`) directly.
pub fn build_install_plan(
    target_manifest_deps: Option<crate::registry::ManifestDeps>,
    target_jar_deps: &JarDeps,
    installed: &[crate::models::InstalledMod],
    loader: &str,
    loader_version: &str,
    on_disk: &std::collections::HashSet<String>,
) -> InstallPlan {
    agora_core::dependency_ops::build_install_plan(
        target_manifest_deps,
        target_jar_deps,
        installed,
        loader,
        loader_version,
        on_disk,
    )
}

/// Refresh dependency identity metadata from the physical JARs of an instance.
///
/// The work, and its on-disk cache, live in
/// `agora_game_minecraft::dependency_service::refresh_installed_jar_metadata`; this only
/// resolves the instance directory from the Tauri app handle.
pub fn refresh_installed_jar_metadata<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    instance_id: &str,
    installed: &mut [crate::models::InstalledMod],
) -> crate::error::LauncherResult<()> {
    let instance_dir = crate::paths::instance_dir(app, instance_id)
        .map_err(|_| crate::error::LauncherError::InstanceCreateFailed)?;
    agora_game_minecraft::dependency_service::refresh_installed_jar_metadata(
        &instance_dir,
        installed,
    )
}
