//! Publishing an episode: the own feed (RSS for Apple, Spotify and every podcast app) on storage
//! the user controls, or a podcast host's API.

pub mod feed;
pub mod hosts;
pub mod http;
pub mod job;
pub mod storage;

/// Seconds since 1970 (UTC).
pub fn unix_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// (year, month, day, hour, minute, second) of a Unix time, UTC.
fn civil(unix: i64) -> (i64, i64, i64, i64, i64, i64) {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d, secs / 3600, secs / 60 % 60, secs % 60)
}

/// "2026-10-14T12:00:00Z".
pub fn iso(unix: i64) -> String {
    let (y, m, d, h, mi, s) = civil(unix);
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

pub fn now_iso() -> String {
    iso(unix_now())
}

/// "2026-10-14 12:00:00" (UTC), the form some hosts take for dates.
pub fn now_utc() -> String {
    let (y, m, d, h, mi, s) = civil(unix_now());
    format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02}")
}

#[cfg(test)]
mod tests {
    #[test]
    fn iso_dates() {
        assert_eq!(super::iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(super::iso(1_791_979_200), "2026-10-14T12:00:00Z");
        assert_eq!(super::iso(951_782_400), "2000-02-29T00:00:00Z");
    }
}
