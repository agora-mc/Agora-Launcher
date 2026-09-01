//! Per-version changelogs baked into the signed `registry.db` at compile time.
//!
//! The compiler hydrates `item_version_changelogs` from Modrinth
//! (`GET /v2/project/{id}/version` → `changelog`) and GitHub
//! (`GET /repos/{owner}/{repo}/releases` → `body`) so the desktop can show
//! "what changed" offline without a runtime API call. Each row is keyed by
//! `(item_id, version)` where `version` is the display version string the
//! resolver already uses (Modrinth `version_number` or GitHub `tag_name`).
//! That is the same string `InstalledMod.version` and `ModVersionCandidate.version`
//! use, so a lookup by installed version just works.
//!
//! The table is generic by design: it stores a source label (`modrinth_id` or
//! `github_release`) but the query layer is source-agnostic. Items with no
//! upstream (e.g. `direct_hash`, `curated_pack`, `technic_pack`) simply have
//! no rows — callers get an empty vec and can show a graceful fallback. The
//! same shape would also fit a future manual `changelog` manifest field without
//! a schema change.
//!
//! Size is bounded at compile time: the hydrator truncates each entry to
//! ~8 KiB and keeps only the most recent 30 non-empty changelogs per item.
//! The read path below is defensive against older databases that predate the
//! table (schema < 9) — it returns `Ok(empty)` rather than erroring, so an
//! older cached `registry.db` degrades to "no changelog available" instead of
//! failing to browse.

use crate::error::{LauncherError, LauncherResult};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

/// One changelog row as stored in `item_version_changelogs`.
///
/// `version` is the display version string (Modrinth `version_number` or GitHub
/// `tag_name`), not the opaque Modrinth `version_id`. Markdown — must be
/// rendered without `dangerouslySetInnerHTML` on the frontend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionChangelog {
    pub item_id: String,
    pub version: String,
    pub changelog: String,
    pub published_at: Option<String>,
    pub source: String,
}

fn has_table(conn: &Connection) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name='item_version_changelogs'",
        [],
        |_| Ok(true),
    )
    .unwrap_or(false)
}

/// List all known changelogs for a single item, newest first.
///
/// Ordering is by `published_at` descending (ISO-8601 strings compare
/// correctly). Rows with a NULL timestamp sort last but still appear — they
/// are ordered by `version` as a deterministic tie-break so the result is
/// stable across SQLite versions.
pub fn get_changelogs_for_item(
    conn: &Connection,
    item_id: &str,
) -> LauncherResult<Vec<VersionChangelog>> {
    if !has_table(conn) {
        return Ok(Vec::new());
    }
    let mut stmt = conn
        .prepare(
            "SELECT item_id, version, changelog, published_at, source \
             FROM item_version_changelogs \
             WHERE item_id = ?1 \
             ORDER BY COALESCE(published_at, '') DESC, version DESC",
        )
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".into(),
            message: e.to_string(),
        })?;
    let rows = stmt
        .query_map([item_id], |row| {
            Ok(VersionChangelog {
                item_id: row.get(0)?,
                version: row.get(1)?,
                changelog: row.get(2)?,
                published_at: row.get(3)?,
                source: row.get(4)?,
            })
        })
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".into(),
            message: e.to_string(),
        })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".into(),
            message: e.to_string(),
        })?);
    }
    Ok(out)
}

/// Fetch the changelog for a single `(item_id, version)` pair, if present.
///
/// Version comparison is exact string equality — callers should pass the
/// verbatim `InstalledMod.version` or `ModVersionCandidate.version`.
pub fn get_changelog_for_version(
    conn: &Connection,
    item_id: &str,
    version: &str,
) -> LauncherResult<Option<VersionChangelog>> {
    if !has_table(conn) {
        return Ok(None);
    }
    let mut stmt = conn
        .prepare(
            "SELECT item_id, version, changelog, published_at, source \
             FROM item_version_changelogs \
             WHERE item_id = ?1 AND version = ?2 \
             LIMIT 1",
        )
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".into(),
            message: e.to_string(),
        })?;
    let mut rows = stmt
        .query_map([item_id, version], |row| {
            Ok(VersionChangelog {
                item_id: row.get(0)?,
                version: row.get(1)?,
                changelog: row.get(2)?,
                published_at: row.get(3)?,
                source: row.get(4)?,
            })
        })
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".into(),
            message: e.to_string(),
        })?;
    if let Some(r) = rows.next() {
        Ok(Some(r.map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".into(),
            message: e.to_string(),
        })?))
    } else {
        Ok(None)
    }
}

