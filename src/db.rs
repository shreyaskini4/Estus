use anyhow::Result;
use chrono::NaiveDate;
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};

use crate::smart::{DriveInfo, SmartSnapshot};

/// Maximum elapsed seconds between consecutive snapshots before attributing writes to offline/gap.
pub const GAP_THRESHOLD_SECS: i64 = 900;

/// Represents daily write totals partitioned into active writes (while Estus was running)
/// and gap writes (while Estus was not running or between distant snapshots).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct DailyWrites {
    pub date: chrono::NaiveDate,
    pub active_bytes: u64,
    pub gap_bytes: u64,
}

impl DailyWrites {
    pub fn total_bytes(&self) -> u64 {
        self.active_bytes.saturating_add(self.gap_bytes)
    }
}

/// Represents an interval between two SMART snapshots in the last 24 hours.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SnapshotInterval {
    pub timestamp: i64,
    pub raw_data_units_written: u64,
    pub delta_gb: f64,
    pub attribution: String, // e.g., local date "YYYY-MM-DD" or "app not running"
}

/// Returns the path to the SQLite database file in the platform-specific data directory.
/// Ensures the parent directory exists.
pub fn get_db_path() -> Result<PathBuf> {
    let proj_dirs = directories::ProjectDirs::from("com", "estus", "Estus")
        .ok_or_else(|| anyhow::anyhow!("Failed to determine project directories"))?;
    let dir = proj_dirs.data_local_dir();
    std::fs::create_dir_all(dir)?;
    Ok(dir.join("estus.db"))
}

/// Initializes database connection using default application path.
pub fn init_db() -> Result<Connection> {
    let path = get_db_path()?;
    open_db(&path)
}

/// Applies SQLite PRAGMAs, creates tables/indexes, and runs database migrations.
pub fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         CREATE TABLE IF NOT EXISTS drives (
             id TEXT PRIMARY KEY,
             serial TEXT NOT NULL,
             model TEXT NOT NULL,
             capacity_bytes INTEGER NOT NULL,
             nickname TEXT,
             enabled INTEGER NOT NULL DEFAULT 1
         );
         CREATE TABLE IF NOT EXISTS ssd_snapshots (
             ts INTEGER NOT NULL,
             drive_id TEXT,
             data_units_written INTEGER NOT NULL,
             data_units_read INTEGER NOT NULL,
             percent_used INTEGER NOT NULL,
             available_spare INTEGER NOT NULL,
             power_on_hours INTEGER NOT NULL,
             power_cycles INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS process_writes (
             ts INTEGER NOT NULL,
             process_name TEXT NOT NULL,
             bytes_delta INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_process_writes_ts ON process_writes(ts);"
    )?;

    // Check if drive_id column exists and whether ts was defined as PRIMARY KEY
    let mut stmt = conn.prepare("PRAGMA table_info(ssd_snapshots)")?;
    let cols_info: Vec<(String, i64)> = stmt
        .query_map([], |row| Ok((row.get(1)?, row.get(5)?)))?
        .filter_map(|r| r.ok())
        .collect();

    let has_drive_id = cols_info.iter().any(|(name, _)| name == "drive_id");
    let ts_is_pk = cols_info.iter().any(|(name, pk)| name == "ts" && *pk > 0);

    if !has_drive_id {
        conn.execute("ALTER TABLE ssd_snapshots ADD COLUMN drive_id TEXT", [])?;
    }

    if ts_is_pk {
        // Migrate table to remove PRIMARY KEY constraint on ts alone so composite (drive_id, ts) uniqueness can be enforced
        conn.execute_batch(
            "CREATE TABLE ssd_snapshots_migrated (
                 ts INTEGER NOT NULL,
                 drive_id TEXT,
                 data_units_written INTEGER NOT NULL,
                 data_units_read INTEGER NOT NULL,
                 percent_used INTEGER NOT NULL,
                 available_spare INTEGER NOT NULL,
                 power_on_hours INTEGER NOT NULL,
                 power_cycles INTEGER NOT NULL
             );
             INSERT INTO ssd_snapshots_migrated (ts, drive_id, data_units_written, data_units_read, percent_used, available_spare, power_on_hours, power_cycles)
             SELECT ts, drive_id, data_units_written, data_units_read, percent_used, available_spare, power_on_hours, power_cycles FROM ssd_snapshots;
             DROP TABLE ssd_snapshots;
             ALTER TABLE ssd_snapshots_migrated RENAME TO ssd_snapshots;"
        )?;
    }

    // Assign existing snapshots with NULL or empty drive_id
    assign_orphan_snapshots(conn)?;

    // Deduplicate any pre-existing records with identical (drive_id, ts) before creating unique index
    conn.execute(
        "DELETE FROM ssd_snapshots WHERE rowid NOT IN (
             SELECT MIN(rowid) FROM ssd_snapshots GROUP BY drive_id, ts
         )",
        [],
    )?;

    conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_ssd_snapshots_unique ON ssd_snapshots(drive_id, ts)",
        [],
    )?;

    // Check if protocol and device_path columns exist in drives
    let mut stmt_d = conn.prepare("PRAGMA table_info(drives)")?;
    let d_cols: Vec<String> = stmt_d
        .query_map([], |row| row.get(1))?
        .filter_map(|r| r.ok())
        .collect();

    if !d_cols.iter().any(|c| c == "protocol") {
        let _ = conn.execute("ALTER TABLE drives ADD COLUMN protocol TEXT DEFAULT ''", []);
    }
    if !d_cols.iter().any(|c| c == "device_path") {
        let _ = conn.execute("ALTER TABLE drives ADD COLUMN device_path TEXT DEFAULT ''", []);
    }

    // Migration to version 2: Purge legacy flawed process write deltas
    let user_version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if user_version < 2 {
        conn.execute("DELETE FROM process_writes", [])?;
        conn.execute("PRAGMA user_version = 2", [])?;
        crate::logger::log_info(
            "Legacy process writes were purged and database migrated to version 2",
        );
    }

    Ok(())
}

