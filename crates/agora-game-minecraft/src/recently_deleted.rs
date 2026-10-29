//! "Recently deleted": Agora's own recoverable holding area for deleted instances.
//!
//! Deleting an instance does not destroy it. The instance folder is renamed
//! (same volume, atomic) into `<data>/recently-deleted/<entry-id>/instance`,
//! next to an `entry.json` that records the instance's database rows
//! (`user_instances` plus everything that cascades from it: import provenance,
//! pack file inventory, launch history, update cache). Restoring puts the
//! folder and those rows back, so the instance behaves exactly as before.
//!
//! A cleanup pass hands entries older than [`RETENTION_DAYS`], or beyond
//! [`SIZE_BUDGET_BYTES`] in total, to the OS trash through the adapter's
//! [`TrashFn`]. Every step fails closed: an error leaves the entry (or the
//! live instance) exactly where it was.
//!
//! Links (symlinks, junctions) inside an instance are never followed: sizes
//! skip them, and before an entry goes to the OS trash the links themselves
//! are removed so no tool downstream can walk out of the data root through
//! them.

use crate::instance_service::TrashFn;
use agora_core::app_paths::validate_path_component;
use agora_core::error::{LauncherError, LauncherResult};
use rusqlite::types::Value as SqlValue;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::path::{Path, PathBuf};

/// How long an entry stays restorable. A month is long enough to notice a
/// mistake after a holiday, short enough that deleted modpacks (often several
/// GB) do not silently eat the disk.
pub const RETENTION_DAYS: i64 = 30;

/// Total size of all entries before the oldest are sent to the OS trash early.
pub const SIZE_BUDGET_BYTES: u64 = 20 * 1024 * 1024 * 1024;

const ENTRY_FILE: &str = "entry.json";
const INSTANCE_SUBDIR: &str = "instance";
const ENTRY_VERSION: u32 = 1;

/// One restorable entry, as shown in My Instances.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecentlyDeletedEntry {
    pub entry_id: String,
    pub instance_id: String,
    pub name: String,
    pub minecraft_version: String,
    pub loader: String,
    /// Launcher the instance was imported from, when it was.
    pub import_source: Option<String>,
    pub deleted_at: String,
    /// When the cleanup pass will send it to the OS trash.
    pub expires_at: String,
    pub size_bytes: u64,
}

/// Result of a restore.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RestoreOutcome {
    pub instance_id: String,
    pub original_instance_id: String,
    /// True when the original id was taken and the instance came back under a
    /// different one (and a "(restored)" suffix on its name).
    pub renamed: bool,
}

/// Result of a cleanup pass.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CleanupSummary {
    pub purged: usize,
    /// Entries that should have been purged but could not be; left in place.
    pub failed: usize,
    pub kept: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TableDump {
    table: String,
    columns: Vec<String>,
    #[serde(default)]
    via: Option<LinkVia>,
    rows: Vec<Vec<Json>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredEntry {
    version: u32,
    #[serde(flatten)]
    info: RecentlyDeletedEntry,
    db: Vec<TableDump>,
}

fn err(code: &str, message: impl Into<String>) -> LauncherError {
    LauncherError::Generic {
        code: code.into(),
        message: message.into(),
    }
}

fn io_err(code: &str, what: &str, error: std::io::Error) -> LauncherError {
    err(code, format!("{what}: {error}"))
}

// ---------------------------------------------------------------------------
// Link-safe filesystem helpers
// ---------------------------------------------------------------------------

fn is_link(metadata: &std::fs::Metadata) -> bool {
    // On Windows std reports junctions and directory symlinks as symlinks.
    metadata.file_type().is_symlink()
}

fn remove_link(path: &Path, metadata: &std::fs::Metadata) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileTypeExt;
        if metadata.file_type().is_symlink_dir() {
            return std::fs::remove_dir(path);
        }
    }
    let _ = metadata;
    std::fs::remove_file(path)
}

/// Total bytes of regular files, never descending into links.
fn tree_size(root: &Path) -> std::io::Result<u64> {
    let mut total = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let metadata = std::fs::symlink_metadata(entry.path())?;
            if is_link(&metadata) {
                continue;
            }
            if metadata.is_dir() {
                stack.push(entry.path());
            } else {
                total = total.saturating_add(metadata.len());
            }
        }
    }
    Ok(total)
}

