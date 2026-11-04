//! Tiny, dependency-free epoch-seconds -> UTC formatter. Deterministic (no
//! "now", no locale, no timezone DB) so snapshot output stays reproducible.

const SECS_PER_DAY: i64 = 86_400;

/// Format `epoch_secs` (UTC) as `MM-DD HH:MM`.
pub(crate) fn format_utc(epoch_secs: i64) -> String {
    let days = epoch_secs.div_euclid(SECS_PER_DAY);
    let secs = epoch_secs.rem_euclid(SECS_PER_DAY);
    let (_, month, day) = civil_from_days(days);
    let hour = secs / 3_600;
    let minute = (secs % 3_600) / 60;
    format!("{month:02}-{day:02} {hour:02}:{minute:02}")
}

/// Format `epoch_secs` (UTC) as `MM-DD HH:MM:SS`, for second-scale candles.
pub(crate) fn format_utc_secs(epoch_secs: i64) -> String {
    format!(
        "{}:{:02}",
        format_utc(epoch_secs),
        epoch_secs.rem_euclid(60)
    )
}

/// Convert days since the Unix epoch (1970-01-01) to a civil `(year, month,
/// day)`. Howard Hinnant's `civil_from_days` algorithm; valid across the full
/// proleptic Gregorian range and for negative inputs.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let year = y + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seconds_variant_includes_seconds() {
        assert_eq!(format_utc_secs(0), "01-01 00:00:00");
        assert_eq!(format_utc_secs(1_790_104_008), "09-22 19:06:48");
    }

    #[test]
    fn epoch_zero_is_unix_origin() {
        assert_eq!(format_utc(0), "01-01 00:00");
    }

    #[test]
    fn one_day_later() {
        assert_eq!(format_utc(86_400), "01-02 00:00");
    }

    #[test]
    fn negative_epoch_goes_before_origin() {
        assert_eq!(format_utc(-86_400), "12-31 00:00");
    }

    #[test]
    fn formats_time_of_day() {
        // 2020-09-13 12:26:40 UTC -> minute precision, seconds dropped.
        assert_eq!(format_utc(1_600_000_000), "09-13 12:26");
    }

    #[test]
    fn another_known_timestamp() {
        // 2023-11-14 22:13:20 UTC
        assert_eq!(format_utc(1_700_000_000), "11-14 22:13");
    }

    #[test]
    fn handles_leap_day() {
        // 2020-02-29 00:00:00 UTC
        assert_eq!(format_utc(1_582_934_400), "02-29 00:00");
    }
}