fn assign_orphan_snapshots(conn: &Connection) -> Result<()> {
    let first_drive_id: String = conn
        .query_row("SELECT id FROM drives LIMIT 1", [], |row| row.get(0))
        .unwrap_or_else(|_| "default".to_string());

    conn.execute(
        "UPDATE ssd_snapshots SET drive_id = ?1 WHERE drive_id IS NULL OR drive_id = ''",
        params![first_drive_id],
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

/// Inserts or replaces a drive metadata record in the drives table.
/// Preserves existing user nickname and enabled preference on conflict.
pub fn upsert_drive(conn: &Connection, drive: &DriveInfo) -> Result<()> {
    conn.execute(
        "INSERT INTO drives (id, serial, model, capacity_bytes, nickname, enabled, protocol, device_path)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET
             serial = excluded.serial,
             model = excluded.model,
             capacity_bytes = excluded.capacity_bytes,
             protocol = excluded.protocol,
             device_path = excluded.device_path,
             nickname = COALESCE(drives.nickname, excluded.nickname),
             enabled = drives.enabled",
        params![
            drive.id,
            drive.serial,
            drive.model,
            drive.capacity_bytes as i64,
            drive.nickname,
            if drive.enabled { 1 } else { 0 },
            drive.protocol,
            drive.device_path,
        ],
    )?;
    Ok(())
}

/// Retrieves all registered drives ordered by model and serial.
pub fn get_drives(conn: &Connection) -> Result<Vec<DriveInfo>> {
    let mut stmt = conn.prepare(
        "SELECT id, serial, model, capacity_bytes, nickname, enabled, COALESCE(protocol, ''), COALESCE(device_path, '')
         FROM drives
         ORDER BY model ASC, serial ASC"
    )?;

    let rows = stmt.query_map([], |row| {
        let enabled_int: i64 = row.get(5)?;
        Ok(DriveInfo {
            id: row.get(0)?,
            serial: row.get(1)?,
            model: row.get(2)?,
            capacity_bytes: row.get::<_, i64>(3)? as u64,
            nickname: row.get(4)?,
            enabled: enabled_int != 0,
            protocol: row.get(6)?,
            device_path: row.get(7)?,
        })
    })?;

    let mut drives = Vec::new();
    for row in rows {
        drives.push(row?);
    }
    Ok(drives)
}

/// Updates user settings (nickname and enabled flag) for a specific drive.
pub fn update_drive_settings(
    conn: &Connection,
    drive_id: &str,
    nickname: Option<&str>,
    enabled: bool,
) -> Result<()> {
    conn.execute(
        "UPDATE drives SET nickname = ?1, enabled = ?2 WHERE id = ?3",
        params![nickname, if enabled { 1 } else { 0 }, drive_id],
    )?;
    Ok(())
}

/// Inserts a SMART snapshot record for a specific drive.
/// Duplicate snapshots with identical (drive_id, ts) are cleanly ignored via INSERT OR IGNORE.
pub fn insert_snapshot(conn: &Connection, drive_id: &str, snapshot: &SmartSnapshot) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO ssd_snapshots (
             ts,
             drive_id,
             data_units_written,
             data_units_read,
             percent_used,
             available_spare,
             power_on_hours,
             power_cycles
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            snapshot.ts,
            drive_id,
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

/// Computes daily total bytes written over the last `days` days based ONLY on SMART snapshots.
/// If `drive_id` is `Some(id)`, returns daily writes for that drive based on SMART snapshots.
/// If `drive_id` is `None`, aggregates and sums writes across all drives by date.
///
/// Consecutive snapshots with elapsed time > `GAP_THRESHOLD_SECS` (900 seconds) are attributed
/// to `gap_bytes` (writes while Estus was not running), while normal intervals <= 900s are attributed
/// to `active_bytes`. The first snapshot for each drive serves as baseline and counts as zero bytes.
/// If a counter reset/wrap occurs (current < previous), the delta is ignored (0 bytes) and a warning is logged.
pub fn daily_writes(
    conn: &Connection,
    drive_id: Option<&str>,
    days: u32,
) -> Result<Vec<DailyWrites>> {
    if days == 0 {
        return Ok(Vec::new());
    }

    // Collect drive IDs to query
    let drive_ids: Vec<String> = match drive_id {
        Some(id) => vec![id.to_string()],
        None => {
            let mut stmt = conn.prepare(
                "SELECT DISTINCT drive_id FROM ssd_snapshots WHERE drive_id IS NOT NULL AND drive_id != ''"
            )?;
            let ids = stmt.query_map([], |r| r.get(0))?;
            ids.filter_map(|r| r.ok()).collect()
        }
    };

    let mut per_day: std::collections::BTreeMap<NaiveDate, DailyWrites> = std::collections::BTreeMap::new();

    for id in &drive_ids {
        let mut stmt = conn.prepare(
            "SELECT ts, data_units_written
             FROM ssd_snapshots
             WHERE drive_id = ?1
             ORDER BY ts ASC"
        )?;
        let rows = stmt.query_map(params![id], |r| {
            let ts: i64 = r.get(0)?;
            let written: i64 = r.get(1)?;
            Ok((ts, written as u64))
        })?;

        let mut prev_snap: Option<(i64, u64)> = None;
        for row in rows {
            let (ts, written) = row?;
            if let Some((prev_ts, prev_written)) = prev_snap
                && let Some(dt) = chrono::DateTime::from_timestamp(ts, 0)
            {
                let date = dt.with_timezone(&chrono::Local).date_naive();
                let entry = per_day.entry(date).or_insert(DailyWrites {
                    date,
                    active_bytes: 0,
                    gap_bytes: 0,
                });

                if written < prev_written {
                    crate::logger::log_warn(&format!(
                        "SMART counter reset/wrap detected for drive '{}': prev={}, current={}. Ignoring delta.",
                        id, prev_written, written
                    ));
                } else {
                    let delta_units = written - prev_written;
                    let delta_bytes = delta_units.saturating_mul(512_000);
                    let elapsed_secs = ts.saturating_sub(prev_ts);
                    if elapsed_secs > GAP_THRESHOLD_SECS {
                        entry.gap_bytes = entry.gap_bytes.saturating_add(delta_bytes);
                    } else {
                        entry.active_bytes = entry.active_bytes.saturating_add(delta_bytes);
                    }
                }
            }
            prev_snap = Some((ts, written));
        }
    }

    let today = chrono::Local::now().date_naive();
    let cutoff_date = today - chrono::Duration::days(days as i64);
    let res: Vec<DailyWrites> = per_day
        .into_values()
        .filter(|dw| dw.date >= cutoff_date)
        .collect();

    Ok(res)
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

/// Retrieves the most recent SMART snapshot for a specified drive.
pub fn latest_snapshot(conn: &Connection, drive_id: &str) -> Result<Option<SmartSnapshot>> {
    let mut stmt = conn.prepare(
        "SELECT ts, data_units_written, data_units_read, percent_used, available_spare, power_on_hours, power_cycles
         FROM ssd_snapshots
         WHERE drive_id = ?1
         ORDER BY ts DESC
         LIMIT 1"
    )?;

    let mut rows = stmt.query_map(params![drive_id], |row| {
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

/// Retrieves the `n` most recent SMART snapshots, optionally filtered by `drive_id`.
pub fn recent_snapshots(
    conn: &Connection,
    drive_id: Option<&str>,
    n: u32,
) -> Result<Vec<SmartSnapshot>> {
    if n == 0 {
        return Ok(Vec::new());
    }

    let mut result = Vec::new();
    if let Some(id) = drive_id {
        let mut stmt = conn.prepare(
            "SELECT ts, data_units_written, data_units_read, percent_used, available_spare, power_on_hours, power_cycles
             FROM ssd_snapshots
             WHERE drive_id = ?1
             ORDER BY ts DESC
             LIMIT ?2"
        )?;
        let rows = stmt.query_map(params![id, n], |row| {
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
        for row in rows {
            result.push(row?);
        }
    } else {
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
        for row in rows {
            result.push(row?);
        }
    }
    Ok(result)
}

/// Prunes process write records older than `days` days.
pub fn prune_old_records(conn: &Connection, days: i64) -> Result<usize> {
    let now = chrono::Utc::now().timestamp();
    let cutoff_ts = now - days * 86_400;
    let deleted = conn.execute(
        "DELETE FROM process_writes WHERE ts < ?1",
        params![cutoff_ts],
    )?;
    Ok(deleted)
}

/// Resets write history: purges all process writes and prunes SSD snapshots
/// so that only the latest snapshot per drive is retained as a baseline.
pub fn reset_history(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM process_writes", [])?;
    tx.execute(
        "DELETE FROM ssd_snapshots
         WHERE rowid NOT IN (
             SELECT MIN(s.rowid)
             FROM ssd_snapshots s
             INNER JOIN (
                 SELECT drive_id, MAX(ts) AS max_ts
                 FROM ssd_snapshots
                 GROUP BY drive_id
             ) m ON (s.drive_id = m.drive_id OR (s.drive_id IS NULL AND m.drive_id IS NULL)) AND s.ts = m.max_ts
             GROUP BY s.drive_id
         )",
        [],
    )?;
    tx.commit()?;
    crate::logger::log_info(
        "History reset executed: cleared process_writes and pruned ssd_snapshots to latest baseline per drive",
    );
    Ok(())
}

/// Helper function to compute snapshot intervals from an optional baseline and a series of consecutive snapshots.
fn compute_intervals_from_snaps(
    baseline_snap: Option<(i64, u64)>,
    snaps: Vec<(i64, u64)>,
) -> Vec<SnapshotInterval> {
    let mut intervals = Vec::with_capacity(snaps.len());
    let mut prev_snap = baseline_snap;

    for (ts, written) in snaps {
        let (delta_gb, attribution) = match prev_snap {
            None => (0.0, "baseline".to_string()),
            Some((prev_ts, prev_written)) => {
                let elapsed_secs = ts.saturating_sub(prev_ts);
                let delta_gb = if written < prev_written {
                    crate::logger::log_warn(&format!(
                        "SMART counter reset/wrap detected: prev={}, current={}. Setting interval delta to 0.0 GB.",
                        prev_written, written
                    ));
                    0.0
                } else {
                    let delta_units = written.saturating_sub(prev_written);
                    (delta_units.saturating_mul(512_000)) as f64 / 1_000_000_000.0
                };
                let attribution = if elapsed_secs > GAP_THRESHOLD_SECS {
                    "app not running".to_string()
                } else {
                    let dt = chrono::DateTime::from_timestamp(ts, 0)
                        .map(|dt| dt.with_timezone(&chrono::Local))
                        .unwrap_or_else(chrono::Local::now);
                    dt.format("%Y-%m-%d").to_string()
                };
                (delta_gb, attribution)
            }
        };

        intervals.push(SnapshotInterval {
            timestamp: ts,
            raw_data_units_written: written,
            delta_gb,
            attribution,
        });

        prev_snap = Some((ts, written));
    }

    intervals
}

fn get_intervals_for_single_drive(
    conn: &Connection,
    drive_id: Option<&str>,
    cutoff_ts: i64,
) -> Result<Vec<SnapshotInterval>> {
    let (baseline, snaps) = if let Some(id) = drive_id {
        let mut baseline_stmt = conn.prepare(
            "SELECT ts, data_units_written FROM ssd_snapshots WHERE drive_id = ?1 AND ts < ?2 ORDER BY ts DESC LIMIT 1"
        )?;
        let mut b_rows = baseline_stmt.query_map(params![id, cutoff_ts], |r| {
            let ts: i64 = r.get(0)?;
            let written: i64 = r.get(1)?;
            Ok((ts, written as u64))
        })?;
        let baseline = b_rows.next().transpose()?;

        let mut stmt = conn.prepare(
            "SELECT ts, data_units_written FROM ssd_snapshots WHERE drive_id = ?1 AND ts >= ?2 ORDER BY ts ASC"
        )?;
        let rows = stmt.query_map(params![id, cutoff_ts], |r| {
            let ts: i64 = r.get(0)?;
            let written: i64 = r.get(1)?;
            Ok((ts, written as u64))
        })?;
        let mut snaps = Vec::new();
        for r in rows {
            snaps.push(r?);
        }
        (baseline, snaps)
    } else {
        let mut baseline_stmt = conn.prepare(
            "SELECT ts, data_units_written FROM ssd_snapshots WHERE ts < ?1 ORDER BY ts DESC LIMIT 1"
        )?;
        let mut b_rows = baseline_stmt.query_map(params![cutoff_ts], |r| {
            let ts: i64 = r.get(0)?;
            let written: i64 = r.get(1)?;
            Ok((ts, written as u64))
        })?;
        let baseline = b_rows.next().transpose()?;

        let mut stmt = conn.prepare(
            "SELECT ts, data_units_written FROM ssd_snapshots WHERE ts >= ?1 ORDER BY ts ASC"
        )?;
        let rows = stmt.query_map(params![cutoff_ts], |r| {
            let ts: i64 = r.get(0)?;
            let written: i64 = r.get(1)?;
            Ok((ts, written as u64))
        })?;
        let mut snaps = Vec::new();
        for r in rows {
            snaps.push(r?);
        }
        (baseline, snaps)
    };

    Ok(compute_intervals_from_snaps(baseline, snaps))
}

/// Retrieves SMART write intervals since a given cutoff timestamp.
pub fn get_intervals_since(
    conn: &Connection,
    drive_id: Option<&str>,
    cutoff_ts: i64,
) -> Result<Vec<SnapshotInterval>> {
    match drive_id {
        Some(id) => get_intervals_for_single_drive(conn, Some(id), cutoff_ts),
        None => {
            let mut stmt = conn.prepare(
                "SELECT DISTINCT drive_id FROM ssd_snapshots WHERE drive_id IS NOT NULL AND drive_id != ''"
            )?;
            let ids: Vec<String> = stmt
                .query_map([], |r| r.get(0))?
                .filter_map(|r| r.ok())
                .collect();

            if ids.is_empty() {
                get_intervals_for_single_drive(conn, None, cutoff_ts)
            } else if ids.len() == 1 {
                get_intervals_for_single_drive(conn, Some(&ids[0]), cutoff_ts)
            } else {
                let mut all_intervals = Vec::new();
                for id in &ids {
                    let mut intervals = get_intervals_for_single_drive(conn, Some(id), cutoff_ts)?;
                    all_intervals.append(&mut intervals);
                }
                all_intervals.sort_by_key(|i| i.timestamp);
                Ok(all_intervals)
            }
        }
    }
}

/// Retrieves SMART write intervals over the last 24 hours (now - 86,400s),
/// fetching the snapshot immediately preceding the 24-hour cutoff as the starting baseline
/// so the first interval has an accurate delta.
///
/// If `drive_id` is `Some(id)`, intervals are computed for that drive.
/// If `drive_id` is `None`, intervals are computed across drives and merged chronologically.
pub fn get_intervals_24h(
    conn: &Connection,
    drive_id: Option<&str>,
) -> Result<Vec<SnapshotInterval>> {
    let now_ts = chrono::Utc::now().timestamp();
    let cutoff_ts = now_ts - 86_400;

    get_intervals_since(conn, drive_id, cutoff_ts)
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

        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table'")
            .unwrap();
        let tables: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        assert!(tables.contains(&"drives".to_string()));
        assert!(tables.contains(&"ssd_snapshots".to_string()));
        assert!(tables.contains(&"process_writes".to_string()));
    }

    #[test]
    fn test_upsert_and_get_drives() -> Result<()> {
        let conn = create_test_db();

        let drive1 = DriveInfo {
            id: "SAMSUNG_980_123".to_string(),
            serial: "123".to_string(),
            model: "Samsung 980".to_string(),
            capacity_bytes: 1_000_000_000_000,
            protocol: "NVMe".to_string(),
            device_path: "/dev/nvme0".to_string(),
            nickname: Some("System Drive".to_string()),
            enabled: true,
        };

        upsert_drive(&conn, &drive1)?;
        let drives = get_drives(&conn)?;
        assert_eq!(drives.len(), 1);
        assert_eq!(drives[0].id, "SAMSUNG_980_123");
        assert_eq!(drives[0].nickname, Some("System Drive".to_string()));

        // Update settings (nickname & enabled)
        update_drive_settings(&conn, "SAMSUNG_980_123", Some("Main OS"), false)?;
        let updated = get_drives(&conn)?;
        assert_eq!(updated[0].nickname, Some("Main OS".to_string()));
        assert!(!updated[0].enabled);

        // Upserting again should preserve user nickname and enabled preference
        let drive1_rediscovered = DriveInfo {
            id: "SAMSUNG_980_123".to_string(),
            serial: "123".to_string(),
            model: "Samsung 980".to_string(),
            capacity_bytes: 1_000_000_000_000,
            protocol: "NVMe".to_string(),
            device_path: "/dev/nvme0".to_string(),
            nickname: None,
            enabled: true,
        };
        upsert_drive(&conn, &drive1_rediscovered)?;
        let preserved = get_drives(&conn)?;
        assert_eq!(preserved[0].nickname, Some("Main OS".to_string()));
        assert!(!preserved[0].enabled);

        Ok(())
    }

    #[test]
    fn test_snapshots_insert_and_retrieve_multiple_drives() -> Result<()> {
        let conn = create_test_db();

        assert!(latest_snapshot(&conn, "drive1")?.is_none());
        assert!(recent_snapshots(&conn, Some("drive1"), 10)?.is_empty());

        let s1 = SmartSnapshot {
            ts: 1000,
            data_units_written: 50,
            data_units_read: 25,
            percent_used: 1,
            available_spare: 100,
            power_on_hours: 10,
            power_cycles: 5,
        };
        insert_snapshot(&conn, "drive1", &s1)?;

        let s2 = SmartSnapshot {
            ts: 2000,
            data_units_written: 100,
            data_units_read: 50,
            percent_used: 2,
            available_spare: 99,
            power_on_hours: 20,
            power_cycles: 6,
        };
        insert_snapshot(&conn, "drive1", &s2)?;

        let s3 = SmartSnapshot {
            ts: 1500,
            data_units_written: 75,
            data_units_read: 35,
            percent_used: 1,
            available_spare: 100,
            power_on_hours: 15,
            power_cycles: 5,
        };
        insert_snapshot(&conn, "drive2", &s3)?;

        let latest_d1 = latest_snapshot(&conn, "drive1")?.expect("expected snapshot");
        assert_eq!(latest_d1, s2);

        let latest_d2 = latest_snapshot(&conn, "drive2")?.expect("expected snapshot");
        assert_eq!(latest_d2, s3);

        let recent_d1 = recent_snapshots(&conn, Some("drive1"), 10)?;
        assert_eq!(recent_d1.len(), 2);
        assert_eq!(recent_d1[0], s2);
        assert_eq!(recent_d1[1], s1);

        let recent_all = recent_snapshots(&conn, None, 10)?;
        assert_eq!(recent_all.len(), 3);
        assert_eq!(recent_all[0], s2);
        assert_eq!(recent_all[1], s3);
        assert_eq!(recent_all[2], s1);

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

        let top = top_processes(&conn, 0, 10)?;
        assert_eq!(top.len(), 4);
        assert_eq!(top[0], ("cargo.exe".to_string(), 800_000));
        assert_eq!(top[1], ("code.exe".to_string(), 700_000));
        assert_eq!(top[2], ("chrome.exe".to_string(), 700_000));
        assert_eq!(top[3], ("system".to_string(), 100_000));

        let top_2 = top_processes(&conn, 0, 2)?;
        assert_eq!(top_2.len(), 2);

        let top_recent = top_processes(&conn, 1500, 10)?;
        assert_eq!(top_recent.len(), 3);
        assert_eq!(top_recent[0], ("cargo.exe".to_string(), 800_000));
        assert_eq!(top_recent[1], ("code.exe".to_string(), 400_000));
        assert_eq!(top_recent[2], ("chrome.exe".to_string(), 200_000));

        Ok(())
    }

    #[test]
    fn test_daily_writes_no_process_writes_fallback() -> Result<()> {
        let conn = create_test_db();

        let now = chrono::Utc::now().timestamp();
        let day_sec = 86_400;

        let ts_today = now;
        let ts_yesterday = now - day_sec;

        insert_process_deltas(&conn, ts_today, &[("proc1".into(), 1000), ("proc2".into(), 500)])?;
        insert_process_deltas(&conn, ts_yesterday, &[("proc1".into(), 2000)])?;

        // Fallback removed: daily_writes must return empty vec when ssd_snapshots has no data
        let writes = daily_writes(&conn, None, 3)?;
        assert!(writes.is_empty(), "Expected no fallback to process_writes");

        let zero = daily_writes(&conn, None, 0)?;
        assert!(zero.is_empty());

        Ok(())
    }

    #[test]
    fn test_daily_writes_with_snapshots_per_drive_and_aggregated() -> Result<()> {
        let conn = create_test_db();
        let now = chrono::Utc::now().timestamp();
        let day_sec = 86_400;

        // Drive 1:
        // Day -2 baseline: 1000 units
        // Day -1: 1050 units (delta 50 units = 25,600,000 bytes)
        // Today: 1150 units (delta 100 units = 51,200,000 bytes)
        insert_snapshot(&conn, "drive1", &SmartSnapshot {
            ts: now - 2 * day_sec,
            data_units_written: 1000,
            data_units_read: 500,
            percent_used: 1,
            available_spare: 100,
            power_on_hours: 10,
            power_cycles: 2,
        })?;
        insert_snapshot(&conn, "drive1", &SmartSnapshot {
            ts: now - day_sec,
            data_units_written: 1050,
            data_units_read: 520,
            percent_used: 1,
            available_spare: 100,
            power_on_hours: 34,
            power_cycles: 2,
        })?;
        insert_snapshot(&conn, "drive1", &SmartSnapshot {
            ts: now,
            data_units_written: 1150,
            data_units_read: 550,
            percent_used: 1,
            available_spare: 100,
            power_on_hours: 58,
            power_cycles: 2,
        })?;

        // Drive 2:
        // Day -1 baseline: 2000 units
        // Today: 2100 units (delta 100 units = 51,200,000 bytes)
        insert_snapshot(&conn, "drive2", &SmartSnapshot {
            ts: now - day_sec,
            data_units_written: 2000,
            data_units_read: 1000,
            percent_used: 2,
            available_spare: 98,
            power_on_hours: 100,
            power_cycles: 5,
        })?;
        insert_snapshot(&conn, "drive2", &SmartSnapshot {
            ts: now,
            data_units_written: 2100,
            data_units_read: 1050,
            percent_used: 2,
            available_spare: 98,
            power_on_hours: 124,
            power_cycles: 5,
        })?;

        // Query drive 1 specifically
        let d1_writes = daily_writes(&conn, Some("drive1"), 3)?;
        assert!(!d1_writes.is_empty());
        let today_d1 = d1_writes.last().unwrap().total_bytes();
        assert_eq!(today_d1, 100 * 512_000);

        // Query drive 2 specifically
        let d2_writes = daily_writes(&conn, Some("drive2"), 3)?;
        assert!(!d2_writes.is_empty());
        let today_d2 = d2_writes.last().unwrap().total_bytes();
        assert_eq!(today_d2, 100 * 512_000);

        // Query aggregated ("All drives")
        let all_writes = daily_writes(&conn, None, 3)?;
        let today_all = all_writes.last().unwrap().total_bytes();
        assert_eq!(today_all, (100 + 100) * 512_000);

        Ok(())
    }

    #[test]
    fn test_normal_deltas_within_gap_threshold() -> Result<()> {
        let conn = create_test_db();
        let now = chrono::Local::now().timestamp();

        // Three snapshots 300 seconds apart (<= 900s)
        let s0 = SmartSnapshot {
            ts: now - 600,
            data_units_written: 500,
            data_units_read: 100,
            percent_used: 1,
            available_spare: 100,
            power_on_hours: 10,
            power_cycles: 1,
        };
        let s1 = SmartSnapshot {
            ts: now - 300,
            data_units_written: 550, // +50 units
            data_units_read: 110,
            percent_used: 1,
            available_spare: 100,
            power_on_hours: 10,
            power_cycles: 1,
        };
        let s2 = SmartSnapshot {
            ts: now,
            data_units_written: 620, // +70 units
            data_units_read: 120,
            percent_used: 1,
            available_spare: 100,
            power_on_hours: 10,
            power_cycles: 1,
        };

        insert_snapshot(&conn, "drive_norm", &s0)?;
        insert_snapshot(&conn, "drive_norm", &s1)?;
        insert_snapshot(&conn, "drive_norm", &s2)?;

        let writes = daily_writes(&conn, Some("drive_norm"), 7)?;
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].active_bytes, (50 + 70) * 512_000);
        assert_eq!(writes[0].gap_bytes, 0);
        assert_eq!(writes[0].total_bytes(), (50 + 70) * 512_000);

        Ok(())
    }

    #[test]
    fn test_first_snapshot_baseline_counts_as_zero() -> Result<()> {
        let conn = create_test_db();
        let now = chrono::Local::now().timestamp();

        let s0 = SmartSnapshot {
            ts: now,
            data_units_written: 1_000_000,
            data_units_read: 500_000,
            percent_used: 5,
            available_spare: 95,
            power_on_hours: 100,
            power_cycles: 10,
        };

        insert_snapshot(&conn, "drive_baseline", &s0)?;

        let writes = daily_writes(&conn, Some("drive_baseline"), 30)?;
        assert!(writes.is_empty(), "First snapshot must count as baseline with zero bytes");

        Ok(())
    }

    #[test]
    fn test_counter_reset_ignored_and_logged() -> Result<()> {
        let conn = create_test_db();
        let now = chrono::Local::now().timestamp();

        // Snapshot 0: baseline (1000 units)
        insert_snapshot(&conn, "drive_reset", &SmartSnapshot {
            ts: now - 600,
            data_units_written: 1000,
            data_units_read: 100,
            percent_used: 1,
            available_spare: 100,
            power_on_hours: 10,
            power_cycles: 1,
        })?;

        // Snapshot 1: reset/wrap (200 units < 1000 units -> delta ignored)
        insert_snapshot(&conn, "drive_reset", &SmartSnapshot {
            ts: now - 300,
            data_units_written: 200,
            data_units_read: 100,
            percent_used: 1,
            available_spare: 100,
            power_on_hours: 10,
            power_cycles: 1,
        })?;

        // Snapshot 2: subsequent normal write (+50 units from 200 to 250)
        insert_snapshot(&conn, "drive_reset", &SmartSnapshot {
            ts: now,
            data_units_written: 250,
            data_units_read: 100,
            percent_used: 1,
            available_spare: 100,
            power_on_hours: 10,
            power_cycles: 1,
        })?;

        let writes = daily_writes(&conn, Some("drive_reset"), 7)?;
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].active_bytes, 50 * 512_000);
        assert_eq!(writes[0].gap_bytes, 0);
        assert_eq!(writes[0].total_bytes(), 50 * 512_000);

        Ok(())
    }

    #[test]
    fn test_gap_detection_over_threshold_attributed_to_gap_bytes() -> Result<()> {
        let conn = create_test_db();
        let now = chrono::Local::now().timestamp();

        // Snapshot 0: baseline
        insert_snapshot(&conn, "drive_gap", &SmartSnapshot {
            ts: now - 3000,
            data_units_written: 100,
            data_units_read: 50,
            percent_used: 1,
            available_spare: 100,
            power_on_hours: 10,
            power_cycles: 1,
        })?;

        // Snapshot 1: 300s later (<= 900s) -> active writes
        insert_snapshot(&conn, "drive_gap", &SmartSnapshot {
            ts: now - 2700,
            data_units_written: 150, // +50 units active
            data_units_read: 50,
            percent_used: 1,
            available_spare: 100,
            power_on_hours: 10,
            power_cycles: 1,
        })?;

        // Snapshot 2: 2400s later (> 900s GAP_THRESHOLD_SECS) -> gap writes (while Estus wasn't running)
        insert_snapshot(&conn, "drive_gap", &SmartSnapshot {
            ts: now - 300,
            data_units_written: 250, // +100 units gap
            data_units_read: 50,
            percent_used: 1,
            available_spare: 100,
            power_on_hours: 10,
            power_cycles: 1,
        })?;

        // Snapshot 3: 300s later (<= 900s) -> active writes again
        insert_snapshot(&conn, "drive_gap", &SmartSnapshot {
            ts: now,
            data_units_written: 290, // +40 units active
            data_units_read: 50,
            percent_used: 1,
            available_spare: 100,
            power_on_hours: 10,
            power_cycles: 1,
        })?;

        let writes = daily_writes(&conn, Some("drive_gap"), 7)?;
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].active_bytes, (50 + 40) * 512_000);
        assert_eq!(writes[0].gap_bytes, 100 * 512_000);
        assert_eq!(writes[0].total_bytes(), 190 * 512_000);

        Ok(())
    }

    #[test]
    fn test_day_boundary_using_local_timezone() -> Result<()> {
        let conn = create_test_db();
        let today = chrono::Local::now().date_naive();
        let yesterday = today - chrono::Duration::days(1);

        let dt_yesterday_late = yesterday
            .and_hms_opt(23, 55, 0)
            .unwrap()
            .and_local_timezone(chrono::Local)
            .single()
            .expect("local datetime unambiguous")
            .timestamp();
        let dt_yesterday_base = dt_yesterday_late - 300;

        let dt_today_early = today
            .and_hms_opt(0, 5, 0)
            .unwrap()
            .and_local_timezone(chrono::Local)
            .single()
            .expect("local datetime unambiguous")
            .timestamp();

        // Snapshot on yesterday evening (baseline)
        insert_snapshot(&conn, "drive_tz", &SmartSnapshot {
            ts: dt_yesterday_base,
            data_units_written: 1000,
            data_units_read: 500,
            percent_used: 1,
            available_spare: 100,
            power_on_hours: 10,
            power_cycles: 1,
        })?;

        // Snapshot on yesterday evening (+50 units)
        insert_snapshot(&conn, "drive_tz", &SmartSnapshot {
            ts: dt_yesterday_late,
            data_units_written: 1050,
            data_units_read: 500,
            percent_used: 1,
            available_spare: 100,
            power_on_hours: 10,
            power_cycles: 1,
        })?;

        // Snapshot across midnight on today early (+80 units, 600s after previous <= 900s)
        insert_snapshot(&conn, "drive_tz", &SmartSnapshot {
            ts: dt_today_early,
            data_units_written: 1130,
            data_units_read: 500,
            percent_used: 1,
            available_spare: 100,
            power_on_hours: 10,
            power_cycles: 1,
        })?;

        let writes = daily_writes(&conn, Some("drive_tz"), 7)?;
        let y_entry = writes.iter().find(|dw| dw.date == yesterday).expect("yesterday entry");
        assert_eq!(y_entry.active_bytes, 50 * 512_000);
        assert_eq!(y_entry.gap_bytes, 0);

        let t_entry = writes.iter().find(|dw| dw.date == today).expect("today entry");
        assert_eq!(t_entry.active_bytes, 80 * 512_000);
        assert_eq!(t_entry.gap_bytes, 0);

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

        let deleted = prune_old_records(&conn, 30)?;
        assert_eq!(deleted, 2);

        let top = top_processes(&conn, 0, 10)?;
        assert_eq!(top.len(), 2);
        assert!(top.iter().any(|(name, _)| name == "recent"));
        assert!(top.iter().any(|(name, _)| name == "now"));

        Ok(())
    }

    #[test]
    fn test_duplicate_snapshot_ignored_without_modification() -> Result<()> {
        let conn = create_test_db();

        let s1 = SmartSnapshot {
            ts: 1_700_000_000,
            data_units_written: 100,
            data_units_read: 50,
            percent_used: 1,
            available_spare: 100,
            power_on_hours: 10,
            power_cycles: 2,
        };

        // First insertion should succeed
        insert_snapshot(&conn, "drive1", &s1)?;

        let retrieved1 = latest_snapshot(&conn, "drive1")?.expect("snapshot should exist");
        assert_eq!(retrieved1.ts, 1_700_000_000);
        assert_eq!(retrieved1.data_units_written, 100);

        // Attempt second insertion with exact same (drive_id, ts) but modified data
        let s2 = SmartSnapshot {
            ts: 1_700_000_000,
            data_units_written: 999, // Should NOT overwrite or bump timestamp
            data_units_read: 999,
            percent_used: 5,
            available_spare: 90,
            power_on_hours: 20,
            power_cycles: 4,
        };

        // Second insertion must succeed without error (ignored)
        insert_snapshot(&conn, "drive1", &s2)?;

        // Verify total row count for drive1 is still 1
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM ssd_snapshots WHERE drive_id = 'drive1'",
            [],
            |r| r.get(0),
        )?;
        assert_eq!(count, 1, "Duplicate snapshot must be ignored, not inserted");

        // Verify data was NOT overwritten
        let retrieved2 = latest_snapshot(&conn, "drive1")?.expect("snapshot should exist");
        assert_eq!(retrieved2.ts, 1_700_000_000, "Timestamp must not be bumped");
        assert_eq!(retrieved2.data_units_written, 100, "Original snapshot must be retained");

        // Inserting the same timestamp for a DIFFERENT drive must succeed
        insert_snapshot(&conn, "drive2", &s2)?;
        let retrieved_d2 = latest_snapshot(&conn, "drive2")?.expect("drive2 snapshot should exist");
        assert_eq!(retrieved_d2.ts, 1_700_000_000, "Different drive with same ts must succeed");
        assert_eq!(retrieved_d2.data_units_written, 999);

        let total_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM ssd_snapshots",
            [],
            |r| r.get(0),
        )?;
        assert_eq!(total_count, 2);

        Ok(())
    }

    #[test]
    fn test_get_db_path() -> Result<()> {
        let path = get_db_path()?;
        assert!(path.ends_with("estus.db"));
        assert!(path.parent().unwrap().exists());
        Ok(())
    }

    #[test]
    fn test_migration_to_version_2() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        // Manually create schema at user_version = 1 with legacy process writes
        conn.execute_batch(
            "PRAGMA user_version = 1;
             CREATE TABLE process_writes (
                 ts INTEGER NOT NULL,
                 process_name TEXT NOT NULL,
                 bytes_delta INTEGER NOT NULL
             );
             CREATE TABLE ssd_snapshots (
                 ts INTEGER NOT NULL,
                 drive_id TEXT,
                 data_units_written INTEGER NOT NULL,
                 data_units_read INTEGER NOT NULL,
                 percent_used INTEGER NOT NULL,
                 available_spare INTEGER NOT NULL,
                 power_on_hours INTEGER NOT NULL,
                 power_cycles INTEGER NOT NULL
             );
             INSERT INTO process_writes (ts, process_name, bytes_delta) VALUES (1000, 'legacy.exe', 999999);
             INSERT INTO ssd_snapshots (ts, drive_id, data_units_written, data_units_read, percent_used, available_spare, power_on_hours, power_cycles)
             VALUES (1000, 'drive1', 100, 50, 1, 100, 10, 1);"
        )?;

        let count_procs_before: i64 = conn.query_row("SELECT COUNT(*) FROM process_writes", [], |r| r.get(0))?;
        assert_eq!(count_procs_before, 1);
        let count_snaps_before: i64 = conn.query_row("SELECT COUNT(*) FROM ssd_snapshots", [], |r| r.get(0))?;
        assert_eq!(count_snaps_before, 1);

        // Run init_schema which triggers migration to version 2
        init_schema(&conn)?;

        // Verify version bumped to 2
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        assert_eq!(version, 2);

        // Verify legacy process writes were purged
        let count_procs_after: i64 = conn.query_row("SELECT COUNT(*) FROM process_writes", [], |r| r.get(0))?;
        assert_eq!(count_procs_after, 0);

        // Verify snapshots are intact
        let count_snaps_after: i64 = conn.query_row("SELECT COUNT(*) FROM ssd_snapshots", [], |r| r.get(0))?;
        assert_eq!(count_snaps_after, 1);

        // Insert new valid process write
        conn.execute("INSERT INTO process_writes (ts, process_name, bytes_delta) VALUES (2000, 'valid.exe', 500)", [])?;

        // Re-running init_schema must NOT purge valid process writes since version is already >= 2
        init_schema(&conn)?;
        let count_procs_final: i64 = conn.query_row("SELECT COUNT(*) FROM process_writes", [], |r| r.get(0))?;
        assert_eq!(count_procs_final, 1);

        Ok(())
    }

    #[test]
    fn test_reset_history() -> Result<()> {
        let conn = create_test_db();
        let now = chrono::Utc::now().timestamp();

        // Populate process writes
        insert_process_deltas(&conn, now - 1000, &[("proc1.exe".to_string(), 5000)])?;
        insert_process_deltas(&conn, now - 500, &[("proc2.exe".to_string(), 10000)])?;
        let proc_count: i64 = conn.query_row("SELECT COUNT(*) FROM process_writes", [], |r| r.get(0))?;
        assert_eq!(proc_count, 2);

        // Populate multiple snapshots for drive1 and drive2
        insert_snapshot(&conn, "drive1", &SmartSnapshot {
            ts: now - 3000,
            data_units_written: 1000,
            data_units_read: 500,
            percent_used: 1,
            available_spare: 100,
            power_on_hours: 10,
            power_cycles: 1,
        })?;
        insert_snapshot(&conn, "drive1", &SmartSnapshot {
            ts: now - 2000,
            data_units_written: 1050,
            data_units_read: 520,
            percent_used: 1,
            available_spare: 100,
            power_on_hours: 11,
            power_cycles: 1,
        })?;
        insert_snapshot(&conn, "drive1", &SmartSnapshot {
            ts: now - 1000,
            data_units_written: 1120, // Latest for drive1
            data_units_read: 550,
            percent_used: 1,
            available_spare: 100,
            power_on_hours: 12,
            power_cycles: 1,
        })?;

        insert_snapshot(&conn, "drive2", &SmartSnapshot {
            ts: now - 2500,
            data_units_written: 2000,
            data_units_read: 1000,
            percent_used: 2,
            available_spare: 99,
            power_on_hours: 50,
            power_cycles: 2,
        })?;
        insert_snapshot(&conn, "drive2", &SmartSnapshot {
            ts: now - 500,
            data_units_written: 2080, // Latest for drive2
            data_units_read: 1040,
            percent_used: 2,
            available_spare: 99,
            power_on_hours: 51,
            power_cycles: 2,
        })?;

        let snap_count_before: i64 = conn.query_row("SELECT COUNT(*) FROM ssd_snapshots", [], |r| r.get(0))?;
        assert_eq!(snap_count_before, 5);

        // Execute reset_history
        reset_history(&conn)?;

        // Verify process_writes is completely emptied
        let proc_count_after: i64 = conn.query_row("SELECT COUNT(*) FROM process_writes", [], |r| r.get(0))?;
        assert_eq!(proc_count_after, 0);

        // Verify ssd_snapshots has exactly 2 records remaining (1 latest per drive)
        let snap_count_after: i64 = conn.query_row("SELECT COUNT(*) FROM ssd_snapshots", [], |r| r.get(0))?;
        assert_eq!(snap_count_after, 2);

        let d1_snap = latest_snapshot(&conn, "drive1")?.expect("drive1 snapshot");
        assert_eq!(d1_snap.ts, now - 1000);
        assert_eq!(d1_snap.data_units_written, 1120);

        let d2_snap = latest_snapshot(&conn, "drive2")?.expect("drive2 snapshot");
        assert_eq!(d2_snap.ts, now - 500);
        assert_eq!(d2_snap.data_units_written, 2080);

        // daily_writes immediately after reset should be empty since only 1 baseline snapshot exists per drive
        let dw = daily_writes(&conn, None, 30)?;
        assert!(dw.is_empty(), "Immediately after reset, daily writes should have no delta");

        // Subsequent snapshot computes writes starting from the retained baseline
        insert_snapshot(&conn, "drive1", &SmartSnapshot {
            ts: now,
            data_units_written: 1150, // +30 units from 1120 baseline
            data_units_read: 560,
            percent_used: 1,
            available_spare: 100,
            power_on_hours: 13,
            power_cycles: 1,
        })?;
        let dw_after = daily_writes(&conn, Some("drive1"), 30)?;
        assert_eq!(dw_after.len(), 1);
        assert_eq!(dw_after[0].total_bytes(), 30 * 512_000);

        // Calling reset_history on a single-snapshot db or empty db should succeed
        reset_history(&conn)?;
        let snap_count_after_second_reset: i64 = conn.query_row("SELECT COUNT(*) FROM ssd_snapshots", [], |r| r.get(0))?;
        assert_eq!(snap_count_after_second_reset, 2);

        Ok(())
    }

    #[test]
    fn test_get_intervals_24h_empty_db() -> Result<()> {
        let conn = create_test_db();

        let intervals_drive = get_intervals_24h(&conn, Some("drive1"))?;
        assert!(intervals_drive.is_empty(), "Empty DB must return empty intervals for specific drive");

        let intervals_all = get_intervals_24h(&conn, None)?;
        assert!(intervals_all.is_empty(), "Empty DB must return empty intervals for All drives");

        Ok(())
    }

    #[test]
    fn test_get_intervals_24h_normal_and_baseline() -> Result<()> {
        let conn = create_test_db();
        let now = chrono::Utc::now().timestamp();

        // Snapshot 1: baseline (no prior snapshot)
        insert_snapshot(&conn, "drive1", &SmartSnapshot {
            ts: now - 600,
            data_units_written: 1000,
            data_units_read: 500,
            percent_used: 1,
            available_spare: 100,
            power_on_hours: 10,
            power_cycles: 1,
        })?;

        // Snapshot 2: 300s later (< 900s, normal interval)
        // 100 units written = 100 * 512_000 / 1e9 = 0.0512 GB
        insert_snapshot(&conn, "drive1", &SmartSnapshot {
            ts: now - 300,
            data_units_written: 1100,
            data_units_read: 550,
            percent_used: 1,
            available_spare: 100,
            power_on_hours: 10,
            power_cycles: 1,
        })?;

        let intervals = get_intervals_24h(&conn, Some("drive1"))?;
        assert_eq!(intervals.len(), 2);

        // First snapshot has no prior snapshot -> baseline
        assert_eq!(intervals[0].timestamp, now - 600);
        assert_eq!(intervals[0].raw_data_units_written, 1000);
        assert_eq!(intervals[0].delta_gb, 0.0);
        assert_eq!(intervals[0].attribution, "baseline");

        // Second snapshot has delta 0.0512 GB and local date attribution
        assert_eq!(intervals[1].timestamp, now - 300);
        assert_eq!(intervals[1].raw_data_units_written, 1100);
        assert!((intervals[1].delta_gb - 0.0512).abs() < 1e-6);

        let expected_date = chrono::DateTime::from_timestamp(now - 300, 0)
            .unwrap()
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d")
            .to_string();
        assert_eq!(intervals[1].attribution, expected_date);

        Ok(())
    }

    #[test]
    fn test_get_intervals_24h_gap_interval() -> Result<()> {
        let conn = create_test_db();
        let now = chrono::Utc::now().timestamp();

        // Snapshot 1: baseline
        insert_snapshot(&conn, "drive1", &SmartSnapshot {
            ts: now - 2500,
            data_units_written: 2000,
            data_units_read: 1000,
            percent_used: 2,
            available_spare: 99,
            power_on_hours: 20,
            power_cycles: 2,
        })?;

        // Snapshot 2: 2000s later (> 900s GAP_THRESHOLD_SECS -> "app not running")
        // 200 units = 200 * 512_000 / 1e9 = 0.1024 GB
        insert_snapshot(&conn, "drive1", &SmartSnapshot {
            ts: now - 500,
            data_units_written: 2200,
            data_units_read: 1100,
            percent_used: 2,
            available_spare: 99,
            power_on_hours: 20,
            power_cycles: 2,
        })?;

        let intervals = get_intervals_24h(&conn, Some("drive1"))?;
        assert_eq!(intervals.len(), 2);

        assert_eq!(intervals[0].attribution, "baseline");
        assert_eq!(intervals[1].attribution, "app not running");
        assert!((intervals[1].delta_gb - 0.1024).abs() < 1e-6);

        Ok(())
    }

    #[test]
    fn test_get_intervals_24h_preceding_baseline() -> Result<()> {
        let conn = create_test_db();
        let now = chrono::Utc::now().timestamp();

        // Baseline snapshot preceding the 24-hour cutoff (now - 86,400s)
        insert_snapshot(&conn, "drive1", &SmartSnapshot {
            ts: now - 90_000,
            data_units_written: 5000,
            data_units_read: 2500,
            percent_used: 5,
            available_spare: 98,
            power_on_hours: 100,
            power_cycles: 10,
        })?;

        // First snapshot within the 24h window (now - 85,000s).
        // Preceding snapshot at now - 90_000s is used as baseline.
        // Elapsed = 5000s > 900s -> "app not running", delta = 50 units (0.0256 GB)
        insert_snapshot(&conn, "drive1", &SmartSnapshot {
            ts: now - 85_000,
            data_units_written: 5050,
            data_units_read: 2525,
            percent_used: 5,
            available_spare: 98,
            power_on_hours: 101,
            power_cycles: 10,
        })?;

        // Second snapshot within 24h window (now - 84,500s).
        // Elapsed = 500s <= 900s -> local date, delta = 50 units (0.0256 GB)
        insert_snapshot(&conn, "drive1", &SmartSnapshot {
            ts: now - 84_500,
            data_units_written: 5100,
            data_units_read: 2550,
            percent_used: 5,
            available_spare: 98,
            power_on_hours: 101,
            power_cycles: 10,
        })?;

        let intervals = get_intervals_24h(&conn, Some("drive1"))?;
        // Only snapshots >= cutoff are returned in the 24h intervals list
        assert_eq!(intervals.len(), 2);

        assert_eq!(intervals[0].timestamp, now - 85_000);
        assert_eq!(intervals[0].raw_data_units_written, 5050);
        assert!((intervals[0].delta_gb - 0.0256).abs() < 1e-6);
        assert_eq!(intervals[0].attribution, "app not running");

        assert_eq!(intervals[1].timestamp, now - 84_500);
        assert_eq!(intervals[1].raw_data_units_written, 5100);
        assert!((intervals[1].delta_gb - 0.0256).abs() < 1e-6);
        assert_ne!(intervals[1].attribution, "app not running");
        assert_ne!(intervals[1].attribution, "baseline");

        Ok(())
    }

    #[test]
    fn test_get_intervals_24h_counter_reset() -> Result<()> {
        let conn = create_test_db();
        let now = chrono::Utc::now().timestamp();

        // Snapshot 1: baseline
        insert_snapshot(&conn, "drive1", &SmartSnapshot {
            ts: now - 600,
            data_units_written: 10000,
            data_units_read: 5000,
            percent_used: 3,
            available_spare: 95,
            power_on_hours: 50,
            power_cycles: 5,
        })?;

        // Snapshot 2: counter reset / drive swap (written drops to 500)
        insert_snapshot(&conn, "drive1", &SmartSnapshot {
            ts: now - 300,
            data_units_written: 500,
            data_units_read: 250,
            percent_used: 1,
            available_spare: 100,
            power_on_hours: 1,
            power_cycles: 1,
        })?;

        let intervals = get_intervals_24h(&conn, Some("drive1"))?;
        assert_eq!(intervals.len(), 2);

        assert_eq!(intervals[0].attribution, "baseline");
        assert_eq!(intervals[0].delta_gb, 0.0);

        // Counter reset must produce delta_gb = 0.0
        assert_eq!(intervals[1].raw_data_units_written, 500);
        assert_eq!(intervals[1].delta_gb, 0.0);

        Ok(())
    }
}
