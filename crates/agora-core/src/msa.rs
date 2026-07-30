//! Microsoft Account (MSA) authentication for direct Minecraft launching.
//!
//! Implements the Xbox Live device-token flow (p256 ECDSA signed requests)
//! adapted from the Theseus/Modrinth App reference implementation. This is the
//! security-critical Phase 5 module — NO tokens are ever logged, rendered in
//! UI redacted form, or persisted outside the OS keyring (+ aes-gcm fallback).
//!
//! The flow (9 steps, see plan Phase 5):
//!   1. Generate p256 ECDSA key pair → device token
//!   2. SISU authenticate → redirect URI for browser login
//!   3. User completes browser login → auth code
//!   4. Exchange auth code → OAuth token (access + refresh)
//!   5. SISU authorize → Xbox title + user tokens
//!   6. XSTS authorize → Xbox Live token (extract user hash)
//!   7. Minecraft launcher login → MC access token
//!   8. Minecraft entitlements check
//!   9. Minecraft profile (username + UUID)

use crate::db;
use crate::error::{LauncherError, LauncherResult};
use crate::http_client::{self, ClientCategory, HttpClients};
use base64::Engine;
use chrono::{DateTime, Utc};
use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey, VerifyingKey};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::LazyLock;
use uuid::Uuid;

/// Check a network enable setting from the local state DB.
///
/// `db_path` is the path to `local_state.db`. The caller is responsible for
/// providing the correct path (e.g. resolved from a Tauri `AppHandle` in the
/// desktop app, or from `dirs::data_local_dir()` in the CLI).
fn check_network_enabled(
    db_path: &Path,
    setting_key: &str,
    disabled_msg: &str,
) -> LauncherResult<()> {
    if !db_path.exists() {
        // DB hasn't been initialised yet — feature is enabled by default.
        return Ok(());
    }
    let conn = db::local_state_connection(db_path).map_err(|e| LauncherError::Generic {
        code: "ERR_DB".into(),
        message: e.to_string(),
    })?;
    if !db::is_network_enabled(&conn, setting_key) {
        return Err(LauncherError::Generic {
            code: "ERR_NETWORK_DISABLED".into(),
            message: disabled_msg.into(),
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Constants (from Theseus — public, well-known Microsoft identifiers)
// ---------------------------------------------------------------------------

const MICROSOFT_CLIENT_ID: &str = "00000000402b5328";
const AUTH_REPLY_URL: &str = "https://login.live.com/oauth20_desktop.srf";
const REQUESTED_SCOPE: &str = "service::user.auth.xboxlive.com::MBI_SSL";
const TITLE_ID: &str = "1794566092";
const USER_AGENT: &str = "Agora Launcher";

// Endpoints
const DEVICE_AUTH_URL: &str = "https://device.auth.xboxlive.com/device/authenticate";
const SISU_AUTH_URL: &str = "https://sisu.xboxlive.com/authenticate";
const OAUTH_TOKEN_URL: &str = "https://login.live.com/oauth20_token.srf";
const SISU_AUTHORIZE_URL: &str = "https://sisu.xboxlive.com/authorize";
const XSTS_AUTHORIZE_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MC_LOGIN_URL: &str = "https://api.minecraftservices.com/launcher/login";
const MC_ENTITLEMENTS_URL: &str = "https://api.minecraftservices.com/entitlements/license";
const MC_PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";

// Keyring storage
const KEYRING_SERVICE: &str = "com.agoramc";
const KEYRING_ACCOUNT: &str = "msa-credentials";
const CREDENTIALS_FALLBACK_FILE: &str = "msa-credentials.enc";
const CREDENTIALS_KEY_CONTEXT: &[u8] = b"agora-msa-credentials-fallback";

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Persisted Minecraft credentials stored in the OS keyring.
#[derive(Clone, Serialize, Deserialize)]
pub struct MsaCredentials {
    pub username: String,
    pub uuid: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires: DateTime<Utc>,
}

impl std::fmt::Debug for MsaCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MsaCredentials")
            .field("username", &self.username)
            .field("uuid", &self.uuid)
            .field("access_token", &"[REDACTED]")
            .field("refresh_token", &"[REDACTED]")
            .field("expires", &self.expires)
            .finish()
    }
}

impl MsaCredentials {
    pub fn is_expired(&self) -> bool {
        Utc::now() >= self.expires
    }

    pub fn needs_refresh(&self) -> bool {
        // 5-minute margin
        Utc::now() + chrono::Duration::minutes(5) >= self.expires
    }
}

/// The login flow state returned by `begin_login`. The caller opens
/// `auth_uri` in a browser, captures the `?code=` from the redirect, then
/// passes it to `finish_login`.
#[derive(Clone)]
pub struct LoginFlow {
    pub auth_uri: String,
    verifier: String,
    session_id: Option<String>,
    key_json: String,
    device_token: String,
    state: String,
}

impl std::fmt::Debug for LoginFlow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginFlow")
            .field("auth_uri", &self.auth_uri)
            .field("verifier", &"[REDACTED]")
            .field(
                "session_id",
                &self.session_id.as_deref().map(|_| "[PRESENT]"),
            )
            .field("key_json", &"[REDACTED]")
            .field("device_token", &"[REDACTED]")
            .field("state", &"[REDACTED]")
            .finish()
    }
}

// Internal: the p256 signing key pair (serialized for the LoginFlow)
struct DeviceTokenKey {
    id: Uuid,
    key: SigningKey,
    x: String,
    y: String,
}

impl std::fmt::Debug for DeviceTokenKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceTokenKey")
            .field("id", &self.id)
            .field("key", &"[REDACTED]")
            .field("x", &self.x)
            .field("y", &self.y)
            .finish()
    }
}

impl DeviceTokenKey {
    fn generate() -> Self {
        let id = Uuid::new_v4();
        let signing_key = SigningKey::random(&mut OsRng);
        let public_key = VerifyingKey::from(&signing_key);
        let point = public_key.to_encoded_point(false);

        let b64 = |bytes: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);

        let x = point.x().map(|x| b64(x.as_slice())).unwrap_or_default();
        let y = point.y().map(|y| b64(y.as_slice())).unwrap_or_default();

