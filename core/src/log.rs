//! Small append-only log (`buddy.log` in the data folder), ported from MIKA's `services/log.rs`.
//! Nothing leaves the machine, and no secret, token or request body is ever written here.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Start fresh past this size so it never grows forever.
const MAX_BYTES: u64 = 1_000_000;

pub fn init(data_dir: &Path) {
    let _ = LOG_PATH.set(data_dir.join("buddy.log"));
}

pub fn line(message: impl AsRef<str>) {
    let Some(path) = LOG_PATH.get() else { return };
    if std::fs::metadata(path).map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
        let _ = std::fs::remove_file(path);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{} {}", utc_stamp(SystemTime::now()), message.as_ref());
    }
}

/// `YYYY-MM-DD HH:MM:SSZ` without a date crate.
fn utc_stamp(at: SystemTime) -> String {
    let [year, month, day, hour, minute, second] = utc_parts(at);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}Z")
}

/// `[year, month, day, hour, minute, second]` in UTC (days-from-civil, Howard Hinnant).
pub(crate) fn utc_parts(at: SystemTime) -> [i64; 6] {
    let secs = at.duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    [year, month, day, rem / 3600, rem % 3600 / 60, rem % 60]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn stamps_known_dates() {
        assert_eq!(utc_stamp(UNIX_EPOCH), "1970-01-01 00:00:00Z");
        assert_eq!(utc_stamp(UNIX_EPOCH + Duration::from_secs(1_790_942_400)), "2026-10-02 12:00:00Z");
        assert_eq!(utc_stamp(UNIX_EPOCH + Duration::from_secs(951_782_400)), "2000-02-29 00:00:00Z");
    }
}
