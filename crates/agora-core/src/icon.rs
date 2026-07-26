//! Safe user-selected icon storage for instances and installed mods.

use crate::app_paths::validate_path_component;
use crate::ctx::Ctx;
use crate::db;
use crate::error::{LauncherError, LauncherResult};
use crate::helpers::{atomic_write_manifest, read_manifest};
use crate::models::InstanceManifest;
use base64::Engine;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const MAX_ICON_BYTES: u64 = 10 * 1024 * 1024;

fn icon_error(message: impl Into<String>) -> LauncherError {
    LauncherError::Generic {
        code: "ERR_ICON".into(),
        message: message.into(),
    }
}

fn extension_and_mime(path: &Path) -> LauncherResult<(&'static str, &'static str)> {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => Ok(("png", "image/png")),
        "jpg" | "jpeg" => Ok(("jpg", "image/jpeg")),
        "webp" => Ok(("webp", "image/webp")),
        "gif" => Ok(("gif", "image/gif")),
        _ => Err(icon_error("Choose a PNG, JPEG, WebP, or GIF image.")),
    }
}

fn read_icon_source(path: &Path) -> LauncherResult<(Vec<u8>, &'static str, &'static str)> {
    let (extension, mime) = extension_and_mime(path)?;
    let metadata = std::fs::metadata(path).map_err(|error| icon_error(error.to_string()))?;
    if !metadata.is_file() {
        return Err(icon_error("The selected icon is not a file."));
    }
    if metadata.len() > MAX_ICON_BYTES {
        return Err(icon_error("Icons must be 10 MB or smaller."));
    }
    let bytes = std::fs::read(path).map_err(|error| icon_error(error.to_string()))?;
    let valid = match extension {
        "png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "jpg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        "webp" => bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP",
        "gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        _ => false,
    };
    if !valid {
        return Err(icon_error(
            "The selected file is not a valid image of its stated type.",
        ));
    }
    Ok((bytes, extension, mime))
}

fn data_url(path: &Path) -> LauncherResult<String> {
    let (bytes, _, mime) = read_icon_source(path)?;
    Ok(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

fn relative_icon_path(path: &Path, instance_dir: &Path) -> LauncherResult<String> {
    let relative = path
        .strip_prefix(instance_dir)
        .map_err(|_| icon_error("Icon path escaped the instance directory."))?;
    Ok(relative.to_string_lossy().replace('\\', "/"))
}

fn resolve_stored_path(instance_dir: &Path, relative: &str) -> LauncherResult<Option<PathBuf>> {
    if relative.is_empty()
        || Path::new(relative).is_absolute()
        || Path::new(relative).components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Ok(None);
    }
    let candidate = instance_dir.join(relative);
    if !candidate.starts_with(instance_dir) || !candidate.is_file() {
        return Ok(None);
    }
    Ok(Some(candidate))
}

fn copy_icon(
    source_path: &Path,
    instance_dir: &Path,
    destination: &Path,
) -> LauncherResult<String> {
    let (bytes, extension, _) = read_icon_source(source_path)?;
    let destination = destination.with_extension(extension);
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|error| icon_error(error.to_string()))?;
    }
    std::fs::write(&destination, bytes).map_err(|error| icon_error(error.to_string()))?;
    relative_icon_path(&destination, instance_dir)
}

