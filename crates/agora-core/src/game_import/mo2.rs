//! Reading a Mod Organizer 2 setup's files (MASTER_SPEC §26.10). Everything here is pure: each
//! function takes the text of one of MO2's files and returns what it says. The files are read by
//! [`super`], which also decides what to do with the names they hold.
//!
//! **Every name in these files is untrusted.** A modlist line can name `..\..\Windows`, a drive, a
//! path with a separator, or a folder that is not there. [`check_mod_folder_name`] is the one gate a
//! name must pass before it is joined to the `mods` folder, and nothing else in the import builds a
//! path from a name.

use std::path::PathBuf;

/// MO2 stores its path settings in `[Settings]`, and the game settings in `[General]`.
const SETTINGS: &str = "Settings";
const GENERAL: &str = "General";
const BASE_DIR: &str = "%BASE_DIR%";

/// The keys this import reads from `ModOrganizer.ini`, with the paths resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mo2Ini {
    pub game_name: Option<String>,
    pub game_path: Option<String>,
    pub selected_profile: Option<String>,
    pub base: PathBuf,
    pub mods: PathBuf,
    pub profiles: PathBuf,
    pub overwrite: PathBuf,
    pub downloads: PathBuf,
    /// Settings that could not be used as written, such as an unknown `%VARIABLE%`.
    pub warnings: Vec<String>,
}

/// Parse `ModOrganizer.ini`. `ini_dir` is the folder holding the file, which is the base directory
/// unless the setup sets `base_directory`.
pub fn parse_ini(text: &str, ini_dir: PathBuf) -> Mo2Ini {
    let mut warnings = Vec::new();
    let get = |section: &str, key: &str| -> Option<String> {
        ini_value(text, section, key).map(|raw| decode_value(&raw))
    };

    let base = match get(SETTINGS, "base_directory").filter(|s| !s.trim().is_empty()) {
        Some(value) => PathBuf::from(value.trim()),
        None => ini_dir,
    };
    let base_text = base.to_string_lossy().to_string();
    let resolve = |key: &str, default: &str, warnings: &mut Vec<String>| -> PathBuf {
        let value = get(SETTINGS, key)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| format!("{BASE_DIR}/{default}"));
        let value = value.trim().replace(BASE_DIR, &base_text);
        if value.contains('%') {
            warnings.push(format!(
                "setting '{key}' uses a variable Agora does not know: {value}"
            ));
        }
        PathBuf::from(value)
    };
    let mods = resolve("mod_directory", "mods", &mut warnings);
    let profiles = resolve("profiles_directory", "profiles", &mut warnings);
    let overwrite = resolve("overwrite_directory", "overwrite", &mut warnings);
    let downloads = resolve("download_directory", "downloads", &mut warnings);

    Mo2Ini {
        game_name: get(GENERAL, "gameName").filter(|s| !s.trim().is_empty()),
        game_path: get(GENERAL, "gamePath").filter(|s| !s.trim().is_empty()),
        selected_profile: get(GENERAL, "selected_profile").filter(|s| !s.trim().is_empty()),
        base,
        mods,
        profiles,
        overwrite,
        downloads,
        warnings,
    }
}

