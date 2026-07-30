use serde::{Deserialize, Serialize};

// Re-export canonical types from core.
pub use agora_core::models::{InstalledMod, InstanceManifest};

/// A row in `local_state.db`'s `user_instances` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceRow {
    pub instance_id: String,
    pub name: String,
    pub minecraft_version: String,
    pub loader: String,
    pub loader_version: String,
    pub is_modpack: bool,
    pub is_locked: bool,
    pub last_launched_at: Option<String>,
    pub jvm_memory_mb: i64,
    #[serde(default = "default_memory_mode")]
    pub jvm_memory_mode: String,
    pub jvm_gc: String,
    pub jvm_custom_args: String,
    pub jvm_always_pre_touch: bool,
    pub created_at: String,
    #[serde(default)]
    pub java_path: Option<String>,
    #[serde(default)]
    pub java_incompatible_override: bool,
    #[serde(default)]
    pub icon_path: Option<String>,
    #[serde(default = "default_launch_mode_override")]
    pub launch_mode_override: String,
    #[serde(default)]
    pub import_source: Option<String>,
}

fn default_launch_mode_override() -> String {
    "auto".to_string()
}

fn default_memory_mode() -> String {
    "manual".to_string()
}

/// JVM configuration assembled from instance settings (see §8.5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JvmConfig {
    pub memory_mb: i64,
    pub gc: String,
    pub custom_args: String,
    pub always_pre_touch: bool,
}

impl JvmConfig {
    /// Build the `javaArgs` string consumed by the Mojang launcher profile.
    pub fn to_args(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        let mem = format!("-Xmx{}M -Xms{}M", self.memory_mb, self.memory_mb);
        parts.push(mem);

        match self.gc.as_str() {
            "zgc" => parts.push("-XX:+UseZGC".to_string()),
            "shenandoah" => parts.push("-XX:+UseShenandoahGC".to_string()),
            "g1gc" => parts.push("-XX:+UseG1GC".to_string()),
            _ => {}
        }

        if !self.custom_args.trim().is_empty() {
            parts.push(self.custom_args.trim().to_string());
        }
        if self.always_pre_touch {
            parts.push("-XX:+UnlockExperimentalVMOptions".to_string());
            parts.push("-XX:+AlwaysPreTouch".to_string());
        }
        parts.join(" ")
    }
}

/// A candidate version returned by the mod version resolution API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModVersionCandidate {
    pub version: String,
    pub filename: String,
    pub download_url: String,
    pub mc_version: Option<String>,
    pub loader: Option<String>,
    pub release_date: Option<String>,
    pub is_compatible: bool,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub sha512: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    /// Compatibility tier: `"compatible"` (exact MC version + loader match),
    /// `"major_match"` (same major version, different minor), or `""` (incompatible).
    #[serde(default)]
    pub version_compat: String,
}
