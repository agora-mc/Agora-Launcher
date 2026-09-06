use crate::ctx::Ctx;
use crate::dependency_ops::VersionGrammar;
use crate::error::{LauncherError, LauncherResult};
use crate::governance::{
    get_governance_summary, list_governance_events, GovernanceEvent, GovernanceSummary,
};
use crate::registry_sync::APP_REGISTRY_SCHEMA_VERSION;
use crate::version_match::{evaluate_version_match, VersionMatch};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json;
use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------------------
// Curated content axes
// ---------------------------------------------------------------------------

/// Curated download strategies that appear in Browse / For You.
///
/// A curated item is derived from the Ed25519-signed registry no matter which
/// host or CDN its file ships from, so *which curated sources are visible* is
/// a content axis: on by default, opt-out per source via
/// `curated_source_<strategy>_enabled` settings. This is deliberately separate
/// from live third-party browsing (`modrinth_enabled`, `technic_enabled`),
/// which is off by default and governs discovery *outside* the catalog only.
///
/// A curated entry's listability and installability depend on these settings
/// alone. Live browsing may enrich its presentation — changelogs, category
/// tags — but must never be the reason a curated entry cannot be listed,
/// resolved, or installed. There is no separate "curated only" mode: with live
/// browsing off by default, curated-only is simply the default state.
///
/// Filtering is a **whitelist** of enabled strategies: any strategy an
/// entry might carry that is not listed here (or not enabled by the user)
/// is hidden — fail closed, never fail open. An entry carrying several
/// download sources stays visible while *any* of them is enabled, because it
/// is still installable from the ones that are.
pub const CURATED_DOWNLOAD_STRATEGIES: [&str; 5] = [
    "modrinth_id",
    "github_release",
    "direct_hash",
    "curated_pack",
    "technic_pack",
];

/// SQL expression yielding an item's download sources as a JSON array.
///
/// Registries compiled before schema 8 have no `download_sources_json`, so the
/// preferred source is synthesized from the legacy columns. The same synthesis
/// covers a value `json_each` could not walk: one unreadable row must not make
/// the whole browse query fail.
const DOWNLOAD_SOURCES_SQL: &str = "CASE      WHEN ri.download_sources_json IS NULL OR ri.download_sources_json IN ('', '[]')           OR NOT json_valid(ri.download_sources_json)      THEN json_array(json_object('strategy', ri.download_strategy, 'identifier', ri.source_identifier))      ELSE ri.download_sources_json END";

/// Build the SQL whitelist fragment + params for the enabled curated
/// strategies. Entries with no enabled source at all are hidden.
fn curated_strategy_whitelist(enabled: &[String]) -> (Option<String>, Vec<String>) {
    let allowed: Vec<String> = CURATED_DOWNLOAD_STRATEGIES
        .iter()
        .map(|s| s.to_string())
        .filter(|s| enabled.iter().any(|e| e == s))
        .collect();
    if allowed.is_empty() {
        return (Some("1 = 0".into()), Vec::new());
    }
    let placeholders = allowed.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    (
        Some(format!(
            "EXISTS (SELECT 1 FROM json_each({DOWNLOAD_SOURCES_SQL}) src              WHERE json_extract(src.value, '$.strategy') IN ({placeholders}))"
        )),
        allowed,
    )
}

/// One place an item's file can be fetched from.
///
/// Items carry an ordered list of these, best first. The resolver walks the
/// list and uses the first source that is both enabled in the user's settings
/// and actually answering, so a rate-limited GitHub or an offline mirror
/// degrades to a fallback instead of a failed install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadSource {
    /// One of [`CURATED_DOWNLOAD_STRATEGIES`].
    pub strategy: String,
    /// What the strategy resolves: a `owner/repo` for `github_release`, a
    /// project id for `modrinth_id`, a pinned URL for `direct_hash` /
    /// `technic_pack`, the pack id for `curated_pack`.
    pub identifier: String,
}

// ---------------------------------------------------------------------------
// RegistryService — core-owned typed service
// ---------------------------------------------------------------------------

/// Core-owned service for reading the curated registry database.
///
/// Every method opens a fresh read-only connection via `Ctx` paths; desktop
/// adapters must use this service rather than opening the database directly.
///
/// ## Debug-only: `AGORA_DEV_REGISTRY_DB`
///
/// When `AGORA_DEV_REGISTRY_DB` is set in debug builds, the service loads the
/// registry from that path. The path must exist and have schema version
/// >= 6 and <= [`APP_REGISTRY_SCHEMA_VERSION`]. If either condition fails,
///
/// an error is returned (fail closed, not silent fallback).
///
/// In release builds the variable is logged and ignored.
#[derive(Clone)]
pub struct RegistryService {
    ctx: Ctx,
}

impl RegistryService {
    pub fn new(ctx: Ctx) -> Self {
        Self { ctx }
    }

    /// Resolve the registry database path, respecting debug-only dev overrides.
    ///
    /// Debug: fails with an error message when `AGORA_DEV_REGISTRY_DB` is set
    /// but the path does not exist or has an unsupported schema version.
    pub fn resolve_db_path(&self) -> LauncherResult<std::path::PathBuf> {
        let default_path = self.ctx.paths.registry_db();

        #[cfg(debug_assertions)]
        if let Ok(dev_path_str) = std::env::var("AGORA_DEV_REGISTRY_DB") {
            let dev_path = std::path::PathBuf::from(&dev_path_str);

            if !dev_path.exists() {
                return Err(LauncherError::Generic {
                    code: "ERR_DEV_REGISTRY_PATH".into(),
                    message: format!(
                        "AGORA_DEV_REGISTRY_DB set to {} but path does not exist",
                        dev_path.display()
                    ),
                });
            }

            let conn =
                crate::db::registry_connection(&dev_path).map_err(|e| LauncherError::Generic {
                    code: "ERR_DEV_REGISTRY_OPEN".into(),
                    message: format!(
                        "AGORA_DEV_REGISTRY_DB path {} cannot be opened: {e}",
                        dev_path.display()
                    ),
                })?;

            let version: i64 = conn
                .query_row(
                    "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or(0);

            if version < 6 {
                return Err(LauncherError::Generic {
                    code: "ERR_DEV_REGISTRY_SCHEMA".into(),
                    message: format!(
                        "AGORA_DEV_REGISTRY_DB path {} has schema v{version}; \
                         minimum supported is 6",
                        dev_path.display()
                    ),
                });
            }

            if version > APP_REGISTRY_SCHEMA_VERSION {
                return Err(LauncherError::Generic {
                    code: "ERR_DEV_REGISTRY_SCHEMA_FUTURE".into(),
                    message: format!(
                        "AGORA_DEV_REGISTRY_DB path {} has schema v{version}; \
                         app supports up to v{APP_REGISTRY_SCHEMA_VERSION}",
                        dev_path.display()
                    ),
                });
            }

            eprintln!(
                "[registry] AGORA_DEV_REGISTRY_DB active: using {} (schema v{version})",
                dev_path.display()
            );
            return Ok(dev_path);
        }

        #[cfg(not(debug_assertions))]
        if std::env::var("AGORA_DEV_REGISTRY_DB").is_ok() {
            eprintln!(
                "[registry] AGORA_DEV_REGISTRY_DB is ignored in release builds. \
                 Using cached registry db."
            );
        }

        Ok(default_path)
    }

    /// Whether a valid development registry override is active.
    ///
    /// Returns `true` only when `AGORA_DEV_REGISTRY_DB` is set AND the path
    /// is valid (exists, schema >=6, <= APP_REGISTRY_SCHEMA_VERSION).
    pub fn development_registry_active(&self) -> bool {
        #[cfg(debug_assertions)]
        {
            self.resolve_db_path()
                .map(|p| p != self.ctx.paths.registry_db())
                .unwrap_or(false)
        }
        #[cfg(not(debug_assertions))]
        {
            false
        }
    }

    /// Open a read-only connection to the resolved registry database.
    pub fn connection(&self) -> LauncherResult<rusqlite::Connection> {
        let path = self.resolve_db_path()?;
        if !path.exists() {
            return Err(LauncherError::RegistryMissing);
        }
        crate::db::registry_connection(&path).map_err(|_| LauncherError::RegistryMissing)
    }

    /// Fetch a single registry item by ID.
    pub fn get_item_by_id(&self, item_id: &str) -> LauncherResult<Option<RegistryItem>> {
        let conn = self.connection()?;
        get_item_by_id(&conn, item_id)
    }

    /// Fetch registry items and their category labels in one read session.
    pub fn get_items_by_ids(
        &self,
        item_ids: &[String],
    ) -> LauncherResult<HashMap<String, RegistryItem>> {
        let conn = self.connection()?;
        let mut items = HashMap::new();
        if item_ids.is_empty() {
            return Ok(items);
        }
        for chunk in item_ids.chunks(500) {
            let placeholders = (1..=chunk.len())
                .map(|index| format!("?{index}"))
                .collect::<Vec<_>>()
                .join(", ");
            let query = format!(
                "SELECT {REGISTRY_ITEM_COLUMNS} FROM registry_items ri WHERE ri.id IN ({placeholders})"
            );
            let mut statement = conn
                .prepare(&query)
                .map_err(|error| LauncherError::Generic {
                    code: "ERR_INVALID_QUERY".to_string(),
                    message: error.to_string(),
                })?;
            let rows = statement
                .query_map(rusqlite::params_from_iter(chunk.iter()), row_to_item)
                .map_err(|error| LauncherError::Generic {
                    code: "ERR_INVALID_QUERY".to_string(),
                    message: error.to_string(),
                })?;
            for row in rows {
                let item = row.map_err(|error| LauncherError::Generic {
                    code: "ERR_INVALID_QUERY".to_string(),
                    message: error.to_string(),
                })?;
                items.insert(item.id.clone(), item);
            }
        }
        Ok(items)
    }

    /// Fetch category display names for registry items in one query.
    pub fn get_item_categories(
        &self,
        item_ids: &[String],
    ) -> LauncherResult<HashMap<String, Vec<String>>> {
        let conn = self.connection()?;
        let mut categories = HashMap::new();
        if item_ids.is_empty() {
            return Ok(categories);
        }
        for chunk in item_ids.chunks(500) {
            let placeholders = (1..=chunk.len())
                .map(|index| format!("?{index}"))
                .collect::<Vec<_>>()
                .join(", ");
            let query = format!(
                "SELECT ic.item_id, c.display_name FROM item_categories ic JOIN categories c ON c.id = ic.category_id WHERE ic.item_id IN ({placeholders}) ORDER BY c.display_name"
            );
            let mut statement = conn
                .prepare(&query)
                .map_err(|error| LauncherError::Generic {
                    code: "ERR_INVALID_QUERY".to_string(),
                    message: error.to_string(),
                })?;
            let rows = statement
                .query_map(rusqlite::params_from_iter(chunk.iter()), |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|error| LauncherError::Generic {
                    code: "ERR_INVALID_QUERY".to_string(),
                    message: error.to_string(),
                })?;
            for row in rows {
                let (item_id, category) = row.map_err(|error| LauncherError::Generic {
                    code: "ERR_INVALID_QUERY".to_string(),
                    message: error.to_string(),
                })?;
                categories
                    .entry(item_id)
                    .or_insert_with(Vec::new)
                    .push(category);
            }
        }
        Ok(categories)
    }

    /// Fetch curated author values when the registry was compiled with the
    /// manifest author column. Older signed registries degrade to no authors.
    pub fn get_item_authors(&self, item_ids: &[String]) -> LauncherResult<HashMap<String, String>> {
        let conn = self.connection()?;
        let mut authors = HashMap::new();
        if item_ids.is_empty() {
            return Ok(authors);
        }
        for chunk in item_ids.chunks(500) {
            let placeholders = (1..=chunk.len())
                .map(|index| format!("?{index}"))
                .collect::<Vec<_>>()
                .join(", ");
            let query =
                format!("SELECT id, author FROM registry_items WHERE id IN ({placeholders})");
            let mut statement = match conn.prepare(&query) {
                Ok(statement) => statement,
                Err(_) => return Ok(HashMap::new()),
            };
            let rows = statement
                .query_map(rusqlite::params_from_iter(chunk.iter()), |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
                })
                .map_err(|error| LauncherError::Generic {
                    code: "ERR_INVALID_QUERY".to_string(),
                    message: error.to_string(),
                })?;
            for row in rows {
                let (id, author) = row.map_err(|error| LauncherError::Generic {
                    code: "ERR_INVALID_QUERY".to_string(),
                    message: error.to_string(),
                })?;
                if let Some(author) = author.filter(|value| !value.trim().is_empty()) {
                    authors.insert(id, author);
                }
            }
        }
        Ok(authors)
    }

