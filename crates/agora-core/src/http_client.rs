//! Category-aware HTTP clients with per-category timeouts, host allowlists,
//! URL scheme/port/IP validation, and redirect re-validation.
//!
//! **All production requests must go through a checked helper** — never call
//! `clients.get(category).get(url).send()` directly, as that bypasses the
//! URL validation, category allowlist, redirect re-validation, and response
//! size limits enforced here.
//!
//! Available helpers:
//! - [`checked_request`] — async GET with full enforcement, returns response.
//! - [`checked_get_bytes`] — async GET, returns validated bytes.
//! - [`checked_request_with_headers`] — async GET with custom headers (auth).
//! - [`blocking_checked_request`] — blocking GET with full enforcement.
//! - [`blocking_checked_get_bytes`] — blocking GET, returns validated bytes.
//!
//! Per-artifact hash verification remains at individual call sites.
//!
//! # Construction
//!
//! - [`HttpClients::new()`] returns `LauncherResult<Self>` — fails if the
//!   TLS backend cannot be initialised.
//! - [`HttpClients::for_testing()`] provides a single-client wrapper for tests
//!   (bypasses policy — never use in production).

use crate::error::{LauncherError, LauncherResult};
use crate::network;
use std::io::Read;
use std::net::IpAddr;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Category-to-allowlist mapping
// ---------------------------------------------------------------------------

/// Per-category host allowlist (first-party domains only).
///
/// Unknown public hosts are rejected in production helpers.
/// Dynamic user-approved endpoints (e.g. custom AI assistant host) must use
/// an explicit policy path.
pub(crate) fn category_allowlist(category: ClientCategory) -> &'static [&'static str] {
    match category {
        ClientCategory::MojangMetadata => &[
            "piston-meta.mojang.com",
            "launchermeta.mojang.com",
            "launcher.mojang.com",
        ],
        ClientCategory::MojangContent => &[
            "resources.download.minecraft.net",
            "libraries.minecraft.net",
            "piston-data.mojang.com",
        ],
        ClientCategory::Loader => &[
            "meta.fabricmc.net",
            "maven.fabricmc.net",
            "maven.quiltmc.org",
            "files.minecraftforge.net",
            "maven.minecraftforge.net",
            "maven.neoforged.net",
            "repo.spongepowered.org",
            "raw.githubusercontent.com",
        ],
        ClientCategory::Modrinth => &["api.modrinth.com", "cdn.modrinth.com"],
        ClientCategory::Modpack => &[
            "cdn.modrinth.com",
            "github.com",
            "objects.githubusercontent.com",
            "releases.githubusercontent.com",
            "release-assets.githubusercontent.com",
        ],
        ClientCategory::GitHub | ClientCategory::Registry => &[
            "github.com",
            "api.github.com",
            "objects.githubusercontent.com",
            "releases.githubusercontent.com",
            "release-assets.githubusercontent.com",
            "raw.githubusercontent.com",
        ],
        ClientCategory::JavaRuntime => &[
            "api.adoptium.net",
            "github.com",
            "objects.githubusercontent.com",
            "release-assets.githubusercontent.com",
        ],
        ClientCategory::Microsoft => &[
            "login.live.com",
            "login.microsoftonline.com",
            "sisu.xboxlive.com",
            "api.minecraftservices.com",
            "xsts.auth.xboxlive.com",
            "user.auth.xboxlive.com",
            "device.auth.xboxlive.com",
        ],
        ClientCategory::AiAssistant => &[
            // Explicitly approved Copilot API hosts. Other user-configured
            // AI endpoints require a separate approval policy and are not
            // accepted by this production helper.
            "api.individual.githubcopilot.com",
            "api.githubcopilot.com",
        ],
        // Deliberately empty: hosts for this category come from the signed
        // manifest via `HostPolicy::SignedManifest`, never from a compile-time
        // list. The empty-list-rejects-everything property means the category
        // fails closed if reached without such a policy.
        ClientCategory::PinnedArtifact => &[],
        // Deliberately empty: hosts are authorized case-by-case by `UserConsented`
        // after the core consent check. The empty list fails closed if the
        // category is ever reached through the generic Allowlist path.
        ClientCategory::ConsentedContent => &[],
    }
}

/// How a request's hostname is authorized.
///
/// The default is the category's compile-time allowlist. A `SignedManifest`
/// host is taken from the Ed25519-signed registry: the curator reviewed the
/// URL and pinned its SHA-256, so the manifest itself satisfies the host
/// allowlist gate. Every other gate (scheme, port, userinfo, IP literals,
/// non-private DNS resolution) still applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostPolicy<'a> {
    /// The category's compile-time allowlist (existing behaviour).
    Allowlist,
    /// A host taken from the Ed25519-signed registry. Redirects are only
    /// permitted to the same host (or an `.<pinned>` subdomain) — a curator
    /// whose URL redirects off-domain should pin the final URL.
    SignedManifest(&'a str),
    /// The request belongs to a content category the user explicitly opted
    /// into (`technic_enabled` / `allow_unverified_packs`). Only the
    /// consent-checked entry points may use this; the consent check lives in
    /// core, so a frontend bug cannot widen it. Every other gate still
    /// applies, and the private/loopback/link-local DNS rejection is retained.
    UserConsented,
}

/// Friendly name for each HTTP client category, used in logging and errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClientCategory {
    /// Mojang metadata: version manifest, version JSON, asset index.
    MojangMetadata,
    /// Mojang content: client JAR, libraries, natives, assets, logging config.
    MojangContent,
    /// Loader metadata and content: pinned profiles, Maven artifacts.
    Loader,
    /// Modrinth API and CDN.
    Modrinth,
    /// Modpack archives, which are substantially larger than individual mods.
    Modpack,
    /// GitHub API and release assets.
    GitHub,
    /// Microsoft/ Xbox Live authentication (MSA).
    Microsoft,
    /// Registry database download from GitHub Releases.
    Registry,
    /// AI assistant / OpenAI-compatible API.
    AiAssistant,
    /// Managed Java runtime metadata and archives.
    JavaRuntime,
    /// Artifacts pinned by SHA-256 in the signed registry; host authorization
    /// comes from the manifest itself (`HostPolicy::SignedManifest`).
    PinnedArtifact,
    /// Content the user explicitly opted into (Technic tiers S/Z). Reached
    /// only through the core consent-gated helpers via `HostPolicy::UserConsented`.
    ConsentedContent,
}

