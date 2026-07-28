//! Governance sandbox readiness - Tauri command adapters and async network checks.
//!
//! Pure logic in `agora_core::governance`; this module adds read-only GitHub
//! API checks for diagnostics (checks 5-13 from the spec).
//!
//! # Network checks (read-only)
//! - `repository_metadata_readable` - GET /repos/:owner/:repo
//! - `issues_enabled` - check repo features from metadata
//! - `discussions_enabled` - check repo features from metadata
//! - `triage_category_exists` - GraphQL query for DiscussionCategory
//! - Required labels and Issue forms via read-only REST requests
//!
//! All checks are read-only. No issues, comments, or reactions are created.

use crate::error::{LauncherError, LauncherResult};

use agora_core::governance::GovernanceService;
pub use agora_core::governance::{
    resolve_github_app_slug, resolve_governance_config as core_resolve_config,
    resolve_governance_environment, resolve_governance_repo,
    run_governance_diagnostics as core_sync_checks, DiagnosticCheck, DiagnosticStatus,
    GovernanceConfig, GovernanceEnvironment, GovernanceEvent, GovernanceSummary, TriagePoll,
};
use agora_core::registry::RegistryService;
use tauri::AppHandle;

// --- Config ---

/// Return the resolved governance configuration.
pub fn get_governance_config(app: &AppHandle) -> GovernanceConfig {
    let ctx = match crate::core_context(app) {
        Ok(c) => c,
        Err(_) => return core_resolve_config(None, None),
    };
    let registry = RegistryService::new(ctx);
    let svc = GovernanceService::new(registry);
    svc.config()
}

// --- Summary (per-item) ---

/// Fetch the governance summary for a single item.
pub fn get_governance_summary(
    app: &AppHandle,
    item_id: &str,
) -> LauncherResult<Option<GovernanceSummary>> {
    let ctx = crate::core_context(app)?;
    let registry = RegistryService::new(ctx);
    let svc = GovernanceService::new(registry);
    svc.get_governance_summary(item_id)
}

// --- Events ---

/// List governance events, optionally filtered by item_id.
pub fn list_governance_events(
    app: &AppHandle,
    item_id: Option<&str>,
) -> LauncherResult<Vec<GovernanceEvent>> {
    let ctx = crate::core_context(app)?;
    let registry = RegistryService::new(ctx);
    let svc = GovernanceService::new(registry);
    svc.list_governance_events(item_id, 100)
}

// --- Diagnostics (sync) ---

/// Run sync governance diagnostics (core checks only).
pub fn run_sync_diagnostics(app: &AppHandle) -> Vec<DiagnosticCheck> {
    let ctx = match crate::core_context(app) {
        Ok(ctx) => ctx,
        Err(_) => return Vec::new(),
    };
    let registry = RegistryService::new(ctx);
    let svc = GovernanceService::new(registry);
    svc.run_diagnostics()
}

// --- Diagnostics (async network checks) ---

