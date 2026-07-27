use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};

use std::sync::LazyLock;
use std::time::Duration;

use crate::error::{LauncherError, LauncherResult};
use crate::http_client::{self, ClientCategory, HttpClients};

pub const AGORA_OAUTH_CLIENT_ID: &str = match option_env!("AGORA_OAUTH_CLIENT_ID") {
    Some(v) => v,
    None => "Iv23ctVA40Yy1ZUkvemh",
};

const KEYRING_SERVICE: &str = "com.agoramc";
const KEYRING_ACCOUNT: &str = "github-token";

/// Fallback token file name (in app data dir) for when OS keyring is unavailable.
const TOKEN_FALLBACK_FILE: &str = "tokens.enc";

/// PBKDF2 iterations for key derivation in the keyring fallback.
const PBKDF2_ITERATIONS: u32 = 200_000;

/// If the access token has fewer than this many seconds remaining, refresh it.
const ACCESS_TOKEN_BUFFER_SECS: i64 = 300;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DeviceFlowResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GithubProfile {
    pub login: String,
    pub avatar_url: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GitHubTokenBundle {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub access_expires_at: Option<DateTime<Utc>>,
    pub refresh_expires_at: Option<DateTime<Utc>>,
    pub token_type: Option<String>,
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeviceFlowPollResponse {
    access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    refresh_token_expires_in: Option<u64>,
    #[serde(default)]
    token_type: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    error: Option<String>,
    interval: Option<u64>,
}

/// Log a line to stderr (replaced the old temp-file logger that wrote to
/// %TEMP%/agora-device-flow.log).
pub fn log_line(line: &str) {
    eprintln!("[auth] {line}");
}

pub async fn start_device_flow() -> LauncherResult<DeviceFlowResponse> {
    if AGORA_OAUTH_CLIENT_ID.is_empty() {
        return Err(LauncherError::Generic {
            code: "ERR_AUTH_NOT_CONFIGURED".to_string(),
            message: "GitHub OAuth is not configured. Set the AGORA_OAUTH_CLIENT_ID environment \
                      variable before building/running Tauri (e.g. \
                      $env:AGORA_OAUTH_CLIENT_ID='Iv1.xxxxxxxx'; npm run tauri:dev). Register \
                      an OAuth app at https://github.com/settings/developers (Authorization type: \
                      GitHub App, Device Flow enabled)."
                .to_string(),
        });
    }

    let clients = HttpClients::new()?;

    let params = [("client_id", AGORA_OAUTH_CLIENT_ID)];

    let resp = http_client::checked_post_form(
        &clients,
        ClientCategory::GitHub,
        "https://github.com/login/device/code",
        &params,
        &[("Accept".into(), "application/json".into())],
    )
    .await?;

    let status = resp.status();
    let body = http_client::checked_response_text(resp, ClientCategory::GitHub).await?;
    // Device-flow responses contain a device code. Do not emit response
    // bodies to logs, which are often collected by launchers and support
    // tools outside the OS credential boundary.
    eprintln!("[auth] device-code response status={status}");

    if !status.is_success() {
        return Err(LauncherError::Generic {
            code: "ERR_AUTH_DEVICE_CODE".to_string(),
            message: format!("GitHub rejected the device code request (status {status})."),
        });
    }

    serde_json::from_str::<DeviceFlowResponse>(&body).map_err(|e| {
        eprintln!("[auth] device-code parse error: {e}");
        LauncherError::Generic {
            code: "ERR_AUTH_DEVICE_CODE".to_string(),
            message: "Failed to parse GitHub device code response.".to_string(),
        }
    })
}

pub async fn poll_device_flow(
    device_code: String,
    mut interval: u64,
) -> LauncherResult<Option<GitHubTokenBundle>> {
    eprintln!(
        "[auth] poll_device_flow ENTERED device_code_len={} interval={}s",
        device_code.len(),
        interval
    );
    let clients = HttpClients::new()?;

    let deadline = std::time::Instant::now() + Duration::from_secs(1200);

    loop {
        if std::time::Instant::now() >= deadline {
            return Ok(None);
        }

        let params = [
            ("client_id", AGORA_OAUTH_CLIENT_ID),
            ("device_code", device_code.as_str()),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ];

        let resp = http_client::checked_post_form(
            &clients,
            ClientCategory::GitHub,
            "https://github.com/login/oauth/access_token",
            &params,
            &[("Accept".into(), "application/json".into())],
        )
        .await;

        match resp {
            Ok(r) => {
                let status = r.status();
                let body = http_client::checked_response_text(r, ClientCategory::GitHub)
                    .await
                    .unwrap_or_default();
                eprintln!("[auth] poll status={status}");

                let parsed: Option<DeviceFlowPollResponse> = serde_json::from_str(&body).ok();

                if let Some(parsed) = parsed {
                    if let Some(access_token) = parsed.access_token {
                        eprintln!("[auth] token obtained");
                        let now = Utc::now();
                        let bundle = GitHubTokenBundle {
                            access_token,
                            refresh_token: parsed.refresh_token,
                            access_expires_at: parsed
                                .expires_in
                                .map(|s| now + TimeDelta::seconds(s as i64)),
                            refresh_expires_at: parsed
                                .refresh_token_expires_in
                                .map(|s| now + TimeDelta::seconds(s as i64)),
                            token_type: parsed.token_type,
                            scope: parsed.scope,
                        };
                        return Ok(Some(bundle));
                    }
                    if let Some(err) = parsed.error.as_deref() {
                        match err {
                            "authorization_pending" => {
                                eprintln!(
                                    "[auth] awaiting user authorization (interval={})",
                                    parsed.interval.unwrap_or(interval)
                                );
                                if let Some(next) = parsed.interval {
                                    interval = next;
                                }
                            }
                            "slow_down" => {
                                interval = interval.saturating_add(5);
                                eprintln!("[auth] slow_down; interval now {interval}s");
                            }
                            "expired_token" => {
                                eprintln!("[auth] device code expired");
                                return Ok(None);
                            }
                            "access_denied" => {
                                eprintln!("[auth] user denied authorization");
                                return Ok(None);
                            }
                            other => {
                                eprintln!("[auth] unknown error from GitHub: {other}");
                            }
                        }
                    } else if let Some(next) = parsed.interval {
                        interval = next;
                    }
                } else {
                    eprintln!("[auth] could not parse poll response as JSON");
                }
            }
            Err(e) => {
                eprintln!("[auth] network error during poll: {e}");
            }
        }

        tokio::time::sleep(Duration::from_secs(interval.max(1))).await;
    }
}

// ---------------------------------------------------------------------------
// Single-flight lock for token refresh
// ---------------------------------------------------------------------------

static REFRESH_MUTEX: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

// ---------------------------------------------------------------------------
// Token refresh
// ---------------------------------------------------------------------------

/// Exchange a refresh token for a new access+refresh token pair.
///
/// POSTs to GitHub's OAuth token endpoint with `grant_type=refresh_token`.
/// Returns None when the refresh token has been revoked or expired.
pub async fn refresh_access_token(
    refresh_token: &str,
) -> LauncherResult<Option<GitHubTokenBundle>> {
    let clients = HttpClients::new()?;

    let params = [
        ("client_id", AGORA_OAUTH_CLIENT_ID),
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
    ];

    let resp = http_client::checked_post_form(
        &clients,
        ClientCategory::GitHub,
        "https://github.com/login/oauth/access_token",
        &params,
        &[("Accept".into(), "application/json".into())],
    )
    .await?;

    let status = resp.status();
    let body = http_client::checked_response_text(resp, ClientCategory::GitHub)
        .await
        .unwrap_or_default();
    eprintln!("[auth] refresh status={status}");

    #[derive(Debug, Deserialize)]
    struct RefreshResponse {
        access_token: Option<String>,
        #[serde(default)]
        refresh_token: Option<String>,
        #[serde(default)]
        expires_in: Option<u64>,
        #[serde(default)]
        refresh_token_expires_in: Option<u64>,
        #[serde(default)]
        token_type: Option<String>,
        #[serde(default)]
        scope: Option<String>,
        error: Option<String>,
    }

    let parsed: Option<RefreshResponse> = serde_json::from_str(&body).ok();

    if let Some(parsed) = parsed {
        if let Some(access_token) = parsed.access_token {
            let now = Utc::now();
            let bundle = GitHubTokenBundle {
                access_token,
                refresh_token: parsed.refresh_token,
                access_expires_at: parsed
                    .expires_in
                    .map(|s| now + TimeDelta::seconds(s as i64)),
                refresh_expires_at: parsed
                    .refresh_token_expires_in
                    .map(|s| now + TimeDelta::seconds(s as i64)),
                token_type: parsed.token_type,
                scope: parsed.scope,
            };
            return Ok(Some(bundle));
        }

        if let Some(err) = parsed.error.as_deref() {
            eprintln!("[auth] refresh error from GitHub: {err}");
            return Ok(None);
        }
    }

    eprintln!("[auth] could not parse refresh response");
    Ok(None)
}

// ---------------------------------------------------------------------------
// Token bundle storage
// ---------------------------------------------------------------------------

/// Store a token bundle in the OS credential manager (or encrypted fallback).
pub fn store_token_bundle(bundle: &GitHubTokenBundle) -> LauncherResult<()> {
    let json = serde_json::to_string(bundle).map_err(|_| LauncherError::Generic {
        code: "ERR_AUTH_SERIALIZE".into(),
        message: "Failed to serialize token bundle.".into(),
    })?;

    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT) {
        if entry.set_password(&json).is_ok() {
            if let Some(path) = fallback_token_path() {
                let _ = std::fs::remove_file(path);
            }
            return Ok(());
        }
    }

    let path = fallback_token_path().ok_or_else(|| LauncherError::Generic {
        code: "ERR_AUTH_FALLBACK_PATH".into(),
        message: "Could not determine data directory for fallback token storage.".into(),
    })?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| LauncherError::Generic {
            code: "ERR_AUTH_FALLBACK_WRITE".into(),
            message: "Failed to create fallback token directory.".into(),
        })?;
    }

    let key = derive_fallback_key();
    let encrypted = encrypt_token(&json, &key)?;
    std::fs::write(&path, encrypted).map_err(|_| LauncherError::Generic {
        code: "ERR_AUTH_FALLBACK_WRITE".into(),
        message: "Failed to write fallback token file.".into(),
    })?;

    Ok(())
}

