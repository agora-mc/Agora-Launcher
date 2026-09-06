//! Local launch history — "did that change make startup worse?"
//!
//! Every launcher forgets what happened last time you played. Agora records a
//! row per launch so it can answer questions no one else can: whether the pack
//! got slower after you added ten mods, how often an instance actually crashes,
//! whether a config change helped.
//!
//! # Local only, deliberately
//!
//! There is no endpoint and there never will be. This lives in `local_state.db`
//! next to everything else the user owns, is deleted with the instance by the
//! foreign key, and is trimmed by [`prune_history`] so it cannot grow without
//! bound. The project's whole premise is a $0 server footprint; telemetry that
//! phoned home would contradict it.
//!
//! # What is actually measured
//!
//! `prep_ms` is Agora's own work before the process starts — materialising the
//! runtime, verifying artifacts, resolving Java. That is measured precisely,
//! because Agora is the one doing it.
//!
//! `duration_ms` is how long the session lasted. It is deliberately *not*
//! called "startup time": Minecraft emits no readiness signal Agora can trust
//! without parsing the game log for loader-specific markers, so claiming to
//! measure time-to-playable would be inventing a number. Trends in `prep_ms`
//! against `enabled_mod_count` are the honest version of the question, and a
//! crash a second after spawn is visible in `duration_ms` regardless.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

/// How a launch ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchResult {
    /// Exited normally.
    Ok,
    /// Non-zero exit or a crash signal.
    Crashed,
    /// The launcher never saw it finish — killed, or Agora closed first.
    Unknown,
}

impl LaunchResult {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Crashed => "crashed",
            Self::Unknown => "unknown",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "ok" => Self::Ok,
            "crashed" => Self::Crashed,
            _ => Self::Unknown,
        }
    }
}

/// One recorded launch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchRecord {
    pub id: i64,
    pub instance_id: String,
    pub started_at: String,
    /// Agora's own preparation time, in milliseconds.
    pub prep_ms: Option<i64>,
    /// Session length, in milliseconds. `None` while still running.
    pub duration_ms: Option<i64>,
    /// `None` while still running.
    pub outcome: Option<LaunchResult>,
    pub enabled_mod_count: i64,
    pub minecraft_version: String,
    pub loader: String,
    pub peak_memory_mb: Option<i64>,
}

/// Aggregate view over an instance's history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchStats {
    pub runs: usize,
    pub crashes: usize,
    /// Median Agora preparation time across runs that recorded one.
    pub median_prep_ms: Option<i64>,
    /// Median prep time of the most recent runs, against everything older —
    /// the pair a UI needs to say "startup got slower".
    pub recent_median_prep_ms: Option<i64>,
    pub earlier_median_prep_ms: Option<i64>,
    /// Enabled mod count on the most recent run, and on the oldest retained one.
    pub latest_mod_count: Option<i64>,
    pub earliest_mod_count: Option<i64>,
}

/// Rows kept per instance. Enough to see a trend, small enough to stay cheap.
pub const HISTORY_LIMIT: usize = 100;

/// Open a launch record. Returns the row id to close it with.
pub fn begin_launch(
    conn: &Connection,
    instance_id: &str,
    started_at: &str,
    enabled_mod_count: i64,
    minecraft_version: &str,
    loader: &str,
) -> anyhow::Result<i64> {
    conn.execute(
        "INSERT INTO launch_history
             (instance_id, started_at, enabled_mod_count, minecraft_version, loader)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            instance_id,
            started_at,
            enabled_mod_count,
            minecraft_version,
            loader
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Record how a launch turned out.
///
/// Never fails the launch itself: callers treat an error here as "we lost a
/// history row", which it is.
pub fn finish_launch(
    conn: &Connection,
    id: i64,
    prep_ms: Option<i64>,
    duration_ms: Option<i64>,
    outcome: LaunchResult,
    peak_memory_mb: Option<i64>,
) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE launch_history
            SET prep_ms = ?2, duration_ms = ?3, outcome = ?4, peak_memory_mb = ?5
          WHERE id = ?1",
        params![id, prep_ms, duration_ms, outcome.as_str(), peak_memory_mb],
    )?;
    Ok(())
}