/// What changed between `from_version` (installed) and `to_version` (candidate).
///
/// Returns the changelogs for all versions **newer than** `from_version`
/// up to and including `to_version`, newest first — exactly what an "update
/// available" panel wants to render. Ordering follows `published_at`
/// ascending internally to determine the interval, then the slice is reversed
/// for display.
///
/// Cases:
/// - `to_version` not found in the table → `Ok(vec![])` (candidate is too
///   new, unreleased, or beyond the 30-row cap — no offline data).
/// - `from_version` not found (e.g. `"unknown"`, very old install, or beyond
///   cap) → returns all rows up to `to_version` inclusive, newest first,
///   so the user still sees *something* rather than nothing.
/// - `from_version == to_version` → `Ok(vec![])`.
/// - `to_version` older than `from_version` (downgrade) → `Ok(vec![])`.
///
/// The function is read-only and never hits the network; it is the offline
/// answer to "should I update?".
pub fn get_changelogs_between(
    conn: &Connection,
    item_id: &str,
    from_version: &str,
    to_version: &str,
) -> LauncherResult<Vec<VersionChangelog>> {
    if from_version == to_version {
        return Ok(Vec::new());
    }
    if !has_table(conn) {
        return Ok(Vec::new());
    }
    // Load all rows for the item in chronological order (oldest first) so
    // we can slice by interval. The `get_changelogs_for_item` ordering is
    // DESC, so we re-query ASC here for the interval math.
    let mut stmt = conn
        .prepare(
            "SELECT item_id, version, changelog, published_at, source \
             FROM item_version_changelogs \
             WHERE item_id = ?1 \
             ORDER BY COALESCE(published_at, '') ASC, version ASC",
        )
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".into(),
            message: e.to_string(),
        })?;
    let rows = stmt
        .query_map([item_id], |row| {
            Ok(VersionChangelog {
                item_id: row.get(0)?,
                version: row.get(1)?,
                changelog: row.get(2)?,
                published_at: row.get(3)?,
                source: row.get(4)?,
            })
        })
        .map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".into(),
            message: e.to_string(),
        })?;
    let mut all_asc: Vec<VersionChangelog> = Vec::new();
    for r in rows {
        all_asc.push(r.map_err(|e| LauncherError::Generic {
            code: "ERR_INVALID_QUERY".into(),
            message: e.to_string(),
        })?);
    }
    if all_asc.is_empty() {
        return Ok(Vec::new());
    }
    let to_idx = all_asc.iter().position(|c| c.version == to_version);
    let Some(to_pos) = to_idx else {
        return Ok(Vec::new());
    };
    let from_pos = all_asc.iter().position(|c| c.version == from_version);
    let start = match from_pos {
        Some(idx) if idx < to_pos => idx + 1,
        Some(idx) if idx >= to_pos => return Ok(Vec::new()), // downgrade or same
        None => 0, // from unknown → show everything up to to
        _ => 0,
    };
    // Slice is inclusive of `to_pos`, exclusive of `from_pos`.
    let mut out = all_asc[start..=to_pos].to_vec();
    // Display newest first.
    out.reverse();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE item_version_changelogs (
                item_id TEXT NOT NULL,
                version TEXT NOT NULL,
                changelog TEXT NOT NULL,
                published_at TEXT,
                source TEXT NOT NULL,
                PRIMARY KEY (item_id, version)
            );
            CREATE INDEX idx_version_changelogs_item_id ON item_version_changelogs(item_id);
            ",
        )
        .unwrap();
        conn
    }

    fn insert_row(
        conn: &Connection,
        item_id: &str,
        version: &str,
        changelog: &str,
        published_at: Option<&str>,
        source: &str,
    ) {
        conn.execute(
            "INSERT INTO item_version_changelogs (item_id, version, changelog, published_at, source) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![item_id, version, changelog, published_at, source],
        )
        .unwrap();
    }

    #[test]
    fn empty_db_returns_empty() {
        let conn = Connection::open_in_memory().unwrap();
        // No table at all (old db)
        let v = get_changelogs_for_item(&conn, "sodium").unwrap();
        assert!(v.is_empty());
        let v = get_changelog_for_version(&conn, "sodium", "1.0").unwrap();
        assert!(v.is_none());
        let v = get_changelogs_between(&conn, "sodium", "1.0", "2.0").unwrap();
        assert!(v.is_empty());
    }

    #[test]
    fn for_item_orders_newest_first() {
        let conn = setup_db();
        insert_row(
            &conn,
            "sodium",
            "1.0",
            "first",
            Some("2024-01-01T00:00:00Z"),
            "modrinth_id",
        );
        insert_row(
            &conn,
            "sodium",
            "1.1",
            "second",
            Some("2024-02-01T00:00:00Z"),
            "modrinth_id",
        );
        insert_row(
            &conn,
            "sodium",
            "1.2",
            "third",
            Some("2024-03-01T00:00:00Z"),
            "modrinth_id",
        );
        let rows = get_changelogs_for_item(&conn, "sodium").unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].version, "1.2");
        assert_eq!(rows[1].version, "1.1");
        assert_eq!(rows[2].version, "1.0");
    }

    #[test]
    fn for_version_exact_match() {
        let conn = setup_db();
        insert_row(
            &conn,
            "sodium",
            "0.6.0",
            "fixes",
            Some("2024-03-01T00:00:00Z"),
            "modrinth_id",
        );
        let hit = get_changelog_for_version(&conn, "sodium", "0.6.0")
            .unwrap()
            .unwrap();
        assert_eq!(hit.changelog, "fixes");
        let miss = get_changelog_for_version(&conn, "sodium", "0.5.0").unwrap();
        assert!(miss.is_none());
    }

    #[test]
    fn between_returns_interval_newest_first() {
        let conn = setup_db();
        insert_row(
            &conn,
            "sodium",
            "1.0",
            "c1",
            Some("2024-01-01T00:00:00Z"),
            "modrinth_id",
        );
        insert_row(
            &conn,
            "sodium",
            "1.1",
            "c2",
            Some("2024-02-01T00:00:00Z"),
            "modrinth_id",
        );
        insert_row(
            &conn,
            "sodium",
            "1.2",
            "c3",
            Some("2024-03-01T00:00:00Z"),
            "modrinth_id",
        );
        insert_row(
            &conn,
            "sodium",
            "1.3",
            "c4",
            Some("2024-04-01T00:00:00Z"),
            "modrinth_id",
        );

        // 1.0 -> 1.3 should return 1.3,1.2,1.1 (newest first, exclusive of from)
        let rows = get_changelogs_between(&conn, "sodium", "1.0", "1.3").unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].version, "1.3");
        assert_eq!(rows[1].version, "1.2");
        assert_eq!(rows[2].version, "1.1");

        // 1.1 -> 1.2 should return just 1.2
        let rows = get_changelogs_between(&conn, "sodium", "1.1", "1.2").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].version, "1.2");
    }

    #[test]
    fn between_unknown_from_returns_up_to_to() {
        let conn = setup_db();
        insert_row(
            &conn,
            "sodium",
            "1.0",
            "c1",
            Some("2024-01-01T00:00:00Z"),
            "modrinth_id",
        );
        insert_row(
            &conn,
            "sodium",
            "1.1",
            "c2",
            Some("2024-02-01T00:00:00Z"),
            "modrinth_id",
        );
        insert_row(
            &conn,
            "sodium",
            "1.2",
            "c3",
            Some("2024-03-01T00:00:00Z"),
            "modrinth_id",
        );
        // from unknown (e.g. "unknown" or very old beyond cap)
        let rows = get_changelogs_between(&conn, "sodium", "unknown", "1.2").unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].version, "1.2");
    }

    #[test]
    fn between_unknown_to_returns_empty() {
        let conn = setup_db();
        insert_row(
            &conn,
            "sodium",
            "1.0",
            "c1",
            Some("2024-01-01T00:00:00Z"),
            "modrinth_id",
        );
        let rows = get_changelogs_between(&conn, "sodium", "1.0", "9.9.9").unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn between_same_version_is_empty() {
        let conn = setup_db();
        insert_row(
            &conn,
            "sodium",
            "1.0",
            "c1",
            Some("2024-01-01T00:00:00Z"),
            "modrinth_id",
        );
        let rows = get_changelogs_between(&conn, "sodium", "1.0", "1.0").unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn between_downgrade_is_empty() {
        let conn = setup_db();
        insert_row(
            &conn,
            "sodium",
            "1.0",
            "c1",
            Some("2024-01-01T00:00:00Z"),
            "modrinth_id",
        );
        insert_row(
            &conn,
            "sodium",
            "1.1",
            "c2",
            Some("2024-02-01T00:00:00Z"),
            "modrinth_id",
        );
        // from 1.1 to 1.0 is a downgrade
        let rows = get_changelogs_between(&conn, "sodium", "1.1", "1.0").unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn github_source_stored_and_returned() {
        let conn = setup_db();
        insert_row(
            &conn,
            "my-mod",
            "v1.2.3",
            "release notes",
            Some("2024-05-01T00:00:00Z"),
            "github_release",
        );
        let hit = get_changelog_for_version(&conn, "my-mod", "v1.2.3")
            .unwrap()
            .unwrap();
        assert_eq!(hit.source, "github_release");
        assert_eq!(hit.changelog, "release notes");
    }
}