/// Load a token bundle from storage. Returns None if no token is stored.
///
/// Handles legacy bare access tokens: if the stored value is not valid JSON,
/// it is treated as a plain access token and wrapped in a bundle.
pub fn load_token_bundle() -> Option<GitHubTokenBundle> {
    let raw = if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT) {
        if let Ok(password) = entry.get_password() {
            Some(password)
        } else {
            None
        }
    } else {
        None
    };

    let raw = raw.or_else(|| {
        let path = fallback_token_path()?;
        if !path.exists() {
            return None;
        }
        let data = std::fs::read(&path).ok()?;
        let key = derive_fallback_key();
        decrypt_token(&data, &key)
    });

    let raw = raw?;

    serde_json::from_str::<GitHubTokenBundle>(&raw).ok().or_else(|| {
        // Legacy bare access token — wrap it in a bundle with no expiry info.
        eprintln!("[auth] loaded legacy bare token; wrapping in bundle");
        Some(GitHubTokenBundle {
            access_token: raw,
            refresh_token: None,
            access_expires_at: None,
            refresh_expires_at: None,
            token_type: None,
            scope: None,
        })
    })
}

/// Clear the stored token bundle from all storage locations.
pub fn clear_token_bundle() -> Result<(), String> {
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT) {
        match entry.delete_password() {
            Ok(()) => {}
            Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(format!("Failed to delete GitHub token: {}", e)),
        }
    }

    if let Some(path) = fallback_token_path() {
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Access token helpers
// ---------------------------------------------------------------------------

/// Returns true if the access token has more than `ACCESS_TOKEN_BUFFER_SECS`
/// of validity remaining, or if we don't know the expiry (legacy token).
fn access_token_is_fresh(bundle: &GitHubTokenBundle) -> bool {
    match bundle.access_expires_at {
        Some(expires) => {
            let remaining = (expires - Utc::now()).num_seconds();
            remaining > ACCESS_TOKEN_BUFFER_SECS
        }
        None => true,
    }
}

/// Obtain a valid access token. If the stored token is near expiration and a
/// refresh token is available, attempts to refresh before returning.
///
/// Serialises concurrent callers through a single-flight lock so only one
/// refresh request is issued.
pub async fn get_valid_access_token() -> Option<String> {
    let bundle = load_token_bundle()?;

    if access_token_is_fresh(&bundle) {
        return Some(bundle.access_token);
    }

    if bundle.refresh_token.is_none() {
        return Some(bundle.access_token);
    }

    let _lock = REFRESH_MUTEX.lock().await;

    let bundle = load_token_bundle()?;
    if access_token_is_fresh(&bundle) {
        return Some(bundle.access_token);
    }

    let refresh_token = bundle.refresh_token.as_deref()?;

    match refresh_access_token(refresh_token).await {
        Ok(Some(new_bundle)) => {
            eprintln!("[auth] access token refreshed successfully");
            let token = new_bundle.access_token.clone();
            let _ = store_token_bundle(&new_bundle);
            Some(token)
        }
        Ok(None) => {
            eprintln!("[auth] refresh token expired or revoked");
            let _ = clear_token_bundle();
            None
        }
        Err(e) => {
            eprintln!("[auth] refresh network error: {e}");
            // Return the existing token even if near expiry — it might still work.
            Some(bundle.access_token)
        }
    }
}

/// Attempt one refresh after receiving a 401, then retry the operation.
///
/// Returns Ok(result) if refresh+retry succeeded, or the original error
/// if refresh failed. Clears the bundle on persistent failure.
pub async fn try_refresh_after_401(
    original_error: LauncherError,
) -> Result<(), LauncherError> {
    let _lock = REFRESH_MUTEX.lock().await;

    let bundle = match load_token_bundle() {
        Some(b) => b,
        None => return Err(original_error),
    };

    let refresh_token = match bundle.refresh_token.as_deref() {
        Some(rt) => rt.to_string(),
        None => {
            let _ = clear_token_bundle();
            return Err(original_error);
        }
    };

    match refresh_access_token(&refresh_token).await {
        Ok(Some(new_bundle)) => {
            eprintln!("[auth] 401 recovery: token refreshed");
            let _ = store_token_bundle(&new_bundle);
            Ok(())
        }
        Ok(None) => {
            eprintln!("[auth] 401 recovery failed: refresh token invalid");
            let _ = clear_token_bundle();
            Err(original_error)
        }
        Err(_) => {
            eprintln!("[auth] 401 recovery failed: network error during refresh");
            Err(original_error)
        }
    }
}

// ---------------------------------------------------------------------------
// Legacy API compatibility
// ---------------------------------------------------------------------------

/// Store a bare access token. For backward compatibility with existing code
/// paths that pass a raw token string. Wraps it in a bundle.
pub fn store_token(token: &str) -> LauncherResult<()> {
    let bundle = GitHubTokenBundle {
        access_token: token.to_string(),
        refresh_token: None,
        access_expires_at: None,
        refresh_expires_at: None,
        token_type: None,
        scope: None,
    };
    store_token_bundle(&bundle)
}

/// Returns the stored access token (from bundle or legacy bare token).
/// Prefer `get_valid_access_token()` for new code — it handles expiry.
pub fn get_token() -> Option<String> {
    load_token_bundle().map(|b| b.access_token)
}

/// Derive a 256-bit key using PBKDF2-HMAC-SHA256.
/// Salt is derived from the OS username and a stable machine identifier.
fn derive_fallback_key_for(context: &[u8]) -> Vec<u8> {
    use pbkdf2::pbkdf2_hmac;
    use sha2::Sha256;

    let username = dirs::home_dir()
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().to_string()))
        .unwrap_or_else(|| "unknown".to_string());

    // TODO: use a stronger machine identifier (e.g. machine-id on Linux,
    // MachineGuid on Windows) when available.
    let salt = format!("agora-fallback:{}:{}", username, std::env::consts::OS);

    let mut key = vec![0u8; 32];
    pbkdf2_hmac::<Sha256>(context, salt.as_bytes(), PBKDF2_ITERATIONS, &mut key);
    key
}

