//! Everything the plugin system persists.
//!
//! Two distinct things live here and are deliberately kept apart:
//!
//! - the **install record** — what the user agreed to. Which plugins exist,
//!   which are switched on, and crucially which capabilities were granted.
//!   The grant is stored *next to* the manifest rather than re-derived from it
//!   on load, so shipping an update that asks for more permission does not
//!   silently receive it.
//! - the **plugin's own state** — settings the host declared and validates,
//!   and free-form data the plugin wrote. Split by `kind` so a plugin cannot
//!   write over a declared setting and leave its own settings page unable to
//!   render the result.
//!
//! Plugin state survives disabling and survives replacing the package, because
//! neither of those is the user saying "throw my data away". Uninstalling asks
//! that question separately.

use crate::error::{LauncherError, LauncherResult};
use agora_plugin_api::capability::CapabilitySet;
use agora_plugin_api::distribution::PublicKey;
use agora_plugin_api::manifest::{PluginId, PluginManifest};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Which kind of stored value a key belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StorageKind {
    /// Declared in the manifest, rendered by the host, validated on write.
    Setting,
    /// Written by the plugin through `storage.*`. Opaque to the host.
    Data,
}

impl StorageKind {
    fn as_str(self) -> &'static str {
        match self {
            StorageKind::Setting => "setting",
            StorageKind::Data => "data",
        }
    }
}

/// Where a plugin's files came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum PluginSource {
    /// Installed from an archive into Agora's own directory.
    Package,
    /// A folder the author is editing right now. Loaded in place, reloadable,
    /// and never copied — so "edit, reload, see it" is the development loop.
    Development { path: PathBuf },
}

impl PluginSource {
    pub fn is_development(&self) -> bool {
        matches!(self, PluginSource::Development { .. })
    }

    fn kind_str(&self) -> &'static str {
        match self {
            PluginSource::Package => "package",
            PluginSource::Development { .. } => "development",
        }
    }
}

/// One installed plugin, as recorded.
#[derive(Debug, Clone)]
pub struct PluginRecord {
    pub manifest: PluginManifest,
    /// Capabilities the user granted. May be narrower than the manifest asks
    /// for if the manifest changed since the grant.
    pub granted: CapabilitySet,
    pub source: PluginSource,
    /// Directory the plugin's files are read from.
    pub install_dir: PathBuf,
    pub enabled: bool,
    pub installed_at: String,
    pub updated_at: String,
    pub data_version: u32,
    /// Why this plugin is not working, if it is not.
    pub last_error: Option<String>,
}

/// The publisher URL, keys, and anti-replay state recorded for an installed
/// plugin's update channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustRecord {
    pub url: String,
    pub keys: Vec<PublicKey>,
    pub highest_sequence: u64,
    pub last_checked_at: Option<String>,
    pub last_result: Option<String>,
}

impl PluginRecord {
    pub fn id(&self) -> &PluginId {
        &self.manifest.id
    }
}

// ---------------------------------------------------------------------------
// Quotas
// ---------------------------------------------------------------------------

/// How many keys one plugin may store in one scope.
pub const MAX_KEYS_PER_SCOPE: usize = 500;

/// How large a single stored value may be, serialised.
pub const MAX_VALUE_BYTES: usize = 64 * 1024;

/// How long a storage key may be.
pub const MAX_KEY_LEN: usize = 128;

// ---------------------------------------------------------------------------
// Install records
// ---------------------------------------------------------------------------

fn db_error(error: impl std::fmt::Display) -> LauncherError {
    LauncherError::Generic {
        code: "ERR_LOCAL_STATE_FAILED".into(),
        message: error.to_string(),
    }
}