/// Run async network-based diagnostic checks.
///
/// All checks are read-only GitHub API calls.  Every API call that returns 401
/// triggers exactly one refresh-and-retry before reporting a permanent failure.
pub async fn run_network_diagnostics(app: &AppHandle) -> Vec<DiagnosticCheck> {
    match crate::auth::get_valid_access_token(app).await {
        Some(_) => {}
        None => {
            return vec![
                DiagnosticCheck {
                    id: "repository_metadata_readable".into(),
                    status: DiagnosticStatus::Fail,
                    message: "Cannot check: no GitHub token available".into(),
                },
                DiagnosticCheck {
                    id: "issues_enabled".into(),
                    status: DiagnosticStatus::Warning,
                    message: "Cannot check: no GitHub token".into(),
                },
                DiagnosticCheck {
                    id: "discussions_enabled".into(),
                    status: DiagnosticStatus::Warning,
                    message: "Cannot check: no GitHub token".into(),
                },
                DiagnosticCheck {
                    id: "triage_category_exists".into(),
                    status: DiagnosticStatus::Warning,
                    message: "Cannot check: no GitHub token".into(),
                },
                DiagnosticCheck {
                    id: "community_review_label_exists".into(),
                    status: DiagnosticStatus::Warning,
                    message: "Cannot check: no GitHub token".into(),
                },
                DiagnosticCheck {
                    id: "registry_submission_label_exists".into(),
                    status: DiagnosticStatus::Warning,
                    message: "Cannot check: no GitHub token".into(),
                },
                DiagnosticCheck {
                    id: "registry_vote_label_exists".into(),
                    status: DiagnosticStatus::Warning,
                    message: "Cannot check: no GitHub token".into(),
                },
                DiagnosticCheck {
                    id: "review_form_exists".into(),
                    status: DiagnosticStatus::Warning,
                    message: "Cannot check: no GitHub token".into(),
                },
                DiagnosticCheck {
                    id: "mod_submission_exists".into(),
                    status: DiagnosticStatus::Warning,
                    message: "Cannot check: no GitHub token".into(),
                },
            ];
        }
    };

    let repo = resolve_governance_repo();
    let client = agora_core::github_ratelimit::github_client();

    let mut checks = Vec::new();
    let (owner, repo_name) = repo.split_once('/').unwrap_or(("", ""));

    // ---- Helper: execute an API call with 401 refresh-and-retry ----
    async fn call_with_401_retry<T, F, Fut>(f: F) -> Result<T, String>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<T, String>>,
    {
        let first = f().await;
        match &first {
            Err(msg) if msg.contains("401") => {
                eprintln!("[governance] 401 on API call; attempting token refresh");
                if agora_core::auth::try_refresh_after_401(LauncherError::AuthExpired)
                    .await
                    .is_ok()
                {
                    return f().await;
                }
                first
            }
            _ => first,
        }
    }

    // 5. repository_metadata_readable (also yields issues + discussions)
    match call_with_401_retry(|| get_repo_meta(client, &repo)).await {
        Ok(meta) => {
            checks.push(DiagnosticCheck {
                id: "repository_metadata_readable".into(),
                status: DiagnosticStatus::Pass,
                message: format!("Repository {} is accessible with token", repo),
            });
            checks.push(DiagnosticCheck {
                id: "issues_enabled".into(),
                status: if meta.has_issues {
                    DiagnosticStatus::Pass
                } else {
                    DiagnosticStatus::Warning
                },
                message: if meta.has_issues {
                    "Issues are enabled on the governance repository".into()
                } else {
                    "Issues are disabled; governance vote issues require issues enabled".into()
                },
            });
            checks.push(DiagnosticCheck {
                id: "discussions_enabled".into(),
                status: if meta.has_discussions {
                    DiagnosticStatus::Pass
                } else {
                    DiagnosticStatus::Warning
                },
                message: if meta.has_discussions {
                    "Discussions are enabled on the governance repository".into()
                } else {
                    "Discussions are disabled; triage polls require discussions enabled".into()
                },
            });
        }
        Err(msg) => {
            checks.push(DiagnosticCheck {
                id: "repository_metadata_readable".into(),
                status: DiagnosticStatus::Fail,
                message: msg.clone(),
            });
            checks.push(DiagnosticCheck {
                id: "issues_enabled".into(),
                status: DiagnosticStatus::Warning,
                message: format!("Cannot check issues: {msg}"),
            });
            checks.push(DiagnosticCheck {
                id: "discussions_enabled".into(),
                status: DiagnosticStatus::Warning,
                message: format!("Cannot check discussions: {msg}"),
            });
        }
    }

    // 8. triage_category_exists
    match call_with_401_retry(|| check_triage_category(client, owner, repo_name)).await {
        Ok(true) => checks.push(DiagnosticCheck {
            id: "triage_category_exists".into(),
            status: DiagnosticStatus::Pass,
            message: "Triage discussion category exists on the repository".into(),
        }),
        Ok(false) => checks.push(DiagnosticCheck {
            id: "triage_category_exists".into(),
            status: DiagnosticStatus::Warning,
            message:
                "Triage discussion category not found; triage polls require a \"Triage\" category"
                    .into(),
        }),
        Err(msg) => checks.push(DiagnosticCheck {
            id: "triage_category_exists".into(),
            status: DiagnosticStatus::Warning,
            message: format!("Cannot check triage category: {msg}"),
        }),
    }

    // 9-11. Label existence checks
    let known_labels: Option<Vec<String>> =
        match call_with_401_retry(|| list_repo_labels(client, owner, repo_name)).await {
            Ok(labels) => Some(labels),
            Err(msg) => {
                for (check_id, label) in [
                    ("community_review_label_exists", "community-review"),
                    ("registry_submission_label_exists", "registry-submission"),
                    ("registry_vote_label_exists", "registry-vote"),
                ] {
                    checks.push(DiagnosticCheck {
                        id: check_id.into(),
                        status: DiagnosticStatus::Warning,
                        message: format!("Cannot check \"{label}\" label: {msg}"),
                    });
                }
                None
            }
        };

    if let Some(known_labels) = known_labels {
        for (check_id, label_name) in [
            ("community_review_label_exists", "community-review"),
            ("registry_submission_label_exists", "registry-submission"),
            ("registry_vote_label_exists", "registry-vote"),
        ] {
            let found = known_labels.iter().any(|l| l == label_name);
            checks.push(DiagnosticCheck {
                id: check_id.into(),
                status: if found {
                    DiagnosticStatus::Pass
                } else {
                    DiagnosticStatus::Warning
                },
                message: if found {
                    format!("\"{label_name}\" label exists on the repository")
                } else {
                    format!("\"{label_name}\" label not found; governance workflows require it")
                },
            });
        }
    }

    // 12-13. Issue template existence checks
    for (check_id, template_path, display) in [
        (
            "review_form_exists",
            ".github/ISSUE_TEMPLATE/review-form.yml",
            "review-form.yml",
        ),
        (
            "mod_submission_exists",
            ".github/ISSUE_TEMPLATE/mod-submission.yml",
            "mod-submission.yml",
        ),
    ] {
        let tmpl_result = call_with_401_retry(|| {
            let path = template_path.to_owned();
            async move { check_template_exists(client, owner, repo_name, &path).await }
        })
        .await;
        match tmpl_result {
            Ok(true) => checks.push(DiagnosticCheck {
                id: check_id.into(),
                status: DiagnosticStatus::Pass,
                message: format!("\"{display}\" issue template exists"),
            }),
            Ok(false) => checks.push(DiagnosticCheck {
                id: check_id.into(),
                status: DiagnosticStatus::Warning,
                message: format!("\"{display}\" issue template not found"),
            }),
            Err(msg) => checks.push(DiagnosticCheck {
                id: check_id.into(),
                status: DiagnosticStatus::Warning,
                message: format!("Cannot check \"{display}\": {msg}"),
            }),
        }
    }

    checks
}

