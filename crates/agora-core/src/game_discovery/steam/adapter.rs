use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use agora_game_api::{InstallCapabilities, InstallKind, StoreId};

use super::vdf_binary::{parse_appinfo_vdf, SteamAppMetadata};
use super::vdf_text::{get_object, get_str, parse_vdf_text, VdfValue};
use crate::game_discovery::volume::VolumeDetector;
use crate::game_discovery::{DiscoveredInstall, DiscoveryReport, DiscoveryWarning};

const KNOWN_STEAM_TOOLS: &[u32] = &[
    228980,  // Steamworks Common Redistributables
    1070560, // Steam Linux Runtime
    1391110, // Steam Linux Runtime - Soldier
    1628350, // Steam Linux Runtime - Sniper
    1493710, // Proton Experimental
    2180100, // Proton Hotfix
    1887720, // Proton 7.0
    2348590, // Proton 8.0
    2805730, // Proton 9.0
    3658110, // Proton 10.0
    250820,  // SteamVR
];

pub struct SteamAdapter {
    root: Option<PathBuf>,
}

impl SteamAdapter {
    pub fn new(root: Option<PathBuf>) -> Self {
        Self { root }
    }

    pub fn discover_system(volume_detector: &VolumeDetector) -> DiscoveryReport {
        let root = crate::game_discovery::platform::find_steam_root();
        Self::new(root).discover_with_detector(volume_detector)
    }

    pub fn discover_from(root: &Path, volume_detector: &VolumeDetector) -> DiscoveryReport {
        Self::new(Some(root.to_path_buf())).discover_with_detector(volume_detector)
    }

    pub fn discover_with_detector(&self, volume_detector: &VolumeDetector) -> DiscoveryReport {
        let Some(root) = &self.root else {
            return DiscoveryReport::default();
        };

        if !root.exists() {
            return DiscoveryReport::default();
        }

        let mut warnings = Vec::new();

        // 1. Resolve library folders
        let mut libraries = Vec::new();
        libraries.push(root.clone());

        let libraryfolders_vdf = root.join("steamapps").join("libraryfolders.vdf");
        if libraryfolders_vdf.exists() {
            match fs::read_to_string(&libraryfolders_vdf) {
                Ok(content) => match parse_vdf_text(&content) {
                    Ok(parsed) => {
                        let root_obj = get_object(&parsed, "libraryfolders").unwrap_or(&parsed);
                        for (_k, v) in root_obj {
                            if let VdfValue::Object(sub) = v {
                                if let Some(path_str) = get_str(sub, "path") {
                                    let lib_path = PathBuf::from(path_str);
                                    if !libraries.iter().any(|p| path_matches(p, &lib_path)) {
                                        libraries.push(lib_path);
                                    }
                                }
                            }
                        }
                    }
                    Err(err) => {
                        warnings.push(DiscoveryWarning {
                            store: StoreId::steam(),
                            message: format!(
                                "Failed to parse {}: {}",
                                libraryfolders_vdf.display(),
                                err
                            ),
                        });
                    }
                },
                Err(err) => {
                    warnings.push(DiscoveryWarning {
                        store: StoreId::steam(),
                        message: format!(
                            "Failed to read {}: {}",
                            libraryfolders_vdf.display(),
                            err
                        ),
                    });
                }
            }
        }

        // 2. Discover apps from appmanifest_*.acf
        struct RawSteamApp {
            app_id: u32,
            name: String,
            location: PathBuf,
            build_id: Option<String>,
        }

        let mut discovered_apps = Vec::new();
        let mut app_ids = HashSet::new();

        for lib in &libraries {
            let steamapps = lib.join("steamapps");
            let read_dir = match fs::read_dir(&steamapps) {
                Ok(rd) => rd,
                Err(_) => continue,
            };

            for entry in read_dir.flatten() {
                let file_name = entry.file_name();
                let name_lossy = file_name.to_string_lossy();
                if !(name_lossy.starts_with("appmanifest_") && name_lossy.ends_with(".acf")) {
                    continue;
                }

                let acf_path = entry.path();
                let content = match fs::read_to_string(&acf_path) {
                    Ok(c) => c,
                    Err(err) => {
                        warnings.push(DiscoveryWarning {
                            store: StoreId::steam(),
                            message: format!("Failed to read {}: {}", acf_path.display(), err),
                        });
                        continue;
                    }
                };

                let parsed = match parse_vdf_text(&content) {
                    Ok(p) => p,
                    Err(err) => {
                        warnings.push(DiscoveryWarning {
                            store: StoreId::steam(),
                            message: format!("Failed to parse {}: {}", acf_path.display(), err),
                        });
                        continue;
                    }
                };

                let app_state = get_object(&parsed, "AppState").unwrap_or(&parsed);
                let appid_str = get_str(app_state, "appid");
                let name = get_str(app_state, "name").unwrap_or("");
                let state_flags_str = get_str(app_state, "StateFlags");
                let installdir = get_str(app_state, "installdir").unwrap_or("");
                let buildid = get_str(app_state, "buildid").filter(|s| !s.is_empty());

                let app_id = match appid_str.and_then(|s| s.parse::<u32>().ok()) {
                    Some(id) => id,
                    None => continue,
                };

                let state_flags = state_flags_str
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0);

                // Installed means bit 4 (value 4) is set AND the location folder exists
                if (state_flags & 4) == 0 {
                    continue;
                }

                let location = steamapps.join("common").join(installdir);
                if !location.exists() {
                    continue;
                }

                if !app_ids.insert(app_id) {
                    continue;
                }
                discovered_apps.push(RawSteamApp {
                    app_id,
                    name: name.to_string(),
                    location,
                    build_id: buildid.map(|s| s.to_string()),
                });
            }
        }