/// Every installed plugin, in id order.
pub fn list(conn: &Connection) -> LauncherResult<Vec<PluginRecord>> {
    let mut stmt = conn
        .prepare(
            "SELECT plugin_id, manifest_json, granted_capabilities, source_kind, source_path,
                    install_dir, enabled, installed_at, updated_at, data_version, last_error
             FROM plugin_installs ORDER BY plugin_id",
        )
        .map_err(db_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)? != 0,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, i64>(9)?,
                row.get::<_, Option<String>>(10)?,
            ))
        })
        .map_err(db_error)?;

    let mut out = Vec::new();
    for row in rows {
        let row = row.map_err(db_error)?;
        // A record whose manifest no longer parses is skipped rather than
        // fatal: one corrupt row must not stop the launcher from starting, and
        // the plugin manager shows the gap as an uninstallable entry.
        let Ok(manifest) = serde_json::from_str::<PluginManifest>(&row.1) else {
            continue;
        };
        let granted: Vec<String> = serde_json::from_str(&row.2).unwrap_or_default();
        let granted = CapabilitySet::from_capabilities(
            granted
                .iter()
                .filter_map(|name| name.parse::<agora_plugin_api::Capability>().ok()),
        );
        let source = match row.3.as_str() {
            "development" => PluginSource::Development {
                path: PathBuf::from(row.4.clone().unwrap_or_default()),
            },
            _ => PluginSource::Package,
        };
        out.push(PluginRecord {
            manifest,
            granted,
            source,
            install_dir: PathBuf::from(row.5),
            enabled: row.6,
            installed_at: row.7,
            updated_at: row.8,
            data_version: row.9.max(0) as u32,
            last_error: row.10,
        });
    }
    Ok(out)
}

pub fn get(conn: &Connection, plugin_id: &PluginId) -> LauncherResult<Option<PluginRecord>> {
    Ok(list(conn)?
        .into_iter()
        .find(|record| record.id() == plugin_id))
}

/// Insert or replace an install record.
#[allow(clippy::too_many_arguments)]
pub fn upsert(
    conn: &Connection,
    manifest: &PluginManifest,
    granted: &CapabilitySet,
    source: &PluginSource,
    install_dir: &std::path::Path,
    enabled: bool,
    now: &str,
) -> LauncherResult<()> {
    let manifest_json = serde_json::to_string(manifest).map_err(db_error)?;
    let granted_json = serde_json::to_string(
        &granted
            .iter()
            .map(|cap| cap.as_str().to_string())
            .collect::<Vec<_>>(),
    )
    .map_err(db_error)?;
    let source_path = match source {
        PluginSource::Development { path } => Some(path.to_string_lossy().to_string()),
        PluginSource::Package => None,
    };

    conn.execute(
        "INSERT INTO plugin_installs
             (plugin_id, version, manifest_json, granted_capabilities, source_kind, source_path,
              install_dir, enabled, installed_at, updated_at, data_version, last_error)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?10, NULL)
         ON CONFLICT(plugin_id) DO UPDATE SET
             version = excluded.version,
             manifest_json = excluded.manifest_json,
             granted_capabilities = excluded.granted_capabilities,
             source_kind = excluded.source_kind,
             source_path = excluded.source_path,
             install_dir = excluded.install_dir,
             enabled = excluded.enabled,
             updated_at = excluded.updated_at,
             data_version = excluded.data_version,
             last_error = NULL",
        params![
            manifest.id.as_str(),
            manifest.version.to_string(),
            manifest_json,
            granted_json,
            source.kind_str(),
            source_path,
            install_dir.to_string_lossy().to_string(),
            enabled as i64,
            now,
            manifest.data_version as i64,
        ],
    )
    .map_err(db_error)?;
    Ok(())
}

pub fn set_enabled(
    conn: &Connection,
    plugin_id: &PluginId,
    enabled: bool,
    now: &str,
) -> LauncherResult<bool> {
    let changed = conn
        .execute(
            "UPDATE plugin_installs SET enabled = ?2, updated_at = ?3 WHERE plugin_id = ?1",
            params![plugin_id.as_str(), enabled as i64, now],
        )
        .map_err(db_error)?;
    Ok(changed > 0)
}

/// Record why a plugin is not working, for the manager to show.
pub fn set_last_error(
    conn: &Connection,
    plugin_id: &PluginId,
    error: Option<&str>,
) -> LauncherResult<()> {
    conn.execute(
        "UPDATE plugin_installs SET last_error = ?2 WHERE plugin_id = ?1",
        params![plugin_id.as_str(), error],
    )
    .map_err(db_error)?;
    Ok(())
}

/// Read the update source and anti-replay state for one plugin.
type TrustRow = (String, String, i64, Option<String>, Option<String>);

