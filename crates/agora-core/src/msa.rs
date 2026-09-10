//! Microsoft Account (MSA) authentication for direct Minecraft launching.
//!
//! Agora authenticates through **its own Entra public-client application** using
//! the OAuth 2.0 device authorization grant (RFC 8628) against the `consumers`
//! authority, which is the tenant Microsoft requires for the `XboxLive.signin`
//! scope. There is no client secret: a public client must not ship one, and the
//! device-code grant does not need one.
//!
//! The flow (see `docs/architecture/layer-ownership.md` for who owns what):
//!   1. Request a device code (`/devicecode`) — user code + verification URI.
//!   2. Poll `/token` until the user approves, declines, or the code expires.
//!   3. Xbox Live user token   (`user.auth.xboxlive.com`, `RpsTicket: d=<token>`)
//!   4. XSTS token            (relying party `rp://api.minecraftservices.com/`)
//!   5. Minecraft token       (`/authentication/login_with_xbox`, `identityToken`)
//!   6. Entitlements          (`/entitlements/mcstore`, response body validated)
//!   7. Java profile          (`/minecraft/profile`)
//!
//! This is a security-critical module. NO device code, token, or raw
//! authentication response body is ever logged, returned across an adapter
//! boundary, or persisted outside the OS keyring (+ aes-gcm fallback).
//!
//! Microsoft, Xbox and Minecraft tokens are deliberately kept distinct: only the
//! *Minecraft* access token (with the expiry Minecraft itself returned) is
//! handed to the game, and only the *Microsoft* refresh token is persisted for
//! renewal.

use crate::db;
use crate::error::{LauncherError, LauncherResult};
use crate::http_client::{self, ClientCategory, HttpClients};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};

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

fn check_msa_network_enabled(db_path: &Path) -> LauncherResult<()> {
    check_network_enabled(
        db_path,
        "network_msa_enabled",
        "Microsoft account login is disabled in Privacy settings.",
    )
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Agora's own Entra (Azure AD) public-client application ID.
///
/// Public by design — a public client has no secret, and the ID appears in every
/// device-code request. There is deliberately no fallback to the official
/// Minecraft launcher's client ID.
pub const AGORA_MSA_CLIENT_ID: &str = "2dbd8051-d5dd-4611-bcae-b38400c1dc30";

/// Version of the authentication flow that issued a stored credential.
///
/// Bumped when the client ID or the token chain changes in a way that makes
/// previously stored refresh tokens unusable. Version 1 was the launcher-ID
/// SISU/device-signing flow; version 2 is Agora's own device-code application.
pub const MSA_AUTH_VERSION: u32 = 2;

/// Xbox sign-in plus a refresh token for silent renewal.
const MSA_SCOPE: &str = "XboxLive.signin offline_access";

// The `consumers` authority: personal Microsoft accounts. `common` and tenant
// authorities reject the XboxLive.signin scope.
const DEVICE_CODE_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const TOKEN_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

const XBL_AUTH_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS_AUTHORIZE_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MC_LOGIN_URL: &str = "https://api.minecraftservices.com/authentication/login_with_xbox";
const MC_ENTITLEMENTS_URL: &str = "https://api.minecraftservices.com/entitlements/mcstore";
const MC_PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";

const USER_AGENT: &str = "Agora Launcher";

/// RFC 8628 default when the server omits `interval`.
const DEFAULT_POLL_INTERVAL_SECS: u64 = 5;
/// RFC 8628 says a `slow_down` adds 5 seconds to the interval.
const SLOW_DOWN_INCREMENT_SECS: u64 = 5;
/// Never poll faster than this, whatever the server says.
const MIN_POLL_INTERVAL_SECS: u64 = 1;
/// Upper bound so a hostile or broken `interval` cannot stall the flow forever.
const MAX_POLL_INTERVAL_SECS: u64 = 60;

/// Fallback when Minecraft omits `expires_in` (it normally returns 86400).
const DEFAULT_MC_TOKEN_LIFETIME_SECS: u64 = 86_400;
/// Upper bound on a Minecraft token lifetime we are willing to trust.
const MAX_MC_TOKEN_LIFETIME_SECS: u64 = 7 * 86_400;

/// Entitlement names that grant Minecraft: Java Edition.
///
/// `product_minecraft` + `game_minecraft` is a normal purchase; the Game Pass
/// products cover subscription access. Bedrock-only entitlements
/// (`*_minecraft_bedrock`) deliberately do not appear here.
const JAVA_ENTITLEMENTS: &[&str] = &[
    "product_minecraft",
    "game_minecraft",
    "product_game_pass_pc",
    "product_game_pass_ultimate",
];

// Keyring storage
const KEYRING_SERVICE: &str = "com.agoramc";
const KEYRING_ACCOUNT: &str = "msa-credentials";
const CREDENTIALS_FALLBACK_FILE: &str = "msa-credentials.enc";
const CREDENTIALS_KEY_CONTEXT: &[u8] = b"agora-msa-credentials-fallback";

/// Message shown when a credential from the pre-migration flow is found.
pub const LEGACY_CREDENTIALS_MESSAGE: &str =
    "Agora now signs in with its own Microsoft application, so your previous \
     session cannot be renewed. Sign in once more to continue using direct \
     launch — your instances and other accounts are untouched.";

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Persisted Minecraft credentials stored in the OS keyring.
///
/// `access_token` is the **Minecraft** token (used to launch the game, expiring
/// at `expires`); `refresh_token` is the **Microsoft** refresh token used to
/// re-run the whole chain. The two are never interchanged.
#[derive(Clone, Serialize, Deserialize)]
pub struct MsaCredentials {
    pub username: String,
    pub uuid: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires: DateTime<Utc>,
    /// Entra application that issued `refresh_token`. Empty for pre-migration
    /// credentials, which no other client ID can refresh.
    #[serde(default)]
    pub client_id: String,
    /// Flow version that produced this credential; 0 for pre-migration.
    #[serde(default)]
    pub auth_version: u32,
}

impl std::fmt::Debug for MsaCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MsaCredentials")
            .field("username", &self.username)
            .field("uuid", &self.uuid)
            .field("access_token", &"[REDACTED]")
            .field("refresh_token", &"[REDACTED]")
            .field("expires", &self.expires)
            .field("client_id", &self.client_id)
            .field("auth_version", &self.auth_version)
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

    /// Whether this credential was issued by a different application than the
    /// one Agora now authenticates with.
    ///
    /// Such a refresh token is only valid for the client that obtained it, so
    /// attempting to renew it under the new application would fail with an
    /// opaque `invalid_grant`. It requires an interactive sign-in instead.
    pub fn needs_reauth(&self) -> bool {
        self.auth_version < MSA_AUTH_VERSION || self.client_id != AGORA_MSA_CLIENT_ID
    }
}

/// A pending device-code login.
///
/// The caller shows `user_code` and `verification_uri` to the user and polls
/// with [`poll_login`]. The `device_code` itself stays private to this module —
/// it is a bearer credential and never crosses an adapter boundary.
#[derive(Clone)]
pub struct MsaDeviceCodeFlow {
    device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_at: DateTime<Utc>,
    pub interval_secs: u64,
}

