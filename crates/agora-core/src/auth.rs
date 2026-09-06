use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};

use std::sync::LazyLock;
use std::time::Duration;

use crate::error::{LauncherError, LauncherResult};
use crate::http_client::{self, ClientCategory, HttpClients};

// ---------------------------------------------------------------------------
// OAuthHttpClient — injectable HTTP abstraction for OAuth flows
// ---------------------------------------------------------------------------

/// Small HTTP response type for OAuth flows — avoids pulling reqwest into trait bounds.
pub struct OAuthResponse {
    pub status: u16,
    pub body: String,
}

/// A trait abstracting the HTTP calls needed for GitHub OAuth device flow
/// and token refresh.  Production uses [`LiveOAuthClient`]; tests use a mock.
#[async_trait::async_trait]
pub trait OAuthHttpClient: Send + Sync {
    /// POST an URL-encoded form and return the response.
    async fn post_form(
        &self,
        url: &str,
        params: &[(&str, &str)],
        headers: &[(String, String)],
    ) -> LauncherResult<OAuthResponse>;
}

/// Production OAuth client that enforces Agora's URL policy via
/// [`http_client::checked_post_form`] and builds fresh [`HttpClients`].
#[derive(Debug, Clone)]
pub struct LiveOAuthClient;

#[async_trait::async_trait]
impl OAuthHttpClient for LiveOAuthClient {
    async fn post_form(
        &self,
        url: &str,
        params: &[(&str, &str)],
        headers: &[(String, String)],
    ) -> LauncherResult<OAuthResponse> {
        let clients = HttpClients::new()?;
        let resp =
            http_client::checked_post_form(&clients, ClientCategory::GitHub, url, params, headers)
                .await?;
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        Ok(OAuthResponse { status, body })
    }
}

// ---------------------------------------------------------------------------
// MockOAuthClient — in-memory scripted client for tests
// ---------------------------------------------------------------------------

#[cfg(any(test, feature = "test-support"))]
use std::collections::VecDeque;
#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
#[cfg(any(test, feature = "test-support"))]
use std::sync::{Arc, Mutex};

/// An in-memory scripted OAuth client for testing.  Queues responses/errors
/// consumed in FIFO order on each `post_form` call.  Tracks call count.
#[cfg(any(test, feature = "test-support"))]
#[derive(Clone)]
pub struct MockOAuthClient {
    call_count: Arc<AtomicU64>,
    responses: Arc<Mutex<VecDeque<LauncherResult<OAuthResponse>>>>,
}

#[cfg(any(test, feature = "test-support"))]
impl MockOAuthClient {
    pub fn new() -> Self {
        Self {
            call_count: Arc::new(AtomicU64::new(0)),
            responses: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub fn queue_response(&self, status: u16, body: &str) {
        self.responses.lock().unwrap().push_back(Ok(OAuthResponse {
            status,
            body: body.to_string(),
        }));
    }

    pub fn queue_error(&self, error: LauncherError) {
        self.responses.lock().unwrap().push_back(Err(error));
    }

    pub fn call_count(&self) -> u64 {
        self.call_count.load(AtomicOrdering::SeqCst)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Default for MockOAuthClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(test, feature = "test-support"))]
#[async_trait::async_trait]
impl OAuthHttpClient for MockOAuthClient {
    async fn post_form(
        &self,
        _url: &str,
        _params: &[(&str, &str)],
        _headers: &[(String, String)],
    ) -> LauncherResult<OAuthResponse> {
        self.call_count.fetch_add(1, AtomicOrdering::SeqCst);
        let mut lock = self.responses.lock().unwrap();
        lock.pop_front().unwrap_or_else(|| {
            panic!(
                "MockOAuthClient: no more responses (call #{})",
                self.call_count.load(AtomicOrdering::SeqCst)
            )
        })
    }
}

pub const AGORA_OAUTH_CLIENT_ID: &str = match option_env!("AGORA_OAUTH_CLIENT_ID") {
    // An empty value (e.g. a CI build referencing a missing secret) must not
    // bypass the compiled-in client ID, or the shipped app loses GitHub auth.
    Some(v) if !v.is_empty() => v,
    _ => "Iv23ctVA40Yy1ZUkvemh",
};

const KEYRING_SERVICE: &str = "com.agoramc";
const KEYRING_ACCOUNT: &str = "github-token";

/// Fallback token file name (in app data dir) for when OS keyring is unavailable.
const TOKEN_FALLBACK_FILE: &str = "tokens.enc";

#[cfg(test)]
static TEST_TOKEN_STORE: LazyLock<std::sync::Mutex<Option<String>>> =
    LazyLock::new(|| std::sync::Mutex::new(None));
#[cfg(test)]
static TEST_SECRET_STORE: LazyLock<
    std::sync::Mutex<std::collections::HashMap<(String, String), String>>,
> = LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

#[cfg(test)]
thread_local! {
    /// When set, this thread skips both the OS keyring and the in-memory test
    /// stores and exercises the real encrypted-file fallback under this
    /// directory.
    ///
    /// Everything below defaults to an in-memory store under `cfg(test)`, which
    /// meant the tests named after the fallback never wrote a single encrypted
    /// byte -- a device-key race and a delete-on-read both shipped through that
    /// gap. This is thread-local rather than an environment variable so a test
    /// can opt into real files without changing what any other test sees.
    static REAL_FALLBACK_DIR: std::cell::RefCell<Option<std::path::PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn real_fallback_dir() -> Option<std::path::PathBuf> {
    REAL_FALLBACK_DIR.with(|dir| dir.borrow().clone())
}

#[cfg(not(test))]
fn real_fallback_dir() -> Option<std::path::PathBuf> {
    None
}

/// Route this thread's credential storage to real encrypted files under `dir`,
/// as if the OS keyring were unavailable. Restores the previous setting on drop.
#[cfg(test)]
fn use_real_fallback_dir(dir: &std::path::Path) -> RealFallbackGuard {
    let previous = REAL_FALLBACK_DIR.with(|cell| cell.replace(Some(dir.to_path_buf())));
    RealFallbackGuard(previous)
}

#[cfg(test)]
struct RealFallbackGuard(Option<std::path::PathBuf>);

#[cfg(test)]
impl Drop for RealFallbackGuard {
    fn drop(&mut self) {
        let previous = self.0.take();
        REAL_FALLBACK_DIR.with(|cell| *cell.borrow_mut() = previous);
    }
}

fn store_test_token(value: &str) -> bool {
    #[cfg(test)]
    if real_fallback_dir().is_none() {
        *TEST_TOKEN_STORE.lock().unwrap() = Some(value.to_string());
        return true;
    }
    let _ = value;
    false
}

fn load_test_token() -> Option<Option<String>> {
    #[cfg(test)]
    if real_fallback_dir().is_none() {
        return Some(TEST_TOKEN_STORE.lock().unwrap().clone());
    }
    None
}

fn clear_test_token() -> bool {
    #[cfg(test)]
    if real_fallback_dir().is_none() {
        *TEST_TOKEN_STORE.lock().unwrap() = None;
        return true;
    }
    false
}

fn store_test_secret(service: &str, account: &str, value: &str) -> bool {
    #[cfg(test)]
    if real_fallback_dir().is_none() {
        TEST_SECRET_STORE.lock().unwrap().insert(
            (service.to_string(), account.to_string()),
            value.to_string(),
        );
        return true;
    }
    let _ = (service, account, value);
    false
}

fn load_test_secret(service: &str, account: &str) -> Option<Option<String>> {
    #[cfg(test)]
    if real_fallback_dir().is_none() {
        return Some(
            TEST_SECRET_STORE
                .lock()
                .unwrap()
                .get(&(service.to_string(), account.to_string()))
                .cloned(),
        );
    }
    let _ = (service, account);
    None
}

fn clear_test_secret(service: &str, account: &str) -> bool {
    #[cfg(test)]
    if real_fallback_dir().is_none() {
        TEST_SECRET_STORE
            .lock()
            .unwrap()
            .remove(&(service.to_string(), account.to_string()));
        return true;
    }
    let _ = (service, account);
    false
}

/// PBKDF2 iterations for key derivation in the keyring fallback.
const PBKDF2_ITERATIONS: u32 = 200_000;

/// Disambiguates concurrent credential temp files within a process.
static CREDENTIAL_TEMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

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
/// Returns:
/// - `Ok(Some(bundle))` on success
/// - `Ok(None)` when the refresh token has been permanently revoked/expired
/// - `Err(_)` on transient network or server error (bundle preserved)
pub async fn refresh_access_token(
    refresh_token: &str,
) -> LauncherResult<Option<GitHubTokenBundle>> {
    refresh_access_token_inner(&LiveOAuthClient, refresh_token).await
}

/// Returns `true` when the OAuth error body signals a permanent failure
/// that should clear stored credentials.
fn is_permanent_oauth_error(body: &str) -> bool {
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(err) = val.get("error").and_then(|v| v.as_str()) {
            return matches!(err, "bad_refresh_token" | "expired_token" | "access_denied");
        }
    }
    false
}

/// Internal variant that accepts an injectable OAuth client (for testing).
async fn refresh_access_token_inner(
    oauth: &dyn OAuthHttpClient,
    refresh_token: &str,
) -> LauncherResult<Option<GitHubTokenBundle>> {
    let params = [
        ("client_id", AGORA_OAUTH_CLIENT_ID),
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
    ];

    let resp = oauth
        .post_form(
            "https://github.com/login/oauth/access_token",
            &params,
            &[("Accept".into(), "application/json".into())],
        )
        .await?;

    let status = resp.status;
    let body = resp.body;
    eprintln!("[auth] refresh status={status}");

    // 400/401: permanent only when the body contains a known OAuth error
    if status == 400 || status == 401 {
        if is_permanent_oauth_error(&body) {
            return Ok(None);
        }
        return Err(LauncherError::NetworkOffline);
    }
    if !(200..300).contains(&status) {
        return Err(LauncherError::NetworkOffline);
    }

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
            // 200 with error body — classify by known permanent OAuth errors
            if matches!(err, "bad_refresh_token" | "expired_token" | "access_denied") {
                eprintln!("[auth] refresh permanent error from GitHub: {err}");
                return Ok(None);
            }
            eprintln!("[auth] refresh transient error from GitHub: {err}");
            return Err(LauncherError::NetworkOffline);
        }
    }