    /// Browse registry items with typed filters.
    /// Registry-wide average approval rate, the anchor for the vote nudge.
    pub fn mean_approval(&self) -> f64 {
        match self.connection() {
            Ok(conn) => registry_mean_approval(&conn),
            Err(_) => crate::ranking::DEFAULT_REGISTRY_MEAN_APPROVAL,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn browse_items(
        &self,
        content_type: Option<&str>,
        category: Option<&str>,
        sort: &SortOption,
        curated_strategies: &[String],
        mc_version: Option<&str>,
        loader: Option<&str>,
        query: Option<&str>,
        limit: i64,
    ) -> LauncherResult<Vec<RegistryItem>> {
        let conn = self.connection()?;
        browse_items(
            &conn,
            content_type,
            category,
            sort,
            curated_strategies,
            mc_version,
            loader,
            query,
            limit,
        )
    }

    /// List all categories from the registry.
    pub fn list_categories(&self) -> LauncherResult<Vec<CategoryInfo>> {
        let conn = self.connection()?;
        list_categories(&conn)
    }

    /// Look up a curated annotation for a registry item by its registry id.
    pub fn get_curated_annotation(
        &self,
        item_id: &str,
    ) -> LauncherResult<Option<CuratedAnnotation>> {
        let conn = self.connection()?;
        get_curated_annotation(&conn, item_id)
    }

    /// Batch compatibility lookup: for each item_id, resolve whether it declares
    /// compatibility with the given minecraft_version + loader.
    pub fn batch_compat_lookup(
        &self,
        item_ids: &[String],
        minecraft_version: &str,
        loader: &str,
    ) -> LauncherResult<std::collections::BTreeMap<String, String>> {
        let conn = self.connection()?;
        let mut result = std::collections::BTreeMap::new();
        for item_id in item_ids {
            let status = get_item_by_id(&conn, item_id)?
                .and_then(|item| item.compatible_versions_json)
                .map(|json| compatibility_from_registry_json(&json, minecraft_version, loader))
                .unwrap_or_default();
            result.insert(item_id.clone(), status);
        }
        Ok(result)
    }

    /// List all mods in a pack, ordered by mod_id.
    pub fn pack_mods_for_pack(&self, pack_id: &str) -> LauncherResult<Vec<PackModRow>> {
        let conn = self.connection()?;
        pack_mods_for_pack(&conn, pack_id)
    }

    /// List audit log entries (newest first); defensively returns `[]` if the
    /// `audit_log` table does not exist in older registry builds.
    pub fn list_audit_log(&self, limit: i64) -> LauncherResult<Vec<AuditLogEntry>> {
        let conn = self.connection()?;
        list_audit_log(&conn, limit)
    }

    /// List registry items whose status is `under_review`, ordered by net_score
    /// ascending (lowest scores first for triage).
    pub fn list_under_review_items(&self) -> LauncherResult<Vec<UnderReviewItem>> {
        let conn = self.connection()?;
        list_under_review_items(&conn)
    }

    /// List recent triage resolutions from the audit log.
    pub fn list_recent_resolutions(&self, limit: u32) -> LauncherResult<Vec<AuditLogEntry>> {
        let conn = self.connection()?;
        list_recent_resolutions(&conn, limit)
    }

    /// Load parsed curator reviews for a single registry item.
    pub fn list_mod_reviews(&self, item_id: String) -> LauncherResult<Vec<ModReview>> {
        let conn = self.connection()?;
        list_mod_reviews(&conn, item_id)
    }

    /// "For You" items (§6.2): boost uninstalled mods whose categories overlap
    /// with the categories of the user's installed mods.
    ///
    /// Ranking: items matching MORE of the user's interest categories rank higher
    /// (`COUNT(ic.category_id) DESC`), with `net_score DESC` as a tiebreak.
    /// Already-installed items are excluded. Degrades to a plain `net_score`
    /// ordering when the user has no installed mods with registry ids, or when
    /// those items expose no resolvable categories.
    #[allow(clippy::too_many_arguments)]
    pub fn for_you_items(
        &self,
        curated_strategies: &[String],
        mc_version: Option<&str>,
        loader: Option<&str>,
        limit: i64,
        modrinth_categories: Option<&[String]>,
        query: Option<&str>,
    ) -> LauncherResult<Vec<RegistryItem>> {
        let installed = collect_installed_registry_ids(&self.ctx.paths.instances_root())?;
        let conn = self.connection()?;
        for_you_items_query(
            &conn,
            &installed,
            curated_strategies,
            mc_version,
            loader,
            limit,
            modrinth_categories,
            query,
        )
    }

    /// Fetch manifest-declared dependencies for a single registry item.
    ///
    /// Calls through to the free `get_manifest_dependencies` function using
    /// an opened read-only connection from the service's context.
    /// Returns `Ok(None)` when the item has no curated dependencies or the
    /// `mod_manual_dependencies` table does not exist.
    pub fn get_manifest_dependencies(&self, item_id: &str) -> LauncherResult<Option<ManifestDeps>> {
        let conn = self.connection()?;
        get_manifest_dependencies(&conn, item_id.to_string())
    }

    /// List all mod jar aliases as `(registry_id, alias)` pairs.
    ///
    /// Defensively returns an empty vec when the `mod_jar_aliases` table does
    /// not exist (older registry.db builds predate this table).
    pub fn get_all_mod_aliases(&self) -> LauncherResult<Vec<(String, String)>> {
        let conn = self.connection()?;
        get_all_mod_aliases(&conn)
    }

    /// List known conflicts from the curated `known_conflicts` table.
    ///
    /// Defensively returns an empty vec when the table does not exist (older
    /// registry.db builds predate this table).
    pub fn known_conflicts(&self) -> LauncherResult<Vec<KnownConflict>> {
        let conn = self.connection()?;
        get_known_conflicts(&conn)
    }

    /// Check whether a registry item has status `under_review`.
    ///
    /// Returns `false` when the item does not exist or when the registry DB
    /// cannot be opened (graceful degradation).
    pub fn is_under_review(&self, item_id: &str) -> LauncherResult<bool> {
        let conn = match self.connection() {
            Ok(c) => c,
            Err(_) => return Ok(false),
        };
        let mut stmt = match conn.prepare(
            "SELECT 1 FROM registry_items WHERE id = ?1 AND status = 'under_review' LIMIT 1",
        ) {
            Ok(s) => s,
            Err(_) => return Ok(false),
        };
        Ok(stmt.exists([item_id]).unwrap_or(false))
    }

    // -----------------------------------------------------------------------
    // Governance schema-7 queries
    // -----------------------------------------------------------------------

    /// Fetch a governance summary for a single item from `governance_summary`.
    ///
    /// Returns `Ok(None)` when the `governance_summary` table does not exist
    /// (schema 6 or earlier) or when the item has no row.
    pub fn get_governance_summary(
        &self,
        item_id: &str,
    ) -> LauncherResult<Option<GovernanceSummary>> {
        let conn = self.connection()?;
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
        let conn = self.connection()?;
        list_governance_events(&conn, item_id, limit)
    }

    // -----------------------------------------------------------------------
    // Per-version changelogs (schema 9)
    // -----------------------------------------------------------------------

    /// List all known changelogs for a single item, newest first.
    ///
    /// Returns `vec![]` when the `item_version_changelogs` table does not
    /// exist (schema < 9) or the item has no changelog rows. The table is
    /// source-agnostic: Modrinth and GitHub rows share the same shape so a
    /// `github_release` item and a `modrinth_id` item are queried the same way.
    pub fn get_version_changelogs(
        &self,
        item_id: &str,
    ) -> LauncherResult<Vec<crate::version_changelogs::VersionChangelog>> {
        let conn = self.connection()?;
        crate::version_changelogs::get_changelogs_for_item(&conn, item_id)
    }

    /// Fetch the changelog for a single `(item_id, version)` pair, if present.
    ///
    /// `version` must be the exact display string (`InstalledMod.version` /
    /// `ModVersionCandidate.version`). Returns `Ok(None)` on older databases
    /// or when the version has no stored changelog (e.g. empty upstream
    /// changelog, `direct_hash` item, or version beyond the 30-row cap).
    pub fn get_version_changelog(
        &self,
        item_id: &str,
        version: &str,
    ) -> LauncherResult<Option<crate::version_changelogs::VersionChangelog>> {
        let conn = self.connection()?;
        crate::version_changelogs::get_changelog_for_version(&conn, item_id, version)
    }

    /// What changed between `from_version` (installed) and `to_version`
    /// (candidate), newest first.
    ///
    /// This is the offline answer to "should I update?". It returns every
    /// stored changelog newer than `from_version` up to and including
    /// `to_version`. See
    /// [`crate::version_changelogs::get_changelogs_between`] for the full
    /// interval semantics (unknown `from`, missing `to`, downgrade, etc.) and
    /// why an empty vec is not an error.
    pub fn get_changelogs_between(
        &self,
        item_id: &str,
        from_version: &str,
        to_version: &str,
    ) -> LauncherResult<Vec<crate::version_changelogs::VersionChangelog>> {
        let conn = self.connection()?;
        crate::version_changelogs::get_changelogs_between(&conn, item_id, from_version, to_version)
    }
}

// ---------------------------------------------------------------------------
// Dev-registry standalone check (no Ctx required)
// ---------------------------------------------------------------------------

/// Check whether `AGORA_DEV_REGISTRY_DB` is set as a compile-time or
/// environment override.  Returns `true` when the env var is present
/// (debug builds only).
///
/// This is a lightweight heuristic used by [`resolve_governance_config`]
/// when a `RegistryService` is not available.  The authoritative check
/// is [`RegistryService::development_registry_active`], which also
/// validates the path and schema.
pub fn check_dev_registry_env_set() -> bool {
    #[cfg(debug_assertions)]
    {
        std::env::var("AGORA_DEV_REGISTRY_DB").is_ok()
    }
    #[cfg(not(debug_assertions))]
    {
        false
    }
}

/// Canonical column list for `registry_items` selects that feed `row_to_item`.
///
/// Single source of truth: `browse_items` and `for_you_items` both select this
/// exact list (with the `ri.` alias) so a schema column addition only needs
/// to be reflected here + in `row_to_item`'s positional reads, not in each query.
/// `get_item_by_id` is a single-table query and uses unprefixed names, so it
/// is kept in sync separately.
pub const REGISTRY_ITEM_COLUMNS: &str = "ri.id, ri.name, ri.content_type, ri.download_strategy,
        ri.source_identifier, ri.sha256, ri.upvotes, ri.downvotes,
        ri.net_score, ri.velocity, ri.status, ri.is_immune,
        ri.immunity_reason, ri.allow_comments, ri.icon_url,
        ri.gallery_urls_json, ri.date_added, ri.compatible_versions_json,
        ri.description, ri.body_markdown, ri.page_url, ri.license_id,
        ri.source_updated_at, ri.modrinth_id, ri.download_sources_json";

/// A registry item row for browsing (§6.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryItem {
    pub id: String,
    pub name: String,
    pub content_type: String,
    pub download_strategy: String,
    pub source_identifier: String,
    pub sha256: String,
    pub upvotes: i64,
    pub downvotes: i64,
    pub net_score: i64,
    pub velocity: f64,
    pub status: String,
    pub is_immune: bool,
    pub immunity_reason: Option<String>,
    pub allow_comments: bool,
    pub icon_url: Option<String>,
    pub gallery_urls_json: Option<String>,
    pub date_added: Option<String>,
    pub compatible_versions_json: Option<String>,
    /// Short tagline hydrated from the upstream source (e.g. Modrinth).
    pub description: Option<String>,
    /// Full markdown body hydrated from the upstream source. Community-authored
    /// markdown — must be rendered without `dangerouslySetInnerHTML`.
    pub body_markdown: Option<String>,
    /// Canonical upstream page URL (e.g. https://modrinth.com/mod/<slug>).
    pub page_url: Option<String>,
    /// SPDX-ish license id hydrated from the upstream source.
    pub license_id: Option<String>,
    /// ISO timestamp of the upstream project's last update (nightly snapshot).
    pub source_updated_at: Option<String>,
    /// Optional Modrinth project id (github_release/direct_hash mods may carry
    /// one when the project also exists on Modrinth). Used as the
    /// version-resolution fallback when the primary source fails.
    pub modrinth_id: Option<String>,
    /// Ordered download sources, best first, as the JSON array the compiler
    /// wrote. Prefer [`RegistryItem::download_sources`] over parsing this
    /// directly — it also covers pre-schema-8 registries, which have no such
    /// column and describe only their preferred source.
    #[serde(default)]
    pub download_sources_json: Option<String>,
    /// Present only for recommendation queries. Explains the concrete ranking
    /// signal instead of asking the frontend to invent generic copy.
    #[serde(default)]
    pub recommendation_reason: Option<String>,
    /// Number of interest categories shared with locally installed items.
    #[serde(default)]
    pub recommendation_overlap: Option<i64>,
}

impl RegistryItem {
    /// Ordered download sources for this item, best first.
    ///
    /// Falls back to the legacy `download_strategy` / `source_identifier` pair
    /// when the row carries no source list — either a pre-schema-8 registry, or
    /// a row written before the compiler normalized the field. The implicit
    /// Modrinth fallback that a `modrinth_id`-carrying entry has always had is
    /// reconstructed in that path too, so behaviour is unchanged for old data.
    ///
    /// The result is never empty: a row with nothing usable still yields its
    /// declared strategy, and the resolver reports the failure per source
    /// rather than silently resolving to nothing.
    pub fn download_sources(&self) -> Vec<DownloadSource> {
        let parsed: Vec<DownloadSource> = self
            .download_sources_json
            .as_deref()
            .map(str::trim)
            .filter(|json| !json.is_empty())
            .and_then(|json| serde_json::from_str::<Vec<DownloadSource>>(json).ok())
            .unwrap_or_default();

        let mut sources: Vec<DownloadSource> = parsed
            .into_iter()
            .filter(|source| !source.strategy.trim().is_empty())
            .collect();

        if sources.is_empty() {
            sources.push(DownloadSource {
                strategy: self.download_strategy.clone(),
                identifier: self.source_identifier.clone(),
            });
            if let Some(modrinth_id) = self
                .modrinth_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
            {
                if self.download_strategy != "modrinth_id" {
                    sources.push(DownloadSource {
                        strategy: "modrinth_id".into(),
                        identifier: modrinth_id.to_string(),
                    });
                }
            }
        }
        sources
    }

    /// Whether any of this item's download sources uses `strategy`.
    pub fn has_download_source(&self, strategy: &str) -> bool {
        self.download_sources()
            .iter()
            .any(|source| source.strategy == strategy)
    }
}

/// Valid sort options for browsing (§6.2).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortOption {
    #[default]
    NetScore,
    Velocity,
    MostDownvoted,
    Newest,
    MostUpvoted,
}

impl SortOption {
    fn order_clause(&self) -> &'static str {
        match self {
            Self::NetScore => "ORDER BY net_score DESC",
            Self::Velocity => "ORDER BY velocity DESC",
            Self::MostDownvoted => "ORDER BY downvotes DESC",
            Self::Newest => "ORDER BY date_added DESC",
            Self::MostUpvoted => "ORDER BY upvotes DESC",
        }
    }
}

