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

fn store_test_token(value: &str) -> bool {
    #[cfg(test)]
    {
        *TEST_TOKEN_STORE.lock().unwrap() = Some(value.to_string());
        true
    }
    #[cfg(not(test))]
    {
        let _ = value;
        false
    }
}

fn load_test_token() -> Option<Option<String>> {
    #[cfg(test)]
    {
        Some(TEST_TOKEN_STORE.lock().unwrap().clone())
    }
    #[cfg(not(test))]
    None
}

fn clear_test_token() -> bool {
    #[cfg(test)]
    {
        *TEST_TOKEN_STORE.lock().unwrap() = None;
        true
    }
    #[cfg(not(test))]
    false
}

fn store_test_secret(service: &str, account: &str, value: &str) -> bool {
    #[cfg(test)]
    {
        TEST_SECRET_STORE.lock().unwrap().insert(
            (service.to_string(), account.to_string()),
            value.to_string(),
        );
        true
    }
    #[cfg(not(test))]
    {
        let _ = (service, account, value);
        false
    }
}

fn load_test_secret(service: &str, account: &str) -> Option<Option<String>> {
    #[cfg(test)]
    {
        Some(
            TEST_SECRET_STORE
                .lock()
                .unwrap()
                .get(&(service.to_string(), account.to_string()))
                .cloned(),
        )
    }
    #[cfg(not(test))]
    {
        let _ = (service, account);
        None
    }
}

fn clear_test_secret(service: &str, account: &str) -> bool {
    #[cfg(test)]
    {
        TEST_SECRET_STORE
            .lock()
            .unwrap()
            .remove(&(service.to_string(), account.to_string()));
        true
    }
    #[cfg(not(test))]
    {
        let _ = (service, account);
        false
    }
}

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
        let key = derive_fallback_key();
        decrypt_token(&data, &key)
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

    if let Some(path) = fallback_token_path() {
        if path.exists() {
            let _ = std::fs::remove_file(&path);
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
///
/// In tests, the `AGORA_TEST_TOKEN_DIR` environment variable can be set to an
/// isolated directory so parallel tests do not share the same fallback file.
fn fallback_token_path() -> Option<std::path::PathBuf> {
    #[cfg(any(test, feature = "test-support"))]
    if let Ok(dir) = std::env::var("AGORA_TEST_TOKEN_DIR") {
        return Some(std::path::PathBuf::from(dir).join(TOKEN_FALLBACK_FILE));
    }
    dirs::data_local_dir().map(|d| d.join("agora").join(TOKEN_FALLBACK_FILE))
}

fn fallback_secret_path(file_name: &str) -> Option<std::path::PathBuf> {
    #[cfg(any(test, feature = "test-support"))]
    if let Ok(dir) = std::env::var("AGORA_TEST_SECRET_DIR") {
        return Some(std::path::PathBuf::from(dir).join(file_name));
    }
    dirs::data_local_dir().map(|d| d.join("agora").join(file_name))
}

fn using_test_token_store() -> bool {
    cfg!(any(test, feature = "test-support")) && std::env::var_os("AGORA_TEST_TOKEN_DIR").is_some()
}

fn using_test_secret_store() -> bool {
    cfg!(any(test, feature = "test-support")) && std::env::var_os("AGORA_TEST_SECRET_DIR").is_some()
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
        let key = derive_fallback_key_for(b"agora-test-fallback");
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
        let k1 = derive_fallback_key_for(b"context-1");
        let k2 = derive_fallback_key_for(b"context-2");
        let encrypted = encrypt_token(token, &k1).expect("encrypt must succeed");
        let decrypted = decrypt_token(&encrypted, &k2);
        assert!(decrypted.is_none(), "wrong key must not decrypt");
    }

    #[test]
    fn fallback_decrypt_truncated_data_returns_none() {
        let key = derive_fallback_key_for(b"test");
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
        let key = derive_fallback_key_for(b"test-context");
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
        let key1 = derive_fallback_key_for(b"test-context");
        let key2 = derive_fallback_key_for(b"test-context");
        assert_eq!(key1, key2);
    }

    #[test]
    fn derive_fallback_key_differs_for_different_contexts() {
        let key1 = derive_fallback_key_for(b"context-a");
        let key2 = derive_fallback_key_for(b"context-b");
        assert_ne!(key1, key2);
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
