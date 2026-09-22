//! Append-only lifecycle log for diagnosing field behavior.
//!
//! Same contract as the Swift sidecar's log: the panel can be dismissed from
//! several paths (save, Esc, hotkey toggle, click-away); when a user reports
//! "it just vanished", this log says which.
//! Tail it live: `tail -f ~/Library/Logs/Shouci/debug.log`

use std::fmt::Write as _;
use std::io::Write as _;
use std::sync::{Mutex, OnceLock};

fn log_file() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| String::from("/tmp"));
    let dir = std::path::PathBuf::from(format!("{home}/Library/Logs/Shouci"));
    let _ignored = std::fs::create_dir_all(&dir);
    dir.join("debug.log")
}

static LOCK: Mutex<()> = Mutex::new(());
static PATH: OnceLock<std::path::PathBuf> = OnceLock::new();

/// Records one timestamped line. Never panics and never blocks the caller
/// for long; failures are silently dropped (logging must not break the app).
pub fn event(message: &str) {
    let Ok(_guard) = LOCK.lock() else {
        return;
    };
    let path = PATH.get_or_init(log_file);
    let mut line = String::new();
    let _ignored = writeln!(
        line,
        "{:.3} {message}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |duty| duty.as_secs_f64())
    );
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ignored = file.write_all(line.as_bytes());
    }
}
