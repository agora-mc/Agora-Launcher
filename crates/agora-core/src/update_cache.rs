//! Persistent update-check cache and bounded background sweep.
//!
//! Mirrors the `modrinth_content_metadata_cache` precedent (db.rs:370,
//! modrinth.rs:94 and :147): the DB owns the durable row, accessors take a
//! `&rusqlite::Connection` or a `Ctx`, and callers handle lock/migration
//! concerns. The persistent store is the source of truth across restarts;
//! the in-memory `AppState::update_candidate_cache` becomes a read-through
//! fast path over it, and the startup sweep refreshes stale entries without
//! ever blocking cold start.

use crate::ctx::Ctx;
use crate::db;
use crate::models::{InstanceManifest, ModVersionCandidate};
use crate::task_scheduler::BlockingPriority;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Information about an available update for an installed content item.
///
/// Shape must stay identical to the Tauri `UpdateInfo` at
/// `desktop/src-tauri/src/commands.rs:5360` and the TS interface at
/// `desktop/src/lib/tauri.ts:633`. Changing fields without coordinating both
/// sides breaks the IPC contract.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateInfo {
    pub filename: String,
    pub mod_jar_id: String,
    pub current_version: String,
    pub latest_version: String,
    pub target_version: String,
    pub source: String,
}

/// How long a cached candidate list is considered fresh for read-through.
///
/// Mirrors `UPDATE_CANDIDATE_CACHE_TTL` at `desktop/src-tauri/src/commands.rs:5369`.
pub const UPDATE_CANDIDATE_CACHE_TTL: Duration = Duration::from_secs(5 * 60);

// ---------------------------------------------------------------------------
// DB accessors — instance update cache
// ---------------------------------------------------------------------------

/// Persist the computed `updates` for `instance_id`.
///
/// Overwrites any prior row. Stores JSON so the shape can evolve without a
/// further migration, but callers should treat parse failures as cache misses.
pub fn set_cached_instance_updates(
    conn: &Connection,
    instance_id: &str,
    updates: &[UpdateInfo],
) -> anyhow::Result<()> {
    let json = serde_json::to_string(updates)?;
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO instance_update_cache (instance_id, updates_json, checked_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(instance_id) DO UPDATE SET
             updates_json = excluded.updates_json,
             checked_at = excluded.checked_at",
        rusqlite::params![instance_id, json, now],
    )?;
    Ok(())
}

/// Load cached updates for a single instance, if any.
///
/// Returns the deserialized updates and the RFC3339 `checked_at`. Corrupt
/// JSON is treated as empty (cache miss) rather than a hard error so a
/// bad write cannot permanently poison the cache.
pub fn get_cached_instance_updates(
    conn: &Connection,
    instance_id: &str,
) -> anyhow::Result<Option<(Vec<UpdateInfo>, String)>> {
    let mut stmt = conn.prepare(
        "SELECT updates_json, checked_at FROM instance_update_cache WHERE instance_id = ?1",
    )?;
    let mut rows = stmt.query([instance_id])?;
    if let Some(row) = rows.next()? {
        let json: String = row.get(0)?;
        let checked_at: String = row.get(1)?;
        let updates: Vec<UpdateInfo> = serde_json::from_str(&json).unwrap_or_default();
        Ok(Some((updates, checked_at)))
    } else {
        Ok(None)
    }
}

/// Load every cached instance row, newest checked first.
///
/// Used for instant UI hydration without network: the shell can render the
/// last sweep's results immediately while the background sweep refreshes.
pub fn get_all_cached_instance_updates(
    conn: &Connection,
) -> anyhow::Result<Vec<(String, Vec<UpdateInfo>, String)>> {
    let mut stmt = conn.prepare(
        "SELECT instance_id, updates_json, checked_at FROM instance_update_cache ORDER BY checked_at DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, json, checked_at) = row?;
        let updates: Vec<UpdateInfo> = serde_json::from_str(&json).unwrap_or_default();
        out.push((id, updates, checked_at));
    }
    Ok(out)
}