/// Browse registry items with optional filters (§6.2).
///
/// `curated_strategies` is the whitelist of curated download strategies to
/// show (see [`CURATED_DOWNLOAD_STRATEGIES`]): items whose strategy is not
/// listed are excluded. This is the per-source content axis — it is separate
/// from live third-party browsing gating. `mc_version` and `loader` filter
/// against `compatible_versions_json` using SQLite's JSON1 functions (same
/// approach as the web `browseItems`).
#[allow(clippy::too_many_arguments)]
pub fn browse_items(
    conn: &Connection,
    content_type: Option<&str>,
    category: Option<&str>,
    sort: &SortOption,
    curated_strategies: &[String],
    mc_version: Option<&str>,
    loader: Option<&str>,
    query: Option<&str>,
    limit: i64,
) -> LauncherResult<Vec<RegistryItem>> {
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    let mut where_parts: Vec<String> = Vec::new();

    if let Some(ct) = content_type {
        where_parts.push("ri.content_type = ?".to_string());
        params.push(Box::new(ct.to_string()));
    }
    if let Some(q) = query {
        let trimmed = q.trim();
        if !trimmed.is_empty() {
            where_parts.push("ri.name LIKE ?".to_string());
            params.push(Box::new(format!("%{}%", trimmed)));
        }
    }
    if let Some(cat) = category {
        where_parts.push("ic.category_id = ?".to_string());
        params.push(Box::new(cat.to_string()));
    }
    if let (Some(clause), values) = curated_strategy_whitelist(curated_strategies) {
        where_parts.push(clause);
        for value in values {
            params.push(Box::new(value));
        }
    }
    if let Some(mv) = mc_version {
        // Only items declaring a compatible_version for this MC version match.
        // json_each over an empty/NULL array yields no rows -> the item is
        // excluded, which is the desired behaviour (no declared support).
        where_parts.push(
            "ri.id IN (SELECT r.id FROM registry_items r, json_each(r.compatible_versions_json) cv \
             WHERE json_extract(cv.value, '$.mc_version') = ?)"
                .to_string(),
        );
        params.push(Box::new(mv.to_string()));
    }
    if let Some(ld) = loader {
        if ld != "all" {
            where_parts.push(
                "ri.id IN (SELECT r.id FROM registry_items r, json_each(r.compatible_versions_json) cv \
                 WHERE json_extract(cv.value, '$.loader') = ?)"
                    .to_string(),
            );
            params.push(Box::new(ld.to_string()));
        }
    }

    let where_clause = if where_parts.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", where_parts.join(" AND "))
    };

    let join = if category.is_some() {
        " JOIN item_categories ic ON ri.id = ic.item_id"
    } else {
        ""
    };

    let sql = format!(
        "SELECT {REGISTRY_ITEM_COLUMNS}
         FROM registry_items ri{join}{where_clause} {order} LIMIT ?",
        join = join,
        where_clause = where_clause,
        order = sort.order_clause(),
    );

    params.push(Box::new(limit));

    let mut stmt = conn.prepare(&sql).map_err(|e| LauncherError::Generic {
        code: "ERR_INVALID_QUERY".to_string(),
        message: e.to_string(),
    })?;

    let rows = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), row_to_item)
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?;

    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?);
    }
    Ok(out)
}

/// Fetch registry items by their ids in one batched read session.
///
/// Returns a map keyed by the item id as stored. Items that are not present
/// in the registry are simply absent from the map.
pub fn get_items_by_ids(
    conn: &Connection,
    item_ids: &[String],
) -> LauncherResult<HashMap<String, RegistryItem>> {
    let mut items = HashMap::new();
    if item_ids.is_empty() {
        return Ok(items);
    }
    for chunk in item_ids.chunks(500) {
        let placeholders = (1..=chunk.len())
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let query = format!(
            "SELECT {REGISTRY_ITEM_COLUMNS} FROM registry_items ri WHERE ri.id IN ({placeholders})"
        );
        let mut statement = conn
            .prepare(&query)
            .map_err(|error| LauncherError::Generic {
                code: "ERR_INVALID_QUERY".to_string(),
                message: error.to_string(),
            })?;
        let rows = statement
            .query_map(rusqlite::params_from_iter(chunk.iter()), row_to_item)
            .map_err(|error| LauncherError::Generic {
                code: "ERR_INVALID_QUERY".to_string(),
                message: error.to_string(),
            })?;
        for row in rows {
            let item = row.map_err(|error| LauncherError::Generic {
                code: "ERR_INVALID_QUERY".to_string(),
                message: error.to_string(),
            })?;
            items.insert(item.id.clone(), item);
        }
    }
    Ok(items)
}

/// Fetch a single registry item by ID.
pub fn get_item_by_id(conn: &Connection, item_id: &str) -> LauncherResult<Option<RegistryItem>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, content_type, download_strategy,
                    source_identifier, sha256, upvotes, downvotes,
                    net_score, velocity, status, is_immune,
                    immunity_reason, allow_comments, icon_url,
                    gallery_urls_json, date_added, compatible_versions_json,
                    description, body_markdown, page_url, license_id,
                    source_updated_at, modrinth_id, download_sources_json
             FROM registry_items WHERE id = ?1",
        )
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?;

    let mut rows = stmt
        .query_map([item_id], row_to_item)
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?;

    if let Some(r) = rows.next() {
        Ok(Some(r.map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?))
    } else {
        Ok(None)
    }
}

/// Category info for the browse filter UI.
#[derive(Debug, Clone, Serialize)]
pub struct CategoryInfo {
    pub id: String,
    pub display_name: String,
    pub is_community: bool,
    pub content_types: Vec<String>,
}

/// List all categories from the registry.
pub fn list_categories(conn: &Connection) -> LauncherResult<Vec<CategoryInfo>> {
    let mut stmt = conn
        .prepare(
            "SELECT c.id, c.display_name, c.is_community,
                    GROUP_CONCAT(DISTINCT ri.content_type)
             FROM categories c
             LEFT JOIN item_categories ic ON ic.category_id = c.id
             LEFT JOIN registry_items ri ON ri.id = ic.item_id
             GROUP BY c.id, c.display_name, c.is_community
             ORDER BY c.display_name",
        )
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?;

    let rows = stmt
        .query_map([], |row| {
            let mut content_types = row
                .get::<_, Option<String>>(3)?
                .unwrap_or_default()
                .split(',')
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>();
            content_types.sort();
            Ok(CategoryInfo {
                id: row.get(0)?,
                display_name: row.get(1)?,
                is_community: row.get(2)?,
                content_types,
            })
        })
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?;

    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?);
    }
    Ok(out)
}

/// A row from the `pack_mods` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackModRow {
    pub pack_id: String,
    pub mod_id: String,
    pub source: String,
    pub version: Option<String>,
    pub status: String,
    pub description: Option<String>,
}

/// List all mods in a pack, ordered by mod_id.
pub fn pack_mods_for_pack(conn: &Connection, pack_id: &str) -> LauncherResult<Vec<PackModRow>> {
    let mut stmt = conn
        .prepare(
            "SELECT pack_id, mod_id, source, version, status, description \
             FROM pack_mods WHERE pack_id = ?1 ORDER BY mod_id ASC",
        )
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?;

    let rows = stmt
        .query_map([pack_id], |row| {
            Ok(PackModRow {
                pack_id: row.get(0)?,
                mod_id: row.get(1)?,
                source: row.get(2)?,
                version: row.get(3)?,
                status: row.get(4)?,
                description: row.get(5)?,
            })
        })
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?;

    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?);
    }
    Ok(out)
}

/// A row from the `audit_log` transparency table (§4.6).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLogEntry {
    pub id: i64,
    pub timestamp: String,
    pub action: String,
    pub details: Option<String>,
}

/// List audit log entries from the registry DB (newest first by id).
///
/// Defensively returns an empty vec if the `audit_log` table does not exist
/// (older registry.db builds predate this table) so the UI degrades gracefully.
pub fn list_audit_log(conn: &Connection, limit: i64) -> LauncherResult<Vec<AuditLogEntry>> {
    let has_table: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='audit_log'",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);
    if !has_table {
        return Ok(Vec::new());
    }

    let mut stmt = conn
        .prepare("SELECT id, timestamp, action, details FROM audit_log ORDER BY id DESC LIMIT ?1")
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?;
    let rows = stmt
        .query_map([limit], |row| {
            Ok(AuditLogEntry {
                id: row.get(0)?,
                timestamp: row.get(1)?,
                action: row.get(2)?,
                details: row.get(3)?,
            })
        })
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?);
    }
    Ok(out)
}

pub fn row_to_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<RegistryItem> {
    Ok(RegistryItem {
        id: row.get(0)?,
        name: row.get(1)?,
        content_type: row.get(2)?,
        download_strategy: row.get(3)?,
        source_identifier: row.get(4)?,
        sha256: row.get(5)?,
        upvotes: row.get(6)?,
        downvotes: row.get(7)?,
        net_score: row.get(8)?,
        velocity: row.get(9)?,
        status: row.get(10)?,
        is_immune: row.get(11)?,
        immunity_reason: row.get(12)?,
        allow_comments: row.get(13)?,
        icon_url: row.get(14)?,
        gallery_urls_json: row.get(15)?,
        date_added: row.get(16)?,
        compatible_versions_json: row.get(17)?,
        description: row.get(18)?,
        body_markdown: row.get(19)?,
        page_url: row.get(20)?,
        license_id: row.get(21)?,
        source_updated_at: row.get(22)?,
        modrinth_id: row.get(23)?,
        download_sources_json: row.get(24)?,
        recommendation_reason: None,
        recommendation_overlap: None,
    })
}

/// An under-review registry item for the Triage Center.
#[derive(Debug, Clone, Serialize)]
pub struct UnderReviewItem {
    pub id: String,
    pub name: String,
    pub content_type: String,
    pub icon_url: Option<String>,
    pub net_score: i64,
}

/// List registry items whose status is `under_review`, ordered by net_score
/// ascending (lowest scores first for triage).
pub fn list_under_review_items(conn: &Connection) -> LauncherResult<Vec<UnderReviewItem>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, content_type, icon_url, net_score \
             FROM registry_items WHERE status = 'under_review' \
             ORDER BY net_score ASC",
        )
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?;

    let rows = stmt
        .query_map([], |row| {
            Ok(UnderReviewItem {
                id: row.get(0)?,
                name: row.get(1)?,
                content_type: row.get(2)?,
                icon_url: row.get(3)?,
                net_score: row.get(4)?,
            })
        })
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?;

    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?);
    }
    Ok(out)
}

/// List recent triage resolutions from the audit log.
///
/// Defensively returns an empty vec if the `audit_log` table does not exist.
pub fn list_recent_resolutions(
    conn: &Connection,
    limit: u32,
) -> LauncherResult<Vec<AuditLogEntry>> {
    let has_table: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='audit_log'",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);
    if !has_table {
        return Ok(Vec::new());
    }

    let mut stmt = conn
        .prepare(
            "SELECT id, timestamp, action, details FROM audit_log \
             WHERE action IN ('triage_archive','triage_keep',
                              'organic_under_review','raid_breaker_offenders') \
             ORDER BY id DESC LIMIT ?",
        )
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?;
    let rows = stmt
        .query_map([limit as i64], |row| {
            Ok(AuditLogEntry {
                id: row.get(0)?,
                timestamp: row.get(1)?,
                action: row.get(2)?,
                details: row.get(3)?,
            })
        })
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?);
    }
    Ok(out)
}

/// A parsed curator review from a `curator_reviews.top_reviews_json` cell.
#[derive(Debug, Clone, Serialize)]
pub struct ModReview {
    pub author: Option<String>,
    pub text: String,
    pub issue_number: i64,
    pub created_at: Option<String>,
}

/// Load parsed curator reviews for a single item.
///
/// Reads the JSON array from `curator_reviews.top_reviews_json`, parses it,
/// and returns the resulting `ModReview` list. On NULL/empty/missing row or
/// parse error, returns `Ok(vec![])`.
pub fn list_mod_reviews(conn: &Connection, item_id: String) -> LauncherResult<Vec<ModReview>> {
    let mut stmt = conn
        .prepare("SELECT top_reviews_json FROM curator_reviews WHERE item_id = ?1")
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?;

    let raw: Option<String> = stmt.query_row([item_id], |row| row.get(0)).ok();

    let json_str = match raw {
        Some(s) if !s.is_empty() => s,
        _ => return Ok(Vec::new()),
    };

    let parsed: Vec<serde_json::Value> = match serde_json::from_str(&json_str) {
        Ok(v) => v,
        Err(_) => return Ok(Vec::new()),
    };

    let mut out = Vec::new();
    for val in parsed {
        out.push(ModReview {
            author: val.get("author").and_then(|a| a.as_str()).map(String::from),
            text: val
                .get("text")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string(),
            issue_number: val
                .get("issue_number")
                .and_then(|i| i.as_i64())
                .unwrap_or(0),
            created_at: val
                .get("created_at")
                .and_then(|c| c.as_str())
                .map(String::from),
        });
    }
    Ok(out)
}

/// A known conflict between two mods (§4.6).
///
/// A conflict may be *unconditional* (the two mods never get along) or
/// *version-windowed* — "mod A together with mod B at 2.3 or later crashes".
/// The windows are per-side range lists interpreted by
/// [`crate::version_match`], the same matcher used for jar-declared
/// incompatibilities, so curated facts and loader metadata never disagree
/// about what `">=2.3"` means. An empty window is unconditional, which is what
/// every pre-window curated fact and every older `registry.db` decays to.
#[derive(Debug, Clone, Serialize)]
pub struct KnownConflict {
    pub mod_a_id: String,
    pub mod_b_id: String,
    pub severity: String,
    pub mitigated_by: Vec<String>,
    pub notes: Option<String>,
    /// Version ranges for `mod_a_id`. Empty means "any version".
    pub mod_a_versions: Vec<String>,
    /// Version ranges for `mod_b_id`. Empty means "any version".
    pub mod_b_versions: Vec<String>,
    /// Grammar both windows are written in. Fabric predicate syntax unless the
    /// curator says `"maven"`.
    pub version_grammar: VersionGrammar,
}

impl KnownConflict {
    /// True when this conflict is version-windowed on either side.
    pub fn is_version_windowed(&self) -> bool {
        !self.mod_a_versions.is_empty() || !self.mod_b_versions.is_empty()
    }

