use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::paths::{Prefix, Via};

/// Agora VFS JSON Configuration format (version 1, or version 2 with `aliases`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VfsConfig {
    pub version: u32,
    pub mount: String,
    pub upper: PathBuf,
    #[serde(default)]
    pub lowers: Vec<PathBuf>,
    pub dll: PathBuf,
    #[serde(default)]
    pub log: Option<PathBuf>,
    #[serde(default)]
    pub verbose: bool,
    #[serde(default)]
    pub ready_event: Option<String>,
    /// Folders that show exactly the same view as `mount` (version 2). Version 1 has none.
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub version: u32,
    pub mount: String, // normalized: uppercase/trimmed, no trailing backslash
    pub mount_original: String, // original casing with no trailing backslash
    pub mount_len: usize,
    pub upper: PathBuf,
    pub lowers: Vec<PathBuf>,
    pub dll: PathBuf,
    pub log: Option<PathBuf>,
    pub verbose: bool,
    pub ready_event: Option<String>,
    /// The mount, then each alias in configuration order. See [`Prefix`].
    pub prefixes: Vec<Prefix>,
}

impl RuntimeConfig {
    /// The folder a prefix names, as configured.
    pub fn prefix(&self, via: Via) -> &Prefix {
        let index = match via {
            Via::Mount => 0,
            Via::Alias(i) => i + 1,
        };
        &self.prefixes[index]
    }
}

fn normalize_win_path(p: &str) -> String {
    p.trim()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_string()
}

/// `inner` is `outer`, or a folder inside it. Both are normalized and lower case.
fn nested(inner: &str, outer: &str) -> bool {
    inner == outer
        || inner
            .strip_prefix(outer)
            .is_some_and(|rest| rest.starts_with('\\'))
}

/// Two folders overlap when one is the other, or lies inside it. Case-insensitive.
fn overlaps(a: &str, b: &str) -> bool {
    let (a, b) = (a.to_ascii_lowercase(), b.to_ascii_lowercase());
    nested(&a, &b) || nested(&b, &a)
}

impl VfsConfig {
    /// Validates and normalizes the parsed configuration.
    pub fn into_runtime(self) -> Result<RuntimeConfig, String> {
        if self.version != 1 && self.version != 2 {
            return Err(format!("Unsupported config version: {}", self.version));
        }
        if self.version == 1 && !self.aliases.is_empty() {
            // A version 1 reader would ignore the aliases and show a tool a vanilla game.
            return Err("aliases need configuration version 2".to_string());
        }
        let mount_clean = normalize_win_path(&self.mount);
        if mount_clean.is_empty() {
            return Err("Mount path cannot be empty".to_string());
        }
        let mount_len = mount_clean.len();
        let upper_clean = normalize_win_path(&self.upper.to_string_lossy());
        let lowers: Vec<String> = self
            .lowers
            .iter()
            .map(|p| normalize_win_path(&p.to_string_lossy()))
            .collect();
        let aliases: Vec<String> = self.aliases.iter().map(|a| normalize_win_path(a)).collect();

        for (i, alias) in aliases.iter().enumerate() {
            if alias.is_empty() {
                return Err("Alias path cannot be empty".to_string());
            }
            let mut others: Vec<(&str, &str)> =
                vec![("mount", &mount_clean), ("upper", &upper_clean)];
            others.extend(lowers.iter().map(|l| ("lower", l.as_str())));
            others.extend(
                aliases
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != i)
                    .map(|(_, a)| ("alias", a.as_str())),
            );
            for (what, other) in others {
                if overlaps(alias, other) {
                    return Err(format!("alias '{alias}' overlaps the {what} '{other}'"));
                }
            }
        }

        let dll_clean = PathBuf::from(normalize_win_path(&self.dll.to_string_lossy()));
        let log_clean = self
            .log
            .map(|p| PathBuf::from(normalize_win_path(&p.to_string_lossy())));

        let mut prefixes = vec![Prefix::new(Via::Mount, mount_clean.clone())];
        for (i, alias) in aliases.iter().enumerate() {
            prefixes.push(Prefix::new(Via::Alias(i), alias.clone()));
        }