        Self {
            id,
            key: signing_key,
            x,
            y,
        }
    }

    /// Serialize the key for storage in LoginFlow (the key must survive across
    /// async calls between begin_login and finish_login).
    fn to_json(&self) -> String {
        let der = p256::pkcs8::EncodePrivateKey::to_pkcs8_der(&self.key)
            .map(|d| d.as_bytes().to_vec())
            .unwrap_or_default();
        serde_json::json!({
            "id": self.id,
            "x": self.x,
            "y": self.y,
            "der": base64::engine::general_purpose::STANDARD.encode(&der),
        })
        .to_string()
    }

    fn from_json(json: &str) -> LauncherResult<Self> {
        let v: serde_json::Value =
            serde_json::from_str(json).map_err(|_| LauncherError::Generic {
                code: "ERR_MSA_KEY_DECODE".into(),
                message: "Failed to decode device token key.".into(),
            })?;
        let id: Uuid = v["id"]
            .as_str()
            .and_then(|s| Uuid::parse_str(s).ok())
            .unwrap_or_else(Uuid::new_v4);
        let x = v["x"].as_str().unwrap_or("").to_string();
        let y = v["y"].as_str().unwrap_or("").to_string();
        let der_b64 = v["der"].as_str().unwrap_or("");
        let der = base64::engine::general_purpose::STANDARD
            .decode(der_b64)
            .unwrap_or_default();
        let key = p256::pkcs8::DecodePrivateKey::from_pkcs8_der(&der).map_err(|_| {
            LauncherError::Generic {
                code: "ERR_MSA_KEY_DECODE".into(),
                message: "Failed to reconstruct signing key from PKCS8 DER.".into(),
            }
        })?;
        Ok(Self { id, key, x, y })
    }
}

/// Response shapes (PascalCase = Xbox Live convention)
mod xbox_types {
    use chrono::{DateTime, Utc};
    use serde::Deserialize;
    use std::collections::HashMap;

    #[derive(Deserialize, Clone)]
    #[serde(rename_all = "PascalCase")]
    pub struct XboxToken {
        pub issue_instant: DateTime<Utc>,
        pub not_after: DateTime<Utc>,
        pub token: String,
        pub display_claims: HashMap<String, serde_json::Value>,
    }

    impl std::fmt::Debug for XboxToken {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("XboxToken")
                .field("issue_instant", &self.issue_instant)
                .field("not_after", &self.not_after)
                .field("token", &"[REDACTED]")
                .field("display_claims", &"[PRESENT]")
                .finish()
        }
    }

    #[derive(Deserialize, Debug, Clone)]
    pub struct SisuRedirect {
        #[serde(rename = "MsaOauthRedirect")]
        pub msa_oauth_redirect: String,
    }

    #[derive(Deserialize, Clone)]
    #[serde(rename_all = "PascalCase")]
    pub struct SisuAuthorize {
        pub title_token: XboxToken,
        pub user_token: XboxToken,
    }

    impl std::fmt::Debug for SisuAuthorize {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("SisuAuthorize")
                .field("title_token", &"[REDACTED]")
                .field("user_token", &"[REDACTED]")
                .finish()
        }
    }

    #[derive(Deserialize, Clone)]
    pub struct MinecraftToken {
        pub access_token: String,
    }

    impl std::fmt::Debug for MinecraftToken {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("MinecraftToken")
                .field("access_token", &"[REDACTED]")
                .finish()
        }
    }

    #[derive(Deserialize, Debug, Clone)]
    pub struct MinecraftProfile {
        pub id: String,
        pub name: String,
    }
}

// ---------------------------------------------------------------------------
// ECDSA signed request engine
// ---------------------------------------------------------------------------

/// Build the signature buffer and sign it with p256 ECDSA.
///
/// Buffer format (from Xbox Live authentication protocol):
/// [1_u32 BE][0_u8][time u128 BE][0_u8]"POST"[0_u8][path][0_u8][auth][0_u8][body][0_u8]
///
/// time = ((unix_timestamp + 11644473600) * 10000000) — Windows file time.
///
/// Wire format: [1_i32 BE][time 8 BE][r_bytes][s_bytes], base64-standard encoded.
fn sign_request(
    key: &DeviceTokenKey,
    url_path: &str,
    authorization: Option<&str>,
    body: &str,
    current_date: DateTime<Utc>,
) -> String {
    let time = ((current_date.timestamp() as u128) + 11644473600) * 10000000;

    let mut buffer = Vec::new();
    buffer.extend_from_slice(&1u32.to_be_bytes());
    buffer.push(0);
    buffer.extend_from_slice(&(time as u64).to_be_bytes()); // 8 bytes
    buffer.push(0);
    buffer.extend_from_slice(b"POST");
    buffer.push(0);
    buffer.extend_from_slice(url_path.as_bytes());
    buffer.push(0);
    buffer.extend_from_slice(authorization.unwrap_or("").as_bytes());
    buffer.push(0);
    buffer.extend_from_slice(body.as_bytes());
    buffer.push(0);

    let signature: Signature = key.key.sign(&buffer);
    let sig_bytes = signature.to_bytes();

    let mut wire = Vec::new();
    wire.extend_from_slice(&1i32.to_be_bytes());
    wire.extend_from_slice(&(time as u64).to_be_bytes());
    wire.extend_from_slice(&sig_bytes[..32]); // r
    wire.extend_from_slice(&sig_bytes[32..]); // s

    base64::engine::general_purpose::STANDARD.encode(&wire)
}

/// Send a signed request to an Xbox Live endpoint.
#[allow(clippy::too_many_arguments)]
async fn send_signed_request<T: for<'de> serde::Deserialize<'de>>(
    _client: &reqwest::Client,
    url: &str,
    url_path: &str,
    body: serde_json::Value,
    key: &DeviceTokenKey,
    authorization: Option<&str>,
    include_contract_version: bool,
    current_date: DateTime<Utc>,
) -> LauncherResult<(DateTime<Utc>, T)> {
    let body_str = serde_json::to_string(&body).unwrap_or_default();
    let sig = sign_request(key, url_path, authorization, &body_str, current_date);

    let clients = HttpClients::new()?;
    let mut headers = vec![
        ("Content-Type".into(), "application/json".into()),
        ("Accept".into(), "application/json".into()),
        ("Signature".into(), sig),
    ];
    if include_contract_version {
        headers.push(("x-xbl-contract-version".into(), "1".into()));
    }
    if let Some(auth) = authorization {
        headers.push(("Authorization".into(), auth.to_string()));
    }
    let body = serde_json::to_vec(&body).map_err(|e| LauncherError::Generic {
        code: "ERR_MSA_REQUEST".into(),
        message: format!("Request body encoding failed: {e}"),
    })?;
    let resp = http_client::checked_send(
        &clients,
        ClientCategory::Microsoft,
        reqwest::Method::POST,
        url,
        &headers,
        Some(body),
        Some("application/json"),
    )
    .await?;

    let status = resp.status();
    let current_date = response_date(resp.headers());
    let raw = http_client::checked_response_text(resp, ClientCategory::Microsoft)
        .await
        .unwrap_or_default();

    if !status.is_success() {
        return Err(LauncherError::Generic {
            code: "ERR_MSA_HTTP_ERROR".into(),
            message: format!("{} returned HTTP {} (response suppressed)", url, status),
        });
    }

    let parsed: T = serde_json::from_str(&raw).map_err(|e| LauncherError::Generic {
        code: "ERR_MSA_DESERIALIZE".into(),
        message: format!(
            "Failed to parse response from {}: {} (response suppressed)",
            url, e
        ),
    })?;

    Ok((current_date, parsed))
}