impl std::fmt::Debug for MsaDeviceCodeFlow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MsaDeviceCodeFlow")
            .field("device_code", &"[REDACTED]")
            .field("user_code", &"[REDACTED]")
            .field("verification_uri", &self.verification_uri)
            .field("expires_at", &self.expires_at)
            .field("interval_secs", &self.interval_secs)
            .finish()
    }
}

/// Cooperative cancellation for an in-flight [`poll_login`].
#[derive(Clone, Debug, Default)]
pub struct MsaLoginCancel(Arc<AtomicBool>);

impl MsaLoginCancel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask the polling loop to stop. Takes effect within a second.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    /// Whether both handles control the same login.
    ///
    /// Lets an adapter tell "my sign-in finished" from "a newer sign-in
    /// replaced mine", so a superseded attempt cannot clear the state of the
    /// one that replaced it.
    pub fn is_same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

// ---------------------------------------------------------------------------
// Error constructors — one place per user-visible authentication outcome
// ---------------------------------------------------------------------------

fn err(code: &str, message: impl Into<String>) -> LauncherError {
    LauncherError::Generic {
        code: code.into(),
        message: message.into(),
    }
}

fn login_cancelled() -> LauncherError {
    err(
        "ERR_MSA_LOGIN_CANCELLED",
        "Microsoft sign-in was cancelled before it completed.",
    )
}

fn login_expired() -> LauncherError {
    err(
        "ERR_MSA_LOGIN_EXPIRED",
        "The sign-in code expired before it was entered. Start the sign-in again to get a new code.",
    )
}

fn login_declined() -> LauncherError {
    err(
        "ERR_MSA_LOGIN_DECLINED",
        "The sign-in request was declined in the browser.",
    )
}

fn app_access_denied() -> LauncherError {
    err(
        "ERR_MSA_APP_ACCESS_DENIED",
        "Microsoft refused this application's sign-in request. Agora's Microsoft \
         application may not yet be approved for the Minecraft API.",
    )
}

// ---------------------------------------------------------------------------
// Step 1: request a device code
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    #[serde(default)]
    interval: Option<u64>,
}

fn clamp_interval(interval: u64) -> u64 {
    interval.clamp(MIN_POLL_INTERVAL_SECS, MAX_POLL_INTERVAL_SECS)
}

/// Begin the device-code login. Returns the user code and verification URI to
/// display; nothing secret is included in the public fields.
///
/// `db_path` is the path to `local_state.db` — the caller must provide the
/// correct path for the running binary (desktop app vs CLI).
pub async fn begin_login(
    clients: &HttpClients,
    db_path: &Path,
) -> LauncherResult<MsaDeviceCodeFlow> {
    check_msa_network_enabled(db_path)?;

    let response = http_client::checked_post_form(
        clients,
        ClientCategory::Microsoft,
        DEVICE_CODE_URL,
        &[("client_id", AGORA_MSA_CLIENT_ID), ("scope", MSA_SCOPE)],
        &[("Accept".into(), "application/json".into())],
    )
    .await?;

    let status = response.status();
    let body = http_client::checked_response_text(response, ClientCategory::Microsoft)
        .await
        .unwrap_or_default();

    if !status.is_success() {
        if let Some(oauth_error) = oauth_error_code(&body) {
            if is_app_access_error(&oauth_error) {
                return Err(app_access_denied());
            }
        }
        return Err(err(
            "ERR_MSA_DEVICE_CODE_HTTP",
            format!("Microsoft rejected the sign-in request (HTTP {status})."),
        ));
    }

    let parsed: DeviceCodeResponse = serde_json::from_str(&body).map_err(|_| {
        err(
            "ERR_MSA_DEVICE_CODE_PARSE",
            "Could not read Microsoft's sign-in response.",
        )
    })?;

    Ok(MsaDeviceCodeFlow {
        device_code: parsed.device_code,
        user_code: parsed.user_code,
        verification_uri: parsed.verification_uri,
        expires_at: Utc::now() + chrono::Duration::seconds(parsed.expires_in as i64),
        interval_secs: clamp_interval(parsed.interval.unwrap_or(DEFAULT_POLL_INTERVAL_SECS)),
    })
}

// ---------------------------------------------------------------------------
// Step 2: poll for the token
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(crate) struct OAuthToken {
    pub(crate) access_token: String,
    pub(crate) refresh_token: String,
}

impl std::fmt::Debug for OAuthToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthToken")
            .field("access_token", &"[REDACTED]")
            .field("refresh_token", &"[REDACTED]")
            .finish()
    }
}

/// One poll result, already classified. Carries no response body.
#[derive(Debug)]
pub(crate) enum DevicePoll {
    Complete(OAuthToken),
    /// User has not finished signing in yet.
    Pending,
    /// Server asked us to back off; the interval grows before the next attempt.
    SlowDown,
    Declined,
    Expired,
    /// Terminal failure; polling must stop.
    Fatal(LauncherError),
}

/// Extract only the OAuth `error` slug. `error_description` is deliberately
/// discarded: it is a raw authentication response field and can echo account
/// details into logs and UI.
fn oauth_error_code(body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()?
        .get("error")?
        .as_str()
        .map(str::to_owned)
}

/// Errors that mean "this application may not sign this user in", as opposed to
/// a problem with the user's own credentials.
fn is_app_access_error(code: &str) -> bool {
    matches!(
        code,
        "unauthorized_client" | "invalid_client" | "invalid_scope" | "access_denied"
    )
}

/// Classify a `/token` response without retaining any of its body.
pub(crate) fn classify_device_poll(status: reqwest::StatusCode, body: &str) -> DevicePoll {
    if status.is_success() {
        return match serde_json::from_str::<OAuthToken>(body) {
            Ok(token) => DevicePoll::Complete(token),
            Err(_) => DevicePoll::Fatal(err(
                "ERR_MSA_TOKEN_PARSE",
                "Could not read Microsoft's sign-in response.",
            )),
        };
    }

    match oauth_error_code(body).as_deref() {
        Some("authorization_pending") => DevicePoll::Pending,
        Some("slow_down") => DevicePoll::SlowDown,
        Some("authorization_declined") => DevicePoll::Declined,
        // `access_denied` on the device-code grant is the user declining in the
        // browser; the application-access variants are reported at /devicecode.
        Some("access_denied") => DevicePoll::Declined,
        Some("expired_token") | Some("code_expired") => DevicePoll::Expired,
        Some("bad_verification_code") => DevicePoll::Fatal(err(
            "ERR_MSA_DEVICE_CODE_INVALID",
            "Microsoft rejected this sign-in code. Start the sign-in again.",
        )),
        Some(code) if is_app_access_error(code) => DevicePoll::Fatal(app_access_denied()),
        Some(code) => DevicePoll::Fatal(err(
            "ERR_MSA_TOKEN_HTTP",
            format!("Microsoft sign-in failed ({code})."),
        )),
        None if status.is_server_error() => DevicePoll::Pending,
        None => DevicePoll::Fatal(err(
            "ERR_MSA_TOKEN_HTTP",
            format!("Microsoft sign-in failed (HTTP {status})."),
        )),
    }
}

/// Injectable seam for the polling loop so timing and outcomes are testable
/// without a network or real clock.
#[async_trait::async_trait]
pub(crate) trait DeviceTokenPoller: Send + Sync {
    async fn poll_once(&self) -> LauncherResult<DevicePoll>;
    async fn sleep(&self, secs: u64);
    fn now(&self) -> DateTime<Utc>;
}

