use std::path::PathBuf;

#[cfg(windows)]
use crate::game_discovery::gog::GogRegistryEntry;
#[cfg(windows)]
use crate::game_discovery::DiscoveryWarning;

/// Find Steam root on the current system.
pub fn find_steam_root() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        find_steam_root_windows()
    }
    #[cfg(target_os = "linux")]
    {
        find_steam_root_linux()
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        None
    }
}

#[cfg(windows)]
fn find_steam_root_windows() -> Option<PathBuf> {
    use winreg::enums::*;
    use winreg::RegKey;

    // 1. HKCU\Software\Valve\Steam (SteamPath)
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(steam_key) = hkcu.open_subkey("Software\\Valve\\Steam") {
        if let Ok(steam_path) = steam_key.get_value::<String, _>("SteamPath") {
            let path = PathBuf::from(steam_path.replace('/', "\\"));
            if path.exists() {
                return Some(path);
            }
        }
    }

    // 2. HKLM\SOFTWARE\WOW6432Node\Valve\Steam (InstallPath)
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    if let Ok(steam_key) = hklm.open_subkey("SOFTWARE\\WOW6432Node\\Valve\\Steam") {
        if let Ok(install_path) = steam_key.get_value::<String, _>("InstallPath") {
            let path = PathBuf::from(install_path.replace('/', "\\"));
            if path.exists() {
                return Some(path);
            }
        }
    }

    None
}

#[cfg(target_os = "linux")]
fn find_steam_root_linux() -> Option<PathBuf> {
    if let Some(home) = dirs::home_dir() {
        let p1 = home.join(".local/share/Steam");
        if p1.exists() {
            return Some(p1);
        }
        let p2 = home.join(".steam/steam");
        if p2.exists() {
            return Some(p2);
        }
    }
    None
}

/// Find GOG registry entries on Windows.
pub fn find_gog_entries() -> Vec<crate::game_discovery::gog::GogRegistryEntry> {
    #[cfg(windows)]
    {
        find_gog_entries_windows()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(windows)]
fn find_gog_entries_windows() -> Vec<GogRegistryEntry> {
    use std::collections::HashMap;
    use winreg::enums::*;
    use winreg::RegKey;

    let mut entries = Vec::new();
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);

    // HKLM\SOFTWARE\WOW6432Node\GOG.com\Games then HKLM\SOFTWARE\GOG.com\Games
    let subkey_paths = [
        "SOFTWARE\\WOW6432Node\\GOG.com\\Games",
        "SOFTWARE\\GOG.com\\Games",
    ];

    for parent_path in subkey_paths {
        let parent_key = match hklm.open_subkey(parent_path) {
            Ok(k) => k,
            Err(_) => continue,
        };

        for subkey_name in parent_key.enum_keys().flatten() {
            let subkey = match parent_key.open_subkey(&subkey_name) {
                Ok(k) => k,
                Err(_) => continue,
            };

            let mut values: HashMap<String, String> = HashMap::new();
            for val_name in subkey.enum_values().flatten().map(|(n, _)| n) {
                let lower_name = val_name.to_ascii_lowercase();
                if let Ok(s) = subkey.get_value::<String, _>(&val_name) {
                    values.insert(lower_name, s);
                } else if let Ok(u) = subkey.get_value::<u32, _>(&val_name) {
                    values.insert(lower_name, u.to_string());
                } else if let Ok(u) = subkey.get_value::<u64, _>(&val_name) {
                    values.insert(lower_name, u.to_string());
                }
            }

            let game_id = values
                .get("gameid")
                .cloned()
                .unwrap_or_else(|| subkey_name.clone());
            let game_name = values
                .get("gamename")
                .cloned()
                .unwrap_or_else(|| subkey_name.clone());
            let path_str = match values.get("path") {
                Some(p) if !p.trim().is_empty() => p.clone(),
                _ => continue,
            };

            let path = PathBuf::from(path_str);
            let ver = values.get("ver").cloned();
            let build_id = values.get("buildid").cloned();
            let exe_file = values.get("exefile").cloned();
            let depends_on = values.get("dependson").cloned();
            let dlc = values.get("dlc").cloned();

            entries.push(GogRegistryEntry {
                game_id,
                game_name,
                path,
                ver,
                build_id,
                exe_file,
                depends_on,
                dlc,
            });
        }
    }

    entries
}