impl ClientCategory {
    fn timeout(&self) -> Duration {
        match self {
            ClientCategory::MojangMetadata => Duration::from_secs(30),
            ClientCategory::MojangContent => Duration::from_secs(120),
            ClientCategory::Loader => Duration::from_secs(60),
            ClientCategory::Modrinth => Duration::from_secs(30),
            ClientCategory::Modpack => Duration::from_secs(5 * 60),
            ClientCategory::GitHub => Duration::from_secs(30),
            ClientCategory::Microsoft => Duration::from_secs(30),
            ClientCategory::Registry => Duration::from_secs(60),
            ClientCategory::AiAssistant => Duration::from_secs(60),
            ClientCategory::JavaRuntime => Duration::from_secs(120),
            ClientCategory::PinnedArtifact => Duration::from_secs(120),
            ClientCategory::ConsentedContent => Duration::from_secs(5 * 60),
        }
    }

    fn user_agent(&self) -> &'static str {
        "AgoraLauncher/1.0"
    }

    fn max_response_bytes(&self) -> Option<u64> {
        match self {
            ClientCategory::MojangContent => Some(200 * 1024 * 1024),
            ClientCategory::Modrinth => Some(200 * 1024 * 1024),
            ClientCategory::Modpack => Some(500 * 1024 * 1024),
            ClientCategory::Loader => Some(100 * 1024 * 1024),
            ClientCategory::Registry => Some(100 * 1024 * 1024),
            ClientCategory::JavaRuntime => Some(512 * 1024 * 1024),
            ClientCategory::PinnedArtifact => Some(100 * 1024 * 1024),
            ClientCategory::ConsentedContent => Some(500 * 1024 * 1024),
            _ => Some(10 * 1024 * 1024),
        }
    }
}

// ---------------------------------------------------------------------------
// HttpClients
// ---------------------------------------------------------------------------

/// A set of pre-built HTTP clients for different network categories.
///
/// Construct via [`HttpClients::new()`] (production) or
/// [`HttpClients::for_testing()`] (tests only — no policy enforcement).
#[derive(Debug, Clone)]
pub struct HttpClients {
    mojang_metadata: reqwest::Client,
    mojang_content: reqwest::Client,
    loader: reqwest::Client,
    modrinth: reqwest::Client,
    github: reqwest::Client,
    microsoft: reqwest::Client,
    registry: reqwest::Client,
    ai_assistant: reqwest::Client,
    java_runtime: reqwest::Client,
}

impl HttpClients {
    /// Build a full set of category-aware clients.
    ///
    /// Returns `Err` if the TLS backend cannot be initialised (fatal).
    pub fn new() -> LauncherResult<Self> {
        Ok(Self {
            mojang_metadata: Self::build_client(ClientCategory::MojangMetadata)?,
            mojang_content: Self::build_client(ClientCategory::MojangContent)?,
            loader: Self::build_client(ClientCategory::Loader)?,
            modrinth: Self::build_client(ClientCategory::Modrinth)?,
            github: Self::build_client(ClientCategory::GitHub)?,
            microsoft: Self::build_client(ClientCategory::Microsoft)?,
            registry: Self::build_client(ClientCategory::Registry)?,
            ai_assistant: Self::build_client(ClientCategory::AiAssistant)?,
            java_runtime: Self::build_client(ClientCategory::JavaRuntime)?,
        })
    }

    /// Build with a single client used for all categories (**testing only**).
    ///
    /// This bypasses category-specific timeouts and policies — never use in
    /// production code. When the actual policy matters, use [`new()`](Self::new)
    /// and override individual clients with `with_*`.
    pub fn for_testing(client: reqwest::Client) -> Self {
        Self {
            mojang_metadata: client.clone(),
            mojang_content: client.clone(),
            loader: client.clone(),
            modrinth: client.clone(),
            github: client.clone(),
            microsoft: client.clone(),
            registry: client.clone(),
            ai_assistant: client.clone(),
            java_runtime: client,
        }
    }

    /// Get the raw client for a category.
    ///
    /// Prefer [`checked_request`] or [`checked_get_bytes`] instead of using
    /// this directly, to ensure policy enforcement.
    pub fn get(&self, category: ClientCategory) -> &reqwest::Client {
        match category {
            ClientCategory::MojangMetadata => &self.mojang_metadata,
            ClientCategory::MojangContent => &self.mojang_content,
            ClientCategory::Loader => &self.loader,
            ClientCategory::Modrinth => &self.modrinth,
            ClientCategory::Modpack => &self.modrinth,
            ClientCategory::GitHub => &self.github,
            ClientCategory::Microsoft => &self.microsoft,
            ClientCategory::Registry => &self.registry,
            ClientCategory::AiAssistant => &self.ai_assistant,
            ClientCategory::JavaRuntime => &self.java_runtime,
            ClientCategory::PinnedArtifact => &self.modrinth,
            ClientCategory::ConsentedContent => &self.modrinth,
        }
    }

    fn build_client(category: ClientCategory) -> LauncherResult<reqwest::Client> {
        // Redirects are handled by checked_request's manual per-hop loop
        // with re-validation. The client itself follows none to prevent
        // any accidental bypass when callers use .get() directly.
        reqwest::Client::builder()
            .timeout(category.timeout())
            .user_agent(category.user_agent())
            .redirect(reqwest::redirect::Policy::none())
            .pool_max_idle_per_host(4)
            .build()
            .map_err(|e| LauncherError::Generic {
                code: "ERR_HTTP_CLIENT_BUILD".into(),
                message: format!("Failed to build HTTP client for {category:?}: {e}"),
            })
    }

    // ------------------------------------------------------------------
    // Builder override helpers (testing)
    // ------------------------------------------------------------------

    /// Replace the Modrinth client (e.g., with a mock).
    pub fn with_modrinth_client(mut self, client: reqwest::Client) -> Self {
        self.modrinth = client;
        self
    }

    /// Replace the GitHub client (e.g., with a mock).
    pub fn with_github_client(mut self, client: reqwest::Client) -> Self {
        self.github = client;
        self
    }
}

// ---------------------------------------------------------------------------
// URL validation (shared by checked_request and checked_get_bytes)
// ---------------------------------------------------------------------------

/// Validate a URL for a given category against all policies.
///
/// Checks:
/// 1. URL parses successfully.
/// 2. Scheme is HTTPS (not HTTP, not file, not data, not anything else).
/// 3. Port is 443 (default HTTPS) — no non-standard ports.
/// 4. No userinfo component (prevents `https://user:pass@host/`).
/// 5. Host is not an IP literal (must be a domain name).
/// 6. Host is on the category allowlist.
/// 7. Host resolves to a non-private/reserved IP (loopback, private,
///    link-local, multicast, unspecified — SSRF protection).
///
/// The allowlist and IP check together reduce but do not fully eliminate
/// DNS rebinding / SSRF risk. Hostname-based allowlists do not protect
/// against an attacker-controlled DNS that resolves an allowlisted domain
/// to a private IP after our check. This is an unavoidable platform
/// limitation without DNS-level pinning (HSTS, CAA, DANE).
pub fn check_request_url(category: ClientCategory, url: &str) -> LauncherResult<reqwest::Url> {
    check_request_url_with_policy(category, url, HostPolicy::Allowlist)
}