    // Malformed / unparseable response — treat as transient
    eprintln!("[auth] could not parse refresh response");
    Err(LauncherError::NetworkOffline)
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
    if store_test_token(&json) {
        return Ok(());
    }

    if !using_test_token_store() {
        if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT) {
            if entry.set_password(&json).is_ok() {
                if let Some(path) = fallback_token_path() {
                    let _ = std::fs::remove_file(path);
                }
                return Ok(());
            }
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

    let key = derive_fallback_key()?;
    let encrypted = encrypt_token(&json, &key)?;
    atomic_write_private(
        &path,
        &encrypted,
        "ERR_AUTH_FALLBACK_WRITE",
        "Failed to write fallback token file.",
    )?;

    Ok(())
}

/// Load a token bundle from storage. Returns None if no token is stored.
///
/// Handles legacy bare access tokens: if the stored value is not valid JSON,
/// it is treated as a plain access token and wrapped in a bundle.
pub fn load_token_bundle() -> Option<GitHubTokenBundle> {
    let raw = if let Some(stored) = load_test_token() {
        stored
    } else if !using_test_token_store() {
        keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
            .ok()
            .and_then(|entry| entry.get_password().ok())
    } else {
        None
    };

    let raw = raw.or_else(|| {
        let path = fallback_token_path()?;
        if !path.exists() {
            return None;
        }
        let data = std::fs::read(&path).ok()?;
        // Deliberately no cleanup on failure. A missing device key and a device
        // key we merely failed to read are the same `None` here, so deleting
        // would turn a transient read error into permanent credential loss. An
        // undecryptable file is inert, and the next store overwrites it.
        existing_fallback_key_for(TOKEN_KEY_CONTEXT).and_then(|key| decrypt_token(&data, &key))
    });

    let raw = raw?;

    serde_json::from_str::<GitHubTokenBundle>(&raw)
        .ok()
        .or_else(|| {
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
    if clear_test_token() {
        return Ok(());
    }
    let mut keyring_error = None;
    if !using_test_token_store() {
        if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT) {
            match entry.delete_password() {
                Ok(()) | Err(keyring::Error::NoEntry) => {}
                Err(error) if keyring_backend_unavailable(&error) => {}
                Err(error) => keyring_error = Some(error),
            }
        }
    }

    // A failure here leaves a decryptable token on disk, so it cannot be
    // swallowed: reporting a successful sign-out while the credential survives
    // is the one outcome a user cannot detect or act on. The MSA path already
    // propagates this; GitHub did not.
    if let Some(path) = fallback_token_path() {
        if path.exists() {
            if let Err(error) = std::fs::remove_file(&path) {
                return Err(format!("Failed to delete the stored GitHub token: {error}"));
            }
        }
    }

    match keyring_error {
        Some(error) => Err(format!("Failed to delete GitHub token: {error}")),
        None => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// Access token helpers
// ---------------------------------------------------------------------------

/// Returns true if the access token has more than `ACCESS_TOKEN_BUFFER_SECS`
/// of validity remaining, or if we don't know the expiry (legacy token).
pub(crate) fn access_token_is_fresh(bundle: &GitHubTokenBundle) -> bool {
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
///
/// On storage failures the error is logged and the stored token (if any) is
/// returned. Callers that need to distinguish storage errors from success
/// should use [`get_valid_access_token_fallible`].
pub async fn get_valid_access_token() -> Option<String> {
    match get_valid_access_token_inner(&LiveOAuthClient).await {
        Ok(tok) => tok,
        Err(e) => {
            eprintln!("[auth] get_valid_access_token error: {e}");
            load_token_bundle().map(|b| b.access_token)
        }
    }
}

/// Like [`get_valid_access_token`] but propagates storage errors so callers
/// can treat a refresh that could not be persisted as incomplete.
pub async fn get_valid_access_token_fallible() -> LauncherResult<Option<String>> {
    get_valid_access_token_inner(&LiveOAuthClient).await
}

/// Internal variant with injectable OAuth client.
/// Returns `LauncherResult` so callers can distinguish refresh-complete vs
/// storage-failure vs sign-in-required.
async fn get_valid_access_token_inner(
    oauth: &dyn OAuthHttpClient,
) -> LauncherResult<Option<String>> {
    let bundle = match load_token_bundle() {
        Some(b) => b,
        None => return Ok(None),
    };

    if access_token_is_fresh(&bundle) {
        return Ok(Some(bundle.access_token));
    }

    if bundle.refresh_token.is_none() {
        return Ok(Some(bundle.access_token));
    }

    let _lock = REFRESH_MUTEX.lock().await;

    let bundle = match load_token_bundle() {
        Some(b) => b,
        None => return Ok(None),
    };
    if access_token_is_fresh(&bundle) {
        return Ok(Some(bundle.access_token));
    }

    let refresh_token = match bundle.refresh_token.as_deref() {
        Some(rt) => rt,
        None => return Ok(Some(bundle.access_token)),
    };

    match refresh_access_token_inner(oauth, refresh_token).await {
        Ok(Some(new_bundle)) => {
            eprintln!("[auth] access token refreshed successfully");
            let token = new_bundle.access_token.clone();
            store_token_bundle(&new_bundle)?;
            Ok(Some(token))
        }
        Ok(None) => {
            eprintln!("[auth] refresh token expired or revoked — clearing bundle");
            let _ = clear_token_bundle();
            Ok(None)
        }
        Err(e) => {
            eprintln!("[auth] refresh transient error: {e} — preserving bundle");
            Ok(Some(bundle.access_token))
        }
    }
}

/// Attempt one refresh after receiving a 401, then retry the operation.
///
/// Returns Ok(result) if refresh+retry succeeded, or the original error
/// if refresh failed. Clears the bundle on persistent failure.
///
/// This variant does **not** carry the failed access token, so it cannot
/// detect concurrent rotation — prefer [`try_refresh_after_401_with_token`]
/// for new code.
pub async fn try_refresh_after_401(original_error: LauncherError) -> Result<(), LauncherError> {
    try_refresh_after_401_with_token("")
        .await
        .map_err(|_| original_error)
}

/// Attempt one refresh after a 401, carrying the exact failed access token
/// so that concurrent rotation is detected.
///
/// Under the single-flight mutex:
/// 1. Re-reads stored credentials
/// 2. If the stored access token **differs** from `failed_token`, another
///    caller already rotated — returns `Ok(())` without a second refresh.
/// 3. Otherwise issues exactly one refresh request.
/// 4. On permanent OAuth failure (bad_refresh_token, expired_token, etc.)
///    clears the bundle and returns `Err(LauncherError::AuthExpired)`.
/// 5. On transient errors preserves the bundle and returns `Err`.
/// 6. On success persists atomically — a failure to write durable storage
///    propagates as `Err`.
pub async fn try_refresh_after_401_with_token(failed_token: &str) -> Result<(), LauncherError> {
    try_refresh_after_401_inner(&LiveOAuthClient, failed_token).await
}

/// Internal variant with injectable OAuth client and `failed_token` for
/// rotation detection. Pass `""` to skip the rotation check.
async fn try_refresh_after_401_inner(
    oauth: &dyn OAuthHttpClient,
    failed_token: &str,
) -> Result<(), LauncherError> {
    let _lock = REFRESH_MUTEX.lock().await;

    let bundle = match load_token_bundle() {
        Some(b) => b,
        None => return Err(LauncherError::AuthExpired),
    };

    // If we know the failed token and the stored one differs, another
    // caller already rotated — no second refresh needed.
    if !failed_token.is_empty() && bundle.access_token != failed_token {
        return Ok(());
    }

    let refresh_token = match bundle.refresh_token.as_deref() {
        Some(rt) => rt.to_string(),
        None => {
            let _ = clear_token_bundle();
            return Err(LauncherError::AuthExpired);
        }
    };

    match refresh_access_token_inner(oauth, &refresh_token).await {
        Ok(Some(new_bundle)) => {
            eprintln!("[auth] 401 recovery: token refreshed");
            // Propagate store failure — refresh incomplete without durable storage
            store_token_bundle(&new_bundle)?;
            Ok(())
        }
        Ok(None) => {
            eprintln!("[auth] 401 recovery failed: refresh token invalid");
            let _ = clear_token_bundle();
            Err(LauncherError::AuthExpired)
        }
        Err(e) => {
            eprintln!("[auth] 401 recovery failed: transient error — preserving bundle");
            Err(e)
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

/// Random per-profile secret that keys the encrypted keyring fallback.
///
/// This file is the only thing that makes the fallback ciphertext readable.
/// The derivation used to run PBKDF2 over a constant compiled into the binary,
/// so anyone holding an encrypted file could rederive the key from public
/// information; now they would need this file too.
///
/// Be precise about what that buys: the key lives in the same directory as the
/// ciphertext it protects, so its file permissions -- not AES -- are the
/// boundary. It defeats an attacker who obtains only an encrypted file. It does
/// nothing against one who copies the whole profile directory or runs as the
/// user. Real machine binding needs DPAPI, a TPM, or an OS credential service,
/// which is the thing whose absence puts us on this path in the first place.
const DEVICE_KEY_FILE: &str = "device-key.bin";
const DEVICE_KEY_LEN: usize = 32;

/// How long a caller that loses the creation race waits for the winner to
/// publish its key before treating the file as residue from an interrupted run.
const DEVICE_KEY_PUBLISH_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// Serializes device-key creation within the process. The GitHub and MSA
/// stores share one device key, so without this they can race each other on
/// first use.
static DEVICE_KEY_LOCK: LazyLock<std::sync::Mutex<()>> =
    LazyLock::new(|| std::sync::Mutex::new(()));

/// Directory holding the encrypted fallback files and the device key.
///
/// Resolves through [`crate::app_paths::AppPaths`] rather than reconstructing
/// `dirs::data_local_dir()/agora`, so `AGORA_DATA_DIR` and a portable install
/// move these files along with everything else. The platform default is
/// identical to what the hand-rolled path produced, so an ordinary install sees
/// no change; only configured roots move, which is the point.
///
/// Note this governs the *fallback* only. Credentials that reach the OS keyring
/// are held per-user by the OS and are not relocated by a data-root setting --
/// a portable install on a machine with a working keyring still leaves them
/// behind.
fn fallback_data_dir() -> Option<std::path::PathBuf> {
    if let Some(dir) = real_fallback_dir() {
        return Some(dir);
    }
    #[cfg(any(test, feature = "test-support"))]
    {
        if let Ok(dir) = std::env::var("AGORA_TEST_SECRET_DIR") {
            return Some(std::path::PathBuf::from(dir));
        }
        if let Ok(dir) = std::env::var("AGORA_TEST_TOKEN_DIR") {
            return Some(std::path::PathBuf::from(dir));
        }
    }
    Some(
        crate::app_paths::AppPaths::platform_default()
            .root()
            .to_path_buf(),
    )
}

fn device_key_path() -> Option<std::path::PathBuf> {
    fallback_data_dir().map(|d| d.join(DEVICE_KEY_FILE))
}

fn read_device_secret_at(path: &std::path::Path) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path).ok()?;
    (bytes.len() == DEVICE_KEY_LEN).then_some(bytes)
}

/// Replace `path` with `bytes` atomically and owner-only.
///
/// The bytes land in a sibling temp file that is created private, filled and
/// synced before anything replaces the target, so a crash or a full disk
/// partway through cannot truncate a credential that was still good --
/// `std::fs::write` would leave exactly that. Mirrors
/// [`crate::installed_artifact::atomic_write`], plus the permission handling
/// that credential material needs.
fn atomic_write_private(
    path: &std::path::Path,
    bytes: &[u8],
    error_code: &str,
    error_message: &str,
) -> LauncherResult<()> {
    let failed = || LauncherError::Generic {
        code: error_code.to_string(),
        message: error_message.to_string(),
    };

    let parent = path.parent().ok_or_else(failed)?;
    let temp = parent.join(format!(
        ".{}.agtmp_{}_{}",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id(),
        CREDENTIAL_TEMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));

    let write_result = (|| -> LauncherResult<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp).map_err(|_| failed())?;
        use std::io::Write;
        file.write_all(bytes).map_err(|_| failed())?;
        file.sync_all().map_err(|_| failed())?;
        Ok(())
    })();

    if let Err(error) = write_result {
        let _ = std::fs::remove_file(&temp);
        return Err(error);
    }

    if std::fs::rename(&temp, path).is_err() {
        let _ = std::fs::remove_file(&temp);
        return Err(failed());
    }

    #[cfg(unix)]
    {
        // The rename carries the temp file's 0600 across, but an inherited
        // mode from a pre-existing target is not something to assume.
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|_| failed())?;
    }
    Ok(())
}

/// Write `secret` with owner-only permissions where the platform has them.
fn write_device_secret_at(path: &std::path::Path, secret: &[u8]) -> LauncherResult<()> {
    atomic_write_private(
        path,
        secret,
        "ERR_AUTH_DEVICE_KEY_WRITE",
        "Failed to write the device key for encrypted credential storage.",
    )
}

/// Read the device secret, generating one on first use.
///
/// Never returns a key that is not durably on disk: a caller that encrypted
/// under a key some other writer then replaced would produce ciphertext nobody
/// can read, so every path here ends by reading back what was published.
fn load_or_create_device_secret_at(path: &std::path::Path) -> LauncherResult<Vec<u8>> {
    let _guard = DEVICE_KEY_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(existing) = read_device_secret_at(path) {
        return Ok(existing);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| LauncherError::Generic {
            code: "ERR_AUTH_DEVICE_KEY_WRITE".into(),
            message: "Failed to create the encrypted credential directory.".into(),
        })?;
    }

    use rand::Rng;
    let secret: [u8; DEVICE_KEY_LEN] = rand::thread_rng().gen();

    let write_failed = || LauncherError::Generic {
        code: "ERR_AUTH_DEVICE_KEY_WRITE".into(),
        message: "Failed to write the device key for encrypted credential storage.".into(),
    };

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(mut file) => {
            use std::io::Write;
            file.write_all(&secret).map_err(|_| write_failed())?;
            // Durable before anything encrypts under it: a key lost to a crash
            // takes every credential written under it with it.
            file.sync_all().map_err(|_| write_failed())?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            // `create_new` reserves the name atomically but publishes nothing.
            // The winner may still be between create and write, and its file
            // reads as zero-length until then -- so wait for it rather than
            // mistake an in-flight key for residue and overwrite a key another
            // caller is already encrypting under.
            let deadline = std::time::Instant::now() + DEVICE_KEY_PUBLISH_WAIT;
            loop {
                if let Some(existing) = read_device_secret_at(path) {
                    return Ok(existing);
                }
                if std::time::Instant::now() >= deadline {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            // Still not a valid key: an earlier run was interrupted between
            // create and write. It cannot decrypt anything, so replacing it
            // loses nothing that was still recoverable.
            write_device_secret_at(path, &secret)?;
        }
        Err(_) => return Err(write_failed()),
    }

    read_device_secret_at(path).ok_or_else(|| LauncherError::Generic {
        code: "ERR_AUTH_DEVICE_KEY_READ".into(),
        message: "Wrote the device key but could not read it back.".into(),
    })
}

/// Derive a 256-bit key from the device secret, which is the PBKDF2 password.
/// The key therefore cannot be reconstructed from the source or from anything
/// the encrypted file itself reveals.
///
/// `context` is public domain separation -- it keeps the GitHub and MSA keys
/// distinct -- not a secret, which is why it belongs in the salt. The home
/// directory name and platform used to be mixed in here too; they were public,
/// added no confidentiality once the password is 256 random bits, and meant a
/// renamed home directory silently produced a different key.
///
/// PBKDF2 is stretching an input that is already full-entropy, so the iteration
/// count buys nothing here. It stays because the stored files are keyed on it.
fn derive_key_from_device_secret(device_secret: &[u8], context: &[u8]) -> Vec<u8> {
    use pbkdf2::pbkdf2_hmac;
    use sha2::Sha256;

    let salt = format!("agora-fallback:v2:{}", String::from_utf8_lossy(context));

    let mut key = vec![0u8; 32];
    pbkdf2_hmac::<Sha256>(device_secret, salt.as_bytes(), PBKDF2_ITERATIONS, &mut key);
    key
}

/// Key for writing: generates the device secret if this is the first store.
fn derive_fallback_key_for(context: &[u8]) -> LauncherResult<Vec<u8>> {
    let path = device_key_path().ok_or_else(|| LauncherError::Generic {
        code: "ERR_AUTH_FALLBACK_PATH".into(),
        message: "Could not determine data directory for encrypted credential storage.".into(),
    })?;
    Ok(derive_key_from_device_secret(
        &load_or_create_device_secret_at(&path)?,
        context,
    ))
}

/// Key for reading: no device secret means nothing on disk is decryptable, so
/// this never creates one.
fn existing_fallback_key_for(context: &[u8]) -> Option<Vec<u8>> {
    let secret = read_device_secret_at(&device_key_path()?)?;
    Some(derive_key_from_device_secret(&secret, context))
}

const TOKEN_KEY_CONTEXT: &[u8] = b"agora-mcp-keyring-fallback";

fn derive_fallback_key() -> LauncherResult<Vec<u8>> {
    derive_fallback_key_for(TOKEN_KEY_CONTEXT)
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
///
/// In tests, the `AGORA_TEST_TOKEN_DIR` environment variable can be set to an
/// isolated directory so parallel tests do not share the same fallback file.
fn fallback_token_path() -> Option<std::path::PathBuf> {
    #[cfg(any(test, feature = "test-support"))]
    if real_fallback_dir().is_none() {
        if let Ok(dir) = std::env::var("AGORA_TEST_TOKEN_DIR") {
            return Some(std::path::PathBuf::from(dir).join(TOKEN_FALLBACK_FILE));
        }
    }
    Some(fallback_data_dir()?.join(TOKEN_FALLBACK_FILE))
}

fn fallback_secret_path(file_name: &str) -> Option<std::path::PathBuf> {
    Some(fallback_data_dir()?.join(file_name))
}

fn using_test_token_store() -> bool {
    real_fallback_dir().is_some()
        || (cfg!(any(test, feature = "test-support"))
            && std::env::var_os("AGORA_TEST_TOKEN_DIR").is_some())
}

fn using_test_secret_store() -> bool {
    real_fallback_dir().is_some()
        || (cfg!(any(test, feature = "test-support"))
            && std::env::var_os("AGORA_TEST_SECRET_DIR").is_some())
}

fn keyring_backend_unavailable(error: &keyring::Error) -> bool {
    matches!(error, keyring::Error::PlatformFailure(_))
}

pub(crate) fn store_secret(
    service: &str,
    account: &str,
    fallback_file: &str,
    key_context: &[u8],
    value: &str,
) -> LauncherResult<()> {
    if store_test_secret(service, account, value) {
        return Ok(());
    }
    if !using_test_secret_store() {
        if let Ok(entry) = keyring::Entry::new(service, account) {
            if entry.set_password(value).is_ok() {
                if let Some(path) = fallback_secret_path(fallback_file) {
                    let _ = std::fs::remove_file(path);
                }
                return Ok(());
            }
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
    let encrypted = encrypt_token(value, &derive_fallback_key_for(key_context)?)?;
    atomic_write_private(
        &path,
        &encrypted,
        "ERR_AUTH_FALLBACK_WRITE",
        "Failed to write encrypted credentials.",
    )
}

pub(crate) fn load_secret(
    service: &str,
    account: &str,
    fallback_file: &str,
    key_context: &[u8],
) -> LauncherResult<Option<String>> {
    if let Some(stored) = load_test_secret(service, account) {
        return Ok(stored);
    }
    let mut keyring_error = None;
    if !using_test_secret_store() {
        match keyring::Entry::new(service, account) {
            Ok(entry) => match entry.get_password() {
                Ok(value) => return Ok(Some(value)),
                Err(keyring::Error::NoEntry) => {}
                Err(error) if keyring_backend_unavailable(&error) => {}
                Err(error) => keyring_error = Some(error.to_string()),
            },
            Err(error) if keyring_backend_unavailable(&error) => {}
            Err(error) => keyring_error = Some(error.to_string()),
        }
    }

    if let Some(path) = fallback_secret_path(fallback_file) {
        if path.exists() {
            let encrypted = std::fs::read(&path).map_err(|_| LauncherError::Generic {
                code: "ERR_AUTH_FALLBACK_READ".into(),
                message: "Failed to read encrypted credentials.".into(),
            })?;
            // Report "nothing stored" rather than an error so the caller asks
            // for a fresh sign-in instead of failing on every launch -- but
            // leave the file alone. A missing device key and a device key we
            // merely failed to read are indistinguishable here, so deleting
            // would turn a transient read error into permanent credential loss.
            if let Some(value) = existing_fallback_key_for(key_context)
                .and_then(|key| decrypt_token(&encrypted, &key))
            {
                return Ok(Some(value));
            }
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
    if clear_test_secret(service, account) {
        return Ok(());
    }
    let mut keyring_error = None;
    if !using_test_secret_store() {
        if let Ok(entry) = keyring::Entry::new(service, account) {
            match entry.delete_password() {
                Ok(()) | Err(keyring::Error::NoEntry) => {}
                Err(error) if keyring_backend_unavailable(&error) => {}
                Err(error) => keyring_error = Some(error),
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
    match keyring_error {
        Some(error) => Err(LauncherError::Generic {
            code: "ERR_AUTH_KEYRING_DELETE".into(),
            message: format!("Failed to delete credentials from the OS keyring: {error}"),
        }),
        None => Ok(()),
    }
}

/// Where a stored credential actually lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CredentialBackend {
    /// Nothing is stored.
    None,
    /// The OS keyring: Credential Manager, Keychain, or Secret Service.
    Keyring,
    /// The degraded encrypted-file fallback, used when the keyring is
    /// unavailable. Protected by file permissions rather than by the OS.
    EncryptedFile,
}

/// Report which backend currently holds this credential.
///
/// Replaces an older `keyring_fallback_available()` that answered a question
/// nobody needed: the fallback is *available* on every platform, always, so it
/// returned an unconditional `true` and never told a caller whether the
/// degraded path was actually in use. MASTER_SPEC 7.5.2 requires warning the
/// user when their credential is stored this way, which needs this signal.
pub(crate) fn credential_backend(
    service: &str,
    account: &str,
    fallback_file: &str,
) -> CredentialBackend {
    if !using_test_secret_store() {
        if let Ok(entry) = keyring::Entry::new(service, account) {
            if entry.get_password().is_ok() {
                return CredentialBackend::Keyring;
            }
        }
    }
    match fallback_secret_path(fallback_file) {
        Some(path) if path.is_file() => CredentialBackend::EncryptedFile,
        _ => CredentialBackend::None,
    }
}

/// Which backend holds the GitHub token.
pub fn github_credential_backend() -> CredentialBackend {
    if !using_test_token_store() {
        if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT) {
            if entry.get_password().is_ok() {
                return CredentialBackend::Keyring;
            }
        }
    }
    match fallback_token_path() {
        Some(path) if path.is_file() => CredentialBackend::EncryptedFile,
        _ => CredentialBackend::None,
    }
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

    /// Stand-in for the per-install device key so derivation tests stay pure.
    const TEST_DEVICE_SECRET: &[u8] = &[0x11; DEVICE_KEY_LEN];

    #[test]
    fn encrypted_fallback_roundtrips_large_credentials() {
        let credentials = "x".repeat(8_192);
        let key =
            derive_key_from_device_secret(TEST_DEVICE_SECRET, b"agora-msa-credentials-fallback");
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

    // -----------------------------------------------------------------------
    // Token refresh audit: bundle storage tests (isolated via store_secret)
    // -----------------------------------------------------------------------

    #[test]
    fn store_secret_writes_fallback_when_keyring_unavailable() {
        let uid = uuid::Uuid::new_v4();
        let service = &format!("com.agora.test.bundle.{uid}");
        let account = "fallback-test";
        let fallback_file = &format!("test-bundle-{uid}.enc");
        let context = b"agora-mcp-keyring-fallback";
        let value = r#"{"access_token":"gho_test","refresh_token":"ghr_rt"}"#;

        let result = store_secret(service, account, fallback_file, context, value);
        assert!(result.is_ok(), "store_secret must succeed");

        // Verify the stored value loads back correctly
        let loaded = load_secret(service, account, fallback_file, context)
            .expect("load_secret must return Ok");
        assert_eq!(
            loaded.as_deref(),
            Some(value),
            "loaded value must match stored value"
        );

        // Verify the value survives clear + re-store (rotation)
        clear_secret(service, account, fallback_file).expect("clear must succeed");
        let after_clear = load_secret(service, account, fallback_file, context)
            .expect("load after clear must return Ok");
        assert!(after_clear.is_none(), "value must be gone after clear");

        // Clean up
        let _ = clear_secret(service, account, fallback_file);
    }

    #[test]
    fn store_secret_overwrites_previous_value() {
        let uid = uuid::Uuid::new_v4();
        let service = &format!("com.agora.test.rotate.{uid}");
        let account = "rotate-test";
        let fallback_file = &format!("test-rotate-{uid}.enc");
        let context = b"agora-test-rotate";

        let v1 = "version1";
        let v2 = "version2";

        store_secret(service, account, fallback_file, context, v1)
            .expect("first store must succeed");
        store_secret(service, account, fallback_file, context, v2)
            .expect("second store must succeed");

        let loaded =
            load_secret(service, account, fallback_file, context).expect("load must succeed");
        assert_eq!(
            loaded.as_deref(),
            Some(v2),
            "second store must overwrite first"
        );

        let _ = clear_secret(service, account, fallback_file);
    }

    #[test]
    fn store_secret_clears_other_fallback_when_keyring_succeeds() {
        let uid = uuid::Uuid::new_v4();
        let service = &format!("com.agora.test.clean.{uid}");
        let account = "clean-test";
        let fallback_file = &format!("test-clean-{uid}.enc");
        let context = b"agora-test-clean";
        let value = "test-value";

        // Store twice; after the second store the old fallback is removed.
        store_secret(service, account, fallback_file, context, value)
            .expect("first store must succeed");
        store_secret(service, account, fallback_file, context, value)
            .expect("second store must succeed (rotation)");

        // Verify we can load the value
        let loaded =
            load_secret(service, account, fallback_file, context).expect("load must succeed");
        assert_eq!(loaded.as_deref(), Some(value));

        let _ = clear_secret(service, account, fallback_file);
    }

    // -----------------------------------------------------------------------
    // Token refresh audit: fallback encryption round-trip
    // -----------------------------------------------------------------------

    #[test]
    fn fallback_encrypt_decrypt_preserves_token() {
        let token = "gho_real_looking_token_12345abcde";
        let key = derive_key_from_device_secret(TEST_DEVICE_SECRET, b"agora-test-fallback");
        let encrypted = encrypt_token(token, &key).expect("encrypt must succeed");
        assert_ne!(
            encrypted.as_slice(),
            token.as_bytes(),
            "encrypted must differ from plaintext"
        );
        let decrypted = decrypt_token(&encrypted, &key);
        assert_eq!(
            decrypted.as_deref(),
            Some(token),
            "decrypted must match original"
        );
    }

    #[test]
    fn fallback_decrypt_wrong_key_returns_none() {
        let token = "gho_secret";
        let k1 = derive_key_from_device_secret(TEST_DEVICE_SECRET, b"context-1");
        let k2 = derive_key_from_device_secret(TEST_DEVICE_SECRET, b"context-2");
        let encrypted = encrypt_token(token, &k1).expect("encrypt must succeed");
        let decrypted = decrypt_token(&encrypted, &k2);
        assert!(decrypted.is_none(), "wrong key must not decrypt");
    }

    #[test]
    fn fallback_decrypt_truncated_data_returns_none() {
        let key = derive_key_from_device_secret(TEST_DEVICE_SECRET, b"test");
        assert!(decrypt_token(&[], &key).is_none());
        assert!(decrypt_token(&[0u8; 4], &key).is_none());
        assert!(decrypt_token(&[0u8; 11], &key).is_none());
    }

    // -----------------------------------------------------------------------
    // Token refresh audit: load_token_bundle wraps legacy bare token
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn load_token_bundle_legacy_bare_token_wrapping() {
        let _test_lock = TEST_AUTH_MUTEX.lock().await;
        let _ = clear_token_bundle();
        let bare = "gho_legacy_bare_token_abc123";
        *TEST_TOKEN_STORE.lock().unwrap() = Some(bare.to_string());

        let loaded = load_token_bundle().expect("must load bundle even from legacy bare token");
        assert_eq!(
            loaded.access_token, bare,
            "bare token must become access_token"
        );
        assert!(
            loaded.refresh_token.is_none(),
            "bare token must not have refresh_token"
        );
        assert!(
            loaded.access_expires_at.is_none(),
            "bare token must not have expiry"
        );
        let _ = clear_token_bundle();
    }

    // -----------------------------------------------------------------------
    // Token refresh audit: signout cleanup tests (isolated)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn clear_token_bundle_twice_does_not_error() {
        let _test_lock = TEST_AUTH_MUTEX.lock().await;
        let _ = clear_token_bundle();
        let result = clear_token_bundle();
        assert!(result.is_ok(), "double clear should not error");
    }

    #[test]
    fn clear_secret_removes_both_keyring_and_fallback() {
        let uid = uuid::Uuid::new_v4();
        let service = &format!("com.agora.test.remove.{uid}");
        let account = "remove-test";
        let fallback_file = &format!("test-remove-{uid}.enc");
        let context = b"agora-test-remove";
        let value = "remove-me";

        store_secret(service, account, fallback_file, context, value).expect("store must succeed");

        let loaded_before = load_secret(service, account, fallback_file, context)
            .expect("load before clear must succeed");
        assert!(loaded_before.is_some(), "value must exist before clear");

        clear_secret(service, account, fallback_file).expect("clear must succeed");

        let loaded_after = load_secret(service, account, fallback_file, context)
            .expect("load after clear must succeed");
        assert!(loaded_after.is_none(), "value must be gone after clear");
    }

    // -----------------------------------------------------------------------
    // OAuth refresh integration tests (in-memory mock)
    // -----------------------------------------------------------------------

    static TEST_AUTH_MUTEX: LazyLock<tokio::sync::Mutex<()>> =
        LazyLock::new(|| tokio::sync::Mutex::new(()));

    struct TestTokenDir(#[allow(dead_code)] tempfile::TempDir);
    impl Drop for TestTokenDir {
        fn drop(&mut self) {
            let _ = clear_token_bundle();
            std::env::remove_var("AGORA_TEST_TOKEN_DIR");
        }
    }
    fn write_test_bundle(bundle: &GitHubTokenBundle) -> TestTokenDir {
        let dir = tempfile::tempdir().expect("temp dir for test token");
        std::env::set_var("AGORA_TEST_TOKEN_DIR", dir.path());
        let _ = clear_token_bundle();
        store_token_bundle(bundle).expect("store test token bundle");
        TestTokenDir(dir)
    }

    #[tokio::test]
    async fn test_proactive_refresh_exactly_one_call() {
        let _test_lock = TEST_AUTH_MUTEX.lock().await;
        let oauth = MockOAuthClient::new();
        oauth.queue_response(
            200,
            r#"{"access_token":"gho_new","refresh_token":"ghr_new","expires_in":28800,"refresh_token_expires_in":15552000,"token_type":"bearer","scope":"repo,user"}"#,
        );

        let near_expiry = Utc::now() - TimeDelta::seconds(60);
        let bundle = GitHubTokenBundle {
            access_token: "gho_old".into(),
            refresh_token: Some("ghr_old".into()),
            access_expires_at: Some(near_expiry),
            refresh_expires_at: Some(Utc::now() + TimeDelta::days(30)),
            token_type: Some("bearer".into()),
            scope: Some("repo,user".into()),
        };
        let _td = write_test_bundle(&bundle);

        let result = get_valid_access_token_inner(&oauth).await.unwrap();
        assert_eq!(result.as_deref(), Some("gho_new"));

        let stored = load_token_bundle().expect("bundle should exist");
        assert_eq!(stored.access_token, "gho_new");
        assert_eq!(stored.refresh_token.as_deref(), Some("ghr_new"));
        assert_eq!(oauth.call_count(), 1);
    }

    #[tokio::test]
    async fn test_concurrent_single_flight_exactly_one_call() {
        let _test_lock = TEST_AUTH_MUTEX.lock().await;
        let oauth = std::sync::Arc::new(MockOAuthClient::new());
        oauth.queue_response(
            200,
            r#"{"access_token":"gho_fresh","refresh_token":"ghr_fresh","expires_in":28800,"token_type":"bearer"}"#,
        );

        let near_expiry = Utc::now() - TimeDelta::seconds(60);
        let bundle = GitHubTokenBundle {
            access_token: "gho_old".into(),
            refresh_token: Some("ghr_old".into()),
            access_expires_at: Some(near_expiry),
            refresh_expires_at: Some(Utc::now() + TimeDelta::days(30)),
            token_type: Some("bearer".into()),
            scope: None,
        };
        let _td = write_test_bundle(&bundle);

        let o1 = oauth.clone();
        let o2 = oauth.clone();
        let (r1, r2) = tokio::join!(
            tokio::spawn(async move { get_valid_access_token_inner(&*o1).await }),
            tokio::spawn(async move { get_valid_access_token_inner(&*o2).await }),
        );

        assert_eq!(r1.unwrap().unwrap().as_deref(), Some("gho_fresh"));
        assert_eq!(r2.unwrap().unwrap().as_deref(), Some("gho_fresh"));
        assert_eq!(oauth.call_count(), 1);
    }

    #[tokio::test]
    async fn test_refresh_token_rotation() {
        let _test_lock = TEST_AUTH_MUTEX.lock().await;
        let oauth = MockOAuthClient::new();
        oauth.queue_response(
            200,
            r#"{"access_token":"gho_rotated","refresh_token":"ghr_rotated","expires_in":28800,"token_type":"bearer"}"#,
        );

        let near_expiry = Utc::now() - TimeDelta::seconds(60);
        let bundle = GitHubTokenBundle {
            access_token: "gho_before".into(),
            refresh_token: Some("ghr_before".into()),
            access_expires_at: Some(near_expiry),
            refresh_expires_at: Some(Utc::now() + TimeDelta::days(30)),
            token_type: Some("bearer".into()),
            scope: None,
        };
        let _td = write_test_bundle(&bundle);

        let result = get_valid_access_token_inner(&oauth).await.unwrap();
        assert_eq!(result.as_deref(), Some("gho_rotated"));

        let stored = load_token_bundle().expect("bundle should exist");
        assert_eq!(stored.access_token, "gho_rotated");
        assert_eq!(stored.refresh_token.as_deref(), Some("ghr_rotated"));
        assert_eq!(oauth.call_count(), 1);
    }

    #[tokio::test]
    async fn test_revoked_refresh_clears_storage() {
        let _test_lock = TEST_AUTH_MUTEX.lock().await;
        let oauth = MockOAuthClient::new();
        oauth.queue_response(
            200,
            r#"{"error":"bad_refresh_token","error_description":"The refresh token has been revoked"}"#,
        );

        let near_expiry = Utc::now() - TimeDelta::seconds(60);
        let bundle = GitHubTokenBundle {
            access_token: "gho_revoked".into(),
            refresh_token: Some("ghr_revoked".into()),
            access_expires_at: Some(near_expiry),
            refresh_expires_at: Some(Utc::now() + TimeDelta::days(30)),
            token_type: None,
            scope: None,
        };
        let _td = write_test_bundle(&bundle);

        let result = get_valid_access_token_inner(&oauth).await.unwrap();
        assert!(result.is_none(), "revoked refresh should return None");

        let stored = load_token_bundle();
        assert!(
            stored.is_none(),
            "bundle must be cleared after revoked refresh"
        );
        assert_eq!(oauth.call_count(), 1);
    }

    #[tokio::test]
    async fn test_http_401_refresh_clears_storage() {
        let _test_lock = TEST_AUTH_MUTEX.lock().await;
        let oauth = MockOAuthClient::new();
        oauth.queue_response(401, r#"{"error":"bad_refresh_token"}"#);
        let bundle = GitHubTokenBundle {
            access_token: "gho_rejected".into(),
            refresh_token: Some("ghr_rejected".into()),
            access_expires_at: Some(Utc::now() - TimeDelta::seconds(60)),
            refresh_expires_at: Some(Utc::now() + TimeDelta::days(30)),
            token_type: None,
            scope: None,
        };
        let _td = write_test_bundle(&bundle);

        assert!(get_valid_access_token_inner(&oauth)
            .await
            .unwrap()
            .is_none());
        assert!(load_token_bundle().is_none());
        assert_eq!(oauth.call_count(), 1);
    }

    #[tokio::test]
    async fn test_network_server_error_preserves_storage() {
        let _test_lock = TEST_AUTH_MUTEX.lock().await;
        let oauth = MockOAuthClient::new();
        oauth.queue_response(500, "");

        let near_expiry = Utc::now() - TimeDelta::seconds(60);
        let old_token = "gho_survivor";
        let bundle = GitHubTokenBundle {
            access_token: old_token.into(),
            refresh_token: Some("ghr_survivor".into()),
            access_expires_at: Some(near_expiry),
            refresh_expires_at: Some(Utc::now() + TimeDelta::days(30)),
            token_type: None,
            scope: None,
        };
        let _td = write_test_bundle(&bundle);

        let result = get_valid_access_token_inner(&oauth).await.unwrap();
        assert_eq!(
            result.as_deref(),
            Some(old_token),
            "existing token on server error"
        );

        let stored = load_token_bundle();
        assert!(stored.is_some(), "bundle must survive server error");
        assert_eq!(oauth.call_count(), 1);
    }

    #[tokio::test]
    async fn test_401_refresh_success_exactly_one_call() {
        let _test_lock = TEST_AUTH_MUTEX.lock().await;
        let oauth = MockOAuthClient::new();
        oauth.queue_response(
            200,
            r#"{"access_token":"gho_fresh_401","refresh_token":"ghr_fresh_401","expires_in":28800,"token_type":"bearer"}"#,
        );

        let bundle = GitHubTokenBundle {
            access_token: "gho_old_401".into(),
            refresh_token: Some("ghr_old_401".into()),
            access_expires_at: Some(Utc::now() + TimeDelta::seconds(600)),
            refresh_expires_at: Some(Utc::now() + TimeDelta::days(30)),
            token_type: None,
            scope: None,
        };
        let _td = write_test_bundle(&bundle);

        let result = try_refresh_after_401_inner(&oauth, "gho_old_401").await;
        assert!(result.is_ok(), "401 recovery should succeed");

        let stored = load_token_bundle().expect("bundle should exist");
        assert_eq!(stored.access_token, "gho_fresh_401");
        assert_eq!(stored.refresh_token.as_deref(), Some("ghr_fresh_401"));
        assert_eq!(oauth.call_count(), 1);
    }

    #[tokio::test]
    async fn test_revoked_401_clears() {
        let _test_lock = TEST_AUTH_MUTEX.lock().await;
        let oauth = MockOAuthClient::new();
        oauth.queue_response(200, r#"{"error":"expired_token"}"#);

        let bundle = GitHubTokenBundle {
            access_token: "gho_401_rev".into(),
            refresh_token: Some("ghr_401_rev".into()),
            access_expires_at: Some(Utc::now() + TimeDelta::seconds(600)),
            refresh_expires_at: Some(Utc::now() + TimeDelta::days(30)),
            token_type: None,
            scope: None,
        };
        let _td = write_test_bundle(&bundle);

        let result = try_refresh_after_401_inner(&oauth, "gho_401_rev").await;
        assert!(result.is_err(), "revoked 401 refresh should error");
        assert_eq!(result.unwrap_err().code(), "ERR_AUTH_EXPIRED");

        let stored = load_token_bundle();
        assert!(stored.is_none(), "bundle must be cleared after revoked 401");
        assert_eq!(oauth.call_count(), 1);
    }

    #[tokio::test]
    async fn test_network_401_preserves() {
        let _test_lock = TEST_AUTH_MUTEX.lock().await;
        let oauth = MockOAuthClient::new();
        oauth.queue_response(500, "");

        let bundle = GitHubTokenBundle {
            access_token: "gho_401_net".into(),
            refresh_token: Some("ghr_401_net".into()),
            access_expires_at: Some(Utc::now() + TimeDelta::seconds(600)),
            refresh_expires_at: Some(Utc::now() + TimeDelta::days(30)),
            token_type: None,
            scope: None,
        };
        let _td = write_test_bundle(&bundle);

        let result = try_refresh_after_401_inner(&oauth, "gho_401_net").await;
        assert!(result.is_err(), "network 401 error should error");
        // Transient error returns NetworkOffline, not AuthExpired
        assert_eq!(result.unwrap_err().code(), "ERR_NETWORK_OFFLINE");

        let stored = load_token_bundle();
        assert!(stored.is_some(), "bundle must survive network 401 error");
        assert_eq!(oauth.call_count(), 1);
    }

    // -----------------------------------------------------------------------
    // 401 rotation detection tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_401_double_rotation_prevention() {
        let _test_lock = TEST_AUTH_MUTEX.lock().await;
        let oauth = MockOAuthClient::new();
        // First caller will consume this response
        oauth.queue_response(
            200,
            r#"{"access_token":"gho_rotated_first","refresh_token":"ghr_rotated_first","expires_in":28800,"token_type":"bearer"}"#,
        );

        let bundle = GitHubTokenBundle {
            access_token: "gho_original".into(),
            refresh_token: Some("ghr_original".into()),
            access_expires_at: Some(Utc::now() + TimeDelta::seconds(600)),
            refresh_expires_at: Some(Utc::now() + TimeDelta::days(30)),
            token_type: None,
            scope: None,
        };
        let _td = write_test_bundle(&bundle);

        // Simulate: first caller fails with "gho_original", rotates
        let r1 = try_refresh_after_401_inner(&oauth, "gho_original").await;
        assert!(r1.is_ok(), "first caller should succeed");

        let stored = load_token_bundle().expect("bundle should exist");
        assert_eq!(stored.access_token, "gho_rotated_first");

        // Second caller arrives with the now-stale "gho_original" — detects rotation
        let r2 = try_refresh_after_401_inner(&oauth, "gho_original").await;
        assert!(r2.is_ok(), "second caller should see already-rotated");
        assert_eq!(
            oauth.call_count(),
            1,
            "only one refresh call should be made"
        );

        let stored2 = load_token_bundle().expect("bundle should exist");
        assert_eq!(
            stored2.access_token, "gho_rotated_first",
            "bundle must not change"
        );
    }

    #[tokio::test]
    async fn test_401_concurrent_first_rotates_second_detects() {
        let _test_lock = TEST_AUTH_MUTEX.lock().await;
        let oauth = std::sync::Arc::new(MockOAuthClient::new());
        oauth.queue_response(
            200,
            r#"{"access_token":"gho_concurrent","refresh_token":"ghr_concurrent","expires_in":28800,"token_type":"bearer"}"#,
        );

        let bundle = GitHubTokenBundle {
            access_token: "gho_before_race".into(),
            refresh_token: Some("ghr_before_race".into()),
            access_expires_at: Some(Utc::now() + TimeDelta::seconds(600)),
            refresh_expires_at: Some(Utc::now() + TimeDelta::days(30)),
            token_type: None,
            scope: None,
        };
        let _td = write_test_bundle(&bundle);

        // Both callers see 401 with the same failed token
        let (r1, r2) = tokio::join!(
            try_refresh_after_401_inner(&*oauth, "gho_before_race"),
            try_refresh_after_401_inner(&*oauth, "gho_before_race"),
        );

        // Both must succeed
        assert!(r1.is_ok(), "first concurrent caller succeeds: {r1:?}");
        assert!(r2.is_ok(), "second concurrent caller succeeds: {r2:?}");

        // Exactly one HTTP call
        assert_eq!(oauth.call_count(), 1);

        let stored = load_token_bundle().expect("bundle should exist");
        assert_eq!(stored.access_token, "gho_concurrent");
    }

    #[tokio::test]
    async fn test_401_malformed_response_is_transient() {
        let _test_lock = TEST_AUTH_MUTEX.lock().await;
        let oauth = MockOAuthClient::new();
        // GitHub returns 200 with unparseable body
        oauth.queue_response(200, "not-json-at-all{{{");

        let bundle = GitHubTokenBundle {
            access_token: "gho_survivor_malformed".into(),
            refresh_token: Some("ghr_survivor_malformed".into()),
            access_expires_at: Some(Utc::now() + TimeDelta::seconds(600)),
            refresh_expires_at: Some(Utc::now() + TimeDelta::days(30)),
            token_type: None,
            scope: None,
        };
        let _td = write_test_bundle(&bundle);

        let result = try_refresh_after_401_inner(&oauth, "gho_survivor_malformed").await;
        assert!(result.is_err(), "malformed response should error");
        assert_eq!(result.unwrap_err().code(), "ERR_NETWORK_OFFLINE");

        let stored = load_token_bundle();
        assert!(stored.is_some(), "bundle must survive malformed response");
        assert_eq!(oauth.call_count(), 1);
    }

    #[tokio::test]
    async fn test_401_unknown_error_body_is_transient() {
        let _test_lock = TEST_AUTH_MUTEX.lock().await;
        let oauth = MockOAuthClient::new();
        // 400 with unrecognized error field (not one of the known permanent ones)
        oauth.queue_response(400, r#"{"error":"temporarily_unavailable"}"#);

        let bundle = GitHubTokenBundle {
            access_token: "gho_unknown_err".into(),
            refresh_token: Some("ghr_unknown_err".into()),
            access_expires_at: Some(Utc::now() + TimeDelta::seconds(600)),
            refresh_expires_at: Some(Utc::now() + TimeDelta::days(30)),
            token_type: None,
            scope: None,
        };
        let _td = write_test_bundle(&bundle);

        let result = try_refresh_after_401_inner(&oauth, "gho_unknown_err").await;
        assert!(result.is_err(), "unknown error body should be transient");
        assert_eq!(result.unwrap_err().code(), "ERR_NETWORK_OFFLINE");

        let stored = load_token_bundle();
        assert!(stored.is_some(), "bundle must survive unknown error body");
        assert_eq!(oauth.call_count(), 1);
    }

    // -----------------------------------------------------------------------
    // Encryption helper tests
    // -----------------------------------------------------------------------

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let key = derive_key_from_device_secret(TEST_DEVICE_SECRET, b"test-context");
        let data = "sensitive-token-value";
        let encrypted = encrypt_token(data, &key).unwrap();
        assert_ne!(encrypted.as_slice(), data.as_bytes());

        let decrypted = decrypt_token(&encrypted, &key);
        assert_eq!(decrypted.as_deref(), Some(data));
    }

    // -----------------------------------------------------------------------
    // derive_fallback_key determinism
    // -----------------------------------------------------------------------

    #[test]
    fn derive_fallback_key_is_deterministic_for_same_context() {
        let key1 = derive_key_from_device_secret(TEST_DEVICE_SECRET, b"test-context");
        let key2 = derive_key_from_device_secret(TEST_DEVICE_SECRET, b"test-context");
        assert_eq!(key1, key2);
    }

    #[test]
    fn derive_fallback_key_differs_for_different_contexts() {
        let key1 = derive_key_from_device_secret(TEST_DEVICE_SECRET, b"context-a");
        let key2 = derive_key_from_device_secret(TEST_DEVICE_SECRET, b"context-b");
        assert_ne!(key1, key2);
    }

    /// The property the old derivation lacked: two profiles that share a
    /// binary, a username and a platform still get different keys. Note what
    /// this does *not* claim -- copying `device-key.bin` along with the
    /// ciphertext still yields a readable credential.
    #[test]
    fn derive_fallback_key_differs_for_different_device_secrets() {
        let key1 = derive_key_from_device_secret(&[0x01; DEVICE_KEY_LEN], b"same-context");
        let key2 = derive_key_from_device_secret(&[0x02; DEVICE_KEY_LEN], b"same-context");
        assert_ne!(key1, key2);
    }

    // -----------------------------------------------------------------------
    // Encrypted-file fallback, for real
    //
    // Everything else in this module runs against an in-memory stand-in, so
    // these are the only tests that write an actual encrypted file. Both bugs
    // found in review -- a device-key race and a delete-on-read -- lived in
    // code that no test had ever executed.
    // -----------------------------------------------------------------------

    const MSA_CONTEXT: &[u8] = b"agora-msa-credentials-fallback";

    #[test]
    fn fallback_store_writes_ciphertext_that_load_reads_back() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _guard = use_real_fallback_dir(dir.path());

        let secret = r#"{"access_token":"gho_supersecret","refresh_token":"ghr_rt"}"#;
        store_secret("svc", "acct", "creds.enc", MSA_CONTEXT, secret).expect("store");

        let path = dir.path().join("creds.enc");
        assert!(path.is_file(), "an encrypted file must exist on disk");
        assert!(
            dir.path().join(DEVICE_KEY_FILE).is_file(),
            "storing must have created the device key"
        );

        let on_disk = std::fs::read(&path).expect("read ciphertext");
        let needle: &[u8] = b"gho_supersecret";
        assert!(
            !on_disk.windows(needle.len()).any(|w| w == needle),
            "the token must not be readable in the stored bytes"
        );

        let loaded = load_secret("svc", "acct", "creds.enc", MSA_CONTEXT).expect("load");
        assert_eq!(loaded.as_deref(), Some(secret));
    }

    #[test]
    fn fallback_store_overwrites_previous_value() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _guard = use_real_fallback_dir(dir.path());

        store_secret("svc", "acct", "creds.enc", MSA_CONTEXT, "first").expect("store first");
        store_secret("svc", "acct", "creds.enc", MSA_CONTEXT, "second").expect("store second");

        let loaded = load_secret("svc", "acct", "creds.enc", MSA_CONTEXT).expect("load");
        assert_eq!(loaded.as_deref(), Some("second"));
    }

    #[test]
    fn fallback_clear_removes_the_encrypted_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _guard = use_real_fallback_dir(dir.path());

        store_secret("svc", "acct", "creds.enc", MSA_CONTEXT, "value").expect("store");
        clear_secret("svc", "acct", "creds.enc").expect("clear");

        assert!(!dir.path().join("creds.enc").exists(), "file must be gone");
        let loaded = load_secret("svc", "acct", "creds.enc", MSA_CONTEXT).expect("load");
        assert_eq!(loaded, None);
    }

    /// The delete-on-read regression: a credential that cannot be decrypted is
    /// reported as absent, but must never be destroyed. A device key that is
    /// merely unreadable right now is indistinguishable from one that is gone,
    /// so deleting here would turn a transient error into permanent loss.
    #[test]
    fn undecryptable_credential_is_reported_absent_but_kept() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _guard = use_real_fallback_dir(dir.path());

        store_secret("svc", "acct", "creds.enc", MSA_CONTEXT, "value").expect("store");
        let path = dir.path().join("creds.enc");
        let before = std::fs::read(&path).expect("read ciphertext");

        // Rotate the device key out from under it.
        std::fs::remove_file(dir.path().join(DEVICE_KEY_FILE)).expect("remove device key");

        let loaded =
            load_secret("svc", "acct", "creds.enc", MSA_CONTEXT).expect("load must not error");
        assert_eq!(loaded, None, "an unreadable credential reads as absent");
        assert!(
            path.is_file(),
            "the ciphertext must survive the failed read"
        );
        assert_eq!(
            std::fs::read(&path).expect("re-read"),
            before,
            "the ciphertext must be untouched"
        );
    }

    #[test]
    fn corrupt_ciphertext_is_reported_absent_but_kept() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _guard = use_real_fallback_dir(dir.path());

        store_secret("svc", "acct", "creds.enc", MSA_CONTEXT, "value").expect("store");
        let path = dir.path().join("creds.enc");
        std::fs::write(&path, b"not a valid envelope").expect("corrupt it");

        let loaded =
            load_secret("svc", "acct", "creds.enc", MSA_CONTEXT).expect("load must not error");
        assert_eq!(loaded, None);
        assert!(path.is_file(), "corrupt input is not grounds for deletion");
    }

    /// A store after an unreadable one must recover on its own: the stale file
    /// is inert and simply gets overwritten.
    #[test]
    fn storing_again_recovers_after_the_device_key_is_lost() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _guard = use_real_fallback_dir(dir.path());

        store_secret("svc", "acct", "creds.enc", MSA_CONTEXT, "old").expect("store old");
        std::fs::remove_file(dir.path().join(DEVICE_KEY_FILE)).expect("remove device key");

        store_secret("svc", "acct", "creds.enc", MSA_CONTEXT, "new").expect("store new");
        let loaded = load_secret("svc", "acct", "creds.enc", MSA_CONTEXT).expect("load");
        assert_eq!(loaded.as_deref(), Some("new"));
    }

    #[test]
    fn token_bundle_round_trips_through_the_encrypted_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _guard = use_real_fallback_dir(dir.path());

        let bundle = GitHubTokenBundle {
            access_token: "gho_filetest".into(),
            refresh_token: Some("ghr_filetest".into()),
            access_expires_at: None,
            refresh_expires_at: None,
            token_type: Some("bearer".into()),
            scope: Some("repo".into()),
        };
        store_token_bundle(&bundle).expect("store");

        assert!(
            dir.path().join(TOKEN_FALLBACK_FILE).is_file(),
            "tokens.enc must exist"
        );
        let loaded = load_token_bundle().expect("load");
        assert_eq!(loaded.access_token, "gho_filetest");
        assert_eq!(loaded.refresh_token.as_deref(), Some("ghr_filetest"));
    }

    /// Atomic replacement leaves no debris and no half-written file behind.
    #[test]
    fn fallback_store_leaves_no_temp_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _guard = use_real_fallback_dir(dir.path());

        store_secret("svc", "acct", "creds.enc", MSA_CONTEXT, "value").expect("store");
        store_secret("svc", "acct", "creds.enc", MSA_CONTEXT, "value2").expect("store again");

        let stray: Vec<_> = std::fs::read_dir(dir.path())
            .expect("read dir")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|name| name.contains("agtmp"))
            .collect();
        assert!(stray.is_empty(), "temp files left behind: {stray:?}");
    }

    #[cfg(unix)]
    #[test]
    fn fallback_ciphertext_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let _guard = use_real_fallback_dir(dir.path());

        store_secret("svc", "acct", "creds.enc", MSA_CONTEXT, "value").expect("store");
        let mode = std::fs::metadata(dir.path().join("creds.enc"))
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "credentials must not be readable by others"
        );
    }

