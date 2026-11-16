use anyhow::Result;
use chrono::NaiveDate;
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};

use crate::smart::SmartSnapshot;

/// Returns the path to the SQLite database file in the platform-specific data directory.
/// Ensures the parent directory exists.
pub fn get_db_path() -> Result<PathBuf> {
    let proj_dirs = directories::ProjectDirs::from("com", "ssdwatch", "SSDWatch")
        .ok_or_else(|| anyhow::anyhow!("Failed to determine project directories"))?;
    let dir = proj_dirs.data_local_dir();
    std::fs::create_dir_all(dir)?;
    Ok(dir.join("ssd_watch.db"))
}

/// Initializes database connection using default application path.
pub fn init_db() -> Result<Connection> {
    let path = get_db_path()?;
    open_db(&path)
}

/// Applies SQLite PRAGMAs and creates tables and indexes if they do not already exist.
pub fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         CREATE TABLE IF NOT EXISTS ssd_snapshots (
             ts INTEGER PRIMARY KEY,
             data_units_written INTEGER NOT NULL,
             data_units_read INTEGER NOT NULL,
             percent_used INTEGER NOT NULL,
             available_spare INTEGER NOT NULL,
             power_on_hours INTEGER NOT NULL,
             power_cycles INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_ssd_snapshots_ts ON ssd_snapshots(ts);
         CREATE TABLE IF NOT EXISTS process_writes (
             ts INTEGER NOT NULL,
             process_name TEXT NOT NULL,
             bytes_delta INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_process_writes_ts ON process_writes(ts);"
    )?;
    Ok(())
}

/// Opens SQLite database at the specified path and initializes WAL mode and tables.
pub fn open_db(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
    init_schema(&conn)?;
    Ok(conn)
}

/// Inserts or replaces a SMART snapshot record in the database.
pub fn insert_snapshot(conn: &Connection, snapshot: &SmartSnapshot) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO ssd_snapshots (
             ts,
             data_units_written,
             data_units_read,
             percent_used,
             available_spare,
             power_on_hours,
             power_cycles
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            snapshot.ts,
            snapshot.data_units_written as i64,
            snapshot.data_units_read as i64,
            snapshot.percent_used as i64,
            snapshot.available_spare as i64,
            snapshot.power_on_hours as i64,
            snapshot.power_cycles as i64,
        ],
    )?;
    Ok(())
}