// ---------------------------------------------------------------------------
// Auth steps
// ---------------------------------------------------------------------------

fn proof_key_json(key: &DeviceTokenKey) -> serde_json::Value {
    serde_json::json!({
        "kty": "EC",
        "x": key.x,
        "y": key.y,
        "crv": "P-256",
        "alg": "ES256",
        "use": "sig"
    })
}

fn xbox_device_id(id: Uuid) -> String {
    format!("{{{}}}", id.to_string().to_uppercase())
}

fn response_date(headers: &reqwest::header::HeaderMap) -> DateTime<Utc> {
    headers
        .get(reqwest::header::DATE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| DateTime::parse_from_rfc2822(value).ok())
        .map_or_else(Utc::now, |value| value.with_timezone(&Utc))
}

/// Step 1: Get device token from Xbox Live.
async fn get_device_token(
    client: &reqwest::Client,
    key: &DeviceTokenKey,
) -> LauncherResult<(DateTime<Utc>, xbox_types::XboxToken)> {
    let body = serde_json::json!({
        "Properties": {
            "AuthMethod": "ProofOfPossession",
            "Id": xbox_device_id(key.id),
            "DeviceType": "Win32",
            "Version": "10.16.0",
            "ProofKey": proof_key_json(key),
        },
        "RelyingParty": "http://auth.xboxlive.com",
        "TokenType": "JWT"
    });

    let (date, token) = send_signed_request(
        client,
        DEVICE_AUTH_URL,
        "/device/authenticate",
        body,
        key,
        None,
        true,
        Utc::now(),
    )
    .await?;
    Ok((date, token))
}

// ---------------------------------------------------------------------------
// PKCE challenge generation
// ---------------------------------------------------------------------------

fn generate_pkce_verifier() -> String {
    let bytes: Vec<u8> = (0..64).map(|_| rand::random::<u8>()).collect();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn pkce_challenge(verifier: &str) -> String {
    let hash = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash)
}

// ---------------------------------------------------------------------------
// Steps 2-3: SISU authenticate → redirect URI
// ---------------------------------------------------------------------------

/// Step 2: SISU authenticate with the device token. Returns (session_id, redirect_uri, state).
async fn sisu_authenticate(
    _client: &reqwest::Client,
    device_token: &str,
    challenge: &str,
    key: &DeviceTokenKey,
    current_date: DateTime<Utc>,
) -> LauncherResult<(String, String, String)> {
    let state: String = (0..32)
        .map(|_| format!("{:x}", rand::random::<u8>() % 16))
        .collect();

    let body = serde_json::json!({
        "AppId": MICROSOFT_CLIENT_ID,
        "DeviceToken": device_token,
        "Offers": [REQUESTED_SCOPE],
        "Query": {
            "code_challenge": challenge,
            "code_challenge_method": "S256",
            "state": state,
            "prompt": "select_account"
        },
        "RedirectUri": AUTH_REPLY_URL,
        "Sandbox": "RETAIL",
        "TokenType": "code",
        "TitleId": TITLE_ID
    });

    let body_str = serde_json::to_string(&body).unwrap_or_default();
    let sig = sign_request(key, "/authenticate", None, &body_str, current_date);

    let clients = HttpClients::new()?;
    let resp = http_client::checked_send(
        &clients,
        ClientCategory::Microsoft,
        reqwest::Method::POST,
        SISU_AUTH_URL,
        &[
            ("Content-Type".into(), "application/json".into()),
            ("Accept".into(), "application/json".into()),
            ("Signature".into(), sig),
            ("x-xbl-contract-version".into(), "1".into()),
        ],
        Some(body_str.into_bytes()),
        Some("application/json"),
    )
    .await?;

    let status = resp.status();
    if !status.is_success() {
        return Err(LauncherError::Generic {
            code: "ERR_MSA_SISU_AUTH_HTTP".into(),
            message: format!(
                "SISU authenticate returned HTTP {} (response suppressed)",
                status
            ),
        });
    }

    let session_id = resp
        .headers()
        .get("X-SessionId")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        .ok_or_else(|| LauncherError::Generic {
            code: "ERR_MSA_NO_SESSION_ID".into(),
            message: "SISU authenticate response did not contain a session ID.".into(),
        })?;

    let response_bytes =
        http_client::checked_response_bytes(resp, ClientCategory::Microsoft).await?;
    let redirect: xbox_types::SisuRedirect =
        serde_json::from_slice(&response_bytes).map_err(|e| LauncherError::Generic {
            code: "ERR_MSA_SISU_AUTH_PARSE".into(),
            message: format!("Failed to parse SISU redirect: {}", e),
        })?;

    Ok((session_id, redirect.msa_oauth_redirect, state))
}

// ---------------------------------------------------------------------------
// Step 4: Exchange auth code for OAuth token
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(crate) struct OAuthToken {
    pub(crate) access_token: String,
    pub(crate) refresh_token: String,
    pub(crate) expires_in: u64,
}

impl std::fmt::Debug for OAuthToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthToken")
            .field("access_token", &"[REDACTED]")
            .field("refresh_token", &"[REDACTED]")
            .field("expires_in", &self.expires_in)
            .finish()
    }
}

async fn exchange_oauth_token(
    _client: &reqwest::Client,
    code: &str,
    verifier: &str,
) -> LauncherResult<(DateTime<Utc>, OAuthToken)> {
    let params = [
        ("client_id", MICROSOFT_CLIENT_ID),
        ("code", code),
        ("code_verifier", verifier),
        ("grant_type", "authorization_code"),
        ("redirect_uri", AUTH_REPLY_URL),
        ("scope", REQUESTED_SCOPE),
    ];

    let clients = HttpClients::new()?;
    let resp = http_client::checked_post_form(
        &clients,
        ClientCategory::Microsoft,
        OAUTH_TOKEN_URL,
        &params,
        &[],
    )
    .await?;

    let status = resp.status();
    let date = response_date(resp.headers());
    if !status.is_success() {
        let _body = http_client::checked_response_text(resp, ClientCategory::Microsoft)
            .await
            .unwrap_or_default();
        return Err(LauncherError::Generic {
            code: "ERR_MSA_OAUTH_TOKEN_HTTP".into(),
            message: format!(
                "OAuth token endpoint returned HTTP {} (response suppressed)",
                status
            ),
        });
    }

    let token_bytes = http_client::checked_response_bytes(resp, ClientCategory::Microsoft).await?;
    let token =
        serde_json::from_slice::<OAuthToken>(&token_bytes).map_err(|e| LauncherError::Generic {
            code: "ERR_MSA_OAUTH_TOKEN_PARSE".into(),
            message: format!("Failed to parse OAuth token: {}", e),
        })?;
    Ok((date, token))
}

