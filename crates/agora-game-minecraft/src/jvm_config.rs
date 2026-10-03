//! JVM settings for an instance: heap, garbage collector and custom arguments,
//! turned into the argument string a launch (direct or delegated) uses.

use serde::{Deserialize, Serialize};

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
        self.to_args_for_java(17)
    }

    /// Build arguments using the selected Java major for automatic GC mode.
    pub fn to_args_for_java(&self, java_version: u32) -> String {
        let selection = crate::gc::GcSelection::from_persisted(&self.gc);
        crate::gc::compute_gc_with_pre_touch(
            java_version,
            self.memory_mb,
            &self.custom_args,
            selection.resolve(),
            Some(self.always_pre_touch),
        )
        .jvm_args
    }
}

/// Infer the Java major used by the official launcher for common Minecraft
/// version ranges. Direct launches still use the resolved runtime's actual
/// major; this is only a safe preview for delegated launcher profiles.
pub fn recommended_java_version_for_minecraft(version: &str) -> u32 {
    let mut parts = version.split('.');
    let first = parts.next().and_then(|part| part.parse::<u32>().ok());
    // Minecraft's post-1.x version format (for example, 26.2) no longer has
    // the leading `1`. Minecraft 26.x requires Java 25.
    if first.is_some_and(|major| major >= 26) {
        return 25;
    }

    let minor = if first == Some(1) {
        parts.next().and_then(|part| part.parse::<u32>().ok())
    } else {
        first
    };
    let patch = parts.next().and_then(|part| part.parse::<u32>().ok());
    match (minor, patch) {
        (Some(major), Some(patch)) if major >= 21 || (major == 20 && patch >= 5) => 21,
        (Some(major), None) if major >= 21 => 21,
        (Some(major), _) if major >= 18 => 17,
        _ => 8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_java_version_tracks_minecraft_requirements() {
        assert_eq!(recommended_java_version_for_minecraft("26.2"), 25);
        assert_eq!(recommended_java_version_for_minecraft("1.21.1"), 21);
        assert_eq!(recommended_java_version_for_minecraft("1.20.4"), 17);
        assert_eq!(recommended_java_version_for_minecraft("1.16.5"), 8);
    }

    #[test]
    fn delegated_profiles_use_the_same_gc_flags_as_preview() {
        let args = JvmConfig {
            memory_mb: 4096,
            gc: "high_efficiency".into(),
            custom_args: String::new(),
            always_pre_touch: false,
        }
        .to_args_for_java(17);
        assert!(args.contains("-XX:+UseG1GC"));
        assert!(!args.contains("AlwaysPreTouch"));

        let args = JvmConfig {
            memory_mb: 4096,
            gc: "low_latency".into(),
            custom_args: String::new(),
            always_pre_touch: true,
        }
        .to_args_for_java(21);
        assert!(args.contains("-XX:+UseZGC"));
        assert!(args.contains("-XX:+ZGenerational"));

        let args = JvmConfig {
            memory_mb: 4096,
            gc: "g1gc".into(),
            custom_args: String::new(),
            always_pre_touch: false,
        }
        .to_args_for_java(25);
        assert!(args.contains("-XX:+UseZGC"));
        assert!(args.contains("-XX:+ZGenerational"));
    }

    #[test]
    fn preview_uses_compatibility_decoding_for_legacy_gc_values() {
        let fixtures = [
            ("auto", crate::gc::GcSelection::Auto),
            ("manual", crate::gc::GcSelection::Manual),
            ("low_latency", crate::gc::GcSelection::LowLatency),
            ("high_efficiency", crate::gc::GcSelection::HighEfficiency),
            ("g1gc", crate::gc::GcSelection::Auto),
            ("zgc", crate::gc::GcSelection::LowLatency),
            ("shenandoah", crate::gc::GcSelection::Auto),
            ("", crate::gc::GcSelection::Auto),
            ("garbage", crate::gc::GcSelection::Auto),
        ];
        for (stored, expected_selection) in fixtures {
            let java_version = if expected_selection == crate::gc::GcSelection::LowLatency {
                17
            } else {
                21
            };
            let preview = JvmConfig {
                memory_mb: 4096,
                gc: stored.into(),
                custom_args: String::new(),
                always_pre_touch: false,
            }
            .to_args_for_java(java_version);
            let expected = crate::gc::compute_gc_with_pre_touch(
                java_version,
                4096,
                "",
                expected_selection.resolve(),
                Some(false),
            )
            .jvm_args;
            assert_eq!(preview, expected, "preview for stored value {stored:?}");
        }
    }
}