pub fn get_trust(conn: &Connection, plugin_id: &PluginId) -> LauncherResult<Option<TrustRecord>> {
    let row: Option<TrustRow> = conn
        .query_row(
            "SELECT update_url, keys_json, highest_sequence, last_checked_at, last_result
             FROM plugin_trust WHERE plugin_id = ?1",
            params![plugin_id.as_str()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()
        .map_err(db_error)?;
    let Some((url, keys_json, highest_sequence, last_checked_at, last_result)) = row else {
        return Ok(None);
    };
    let keys = serde_json::from_str(&keys_json).map_err(db_error)?;
    let highest_sequence = u64::try_from(highest_sequence)
        .map_err(|error| db_error(format!("invalid stored plugin update sequence: {error}")))?;
    Ok(Some(TrustRecord {
        url,
        keys,
        highest_sequence,
        last_checked_at,
        last_result,
    }))
}

/// Store a plugin's update source without disturbing its anti-replay floor.
pub fn put_trust(
    conn: &Connection,
    plugin_id: &PluginId,
    url: &str,
    keys: &[PublicKey],
) -> LauncherResult<()> {
    let keys_json = serde_json::to_string(keys).map_err(db_error)?;
    conn.execute(
        "INSERT INTO plugin_trust (plugin_id, update_url, keys_json)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(plugin_id) DO UPDATE SET
             update_url = excluded.update_url,
             keys_json = excluded.keys_json",
        params![plugin_id.as_str(), url, keys_json],
    )
    .map_err(db_error)?;
    Ok(())
}

/// Record the result of a check while preserving the greatest sequence seen.
pub fn record_check(
    conn: &Connection,
    plugin_id: &PluginId,
    sequence: u64,
    at: &str,
    result: &str,
) -> LauncherResult<()> {
    let sequence = i64::try_from(sequence)
        .map_err(|error| db_error(format!("plugin update sequence is out of range: {error}")))?;
    conn.execute(
        "UPDATE plugin_trust
         SET highest_sequence = MAX(highest_sequence, ?2),
             last_checked_at = ?3,
             last_result = ?4
         WHERE plugin_id = ?1",
        params![plugin_id.as_str(), sequence, at, result],
    )
    .map_err(db_error)?;
    Ok(())
}

/// Remove the update source and its anti-replay state.
pub fn clear_trust(conn: &Connection, plugin_id: &PluginId) -> LauncherResult<()> {
    conn.execute(
        "DELETE FROM plugin_trust WHERE plugin_id = ?1",
        params![plugin_id.as_str()],
    )
    .map_err(db_error)?;
    Ok(())
}

/// Remove the install record. `purge_data` decides whether the plugin's own
/// settings and data go with it — a distinct choice, never implied by
/// uninstalling.
pub fn remove(conn: &Connection, plugin_id: &PluginId, purge_data: bool) -> LauncherResult<()> {
    if purge_data {
        // `plugin_storage` and `plugin_data_checkpoints` cascade from here, so
        // deleting the record is what actually discards the user's data.
        conn.execute(
            "DELETE FROM plugin_installs WHERE plugin_id = ?1",
            params![plugin_id.as_str()],
        )
        .map_err(db_error)?;
        return Ok(());
    }

    // Keeping the data means keeping the row the data hangs off. The record
    // becomes a tombstone — no install directory, not enabled — which the
    // registry filters out of "installed plugins" and a later reinstall of the
    // same plugin id adopts, restoring the user's settings.
    conn.execute(
        "UPDATE plugin_installs
         SET enabled = 0, install_dir = '', last_error = 'uninstalled; data retained'
         WHERE plugin_id = ?1",
        params![plugin_id.as_str()],
    )
    .map_err(db_error)?;
    Ok(())
}

/// Whether a record is a tombstone left by an uninstall that kept data.
///
/// Tombstones are not plugins. They exist so that reinstalling `acme.dashboard`
/// next month finds the settings the user had configured this month.
pub fn is_tombstone(record: &PluginRecord) -> bool {
    record.install_dir.as_os_str().is_empty()
}

// ---------------------------------------------------------------------------
// Plugin storage
// ---------------------------------------------------------------------------

/// Read one value. `instance_id` scopes the key; `None` is the global scope.
pub fn storage_get(
    conn: &Connection,
    plugin_id: &PluginId,
    kind: StorageKind,
    instance_id: Option<&str>,
    key: &str,
) -> LauncherResult<Option<serde_json::Value>> {
    let text: Option<String> = conn
        .query_row(
            "SELECT value_json FROM plugin_storage
             WHERE plugin_id = ?1 AND kind = ?2 AND instance_id = ?3 AND key = ?4",
            params![
                plugin_id.as_str(),
                kind.as_str(),
                instance_id.unwrap_or(""),
                key
            ],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_error)?;
    Ok(text.and_then(|text| serde_json::from_str(&text).ok()))
}

/// Every value in one scope.
pub fn storage_all(
    conn: &Connection,
    plugin_id: &PluginId,
    kind: StorageKind,
    instance_id: Option<&str>,
) -> LauncherResult<serde_json::Map<String, serde_json::Value>> {
    let mut stmt = conn
        .prepare(
            "SELECT key, value_json FROM plugin_storage
             WHERE plugin_id = ?1 AND kind = ?2 AND instance_id = ?3 ORDER BY key",
        )
        .map_err(db_error)?;
    let rows = stmt
        .query_map(
            params![plugin_id.as_str(), kind.as_str(), instance_id.unwrap_or("")],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(db_error)?;
    let mut out = serde_json::Map::new();
    for row in rows {
        let (key, text) = row.map_err(db_error)?;
        if let Ok(value) = serde_json::from_str(&text) {
            out.insert(key, value);
        }
    }
    Ok(out)
}

/// Write one value, enforcing the per-plugin quota.
pub fn storage_set(
    conn: &Connection,
    plugin_id: &PluginId,
    kind: StorageKind,
    instance_id: Option<&str>,
    key: &str,
    value: &serde_json::Value,
) -> LauncherResult<()> {
    if key.is_empty() || key.len() > MAX_KEY_LEN {
        return Err(LauncherError::Generic {
            code: "ERR_PLUGIN_STORAGE_KEY".into(),
            message: format!("a storage key must be 1-{MAX_KEY_LEN} characters"),
        });
    }
    let text = serde_json::to_string(value).map_err(db_error)?;
    if text.len() > MAX_VALUE_BYTES {
        return Err(LauncherError::Generic {
            code: "ERR_PLUGIN_STORAGE_QUOTA".into(),
            message: format!(
                "that value is {} bytes; a plugin may store {MAX_VALUE_BYTES} per key",
                text.len()
            ),
        });
    }

    // Counted before the insert, and only when the key is new, so overwriting
    // an existing key keeps working at the quota rather than wedging a plugin
    // that has filled it.
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM plugin_storage
             WHERE plugin_id = ?1 AND kind = ?2 AND instance_id = ?3 AND key = ?4",
            params![
                plugin_id.as_str(),
                kind.as_str(),
                instance_id.unwrap_or(""),
                key
            ],
            |row| row.get(0),
        )
        .map_err(db_error)?;
    if exists == 0 {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM plugin_storage
                 WHERE plugin_id = ?1 AND kind = ?2 AND instance_id = ?3",
                params![plugin_id.as_str(), kind.as_str(), instance_id.unwrap_or("")],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if count as usize >= MAX_KEYS_PER_SCOPE {
            return Err(LauncherError::Generic {
                code: "ERR_PLUGIN_STORAGE_QUOTA".into(),
                message: format!(
                    "a plugin may store {MAX_KEYS_PER_SCOPE} keys per scope; `{}` is full",
                    plugin_id.as_str()
                ),
            });
        }
    }

    conn.execute(
        "INSERT INTO plugin_storage (plugin_id, kind, instance_id, key, value_json)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(plugin_id, kind, instance_id, key)
         DO UPDATE SET value_json = excluded.value_json",
        params![
            plugin_id.as_str(),
            kind.as_str(),
            instance_id.unwrap_or(""),
            key,
            text
        ],
    )
    .map_err(db_error)?;
    Ok(())
}

pub fn storage_remove(
    conn: &Connection,
    plugin_id: &PluginId,
    kind: StorageKind,
    instance_id: Option<&str>,
    key: &str,
) -> LauncherResult<bool> {
    let removed = conn
        .execute(
            "DELETE FROM plugin_storage
             WHERE plugin_id = ?1 AND kind = ?2 AND instance_id = ?3 AND key = ?4",
            params![
                plugin_id.as_str(),
                kind.as_str(),
                instance_id.unwrap_or(""),
                key
            ],
        )
        .map_err(db_error)?;
    Ok(removed > 0)
}

// ---------------------------------------------------------------------------
// Data checkpoints
// ---------------------------------------------------------------------------

/// Copy a plugin's whole stored state aside before a migration runs.
///
/// Taken when `dataVersion` changes on upgrade. If the new version's migration
/// mangles the data, this is what "restore" restores — which is the honest
/// version of rollback, because putting the old package bytes back does not by
/// itself undo a write.
pub fn capture_checkpoint(
    conn: &Connection,
    plugin_id: &PluginId,
    from_version: &str,
    data_version: u32,
    now: &str,
) -> LauncherResult<()> {
    let settings = storage_all_scopes(conn, plugin_id)?;
    let payload = serde_json::to_string(&settings).map_err(db_error)?;
    conn.execute(
        "INSERT INTO plugin_data_checkpoints
             (plugin_id, from_version, data_version, captured_at, payload_json)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            plugin_id.as_str(),
            from_version,
            data_version as i64,
            now,
            payload
        ],
    )
    .map_err(db_error)?;
    // Two is enough to recover from a bad upgrade without the table growing
    // without bound for a plugin that ships often.
    conn.execute(
        "DELETE FROM plugin_data_checkpoints
         WHERE plugin_id = ?1 AND id NOT IN (
             SELECT id FROM plugin_data_checkpoints WHERE plugin_id = ?1
             ORDER BY captured_at DESC, id DESC LIMIT 2
         )",
        params![plugin_id.as_str()],
    )
    .map_err(db_error)?;
    Ok(())
}

/// One stored row, flattened for checkpointing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageRow {
    pub kind: String,
    pub instance_id: String,
    pub key: String,
    pub value: serde_json::Value,
}