/// A link taken out of a tree, with what is needed to put it back.
struct RemovedLink {
    path: PathBuf,
    target: PathBuf,
    is_dir: bool,
    junction: bool,
}

fn recreate_link(link: &RemovedLink) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        if link.junction {
            return junction::create(&link.target, &link.path);
        }
        if link.is_dir {
            return std::os::windows::fs::symlink_dir(&link.target, &link.path);
        }
        std::os::windows::fs::symlink_file(&link.target, &link.path)
    }
    #[cfg(unix)]
    {
        let _ = (link.is_dir, link.junction);
        std::os::unix::fs::symlink(&link.target, &link.path)
    }
}

/// Put removed links back; reports how many could not be recreated.
fn restore_links(links: &[RemovedLink]) -> usize {
    links.iter().filter(|l| recreate_link(l).is_err()).count()
}

/// Remove every link under `root` (the links only, never their targets).
/// If anything fails, the links already removed are put back.
fn detach_links(root: &Path) -> std::io::Result<Vec<RemovedLink>> {
    let mut removed: Vec<RemovedLink> = Vec::new();
    let result = (|| -> std::io::Result<()> {
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir)? {
                let entry = entry?;
                let path = entry.path();
                let metadata = std::fs::symlink_metadata(&path)?;
                if is_link(&metadata) {
                    #[cfg(windows)]
                    let (target, junction, is_dir) = {
                        use std::os::windows::fs::FileTypeExt;
                        let junction = junction::exists(&path).unwrap_or(false);
                        let target = if junction {
                            junction::get_target(&path)?
                        } else {
                            std::fs::read_link(&path)?
                        };
                        (target, junction, metadata.file_type().is_symlink_dir())
                    };
                    #[cfg(unix)]
                    let (target, junction, is_dir) = (std::fs::read_link(&path)?, false, false);
                    remove_link(&path, &metadata)?;
                    removed.push(RemovedLink {
                        path,
                        target,
                        is_dir,
                        junction,
                    });
                } else if metadata.is_dir() {
                    stack.push(path);
                }
            }
        }
        Ok(())
    })();
    match result {
        Ok(()) => Ok(removed),
        Err(error) => {
            restore_links(&removed);
            Err(error)
        }
    }
}

/// True when `path` exists and is a plain directory (not a link to one).
fn is_plain_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|m| !is_link(&m) && m.is_dir())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Database state capture / replay
// ---------------------------------------------------------------------------

fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn db_err(error: impl std::fmt::Display) -> LauncherError {
    err("ERR_LOCAL_STATE_FAILED", error.to_string())
}

/// Tables that hold permissions rather than data about an instance. Their rows
/// are captured (so Restore under the original id is exact) but are never
/// replayed onto a *different* id: a grant must not carry over to another
/// instance.
const PERMISSION_TABLES: &[&str] = &["mcp_approval_grants"];

/// How a table is tied to an instance: directly by an `instance_id` column, or
/// through a foreign key to a table that is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LinkVia {
    pub col: String,
    pub parent: String,
    pub parent_col: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TableSpec {
    pub table: String,
    pub columns: Vec<String>,
    pub via: Option<LinkVia>,
}

impl TableSpec {
    fn filter(&self) -> String {
        match &self.via {
            None => "instance_id = ?1".to_string(),
            Some(v) => format!(
                "{} IN (SELECT {} FROM {} WHERE instance_id = ?1)",
                quote_ident(&v.col),
                quote_ident(&v.parent_col),
                quote_ident(&v.parent)
            ),
        }
    }
}