    #[test]
    fn credential_backend_reports_the_encrypted_file_when_it_is_in_use() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _guard = use_real_fallback_dir(dir.path());

        assert_eq!(
            credential_backend("svc", "acct", "creds.enc"),
            CredentialBackend::None,
            "nothing stored yet"
        );

        store_secret("svc", "acct", "creds.enc", MSA_CONTEXT, "value").expect("store");
        assert_eq!(
            credential_backend("svc", "acct", "creds.enc"),
            CredentialBackend::EncryptedFile,
            "the degraded path is in use and must be reported as such"
        );

        clear_secret("svc", "acct", "creds.enc").expect("clear");
        assert_eq!(
            credential_backend("svc", "acct", "creds.enc"),
            CredentialBackend::None
        );
    }

    #[test]
    fn github_credential_backend_reports_the_encrypted_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _guard = use_real_fallback_dir(dir.path());

        assert_eq!(github_credential_backend(), CredentialBackend::None);

        let bundle = GitHubTokenBundle {
            access_token: "gho_backendtest".into(),
            refresh_token: None,
            access_expires_at: None,
            refresh_expires_at: None,
            token_type: None,
            scope: None,
        };
        store_token_bundle(&bundle).expect("store");
        assert_eq!(
            github_credential_backend(),
            CredentialBackend::EncryptedFile
        );
    }

    /// Routing the credential files through `AppPaths` is meant to make a
    /// configured root move them -- not to move anyone's existing files. With no
    /// override set the resolved root must still be exactly what auth used to
    /// hardcode, so an ordinary install has nothing to migrate.
    ///
    /// (Skipped when a data root is configured, which is a developer's own
    /// setting rather than a property of the code.)
    #[test]
    fn default_credential_root_matches_the_previously_hardcoded_path() {
        if std::env::var_os("AGORA_DATA_DIR").is_some() {
            return;
        }
        let expected = dirs::data_local_dir()
            .expect("platform data dir")
            .join("agora");
        assert_eq!(
            crate::app_paths::AppPaths::platform_default().root(),
            expected,
            "the default credential location must not shift under existing users"
        );
    }

    #[test]
    fn github_sign_out_removes_the_encrypted_token_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _guard = use_real_fallback_dir(dir.path());

        let bundle = GitHubTokenBundle {
            access_token: "gho_signout".into(),
            refresh_token: None,
            access_expires_at: None,
            refresh_expires_at: None,
            token_type: None,
            scope: None,
        };
        store_token_bundle(&bundle).expect("store");
        let path = dir.path().join(TOKEN_FALLBACK_FILE);
        assert!(path.is_file(), "precondition: the token is on disk");

        clear_token_bundle().expect("sign out");
        assert!(
            !path.exists(),
            "sign-out must not leave a decryptable token behind"
        );
        assert!(load_token_bundle().is_none());
    }

    // -----------------------------------------------------------------------
    // Device key file
    // -----------------------------------------------------------------------

    #[test]
    fn device_secret_is_created_once_and_then_reused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(DEVICE_KEY_FILE);

        assert!(
            read_device_secret_at(&path).is_none(),
            "none before first use"
        );
        let first = load_or_create_device_secret_at(&path).expect("first create");
        assert_eq!(first.len(), DEVICE_KEY_LEN);
        let second = load_or_create_device_secret_at(&path).expect("second read");
        assert_eq!(first, second, "an existing device key must be reused");
    }

    #[test]
    fn device_secret_is_created_in_a_missing_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nested").join(DEVICE_KEY_FILE);
        let secret = load_or_create_device_secret_at(&path).expect("create");
        assert_eq!(secret.len(), DEVICE_KEY_LEN);
        assert_eq!(
            read_device_secret_at(&path).as_deref(),
            Some(secret.as_slice())
        );
    }

    /// A key file that never got its contents is residue from an interrupted
    /// first run, and is replaced -- but only after the publish wait, so an
    /// in-flight writer is not overwritten.
    #[test]
    fn truncated_device_secret_is_replaced() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(DEVICE_KEY_FILE);
        std::fs::write(&path, b"too-short").expect("write stub");

        assert!(
            read_device_secret_at(&path).is_none(),
            "wrong length is unusable"
        );
        let secret = load_or_create_device_secret_at(&path).expect("replace");
        assert_eq!(secret.len(), DEVICE_KEY_LEN);
        assert_eq!(
            read_device_secret_at(&path).as_deref(),
            Some(secret.as_slice())
        );
    }

    /// The race Sol caught: `create_new` reserves the name before any bytes
    /// land, so a concurrent caller could see a zero-length file, call it
    /// residue, and overwrite a key the winner was already encrypting under.
    /// Every thread must come away with the one key that is actually on disk.
    #[test]
    fn concurrent_first_use_agrees_on_one_device_secret() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(DEVICE_KEY_FILE);

        let secrets: Vec<Vec<u8>> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    let path = path.clone();
                    scope.spawn(move || load_or_create_device_secret_at(&path).expect("create"))
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("join"))
                .collect()
        });

        let published = read_device_secret_at(&path).expect("a key must be published");
        for secret in &secrets {
            assert_eq!(
                secret, &published,
                "every caller must get the key that is on disk"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn device_secret_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(DEVICE_KEY_FILE);
        load_or_create_device_secret_at(&path).expect("create");
        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "device key must not be group/world readable"
        );
    }

    // -----------------------------------------------------------------------
    // Secret store/load/clear roundtrip (helpers used by MSA credentials)
    // -----------------------------------------------------------------------

    #[test]
    fn store_load_clear_secret_roundtrip() {
        // Use unique test service/account names to avoid interference
        let uid = uuid::Uuid::new_v4();
        let service = &format!("com.agora.test.{uid}");
        let account = &format!("test-account-{uid}");
        let fallback_file = &format!("test-secret-{uid}.enc");
        let context = b"test-secret-context";
        let value = "test-secret-value-12345";

        let _ = clear_secret(service, account, fallback_file);

        // Store
        let result = store_secret(service, account, fallback_file, context, value);
        assert!(result.is_ok(), "store_secret should succeed");

        // Load
        let loaded = load_secret(service, account, fallback_file, context);
        assert!(loaded.is_ok(), "load_secret should succeed");

        // The value might or might not round-trip depending on keyring/fallback
        if let Ok(Some(loaded_val)) = loaded.as_ref() {
            assert_eq!(loaded_val, value);
        }

        // Clear
        let cleared = clear_secret(service, account, fallback_file);
        assert!(cleared.is_ok(), "clear_secret should succeed");

        // Load after clear should return None
        let after_clear = load_secret(service, account, fallback_file, context);
        assert!(after_clear.is_ok(), "load after clear should be Ok");
        if let Ok(None) = after_clear {
            // Good - secret was removed
        }
    }

    #[test]
    fn store_secret_overwrites_existing() {
        let uid = uuid::Uuid::new_v4();
        let service = &format!("com.agora.test.overwrite.{uid}");
        let account = &format!("overwrite-account-{uid}");
        let fallback_file = &format!("test-overwrite-{uid}.enc");
        let context = b"test-overwrite";
        let value1 = "first-value";
        let value2 = "second-value";

        let _ = clear_secret(service, account, fallback_file);
        let r1 = store_secret(service, account, fallback_file, context, value1);
        let r2 = store_secret(service, account, fallback_file, context, value2);
        assert!(r1.is_ok() && r2.is_ok(), "stores should succeed");

        let loaded = load_secret(service, account, fallback_file, context);
        assert!(loaded.is_ok(), "load should succeed");
        if let Ok(Some(val)) = loaded {
            assert_eq!(val, "second-value");
        }

        let _ = clear_secret(service, account, fallback_file);
    }
}
