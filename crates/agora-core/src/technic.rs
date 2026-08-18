//! Technic modpack source (consent tiers S and Z).
//!
//! Technic is *more* open than Modrinth: no API key, just the mandatory
//! `?build=stable4` build gate, open file downloads, and no integrity floor.
//! Measured 2026-08: bare-zip packs dominate (Dropbox, `puu.sh`, `cld.pt`,
//! raw IPs); Solder packs report MD5 only; and `forge` is `None` on every
//! pack — the loader ships as a rehosted zip mod entry.
//!
//! Agora classifies Technic content into consent tiers:
//! - **Solder (S)** — each mod is individually addressable with an MD5 (when
//!   present) over HTTPS. Reasonable transport-integrity, never authenticity.
//! - **Zip (Z)** — a bare archive with no integrity information at all.
//!
//! All of it rides the non-negotiable floor: no loopback / private / link-local
//! destinations, per-category size caps, path-traversal rejection, and
//! per-hop redirect re-validation. Every request here goes through
//! [`ClientCategory::ConsentedContent`] with
//! [`HostPolicy::UserConsented`](crate::http_client::HostPolicy::UserConsented),
//! and the consent gate lives in core: `technic_enabled` for both tiers and
//! additionally `allow_unverified_packs` for Tier Z. The category's empty
//! allowlist keeps the generic Allowlist path failed-closed.

use crate::ctx::Ctx;
use crate::db;
use crate::error::{LauncherError, LauncherResult};
use crate::http_client::{self, ClientCategory, HostPolicy, HttpClients};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

const TECHNIC_API: &str = "https://api.technicpack.net";

/// Consent tier for a Technic pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TechnicTier {
    /// Solder-backed: each mod is individually addressable (MD5 when reported).
    Solder,
    /// A bare archive with no integrity information.
    Zip,
}

impl TechnicTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Solder => "solder",
            Self::Zip => "zip",
        }
    }
}

/// A single Technic search result, classified into a consent tier.
#[derive(Debug, Clone, Serialize)]
pub struct TechnicSearchResult {
    pub slug: String,
    pub title: String,
    pub description: String,
    pub installs: u64,
    pub likes: u64,
    pub author: Option<String>,
    pub page_url: String,
    pub tier: TechnicTier,
}

/// Full detail for a Technic pack, with the tier and whether the user's
/// current consent settings permit installing it.
#[derive(Debug, Clone, Serialize)]
pub struct TechnicPackDetail {
    pub slug: String,
    pub title: String,
    pub description: String,
    pub installs: u64,
    pub likes: u64,
    pub author: Option<String>,
    pub solder: Option<String>,
    pub recommended_build: Option<String>,
    pub minecraft: Option<String>,
    pub website: Option<String>,
    pub page_url: String,
    /// Direct pack download URL, present only for no-Solder (zip) packs.
    pub download_url: Option<String>,
    pub tier: TechnicTier,
    /// Whether the user's current consent settings allow installing this tier.
    /// Tier Z stays *visible* but not installable until `allow_unverified_packs`.
    pub permitted: bool,
}

// ---------------------------------------------------------------------------
// Consent gate (core-owned; a frontend bug cannot widen it)
// ---------------------------------------------------------------------------

/// Read a boolean setting, defaulting to `default` when absent or unreadable.
/// Accepts JSON booleans and legacy JSON strings (`"true"` / `"1"`).
fn setting_bool(conn: &Connection, key: &str, default: bool) -> bool {
    db::get_setting(conn, key)
        .ok()
        .flatten()
        .map(|value| match value {
            serde_json::Value::Bool(b) => b,
            serde_json::Value::String(s) => s == "true" || s == "1",
            serde_json::Value::Number(n) => n.as_i64() == Some(1),
            _ => default,
        })
        .unwrap_or(default)
}

fn consent_backend_available(conn: &Connection) -> bool {
    !db::is_lockdown_enabled(conn)
}