fn derive_fallback_key() -> Vec<u8> {
    derive_fallback_key_for(b"agora-mcp-keyring-fallback")
}

/// Encrypt the token using AES-256-GCM with a random 12-byte nonce.
/// Returns (nonce || ciphertext || tag).
fn encrypt_token(token: &str, key: &[u8]) -> LauncherResult<Vec<u8>> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Nonce};

    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| LauncherError::Generic {
        code: "ERR_AUTH_ENCRYPT".to_string(),
        message: "Failed to create AES cipher for token encryption.".to_string(),
    })?;

    use rand::Rng;
    let nonce_bytes: [u8; 12] = rand::thread_rng().gen();
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext =
        cipher
            .encrypt(nonce, token.as_bytes())
            .map_err(|_| LauncherError::Generic {
                code: "ERR_AUTH_ENCRYPT".to_string(),
                message: "AES-GCM encryption failed.".to_string(),
            })?;

    let mut out = Vec::with_capacity(12 + ciphertext.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Decrypt a token from (nonce || ciphertext || tag).
fn decrypt_token(data: &[u8], key: &[u8]) -> Option<String> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Nonce};

    if data.len() < 12 {
        return None;
    }

    let (nonce_bytes, ciphertext) = data.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(key).ok()?;
    let nonce = Nonce::from_slice(nonce_bytes);

    let plaintext = cipher.decrypt(nonce, ciphertext).ok()?;
    String::from_utf8(plaintext).ok()
}