/// A network error worth retrying while the device code is still alive.
///
/// Everything else — lockdown, a disabled endpoint, a missing gate, an HTTP
/// policy refusal — is a decision, not a blip, so polling stops immediately
/// rather than hammering an endpoint the user has switched off.
fn is_transient_poll_error(error: &LauncherError) -> bool {
    match error {
        LauncherError::NetworkOffline => true,
        LauncherError::Generic { code, .. } => code == "ERR_NETWORK",
        _ => false,
    }
}

/// The polling loop: respects the server interval, backs off on `slow_down`,
/// stops on cancellation, expiry, denial, and terminal errors.
pub(crate) async fn run_device_poll(
    poller: &dyn DeviceTokenPoller,
    interval_secs: u64,
    expires_at: DateTime<Utc>,
    cancel: &MsaLoginCancel,
) -> LauncherResult<OAuthToken> {
    let mut interval = clamp_interval(interval_secs);

    loop {
        if cancel.is_cancelled() {
            return Err(login_cancelled());
        }
        if poller.now() >= expires_at {
            return Err(login_expired());
        }

        match poller.poll_once().await {
            Ok(DevicePoll::Complete(token)) => return Ok(token),
            Ok(DevicePoll::Pending) => {}
            Ok(DevicePoll::SlowDown) => {
                interval = clamp_interval(interval.saturating_add(SLOW_DOWN_INCREMENT_SECS));
            }
            Ok(DevicePoll::Declined) => return Err(login_declined()),
            Ok(DevicePoll::Expired) => return Err(login_expired()),
            Ok(DevicePoll::Fatal(error)) => return Err(error),
            Err(error) if is_transient_poll_error(&error) => {}
            Err(error) => return Err(error),
        }

        // Wait out the interval a second at a time so cancellation is prompt
        // and an expired code is noticed without an extra request.
        for _ in 0..interval {
            if cancel.is_cancelled() {
                return Err(login_cancelled());
            }
            if poller.now() >= expires_at {
                return Err(login_expired());
            }
            poller.sleep(1).await;
        }
    }
}

/// Production poller: one checked HTTP request per attempt, re-checking the
/// Microsoft-authentication preference every time so switching it off mid-flow
/// stops the polling.
struct LiveDeviceTokenPoller<'a> {
    clients: &'a HttpClients,
    db_path: &'a Path,
    device_code: &'a str,
}

#[async_trait::async_trait]
impl DeviceTokenPoller for LiveDeviceTokenPoller<'_> {
    async fn poll_once(&self) -> LauncherResult<DevicePoll> {
        check_msa_network_enabled(self.db_path)?;
        let response = http_client::checked_post_form(
            self.clients,
            ClientCategory::Microsoft,
            TOKEN_URL,
            &[
                ("client_id", AGORA_MSA_CLIENT_ID),
                ("grant_type", DEVICE_CODE_GRANT),
                ("device_code", self.device_code),
            ],
            &[("Accept".into(), "application/json".into())],
        )
        .await?;

        let status = response.status();
        let body = http_client::checked_response_text(response, ClientCategory::Microsoft)
            .await
            .unwrap_or_default();
        Ok(classify_device_poll(status, &body))
    }

    async fn sleep(&self, secs: u64) {
        tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
    }

    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// Poll until the user completes sign-in, then run the Minecraft chain and
/// persist the credentials.
///
/// Blocks for as long as the device code is valid; callers run it on a
/// background task and use `cancel` to stop it.
pub async fn poll_login(
    clients: &HttpClients,
    flow: &MsaDeviceCodeFlow,
    db_path: &Path,
    cancel: &MsaLoginCancel,
) -> LauncherResult<MsaCredentials> {
    check_msa_network_enabled(db_path)?;

    let poller = LiveDeviceTokenPoller {
        clients,
        db_path,
        device_code: &flow.device_code,
    };
    let oauth = run_device_poll(&poller, flow.interval_secs, flow.expires_at, cancel).await?;

    let credentials = run_minecraft_chain(clients, &oauth, true).await?;
    store_credentials(&credentials)?;
    Ok(credentials)
}

// ---------------------------------------------------------------------------
// Refresh
// ---------------------------------------------------------------------------

async fn refresh_oauth_token(
    clients: &HttpClients,
    refresh_token: &str,
) -> LauncherResult<OAuthToken> {
    let response = http_client::checked_post_form(
        clients,
        ClientCategory::Microsoft,
        TOKEN_URL,
        &[
            ("client_id", AGORA_MSA_CLIENT_ID),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("scope", MSA_SCOPE),
        ],
        &[("Accept".into(), "application/json".into())],
    )
    .await?;

    let status = response.status();
    let body = http_client::checked_response_text(response, ClientCategory::Microsoft)
        .await
        .unwrap_or_default();

    if !status.is_success() {
        if is_msa_permanent_refresh_error(&body) {
            return Err(LauncherError::MsaAuthRequired);
        }
        if let Some(code) = oauth_error_code(&body) {
            if is_app_access_error(&code) {
                return Err(app_access_denied());
            }
        }
        return Err(err(
            "ERR_MSA_OAUTH_REFRESH_HTTP",
            format!("Microsoft could not renew the session (HTTP {status})."),
        ));
    }

    serde_json::from_str::<OAuthToken>(&body).map_err(|_| {
        err(
            "ERR_MSA_OAUTH_REFRESH_PARSE",
            "Could not read Microsoft's session renewal response.",
        )
    })
}

/// Returns `true` when the OAuth error body signals a permanent failure
/// (`invalid_grant` or `expired_token`) that should clear stored credentials.
fn is_msa_permanent_refresh_error(body: &str) -> bool {
    matches!(
        oauth_error_code(body).as_deref(),
        Some("invalid_grant") | Some("expired_token")
    )
}

// ---------------------------------------------------------------------------
// Steps 3-4: Xbox Live user token → XSTS
// ---------------------------------------------------------------------------

/// Response shapes (PascalCase = Xbox Live convention)
mod xbox_types {
    use serde::Deserialize;
    use std::collections::HashMap;

    #[derive(Deserialize, Clone)]
    #[serde(rename_all = "PascalCase")]
    pub struct XboxToken {
        pub token: String,
        pub display_claims: HashMap<String, serde_json::Value>,
    }

    impl std::fmt::Debug for XboxToken {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("XboxToken")
                .field("token", &"[REDACTED]")
                .field("display_claims", &"[PRESENT]")
                .finish()
        }
    }

    #[derive(Deserialize, Clone)]
    pub struct MinecraftToken {
        pub access_token: String,
        #[serde(default)]
        pub expires_in: Option<u64>,
    }

    impl std::fmt::Debug for MinecraftToken {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("MinecraftToken")
                .field("access_token", &"[REDACTED]")
                .field("expires_in", &self.expires_in)
                .finish()
        }
    }

    #[derive(Deserialize, Debug, Clone)]
    pub struct MinecraftProfile {
        pub id: String,
        pub name: String,
    }

    #[derive(Deserialize, Debug, Clone, Default)]
    pub struct EntitlementsResponse {
        #[serde(default)]
        pub items: Vec<EntitlementItem>,
    }

    #[derive(Deserialize, Debug, Clone)]
    pub struct EntitlementItem {
        #[serde(default)]
        pub name: String,
    }
}