/// Validate the user's consent for a Technic tier *before* any request is
/// issued. Tier S requires `technic_enabled`; Tier Z additionally requires
/// `allow_unverified_packs`. Consent is default-off.
pub fn consent_for_tier(conn: &rusqlite::Connection, tier: TechnicTier) -> LauncherResult<()> {
    if !consent_backend_available(conn) {
        return Err(LauncherError::Generic {
            code: "ERR_NETWORK_DISABLED".into(),
            message: "Technic content requires network access, which is disabled by Privacy Lockdown Mode.".into(),
        });
    }
    let technic_enabled = setting_bool(conn, "technic_enabled", false);
    if !technic_enabled {
        return Err(LauncherError::Generic {
            code: "ERR_TECHNIC_DISABLED".into(),
            message: "Technic browsing is disabled in Settings.".into(),
        });
    }
    if tier == TechnicTier::Zip {
        let allow_unverified = setting_bool(conn, "allow_unverified_packs", false);
        if !allow_unverified {
            return Err(LauncherError::Generic {
                code: "ERR_UNVERIFIED_PACKS_DISABLED".into(),
                message: "Unverified zip packs are disabled in Settings; Agora cannot verify these files.".into(),
            });
        }
    }
    Ok(())
}

/// Whether the current settings permit a given tier (no error text).
pub fn tier_permitted(conn: &rusqlite::Connection, tier: TechnicTier) -> bool {
    consent_for_tier(conn, tier).is_ok()
}

// ---------------------------------------------------------------------------
// HTTP — always through ConsentedContent + UserConsented
// ---------------------------------------------------------------------------

async fn technic_get_bytes(clients: &HttpClients, url: &str) -> LauncherResult<Vec<u8>> {
    // The request itself is user-consented third-party content, so it is
    // logged here like any other consented-content fetch. The consent gate is
    // enforced at the API boundary (search/detail) and in the install path.
    eprintln!(
        "[technic] fetch host={} url={}",
        reqwest::Url::parse(url)
            .ok()
            .and_then(|parsed| parsed.host_str().map(str::to_string))
            .unwrap_or_default(),
        crate::network::sanitized_url_for_log(url)
    );
    http_client::checked_get_bytes_with_policy(
        clients,
        ClientCategory::ConsentedContent,
        url,
        HostPolicy::UserConsented,
    )
    .await
}

async fn technic_get_json<T: serde::de::DeserializeOwned>(
    clients: &HttpClients,
    url: &str,
) -> LauncherResult<T> {
    let bytes = technic_get_bytes(clients, url).await?;
    serde_json::from_slice(&bytes).map_err(|error| LauncherError::Generic {
        code: "ERR_TECHNIC_DECODE".into(),
        message: format!("Failed to decode Technic response: {error}"),
    })
}

// ---------------------------------------------------------------------------
// Raw API shapes (measured 2026-08 against api.technicpack.net)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct TechnicSearchHit {
    /// Display name when present in the hit.
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    slug: Option<String>,
    /// Modpack page URL.
    #[serde(default)]
    url: Option<String>,
}

#[derive(Deserialize)]
struct TechnicSearchResponse {
    #[serde(default)]
    modpacks: Vec<TechnicSearchHit>,
    // Most clients read `modpacks`; tolerate the older `results` shape too.
    #[serde(default)]
    results: Vec<TechnicSearchHit>,
}