// ---------------------------------------------------------------------------
// GitHub API helpers (read-only)
// ---------------------------------------------------------------------------

struct RepoMeta {
    has_issues: bool,
    has_discussions: bool,
}

/// Fetch repository metadata (issues + discussions enabled).
async fn get_repo_meta(client: &reqwest::Client, repo: &str) -> Result<RepoMeta, String> {
    let token = agora_core::auth::get_token().ok_or("No token available".to_string())?;
    let url = format!("https://api.github.com/repos/{}", repo);
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", "agora-launcher")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("Network error getting repo metadata: {e}"))?;

    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("401".to_string());
    }
    if !resp.status().is_success() {
        return Err(format!("GitHub API returned HTTP {}", resp.status()));
    }

    #[derive(serde::Deserialize)]
    struct RepoResponse {
        has_issues: bool,
        has_discussions: bool,
    }

    let body: RepoResponse =
        serde_json::from_str(&resp.text().await.map_err(|e| format!("Parse error: {e}"))?)
            .map_err(|e| format!("JSON parse error: {e}"))?;

    Ok(RepoMeta {
        has_issues: body.has_issues,
        has_discussions: body.has_discussions,
    })
}

/// Run a GraphQL query to check for a discussion category named "Triage".
async fn check_triage_category(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
) -> Result<bool, String> {
    let token = agora_core::auth::get_token().ok_or("No token available".to_string())?;
    let query = serde_json::json!({
        "query": r#"
            query($owner: String!, $repo: String!) {
                repository(owner: $owner, name: $repo) {
                    discussionCategories(first: 20) {
                        nodes { name slug }
                    }
                }
            }
        "#,
        "variables": { "owner": owner, "repo": repo },
    });

    let resp = client
        .post("https://api.github.com/graphql")
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", "agora-launcher")
        .header("Content-Type", "application/json")
        .json(&query)
        .send()
        .await
        .map_err(|e| format!("Network error checking triage category: {e}"))?;

    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("401".to_string());
    }
    if !resp.status().is_success() {
        return Err(format!("GraphQL query failed: HTTP {}", resp.status()));
    }

    #[derive(serde::Deserialize)]
    struct GraphQLResponse {
        data: Option<GraphQLData>,
    }
    #[derive(serde::Deserialize)]
    struct GraphQLData {
        repository: Option<GraphQLRepo>,
    }
    #[derive(serde::Deserialize)]
    struct GraphQLRepo {
        discussion_categories: Option<GraphQLCategories>,
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct GraphQLCategories {
        nodes: Option<Vec<GraphQLCategory>>,
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct GraphQLCategory {
        name: String,
        slug: String,
    }

    let body: GraphQLResponse =
        serde_json::from_str(&resp.text().await.map_err(|e| format!("Parse error: {e}"))?)
            .map_err(|e| format!("JSON parse error: {e}"))?;

    let categories = body
        .data
        .and_then(|d| d.repository)
        .and_then(|r| r.discussion_categories)
        .and_then(|c| c.nodes)
        .unwrap_or_default();

    Ok(categories.iter().any(|cat| {
        cat.name.eq_ignore_ascii_case("Triage") || cat.slug.eq_ignore_ascii_case("triage")
    }))
}

/// Fetch all label names from the repository.
async fn list_repo_labels(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
) -> Result<Vec<String>, String> {
    let token = agora_core::auth::get_token().ok_or("No token available".to_string())?;
    let url = format!("https://api.github.com/repos/{owner}/{repo}/labels?per_page=100");
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", "agora-launcher")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("Network error listing labels: {e}"))?;

    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("401".to_string());
    }
    if !resp.status().is_success() {
        return Err(format!("GitHub API returned HTTP {}", resp.status()));
    }

    #[derive(serde::Deserialize)]
    struct LabelResponse {
        name: String,
    }

    let body: Vec<LabelResponse> =
        serde_json::from_str(&resp.text().await.map_err(|e| format!("Parse error: {e}"))?)
            .map_err(|e| format!("JSON parse error: {e}"))?;

    Ok(body.into_iter().map(|l| l.name).collect())
}