    /// Whether one side's window admits `installed_version`.
    ///
    /// An unconditional window admits anything, including an unknown version.
    /// A *conditional* window with an unknown installed version does **not**
    /// match: a curator said "only 2.3 and up", and we cannot tell whether
    /// this jar is 2.3, so warning anyway would put a blocker in front of a
    /// user who may well be fine. This mirrors how `health.rs` treats
    /// jar-declared incompatibilities with unparseable versions.
    fn side_applies(&self, ranges: &[String], installed_version: Option<&str>) -> bool {
        if ranges.is_empty() {
            return true;
        }
        let Some(version) = installed_version.map(str::trim).filter(|v| !v.is_empty()) else {
            return matches!(
                evaluate_version_match(ranges, "", self.version_grammar == VersionGrammar::Fabric),
                VersionMatch::Unconditional
            );
        };
        matches!(
            evaluate_version_match(
                ranges,
                version,
                self.version_grammar == VersionGrammar::Fabric
            ),
            VersionMatch::Matched | VersionMatch::Unconditional
        )
    }

    /// Whether this conflict applies to the versions actually installed.
    ///
    /// Both sides must be admitted. Callers pass `None` for a version they do
    /// not know; see [`KnownConflict::side_applies`] for why that is not the
    /// same as a match.
    pub fn applies_to_versions(
        &self,
        mod_a_version: Option<&str>,
        mod_b_version: Option<&str>,
    ) -> bool {
        self.side_applies(&self.mod_a_versions, mod_a_version)
            && self.side_applies(&self.mod_b_versions, mod_b_version)
    }
}

/// A parsed mod dependency set from `mod_manual_dependencies`.
#[derive(Debug, Clone, Serialize)]
pub struct ManifestDeps {
    pub required: Vec<String>,
    pub optional: Vec<String>,
    pub incompatible: Vec<String>,
}

/// Return manual dependency information for a single item from the
/// `mod_manual_dependencies` table.
///
/// Defensively returns `Ok(None)` if the `mod_manual_dependencies` table does
/// not exist (older registry.db builds predate this table) so the UI degrades
/// gracefully. Parses each JSON column string into `Vec<String>`; NULL/empty/
/// parse errors yield an empty vec.
pub fn get_manifest_dependencies(
    conn: &Connection,
    item_id: String,
) -> LauncherResult<Option<ManifestDeps>> {
    let has_table: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='mod_manual_dependencies'",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);
    if !has_table {
        return Ok(None);
    }

    let mut stmt = conn
        .prepare(
            "SELECT required_json, optional_json, incompatible_json \
             FROM mod_manual_dependencies WHERE item_id = ?",
        )
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?;

    let mut row = stmt
        .query_map([item_id], |row| {
            let required_json: Option<String> = row.get(0)?;
            let optional_json: Option<String> = row.get(1)?;
            let incompatible_json: Option<String> = row.get(2)?;

            let required: Vec<String> = required_json
                .filter(|s| !s.is_empty())
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();

            let optional: Vec<String> = optional_json
                .filter(|s| !s.is_empty())
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();

            let incompatible: Vec<String> = incompatible_json
                .filter(|s| !s.is_empty())
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();

            Ok(ManifestDeps {
                required,
                optional,
                incompatible,
            })
        })
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?;

    match row.next() {
        Some(Ok(deps)) => Ok(Some(deps)),
        Some(Err(e)) => Err(LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        }),
        None => Ok(None),
    }
}

/// Return manual dependency information for ALL items that have curated
/// dependencies in the `mod_manual_dependencies` table.
///
/// Defensively returns an empty map if the table does not exist (older
/// registry.db builds predate this table).
pub fn get_all_manifest_dependencies(
    conn: &Connection,
) -> LauncherResult<std::collections::HashMap<String, ManifestDeps>> {
    let has_table: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='mod_manual_dependencies'",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);
    if !has_table {
        return Ok(std::collections::HashMap::new());
    }

    let mut stmt = conn
        .prepare(
            "SELECT item_id, required_json, optional_json, incompatible_json \
             FROM mod_manual_dependencies",
        )
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?;

    let rows = stmt
        .query_map([], |row| {
            let item_id: String = row.get(0)?;
            let required_json: Option<String> = row.get(1)?;
            let optional_json: Option<String> = row.get(2)?;
            let incompatible_json: Option<String> = row.get(3)?;

            let required: Vec<String> = required_json
                .filter(|s| !s.is_empty())
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            let optional: Vec<String> = optional_json
                .filter(|s| !s.is_empty())
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            let incompatible: Vec<String> = incompatible_json
                .filter(|s| !s.is_empty())
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();

            Ok((
                item_id,
                ManifestDeps {
                    required,
                    optional,
                    incompatible,
                },
            ))
        })
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?;

    let mut out = std::collections::HashMap::new();
    for (item_id, deps) in rows.flatten() {
        out.insert(item_id, deps);
    }
    Ok(out)
}

/// Average approval rate (`upvotes / (upvotes + downvotes)`) across every
/// registry item that has at least one vote.
///
/// This is the anchor the vote nudge pulls low-vote items toward, so a brand-new
/// item sits near the registry norm rather than at an extreme. Returns
/// [`ranking::DEFAULT_REGISTRY_MEAN_APPROVAL`] when nothing has been voted on —
/// which is the case for the entire shipped registry today.
pub fn registry_mean_approval(conn: &Connection) -> f64 {
    let mean: Option<f64> = conn
        .query_row(
            "SELECT AVG(CAST(upvotes AS REAL) / (upvotes + downvotes))              FROM registry_items              WHERE status = 'active' AND (upvotes + downvotes) > 0",
            [],
            |row| row.get(0),
        )
        .ok()
        .flatten();
    match mean {
        Some(value) if value.is_finite() => value.clamp(0.0, 1.0),
        _ => crate::ranking::DEFAULT_REGISTRY_MEAN_APPROVAL,
    }
}

/// List `(item_id, display_name)` for every active catalog mod.
///
/// Used by the health scan to tell a player whether a missing dependency is
/// something Agora can install for them or something they have to source
/// themselves. Only the id and name are read, so this stays cheap enough to
/// run on the pre-launch path.
pub fn get_catalog_mod_names(conn: &Connection) -> LauncherResult<Vec<(String, String)>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name FROM registry_items \
             WHERE content_type = 'mod' AND status = 'active'",
        )
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?;

    let rows = stmt
        .query_map([], |row| {
            let id: String = row.get(0)?;
            let name: String = row.get(1)?;
            Ok((id, name))
        })
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?;

    Ok(rows.flatten().collect())
}

/// List all mod JAR aliases from the `mod_jar_aliases` table.
///
/// Returns `(registry_id, alias)` tuples. Defensively returns an empty vec
/// if the `mod_jar_aliases` table does not exist (older registry.db builds
/// predate this table) so the UI degrades gracefully.
pub fn get_all_mod_aliases(conn: &Connection) -> LauncherResult<Vec<(String, String)>> {
    let has_table: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='mod_jar_aliases'",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);
    if !has_table {
        return Ok(Vec::new());
    }

    let mut stmt = conn
        .prepare("SELECT registry_id, alias FROM mod_jar_aliases")
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?;

    let rows = stmt
        .query_map([], |row| {
            let registry_id: String = row.get(0)?;
            let alias: String = row.get(1)?;
            Ok((registry_id, alias))
        })
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?;

    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?);
    }
    Ok(out)
}

/// Resolve a mod JAR alias to its registry item ID.
///
/// Returns `Ok(Some(registry_id))` when the alias is found, `Ok(None)` when
/// it is not. Defensively returns `Ok(None)` if the `mod_jar_aliases` table
/// does not exist (older registry.db builds predate this table).
pub fn resolve_alias(conn: &Connection, alias: &str) -> LauncherResult<Option<String>> {
    let has_table: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='mod_jar_aliases'",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);
    if !has_table {
        return Ok(None);
    }

    let mut stmt = conn
        .prepare("SELECT registry_id FROM mod_jar_aliases WHERE alias = ?")
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?;

    let mut rows =
        stmt.query_map([alias], |row| row.get(0))
            .map_err(|e| LauncherError::Generic {
                code: "ERR_INVALID_QUERY".to_string(),
                message: e.to_string(),
            })?;

    if let Some(r) = rows.next() {
        Ok(Some(r.map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?))
    } else {
        Ok(None)
    }
}

/// A lightweight annotation for Modrinth projects that have a curated entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CuratedAnnotation {
    pub id: String,
    pub name: String,
    pub curator_note: Option<String>,
    pub net_score: Option<f64>,
    pub is_immune: bool,
    pub base_categories: Vec<String>,
}

/// Look up a curated annotation for a registry item by its registry id.
///
/// Queries `registry_items` by `id`, LEFT JOINs `curator_reviews` to pull the
/// real curator note from `curator_reviews.curator_note` (rather than reusing
/// the registry item description), then fetches the item's base categories
/// from `item_categories`. Returns `None` when no curated entry exists for the
/// given registry item id.
///
/// Lookup is intentionally by `registry_items.id`, not `modrinth_id`, so
/// curated entries without a Modrinth link still surface. Callers passing a
/// Modrinth project id will get `None` even if a curated entry for that
/// Modrinth project exists — the registry id is the canonical identity, and
/// Modrinth-only results must be resolved to a registry id before being
/// treated as curated.
pub fn get_curated_annotation(
    conn: &Connection,
    item_id: &str,
) -> LauncherResult<Option<CuratedAnnotation>> {
    let mut stmt = conn
        .prepare(
            "SELECT ri.id, ri.name, ri.net_score, ri.is_immune, cr.curator_note \
             FROM registry_items ri \
             LEFT JOIN curator_reviews cr ON cr.item_id = ri.id \
             WHERE ri.id = ?1",
        )
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?;

    let mut rows = stmt
        .query_map([item_id], |row| {
            let id: String = row.get(0)?;
            let name: String = row.get(1)?;
            let net_score: i64 = row.get(2)?;
            let is_immune: bool = row.get(3)?;
            let curator_note: Option<String> = row.get(4)?;
            Ok((id, name, net_score, is_immune, curator_note))
        })
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?;

    let (id, name, net_score, is_immune, curator_note) = match rows.next() {
        Some(Ok(r)) => r,
        Some(Err(e)) => {
            return Err(LauncherError::Generic {
                code: "ERR_INVALID_QUERY".to_string(),
                message: e.to_string(),
            })
        }
        None => return Ok(None),
    };

    // Fetch base categories for this curated item.
    let mut cat_stmt = conn
        .prepare("SELECT category_id FROM item_categories WHERE item_id = ?1")
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?;
    let cat_rows = cat_stmt
        .query_map([&id], |row| row.get::<_, String>(0))
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?;

    let mut base_categories: Vec<String> = Vec::new();
    for r in cat_rows {
        base_categories.push(r.map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?);
    }

    Ok(Some(CuratedAnnotation {
        id,
        name,
        curator_note,
        net_score: Some(net_score as f64),
        is_immune,
        base_categories,
    }))
}

/// True when `table` exists and carries `column`.
///
/// Used to keep queries forward-compatible with registries signed by an older
/// compiler: a missing column is a normal, expected state, not an error.
fn table_has_column(conn: &Connection, table: &str, column: &str) -> bool {
    let Ok(mut stmt) = conn.prepare(&format!("PRAGMA table_info({table})")) else {
        return false;
    };
    let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(1)) else {
        return false;
    };
    let names: Vec<String> = rows.flatten().collect();
    names.iter().any(|name| name == column)
}

/// Parse a `mod_a_versions_json` / `mod_b_versions_json` cell into ranges.
///
/// NULL, empty, malformed JSON and non-string members all decay to "no
/// window", i.e. unconditional. A curated fact that fails to parse should warn
/// about the pair rather than vanish — the pair is the part a curator vouched
/// for, the window is the refinement.
fn parse_version_ranges(raw: Option<String>) -> Vec<String> {
    raw.filter(|s| !s.trim().is_empty())
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .map(|ranges| {
            ranges
                .into_iter()
                .map(|r| r.trim().to_string())
                .filter(|r| !r.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// List known conflicts from the `known_conflicts` table.
///
/// Defensively returns an empty vec if the `known_conflicts` table does not
/// exist (older registry.db builds predate this table) so the UI degrades
/// gracefully. Parses `mitigated_by_json` (a JSON array string, may be
/// NULL/empty) into `Vec<String>`; parse errors yield an empty vec.
pub fn get_known_conflicts(conn: &Connection) -> LauncherResult<Vec<KnownConflict>> {
    let has_table: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='known_conflicts'",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);
    if !has_table {
        return Ok(Vec::new());
    }

    // Version windows arrived after the table did. A signed registry.db built
    // by an older compiler has the rows but not the columns, so select them
    // only when they exist; every fact then reads as unconditional, which is
    // exactly what it meant before windows were expressible.
    let has_windows = table_has_column(conn, "known_conflicts", "mod_a_versions_json");
    let sql = if has_windows {
        "SELECT mod_a_id, mod_b_id, severity, mitigated_by_json, notes, \
         mod_a_versions_json, mod_b_versions_json, version_grammar \
         FROM known_conflicts"
    } else {
        "SELECT mod_a_id, mod_b_id, severity, mitigated_by_json, notes, \
         NULL, NULL, NULL \
         FROM known_conflicts"
    };
    let mut stmt = conn.prepare(sql).map_err(|e| LauncherError::Generic {
        code: "ERR_INVALID_QUERY".to_string(),
        message: e.to_string(),
    })?;

    let rows = stmt
        .query_map([], |row| {
            let mitigated_by_json: Option<String> = row.get(3)?;
            let mitigated_by: Vec<String> = mitigated_by_json
                .filter(|s| !s.is_empty())
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            let grammar: Option<String> = row.get(7)?;
            Ok(KnownConflict {
                mod_a_id: row.get(0)?,
                mod_b_id: row.get(1)?,
                severity: row.get(2)?,
                mitigated_by,
                notes: row.get(4)?,
                mod_a_versions: parse_version_ranges(row.get(5)?),
                mod_b_versions: parse_version_ranges(row.get(6)?),
                version_grammar: match grammar.as_deref().map(str::trim) {
                    Some(g) if g.eq_ignore_ascii_case("maven") => VersionGrammar::Maven,
                    _ => VersionGrammar::Fabric,
                },
            })
        })
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?;

    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?);
    }
    Ok(out)
}

/// Parse registry-item `compatible_versions_json` and return `"compatible"`,
/// `"major_match"`, or `""` based on exact/major/absent match against the
/// given Minecraft version + loader.
///
/// Used by both [`RegistryService::batch_compat_lookup`] and the desktop
/// command layer for quick per-card compatibility badges.
pub fn compatibility_from_registry_json(
    json: &str,
    minecraft_version: &str,
    loader: &str,
) -> String {
    let Ok(entries) = serde_json::from_str::<Vec<serde_json::Value>>(json) else {
        return String::new();
    };
    let loader_matches = |entry: &serde_json::Value| {
        entry
            .get("loader")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| value.eq_ignore_ascii_case(loader))
    };
    if entries.iter().any(|entry| {
        loader_matches(entry)
            && entry.get("mc_version").and_then(serde_json::Value::as_str)
                == Some(minecraft_version)
    }) {
        return "compatible".into();
    }
    let requested_major = minecraft_version
        .split('.')
        .take(2)
        .collect::<Vec<_>>()
        .join(".");
    if entries.iter().any(|entry| {
        loader_matches(entry)
            && entry
                .get("mc_version")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|version| {
                    version.split('.').take(2).collect::<Vec<_>>().join(".") == requested_major
                })
    }) {
        "major_match".into()
    } else {
        String::new()
    }
}

