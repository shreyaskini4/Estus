use chrono::NaiveDate;

/// Standard decimal gigabyte: 1,000,000,000 bytes (storage industry standard matching NVMe SMART specifications).
pub const BYTES_PER_GB: f64 = 1_000_000_000.0;

/// Calculate average GB/day over the last 7 days (or up to 7 days if fewer days of data exist).
///
/// Returns 0.0 if `daily_writes` is empty.
pub fn average_daily_writes_gb(daily_writes: &[(NaiveDate, u64)]) -> f64 {
    if daily_writes.is_empty() {
        return 0.0;
    }

    let mut sorted = daily_writes.to_vec();
    sorted.sort_by_key(|(date, _)| *date);

    let count = sorted.len().min(7);
    let start_idx = sorted.len() - count;
    let recent = &sorted[start_idx..];

    let total_bytes: f64 = recent.iter().map(|(_, bytes)| *bytes as f64).sum();
    (total_bytes / count as f64) / BYTES_PER_GB
}

/// Projected days until 100% `percentage_used` using the latest `percent_used` and the recent write rate (`avg_daily_bytes`).
///
/// Returns:
/// - `Some(0)` if `current_percent_used >= 100` (already depleted/exceeded).
/// - `None` if `avg_daily_bytes == 0` (indefinite lifespan / no writes occurring).
/// - `None` if `current_percent_used == 0` (brand new drive, cannot estimate wear rate without baseline wear).
/// - `Some(remaining_days)` based on remaining percentage and observed wear rate.
pub fn projected_days_to_depletion(
    current_percent_used: u8,
    avg_daily_bytes: u64,
    total_host_bytes_written: u64,
) -> Option<u64> {
    if current_percent_used >= 100 {
        return Some(0);
    }

    if avg_daily_bytes == 0 || current_percent_used == 0 {
        return None;
    }

    let remaining_percent = 100u64.saturating_sub(current_percent_used as u64);

    // Use u128 to prevent integer overflow during multiplication.
    let remaining_bytes = (total_host_bytes_written as u128 * remaining_percent as u128)
        / (current_percent_used as u128);

    let remaining_days = remaining_bytes / (avg_daily_bytes as u128);

    Some(remaining_days.min(u64::MAX as u128) as u64)
}

/// Returns GB written per power-on hour.
///
/// Returns 0.0 if `power_on_hours == 0`.
pub fn writes_per_power_on_hour_gb(total_bytes_written: u64, power_on_hours: u64) -> f64 {
    if power_on_hours == 0 {
        return 0.0;
    }

    (total_bytes_written as f64 / BYTES_PER_GB) / (power_on_hours as f64)
}

/// Returns the lifetime average daily write rate in GB from total lifetime writes and power-on hours.
///
/// Uses the physical relation: Daily GB = (Writes per Hour in GB) * 24 hours.
/// Returns 0.0 if `power_on_hours == 0`.
pub fn historical_daily_writes_gb(total_bytes_written: u64, power_on_hours: u64) -> f64 {
    writes_per_power_on_hour_gb(total_bytes_written, power_on_hours) * 24.0
}