/// Every table whose rows belong to an instance, enumerated from the schema:
/// each table with an `instance_id` column, then tables that point at one of
/// those through a foreign key (e.g. survival mods). Parents come first.
/// Anything tied to an instance in a way this cannot express makes delete fail
/// closed rather than leave rows behind.
pub(crate) fn instance_tables(conn: &Connection) -> LauncherResult<Vec<TableSpec>> {
    struct Fk {
        parent: String,
        from: String,
        to: Option<String>,
    }
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .map_err(db_err)?;
    let names: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .map_err(db_err)?
        .collect::<Result<_, _>>()
        .map_err(db_err)?;
    let mut columns: std::collections::HashMap<String, Vec<String>> = Default::default();
    let mut fks: std::collections::HashMap<String, Vec<Fk>> = Default::default();
    for table in &names {
        let mut info = conn
            .prepare(&format!("PRAGMA table_info({})", quote_ident(table)))
            .map_err(db_err)?;
        let cols: Vec<String> = info
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(db_err)?
            .collect::<Result<_, _>>()
            .map_err(db_err)?;
        columns.insert(table.clone(), cols);
        let mut fk = conn
            .prepare(&format!("PRAGMA foreign_key_list({})", quote_ident(table)))
            .map_err(db_err)?;
        let list: Vec<Fk> = fk
            .query_map([], |row| {
                Ok(Fk {
                    parent: row.get(2)?,
                    from: row.get(3)?,
                    to: row.get(4)?,
                })
            })
            .map_err(db_err)?
            .collect::<Result<_, _>>()
            .map_err(db_err)?;
        fks.insert(table.clone(), list);
    }
    let direct: Vec<&String> = names
        .iter()
        .filter(|t| columns[*t].iter().any(|c| c == "instance_id"))
        .collect();
    if !direct.iter().any(|t| *t == "user_instances") {
        return Err(err(
            "ERR_INSTANCE_DELETE",
            "user_instances has no instance_id.",
        ));
    }
    let mut placed: Vec<TableSpec> = vec![TableSpec {
        table: "user_instances".into(),
        columns: columns["user_instances"].clone(),
        via: None,
    }];
    let is_placed = |placed: &Vec<TableSpec>, t: &str| placed.iter().any(|s| s.table == t);
    // Direct tables, parents before children.
    let mut remaining: Vec<&String> = direct
        .iter()
        .copied()
        .filter(|t| *t != "user_instances")
        .collect();
    while !remaining.is_empty() {
        let ready = remaining.iter().position(|t| {
            fks[*t]
                .iter()
                .all(|f| is_placed(&placed, &f.parent) || !direct.iter().any(|d| **d == f.parent))
        });
        let t = remaining.remove(ready.unwrap_or(0));
        placed.push(TableSpec {
            table: t.clone(),
            columns: columns[t].clone(),
            via: None,
        });
    }
    // Tables without instance_id that point at an instance table.
    let direct_count = placed.len();
    loop {
        let mut grew = false;
        for t in &names {
            if is_placed(&placed, t) {
                continue;
            }
            let Some(f) = fks[t]
                .iter()
                .find(|f| placed[..direct_count].iter().any(|s| s.table == f.parent))
            else {
                continue;
            };
            let parent_col = f.to.clone().ok_or_else(|| {
                err(
                    "ERR_INSTANCE_DELETE",
                    format!("Table '{t}' references an instance table without naming a column."),
                )
            })?;
            placed.push(TableSpec {
                table: t.clone(),
                columns: columns[t].clone(),
                via: Some(LinkVia {
                    col: f.from.clone(),
                    parent: f.parent.clone(),
                    parent_col,
                }),
            });
            grew = true;
        }
        if !grew {
            break;
        }
    }
    // Anything still pointing at a captured table is a shape we cannot restore.
    for t in &names {
        if !is_placed(&placed, t) && fks[t].iter().any(|f| is_placed(&placed, &f.parent)) {
            return Err(err(
                "ERR_INSTANCE_DELETE",
                format!(
                    "Table '{t}' depends on instance data in a way that cannot be restored; refusing to delete."
                ),
            ));
        }
    }
    Ok(placed)
}

fn sql_to_json(value: SqlValue) -> LauncherResult<Json> {
    Ok(match value {
        SqlValue::Null => Json::Null,
        SqlValue::Integer(i) => Json::from(i),
        SqlValue::Real(f) => serde_json::Number::from_f64(f)
            .map(Json::Number)
            .ok_or_else(|| err("ERR_INSTANCE_DELETE", "A stored number is not finite."))?,
        SqlValue::Text(s) => Json::String(s),
        SqlValue::Blob(bytes) => {
            let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            serde_json::json!({ "$blob": hex })
        }
    })
}