/// Collect the set of installed-mod registry ids across all local instance
/// manifests under the given instances root. Used by the "For You" ranking
/// (§6.2) to suppress already-installed items and to derive the user's category
/// interests. Best-effort: malformed manifests are silently skipped.
fn collect_installed_registry_ids(
    instances_root: &std::path::Path,
) -> LauncherResult<HashSet<String>> {
    let mut ids: HashSet<String> = HashSet::new();
    let entries = match std::fs::read_dir(instances_root) {
        Ok(e) => e,
        Err(_) => return Ok(ids),
    };
    for entry in entries.flatten() {
        let manifest_path = entry.path().join("instance_manifest.json");
        if let Ok(manifest) = crate::helpers::read_manifest(&manifest_path) {
            for m in &manifest.mods {
                if let Some(rid) = &m.registry_id {
                    let rid = rid.trim();
                    if !rid.is_empty() {
                        ids.insert(rid.to_string());
                    }
                }
            }
            for m in &manifest.resourcepacks {
                if let Some(rid) = &m.registry_id {
                    let rid = rid.trim();
                    if !rid.is_empty() {
                        ids.insert(rid.to_string());
                    }
                }
            }
            for m in &manifest.shaders {
                if let Some(rid) = &m.registry_id {
                    let rid = rid.trim();
                    if !rid.is_empty() {
                        ids.insert(rid.to_string());
                    }
                }
            }
            for m in &manifest.datapacks {
                if let Some(rid) = &m.registry_id {
                    let rid = rid.trim();
                    if !rid.is_empty() {
                        ids.insert(rid.to_string());
                    }
                }
            }
            for m in &manifest.worlds {
                if let Some(rid) = &m.registry_id {
                    let rid = rid.trim();
                    if !rid.is_empty() {
                        ids.insert(rid.to_string());
                    }
                }
            }
        }
    }
    Ok(ids)
}