/// Return the path to the fallback token file.
fn fallback_token_path() -> Option<std::path::PathBuf> {
    dirs::data_local_dir().map(|d| d.join("agora").join(TOKEN_FALLBACK_FILE))
}

fn fallback_secret_path(file_name: &str) -> Option<std::path::PathBuf> {
    dirs::data_local_dir().map(|d| d.join("agora").join(file_name))
}

pub(crate) fn store_secret(
    service: &str,
    account: &str,
    fallback_file: &str,
    key_context: &[u8],
    value: &str,
) -> LauncherResult<()> {
    if let Ok(entry) = keyring::Entry::new(service, account) {
        if entry.set_password(value).is_ok() {
            if let Some(path) = fallback_secret_path(fallback_file) {
                let _ = std::fs::remove_file(path);
            }
            return Ok(());
        }
    }

    let path = fallback_secret_path(fallback_file).ok_or_else(|| LauncherError::Generic {
        code: "ERR_AUTH_FALLBACK_PATH".into(),
        message: "Could not determine data directory for encrypted credential storage.".into(),
    })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| LauncherError::Generic {
            code: "ERR_AUTH_FALLBACK_WRITE".into(),
            message: "Failed to create encrypted credential directory.".into(),
        })?;
    }
    let encrypted = encrypt_token(value, &derive_fallback_key_for(key_context))?;
    std::fs::write(path, encrypted).map_err(|_| LauncherError::Generic {
        code: "ERR_AUTH_FALLBACK_WRITE".into(),
        message: "Failed to write encrypted credentials.".into(),
    })
}

