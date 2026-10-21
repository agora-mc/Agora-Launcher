//! Whether a framework (SKSE, for example) is installed in an instance, and which version it is
//! (MASTER_SPEC §26.6, §26.8).
//!
//! Presence is judged the way launch judges it: against the paths the game will see, from the
//! instance's deployment plan. The version comes from the marker file's own bytes, so it is the
//! version of the file that will run.

use std::path::PathBuf;

use agora_game_api::{FrameworkDefinition, FrameworkVersionSource, GameDefinition};

use crate::ctx::Ctx;
use crate::error::{LauncherError, LauncherResult};

/// What an instance has of one framework.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameworkStatus {
    /// The game's files have no marker for this framework.
    Absent,
    /// The marker is there. `version` is `None` when it could not be read; `unreadable` says why.
    Present {
        version: Option<String>,
        unreadable: Option<String>,
    },
    /// The framework declares no marker, so core cannot tell whether it is installed.
    NotDetectable,
}

/// The framework's status in `instance_id`, judged over the files the game will see.
pub fn framework_status(
    ctx: &Ctx,
    instance_id: &str,
    definition: &GameDefinition,
    framework: &FrameworkDefinition,
) -> LauncherResult<FrameworkStatus> {
    let Some(detection) = &framework.detection else {
        return Ok(FrameworkStatus::NotDetectable);
    };
    let marker = detection.marker.as_str();
    let mode = crate::game_instance::visible_mode(ctx, instance_id, definition).map_err(|e| {
        LauncherError::Generic {
            code: "ERR_FRAMEWORK_CHECK".into(),
            message: e.to_string(),
        }
    })?;
    let visible =
        crate::game_deploy::visible_paths(ctx, instance_id, definition, mode).map_err(|e| {
            LauncherError::Generic {
                code: "ERR_FRAMEWORK_CHECK".into(),
                message: e.to_string(),
            }
        })?;
    if !visible.iter().any(|path| path.eq_ignore_ascii_case(marker)) {
        return Ok(FrameworkStatus::Absent);
    }

    let Some(version_source) = detection.version else {
        return Ok(FrameworkStatus::Present {
            version: None,
            unreadable: Some("the game definition says nowhere to read its version".into()),
        });
    };
    let source =
        crate::game_deploy::visible_file_source(ctx, instance_id, definition, mode, marker)
            .map_err(|e| LauncherError::Generic {
                code: "ERR_FRAMEWORK_CHECK".into(),
                message: e.to_string(),
            })?;
    Ok(match read_version(version_source, source) {
        Ok(version) => FrameworkStatus::Present {
            version: Some(version),
            unreadable: None,
        },
        Err(reason) => FrameworkStatus::Present {
            version: None,
            unreadable: Some(reason),
        },
    })
}

/// The version of the marker file at `source`, as the manifests write it, or why it cannot be read.
fn read_version(source: FrameworkVersionSource, path: Option<PathBuf>) -> Result<String, String> {
    let Some(path) = path else {
        return Err("the marker file's bytes could not be located".into());
    };
    let bytes = std::fs::read(&path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    match source {
        FrameworkVersionSource::PeFileVersion => {
            let components =
                crate::pe_version::read_file_version(&bytes).map_err(|e| e.to_string())?;
            Ok(display_pe_version(components))
        }
    }
}

/// A PE `FileVersion` as a framework manifest writes it. A leading zero component is dropped, so
/// SKSE's `0.2.2.6` is `2.2.6`; any other first component is kept.
pub fn display_pe_version(components: [u16; 4]) -> String {
    let parts: Vec<String> = components.iter().map(u16::to_string).collect();
    if components[0] == 0 {
        parts[1..].join(".")
    } else {
        parts.join(".")
    }
}

/// Whether dotted-number `version` is at least `minimum`. Missing trailing components count as
/// zero, so `2.2.6` equals `2.2.6.0`. `None` when either side is not dotted numbers.
pub fn version_at_least(version: &str, minimum: &str) -> Option<bool> {
    let left = parse_dotted(version)?;
    let right = parse_dotted(minimum)?;
    let width = left.len().max(right.len());
    for index in 0..width {
        let a = left.get(index).copied().unwrap_or(0);
        let b = right.get(index).copied().unwrap_or(0);
        if a != b {
            return Some(a > b);
        }
    }
    Some(true)
}

fn parse_dotted(value: &str) -> Option<Vec<u64>> {
    let parts: Option<Vec<u64>> = value
        .trim()
        .split('.')
        .map(|part| {
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                None
            } else {
                part.parse().ok()
            }
        })
        .collect();
    parts.filter(|parts| !parts.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_skse_loader_version_reads_as_the_manifest_writes_it() {
        assert_eq!(display_pe_version([0, 2, 2, 6]), "2.2.6");
        assert_eq!(display_pe_version([1, 6, 1170, 0]), "1.6.1170.0");
    }

    #[test]
    fn versions_compare_numerically_with_missing_parts_as_zero() {
        assert_eq!(version_at_least("2.2.6", "2.2.6"), Some(true));
        assert_eq!(version_at_least("2.2.6.0", "2.2.6"), Some(true));
        assert_eq!(version_at_least("2.2.10", "2.2.6"), Some(true));
        assert_eq!(version_at_least("2.2.5", "2.2.6"), Some(false));
        assert_eq!(version_at_least("2.1.9", "2.2.6"), Some(false));
        assert_eq!(version_at_least("3.0", "2.2.6"), Some(true));
    }

    #[test]
    fn a_version_that_is_not_dotted_numbers_cannot_be_compared() {
        assert_eq!(version_at_least("2.2.6-beta", "2.2.6"), None);
        assert_eq!(version_at_least("", "2.2.6"), None);
        assert_eq!(version_at_least("2..6", "2.2.6"), None);
    }
}