fn json_to_sql(value: &Json) -> LauncherResult<SqlValue> {
    Ok(match value {
        Json::Null => SqlValue::Null,
        Json::Number(n) => match n.as_i64() {
            Some(i) => SqlValue::Integer(i),
            None => SqlValue::Real(
                n.as_f64()
                    .ok_or_else(|| err("ERR_INSTANCE_RESTORE", "Unreadable number in entry."))?,
            ),
        },
        Json::String(s) => SqlValue::Text(s.clone()),
        Json::Object(map) if map.len() == 1 => {
            let hex = map
                .get("$blob")
                .and_then(Json::as_str)
                .ok_or_else(|| err("ERR_INSTANCE_RESTORE", "Unreadable value in entry."))?;
            if hex.len() % 2 != 0 {
                return Err(err("ERR_INSTANCE_RESTORE", "Unreadable blob in entry."));
            }
            let mut bytes = Vec::with_capacity(hex.len() / 2);
            for i in (0..hex.len()).step_by(2) {
                bytes.push(
                    hex.get(i..i + 2)
                        .and_then(|h| u8::from_str_radix(h, 16).ok())
                        .ok_or_else(|| err("ERR_INSTANCE_RESTORE", "Unreadable blob in entry."))?,
                );
            }
            SqlValue::Blob(bytes)
        }
        _ => {
            return Err(err(
                "ERR_INSTANCE_RESTORE",
                "Unexpected value type in entry.",
            ))
        }
    })
}

pub(crate) fn capture_db_state(
    conn: &Connection,
    instance_id: &str,
) -> LauncherResult<Vec<TableDump>> {
    let mut dumps = Vec::new();
    for spec in instance_tables(conn)? {
        let sql = format!(
            "SELECT * FROM {} WHERE {}",
            quote_ident(&spec.table),
            spec.filter()
        );
        let mut stmt = conn.prepare(&sql).map_err(db_err)?;
        let width = spec.columns.len();
        let mut rows = Vec::new();
        let mut cursor = stmt.query([instance_id]).map_err(db_err)?;
        while let Some(row) = cursor.next().map_err(db_err)? {
            let mut values = Vec::with_capacity(width);
            for i in 0..width {
                values.push(sql_to_json(row.get::<_, SqlValue>(i).map_err(db_err)?)?);
            }
            rows.push(values);
        }
        dumps.push(TableDump {
            table: spec.table,
            columns: spec.columns,
            via: spec.via,
            rows,
        });
    }
    Ok(dumps)
}

/// Remove every row belonging to the instance, children first, in one
/// transaction: all or nothing.
pub(crate) fn delete_instance_rows(conn: &Connection, instance_id: &str) -> LauncherResult<()> {
    let specs = instance_tables(conn)?;
    let tx = conn.unchecked_transaction().map_err(db_err)?;
    for spec in specs.iter().rev() {
        tx.execute(
            &format!(
                "DELETE FROM {} WHERE {}",
                quote_ident(&spec.table),
                spec.filter()
            ),
            [instance_id],
        )
        .map_err(db_err)?;
    }
    tx.commit().map_err(db_err)
}

/// Check an entry's captured rows against the live schema and the entry's own
/// identity before anything is written: known tables and columns only, the
/// same link shape, well-formed values, and every row belongs to this
/// instance. The entry is data, never instructions.
fn validate_dump(conn: &Connection, dumps: &[TableDump], original_id: &str) -> LauncherResult<()> {
    let specs = instance_tables(conn)?;
    let mut saw_instance_row = false;
    for dump in dumps {
        let spec = specs
            .iter()
            .find(|s| s.table == dump.table)
            .ok_or_else(|| {
                err(
                    "ERR_INSTANCE_RESTORE",
                    format!("Entry mentions unknown table '{}'.", dump.table),
                )
            })?;
        if spec.via != dump.via {
            return Err(err(
                "ERR_INSTANCE_RESTORE",
                "Entry table links do not match the schema.",
            ));
        }
        if dump.columns.iter().any(|c| !spec.columns.contains(c)) {
            return Err(err(
                "ERR_INSTANCE_RESTORE",
                format!("Entry has unknown columns for '{}'.", dump.table),
            ));
        }
        let id_col = dump.columns.iter().position(|c| c == "instance_id");
        for row in &dump.rows {
            if row.len() != dump.columns.len() {
                return Err(err("ERR_INSTANCE_RESTORE", "Entry rows are malformed."));
            }
            for value in row {
                json_to_sql(value)?;
            }
            if let Some(i) = id_col {
                if row[i].as_str() != Some(original_id) {
                    return Err(err(
                        "ERR_INSTANCE_RESTORE",
                        "Entry contains rows for a different instance.",
                    ));
                }
            }
        }
        if dump.table == "user_instances" {
            if dump.rows.len() != 1 {
                return Err(err(
                    "ERR_INSTANCE_RESTORE",
                    "Entry must hold exactly one instance row.",
                ));
            }
            saw_instance_row = true;
        }
    }
    if !saw_instance_row {
        return Err(err("ERR_INSTANCE_RESTORE", "Entry has no instance row."));
    }
    Ok(())
}