/// Most recent launches first.
pub fn list_history(
    conn: &Connection,
    instance_id: &str,
    limit: usize,
) -> anyhow::Result<Vec<LaunchRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id, instance_id, started_at, prep_ms, duration_ms, outcome,
                enabled_mod_count, minecraft_version, loader, peak_memory_mb
           FROM launch_history
          WHERE instance_id = ?1
          ORDER BY started_at DESC, id DESC
          LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![instance_id, limit as i64], |row| {
        Ok(LaunchRecord {
            id: row.get(0)?,
            instance_id: row.get(1)?,
            started_at: row.get(2)?,
            prep_ms: row.get(3)?,
            duration_ms: row.get(4)?,
            outcome: row
                .get::<_, Option<String>>(5)?
                .map(|value| LaunchResult::parse(&value)),
            enabled_mod_count: row.get(6)?,
            minecraft_version: row.get(7)?,
            loader: row.get(8)?,
            peak_memory_mb: row.get(9)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Drop everything past [`HISTORY_LIMIT`] for one instance.
pub fn prune_history(conn: &Connection, instance_id: &str) -> anyhow::Result<usize> {
    let removed = conn.execute(
        "DELETE FROM launch_history
          WHERE instance_id = ?1
            AND id NOT IN (
                SELECT id FROM launch_history
                 WHERE instance_id = ?1
                 ORDER BY started_at DESC, id DESC
                 LIMIT ?2
            )",
        params![instance_id, HISTORY_LIMIT as i64],
    )?;
    Ok(removed)
}

fn median(mut values: Vec<i64>) -> Option<i64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let mid = values.len() / 2;
    // Even counts take the lower of the two middles rather than averaging:
    // these are observed timings, and reporting a value that was never
    // measured invites more precision than the data supports.
    Some(values[mid.saturating_sub(usize::from(values.len().is_multiple_of(2)))])
}

/// Summarize a history, newest first.
///
/// The recent/earlier split is what makes the numbers actionable: a single
/// median cannot say whether something got worse, and a raw last-vs-first
/// comparison is dominated by noise from one cold start.
pub fn summarize(records: &[LaunchRecord]) -> LaunchStats {
    let finished: Vec<&LaunchRecord> = records
        .iter()
        .filter(|record| record.outcome.is_some())
        .collect();
    let crashes = finished
        .iter()
        .filter(|record| record.outcome == Some(LaunchResult::Crashed))
        .count();

    let prep = |slice: &[&LaunchRecord]| -> Option<i64> {
        median(slice.iter().filter_map(|record| record.prep_ms).collect())
    };

    // Split the finished runs in half; with fewer than four there is no trend
    // worth claiming, so both halves stay empty.
    let (recent, earlier) = if finished.len() >= 4 {
        let half = finished.len() / 2;
        (&finished[..half], &finished[half..])
    } else {
        (&finished[..0], &finished[..0])
    };

    LaunchStats {
        runs: records.len(),
        crashes,
        median_prep_ms: prep(&finished),
        recent_median_prep_ms: prep(recent),
        earlier_median_prep_ms: prep(earlier),
        latest_mod_count: records.first().map(|record| record.enabled_mod_count),
        earliest_mod_count: records.last().map(|record| record.enabled_mod_count),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::run_migrations(&conn).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        conn.execute(
            "INSERT INTO user_instances
                (instance_id, name, minecraft_version, loader, loader_version, created_at)
             VALUES ('inst', 'Inst', '1.21', 'fabric', '0.15.0', '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn
    }

    fn record(conn: &Connection, at: &str, prep: i64, mods: i64, outcome: LaunchResult) -> i64 {
        let id = begin_launch(conn, "inst", at, mods, "1.21", "fabric").unwrap();
        finish_launch(conn, id, Some(prep), Some(60_000), outcome, None).unwrap();
        id
    }

    #[test]
    fn a_launch_round_trips() {
        let conn = conn();
        let id = begin_launch(&conn, "inst", "2026-01-01T00:00:00Z", 12, "1.21", "fabric").unwrap();
        // An open row is visible immediately — a session that never ends is
        // still a launch that happened.
        let open = list_history(&conn, "inst", 10).unwrap();
        assert_eq!(open.len(), 1);
        assert!(open[0].outcome.is_none());
        assert_eq!(open[0].enabled_mod_count, 12);

        finish_launch(
            &conn,
            id,
            Some(4200),
            Some(90_000),
            LaunchResult::Ok,
            Some(2048),
        )
        .unwrap();
        let done = list_history(&conn, "inst", 10).unwrap();
        assert_eq!(done[0].outcome, Some(LaunchResult::Ok));
        assert_eq!(done[0].prep_ms, Some(4200));
        assert_eq!(done[0].peak_memory_mb, Some(2048));
    }

    #[test]
    fn history_is_newest_first_and_scoped_to_the_instance() {
        let conn = conn();
        conn.execute(
            "INSERT INTO user_instances
                (instance_id, name, minecraft_version, loader, loader_version, created_at)
             VALUES ('other', 'Other', '1.21', 'fabric', '0.15.0', '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        record(&conn, "2026-01-01T00:00:00Z", 1000, 5, LaunchResult::Ok);
        record(&conn, "2026-03-01T00:00:00Z", 2000, 6, LaunchResult::Ok);
        begin_launch(&conn, "other", "2026-02-01T00:00:00Z", 99, "1.21", "fabric").unwrap();

        let rows = list_history(&conn, "inst", 10).unwrap();
        assert_eq!(rows.len(), 2, "the other instance's run must not appear");
        assert_eq!(rows[0].started_at, "2026-03-01T00:00:00Z");
    }

    #[test]
    fn deleting_an_instance_takes_its_history_with_it() {
        // The foreign key is the whole privacy story: history cannot outlive
        // the thing it describes.
        let conn = conn();
        record(&conn, "2026-01-01T00:00:00Z", 1000, 5, LaunchResult::Ok);
        conn.execute("DELETE FROM user_instances WHERE instance_id = 'inst'", [])
            .unwrap();
        assert!(list_history(&conn, "inst", 10).unwrap().is_empty());
    }

    #[test]
    fn pruning_keeps_the_newest_and_drops_the_rest() {
        let conn = conn();
        for index in 0..(HISTORY_LIMIT + 10) {
            record(
                &conn,
                &format!("2026-01-01T00:00:{:02}Z", index % 60),
                1000 + index as i64,
                5,
                LaunchResult::Ok,
            );
        }
        let removed = prune_history(&conn, "inst").unwrap();
        assert_eq!(removed, 10);
        assert_eq!(
            list_history(&conn, "inst", 1000).unwrap().len(),
            HISTORY_LIMIT
        );
        // Idempotent — a second sweep has nothing left to do.
        assert_eq!(prune_history(&conn, "inst").unwrap(), 0);
    }

    #[test]
    fn summarize_of_nothing_claims_nothing() {
        // The degenerate case: no history must not read as "0 ms, very fast".
        let stats = summarize(&[]);
        assert_eq!(stats.runs, 0);
        assert_eq!(stats.crashes, 0);
        assert!(stats.median_prep_ms.is_none());
        assert!(stats.recent_median_prep_ms.is_none());
        assert!(stats.latest_mod_count.is_none());
    }

    #[test]
    fn a_trend_is_only_claimed_once_there_is_enough_to_compare() {
        let conn = conn();
        for index in 0..3 {
            record(
                &conn,
                &format!("2026-01-0{}T00:00:00Z", index + 1),
                1000,
                5,
                LaunchResult::Ok,
            );
        }
        let stats = summarize(&list_history(&conn, "inst", 100).unwrap());
        assert!(
            stats.recent_median_prep_ms.is_none(),
            "three runs is not a trend"
        );
        assert_eq!(
            stats.median_prep_ms,
            Some(1000),
            "but a median is still fine"
        );
    }

    #[test]
    fn a_slowdown_shows_up_as_recent_worse_than_earlier() {
        let conn = conn();
        // Oldest four are fast, newest four are slow — the shape of "you added
        // mods and startup got worse".
        for (index, prep) in [1000, 1000, 1100, 1000, 5000, 5200, 5100, 5000]
            .into_iter()
            .enumerate()
        {
            record(
                &conn,
                &format!("2026-01-{:02}T00:00:00Z", index + 1),
                prep,
                if index < 4 { 10 } else { 40 },
                LaunchResult::Ok,
            );
        }
        let stats = summarize(&list_history(&conn, "inst", 100).unwrap());
        let recent = stats.recent_median_prep_ms.expect("enough runs");
        let earlier = stats.earlier_median_prep_ms.expect("enough runs");
        assert!(recent > earlier, "recent {recent} vs earlier {earlier}");
        assert_eq!(stats.latest_mod_count, Some(40));
        assert_eq!(stats.earliest_mod_count, Some(10));
    }

    #[test]
    fn crashes_are_counted_separately_from_runs() {
        let conn = conn();
        record(&conn, "2026-01-01T00:00:00Z", 1000, 5, LaunchResult::Ok);
        record(
            &conn,
            "2026-01-02T00:00:00Z",
            1000,
            5,
            LaunchResult::Crashed,
        );
        let stats = summarize(&list_history(&conn, "inst", 100).unwrap());
        assert_eq!(stats.runs, 2);
        assert_eq!(stats.crashes, 1);
    }
}