/// "For You" ranking query — extracted from [`RegistryService::for_you_items`]
/// so it can be unit-tested directly against a connection.
#[allow(clippy::too_many_arguments)]
fn for_you_items_query(
    conn: &Connection,
    installed: &HashSet<String>,
    curated_strategies: &[String],
    mc_version: Option<&str>,
    loader: Option<&str>,
    limit: i64,
    modrinth_categories: Option<&[String]>,
    query: Option<&str>,
) -> LauncherResult<Vec<RegistryItem>> {
    // Derive interest categories from installed items.
    let mut interest: Vec<String> = Vec::new();
    if !installed.is_empty() {
        let installed_ph = installed.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let interest_sql = format!(
            "SELECT DISTINCT category_id FROM item_categories WHERE item_id IN ({installed_ph})"
        );
        let mut stmt = conn
            .prepare(&interest_sql)
            .map_err(|e| LauncherError::Generic {
                code: "ERR_INVALID_QUERY".to_string(),
                message: e.to_string(),
            })?;
        let interest_params: Vec<Box<dyn rusqlite::ToSql>> = installed
            .iter()
            .map(|s| Box::new(s.clone()) as Box<dyn rusqlite::ToSql>)
            .collect();
        let interest_rows = stmt
            .query_map(rusqlite::params_from_iter(interest_params.iter()), |row| {
                let cat: String = row.get(0)?;
                Ok(cat)
            })
            .map_err(|e| LauncherError::Generic {
                code: "ERR_INVALID_QUERY".to_string(),
                message: e.to_string(),
            })?;
        for r in interest_rows {
            interest.push(r.map_err(|e| LauncherError::Generic {
                code: "ERR_INVALID_QUERY".to_string(),
                message: e.to_string(),
            })?);
        }
    }

    // Merge Modrinth category facets from the Browse page filter state.
    if let Some(mr_cats) = modrinth_categories {
        for cat in mr_cats {
            if !interest.contains(cat) {
                interest.push(cat.clone());
            }
        }
    }

    // No interest signal at all → degrade to net_score browse.
    if interest.is_empty() {
        let sort = SortOption::NetScore;
        let mut items = browse_items(
            conn,
            None,
            None,
            &sort,
            curated_strategies,
            mc_version,
            loader,
            query,
            limit,
        )?;
        if !installed.is_empty() {
            items.retain(|item| !installed.contains(&item.id));
        }
        for item in &mut items {
            item.recommendation_reason = Some(format!(
                "Recommended by Agora's curated score (net score {}).",
                item.net_score
            ));
            item.recommendation_overlap = Some(0);
        }
        return Ok(items);
    }

    let interest_ph = interest.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let mut where_parts: Vec<String> = Vec::new();
    let (strategy_clause, strategy_values) = curated_strategy_whitelist(curated_strategies);
    if let Some(clause) = strategy_clause {
        where_parts.push(clause);
    }
    if query.is_some_and(|q| !q.trim().is_empty()) {
        where_parts.push("ri.name LIKE ?".to_string());
    }
    if mc_version.is_some() {
        where_parts.push(
            "ri.id IN (SELECT r.id FROM registry_items r, json_each(r.compatible_versions_json) cv \
             WHERE json_extract(cv.value, '$.mc_version') = ?)"
                .to_string(),
        );
    }
    if let Some(ld) = loader {
        if ld != "all" {
            where_parts.push(
                "ri.id IN (SELECT r.id FROM registry_items r, json_each(r.compatible_versions_json) cv \
                 WHERE json_extract(cv.value, '$.loader') = ?)"
                    .to_string(),
            );
        }
    }
    if !installed.is_empty() {
        let installed_ph = installed.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        where_parts.push(format!("ri.id NOT IN ({installed_ph})"));
    }
    let where_clause = if where_parts.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", where_parts.join(" AND "))
    };

    let sql = format!(
        "SELECT {REGISTRY_ITEM_COLUMNS}, COUNT(ic.category_id) AS recommendation_overlap
         FROM registry_items ri
         LEFT JOIN item_categories ic ON ri.id = ic.item_id AND ic.category_id IN ({interest_ph})
         {where_clause}
         GROUP BY ri.id
         ORDER BY COUNT(ic.category_id) DESC, ri.net_score DESC
         LIMIT ?"
    );

    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    for cat in &interest {
        params.push(Box::new(cat.clone()));
    }
    for value in strategy_values {
        params.push(Box::new(value));
    }
    if let Some(q) = query.filter(|q| !q.trim().is_empty()) {
        params.push(Box::new(format!("%{}%", q.trim())));
    }
    if let Some(mv) = mc_version {
        params.push(Box::new(mv.to_string()));
    }
    if let Some(ld) = loader {
        if ld != "all" {
            params.push(Box::new(ld.to_string()));
        }
    }
    for id in installed {
        params.push(Box::new(id.clone()));
    }
    params.push(Box::new(limit));

    let mut stmt = conn.prepare(&sql).map_err(|e| LauncherError::Generic {
        code: "ERR_INVALID_QUERY".to_string(),
        message: e.to_string(),
    })?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |row| {
            let mut item = row_to_item(row)?;
            // Trails REGISTRY_ITEM_COLUMNS, so its index moves with that list.
            let overlap: i64 = row.get(25)?;
            item.recommendation_overlap = Some(overlap);
            item.recommendation_reason = Some(if overlap == 1 {
                "Shares 1 category with mods installed in your instances.".to_string()
            } else {
                format!("Shares {overlap} categories with mods installed in your instances.")
            });
            Ok(item)
        })
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".to_string(),
            message: e.to_string(),
        })?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::registry_connection;
    use tempfile::TempDir;

    fn all_curated_strategies() -> Vec<String> {
        CURATED_DOWNLOAD_STRATEGIES
            .iter()
            .map(|strategy| strategy.to_string())
            .collect()
    }

    fn temp_registry_db() -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("registry.db");

        // Create a minimal registry database with all tables the tests need.
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "
            CREATE TABLE registry_items (
                id TEXT PRIMARY KEY,
                name TEXT,
                content_type TEXT,
                download_strategy TEXT,
                source_identifier TEXT,
                sha256 TEXT,
                upvotes INTEGER,
                downvotes INTEGER,
                net_score INTEGER,
                velocity REAL,
                status TEXT,
                is_immune INTEGER,
                immunity_reason TEXT,
                allow_comments INTEGER,
                icon_url TEXT,
                gallery_urls_json TEXT,
                date_added TEXT,
                compatible_versions_json TEXT,
                description TEXT,
                body_markdown TEXT,
                page_url TEXT,
                license_id TEXT,
                source_updated_at TEXT,
                modrinth_id TEXT,
                download_sources_json TEXT
            );

            CREATE TABLE categories (
                id TEXT PRIMARY KEY,
                display_name TEXT,
                is_community INTEGER
            );

            CREATE TABLE item_categories (
                item_id TEXT,
                category_id TEXT
            );

            CREATE TABLE pack_mods (
                pack_id TEXT,
                mod_id TEXT,
                source TEXT,
                version TEXT,
                status TEXT,
                description TEXT
            );

            CREATE TABLE audit_log (
                id INTEGER PRIMARY KEY,
                timestamp TEXT,
                action TEXT,
                details TEXT
            );

            CREATE TABLE curator_reviews (
                item_id TEXT PRIMARY KEY,
                top_reviews_json TEXT
            );

            CREATE TABLE mod_manual_dependencies (
                item_id TEXT PRIMARY KEY,
                required_json TEXT,
                optional_json TEXT,
                incompatible_json TEXT
            );

            CREATE TABLE mod_jar_aliases (
                registry_id TEXT,
                alias TEXT
            );

            CREATE TABLE known_conflicts (
                mod_a_id TEXT,
                mod_b_id TEXT,
                severity TEXT,
                mitigated_by_json TEXT,
                notes TEXT
            );

            INSERT INTO registry_items VALUES (
                'test-mod-1', 'Test Mod 1', 'mod', 'github_release',
                'owner/repo', 'abc123', 10, 2, 8, 1.5, 'approved',
                0, NULL, 1, 'https://example.com/icon.png', NULL,
                '2024-01-01T00:00:00Z', '[{\"mc_version\":\"1.20.1\",\"loader\":\"fabric\"}]',
                'A test mod', NULL, 'https://example.com/mod1', 'MIT',
                '2024-01-01T00:00:00Z', NULL, NULL);

            INSERT INTO categories VALUES ('fabric', 'Fabric', 0);
            INSERT INTO item_categories VALUES ('test-mod-1', 'fabric');

            INSERT INTO pack_mods VALUES ('test-pack', 'test-mod-1', 'modrinth', '1.0.0', 'installed', 'A pack mod');

            INSERT INTO audit_log VALUES (1, '2024-01-01T00:00:00Z', 'triage_keep', 'Kept item');

            INSERT INTO curator_reviews VALUES ('test-mod-1', '[{\"author\":\"curator1\",\"text\":\"Great mod!\",\"issue_number\":42,\"created_at\":\"2024-01-01\"}]');

            INSERT INTO mod_manual_dependencies VALUES ('test-mod-1', '[\"dep-1\"]', '[\"opt-1\"]', '[\"incomp-1\"]');

            INSERT INTO mod_jar_aliases VALUES ('test-mod-1', 'testmod.jar');

            INSERT INTO known_conflicts VALUES ('mod-a', 'mod-b', 'hard', '[\"workaround\"]', 'Known issue');
            ",
        )
        .unwrap();

        dir
    }

    #[test]
    fn test_get_item_by_id() {
        let dir = temp_registry_db();
        let conn = registry_connection(&dir.path().join("registry.db")).unwrap();
        let item = get_item_by_id(&conn, "test-mod-1").unwrap().unwrap();
        assert_eq!(item.name, "Test Mod 1");
        assert_eq!(item.content_type, "mod");
        assert_eq!(item.net_score, 8);
    }

    #[test]
    fn test_get_item_by_id_not_found() {
        let dir = temp_registry_db();
        let conn = registry_connection(&dir.path().join("registry.db")).unwrap();
        let item = get_item_by_id(&conn, "nonexistent").unwrap();
        assert!(item.is_none());
    }

    #[test]
    fn test_list_categories() {
        let dir = temp_registry_db();
        let conn = registry_connection(&dir.path().join("registry.db")).unwrap();
        let cats = list_categories(&conn).unwrap();
        assert_eq!(cats.len(), 1);
        assert_eq!(cats[0].display_name, "Fabric");
        assert_eq!(cats[0].content_types, vec!["mod"]);
    }

    #[test]
    fn test_pack_mods_for_pack() {
        let dir = temp_registry_db();
        let conn = registry_connection(&dir.path().join("registry.db")).unwrap();
        let mods = pack_mods_for_pack(&conn, "test-pack").unwrap();
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].mod_id, "test-mod-1");
    }

    #[test]
    fn test_list_audit_log() {
        let dir = temp_registry_db();
        let conn = registry_connection(&dir.path().join("registry.db")).unwrap();
        let entries = list_audit_log(&conn, 10).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, "triage_keep");
    }

    #[test]
    fn test_list_under_review_items_empty() {
        let dir = temp_registry_db();
        let conn = registry_connection(&dir.path().join("registry.db")).unwrap();
        let items = list_under_review_items(&conn).unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn test_list_recent_resolutions() {
        let dir = temp_registry_db();
        let conn = registry_connection(&dir.path().join("registry.db")).unwrap();
        let entries = list_recent_resolutions(&conn, 10).unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn test_list_mod_reviews() {
        let dir = temp_registry_db();
        let conn = registry_connection(&dir.path().join("registry.db")).unwrap();
        let reviews = list_mod_reviews(&conn, "test-mod-1".to_string()).unwrap();
        assert_eq!(reviews.len(), 1);
        assert_eq!(reviews[0].author, Some("curator1".to_string()));
        assert_eq!(reviews[0].text, "Great mod!");
    }

    #[test]
    fn test_list_mod_reviews_empty() {
        let dir = temp_registry_db();
        let conn = registry_connection(&dir.path().join("registry.db")).unwrap();
        let reviews = list_mod_reviews(&conn, "nonexistent".to_string()).unwrap();
        assert!(reviews.is_empty());
    }

    #[test]
    fn test_get_manifest_dependencies() {
        let dir = temp_registry_db();
        let conn = registry_connection(&dir.path().join("registry.db")).unwrap();
        let deps = get_manifest_dependencies(&conn, "test-mod-1".to_string())
            .unwrap()
            .unwrap();
        assert_eq!(deps.required, vec!["dep-1"]);
        assert_eq!(deps.optional, vec!["opt-1"]);
        assert_eq!(deps.incompatible, vec!["incomp-1"]);
    }

    #[test]
    fn test_get_manifest_dependencies_none() {
        let dir = temp_registry_db();
        let conn = registry_connection(&dir.path().join("registry.db")).unwrap();
        let deps = get_manifest_dependencies(&conn, "nonexistent".to_string()).unwrap();
        assert!(deps.is_none());
    }

    #[test]
    fn test_get_all_mod_aliases() {
        let dir = temp_registry_db();
        let conn = registry_connection(&dir.path().join("registry.db")).unwrap();
        let aliases = get_all_mod_aliases(&conn).unwrap();
        assert_eq!(aliases.len(), 1);
        assert_eq!(aliases[0].0, "test-mod-1");
        assert_eq!(aliases[0].1, "testmod.jar");
    }

    #[test]
    fn test_resolve_alias() {
        let dir = temp_registry_db();
        let conn = registry_connection(&dir.path().join("registry.db")).unwrap();
        let resolved = resolve_alias(&conn, "testmod.jar").unwrap().unwrap();
        assert_eq!(resolved, "test-mod-1");
    }

    #[test]
    fn test_resolve_alias_not_found() {
        let dir = temp_registry_db();
        let conn = registry_connection(&dir.path().join("registry.db")).unwrap();
        let resolved = resolve_alias(&conn, "nonexistent.jar").unwrap();
        assert!(resolved.is_none());
    }

    #[test]
    fn test_get_known_conflicts() {
        let dir = temp_registry_db();
        let conn = registry_connection(&dir.path().join("registry.db")).unwrap();
        let conflicts = get_known_conflicts(&conn).unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].mod_a_id, "mod-a");
        assert_eq!(conflicts[0].mod_b_id, "mod-b");
        assert_eq!(conflicts[0].severity, "hard");
        assert_eq!(conflicts[0].mitigated_by, vec!["workaround".to_string()]);
        // This fixture predates version windows, exercising the fallback
        // SELECT: the row still loads and reads as unconditional.
        assert!(conflicts[0].mod_a_versions.is_empty());
        assert!(conflicts[0].mod_b_versions.is_empty());
        assert!(!conflicts[0].is_version_windowed());
    }

    /// A registry.db carrying the version-window columns, for the tests below.
    fn windowed_conflicts_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE known_conflicts (
                 mod_a_id TEXT, mod_b_id TEXT, severity TEXT,
                 mitigated_by_json TEXT, notes TEXT,
                 mod_a_versions_json TEXT, mod_b_versions_json TEXT,
                 version_grammar TEXT
             );
             INSERT INTO known_conflicts VALUES
                 ('mod-a','mod-b','hard','[]',NULL,'[\">=2.3\"]',NULL,'fabric'),
                 ('mvn-a','mvn-b','weak',NULL,NULL,'[\"[1.0,2.0)\"]',NULL,'maven'),
                 ('bad-a','bad-b','hard',NULL,NULL,'not json',NULL,NULL);",
        )
        .unwrap();
        conn
    }

    #[test]
    fn known_conflicts_parse_version_windows() {
        let conn = windowed_conflicts_conn();
        let conflicts = get_known_conflicts(&conn).unwrap();
        let a = conflicts.iter().find(|c| c.mod_a_id == "mod-a").unwrap();
        assert_eq!(a.mod_a_versions, vec![">=2.3".to_string()]);
        assert!(a.mod_b_versions.is_empty());
        assert_eq!(a.version_grammar, VersionGrammar::Fabric);
        assert!(a.is_version_windowed());

        let m = conflicts.iter().find(|c| c.mod_a_id == "mvn-a").unwrap();
        assert_eq!(m.version_grammar, VersionGrammar::Maven);
    }

    #[test]
    fn malformed_version_window_widens_rather_than_hides_the_conflict() {
        let conn = windowed_conflicts_conn();
        let conflicts = get_known_conflicts(&conn).unwrap();
        let bad = conflicts.iter().find(|c| c.mod_a_id == "bad-a").unwrap();
        assert!(bad.mod_a_versions.is_empty());
        // An unparseable window must not silently mute a curated pair.
        assert!(bad.applies_to_versions(Some("9.9"), Some("1.0")));
        assert!(bad.applies_to_versions(None, None));
    }

    #[test]
    fn version_window_admits_and_excludes_installed_versions() {
        let conn = windowed_conflicts_conn();
        let conflicts = get_known_conflicts(&conn).unwrap();
        let a = conflicts.iter().find(|c| c.mod_a_id == "mod-a").unwrap();
        assert!(a.applies_to_versions(Some("2.4"), Some("1.0")));
        assert!(!a.applies_to_versions(Some("2.2"), Some("1.0")));
        // An unwindowed side accepts anything, including an unknown version.
        assert!(a.applies_to_versions(Some("2.4"), None));

        let m = conflicts.iter().find(|c| c.mod_a_id == "mvn-a").unwrap();
        assert!(m.applies_to_versions(Some("1.5"), None));
        assert!(!m.applies_to_versions(Some("2.0"), None));
    }

    #[test]
    fn conditional_window_does_not_match_an_unreadable_version() {
        let conn = windowed_conflicts_conn();
        let conflicts = get_known_conflicts(&conn).unwrap();
        let a = conflicts.iter().find(|c| c.mod_a_id == "mod-a").unwrap();
        // The curator scoped this to >=2.3; we cannot read the installed
        // version, so we do not put a blocker in front of the user.
        assert!(!a.applies_to_versions(None, Some("1.0")));
        assert!(!a.applies_to_versions(Some("   "), Some("1.0")));
    }

    #[test]
    fn test_browse_items_basic() {
        let dir = temp_registry_db();
        let conn = registry_connection(&dir.path().join("registry.db")).unwrap();
        let sort = SortOption::NetScore;
        let items = browse_items(
            &conn,
            None,
            None,
            &sort,
            &all_curated_strategies(),
            None,
            None,
            None,
            100,
        )
        .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Test Mod 1");
    }

    #[test]
    fn test_browse_items_with_content_type() {
        let dir = temp_registry_db();
        let conn = registry_connection(&dir.path().join("registry.db")).unwrap();
        let sort = SortOption::NetScore;
        let items = browse_items(
            &conn,
            Some("mod"),
            None,
            &sort,
            &all_curated_strategies(),
            None,
            None,
            None,
            100,
        )
        .unwrap();
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn test_browse_items_with_category() {
        let dir = temp_registry_db();
        let conn = registry_connection(&dir.path().join("registry.db")).unwrap();
        let sort = SortOption::NetScore;
        let items = browse_items(
            &conn,
            None,
            Some("fabric"),
            &sort,
            &all_curated_strategies(),
            None,
            None,
            None,
            100,
        )
        .unwrap();
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn test_browse_items_with_query() {
        let dir = temp_registry_db();
        let conn = registry_connection(&dir.path().join("registry.db")).unwrap();
        let sort = SortOption::NetScore;
        let matching = browse_items(
            &conn,
            None,
            None,
            &sort,
            &all_curated_strategies(),
            None,
            None,
            Some("test mod"),
            100,
        )
        .unwrap();
        let missing = browse_items(
            &conn,
            None,
            None,
            &sort,
            &all_curated_strategies(),
            None,
            None,
            Some("does not exist"),
            100,
        )
        .unwrap();
        assert_eq!(matching.len(), 1);
        assert!(missing.is_empty());
    }

    #[test]
    fn test_browse_items_filters_by_curated_strategy_whitelist() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("registry.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "
            CREATE TABLE registry_items (
                id TEXT PRIMARY KEY, name TEXT, content_type TEXT,
                download_strategy TEXT, source_identifier TEXT, sha256 TEXT,
                upvotes INTEGER, downvotes INTEGER, net_score INTEGER,
                velocity REAL, status TEXT, is_immune INTEGER,
                immunity_reason TEXT, allow_comments INTEGER, icon_url TEXT,
                gallery_urls_json TEXT, date_added TEXT,
                compatible_versions_json TEXT, description TEXT,
                body_markdown TEXT, page_url TEXT, license_id TEXT,
                source_updated_at TEXT, modrinth_id TEXT,
                download_sources_json TEXT
            );
            CREATE TABLE item_categories (item_id TEXT, category_id TEXT);
            INSERT INTO registry_items VALUES (
                'git-mod', 'Git Mod', 'mod', 'github_release', 'owner/repo',
                'abc', 10, 2, 8, 1.5, 'approved', 0, NULL, 1, NULL, NULL,
                '2024-01-01T00:00:00Z', NULL, '', NULL, NULL, NULL, NULL, NULL, NULL);
            INSERT INTO registry_items VALUES (
                'mr-mod', 'Modrinth Mod', 'mod', 'modrinth_id', 'mr-id',
                'def', 10, 2, 8, 1.5, 'approved', 0, NULL, 1, NULL, NULL,
                '2024-01-01T00:00:00Z', NULL, '', NULL, NULL, NULL, NULL, 'mr-id', NULL);
            INSERT INTO registry_items VALUES (
                'dh-mod', 'Direct Hash Mod', 'mod', 'direct_hash',
                'https://example.com/files/dh.jar', 'deadbeef', 10, 2, 8, 1.5,
                'approved', 0, NULL, 1, NULL, NULL, '2024-01-01T00:00:00Z',
                NULL, '', NULL, NULL, NULL, NULL, NULL, NULL);
            ",
        )
        .unwrap();
        let sort = SortOption::NetScore;

        let all = browse_items(
            &conn,
            None,
            None,
            &sort,
            &all_curated_strategies(),
            None,
            None,
            None,
            100,
        )
        .unwrap();
        assert_eq!(
            all.len(),
            3,
            "all curated sources enabled shows every strategy"
        );

        let no_modrinth: Vec<String> = vec!["github_release".into(), "direct_hash".into()];
        let filtered = browse_items(
            &conn,
            None,
            None,
            &sort,
            &no_modrinth,
            None,
            None,
            None,
            100,
        )
        .unwrap();
        assert_eq!(filtered.len(), 2);
        assert!(!filtered.iter().any(|item| item.id == "mr-mod"));

        let only_direct: Vec<String> = vec!["direct_hash".into()];
        let direct = browse_items(
            &conn,
            None,
            None,
            &sort,
            &only_direct,
            None,
            None,
            None,
            100,
        )
        .unwrap();
        assert_eq!(direct.len(), 1);
        assert_eq!(direct[0].id, "dh-mod");

        let empty: Vec<String> = vec![];
        let none = browse_items(&conn, None, None, &sort, &empty, None, None, None, 100).unwrap();
        assert!(
            none.is_empty(),
            "a whitelist with nothing enabled must hide everything"
        );
    }

    #[test]
    fn browse_keeps_an_item_while_any_of_its_sources_is_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("registry.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "
            CREATE TABLE registry_items (
                id TEXT PRIMARY KEY, name TEXT, content_type TEXT,
                download_strategy TEXT, source_identifier TEXT, sha256 TEXT,
                upvotes INTEGER, downvotes INTEGER, net_score INTEGER,
                velocity REAL, status TEXT, is_immune INTEGER,
                immunity_reason TEXT, allow_comments INTEGER, icon_url TEXT,
                gallery_urls_json TEXT, date_added TEXT,
                compatible_versions_json TEXT, description TEXT,
                body_markdown TEXT, page_url TEXT, license_id TEXT,
                source_updated_at TEXT, modrinth_id TEXT,
                download_sources_json TEXT
            );
            CREATE TABLE item_categories (item_id TEXT, category_id TEXT);
            INSERT INTO registry_items VALUES (
                'dual', 'Dual Source Mod', 'mod', 'modrinth_id', 'mr-id',
                'abc', 10, 2, 8, 1.5, 'approved', 0, NULL, 1, NULL, NULL,
                '2024-01-01T00:00:00Z', NULL, '', NULL, NULL, NULL, NULL, 'mr-id',
                '[{\"strategy\":\"modrinth_id\",\"identifier\":\"mr-id\"},
                  {\"strategy\":\"github_release\",\"identifier\":\"owner/repo\"}]');
            ",
        )
        .unwrap();
        let sort = SortOption::NetScore;

        // Preferred source off, fallback still on: the item is installable, so
        // it must stay browsable.
        let github_only: Vec<String> = vec!["github_release".into()];
        let visible = browse_items(
            &conn,
            None,
            None,
            &sort,
            &github_only,
            None,
            None,
            None,
            100,
        )
        .unwrap();
        assert_eq!(visible.len(), 1, "fallback source keeps the item visible");

        // Every source the item declares is off: nothing can serve it.
        let unrelated: Vec<String> = vec!["direct_hash".into()];
        let hidden =
            browse_items(&conn, None, None, &sort, &unrelated, None, None, None, 100).unwrap();
        assert!(hidden.is_empty(), "no enabled source must hide the item");
    }

    fn item_with_sources(
        strategy: &str,
        identifier: &str,
        modrinth_id: Option<&str>,
        sources_json: Option<&str>,
    ) -> RegistryItem {
        RegistryItem {
            id: "test".into(),
            name: "Test".into(),
            content_type: "mod".into(),
            download_strategy: strategy.into(),
            source_identifier: identifier.into(),
            sha256: "a".repeat(64),
            upvotes: 0,
            downvotes: 0,
            net_score: 0,
            velocity: 0.0,
            status: "active".into(),
            is_immune: false,
            immunity_reason: None,
            allow_comments: true,
            icon_url: None,
            gallery_urls_json: None,
            date_added: None,
            compatible_versions_json: None,
            description: None,
            body_markdown: None,
            page_url: None,
            license_id: None,
            source_updated_at: None,
            modrinth_id: modrinth_id.map(str::to_string),
            download_sources_json: sources_json.map(str::to_string),
            recommendation_reason: None,
            recommendation_overlap: None,
        }
    }

    #[test]
    fn a_malformed_source_list_does_not_break_browse() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("registry.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "
            CREATE TABLE registry_items (
                id TEXT PRIMARY KEY, name TEXT, content_type TEXT,
                download_strategy TEXT, source_identifier TEXT, sha256 TEXT,
                upvotes INTEGER, downvotes INTEGER, net_score INTEGER,
                velocity REAL, status TEXT, is_immune INTEGER,
                immunity_reason TEXT, allow_comments INTEGER, icon_url TEXT,
                gallery_urls_json TEXT, date_added TEXT,
                compatible_versions_json TEXT, description TEXT,
                body_markdown TEXT, page_url TEXT, license_id TEXT,
                source_updated_at TEXT, modrinth_id TEXT,
                download_sources_json TEXT
            );
            CREATE TABLE item_categories (item_id TEXT, category_id TEXT);
            INSERT INTO registry_items VALUES (
                'broken', 'Broken Sources', 'mod', 'github_release', 'owner/repo',
                'abc', 10, 2, 8, 1.5, 'approved', 0, NULL, 1, NULL, NULL,
                '2024-01-01T00:00:00Z', NULL, '', NULL, NULL, NULL, NULL, NULL,
                'not json at all');
            ",
        )
        .unwrap();

        // The row falls back to its legacy columns rather than taking the whole
        // query down with it.
        let sort = SortOption::NetScore;
        let items = browse_items(
            &conn,
            None,
            None,
            &sort,
            &all_curated_strategies(),
            None,
            None,
            None,
            100,
        )
        .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].download_sources()[0].strategy, "github_release");
    }

    #[test]
    fn download_sources_preserve_curator_order() {
        let item = item_with_sources(
            "modrinth_id",
            "mr-id",
            Some("mr-id"),
            Some(
                r#"[{"strategy":"modrinth_id","identifier":"mr-id"},
                    {"strategy":"github_release","identifier":"owner/repo"}]"#,
            ),
        );
        let sources = item.download_sources();
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].strategy, "modrinth_id");
        assert_eq!(sources[1].strategy, "github_release");
        assert_eq!(sources[1].identifier, "owner/repo");
        assert!(item.has_download_source("github_release"));
        assert!(!item.has_download_source("direct_hash"));
    }

    #[test]
    fn download_sources_reconstruct_the_legacy_modrinth_fallback() {
        // A pre-schema-8 row: no source list, but a modrinth_id the resolver
        // has always fallen back to.
        let item = item_with_sources("github_release", "owner/repo", Some("mr-id"), None);
        let sources = item.download_sources();
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].strategy, "github_release");
        assert_eq!(sources[0].identifier, "owner/repo");
        assert_eq!(sources[1].strategy, "modrinth_id");
        assert_eq!(sources[1].identifier, "mr-id");
    }

    #[test]
    fn download_sources_never_empty_and_never_duplicate_modrinth() {
        let single = item_with_sources("modrinth_id", "mr-id", Some("mr-id"), None);
        assert_eq!(single.download_sources().len(), 1);

        let malformed = item_with_sources("github_release", "owner/repo", None, Some("not json"));
        let sources = malformed.download_sources();
        assert_eq!(sources.len(), 1, "unparseable JSON falls back to the row");
        assert_eq!(sources[0].strategy, "github_release");
    }

    #[test]
    fn test_sort_option_default() {
        let sort = SortOption::default();
        assert!(matches!(sort, SortOption::NetScore));
    }

    #[test]
    fn test_get_curated_annotation_found() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("registry.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "
            CREATE TABLE registry_items (
                id TEXT PRIMARY KEY, name TEXT, content_type TEXT,
                download_strategy TEXT, source_identifier TEXT, sha256 TEXT,
                upvotes INTEGER, downvotes INTEGER, net_score INTEGER,
                velocity REAL, status TEXT, is_immune INTEGER,
                immunity_reason TEXT, allow_comments INTEGER, icon_url TEXT,
                gallery_urls_json TEXT, date_added TEXT,
                compatible_versions_json TEXT, description TEXT,
                body_markdown TEXT, page_url TEXT, license_id TEXT,
                source_updated_at TEXT, modrinth_id TEXT,
                download_sources_json TEXT
            );
            CREATE TABLE item_categories (
                item_id TEXT, category_id TEXT
            );
            CREATE TABLE curator_reviews (
                item_id TEXT PRIMARY KEY,
                curator_note TEXT,
                top_reviews_json TEXT
            );
            INSERT INTO registry_items VALUES (
                'curated-mod', 'Curated Mod', 'mod', 'modrinth_id',
                'mr-id-123', 'abc', 100, 5, 85, 2.0, 'approved',
                1, NULL, 1, NULL, NULL, '2024-06-01T00:00:00Z', NULL,
                'A curated mod description', NULL, NULL, NULL, NULL, 'mr-id-123', NULL);
            INSERT INTO item_categories VALUES ('curated-mod', 'fabric');
            INSERT INTO item_categories VALUES ('curated-mod', 'adventure');
            INSERT INTO curator_reviews VALUES ('curated-mod', 'A real curator note.', '[]');
            ",
        )
        .unwrap();

        // Lookup must use the registry item id, not the modrinth id. The
        // prior implementation queried by modrinth_id, which silently dropped
        // curated entries that lacked one.
        let annotation = get_curated_annotation(&conn, "curated-mod")
            .unwrap()
            .expect("Expected some annotation");
        assert_eq!(annotation.id, "curated-mod");
        assert_eq!(annotation.name, "Curated Mod");
        assert!(annotation.is_immune);
        assert_eq!(annotation.net_score, Some(85.0));
        // The curator note now comes from curator_reviews.curator_note, not
        // from the registry item description.
        assert_eq!(
            annotation.curator_note,
            Some("A real curator note.".to_string())
        );
        assert_eq!(annotation.base_categories.len(), 2);
        assert!(annotation.base_categories.contains(&"fabric".to_string()));
        assert!(annotation
            .base_categories
            .contains(&"adventure".to_string()));
    }

    #[test]
    fn test_get_curated_annotation_found_without_modrinth_id() {
        // Regression: registry entries with no modrinth_id must still surface
        // curated features when looked up by their registry id. The prior
        // implementation queried by modrinth_id and so vanished these entries
        // from the curated UI.
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("registry.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "
            CREATE TABLE registry_items (
                id TEXT PRIMARY KEY, name TEXT, content_type TEXT,
                download_strategy TEXT, source_identifier TEXT, sha256 TEXT,
                upvotes INTEGER, downvotes INTEGER, net_score INTEGER,
                velocity REAL, status TEXT, is_immune INTEGER,
                immunity_reason TEXT, allow_comments INTEGER, icon_url TEXT,
                gallery_urls_json TEXT, date_added TEXT,
                compatible_versions_json TEXT, description TEXT,
                body_markdown TEXT, page_url TEXT, license_id TEXT,
                source_updated_at TEXT, modrinth_id TEXT,
                download_sources_json TEXT
            );
            CREATE TABLE item_categories (
                item_id TEXT, category_id TEXT
            );
            CREATE TABLE curator_reviews (
                item_id TEXT PRIMARY KEY,
                curator_note TEXT,
                top_reviews_json TEXT
            );
            INSERT INTO registry_items VALUES (
                'github-mod', 'GitHub Mod', 'mod', 'github_release',
                'owner/repo', 'abc', 40, 3, 37, 1.0, 'approved',
                0, NULL, 1, NULL, NULL, '2024-06-01T00:00:00Z', NULL,
                'A github mod description', NULL, NULL, NULL, NULL, NULL, NULL);
            INSERT INTO item_categories VALUES ('github-mod', 'performance');
            INSERT INTO curator_reviews VALUES ('github-mod', 'Note from curators.', '[]');
            ",
        )
        .unwrap();

        let annotation = get_curated_annotation(&conn, "github-mod")
            .unwrap()
            .expect("Expected some annotation");
        assert_eq!(annotation.id, "github-mod");
        assert_eq!(annotation.name, "GitHub Mod");
        assert!(!annotation.is_immune);
        assert_eq!(annotation.net_score, Some(37.0));
        assert_eq!(
            annotation.curator_note,
            Some("Note from curators.".to_string())
        );
        assert_eq!(annotation.base_categories, vec!["performance".to_string()]);
    }

    #[test]
    fn test_get_curated_annotation_ignores_modrinth_id_lookup() {
        // Contract: a Modrinth project id passed as the lookup key must NOT
        // resolve to a curated entry even when one exists for that project;
        // the registry id is the canonical identity and Modrinth-only results
        // must be resolved to a registry id before receiving curated features.
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("registry.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "
            CREATE TABLE registry_items (
                id TEXT PRIMARY KEY, name TEXT, content_type TEXT,
                download_strategy TEXT, source_identifier TEXT, sha256 TEXT,
                upvotes INTEGER, downvotes INTEGER, net_score INTEGER,
                velocity REAL, status TEXT, is_immune INTEGER,
                immunity_reason TEXT, allow_comments INTEGER, icon_url TEXT,
                gallery_urls_json TEXT, date_added TEXT,
                compatible_versions_json TEXT, description TEXT,
                body_markdown TEXT, page_url TEXT, license_id TEXT,
                source_updated_at TEXT, modrinth_id TEXT,
                download_sources_json TEXT
            );
            CREATE TABLE item_categories (
                item_id TEXT, category_id TEXT
            );
            CREATE TABLE curator_reviews (
                item_id TEXT PRIMARY KEY,
                curator_note TEXT,
                top_reviews_json TEXT
            );
            INSERT INTO registry_items VALUES (
                'curated-mod', 'Curated Mod', 'mod', 'modrinth_id',
                'mr-id-123', 'abc', 100, 5, 85, 2.0, 'approved',
                1, NULL, 1, NULL, NULL, '2024-06-01T00:00:00Z', NULL,
                'A curated mod description', NULL, NULL, NULL, NULL, 'mr-id-123', NULL);
            INSERT INTO curator_reviews VALUES ('curated-mod', 'Note.', '[]');
            ",
        )
        .unwrap();

        let result = get_curated_annotation(&conn, "mr-id-123").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_get_curated_annotation_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("registry.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "
            CREATE TABLE registry_items (
                id TEXT PRIMARY KEY, name TEXT, content_type TEXT,
                download_strategy TEXT, source_identifier TEXT, sha256 TEXT,
                upvotes INTEGER, downvotes INTEGER, net_score INTEGER,
                velocity REAL, status TEXT, is_immune INTEGER,
                immunity_reason TEXT, allow_comments INTEGER, icon_url TEXT,
                gallery_urls_json TEXT, date_added TEXT,
                compatible_versions_json TEXT, description TEXT,
                body_markdown TEXT, page_url TEXT, license_id TEXT,
                source_updated_at TEXT, modrinth_id TEXT,
                download_sources_json TEXT
            );
            CREATE TABLE item_categories (
                item_id TEXT, category_id TEXT
            );
            CREATE TABLE curator_reviews (
                item_id TEXT PRIMARY KEY,
                curator_note TEXT,
                top_reviews_json TEXT
            );
            ",
        )
        .unwrap();

        let result = get_curated_annotation(&conn, "nonexistent-id").unwrap();
        assert!(result.is_none());
    }

    // -------------------------------------------------------------------
    // RegistryService tests
    // -------------------------------------------------------------------

    #[test]
    fn service_get_item_by_id_found() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("registry.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE registry_items (
                 id TEXT PRIMARY KEY, name TEXT, content_type TEXT,
                 download_strategy TEXT, source_identifier TEXT, sha256 TEXT,
                 upvotes INTEGER, downvotes INTEGER, net_score INTEGER,
                 velocity REAL, status TEXT, is_immune INTEGER,
                 immunity_reason TEXT, allow_comments INTEGER, icon_url TEXT,
                 gallery_urls_json TEXT, date_added TEXT,
                 compatible_versions_json TEXT, description TEXT,
                 body_markdown TEXT, page_url TEXT, license_id TEXT,
                 source_updated_at TEXT, modrinth_id TEXT,
                 download_sources_json TEXT
             );
             INSERT INTO registry_items VALUES (
                 'svc-test-1', 'Svc Test Mod', 'mod', 'github_release',
                 'owner/repo', 'abc', 10, 2, 8, 1.5, 'approved',
                 0, NULL, 1, NULL, NULL,
                 '2024-01-01T00:00:00Z', NULL,
                 'A service test mod', NULL, NULL, NULL, NULL, NULL, NULL);",
        )
        .unwrap();
        drop(conn);

        // Move registry.db to the Ctx-managed path.
        let ctx = Ctx::for_testing(dir.path().to_owned());
        // Actually the ctx already has db_path = dir.path().join("registry.db")
        // since we created it there.

        let svc = RegistryService::new(ctx);
        let item = svc.get_item_by_id("svc-test-1").unwrap().unwrap();
        assert_eq!(item.name, "Svc Test Mod");
        assert_eq!(item.content_type, "mod");
    }

    #[test]
    fn service_get_item_by_id_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("registry.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE registry_items (
                 id TEXT PRIMARY KEY, name TEXT, content_type TEXT,
                 download_strategy TEXT, source_identifier TEXT, sha256 TEXT,
                 upvotes INTEGER, downvotes INTEGER, net_score INTEGER,
                 velocity REAL, status TEXT, is_immune INTEGER,
                 immunity_reason TEXT, allow_comments INTEGER, icon_url TEXT,
                 gallery_urls_json TEXT, date_added TEXT,
                 compatible_versions_json TEXT, description TEXT,
                 body_markdown TEXT, page_url TEXT, license_id TEXT,
                 source_updated_at TEXT, modrinth_id TEXT,
                 download_sources_json TEXT
             );",
        )
        .unwrap();
        drop(conn);

        let ctx = Ctx::for_testing(dir.path().to_owned());
        let svc = RegistryService::new(ctx);
        let item = svc.get_item_by_id("nonexistent").unwrap();
        assert!(item.is_none());
    }

    #[test]
    fn service_get_item_by_id_missing_registry() {
        let dir = tempfile::tempdir().unwrap();
        // No registry.db created at all.
        let ctx = Ctx::for_testing(dir.path().to_owned());
        let svc = RegistryService::new(ctx);
        let err = svc.get_item_by_id("anything").unwrap_err();
        assert_eq!(err.code(), "ERR_REGISTRY_MISSING");
    }

    // -------------------------------------------------------------------
    // compatibility_from_registry_json tests
    // -------------------------------------------------------------------

    const COMPAT: &str = r#"[
        {"mc_version":"1.21.1","loader":"fabric"},
        {"mc_version":"1.20.4","loader":"neoforge"}
    ]"#;

    #[test]
    fn compat_exact_match() {
        assert_eq!(
            compatibility_from_registry_json(COMPAT, "1.21.1", "fabric"),
            "compatible"
        );
    }

    #[test]
    fn compat_major_match() {
        assert_eq!(
            compatibility_from_registry_json(COMPAT, "1.21.4", "fabric"),
            "major_match"
        );
    }

    #[test]
    fn compat_requires_loader_match() {
        assert_eq!(
            compatibility_from_registry_json(COMPAT, "1.21.1", "neoforge"),
            ""
        );
    }

    #[test]
    fn compat_malformed_metadata_is_empty() {
        assert_eq!(
            compatibility_from_registry_json("not-json", "1.21.1", "fabric"),
            ""
        );
    }

    // -------------------------------------------------------------------
    // for_you_items_query tests
    // -------------------------------------------------------------------

    #[test]
    fn for_you_items_empty_interests_degrades_to_net_score() {
        let dir = temp_registry_db();
        let conn = registry_connection(&dir.path().join("registry.db")).unwrap();
        let installed = HashSet::new();
        let items = for_you_items_query(
            &conn,
            &installed,
            &all_curated_strategies(),
            None,
            None,
            100,
            None,
            None,
        )
        .unwrap();
        // test-mod-1 is the only item and net_score 8
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Test Mod 1");
        assert_eq!(items[0].recommendation_overlap, Some(0));
        assert!(items[0]
            .recommendation_reason
            .as_ref()
            .unwrap()
            .contains("net score"));
    }

    #[test]
    fn for_you_items_suppresses_installed() {
        let dir = temp_registry_db();
        let conn = registry_connection(&dir.path().join("registry.db")).unwrap();
        let mut installed = HashSet::new();
        installed.insert("test-mod-1".to_string());
        let items = for_you_items_query(
            &conn,
            &installed,
            &all_curated_strategies(),
            None,
            None,
            100,
            None,
            None,
        )
        .unwrap();
        // test-mod-1 should be filtered out, leaving none
        assert!(items.is_empty());
    }

    #[test]
    fn for_you_items_with_modrinth_categories_merged() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("registry.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE registry_items (
                id TEXT PRIMARY KEY, name TEXT, content_type TEXT,
                download_strategy TEXT, source_identifier TEXT, sha256 TEXT,
                upvotes INTEGER, downvotes INTEGER, net_score INTEGER,
                velocity REAL, status TEXT, is_immune INTEGER,
                immunity_reason TEXT, allow_comments INTEGER, icon_url TEXT,
                gallery_urls_json TEXT, date_added TEXT,
                compatible_versions_json TEXT, description TEXT,
                body_markdown TEXT, page_url TEXT, license_id TEXT,
                source_updated_at TEXT, modrinth_id TEXT,
                download_sources_json TEXT
            );
            CREATE TABLE item_categories (
                item_id TEXT, category_id TEXT
            );
            INSERT INTO registry_items VALUES (
                'rec-mod', 'Rec Mod', 'mod', 'github_release',
                'owner/repo', 'abc', 10, 2, 8, 1.5, 'approved',
                0, NULL, 1, NULL, NULL, '2024-01-01T00:00:00Z', NULL,
                'A recommended mod', NULL, NULL, NULL, NULL, NULL, NULL);
            INSERT INTO item_categories VALUES ('rec-mod', 'adventure');
            ",
        )
        .unwrap();
        drop(conn);
        let conn = registry_connection(&db_path).unwrap();
        let installed = HashSet::new();
        // No installed items → empty interest → degrad to browse.
        // Provide Modrinth categories that are NOT in the db; they should
        // still be merged (though they won't affect the result without items).
        let modrinth_cats = vec!["adventure".to_string()];
        let items = for_you_items_query(
            &conn,
            &installed,
            &all_curated_strategies(),
            None,
            None,
            100,
            Some(&modrinth_cats),
            None,
        )
        .unwrap();
        // With no installed items, interest is empty (no installed items
        // to derive from), so the Modrinth cats are the only input → still
        // interest.is_empty() because we only merge with modrinth_categories
        // when the user has no installed mods. The fallback net_score path
        // is used, which returns rec-mod.
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Rec Mod");
    }

    #[test]
    fn collect_installed_registry_ids_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let ids = collect_installed_registry_ids(dir.path()).unwrap();
        assert!(ids.is_empty());
    }

    #[test]
    fn collect_installed_registry_ids_from_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let inst_dir = dir.path().join("my-instance");
        std::fs::create_dir_all(&inst_dir).unwrap();
        let manifest = r#"{
            "instance_id": "my-instance",
            "name": "My Instance",
            "minecraft_version": "1.20.1",
            "loader": "fabric",
            "loader_version": "0.15.0",
            "mods": [
                {"filename": "a.jar", "source": "modrinth", "sha256": "a", "installed_at": "now", "registry_id": "mod-a"},
                {"filename": "b.jar", "source": "modrinth", "sha256": "b", "installed_at": "now", "registry_id": "mod-b"}
            ],
            "resourcepacks": [],
            "shaders": [],
            "datapacks": [],
            "worlds": [],
            "user_preferences": {}
        }"#;
        std::fs::write(inst_dir.join("instance_manifest.json"), manifest).unwrap();
        let ids = collect_installed_registry_ids(dir.path()).unwrap();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains("mod-a"));
        assert!(ids.contains("mod-b"));
    }

    #[test]
    fn collect_installed_registry_ids_skips_missing_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let inst_dir = dir.path().join("empty-instance");
        std::fs::create_dir_all(&inst_dir).unwrap();
        // No manifest file — should not panic
        let ids = collect_installed_registry_ids(dir.path()).unwrap();
        assert!(ids.is_empty());
    }

    #[test]
    fn collect_installed_registry_ids_skips_malformed_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let inst_dir = dir.path().join("bad-instance");
        std::fs::create_dir_all(&inst_dir).unwrap();
        std::fs::write(inst_dir.join("instance_manifest.json"), "not-json").unwrap();
        let ids = collect_installed_registry_ids(dir.path()).unwrap();
        assert!(ids.is_empty());
    }

    #[test]
    fn for_you_items_query_with_interest_ranking() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("registry.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE registry_items (
                id TEXT PRIMARY KEY, name TEXT, content_type TEXT,
                download_strategy TEXT, source_identifier TEXT, sha256 TEXT,
                upvotes INTEGER, downvotes INTEGER, net_score INTEGER,
                velocity REAL, status TEXT, is_immune INTEGER,
                immunity_reason TEXT, allow_comments INTEGER, icon_url TEXT,
                gallery_urls_json TEXT, date_added TEXT,
                compatible_versions_json TEXT, description TEXT,
                body_markdown TEXT, page_url TEXT, license_id TEXT,
                source_updated_at TEXT, modrinth_id TEXT,
                download_sources_json TEXT
            );
            CREATE TABLE item_categories (
                item_id TEXT, category_id TEXT
            );
            -- Simulate installed items that expose 'fabric' and 'adventure' interests.
            INSERT INTO item_categories VALUES ('installed-mod', 'fabric');
            INSERT INTO item_categories VALUES ('installed-mod', 'adventure');
            -- Item with 'fabric' category (will be matched by interest from installed)
            INSERT INTO registry_items VALUES (
                'rec-1', 'Rec One', 'mod', 'github_release',
                'a/a', 'a', 10, 2, 8, 1.5, 'approved',
                0, NULL, 1, NULL, NULL, '2024-01-01T00:00:00Z', NULL,
                'A', NULL, NULL, NULL, NULL, NULL, NULL);
            -- Item with 'fabric' AND 'adventure' categories (higher overlap)
            INSERT INTO registry_items VALUES (
                'rec-2', 'Rec Two', 'mod', 'github_release',
                'b/b', 'b', 10, 2, 5, 1.0, 'approved',
                0, NULL, 1, NULL, NULL, '2024-01-01T00:00:00Z', NULL,
                'B', NULL, NULL, NULL, NULL, NULL, NULL);
            INSERT INTO item_categories VALUES ('rec-1', 'fabric');
            INSERT INTO item_categories VALUES ('rec-2', 'fabric');
            INSERT INTO item_categories VALUES ('rec-2', 'adventure');
            ",
        )
        .unwrap();
        drop(conn);
        let conn = registry_connection(&db_path).unwrap();

        let mut installed = HashSet::new();
        installed.insert("installed-mod".to_string());

        let items = for_you_items_query(
            &conn,
            &installed,
            &all_curated_strategies(),
            None,
            None,
            100,
            None,
            None,
        )
        .unwrap();

        // rec-2 has overlap 2 (fabric + adventure), rec-1 has overlap 1 (fabric)
        assert_eq!(items.len(), 2, "should return both uninstalled items");
        // rec-2 should be first (higher overlap)
        assert_eq!(items[0].name, "Rec Two");
        assert_eq!(items[0].recommendation_overlap, Some(2));
        assert!(items[0]
            .recommendation_reason
            .as_ref()
            .unwrap()
            .contains("2 categories"));
        // rec-1 should be second
        assert_eq!(items[1].name, "Rec One");
        assert_eq!(items[1].recommendation_overlap, Some(1));
        assert!(items[1]
            .recommendation_reason
            .as_ref()
            .unwrap()
            .contains("1 category"));
    }

    // -------------------------------------------------------------------
    // RegistryService::known_conflicts / is_under_review tests
    // -------------------------------------------------------------------

    #[test]
    fn service_known_conflicts_found() {
        let dir = temp_registry_db();
        let ctx = Ctx::for_testing(dir.path().to_owned());
        let svc = RegistryService::new(ctx);

        let conflicts = svc.known_conflicts().unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].mod_a_id, "mod-a");
        assert_eq!(conflicts[0].mod_b_id, "mod-b");
        assert_eq!(conflicts[0].severity, "hard");
        assert_eq!(conflicts[0].mitigated_by, vec!["workaround".to_string()]);
    }

    #[test]
    fn service_known_conflicts_missing_registry() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx::for_testing(dir.path().to_owned());
        let svc = RegistryService::new(ctx);
        let err = svc.known_conflicts().unwrap_err();
        assert_eq!(err.code(), "ERR_REGISTRY_MISSING");
    }

    #[test]
    fn service_is_under_review_when_present() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("registry.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE registry_items (
                 id TEXT PRIMARY KEY, name TEXT, content_type TEXT,
                 download_strategy TEXT, source_identifier TEXT, sha256 TEXT,
                 upvotes INTEGER, downvotes INTEGER, net_score INTEGER,
                 velocity REAL, status TEXT, is_immune INTEGER,
                 immunity_reason TEXT, allow_comments INTEGER, icon_url TEXT,
                 gallery_urls_json TEXT, date_added TEXT,
                 compatible_versions_json TEXT, description TEXT,
                 body_markdown TEXT, page_url TEXT, license_id TEXT,
                 source_updated_at TEXT, modrinth_id TEXT,
                 download_sources_json TEXT
             );
             INSERT INTO registry_items VALUES (
                 'review-mod', 'Review Mod', 'mod', 'github_release',
                 'r/r', 'abc', 0, 0, 0, 0.0, 'under_review',
                 0, NULL, 1, NULL, NULL, '2024-06-01T00:00:00Z', NULL,
                 NULL, NULL, NULL, NULL, NULL, NULL, NULL);",
        )
        .unwrap();
        drop(conn);

        let ctx = Ctx::for_testing(dir.path().to_owned());
        let svc = RegistryService::new(ctx);

        assert!(svc.is_under_review("review-mod").unwrap());
        assert!(!svc.is_under_review("unknown-mod").unwrap());
    }

    #[test]
    fn service_is_under_review_missing_registry_returns_false() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx::for_testing(dir.path().to_owned());
        let svc = RegistryService::new(ctx);
        // Graceful degradation: missing registry → false
        let result = svc.is_under_review("anything").unwrap();
        assert!(!result);
    }
}