impl xbox_types::XboxToken {
    /// The user hash (`uhs`) that pairs with the token in `XBL3.0 x=<uhs>;<tok>`.
    fn user_hash(&self) -> Option<String> {
        self.display_claims
            .get("xui")?
            .get(0)?
            .get("uhs")?
            .as_str()
            .map(str::to_owned)
    }
}

/// Step 3: exchange the Microsoft access token for an Xbox Live user token.
async fn xbl_authenticate(
    clients: &HttpClients,
    ms_access_token: &str,
) -> LauncherResult<(String, String)> {
    let body = serde_json::json!({
        "Properties": {
            "AuthMethod": "RPS",
            "SiteName": "user.auth.xboxlive.com",
            // `d=` is the ticket form for an Azure/Entra OAuth access token.
            "RpsTicket": format!("d={ms_access_token}"),
        },
        "RelyingParty": "http://auth.xboxlive.com",
        "TokenType": "JWT"
    });

    let response = http_client::checked_post_json(
        clients,
        ClientCategory::Microsoft,
        XBL_AUTH_URL,
        &body,
        &[
            ("Accept".into(), "application/json".into()),
            ("User-Agent".into(), USER_AGENT.into()),
        ],
    )
    .await?;

    let status = response.status();
    let raw = http_client::checked_response_bytes(response, ClientCategory::Microsoft)
        .await
        .unwrap_or_default();

    if !status.is_success() {
        return Err(err(
            "ERR_MSA_XBOX_AUTH_HTTP",
            format!("Xbox Live rejected the sign-in (HTTP {status})."),
        ));
    }

    let token: xbox_types::XboxToken = serde_json::from_slice(&raw).map_err(|_| {
        err(
            "ERR_MSA_XBOX_AUTH_PARSE",
            "Could not read the Xbox Live sign-in response.",
        )
    })?;
    let uhs = token.user_hash().ok_or_else(|| {
        err(
            "ERR_MSA_NO_UHS",
            "The Xbox Live response did not contain a user hash.",
        )
    })?;
    Ok((uhs, token.token))
}

/// Map a documented XSTS `XErr` code to an actionable message.
pub(crate) fn xsts_error_for(xerr: u64) -> Option<LauncherError> {
    let (code, message) = match xerr {
        2148916227 => (
            "ERR_MSA_XBOX_BANNED",
            "This Xbox account has been banned from Xbox Live.",
        ),
        2148916233 => (
            "ERR_MSA_XBOX_NO_ACCOUNT",
            "This Microsoft account has no Xbox profile. Create one at xbox.com, then sign in again.",
        ),
        2148916235 => (
            "ERR_MSA_XBOX_REGION",
            "Xbox Live is not available in this account's country or region.",
        ),
        2148916236 | 2148916237 => (
            "ERR_MSA_XBOX_ADULT_VERIFICATION",
            "This account needs adult verification before it can use Xbox Live.",
        ),
        2148916238 => (
            "ERR_MSA_XBOX_CHILD_ACCOUNT",
            "This is a child account. An adult must add it to a Microsoft family group before it can sign in.",
        ),
        _ => return None,
    };
    Some(err(code, message))
}

fn parse_xerr(body: &str) -> Option<u64> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let xerr = value.get("XErr")?;
    xerr.as_u64()
        .or_else(|| xerr.as_str().and_then(|s| s.parse().ok()))
}

/// Step 4: exchange the Xbox user token for an XSTS token scoped to Minecraft.
async fn xsts_authorize(
    clients: &HttpClients,
    xbl_token: &str,
) -> LauncherResult<(String, String)> {
    let body = serde_json::json!({
        "Properties": {
            "SandboxId": "RETAIL",
            "UserTokens": [xbl_token],
        },
        "RelyingParty": "rp://api.minecraftservices.com/",
        "TokenType": "JWT"
    });

    let response = http_client::checked_post_json(
        clients,
        ClientCategory::Microsoft,
        XSTS_AUTHORIZE_URL,
        &body,
        &[
            ("Accept".into(), "application/json".into()),
            ("User-Agent".into(), USER_AGENT.into()),
        ],
    )
    .await?;

    let status = response.status();
    let raw = http_client::checked_response_text(response, ClientCategory::Microsoft)
        .await
        .unwrap_or_default();

    if !status.is_success() {
        if let Some(specific) = parse_xerr(&raw).and_then(xsts_error_for) {
            return Err(specific);
        }
        return Err(err(
            "ERR_MSA_XSTS_HTTP",
            format!("Xbox Live authorization failed (HTTP {status})."),
        ));
    }

    let token: xbox_types::XboxToken = serde_json::from_str(&raw).map_err(|_| {
        err(
            "ERR_MSA_XSTS_PARSE",
            "Could not read the Xbox Live authorization response.",
        )
    })?;
    let uhs = token.user_hash().ok_or_else(|| {
        err(
            "ERR_MSA_NO_UHS",
            "The Xbox Live authorization response did not contain a user hash.",
        )
    })?;
    Ok((uhs, token.token))
}

// ---------------------------------------------------------------------------
// Steps 5-7: Minecraft token → entitlements → profile
// ---------------------------------------------------------------------------

/// Step 5: trade the XSTS token for a Minecraft access token and its lifetime.
async fn minecraft_login(
    clients: &HttpClients,
    uhs: &str,
    xsts_token: &str,
) -> LauncherResult<(String, Option<u64>)> {
    let body = serde_json::json!({
        "identityToken": format!("XBL3.0 x={uhs};{xsts_token}"),
    });

    let response = http_client::checked_post_json(
        clients,
        ClientCategory::Microsoft,
        MC_LOGIN_URL,
        &body,
        &[
            ("Accept".into(), "application/json".into()),
            ("User-Agent".into(), USER_AGENT.into()),
        ],
    )
    .await?;

    let status = response.status();
    let raw = http_client::checked_response_bytes(response, ClientCategory::Microsoft)
        .await
        .unwrap_or_default();

    if status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::UNAUTHORIZED {
        // Minecraft Services rejects tokens from applications it has not
        // granted API access to, which is exactly what a pending approval
        // looks like from here.
        return Err(app_access_denied());
    }
    if !status.is_success() {
        return Err(err(
            "ERR_MSA_MC_TOKEN_HTTP",
            format!("Minecraft sign-in failed (HTTP {status})."),
        ));
    }

    let token: xbox_types::MinecraftToken = serde_json::from_slice(&raw).map_err(|_| {
        err(
            "ERR_MSA_MC_TOKEN_PARSE",
            "Could not read the Minecraft sign-in response.",
        )
    })?;
    Ok((token.access_token, token.expires_in))
}

/// Whether an entitlements payload actually grants Minecraft: Java Edition.
///
/// A 200 alone proves nothing: the endpoint answers successfully with an empty
/// `items` array for an account that does not own the game.
pub(crate) fn entitlements_grant_java(body: &str) -> bool {
    let parsed: xbox_types::EntitlementsResponse = match serde_json::from_str(body) {
        Ok(parsed) => parsed,
        Err(_) => return false,
    };
    parsed
        .items
        .iter()
        .any(|item| JAVA_ENTITLEMENTS.contains(&item.name.as_str()))
}