/// Find Epic Games Launcher manifests folder.
pub fn find_epic_manifests_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let prog_data =
            std::env::var("ProgramData").unwrap_or_else(|_| "C:\\ProgramData".to_string());
        let path = PathBuf::from(prog_data)
            .join("Epic")
            .join("EpicGamesLauncher")
            .join("Data")
            .join("Manifests");
        if path.exists() {
            Some(path)
        } else {
            None
        }
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Find Microsoft Store roots and AppModel package folders on Windows.
pub fn find_microsoft_store_inputs() -> (
    Vec<PathBuf>,
    Vec<PathBuf>,
    Vec<crate::game_discovery::DiscoveryWarning>,
) {
    #[cfg(windows)]
    {
        find_microsoft_store_inputs_windows()
    }
    #[cfg(not(windows))]
    {
        (Vec::new(), Vec::new(), Vec::new())
    }
}

#[cfg(windows)]
fn find_microsoft_store_inputs_windows() -> (Vec<PathBuf>, Vec<PathBuf>, Vec<DiscoveryWarning>) {
    use crate::game_discovery::microsoft_store::gaming_root::parse_gaming_root;
    use agora_game_api::StoreId;
    use std::fs;
    use std::path::Path;
    use winreg::enums::*;
    use winreg::RegKey;

    let mut roots = Vec::new();
    let mut warnings = Vec::new();

    // 1. Scan drive letters A-Z
    for letter in b'A'..=b'Z' {
        let drive_root_str = format!("{}:\\", letter as char);
        let drive_root = Path::new(&drive_root_str);
        if !drive_root.exists() {
            continue;
        }

        let gaming_root_path = drive_root.join(".GamingRoot");
        if gaming_root_path.exists() {
            match fs::read(&gaming_root_path) {
                Ok(data) => match parse_gaming_root(&data) {
                    Ok(rel_paths) => {
                        for rel in rel_paths {
                            let root = drive_root.join(rel);
                            if root.exists() && !roots.contains(&root) {
                                roots.push(root);
                            }
                        }
                    }
                    Err(err) => {
                        warnings.push(DiscoveryWarning {
                            store: StoreId::microsoft_store(),
                            message: format!(
                                "Failed to parse {}: {}",
                                gaming_root_path.display(),
                                err
                            ),
                        });
                        let fallback = drive_root.join("XboxGames");
                        if fallback.exists() && !roots.contains(&fallback) {
                            roots.push(fallback);
                        }
                    }
                },
                Err(err) => {
                    warnings.push(DiscoveryWarning {
                        store: StoreId::microsoft_store(),
                        message: format!("Failed to read {}: {}", gaming_root_path.display(), err),
                    });
                    let fallback = drive_root.join("XboxGames");
                    if fallback.exists() && !roots.contains(&fallback) {
                        roots.push(fallback);
                    }
                }
            }
        } else {
            let xbox_games = drive_root.join("XboxGames");
            if xbox_games.exists() && !roots.contains(&xbox_games) {
                roots.push(xbox_games);
            }
        }
    }

    // 2. Scan AppModel Packages
    let mut package_folders = Vec::new();
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let appmodel_subpath = "Software\\Classes\\Local Settings\\Software\\Microsoft\\Windows\\CurrentVersion\\AppModel\\Repository\\Packages";
    if let Ok(packages_key) = hkcu.open_subkey(appmodel_subpath) {
        for subkey_name in packages_key.enum_keys().flatten() {
            if let Ok(subkey) = packages_key.open_subkey(&subkey_name) {
                if let Ok(pkg_root) = subkey.get_value::<String, _>("PackageRootFolder") {
                    let folder = PathBuf::from(pkg_root);
                    if folder.join("MicrosoftGame.config").exists()
                        && !package_folders.contains(&folder)
                    {
                        package_folders.push(folder);
                    }
                }
            }
        }
    }

    (roots, package_folders, warnings)
}
