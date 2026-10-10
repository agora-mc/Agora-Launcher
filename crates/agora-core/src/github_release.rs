//! The GitHub releases listing, for every game (MASTER_SPEC §26.8).
//!
//! Each game turns the releases into its own candidates: Minecraft reads Minecraft versions out of
//! asset names, and the catalog install takes the newest release with a matching asset. The request,
//! the stored-token retry, the rate limit and the page count are the same for both, so they live
//! here once.

use serde::Deserialize;

use crate::error::{LauncherError, LauncherResult};
use crate::http_client::{self, ClientCategory, HttpClients};

/// One release from `GET /repos/{owner}/{repo}/releases`.
#[derive(Debug, Clone, Deserialize)]
pub struct GitHubRelease {
    pub tag_name: String,
    pub published_at: Option<String>,
    /// A draft is visible only to people with write access; it is never a download to offer.
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    pub assets: Vec<GitHubReleaseAsset>,
}

/// One file attached to a release.
#[derive(Debug, Clone, Deserialize)]
pub struct GitHubReleaseAsset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub size: Option<u64>,
    /// GitHub's published digest, `sha256:<hex>`, when GitHub has one for the file.
    #[serde(default)]
    pub digest: Option<String>,
}

/// How to authenticate a releases request. Releases are public, so a missing or refused token only
/// ever costs the anonymous rate limit, never the listing.
#[derive(Debug, Clone, Default)]
pub struct GitHubAuth {
    /// A bearer token, typically the signed-in user's.
    pub token: Option<String>,
    /// Whether the token came from the OS credential store, so a refused token is forgotten.
    pub clear_stored_on_unauthorized: bool,
}

/// One page of `GET /repos/{source}/releases` (100 per page), and the total number of pages from
/// the `Link` header. `source` is `owner/repo`.
pub async fn list_releases_page(
    clients: &HttpClients,
    source: &str,
    page: u32,
    auth: &GitHubAuth,
) -> LauncherResult<(Vec<GitHubRelease>, u32)> {
    let url = format!("https://api.github.com/repos/{source}/releases?per_page=100&page={page}");

    let headers = github_auth_headers(auth.token.as_deref());
    let mut response = send_releases_request(clients, &url, &headers).await?;

    // Release listings are public. A stale or malformed stored token must not turn a public request
    // into a hard failure, and must not be retried with the same invalid Authorization header.
    if response.status() == reqwest::StatusCode::UNAUTHORIZED && auth.token.is_some() {
        if auth.clear_stored_on_unauthorized {
            // Attempt a single token refresh before falling back to anonymous.
            if crate::auth::try_refresh_after_401_with_token(
                auth.token.as_deref().unwrap_or_default(),
            )
            .await
            .is_ok()
            {
                if let Some(new_token) = crate::auth::get_valid_access_token().await {
                    let new_headers = github_auth_headers(Some(&new_token));
                    response = send_releases_request(clients, &url, &new_headers).await?;
                } else {
                    response = send_releases_request(clients, &url, &[]).await?;
                }
            } else {
                let _ = crate::auth::clear_token();
                response = send_releases_request(clients, &url, &[]).await?;
            }
        } else {
            response = send_releases_request(clients, &url, &[]).await?;
        }
    }

    if crate::github_ratelimit::is_rate_limit_response(&response) {
        let retry = crate::github_ratelimit::parse_retry_after(&response);
        crate::github_ratelimit::report_rate_limit(retry).await;
        return Err(LauncherError::Generic {
            code: "ERR_RATE_LIMITED".into(),
            message: format!("GitHub rate limit hit while fetching releases for {source}."),
        });
    }

    let link_value = response
        .headers()
        .get("link")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let releases: Vec<GitHubRelease> = response
        .error_for_status()
        .map_err(|e| LauncherError::Generic {
            code: "ERR_NETWORK".into(),
            message: format!("GitHub API request failed: {e}"),
        })?
        .json()
        .await
        .map_err(|_| LauncherError::Generic {
            code: "ERR_NETWORK".into(),
            message: "Failed to parse GitHub releases response.".into(),
        })?;

    Ok((releases, parse_link_total_pages(link_value.as_deref())))
}

async fn send_releases_request(
    clients: &HttpClients,
    url: &str,
    headers: &[(String, String)],
) -> LauncherResult<reqwest::Response> {
    let _permit = crate::github_ratelimit::acquire_github_permit().await;
    http_client::checked_send(
        clients,
        ClientCategory::GitHub,
        reqwest::Method::GET,
        url,
        headers,
        None,
        None,
    )
    .await
}

pub fn github_auth_headers(token: Option<&str>) -> Vec<(String, String)> {
    token
        .map(|token| vec![("Authorization".into(), format!("Bearer {token}"))])
        .unwrap_or_default()
}

/// Parse the GitHub API `Link` response header to discover the total number of pages.
pub fn parse_link_total_pages(header_value: Option<&str>) -> u32 {
    let value = match header_value {
        Some(v) => v,
        None => return 1,
    };
    for part in value.split(',') {
        let trimmed = part.trim();
        if trimmed.contains("rel=\"last\"") {
            if let Some(close) = trimmed.rfind('>') {
                let substr = &trimmed[..close];
                if let Some(open) = substr.rfind('<') {
                    let url = &substr[open + 1..];
                    for segment in url.split(&['?', '&'][..]) {
                        if let Some(num) = segment.strip_prefix("page=") {
                            return num.parse::<u32>().unwrap_or(1);
                        }
                    }
                }
            }
        }
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_page_number_comes_from_the_link_header() {
        assert_eq!(
            parse_link_total_pages(Some("<https://api.github.com/repos/owner/repo/releases?page=2>; rel=\"next\", <https://api.github.com/repos/owner/repo/releases?page=5>; rel=\"last\"")),
            5
        );
        assert_eq!(parse_link_total_pages(None), 1);
    }

    #[test]
    fn a_bearer_header_is_sent_only_with_a_token() {
        assert_eq!(
            github_auth_headers(Some("gho_test_token")),
            vec![(
                "Authorization".to_string(),
                "Bearer gho_test_token".to_string()
            )]
        );
        assert!(github_auth_headers(None).is_empty());
    }
}