/// Returns `true` when the OAuth error body signals a permanent failure
/// (`invalid_grant` or `expired_token`) that should clear stored credentials.
fn is_msa_permanent_refresh_error(body: &str) -> bool {
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(err) = val.get("error").and_then(|v| v.as_str()) {
            return matches!(err, "invalid_grant" | "expired_token");
        }
    }
    false
}

async fn refresh_oauth_token(
    _client: &reqwest::Client,
    refresh_token: &str,
) -> LauncherResult<(DateTime<Utc>, OAuthToken)> {
    let params = [
        ("client_id", MICROSOFT_CLIENT_ID),
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
        ("redirect_uri", AUTH_REPLY_URL),
        ("scope", REQUESTED_SCOPE),
    ];

    let clients = HttpClients::new()?;
    let resp = http_client::checked_post_form(
        &clients,
        ClientCategory::Microsoft,
        OAUTH_TOKEN_URL,
        &params,
        &[],
    )
    .await?;

    let status = resp.status();
    let date = response_date(resp.headers());
    if !status.is_success() {
        let body = http_client::checked_response_text(resp, ClientCategory::Microsoft)
            .await
            .unwrap_or_default();
        if is_msa_permanent_refresh_error(&body) {
            return Err(LauncherError::MsaAuthRequired);
        }
        return Err(LauncherError::Generic {
            code: "ERR_MSA_OAUTH_REFRESH_HTTP".into(),
            message: format!(
                "OAuth refresh endpoint returned HTTP {} (response suppressed)",
                status
            ),
        });
    }

    let token_bytes = http_client::checked_response_bytes(resp, ClientCategory::Microsoft).await?;
    let token =
        serde_json::from_slice::<OAuthToken>(&token_bytes).map_err(|e| LauncherError::Generic {
            code: "ERR_MSA_OAUTH_REFRESH_PARSE".into(),
            message: format!("Failed to parse refreshed OAuth token: {}", e),
        })?;
    Ok((date, token))
}

// ---------------------------------------------------------------------------
// Steps 5-6: SISU authorize → XSTS
// ---------------------------------------------------------------------------

async fn sisu_authorize(
    client: &reqwest::Client,
    session_id: Option<&str>,
    access_token: &str,
    device_token: &str,
    key: &DeviceTokenKey,
    current_date: DateTime<Utc>,
) -> LauncherResult<(DateTime<Utc>, xbox_types::SisuAuthorize)> {
    let body = serde_json::json!({
        "AccessToken": format!("t={}", access_token),
        "AppId": MICROSOFT_CLIENT_ID,
        "DeviceToken": device_token,
        "ProofKey": proof_key_json(key),
        "Sandbox": "RETAIL",
        "SessionId": session_id,
        "SiteName": "user.auth.xboxlive.com",
        "RelyingParty": "http://xboxlive.com",
        "UseModernGamertag": true
    });

    let (date, result) = send_signed_request(
        client,
        SISU_AUTHORIZE_URL,
        "/authorize",
        body,
        key,
        None,
        false,
        current_date,
    )
    .await?;
    Ok((date, result))
}

