use std::fs;
use std::path::{Path, PathBuf};

use agora_game_api::{InstallCapabilities, InstallKind, StoreId};
use serde::Deserialize;

use crate::game_discovery::volume::VolumeDetector;
use crate::game_discovery::{DiscoveredInstall, DiscoveryReport, DiscoveryWarning};

#[derive(Deserialize)]
#[serde(untagged)]
enum StringOrVec {
    Single(String),
    Multiple(Vec<String>),
}

impl StringOrVec {
    fn into_vec(self) -> Vec<String> {
        match self {
            StringOrVec::Single(s) => s.split(',').map(|p| p.trim().to_string()).collect(),
            StringOrVec::Multiple(vec) => {
                let mut out = Vec::new();
                for s in vec {
                    for part in s.split(',') {
                        out.push(part.trim().to_string());
                    }
                }
                out
            }
        }
    }
}

#[derive(Deserialize)]
struct EpicItem {
    #[serde(default, rename = "AppName")]
    app_name: String,
    #[serde(default, rename = "DisplayName")]
    display_name: String,
    #[serde(default, rename = "InstallLocation")]
    install_location: String,
    #[serde(default, rename = "AppVersionString")]
    app_version_string: Option<String>,
    #[serde(default, rename = "LaunchExecutable")]
    launch_executable: Option<String>,
    #[serde(default, rename = "MainGameAppName")]
    main_game_app_name: Option<String>,
    #[serde(default, rename = "AppCategories")]
    app_categories: Option<StringOrVec>,
    #[serde(default, rename = "bIsIncompleteInstall")]
    is_incomplete_install: Option<bool>,
}

pub struct EpicAdapter;

impl EpicAdapter {
    pub fn discover_system(volume_detector: &VolumeDetector) -> DiscoveryReport {
        let dir = crate::game_discovery::platform::find_epic_manifests_dir();
        match dir {
            Some(d) => Self::discover_from(&d, volume_detector),
            None => DiscoveryReport::default(),
        }
    }

    pub fn discover_from(
        manifests_dir: &Path,
        volume_detector: &VolumeDetector,
    ) -> DiscoveryReport {
        if !manifests_dir.exists() {
            return DiscoveryReport::default();
        }

        let read_dir = match fs::read_dir(manifests_dir) {
            Ok(rd) => rd,
            Err(_) => return DiscoveryReport::default(),
        };

        let mut installs = Vec::new();
        let mut warnings = Vec::new();

        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("item") {
                continue;
            }

            let content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(err) => {
                    warnings.push(DiscoveryWarning {
                        store: StoreId::epic(),
                        message: format!("Failed to read {}: {}", path.display(), err),
                    });
                    continue;
                }
            };

            let item: EpicItem = match serde_json::from_str(&content) {
                Ok(item) => item,
                Err(err) => {
                    warnings.push(DiscoveryWarning {
                        store: StoreId::epic(),
                        message: format!("Failed to parse {}: {}", path.display(), err),
                    });
                    continue;
                }
            };

            if item.is_incomplete_install == Some(true) {
                continue;
            }

            if item.app_name.trim().is_empty() {
                continue;
            }

            let main_game = item
                .main_game_app_name
                .as_ref()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty());

            let is_addon = match main_game {
                Some(main) => main != item.app_name.trim(),
                None => false,
            };

            let categories = item
                .app_categories
                .map(|c| c.into_vec())
                .unwrap_or_default();
            let has_games_cat = categories.iter().any(|c| c.eq_ignore_ascii_case("games"));

            let (kind, parent_product) = if is_addon {
                (InstallKind::AddOn, main_game.map(|s| s.to_string()))
            } else if has_games_cat {
                (InstallKind::BaseGame, None)
            } else {
                (InstallKind::Tool, None)
            };

            let executables = match item.launch_executable.filter(|s| !s.trim().is_empty()) {
                Some(exe) => vec![exe],
                None => Vec::new(),
            };

            let location = PathBuf::from(item.install_location);
            let volume = volume_detector.get_volume_info(&location);

            installs.push(DiscoveredInstall {
                store: StoreId::epic(),
                product: item.app_name,
                name: item.display_name,
                kind,
                parent_product,
                location,
                store_version: item.app_version_string.filter(|s| !s.trim().is_empty()),
                store_build: None,
                executables,
                capabilities: InstallCapabilities {
                    executables_readable: true,
                    accepts_new_files: true,
                    relocatable: true,
                },
                volume,
            });
        }

        DiscoveryReport { installs, warnings }
    }
}
