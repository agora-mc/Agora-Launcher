//! Game folders the user adds by hand (MASTER_SPEC §26.3).
//!
//! A store scan finds what a store knows. A copy on another drive, a portable install or a folder
//! another manager prepared is found only because the user points Agora at it. The list lives in
//! `game-folders.json` under the data root. Discovery lists each folder as a `direct` install, and
//! identification matches it to a game by the same rule as a store install: the game's executable
//! must be inside it, and the runtime comes from that executable's version.
//!
//! Every refusal names its reason, and a damaged list is an error rather than an empty list:
//! guessing "no folders" would make every install in the list silently disappear.

use std::path::{Path, PathBuf};

use agora_game_api::{InstallCapabilities, InstallKind, StoreId};
use serde::{Deserialize, Serialize};

use super::volume::VolumeDetector;
use super::{discover_all, DiscoveredInstall, DiscoveryWarning};
use crate::app_paths::AppPaths;
use crate::error::{LauncherError, LauncherResult};
use crate::game_registry::GameRegistry;

/// The file, under the data root, that holds the user's folders.
pub const FOLDERS_FILE: &str = "game-folders.json";
const FOLDERS_VERSION: u32 = 1;

/// The folder is not there, or cannot be named (a missing drive counts).
pub const ERR_FOLDER_MISSING: &str = "ERR_GAME_FOLDER_MISSING";
/// The path names a file, not a folder.
pub const ERR_FOLDER_NOT_A_FOLDER: &str = "ERR_GAME_FOLDER_NOT_A_FOLDER";
/// The path is empty, or not a usable path at all.
pub const ERR_FOLDER_INVALID: &str = "ERR_GAME_FOLDER_INVALID";
/// The folder is already in the list.
pub const ERR_FOLDER_DUPLICATE: &str = "ERR_GAME_FOLDER_DUPLICATE";
/// The folder is inside (or is) an install Agora already knows.
pub const ERR_FOLDER_INSIDE_INSTALL: &str = "ERR_GAME_FOLDER_INSIDE_INSTALL";
/// No game Agora knows has its executable in the folder, or more than one does.
pub const ERR_FOLDER_NOT_A_GAME: &str = "ERR_GAME_FOLDER_NOT_A_GAME";
/// The folder to remove is not in the list.
pub const ERR_FOLDER_NOT_LISTED: &str = "ERR_GAME_FOLDER_NOT_LISTED";
/// The list file exists but cannot be read or understood.
pub const ERR_FOLDERS_UNREADABLE: &str = "ERR_GAME_FOLDERS_UNREADABLE";
/// The list file could not be written.
pub const ERR_FOLDERS_WRITE: &str = "ERR_GAME_FOLDERS_WRITE";

#[derive(Debug, Serialize, Deserialize)]
struct FolderFile {
    version: u32,
    folders: Vec<String>,
}

/// A folder that `add_folder` accepted, as it is stored, and the game it holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddedFolder {
    pub path: String,
    pub game_id: String,
    pub game_name: String,
}

fn refused(code: &str, message: impl Into<String>) -> LauncherError {
    LauncherError::Generic {
        code: code.to_string(),
        message: message.into(),
    }
}

/// Where the list is kept.
pub fn folders_file(paths: &AppPaths) -> PathBuf {
    paths.root().join(FOLDERS_FILE)
}

/// The folders the user added, as stored. A missing file is an empty list; a file that cannot be
/// read or parsed is an error, never an empty list.
pub fn list_folders(paths: &AppPaths) -> LauncherResult<Vec<String>> {
    let path = folders_file(paths);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(refused(
                ERR_FOLDERS_UNREADABLE,
                format!(
                    "Agora could not read your game folders list at {}: {error}",
                    path.display()
                ),
            ))
        }
    };
    let file: FolderFile = serde_json::from_slice(&bytes).map_err(|error| {
        refused(
            ERR_FOLDERS_UNREADABLE,
            format!(
                "Your game folders list at {} is damaged ({error}). Fix or delete the file. Agora will not guess which folders you added.",
                path.display()
            ),
        )
    })?;
    if file.version != FOLDERS_VERSION {
        return Err(refused(
            ERR_FOLDERS_UNREADABLE,
            format!(
                "Your game folders list at {} is version {}, which this Agora does not read.",
                path.display(),
                file.version
            ),
        ));
    }
    Ok(file.folders)
}

