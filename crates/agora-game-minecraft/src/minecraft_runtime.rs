//! Agora-owned Minecraft runtime root
//!
//! Direct launch uses this directory instead of the official `.minecraft`.
//! All shared artifacts (client JARs, version JSONs, loader profiles,
//! libraries, assets, natives, logging configs) are stored here.
//!
//! Instance-specific content (mods, config, saves, logs) remains under
//! `instances/<instance-id>/`.
//!
//! The official `.minecraft` directory is optional and read-only for direct
//! launch.  Mojang-launcher import and delegated launch may still reference it.

use agora_core::error::{LauncherError, LauncherResult};
use std::path::{Path, PathBuf};

/// Resolved layout of the Agora-owned Minecraft runtime root.
#[derive(Debug, Clone)]
pub struct MinecraftRuntimeLayout {
    pub root: PathBuf,
    pub versions: PathBuf,
    pub libraries: PathBuf,
    pub assets: PathBuf,
    pub logging: PathBuf,
    pub natives: PathBuf,
}

/// Bootstrap the Minecraft runtime layout under `root`.
///
/// Creates all required directories and a minimal `launcher_profiles.json`
/// if one does not already exist.  The official Mojang installer (Forge,
/// NeoForge) requires this file to be present in its target directory.
pub fn ensure_runtime_layout(root: &Path) -> LauncherResult<MinecraftRuntimeLayout> {
    let layout = MinecraftRuntimeLayout {
        root: root.to_path_buf(),
        versions: root.join("versions"),
        libraries: root.join("libraries"),
        assets: root.join("assets"),
        logging: root.join("logging"),
        natives: root.join("natives"),
    };

    for dir in [
        &layout.root,
        &layout.versions,
        &layout.libraries,
        &layout.assets,
        &layout.logging,
        &layout.natives,
    ] {
        std::fs::create_dir_all(dir).map_err(|error| LauncherError::Generic {
            code: "ERR_RUNTIME_LAYOUT".into(),
            message: format!("Failed to create {}: {error}", dir.display()),
        })?;
    }

    ensure_minimal_launcher_profiles(root)?;

    Ok(layout)
}

/// Create a minimal `launcher_profiles.json` if none exists.
///
/// Forge's official installer requires either `launcher_profiles.json` or
/// `launcher_profiles_microsoft_store.json` to exist in its target directory.
/// This empty profile satisfies that requirement without linking to the
/// official Mojang launcher.
fn ensure_minimal_launcher_profiles(root: &Path) -> LauncherResult<()> {
    let path = root.join("launcher_profiles.json");

    if path.is_file() {
        return Ok(());
    }

    crate::launch_planner::atomic_write(
        &path,
        br#"{
  "profiles": {}
}
"#,
    )?;

    Ok(())
}