fn storage_all_scopes(conn: &Connection, plugin_id: &PluginId) -> LauncherResult<Vec<StorageRow>> {
    let mut stmt = conn
        .prepare(
            "SELECT kind, instance_id, key, value_json FROM plugin_storage
             WHERE plugin_id = ?1 ORDER BY kind, instance_id, key",
        )
        .map_err(db_error)?;
    let rows = stmt
        .query_map(params![plugin_id.as_str()], |row| {
            Ok(StorageRow {
                kind: row.get(0)?,
                instance_id: row.get(1)?,
                key: row.get(2)?,
                value: serde_json::from_str(&row.get::<_, String>(3)?)
                    .unwrap_or(serde_json::Value::Null),
            })
        })
        .map_err(db_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(db_error)
}

/// What a checkpoint holds, for the manager to describe before restoring.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointSummary {
    pub from_version: String,
    pub data_version: u32,
    pub captured_at: String,
    pub entry_count: usize,
}

pub fn latest_checkpoint(
    conn: &Connection,
    plugin_id: &PluginId,
) -> LauncherResult<Option<CheckpointSummary>> {
    let row: Option<(String, i64, String, String)> = conn
        .query_row(
            "SELECT from_version, data_version, captured_at, payload_json
             FROM plugin_data_checkpoints WHERE plugin_id = ?1
             ORDER BY captured_at DESC, id DESC LIMIT 1",
            params![plugin_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(db_error)?;
    Ok(
        row.map(|(from_version, data_version, captured_at, payload)| {
            let entries: Vec<StorageRow> = serde_json::from_str(&payload).unwrap_or_default();
            CheckpointSummary {
                from_version,
                data_version: data_version.max(0) as u32,
                captured_at,
                entry_count: entries.len(),
            }
        }),
    )
}

/// Put a plugin's state back to its most recent checkpoint.
///
/// Returns the number of entries restored, or `None` when there is nothing to
/// restore — which the caller must report as "no rollback available" rather
/// than implying the old state came back.
pub fn restore_latest_checkpoint(
    conn: &Connection,
    plugin_id: &PluginId,
) -> LauncherResult<Option<usize>> {
    let payload: Option<String> = conn
        .query_row(
            "SELECT payload_json FROM plugin_data_checkpoints WHERE plugin_id = ?1
             ORDER BY captured_at DESC, id DESC LIMIT 1",
            params![plugin_id.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_error)?;
    let Some(payload) = payload else {
        return Ok(None);
    };
    let entries: Vec<StorageRow> = serde_json::from_str(&payload).map_err(db_error)?;

    // One transaction, because the first statement deletes everything the
    // plugin currently has. Failing partway through an un-transacted restore
    // would leave the user with neither their current settings nor the
    // checkpoint they were trying to get back to.
    let tx = conn.unchecked_transaction().map_err(db_error)?;
    tx.execute(
        "DELETE FROM plugin_storage WHERE plugin_id = ?1",
        params![plugin_id.as_str()],
    )
    .map_err(db_error)?;
    for entry in &entries {
        let text = serde_json::to_string(&entry.value).map_err(db_error)?;
        tx.execute(
            "INSERT OR REPLACE INTO plugin_storage
                 (plugin_id, kind, instance_id, key, value_json)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                plugin_id.as_str(),
                entry.kind,
                entry.instance_id,
                entry.key,
                text
            ],
        )
        .map_err(db_error)?;
    }
    tx.commit().map_err(db_error)?;
    Ok(Some(entries.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tempfile::TempDir;

    fn installed_plugin() -> (TempDir, Connection, PluginId) {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("local_state.db");
        crate::db::init_local_state_db(&path).unwrap();
        let conn = crate::db::local_state_connection(&path).unwrap();
        let plugin_id = PluginId::parse("acme.trust").unwrap();
        conn.execute(
            "INSERT INTO plugin_installs
                 (plugin_id, version, manifest_json, source_kind, install_dir,
                  installed_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                plugin_id.as_str(),
                "1.0.0",
                "{}",
                "package",
                "plugin",
                "now",
                "now"
            ],
        )
        .unwrap();
        (temp, conn, plugin_id)
    }

    fn key(id: &str) -> PublicKey {
        PublicKey {
            id: id.into(),
            algorithm: "ed25519".into(),
            public_key: format!("{id}-public-key"),
        }
    }

    #[test]
    fn replacing_trust_preserves_the_highest_sequence_already_seen() {
        let (_temp, conn, plugin_id) = installed_plugin();
        let first_key = key("first");
        let second_key = key("second");

        put_trust(
            &conn,
            &plugin_id,
            "https://updates.example.com/first.json",
            &[first_key],
        )
        .unwrap();
        record_check(&conn, &plugin_id, 9, "first-check", "available").unwrap();

        // Key rotation changes who may sign future documents, but must not make
        // an already rejected old document acceptable again.
        put_trust(
            &conn,
            &plugin_id,
            "https://updates.example.com/second.json",
            std::slice::from_ref(&second_key),
        )
        .unwrap();

        let trust = get_trust(&conn, &plugin_id).unwrap().unwrap();
        assert_eq!(trust.url, "https://updates.example.com/second.json");
        assert_eq!(trust.keys, vec![second_key]);
        assert_eq!(trust.highest_sequence, 9);
    }

    #[test]
    fn recording_an_older_check_never_lowers_the_replay_floor() {
        let (_temp, conn, plugin_id) = installed_plugin();
        let trusted_key = key("publisher");
        put_trust(
            &conn,
            &plugin_id,
            "https://updates.example.com/update.json",
            &[trusted_key],
        )
        .unwrap();

        record_check(&conn, &plugin_id, 11, "newer-check", "up to date").unwrap();
        record_check(&conn, &plugin_id, 4, "older-check", "stale response").unwrap();

        let trust = get_trust(&conn, &plugin_id).unwrap().unwrap();
        assert_eq!(trust.highest_sequence, 11);
        assert_eq!(trust.last_checked_at.as_deref(), Some("older-check"));
        assert_eq!(trust.last_result.as_deref(), Some("stale response"));
    }
}