/// A folder the user added, and whether it is still on disk. A folder that is gone is listed, so
/// the user can remove it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListedFolder {
    pub path: String,
    pub present: bool,
}

/// The folders the user added, each with whether it is still there.
pub fn list_folder_status(paths: &AppPaths) -> LauncherResult<Vec<ListedFolder>> {
    Ok(list_folders(paths)?
        .into_iter()
        .map(|path| {
            let present = Path::new(&path).is_dir();
            ListedFolder { path, present }
        })
        .collect())
}

fn save_folders(paths: &AppPaths, folders: &[String]) -> LauncherResult<()> {
    let path = folders_file(paths);
    let write_failed = |error: std::io::Error| {
        refused(
            ERR_FOLDERS_WRITE,
            format!(
                "Agora could not save your game folders list at {}: {error}",
                path.display()
            ),
        )
    };
    let root = paths.root();
    std::fs::create_dir_all(root).map_err(write_failed)?;
    let bytes = serde_json::to_vec_pretty(&FolderFile {
        version: FOLDERS_VERSION,
        folders: folders.to_vec(),
    })
    .map_err(|error| {
        refused(
            ERR_FOLDERS_WRITE,
            format!("Agora could not encode your game folders: {error}"),
        )
    })?;
    let tmp = root.join(format!(
        "{FOLDERS_FILE}.{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    let result = write_synced(&tmp, &bytes).and_then(|()| std::fs::rename(&tmp, &path));
    if let Err(error) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(write_failed(error));
    }
    Ok(())
}

fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// The folder as a path the user would recognise: the real path, without Windows' `\\?\` prefix.
pub fn display_path(canonical: &Path) -> String {
    let text = canonical.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        return rest.to_string();
    }
    text.into_owned()
}

/// The identity of a folder for comparisons. Two spellings of one folder (case, a trailing
/// separator) give the same key on Windows, where names are case-insensitive.
pub fn folder_key(display: &str) -> String {
    let trimmed = display.trim().trim_end_matches(['\\', '/']);
    if cfg!(windows) {
        // Windows drops trailing dots and spaces from a name, so `Skyrim.` is `Skyrim`.
        trimmed.trim_end_matches(['.', ' ']).to_lowercase()
    } else {
        trimmed.to_string()
    }
}

/// Whether the last part of `path` is a Windows device name (`NUL`, `CON`, `COM1`...). Windows
/// opens those whatever folder they sit in, so they are never a game folder, and `NUL.txt` is
/// `NUL` too.
fn names_a_windows_device(path: &str) -> bool {
    if !cfg!(windows) {
        return false;
    }
    let last = path
        .rsplit(['\\', '/'])
        .find(|part| !part.is_empty())
        .unwrap_or("");
    let stem = last
        .trim_end_matches(['.', ' '])
        .split('.')
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0')
}

/// Whether `child` is strictly inside `parent`, comparing whole path components.
fn is_strictly_inside(child: &str, parent: &str) -> bool {
    child != parent && Path::new(child).starts_with(Path::new(parent))
}

/// Add a folder the user picked. It must exist, be a folder, not be one Agora already lists or a
/// store install, and hold exactly one game's executable.
pub fn add_folder(
    paths: &AppPaths,
    registry: &GameRegistry,
    requested: &str,
) -> LauncherResult<AddedFolder> {
    let store_installs = discover_all().installs;
    add_folder_checked(paths, registry, requested, &store_installs)
}

