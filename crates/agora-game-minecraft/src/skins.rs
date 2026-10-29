//! Change the signed-in account's Minecraft skin and cape, and keep a small
//! library of skins on this computer to switch between.
//!
//! Talks to the same Minecraft services API the official launcher uses, with
//! the active Microsoft account's token. The library is local files only;
//! choosing a skin from it uploads that file.

use agora_core::ctx::Ctx;
use agora_core::error::{LauncherError, LauncherResult};
use agora_core::http_client::ClientCategory;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";
const SKINS_URL: &str = "https://api.minecraftservices.com/minecraft/profile/skins";
const ACTIVE_SKIN_URL: &str = "https://api.minecraftservices.com/minecraft/profile/skins/active";
const ACTIVE_CAPE_URL: &str = "https://api.minecraftservices.com/minecraft/profile/capes/active";
/// Skin files are a few kilobytes; anything near this is not a skin.
const MAX_SKIN_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkinVariant {
    /// Four-pixel arms ("Steve").
    Classic,
    /// Three-pixel arms ("Alex").
    Slim,
}

impl SkinVariant {
    fn api_name(self) -> &'static str {
        match self {
            SkinVariant::Classic => "classic",
            SkinVariant::Slim => "slim",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cape {
    pub id: String,
    pub alias: String,
    pub active: bool,
    /// The cape texture as a `data:` URL, when it could be fetched.
    pub image_data_url: Option<String>,
}

/// What the account is wearing now, and the capes it owns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkinProfile {
    pub username: String,
    pub variant: SkinVariant,
    /// The active skin texture as a `data:` URL.
    pub skin_data_url: Option<String>,
    pub capes: Vec<Cape>,
}

/// A skin saved on this computer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSkin {
    pub id: String,
    pub name: String,
    pub variant: SkinVariant,
    pub data_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LibraryEntry {
    id: String,
    name: String,
    variant: SkinVariant,
}

fn error(code: &str, message: impl Into<String>) -> LauncherError {
    LauncherError::Generic {
        code: code.into(),
        message: message.into(),
    }
}

async fn token(ctx: &Ctx) -> LauncherResult<String> {
    match crate::msa::get_valid_credentials(&ctx.http_clients).await {
        crate::msa::MsaCredentialOutcome::Valid(creds) => Ok(creds.access_token),
        crate::msa::MsaCredentialOutcome::SignInRequired => Err(LauncherError::MsaAuthRequired),
        crate::msa::MsaCredentialOutcome::RefreshFailed(error) => Err(error),
    }
}

fn bearer(token: &str) -> Vec<(String, String)> {
    vec![("Authorization".into(), format!("Bearer {token}"))]
}

/// Check that `bytes` is a PNG of a size Minecraft accepts (64×64, or the
/// legacy 64×32), reading the header rather than decoding the image.
pub fn validate_skin_png(bytes: &[u8]) -> LauncherResult<()> {
    let invalid = || {
        error(
            "ERR_SKIN_FILE",
            "A skin must be a 64×64 (or 64×32) PNG image.",
        )
    };
    if bytes.len() > MAX_SKIN_BYTES || bytes.len() < 24 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n")
    {
        return Err(invalid());
    }
    if &bytes[12..16] != b"IHDR" {
        return Err(invalid());
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    if width == 64 && (height == 64 || height == 32) {
        Ok(())
    } else {
        Err(invalid())
    }
}

fn png_data_url(bytes: &[u8]) -> String {
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

/// Fetch a texture from Mojang's texture server. The API hands out `http://`
/// links; the same files are served over HTTPS.
async fn texture(ctx: &Ctx, url: &str) -> Option<String> {
    let url = url.replacen("http://", "https://", 1);
    if !url.starts_with("https://textures.minecraft.net/") {
        return None;
    }
    let bytes = agora_core::http_client::checked_get_bytes(
        &ctx.http_clients,
        ClientCategory::Microsoft,
        &url,
    )
    .await
    .ok()?;
    (bytes.len() <= MAX_SKIN_BYTES && bytes.starts_with(b"\x89PNG")).then(|| png_data_url(&bytes))
}

async fn profile_json(ctx: &Ctx, token: &str) -> LauncherResult<serde_json::Value> {
    let response = agora_core::http_client::checked_send(
        &ctx.http_clients,
        ClientCategory::Microsoft,
        reqwest::Method::GET,
        PROFILE_URL,
        &bearer(token),
        None,
        None,
    )
    .await?;
    read_reply(response, "read your Minecraft profile").await
}

async fn read_reply(response: reqwest::Response, doing: &str) -> LauncherResult<serde_json::Value> {
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(LauncherError::MsaAuthRequired);
    }
    if !status.is_success() {
        let reason = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|body| {
                body.get("errorMessage")
                    .or_else(|| body.get("error"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| status.to_string());
        return Err(error(
            "ERR_SKIN_API",
            format!("Could not {doing}: {reason}"),
        ));
    }
    Ok(serde_json::from_str(&text).unwrap_or(serde_json::Value::Null))
}

/// The active account's skin and capes.
pub async fn profile(ctx: &Ctx) -> LauncherResult<SkinProfile> {
    let token = token(ctx).await?;
    let json = profile_json(ctx, &token).await?;
    let active_skin = json
        .get("skins")
        .and_then(|s| s.as_array())
        .and_then(|skins| {
            skins
                .iter()
                .find(|skin| skin.get("state").and_then(|s| s.as_str()) == Some("ACTIVE"))
                .cloned()
        });
    let variant = match active_skin
        .as_ref()
        .and_then(|skin| skin.get("variant"))
        .and_then(|v| v.as_str())
    {
        Some(v) if v.eq_ignore_ascii_case("slim") => SkinVariant::Slim,
        _ => SkinVariant::Classic,
    };
    let skin_data_url = match active_skin
        .as_ref()
        .and_then(|skin| skin.get("url"))
        .and_then(|u| u.as_str())
    {
        Some(url) => texture(ctx, url).await,
        None => None,
    };
    let mut capes = Vec::new();
    for cape in json
        .get("capes")
        .and_then(|c| c.as_array())
        .into_iter()
        .flatten()
    {
        let (Some(id), Some(alias)) = (
            cape.get("id").and_then(|v| v.as_str()),
            cape.get("alias").and_then(|v| v.as_str()),
        ) else {
            continue;
        };
        let image_data_url = match cape.get("url").and_then(|u| u.as_str()) {
            Some(url) => texture(ctx, url).await,
            None => None,
        };
        capes.push(Cape {
            id: id.to_string(),
            alias: alias.to_string(),
            active: cape.get("state").and_then(|s| s.as_str()) == Some("ACTIVE"),
            image_data_url,
        });
    }
    Ok(SkinProfile {
        username: json
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or_default()
            .to_string(),
        variant,
        skin_data_url,
        capes,
    })
}

/// Upload a skin image and make it the active skin.
pub async fn upload_skin(ctx: &Ctx, png: &[u8], variant: SkinVariant) -> LauncherResult<()> {
    validate_skin_png(png)?;
    let token = token(ctx).await?;
    let boundary = format!("agora-{}", uuid::Uuid::new_v4().simple());
    let mut body = Vec::with_capacity(png.len() + 512);
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"variant\"\r\n\r\n{}\r\n",
            variant.api_name()
        )
        .as_bytes(),
    );
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"skin.png\"\r\nContent-Type: image/png\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(png);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let content_type = format!("multipart/form-data; boundary={boundary}");
    let response = agora_core::http_client::checked_send(
        &ctx.http_clients,
        ClientCategory::Microsoft,
        reqwest::Method::POST,
        SKINS_URL,
        &bearer(&token),
        Some(body),
        Some(&content_type),
    )
    .await?;
    read_reply(response, "change your skin").await.map(|_| ())
}

/// Go back to the default skin.
pub async fn reset_skin(ctx: &Ctx) -> LauncherResult<()> {
    let token = token(ctx).await?;
    let response = agora_core::http_client::checked_send(
        &ctx.http_clients,
        ClientCategory::Microsoft,
        reqwest::Method::DELETE,
        ACTIVE_SKIN_URL,
        &bearer(&token),
        None,
        None,
    )
    .await?;
    read_reply(response, "reset your skin").await.map(|_| ())
}

/// Show one of the account's capes, or hide capes with `None`.
pub async fn set_cape(ctx: &Ctx, cape_id: Option<&str>) -> LauncherResult<()> {
    let token = token(ctx).await?;
    let response = match cape_id {
        Some(id) => {
            let body = serde_json::to_vec(&serde_json::json!({ "capeId": id }))
                .map_err(|e| error("ERR_SKIN_API", e.to_string()))?;
            agora_core::http_client::checked_send(
                &ctx.http_clients,
                ClientCategory::Microsoft,
                reqwest::Method::PUT,
                ACTIVE_CAPE_URL,
                &bearer(&token),
                Some(body),
                Some("application/json"),
            )
            .await?
        }
        None => {
            agora_core::http_client::checked_send(
                &ctx.http_clients,
                ClientCategory::Microsoft,
                reqwest::Method::DELETE,
                ACTIVE_CAPE_URL,
                &bearer(&token),
                None,
                None,
            )
            .await?
        }
    };
    read_reply(response, "change your cape").await.map(|_| ())
}

// ---------------------------------------------------------------------------
// Local library
// ---------------------------------------------------------------------------

fn library_dir(ctx: &Ctx) -> PathBuf {
    ctx.paths.root().join("skins")
}

fn read_index(dir: &Path) -> Vec<LibraryEntry> {
    std::fs::read_to_string(dir.join("library.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_index(dir: &Path, entries: &[LibraryEntry]) -> LauncherResult<()> {
    let text = serde_json::to_string_pretty(entries)
        .map_err(|e| error("ERR_SKIN_LIBRARY", e.to_string()))?;
    let partial = dir.join("library.json.partial");
    std::fs::write(&partial, text)
        .and_then(|()| std::fs::rename(&partial, dir.join("library.json")))
        .map_err(|e| {
            error(
                "ERR_SKIN_LIBRARY",
                format!("Could not save the skin library: {e}"),
            )
        })
}

/// Saved skins, oldest first.
pub fn library(ctx: &Ctx) -> Vec<SavedSkin> {
    let dir = library_dir(ctx);
    read_index(&dir)
        .into_iter()
        .filter_map(|entry| {
            let bytes = std::fs::read(dir.join(format!("{}.png", entry.id))).ok()?;
            Some(SavedSkin {
                data_url: png_data_url(&bytes),
                id: entry.id,
                name: entry.name,
                variant: entry.variant,
            })
        })
        .collect()
}

/// Save a skin image into the library.
pub fn save_to_library(
    ctx: &Ctx,
    name: &str,
    variant: SkinVariant,
    png: &[u8],
) -> LauncherResult<SavedSkin> {
    validate_skin_png(png)?;
    let name = match name.trim() {
        "" => "Skin",
        name => name,
    };
    let dir = library_dir(ctx);
    std::fs::create_dir_all(&dir).map_err(|e| error("ERR_SKIN_LIBRARY", e.to_string()))?;
    let id = uuid::Uuid::new_v4().simple().to_string();
    std::fs::write(dir.join(format!("{id}.png")), png)
        .map_err(|e| error("ERR_SKIN_LIBRARY", format!("Could not save the skin: {e}")))?;
    let mut entries = read_index(&dir);
    entries.push(LibraryEntry {
        id: id.clone(),
        name: name.chars().take(64).collect(),
        variant,
    });
    write_index(&dir, &entries)?;
    Ok(SavedSkin {
        data_url: png_data_url(png),
        id,
        name: name.to_string(),
        variant,
    })
}

pub fn remove_from_library(ctx: &Ctx, id: &str) -> LauncherResult<()> {
    let dir = library_dir(ctx);
    let mut entries = read_index(&dir);
    let before = entries.len();
    entries.retain(|entry| entry.id != id);
    if entries.len() == before {
        return Err(error(
            "ERR_SKIN_LIBRARY",
            "That skin is not in the library.",
        ));
    }
    write_index(&dir, &entries)?;
    if id.chars().all(|c| c.is_ascii_hexdigit()) {
        let _ = std::fs::remove_file(dir.join(format!("{id}.png")));
    }
    Ok(())
}

/// Upload a saved skin and make it active.
pub async fn use_saved(ctx: &Ctx, id: &str) -> LauncherResult<()> {
    let dir = library_dir(ctx);
    let entry = read_index(&dir)
        .into_iter()
        .find(|entry| entry.id == id)
        .ok_or_else(|| error("ERR_SKIN_LIBRARY", "That skin is not in the library."))?;
    let png = std::fs::read(dir.join(format!("{}.png", entry.id)))
        .map_err(|e| error("ERR_SKIN_LIBRARY", format!("Could not read the skin: {e}")))?;
    upload_skin(ctx, &png, entry.variant).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        bytes
    }

    #[test]
    fn only_minecraft_sized_pngs_are_skins() {
        assert!(validate_skin_png(&png(64, 64)).is_ok());
        assert!(validate_skin_png(&png(64, 32)).is_ok());
        assert!(validate_skin_png(&png(128, 128)).is_err());
        assert!(validate_skin_png(b"GIF89a not a png at all......").is_err());
    }

    #[test]
    fn library_saves_lists_and_removes() {
        let root = tempfile::tempdir().unwrap();
        let ctx = Ctx::for_testing(root.path().to_path_buf());
        let saved = save_to_library(&ctx, "Knight", SkinVariant::Slim, &png(64, 64)).unwrap();
        let listed = library(&ctx);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "Knight");
        assert_eq!(listed[0].variant, SkinVariant::Slim);
        assert!(listed[0].data_url.starts_with("data:image/png;base64,"));
        assert!(save_to_library(&ctx, "Big", SkinVariant::Classic, &png(128, 128)).is_err());
        remove_from_library(&ctx, &saved.id).unwrap();
        assert!(library(&ctx).is_empty());
        assert!(remove_from_library(&ctx, &saved.id).is_err());
    }
}