/// Validate a URL against all policies using an explicit host-authorization
/// policy.
///
/// Identical to [`check_request_url`] except that gate 5 (host authorization)
/// is satisfied either by the category allowlist or, under
/// [`HostPolicy::SignedManifest`], by the pinned manifest host itself (or one
/// of its subdomains). All other gates are unchanged.
pub fn check_request_url_with_policy(
    category: ClientCategory,
    url: &str,
    policy: HostPolicy<'_>,
) -> LauncherResult<reqwest::Url> {
    let parsed = reqwest::Url::parse(url).map_err(|_| LauncherError::Generic {
        code: "ERR_INVALID_URL".into(),
        message: format!("Cannot parse URL: {url}"),
    })?;

    // 1. Scheme. Consented-content categories may use plain HTTP (the consent
    //    covers the transport the author actually offers); everything else
    //    requires HTTPS.
    let consented = category == ClientCategory::ConsentedContent;
    let scheme_ok = parsed.scheme() == "https" || (consented && parsed.scheme() == "http");
    if !scheme_ok {
        return Err(LauncherError::Generic {
            code: "ERR_HTTP_SCHEME".into(),
            message: format!(
                "URL scheme must be HTTPS: {}",
                network::sanitized_url_for_log(url)
            ),
        });
    }

    // 2. Port. Consented-content categories may use any non-zero port; all
    //    other categories must use the default HTTPS port (443).
    if let Some(port) = parsed.port() {
        let port_ok = if consented { port != 0 } else { port == 443 };
        if !port_ok {
            return Err(LauncherError::Generic {
                code: "ERR_HTTP_PORT".into(),
                message: format!(
                    "Non-standard port {port} blocked for {}",
                    network::sanitized_url_for_log(url)
                ),
            });
        }
    }

    // 3. No userinfo (username:password in URL).
    if parsed.username() != "" || parsed.password().is_some() {
        return Err(LauncherError::Generic {
            code: "ERR_HTTP_USERINFO".into(),
            message: "URL must not contain userinfo (username:password).".into(),
        });
    }

    let host = parsed.host_str().ok_or_else(|| LauncherError::Generic {
        code: "ERR_INVALID_URL".into(),
        message: format!("URL has no host: {}", network::sanitized_url_for_log(url)),
    })?;

    // 4. Reject IP literals — require domain names.
    if host.parse::<IpAddr>().is_ok() {
        return Err(LauncherError::Generic {
            code: "ERR_HTTP_IP_LITERAL".into(),
            message: format!("IP literal {host} blocked; domain name required"),
        });
    }

    // 5. Host authorization.
    //
    // An empty allowlist rejects ALL hosts (the category has no approved
    // endpoints). Non-empty allowlists support exact and subdomain matching
    // (e.g. "objects.githubusercontent.com" matches when "github.com" is in
    // the list via ends_with(".github.com")). Under a SignedManifest policy
    // the pinned host (or a subdomain of it) replaces the allowlist — the
    // curator case-by-case approval *is* the authorization.
    let host_ok = host_authorized(category, host, policy);
    if !host_ok {
        return Err(LauncherError::Generic {
            code: "ERR_HTTP_HOST_NOT_ALLOWED".into(),
            message: format!("Host {host} is not in the {category:?} allowlist",),
        });
    }

    // 6. SSRF protection: DNS resolution check for private IPs.
    //    Note: this is best-effort — the IP may change between check and
    //    connect. DNS-level hostname validation without DNSSEC cannot
    //    fully prevent DNS rebinding on attacker-controlled infrastructure.
    //    For first-party allowlisted domains this risk is negligible.
    //    Every resolved address is checked, not just the first: a host that
    //    resolves to both a public and a private address must not pass because
    //    the public one happened to sort first.
    if let Ok(addrs) = parsed.socket_addrs(|| None) {
        if addrs.iter().any(|addr| is_blocked_ip(addr.ip())) {
            return Err(LauncherError::Generic {
                code: "ERR_SSRF_BLOCKED".into(),
                message: format!("Request blocked: {host} resolves to a private/reserved address"),
            });
        }
    }

    Ok(parsed)
}

/// Whether an address is private or reserved, and so not a legitimate
/// destination for an outbound request.
///
/// The IPv6 ranges are spelled out because the matching `Ipv6Addr` helpers
/// (`is_unique_local`, `is_unicast_link_local`) are still unstable. An
/// IPv4-mapped or IPv4-compatible address is re-checked under the IPv4 rules so
/// that `::ffff:127.0.0.1` cannot slip past them by wearing a v6 shape.
fn is_blocked_ip(ip: IpAddr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return true;
    }
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4() {
                return is_blocked_ip(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            // fc00::/7 unique local, fe80::/10 link-local unicast.
            (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
    }
}

/// Decide whether `host` is authorized under `policy`.
fn host_authorized(category: ClientCategory, host: &str, policy: HostPolicy<'_>) -> bool {
    match policy {
        HostPolicy::Allowlist => {
            let allowlist = category_allowlist(category);
            allowlist
                .iter()
                .any(|allowed| host == *allowed || host.ends_with(&format!(".{allowed}")))
        }
        HostPolicy::SignedManifest(pinned) => {
            host == pinned || host.ends_with(&format!(".{pinned}"))
        }
        HostPolicy::UserConsented => {
            // Host authorization is delegated to the core consent check that
            // precedes this request. The empty ConsentedContent allowlist
            // keeps the generic Allowlist path failed-closed.
            category == ClientCategory::ConsentedContent
        }
    }
}

// ---------------------------------------------------------------------------
// Checked request helpers (with per-hop redirect re-validation)
// ---------------------------------------------------------------------------

/// Perform a GET with full URL validation and per-hop redirect re-validation.
///
/// Validates the initial URL and each redirect hop against the same category
/// policies. Returns the final response.
pub async fn checked_request(
    clients: &HttpClients,
    category: ClientCategory,
    url: &str,
) -> LauncherResult<reqwest::Response> {
    checked_request_with_policy(clients, category, url, HostPolicy::Allowlist).await
}

/// Perform a GET with full URL validation and per-hop redirect re-validation
/// under an explicit host-authorization [`HostPolicy`].
///
/// The same policy is applied to the initial URL and every redirect hop, so a
/// `SignedManifest` pinned artifact cannot escape to an off-domain host via a
/// redirect.
pub async fn checked_request_with_policy(
    clients: &HttpClients,
    category: ClientCategory,
    url: &str,
    policy: HostPolicy<'_>,
) -> LauncherResult<reqwest::Response> {
    // Validate initial URL against category policies.
    let _validated = check_request_url_with_policy(category, url, policy)?;

    let client = clients.get(category);

    // Build a custom redirect policy that re-validates each hop.
    let mut remaining_redirects: u8 = 10;
    let mut current_url = url.to_string();

    loop {
        let response =
            client
                .get(&current_url)
                .send()
                .await
                .map_err(|e| LauncherError::Generic {
                    code: "ERR_NETWORK".into(),
                    message: format!(
                        "HTTP GET failed for {}: {e}",
                        network::sanitized_url_for_log(&current_url)
                    ),
                })?;

        // Check if the response is a redirect.
        let status = response.status();
        if status.is_redirection() {
            if remaining_redirects == 0 {
                return Err(LauncherError::Generic {
                    code: "ERR_TOO_MANY_REDIRECTS".into(),
                    message: format!(
                        "Too many redirects for {}",
                        network::sanitized_url_for_log(url)
                    ),
                });
            }

            // Get the Location header.
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| LauncherError::Generic {
                    code: "ERR_REDIRECT_NO_LOCATION".into(),
                    message: "Redirect without Location header".to_string(),
                })?;

            // Resolve relative redirects against the current URL.
            let next_url = reqwest::Url::parse(&current_url)
                .ok()
                .and_then(|base| {
                    reqwest::Url::parse(location)
                        .or_else(|_| base.join(location))
                        .ok()
                })
                .ok_or_else(|| LauncherError::Generic {
                    code: "ERR_INVALID_REDIRECT".into(),
                    message: format!("Cannot resolve redirect Location: {location}"),
                })?;

            // Re-validate the redirect target against the same policies.
            let _ = check_request_url_with_policy(category, next_url.as_str(), policy)?;

            remaining_redirects -= 1;
            current_url = next_url.to_string();
            continue;
        }

        return Ok(response);
    }
}

