use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

/// Periodically collects per-process disk write metrics using minimal sysinfo overhead.
pub struct ProcCollector {
    pub prev_readings: HashMap<Pid, u64>,
    pub sys: System,
}

impl ProcCollector {
    /// Creates a new `ProcCollector` and takes an initial baseline reading
    /// of all currently running processes so subsequent polls measure true deltas.
    pub fn new() -> Self {
        let mut sys = System::new().expect("failed to initialize sysinfo System");
        let refresh_kind = ProcessRefreshKind::nothing().with_disk_usage();
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh_kind);

        let mut prev_readings = HashMap::with_capacity(sys.processes().len());
        for (pid, proc) in sys.processes() {
            prev_readings.insert(*pid, proc.disk_usage().total_written_bytes);
        }

        Self {
            prev_readings,
            sys,
        }
    }

    /// Refreshes processes disk usage and computes write byte deltas aggregated by process name.
    ///
    /// Negligible CPU overhead is achieved by refreshing ONLY disk usage via
    /// `ProcessRefreshKind::nothing().with_disk_usage()`.
    ///
    /// Returns a list of `(process_name, delta_bytes)` sorted descending by bytes written.
    /// Only processes with `delta > 0` are returned.
    pub fn poll_deltas(&mut self) -> Vec<(String, u64)> {
        let refresh_kind = ProcessRefreshKind::nothing().with_disk_usage();
        // remove_dead_processes = true
        self.sys
            .refresh_processes_specifics(ProcessesToUpdate::All, true, refresh_kind);

        let current_readings: Vec<(Pid, String, u64)> = self
            .sys
            .processes()
            .iter()
            .map(|(&pid, proc)| {
                (
                    pid,
                    proc.name().to_string_lossy().to_string(),
                    proc.disk_usage().total_written_bytes,
                )
            })
            .collect();

        // Prune exited processes from prev_readings to prevent memory unbounded growth
        let active_pids: std::collections::HashSet<Pid> =
            self.sys.processes().keys().copied().collect();
        self.prev_readings.retain(|pid, _| active_pids.contains(pid));

        calculate_deltas(&mut self.prev_readings, &current_readings)
    }

    /// Returns a reference to the previous readings map.
    pub fn prev_readings(&self) -> &HashMap<Pid, u64> {
        &self.prev_readings
    }
}

impl Default for ProcCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// Computes deltas between `prev_readings` and `current_readings`, aggregates
/// them by process name, and filters out zero or negative deltas.
///
/// Rules:
/// - `delta = current_written - prev_written`
/// - If a PID appears for the first time, its baseline is recorded into `prev_readings`
///   and no delta is emitted (treated as 0 delta / skipped).
/// - If `current_written < prev_written` (PID reuse or counter wrap), the delta is ignored
///   and `prev_readings` is reset to the current reading.
/// - Deltas are aggregated by process name so multiple threads / instances sum together.
/// - Processes with `delta == 0` are omitted to save storage and keep overhead minimal.
pub fn calculate_deltas(
    prev_readings: &mut HashMap<Pid, u64>,
    current_readings: &[(Pid, String, u64)],
) -> Vec<(String, u64)> {
    let mut aggregated: HashMap<String, u64> = HashMap::new();

    for (pid, name, current_written) in current_readings {
        if let Some(&prev_written) = prev_readings.get(pid)
            && *current_written > prev_written {
                let delta = *current_written - prev_written;
                *aggregated.entry(name.clone()).or_insert(0) += delta;
            }
        // Update baseline for next iteration (handles normal increase, reset on PID reuse, or new PID)
        prev_readings.insert(*pid, *current_written);
    }

    let mut result: Vec<(String, u64)> = aggregated
        .into_iter()
        .filter(|(_, delta)| *delta > 0)
        .collect();

    // Sort descending by bytes written, then by process name for determinism
    result.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    result
}

/// Convenience function that refreshes `sys`, reads disk usage, and updates `prev_readings`.
pub fn collect_process_deltas(
    sys: &mut System,
    prev_readings: &mut HashMap<Pid, u64>,
) -> Vec<(String, u64)> {
    let refresh_kind = ProcessRefreshKind::nothing().with_disk_usage();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh_kind);

    let current_readings: Vec<(Pid, String, u64)> = sys
        .processes()
        .iter()
        .map(|(&pid, proc)| {
            (
                pid,
                proc.name().to_string_lossy().to_string(),
                proc.disk_usage().total_written_bytes,
            )
        })
        .collect();

    let active_pids: std::collections::HashSet<Pid> = sys.processes().keys().copied().collect();
    prev_readings.retain(|pid, _| active_pids.contains(pid));

    calculate_deltas(prev_readings, &current_readings)
}

/// Performs a single collection step: polls process write deltas and, if a database
/// connection is supplied, batch-inserts them into the SQLite database.
pub fn step_proc_collector(
    collector: &mut ProcCollector,
    conn: Option<&rusqlite::Connection>,
) -> anyhow::Result<Vec<(String, u64)>> {
    let deltas = collector.poll_deltas();
    if let Some(c) = conn
        && !deltas.is_empty() {
            let ts = chrono::Utc::now().timestamp();
            crate::db::insert_process_deltas(c, ts, &deltas)?;
        }
    Ok(deltas)
}