/// Re-insert captured rows in one transaction (so a failure inserts nothing),
/// optionally under a different instance id. Permission rows are skipped when
/// the id changes.
pub(crate) fn replay_db_state(
    conn: &mut Connection,
    dumps: &[TableDump],
    original_id: &str,
    new_id: &str,
    new_name: Option<&str>,
) -> LauncherResult<()> {
    validate_dump(conn, dumps, original_id)?;
    let renamed = new_id != original_id;
    let tx = conn.transaction().map_err(db_err)?;
    for dump in dumps {
        if dump.rows.is_empty() || (renamed && PERMISSION_TABLES.contains(&dump.table.as_str())) {
            continue;
        }
        let id_col = dump.columns.iter().position(|c| c == "instance_id");
        let name_col = (dump.table == "user_instances")
            .then(|| dump.columns.iter().position(|c| c == "name"))
            .flatten();
        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            quote_ident(&dump.table),
            dump.columns
                .iter()
                .map(|c| quote_ident(c))
                .collect::<Vec<_>>()
                .join(", "),
            (1..=dump.columns.len())
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let mut stmt = tx.prepare(&sql).map_err(|e| {
            err(
                "ERR_INSTANCE_RESTORE",
                format!("Cannot restore '{}': {e}", dump.table),
            )
        })?;
        for row in &dump.rows {
            let mut params = Vec::with_capacity(row.len());
            for (i, value) in row.iter().enumerate() {
                if Some(i) == id_col {
                    params.push(SqlValue::Text(new_id.to_string()));
                } else if Some(i) == name_col && new_name.is_some() {
                    params.push(SqlValue::Text(new_name.unwrap_or_default().to_string()));
                } else {
                    params.push(json_to_sql(value)?);
                }
            }
            stmt.execute(rusqlite::params_from_iter(params))
                .map_err(|e| {
                    err(
                        "ERR_INSTANCE_RESTORE",
                        format!("Cannot restore '{}': {e}", dump.table),
                    )
                })?;
        }
    }
    tx.commit().map_err(db_err)
}

// ---------------------------------------------------------------------------
// Entry storage
// ---------------------------------------------------------------------------

fn entry_dir(root: &Path, entry_id: &str) -> LauncherResult<PathBuf> {
    validate_path_component(entry_id)?;
    Ok(root.join(entry_id))
}

fn read_entry(root: &Path, entry_id: &str) -> LauncherResult<StoredEntry> {
    let stored = read_entry_file(root, entry_id)?;
    let dir = entry_dir(root, entry_id)?;
    if !is_plain_dir(&dir.join(INSTANCE_SUBDIR)) {
        return Err(err(
            "ERR_INSTANCE_RESTORE",
            "The deleted instance's files are missing from this entry.",
        ));
    }
    Ok(stored)
}

/// The entry's record, whether or not its files are still inside.
fn read_entry_file(root: &Path, entry_id: &str) -> LauncherResult<StoredEntry> {
    let dir = entry_dir(root, entry_id)?;
    let text = std::fs::read_to_string(dir.join(ENTRY_FILE))
        .map_err(|e| io_err("ERR_INSTANCE_RESTORE", "Cannot read the deleted entry", e))?;
    let stored: StoredEntry = serde_json::from_str(&text)
        .map_err(|e| err("ERR_INSTANCE_RESTORE", format!("Entry is unreadable: {e}")))?;
    if stored.version != ENTRY_VERSION || stored.info.entry_id != entry_id {
        return Err(err(
            "ERR_INSTANCE_RESTORE",
            "Entry is from a different version or does not match its folder.",
        ));
    }
    validate_path_component(&stored.info.instance_id)?;
    Ok(stored)
}

fn expires_at(deleted_at: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(deleted_at)
        .map(|t| (t + chrono::Duration::days(RETENTION_DAYS)).to_rfc3339())
        .unwrap_or_default()
}