/// Delete a single instance's cached row, if present.
pub fn delete_cached_instance_updates(conn: &Connection, instance_id: &str) -> anyhow::Result<()> {
    conn.execute(
        "DELETE FROM instance_update_cache WHERE instance_id = ?1",
        [instance_id],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// DB accessors — candidate cache (read-through for AppState)
// ---------------------------------------------------------------------------

/// Persist a candidate list for `cache_key` (e.g. `source_id\\nmc_version\\nloader`).
pub fn set_cached_candidates(
    conn: &Connection,
    cache_key: &str,
    candidates: &[ModVersionCandidate],
) -> anyhow::Result<()> {
    let json = serde_json::to_string(candidates)?;
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO update_candidate_cache (cache_key, candidates_json, fetched_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(cache_key) DO UPDATE SET
             candidates_json = excluded.candidates_json,
             fetched_at = excluded.fetched_at",
        rusqlite::params![cache_key, json, now],
    )?;
    Ok(())
}

/// Load candidates for `cache_key` if they exist and are within TTL.
///
/// Corrupt JSON or an expired `fetched_at` is treated as a miss so the
/// caller falls through to network. Stale rows remain on disk until
/// overwritten or pruned; they are not hard-deleted here to keep reads cheap.
pub fn get_cached_candidates(
    conn: &Connection,
    cache_key: &str,
) -> anyhow::Result<Option<Vec<ModVersionCandidate>>> {
    let mut stmt = conn.prepare(
        "SELECT candidates_json, fetched_at FROM update_candidate_cache WHERE cache_key = ?1",
    )?;
    let mut rows = stmt.query([cache_key])?;
    if let Some(row) = rows.next()? {
        let json: String = row.get(0)?;
        let fetched_at: String = row.get(1)?;
        // Enforce the same 5-minute TTL as the in-memory cache.
        if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(&fetched_at) {
            let age = chrono::Utc::now().signed_duration_since(parsed.with_timezone(&chrono::Utc));
            if age.num_seconds() < 0
                || age.num_seconds() as u64 >= UPDATE_CANDIDATE_CACHE_TTL.as_secs()
            {
                return Ok(None);
            }
        } else {
            return Ok(None);
        }
        let candidates: Vec<ModVersionCandidate> = serde_json::from_str(&json).unwrap_or_default();
        Ok(Some(candidates))
    } else {
        Ok(None)
    }
}

/// Load candidates ignoring TTL (for debugging or sweep-warming). Not used
/// for read-through freshness checks.
pub fn get_cached_candidates_stale(
    conn: &Connection,
    cache_key: &str,
) -> anyhow::Result<Option<Vec<ModVersionCandidate>>> {
    let mut stmt =
        conn.prepare("SELECT candidates_json FROM update_candidate_cache WHERE cache_key = ?1")?;
    let mut rows = stmt.query([cache_key])?;
    if let Some(row) = rows.next()? {
        let json: String = row.get(0)?;
        let candidates: Vec<ModVersionCandidate> = serde_json::from_str(&json).unwrap_or_default();
        Ok(Some(candidates))
    } else {
        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// Ctx convenience wrappers (mirror modrinth.rs:94,147 pattern)
// ---------------------------------------------------------------------------

pub fn get_cached_instance_updates_ctx(
    ctx: &Ctx,
    instance_id: &str,
) -> anyhow::Result<Option<(Vec<UpdateInfo>, String)>> {
    let conn = db::local_state_connection(&ctx.paths.local_state_db())?;
    get_cached_instance_updates(&conn, instance_id)
}

pub fn set_cached_instance_updates_ctx(
    ctx: &Ctx,
    instance_id: &str,
    updates: &[UpdateInfo],
) -> anyhow::Result<()> {
    let conn = db::local_state_connection(&ctx.paths.local_state_db())?;
    set_cached_instance_updates(&conn, instance_id, updates)
}

pub fn get_all_cached_instance_updates_ctx(
    ctx: &Ctx,
) -> anyhow::Result<Vec<(String, Vec<UpdateInfo>, String)>> {
    let conn = db::local_state_connection(&ctx.paths.local_state_db())?;
    get_all_cached_instance_updates(&conn)
}

pub fn delete_cached_instance_updates_ctx(ctx: &Ctx, instance_id: &str) -> anyhow::Result<()> {
    let conn = db::local_state_connection(&ctx.paths.local_state_db())?;
    delete_cached_instance_updates(&conn, instance_id)
}

// ---------------------------------------------------------------------------
// Sweep
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct SweepSummary {
    pub total: usize,
    pub updated: usize,
    pub skipped: usize,
    pub failed: usize,
    /// Whether the sweep was skipped entirely due to offline/lockdown.
    pub offline_skipped: bool,
}

/// User-visible default: check for updates at most twice a day.
///
/// Mods rarely release more than once per day, and the sweep fans out to one
/// Modrinth request per installed mod. 12h halves the traffic for someone
/// who launches the app five times a day without making the badge feel stale.
pub const DEFAULT_SWEEP_INTERVAL_HOURS: u64 = 12;

/// Read the sweep interval in hours from `user_settings`.
///
/// 0 means “manual only” (no automatic refresh). Missing or malformed values
/// fall back to `DEFAULT_SWEEP_INTERVAL_HOURS`.
pub fn get_sweep_interval_hours(conn: &Connection) -> u64 {
    match db::get_setting(conn, "update_sweep_interval_hours")
        .ok()
        .flatten()
    {
        Some(v) => match v {
            serde_json::Value::Number(n) => n.as_u64().unwrap_or(DEFAULT_SWEEP_INTERVAL_HOURS),
            serde_json::Value::String(s) => {
                s.parse::<u64>().unwrap_or(DEFAULT_SWEEP_INTERVAL_HOURS)
            }
            serde_json::Value::Bool(b) => {
                if b {
                    DEFAULT_SWEEP_INTERVAL_HOURS
                } else {
                    0
                }
            }
            _ => DEFAULT_SWEEP_INTERVAL_HOURS,
        },
        None => DEFAULT_SWEEP_INTERVAL_HOURS,
    }
}

/// Whether a cached `checked_at` is stale enough to warrant a refresh.
///
/// 0 means never stale (manual only). `None` (no cached row) is always stale.
/// Parse failures are treated as stale so a corrupt timestamp does not block
/// future refreshes.
pub fn is_checked_at_stale(
    checked_at: Option<&str>,
    interval_hours: u64,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    if interval_hours == 0 {
        return false;
    }
    let Some(checked_at) = checked_at else {
        return true;
    };
    let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(checked_at) else {
        return true;
    };
    let parsed_utc = parsed.with_timezone(&chrono::Utc);
    let age = now.signed_duration_since(parsed_utc);
    // Future timestamps (clock skew) are treated as fresh.
    if age.num_seconds() < 0 {
        return false;
    }
    age.num_seconds() as u64 >= interval_hours * 3600
}

/// Wrapper for `is_checked_at_stale` using `Utc::now()` for production calls.
pub fn is_instance_stale(conn: &Connection, instance_id: &str, interval_hours: u64) -> bool {
    if interval_hours == 0 {
        return false;
    }
    let checked_at = get_cached_instance_updates(conn, instance_id)
        .ok()
        .flatten()
        .map(|(_, ts)| ts);
    is_checked_at_stale(checked_at.as_deref(), interval_hours, chrono::Utc::now())
}

/// Check if network is available for update checks.
///
/// Uses `NetworkPolicy` + direct Modrinth toggle. Lockdown is global block.
/// Returns `false` when sweep should be silent offline rather than error.
fn is_network_available_for_updates(conn: &Connection) -> bool {
    if db::is_lockdown_enabled(conn) {
        return false;
    }
    // Modrinth CDN and curated GitHub releases are the primary sources.
    // If both are disabled, an update sweep would produce no candidates.
    let modrinth = db::is_network_enabled(conn, "network_modrinth_enabled");
    let github = db::is_network_enabled(conn, "network_registry_sync_enabled")
        || db::is_network_enabled(conn, "network_modrinth_cdn_enabled");
    // At least one source should be enabled; otherwise treat as offline.
    modrinth || github
}

/// Core-owned bounded background sweep that refreshes update caches for all
/// instances.
///
/// Contract (mirrors `maintenance::prewarm_recent_instances` at
/// `crates/agora-core/src/maintenance.rs:32`):
/// - Runs on `BlockingPriority::Background` for the DB listing phase.
/// - Never blocks launch: acquires per-instance locks with 25ms timeout and
///   skips contended instances.
/// - Respects `NetworkPolicy`/lockdown silently (no error, `offline_skipped`).
/// - Individual instance failures are counted, not bubbled as sweep failure.
/// - All network work is bounded and best-effort; a cold start never awaits
///   this beyond spawning it.
pub async fn sweep_all_updates(ctx: Ctx) -> Result<SweepSummary, String> {
    // Phase 1: list instances on the background scheduler lane.
    let scheduler = ctx.task_scheduler.clone();
    let ctx_for_list = ctx.clone();
    let instances = scheduler
        .run_blocking(BlockingPriority::Background, move || {
            let conn = db::local_state_connection(&ctx_for_list.paths.local_state_db())
                .map_err(|e| format!("could not open local state for update sweep: {e}"))?;
            // Check network availability before doing any network work.
            if !is_network_available_for_updates(&conn) {
                return Ok::<Option<Vec<crate::models::InstanceRow>>, String>(None);
            }
            let rows = db::list_instances(&conn)
                .map_err(|e| format!("could not list instances for update sweep: {e}"))?;
            Ok(Some(rows))
        })
        .await
        .map_err(|e| format!("update sweep worker failed: {e}"))??;

    let Some(rows) = instances else {
        return Ok(SweepSummary {
            offline_skipped: true,
            ..Default::default()
        });
    };

    // Respect the user-configured sweep interval. 0 means manual only.
    let interval_hours = db::local_state_connection(&ctx.paths.local_state_db())
        .ok()
        .map(|c| get_sweep_interval_hours(&c))
        .unwrap_or(DEFAULT_SWEEP_INTERVAL_HOURS);
    if interval_hours == 0 {
        return Ok(SweepSummary {
            total: rows.len(),
            skipped: rows.len(),
            ..Default::default()
        });
    }

    let mut summary = SweepSummary {
        total: rows.len(),
        ..Default::default()
    };

    // Drop DB connection before network phase.

    for row in rows {
        // Staleness check: only refresh if checked_at is older than the interval.
        // No row (never checked) is considered stale.
        let should_refresh = {
            let conn = match db::local_state_connection(&ctx.paths.local_state_db()) {
                Ok(c) => c,
                Err(_) => {
                    summary.failed += 1;
                    continue;
                }
            };
            let cached = get_cached_instance_updates(&conn, &row.instance_id)
                .ok()
                .flatten()
                .map(|(_, ts)| ts);
            is_checked_at_stale(cached.as_deref(), interval_hours, chrono::Utc::now())
        };
        if !should_refresh {
            summary.skipped += 1;
            continue;
        }

        // Respect per-instance lock like maintenance.rs:66-80.
        let lock = match ctx.lock_manager.acquire_with_timeout(
            crate::lock_manager::LockResource::Instance(row.instance_id.clone()),
            "update-sweep",
            Duration::from_millis(25),
            None,
        ) {
            Ok(lock) => lock,
            Err(_) => {
                summary.skipped += 1;
                continue;
            }
        };
        // This is a probe, not a long-lived lock. Release immediately so a
        // concurrent launch/install never waits behind the sweep.
        drop(lock);

        // Perform the check. Errors are per-instance, not sweep-wide.
        match check_single_instance_updates(&ctx, &row.instance_id).await {
            Ok(updates) => {
                // Persist regardless of empty (empty means no updates, still valid).
                if let Ok(conn) = db::local_state_connection(&ctx.paths.local_state_db()) {
                    let _ = set_cached_instance_updates(&conn, &row.instance_id, &updates);
                }
                summary.updated += 1;
            }
            Err(_) => {
                summary.failed += 1;
            }
        }
    }

    Ok(summary)
}

/// Check a single instance for updates using core services only (no AppHandle).
///
/// Replicates the logic in `desktop/src-tauri/src/commands.rs:5403` but via
/// `Ctx` + `Resolver`/`ModrinthService` so the core stays host-independent.
pub async fn check_single_instance_updates(
    ctx: &Ctx,
    instance_id: &str,
) -> anyhow::Result<Vec<UpdateInfo>> {
    // Validate and resolve manifest path via core AppPaths.
    let manifest_path = ctx
        .paths
        .instance_manifest(instance_id)
        .map_err(|e| anyhow::anyhow!("invalid instance id: {e}"))?;
    let manifest_text = std::fs::read_to_string(&manifest_path)
        .map_err(|e| anyhow::anyhow!("could not read manifest: {e}"))?;
    let manifest: InstanceManifest = serde_json::from_str(&manifest_text)
        .map_err(|e| anyhow::anyhow!("could not parse manifest: {e}"))?;

    let mut updates = Vec::new();

    // We need the instance row for mc_version/loader only for Modrinth candidates
    // that scope to instance. The manifest already carries those, so we can skip DB read.

    let all_mods = manifest
        .mods
        .iter()
        .chain(manifest.resourcepacks.iter())
        .chain(manifest.shaders.iter())
        .chain(manifest.datapacks.iter())
        .chain(manifest.worlds.iter());

    for installed in all_mods {
        if let Some(project_id) = installed
            .modrinth_id
            .as_deref()
            .filter(|_| installed.source == "modrinth_raw")
        {
            let project_type = match installed.content_type.as_str() {
                "resourcepack" | "resourcepacks" => "resourcepack",
                "shader" | "shaders" | "shaderpack" | "shaderpacks" => "shader",
                "datapack" | "datapacks" => "datapack",
                "world" | "worlds" => "modpack",
                _ => "mod",
            };
            // Use ModrinthService (core) instead of desktop adapter.
            let svc = crate::modrinth::ModrinthService::new(ctx.clone());
            let candidates = match svc
                .list_raw_modrinth_versions(Some(instance_id), project_id, Some(project_type))
                .await
            {
                Ok(c) => c,
                Err(_) => continue,
            };
            let Some(candidate) = candidates.first() else {
                continue;
            };
            let current = installed.version.as_deref().unwrap_or("");
            if (current == candidate.version_id || current == candidate.version)
                && installed.filename == candidate.filename
            {
                continue;
            }
            updates.push(UpdateInfo {
                filename: installed.filename.clone(),
                mod_jar_id: project_id.to_string(),
                current_version: installed
                    .version
                    .clone()
                    .unwrap_or_else(|| "unknown".into()),
                latest_version: candidate.version.clone(),
                target_version: candidate.version_id.clone(),
                source: installed.source.clone(),
            });
            continue;
        }

        let Some(registry_id) = installed.registry_id.as_deref() else {
            continue;
        };

        // Curated path: resolver via core.
        let item = {
            // Registry DB is separate; open it read-only.
            let reg_conn = match db::registry_connection(&ctx.paths.registry_db()) {
                Ok(c) => c,
                Err(_) => continue,
            };
            // Load item from registry. If not found, skip.
            match crate::registry::get_item_by_id(&reg_conn, registry_id) {
                Ok(Some(it)) => it,
                _ => continue,
            }
        };

        // Build resolver and list candidates. Use bounded (update) path.
        let resolver = crate::resolver::Resolver::new(ctx.clone());
        let candidates = match resolver
            .list_curated_versions_for_update(&item, &manifest.minecraft_version, &manifest.loader)
            .await
        {
            Ok(c) => c,
            Err(_) => continue,
        };
        let Some(candidate) = candidates
            .iter()
            .find(|c| c.is_compatible)
            .or_else(|| candidates.first())
        else {
            continue;
        };
        let same_version = installed.version.as_deref() == Some(candidate.version.as_str());
        let same_filename = installed.filename == candidate.filename;
        let same_hash = candidate
            .sha256
            .as_deref()
            .map(|h| h.eq_ignore_ascii_case(&installed.sha256))
            .unwrap_or(true);
        if same_version && same_filename && same_hash {
            continue;
        }
        updates.push(UpdateInfo {
            filename: installed.filename.clone(),
            mod_jar_id: registry_id.to_string(),
            current_version: installed
                .version
                .clone()
                .unwrap_or_else(|| "unknown".into()),
            latest_version: candidate.version.clone(),
            target_version: candidate.version.clone(),
            source: installed.source.clone(),
        });
    }

    Ok(updates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn test_conn() -> (Connection, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("local_state.db");
        crate::db::init_local_state_db(&path).expect("init db");
        // Open exactly as production does. `instance_update_cache` has an FK to
        // `user_instances`, which SQLite only enforces with `foreign_keys = ON`;
        // opening raw here would let a test pass against a constraint the real
        // app path would reject.
        let conn = crate::db::local_state_connection(&path).expect("open db");
        // Ensure v11 migrated.
        let version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, crate::db::LOCAL_STATE_SCHEMA_VERSION);
        (conn, dir)
    }

    fn sample_updates() -> Vec<UpdateInfo> {
        vec![UpdateInfo {
            filename: "sodium-0.6.0.jar".into(),
            mod_jar_id: "sodium".into(),
            current_version: "0.5.0".into(),
            latest_version: "0.6.0".into(),
            target_version: "0.6.0".into(),
            source: "registry".into(),
        }]
    }

    #[test]
    fn set_and_get_instance_updates_roundtrip() {
        let (conn, _dir) = test_conn();
        // Need an instance row for FK.
        let row = crate::models::InstanceRow {
            instance_id: "test-instance".into(),
            name: "Test".into(),
            minecraft_version: "1.21.1".into(),
            loader: "fabric".into(),
            loader_version: "0.16.0".into(),
            is_modpack: false,
            is_locked: false,
            last_launched_at: None,
            jvm_memory_mb: 4096,
            jvm_memory_mode: "auto".into(),
            jvm_gc: "auto".into(),
            jvm_custom_args: String::new(),
            jvm_always_pre_touch: true,
            created_at: chrono::Utc::now().to_rfc3339(),
            java_path: None,
            java_incompatible_override: false,
            icon_path: None,
            launch_mode_override: "auto".into(),
            import_source: None,
        };
        crate::db::upsert_instance(&conn, &row).unwrap();

        let updates = sample_updates();
        set_cached_instance_updates(&conn, &row.instance_id, &updates).unwrap();
        let loaded = get_cached_instance_updates(&conn, &row.instance_id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.0, updates);
    }

    #[test]
    fn get_all_instance_updates_orders_by_checked_at() {
        let (conn, _dir) = test_conn();
        for id in ["a", "b"] {
            let row = crate::models::InstanceRow {
                instance_id: id.into(),
                name: id.into(),
                minecraft_version: "1.21.1".into(),
                loader: "fabric".into(),
                loader_version: "0.16.0".into(),
                is_modpack: false,
                is_locked: false,
                last_launched_at: None,
                jvm_memory_mb: 4096,
                jvm_memory_mode: "auto".into(),
                jvm_gc: "auto".into(),
                jvm_custom_args: String::new(),
                jvm_always_pre_touch: true,
                created_at: chrono::Utc::now().to_rfc3339(),
                java_path: None,
                java_incompatible_override: false,
                icon_path: None,
                launch_mode_override: "auto".into(),
                import_source: None,
            };
            crate::db::upsert_instance(&conn, &row).unwrap();
            set_cached_instance_updates(&conn, id, &sample_updates()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let all = get_all_cached_instance_updates(&conn).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn candidate_cache_ttl() {
        let (conn, _dir) = test_conn();
        let key = "sodium\n1.21.1\nfabric";
        let candidates = vec![ModVersionCandidate {
            version: "0.6.0".into(),
            filename: "sodium-0.6.0.jar".into(),
            download_url: "https://example.com/sodium.jar".into(),
            mc_version: Some("1.21.1".into()),
            loader: Some("fabric".into()),
            release_date: None,
            is_compatible: true,
            sha1: None,
            sha256: None,
            sha512: None,
            size: None,
            version_compat: "compatible".into(),
            is_prerelease: false,
            source_strategy: Some("github_release".into()),
            source_identifier: Some("example/sodium".into()),
        }];
        set_cached_candidates(&conn, key, &candidates).unwrap();
        // Fresh should be readable.
        let fresh = get_cached_candidates(&conn, key).unwrap();
        assert!(fresh.is_some());
        // Simulate expired by backdating fetched_at.
        conn.execute(
            "UPDATE update_candidate_cache SET fetched_at = ?1 WHERE cache_key = ?2",
            rusqlite::params![
                chrono::Utc::now()
                    .checked_sub_signed(chrono::Duration::seconds(600))
                    .unwrap()
                    .to_rfc3339(),
                key
            ],
        )
        .unwrap();
        let stale = get_cached_candidates(&conn, key).unwrap();
        assert!(stale.is_none(), "expired candidate should be miss");
        // Stale read ignoring TTL should still return data.
        let stale_ignored = get_cached_candidates_stale(&conn, key).unwrap();
        assert!(stale_ignored.is_some());
    }

    #[test]
    fn update_info_shape_is_stable() {
        // Ensure serialized shape matches desktop expectation.
        let info = sample_updates().into_iter().next().unwrap();
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(
            json.get("filename").unwrap().as_str().unwrap(),
            "sodium-0.6.0.jar"
        );
        assert_eq!(json.get("mod_jar_id").unwrap().as_str().unwrap(), "sodium");
        assert!(json.get("current_version").is_some());
        assert!(json.get("latest_version").is_some());
        assert!(json.get("target_version").is_some());
        assert!(json.get("source").is_some());
    }

    #[test]
    fn is_network_available_respects_lockdown() {
        let (conn, _dir) = test_conn();
        // By default, not in lockdown, network available (modrinth true by init).
        assert!(is_network_available_for_updates(&conn));
        crate::db::set_setting(&conn, "network_lockdown_enabled", &serde_json::json!(true))
            .unwrap();
        assert!(!is_network_available_for_updates(&conn));
    }

    #[test]
    fn sweep_interval_defaults_to_12_when_missing() {
        let (conn, _dir) = test_conn();
        // init_local_state_db seeds 12, but delete to test fallback
        conn.execute(
            "DELETE FROM user_settings WHERE key = 'update_sweep_interval_hours'",
            [],
        )
        .unwrap();
        assert_eq!(get_sweep_interval_hours(&conn), 12);
    }

    #[test]
    fn sweep_interval_zero_means_off() {
        let (conn, _dir) = test_conn();
        crate::db::set_setting(&conn, "update_sweep_interval_hours", &serde_json::json!(0))
            .unwrap();
        assert_eq!(get_sweep_interval_hours(&conn), 0);
        // 0 means never stale
        let now = chrono::Utc::now();
        let recent = (now - chrono::Duration::hours(1)).to_rfc3339();
        assert!(!is_checked_at_stale(Some(&recent), 0, now));
        // Missing would also be not stale when interval is 0
        assert!(!is_checked_at_stale(None, 0, now));
    }

    #[test]
    fn sweep_interval_string_and_number_parsing() {
        let (conn, _dir) = test_conn();
        crate::db::set_setting(
            &conn,
            "update_sweep_interval_hours",
            &serde_json::Value::String("6".into()),
        )
        .unwrap();
        assert_eq!(get_sweep_interval_hours(&conn), 6);

        crate::db::set_setting(&conn, "update_sweep_interval_hours", &serde_json::json!(24))
            .unwrap();
        assert_eq!(get_sweep_interval_hours(&conn), 24);
    }

    #[test]
    fn checked_at_staleness_respects_interval() {
        let now = chrono::Utc::now();
        let fresh = (now - chrono::Duration::hours(1)).to_rfc3339();
        let old = (now - chrono::Duration::hours(13)).to_rfc3339();
        // 12h interval: 1h ago is fresh (not stale), 13h ago is stale
        assert!(!is_checked_at_stale(Some(&fresh), 12, now));
        assert!(is_checked_at_stale(Some(&old), 12, now));
        // Exactly at boundary is stale
        let at_boundary = (now - chrono::Duration::hours(12)).to_rfc3339();
        assert!(is_checked_at_stale(Some(&at_boundary), 12, now));
        // Missing is always stale when interval >0
        assert!(is_checked_at_stale(None, 12, now));
        // Corrupt timestamp is stale
        assert!(is_checked_at_stale(Some("not-a-date"), 12, now));
        // Future timestamp is not stale (clock skew)
        let future = (now + chrono::Duration::hours(1)).to_rfc3339();
        assert!(!is_checked_at_stale(Some(&future), 12, now));
    }
}
