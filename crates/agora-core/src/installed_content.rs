//! Inventory rows for installed instance content.

use crate::helpers::content_subdir;
use crate::models::{InstalledMod, InstanceManifest};
use crate::registry::{RegistryItem, RegistryService};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CurationStatus {
    Curated,
    UnderReview,
    Uncurated,
    Archived,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataStatus {
    Complete,
    Partial,
    Unavailable,
}

/// A manifest entry enriched with local filesystem and cached registry data.
#[derive(Debug, Clone, Serialize)]
pub struct InstalledContentRow {
    pub key: String,
    pub filename: String,
    pub display_name: String,
    pub version: Option<String>,
    pub content_type: String,
    pub enabled: bool,
    pub installed_at: String,
    pub source: String,
    pub source_label: String,
    pub source_url: Option<String>,
    pub registry_id: Option<String>,
    pub modrinth_id: Option<String>,
    pub mod_jar_id: Option<String>,
    pub loader_mod_id: Option<String>,
    pub size_bytes: Option<u64>,
    pub file_present: bool,
    pub resolved_path: Option<String>,
    pub author: Option<String>,
    pub categories: Vec<String>,
    pub icon_url: Option<String>,
    pub curation_status: CurationStatus,
    pub agora_score: Option<i64>,
    pub modrinth_downloads: Option<i64>,
    pub metadata_status: MetadataStatus,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstalledContentMetadata {
    pub key: String,
    pub display_name: Option<String>,
    pub icon_url: Option<String>,
    pub author: Option<String>,
}

/// Build all rows for an instance. Registry failures degrade to manifest and
/// filesystem data so a stale or missing cache never hides installed content.
pub fn list_installed_content(
    instance_dir: &Path,
    manifest: &InstanceManifest,
    content_type: Option<&str>,
    registry: Option<&RegistryService>,
) -> Vec<InstalledContentRow> {
    let entries = manifest
        .mods
        .iter()
        .chain(manifest.resourcepacks.iter())
        .chain(manifest.shaders.iter())
        .chain(manifest.datapacks.iter())
        .chain(manifest.worlds.iter())
        .filter(|entry| content_type_matches(&entry.content_type, content_type));

    let entries: Vec<&InstalledMod> = entries.collect();
    let registry_items = registry
        .map(|service| {
            let ids = entries
                .iter()
                .filter_map(|entry| entry.registry_id.clone())
                .collect::<Vec<_>>();
            service.get_items_by_ids(&ids).unwrap_or_default()
        })
        .unwrap_or_default();
    let categories = registry
        .map(|service| {
            let ids = entries
                .iter()
                .filter_map(|entry| entry.registry_id.clone())
                .collect::<Vec<_>>();
            service.get_item_categories(&ids).unwrap_or_default()
        })
        .unwrap_or_default();
    let authors = registry
        .map(|service| {
            let ids = entries
                .iter()
                .filter_map(|entry| entry.registry_id.clone())
                .collect::<Vec<_>>();
            service.get_item_authors(&ids).unwrap_or_default()
        })
        .unwrap_or_default();

    entries
        .into_iter()
        .map(|entry| {
            let registry_item = entry
                .registry_id
                .as_ref()
                .and_then(|id| registry_items.get(id));
            let item_categories = entry
                .registry_id
                .as_ref()
                .and_then(|id| categories.get(id))
                .cloned()
                .unwrap_or_default();
            let author = entry
                .registry_id
                .as_ref()
                .and_then(|id| authors.get(id))
                .cloned();
            build_row(instance_dir, entry, registry_item, item_categories, author)
        })
        .collect()
}

fn content_type_matches(value: &str, requested: Option<&str>) -> bool {
    let Some(requested) = requested else {
        return true;
    };
    normalize_content_type(value) == normalize_content_type(requested)
}

fn normalize_content_type(value: &str) -> &str {
    match value {
        "resourcepacks" | "resource_pack" | "resource-packs" => "resourcepack",
        "shaderpacks" | "shaderpack" | "shaders" => "shader",
        "datapacks" | "data_pack" | "data-packs" => "datapack",
        "worlds" | "save" | "saves" => "world",
        _ => value,
    }
}

fn build_row(
    instance_dir: &Path,
    entry: &InstalledMod,
    registry_item: Option<&RegistryItem>,
    categories: Vec<String>,
    author: Option<String>,
) -> InstalledContentRow {
    let (resolved_path, size_bytes) = resolve_file(instance_dir, entry);
    let file_present = resolved_path.is_some();
    let source_label = source_label(&entry.source);
    let curation_status = registry_item
        .map(|item| curation_status(&item.status))
        .unwrap_or(CurationStatus::Unknown);
    let metadata_status = if registry_item.is_some() {
        MetadataStatus::Complete
    } else if entry.registry_id.is_some() || entry.modrinth_id.is_some() {
        MetadataStatus::Partial
    } else {
        MetadataStatus::Unavailable
    };
    let display_name = registry_item
        .map(|item| item.name.clone())
        .unwrap_or_else(|| filename_display_name(&entry.filename));

    InstalledContentRow {
        key: format!(
            "{}:{}:{}",
            normalize_content_type(&entry.content_type),
            entry.filename,
            entry.sha256
        ),
        filename: entry.filename.clone(),
        display_name,
        version: entry.version.clone(),
        content_type: normalize_content_type(&entry.content_type).to_string(),
        enabled: entry.enabled,
        installed_at: entry.installed_at.clone(),
        source: entry.source.clone(),
        source_label,
        source_url: entry.source_url.clone(),
        registry_id: entry.registry_id.clone(),
        modrinth_id: entry
            .modrinth_id
            .clone()
            .or_else(|| registry_item.and_then(|item| item.modrinth_id.clone())),
        mod_jar_id: entry.mod_jar_id.clone(),
        loader_mod_id: entry.mod_jar_id.clone(),
        size_bytes,
        file_present,
        resolved_path,
        author,
        categories: if categories.is_empty() {
            vec!["Uncategorized".to_string()]
        } else {
            categories
        },
        icon_url: registry_item.and_then(|item| item.icon_url.clone()),
        curation_status,
        agora_score: registry_item.map(|item| item.net_score),
        modrinth_downloads: None,
        metadata_status,
    }
}

fn resolve_file(instance_dir: &Path, entry: &InstalledMod) -> (Option<String>, Option<u64>) {
    if !safe_filename(&entry.filename) {
        return (None, None);
    }
    let base = instance_dir.join(content_subdir(&entry.content_type));
    let candidates = if entry.enabled {
        vec![
            base.join(&entry.filename),
            base.join(format!("{}.disabled", entry.filename)),
        ]
    } else {
        vec![
            base.join(format!("{}.disabled", entry.filename)),
            base.join(&entry.filename),
        ]
    };
    candidates
        .into_iter()
        .find_map(|path| {
            std::fs::metadata(&path)
                .ok()
                .map(|metadata| (path, metadata.len()))
        })
        .map(|(path, size)| (Some(path.to_string_lossy().into_owned()), Some(size)))
        .unwrap_or((None, None))
}

fn safe_filename(filename: &str) -> bool {
    !filename.is_empty()
        && filename != "."
        && filename != ".."
        && !filename.contains('/')
        && !filename.contains('\\')
        && !filename.contains('\0')
}

fn filename_display_name(filename: &str) -> String {
    Path::new(filename)
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or(filename)
        .to_string()
}

fn source_label(source: &str) -> String {
    let normalized = source.trim().to_ascii_lowercase().replace([' ', '-'], "_");
    if normalized == "modrinth" || normalized == "modrinth_raw" {
        return "Modrinth".to_string();
    }
    if normalized == "modrinth_pack" {
        return "Modrinth Pack".to_string();
    }
    if normalized.contains("github") {
        return "GitHub Release".to_string();
    }
    if normalized == "registry" || normalized == "curated" {
        return "Agora Registry".to_string();
    }
    if normalized.contains("manual") || normalized == "local" {
        return "Manual".to_string();
    }
    "Other".to_string()
}

fn curation_status(status: &str) -> CurationStatus {
    match status.trim().to_ascii_lowercase().as_str() {
        "active" | "curated" => CurationStatus::Curated,
        "under_review" | "under-review" => CurationStatus::UnderReview,
        "archived" => CurationStatus::Archived,
        "uncurated" => CurationStatus::Uncurated,
        _ => CurationStatus::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn manifest_entry(filename: &str, content_type: &str, enabled: bool) -> InstalledMod {
        InstalledMod {
            pack_managed: false,
            filename: filename.to_string(),
            registry_id: None,
            modrinth_id: None,
            source: "manual_drag_drop".to_string(),
            source_url: None,
            version: None,
            sha256: "hash".to_string(),
            installed_at: "not-a-timestamp".to_string(),
            java_packages: Vec::new(),
            mod_jar_id: None,
            provided_mod_ids: Vec::new(),
            enabled,
            content_type: content_type.to_string(),
            depends_on: Vec::new(),
            optional_deps: Vec::new(),
            incompatible_deps: Vec::new(),
        }
    }

    fn temp_dir() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("agora-installed-content-{suffix}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn resolves_enabled_and_disabled_files_and_reports_missing_rows() {
        let dir = temp_dir();
        fs::create_dir_all(dir.join("mods")).unwrap();
        fs::create_dir_all(dir.join("resourcepacks")).unwrap();
        fs::write(dir.join("mods/enabled.jar"), b"12345").unwrap();
        fs::write(dir.join("mods/disabled.jar.disabled"), b"12").unwrap();
        fs::write(dir.join("resourcepacks/pack.zip"), b"x").unwrap();

        let mut manifest = InstanceManifest {
            manifest_version: crate::models::CURRENT_MANIFEST_VERSION,
            pack_origin: None,
            instance_id: "test".to_string(),
            name: "Test".to_string(),
            created_from_pack: None,
            minecraft_version: "1.21".to_string(),
            loader: "fabric".to_string(),
            loader_version: "1".to_string(),
            is_locked: false,
            mods: vec![
                manifest_entry("enabled.jar", "mod", true),
                manifest_entry("disabled.jar", "mod", false),
                manifest_entry("missing.jar", "mod", true),
            ],
            resourcepacks: vec![manifest_entry("pack.zip", "resourcepack", true)],
            shaders: Vec::new(),
            datapacks: Vec::new(),
            worlds: Vec::new(),
            user_preferences: serde_json::Value::Object(Default::default()),
        };
        let rows = list_installed_content(&dir, &manifest, None, None);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].size_bytes, Some(5));
        assert_eq!(rows[1].size_bytes, Some(2));
        assert!(!rows[2].file_present);
        assert_eq!(rows[3].content_type, "resourcepack");
        assert_eq!(rows[3].source_label, "Manual");
        manifest.mods.clear();
        let filtered = list_installed_content(&dir, &manifest, Some("resourcepack"), None);
        assert_eq!(filtered.len(), 1);
        let _ = fs::remove_dir_all(dir);
    }
}