#[derive(Deserialize)]
struct TechnicModpackResponse {
    #[serde(default)]
    name: Option<String>,
    #[serde(rename = "displayName", alias = "display_name", default)]
    display_name: Option<String>,
    /// Author username.
    #[serde(default)]
    user: Option<String>,
    /// Direct pack download URL — present only for no-Solder (zip) packs.
    #[serde(default)]
    url: Option<String>,
    #[serde(rename = "platformUrl", alias = "platform_url", default)]
    platform_url: Option<String>,
    #[serde(default)]
    minecraft: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    solder: Option<String>,
    #[serde(default)]
    recommended: Option<String>,
    #[serde(default)]
    latest: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    website: Option<String>,
    #[serde(default)]
    link: Option<String>,
    /// Install/run popularity counters (present on the detail endpoint).
    #[serde(default)]
    installs: Option<u64>,
    #[serde(default)]
    runs: Option<u64>,
    /// The API reports `{"error": "..."}` for missing/refused packs.
    #[serde(default)]
    error: Option<String>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Search Technic for modpacks, classifying each page member into its tier.
///
/// The search index does not expose the Solder flag, so each result needs a
/// detail fetch to classify. `limit` caps the page (and thus the number of
/// detail calls). Consent for browsing is Tier-S-level: a result that turns
/// out to be a bare zip stays in the list but is reported as Tier Z and the
/// frontend keeps it non-installable until `allow_unverified_packs`.
pub async fn search_technic(
    ctx: &Ctx,
    query: &str,
    limit: u32,
) -> LauncherResult<Vec<TechnicSearchResult>> {
    let conn = db::local_state_connection(&ctx.paths.local_state_db()).map_err(|error| {
        LauncherError::Generic {
            code: "ERR_LOCAL_STATE_FAILED".into(),
            message: error.to_string(),
        }
    })?;
    consent_for_tier(&conn, TechnicTier::Solder)?;
    drop(conn);
    search_technic_http(query, limit).await
}

/// HTTP-only search; callers must have already validated consent.
pub async fn search_technic_http(
    query: &str,
    limit: u32,
) -> LauncherResult<Vec<TechnicSearchResult>> {
    let clients = HttpClients::new()?;
    let limit = limit.clamp(1, 50);
    let q = query.trim();
    let url = format!(
        "{TECHNIC_API}/search?build=stable4&q={}&limit={limit}",
        urlencoding::encode(q)
    );
    let response: TechnicSearchResponse = technic_get_json(&clients, &url).await?;

    let mut hits: Vec<TechnicSearchHit> = response.modpacks;
    hits.extend(response.results);
    hits.truncate(limit as usize);

    let mut results = Vec::with_capacity(hits.len());
    for hit in hits {
        let slug = hit
            .slug
            .or(derive_slug_from_url(&hit.url))
            .unwrap_or_default();
        if slug.is_empty() {
            continue;
        }
        let detail = pack_detail_http(&clients, &slug).await.ok();
        let tier = match detail {
            Some(ref detail)
                if detail
                    .solder
                    .as_deref()
                    .is_some_and(|solder| !solder.is_empty()) =>
            {
                TechnicTier::Solder
            }
            // No reachable solder endpoint (or no detail at all): bare zip.
            _ => TechnicTier::Zip,
        };
        let title = detail
            .as_ref()
            .map(|d| d.title.clone())
            .or_else(|| hit.name.clone())
            .unwrap_or_else(|| slug.clone());
        let page_url = detail
            .as_ref()
            .map(|d| d.page_url.clone())
            .or_else(|| hit.url.clone())
            .unwrap_or_else(|| format!("https://www.technicpack.net/modpack/{slug}"));
        results.push(TechnicSearchResult {
            title,
            slug: slug.clone(),
            description: detail
                .as_ref()
                .map(|d| d.description.clone())
                .unwrap_or_default(),
            installs: detail.as_ref().map(|d| d.installs).unwrap_or(0),
            likes: detail.as_ref().map(|d| d.likes).unwrap_or(0),
            author: detail.as_ref().and_then(|d| d.author.clone()),
            page_url,
            tier,
        });
    }
    Ok(results)
}

/// Derive a Technic slug from a page URL like
/// `https://www.technicpack.net/modpack/hexxit.552552`.
fn derive_slug_from_url(url: &Option<String>) -> Option<String> {
    let url = url.as_deref()?;
    let last = url.rsplit('/').next()?;
    let slug = last
        .split('.')
        .next()
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    if slug.is_empty() {
        None
    } else {
        Some(slug)
    }
}

/// Fetch full detail for a Technic pack, classifying its tier.
pub async fn pack_detail(ctx: &Ctx, slug: &str) -> LauncherResult<TechnicPackDetail> {
    let conn = db::local_state_connection(&ctx.paths.local_state_db()).map_err(|error| {
        LauncherError::Generic {
            code: "ERR_LOCAL_STATE_FAILED".into(),
            message: error.to_string(),
        }
    })?;
    consent_for_tier(&conn, TechnicTier::Solder)?;
    let clients = HttpClients::new()?;
    let mut detail = pack_detail_http(&clients, slug).await?;
    let permitted_conn =
        db::local_state_connection(&ctx.paths.local_state_db()).map_err(|error| {
            LauncherError::Generic {
                code: "ERR_LOCAL_STATE_FAILED".into(),
                message: error.to_string(),
            }
        })?;
    detail.permitted = tier_permitted(&permitted_conn, detail.tier);
    Ok(detail)
}

async fn pack_detail_http(clients: &HttpClients, slug: &str) -> LauncherResult<TechnicPackDetail> {
    let slug = slug.trim();
    if !valid_slug(slug) {
        return Err(LauncherError::Generic {
            code: "ERR_TECHNIC_SLUG".into(),
            message: format!("Invalid Technic modpack slug: {slug:?}"),
        });
    }
    let url = format!("{TECHNIC_API}/modpack/{slug}?build=stable4");
    let response: TechnicModpackResponse = technic_get_json(clients, &url).await?;
    if let Some(error) = response.error.filter(|error| !error.trim().is_empty()) {
        return Err(LauncherError::Generic {
            code: "ERR_TECHNIC_NOT_FOUND".into(),
            message: format!("Technic reports {slug:?} is unavailable: {error}"),
        });
    }
    let has_solder = response
        .solder
        .as_deref()
        .is_some_and(|solder| !solder.is_empty());
    let tier = if has_solder {
        TechnicTier::Solder
    } else {
        TechnicTier::Zip
    };
    let title = response
        .display_name
        .or_else(|| response.name.clone())
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| slug.to_string());
    Ok(TechnicPackDetail {
        slug: slug.to_string(),
        title,
        description: response.description.unwrap_or_default(),
        installs: response.installs.unwrap_or(0),
        likes: response.runs.unwrap_or(0),
        author: response.user,
        solder: response.solder,
        recommended_build: response
            .recommended
            .or(response.latest)
            .or(response.version),
        minecraft: response.minecraft,
        website: response.website,
        page_url: response
            .platform_url
            .or(response.link)
            .unwrap_or_else(|| format!("https://www.technicpack.net/modpack/{slug}")),
        download_url: response.url,
        tier,
        permitted: true,
    })
}