/// Send an HTTP request through the category policy path.
///
/// Custom headers are applied only to the initial request. A request body is
/// replayed only for 307/308 redirects; 301/302/303 redirects become GETs so
/// credentials or form bodies are never silently replayed to a new endpoint.
pub async fn checked_send(
    clients: &HttpClients,
    category: ClientCategory,
    method: reqwest::Method,
    url: &str,
    headers: &[(String, String)],
    body: Option<Vec<u8>>,
    content_type: Option<&str>,
) -> LauncherResult<reqwest::Response> {
    check_request_url(category, url)?;
    let client = clients.get(category);
    let mut current_method = method;
    let mut current_url = url.to_string();
    let mut current_body = body;
    let mut first_request = true;
    let mut remaining_redirects = 10u8;

    loop {
        let mut request = client.request(current_method.clone(), &current_url);
        if first_request {
            for (key, value) in headers {
                request = request.header(key, value);
            }
            if let Some(content_type) = content_type {
                request = request.header(reqwest::header::CONTENT_TYPE, content_type);
            }
        }
        if let Some(body) = current_body.clone() {
            request = request.body(body);
        }

        let response = request.send().await.map_err(|e| LauncherError::Generic {
            code: "ERR_NETWORK".into(),
            message: format!(
                "HTTP request failed for {}: {e}",
                network::sanitized_url_for_log(&current_url)
            ),
        })?;
        if !response.status().is_redirection() {
            return Ok(response);
        }
        if remaining_redirects == 0 {
            return Err(LauncherError::Generic {
                code: "ERR_TOO_MANY_REDIRECTS".into(),
                message: format!(
                    "Too many redirects for {}",
                    network::sanitized_url_for_log(url)
                ),
            });
        }
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| LauncherError::Generic {
                code: "ERR_REDIRECT_NO_LOCATION".into(),
                message: "Redirect without Location header".into(),
            })?;
        let next_url = reqwest::Url::parse(&current_url)
            .ok()
            .and_then(|base| {
                reqwest::Url::parse(location)
                    .or_else(|_| base.join(location))
                    .ok()
            })
            .ok_or_else(|| LauncherError::Generic {
                code: "ERR_INVALID_REDIRECT".into(),
                message: "Cannot resolve redirect Location".into(),
            })?;
        check_request_url(category, next_url.as_str())?;

        match response.status().as_u16() {
            307 | 308 => {}
            _ => {
                current_method = reqwest::Method::GET;
                current_body = None;
            }
        }
        current_url = next_url.to_string();
        first_request = false;
        remaining_redirects -= 1;
    }
}

/// Send an URL-encoded form through the checked policy path.
pub async fn checked_post_form(
    clients: &HttpClients,
    category: ClientCategory,
    url: &str,
    fields: &[(&str, &str)],
    headers: &[(String, String)],
) -> LauncherResult<reqwest::Response> {
    let encoded = fields
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                urlencoding::encode(key),
                urlencoding::encode(value)
            )
        })
        .collect::<Vec<_>>()
        .join("&");
    checked_send(
        clients,
        category,
        reqwest::Method::POST,
        url,
        headers,
        Some(encoded.into_bytes()),
        Some("application/x-www-form-urlencoded"),
    )
    .await
}

/// Send a JSON body through the checked policy path.
pub async fn checked_post_json(
    clients: &HttpClients,
    category: ClientCategory,
    url: &str,
    body: &serde_json::Value,
    headers: &[(String, String)],
) -> LauncherResult<reqwest::Response> {
    let body = serde_json::to_vec(body).map_err(|e| LauncherError::Generic {
        code: "ERR_JSON_ENCODE".into(),
        message: format!("Failed to encode request body: {e}"),
    })?;
    checked_send(
        clients,
        category,
        reqwest::Method::POST,
        url,
        headers,
        Some(body),
        Some("application/json"),
    )
    .await
}

/// Fetch JSON through the checked policy path.
pub async fn checked_get_json<T: serde::de::DeserializeOwned>(
    clients: &HttpClients,
    category: ClientCategory,
    url: &str,
) -> LauncherResult<T> {
    let bytes = checked_get_bytes(clients, category, url).await?;
    serde_json::from_slice(&bytes).map_err(|e| LauncherError::Generic {
        code: "ERR_JSON_DECODE".into(),
        message: format!("Failed to decode response JSON: {e}"),
    })
}

/// Download all bytes from a category-routed URL with full validation.
///
/// Validates the initial URL and each redirect hop, verifies HTTP success,
/// and enforces per-category response size limits. Pre-checks
/// Content-Length (fast reject) and validates the full body against the cap.
pub async fn checked_get_bytes(
    clients: &HttpClients,
    category: ClientCategory,
    url: &str,
) -> LauncherResult<Vec<u8>> {
    checked_get_bytes_inner(
        clients,
        category,
        url,
        HostPolicy::Allowlist,
        |_downloaded, _total| {},
    )
    .await
}