/// The raw value of `key` in `[section]`, with the file's own quoting and escaping still in it.
fn ini_value(text: &str, section: &str, key: &str) -> Option<String> {
    let mut current = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            current = name.to_string();
            continue;
        }
        if !current.eq_ignore_ascii_case(section) {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            if k.trim() == key {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

/// The value MO2 means by a settings string: `@ByteArray(...)` wraps the text, and backslashes
/// are doubled in the file (`D:\\Games` is `D:\Games`).
pub fn decode_value(raw: &str) -> String {
    let inner = match raw
        .strip_prefix("@ByteArray(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        Some(inner) => inner,
        None => raw,
    };
    unescape(inner)
}

fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek() {
                Some('\\') => {
                    out.push('\\');
                    chars.next();
                }
                Some('"') => {
                    out.push('"');
                    chars.next();
                }
                _ => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// What a line of `modlist.txt` says about its mod. The first line is the highest priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModState {
    Enabled,
    Disabled,
    /// `*`: not managed by MO2 (the game's own DLC and Creation Club files, for instance).
    Unmanaged,
    /// A line MO2 would not write: neither `+`, `-` nor `*`.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModlistLine {
    /// 1-based line number in the file.
    pub line: usize,
    pub state: ModState,
    /// The name exactly as written after the prefix. Not yet checked: see [`check_mod_folder_name`].
    pub name: String,
}

/// Parse `modlist.txt`, in file order (highest priority first). Comments and blank lines are
/// skipped; the numbering of the rest is the file's own.
pub fn parse_modlist(text: &str) -> Vec<ModlistLine> {
    let mut out = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim_end_matches('\r');
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let (state, name) = match line.chars().next() {
            Some('+') => (ModState::Enabled, &line[1..]),
            Some('-') => (ModState::Disabled, &line[1..]),
            Some('*') => (ModState::Unmanaged, &line[1..]),
            _ => (ModState::Unknown, line),
        };
        out.push(ModlistLine {
            line: index + 1,
            state,
            name: name.to_string(),
        });
    }
    out
}

/// A plugin line of `plugins.txt` or `loadorder.txt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginLine {
    pub name: String,
    pub active: bool,
}

/// Parse `plugins.txt`: a `*` marks an active plugin, and a line without one is listed but
/// inactive.
pub fn parse_plugins(text: &str) -> Vec<PluginLine> {
    text.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| match l.strip_prefix('*') {
            Some(name) => PluginLine {
                name: name.trim().to_string(),
                active: true,
            },
            None => PluginLine {
                name: l.to_string(),
                active: false,
            },
        })
        .filter(|p| !p.name.is_empty())
        .collect()
}

/// Parse `loadorder.txt`: every plugin MO2 knows, active or not, in load order.
pub fn parse_loadorder(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.to_string())
        .collect()
}

/// One line of `lockedorder.txt`: `<plugin name>|<priority>`, where the priority is the plugin's
/// position MO2 keeps it at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockLine {
    pub line: usize,
    pub name: String,
    /// `None` when the priority is not a non-negative number.
    pub priority: Option<u64>,
}

/// Parse `lockedorder.txt` (MO2 writes `# This file was automatically generated by Mod
/// Organizer.` first, then one `name|priority` line per lock).
pub fn parse_lockedorder(text: &str) -> Vec<LockLine> {
    let mut out = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (name, priority) = match line.rsplit_once('|') {
            Some((name, priority)) => (name.trim(), priority.trim().parse::<u64>().ok()),
            None => (line, None),
        };
        out.push(LockLine {
            line: index + 1,
            name: name.to_string(),
            priority,
        });
    }
    out
}

/// The profile's `settings.ini`: whether it keeps its own INIs and its own saves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProfileSettings {
    pub local_settings: bool,
    pub local_saves: bool,
}

pub fn parse_profile_settings(text: &str) -> ProfileSettings {
    let flag = |key: &str| {
        ini_value(text, GENERAL, key)
            .map(|v| v.trim().eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    };
    ProfileSettings {
        local_settings: flag("LocalSettings"),
        local_saves: flag("LocalSaves"),
    }
}

/// What a mod's `meta.ini` says. These are claims: nothing here checks them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MetaIni {
    pub modid: Option<u64>,
    pub version: Option<String>,
    pub installation_file: Option<String>,
    pub repository: Option<String>,
    pub game_name: Option<String>,
    /// Keys that were present but could not be used.
    pub warnings: Vec<String>,
}