/// Move a live instance directory into a new entry. Called with the instance
/// lock held and the row still present; deletes the row (and its cascade)
/// last. On any failure nothing is changed.
pub(crate) fn stash_instance(
    root: &Path,
    conn: &Connection,
    row: &agora_core::models::InstanceRow,
    instance_dir: &Path,
    now: chrono::DateTime<chrono::Utc>,
) -> LauncherResult<RecentlyDeletedEntry> {
    let plain = is_plain_dir(instance_dir);
    if !plain {
        // A link in the instance's own slot: never move or follow it.
        return Err(err(
            "ERR_INSTANCE_DELETE",
            "The instance folder is a link rather than a real folder; nothing was deleted.",
        ));
    }
    let size_bytes = tree_size(instance_dir)
        .map_err(|e| io_err("ERR_INSTANCE_DELETE", "Cannot measure the instance", e))?;
    let entry_id = format!("{}-{}", now.format("%Y%m%dT%H%M%SZ"), uuid::Uuid::new_v4());
    let deleted_at = now.to_rfc3339();
    let info = RecentlyDeletedEntry {
        entry_id: entry_id.clone(),
        instance_id: row.instance_id.clone(),
        name: row.name.clone(),
        minecraft_version: row.minecraft_version.clone(),
        loader: row.loader.clone(),
        import_source: row.import_source.clone(),
        expires_at: expires_at(&deleted_at),
        deleted_at,
        size_bytes,
    };
    let stored = StoredEntry {
        version: ENTRY_VERSION,
        info: info.clone(),
        db: capture_db_state(conn, &row.instance_id)?,
    };
    let bytes = serde_json::to_vec_pretty(&stored)
        .map_err(|e| err("ERR_INSTANCE_DELETE", format!("Cannot record entry: {e}")))?;

    let dir = entry_dir(root, &entry_id)?;
    let abandon = |dir: &Path| {
        let _ = std::fs::remove_dir_all(dir);
    };
    std::fs::create_dir_all(&dir)
        .map_err(|e| io_err("ERR_INSTANCE_DELETE", "Cannot create Recently deleted", e))?;
    let tmp = dir.join("entry.json.tmp");
    if let Err(e) =
        std::fs::write(&tmp, &bytes).and_then(|_| std::fs::rename(&tmp, dir.join(ENTRY_FILE)))
    {
        abandon(&dir);
        return Err(io_err("ERR_INSTANCE_DELETE", "Cannot record entry", e));
    }
    let target = dir.join(INSTANCE_SUBDIR);
    if let Err(e) = std::fs::rename(instance_dir, &target) {
        abandon(&dir);
        return Err(err(
            "ERR_INSTANCE_DELETE",
            format!(
                "Could not move the instance to Recently deleted (is the game running?): {e}. \
                 Nothing was deleted."
            ),
        ));
    }
    if let Err(e) = delete_instance_rows(conn, &row.instance_id) {
        match std::fs::rename(&target, instance_dir) {
            Ok(()) => abandon(&dir),
            Err(_) => {
                // The files could not go back; keep the entry so they stay
                // reachable and restorable.
            }
        }
        return Err(e);
    }
    Ok(info)
}

/// All valid entries, newest first. Unreadable or incomplete entries are
/// skipped (and left on disk).
pub fn list_entries(root: &Path) -> Vec<RecentlyDeletedEntry> {
    let Ok(read) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out: Vec<RecentlyDeletedEntry> = read
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .filter_map(|id| read_entry(root, &id).ok())
        .map(|mut s| {
            s.info.expires_at = expires_at(&s.info.deleted_at);
            s.info
        })
        .collect();
    out.sort_by(|a, b| b.deleted_at.cmp(&a.deleted_at));
    out
}

/// Pick the id to restore under: the original when free, else a free
/// `<id>-restored[-n]`.
pub(crate) fn choose_restore_id(
    original: &str,
    is_free: &dyn Fn(&str) -> bool,
) -> LauncherResult<(String, bool)> {
    if is_free(original) {
        return Ok((original.to_string(), false));
    }
    for n in 1..=100 {
        let candidate = if n == 1 {
            format!("{original}-restored")
        } else {
            format!("{original}-restored-{n}")
        };
        if validate_path_component(&candidate).is_ok() && is_free(&candidate) {
            return Ok((candidate, true));
        }
    }
    Err(err(
        "ERR_INSTANCE_RESTORE",
        "Could not find a free name to restore this instance under.",
    ))
}

