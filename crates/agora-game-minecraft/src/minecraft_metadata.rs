//! Mojang metadata operations — version manifest and version JSON resolution.
//!
//! All metadata is resolved against the Agora-owned runtime root, NOT the
//! official `.minecraft` directory.  The official Mojang Launcher is optional
//! for direct launch.

use crate::launch::{MojangVersionManifest, MojangVersionRef, VersionInfo};
use crate::launch_planner;
use crate::launch_planner::LaunchHttpClients;
use agora_core::error::{LauncherError, LauncherResult};
use agora_core::network::{NetworkCategory, NetworkPolicy};
use std::path::Path;

const VERSION_MANIFEST_URL: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

/// Ensure the base version metadata JSON is available in the Agora runtime
/// root and return the parsed [`VersionInfo`].
///
/// Resolution order:
/// 1. Cache hit: `<runtime-root>/versions/<id>/<id>.json` already exists.
/// 2. Network: download version manifest → find version → download version
///    JSON → write to cache → return.
/// 3. Error: neither cached nor allowed to fetch.
///
/// HTTP clients are constructed internally using the planner's redirect-safe
/// policy — callers do not need to supply clients.
pub async fn ensure_base_version_metadata(
    minecraft_root: &Path,
    version_id: &str,
    policy: &NetworkPolicy,
) -> LauncherResult<VersionInfo> {
    // 0. Validate version_id to prevent path traversal.
    if version_id.is_empty()
        || version_id.contains('/')
        || version_id.contains('\\')
        || version_id.contains('\0')
        || version_id.contains("..")
    {
        return Err(LauncherError::Generic {
            code: "ERR_INVALID_VERSION_ID".into(),
            message: format!("Invalid version_id: {version_id}"),
        });
    }

    // 1. Check cache first.
    let version_path = minecraft_root
        .join("versions")
        .join(version_id)
        .join(format!("{version_id}.json"));

    if let Ok(data) = tokio::fs::read_to_string(&version_path).await {
        if let Ok(info) = serde_json::from_str::<VersionInfo>(&data) {
            return Ok(info);
        }
    }

    // 2. Check network policy before any outward request.
    policy.check(NetworkCategory::MojangMetadata)?;

    // 3. Build redirect-safe HTTP clients (reuses planner's policy).
    let clients = LaunchHttpClients::new()?;

    // 4. Download version manifest cache-first.
    let metadata_dir = minecraft_root.join("metadata");
    let manifest_path = metadata_dir.join("version_manifest_v2.json");

    let manifest: MojangVersionManifest = launch_planner::load_json_cache_first(
        clients.for_category(NetworkCategory::MojangMetadata),
        VERSION_MANIFEST_URL,
        &manifest_path,
        None,
        policy,
        NetworkCategory::MojangMetadata,
    )
    .await?;

    // 5. Find the requested version in the manifest.
    let version_ref = manifest
        .versions
        .iter()
        .find(|v: &&MojangVersionRef| v.id == version_id)
        .ok_or(LauncherError::GameVersionNotFound)?;

    // 6. Ensure the versions directory exists.
    let version_dir = minecraft_root.join("versions").join(version_id);
    tokio::fs::create_dir_all(&version_dir)
        .await
        .map_err(|error| LauncherError::Generic {
            code: "ERR_VERSION_DIR".into(),
            message: format!("Failed to create version dir: {error}"),
        })?;

    // 7. Download the version JSON cache-first.
    let info: VersionInfo = launch_planner::load_json_cache_first(
        clients.for_category(NetworkCategory::MojangMetadata),
        &version_ref.url,
        &version_path,
        version_ref.sha1.as_deref(),
        policy,
        NetworkCategory::MojangMetadata,
    )
    .await?;

    Ok(info)
}

/// Snapshot, pre-release and release-candidate versions, newest first, for
/// vanilla instances. Old alpha and beta versions are left out: they need
/// launch fixes Agora does not carry.
pub async fn list_snapshot_versions(
    minecraft_root: &Path,
    policy: &NetworkPolicy,
) -> LauncherResult<Vec<String>> {
    policy.check(NetworkCategory::MojangMetadata)?;
    let clients = LaunchHttpClients::new()?;
    let manifest: MojangVersionManifest = launch_planner::load_json_cache_first(
        clients.for_category(NetworkCategory::MojangMetadata),
        VERSION_MANIFEST_URL,
        &minecraft_root
            .join("metadata")
            .join("version_manifest_v2.json"),
        None,
        policy,
        NetworkCategory::MojangMetadata,
    )
    .await?;
    Ok(snapshot_ids(&manifest))
}

fn snapshot_ids(manifest: &MojangVersionManifest) -> Vec<String> {
    manifest
        .versions
        .iter()
        .filter(|version| version.type_ == "snapshot")
        .map(|version| version.id.clone())
        .collect()
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;

    #[test]
    fn only_snapshot_types_are_listed_newest_first() {
        let manifest: MojangVersionManifest = serde_json::from_value(serde_json::json!({
            "latest": {"release": "26.3", "snapshot": "26w44a"},
            "versions": [
                {"id": "26w44a", "type": "snapshot", "url": "https://x/1"},
                {"id": "26.3", "type": "release", "url": "https://x/2"},
                {"id": "26.3-pre1", "type": "snapshot", "url": "https://x/3"},
                {"id": "b1.7.3", "type": "old_beta", "url": "https://x/4"},
                {"id": "a1.0.4", "type": "old_alpha", "url": "https://x/5"}
            ]
        }))
        .unwrap();
        assert_eq!(snapshot_ids(&manifest), vec!["26w44a", "26.3-pre1"]);
    }
}