/// Batch-inserts process write deltas for a given timestamp using a transaction.
pub fn insert_process_deltas(conn: &Connection, ts: i64, deltas: &[(String, u64)]) -> Result<()> {
    if deltas.is_empty() {
        return Ok(());
    }
    let tx = conn.unchecked_transaction()?;
    {
        let mut stmt = tx.prepare_cached(
            "INSERT INTO process_writes (ts, process_name, bytes_delta) VALUES (?1, ?2, ?3)"
        )?;
        for (name, bytes) in deltas {
            stmt.execute(params![ts, name, *bytes as i64])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Computes daily total bytes written over the last `days` days, ordered chronologically ascending.
pub fn daily_writes(conn: &Connection, days: u32) -> Result<Vec<(NaiveDate, u64)>> {
    if days == 0 {
        return Ok(Vec::new());
    }
    let cutoff_ts = chrono::Utc::now().timestamp() - (days as i64) * 86_400;
    let mut stmt = conn.prepare(
        "SELECT COALESCE(date(ts, 'unixepoch', 'localtime'), date(ts, 'unixepoch')) AS day_str,
                SUM(bytes_delta) AS total_bytes
         FROM process_writes
         WHERE ts >= ?1
         GROUP BY day_str
         ORDER BY day_str ASC"
    )?;

    let rows = stmt.query_map(params![cutoff_ts], |row| {
        let date_str: String = row.get(0)?;
        let total: i64 = row.get(1)?;
        Ok((date_str, total))
    })?;

    let mut result = Vec::new();
    for row in rows {
        let (date_str, total) = row?;
        if let Ok(date) = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d") {
            result.push((date, total.max(0) as u64));
        }
    }
    Ok(result)
}

/// Retrieves the top processes by total bytes written since `since_ts`, ordered descending.
pub fn top_processes(conn: &Connection, since_ts: i64, limit: u32) -> Result<Vec<(String, u64)>> {
    let mut stmt = conn.prepare(
        "SELECT process_name, SUM(bytes_delta) AS total
         FROM process_writes
         WHERE ts >= ?1
         GROUP BY process_name
         ORDER BY total DESC
         LIMIT ?2"
    )?;

    let rows = stmt.query_map(params![since_ts, limit], |row| {
        let name: String = row.get(0)?;
        let total: i64 = row.get(1)?;
        Ok((name, total.max(0) as u64))
    })?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

/// Retrieves the most recent SMART snapshot, if any exists.
pub fn latest_snapshot(conn: &Connection) -> Result<Option<SmartSnapshot>> {
    let mut stmt = conn.prepare(
        "SELECT ts, data_units_written, data_units_read, percent_used, available_spare, power_on_hours, power_cycles
         FROM ssd_snapshots
         ORDER BY ts DESC
         LIMIT 1"
    )?;

    let mut rows = stmt.query_map([], |row| {
        Ok(SmartSnapshot {
            ts: row.get(0)?,
            data_units_written: row.get::<_, i64>(1)? as u64,
            data_units_read: row.get::<_, i64>(2)? as u64,
            percent_used: row.get::<_, u8>(3)?,
            available_spare: row.get::<_, u8>(4)?,
            power_on_hours: row.get::<_, i64>(5)? as u64,
            power_cycles: row.get::<_, i64>(6)? as u64,
        })
    })?;

    if let Some(res) = rows.next() {
        Ok(Some(res?))
    } else {
        Ok(None)
    }
}

/// Retrieves the `n` most recent SMART snapshots, ordered by `ts DESC`.
pub fn recent_snapshots(conn: &Connection, n: u32) -> Result<Vec<SmartSnapshot>> {
    if n == 0 {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(
        "SELECT ts, data_units_written, data_units_read, percent_used, available_spare, power_on_hours, power_cycles
         FROM ssd_snapshots
         ORDER BY ts DESC
         LIMIT ?1"
    )?;

    let rows = stmt.query_map(params![n], |row| {
        Ok(SmartSnapshot {
            ts: row.get(0)?,
            data_units_written: row.get::<_, i64>(1)? as u64,
            data_units_read: row.get::<_, i64>(2)? as u64,
            percent_used: row.get::<_, u8>(3)?,
            available_spare: row.get::<_, u8>(4)?,
            power_on_hours: row.get::<_, i64>(5)? as u64,
            power_cycles: row.get::<_, i64>(6)? as u64,
        })
    })?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

/// Prunes process write records older than `days` days.
/// Deletes rows where `ts < cutoff_ts` (cutoff_ts = now - days * 86400).
pub fn prune_old_records(conn: &Connection, days: i64) -> Result<usize> {
    let now = chrono::Utc::now().timestamp();
    let cutoff_ts = now - days * 86_400;
    let deleted = conn.execute(
        "DELETE FROM process_writes WHERE ts < ?1",
        params![cutoff_ts],
    )?;
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_db() -> Connection {
        let conn = Connection::open_in_memory().expect("failed to open in-memory db");
        init_schema(&conn).expect("failed to init schema");
        conn
    }

    #[test]
    fn test_schema_init() {
        let conn = create_test_db();

        // Verify tables exist
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table'")
            .unwrap();
        let tables: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        assert!(tables.contains(&"ssd_snapshots".to_string()));
        assert!(tables.contains(&"process_writes".to_string()));
    }

    #[test]
    fn test_snapshots_insert_and_retrieve() -> Result<()> {
        let conn = create_test_db();

        // Initially empty
        assert!(latest_snapshot(&conn)?.is_none());
        assert!(recent_snapshots(&conn, 10)?.is_empty());

        let s1 = SmartSnapshot {
            ts: 1000,
            data_units_written: 50,
            data_units_read: 25,
            percent_used: 1,
            available_spare: 100,
            power_on_hours: 10,
            power_cycles: 5,
        };
        insert_snapshot(&conn, &s1)?;

        let latest = latest_snapshot(&conn)?.expect("expected snapshot");
        assert_eq!(latest, s1);

        let s2 = SmartSnapshot {
            ts: 2000,
            data_units_written: 100,
            data_units_read: 50,
            percent_used: 2,
            available_spare: 99,
            power_on_hours: 20,
            power_cycles: 6,
        };
        insert_snapshot(&conn, &s2)?;

        let latest2 = latest_snapshot(&conn)?.expect("expected snapshot");
        assert_eq!(latest2, s2);

        let s3 = SmartSnapshot {
            ts: 1500,
            data_units_written: 75,
            data_units_read: 35,
            percent_used: 1,
            available_spare: 100,
            power_on_hours: 15,
            power_cycles: 5,
        };
        insert_snapshot(&conn, &s3)?;

        // recent_snapshots with n=2 should return [s2 (2000), s3 (1500)]
        let recent2 = recent_snapshots(&conn, 2)?;
        assert_eq!(recent2.len(), 2);
        assert_eq!(recent2[0], s2);
        assert_eq!(recent2[1], s3);

        // recent_snapshots with n=10 should return all 3 descending
        let recent_all = recent_snapshots(&conn, 10)?;
        assert_eq!(recent_all.len(), 3);
        assert_eq!(recent_all[0], s2);
        assert_eq!(recent_all[1], s3);
        assert_eq!(recent_all[2], s1);

        // n=0 should return empty
        assert!(recent_snapshots(&conn, 0)?.is_empty());

        Ok(())
    }

    #[test]
    fn test_process_deltas_and_top_processes() -> Result<()> {
        let conn = create_test_db();

        let ts1 = 1000;
        let deltas1 = vec![
            ("chrome.exe".to_string(), 500_000),
            ("code.exe".to_string(), 300_000),
            ("system".to_string(), 100_000),
        ];
        insert_process_deltas(&conn, ts1, &deltas1)?;

        let ts2 = 2000;
        let deltas2 = vec![
            ("chrome.exe".to_string(), 200_000),
            ("code.exe".to_string(), 400_000),
            ("cargo.exe".to_string(), 800_000),
        ];
        insert_process_deltas(&conn, ts2, &deltas2)?;

        // Query top processes since ts=0
        let top = top_processes(&conn, 0, 10)?;
        assert_eq!(top.len(), 4);
        assert_eq!(top[0], ("cargo.exe".to_string(), 800_000));
        assert_eq!(top[1], ("code.exe".to_string(), 700_000));
        assert_eq!(top[2], ("chrome.exe".to_string(), 700_000));
        assert_eq!(top[3], ("system".to_string(), 100_000));

        // Limit test
        let top_2 = top_processes(&conn, 0, 2)?;
        assert_eq!(top_2.len(), 2);

        // since_ts filter test
        let top_recent = top_processes(&conn, 1500, 10)?;
        assert_eq!(top_recent.len(), 3);
        assert_eq!(top_recent[0], ("cargo.exe".to_string(), 800_000));
        assert_eq!(top_recent[1], ("code.exe".to_string(), 400_000));
        assert_eq!(top_recent[2], ("chrome.exe".to_string(), 200_000));

        Ok(())
    }

    #[test]
    fn test_daily_writes_aggregation() -> Result<()> {
        let conn = create_test_db();

        let now = chrono::Utc::now().timestamp();
        let day_sec = 86_400;

        // Insert deltas: today, yesterday, 2 days ago, 5 days ago
        let ts_today = now;
        let ts_yesterday = now - day_sec;
        let ts_2d_ago = now - 2 * day_sec;
        let ts_5d_ago = now - 5 * day_sec;
        let ts_old = now - 20 * day_sec;

        insert_process_deltas(&conn, ts_today, &[("proc1".into(), 1000), ("proc2".into(), 500)])?;
        insert_process_deltas(&conn, ts_yesterday, &[("proc1".into(), 2000)])?;
        insert_process_deltas(&conn, ts_2d_ago, &[("proc3".into(), 3000)])?;
        insert_process_deltas(&conn, ts_5d_ago, &[("proc1".into(), 4000)])?;
        insert_process_deltas(&conn, ts_old, &[("proc1".into(), 50000)])?;

        // Query daily writes for last 3 days
        let writes = daily_writes(&conn, 3)?;
        // Should include at least 2d ago, yesterday, today (order ascending)
        assert!(!writes.is_empty());
        for i in 1..writes.len() {
            assert!(writes[i].0 >= writes[i - 1].0);
        }

        // Query daily writes with days=0
        let zero = daily_writes(&conn, 0)?;
        assert!(zero.is_empty());

        Ok(())
    }

    #[test]
    fn test_prune_old_records() -> Result<()> {
        let conn = create_test_db();
        let now = chrono::Utc::now().timestamp();
        let day_sec = 86_400;

        insert_process_deltas(&conn, now - 100 * day_sec, &[("old1".into(), 100)])?;
        insert_process_deltas(&conn, now - 50 * day_sec, &[("old2".into(), 200)])?;
        insert_process_deltas(&conn, now - 10 * day_sec, &[("recent".into(), 300)])?;
        insert_process_deltas(&conn, now, &[("now".into(), 400)])?;

        // Prune older than 30 days -> should delete 2 records
        let deleted = prune_old_records(&conn, 30)?;
        assert_eq!(deleted, 2);

        // Verify remaining records
        let top = top_processes(&conn, 0, 10)?;
        assert_eq!(top.len(), 2);
        assert!(top.iter().any(|(name, _)| name == "recent"));
        assert!(top.iter().any(|(name, _)| name == "now"));

        Ok(())
    }

    #[test]
    fn test_get_db_path() -> Result<()> {
        let path = get_db_path()?;
        assert!(path.ends_with("ssd_watch.db"));
        assert!(path.parent().unwrap().exists());
        Ok(())
    }
}
