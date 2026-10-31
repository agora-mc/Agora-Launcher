use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use agora_game_api::{InstallCapabilities, InstallKind, StoreId};
use serde::Deserialize;

use crate::game_discovery::volume::VolumeDetector;
use crate::game_discovery::{DiscoveredInstall, DiscoveryReport, DiscoveryWarning};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GogRegistryEntry {
    pub game_id: String,
    pub game_name: String,
    pub path: PathBuf,
    pub ver: Option<String>,
    pub build_id: Option<String>,
    pub exe_file: Option<String>,
    pub depends_on: Option<String>,
    pub dlc: Option<String>,
}

#[derive(Deserialize)]
struct GogInfoFile {
    #[serde(default, rename = "playTasks")]
    play_tasks: Vec<GogPlayTask>,
}

#[derive(Deserialize)]
struct GogPlayTask {
    #[serde(default, rename = "type")]
    task_type: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    path: String,
}

pub struct GogAdapter;

impl GogAdapter {
    pub fn discover_system(volume_detector: &VolumeDetector) -> DiscoveryReport {
        let entries = crate::game_discovery::platform::find_gog_entries();
        Self::discover_from(&entries, volume_detector)
    }

    pub fn discover_from(
        entries: &[GogRegistryEntry],
        volume_detector: &VolumeDetector,
    ) -> DiscoveryReport {
        let mut installs = Vec::new();
        let mut warnings = Vec::new();
        let mut seen_ids = HashSet::new();

        for entry in entries {
            if entry.game_id.trim().is_empty() {
                continue;
            }
            if !seen_ids.insert(entry.game_id.clone()) {
                // Deduplicate by gameID
                continue;
            }

            // Classification
            let depends_on_clean = entry
                .depends_on
                .as_ref()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty());

            let (kind, parent_product) = match depends_on_clean {
                Some(parent) => (InstallKind::AddOn, Some(parent.to_string())),
                None => (InstallKind::BaseGame, None),
            };

            // Executables
            let mut executables = Vec::new();
            let info_path = entry.path.join(format!("goggame-{}.info", entry.game_id));

            let mut loaded_from_info = false;
            if info_path.exists() {
                match fs::read_to_string(&info_path) {
                    Ok(content) => match serde_json::from_str::<GogInfoFile>(&content) {
                        Ok(info) => {
                            let mut game_tasks = Vec::new();
                            let mut launcher_tasks = Vec::new();

                            for task in info.play_tasks {
                                if task.task_type == "FileTask" && !task.path.trim().is_empty() {
                                    if task.category.eq_ignore_ascii_case("game") {
                                        game_tasks.push(task.path);
                                    } else if task.category.eq_ignore_ascii_case("launcher") {
                                        launcher_tasks.push(task.path);
                                    }
                                }
                            }

                            if !game_tasks.is_empty() || !launcher_tasks.is_empty() {
                                executables.extend(game_tasks);
                                executables.extend(launcher_tasks);
                                loaded_from_info = true;
                            }
                        }
                        Err(err) => {
                            warnings.push(DiscoveryWarning {
                                store: StoreId::gog(),
                                message: format!(
                                    "Failed to parse {}: {}",
                                    info_path.display(),
                                    err
                                ),
                            });
                        }
                    },
                    Err(err) => {
                        warnings.push(DiscoveryWarning {
                            store: StoreId::gog(),
                            message: format!("Failed to read {}: {}", info_path.display(), err),
                        });
                    }
                }
            }

            if !loaded_from_info {
                if let Some(exe) = &entry.exe_file {
                    if !exe.trim().is_empty() {
                        executables.push(exe.clone());
                    }
                }
            }

            let volume = volume_detector.get_volume_info(&entry.path);

            installs.push(DiscoveredInstall {
                store: StoreId::gog(),
                product: entry.game_id.clone(),
                name: entry.game_name.clone(),
                kind,
                parent_product,
                location: entry.path.clone(),
                store_version: entry.ver.clone().filter(|s| !s.is_empty()),
                store_build: entry.build_id.clone().filter(|s| !s.is_empty()),
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
