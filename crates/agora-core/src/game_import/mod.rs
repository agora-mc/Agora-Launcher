//! Importing an existing Mod Organizer 2 setup (MASTER_SPEC §26.10).
//!
//! An import is read-only on the setup. Every mod folder the chosen profile lists, enabled or not,
//! is stored in the content store as it is on disk ("bytes first"), so a local edit survives and
//! nothing is downloaded. Then an instance is made, pinned to a base for the game's install, and its
//! content layers, generated layers, plugin list, INI copies and saves choice are set from the
//! profile. Nothing is written under the setup, and nothing under the player's Documents unless the
//! caller asks for saves to be copied.
//!
//! The module is split in two: [`mo2`] reads MO2's files and checks their names, and this module
//! decides what they mean for a game and what to store.

pub mod mo2;

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use agora_game_api::{glob_match, BaseReference, GameDefinition, GameId, RelPath, StoreId, ToolId};
use serde::{Deserialize, Serialize};

use crate::content_store::{self, ContentError, ContentSource, Mo2Provenance};
use crate::ctx::Ctx;
use crate::game_base::{BaseMode, BuildOptions, BuildOutcome};
use crate::game_deploy::{self, DeployError};
use crate::game_import::mo2::{Mo2Ini, ModState};
use crate::game_ini;
use crate::game_instance::{self, GameInstanceManifest, InstanceError, SavesChoice};
use crate::game_load_order::{self, LoadOrderError};
use crate::game_plugins::{self, PluginListError};
use crate::game_registry::{GameInventory, IdentifiedInstall, RuntimeResolution};
use crate::game_saves::{self, SavesError};
use crate::game_tools::{self, ToolError};
use crate::lock_manager::LockResource;

