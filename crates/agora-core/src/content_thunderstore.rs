//! Thunderstore package support for BepInEx games (MASTER_SPEC §26.6).
//!
//! Recognizes Thunderstore packages, derives items according to r2modman / Thunderstore
//! layout conventions, checks for destination collisions across instance layers,
//! and reports missing dependencies.

use std::collections::HashSet;
use std::path::Path;

use agora_game_api::{
    extract_thunderstore_package_id, is_thunderstore_name, map_thunderstore_bepinex, Layer,
    LayerSource, RelPath, ThunderstoreManifest, ThunderstoreMappingError,
};

use crate::content_store::{self, ContentError, ContentItem, ContentSource};
use crate::ctx::Ctx;
use crate::game_deploy::{self, DeployError};
use crate::game_instance::{self, InstanceError};

#[derive(Debug, thiserror::Error)]
pub enum ThunderstoreInstallError {
    #[error("not a Thunderstore package")]
    NotThunderstorePackage,
    #[error(
        "file '{path}' in package '{package}' collides with existing package '{existing_package}'"
    )]
    FileCollision {
        path: String,
        package: String,
        existing_package: String,
    },
    #[error("content error: {0}")]
    Content(#[from] ContentError),
    #[error("mapping error: {0}")]
    Mapping(#[from] ThunderstoreMappingError),
    #[error("instance error: {0}")]
    Instance(#[from] InstanceError),
    #[error("deploy error: {0}")]
    Deploy(#[from] DeployError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone)]
pub struct ThunderstoreInstallOutcome {
    pub derived_item: ContentItem,
    pub layer: Layer,
    pub package_id: String,
    pub version: String,
    pub summary: String,
    pub missing_dependencies: Vec<String>,
}

/// Parse `manifest.json` from the top level of an item, if present and valid.
pub fn parse_manifest(
    ctx: &Ctx,
    item_id: &str,
) -> Result<Option<ThunderstoreManifest>, ContentError> {
    let item = content_store::get_item(ctx, item_id)?;
    let manifest_file = item
        .files
        .iter()
        .find(|f| f.path.as_str().eq_ignore_ascii_case("manifest.json"));
    let Some(manifest_file) = manifest_file else {
        return Ok(None);
    };

    // A real manifest is a few hundred bytes; a huge one is not a package, and is never read.
    if manifest_file.size > MAX_MANIFEST_BYTES {
        return Ok(None);
    }
    let obj_path = ctx.paths.content_object_path(&manifest_file.sha256);
    // A missing or unreadable object is a damaged store, not "not a package".
    let bytes = std::fs::read(&obj_path)?;
    Ok(parse_manifest_text(&bytes))
}

/// The largest `manifest.json` read as a Thunderstore manifest.
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

/// Parse a Thunderstore `manifest.json`: a JSON object with string `name` and `version_number` and
/// an array of strings `dependencies`, after an optional UTF-8 byte-order mark (real packages have
/// one). The name must follow Thunderstore's rule, since it becomes a folder name. Anything else is
/// not a package. `serde_json` bounds nesting, so a hostile file cannot exhaust the stack.
pub fn parse_manifest_text(bytes: &[u8]) -> Option<ThunderstoreManifest> {
    let bytes = bytes
        .strip_prefix(b"\xEF\xBB\xBF".as_slice())
        .unwrap_or(bytes);
    let manifest: ThunderstoreManifest = serde_json::from_slice(bytes).ok()?;
    is_thunderstore_name(&manifest.name).then_some(manifest)
}

/// Determine the package ID for an item (`Namespace-Name` if from archive filename, else `Name`).
pub fn resolve_package_id(item: &ContentItem, manifest: &ThunderstoreManifest) -> String {
    let mut file_stem = item.name.as_str();
    for s in &item.sources {
        if let ContentSource::Archive { path, .. } = s {
            if let Some(name) = Path::new(path).file_name() {
                if let Some(stem_str) = name.to_str() {
                    file_stem = stem_str;
                    break;
                }
            }
        }
    }
    extract_thunderstore_package_id(file_stem, &manifest.name, &manifest.version_number)
}

/// Install a Thunderstore package into an instance as a derived item.
pub fn install_thunderstore(
    ctx: &Ctx,
    instance_id: &str,
    item_id: &str,
) -> Result<ThunderstoreInstallOutcome, ThunderstoreInstallError> {
    let item = content_store::get_item(ctx, item_id)?;
    let manifest =
        parse_manifest(ctx, item_id)?.ok_or(ThunderstoreInstallError::NotThunderstorePackage)?;

    let package_id = resolve_package_id(&item, &manifest);

    let file_paths: Vec<RelPath> = item.files.iter().map(|f| f.path.clone()).collect();
    let mappings = map_thunderstore_bepinex(&file_paths, &package_id)?;

    // Build derived files: map each destination path to the source object's sha256
    let mut derived_files: Vec<(RelPath, String)> = Vec::with_capacity(mappings.len());
    for (src, dest) in &mappings {
        let source_file = item
            .files
            .iter()
            .find(|f| &f.path == src)
            .ok_or_else(|| ContentError::Other(format!("missing source file '{src}'")))?;
        derived_files.push((dest.clone(), source_file.sha256.clone()));
    }

    // Check for collisions with existing instance content layers
    let instance_manifest = game_instance::get_manifest(ctx, instance_id)?;
    for layer in instance_manifest.layers.layers() {
        if let LayerSource::Content {
            content: existing_id,
        } = &layer.source
        {
            if let Ok(existing_item) = content_store::get_item(ctx, existing_id) {
                let existing_pkg = existing_item
                    .sources
                    .iter()
                    .find_map(|s| match s {
                        ContentSource::Thunderstore { package, .. } => Some(package.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| existing_item.name.clone());

                for existing_file in &existing_item.files {
                    let mounted = if layer.mount_path.as_str().is_empty() {
                        existing_file.path.as_str().to_string()
                    } else {
                        format!(
                            "{}/{}",
                            layer.mount_path.as_str().trim_end_matches('/'),
                            existing_file.path.as_str()
                        )
                    };

                    for (dest, _) in &derived_files {
                        if dest.as_str().eq_ignore_ascii_case(&mounted) {
                            return Err(ThunderstoreInstallError::FileCollision {
                                path: dest.to_string(),
                                package: package_id.clone(),
                                existing_package: existing_pkg,
                            });
                        }
                    }
                }
            }
        }
    }

    // Derive item in content store
    let derived_name = format!("{package_id} {}", manifest.version_number);
    let source = ContentSource::Thunderstore {
        from_item: item_id.to_string(),
        package: package_id.clone(),
        version: manifest.version_number.clone(),
        added_at_unix_ms: content_store::now_unix_ms(),
    };
    let outcome = content_store::derive_item(ctx, item_id, derived_files, &derived_name, source)?;
    let derived = outcome.item().clone();

    // Add derived item to instance with mount_path "" and source_path ""
    let layer = game_deploy::add_content(ctx, instance_id, &derived.item_id, Some(""), Some(""))?;

    // Check dependencies against all content layers now in instance
    let updated_manifest = game_instance::get_manifest(ctx, instance_id)?;
    let mut provided_packages = HashSet::new();
    // Only enabled layers provide a package: a disabled pack does not load.
    for l in updated_manifest
        .layers
        .layers()
        .iter()
        .filter(|l| l.enabled)
    {
        if let LayerSource::Content { content: id } = &l.source {
            if let Ok(it) = content_store::get_item(ctx, id) {
                for s in &it.sources {
                    if let ContentSource::Thunderstore { package, .. } = s {
                        provided_packages.insert(package.to_ascii_lowercase());
                        if let Some((_, name)) = package.split_once('-') {
                            provided_packages.insert(name.to_ascii_lowercase());
                        }
                    }
                }
                provided_packages.insert(it.name.to_ascii_lowercase());
                if let Some(file_stem) = it.name.strip_suffix(".zip") {
                    provided_packages.insert(file_stem.to_ascii_lowercase());
                }
            }
        }
    }

    let mut missing_dependencies = Vec::new();
    for dep in &manifest.dependencies {
        // dep is of the form Namespace-Name-Version
        let dep_pkg = match dep.rsplit_once('-') {
            Some((pkg, _)) => pkg,
            None => dep.as_str(),
        };

        let found = provided_packages.contains(&dep_pkg.to_ascii_lowercase())
            || if let Some((_, name)) = dep_pkg.split_once('-') {
                provided_packages.contains(&name.to_ascii_lowercase())
            } else {
                false
            };

        if !found {
            missing_dependencies.push(dep_pkg.to_string());
        }
    }

    // Build human-readable summary of mappings
    let summary = build_mapping_summary(&file_paths, &mappings, &package_id);

    Ok(ThunderstoreInstallOutcome {
        derived_item: derived,
        layer,
        package_id,
        version: manifest.version_number,
        summary,
        missing_dependencies,
    })
}

fn build_mapping_summary(
    src_paths: &[RelPath],
    mappings: &[(RelPath, RelPath)],
    pkg: &str,
) -> String {
    // Check if BepInEx pack:
    for p in src_paths {
        let parts: Vec<&str> = p.as_str().split('/').filter(|s| !s.is_empty()).collect();
        if parts.len() >= 4
            && parts[1].eq_ignore_ascii_case("BepInEx")
            && parts[2].eq_ignore_ascii_case("core")
            && parts[3].eq_ignore_ascii_case("BepInEx.Preloader.dll")
        {
            return format!("{}/ → <root>", parts[0]);
        }
    }

    let mut rules = Vec::new();
    let mut has_plugins = false;
    let mut has_patchers = false;
    let mut has_monomod = false;
    let mut has_config = false;
    let mut has_core = false;
    let mut root_files = Vec::new();

    for (src, _) in mappings {
        let parts: Vec<&str> = src.as_str().split('/').filter(|s| !s.is_empty()).collect();
        if parts.is_empty() {
            continue;
        }
        let top = parts[0].to_ascii_lowercase();
        match top.as_str() {
            "plugins" => has_plugins = true,
            "patchers" => has_patchers = true,
            "monomod" => has_monomod = true,
            "config" => has_config = true,
            "core" => has_core = true,
            "manifest.json" | "icon.png" | "readme.md" | "changelog.md" => {}
            _ => {
                if parts.len() == 1 {
                    root_files.push(parts[0].to_string());
                } else {
                    root_files.push(format!("{}/", parts[0]));
                }
            }
        }
    }

    if has_plugins {
        rules.push(format!("plugins/ → BepInEx/plugins/{pkg}/"));
    }
    if has_patchers {
        rules.push(format!("patchers/ → BepInEx/patchers/{pkg}/"));
    }
    if has_monomod {
        rules.push(format!("monomod/ → BepInEx/monomod/{pkg}/"));
    }
    if has_config {
        rules.push("config/ → BepInEx/config/".to_string());
    }
    if has_core {
        rules.push("core/ → BepInEx/core/".to_string());
    }

    for f in &root_files {
        rules.push(format!("{f} → BepInEx/plugins/{pkg}/{f}"));
    }

    if rules.is_empty() {
        format!("<root> → BepInEx/plugins/{pkg}/")
    } else {
        rules.join(", ")
    }
}