/// A slug is never joined into a path, but it is embedded in a URL; reject
/// separators and whitespace so it cannot be smuggled into another endpoint.
fn valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && !slug.contains('/')
        && !slug.contains('\\')
        && !slug.contains(char::is_whitespace)
}

// ---------------------------------------------------------------------------
// Solder build resolution (Tier S install)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct SolderModRaw {
    name: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    md5: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

#[derive(Deserialize)]
struct SolderModpackMeta {
    #[serde(rename = "name", default)]
    name: Option<String>,
    #[serde(rename = "display_name", default)]
    display_name: Option<String>,
}

#[derive(Deserialize)]
struct SolderBuildResponse {
    #[serde(default)]
    minecraft: Option<String>,
    #[serde(default)]
    modpack: Option<SolderModpackMeta>,
    #[serde(default)]
    mods: Vec<SolderModRaw>,
}

/// Resolve a Technic Solder build into an installable pack (Tier S).
///
/// `solder` is the Solder base URL from the pack detail (may be a plain-HTTP
/// raw-IP endpoint — the consented-content policy covers it). Every mod becomes
/// an individually-addressable entry verified by its reported MD5 (when
/// present). The rehosted `forge` loader zip is never installed: its version
/// is recovered and returned so the pipeline can resolve the official loader
/// from Agora's pinned `loader-manifests/`. Unknown loader versions fail
/// there with a clear message rather than us guessing.
pub async fn resolve_solder_build(
    ctx: &Ctx,
    solder: &str,
    slug: &str,
    build: &str,
) -> LauncherResult<crate::import::TechnicSolderPack> {
    let conn = db::local_state_connection(&ctx.paths.local_state_db()).map_err(|error| {
        LauncherError::Generic {
            code: "ERR_LOCAL_STATE_FAILED".into(),
            message: error.to_string(),
        }
    })?;
    consent_for_tier(&conn, TechnicTier::Solder)?;
    drop(conn);
    let clients = HttpClients::new()?;
    resolve_solder_build_http(&clients, solder, slug, build).await
}