/// Parse a mod's `meta.ini`. A file that is not text, or has no `[General]` section, is an error
/// (the caller records it as unreadable and imports the mod anyway).
pub fn parse_meta_ini(bytes: &[u8]) -> Result<MetaIni, String> {
    if bytes.contains(&0) {
        return Err("the file holds binary data, not settings".to_string());
    }
    let text = String::from_utf8_lossy(bytes);
    if !text
        .lines()
        .any(|l| l.trim().eq_ignore_ascii_case("[General]"))
    {
        return Err("the file has no [General] section".to_string());
    }
    let mut meta = MetaIni::default();
    let value = |key: &str| ini_value(&text, GENERAL, key).filter(|v| !v.is_empty());
    if let Some(raw) = value("modid") {
        match raw.parse::<u64>() {
            // MO2 writes 0 for a mod with no Nexus id (generated output, for instance).
            Ok(0) => {}
            Ok(id) => meta.modid = Some(id),
            Err(_) => meta.warnings.push(format!("modid '{raw}' is not a number")),
        }
    }
    meta.version = value("version");
    meta.installation_file = value("installationFile").map(|v| decode_value(&v));
    meta.repository = value("repository");
    meta.game_name = value("gameName");
    Ok(meta)
}

/// Whether a name is safe to join to the `mods` folder: one plain folder name, with no separator,
/// no drive or stream marker, no `.` or `..`, no control character, no trailing dot or space, and
/// not a Windows device name. Returns the reason it is not.
pub fn check_mod_folder_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("the name is empty".into());
    }
    if name == "." || name == ".." {
        return Err(format!("'{name}' is not a folder name"));
    }
    if let Some(c) = name.chars().find(|c| matches!(c, '/' | '\\' | ':')) {
        return Err(format!(
            "the name contains '{c}', so it is a path, not a folder in mods"
        ));
    }
    if name.chars().any(|c| c.is_control()) {
        return Err("the name contains a control character".into());
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Err("the name ends in a dot or a space, which Windows strips".into());
    }
    let stem = name.split('.').next().unwrap_or("");
    if crate::content_store::is_windows_device_name(stem) {
        return Err(format!("the name is the Windows device name '{stem}'"));
    }
    Ok(())
}

/// Whether a file name is a plugin the game loads (`.esp`, `.esm`, `.esl`).
pub fn is_plugin_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".esp") || lower.ends_with(".esm") || lower.ends_with(".esl")
}

#[cfg(test)]
mod tests {
    use super::*;

    const INI: &str = "[General]\r\ngameName=Skyrim Special Edition\r\n\
selected_profile=@ByteArray(Grounded Apocalypse)\r\n\
gamePath=@ByteArray(D:\\\\SteamLibrary\\\\steamapps\\\\common\\\\Skyrim Special Edition)\r\n\
[Settings]\r\nmod_directory=%BASE_DIR%/mods2\r\n[Other]\r\nmods=x\r\n";

    #[test]
    fn reads_byte_array_and_doubled_backslashes() {
        let ini = parse_ini(INI, PathBuf::from(r"D:\Setup"));
        assert_eq!(ini.game_name.as_deref(), Some("Skyrim Special Edition"));
        assert_eq!(ini.selected_profile.as_deref(), Some("Grounded Apocalypse"));
        assert_eq!(
            ini.game_path.as_deref(),
            Some(r"D:\SteamLibrary\steamapps\common\Skyrim Special Edition")
        );
    }

    #[test]
    fn resolves_base_dir_and_defaults_relative_to_the_base() {
        let ini = parse_ini(INI, PathBuf::from(r"D:\Setup"));
        assert_eq!(ini.base, PathBuf::from(r"D:\Setup"));
        assert_eq!(ini.mods, PathBuf::from(r"D:\Setup/mods2"));
        assert_eq!(ini.profiles, PathBuf::from(r"D:\Setup/profiles"));
        assert_eq!(ini.overwrite, PathBuf::from(r"D:\Setup/overwrite"));
        assert!(ini.warnings.is_empty());
    }

    #[test]
    fn base_directory_setting_moves_the_defaults() {
        let text = "[Settings]\nbase_directory=E:\\\\Mods\\\\Base\n";
        let ini = parse_ini(text, PathBuf::from(r"D:\Setup"));
        assert_eq!(ini.base, PathBuf::from(r"E:\Mods\Base"));
        assert_eq!(ini.mods, PathBuf::from(r"E:\Mods\Base/mods"));
    }

