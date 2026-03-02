fn main() {
    // Embed build date as YYYY-MM-DD (pure Rust, no shell dependency)
    let now = {
        let duration = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let secs = duration.as_secs();
        // Convert Unix timestamp to date using civil day calculation
        let days = (secs / 86400) as i64;
        let (year, month, day) = civil_from_days(days);
        format!("{year}-{month:02}-{day:02}")
    };
    println!("cargo:rustc-env=BUILD_DATE={now}");
}

/// Convert days since Unix epoch to (year, month, day).
/// Algorithm from Howard Hinnant's `chrono`-compatible civil calendar.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}