/// Restore an entry. Caller holds the lock for `new_id`.
pub(crate) fn restore_entry(
    root: &Path,
    entry_id: &str,
    conn: &mut Connection,
    instances_root: &Path,
    is_free: &dyn Fn(&str) -> bool,
) -> LauncherResult<RestoreOutcome> {
    let stored = read_entry(root, entry_id)?;
    let original = stored.info.instance_id.clone();
    validate_path_component(&original)?;
    let (new_id, renamed) = choose_restore_id(&original, is_free)?;
    let source = entry_dir(root, entry_id)?.join(INSTANCE_SUBDIR);
    let dest = instances_root.join(&new_id);
    if std::fs::symlink_metadata(&dest).is_ok() {
        return Err(err(
            "ERR_INSTANCE_RESTORE",
            "A folder already exists where this instance would be restored.",
        ));
    }
    std::fs::create_dir_all(instances_root)
        .map_err(|e| io_err("ERR_INSTANCE_RESTORE", "Cannot prepare instances folder", e))?;

    let new_name = renamed.then(|| format!("{} (restored)", stored.info.name));
    let manifest_path = source.join("instance_manifest.json");
    let original_manifest = if renamed {
        std::fs::read(&manifest_path).ok()
    } else {
        None
    };
    // Rewrite the manifest id/name in the entry first only when renaming; the
    // original bytes are put back on any failure.
    if renamed {
        if let Some(bytes) = &original_manifest {
            let mut value: Json = serde_json::from_slice(bytes)
                .map_err(|e| err("ERR_INSTANCE_RESTORE", format!("Manifest unreadable: {e}")))?;
            if let Some(object) = value.as_object_mut() {
                object.insert("instance_id".into(), Json::String(new_id.clone()));
                if let Some(name) = &new_name {
                    object.insert("name".into(), Json::String(name.clone()));
                }
            }
            let out = serde_json::to_vec_pretty(&value)
                .map_err(|e| err("ERR_INSTANCE_RESTORE", e.to_string()))?;
            std::fs::write(&manifest_path, out)
                .map_err(|e| io_err("ERR_INSTANCE_RESTORE", "Cannot update manifest", e))?;
        }
    }
    let undo_manifest = |bytes: &Option<Vec<u8>>| {
        if let Some(bytes) = bytes {
            let _ = std::fs::write(&manifest_path, bytes);
        }
    };
    if let Err(e) = std::fs::rename(&source, &dest) {
        undo_manifest(&original_manifest);
        return Err(io_err(
            "ERR_INSTANCE_RESTORE",
            "Cannot move the files back",
            e,
        ));
    }
    if let Err(e) = replay_db_state(conn, &stored.db, &original, &new_id, new_name.as_deref()) {
        if std::fs::rename(&dest, &source).is_ok() {
            undo_manifest(&original_manifest);
        }
        return Err(e);
    }
    // Committed. Tidy the (now empty) entry; if that fails the entry is
    // skipped by listing because its instance folder is gone.
    let dir = entry_dir(root, entry_id)?;
    let _ = std::fs::remove_file(dir.join(ENTRY_FILE));
    let _ = std::fs::remove_dir(&dir);
    Ok(RestoreOutcome {
        instance_id: new_id,
        original_instance_id: original,
        renamed,
    })
}

/// Send one entry to the OS trash. The links inside are detached first; any
/// failure leaves the entry in place.
pub fn purge_entry(root: &Path, entry_id: &str, trash: &TrashFn) -> LauncherResult<()> {
    let dir = entry_dir(root, entry_id)?;
    // Refuse anything that is not a plain folder directly under the root.
    if !is_plain_dir(&dir) {
        return Err(err(
            "ERR_INSTANCE_DELETE",
            "Entry is not a plain folder; leaving it alone.",
        ));
    }
    let links = detach_links(&dir).map_err(|e| {
        io_err(
            "ERR_INSTANCE_DELETE",
            "Cannot prepare entry for the trash",
            e,
        )
    })?;
    if let Err(error) = trash(&dir) {
        // Leave the entry exactly as it was: put the links back.
        let lost = restore_links(&links);
        if lost > 0 {
            return Err(err(
                "ERR_INSTANCE_DELETE",
                format!("{error} ({lost} link(s) in the entry could not be put back.)"),
            ));
        }
        return Err(error);
    }
    Ok(())
}