/// Download bytes under an explicit host-authorization [`HostPolicy`].
///
/// Identical to [`checked_get_bytes`] except that host authorization uses the
/// given policy instead of the category allowlist.
pub async fn checked_get_bytes_with_policy(
    clients: &HttpClients,
    category: ClientCategory,
    url: &str,
    policy: HostPolicy<'_>,
) -> LauncherResult<Vec<u8>> {
    checked_get_bytes_inner(clients, category, url, policy, |_downloaded, _total| {}).await
}

/// Download bytes while reporting cumulative body progress after each chunk.
///
/// The callback is synchronous and runs on the async task that owns the
/// response. It must remain lightweight and must not perform blocking I/O.
pub async fn checked_get_bytes_with_progress<F>(
    clients: &HttpClients,
    category: ClientCategory,
    url: &str,
    on_progress: F,
) -> LauncherResult<Vec<u8>>
where
    F: FnMut(u64, Option<u64>),
{
    checked_get_bytes_inner(clients, category, url, HostPolicy::Allowlist, on_progress).await
}

async fn checked_get_bytes_inner<F>(
    clients: &HttpClients,
    category: ClientCategory,
    url: &str,
    policy: HostPolicy<'_>,
    mut on_progress: F,
) -> LauncherResult<Vec<u8>>
where
    F: FnMut(u64, Option<u64>),
{
    let mut response = checked_request_with_policy(clients, category, url, policy).await?;

    if !response.status().is_success() {
        log_response_error(&response, &format!("{category:?} GET"));
        return Err(LauncherError::Generic {
            code: "ERR_HTTP_STATUS".into(),
            message: format!(
                "HTTP {} for {}",
                response.status(),
                network::sanitized_url_for_log(url)
            ),
        });
    }

    let max = category.max_response_bytes().unwrap_or(10 * 1024 * 1024) as usize;

    // Pre-check content-length header if available (fast reject).
    if let Some(cl) = response.content_length() {
        if cl as usize > max {
            return Err(LauncherError::Generic {
                code: "ERR_RESPONSE_TOO_LARGE".into(),
                message: format!("Content-Length {cl} exceeds maximum {max}"),
            });
        }
    }

    let cap = response.content_length().unwrap_or(0).min(max as u64) as usize;
    let mut data = Vec::with_capacity(cap);
    let mut total = 0usize;
    let content_length = response.content_length();
    on_progress(0, content_length);
    loop {
        let chunk = response.chunk().await.map_err(|e| LauncherError::Generic {
            code: "ERR_NETWORK".into(),
            message: format!("Failed to read response chunk: {e}"),
        })?;
        let Some(chunk) = chunk else { break };
        total = total.saturating_add(chunk.len());
        if total > max {
            return Err(LauncherError::Generic {
                code: "ERR_RESPONSE_TOO_LARGE".into(),
                message: format!("Response exceeds {max} bytes (read {total})"),
            });
        }
        data.extend_from_slice(&chunk);
        on_progress(total as u64, content_length);
    }
    Ok(data)
}

/// Read an already-validated response with the category size limit.
pub async fn checked_response_bytes(
    mut response: reqwest::Response,
    category: ClientCategory,
) -> LauncherResult<Vec<u8>> {
    let max = category.max_response_bytes().unwrap_or(10 * 1024 * 1024) as usize;
    if response
        .content_length()
        .is_some_and(|length| length as usize > max)
    {
        return Err(LauncherError::Generic {
            code: "ERR_RESPONSE_TOO_LARGE".into(),
            message: format!("Content-Length exceeds maximum {max}"),
        });
    }
    let mut data = Vec::new();
    let mut total = 0usize;
    loop {
        let chunk = response.chunk().await.map_err(|e| LauncherError::Generic {
            code: "ERR_NETWORK".into(),
            message: format!("Failed to read response chunk: {e}"),
        })?;
        let Some(chunk) = chunk else { break };
        total = total.saturating_add(chunk.len());
        if total > max {
            return Err(LauncherError::Generic {
                code: "ERR_RESPONSE_TOO_LARGE".into(),
                message: format!("Response exceeds {max} bytes"),
            });
        }
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}

/// Read a checked response as UTF-8 with the category size limit.
pub async fn checked_response_text(
    response: reqwest::Response,
    category: ClientCategory,
) -> LauncherResult<String> {
    let bytes = checked_response_bytes(response, category).await?;
    String::from_utf8(bytes).map_err(|e| LauncherError::Generic {
        code: "ERR_RESPONSE_ENCODING".into(),
        message: format!("Response was not valid UTF-8: {e}"),
    })
}

/// Perform a GET with full URL validation, custom headers, and per-hop redirects.
///
/// Same as [`checked_request`] but accepts extra headers to inject
/// (e.g. `Authorization`, `Accept`). Headers are applied to the initial
/// request only; redirects get no custom headers (auth tokens are scoped
/// to the initial endpoint).
pub async fn checked_request_with_headers(
    clients: &HttpClients,
    category: ClientCategory,
    url: &str,
    headers: Vec<(String, String)>,
) -> LauncherResult<reqwest::Response> {
    let _validated = check_request_url(category, url)?;
    let client = clients.get(category);

    let mut remaining_redirects: u8 = 10;
    let mut current_url = url.to_string();
    let mut first = true;

    loop {
        let mut req = client.get(&current_url);
        if first {
            for (k, v) in &headers {
                req = req.header(k.as_str(), v.as_str());
            }
        }

        let response = req.send().await.map_err(|e| LauncherError::Generic {
            code: "ERR_NETWORK".into(),
            message: format!(
                "HTTP GET failed for {}: {e}",
                network::sanitized_url_for_log(&current_url)
            ),
        })?;

        if response.status().is_redirection() {
            if remaining_redirects == 0 {
                return Err(LauncherError::Generic {
                    code: "ERR_TOO_MANY_REDIRECTS".into(),
                    message: format!(
                        "Too many redirects for {}",
                        network::sanitized_url_for_log(url)
                    ),
                });
            }

            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| LauncherError::Generic {
                    code: "ERR_REDIRECT_NO_LOCATION".into(),
                    message: "Redirect without Location header".into(),
                })?;

            let next_url = reqwest::Url::parse(&current_url)
                .ok()
                .and_then(|base| {
                    reqwest::Url::parse(location)
                        .or_else(|_| base.join(location))
                        .ok()
                })
                .ok_or_else(|| LauncherError::Generic {
                    code: "ERR_INVALID_REDIRECT".into(),
                    message: format!("Cannot resolve redirect Location: {location}"),
                })?;

            let _ = check_request_url(category, next_url.as_str())?;
            remaining_redirects -= 1;
            current_url = next_url.to_string();
            first = false;
            continue;
        }

        return Ok(response);
    }
}