/// `add_folder` with the store installs given, so the refusals about them can be tested without
/// a real Steam or GOG on the machine.
fn add_folder_checked(
    paths: &AppPaths,
    registry: &GameRegistry,
    requested: &str,
    store_installs: &[DiscoveredInstall],
) -> LauncherResult<AddedFolder> {
    let requested = requested.trim();
    if requested.is_empty() {
        return Err(refused(
            ERR_FOLDER_INVALID,
            "No folder was given. Pick the folder the game is installed in.",
        ));
    }
    if requested.contains('\0') {
        return Err(refused(
            ERR_FOLDER_INVALID,
            "That is not a usable folder path.",
        ));
    }
    if names_a_windows_device(requested) {
        return Err(refused(
            ERR_FOLDER_INVALID,
            format!("{requested} is the name of a Windows device, not a game folder."),
        ));
    }
    let raw = Path::new(requested);
    let metadata = std::fs::metadata(raw).map_err(|_| {
        refused(
            ERR_FOLDER_MISSING,
            format!(
                "{requested} does not exist. Check the spelling, or that its drive is connected."
            ),
        )
    })?;
    if !metadata.is_dir() {
        return Err(refused(
            ERR_FOLDER_NOT_A_FOLDER,
            format!("{requested} is a file, not a folder. Pick the folder the game is in."),
        ));
    }
    let canonical = std::fs::canonicalize(raw).map_err(|_| {
        refused(
            ERR_FOLDER_MISSING,
            format!("{requested} could not be opened. Check that it is still there."),
        )
    })?;
    let display = display_path(&canonical);
    let key = folder_key(&display);

    let mut folders = list_folders(paths)?;
    if folders.iter().any(|listed| folder_key(listed) == key) {
        return Err(refused(
            ERR_FOLDER_DUPLICATE,
            format!("{display} is already in your game folders."),
        ));
    }
    for listed in &folders {
        if is_strictly_inside(&key, &folder_key(listed)) {
            return Err(refused(
                ERR_FOLDER_INSIDE_INSTALL,
                format!("{display} is inside {listed}, which is already in your game folders."),
            ));
        }
    }

    for install in store_installs {
        let install_key = folder_key(&display_path(&install.location));
        if install_key == key {
            return Err(refused(
                ERR_FOLDER_INSIDE_INSTALL,
                format!(
                    "{display} is already found as {} from {}, so it does not need adding.",
                    install.name, install.store
                ),
            ));
        }
        if is_strictly_inside(&key, &install_key) {
            return Err(refused(
                ERR_FOLDER_INSIDE_INSTALL,
                format!(
                    "{display} is inside {}, which Agora already finds from {}. Add the game's own folder instead.",
                    install.name, install.store
                ),
            ));
        }
    }

    let matches = registry.games_with_executable_in(&canonical);
    let game_id = match matches.as_slice() {
        [only] => only.clone(),
        [] => {
            let expected: Vec<String> = registry
                .games()
                .flat_map(|game| {
                    game.version_sources
                        .iter()
                        .filter_map(move |source| match source {
                            agora_game_api::VersionSource::Executable { path } => {
                                Some(format!("{} ({})", path.as_str(), game.name))
                            }
                            _ => None,
                        })
                })
                .collect();
            let looking_for = if expected.is_empty() {
                "no game Agora knows has a file it can look for".to_string()
            } else {
                format!("the game's executable: {}", expected.join(", "))
            };
            return Err(refused(
                ERR_FOLDER_NOT_A_GAME,
                format!("No game Agora knows is in {display}. Agora looks for {looking_for}."),
            ));
        }
        many => {
            let names: Vec<String> = many.iter().map(|id| id.to_string()).collect();
            return Err(refused(
                ERR_FOLDER_NOT_A_GAME,
                format!(
                    "{display} matches more than one game ({}), so Agora cannot tell which it is.",
                    names.join(", ")
                ),
            ));
        }
    };

    let game_name = registry
        .game(&game_id)
        .map(|game| game.name.clone())
        .unwrap_or_else(|| game_id.to_string());
    folders.push(display.clone());
    save_folders(paths, &folders)?;
    Ok(AddedFolder {
        path: display,
        game_id: game_id.to_string(),
        game_name,
    })
}

/// Remove a folder from the list. It may be gone from disk now, so the match is on the spelling
/// as well as the real path. Returns the stored path that was removed.
pub fn remove_folder(paths: &AppPaths, requested: &str) -> LauncherResult<String> {
    let requested = requested.trim();
    if requested.is_empty() {
        return Err(refused(
            ERR_FOLDER_INVALID,
            "No folder was given to remove.",
        ));
    }
    let key = match std::fs::canonicalize(requested) {
        Ok(canonical) => folder_key(&display_path(&canonical)),
        Err(_) => folder_key(requested),
    };
    let mut folders = list_folders(paths)?;
    let Some(index) = folders.iter().position(|listed| folder_key(listed) == key) else {
        return Err(refused(
            ERR_FOLDER_NOT_LISTED,
            format!("{requested} is not in your game folders, so there is nothing to remove."),
        ));
    };
    let removed = folders.remove(index);
    save_folders(paths, &folders)?;
    Ok(removed)
}