async fn resolve_solder_build_http(
    clients: &HttpClients,
    solder: &str,
    slug: &str,
    build: &str,
) -> LauncherResult<crate::import::TechnicSolderPack> {
    if !valid_slug(slug) {
        return Err(LauncherError::Generic {
            code: "ERR_TECHNIC_SLUG".into(),
            message: format!("Invalid Technic modpack slug: {slug:?}"),
        });
    }
    if build.trim().is_empty() || build.contains('/') || build.contains('\\') {
        return Err(LauncherError::Generic {
            code: "ERR_TECHNIC_BUILD".into(),
            message: format!("Invalid Technic build identifier: {build:?}"),
        });
    }
    let base = solder.trim().trim_end_matches('/');
    if base.is_empty() {
        return Err(LauncherError::Generic {
            code: "ERR_TECHNIC_SOLDER".into(),
            message: "The Technic pack does not report a usable Solder endpoint.".into(),
        });
    }
    let url = format!("{base}/api/modpack/{slug}/{build}");
    let response: SolderBuildResponse = technic_get_json(clients, &url).await?;

    // The loader version is recovered from the rehosted `forge` mod entry. It
    // is never downloaded or installed from Technic — it only names the
    // official loader version to resolve from the pinned catalog.
    let mut forge_version: Option<String> = None;
    let mut mods = Vec::new();
    for raw in response.mods {
        if raw.name.eq_ignore_ascii_case("forge") {
            forge_version = raw.version.filter(|v| !v.trim().is_empty());
            continue;
        }
        let Some(url_value) = raw.url.filter(|u| !u.trim().is_empty()) else {
            continue;
        };
        mods.push(crate::import::TechnicSolderMod {
            name: raw.name,
            url: url_value,
            md5: raw.md5.filter(|m| !m.trim().is_empty()),
        });
    }
    let loader = if forge_version.is_some() { "forge" } else { "" };
    let display_name = response
        .modpack
        .as_ref()
        .and_then(|meta| meta.display_name.clone())
        .or_else(|| response.modpack.as_ref().and_then(|meta| meta.name.clone()))
        .unwrap_or_else(|| slug.to_string());
    Ok(crate::import::TechnicSolderPack {
        display_name,
        minecraft_version: response.minecraft.unwrap_or_default(),
        loader: loader.into(),
        loader_version: forge_version.unwrap_or_default(),
        mods,
    })
}

// ---------------------------------------------------------------------------
// Install entry points — consent gates live here in core
// ---------------------------------------------------------------------------

/// Install a resolved Solder pack through the standard import pipeline.
/// Re-checks Tier S consent (the `resolve_solder_build` resolution already
/// did, but the import must not be reachable without it).
pub async fn install_solder_pack(
    ctx: &Ctx,
    pack: crate::import::TechnicSolderPack,
) -> LauncherResult<crate::import::ImportResult> {
    let conn = db::local_state_connection(&ctx.paths.local_state_db()).map_err(|error| {
        LauncherError::Generic {
            code: "ERR_LOCAL_STATE_FAILED".into(),
            message: error.to_string(),
        }
    })?;
    consent_for_tier(&conn, TechnicTier::Solder)?;
    drop(conn);
    let svc = crate::import_service::ImportService::new(ctx.clone());
    svc.run_import(crate::import_service::ImportRequest {
        source: crate::import_service::ImportSource::TechnicSolder(pack),
        symlink_saves: false,
    })
    .await
}