/// The `overwrite` files that no tool claims become one content layer with this name.
pub const OVERWRITE_LAYER_NAME: &str = "MO2 overwrite";

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("no ModOrganizer.ini at {0}")]
    NoSetup(PathBuf),
    #[error("cannot read {path}: {reason}")]
    Unreadable { path: PathBuf, reason: String },
    #[error("profile '{profile}' is not in this setup (its profiles are in {folder})")]
    NoProfile { profile: String, folder: PathBuf },
    #[error("the profile name '{profile}' is not a folder name: {reason}")]
    BadProfileName { profile: String, reason: String },
    #[error("the setup's game '{0}' matches no game Agora supports (no package names it for Mod Organizer 2)")]
    UnknownGame(String),
    #[error("the setup names no game folder (gamePath is missing from ModOrganizer.ini)")]
    NoGamePath,
    #[error("no discovered install of {game} is at {path}. Install the game, or point the setup at the folder the game is in; `agora games list` shows what Agora found")]
    NoInstall { game: String, path: PathBuf },
    #[error(
        "the install at {path} is not identified ({reasons}), so it cannot be pinned to a base"
    )]
    Unidentified { path: PathBuf, reasons: String },
    #[error("this setup and profile were already imported as instance '{instance}'. Give --name to import them again as a new instance")]
    AlreadyImported { instance: String },
    #[error("the setup's mod folder cannot be read: {0}")]
    ModsUnreadable(String),
    #[error("the setup has no modlist.txt for profile '{0}'")]
    NoModlist(String),
    #[error("the overwrite folder cannot be read: {0}")]
    OverwriteUnreadable(String),
    #[error("cannot store mod '{mod_folder}': {reason}")]
    Store { mod_folder: String, reason: String },
    #[error(transparent)]
    Content(#[from] ContentError),
    #[error(transparent)]
    Instance(#[from] InstanceError),
    #[error(transparent)]
    Deploy(#[from] DeployError),
    #[error(transparent)]
    Tool(#[from] ToolError),
    #[error(transparent)]
    PluginList(#[from] PluginListError),
    #[error(transparent)]
    LoadOrder(#[from] LoadOrderError),
    #[error(transparent)]
    Saves(#[from] SavesError),
    #[error(transparent)]
    Lock(#[from] crate::error::LauncherError),
    #[error(transparent)]
    Ini(#[from] game_ini::IniError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

/// Recorded on an instance that came from an import, so a second import of the same setup and
/// profile can be refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportRecord {
    /// `mo2` for now.
    pub kind: String,
    pub setup: String,
    pub profile: String,
    pub imported_at_unix_ms: i64,
}

// ---------------------------------------------------------------------------
// Finding setups
// ---------------------------------------------------------------------------

/// One MO2 setup found on this machine, without reading its mods.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupFound {
    pub ini_path: PathBuf,
    pub game_name: Option<String>,
    pub profiles: Vec<String>,
    /// Folders in the setup's `mods` folder. `None` when that folder cannot be read.
    pub mod_count: Option<usize>,
    /// Where it was found: the usual place, or the drive it was on.
    pub found_by: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanReport {
    pub setups: Vec<SetupFound>,
    pub warnings: Vec<String>,
}

/// Look for MO2 setups: the instance folders under `%LOCALAPPDATA%\ModOrganizer`, and a shallow scan
/// of each fixed drive for `ModOrganizer.ini` (to depth 3, skipping `Windows`, `Program Files*` and
/// `$Recycle.Bin`). The mods are not read; only the setup's own folders are listed.
pub fn scan() -> ScanReport {
    let mut report = ScanReport::default();
    let mut seen: HashSet<String> = HashSet::new();
    let mut found: Vec<(PathBuf, String)> = Vec::new();

    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let root = PathBuf::from(local).join("ModOrganizer");
        if let Ok(entries) = std::fs::read_dir(&root) {
            for entry in entries.flatten() {
                let ini = entry.path().join("ModOrganizer.ini");
                if ini.is_file() {
                    found.push((ini, "the usual place (ModOrganizer folder)".to_string()));
                }
            }
        }
    }

    for drive in fixed_drives() {
        scan_dir(&drive, 0, &mut found);
    }

    for (ini, how) in found {
        let key = ini.to_string_lossy().to_ascii_lowercase();
        if !seen.insert(key) {
            continue;
        }
        report.setups.push(summarise(&ini, how));
    }
    report.setups.sort_by(|a, b| a.ini_path.cmp(&b.ini_path));
    report
}

fn summarise(ini_path: &Path, found_by: String) -> SetupFound {
    let text = std::fs::read(ini_path)
        .map(|b| String::from_utf8_lossy(&b).to_string())
        .unwrap_or_default();
    let dir = ini_path.parent().map(Path::to_path_buf).unwrap_or_default();
    let ini = mo2::parse_ini(&text, dir);
    let profiles = list_dirs(&ini.profiles);
    let mod_count = std::fs::read_dir(&ini.mods)
        .ok()
        .map(|rd| rd.flatten().filter(|e| e.path().is_dir()).count());
    SetupFound {
        ini_path: ini_path.to_path_buf(),
        game_name: ini.game_name,
        profiles,
        mod_count,
        found_by,
    }
}

fn list_dirs(path: &Path) -> Vec<String> {
    let mut names: Vec<String> = match std::fs::read_dir(path) {
        Ok(rd) => rd
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect(),
        Err(_) => Vec::new(),
    };
    names.sort();
    names
}

const SCAN_DEPTH: usize = 3;
/// Folders the drive scan does not enter: Windows, the recycle bin, and every `Program Files*`.
const SKIPPED_DIRS: [&str; 2] = ["windows", "$recycle.bin"];

fn scan_dir(dir: &Path, depth: usize, found: &mut Vec<(PathBuf, String)>) {
    let ini = dir.join("ModOrganizer.ini");
    if ini.is_file() {
        found.push((ini, format!("drive scan ({})", dir.display())));
    }
    if depth >= SCAN_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.is_dir() || content_store::is_reparse_point_or_symlink(&meta) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if SKIPPED_DIRS.contains(&name.as_str()) || name.starts_with("program files") {
            continue;
        }
        scan_dir(&path, depth + 1, found);
    }
}

#[cfg(windows)]
fn fixed_drives() -> Vec<PathBuf> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
    // GetDriveType's DRIVE_FIXED: a hard disk, not removable, network or optical.
    const DRIVE_FIXED: u32 = 3;
    let mut out = Vec::new();
    for letter in b'A'..=b'Z' {
        let root = format!("{}:\\", letter as char);
        let wide: Vec<u16> = std::ffi::OsStr::new(&root)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: `wide` is a valid, NUL-terminated UTF-16 string that lives across the call.
        let kind = unsafe { GetDriveTypeW(wide.as_ptr()) };
        if kind == DRIVE_FIXED {
            out.push(PathBuf::from(root));
        }
    }
    out
}

#[cfg(not(windows))]
fn fixed_drives() -> Vec<PathBuf> {
    Vec::new()
}

// ---------------------------------------------------------------------------
// Planning
// ---------------------------------------------------------------------------

/// What an import was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mo2Request {
    /// The setup's `ModOrganizer.ini`.
    pub ini_path: PathBuf,
    pub profile: String,
    /// The new instance's name. Required to import a setup and profile a second time.
    pub name: Option<String>,
    /// Copy the profile's saves into the instance's own save folder (under Documents).
    pub copy_saves: bool,
}

/// A mod folder the profile lists, and what reading it found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedMod {
    /// 1-based line of `modlist.txt`.
    pub line: usize,
    pub folder: String,
    pub enabled: bool,
    pub path: PathBuf,
    pub files: usize,
    pub bytes: u64,
    pub provenance: Mo2Provenance,
    /// Plugins at the mod's root, which the game loads from `Data`.
    pub plugin_files: Vec<String>,
    /// Whether the mod's top-level `meta.ini` was left out of the stored files (it is MO2's
    /// metadata; its claims are still read as provenance).
    pub meta_ini_excluded: bool,
}

/// The files of a mod folder that are stored as its content. A `meta.ini` at the top of the folder
/// is MO2's own metadata, so it is left out (the caller reads it as provenance). A `meta.ini`
/// deeper in the folder is the mod's content and stays. Returns the files and whether one was left
/// out.
fn collect_mod_files(path: &Path) -> Result<(Vec<(RelPath, PathBuf)>, bool), ContentError> {
    let all = content_store::collect_folder_files(path)?;
    let mut kept = Vec::with_capacity(all.len());
    let mut excluded = false;
    for (rel, abs) in all {
        let text = rel.as_str();
        if !text.contains('/') && text.eq_ignore_ascii_case("meta.ini") {
            excluded = true;
            continue;
        }
        kept.push((rel, abs));
    }
    Ok((kept, excluded))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem {
    /// The `modlist.txt` line, or `None` for a problem with another file.
    pub line: Option<usize>,
    pub entry: String,
    pub reason: String,
}

/// The files a tool claims in `overwrite`, as one generated layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratedPlan {
    pub tool: String,
    pub name: String,
    pub files: usize,
    pub bytes: u64,
    /// A sample of the game-relative paths (the whole list is in the copy made at import).
    pub sample: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverwritePlan {
    /// The setup's `overwrite` folder, as the setup's settings name it.
    pub folder: PathBuf,
    pub present: bool,
    pub files: usize,
    pub bytes: u64,
    pub generated: Vec<GeneratedPlan>,
    pub rest_files: usize,
    pub rest_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginLinePlan {
    pub name: String,
    pub active: bool,
    pub managed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginPlan {
    /// Whether the game keeps a plugin list at all.
    pub kept: bool,
    pub lines: Vec<PluginLinePlan>,
    pub active: usize,
    pub locked: Vec<String>,
    /// Plugins the game always loads, which MO2 lists but Agora does not write.
    pub implicit_skipped: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IniPlan {
    pub name: String,
    pub instance_path: String,
    pub from: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavesPlan {
    /// The profile keeps its own saves (`LocalSaves=true`).
    pub local: bool,
    pub files: usize,
    pub bytes: u64,
    pub from: Option<PathBuf>,
    /// Where the saves go: the instance's own save folder. Known once the instance exists.
    pub to: Option<PathBuf>,
    pub note: String,
}

/// Everything an import would do, read from the setup and checked. Nothing is stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Mo2Plan {
    pub setup: PathBuf,
    pub profile: String,
    pub mo2_game_name: String,
    pub game: GameId,
    pub install_id: String,
    pub install_location: PathBuf,
    pub store: String,
    pub instance_name: String,
    /// Enabled and disabled mods, highest priority first (the order of `modlist.txt`).
    pub mods: Vec<PlannedMod>,
    pub mods_enabled: usize,
    pub mods_disabled: usize,
    pub files: usize,
    pub bytes: u64,
    /// Top-level `meta.ini` files left out of the stored content, across all mods.
    pub meta_ini_excluded: usize,
    pub separators: usize,
    pub unmanaged: Vec<String>,
    pub problems: Vec<Problem>,
    pub overwrite: OverwritePlan,
    pub plugins: PluginPlan,
    pub local_settings: bool,
    pub inis: Vec<IniPlan>,
    pub saves: SavesPlan,
    pub warnings: Vec<String>,
}

/// The `Data` folder the game's content mounts at, from its content layout.
fn mount_point(def: &GameDefinition) -> String {
    def.content_layout
        .as_ref()
        .map(|l| l.data_path.as_str().to_string())
        .unwrap_or_else(|| "Data".to_string())
}

fn normalise_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

fn read_text_optional(path: &Path) -> Result<Option<String>, ImportError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(String::from_utf8_lossy(&bytes).to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(ImportError::Unreadable {
            path: path.to_path_buf(),
            reason: e.to_string(),
        }),
    }
}

/// Plan an import: read the setup, match its game to an install, and list what would be stored and
/// set up. Reads files only; this is also what `--dry-run` shows.
pub fn plan(
    ctx: &Ctx,
    inventory: &GameInventory,
    request: &Mo2Request,
) -> Result<Mo2Plan, ImportError> {
    let ini_path = &request.ini_path;
    if !ini_path.is_file() {
        return Err(ImportError::NoSetup(ini_path.clone()));
    }
    let ini_text = read_text_optional(ini_path)?.unwrap_or_default();
    let ini_dir = ini_path.parent().map(Path::to_path_buf).unwrap_or_default();
    let ini: Mo2Ini = mo2::parse_ini(&ini_text, ini_dir);
    let mut warnings = ini.warnings.clone();

    // The game: its MO2 name, then the install at the setup's gamePath.
    let mo2_game = ini
        .game_name
        .clone()
        .ok_or_else(|| ImportError::UnknownGame("(none)".to_string()))?;
    let def = ctx
        .games
        .games()
        .find(|g| {
            g.mo2_game_name
                .as_deref()
                .is_some_and(|n| n.eq_ignore_ascii_case(&mo2_game))
        })
        .ok_or_else(|| ImportError::UnknownGame(mo2_game.clone()))?;
    let game_path = ini.game_path.clone().ok_or(ImportError::NoGamePath)?;
    let game_path_buf = PathBuf::from(&game_path);
    let install: &IdentifiedInstall = inventory
        .installs
        .iter()
        .find(|i| {
            i.game == def.id
                && normalise_path(&i.discovered.location) == normalise_path(&game_path_buf)
        })
        .ok_or_else(|| ImportError::NoInstall {
            game: def.name.clone(),
            path: game_path_buf.clone(),
        })?;
    if let RuntimeResolution::Unidentified { reasons } = &install.runtime {
        return Err(ImportError::Unidentified {
            path: game_path_buf,
            reasons: reasons.join("; "),
        });
    }

    // The profile.
    RelPath::new(&request.profile).map_err(|_| ImportError::BadProfileName {
        profile: request.profile.clone(),
        reason: "it is not a plain relative folder name".into(),
    })?;
    if let Err(reason) = mo2::check_mod_folder_name(&request.profile) {
        return Err(ImportError::BadProfileName {
            profile: request.profile.clone(),
            reason,
        });
    }
    let profile_dir = ini.profiles.join(&request.profile);
    if !profile_dir.is_dir() {
        return Err(ImportError::NoProfile {
            profile: request.profile.clone(),
            folder: ini.profiles.clone(),
        });
    }
    let modlist = read_text_optional(&profile_dir.join("modlist.txt"))?
        .ok_or_else(|| ImportError::NoModlist(request.profile.clone()))?;
    let settings = read_text_optional(&profile_dir.join("settings.ini"))?
        .map(|t| mo2::parse_profile_settings(&t))
        .unwrap_or_else(|| {
            warnings.push(
                "the profile has no settings.ini: its own INIs and saves are treated as off".into(),
            );
            mo2::ProfileSettings::default()
        });

    // The mods, in priority order.
    let mut mods: Vec<PlannedMod> = Vec::new();
    let mut problems: Vec<Problem> = Vec::new();
    let mut separators = 0usize;
    let mut unmanaged: Vec<String> = Vec::new();
    let mut seen_names: HashSet<String> = HashSet::new();
    for line in mo2::parse_modlist(&modlist) {
        if line.name.to_ascii_lowercase().ends_with("_separator") {
            separators += 1;
            continue;
        }
        match line.state {
            ModState::Unmanaged => {
                unmanaged.push(line.name.clone());
                continue;
            }
            ModState::Unknown => {
                problems.push(Problem {
                    line: Some(line.line),
                    entry: line.name.clone(),
                    reason: "not a line MO2 writes (expected a +, - or * prefix); skipped".into(),
                });
                continue;
            }
            ModState::Enabled | ModState::Disabled => {}
        }
        if let Err(reason) = mo2::check_mod_folder_name(&line.name) {
            problems.push(Problem {
                line: Some(line.line),
                entry: line.name.clone(),
                reason: format!("{reason}; not read"),
            });
            continue;
        }
        if !seen_names.insert(line.name.to_ascii_lowercase()) {
            problems.push(Problem {
                line: Some(line.line),
                entry: line.name.clone(),
                reason: "listed twice in modlist.txt; the second line is skipped".into(),
            });
            continue;
        }
        let path = ini.mods.join(&line.name);
        match std::fs::symlink_metadata(&path) {
            Err(_) => {
                problems.push(Problem {
                    line: Some(line.line),
                    entry: line.name.clone(),
                    reason: "the mod's folder is missing from the mods folder; skipped".into(),
                });
                continue;
            }
            Ok(meta) => {
                if !meta.is_dir() || content_store::is_reparse_point_or_symlink(&meta) {
                    problems.push(Problem {
                        line: Some(line.line),
                        entry: line.name.clone(),
                        reason: "not a plain folder (a file, a link or a junction); skipped".into(),
                    });
                    continue;
                }
            }
        }
        let (files, meta_ini_excluded) = match collect_mod_files(&path) {
            Ok(found) => found,
            Err(e) => {
                problems.push(Problem {
                    line: Some(line.line),
                    entry: line.name.clone(),
                    reason: format!("cannot be read: {e}; skipped"),
                });
                continue;
            }
        };
        if files.is_empty() {
            problems.push(Problem {
                line: Some(line.line),
                entry: line.name.clone(),
                reason: "the folder holds no files; skipped".into(),
            });
            continue;
        }
        let mut bytes = 0u64;
        let mut plugin_files = Vec::new();
        for (rel, abs) in &files {
            bytes += std::fs::metadata(abs).map(|m| m.len()).unwrap_or(0);
            let rel_text = rel.as_str();
            if !rel_text.contains('/') && mo2::is_plugin_file(rel_text) {
                plugin_files.push(rel_text.to_string());
            }
        }
        let provenance = read_provenance(&path);
        mods.push(PlannedMod {
            line: line.line,
            folder: line.name.clone(),
            enabled: line.state == ModState::Enabled,
            path,
            files: files.len(),
            bytes,
            provenance,
            plugin_files,
            meta_ini_excluded,
        });
    }

    // Refuse a second import of the same setup and profile, unless it has a new name.
    if request.name.is_none() {
        if let Some(instance) = find_import(ctx, ini_path, &request.profile)? {
            return Err(ImportError::AlreadyImported { instance });
        }
    }

    // `overwrite`: the tool outputs, and the rest.
    let overwrite_dir = ini.overwrite.clone();
    let overwrite_present = overwrite_dir.is_dir();
    // The game's own data folder: a plugin the install already has is the game's, never managed.
    let base_data = install.discovered.location.join(mount_point(def));
    let mut overwrite_plugins: Vec<String> = Vec::new();
    let overwrite = if overwrite_present {
        let files = content_store::collect_folder_files(&overwrite_dir)
            .map_err(|e| ImportError::OverwriteUnreadable(e.to_string()))?;
        let split = split_overwrite(ctx, def, files);
        // The rest of overwrite becomes the "MO2 overwrite" content layer, which deploys its
        // root plugins into the plugin folder. Those are managed, as an enabled mod's are. A
        // plugin the install already has is the game's own, so it stays unmanaged. Generated
        // files are not counted: a deploy does not treat their plugins as deployed by a layer.
        overwrite_plugins = split
            .rest
            .iter()
            .map(|(rel, _)| rel.as_str())
            .filter(|rel| !rel.contains('/') && mo2::is_plugin_file(rel))
            .filter(|rel| !base_data.join(rel).is_file())
            .map(str::to_string)
            .collect();
        let mut total_files = 0usize;
        let mut total_bytes = 0u64;
        let mut generated = Vec::new();
        for (tool, claimed) in &split.generated {
            let mut bytes = 0u64;
            for (_, _, abs) in claimed {
                bytes += std::fs::metadata(abs).map(|m| m.len()).unwrap_or(0);
            }
            total_files += claimed.len();
            total_bytes += bytes;
            generated.push(GeneratedPlan {
                tool: tool.to_string(),
                name: ctx
                    .games
                    .tool(&def.id, tool)
                    .map(|t| t.name.clone())
                    .unwrap_or_else(|| tool.to_string()),
                files: claimed.len(),
                bytes,
                sample: claimed.iter().take(20).map(|(g, _, _)| g.clone()).collect(),
            });
        }
        let mut rest_bytes = 0u64;
        for (_, abs) in &split.rest {
            rest_bytes += std::fs::metadata(abs).map(|m| m.len()).unwrap_or(0);
        }
        OverwritePlan {
            folder: overwrite_dir.clone(),
            present: true,
            files: total_files + split.rest.len(),
            bytes: total_bytes + rest_bytes,
            generated,
            rest_files: split.rest.len(),
            rest_bytes,
        }
    } else {
        OverwritePlan {
            folder: overwrite_dir.clone(),
            present: false,
            files: 0,
            bytes: 0,
            generated: Vec::new(),
            rest_files: 0,
            rest_bytes: 0,
        }
    };

    let store = install.discovered.store.clone();
    let plugins = plan_plugins(
        def,
        &profile_dir,
        &mods,
        &overwrite_plugins,
        &base_data,
        &mut warnings,
    )?;
    let inis = plan_inis(def, &store, &profile_dir, settings.local_settings);
    let saves = plan_saves(def, &store, &profile_dir, settings.local_saves)?;

    let enabled = mods.iter().filter(|m| m.enabled).count();
    let file_total: usize = mods.iter().map(|m| m.files).sum::<usize>() + overwrite.files;
    let byte_total: u64 = mods.iter().map(|m| m.bytes).sum::<u64>() + overwrite.bytes;
    let meta_ini_excluded = mods.iter().filter(|m| m.meta_ini_excluded).count();

    Ok(Mo2Plan {
        setup: ini_path.clone(),
        profile: request.profile.clone(),
        mo2_game_name: mo2_game,
        game: def.id.clone(),
        install_id: install.install_id.as_str().to_string(),
        install_location: install.discovered.location.clone(),
        store: store.as_str().to_string(),
        instance_name: request
            .name
            .clone()
            .unwrap_or_else(|| format!("{} (MO2 import)", request.profile)),
        mods_enabled: enabled,
        mods_disabled: mods.len() - enabled,
        mods,
        files: file_total,
        bytes: byte_total,
        meta_ini_excluded,
        separators,
        unmanaged,
        problems,
        overwrite,
        plugins,
        local_settings: settings.local_settings,
        inis,
        saves,
        warnings,
    })
}

/// Read a mod's `meta.ini` as its claimed provenance. A missing file is recorded as absent; one
/// that cannot be read as settings is recorded as unreadable. Neither stops the import.
fn read_provenance(mod_dir: &Path) -> Mo2Provenance {
    let path = mod_dir.join("meta.ini");
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Mo2Provenance::Absent,
        Err(e) => {
            return Mo2Provenance::Unreadable {
                reason: e.to_string(),
            }
        }
    };
    match mo2::parse_meta_ini(&bytes) {
        Ok(meta) => Mo2Provenance::Claimed {
            modid: meta.modid,
            version: meta.version,
            installation_file: meta.installation_file,
            repository: meta.repository,
            game_name: meta.game_name,
            note: "provenance claimed, not verified".into(),
        },
        Err(reason) => Mo2Provenance::Unreadable { reason },
    }
}

/// The `overwrite` files split by the tool that claims them. A file belongs to the first tool (in
/// the game's declared order) whose output patterns match its game path, `Data/...`.
struct OverwriteSplit {
    /// Per tool: (game-relative path, its name in overwrite, the file).
    generated: BTreeMap<ToolId, Vec<(String, String, PathBuf)>>,
    rest: Vec<(RelPath, PathBuf)>,
}

fn split_overwrite(
    ctx: &Ctx,
    def: &GameDefinition,
    files: Vec<(RelPath, PathBuf)>,
) -> OverwriteSplit {
    let mount = mount_point(def);
    let patterns: Vec<(ToolId, Vec<String>)> = def
        .tool_ids
        .iter()
        .filter_map(|id| {
            ctx.games.tool(&def.id, id).map(|t| {
                (
                    id.clone(),
                    t.output_patterns
                        .iter()
                        .map(|p| p.to_ascii_lowercase())
                        .collect(),
                )
            })
        })
        .collect();
    let mut generated: BTreeMap<ToolId, Vec<(String, String, PathBuf)>> = BTreeMap::new();
    let mut rest = Vec::new();
    for (rel, abs) in files {
        let game_path = if mount.is_empty() {
            rel.as_str().to_string()
        } else {
            format!("{mount}/{}", rel.as_str())
        };
        let lower = game_path.to_ascii_lowercase();
        let owner = patterns
            .iter()
            .find(|(_, pats)| pats.iter().any(|p| !p.is_empty() && glob_match(p, &lower)));
        match owner {
            Some((tool, _)) => generated.entry(tool.clone()).or_default().push((
                game_path,
                rel.as_str().to_string(),
                abs,
            )),
            None => rest.push((rel, abs)),
        }
    }
    OverwriteSplit { generated, rest }
}

fn plan_plugins(
    def: &GameDefinition,
    profile_dir: &Path,
    mods: &[PlannedMod],
    overwrite_plugins: &[String],
    base_data: &Path,
    warnings: &mut Vec<String>,
) -> Result<PluginPlan, ImportError> {
    let Some(rule) = &def.plugin_list else {
        return Ok(PluginPlan {
            kept: false,
            lines: Vec::new(),
            active: 0,
            locked: Vec::new(),
            implicit_skipped: Vec::new(),
            notes: vec!["the game keeps no plugin list".into()],
        });
    };
    let mut notes = Vec::new();
    let order = match read_text_optional(&profile_dir.join("loadorder.txt"))? {
        Some(text) => mo2::parse_loadorder(&text),
        None => {
            notes.push("the profile has no loadorder.txt; the order comes from plugins.txt".into());
            Vec::new()
        }
    };
    let listed = match read_text_optional(&profile_dir.join("plugins.txt"))? {
        Some(text) => mo2::parse_plugins(&text),
        None => {
            warnings.push("the profile has no plugins.txt; no plugin is active".into());
            Vec::new()
        }
    };
    let active_names: HashSet<String> = listed
        .iter()
        .filter(|p| p.active)
        .map(|p| p.name.to_ascii_lowercase())
        .collect();
    let implicit: HashSet<String> = rule
        .implicit
        .iter()
        .map(|n| n.to_ascii_lowercase())
        .collect();

    let managed: Vec<String> = {
        let mut seen = HashSet::new();
        mods.iter()
            .filter(|m| m.enabled)
            .flat_map(|m| m.plugin_files.iter().cloned())
            // A plugin the install already has is the game's, so it is never managed.
            .filter(|n| !base_data.join(n).is_file())
            .filter(|n| seen.insert(n.to_ascii_lowercase()))
            .collect()
    };
    let managed_keys: HashSet<String> = managed.iter().map(|n| n.to_ascii_lowercase()).collect();
    // The overwrite plugins are managed only where MO2 lists them: a line is never added for
    // one, so a plugin MO2 does not list keeps the import's usual handling.
    let overwrite_keys: HashSet<String> = overwrite_plugins
        .iter()
        .map(|n| n.to_ascii_lowercase())
        .collect();

    let mut lines: Vec<PluginLinePlan> = Vec::new();
    let mut implicit_skipped = Vec::new();
    let mut have: HashSet<String> = HashSet::new();
    let names = order
        .iter()
        .cloned()
        .chain(listed.iter().map(|p| p.name.clone()));
    for name in names {
        let key = name.to_ascii_lowercase();
        if implicit.contains(&key) {
            if !implicit_skipped
                .iter()
                .any(|n: &String| n.eq_ignore_ascii_case(&name))
            {
                implicit_skipped.push(name);
            }
            continue;
        }
        if !have.insert(key.clone()) {
            continue;
        }
        lines.push(PluginLinePlan {
            name,
            active: active_names.contains(&key),
            managed: managed_keys.contains(&key) || overwrite_keys.contains(&key),
        });
    }
    for name in &managed {
        if have.insert(name.to_ascii_lowercase()) {
            notes.push(format!(
                "{name} is in an enabled mod but not in the profile's plugin lists; it is kept inactive"
            ));
            lines.push(PluginLinePlan {
                name: name.clone(),
                active: false,
                managed: true,
            });
        }
    }
    let mut locked = Vec::new();
    if let Some(text) = read_text_optional(&profile_dir.join("lockedorder.txt"))? {
        for lock in mo2::parse_lockedorder(&text) {
            match lines
                .iter()
                .find(|l| l.name.eq_ignore_ascii_case(&lock.name))
            {
                Some(line) => locked.push(line.name.clone()),
                None => notes.push(format!(
                    "the lock on {} names no plugin in the profile; it is not kept",
                    lock.name
                )),
            }
            if lock.priority.is_none() {
                notes.push(format!("the lock on {} has no usable priority", lock.name));
            }
        }
    }
    Ok(PluginPlan {
        kept: true,
        active: lines.iter().filter(|l| l.active).count(),
        lines,
        locked,
        implicit_skipped,
        notes,
    })
}

fn plan_inis(
    def: &GameDefinition,
    store: &StoreId,
    profile_dir: &Path,
    local: bool,
) -> Vec<IniPlan> {
    if !local {
        return Vec::new();
    }
    let mut out = Vec::new();
    for mapping in def.user_files.iter().filter(|m| m.applies_to_store(store)) {
        let instance_path = mapping.instance_path.as_str();
        if !instance_path.to_ascii_lowercase().ends_with(".ini") {
            continue;
        }
        let name = instance_path.rsplit('/').next().unwrap_or(instance_path);
        let Ok(entries) = std::fs::read_dir(profile_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(name)
                && entry.path().is_file()
            {
                out.push(IniPlan {
                    name: name.to_string(),
                    instance_path: instance_path.to_string(),
                    from: entry.path(),
                });
                break;
            }
        }
    }
    out
}

fn plan_saves(
    def: &GameDefinition,
    store: &StoreId,
    profile_dir: &Path,
    local: bool,
) -> Result<SavesPlan, ImportError> {
    if !local {
        return Ok(SavesPlan {
            local: false,
            files: 0,
            bytes: 0,
            from: None,
            to: None,
            note: "the profile shares the game's saves".into(),
        });
    }
    if game_saves::rule_for(def, store).is_none() {
        return Ok(SavesPlan {
            local: true,
            files: 0,
            bytes: 0,
            from: None,
            to: None,
            note: "the game has no save-folder setting for this store, so the saves stay shared"
                .into(),
        });
    }
    let from = profile_dir.join("saves");
    let mut files = 0usize;
    let mut bytes = 0u64;
    if let Ok(entries) = std::fs::read_dir(&from) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                if meta.is_file() {
                    files += 1;
                    bytes += meta.len();
                }
            }
        }
    }
    Ok(SavesPlan {
        local: true,
        files,
        bytes,
        from: Some(from),
        to: None,
        note: String::new(),
    })
}

/// The instance an earlier import of this setup and profile made, if there is one.
fn find_import(ctx: &Ctx, ini_path: &Path, profile: &str) -> Result<Option<String>, ImportError> {
    let setup = normalise_path(ini_path);
    for record in game_instance::list(ctx)? {
        let manifest = match game_instance::get_manifest(ctx, &record.instance_id) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if let Some(rec) = manifest.imported_from {
            if rec.kind == "mo2"
                && rec.profile == profile
                && normalise_path(Path::new(&rec.setup)) == setup
            {
                return Ok(Some(record.name.clone()));
            }
        }
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// Running
// ---------------------------------------------------------------------------

/// What a run did, after its plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Mo2RunReport {
    pub plan: Mo2Plan,
    /// The run stopped before the instance was made (`stop_after_mods`). Run it again to finish.
    pub interrupted: bool,
    pub stored_mods: usize,
    pub instance_id: Option<String>,
    pub base_id: Option<String>,
    pub base_built: Option<bool>,
    pub content_layers: usize,
    pub disabled_layers: usize,
    /// Mods whose bytes are the same as a layer already in the instance.
    pub duplicates: Vec<String>,
    pub generated_layers: Vec<String>,
    pub plugins_written: Option<usize>,
    /// Problems the game's rules find in the imported plugin order (MO2's order is kept as it is).
    pub plugin_findings: Vec<String>,
    pub inis_copied: Vec<String>,
    pub saves_copied: usize,
    pub saves_skipped_existing: usize,
    pub saves_folder: Option<PathBuf>,
    /// What still has to be done by hand, if anything.
    pub next_steps: Vec<String>,
}

/// Progress for a long import: mods stored so far, and bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportProgress {
    pub mods_done: usize,
    pub mods_total: usize,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

/// Test hooks: stop after storing this many mods, as an interruption would.
#[derive(Debug, Clone, Copy, Default)]
pub struct RunOptions {
    pub stop_after_mods: Option<usize>,
}

/// Run an import: store the bytes, make the instance, and set it up from the profile. The plan is
/// made first, so a refusal happens before anything is written.
pub fn run(
    ctx: &Ctx,
    inventory: &GameInventory,
    request: &Mo2Request,
    options: RunOptions,
    progress: &dyn Fn(ImportProgress),
) -> Result<Mo2RunReport, ImportError> {
    let plan = plan(ctx, inventory, request)?;
    let install = inventory
        .installs
        .iter()
        .find(|i| {
            i.game == plan.game
                && normalise_path(&i.discovered.location) == normalise_path(&plan.install_location)
        })
        .ok_or_else(|| {
            ImportError::Other("the install vanished between planning and running".into())
        })?
        .clone();
    let def = ctx
        .games
        .game(&plan.game)
        .cloned()
        .ok_or_else(|| ImportError::Other("the game definition is missing".into()))?;
    let mut report = Mo2RunReport {
        plan: plan.clone(),
        interrupted: false,
        stored_mods: 0,
        instance_id: None,
        base_id: None,
        base_built: None,
        content_layers: 0,
        disabled_layers: 0,
        duplicates: Vec::new(),
        generated_layers: Vec::new(),
        plugins_written: None,
        plugin_findings: Vec::new(),
        inis_copied: Vec::new(),
        saves_copied: 0,
        saves_skipped_existing: 0,
        saves_folder: None,
        next_steps: Vec::new(),
    };

    // 1. Bytes first: every listed mod, as it is on disk.
    let mods_total = plan.mods.len();
    let mut stored: Vec<(usize, String)> = Vec::new();
    let mut bytes_done = 0u64;
    for (index, planned) in plan.mods.iter().enumerate() {
        let source = ContentSource::Mo2Import {
            setup: plan.setup.to_string_lossy().to_string(),
            mo2_folder: planned.folder.clone(),
            provenance: planned.provenance.clone(),
            added_at_unix_ms: content_store::now_unix_ms(),
        };
        let store_error = |e: ContentError| ImportError::Store {
            mod_folder: planned.folder.clone(),
            reason: e.to_string(),
        };
        let (files, _) = collect_mod_files(&planned.path).map_err(store_error)?;
        let outcome = content_store::add_collected(ctx, &planned.folder, files, source)
            .map_err(store_error)?;
        stored.push((index, outcome.item().item_id.clone()));
        bytes_done += planned.bytes;
        report.stored_mods += 1;
        progress(ImportProgress {
            mods_done: index + 1,
            mods_total,
            bytes_done,
            bytes_total: plan.bytes,
        });
        if options
            .stop_after_mods
            .is_some_and(|n| report.stored_mods >= n)
            && index + 1 < mods_total
        {
            report.interrupted = true;
            return Ok(report);
        }
    }

    // `overwrite`: the rest becomes one content layer; the tools' files were split off below.
    let overwrite_split = if plan.overwrite.present {
        let files = content_store::collect_folder_files(&plan.overwrite.folder)
            .map_err(|e| ImportError::OverwriteUnreadable(e.to_string()))?;
        Some(split_overwrite(ctx, &def, files))
    } else {
        None
    };
    let overwrite_item = match &overwrite_split {
        Some(split) if !split.rest.is_empty() => {
            let source = ContentSource::Mo2Import {
                setup: plan.setup.to_string_lossy().to_string(),
                mo2_folder: "overwrite".into(),
                provenance: Mo2Provenance::Absent,
                added_at_unix_ms: content_store::now_unix_ms(),
            };
            let files = split.rest.clone();
            let outcome = content_store::add_collected(ctx, OVERWRITE_LAYER_NAME, files, source)?;
            Some(outcome.item().item_id.clone())
        }
        _ => None,
    };

    // 2. The instance, made only now that everything is stored.
    let outcome = game_instance::create_with_options(
        ctx,
        &install,
        &def,
        &plan.instance_name,
        None,
        BaseMode::Linked,
        BuildOptions::default(),
        &|_| {},
    )?;
    let instance_id = outcome.instance_id.clone();
    report.instance_id = Some(instance_id.clone());
    report.base_id = match &outcome.base {
        BaseReference::Pinned { id, .. } => Some(id.clone()),
        BaseReference::Unpinned { .. } => None,
    };
    report.base_built = outcome
        .build_outcome
        .as_ref()
        .map(|b| matches!(b, BuildOutcome::Built { .. }));

    // Everything from here sets the new instance up. If any of it fails, the half-made instance is
    // removed, so a second run starts clean rather than being refused as already imported.
    let populated: Result<(), ImportError> = (|| {
        let mut manifest: GameInstanceManifest = game_instance::get_manifest(ctx, &instance_id)?;
        manifest.imported_from = Some(ImportRecord {
            kind: "mo2".into(),
            setup: plan.setup.to_string_lossy().to_string(),
            profile: plan.profile.clone(),
            imported_at_unix_ms: content_store::now_unix_ms(),
        });
        game_deploy::write_instance_manifest_atomic(ctx, &instance_id, &manifest)?;

        // 3. Content layers, lowest priority first: the last line of modlist.txt is the lowest layer.
        let mount = mount_point(&def);
        let mount_arg = if mount.is_empty() {
            None
        } else {
            Some(mount.as_str())
        };
        for (index, item_id) in stored.iter().rev() {
            let planned = &plan.mods[*index];
            let existing = game_instance::get_manifest(ctx, &instance_id)?
            .layers
            .layers()
            .iter()
            .any(|l| matches!(&l.source, agora_game_api::LayerSource::Content { content } if content == item_id));
            if existing {
                report.duplicates.push(format!(
                    "{} (same bytes as a layer already here)",
                    planned.folder
                ));
                if planned.enabled {
                    game_deploy::set_content_enabled(ctx, &instance_id, item_id, true)?;
                }
                continue;
            }
            game_deploy::add_content(ctx, &instance_id, item_id, mount_arg, None)?;
            report.content_layers += 1;
            if !planned.enabled {
                game_deploy::set_content_enabled(ctx, &instance_id, item_id, false)?;
                report.disabled_layers += 1;
            }
        }
        if let Some(item_id) = &overwrite_item {
            game_deploy::add_content(ctx, &instance_id, item_id, mount_arg, None)?;
            report.content_layers += 1;
        }

        // 4. Generated layers, one per tool, from the files the tools claim.
        if let Some(split) = &overwrite_split {
            for (tool, claimed) in &split.generated {
                let files: Vec<(String, PathBuf)> = claimed
                    .iter()
                    .map(|(game, _, abs)| (game.clone(), abs.clone()))
                    .collect();
                game_tools::import_output(ctx, &instance_id, &def, tool, &files)?;
                report
                    .generated_layers
                    .push(format!("{tool} ({} files, inputs unknown)", files.len()));
            }
        }

        // 5. The plugin list: MO2's order and activation, and its locks.
        if plan.plugins.kept {
            let lines: Vec<(String, bool)> = plan
                .plugins
                .lines
                .iter()
                .map(|l| (l.name.clone(), l.active))
                .collect();
            let managed: Vec<String> = plan
                .plugins
                .lines
                .iter()
                .filter(|l| l.managed)
                .map(|l| l.name.clone())
                .collect();
            let _lock = ctx.lock_manager.acquire(
                LockResource::Instance(instance_id.clone()),
                "mo2-import-plugins",
            )?;
            game_plugins::import_list(
                ctx,
                &instance_id,
                &def,
                &lines,
                &managed,
                &plan.plugins.locked,
            )?;
            report.plugins_written = Some(lines.len());
            // The order MO2 kept is checked against the game's rules; a finding is reported, not fixed.
            let findings = game_load_order::check(ctx, &instance_id, &def)?;
            if !findings.is_empty() {
                report.plugin_findings = game_load_order::describe_findings(&findings)
                    .split("; ")
                    .map(str::to_string)
                    .collect();
            }
        }

        // 6. Local INIs: the profile's copies become the instance's copies.
        for ini in &plan.inis {
            let bytes = std::fs::read(&ini.from)?;
            let dest = game_ini::join_rel(
                &ctx.paths
                    .instance_dir(&instance_id)
                    .map_err(|e| ImportError::Other(e.to_string()))?,
                &ini.instance_path,
            );
            game_ini::write_atomic(&dest, &bytes)?;
            report.inis_copied.push(ini.name.clone());
        }

        // 7. Saves: the choice is `own`. Copying into Documents needs the flag.
        if plan.saves.local {
            game_saves::set_choice_inner(
                ctx,
                &instance_id,
                &def,
                SavesChoice::Own,
                request.copy_saves,
            )?;
            let rule = game_saves::rule_for(&def, &install.discovered.store).cloned();
            if let Some(rule) = rule {
                let folder = game_saves::own_folder(&rule, &instance_id)?;
                report.saves_folder = Some(folder.clone());
                if request.copy_saves {
                    if let Some(from) = &plan.saves.from {
                        if let Ok(entries) = std::fs::read_dir(from) {
                            for entry in entries.flatten() {
                                if !entry.path().is_file() {
                                    continue;
                                }
                                let dest = folder.join(entry.file_name());
                                if dest.exists() {
                                    report.saves_skipped_existing += 1;
                                    continue;
                                }
                                std::fs::copy(entry.path(), &dest)?;
                                report.saves_copied += 1;
                            }
                        }
                    }
                } else if plan.saves.files > 0 {
                    report.next_steps.push(format!(
                    "{} save file(s) were not copied (saves go under Documents, which needs --copy-saves). To copy them by hand, copy the files in {} into {}",
                    plan.saves.files,
                    plan.saves.from.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
                    folder.display()
                ));
                }
            }
        }

        Ok(())
    })();
    if let Err(error) = populated {
        let _ = game_instance::delete(ctx, &instance_id);
        return Err(error);
    }

    report.next_steps.extend(plan.problems.iter().map(|p| {
        format!(
            "line {}: {} was not imported: {}",
            p.line.map(|l| l.to_string()).unwrap_or_default(),
            p.entry,
            p.reason
        )
    }));
    Ok(report)
}