/// Blocking GET with full URL validation and per-hop redirect re-validation.
///
/// Uses `reqwest::blocking::Client` internally with the same policy as
/// [`checked_request`]. Use this in synchronous code paths only.
pub fn blocking_checked_request(
    _clients: &HttpClients,
    category: ClientCategory,
    url: &str,
) -> LauncherResult<reqwest::blocking::Response> {
    blocking_checked_request_with_policy(_clients, category, url, HostPolicy::Allowlist)
}

/// Blocking GET with full URL validation under an explicit host policy.
///
/// Same as [`blocking_checked_request`] but host authorization uses `policy`
/// instead of the category allowlist (used by consented-content imports that
/// run inside blocking workers).
pub fn blocking_checked_request_with_policy(
    _clients: &HttpClients,
    category: ClientCategory,
    url: &str,
    policy: HostPolicy<'_>,
) -> LauncherResult<reqwest::blocking::Response> {
    let _validated = check_request_url_with_policy(category, url, policy)?;

    // Build a blocking client with the same security posture.
    let client = reqwest::blocking::Client::builder()
        .timeout(category.timeout())
        .user_agent(category.user_agent())
        .redirect(reqwest::redirect::Policy::none())
        .pool_max_idle_per_host(4)
        .build()
        .map_err(|e| LauncherError::Generic {
            code: "ERR_HTTP_CLIENT_BUILD".into(),
            message: format!("Failed to build blocking HTTP client: {e}"),
        })?;

    let mut remaining_redirects: u8 = 10;
    let mut current_url = url.to_string();

    loop {
        let response = client
            .get(&current_url)
            .send()
            .map_err(|e| LauncherError::Generic {
                code: "ERR_NETWORK".into(),
                message: format!(
                    "HTTP GET failed for {}: {e}",
                    network::sanitized_url_for_log(url)
                ),
            })?;

        if response.status().is_redirection() {
            if remaining_redirects == 0 {
                return Err(LauncherError::Generic {
                    code: "ERR_TOO_MANY_REDIRECTS".into(),
                    message: format!(
                        "Too many redirects for {}",
                        network::sanitized_url_for_log(url)
                    ),
                });
            }

            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| LauncherError::Generic {
                    code: "ERR_REDIRECT_NO_LOCATION".into(),
                    message: "Redirect without Location header".into(),
                })?;

            let next_url = reqwest::Url::parse(&current_url)
                .ok()
                .and_then(|base| {
                    reqwest::Url::parse(location)
                        .or_else(|_| base.join(location))
                        .ok()
                })
                .ok_or_else(|| LauncherError::Generic {
                    code: "ERR_INVALID_REDIRECT".into(),
                    message: format!("Cannot resolve redirect Location: {location}"),
                })?;

            let _ = check_request_url_with_policy(category, next_url.as_str(), policy)?;
            remaining_redirects -= 1;
            current_url = next_url.to_string();
            continue;
        }

        return Ok(response);
    }
}

/// Blocking GET returning validated bytes with size enforcement.
pub fn blocking_checked_get_bytes(
    clients: &HttpClients,
    category: ClientCategory,
    url: &str,
) -> LauncherResult<Vec<u8>> {
    blocking_checked_get_bytes_with_policy(clients, category, url, HostPolicy::Allowlist)
}

/// Blocking GET returning validated bytes with size enforcement under an
/// explicit host policy.
pub fn blocking_checked_get_bytes_with_policy(
    clients: &HttpClients,
    category: ClientCategory,
    url: &str,
    policy: HostPolicy<'_>,
) -> LauncherResult<Vec<u8>> {
    let response = blocking_checked_request_with_policy(clients, category, url, policy)?;

    if !response.status().is_success() {
        return Err(LauncherError::Generic {
            code: "ERR_HTTP_STATUS".into(),
            message: format!(
                "HTTP {} for {}",
                response.status(),
                network::sanitized_url_for_log(url)
            ),
        });
    }

    let max = category.max_response_bytes().unwrap_or(10 * 1024 * 1024);
    let mut limited = response.take(max.saturating_add(1));
    let mut bytes = Vec::new();
    limited
        .read_to_end(&mut bytes)
        .map_err(|e| LauncherError::Generic {
            code: "ERR_NETWORK".into(),
            message: format!("Failed to read response body: {e}"),
        })?;
    if bytes.len() as u64 > max {
        return Err(LauncherError::Generic {
            code: "ERR_RESPONSE_TOO_LARGE".into(),
            message: format!(
                "Response exceeds {} bytes (downloaded {})",
                max,
                bytes.len()
            ),
        });
    }
    Ok(bytes.to_vec())
}