/// Install a consented Technic zip archive. Tier Z requires BOTH
/// `technic_enabled` and `allow_unverified_packs`; this is the only entry
/// point that may start a zip import.
pub async fn install_zip_pack(
    ctx: &Ctx,
    pack: crate::import::TechnicZipPack,
) -> LauncherResult<crate::import::ImportResult> {
    let conn = db::local_state_connection(&ctx.paths.local_state_db()).map_err(|error| {
        LauncherError::Generic {
            code: "ERR_LOCAL_STATE_FAILED".into(),
            message: error.to_string(),
        }
    })?;
    consent_for_tier(&conn, TechnicTier::Zip)?;
    drop(conn);
    let svc = crate::import_service::ImportService::new(ctx.clone());
    svc.run_import(crate::import_service::ImportRequest {
        source: crate::import_service::ImportSource::TechnicZip(pack),
        symlink_saves: false,
    })
    .await
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctx::Ctx;

    fn mem_settings() -> (rusqlite::Connection, Ctx, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "agora-technic-tests-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let ctx = Ctx::for_testing(root.clone());
        crate::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();
        let conn = db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        (conn, ctx, root)
    }

    #[test]
    fn consent_defaults_to_off() {
        let (conn, _ctx, root) = mem_settings();
        assert!(consent_for_tier(&conn, TechnicTier::Solder).is_err());
        assert!(consent_for_tier(&conn, TechnicTier::Zip).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn technic_enabled_allows_solder_but_not_zip() {
        let (conn, _ctx, root) = mem_settings();
        db::set_setting(&conn, "technic_enabled", &serde_json::Value::Bool(true)).unwrap();
        assert!(consent_for_tier(&conn, TechnicTier::Solder).is_ok());
        assert!(consent_for_tier(&conn, TechnicTier::Zip).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn zip_requires_allow_unverified_packs() {
        let (conn, _ctx, root) = mem_settings();
        db::set_setting(&conn, "technic_enabled", &serde_json::Value::Bool(true)).unwrap();
        db::set_setting(
            &conn,
            "allow_unverified_packs",
            &serde_json::Value::Bool(true),
        )
        .unwrap();
        assert!(consent_for_tier(&conn, TechnicTier::Solder).is_ok());
        assert!(consent_for_tier(&conn, TechnicTier::Zip).is_ok());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn lockdown_blocks_every_tier() {
        let (conn, _ctx, root) = mem_settings();
        db::set_setting(&conn, "technic_enabled", &serde_json::Value::Bool(true)).unwrap();
        db::set_setting(
            &conn,
            "allow_unverified_packs",
            &serde_json::Value::Bool(true),
        )
        .unwrap();
        db::set_setting(
            &conn,
            "network_lockdown_enabled",
            &serde_json::Value::Bool(true),
        )
        .unwrap();
        assert!(consent_for_tier(&conn, TechnicTier::Solder).is_err());
        assert!(consent_for_tier(&conn, TechnicTier::Zip).is_err());
        assert!(!tier_permitted(&conn, TechnicTier::Solder));
        assert!(!tier_permitted(&conn, TechnicTier::Zip));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn tier_permitted_reflects_consent() {
        let (conn, _ctx, root) = mem_settings();
        assert!(!tier_permitted(&conn, TechnicTier::Solder));
        db::set_setting(&conn, "technic_enabled", &serde_json::Value::Bool(true)).unwrap();
        assert!(tier_permitted(&conn, TechnicTier::Solder));
        assert!(!tier_permitted(&conn, TechnicTier::Zip));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn pack_detail_rejects_polluted_slugs() {
        for slug in ["", "../evil", "a/b", "a\\b", "a b"] {
            assert!(!valid_slug(slug), "slug {slug:?} must be rejected");
        }
        for slug in ["skyfactory", "pack-with-dashes", "123"] {
            assert!(valid_slug(slug), "slug {slug:?} must be accepted");
        }
    }
}