/// Runs the background process monitoring loop every `interval` (e.g. 30 seconds),
/// saving write deltas to SQLite until `shutdown_signal` is set.
pub fn run_proc_collector_loop(
    conn: Arc<Mutex<rusqlite::Connection>>,
    shutdown_signal: Arc<AtomicBool>,
    interval: Duration,
) -> anyhow::Result<()> {
    let mut collector = ProcCollector::new();

    while !shutdown_signal.load(Ordering::Relaxed) {
        // Sleep in small increments for responsive shutdown
        let start = std::time::Instant::now();
        while start.elapsed() < interval {
            if shutdown_signal.load(Ordering::Relaxed) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(250));
        }

        let deltas = collector.poll_deltas();
        if !deltas.is_empty() {
            let ts = chrono::Utc::now().timestamp();
            if let Ok(c) = conn.lock() {
                let _ = crate::db::insert_process_deltas(&c, ts, &deltas);
            }
        }
    }
    Ok(())
}

/// Spawns a background thread running `run_proc_collector_loop`.
pub fn spawn_proc_monitor_thread(
    conn: Arc<Mutex<rusqlite::Connection>>,
    shutdown_signal: Arc<AtomicBool>,
    interval: Duration,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("ssd-watch-proc-monitor".to_string())
        .spawn(move || {
            if let Err(e) = run_proc_collector_loop(conn, shutdown_signal, interval) {
                eprintln!("Process monitor thread encountered error: {e}");
            }
        })
        .expect("failed to spawn proc monitor thread")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normal_delta_increase() {
        let mut prev = HashMap::new();
        let pid1 = Pid::from(101);
        let pid2 = Pid::from(102);

        prev.insert(pid1, 1_000);
        prev.insert(pid2, 2_000);

        let current = vec![
            (pid1, "editor.exe".to_string(), 1_500),
            (pid2, "browser.exe".to_string(), 2_800),
        ];

        let deltas = calculate_deltas(&mut prev, &current);

        assert_eq!(deltas.len(), 2);
        // Sorted descending by bytes: browser.exe (800), editor.exe (500)
        assert_eq!(deltas[0], ("browser.exe".to_string(), 800));
        assert_eq!(deltas[1], ("editor.exe".to_string(), 500));

        // Baseline in prev_readings should be updated
        assert_eq!(prev.get(&pid1), Some(&1_500));
        assert_eq!(prev.get(&pid2), Some(&2_800));
    }

    #[test]
    fn test_pid_reuse_resets_and_ignores_negative_delta() {
        let mut prev = HashMap::new();
        let pid1 = Pid::from(200);

        // Previous process had written 10 MB
        prev.insert(pid1, 10_000_000);

        // Process terminated, new process got reused PID 200 and has only written 500 bytes
        let current = vec![(pid1, "new_process.exe".to_string(), 500)];

        let deltas = calculate_deltas(&mut prev, &current);

        // Negative delta should be skipped / ignored, so result is empty
        assert!(deltas.is_empty());

        // Baseline should be reset to the new process's current reading
        assert_eq!(prev.get(&pid1), Some(&500));

        // On subsequent poll, delta should be computed against the new baseline
        let next_poll = vec![(pid1, "new_process.exe".to_string(), 1_200)];
        let deltas2 = calculate_deltas(&mut prev, &next_poll);
        assert_eq!(deltas2, vec![("new_process.exe".to_string(), 700)]);
        assert_eq!(prev.get(&pid1), Some(&1_200));
    }

    #[test]
    fn test_multiple_processes_same_name_aggregating() {
        let mut prev = HashMap::new();
        let p1 = Pid::from(10);
        let p2 = Pid::from(20);
        let p3 = Pid::from(30);

        prev.insert(p1, 100);
        prev.insert(p2, 200);
        prev.insert(p3, 500);

        // p1 and p2 both share the name "chrome.exe", p3 is "explorer.exe"
        let current = vec![
            (p1, "chrome.exe".to_string(), 350),  // delta = 250
            (p2, "chrome.exe".to_string(), 450),  // delta = 250
            (p3, "explorer.exe".to_string(), 600), // delta = 100
        ];

        let deltas = calculate_deltas(&mut prev, &current);

        // chrome.exe: 250 + 250 = 500
        // explorer.exe: 100
        assert_eq!(deltas.len(), 2);
        assert_eq!(deltas[0], ("chrome.exe".to_string(), 500));
        assert_eq!(deltas[1], ("explorer.exe".to_string(), 100));
    }

    #[test]
    fn test_new_pid_appearing_for_first_time() {
        let mut prev = HashMap::new();
        let pid = Pid::from(999);

        // New PID not present in prev_readings
        let current = vec![(pid, "background_service.exe".to_string(), 50_000)];

        let deltas = calculate_deltas(&mut prev, &current);

        // Since there is no prior baseline, delta is 0 / ignored to avoid false write spike
        assert!(deltas.is_empty());

        // Baseline should now be stored in prev
        assert_eq!(prev.get(&pid), Some(&50_000));

        // Next poll computes the delta accurately
        let next = vec![(pid, "background_service.exe".to_string(), 55_000)];
        let deltas2 = calculate_deltas(&mut prev, &next);
        assert_eq!(deltas2, vec![("background_service.exe".to_string(), 5_000)]);
    }

    #[test]
    fn test_zero_delta_filtered_out() {
        let mut prev = HashMap::new();
        let pid1 = Pid::from(100);
        let pid2 = Pid::from(101);

        prev.insert(pid1, 1_000);
        prev.insert(pid2, 2_000);

        let current = vec![
            (pid1, "idle.exe".to_string(), 1_000),      // delta = 0
            (pid2, "active.exe".to_string(), 2_100),    // delta = 100
        ];

        let deltas = calculate_deltas(&mut prev, &current);

        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0], ("active.exe".to_string(), 100));
    }

    #[test]
    fn test_proc_collector_live_instantiation() {
        let mut collector = ProcCollector::new();
        let deltas = collector.poll_deltas();
        // Should execute without panicking; deltas could be empty or non-empty depending on system activity
        println!("Collected {} process deltas in test", deltas.len());
    }
}