/// Log a sanitized summary of an HTTP response for diagnostics.
pub fn log_response_error(response: &reqwest::Response, context: &str) {
    let status = response.status();
    let url = response.url().as_str();
    eprintln!(
        "[http_client] {context}: HTTP {status} for {url}",
        context = context,
        status = status,
        url = network::sanitized_url_for_log(url),
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_succeeds() {
        let clients = HttpClients::new();
        assert!(clients.is_ok());
    }

    #[test]
    fn test_for_testing_does_not_panic() {
        let client = reqwest::Client::new();
        let clients = HttpClients::for_testing(client);
        assert!(clients.get(ClientCategory::GitHub) as *const _ as usize > 0);
    }

    #[test]
    fn test_categories_have_timeouts() {
        for cat in &[
            ClientCategory::MojangMetadata,
            ClientCategory::MojangContent,
            ClientCategory::Loader,
            ClientCategory::Modrinth,
            ClientCategory::Modpack,
            ClientCategory::GitHub,
            ClientCategory::Microsoft,
            ClientCategory::Registry,
            ClientCategory::AiAssistant,
            ClientCategory::PinnedArtifact,
            ClientCategory::ConsentedContent,
        ] {
            assert!(
                cat.timeout() >= Duration::from_secs(1),
                "{:?} timeout too short",
                cat
            );
        }
    }

    #[test]
    fn test_modpack_category_allows_large_pack_entries() {
        assert_eq!(
            ClientCategory::Modpack.max_response_bytes(),
            Some(500 * 1024 * 1024)
        );
        assert!(
            ClientCategory::Modpack.max_response_bytes()
                > ClientCategory::Modrinth.max_response_bytes()
        );
    }

    // ------------------------------------------------------------------
    // URL validation tests
    // ------------------------------------------------------------------

    #[test]
    fn test_check_rejects_http() {
        let err =
            check_request_url(ClientCategory::GitHub, "http://github.com/file.jar").unwrap_err();
        assert_eq!(err.code(), "ERR_HTTP_SCHEME");
    }

    #[test]
    fn test_check_rejects_non_standard_port() {
        let err = check_request_url(ClientCategory::GitHub, "https://github.com:8080/file.jar")
            .unwrap_err();
        assert_eq!(err.code(), "ERR_HTTP_PORT");
    }

    #[test]
    fn test_check_rejects_userinfo() {
        let err = check_request_url(
            ClientCategory::GitHub,
            "https://user:pass@github.com/file.jar",
        )
        .unwrap_err();
        assert_eq!(err.code(), "ERR_HTTP_USERINFO");
    }

    #[test]
    fn test_check_rejects_ip_literal() {
        let err =
            check_request_url(ClientCategory::GitHub, "https://192.168.1.1/file.jar").unwrap_err();
        assert_eq!(err.code(), "ERR_HTTP_IP_LITERAL");
    }

    #[test]
    fn test_check_rejects_loopback() {
        let err =
            check_request_url(ClientCategory::GitHub, "https://127.0.0.1/file.jar").unwrap_err();
        assert_eq!(err.code(), "ERR_HTTP_IP_LITERAL");
    }

    #[test]
    fn test_check_allows_legitimate_github() {
        assert!(check_request_url(
            ClientCategory::GitHub,
            "https://github.com/owner/repo/releases/download/v1/file.jar"
        )
        .is_ok());
    }

    #[test]
    fn test_check_allows_legitimate_mojang() {
        assert!(check_request_url(
            ClientCategory::MojangMetadata,
            "https://piston-meta.mojang.com/manifest.json"
        )
        .is_ok());
    }

    #[test]
    fn test_check_rejects_unknown_host() {
        let err = check_request_url(
            ClientCategory::GitHub,
            "https://evil.example.com/malware.jar",
        )
        .unwrap_err();
        assert_eq!(err.code(), "ERR_HTTP_HOST_NOT_ALLOWED");
    }

    #[test]
    fn test_check_rejects_category_mismatch() {
        // github.com is in GitHub allowlist, not Modrinth.
        let err = check_request_url(
            ClientCategory::Modrinth,
            "https://github.com/owner/repo/file.jar",
        )
        .unwrap_err();
        assert_eq!(err.code(), "ERR_HTTP_HOST_NOT_ALLOWED");
    }

    #[test]
    fn test_check_allows_subdomain_of_allowlisted_host() {
        // Subdomains of allowlisted hosts are allowed (e.g., github.com
        // covers *.github.com since GitHub controls all its subdomains).
        assert!(check_request_url(
            ClientCategory::GitHub,
            "https://evil.github.com/malware.jar"
        )
        .is_ok());
    }

    #[test]
    fn test_check_rejects_host_not_in_allowlist_at_all() {
        // A completely unrelated host should be rejected.
        let err = check_request_url(ClientCategory::GitHub, "https://not-github.com/file.jar")
            .unwrap_err();
        assert_eq!(err.code(), "ERR_HTTP_HOST_NOT_ALLOWED");
    }

    #[test]
    fn test_check_allows_subdomain() {
        // objects.githubusercontent.com IS in the GitHub allowlist.
        assert!(check_request_url(
            ClientCategory::GitHub,
            "https://objects.githubusercontent.com/asset.zip"
        )
        .is_ok());
    }

    #[test]
    fn test_check_rejects_invalid_url() {
        let err = check_request_url(ClientCategory::GitHub, "not a url").unwrap_err();
        assert_eq!(err.code(), "ERR_INVALID_URL");
    }

    #[test]
    fn test_ai_assistant_rejects_unapproved_host() {
        // Only the explicitly approved Copilot hosts are accepted.
        let err = check_request_url(
            ClientCategory::AiAssistant,
            "https://api.openai.com/v1/chat/completions",
        )
        .unwrap_err();
        assert_eq!(err.code(), "ERR_HTTP_HOST_NOT_ALLOWED");
    }

    // ------------------------------------------------------------------
    // SignedManifest host policy (Phase 0: direct_hash fetching)
    // ------------------------------------------------------------------

    #[test]
    fn test_pinned_artifact_allowlist_is_empty() {
        assert!(
            category_allowlist(ClientCategory::PinnedArtifact).is_empty(),
            "PinnedArtifact hosts come from the signed manifest, never a compile-time list"
        );
    }

    #[test]
    fn test_pinned_artifact_with_allowlist_policy_rejects_everything() {
        // The empty-list-rejects-everything property: reaching PinnedArtifact
        // without a SignedManifest policy fails closed on every host.
        let err = check_request_url(ClientCategory::PinnedArtifact, "https://github.com/x.jar")
            .unwrap_err();
        assert_eq!(err.code(), "ERR_HTTP_HOST_NOT_ALLOWED");
        let err = check_request_url_with_policy(
            ClientCategory::PinnedArtifact,
            "https://cdn.modrinth.com/x.jar",
            HostPolicy::Allowlist,
        )
        .unwrap_err();
        assert_eq!(err.code(), "ERR_HTTP_HOST_NOT_ALLOWED");
    }

    #[test]
    fn test_signed_manifest_policy_allows_pinned_host() {
        assert!(check_request_url_with_policy(
            ClientCategory::PinnedArtifact,
            "https://developer.com/releases/mod-v1.0.0.jar",
            HostPolicy::SignedManifest("developer.com"),
        )
        .is_ok());
    }

    #[test]
    fn test_signed_manifest_policy_allows_pinned_host_subdomain() {
        assert!(check_request_url_with_policy(
            ClientCategory::PinnedArtifact,
            "https://files.developer.com/releases/mod-v1.0.0.jar",
            HostPolicy::SignedManifest("developer.com"),
        )
        .is_ok());
    }

    #[test]
    fn test_signed_manifest_policy_rejects_other_host() {
        let err = check_request_url_with_policy(
            ClientCategory::PinnedArtifact,
            "https://evil.example.com/malware.jar",
            HostPolicy::SignedManifest("developer.com"),
        )
        .unwrap_err();
        assert_eq!(err.code(), "ERR_HTTP_HOST_NOT_ALLOWED");
    }

    #[test]
    fn test_signed_manifest_policy_still_rejects_plain_http() {
        let err = check_request_url_with_policy(
            ClientCategory::PinnedArtifact,
            "http://developer.com/releases/mod-v1.0.0.jar",
            HostPolicy::SignedManifest("developer.com"),
        )
        .unwrap_err();
        assert_eq!(err.code(), "ERR_HTTP_SCHEME");
    }

    #[test]
    fn test_signed_manifest_policy_still_rejects_non_443_port() {
        let err = check_request_url_with_policy(
            ClientCategory::PinnedArtifact,
            "https://developer.com:8080/releases/mod-v1.0.0.jar",
            HostPolicy::SignedManifest("developer.com"),
        )
        .unwrap_err();
        assert_eq!(err.code(), "ERR_HTTP_PORT");
    }

    #[test]
    fn test_signed_manifest_policy_still_rejects_raw_ip() {
        // An IP literal is rejected before the policy is consulted, so a
        // pinned "host" cannot be an IP literal either.
        let err = check_request_url_with_policy(
            ClientCategory::PinnedArtifact,
            "https://192.99.59.1/mod.jar",
            HostPolicy::SignedManifest("192.99.59.1"),
        )
        .unwrap_err();
        assert_eq!(err.code(), "ERR_HTTP_IP_LITERAL");
    }

    #[test]
    fn test_signed_manifest_policy_still_rejects_loopback_resolution() {
        // SSRF floor: even a curatively pinned hostname must not resolve to a
        // private/loopback address.
        let err = check_request_url_with_policy(
            ClientCategory::PinnedArtifact,
            "https://localhost/mod.jar",
            HostPolicy::SignedManifest("localhost"),
        )
        .unwrap_err();
        assert_eq!(err.code(), "ERR_SSRF_BLOCKED");
    }

    #[test]
    fn blocked_ip_covers_ipv6_private_ranges() {
        use std::net::{Ipv4Addr, Ipv6Addr};

        // Unique local (fc00::/7) and link-local unicast (fe80::/10).
        assert!(is_blocked_ip(IpAddr::V6(
            "fd00::1".parse::<Ipv6Addr>().unwrap()
        )));
        assert!(is_blocked_ip(IpAddr::V6(
            "fc00::1".parse::<Ipv6Addr>().unwrap()
        )));
        assert!(is_blocked_ip(IpAddr::V6(
            "fe80::1".parse::<Ipv6Addr>().unwrap()
        )));
        // Loopback and unspecified.
        assert!(is_blocked_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert!(is_blocked_ip(IpAddr::V6(Ipv6Addr::UNSPECIFIED)));
        // An IPv4-mapped address must be judged by the IPv4 rules, not waved
        // through for being v6-shaped.
        assert!(is_blocked_ip(IpAddr::V6(
            "::ffff:127.0.0.1".parse::<Ipv6Addr>().unwrap()
        )));
        assert!(is_blocked_ip(IpAddr::V6(
            "::ffff:192.168.1.1".parse::<Ipv6Addr>().unwrap()
        )));
        // A global unicast v6 address is still allowed.
        assert!(!is_blocked_ip(IpAddr::V6(
            "2606:4700:4700::1111".parse::<Ipv6Addr>().unwrap()
        )));
        // IPv4 behaviour is unchanged.
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(169, 254, 1, 1))));
        assert!(!is_blocked_ip(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));
    }

    #[test]
    fn test_signed_manifest_policy_applies_to_redirect_hops() {
        // The redirect re-validation uses the same policy as the initial URL.
        // An off-domain Location header must be rejected exactly like the
        // non-pinned allowlist path rejects unknown hosts.
        let err = check_request_url_with_policy(
            ClientCategory::PinnedArtifact,
            "https://cdn.evil.example.com/redirect.jar",
            HostPolicy::SignedManifest("developer.com"),
        )
        .unwrap_err();
        assert_eq!(err.code(), "ERR_HTTP_HOST_NOT_ALLOWED");
    }

    // ------------------------------------------------------------------
    // ConsentedContent policy (Phase 3: technic tiers S/Z)
    // ------------------------------------------------------------------

    #[test]
    fn test_consented_content_allowlist_is_empty() {
        assert!(
            category_allowlist(ClientCategory::ConsentedContent).is_empty(),
            "ConsentedContent hosts come from explicit user consent, never a compile-time list"
        );
    }

    #[test]
    fn test_consented_content_fails_closed_without_consent_policy() {
        // Reaching the category through the generic Allowlist path must reject
        // every host, even with HTTP/non-443 relaxation in place.
        let err = check_request_url(
            ClientCategory::ConsentedContent,
            "http://dropbox.com/anything",
        )
        .unwrap_err();
        assert_eq!(err.code(), "ERR_HTTP_HOST_NOT_ALLOWED");
    }

    #[test]
    fn test_consented_content_allows_http_and_non_443_under_consent() {
        assert!(check_request_url_with_policy(
            ClientCategory::ConsentedContent,
            "http://example.com/pack.zip",
            HostPolicy::UserConsented,
        )
        .is_ok());
        assert!(check_request_url_with_policy(
            ClientCategory::ConsentedContent,
            "http://example.com:8080/pack.zip",
            HostPolicy::UserConsented,
        )
        .is_ok());
        assert!(check_request_url_with_policy(
            ClientCategory::ConsentedContent,
            "https://example.com/pack.zip",
            HostPolicy::UserConsented,
        )
        .is_ok());
    }

    #[test]
    fn test_consented_content_still_rejects_raw_ip() {
        let err = check_request_url_with_policy(
            ClientCategory::ConsentedContent,
            "http://192.99.59.1/pack.zip",
            HostPolicy::UserConsented,
        )
        .unwrap_err();
        assert_eq!(err.code(), "ERR_HTTP_IP_LITERAL");
    }

    #[test]
    fn test_consented_content_still_rejects_private_resolution() {
        // SSRF floor retained even for consented plain-HTTP content.
        let err = check_request_url_with_policy(
            ClientCategory::ConsentedContent,
            "http://localhost:8080/pack.zip",
            HostPolicy::UserConsented,
        )
        .unwrap_err();
        assert_eq!(err.code(), "ERR_SSRF_BLOCKED");
    }

    #[test]
    fn test_consented_content_still_rejects_userinfo() {
        let err = check_request_url_with_policy(
            ClientCategory::ConsentedContent,
            "http://user:pass@example.com/pack.zip",
            HostPolicy::UserConsented,
        )
        .unwrap_err();
        assert_eq!(err.code(), "ERR_HTTP_USERINFO");
    }

    // ------------------------------------------------------------------
    // Allowlist contents
    // ------------------------------------------------------------------

    #[test]
    fn test_category_allowlist_nonempty() {
        assert!(!category_allowlist(ClientCategory::MojangMetadata).is_empty());
        assert!(!category_allowlist(ClientCategory::GitHub).is_empty());
        assert!(!category_allowlist(ClientCategory::Microsoft).is_empty());
        assert!(!category_allowlist(ClientCategory::Modrinth).is_empty());
        assert!(!category_allowlist(ClientCategory::Modpack).is_empty());
        assert!(!category_allowlist(ClientCategory::Loader).is_empty());
        assert!(!category_allowlist(ClientCategory::MojangContent).is_empty());
        assert!(!category_allowlist(ClientCategory::Registry).is_empty());
        assert!(!category_allowlist(ClientCategory::AiAssistant).is_empty());
    }
}
