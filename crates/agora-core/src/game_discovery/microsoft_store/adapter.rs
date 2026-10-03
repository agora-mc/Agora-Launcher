use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use agora_game_api::{InstallCapabilities, InstallKind, StoreId};

use super::config::parse_microsoft_game_config;
use crate::game_discovery::volume::VolumeDetector;
use crate::game_discovery::{DiscoveredInstall, DiscoveryReport, DiscoveryWarning};

pub struct MicrosoftStoreAdapter;

impl MicrosoftStoreAdapter {
    pub fn discover_system(volume_detector: &VolumeDetector) -> DiscoveryReport {
        let (roots, packages, warnings) =
            crate::game_discovery::platform::find_microsoft_store_inputs();
        let mut report = Self::discover_from(&roots, &packages, volume_detector);
        report.warnings.extend(warnings);
        report
    }

    pub fn discover_from(
        gaming_roots: &[PathBuf],
        appmodel_package_folders: &[PathBuf],
        volume_detector: &VolumeDetector,
    ) -> DiscoveryReport {
        let mut installs = Vec::new();
        let mut warnings = Vec::new();
        let mut seen_products = HashSet::new();

        // 1. Scan XboxGames / GamingRoots
        for root in gaming_roots {
            let read_dir = match fs::read_dir(root) {
                Ok(rd) => rd,
                Err(_) => continue,
            };

            for entry in read_dir.flatten() {
                let folder_path = entry.path();
                if !folder_path.is_dir() {
                    continue;
                }

                let folder_name = folder_path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();

                let content_dir = folder_path.join("Content");
                let config_path = content_dir.join("MicrosoftGame.config");

                // Skip folders without MicrosoftGame.config silently
                if !config_path.exists() {
                    continue;
                }

                process_config_file(
                    &config_path,
                    &content_dir,
                    &folder_name,
                    volume_detector,
                    &mut seen_products,
                    &mut installs,
                    &mut warnings,
                );
            }
        }

        // 2. Scan AppModel packages
        for folder in appmodel_package_folders {
            let config_path = folder.join("MicrosoftGame.config");
            if !config_path.exists() {
                continue;
            }

            let folder_name = folder
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();

            process_config_file(
                &config_path,
                folder,
                &folder_name,
                volume_detector,
                &mut seen_products,
                &mut installs,
                &mut warnings,
            );
        }

        DiscoveryReport { installs, warnings }
    }
}

fn process_config_file(
    config_path: &Path,
    location: &Path,
    folder_name: &str,
    volume_detector: &VolumeDetector,
    seen_products: &mut HashSet<String>,
    installs: &mut Vec<DiscoveredInstall>,
    warnings: &mut Vec<DiscoveryWarning>,
) {
    let content = match fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(err) => {
            warnings.push(DiscoveryWarning {
                store: StoreId::microsoft_store(),
                message: format!("Failed to read {}: {}", config_path.display(), err),
            });
            return;
        }
    };

    let config = match parse_microsoft_game_config(&content) {
        Ok(c) => c,
        Err(err) => {
            warnings.push(DiscoveryWarning {
                store: StoreId::microsoft_store(),
                message: format!("Failed to parse {}: {}", config_path.display(), err),
            });
            return;
        }
    };

    if !seen_products.insert(config.identity_name.clone()) {
        // Deduplicate by Identity Name
        return;
    }

    // Name resolution: ShellVisuals DefaultDisplayName unless it starts with ms-resource:
    let display_name = match &config.default_display_name {
        Some(name) if !name.starts_with("ms-resource:") && !name.trim().is_empty() => name.clone(),
        _ => folder_name.to_string(),
    };

    // Classification
    let (kind, parent_product) = if let Some(parent) = config.main_package_dependency {
        (InstallKind::AddOn, Some(parent))
    } else if config.has_executable_list {
        (InstallKind::BaseGame, None)
    } else {
        (InstallKind::AddOn, None)
    };

    // Executables readability capability
    let executables_readable = if let Some(first_exe) = config.executables.first() {
        let exe_path = location.join(first_exe);
        match File::open(&exe_path) {
            Ok(mut file) => {
                let mut byte = [0u8; 1];
                file.read_exact(&mut byte).is_ok()
            }
            Err(_) => false,
        }
    } else {
        false
    };

    let volume = volume_detector.get_volume_info(location);

    installs.push(DiscoveredInstall {
        store: StoreId::microsoft_store(),
        product: config.identity_name,
        name: display_name,
        kind,
        parent_product,
        location: location.to_path_buf(),
        store_version: config.identity_version,
        store_build: None,
        executables: config.executables,
        capabilities: InstallCapabilities {
            executables_readable,
            accepts_new_files: true,
            relocatable: false,
        },
        volume,
    });
}
