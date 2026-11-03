//! A minimal logger.
//!
//! The agent runs as a system service without a logging daemon: the messages
//! go to the console (the s6 supervision tree forwards them) and to the
//! journal-like file in the state partition when it is writable.

use log::{Level, LevelFilter, Metadata, Record};
use std::io::Write;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);

/// Open the log file (best effort: the agent must start even without it).
pub fn open_file(path: &std::path::Path) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        Ok(file) => {
            if let Ok(mut guard) = FILE.lock() {
                *guard = Some(file);
            }
        }
        Err(err) => eprintln!("nrzube-agent: cannot open the log file {path:?}: {err}"),
    }
}

struct Logger;

impl log::Log for Logger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= Level::Info
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!(
            "{} {:<5} {}\n",
            timestamp(),
            record.level(),
            record.args()
        );
        let mut stdout = std::io::stdout();
        let _ = stdout.write_all(line.as_bytes());
        let _ = stdout.flush();

        if let Ok(mut guard) = FILE.lock() {
            if let Some(file) = guard.as_mut() {
                let _ = file.write_all(line.as_bytes());
                let _ = file.flush();
            }
        }
    }

    fn flush(&self) {
        let _ = std::io::stdout().flush();
    }
}

/// Install the logger.
pub fn init(level: LevelFilter) {
    let _ = log::set_boxed_logger(Box::new(Logger));
    log::set_max_level(level);
}

/// UTC timestamp, `YYYY-MM-DDThh:mm:ssZ`.
pub fn timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let (year, month, day, hour, minute, second) = civil_from_unix(now);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Convert a UNIX timestamp into a UTC calendar date.
fn civil_from_unix(timestamp: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = timestamp.div_euclid(86_400);
    let seconds = timestamp.rem_euclid(86_400);

    // Howard Hinnant's civil_from_days algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if m <= 2 { y + 1 } else { y };

    (
        year,
        m,
        d,
        (seconds / 3600) as u32,
        ((seconds % 3600) / 60) as u32,
        (seconds % 60) as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_a_known_timestamp() {
        // 2024-02-29T12:34:56Z
        let (y, m, d, hh, mm, ss) = civil_from_unix(1_709_210_096);
        assert_eq!((y, m, d, hh, mm, ss), (2024, 2, 29, 12, 34, 56));
    }

    #[test]
    fn converts_the_epoch() {
        let (y, m, d, hh, mm, ss) = civil_from_unix(0);
        assert_eq!((y, m, d, hh, mm, ss), (1970, 1, 1, 0, 0, 0));
    }
}