/// Step 6: confirm the account owns Minecraft: Java Edition.
async fn check_entitlements(clients: &HttpClients, mc_access_token: &str) -> LauncherResult<()> {
    let response = http_client::checked_request_with_headers(
        clients,
        ClientCategory::Microsoft,
        MC_ENTITLEMENTS_URL,
        vec![
            ("Authorization".into(), format!("Bearer {mc_access_token}")),
            ("Accept".into(), "application/json".into()),
            ("User-Agent".into(), USER_AGENT.into()),
        ],
    )
    .await?;

    let status = response.status();
    let raw = http_client::checked_response_text(response, ClientCategory::Microsoft)
        .await
        .unwrap_or_default();

    if !status.is_success() {
        return Err(err(
            "ERR_MSA_ENTITLEMENTS_HTTP",
            format!("Could not check Minecraft ownership (HTTP {status})."),
        ));
    }

    if !entitlements_grant_java(&raw) {
        return Err(err(
            "ERR_MSA_NO_ENTITLEMENT",
            "This Microsoft account does not own Minecraft: Java Edition. \
             Buy it at minecraft.net, or sign in with the account that owns it.",
        ));
    }
    Ok(())
}

/// Step 7: fetch the Java profile (username + UUID).
async fn minecraft_profile(
    clients: &HttpClients,
    mc_access_token: &str,
) -> LauncherResult<(String, String)> {
    let response = http_client::checked_request_with_headers(
        clients,
        ClientCategory::Microsoft,
        MC_PROFILE_URL,
        vec![
            ("Authorization".into(), format!("Bearer {mc_access_token}")),
            ("Accept".into(), "application/json".into()),
            ("User-Agent".into(), USER_AGENT.into()),
        ],
    )
    .await?;

    let status = response.status();
    let raw = http_client::checked_response_bytes(response, ClientCategory::Microsoft)
        .await
        .unwrap_or_default();

    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(err(
            "ERR_MSA_NO_PROFILE",
            "This account owns Minecraft: Java Edition but has no profile yet. \
             Create a Minecraft username at minecraft.net, then sign in again.",
        ));
    }
    if !status.is_success() {
        return Err(err(
            "ERR_MSA_PROFILE_HTTP",
            format!("Could not read the Minecraft profile (HTTP {status})."),
        ));
    }

    let profile: xbox_types::MinecraftProfile = serde_json::from_slice(&raw).map_err(|_| {
        err(
            "ERR_MSA_PROFILE_PARSE",
            "Could not read the Minecraft profile response.",
        )
    })?;
    Ok((profile.name, profile.id))
}

/// Run Xbox Live → XSTS → Minecraft for a freshly issued Microsoft token.
///
/// The profile is always re-read, so a renamed account stays correct across a
/// refresh as well as a fresh sign-in.
async fn run_minecraft_chain(
    clients: &HttpClients,
    oauth: &OAuthToken,
    verify_entitlements: bool,
) -> LauncherResult<MsaCredentials> {
    let (uhs, xbl_token) = xbl_authenticate(clients, &oauth.access_token).await?;
    let (xsts_uhs, xsts_token) = xsts_authorize(clients, &xbl_token).await?;
    // XSTS restates the user hash; prefer its copy and fall back to the Xbox one.
    let uhs = if xsts_uhs.is_empty() { uhs } else { xsts_uhs };

    let (mc_access_token, expires_in) = minecraft_login(clients, &uhs, &xsts_token).await?;

    if verify_entitlements {
        check_entitlements(clients, &mc_access_token).await?;
    }

    let (username, uuid) = minecraft_profile(clients, &mc_access_token).await?;

    // Minecraft's own lifetime governs the token we launch the game with.
    // Clamped so a nonsensical value cannot mask an expired token.
    let lifetime = expires_in
        .unwrap_or(DEFAULT_MC_TOKEN_LIFETIME_SECS)
        .clamp(1, MAX_MC_TOKEN_LIFETIME_SECS) as i64;

    Ok(MsaCredentials {
        username,
        uuid,
        access_token: mc_access_token,
        refresh_token: oauth.refresh_token.clone(),
        expires: Utc::now() + chrono::Duration::seconds(lifetime),
        client_id: AGORA_MSA_CLIENT_ID.to_string(),
        auth_version: MSA_AUTH_VERSION,
    })
}

/// Refresh credentials through Agora's application and repeat the Xbox and
/// Minecraft exchanges.
///
/// Credentials issued by the pre-migration flow are refused rather than sent to
/// the new application, whose refresh endpoint could only answer with an opaque
/// `invalid_grant`.
pub async fn refresh_credentials(
    clients: &HttpClients,
    creds: &MsaCredentials,
    db_path: &Path,
) -> LauncherResult<MsaCredentials> {
    check_msa_network_enabled(db_path)?;
    if creds.needs_reauth() {
        return Err(err(
            "ERR_MSA_LEGACY_CREDENTIALS",
            LEGACY_CREDENTIALS_MESSAGE,
        ));
    }

    let oauth = refresh_oauth_token(clients, &creds.refresh_token).await?;
    let refreshed = run_minecraft_chain(clients, &oauth, false).await?;
    store_credentials(&refreshed)?;
    Ok(refreshed)
}

// ---------------------------------------------------------------------------
// Credential storage
// ---------------------------------------------------------------------------

/// Which backend holds the Microsoft credentials.
///
/// `CredentialBackend::EncryptedFile` means the OS keyring was unavailable and
/// the credentials sit in a file guarded only by its permissions -- the state
/// MASTER_SPEC 7.5.2 requires Settings to warn about.
pub fn credentials_backend() -> crate::auth::CredentialBackend {
    crate::auth::credential_backend(KEYRING_SERVICE, KEYRING_ACCOUNT, CREDENTIALS_FALLBACK_FILE)
}

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
        clients: &HttpClients,
        credentials: &MsaCredentials,
    ) -> LauncherResult<MsaCredentials>;
}

/// Production MSA refresh client that delegates to the real HTTP endpoints.
pub(crate) struct LiveMsaRefreshHttp;

#[async_trait::async_trait]
impl MsaRefreshHttp for LiveMsaRefreshHttp {
    async fn refresh_credentials(
        &self,
        clients: &HttpClients,
        credentials: &MsaCredentials,
    ) -> LauncherResult<MsaCredentials> {
        let oauth = refresh_oauth_token(clients, &credentials.refresh_token).await?;
        run_minecraft_chain(clients, &oauth, false).await
    }
}

#[cfg(any(test, feature = "test-support"))]
use std::collections::VecDeque;
#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
#[cfg(any(test, feature = "test-support"))]
use std::sync::Mutex;

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
        _clients: &HttpClients,
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
            client_id: AGORA_MSA_CLIENT_ID.to_string(),
            auth_version: MSA_AUTH_VERSION,
        })
    }
}

/// Obtain valid MSA credentials with single-flight refresh and double-check.
///
/// 1. Loads stored credentials; returns `SignInRequired` if none exist or if
///    they came from the pre-migration flow.
/// 2. If still within the 5‑minute margin, returns `Valid` immediately.
/// 3. Acquires a refresh mutex and re‑checks (another caller may have refreshed).
/// 4. Refreshes the Microsoft token and repeats the Xbox/Minecraft exchanges.
///    - On `invalid_grant` / `expired_token`, clears credentials and returns `SignInRequired`.
///    - On network / 5xx / malformed responses, preserves credentials and returns `RefreshFailed`.
///    - On success, persists the result and returns `Valid`.
pub async fn get_valid_credentials(clients: &HttpClients) -> MsaCredentialOutcome {
    // A refused request is a temporary condition — Lockdown Mode or a disabled
    // endpoint — so stored credentials must be preserved rather than treated as
    // an invalidated session.
    if let Err(error) = crate::network_gate::authorize(ClientCategory::Microsoft) {
        return MsaCredentialOutcome::RefreshFailed(error);
    }
    get_valid_credentials_inner(&LiveMsaRefreshHttp, clients).await
}