/// The user's folders as discovery lists them: a `direct` install for each one still on disk.
/// A folder that is gone is a warning naming it, not a silent drop.
pub fn discover_folders(paths: &AppPaths) -> (Vec<DiscoveredInstall>, Vec<DiscoveryWarning>) {
    let folders = match list_folders(paths) {
        Ok(folders) => folders,
        Err(error) => {
            return (
                Vec::new(),
                vec![DiscoveryWarning {
                    store: StoreId::direct(),
                    message: format!("The game folders you added were not searched: {error}"),
                }],
            )
        }
    };
    let detector = VolumeDetector::new();
    let mut installs = Vec::new();
    let mut warnings = Vec::new();
    for folder in folders {
        let location = PathBuf::from(&folder);
        if !location.is_dir() {
            warnings.push(DiscoveryWarning {
                store: StoreId::direct(),
                message: format!("{folder} is in your game folders but is not there now, so it was not searched."),
            });
            continue;
        }
        let name = location
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| folder.clone());
        installs.push(DiscoveredInstall {
            store: StoreId::direct(),
            product: folder_key(&folder),
            name,
            kind: InstallKind::BaseGame,
            parent_product: None,
            volume: detector.get_volume_info(&location),
            location,
            store_version: None,
            store_build: None,
            executables: Vec::new(),
            capabilities: InstallCapabilities {
                executables_readable: true,
                accepts_new_files: true,
                relocatable: true,
            },
        });
    }
    (installs, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agora_game_api::{
        DeploymentStrategy, GameDefinition, GameId, GamePackage, PackageDefinition, RelPath,
        VersionSource,
    };
    use std::sync::Arc;

    struct Package(PackageDefinition);

    impl GamePackage for Package {
        fn definition(&self) -> &PackageDefinition {
            &self.0
        }
    }

    fn game(id: &str, name: &str, executable: &str) -> GameDefinition {
        GameDefinition {
            mo2_game_name: None,
            id: GameId::new(id).expect("valid game id"),
            name: name.to_string(),
            stores: Vec::new(),
            version_sources: vec![VersionSource::Executable {
                path: RelPath::new(executable).expect("valid path"),
            }],
            deployment: DeploymentStrategy::Redirect,
            content_rules: Vec::new(),
            native_code_patterns: Vec::new(),
            framework_ids: Vec::new(),
            tool_ids: Vec::new(),
            launch: None,
            log_paths: Vec::new(),
            crash_paths: Vec::new(),
            user_files: Vec::new(),
            save_paths: Vec::new(),
            linked_archive_patterns: Vec::new(),
            declared_writes: Vec::new(),
            excluded_paths: Vec::new(),
            content_layout: None,
            plugin_list: None,
            runtime_files: Vec::new(),
            save_location: Vec::new(),
            launch_alternatives: Vec::new(),
            copy_patterns: Vec::new(),
        }
    }

    fn registry_with(games: Vec<GameDefinition>) -> GameRegistry {
        let package = Package(PackageDefinition {
            id: "test.folders".to_string(),
            version: semver::Version::new(0, 1, 0),
            api_range: semver::VersionReq::parse(">=0.1, <0.2").expect("valid range"),
            parents: Vec::new(),
            games,
            frameworks: Vec::new(),
            tools: Vec::new(),
        });
        let mut builder = GameRegistry::builder();
        builder
            .add(
                crate::game_registry::PackageSource::Compiled {
                    crate_name: "test-folders".to_string(),
                },
                Arc::new(package),
            )
            .expect("the test package registers");
        builder.build()
    }

    fn paths_in(dir: &Path) -> AppPaths {
        AppPaths::from_root(dir.join("data"))
    }

    /// `add_folder` with no store installs, so a test never reads the real machine's stores.
    fn add(
        paths: &AppPaths,
        registry: &GameRegistry,
        requested: &str,
    ) -> LauncherResult<AddedFolder> {
        add_folder_checked(paths, registry, requested, &[])
    }

    fn store_install_at(location: &Path, name: &str) -> DiscoveredInstall {
        DiscoveredInstall {
            store: StoreId::steam(),
            product: "489830".to_string(),
            name: name.to_string(),
            kind: InstallKind::BaseGame,
            parent_product: None,
            location: location.to_path_buf(),
            store_version: None,
            store_build: None,
            executables: Vec::new(),
            capabilities: InstallCapabilities {
                executables_readable: true,
                accepts_new_files: true,
                relocatable: true,
            },
            volume: None,
        }
    }

    #[test]
    fn a_missing_list_is_empty_and_a_damaged_one_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(tmp.path());
        assert!(list_folders(&paths).unwrap().is_empty());

        std::fs::create_dir_all(paths.root()).unwrap();
        std::fs::write(folders_file(&paths), b"{ not json").unwrap();
        let error = list_folders(&paths).unwrap_err();
        assert_eq!(error.code(), ERR_FOLDERS_UNREADABLE);
    }

    #[test]
    fn a_list_from_another_version_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(tmp.path());
        std::fs::create_dir_all(paths.root()).unwrap();
        std::fs::write(folders_file(&paths), br#"{"version": 9, "folders": []}"#).unwrap();
        assert_eq!(
            list_folders(&paths).unwrap_err().code(),
            ERR_FOLDERS_UNREADABLE
        );
    }

    #[test]
    fn adding_a_folder_with_the_game_executable_lists_it_and_discovery_finds_it() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(tmp.path());
        let game_dir = tmp.path().join("SkyrimCopy");
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::write(game_dir.join("SkyrimSE.exe"), b"not really an exe").unwrap();
        let registry = registry_with(vec![game(
            "skyrim-se",
            "Skyrim Special Edition",
            "SkyrimSE.exe",
        )]);

        let added = add(&paths, &registry, &format!("{}/", game_dir.display())).unwrap();
        assert_eq!(added.game_id, "skyrim-se");
        assert_eq!(list_folders(&paths).unwrap(), vec![added.path.clone()]);

        let (installs, warnings) = discover_folders(&paths);
        assert!(warnings.is_empty());
        assert_eq!(installs.len(), 1);
        assert_eq!(installs[0].store, StoreId::direct());
        assert_eq!(installs[0].name, "SkyrimCopy");

        // Identification claims it by its executable, the same way it claims a store install.
        let report = crate::game_discovery::DiscoveryReport { installs, warnings };
        let inventory =
            crate::game_registry::identify_installs(&registry, &report, &|_: &Path| None);
        assert_eq!(
            inventory.installs.len(),
            1,
            "the folder is identified as a game install"
        );
        assert_eq!(inventory.installs[0].game.to_string(), "skyrim-se");
    }

    #[test]
    fn the_shared_install_inventory_includes_a_custom_folder() {
        // `installed_games` is what the desktop and the CLI ask for installs; it must see the
        // folders the user added, not only the stores.
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path().join("MyDndCopy");
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::write(game_dir.join("OnlyHere.exe"), b"x").unwrap();
        let registry = std::sync::Arc::new(registry_with(vec![game(
            "folder-only-game",
            "Folder Only Game",
            "OnlyHere.exe",
        )]));
        let ctx =
            crate::ctx::CoreContext::for_testing(tmp.path().join("data")).with_games(registry);
        add_folder_checked(&ctx.paths, &ctx.games, &game_dir.display().to_string(), &[]).unwrap();

        let inventory = crate::game_registry::installed_games(&ctx);
        assert!(
            inventory
                .installs
                .iter()
                .any(|i| i.game.as_str() == "folder-only-game"),
            "the custom folder is in the inventory"
        );
    }

    #[test]
    fn a_folder_in_the_list_reports_whether_it_is_still_there() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(tmp.path());
        let game_dir = tmp.path().join("Skyrim");
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::write(game_dir.join("SkyrimSE.exe"), b"x").unwrap();
        let registry = registry_with(vec![game("skyrim-se", "Skyrim", "SkyrimSE.exe")]);
        let added = add(&paths, &registry, &game_dir.display().to_string()).unwrap();

        assert_eq!(
            list_folder_status(&paths).unwrap(),
            vec![ListedFolder {
                path: added.path.clone(),
                present: true
            }]
        );
        std::fs::remove_dir_all(&game_dir).unwrap();
        assert_eq!(
            list_folder_status(&paths).unwrap(),
            vec![ListedFolder {
                path: added.path,
                present: false
            }],
            "a folder that is gone is still listed, so the user can remove it"
        );
    }

    #[test]
    fn a_folder_that_is_a_store_install_or_inside_one_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(tmp.path());
        let steam_dir = tmp.path().join("steamapps").join("Skyrim");
        let inner = steam_dir.join("Data");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::write(steam_dir.join("SkyrimSE.exe"), b"x").unwrap();
        std::fs::write(inner.join("SkyrimSE.exe"), b"x").unwrap();
        let registry = registry_with(vec![game("skyrim-se", "Skyrim", "SkyrimSE.exe")]);
        let stores = [store_install_at(&steam_dir, "Skyrim")];

        let same = add_folder_checked(&paths, &registry, &steam_dir.display().to_string(), &stores);
        assert_eq!(
            same.unwrap_err().code(),
            ERR_FOLDER_INSIDE_INSTALL,
            "the store already finds it"
        );
        let within = add_folder_checked(&paths, &registry, &inner.display().to_string(), &stores);
        assert_eq!(within.unwrap_err().code(), ERR_FOLDER_INSIDE_INSTALL);
        assert!(list_folders(&paths).unwrap().is_empty());
    }

    #[test]
    fn a_windows_device_name_is_refused_as_not_a_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(tmp.path());
        let registry = registry_with(vec![game("skyrim-se", "Skyrim", "SkyrimSE.exe")]);
        if cfg!(windows) {
            for name in [
                "NUL",
                "nul",
                "NUL.txt",
                "C:\\Games\\CON",
                "COM1",
                "lpt9.log",
                "AUX.",
            ] {
                assert_eq!(
                    add(&paths, &registry, name).unwrap_err().code(),
                    ERR_FOLDER_INVALID,
                    "{name} names a device"
                );
            }
            assert!(
                !names_a_windows_device("C:\\Games\\CONSOLE"),
                "a longer name is a folder"
            );
            assert!(
                !names_a_windows_device("C:\\Games\\COM10"),
                "COM10 is not a device name"
            );
        }
        assert!(list_folders(&paths).unwrap().is_empty());
    }

    #[test]
    fn a_folder_spelt_with_trailing_dots_or_spaces_is_the_same_folder() {
        if !cfg!(windows) {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(tmp.path());
        let game_dir = tmp.path().join("Skyrim");
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::write(game_dir.join("SkyrimSE.exe"), b"x").unwrap();
        let registry = registry_with(vec![game("skyrim-se", "Skyrim", "SkyrimSE.exe")]);
        add(&paths, &registry, &game_dir.display().to_string()).unwrap();

        for spelling in [
            format!("{}.", game_dir.display()),
            format!("{} ", game_dir.display()),
        ] {
            assert_eq!(
                add(&paths, &registry, &spelling).unwrap_err().code(),
                ERR_FOLDER_DUPLICATE,
                "{spelling:?} is the folder already listed"
            );
        }
        assert_eq!(
            folder_key("C:\\Games\\Skyrim."),
            folder_key("c:\\games\\skyrim")
        );
        assert_eq!(list_folders(&paths).unwrap().len(), 1);
    }

    #[test]
    fn the_same_folder_spelt_differently_is_a_duplicate() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(tmp.path());
        let game_dir = tmp.path().join("Skyrim");
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::write(game_dir.join("SkyrimSE.exe"), b"x").unwrap();
        let registry = registry_with(vec![game("skyrim-se", "Skyrim", "SkyrimSE.exe")]);

        add(&paths, &registry, &game_dir.display().to_string()).unwrap();
        let trailing = format!("{}{}", game_dir.display(), std::path::MAIN_SEPARATOR);
        let error = add(&paths, &registry, &trailing).unwrap_err();
        assert_eq!(error.code(), ERR_FOLDER_DUPLICATE);
        if cfg!(windows) {
            // Windows names are case-insensitive, so another casing is the same folder.
            let shouted = game_dir.display().to_string().to_uppercase();
            assert_eq!(
                add(&paths, &registry, &shouted).unwrap_err().code(),
                ERR_FOLDER_DUPLICATE
            );
        }
        assert_eq!(list_folders(&paths).unwrap().len(), 1);
    }

    #[test]
    fn an_empty_or_missing_or_file_path_is_refused_with_its_reason() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(tmp.path());
        let registry = registry_with(vec![game("skyrim-se", "Skyrim", "SkyrimSE.exe")]);

        assert_eq!(
            add(&paths, &registry, "   ").unwrap_err().code(),
            ERR_FOLDER_INVALID
        );
        let missing = tmp.path().join("nope");
        assert_eq!(
            add(&paths, &registry, &missing.display().to_string())
                .unwrap_err()
                .code(),
            ERR_FOLDER_MISSING
        );
        let file = tmp.path().join("SkyrimSE.exe");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(
            add(&paths, &registry, &file.display().to_string())
                .unwrap_err()
                .code(),
            ERR_FOLDER_NOT_A_FOLDER
        );
        assert!(list_folders(&paths).unwrap().is_empty());
    }

    #[test]
    fn a_folder_with_no_known_game_is_refused_and_says_what_it_looks_for() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(tmp.path());
        let plain = tmp.path().join("Documents");
        std::fs::create_dir_all(&plain).unwrap();
        let registry = registry_with(vec![game("skyrim-se", "Skyrim", "SkyrimSE.exe")]);

        let error = add(&paths, &registry, &plain.display().to_string()).unwrap_err();
        assert_eq!(error.code(), ERR_FOLDER_NOT_A_GAME);
        assert!(error.to_string().contains("SkyrimSE.exe"), "{error}");
    }

    #[test]
    fn a_folder_matching_two_games_is_refused_rather_than_guessed() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(tmp.path());
        let both = tmp.path().join("Both");
        std::fs::create_dir_all(&both).unwrap();
        std::fs::write(both.join("Shared.exe"), b"x").unwrap();
        let registry = registry_with(vec![
            game("game-a", "Game A", "Shared.exe"),
            game("game-b", "Game B", "Shared.exe"),
        ]);

        let error = add(&paths, &registry, &both.display().to_string()).unwrap_err();
        assert_eq!(error.code(), ERR_FOLDER_NOT_A_GAME);
        assert!(list_folders(&paths).unwrap().is_empty());
    }

    #[test]
    fn a_folder_inside_one_already_listed_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(tmp.path());
        let game_dir = tmp.path().join("Skyrim");
        let inner = game_dir.join("Mods");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::write(game_dir.join("SkyrimSE.exe"), b"x").unwrap();
        std::fs::write(inner.join("SkyrimSE.exe"), b"x").unwrap();
        let registry = registry_with(vec![game("skyrim-se", "Skyrim", "SkyrimSE.exe")]);

        add(&paths, &registry, &game_dir.display().to_string()).unwrap();
        let error = add(&paths, &registry, &inner.display().to_string()).unwrap_err();
        assert_eq!(error.code(), ERR_FOLDER_INSIDE_INSTALL);
    }

    #[test]
    fn removing_a_folder_that_is_gone_from_disk_still_works() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(tmp.path());
        let game_dir = tmp.path().join("Skyrim");
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::write(game_dir.join("SkyrimSE.exe"), b"x").unwrap();
        let registry = registry_with(vec![game("skyrim-se", "Skyrim", "SkyrimSE.exe")]);
        let added = add(&paths, &registry, &game_dir.display().to_string()).unwrap();

        std::fs::remove_dir_all(&game_dir).unwrap();
        let (installs, warnings) = discover_folders(&paths);
        assert!(installs.is_empty());
        assert_eq!(
            warnings.len(),
            1,
            "a gone folder is named, not dropped silently"
        );

        assert_eq!(remove_folder(&paths, &added.path).unwrap(), added.path);
        assert!(list_folders(&paths).unwrap().is_empty());
    }

    #[test]
    fn removing_a_folder_that_was_never_added_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(tmp.path());
        let error =
            remove_folder(&paths, &tmp.path().join("elsewhere").display().to_string()).unwrap_err();
        assert_eq!(error.code(), ERR_FOLDER_NOT_LISTED);
        assert_eq!(
            remove_folder(&paths, "  ").unwrap_err().code(),
            ERR_FOLDER_INVALID
        );
    }

    #[test]
    fn keys_ignore_case_and_trailing_separators_on_windows_only() {
        if cfg!(windows) {
            assert_eq!(
                folder_key("D:\\Games\\Skyrim\\"),
                folder_key("d:\\games\\skyrim")
            );
        } else {
            assert_ne!(folder_key("/games/Skyrim"), folder_key("/games/skyrim"));
        }
        assert_eq!(display_path(Path::new(r"\\?\D:\Games")), r"D:\Games");
        assert_eq!(
            display_path(Path::new(r"\\?\UNC\server\share")),
            r"\\server\share"
        );
    }
}
