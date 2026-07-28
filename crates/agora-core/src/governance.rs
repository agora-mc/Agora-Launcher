//! Governance sandbox readiness: types, environment resolution, diagnostics.
//!
//! All network API calls are read-only (GraphQL queries). No POST/PUT/DELETE.
//! All SQL queries use parameterized statements.
//!
//! Flag review, admin-alert repo creation, and rate-limit paths are removed.

use crate::error::{LauncherError, LauncherResult};
use crate::registry::RegistryService;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// GovernanceEnvironment — compile-time resolution
// ---------------------------------------------------------------------------

/// Governance environment discriminator.
///
/// Resolved at compile time from `AGORA_GOVERNANCE_ENV`:
/// - `"sandbox"` or `"sandbox_dev"` → Sandbox
/// - any other value (or unset) → Production
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GovernanceEnvironment {
    Production,
    Sandbox,
}

// ---------------------------------------------------------------------------
// GovernanceConfig — resolved environment config
// ---------------------------------------------------------------------------

/// Fully resolved governance configuration for the current process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceConfig {
    /// Repository in `owner/repo` form.
    pub repository: String,
    /// Environment discriminator.
    pub environment: GovernanceEnvironment,
    /// GitHub App slug (from `AGORA_GITHUB_APP_SLUG` or None).
    pub github_app_slug: Option<String>,
    /// Whether a valid debug-only dev registry override is active.
    pub development_registry: bool,
}

// ---------------------------------------------------------------------------
// GovernanceSummary — per-item schema-7 governance state
// ---------------------------------------------------------------------------

/// Governance summary for a single registry item from the `governance_summary`
/// table (schema 7+).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceSummary {
    pub item_id: String,
    pub vote_issue_number: Option<i64>,
    pub vote_issue_url: Option<String>,
    pub raw_upvotes: i64,
    pub raw_downvotes: i64,
    pub counted_upvotes: i64,
    pub counted_downvotes: i64,
    pub quarantined_upvotes: i64,
    pub quarantined_downvotes: i64,
    pub conflicted_users: i64,
    pub status_reason: Option<String>,
    pub compiled_at: String,
}

// ---------------------------------------------------------------------------
// GovernanceEvent — schema-7 governance event row
// ---------------------------------------------------------------------------

/// A governance event from the `governance_events` table (schema 7+).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceEvent {
    pub event_id: String,
    pub item_id: String,
    pub event_type: String,
    pub status: String,
    pub detected_at: String,
    pub affected_reactions: i64,
    pub details_json: Option<String>,
}

// ---------------------------------------------------------------------------
// TriagePoll — read-only GitHub Discussions query result
// ---------------------------------------------------------------------------

/// A live triage poll for a given mod, fetched from GitHub Discussions.
#[derive(Debug, Serialize, Clone)]
pub struct TriagePoll {
    pub discussion_url: Option<String>,
    pub keep_votes: i64,
    pub remove_votes: i64,
}

// ---------------------------------------------------------------------------
// DiagnosticCheck / DiagnosticStatus — diagnostics output
// ---------------------------------------------------------------------------

/// Status of a single diagnostic check.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticStatus {
    Pass,
    Warning,
    Fail,
}

/// A single governance diagnostic check result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticCheck {
    pub id: String,
    pub status: DiagnosticStatus,
    pub message: String,
}

// ---------------------------------------------------------------------------
// Environment resolution
// ---------------------------------------------------------------------------

/// Resolve the governance environment from compile-time `AGORA_GOVERNANCE_ENV`.
pub fn resolve_governance_environment() -> GovernanceEnvironment {
    match option_env!("AGORA_GOVERNANCE_ENV") {
        Some(v) if v.eq_ignore_ascii_case("sandbox") || v.eq_ignore_ascii_case("sandbox_dev") => {
            GovernanceEnvironment::Sandbox
        }
        _ => GovernanceEnvironment::Production,
    }
}

/// Resolve the governance repository.
///
/// Priority (production / non-debug):
/// 1. Compile-time `AGORA_GOVERNANCE_REPO` (via `option_env!`)
/// 2. Compile-time `AGORA_REGISTRY_REPO` (via `option_env!`)
/// 3. Built-in default `"jarjarpfeil/Agora-Launcher"`
///
/// In debug builds, runtime env vars `AGORA_GOVERNANCE_REPO` and
/// `AGORA_REGISTRY_REPO` take priority before compile-time values.
pub fn resolve_governance_repo() -> String {
    #[cfg(debug_assertions)]
    {
        if let Ok(repo) = std::env::var("AGORA_GOVERNANCE_REPO") {
            if !repo.is_empty() {
                return repo;
            }
        }
        if let Ok(repo) = std::env::var("AGORA_REGISTRY_REPO") {
            if !repo.is_empty() {
                return repo;
            }
        }
    }
    option_env!("AGORA_GOVERNANCE_REPO")
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            option_env!("AGORA_REGISTRY_REPO")
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "jarjarpfeil/Agora-Launcher".into())
}

