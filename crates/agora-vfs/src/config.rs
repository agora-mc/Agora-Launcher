use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Agora VFS JSON Configuration format (version 1)
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
}

fn normalize_win_path(p: &str) -> String {
    p.trim()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_string()
}

impl VfsConfig {
    /// Validates and normalizes the parsed configuration.
    pub fn into_runtime(self) -> Result<RuntimeConfig, String> {
        if self.version != 1 {
            return Err(format!("Unsupported config version: {}", self.version));
        }
        let mount_clean = normalize_win_path(&self.mount);
        if mount_clean.is_empty() {
            return Err("Mount path cannot be empty".to_string());
        }
        let mount_len = mount_clean.len();
        let upper_clean = PathBuf::from(normalize_win_path(&self.upper.to_string_lossy()));
        let lowers = self
            .lowers
            .into_iter()
            .map(|p| PathBuf::from(normalize_win_path(&p.to_string_lossy())))
            .collect();
        let dll_clean = PathBuf::from(normalize_win_path(&self.dll.to_string_lossy()));
        let log_clean = self
            .log
            .map(|p| PathBuf::from(normalize_win_path(&p.to_string_lossy())));

        Ok(RuntimeConfig {
            version: self.version,
            mount_len,
            mount_original: mount_clean.clone(),
            mount: mount_clean,
            upper: upper_clean,
            lowers,
            dll: dll_clean,
            log: log_clean,
            verbose: self.verbose,
            ready_event: self.ready_event,
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