        // 3. Parse appcache/appinfo.vdf if present
        let appinfo_path = root.join("appcache").join("appinfo.vdf");
        let metadata_map: Option<HashMap<u32, SteamAppMetadata>> = if appinfo_path.exists() {
            match fs::read(&appinfo_path) {
                Ok(bytes) => match parse_appinfo_vdf(&bytes, Some(&app_ids)) {
                    Ok(map) => Some(map),
                    Err(err) => {
                        warnings.push(DiscoveryWarning {
                            store: StoreId::steam(),
                            message: format!("Failed to parse {}: {}", appinfo_path.display(), err),
                        });
                        None
                    }
                },
                Err(err) => {
                    warnings.push(DiscoveryWarning {
                        store: StoreId::steam(),
                        message: format!("Failed to read {}: {}", appinfo_path.display(), err),
                    });
                    None
                }
            }
        } else {
            // Warn once when appinfo.vdf is missing but apps were discovered
            if !discovered_apps.is_empty() {
                warnings.push(DiscoveryWarning {
                    store: StoreId::steam(),
                    message: format!("Missing {}", appinfo_path.display()),
                });
            }
            None
        };

        // 4. Classify and produce DiscoveredInstall
        let mut installs = Vec::new();
        for app in discovered_apps {
            let (kind, parent_product, executables) =
                if let Some(meta) = metadata_map.as_ref().and_then(|m| m.get(&app.app_id)) {
                    let app_type = meta.app_type.as_deref().unwrap_or("");
                    let kind = if app_type.eq_ignore_ascii_case("game")
                        || app_type.eq_ignore_ascii_case("demo")
                    {
                        InstallKind::BaseGame
                    } else if app_type.eq_ignore_ascii_case("dlc") {
                        InstallKind::AddOn
                    } else {
                        InstallKind::Tool
                    };

                    let parent_product = if kind == InstallKind::AddOn {
                        meta.parent.clone()
                    } else {
                        None
                    };

                    (kind, parent_product, meta.executables.clone())
                } else {
                    // Fallback classification
                    let is_tool = KNOWN_STEAM_TOOLS.contains(&app.app_id)
                        || app.name.contains("Proton")
                        || app.name.contains("Steam Linux Runtime")
                        || app.name.contains("Redistributable");

                    let kind = if is_tool {
                        InstallKind::Tool
                    } else {
                        InstallKind::BaseGame
                    };

                    (kind, None, Vec::new())
                };

            let volume = volume_detector.get_volume_info(&app.location);

            installs.push(DiscoveredInstall {
                store: StoreId::steam(),
                product: app.app_id.to_string(),
                name: app.name,
                kind,
                parent_product,
                location: app.location,
                store_version: None,
                store_build: app.build_id,
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

fn path_matches(a: &Path, b: &Path) -> bool {
    if let (Ok(ca), Ok(cb)) = (fs::canonicalize(a), fs::canonicalize(b)) {
        ca == cb
    } else {
        let na = a.to_string_lossy().replace('/', "\\").to_ascii_lowercase();
        let nb = b.to_string_lossy().replace('/', "\\").to_ascii_lowercase();
        na.trim_end_matches('\\') == nb.trim_end_matches('\\')
    }
}