/// Purge entries older than the retention window, then the oldest until the
/// total fits the size budget. Also removes empty shells left by a restore
/// whose tidy-up failed. Never errors as a whole: failures are counted.
pub fn cleanup(root: &Path, now: chrono::DateTime<chrono::Utc>, trash: &TrashFn) -> CleanupSummary {
    cleanup_with_budget(root, now, trash, SIZE_BUDGET_BYTES)
}

pub(crate) fn cleanup_with_budget(
    root: &Path,
    now: chrono::DateTime<chrono::Utc>,
    trash: &TrashFn,
    budget: u64,
) -> CleanupSummary {
    let mut summary = CleanupSummary::default();
    // Truly empty folders hold nothing; `remove_dir` refuses anything else.
    if let Ok(read) = std::fs::read_dir(root) {
        for item in read.filter_map(Result::ok) {
            let path = item.path();
            if is_plain_dir(&path) {
                let _ = std::fs::remove_dir(&path);
            }
        }
    }
    let mut entries = list_entries(root);
    entries.sort_by(|a, b| a.deleted_at.cmp(&b.deleted_at)); // oldest first
    let mut total: u64 = entries.iter().map(|e| e.size_bytes).sum();
    for entry in entries {
        let expired = chrono::DateTime::parse_from_rfc3339(&entry.expires_at)
            .map(|t| t <= now)
            .unwrap_or(false);
        let over_budget = total > budget;
        if !(expired || over_budget) {
            summary.kept += 1;
            continue;
        }
        match purge_entry(root, &entry.entry_id, trash) {
            Ok(()) => {
                summary.purged += 1;
                total = total.saturating_sub(entry.size_bytes);
            }
            Err(_) => {
                summary.failed += 1;
            }
        }
    }
    summary
}

/// Repair what a crash can leave between the steps of delete and restore.
/// Returns how many entries were repaired. Each case is recognised by the
/// instance's own `created_at`, so a different instance that merely reuses the
/// id is never touched.
///
/// - Delete moved the folder but died before removing the rows: finish the
///   row removal (the entry holds everything).
/// - Restore moved the folder back but died before restoring the rows (same
///   id): replay them, then retire the entry.
/// - Restore committed but could not tidy the entry: retire the entry.
pub(crate) fn reconcile(root: &Path, conn: &mut Connection, instances_root: &Path) -> usize {
    let Ok(read) = std::fs::read_dir(root) else {
        return 0;
    };
    let mut repaired = 0;
    for item in read.filter_map(Result::ok) {
        let Ok(entry_id) = item.file_name().into_string() else {
            continue;
        };
        let Ok(stored) = read_entry_file(root, &entry_id) else {
            continue;
        };
        let id = stored.info.instance_id.clone();
        let Some(dumped_created) = stored
            .db
            .iter()
            .find(|d| d.table == "user_instances")
            .and_then(|d| {
                let col = d.columns.iter().position(|c| c == "created_at")?;
                d.rows.first()?.get(col)?.as_str().map(str::to_owned)
            })
        else {
            continue;
        };
        let live_created: Option<String> = conn
            .query_row(
                "SELECT created_at FROM user_instances WHERE instance_id = ?1",
                [&id],
                |row| row.get(0),
            )
            .ok();
        let same_instance = live_created.as_deref() == Some(dumped_created.as_str());
        let Ok(dir) = entry_dir(root, &entry_id) else {
            continue;
        };
        let has_files = is_plain_dir(&dir.join(INSTANCE_SUBDIR));
        let live_dir = instances_root.join(&id);
        if has_files && same_instance && std::fs::symlink_metadata(&live_dir).is_err() {
            if delete_instance_rows(conn, &id).is_ok() {
                repaired += 1;
            }
        } else if !has_files && live_created.is_none() && is_plain_dir(&live_dir) {
            if replay_db_state(conn, &stored.db, &id, &id, None).is_ok() {
                let _ = std::fs::remove_file(dir.join(ENTRY_FILE));
                let _ = std::fs::remove_dir(&dir);
                repaired += 1;
            }
        } else if !has_files && same_instance {
            let _ = std::fs::remove_file(dir.join(ENTRY_FILE));
            let _ = std::fs::remove_dir(&dir);
            repaired += 1;
        }
    }
    repaired
}