/// Resolve the GitHub App slug.
///
/// Priority (production / non-debug):
/// 1. Compile-time `AGORA_GITHUB_APP_SLUG` (via `option_env!`)
///
/// In debug builds, runtime `AGORA_GITHUB_APP_SLUG` takes priority.
pub fn resolve_github_app_slug() -> Option<String> {
    #[cfg(debug_assertions)]
    {
        if let Ok(slug) = std::env::var("AGORA_GITHUB_APP_SLUG") {
            if !slug.is_empty() {
                return Some(slug);
            }
        }
    }
    option_env!("AGORA_GITHUB_APP_SLUG")
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Build the fully resolved [`GovernanceConfig`].
///
/// `dev_registry_valid` should be `Some(true)` when a valid dev-registry
/// override is confirmed active, `Some(false)` when not, or `None` to use
/// a heuristic check.
pub fn resolve_governance_config(dev_registry_valid: Option<bool>) -> GovernanceConfig {
    let dev_registry = dev_registry_valid.unwrap_or_else(|| {
        #[cfg(debug_assertions)]
        {
            // Best-effort check without a full Ctx: verify the env var is set
            // AND the path is usable (the RegistryService resolves this
            // precisely at open time).
            crate::registry::check_dev_registry_env_set()
        }
        #[cfg(not(debug_assertions))]
        {
            false
        }
    });

    GovernanceConfig {
        repository: resolve_governance_repo(),
        environment: resolve_governance_environment(),
        github_app_slug: resolve_github_app_slug(),
        development_registry: dev_registry,
    }
}

// ---------------------------------------------------------------------------
// Read-only SQL queries (governance_summary / governance_events tables)
// ---------------------------------------------------------------------------

/// Check whether a table exists in the opened database.
pub fn table_exists(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
        [name],
        |_| Ok(true),
    )
    .unwrap_or(false)
}

/// Fetch a governance summary for a single item from `governance_summary`.
///
/// Returns `None` when the table does not exist (schema 6 or earlier) or when
/// the item has no row in the table.
pub fn get_governance_summary(
    conn: &Connection,
    item_id: &str,
) -> LauncherResult<Option<GovernanceSummary>> {
    if !table_exists(conn, "governance_summary") {
        return Ok(None);
    }

    let mut stmt = conn
        .prepare(
            "SELECT item_id, vote_issue_number, vote_issue_url,
                    raw_upvotes, raw_downvotes,
                    counted_upvotes, counted_downvotes,
                    quarantined_upvotes, quarantined_downvotes,
                    conflicted_users, status_reason, compiled_at
             FROM governance_summary WHERE item_id = ?1",
        )
        .map_err(|e| LauncherError::Generic {
            code: "ERR_GOVERNANCE_QUERY".into(),
            message: e.to_string(),
        })?;

    let mut rows = stmt
        .query_map([item_id], |row| {
            Ok(GovernanceSummary {
                item_id: row.get(0)?,
                vote_issue_number: row.get(1)?,
                vote_issue_url: row.get(2)?,
                raw_upvotes: row.get(3)?,
                raw_downvotes: row.get(4)?,
                counted_upvotes: row.get(5)?,
                counted_downvotes: row.get(6)?,
                quarantined_upvotes: row.get(7)?,
                quarantined_downvotes: row.get(8)?,
                conflicted_users: row.get(9)?,
                status_reason: row.get(10)?,
                compiled_at: row.get(11)?,
            })
        })
        .map_err(|e| LauncherError::Generic {
            code: "ERR_GOVERNANCE_QUERY".into(),
            message: e.to_string(),
        })?;

    match rows.next() {
        Some(Ok(summary)) => Ok(Some(summary)),
        Some(Err(e)) => Err(LauncherError::Generic {
            code: "ERR_GOVERNANCE_QUERY".into(),
            message: e.to_string(),
        }),
        None => Ok(None),
    }
}

/// List governance events, optionally filtered by item_id.
///
/// Returns `vec![]` when the `governance_events` table does not exist
/// (schema 6 or earlier).
pub fn list_governance_events(
    conn: &Connection,
    item_id: Option<&str>,
    limit: i64,
) -> LauncherResult<Vec<GovernanceEvent>> {
    if !table_exists(conn, "governance_events") {
        return Ok(Vec::new());
    }

    let (sql, params): (String, Vec<Box<dyn rusqlite::ToSql>>) = if let Some(iid) = item_id {
        (
            "SELECT event_id, item_id, event_type, status, detected_at, \
                    affected_reactions, details_json
             FROM governance_events WHERE item_id = ?1 \
             ORDER BY rowid DESC LIMIT ?2"
                .into(),
            vec![Box::new(iid.to_string()), Box::new(limit)],
        )
    } else {
        (
            "SELECT event_id, item_id, event_type, status, detected_at, \
                    affected_reactions, details_json
             FROM governance_events \
             ORDER BY rowid DESC LIMIT ?1"
                .into(),
            vec![Box::new(limit)],
        )
    };

    let mut stmt = conn.prepare(&sql).map_err(|e| LauncherError::Generic {
        code: "ERR_GOVERNANCE_QUERY".into(),
        message: e.to_string(),
    })?;

    let rows = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |row| {
            Ok(GovernanceEvent {
                event_id: row.get(0)?,
                item_id: row.get(1)?,
                event_type: row.get(2)?,
                status: row.get(3)?,
                detected_at: row.get(4)?,
                affected_reactions: row.get(5)?,
                details_json: row.get(6)?,
            })
        })
        .map_err(|e| LauncherError::Generic {
            code: "ERR_GOVERNANCE_QUERY".into(),
            message: e.to_string(),
        })?;

    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| LauncherError::Generic {
            code: "ERR_GOVERNANCE_QUERY".into(),
            message: e.to_string(),
        })?);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Governance diagnostics — structured read-only checks