        Ok(RuntimeConfig {
            version: self.version,
            mount_len,
            mount_original: mount_clean.clone(),
            mount: mount_clean,
            upper: PathBuf::from(upper_clean),
            lowers: lowers.into_iter().map(PathBuf::from).collect(),
            dll: dll_clean,
            log: log_clean,
            verbose: self.verbose,
            ready_event: self.ready_event,
            prefixes,
        })
    }

    /// Load and validate configuration from the JSON string.
    pub fn from_json_str(json: &str) -> Result<RuntimeConfig, String> {
        let parsed: VfsConfig =
            serde_json::from_str(json).map_err(|e| format!("JSON parse error: {e}"))?;
        parsed.into_runtime()
    }
}

/// Load configuration from file pointed to by AGORA_VFS_CONFIG
pub fn load_config_from_env() -> Option<RuntimeConfig> {
    let path = std::env::var_os("AGORA_VFS_CONFIG")?;
    let content = std::fs::read_to_string(path).ok()?;
    match VfsConfig::from_json_str(&content) {
        Ok(cfg) => Some(cfg),
        Err(e) => {
            eprintln!("[agora-vfs] Invalid config: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(version: u32, aliases: &[&str]) -> String {
        let aliases: Vec<String> = aliases
            .iter()
            .map(|a| format!("\"{}\"", a.replace('\\', "\\\\")))
            .collect();
        format!(
            r#"{{
                "version": {version},
                "mount": "D:\\Games\\Skyrim",
                "upper": "C:\\Instances\\upper",
                "lowers": ["D:\\Games\\Skyrim", "D:\\Bases\\base"],
                "dll": "C:\\Agora\\agora_vfs.dll",
                "aliases": [{}]
            }}"#,
            aliases.join(", ")
        )
    }

    #[test]
    fn versions_one_and_two_are_read_and_three_is_refused() {
        assert!(VfsConfig::from_json_str(&config(1, &[])).is_ok());
        assert!(VfsConfig::from_json_str(&config(2, &[])).is_ok());
        assert!(VfsConfig::from_json_str(&config(2, &[r"E:\Steam\Skyrim"])).is_ok());
        let err = VfsConfig::from_json_str(&config(3, &[])).unwrap_err();
        assert!(err.contains("Unsupported config version: 3"), "{err}");
    }

    #[test]
    fn version_one_with_aliases_is_refused_so_an_old_reader_never_shows_a_vanilla_game() {
        let err = VfsConfig::from_json_str(&config(1, &[r"E:\Steam\Skyrim"])).unwrap_err();
        assert!(err.contains("version 2"), "{err}");
    }

    #[test]
    fn an_alias_that_is_empty_or_overlaps_another_folder_is_refused() {
        let cases: &[(&str, &str)] = &[
            ("", "Alias path cannot be empty"),
            ("   ", "Alias path cannot be empty"),
            (r"D:\Games\Skyrim", "overlaps the mount"),
            (r"d:\games\SKYRIM\", "overlaps the mount"),
            (r"D:\Games", "overlaps the mount"),
            (r"D:\Games\Skyrim\Data", "overlaps the mount"),
            (r"C:\Instances\upper", "overlaps the upper"),
            (r"C:\Instances\upper\sub", "overlaps the upper"),
            (r"D:\Bases\base\", "overlaps the lower"),
            (r"D:\Bases", "overlaps the lower"),
        ];
        for (alias, why) in cases {
            let err = VfsConfig::from_json_str(&config(2, &[alias])).unwrap_err();
            assert!(err.contains(why), "{alias:?}: {err}");
        }
    }

    #[test]
    fn two_aliases_must_not_overlap_each_other() {
        let err =
            VfsConfig::from_json_str(&config(2, &[r"E:\Steam", r"e:\steam\Skyrim"])).unwrap_err();
        assert!(err.contains("overlaps the alias"), "{err}");
    }

    #[test]
    fn a_sibling_whose_name_only_starts_the_same_is_not_an_overlap() {
        let cfg = VfsConfig::from_json_str(&config(2, &[r"D:\Games\Skyrim2", r"E:\Steam\Skyrim"]))
            .expect("siblings are fine");
        assert_eq!(cfg.prefixes.len(), 3);
        assert_eq!(cfg.prefixes[1].original, r"D:\Games\Skyrim2");
        assert_eq!(cfg.prefixes[2].original, r"E:\Steam\Skyrim");
    }
}