/// Returns true if `today_writes_bytes` in GB strictly exceeds `threshold_gb`.
///
/// Returns false if `threshold_gb <= 0.0` or `threshold_gb.is_nan()`.
pub fn is_daily_threshold_exceeded(today_writes_bytes: u64, threshold_gb: f64) -> bool {
    if threshold_gb <= 0.0 || threshold_gb.is_nan() {
        return false;
    }

    let today_gb = today_writes_bytes as f64 / BYTES_PER_GB;
    today_gb > threshold_gb
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn test_average_daily_writes_gb_empty() {
        assert_eq!(average_daily_writes_gb(&[]), 0.0);
    }

    #[test]
    fn test_average_daily_writes_gb_fewer_than_7_days() {
        let d1 = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
        let d2 = NaiveDate::from_ymd_opt(2026, 11, 2).unwrap();
        let d3 = NaiveDate::from_ymd_opt(2026, 11, 3).unwrap();

        // 10 GB, 20 GB, 30 GB -> average is 20 GB
        let data = vec![
            (d1, 10_000_000_000),
            (d2, 20_000_000_000),
            (d3, 30_000_000_000),
        ];

        let avg = average_daily_writes_gb(&data);
        assert!((avg - 20.0).abs() < 1e-6);
    }

    #[test]
    fn test_average_daily_writes_gb_exact_7_days() {
        let mut data = Vec::new();
        for day in 1..=7 {
            let d = NaiveDate::from_ymd_opt(2026, 11, day).unwrap();
            data.push((d, 5_000_000_000)); // 5 GB every day
        }

        let avg = average_daily_writes_gb(&data);
        assert!((avg - 5.0).abs() < 1e-6);
    }

    #[test]
    fn test_average_daily_writes_gb_more_than_7_days() {
        let mut data = Vec::new();
        // Days 1..=3: 100 GB each (older days, should be ignored)
        for day in 1..=3 {
            let d = NaiveDate::from_ymd_opt(2026, 11, day).unwrap();
            data.push((d, 100_000_000_000));
        }
        // Days 4..=10: 10 GB each (last 7 days)
        for day in 4..=10 {
            let d = NaiveDate::from_ymd_opt(2026, 11, day).unwrap();
            data.push((d, 10_000_000_000));
        }

        let avg = average_daily_writes_gb(&data);
        // Average of last 7 days should be 10.0 GB
        assert!((avg - 10.0).abs() < 1e-6);

        // Also test with unsorted order to ensure sorting by date works correctly
        data.reverse();
        let avg_reversed = average_daily_writes_gb(&data);
        assert!((avg_reversed - 10.0).abs() < 1e-6);
    }

    #[test]
    fn test_projected_days_to_depletion_zero_percent() {
        // Brand new drive: 0% used should return None
        let res = projected_days_to_depletion(0, 10_000_000_000, 50_000_000_000);
        assert_eq!(res, None);
    }

    #[test]
    fn test_projected_days_to_depletion_fifty_percent() {
        // 50% used after 100,000 GB written.
        // Each 1% is 2,000 GB.
        // Remaining 50% is 100,000 GB.
        // At 10,000 GB/day, remaining days should be 10.
        let res = projected_days_to_depletion(
            50,
            10_000 * 1_000_000_000,
            100_000 * 1_000_000_000,
        );
        assert_eq!(res, Some(10));

        // 10% used after 10 GB written.
        // Remaining 90% is 90 GB.
        // At 1 GB/day, remaining days should be 90.
        let res2 = projected_days_to_depletion(10, 1_000_000_000, 10_000_000_000);
        assert_eq!(res2, Some(90));
    }

    #[test]
    fn test_projected_days_to_depletion_hundred_percent() {
        // Exactly 100% used
        assert_eq!(
            projected_days_to_depletion(100, 10_000_000_000, 500_000_000_000),
            Some(0)
        );

        // Over 100% used (e.g. 105%)
        assert_eq!(
            projected_days_to_depletion(105, 10_000_000_000, 600_000_000_000),
            Some(0)
        );

        // 100% used even if avg_daily_bytes is 0
        assert_eq!(
            projected_days_to_depletion(100, 0, 500_000_000_000),
            Some(0)
        );
    }

    #[test]
    fn test_projected_days_to_depletion_zero_daily_bytes() {
        // 0 daily bytes should return None (lifespan indefinite)
        assert_eq!(
            projected_days_to_depletion(25, 0, 100_000_000_000),
            None
        );
    }

    #[test]
    fn test_writes_per_power_on_hour_gb_zero_hours() {
        assert_eq!(writes_per_power_on_hour_gb(100_000_000_000, 0), 0.0);
    }

    #[test]
    fn test_writes_per_power_on_hour_gb_normal() {
        // 500 GB in 50 hours -> 10.0 GB/hour
        let res = writes_per_power_on_hour_gb(500 * 1_000_000_000, 50);
        assert!((res - 10.0).abs() < 1e-6);

        // 0 bytes in 100 hours -> 0.0 GB/hour
        let res_zero = writes_per_power_on_hour_gb(0, 100);
        assert_eq!(res_zero, 0.0);
    }

    #[test]
    fn test_historical_daily_writes_gb() {
        // 240 GB in 24 hours -> 10 GB/hr * 24 = 240 GB/day
        let res = historical_daily_writes_gb(240 * 1_000_000_000, 24);
        assert!((res - 240.0).abs() < 1e-6);

        // 0 hours -> 0.0
        assert_eq!(historical_daily_writes_gb(500_000_000_000, 0), 0.0);
    }

    #[test]
    fn test_is_daily_threshold_exceeded_below() {
        // 49 GB written with 50.0 GB threshold -> false
        assert!(!is_daily_threshold_exceeded(49 * 1_000_000_000, 50.0));
    }

    #[test]
    fn test_is_daily_threshold_exceeded_exact() {
        // Exactly 50 GB written with 50.0 GB threshold -> false (does not exceed)
        assert!(!is_daily_threshold_exceeded(50 * 1_000_000_000, 50.0));
    }

    #[test]
    fn test_is_daily_threshold_exceeded_above() {
        // 51 GB written with 50.0 GB threshold -> true
        assert!(is_daily_threshold_exceeded(51 * 1_000_000_000, 50.0));
    }

    #[test]
    fn test_is_daily_threshold_exceeded_non_positive_threshold() {
        // Threshold <= 0.0 should always return false
        assert!(!is_daily_threshold_exceeded(100 * 1_000_000_000, 0.0));
        assert!(!is_daily_threshold_exceeded(100 * 1_000_000_000, -10.0));
    }
}
