use anyhow::Result;
use chrono::NaiveDate;
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};

use crate::smart::{DriveInfo, SmartSnapshot};

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
             ts INTEGER PRIMARY KEY,
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

    // Check if drive_id column exists in ssd_snapshots
    let mut stmt = conn.prepare("PRAGMA table_info(ssd_snapshots)")?;
    let columns = stmt.query_map([], |row| {
        let name: String = row.get(1)?;
        Ok(name)
    })?;
    let col_names: Vec<String> = columns.filter_map(|r| r.ok()).collect();

    if !col_names.iter().any(|c| c == "drive_id") {
        conn.execute("ALTER TABLE ssd_snapshots ADD COLUMN drive_id TEXT", [])?;
    }

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_ssd_snapshots_drive_ts ON ssd_snapshots(drive_id, ts)",
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

    // Assign existing snapshots with NULL or empty drive_id
    assign_orphan_snapshots(conn)?;

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

/// Inserts or replaces a SMART snapshot record for a specific drive.
/// If `ts` is already occupied by a different drive, resolves collision by adjusting timestamp.
pub fn insert_snapshot(conn: &Connection, drive_id: &str, snapshot: &SmartSnapshot) -> Result<()> {
    let mut actual_ts = snapshot.ts;
    loop {
        let existing_drive: Option<String> = conn
            .query_row(
                "SELECT drive_id FROM ssd_snapshots WHERE ts = ?1",
                params![actual_ts],
                |r| r.get(0),
            )
            .ok();

        match existing_drive {
            Some(ref existing) if existing != drive_id => {
                actual_ts += 1;
            }
            _ => break,
        }
    }

    conn.execute(
        "INSERT OR REPLACE INTO ssd_snapshots (
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
            actual_ts,
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

/// Computes daily total bytes written over the last `days` days.
/// If `drive_id` is `Some(id)`, returns daily writes for that drive based on SMART snapshots.
/// If `drive_id` is `None`, aggregates and sums writes across all drives by date.
pub fn daily_writes(
    conn: &Connection,
    drive_id: Option<&str>,
    days: u32,
) -> Result<Vec<(NaiveDate, u64)>> {
    if days == 0 {
        return Ok(Vec::new());
    }

    let now = chrono::Utc::now().timestamp();
    let cutoff_ts = now - (days as i64) * 86_400;

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

    let mut per_day: std::collections::BTreeMap<NaiveDate, u64> = std::collections::BTreeMap::new();
    let mut has_snapshot_data = false;

    for id in &drive_ids {
        let mut stmt = conn.prepare(
            "SELECT ts, data_units_written
             FROM ssd_snapshots
             WHERE drive_id = ?1 AND ts >= ?2
             ORDER BY ts ASC"
        )?;
        let rows = stmt.query_map(params![id, cutoff_ts - 86_400], |r| {
            let ts: i64 = r.get(0)?;
            let written: i64 = r.get(1)?;
            Ok((ts, written as u64))
        })?;

        let mut prev_written: Option<u64> = None;
        for row in rows {
            let (ts, written) = row?;
            has_snapshot_data = true;
            if let Some(prev) = prev_written {
                let delta_units = written.saturating_sub(prev);
                let delta_bytes = delta_units.saturating_mul(512_000);
                if let Some(dt) = chrono::DateTime::from_timestamp(ts, 0) {
                    let date = dt.with_timezone(&chrono::Local).date_naive();
                    *per_day.entry(date).or_insert(0) += delta_bytes;
                }
            }
            prev_written = Some(written);
        }
    }

    if has_snapshot_data && !per_day.is_empty() {
        let today = chrono::Local::now().date_naive();
        let cutoff_date = today - chrono::Duration::days(days as i64);
        let res: Vec<(NaiveDate, u64)> = per_day
            .into_iter()
            .filter(|(d, _)| *d >= cutoff_date)
            .collect();
        return Ok(res);
    }

    // Fallback: If drive_id is None (or snapshots empty), aggregate process_writes
    if drive_id.is_none() {
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
        return Ok(result);
    }

    Ok(Vec::new())
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
    fn test_daily_writes_aggregation() -> Result<()> {
        let conn = create_test_db();

        let now = chrono::Utc::now().timestamp();
        let day_sec = 86_400;

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

        let writes = daily_writes(&conn, None, 3)?;
        assert!(!writes.is_empty());
        for i in 1..writes.len() {
            assert!(writes[i].0 >= writes[i - 1].0);
        }

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
        let today_d1 = d1_writes.last().unwrap().1;
        assert_eq!(today_d1, 100 * 512_000);

        // Query drive 2 specifically
        let d2_writes = daily_writes(&conn, Some("drive2"), 3)?;
        assert!(!d2_writes.is_empty());
        let today_d2 = d2_writes.last().unwrap().1;
        assert_eq!(today_d2, 100 * 512_000);

        // Query aggregated ("All drives")
        let all_writes = daily_writes(&conn, None, 3)?;
        let today_all = all_writes.last().unwrap().1;
        assert_eq!(today_all, (100 + 100) * 512_000);

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
    fn test_get_db_path() -> Result<()> {
        let path = get_db_path()?;
        assert!(path.ends_with("estus.db"));
        assert!(path.parent().unwrap().exists());
        Ok(())
    }
}