// ---------------------------------------------------------------------------

/// Run governance diagnostics (sync checks only).
///
/// Checks 1-4, plus governance table parses (14-16) and schema validation.
/// Network checks (5-13) are performed by the desktop async command layer.
pub fn run_governance_diagnostics(
    registry_conn: Option<&Connection>,
    config: &GovernanceConfig,
) -> Vec<DiagnosticCheck> {
    let mut checks = Vec::new();

    // Schema version and table presence (used by multiple checks below)
    let (schema_version, has_summary_table, has_events_table) = match registry_conn {
        Some(conn) => {
            let sv: i64 = conn
                .query_row(
                    "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            let hst = table_exists(conn, "governance_summary");
            let het = table_exists(conn, "governance_events");
            (sv, hst, het)
        }
        None => (0, false, false),
    };

    // 1. oauth_client_id
    let oauth_configured = !crate::auth::AGORA_OAUTH_CLIENT_ID.is_empty();
    checks.push(DiagnosticCheck {
        id: "oauth_client_id".into(),
        status: if oauth_configured {
            DiagnosticStatus::Pass
        } else {
            DiagnosticStatus::Fail
        },
        message: if oauth_configured {
            "GitHub OAuth client ID is configured".into()
        } else {
            "AGORA_OAUTH_CLIENT_ID is empty or not set".into()
        },
    });

    // 2. github_token_available
    let stored_bundle = crate::auth::load_token_bundle();
    let has_token = stored_bundle.is_some();
    checks.push(DiagnosticCheck {
        id: "github_token_available".into(),
        status: if has_token {
            DiagnosticStatus::Pass
        } else {
            DiagnosticStatus::Fail
        },
        message: if has_token {
            "GitHub token is stored in credential manager".into()
        } else {
            "No GitHub token stored; sign in via Settings".into()
        },
    });

    // 3. github_token_valid_or_refreshable
    let has_refresh = stored_bundle
        .as_ref()
        .and_then(|b| b.refresh_token.as_deref())
        .map(|rt| !rt.is_empty())
        .unwrap_or(false);
    let token_fresh = stored_bundle
        .as_ref()
        .map(|b| crate::auth::access_token_is_fresh(b))
        .unwrap_or(false);
    let token_valid_or_refreshable = has_token && (token_fresh || has_refresh);
    checks.push(DiagnosticCheck {
        id: "github_token_valid_or_refreshable".into(),
        status: if token_valid_or_refreshable {
            DiagnosticStatus::Pass
        } else if has_token && !has_refresh {
            DiagnosticStatus::Warning
        } else {
            DiagnosticStatus::Fail
        },
        message: if token_fresh {
            "Access token is fresh and ready".into()
        } else if has_refresh {
            "Access token needs refresh; refresh token is available".into()
        } else if has_token {
            "Legacy token without refresh — cannot rotate, may expire".into()
        } else {
            "No token available; sign in required".into()
        },
    });

    // 4. governance_repo_resolved
    let repo_ok = !config.repository.is_empty() && config.repository.contains('/');
    checks.push(DiagnosticCheck {
        id: "governance_repo_resolved".into(),
        status: if repo_ok {
            DiagnosticStatus::Pass
        } else {
            DiagnosticStatus::Fail
        },
        message: if repo_ok {
            format!("Governance repository: {}", config.repository)
        } else {
            "Governance repository could not be resolved from env or build".into()
        },
    });

    // Schema-level checks
    checks.push(DiagnosticCheck {
        id: "registry_schema".into(),
        status: if schema_version >= 6 {
            DiagnosticStatus::Pass
        } else if schema_version > 0 {
            DiagnosticStatus::Fail
        } else {
            DiagnosticStatus::Warning
        },
        message: if schema_version > 0 {
            format!("Registry schema version: {schema_version}")
        } else {
            "Registry database is not available".into()
        },
    });

    // governance_summary table check
    checks.push(DiagnosticCheck {
        id: "governance_summary_table".into(),
        status: if has_summary_table {
            DiagnosticStatus::Pass
        } else if schema_version >= 7 {
            DiagnosticStatus::Fail
        } else {
            DiagnosticStatus::Warning
        },
        message: if has_summary_table {
            "governance_summary table exists (schema 7+)".into()
        } else if schema_version >= 7 {
            "governance_summary table missing in schema 7 database".into()
        } else {
            "governance_summary requires schema 7+; registry is schema 6".into()
        },
    });

    // governance_events table check
    checks.push(DiagnosticCheck {
        id: "governance_events_table".into(),
        status: if has_events_table {
            DiagnosticStatus::Pass
        } else if schema_version >= 7 {
            DiagnosticStatus::Fail
        } else {
            DiagnosticStatus::Warning
        },
        message: if has_events_table {
            "governance_events table exists (schema 7+)".into()
        } else if schema_version >= 7 {
            "governance_events table missing in schema 7 database".into()
        } else {
            "governance_events requires schema 7+; registry is schema 6".into()
        },
    });

    // 14. vote_issues_parses — query governance_summary and validate parsing
    if let Some(conn) = registry_conn {
        if has_summary_table {
            let parse_ok = conn
                .prepare("SELECT COUNT(*) FROM governance_summary")
                .and_then(|mut s| s.query_row([], |row| row.get::<_, i64>(0)))
                .is_ok();
            let row_count = if parse_ok {
                conn.query_row("SELECT COUNT(*) FROM governance_summary", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap_or(0)
            } else {
                0
            };
            checks.push(DiagnosticCheck {
                id: "vote_issues_parses".into(),
                status: if parse_ok {
                    DiagnosticStatus::Pass
                } else {
                    DiagnosticStatus::Fail
                },
                message: if parse_ok {
                    format!("governance_summary table readable with {row_count} vote record(s)")
                } else {
                    "governance_summary table could not be queried".into()
                },
            });
        } else {
            checks.push(DiagnosticCheck {
                id: "vote_issues_parses".into(),
                status: DiagnosticStatus::Warning,
                message: "governance_summary table unavailable (schema 6)".into(),
            });
        }
    } else {
        checks.push(DiagnosticCheck {
            id: "vote_issues_parses".into(),
            status: DiagnosticStatus::Warning,
            message: "Cannot parse vote issues: registry database unavailable".into(),
        });
    }

    // 15. quarantine_decisions_parses — query governance_events for quarantines
    if let Some(conn) = registry_conn {
        if has_events_table {
            let parse_ok = conn
                .prepare("SELECT COUNT(*) FROM governance_events")
                .is_ok();
            let quarantine_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM governance_events WHERE event_type = 'vote_quarantine'",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            checks.push(DiagnosticCheck {
                id: "quarantine_decisions_parses".into(),
                status: if parse_ok {
                    DiagnosticStatus::Pass
                } else {
                    DiagnosticStatus::Fail
                },
                message: if parse_ok {
                    format!("governance_events readable; {quarantine_count} quarantine event(s)")
                } else {
                    "governance_events table could not be queried".into()
                },
            });
        } else {
            checks.push(DiagnosticCheck {
                id: "quarantine_decisions_parses".into(),
                status: DiagnosticStatus::Warning,
                message: "governance_events table unavailable (schema 6)".into(),
            });
        }
    } else {
        checks.push(DiagnosticCheck {
            id: "quarantine_decisions_parses".into(),
            status: DiagnosticStatus::Warning,
            message: "Cannot parse quarantine decisions: registry unavailable".into(),
        });
    }

    // 16. governance_state_parses — combined governance_summary + events readable
    if let Some(conn) = registry_conn {
        let has_gov_state = has_summary_table || has_events_table;
        let summary_ok = if has_summary_table {
            conn.prepare("SELECT COUNT(*) FROM governance_summary")
                .is_ok()
        } else {
            false
        };
        let events_ok = if has_events_table {
            conn.prepare("SELECT COUNT(*) FROM governance_events")
                .is_ok()
        } else {
            false
        };
        let state_readable = (has_summary_table && summary_ok) || (has_events_table && events_ok);
        checks.push(DiagnosticCheck {
            id: "governance_state_parses".into(),
            status: if state_readable {
                DiagnosticStatus::Pass
            } else if has_gov_state {
                DiagnosticStatus::Fail
            } else {
                DiagnosticStatus::Warning
            },
            message: if state_readable {
                "Governance state is readable from registry".into()
            } else if has_gov_state {
                "Governance tables exist but could not be queried".into()
            } else {
                "No governance tables present (schema 6)".into()
            },
        });
    } else {
        checks.push(DiagnosticCheck {
            id: "governance_state_parses".into(),
            status: DiagnosticStatus::Warning,
            message: "Cannot parse governance state: registry unavailable".into(),
        });
    }

    checks
}

