// SPDX-License-Identifier: GPL-3.0-or-later
//! The setup log: `setup.log` next to pkp.exe. Appended to by every Set up (timestamped, UTC), so a failure can be
//! diagnosed from one file. Holds the steps, the commands run, the tools' error output and the end of their output on a
//! failure. Rotated to `setup.log.prev` when it passes 2 MB. Idea from pokopia-native (MIT), nxs2.py `run` (per-stage
//! logs) and nxs2_gui.py `log_line` @7c3655d.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub const FILE: &str = "setup.log";
const MAX_BYTES: u64 = 2 << 20;

pub struct SetupLog {
    path: PathBuf,
    lock: Mutex<()>,
}

pub fn path_of(game_dir: &Path) -> PathBuf {
    game_dir.join(FILE)
}

impl SetupLog {
    /// Opens (rotating a big old log) the log of a game folder.
    pub fn open(game_dir: &Path) -> SetupLog {
        let path = path_of(game_dir);
        if fs::metadata(&path).map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
            let prev = game_dir.join(format!("{}.prev", FILE));
            let _ = fs::remove_file(&prev);
            let _ = fs::rename(&path, &prev);
        }
        SetupLog { path, lock: Mutex::new(()) }
    }

    /// Appends text, one stamped line per line. A log that cannot be written never stops setup.
    pub fn line(&self, text: &str) {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&self.path) {
            let _ = f.write_all(format_lines(secs, text).as_bytes());
        }
    }
}

/// `2026-11-05 14:03:09Z text`, once per line of `text`.
pub fn format_lines(secs: u64, text: &str) -> String {
    let stamp = stamp(secs);
    let mut out = String::new();
    for l in text.lines() {
        out.push_str(&stamp);
        out.push(' ');
        out.push_str(l.trim_end());
        out.push('\n');
    }
    if out.is_empty() {
        out.push_str(&stamp);
        out.push('\n');
    }
    out
}

/// Seconds since the Unix epoch as `YYYY-MM-DD HH:MM:SSZ` (UTC).
pub fn stamp(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    // civil date from days since 1970-01-01 (Howard Hinnant's algorithm)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}Z", y, m, d, rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Whether a progress report is worth a log line: a new phase, a new message of a phase without a count, or a count that
/// moved 5 % since the last logged one. `last` is (phase, percent logged).
pub fn progress_worth_logging(last: &mut (String, u64, String), phase: &str, done: u64, total: u64, message: &str) -> bool {
    let pct = if total > 0 { 100 * done / total } else { 0 };
    let log = if last.0 != phase {
        true
    } else if total == 0 {
        last.2 != message
    } else {
        pct >= last.1 + 5 || pct < last.1
    };
    if log {
        *last = (phase.to_string(), pct, message.to_string());
    }
    log
}