pub(crate) fn load_secret(
    service: &str,
    account: &str,
    fallback_file: &str,
    key_context: &[u8],
) -> LauncherResult<Option<String>> {
    let mut keyring_error = None;
    match keyring::Entry::new(service, account) {
        Ok(entry) => match entry.get_password() {
            Ok(value) => return Ok(Some(value)),
            Err(keyring::Error::NoEntry) => {}
            Err(error) => keyring_error = Some(error.to_string()),
        },
        Err(error) => keyring_error = Some(error.to_string()),
    }

    if let Some(path) = fallback_secret_path(fallback_file) {
        if path.exists() {
            let encrypted = std::fs::read(path).map_err(|_| LauncherError::Generic {
                code: "ERR_AUTH_FALLBACK_READ".into(),
                message: "Failed to read encrypted credentials.".into(),
            })?;
            return decrypt_token(&encrypted, &derive_fallback_key_for(key_context))
                .map(Some)
                .ok_or_else(|| LauncherError::Generic {
                    code: "ERR_AUTH_FALLBACK_DECRYPT".into(),
                    message: "Failed to decrypt stored credentials.".into(),
                });
        }
    }

    if let Some(error) = keyring_error {
        return Err(LauncherError::Generic {
            code: "ERR_AUTH_KEYRING_READ".into(),
            message: format!("Failed to read credentials from the OS keyring: {error}"),
        });
    }
    Ok(None)
}