    #[test]
    fn an_unknown_variable_is_a_warning() {
        let text = "[Settings]\nmod_directory=%SOMEWHERE%/mods\n";
        let ini = parse_ini(text, PathBuf::from(r"D:\Setup"));
        assert_eq!(ini.warnings.len(), 1);
    }

    #[test]
    fn modlist_keeps_order_and_states() {
        let text = "# generated\r\n+501 - DynDOLOD Output\r\n-430 - Old\r\n*Creation Club: x\r\n?odd\r\n\r\n+Thing_separator\r\n";
        let lines = parse_modlist(text);
        let states: Vec<_> = lines.iter().map(|l| (l.state, l.name.as_str())).collect();
        assert_eq!(
            states,
            vec![
                (ModState::Enabled, "501 - DynDOLOD Output"),
                (ModState::Disabled, "430 - Old"),
                (ModState::Unmanaged, "Creation Club: x"),
                (ModState::Unknown, "?odd"),
                (ModState::Enabled, "Thing_separator"),
            ]
        );
        assert_eq!(lines[0].line, 2, "line numbers count comments");
    }

    #[test]
    fn plugins_and_loadorder_and_locks() {
        let plugins = parse_plugins("# header\r\n*active.esp\r\ninactive.esp\r\n");
        assert_eq!(plugins.len(), 2);
        assert!(plugins[0].active && !plugins[1].active);
        assert_eq!(
            parse_loadorder("# h\r\nSkyrim.esm\r\nactive.esp\r\ninactive.esp\r\n"),
            vec!["Skyrim.esm", "active.esp", "inactive.esp"]
        );
        let locks = parse_lockedorder("# This file was automatically generated by Mod Organizer.\r\nactive.esp|2\r\nbad.esp|-1\r\n");
        assert_eq!(locks[0].name, "active.esp");
        assert_eq!(locks[0].priority, Some(2));
        assert_eq!(locks[1].priority, None);
    }

    #[test]
    fn profile_settings_flags() {
        let s = parse_profile_settings("[General]\r\nLocalSaves=true\r\nLocalSettings=True\r\n");
        assert!(s.local_saves && s.local_settings);
        assert_eq!(
            parse_profile_settings("[General]\nLocalSaves=false\n"),
            ProfileSettings::default()
        );
    }

    #[test]
    fn meta_ini_reads_claims_and_refuses_garbage() {
        let meta = parse_meta_ini(
            b"[General]\r\ngameName=skyrimspecialedition\r\nmodid=2014\r\nversion=1.2\r\ninstallationFile=D:/x/Some Mod-2014.7z\r\nrepository=Nexus\r\n",
        )
        .unwrap();
        assert_eq!(meta.modid, Some(2014));
        assert_eq!(
            meta.installation_file.as_deref(),
            Some("D:/x/Some Mod-2014.7z")
        );
        let bad = parse_meta_ini(b"[General]\nmodid=abc\n").unwrap();
        assert_eq!(bad.modid, None);
        assert_eq!(bad.warnings.len(), 1);
        let none = parse_meta_ini(b"[General]\nmodid=0\n").unwrap();
        assert_eq!(none.modid, None, "0 means no Nexus id");
        assert!(parse_meta_ini(&[0u8, 159, 146, 150, 0, 1]).is_err());
        assert!(parse_meta_ini(b"no section here").is_err());
    }

    #[test]
    fn hostile_mod_names_are_refused() {
        for name in [
            r"..\outside",
            r"..\..\Windows",
            r"C:\Windows",
            "C:Windows",
            "a/b",
            "..",
            ".",
            "",
            "CON",
            "nul.txt",
            "trailing.",
            "trailing ",
            "bell\u{7}",
        ] {
            assert!(check_mod_folder_name(name).is_err(), "accepted {name:?}");
        }
        for name in [
            "090 - Nemesis Unlimited Behavior Engine 0.84",
            "236 - Hair Specular 149011 1.1.2 2026-06-18T17-36Z N2AYL5OnR",
            "501 - DynDOLOD Output",
        ] {
            assert!(check_mod_folder_name(name).is_ok(), "refused {name:?}");
        }
    }
}