// ---------------------------------------------------------------------------
// GovernanceService — core-owned typed service (read-only, dev-override aware)
// ---------------------------------------------------------------------------

/// Read-only governance operations.
///
/// Uses [`RegistryService`] internally for database connections, which
/// automatically respects the `AGORA_DEV_REGISTRY_DB` debug override.
#[derive(Clone)]
pub struct GovernanceService {
    registry: RegistryService,
}

impl GovernanceService {
    pub fn new(registry: RegistryService) -> Self {
        Self { registry }
    }

    /// Return the resolved governance config.
    ///
    /// `development_registry` is set to true only when a valid dev override
    /// is confirmed active (env var set AND path valid).
    pub fn config(&self) -> GovernanceConfig {
        let dev_active = self.registry.development_registry_active();
        resolve_governance_config(Some(dev_active))
    }

    /// Fetch a governance summary for a single item from `governance_summary`.
    ///
    /// Returns `Ok(None)` when the `governance_summary` table does not exist
    /// (schema 6 or earlier).
    pub fn get_governance_summary(
        &self,
        item_id: &str,
    ) -> LauncherResult<Option<GovernanceSummary>> {
        let conn = self.registry.connection()?;
        get_governance_summary(&conn, item_id)
    }

    /// List governance events, optionally filtered by `item_id`.
    ///
    /// Returns `vec![]` when the `governance_events` table does not exist
    /// (schema 6 or earlier).
    pub fn list_governance_events(
        &self,
        item_id: Option<&str>,
        limit: i64,
    ) -> LauncherResult<Vec<GovernanceEvent>> {
        let conn = self.registry.connection()?;
        list_governance_events(&conn, item_id, limit)
    }