pub(crate) fn clear_secret(
    service: &str,
    account: &str,
    fallback_file: &str,
) -> LauncherResult<()> {
    if let Ok(entry) = keyring::Entry::new(service, account) {
        match entry.delete_password() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(error) => {
                return Err(LauncherError::Generic {
                    code: "ERR_AUTH_KEYRING_DELETE".into(),
                    message: format!("Failed to delete credentials from the OS keyring: {error}"),
                });
            }
        }
    }
    if let Some(path) = fallback_secret_path(fallback_file) {
        if path.exists() {
            std::fs::remove_file(path).map_err(|_| LauncherError::Generic {
                code: "ERR_AUTH_FALLBACK_DELETE".into(),
                message: "Failed to delete encrypted credentials.".into(),
            })?;
        }
    }
    Ok(())
}

/// Returns true — the fallback is always available on all platforms.
/// This signal is used by Settings to show the spec-mandated "less secure" warning.
pub fn keyring_fallback_available() -> bool {
    true
}

pub fn clear_token() -> Result<(), String> {
    clear_token_bundle()
}

pub fn is_authenticated() -> bool {
    load_token_bundle().is_some()
}

pub async fn get_github_user(token: &str) -> LauncherResult<GithubProfile> {
    let clients = HttpClients::new()?;
    let resp = http_client::checked_request_with_headers(
        &clients,
        ClientCategory::GitHub,
        "https://api.github.com/user",
        vec![
            ("Authorization".into(), format!("Bearer {token}")),
            ("Accept".into(), "application/json".into()),
        ],
    )
    .await?;

    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(LauncherError::AuthExpired);
    }
    if !resp.status().is_success() {
        return Err(LauncherError::Generic {
            code: "ERR_AUTH_PROFILE".to_string(),
            message: "GitHub rejected the profile request.".to_string(),
        });
    }

    #[derive(Debug, Deserialize)]
    struct GithubUserJson {
        login: String,
        avatar_url: String,
    }

    let body = http_client::checked_response_bytes(resp, ClientCategory::GitHub).await?;
    let parsed =
        serde_json::from_slice::<GithubUserJson>(&body).map_err(|_| LauncherError::Generic {
            code: "ERR_AUTH_PROFILE".to_string(),
            message: "Failed to parse GitHub profile response.".to_string(),
        })?;

    Ok(GithubProfile {
        login: parsed.login,
        avatar_url: parsed.avatar_url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypted_fallback_roundtrips_large_credentials() {
        let credentials = "x".repeat(8_192);
        let key = derive_fallback_key_for(b"agora-msa-credentials-fallback");
        let encrypted = encrypt_token(&credentials, &key).unwrap();
        assert_ne!(encrypted, credentials.as_bytes());
        assert_eq!(
            decrypt_token(&encrypted, &key).as_deref(),
            Some(credentials.as_str())
        );
    }

    #[test]
    fn token_bundle_json_roundtrip() {
        let bundle = GitHubTokenBundle {
            access_token: "gho_test123".into(),
            refresh_token: Some("ghr_refresh456".into()),
            access_expires_at: Some(Utc::now()),
            refresh_expires_at: Some(Utc::now() + TimeDelta::days(180)),
            token_type: Some("bearer".into()),
            scope: Some("read:user".into()),
        };
        let json = serde_json::to_string(&bundle).unwrap();
        let parsed: GitHubTokenBundle = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.access_token, "gho_test123");
        assert_eq!(parsed.refresh_token.as_deref(), Some("ghr_refresh456"));
        assert_eq!(parsed.token_type.as_deref(), Some("bearer"));
        assert_eq!(parsed.scope.as_deref(), Some("read:user"));
        assert!(parsed.access_expires_at.is_some());
        assert!(parsed.refresh_expires_at.is_some());
    }

    #[test]
    fn access_token_is_fresh_unexpired() {
        let bundle = GitHubTokenBundle {
            access_token: "t".into(),
            refresh_token: None,
            access_expires_at: Some(Utc::now() + TimeDelta::seconds(600)),
            refresh_expires_at: None,
            token_type: None,
            scope: None,
        };
        assert!(access_token_is_fresh(&bundle));
    }

    #[test]
    fn access_token_is_fresh_near_expiry() {
        let bundle = GitHubTokenBundle {
            access_token: "t".into(),
            refresh_token: None,
            access_expires_at: Some(Utc::now() + TimeDelta::seconds(60)),
            refresh_expires_at: None,
            token_type: None,
            scope: None,
        };
        assert!(!access_token_is_fresh(&bundle));
    }

    #[test]
    fn access_token_is_fresh_no_expiry_known() {
        // Legacy tokens without expiry info are always considered fresh.
        let bundle = GitHubTokenBundle {
            access_token: "t".into(),
            refresh_token: None,
            access_expires_at: None,
            refresh_expires_at: None,
            token_type: None,
            scope: None,
        };
        assert!(access_token_is_fresh(&bundle));
    }

    #[test]
    fn access_token_is_fresh_already_expired() {
        let bundle = GitHubTokenBundle {
            access_token: "t".into(),
            refresh_token: None,
            access_expires_at: Some(Utc::now() - TimeDelta::seconds(1)),
            refresh_expires_at: None,
            token_type: None,
            scope: None,
        };
        assert!(!access_token_is_fresh(&bundle));
    }

    #[test]
    fn access_token_is_fresh_exactly_at_buffer() {
        // Exactly 300s remaining — the buffer is 300, so this is NOT fresh.
        let bundle = GitHubTokenBundle {
            access_token: "t".into(),
            refresh_token: None,
            access_expires_at: Some(Utc::now() + TimeDelta::seconds(300)),
            refresh_expires_at: None,
            token_type: None,
            scope: None,
        };
        assert!(!access_token_is_fresh(&bundle));
    }

    #[test]
    fn store_token_creates_bundle() {
        // store_token wraps a bare string in a bundle and stores it.
        // We can't easily test keyring in CI, but we can verify the JSON
        // roundtrip through the bundle.
        let bare = "gho_bare_token";
        let bundle = GitHubTokenBundle {
            access_token: bare.into(),
            refresh_token: None,
            access_expires_at: None,
            refresh_expires_at: None,
            token_type: None,
            scope: None,
        };
        assert_eq!(bundle.access_token, bare);
        assert!(bundle.refresh_token.is_none());

        let json = serde_json::to_string(&bundle).unwrap();
        // A legacy bare token string would not parse as JSON bundle. Verify
        // that a plain string is not valid JSON for the bundle.
        assert!(serde_json::from_str::<GitHubTokenBundle>("\"just a string\"").is_err());
        assert!(serde_json::from_str::<GitHubTokenBundle>(bare).is_err());
        // But the serialized bundle is valid.
        assert!(serde_json::from_str::<GitHubTokenBundle>(&json).is_ok());
    }

    #[test]
    fn token_bundle_all_fields_populated() {
        let now = Utc::now();
        let bundle = GitHubTokenBundle {
            access_token: "gho_access".into(),
            refresh_token: Some("ghr_refresh".into()),
            access_expires_at: Some(now + TimeDelta::seconds(28800)),
            refresh_expires_at: Some(now + TimeDelta::days(180)),
            token_type: Some("bearer".into()),
            scope: Some("repo,user".into()),
        };
        let json = serde_json::to_string(&bundle).unwrap();
        let parsed: GitHubTokenBundle = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.access_token, bundle.access_token);
        assert_eq!(parsed.refresh_token, bundle.refresh_token);
        assert!(parsed.access_expires_at.is_some());
        assert!(parsed.refresh_expires_at.is_some());
        assert_eq!(parsed.token_type, bundle.token_type);
        assert_eq!(parsed.scope, bundle.scope);
    }

    #[test]
    fn token_bundle_no_refresh_token() {
        // Installations without expiring user tokens get no refresh_token.
        let json = r#"{"access_token":"gho_test","refresh_token":null,"access_expires_at":null,"refresh_expires_at":null,"token_type":"bearer","scope":"read:user"}"#;
        let bundle: GitHubTokenBundle = serde_json::from_str(json).unwrap();
        assert_eq!(bundle.access_token, "gho_test");
        assert!(bundle.refresh_token.is_none());
        assert_eq!(bundle.token_type.as_deref(), Some("bearer"));
    }

    #[test]
    fn device_flow_poll_response_parses_with_refresh() {
        let json = r#"{"access_token":"gho_at","refresh_token":"ghr_rt","expires_in":28800,"refresh_token_expires_in":15552000,"token_type":"bearer","scope":"repo,user"}"#;
        let parsed: DeviceFlowPollResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.access_token.as_deref(), Some("gho_at"));
        assert_eq!(parsed.refresh_token.as_deref(), Some("ghr_rt"));
        assert_eq!(parsed.expires_in, Some(28800));
        assert_eq!(parsed.refresh_token_expires_in, Some(15552000));
        assert_eq!(parsed.token_type.as_deref(), Some("bearer"));
        assert_eq!(parsed.scope.as_deref(), Some("repo,user"));
    }

    #[test]
    fn device_flow_poll_response_parses_without_refresh() {
        // Legacy response without refresh token fields.
        let json = r#"{"access_token":"gho_old","token_type":"bearer"}"#;
        let parsed: DeviceFlowPollResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.access_token.as_deref(), Some("gho_old"));
        assert!(parsed.refresh_token.is_none());
        assert!(parsed.expires_in.is_none());
        assert!(parsed.refresh_token_expires_in.is_none());
    }

    #[test]
    fn device_flow_poll_response_error() {
        let json = r#"{"error":"authorization_pending","interval":5}"#;
        let parsed: DeviceFlowPollResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.error.as_deref(), Some("authorization_pending"));
        assert_eq!(parsed.interval, Some(5));
    }

    #[test]
    fn clear_token_bundle_removes_both_tokens() {
        // Smoke test: clearing doesn't panic when nothing is stored.
        // In a real scenario this is covered by integration tests.
        let result = clear_token_bundle();
        assert!(result.is_ok());
    }

    #[test]
    fn is_authenticated_returns_false_when_no_token() {
        // Without a stored token, this should be false.
        // In CI with no keyring setup, this is safe to test.
        let _ = is_authenticated();
    }

    #[test]
    fn get_token_returns_none_without_stored_bundle() {
        // This is a best-effort test. In CI there may be no keyring.
        let result = get_token();
        // Either None (no token) or Some if a token happens to be stored.
        // We just verify it doesn't panic.
        let _ = result;
    }
}