/// Internal variant with injectable HTTP — enables deterministic testing.
async fn get_valid_credentials_inner(
    http: &dyn MsaRefreshHttp,
    clients: &HttpClients,
) -> MsaCredentialOutcome {
    // 1. Load stored credentials
    let creds = match load_credentials() {
        Ok(Some(c)) => c,
        Ok(None) => return MsaCredentialOutcome::SignInRequired,
        Err(e) => return MsaCredentialOutcome::RefreshFailed(e),
    };

    // A credential from the old application cannot be renewed by this one.
    // It is kept on disk so the UI can explain the one-time sign-in rather
    // than silently forgetting the account.
    if creds.needs_reauth() {
        return MsaCredentialOutcome::SignInRequired;
    }

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
    if creds.needs_reauth() {
        return MsaCredentialOutcome::SignInRequired;
    }
    if !creds.needs_refresh() {
        return MsaCredentialOutcome::Valid(creds);
    }

    // 4. Complete refresh chain — the permanent/transient decision point.
    let refreshed = match http.refresh_credentials(clients, &creds).await {
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

    fn status(code: u16) -> reqwest::StatusCode {
        reqwest::StatusCode::from_u16(code).unwrap()
    }

    // -----------------------------------------------------------------------
    // Configuration
    // -----------------------------------------------------------------------

    #[test]
    fn uses_agora_public_client_and_consumers_authority() {
        assert_eq!(AGORA_MSA_CLIENT_ID, "2dbd8051-d5dd-4611-bcae-b38400c1dc30");
        assert_eq!(MSA_SCOPE, "XboxLive.signin offline_access");
        assert!(DEVICE_CODE_URL.starts_with("https://login.microsoftonline.com/consumers/"));
        assert!(TOKEN_URL.starts_with("https://login.microsoftonline.com/consumers/"));
        assert!(DEVICE_CODE_URL.ends_with("/oauth2/v2.0/devicecode"));
        assert!(TOKEN_URL.ends_with("/oauth2/v2.0/token"));
    }

    #[test]
    fn secrets_are_redacted_in_debug_output() {
        let flow = MsaDeviceCodeFlow {
            device_code: "super-secret-device-code".into(),
            user_code: "ABCD-EFGH".into(),
            verification_uri: "https://microsoft.com/link".into(),
            expires_at: Utc::now(),
            interval_secs: 5,
        };
        let rendered = format!("{flow:?}");
        assert!(!rendered.contains("super-secret-device-code"));
        assert!(!rendered.contains("ABCD-EFGH"));

        let rendered = format!("{:?}", current_creds());
        assert!(!rendered.contains("mc_test_access"));
        assert!(!rendered.contains("mc_test_refresh"));
    }

    // -----------------------------------------------------------------------
    // Device-code poll classification
    // -----------------------------------------------------------------------

    #[test]
    fn poll_success_yields_tokens() {
        let body = r#"{"access_token":"ms-access","refresh_token":"ms-refresh","expires_in":3600}"#;
        match classify_device_poll(status(200), body) {
            DevicePoll::Complete(token) => {
                assert_eq!(token.access_token, "ms-access");
                assert_eq!(token.refresh_token, "ms-refresh");
            }
            other => panic!("expected Complete, got {other:?}"),
        }
    }

    #[test]
    fn poll_classifies_pending_slowdown_declined_and_expiry() {
        assert!(matches!(
            classify_device_poll(status(400), r#"{"error":"authorization_pending"}"#),
            DevicePoll::Pending
        ));
        assert!(matches!(
            classify_device_poll(status(400), r#"{"error":"slow_down"}"#),
            DevicePoll::SlowDown
        ));
        assert!(matches!(
            classify_device_poll(status(400), r#"{"error":"authorization_declined"}"#),
            DevicePoll::Declined
        ));
        assert!(matches!(
            classify_device_poll(status(400), r#"{"error":"access_denied"}"#),
            DevicePoll::Declined
        ));
        assert!(matches!(
            classify_device_poll(status(400), r#"{"error":"expired_token"}"#),
            DevicePoll::Expired
        ));
        // A 5xx with no OAuth error is a blip, not a decision.
        assert!(matches!(
            classify_device_poll(status(503), "<html>gateway</html>"),
            DevicePoll::Pending
        ));
    }

    #[test]
    fn poll_reports_application_access_denial_distinctly() {
        for code in ["unauthorized_client", "invalid_client", "invalid_scope"] {
            let body = format!(r#"{{"error":"{code}"}}"#);
            match classify_device_poll(status(400), &body) {
                DevicePoll::Fatal(LauncherError::Generic { code, .. }) => {
                    assert_eq!(code, "ERR_MSA_APP_ACCESS_DENIED");
                }
                other => panic!("expected application-access denial, got {other:?}"),
            }
        }
    }

    #[test]
    fn poll_never_echoes_error_descriptions() {
        let body = r#"{"error":"invalid_request","error_description":"AADSTS900144 user@example.com secret detail"}"#;
        let outcome = classify_device_poll(status(400), body);
        let rendered = format!("{outcome:?}");
        assert!(!rendered.contains("user@example.com"));
        assert!(!rendered.contains("secret detail"));
        assert!(rendered.contains("invalid_request"));
    }

    // -----------------------------------------------------------------------
    // Polling loop
    // -----------------------------------------------------------------------

    /// Scripted poller with a virtual clock: `sleep` advances time instead of
    /// waiting, so expiry and backoff are exercised without real delays.
    struct ScriptedPoller {
        script: Mutex<VecDeque<LauncherResult<DevicePoll>>>,
        now: Mutex<DateTime<Utc>>,
        slept: AtomicU64,
        calls: AtomicU64,
        cancel_after: Option<(u64, MsaLoginCancel)>,
    }

    impl ScriptedPoller {
        fn new(script: Vec<LauncherResult<DevicePoll>>) -> Self {
            Self {
                script: Mutex::new(script.into()),
                now: Mutex::new(Utc::now()),
                slept: AtomicU64::new(0),
                calls: AtomicU64::new(0),
                cancel_after: None,
            }
        }

        fn cancelling_after(mut self, calls: u64, cancel: MsaLoginCancel) -> Self {
            self.cancel_after = Some((calls, cancel));
            self
        }

        fn slept(&self) -> u64 {
            self.slept.load(AtomicOrdering::SeqCst)
        }

        fn calls(&self) -> u64 {
            self.calls.load(AtomicOrdering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl DeviceTokenPoller for ScriptedPoller {
        async fn poll_once(&self) -> LauncherResult<DevicePoll> {
            let call = self.calls.fetch_add(1, AtomicOrdering::SeqCst) + 1;
            if let Some((after, cancel)) = &self.cancel_after {
                if call >= *after {
                    cancel.cancel();
                }
            }
            self.script
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Ok(DevicePoll::Pending))
        }

        async fn sleep(&self, secs: u64) {
            self.slept.fetch_add(secs, AtomicOrdering::SeqCst);
            let mut now = self.now.lock().unwrap();
            *now += chrono::Duration::seconds(secs as i64);
        }

        fn now(&self) -> DateTime<Utc> {
            *self.now.lock().unwrap()
        }
    }

    fn far_future() -> DateTime<Utc> {
        Utc::now() + chrono::Duration::hours(1)
    }

    #[tokio::test]
    async fn poll_loop_returns_token_after_pending() {
        let poller = ScriptedPoller::new(vec![
            Ok(DevicePoll::Pending),
            Ok(DevicePoll::Pending),
            Ok(DevicePoll::Complete(OAuthToken {
                access_token: "ms-access".into(),
                refresh_token: "ms-refresh".into(),
            })),
        ]);
        let token = run_device_poll(&poller, 5, far_future(), &MsaLoginCancel::new())
            .await
            .expect("device poll should complete");
        assert_eq!(token.access_token, "ms-access");
        // Two pending answers, five seconds of waiting each — no busy polling.
        assert_eq!(poller.slept(), 10);
    }

    #[tokio::test]
    async fn poll_loop_backs_off_on_slow_down() {
        let poller = ScriptedPoller::new(vec![
            Ok(DevicePoll::SlowDown),
            Ok(DevicePoll::Complete(OAuthToken {
                access_token: "ms-access".into(),
                refresh_token: "ms-refresh".into(),
            })),
        ]);
        run_device_poll(&poller, 5, far_future(), &MsaLoginCancel::new())
            .await
            .expect("device poll should complete");
        // 5s interval + 5s slow_down increment.
        assert_eq!(poller.slept(), 10);
    }

    #[tokio::test]
    async fn poll_loop_stops_when_cancelled() {
        let cancel = MsaLoginCancel::new();
        let poller = ScriptedPoller::new(vec![Ok(DevicePoll::Pending), Ok(DevicePoll::Pending)])
            .cancelling_after(1, cancel.clone());

        let error = run_device_poll(&poller, 5, far_future(), &cancel)
            .await
            .expect_err("cancellation must stop the loop");
        assert!(format!("{error:?}").contains("ERR_MSA_LOGIN_CANCELLED"));
        assert_eq!(poller.calls(), 1, "must not poll again after cancellation");
        // Cancellation is noticed within the first second of the wait.
        assert!(poller.slept() <= 1);
    }

    #[tokio::test]
    async fn poll_loop_stops_at_expiry() {
        let poller = ScriptedPoller::new(vec![]);
        let expires_at = poller.now() + chrono::Duration::seconds(12);
        let error = run_device_poll(&poller, 5, expires_at, &MsaLoginCancel::new())
            .await
            .expect_err("expiry must stop the loop");
        assert!(format!("{error:?}").contains("ERR_MSA_LOGIN_EXPIRED"));
    }

    #[tokio::test]
    async fn poll_loop_reports_denial() {
        let poller = ScriptedPoller::new(vec![Ok(DevicePoll::Declined)]);
        let error = run_device_poll(&poller, 5, far_future(), &MsaLoginCancel::new())
            .await
            .expect_err("denial must stop the loop");
        assert!(format!("{error:?}").contains("ERR_MSA_LOGIN_DECLINED"));
    }

    #[tokio::test]
    async fn poll_loop_retries_transient_network_errors_but_not_policy_refusals() {
        let poller = ScriptedPoller::new(vec![
            Err(LauncherError::NetworkOffline),
            Ok(DevicePoll::Complete(OAuthToken {
                access_token: "ms-access".into(),
                refresh_token: "ms-refresh".into(),
            })),
        ]);
        run_device_poll(&poller, 5, far_future(), &MsaLoginCancel::new())
            .await
            .expect("a network blip must not end the login");

        // Lockdown / disabled endpoint / disabled MSA preference must stop it.
        for code in [
            "ERR_NETWORK_LOCKDOWN",
            "ERR_NETWORK_ENDPOINT_DISABLED",
            "ERR_NETWORK_GATE_MISSING",
            "ERR_NETWORK_DISABLED",
        ] {
            let poller = ScriptedPoller::new(vec![Err(err(code, "blocked"))]);
            let error = run_device_poll(&poller, 5, far_future(), &MsaLoginCancel::new())
                .await
                .expect_err("a policy refusal must stop the loop");
            assert!(format!("{error:?}").contains(code));
            assert_eq!(poller.calls(), 1);
        }
    }

    // -----------------------------------------------------------------------
    // Xbox / Minecraft response validation
    // -----------------------------------------------------------------------

    #[test]
    fn xsts_errors_are_distinguished() {
        let cases = [
            (2148916233u64, "ERR_MSA_XBOX_NO_ACCOUNT"),
            (2148916227, "ERR_MSA_XBOX_BANNED"),
            (2148916235, "ERR_MSA_XBOX_REGION"),
            (2148916238, "ERR_MSA_XBOX_CHILD_ACCOUNT"),
            (2148916236, "ERR_MSA_XBOX_ADULT_VERIFICATION"),
        ];
        for (xerr, expected) in cases {
            match xsts_error_for(xerr) {
                Some(LauncherError::Generic { code, .. }) => assert_eq!(code, expected),
                other => panic!("expected {expected} for {xerr}, got {other:?}"),
            }
        }
        assert!(xsts_error_for(1).is_none());
    }

    #[test]
    fn xerr_parses_from_number_or_string() {
        assert_eq!(parse_xerr(r#"{"XErr":2148916233}"#), Some(2148916233));
        assert_eq!(parse_xerr(r#"{"XErr":"2148916233"}"#), Some(2148916233));
        assert_eq!(parse_xerr("not json"), None);
        assert_eq!(parse_xerr("{}"), None);
    }

    #[test]
    fn entitlements_require_a_java_item_not_just_a_200() {
        // The regression this guards: an empty items array is a successful
        // response from an account that owns nothing.
        assert!(!entitlements_grant_java(r#"{"items":[]}"#));
        assert!(!entitlements_grant_java(r#"{}"#));
        assert!(!entitlements_grant_java("not json"));
        // Bedrock ownership does not grant Java.
        assert!(!entitlements_grant_java(
            r#"{"items":[{"name":"product_minecraft_bedrock"},{"name":"game_minecraft_bedrock"}]}"#
        ));
        assert!(entitlements_grant_java(
            r#"{"items":[{"name":"product_minecraft"},{"name":"game_minecraft"}]}"#
        ));
        assert!(entitlements_grant_java(
            r#"{"items":[{"name":"product_game_pass_ultimate"}]}"#
        ));
    }

    #[test]
    fn user_hash_is_read_from_display_claims() {
        let token: xbox_types::XboxToken = serde_json::from_str(
            r#"{"Token":"xbl","DisplayClaims":{"xui":[{"uhs":"user-hash"}]}}"#,
        )
        .unwrap();
        assert_eq!(token.user_hash().as_deref(), Some("user-hash"));

        let token: xbox_types::XboxToken =
            serde_json::from_str(r#"{"Token":"xbl","DisplayClaims":{}}"#).unwrap();
        assert!(token.user_hash().is_none());
    }

    // -----------------------------------------------------------------------
    // Credential expiry, migration, and storage
    // -----------------------------------------------------------------------

    fn current_creds() -> MsaCredentials {
        MsaCredentials {
            username: "test_user".into(),
            uuid: "abc-def-ghi".into(),
            access_token: "mc_test_access".into(),
            refresh_token: "mc_test_refresh".into(),
            expires: Utc::now() + chrono::Duration::hours(1),
            client_id: AGORA_MSA_CLIENT_ID.into(),
            auth_version: MSA_AUTH_VERSION,
        }
    }

    fn expired_creds() -> MsaCredentials {
        let mut c = current_creds();
        c.expires = Utc::now() - chrono::Duration::minutes(1);
        c
    }

    #[test]
    fn credentials_expiry_logic() {
        assert!(expired_creds().is_expired());
        assert!(expired_creds().needs_refresh());
        assert!(!current_creds().is_expired());
        assert!(!current_creds().needs_refresh());
    }

    #[test]
    fn legacy_credentials_are_detected_and_migrate_by_signing_in() {
        // Credentials written by the old flow have no client_id/auth_version.
        let legacy: MsaCredentials = serde_json::from_str(
            r#"{"username":"old","uuid":"u","access_token":"a","refresh_token":"r",
                "expires":"2099-01-01T00:00:00Z"}"#,
        )
        .expect("pre-migration credentials must still deserialize");
        assert_eq!(legacy.auth_version, 0);
        assert!(legacy.client_id.is_empty());
        assert!(legacy.needs_reauth());

        // A credential from some other application is equally unusable.
        let mut foreign = current_creds();
        foreign.client_id = "00000000-0000-0000-0000-000000000000".into();
        assert!(foreign.needs_reauth());

        assert!(!current_creds().needs_reauth());
    }

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

    fn test_clients() -> HttpClients {
        HttpClients::new().expect("http clients")
    }

    #[tokio::test]
    async fn test_get_valid_credentials_fresh_returns_immediately() {
        let _test_lock = TEST_MSA_MUTEX.lock().await;
        let http = MockMsaRefreshHttp::new();
        let _td = write_test_msa_creds(&current_creds());

        let outcome = get_valid_credentials_inner(&http, &test_clients()).await;

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
        let _td = write_test_msa_creds(&current_creds());

        // Clear after writing so no credentials exist
        let _ = clear_credentials();

        let outcome = get_valid_credentials_inner(&http, &test_clients()).await;

        assert!(matches!(outcome, MsaCredentialOutcome::SignInRequired));
    }

    #[tokio::test]
    async fn test_get_valid_credentials_refresh_success() {
        let _test_lock = TEST_MSA_MUTEX.lock().await;
        let http = MockMsaRefreshHttp::new();
        http.queue_success("new_mc_token", "new_refresh_token", 28800);
        let _td = write_test_msa_creds(&expired_creds());

        let outcome = get_valid_credentials_inner(&http, &test_clients()).await;

        match outcome {
            MsaCredentialOutcome::Valid(c) => {
                assert_eq!(c.username, "test_user");
                assert_eq!(c.access_token, "new_mc_token");
                assert_eq!(c.refresh_token, "new_refresh_token");
                assert_eq!(c.client_id, AGORA_MSA_CLIENT_ID);
                assert_eq!(c.auth_version, MSA_AUTH_VERSION);
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
        assert_eq!(http.call_count(), 1);
    }

    #[tokio::test]
    async fn test_legacy_credentials_require_sign_in_without_refresh_attempt() {
        let _test_lock = TEST_MSA_MUTEX.lock().await;
        let http = MockMsaRefreshHttp::new();
        // No responses queued: a refresh attempt would panic the mock.
        let mut legacy = expired_creds();
        legacy.client_id = String::new();
        legacy.auth_version = 0;
        let _td = write_test_msa_creds(&legacy);

        let outcome = get_valid_credentials_inner(&http, &test_clients()).await;

        assert!(matches!(outcome, MsaCredentialOutcome::SignInRequired));
        assert_eq!(
            http.call_count(),
            0,
            "a legacy refresh token must never be sent to the new application"
        );
        // Kept on disk so the UI can explain the one-time sign-in.
        assert!(load_credentials().unwrap().is_some());
    }

    #[tokio::test]
    async fn test_get_valid_credentials_invalid_grant_is_permanent() {
        let _test_lock = TEST_MSA_MUTEX.lock().await;
        let http = MockMsaRefreshHttp::new();
        http.queue_error(LauncherError::MsaAuthRequired);
        let _td = write_test_msa_creds(&expired_creds());

        let outcome = get_valid_credentials_inner(&http, &test_clients()).await;
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
        let _td = write_test_msa_creds(&expired_creds());

        let outcome = get_valid_credentials_inner(&http, &test_clients()).await;
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
        let _td = write_test_msa_creds(&current_creds());

        let outcome = get_valid_credentials_inner(&http, &test_clients()).await;
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

    /// Build a local state DB with one network setting written.
    fn state_db_with(dir: &std::path::Path, key: &str, enabled: bool) -> std::path::PathBuf {
        let db_path = dir.join("local_state.db");
        db::init_local_state_db(&db_path).expect("init local state db");
        let conn = db::local_state_connection(&db_path).expect("open local state db");
        db::set_setting(&conn, key, &serde_json::json!(enabled)).expect("write setting");
        db_path
    }

    #[tokio::test]
    async fn login_and_refresh_stop_when_msa_networking_is_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = state_db_with(dir.path(), "network_msa_enabled", false);

        let error = begin_login(&test_clients(), &db_path)
            .await
            .expect_err("sign-in must be refused when the MSA preference is off");
        assert!(format!("{error:?}").contains("ERR_NETWORK_DISABLED"));

        let flow = MsaDeviceCodeFlow {
            device_code: "device".into(),
            user_code: "ABCD-EFGH".into(),
            verification_uri: "https://microsoft.com/link".into(),
            expires_at: far_future(),
            interval_secs: 5,
        };
        let error = poll_login(&test_clients(), &flow, &db_path, &MsaLoginCancel::new())
            .await
            .expect_err("polling must be refused when the MSA preference is off");
        assert!(format!("{error:?}").contains("ERR_NETWORK_DISABLED"));

        let error = refresh_credentials(&test_clients(), &current_creds(), &db_path)
            .await
            .expect_err("refresh must be refused when the MSA preference is off");
        assert!(format!("{error:?}").contains("ERR_NETWORK_DISABLED"));
    }

    #[tokio::test]
    async fn lockdown_mode_blocks_login_and_refresh() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = state_db_with(dir.path(), "network_lockdown_enabled", true);

        let error = begin_login(&test_clients(), &db_path)
            .await
            .expect_err("Lockdown Mode must refuse sign-in");
        assert!(format!("{error:?}").contains("ERR_NETWORK_DISABLED"));

        let error = refresh_credentials(&test_clients(), &current_creds(), &db_path)
            .await
            .expect_err("Lockdown Mode must refuse refresh");
        assert!(format!("{error:?}").contains("ERR_NETWORK_DISABLED"));
    }

    #[tokio::test]
    async fn test_refresh_credentials_refuses_legacy_without_network() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("local_state.db");
        let mut legacy = current_creds();
        legacy.auth_version = 0;
        legacy.client_id = String::new();

        let error = refresh_credentials(&test_clients(), &legacy, &db_path)
            .await
            .expect_err("legacy credentials must not be refreshed");
        assert!(format!("{error:?}").contains("ERR_MSA_LEGACY_CREDENTIALS"));
    }
}