/// Check whether a specific file exists in the `.github/ISSUE_TEMPLATE/`
/// directory via the read-only Contents API.
async fn check_template_exists(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
    path: &str,
) -> Result<bool, String> {
    let token = agora_core::auth::get_token().ok_or("No token available".to_string())?;
    let url = format!("https://api.github.com/repos/{owner}/{repo}/contents/{path}");
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", "agora-launcher")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("Network error checking template: {e}"))?;

    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("401".to_string());
    }
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(false);
    }
    if !resp.status().is_success() {
        return Err(format!("GitHub API returned HTTP {}", resp.status()));
    }
    Ok(true)
}

// --- Triage poll (read-only GraphQL) ---

pub async fn fetch_triage_poll(app: &AppHandle, mod_id: String) -> LauncherResult<TriagePoll> {
    let mut token = crate::auth::get_valid_access_token(app)
        .await
        .ok_or(LauncherError::AuthRequired)?;

    let _permit = agora_core::github_ratelimit::acquire_github_permit().await;

    let governance_repo_str = resolve_governance_repo();
    let search_query = format!(
        "repo:{owner}/{repo} [Community Triage] {mod_id}",
        owner = governance_repo_str.split('/').next().unwrap_or(""),
        repo = governance_repo_str.split('/').nth(1).unwrap_or(""),
        mod_id = mod_id,
    );

    let search_body = serde_json::json!({
        "query": r#"
            query ($q: String!) {
              search(query: $q, type: DISCUSSION, first: 5) {
                nodes {
                  ... on Discussion {
                    id
                    url
                    title
                  }
                }
              }
            }
        "#,
        "variables": { "q": search_query },
    });

    let mut resp = agora_core::github_ratelimit::github_client()
        .post("https://api.github.com/graphql")
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", "agora-launcher")
        .header("Content-Type", "application/json")
        .json(&search_body)
        .send()
        .await
        .map_err(|_| LauncherError::NetworkOffline)?;

    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        crate::auth::log_line("GitHub token expired during triage poll search; attempting refresh");
        if agora_core::auth::try_refresh_after_401(LauncherError::AuthExpired)
            .await
            .is_ok()
        {
            if let Some(new_token) = crate::auth::get_token(app) {
                token = new_token;
                let retry_resp = agora_core::github_ratelimit::github_client()
                    .post("https://api.github.com/graphql")
                    .header("Authorization", format!("Bearer {}", token))
                    .header("User-Agent", "agora-launcher")
                    .header("Content-Type", "application/json")
                    .json(&search_body)
                    .send()
                    .await
                    .map_err(|_| LauncherError::NetworkOffline)?;
                if retry_resp.status() != reqwest::StatusCode::UNAUTHORIZED {
                    resp = retry_resp;
                } else {
                    let _ = crate::auth::clear_token(app);
                    return Err(LauncherError::AuthExpired);
                }
            }
        } else {
            let _ = crate::auth::clear_token(app);
            return Err(LauncherError::AuthExpired);
        }
    }
    if agora_core::github_ratelimit::is_rate_limit_response(&resp) {
        let retry = agora_core::github_ratelimit::parse_retry_after(&resp);
        agora_core::github_ratelimit::report_rate_limit(retry).await;
        return Err(LauncherError::Generic {
            code: "ERR_RATE_LIMITED".to_string(),
            message: format!("GitHub rate limited triage poll for {mod_id}."),
        });
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(LauncherError::Generic {
            code: "ERR_TRIAGE_POLL".to_string(),
            message: format!("Triage poll search failed (status {status}): {body}"),
        });
    }

    // (remaining fetch_triage_poll is unchanged; see previous impl)
    // Deserialize search results
    #[derive(Debug, serde::Deserialize)]
    struct SearchResponse {
        search: Option<SearchPayload>,
    }
    #[derive(Debug, serde::Deserialize)]
    struct SearchPayload {
        nodes: Option<Vec<DiscussionNode>>,
    }
    #[derive(Debug, serde::Deserialize)]
    struct DiscussionNode {
        id: String,
        url: String,
        title: String,
    }

    let search_resp: SearchResponse = resp.json().await.map_err(|_| LauncherError::Generic {
        code: "ERR_TRIAGE_POLL".to_string(),
        message: "Failed to parse triage poll search response.".to_string(),
    })?;

    let nodes = search_resp.search.and_then(|s| s.nodes).unwrap_or_default();
    let discussion = nodes
        .into_iter()
        .find(|d| d.title.contains(&mod_id))
        .ok_or_else(|| LauncherError::Generic {
            code: "ERR_TRIAGE_POLL".to_string(),
            message: format!("No triage discussion found for mod '{mod_id}'."),
        })?;

    let reactions_body = serde_json::json!({
        "query": r#"
            query ($id: ID!) {
              node(id: $id) {
                ... on Discussion {
                  url
                  reactions(first: 100) {
                    nodes {
                      user { login }
                      content
                    }
                  }
                }
              }
            }
        "#,
        "variables": { "id": discussion.id },
    });

    let mut resp2 = agora_core::github_ratelimit::github_client()
        .post("https://api.github.com/graphql")
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", "agora-launcher")
        .header("Content-Type", "application/json")
        .json(&reactions_body)
        .send()
        .await
        .map_err(|_| LauncherError::NetworkOffline)?;

    if resp2.status() == reqwest::StatusCode::UNAUTHORIZED {
        crate::auth::log_line(
            "GitHub token expired during triage poll reactions; attempting refresh",
        );
        if agora_core::auth::try_refresh_after_401(LauncherError::AuthExpired)
            .await
            .is_ok()
        {
            if let Some(new_token) = crate::auth::get_token(app) {
                token = new_token;
                let retry_resp = agora_core::github_ratelimit::github_client()
                    .post("https://api.github.com/graphql")
                    .header("Authorization", format!("Bearer {}", token))
                    .header("User-Agent", "agora-launcher")
                    .header("Content-Type", "application/json")
                    .json(&reactions_body)
                    .send()
                    .await
                    .map_err(|_| LauncherError::NetworkOffline)?;
                if retry_resp.status() != reqwest::StatusCode::UNAUTHORIZED {
                    resp2 = retry_resp;
                } else {
                    let _ = crate::auth::clear_token(app);
                    return Err(LauncherError::AuthExpired);
                }
            }
        } else {
            let _ = crate::auth::clear_token(app);
            return Err(LauncherError::AuthExpired);
        }
    }
    if agora_core::github_ratelimit::is_rate_limit_response(&resp2) {
        let retry = agora_core::github_ratelimit::parse_retry_after(&resp2);
        agora_core::github_ratelimit::report_rate_limit(retry).await;
        return Err(LauncherError::Generic {
            code: "ERR_RATE_LIMITED".to_string(),
            message: format!("GitHub rate limited triage poll reactions for {mod_id}."),
        });
    }
    if !resp2.status().is_success() {
        let status = resp2.status();
        let body = resp2.text().await.unwrap_or_default();
        return Err(LauncherError::Generic {
            code: "ERR_TRIAGE_POLL".to_string(),
            message: format!("Triage poll reactions failed (status {status}): {body}"),
        });
    }

    #[derive(Debug, serde::Deserialize)]
    struct ReactionsResponse {
        node: Option<ReactionsPayload>,
    }
    #[derive(Debug, serde::Deserialize)]
    struct ReactionsPayload {
        reactions: Option<ReactionsPayloadInner>,
    }
    #[derive(Debug, serde::Deserialize)]
    struct ReactionsPayloadInner {
        nodes: Option<Vec<ReactionNode>>,
    }
    #[derive(Debug, serde::Deserialize)]
    struct ReactionNode {
        content: String,
    }

    let reactions_resp: ReactionsResponse =
        resp2.json().await.map_err(|_| LauncherError::Generic {
            code: "ERR_TRIAGE_POLL".to_string(),
            message: "Failed to parse triage poll reactions response.".to_string(),
        })?;

    let (keep_votes, remove_votes) = reactions_resp
        .node
        .and_then(|n| n.reactions)
        .and_then(|r| r.nodes)
        .map(|nodes| {
            let mut keep = 0i64;
            let mut remove = 0i64;
            for rxn in nodes {
                match rxn.content.as_str() {
                    "THUMBS_UP" | "+1" | "HOORAY" => keep += 1,
                    "THUMBS_DOWN" | "-1" => remove += 1,
                    _ => {}
                }
            }
            (keep, remove)
        })
        .unwrap_or((0, 0));

    Ok(TriagePoll {
        discussion_url: Some(discussion.url),
        keep_votes,
        remove_votes,
    })
}