fn remove_stored_icon(instance_dir: &Path, relative: Option<&str>) {
    if let Some(relative) = relative {
        if let Ok(Some(path)) = resolve_stored_path(instance_dir, relative) {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn custom_mod_icons(
    manifest: &mut InstanceManifest,
) -> &mut serde_json::Map<String, serde_json::Value> {
    if !manifest.user_preferences.is_object() {
        manifest.user_preferences = serde_json::json!({});
    }
    let preferences = manifest
        .user_preferences
        .as_object_mut()
        .expect("user_preferences was normalized to an object");
    let entry = preferences
        .entry("agora_custom_mod_icons")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    if !entry.is_object() {
        *entry = serde_json::Value::Object(serde_json::Map::new());
    }
    entry
        .as_object_mut()
        .expect("agora_custom_mod_icons was normalized to an object")
}

fn existing_custom_mod_icon(manifest: &InstanceManifest, filename: &str) -> Option<String> {
    manifest
        .user_preferences
        .get("agora_custom_mod_icons")
        .and_then(serde_json::Value::as_object)
        .and_then(|icons| icons.get(filename))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

pub fn set_instance_icon(
    ctx: &Ctx,
    instance_id: &str,
    source_path: &Path,
) -> LauncherResult<String> {
    validate_path_component(instance_id)?;
    let instance_dir = ctx.paths.instance_dir(instance_id)?;
    let conn = db::local_state_connection(&ctx.paths.local_state_db()).map_err(|error| {
        LauncherError::Generic {
            code: "ERR_LOCAL_STATE_FAILED".into(),
            message: error.to_string(),
        }
    })?;
    let row = db::get_instance(&conn, instance_id)
        .map_err(|error| icon_error(error.to_string()))?
        .ok_or_else(|| icon_error("Instance not found."))?;
    if row.is_locked {
        return Err(LauncherError::InstanceLocked);
    }
    if !row.is_modpack {
        return Err(icon_error(
            "Custom pack images are only available for modpack instances.",
        ));
    }
    let relative = copy_icon(
        source_path,
        &instance_dir,
        &instance_dir.join(".agora/instance-icon"),
    )?;
    remove_stored_icon(&instance_dir, row.icon_path.as_deref());
    db::set_instance_icon_path(&conn, instance_id, Some(&relative)).map_err(|error| {
        LauncherError::Generic {
            code: "ERR_LOCAL_STATE_FAILED".into(),
            message: error.to_string(),
        }
    })?;
    data_url(&instance_dir.join(&relative))
}

pub fn set_mod_icon(
    ctx: &Ctx,
    instance_id: &str,
    filename: &str,
    source_path: &Path,
) -> LauncherResult<String> {
    validate_path_component(instance_id)?;
    if filename.is_empty()
        || filename.contains('/')
        || filename.contains('\\')
        || filename.contains('\0')
    {
        return Err(icon_error("Invalid installed mod filename."));
    }
    let instance_dir = ctx.paths.instance_dir(instance_id)?;
    let manifest_path = ctx.paths.instance_manifest(instance_id)?;
    let mut manifest = read_manifest(&manifest_path)?;
    if manifest.is_locked {
        return Err(LauncherError::InstanceLocked);
    }
    let installed_mod = manifest
        .mods
        .iter()
        .find(|entry| entry.filename == filename)
        .ok_or_else(|| icon_error("Installed mod not found."))?;
    let mut digest = Sha256::new();
    digest.update(filename.as_bytes());
    digest.update(installed_mod.sha256.as_bytes());
    let stem = hex::encode(digest.finalize());
    let relative = copy_icon(
        source_path,
        &instance_dir,
        &instance_dir.join(format!(".agora/mod-icons/{stem}")),
    )?;
    let old_relative = existing_custom_mod_icon(&manifest, filename);
    remove_stored_icon(&instance_dir, old_relative.as_deref());
    custom_mod_icons(&mut manifest).insert(
        filename.to_owned(),
        serde_json::Value::String(relative.clone()),
    );
    atomic_write_manifest(&manifest_path, &manifest)?;
    data_url(&instance_dir.join(relative))
}

pub fn get_custom_icon(
    ctx: &Ctx,
    instance_id: &str,
    target: &str,
    filename: Option<&str>,
) -> LauncherResult<Option<String>> {
    validate_path_component(instance_id)?;
    let instance_dir = ctx.paths.instance_dir(instance_id)?;
    let relative = if target == "instance" {
        let conn = db::local_state_connection(&ctx.paths.local_state_db()).map_err(|error| {
            LauncherError::Generic {
                code: "ERR_LOCAL_STATE_FAILED".into(),
                message: error.to_string(),
            }
        })?;
        db::get_instance(&conn, instance_id)
            .map_err(|error| icon_error(error.to_string()))?
            .and_then(|row| row.icon_path)
    } else if target == "mod" {
        let filename = filename.ok_or_else(|| icon_error("Mod filename is required."))?;
        let manifest = read_manifest(&ctx.paths.instance_manifest(instance_id)?)?;
        existing_custom_mod_icon(&manifest, filename)
    } else {
        return Err(icon_error("Unknown icon target."));
    };
    let Some(path) =
        relative.and_then(|value| resolve_stored_path(&instance_dir, &value).ok().flatten())
    else {
        return Ok(None);
    };
    Ok(Some(data_url(&path)?))
}