    /// Run governance diagnostics (sync checks only).
    ///
    /// Network checks (repository_metadata_readable, issues_enabled,
    /// discussions_enabled, labels, templates) must be performed by the
    /// desktop async command layer with auth.
    pub fn run_diagnostics(&self) -> Vec<DiagnosticCheck> {
        let conn = self.registry.connection().ok();
        let c = self.config();
        run_governance_diagnostics(conn.as_ref(), &c)
    }
}

// ---------------------------------------------------------------------------
// Legacy compatibility exports
// ---------------------------------------------------------------------------

pub fn governance_repo(_cli_override: Option<&str>) -> String {
    resolve_governance_repo()
}

pub fn is_development_registry() -> bool {
    #[cfg(debug_assertions)]
    {
        crate::registry::check_dev_registry_env_set()
    }
    #[cfg(not(debug_assertions))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed_schema7_registry(reg_path: &std::path::Path) {
        let conn = rusqlite::Connection::open(reg_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER PRIMARY KEY);
             INSERT INTO schema_version (version) VALUES (7);

             CREATE TABLE registry_items (
                 id TEXT PRIMARY KEY, name TEXT, content_type TEXT,
                 download_strategy TEXT, source_identifier TEXT, sha256 TEXT,
                 upvotes INTEGER DEFAULT 0, downvotes INTEGER DEFAULT 0,
                 net_score INTEGER DEFAULT 0, velocity REAL DEFAULT 0.0,
                 status TEXT DEFAULT 'active', is_immune INTEGER DEFAULT 0,
                 immunity_reason TEXT, allow_comments INTEGER DEFAULT 1,
                 icon_url TEXT, gallery_urls_json TEXT, date_added TEXT,
                 compatible_versions_json TEXT, description TEXT,
                 body_markdown TEXT, page_url TEXT, license_id TEXT,
                 source_updated_at TEXT, modrinth_id TEXT
             );
             INSERT INTO registry_items (id, name)
             VALUES ('test-mod', 'Test Mod');

             CREATE TABLE governance_summary (
                 item_id TEXT PRIMARY KEY,
                 vote_issue_number INTEGER,
                 vote_issue_url TEXT,
                 raw_upvotes INTEGER NOT NULL DEFAULT 0,
                 raw_downvotes INTEGER NOT NULL DEFAULT 0,
                 counted_upvotes INTEGER NOT NULL DEFAULT 0,
                 counted_downvotes INTEGER NOT NULL DEFAULT 0,
                 quarantined_upvotes INTEGER NOT NULL DEFAULT 0,
                 quarantined_downvotes INTEGER NOT NULL DEFAULT 0,
                 conflicted_users INTEGER NOT NULL DEFAULT 0,
                 status_reason TEXT,
                 compiled_at TEXT NOT NULL
             );
             INSERT INTO governance_summary
                 (item_id, vote_issue_number, vote_issue_url,
                  raw_upvotes, raw_downvotes, counted_upvotes, counted_downvotes,
                  quarantined_upvotes, quarantined_downvotes, conflicted_users,
                  status_reason, compiled_at)
             VALUES ('test-mod', 42, 'https://github.com/test/repo/issues/42',
                     10, 3, 8, 2, 2, 1, 0, NULL, '2025-01-15T00:00:00Z');

             CREATE TABLE governance_events (
                 event_id TEXT PRIMARY KEY,
                 item_id TEXT NOT NULL,
                 event_type TEXT NOT NULL,
                 status TEXT NOT NULL,
                 detected_at TEXT NOT NULL,
                 affected_reactions INTEGER NOT NULL DEFAULT 0,
                 details_json TEXT
             );
             INSERT INTO governance_events VALUES
                 ('evt-001', 'test-mod', 'vote_quarantine', 'active',
                  '2025-01-16T00:00:00Z', 3, '{\"reason\":\"suspicious_pattern\"}'),
                 ('evt-002', 'test-mod', 'status_change', 'under_review',
                  '2025-01-17T00:00:00Z', 0, NULL);",
        )
        .unwrap();
        drop(conn);
    }

    fn seed_schema6_registry(reg_path: &std::path::Path) {
        let conn = rusqlite::Connection::open(reg_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER PRIMARY KEY);
             INSERT INTO schema_version (version) VALUES (6);
             CREATE TABLE registry_items (
                 id TEXT PRIMARY KEY, name TEXT
             );
             INSERT INTO registry_items (id, name) VALUES ('test-mod', 'Test Mod');",
        )
        .unwrap();
        drop(conn);
    }

    // -----------------------------------------------------------------------
    // Environment resolution tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_governance_environment_default_is_production() {
        let env = resolve_governance_environment();
        assert_eq!(env, GovernanceEnvironment::Production);
    }

    #[test]
    fn test_resolve_governance_repo_fallback() {
        let repo = resolve_governance_repo();
        assert!(!repo.is_empty());
        assert!(repo.contains('/'), "repo should be owner/repo format");
    }

    #[test]
    fn test_resolve_governance_config_default_fields() {
        let config = resolve_governance_config(None);
        assert!(!config.repository.is_empty());
    }

    #[test]
    fn test_development_registry_flagged_when_valid() {
        let config = resolve_governance_config(Some(true));
        assert!(config.development_registry);
        let config = resolve_governance_config(Some(false));
        assert!(!config.development_registry);
    }

    // -----------------------------------------------------------------------
    // GovernanceSummary tests (schema 7 — governance_summary table)
    // -----------------------------------------------------------------------

    #[test]
    fn test_get_governance_summary_found() {
        let root =
            std::env::temp_dir().join(format!("agora-gov-summary-ok-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let reg_path = root.join("registry.db");
        seed_schema7_registry(&reg_path);

        let conn = crate::db::registry_connection(&reg_path).unwrap();
        let summary = get_governance_summary(&conn, "test-mod").unwrap();

        assert!(summary.is_some());
        let s = summary.unwrap();
        assert_eq!(s.item_id, "test-mod");
        assert_eq!(s.vote_issue_number, Some(42));
        assert_eq!(
            s.vote_issue_url.as_deref(),
            Some("https://github.com/test/repo/issues/42")
        );
        assert_eq!(s.raw_upvotes, 10);
        assert_eq!(s.raw_downvotes, 3);
        assert_eq!(s.counted_upvotes, 8);
        assert_eq!(s.counted_downvotes, 2);
        assert_eq!(s.quarantined_upvotes, 2);
        assert_eq!(s.quarantined_downvotes, 1);
        assert_eq!(s.conflicted_users, 0);
        assert_eq!(s.compiled_at, "2025-01-15T00:00:00Z");

        drop(conn);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn test_get_governance_summary_not_found_returns_none() {
        let root =
            std::env::temp_dir().join(format!("agora-gov-summary-none-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let reg_path = root.join("registry.db");
        seed_schema7_registry(&reg_path);

        let conn = crate::db::registry_connection(&reg_path).unwrap();
        let summary = get_governance_summary(&conn, "nonexistent-mod").unwrap();
        assert!(summary.is_none());

        drop(conn);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn test_get_governance_summary_null_vote_issue_number() {
        let root =
            std::env::temp_dir().join(format!("agora-gov-summary-null-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let reg_path = root.join("registry.db");
        let conn = rusqlite::Connection::open(&reg_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER PRIMARY KEY);
             INSERT INTO schema_version (version) VALUES (7);
             CREATE TABLE governance_summary (
                 item_id TEXT PRIMARY KEY,
                 vote_issue_number INTEGER,
                 vote_issue_url TEXT,
                 raw_upvotes INTEGER NOT NULL DEFAULT 0,
                 raw_downvotes INTEGER NOT NULL DEFAULT 0,
                 counted_upvotes INTEGER NOT NULL DEFAULT 0,
                 counted_downvotes INTEGER NOT NULL DEFAULT 0,
                 quarantined_upvotes INTEGER NOT NULL DEFAULT 0,
                 quarantined_downvotes INTEGER NOT NULL DEFAULT 0,
                 conflicted_users INTEGER NOT NULL DEFAULT 0,
                 status_reason TEXT,
                 compiled_at TEXT NOT NULL
             );
             INSERT INTO governance_summary
                 (item_id, vote_issue_number, vote_issue_url, raw_upvotes,
                  raw_downvotes, counted_upvotes, counted_downvotes,
                  quarantined_upvotes, quarantined_downvotes, conflicted_users,
                  status_reason, compiled_at)
             VALUES ('no-vote', NULL, NULL, 0, 0, 0, 0, 0, 0, 0, NULL, '2025-01-01T00:00:00Z');",
        )
        .unwrap();
        drop(conn);

        let conn = crate::db::registry_connection(&reg_path).unwrap();
        let summary = get_governance_summary(&conn, "no-vote").unwrap();
        assert!(summary.is_some());
        assert!(summary.unwrap().vote_issue_number.is_none());

        drop(conn);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn test_get_governance_summary_schema6_graceful() {
        let root =
            std::env::temp_dir().join(format!("agora-gov-summary-s6-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let reg_path = root.join("registry.db");
        seed_schema6_registry(&reg_path);

        let conn = crate::db::registry_connection(&reg_path).unwrap();
        let summary = get_governance_summary(&conn, "test-mod").unwrap();
        assert!(
            summary.is_none(),
            "schema 6 has no governance_summary table"
        );

        drop(conn);
        let _ = std::fs::remove_dir_all(root);
    }

    // -----------------------------------------------------------------------
    // GovernanceEvent tests (schema 7 — governance_events table)
    // -----------------------------------------------------------------------

    #[test]
    fn test_list_governance_events_all() {
        let root =
            std::env::temp_dir().join(format!("agora-gov-events-all-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let reg_path = root.join("registry.db");
        seed_schema7_registry(&reg_path);

        let conn = crate::db::registry_connection(&reg_path).unwrap();
        let events = list_governance_events(&conn, None, 10).unwrap();

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_id, "evt-002");
        assert_eq!(events[0].item_id, "test-mod");
        assert_eq!(events[0].event_type, "status_change");
        assert_eq!(events[0].affected_reactions, 0);
        assert!(events[0].details_json.is_none());

        assert_eq!(events[1].event_id, "evt-001");
        assert_eq!(events[1].event_type, "vote_quarantine");
        assert_eq!(events[1].affected_reactions, 3);
        assert!(events[1].details_json.is_some());

        drop(conn);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn test_list_governance_events_filtered_by_item() {
        let root =
            std::env::temp_dir().join(format!("agora-gov-events-filter-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let reg_path = root.join("registry.db");
        seed_schema7_registry(&reg_path);

        let conn = crate::db::registry_connection(&reg_path).unwrap();
        let events = list_governance_events(&conn, Some("test-mod"), 10).unwrap();
        assert_eq!(events.len(), 2);

        let events_none = list_governance_events(&conn, Some("other-mod"), 10).unwrap();
        assert!(events_none.is_empty());

        drop(conn);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn test_list_governance_events_schema6_graceful() {
        let root =
            std::env::temp_dir().join(format!("agora-gov-events-s6-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let reg_path = root.join("registry.db");
        seed_schema6_registry(&reg_path);

        let conn = crate::db::registry_connection(&reg_path).unwrap();
        let events = list_governance_events(&conn, None, 10).unwrap();
        assert!(events.is_empty(), "schema 6 has no governance_events table");

        drop(conn);
        let _ = std::fs::remove_dir_all(root);
    }

    // -----------------------------------------------------------------------
    // DiagnosticStatus serialization tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_diagnostic_status_serialization_pass() {
        let val = serde_json::to_value(DiagnosticStatus::Pass).unwrap();
        assert_eq!(val, serde_json::json!("pass"));
    }

    #[test]
    fn test_diagnostic_status_serialization_warning() {
        let val = serde_json::to_value(DiagnosticStatus::Warning).unwrap();
        assert_eq!(val, serde_json::json!("warning"));
    }

    #[test]
    fn test_diagnostic_status_serialization_fail() {
        let val = serde_json::to_value(DiagnosticStatus::Fail).unwrap();
        assert_eq!(val, serde_json::json!("fail"));
    }

    // -----------------------------------------------------------------------
    // Diagnostics tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_diagnostics_contains_required_check_ids() {
        let config = GovernanceConfig {
            repository: "owner/repo".into(),
            environment: GovernanceEnvironment::Production,
            github_app_slug: None,
            development_registry: false,
        };
        let checks = run_governance_diagnostics(None, &config);
        let ids: Vec<&str> = checks.iter().map(|c| c.id.as_str()).collect();

        // Sync checks that must always be present
        assert!(
            ids.contains(&"oauth_client_id"),
            "missing oauth_client_id in: {ids:?}"
        );
        assert!(
            ids.contains(&"github_token_available"),
            "missing github_token_available in: {ids:?}"
        );
        assert!(
            ids.contains(&"github_token_valid_or_refreshable"),
            "missing github_token_valid_or_refreshable in: {ids:?}"
        );
        assert!(
            ids.contains(&"governance_repo_resolved"),
            "missing governance_repo_resolved in: {ids:?}"
        );
        assert!(
            ids.contains(&"vote_issues_parses"),
            "missing vote_issues_parses in: {ids:?}"
        );
        assert!(
            ids.contains(&"quarantine_decisions_parses"),
            "missing quarantine_decisions_parses in: {ids:?}"
        );
        assert!(
            ids.contains(&"governance_state_parses"),
            "missing governance_state_parses in: {ids:?}"
        );
    }

    #[test]
    fn test_diagnostics_production_config() {
        let config = GovernanceConfig {
            repository: "owner/repo".into(),
            environment: GovernanceEnvironment::Production,
            github_app_slug: Some("my-app".into()),
            development_registry: false,
        };
        let checks = run_governance_diagnostics(None, &config);

        let repo_check = checks
            .iter()
            .find(|c| c.id == "governance_repo_resolved")
            .unwrap();
        assert_eq!(repo_check.status, DiagnosticStatus::Pass);
    }

    #[test]
    fn test_diagnostics_empty_repo_fails() {
        let config = GovernanceConfig {
            repository: String::new(),
            environment: GovernanceEnvironment::Production,
            github_app_slug: None,
            development_registry: false,
        };
        let checks = run_governance_diagnostics(None, &config);

        let repo_check = checks
            .iter()
            .find(|c| c.id == "governance_repo_resolved")
            .unwrap();
        assert_eq!(repo_check.status, DiagnosticStatus::Fail);
    }

    #[test]
    fn test_diagnostics_with_schema7_registry() {
        let root = std::env::temp_dir().join(format!("agora-gov-diag-s7-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let reg_path = root.join("registry.db");
        seed_schema7_registry(&reg_path);

        let config = GovernanceConfig {
            repository: "jarjarpfeil/Agora-Launcher".into(),
            environment: GovernanceEnvironment::Production,
            github_app_slug: None,
            development_registry: false,
        };
        let conn = crate::db::registry_connection(&reg_path).unwrap();
        let checks = run_governance_diagnostics(Some(&conn), &config);

        assert!(
            checks.iter().any(|c| {
                c.id == "governance_summary_table" && c.status == DiagnosticStatus::Pass
            }),
            "governance_summary_table should Pass with schema 7"
        );
        assert!(
            checks
                .iter()
                .any(|c| c.id == "governance_events_table" && c.status == DiagnosticStatus::Pass),
            "governance_events_table should Pass with schema 7"
        );
        assert!(
            checks
                .iter()
                .any(|c| c.id == "vote_issues_parses" && c.status == DiagnosticStatus::Pass),
            "vote_issues_parses should Pass with schema 7"
        );
        assert!(
            checks
                .iter()
                .any(|c| c.id == "governance_state_parses" && c.status == DiagnosticStatus::Pass),
            "governance_state_parses should Pass with schema 7"
        );

        drop(conn);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn test_diagnostics_with_schema6_registry() {
        let root = std::env::temp_dir().join(format!("agora-gov-diag-s6-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let reg_path = root.join("registry.db");
        seed_schema6_registry(&reg_path);

        let config = GovernanceConfig {
            repository: "jarjarpfeil/Agora-Launcher".into(),
            environment: GovernanceEnvironment::Production,
            github_app_slug: None,
            development_registry: false,
        };
        let conn = crate::db::registry_connection(&reg_path).unwrap();
        let checks = run_governance_diagnostics(Some(&conn), &config);

        // Schema 6: governance tables should report Warning
        let sum_check = checks
            .iter()
            .find(|c| c.id == "governance_summary_table")
            .unwrap();
        assert_eq!(
            sum_check.status,
            DiagnosticStatus::Warning,
            "schema 6 should Warning for missing governance_summary table"
        );

        let state_check = checks
            .iter()
            .find(|c| c.id == "governance_state_parses")
            .unwrap();
        assert_eq!(
            state_check.status,
            DiagnosticStatus::Warning,
            "schema 6 should Warning for governance_state_parses"
        );

        drop(conn);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn test_diagnostics_includes_oauth_check() {
        let config = GovernanceConfig {
            repository: "owner/repo".into(),
            environment: GovernanceEnvironment::Production,
            github_app_slug: None,
            development_registry: false,
        };
        let checks = run_governance_diagnostics(None, &config);
        assert!(checks.iter().any(|c| c.id == "oauth_client_id"));
    }

    #[test]
    fn test_diagnostics_includes_token_check() {
        let config = GovernanceConfig {
            repository: "owner/repo".into(),
            environment: GovernanceEnvironment::Production,
            github_app_slug: None,
            development_registry: false,
        };
        let checks = run_governance_diagnostics(None, &config);
        assert!(checks.iter().any(|c| c.id == "github_token_available"));
    }

    // -----------------------------------------------------------------------
    // GovernanceService tests
    // -----------------------------------------------------------------------

    fn make_registry_svc(reg_path: &std::path::Path) -> RegistryService {
        let root = std::env::temp_dir().join(format!("agora-gov-svc-{}", uuid::Uuid::new_v4()));
        let ctx = crate::ctx::Ctx::for_testing(root.clone());
        std::fs::copy(reg_path, &ctx.paths.registry_db()).unwrap();
        RegistryService::new(ctx)
    }

    #[test]
    fn test_governance_service_get_summary() {
        let root =
            std::env::temp_dir().join(format!("agora-gov-svc-getsum-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let reg_path = root.join("registry.db");
        seed_schema7_registry(&reg_path);

        let reg_svc = make_registry_svc(&reg_path);
        let svc = GovernanceService::new(reg_svc);
        let summary = svc.get_governance_summary("test-mod").unwrap();
        assert!(summary.is_some());
        assert_eq!(summary.unwrap().vote_issue_number, Some(42));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn test_governance_service_list_events() {
        let root =
            std::env::temp_dir().join(format!("agora-gov-svc-events-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let reg_path = root.join("registry.db");
        seed_schema7_registry(&reg_path);

        let reg_svc = make_registry_svc(&reg_path);
        let svc = GovernanceService::new(reg_svc);
        let events = svc.list_governance_events(Some("test-mod"), 10).unwrap();
        assert_eq!(events.len(), 2);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn test_governance_service_schema6_graceful() {
        let root = std::env::temp_dir().join(format!("agora-gov-svc-s6-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let reg_path = root.join("registry.db");
        seed_schema6_registry(&reg_path);

        let reg_svc = make_registry_svc(&reg_path);
        let svc = GovernanceService::new(reg_svc);

        let summary = svc.get_governance_summary("test-mod").unwrap();
        assert!(summary.is_none(), "schema 6 should return None");

        let events = svc.list_governance_events(None, 10).unwrap();
        assert!(events.is_empty(), "schema 6 should return empty events");

        let _ = std::fs::remove_dir_all(root);
    }
}