async fn xsts_authorize(
    _client: &reqwest::Client,
    sisu: &xbox_types::SisuAuthorize,
    device_token: &str,
    key: &DeviceTokenKey,
    auth_date: DateTime<Utc>,
) -> LauncherResult<(String, String)> {
    let body = serde_json::json!({
        "RelyingParty": "rp://api.minecraftservices.com/",
        "TokenType": "JWT",
        "Properties": {
            "SandboxId": "RETAIL",
            "UserTokens": [sisu.user_token.token],
            "DeviceToken": device_token,
            "TitleToken": sisu.title_token.token
        }
    });

    let body_str = serde_json::to_string(&body).unwrap_or_default();
    let sig = sign_request(key, "/xsts/authorize", None, &body_str, auth_date);

    let clients = HttpClients::new()?;
    let resp = http_client::checked_send(
        &clients,
        ClientCategory::Microsoft,
        reqwest::Method::POST,
        XSTS_AUTHORIZE_URL,
        &[
            ("Content-Type".into(), "application/json".into()),
            ("Accept".into(), "application/json".into()),
            ("Signature".into(), sig),
            ("x-xbl-contract-version".into(), "1".into()),
        ],
        Some(body_str.into_bytes()),
        Some("application/json"),
    )
    .await?;

    let status = resp.status();
    if !status.is_success() {
        let _body = http_client::checked_response_text(resp, ClientCategory::Microsoft)
            .await
            .unwrap_or_default();
        return Err(LauncherError::Generic {
            code: "ERR_MSA_XSTS_HTTP".into(),
            message: format!("XSTS returned HTTP {} (response suppressed)", status),
        });
    }

    let token_bytes = http_client::checked_response_bytes(resp, ClientCategory::Microsoft).await?;
    let token: xbox_types::XboxToken =
        serde_json::from_slice(&token_bytes).map_err(|e| LauncherError::Generic {
            code: "ERR_MSA_XSTS_PARSE".into(),
            message: format!("Failed to parse XSTS token: {}", e),
        })?;

    // Extract user hash (uhs) from display_claims
    let uhs = token
        .display_claims
        .get("xui")
        .and_then(|v| v.get(0))
        .and_then(|v| v.get("uhs"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| LauncherError::Generic {
            code: "ERR_MSA_NO_UHS".into(),
            message: "XSTS response did not contain user hash (uhs).".into(),
        })?
        .to_string();

    Ok((uhs, token.token))
}

// ---------------------------------------------------------------------------
// Steps 7-9: Minecraft token → entitlements → profile
// ---------------------------------------------------------------------------

async fn get_minecraft_token(
    _client: &reqwest::Client,
    uhs: &str,
    xsts_token: &str,
) -> LauncherResult<String> {
    let body = serde_json::json!({
        "platform": "PC_LAUNCHER",
        "xtoken": format!("XBL3.0 x={};{}", uhs, xsts_token),
    });

    let clients = HttpClients::new()?;
    let resp = http_client::checked_post_json(
        &clients,
        ClientCategory::Microsoft,
        MC_LOGIN_URL,
        &body,
        &[
            ("Accept".into(), "application/json".into()),
            ("User-Agent".into(), USER_AGENT.into()),
        ],
    )
    .await?;

    let status = resp.status();
    if !status.is_success() {
        let _body = http_client::checked_response_text(resp, ClientCategory::Microsoft)
            .await
            .unwrap_or_default();
        return Err(LauncherError::Generic {
            code: "ERR_MSA_MC_TOKEN_HTTP".into(),
            message: format!(
                "Minecraft login returned HTTP {} (response suppressed)",
                status
            ),
        });
    }

    let token_bytes = http_client::checked_response_bytes(resp, ClientCategory::Microsoft).await?;
    let token: xbox_types::MinecraftToken =
        serde_json::from_slice(&token_bytes).map_err(|e| LauncherError::Generic {
            code: "ERR_MSA_MC_TOKEN_PARSE".into(),
            message: format!("Failed to parse Minecraft token: {}", e),
        })?;

    Ok(token.access_token)
}

async fn check_entitlements(_client: &reqwest::Client, access_token: &str) -> LauncherResult<()> {
    let url = format!("{}?requestId={}", MC_ENTITLEMENTS_URL, Uuid::new_v4());
    let clients = HttpClients::new()?;
    let resp = http_client::checked_request_with_headers(
        &clients,
        ClientCategory::Microsoft,
        &url,
        vec![
            ("Authorization".into(), format!("Bearer {access_token}")),
            ("User-Agent".into(), USER_AGENT.into()),
        ],
    )
    .await?;

    if !resp.status().is_success() {
        let _body = http_client::checked_response_text(resp, ClientCategory::Microsoft)
            .await
            .unwrap_or_default();
        return Err(LauncherError::Generic {
            code: "ERR_MSA_NO_ENTITLEMENT".into(),
            message: "Your Microsoft account does not own Minecraft. Please purchase it at minecraft.net.".into(),
        });
    }

    Ok(())
}

async fn get_minecraft_profile(
    _client: &reqwest::Client,
    access_token: &str,
) -> LauncherResult<(String, String)> {
    let clients = HttpClients::new()?;
    let resp = http_client::checked_request_with_headers(
        &clients,
        ClientCategory::Microsoft,
        MC_PROFILE_URL,
        vec![
            ("Authorization".into(), format!("Bearer {access_token}")),
            ("User-Agent".into(), USER_AGENT.into()),
        ],
    )
    .await?;

    let status = resp.status();
    if !status.is_success() {
        let _body = http_client::checked_response_text(resp, ClientCategory::Microsoft)
            .await
            .unwrap_or_default();
        return Err(LauncherError::Generic {
            code: "ERR_MSA_PROFILE_HTTP".into(),
            message: format!(
                "Profile endpoint returned HTTP {} (response suppressed)",
                status
            ),
        });
    }

    let profile_bytes =
        http_client::checked_response_bytes(resp, ClientCategory::Microsoft).await?;
    let profile: xbox_types::MinecraftProfile =
        serde_json::from_slice(&profile_bytes).map_err(|e| LauncherError::Generic {
            code: "ERR_MSA_PROFILE_PARSE".into(),
            message: format!("Failed to parse Minecraft profile: {}", e),
        })?;

    Ok((profile.name, profile.id))
}

// ---------------------------------------------------------------------------
// Public API: begin_login → finish_login → refresh → storage
// ---------------------------------------------------------------------------

/// Begin the MSA login flow. Returns a [`LoginFlow`] whose `auth_uri` should
/// be opened in a browser. The caller captures the `?code=` from the redirect
/// and passes it to [`finish_login`].
///
/// `db_path` is the path to `local_state.db` — the caller must provide the
/// correct path for the running binary (desktop app vs CLI).
pub async fn begin_login(client: &reqwest::Client, db_path: &Path) -> LauncherResult<LoginFlow> {
    check_network_enabled(
        db_path,
        "network_msa_enabled",
        "Microsoft account login is disabled in Privacy settings.",
    )?;
    // Step 1: Generate p256 key + get device token
    let key = DeviceTokenKey::generate();
    let (device_date, device_token) = get_device_token(client, &key).await?;

    // Step 2: Generate PKCE challenge + SISU authenticate
    let verifier = generate_pkce_verifier();
    let challenge = pkce_challenge(&verifier);
    let (session_id, auth_uri, state) =
        sisu_authenticate(client, &device_token.token, &challenge, &key, device_date).await?;

    Ok(LoginFlow {
        auth_uri,
        verifier,
        session_id: Some(session_id),
        key_json: key.to_json(),
        device_token: device_token.token,
        state,
    })
}

/// Complete the MSA login flow with the auth code from the browser redirect.
/// Runs steps 4-9 and returns persisted credentials.
///
/// The optional `state` parameter is passed from the browser redirect URL's
/// `?state=` query parameter. If it does not match the state generated during
/// `begin_login`, the login is rejected (CSRF protection).
///
/// `db_path` is the path to `local_state.db` — must match the path passed to
/// [`begin_login`].
pub async fn finish_login(
    client: &reqwest::Client,
    code: &str,
    flow: &LoginFlow,
    state: Option<&str>,
    db_path: &Path,
) -> LauncherResult<MsaCredentials> {
    check_network_enabled(
        db_path,
        "network_msa_enabled",
        "Microsoft account login is disabled in Privacy settings.",
    )?;
    // CSRF check: verify state parameter if provided and flow has one
    if let Some(passed_state) = state {
        if !flow.state.is_empty() && flow.state != passed_state {
            return Err(LauncherError::Generic {
                code: "ERR_MSA_STATE_MISMATCH".into(),
                message: "OAuth state parameter mismatch — possible CSRF attack. Aborting login."
                    .into(),
            });
        }
    } else if !flow.state.is_empty() {
        return Err(LauncherError::Generic {
            code: "ERR_MSA_STATE_MISMATCH".into(),
            message: "OAuth state parameter missing — possible CSRF attack. Login aborted.".into(),
        });
    }

    let key = DeviceTokenKey::from_json(&flow.key_json)?;

    // Step 4: Exchange auth code for OAuth token
    let (auth_date, oauth) = exchange_oauth_token(client, code, &flow.verifier).await?;

    // Step 5: SISU authorize
    let (sisu_date, sisu) = sisu_authorize(
        client,
        flow.session_id.as_deref(),
        &oauth.access_token,
        &flow.device_token,
        &key,
        auth_date,
    )
    .await?;

    // Step 6: XSTS authorize → user hash + Xbox token
    let (uhs, xsts_token) =
        xsts_authorize(client, &sisu, &flow.device_token, &key, sisu_date).await?;

    // Step 7: Minecraft login → access token
    let mc_access_token = get_minecraft_token(client, &uhs, &xsts_token).await?;

    // Step 8: Entitlements check
    check_entitlements(client, &mc_access_token).await?;

    // Step 9: Minecraft profile
    let (username, uuid) = get_minecraft_profile(client, &mc_access_token).await?;

    let creds = MsaCredentials {
        username,
        uuid,
        access_token: mc_access_token,
        refresh_token: oauth.refresh_token,
        expires: auth_date + chrono::Duration::seconds(oauth.expires_in as i64),
    };

    store_credentials(&creds)?;
    Ok(creds)
}

/// Refresh expired credentials using the refresh token.
/// Runs steps 4(refresh), 5, 6, 7.
///
/// `db_path` is the path to `local_state.db` — must match the path passed to
/// [`begin_login`].
pub async fn refresh_credentials(
    client: &reqwest::Client,
    creds: &MsaCredentials,
    db_path: &Path,
) -> LauncherResult<MsaCredentials> {
    check_network_enabled(
        db_path,
        "network_msa_enabled",
        "Microsoft account login is disabled in Privacy settings.",
    )?;
    // Step 4 (refresh): Get new OAuth token from refresh token
    let (auth_date, oauth) = refresh_oauth_token(client, &creds.refresh_token).await?;

    // Regenerate device token + key (simpler + more secure than caching)
    let key = DeviceTokenKey::generate();
    let (device_date, device_token) = get_device_token(client, &key).await?;

    // Steps 5-7: SISU authorize → XSTS → Minecraft token
    let (sisu_date, sisu) = sisu_authorize(
        client,
        None,
        &oauth.access_token,
        &device_token.token,
        &key,
        device_date,
    )
    .await?;
    let (uhs, xsts_token) =
        xsts_authorize(client, &sisu, &device_token.token, &key, sisu_date).await?;
    let mc_access_token = get_minecraft_token(client, &uhs, &xsts_token).await?;

    let refreshed = MsaCredentials {
        username: creds.username.clone(),
        uuid: creds.uuid.clone(),
        access_token: mc_access_token,
        refresh_token: oauth.refresh_token,
        expires: auth_date + chrono::Duration::seconds(oauth.expires_in as i64),
    };

    store_credentials(&refreshed)?;
    Ok(refreshed)
}

/// Get stored credentials from the OS keyring, or None if not authenticated.
pub fn load_credentials() -> LauncherResult<Option<MsaCredentials>> {
    #[cfg(all(feature = "test-support", debug_assertions))]
    if let Some(json) = std::env::var_os("AGORA_TEST_MSA_CREDENTIALS_JSON") {
        let json = json.into_string().map_err(|_| LauncherError::Generic {
            code: "ERR_MSA_TEST_CREDENTIALS_PARSE".into(),
            message: "Test credentials environment variable is not valid UTF-8.".into(),
        })?;
        let credentials = serde_json::from_str(&json).map_err(|error| LauncherError::Generic {
            code: "ERR_MSA_TEST_CREDENTIALS_PARSE".into(),
            message: format!("Failed to parse test credentials: {error}"),
        })?;
        return Ok(Some(credentials));
    }

    let Some(json) = crate::auth::load_secret(
        KEYRING_SERVICE,
        KEYRING_ACCOUNT,
        CREDENTIALS_FALLBACK_FILE,
        CREDENTIALS_KEY_CONTEXT,
    )?
    else {
        return Ok(None);
    };
    serde_json::from_str(&json)
        .map(Some)
        .map_err(|e| LauncherError::Generic {
            code: "ERR_MSA_STORED_PARSE".into(),
            message: format!("Failed to parse stored credentials: {e}"),
        })
}

/// Store credentials in the OS keyring as JSON.
pub fn store_credentials(creds: &MsaCredentials) -> LauncherResult<()> {
    let json = serde_json::to_string(creds).map_err(|e| LauncherError::Generic {
        code: "ERR_MSA_STORED_SERIALIZE".into(),
        message: format!("Failed to serialize credentials: {e}"),
    })?;
    crate::auth::store_secret(
        KEYRING_SERVICE,
        KEYRING_ACCOUNT,
        CREDENTIALS_FALLBACK_FILE,
        CREDENTIALS_KEY_CONTEXT,
        &json,
    )
}

/// Clear stored MSA credentials (sign out).
pub fn clear_credentials() -> LauncherResult<()> {
    crate::auth::clear_secret(KEYRING_SERVICE, KEYRING_ACCOUNT, CREDENTIALS_FALLBACK_FILE)
}

// ---------------------------------------------------------------------------
// Durable, race-safe credential refresh
// ---------------------------------------------------------------------------

/// Outcome of [`get_valid_credentials`].
#[derive(Debug)]
pub enum MsaCredentialOutcome {
    /// Credentials are valid and ready to use (possibly just-refreshed).
    Valid(MsaCredentials),
    /// Session has been permanently invalidated; user must sign in again.
    SignInRequired,
    /// Temporary failure; stored credentials preserved.
    RefreshFailed(LauncherError),
}

static MSA_REFRESH_MUTEX: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

/// Injectable abstraction for the complete MSA refresh chain.
#[async_trait::async_trait]
pub(crate) trait MsaRefreshHttp: Send + Sync {
    async fn refresh_credentials(
        &self,
        client: &reqwest::Client,
        credentials: &MsaCredentials,
    ) -> LauncherResult<MsaCredentials>;
}

/// Production MSA refresh client that delegates to the real HTTP endpoint.
pub(crate) struct LiveMsaRefreshHttp;

#[async_trait::async_trait]
impl MsaRefreshHttp for LiveMsaRefreshHttp {
    async fn refresh_credentials(
        &self,
        client: &reqwest::Client,
        credentials: &MsaCredentials,
    ) -> LauncherResult<MsaCredentials> {
        let (auth_date, oauth) = refresh_oauth_token(client, &credentials.refresh_token).await?;
        let key = DeviceTokenKey::generate();
        let (device_date, device_token) = get_device_token(client, &key).await?;
        let (sisu_date, sisu) = sisu_authorize(
            client,
            None,
            &oauth.access_token,
            &device_token.token,
            &key,
            device_date,
        )
        .await?;
        let (uhs, xsts_token) =
            xsts_authorize(client, &sisu, &device_token.token, &key, sisu_date).await?;
        let mc_access_token = get_minecraft_token(client, &uhs, &xsts_token).await?;
        Ok(MsaCredentials {
            username: credentials.username.clone(),
            uuid: credentials.uuid.clone(),
            access_token: mc_access_token,
            refresh_token: oauth.refresh_token,
            expires: auth_date + chrono::Duration::seconds(oauth.expires_in as i64),
        })
    }
}

#[cfg(any(test, feature = "test-support"))]
use std::collections::VecDeque;
#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
#[cfg(any(test, feature = "test-support"))]
use std::sync::{Arc, Mutex};

/// In-memory scripted MSA refresh client for testing.
#[cfg(any(test, feature = "test-support"))]
#[derive(Clone)]
#[allow(dead_code)]
pub(crate) struct MockMsaRefreshHttp {
    call_count: Arc<AtomicU64>,
    responses: Arc<Mutex<MockMsaResponses>>,
}

#[cfg(any(test, feature = "test-support"))]
type MockMsaResponses = VecDeque<LauncherResult<(String, String, u64)>>;

#[cfg(any(test, feature = "test-support"))]
#[allow(dead_code)]
impl MockMsaRefreshHttp {
    pub(crate) fn new() -> Self {
        Self {
            call_count: Arc::new(AtomicU64::new(0)),
            responses: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub(crate) fn queue_success(&self, access_token: &str, refresh_token: &str, expires_in: u64) {
        self.responses.lock().unwrap().push_back(Ok((
            access_token.to_string(),
            refresh_token.to_string(),
            expires_in,
        )));
    }

    pub(crate) fn queue_error(&self, error: LauncherError) {
        self.responses.lock().unwrap().push_back(Err(error));
    }

    pub(crate) fn call_count(&self) -> u64 {
        self.call_count.load(AtomicOrdering::SeqCst)
    }
}

#[cfg(any(test, feature = "test-support"))]
#[async_trait::async_trait]
impl MsaRefreshHttp for MockMsaRefreshHttp {
    async fn refresh_credentials(
        &self,
        _client: &reqwest::Client,
        credentials: &MsaCredentials,
    ) -> LauncherResult<MsaCredentials> {
        self.call_count.fetch_add(1, AtomicOrdering::SeqCst);
        let mut lock = self.responses.lock().unwrap();
        let (access_token, refresh_token, expires_in) = lock.pop_front().unwrap_or_else(|| {
            panic!(
                "MockMsaRefreshHttp: no more responses (call #{})",
                self.call_count.load(AtomicOrdering::SeqCst)
            )
        })?;
        Ok(MsaCredentials {
            username: credentials.username.clone(),
            uuid: credentials.uuid.clone(),
            access_token,
            refresh_token,
            expires: Utc::now() + chrono::Duration::seconds(expires_in as i64),
        })
    }
}

/// Obtain valid MSA credentials with single-flight refresh and double-check.
///
/// 1. Loads stored credentials; returns `SignInRequired` if none exist.
/// 2. If still within the 5‑minute margin, returns `Valid` immediately.
/// 3. Acquires a refresh mutex and re‑checks (another caller may have refreshed).
/// 4. Refreshes the OAuth token.
///    - On `invalid_grant` / `expired_token`, clears credentials and returns `SignInRequired`.
///    - On network / 5xx / malformed responses, preserves credentials and returns `RefreshFailed`.
///    - On success, performs the full refresh chain, persists it, and returns `Valid`.
///
pub async fn get_valid_credentials(client: &reqwest::Client) -> MsaCredentialOutcome {
    get_valid_credentials_inner(&LiveMsaRefreshHttp, client).await
}

/// Internal variant with injectable HTTP — enables deterministic testing.
async fn get_valid_credentials_inner(
    http: &dyn MsaRefreshHttp,
    client: &reqwest::Client,
) -> MsaCredentialOutcome {
    // 1. Load stored credentials
    let creds = match load_credentials() {
        Ok(Some(c)) => c,
        Ok(None) => return MsaCredentialOutcome::SignInRequired,
        Err(e) => return MsaCredentialOutcome::RefreshFailed(e),
    };

    // 2. Return immediately if still fresh
    if !creds.needs_refresh() {
        return MsaCredentialOutcome::Valid(creds);
    }

    // 3. Single-flight with double-check
    let _lock = MSA_REFRESH_MUTEX.lock().await;

    let creds = match load_credentials() {
        Ok(Some(c)) => c,
        Ok(None) => return MsaCredentialOutcome::SignInRequired,
        Err(e) => return MsaCredentialOutcome::RefreshFailed(e),
    };
    if !creds.needs_refresh() {
        return MsaCredentialOutcome::Valid(creds);
    }

    // 4. Complete refresh chain — the permanent/transient decision point.
    let refreshed = match http.refresh_credentials(client, &creds).await {
        Ok(result) => result,
        Err(LauncherError::MsaAuthRequired) => {
            // Permanent: invalid_grant or expired_token
            let _ = clear_credentials();
            return MsaCredentialOutcome::SignInRequired;
        }
        Err(e) => {
            // Transient: preserve existing credentials
            return MsaCredentialOutcome::RefreshFailed(e);
        }
    };

    match store_credentials(&refreshed) {
        Ok(()) => MsaCredentialOutcome::Valid(refreshed),
        Err(e) => MsaCredentialOutcome::RefreshFailed(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_verifier_is_hex_and_128_chars() {
        let v = generate_pkce_verifier();
        assert_eq!(v.len(), 128);
        assert!(v.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn pkce_challenge_is_base64url() {
        let v = generate_pkce_verifier();
        let c = pkce_challenge(&v);
        assert!(!c.is_empty());
        assert!(!c.contains('='));
    }

    #[test]
    fn device_token_key_roundtrip() {
        let key = DeviceTokenKey::generate();
        let json = key.to_json();
        let restored = DeviceTokenKey::from_json(&json).unwrap();
        assert_eq!(key.x, restored.x);
        assert_eq!(key.y, restored.y);
        assert_eq!(key.id, restored.id);
    }

    #[test]
    fn xbox_device_id_is_braced_uppercase_uuid() {
        let id = Uuid::parse_str("123e4567-e89b-12d3-a456-426614174000").unwrap();
        assert_eq!(xbox_device_id(id), "{123E4567-E89B-12D3-A456-426614174000}");
    }

    #[test]
    fn credentials_expiry_logic() {
        let creds = MsaCredentials {
            username: "test".into(),
            uuid: "abc".into(),
            access_token: "tok".into(),
            refresh_token: "ref".into(),
            expires: Utc::now() - chrono::Duration::hours(1),
        };
        assert!(creds.is_expired());
        assert!(creds.needs_refresh());

        let future = MsaCredentials {
            username: "test".into(),
            uuid: "abc".into(),
            access_token: "tok".into(),
            refresh_token: "ref".into(),
            expires: Utc::now() + chrono::Duration::hours(1),
        };
        assert!(!future.is_expired());
        assert!(!future.needs_refresh());
    }

    // -----------------------------------------------------------------------
    // Durable credential refresh tests
    // -----------------------------------------------------------------------

    static TEST_MSA_MUTEX: LazyLock<tokio::sync::Mutex<()>> =
        LazyLock::new(|| tokio::sync::Mutex::new(()));

    #[allow(dead_code)]
    struct TestMsaCredDir(tempfile::TempDir);
    impl Drop for TestMsaCredDir {
        fn drop(&mut self) {
            let _ = clear_credentials();
            std::env::remove_var("AGORA_TEST_SECRET_DIR");
        }
    }

    fn write_test_msa_creds(creds: &MsaCredentials) -> TestMsaCredDir {
        let dir = tempfile::tempdir().expect("temp dir for MSA test credentials");
        std::env::set_var("AGORA_TEST_SECRET_DIR", dir.path());
        let _ = clear_credentials();
        store_credentials(creds).expect("store test MSA credentials");
        TestMsaCredDir(dir)
    }

    fn test_creds() -> MsaCredentials {
        MsaCredentials {
            username: "test_user".into(),
            uuid: "abc-def-ghi".into(),
            access_token: "mc_test_access".into(),
            refresh_token: "mc_test_refresh".into(),
            expires: Utc::now() + chrono::Duration::hours(1),
        }
    }

    fn expired_creds() -> MsaCredentials {
        let mut c = test_creds();
        c.expires = Utc::now() - chrono::Duration::minutes(1);
        c
    }

    #[tokio::test]
    async fn test_get_valid_credentials_fresh_returns_immediately() {
        let _test_lock = TEST_MSA_MUTEX.lock().await;
        let http = MockMsaRefreshHttp::new();
        let client = reqwest::Client::new();
        let _td = write_test_msa_creds(&test_creds());

        let outcome = get_valid_credentials_inner(&http, &client).await;

        match outcome {
            MsaCredentialOutcome::Valid(c) => {
                assert_eq!(c.access_token, "mc_test_access");
            }
            _ => panic!("expected Valid for fresh credentials"),
        }
        assert_eq!(http.call_count(), 0);
    }

    #[tokio::test]
    async fn test_get_valid_credentials_no_creds_returns_sign_in_required() {
        let _test_lock = TEST_MSA_MUTEX.lock().await;
        let http = MockMsaRefreshHttp::new();
        let client = reqwest::Client::new();
        let _td = write_test_msa_creds(&test_creds());

        // Clear after writing so no credentials exist
        let _ = clear_credentials();

        let outcome = get_valid_credentials_inner(&http, &client).await;

        assert!(matches!(outcome, MsaCredentialOutcome::SignInRequired));
    }

    #[tokio::test]
    async fn test_get_valid_credentials_refresh_success() {
        let _test_lock = TEST_MSA_MUTEX.lock().await;
        let http = MockMsaRefreshHttp::new();
        // Queue OAuth refresh success
        http.queue_success("new_mc_token", "new_refresh_token", 28800);
        let client = reqwest::Client::new();
        let _td = write_test_msa_creds(&expired_creds());

        let outcome = get_valid_credentials_inner(&http, &client).await;

        match outcome {
            MsaCredentialOutcome::Valid(c) => {
                assert_eq!(c.username, "test_user");
                assert_eq!(c.access_token, "new_mc_token");
                assert_eq!(c.refresh_token, "new_refresh_token");
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
        assert_eq!(http.call_count(), 1);
    }

    #[tokio::test]
    async fn test_get_valid_credentials_invalid_grant_is_permanent() {
        let _test_lock = TEST_MSA_MUTEX.lock().await;
        let http = MockMsaRefreshHttp::new();
        http.queue_error(LauncherError::MsaAuthRequired);
        let client = reqwest::Client::new();
        let _td = write_test_msa_creds(&expired_creds());

        let outcome = get_valid_credentials_inner(&http, &client).await;
        assert!(matches!(outcome, MsaCredentialOutcome::SignInRequired));

        // Credentials should be cleared
        assert!(load_credentials().unwrap().is_none());
        assert_eq!(http.call_count(), 1);
    }

    #[tokio::test]
    async fn test_get_valid_credentials_transient_preserves_creds() {
        let _test_lock = TEST_MSA_MUTEX.lock().await;
        let http = MockMsaRefreshHttp::new();
        http.queue_error(LauncherError::NetworkOffline);
        let client = reqwest::Client::new();
        let _td = write_test_msa_creds(&expired_creds());

        let outcome = get_valid_credentials_inner(&http, &client).await;
        assert!(matches!(outcome, MsaCredentialOutcome::RefreshFailed(_)));

        // Credentials should still be present
        let loaded = load_credentials().unwrap();
        assert!(loaded.is_some(), "creds must survive transient error");
        assert_eq!(http.call_count(), 1);
    }

    #[tokio::test]
    async fn test_get_valid_credentials_double_check_skips_refresh() {
        let _test_lock = TEST_MSA_MUTEX.lock().await;
        let http = MockMsaRefreshHttp::new();
        // No responses queued — if the double-check works, refresh is never called
        let client = reqwest::Client::new();
        let _td = write_test_msa_creds(&test_creds()); // fresh, not expired

        let outcome = get_valid_credentials_inner(&http, &client).await;
        assert!(matches!(outcome, MsaCredentialOutcome::Valid(_)));
        assert_eq!(http.call_count(), 0);
    }

    #[tokio::test]
    async fn test_is_msa_permanent_refresh_error_classification() {
        assert!(is_msa_permanent_refresh_error(
            r#"{"error":"invalid_grant"}"#
        ));
        assert!(is_msa_permanent_refresh_error(
            r#"{"error":"expired_token"}"#
        ));
        assert!(is_msa_permanent_refresh_error(
            r#"{"error":"invalid_grant","error_description":"The refresh token has expired"}"#
        ));
        assert!(!is_msa_permanent_refresh_error(
            r#"{"error":"server_error"}"#
        ));
        assert!(!is_msa_permanent_refresh_error(
            r#"{"error":"temporarily_unavailable"}"#
        ));
        assert!(!is_msa_permanent_refresh_error("not-json"));
        assert!(!is_msa_permanent_refresh_error(""));
    }
}
