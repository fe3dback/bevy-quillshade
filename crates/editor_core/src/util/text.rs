//! Text helpers.

/// `n` and the noun in the right number: `1 node`, `2 nodes`, `0 nodes`.
pub fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Formats a byte count into a human-readable string (`B`, `KB`, or `MB`).
pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.2} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Formats a [`std::time::SystemTime`] into a UTC civil date-time string (`YYYY-MM-DD HH:MM:SS`).
pub fn format_system_time(time: std::time::SystemTime) -> String {
    let secs = time.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let hour = rem / 3_600;
    let min = (rem % 3_600) / 60;
    let sec = rem % 60;
    // Howard Hinnant's civil_from_days algorithm.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!("{year:04}-{m:02}-{d:02} {hour:02}:{min:02}:{sec:02}")
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::{format_bytes, format_system_time, plural};

    #[test]
    fn plural_forms() {
        assert_eq!(plural(1, "node", "nodes"), "1 node");
        assert_eq!(plural(2, "node", "nodes"), "2 nodes");
        assert_eq!(plural(0, "node", "nodes"), "0 nodes");
    }

    #[test]
    fn format_bytes_units() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1536), "1.50 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.00 MB");
    }

    #[test]
    fn format_system_time_civil_dates() {
        assert_eq!(format_system_time(UNIX_EPOCH), "1970-01-01 00:00:00");
        // Leap day: 2024-02-29 12:34:56 UTC.
        let leap_2024 = UNIX_EPOCH + Duration::from_secs(19_782 * 86_400 + 12 * 3_600 + 34 * 60 + 56);
        assert_eq!(format_system_time(leap_2024), "2024-02-29 12:34:56");
        // Non-leap year date: 2026-11-04 18:47:14 UTC.
        let nov_2026 = UNIX_EPOCH + Duration::from_secs(20_761 * 86_400 + 18 * 3_600 + 47 * 60 + 14);
        assert_eq!(format_system_time(nov_2026), "2026-11-04 18:47:14");
    }
}
