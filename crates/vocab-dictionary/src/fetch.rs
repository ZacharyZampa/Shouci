//! First-run download + ingest of CC-CEDICT, frequency, and HSK.
//! Existing installs refresh when the calendar month changes.
//!
//! Nothing here prints: progress goes to the caller's callback, and a failed
//! refresh comes back as [`Ensured::RefreshFailed`] for the caller to show.

use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::Connection;
use vocab_core::{Result, VocabError};

use crate::{
    CedictSource, FrequencySource, HskSource, IngestSource, build_dictionary_db_with_layers,
};

const CEDICT_URL: &str =
    "https://www.mdbg.net/chinese/export/cedict/cedict_1_0_ts_utf-8_mdbg.txt.gz";
const FREQ_URL: &str = "https://raw.githubusercontent.com/hermitdave/FrequencyWords/master/content/2018/zh_cn/zh_cn_50k.txt";
const HSK_URL: &str = "https://raw.githubusercontent.com/ivankra/hsk30/master/hsk30-expanded.csv";

/// What a fetch is doing, for progress display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchStage {
    /// Another process is building this dictionary; waiting for it.
    Waiting,
    Downloading,
    /// Parsing the downloads into the database.
    Building,
}

/// What [`ensure_dictionary_db`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ensured {
    /// Present and current (or refreshing is turned off).
    Current,
    /// Built for the first time.
    Built,
    /// Replaced with this month's sources.
    Refreshed,
    /// This month's refresh failed; the previous build is still in use.
    RefreshFailed(String),
}

/// What a dictionary file needs, without doing anything about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildState {
    Missing,
    /// Present but cannot be opened (a half-copied or damaged file). It will
    /// be rebuilt.
    Unusable(String),
    /// Usable, and either refreshed this month, refresh is off, or a refresh
    /// already failed today.
    Current,
    /// Usable, and from an earlier month.
    RefreshDue,
}

/// How long a fetch lock may sit untouched before it is treated as left
/// behind by a process that died mid-fetch.
const STALE_LOCK: Duration = Duration::from_secs(30 * 60);

/// How long to wait for another process's first build before giving up.
const FIRST_BUILD_WAIT: Duration = Duration::from_secs(15 * 60);

/// Whether `output` needs building or refreshing.
#[must_use]
pub fn build_state(output: &Path) -> BuildState {
    if !output.is_file() {
        return BuildState::Missing;
    }
    if let Err(err) = crate::SqliteDictionary::open(output).and_then(|dict| dict.info(Some(output)))
    {
        return BuildState::Unusable(err.to_string());
    }
    let refresh = !skip_refresh()
        && due_for_monthly_refresh(output, &current_year_month())
        && !refresh_failed_today(output);
    if refresh {
        BuildState::RefreshDue
    } else {
        BuildState::Current
    }
}

/// Builds `output` if missing or unusable. If it was fetched in an earlier
/// calendar month, re-downloads and replaces it; a failed refresh keeps the
/// current file and is not retried until the next day. Set
/// `SHOUCI_SKIP_DICTIONARY_REFRESH=1` to never refresh.
///
/// One process fetches at a time (a lock file next to `output`). While
/// another process holds it, a usable build is used as-is, and a missing one
/// is waited for. A lock untouched for 30 minutes is assumed abandoned.
///
/// # Errors
///
/// When a first build fails (no network, say). A failed refresh is not an
/// error; see [`Ensured::RefreshFailed`].
pub fn ensure_dictionary_db(
    output: &Path,
    progress: &mut dyn FnMut(FetchStage),
) -> Result<Ensured> {
    match build_state(output) {
        BuildState::Current => return Ok(Ensured::Current),
        BuildState::Unusable(_) => {
            // A cache, not user data: rebuild it.
            let _ = std::fs::remove_file(output);
        }
        BuildState::Missing | BuildState::RefreshDue => {}
    }
    let lock_path = output.with_extension("fetch.lock");
    let started = Instant::now();
    let mut waiting = false;
    loop {
        if build_state(output) == BuildState::Current {
            return Ok(Ensured::Current);
        }
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(_lock) => {
                let result = complete_fetch(output, progress);
                let _ = std::fs::remove_file(&lock_path);
                return result;
            }
            Err(err) if err.kind() == ErrorKind::AlreadyExists => {
                if lock_is_stale(&lock_path) {
                    let _ = std::fs::remove_file(&lock_path);
                    continue;
                }
                if build_state(output) == BuildState::RefreshDue {
                    // Someone else is refreshing; this month's copy can wait.
                    return Ok(Ensured::Current);
                }
                if !waiting {
                    progress(FetchStage::Waiting);
                    waiting = true;
                }
                if started.elapsed() > FIRST_BUILD_WAIT {
                    return Err(VocabError::unavailable(format!(
                        "timed out waiting for another process to build {}",
                        output.display()
                    )));
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            Err(_) => return complete_fetch(output, progress),
        }
    }
}

fn lock_is_stale(lock: &Path) -> bool {
    std::fs::metadata(lock)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age > STALE_LOCK)
}

fn complete_fetch(output: &Path, progress: &mut dyn FnMut(FetchStage)) -> Result<Ensured> {
    if output.is_file() {
        Ok(match fetch_and_ingest(output, progress) {
            Ok(()) => Ensured::Refreshed,
            Err(err) => {
                let _ = std::fs::write(failed_stamp_path(output), today());
                Ensured::RefreshFailed(err.to_string())
            }
        })
    } else {
        fetch_and_ingest(output, progress).map(|()| Ensured::Built)
    }
}

fn failed_stamp_path(db: &Path) -> PathBuf {
    db.with_extension("refresh-failed")
}

fn refresh_failed_today(db: &Path) -> bool {
    std::fs::read_to_string(failed_stamp_path(db)).is_ok_and(|day| day.trim() == today())
}

fn today() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (year, month, day) = civil_utc(i64::try_from(secs / 86_400).unwrap_or(0));
    format!("{year:04}-{month:02}-{day:02}")
}

fn skip_refresh() -> bool {
    matches!(
        std::env::var("SHOUCI_SKIP_DICTIONARY_REFRESH"),
        Ok(value) if value != "0" && !value.is_empty()
    )
}

fn stamp_path(db: &Path) -> PathBuf {
    db.with_extension("fetched")
}

fn current_year_month() -> String {
    year_month_from_unix(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    )
}

fn year_month_from_unix(secs: u64) -> String {
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let (year, month, _) = civil_utc(days);
    format!("{year:04}-{month:02}")
}

/// Days since Unix epoch → UTC year/month/day (Howard Hinnant).
fn civil_utc(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = u32::try_from(z - era * 146_097).unwrap_or(0);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = i32::try_from(yoe).unwrap_or(0) + i32::try_from(era).unwrap_or(0) * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

fn recorded_year_month(db: &Path) -> Option<String> {
    if let Ok(text) = std::fs::read_to_string(stamp_path(db)) {
        let stamp = text.trim();
        if stamp.len() == 7 && stamp.as_bytes()[4] == b'-' {
            return Some(stamp.to_owned());
        }
    }
    let modified = std::fs::metadata(db).ok()?.modified().ok()?;
    let secs = modified.duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some(year_month_from_unix(secs))
}

fn due_for_monthly_refresh(db: &Path, current: &str) -> bool {
    recorded_year_month(db).is_none_or(|prev| prev.as_str() < current)
}

fn write_stamp(db: &Path, stamp: &str) -> Result<()> {
    std::fs::write(stamp_path(db), stamp)
        .map_err(|err| VocabError::new(format!("stamp {}: {err}", stamp_path(db).display())))
}

fn fetch_and_ingest(output: &Path, progress: &mut dyn FnMut(FetchStage)) -> Result<()> {
    let parent = output.parent().filter(|p| !p.as_os_str().is_empty());
    let parent = parent.unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|err| VocabError::new(format!("create {}: {err}", parent.display())))?;
    let sources = parent.join("sources");
    std::fs::create_dir_all(&sources)
        .map_err(|err| VocabError::new(format!("create {}: {err}", sources.display())))?;

    progress(FetchStage::Downloading);
    let cedict_gz = sources.join("cc-cedict.txt.gz");
    let cedict = sources.join("cc-cedict-1.0.u8.txt");
    let freq = sources.join("opensubtitles-zh-cn-50k-2018.txt");
    let hsk = sources.join("hsk30-expanded.csv");
    download(CEDICT_URL, &cedict_gz, true)?;
    decompress_gz(&cedict_gz, &cedict, true)?;
    download(FREQ_URL, &freq, true)?;
    download(HSK_URL, &hsk, true)?;

    let artifact = std::fs::read(&cedict)
        .map_err(|err| VocabError::new(format!("read {}: {err}", cedict.display())))?;
    let frequency_bytes = std::fs::read(&freq)
        .map_err(|err| VocabError::new(format!("read {}: {err}", freq.display())))?;
    let hsk_bytes = std::fs::read(&hsk)
        .map_err(|err| VocabError::new(format!("read {}: {err}", hsk.display())))?;

    let tmp = output.with_extension("building.db");
    let _ = std::fs::remove_file(&tmp);
    let mut conn = Connection::open(&tmp)
        .map_err(|err| VocabError::new(format!("open {}: {err}", tmp.display())))?;
    let frequency = FrequencySource::default();
    let hsk_source = HskSource::default();
    let layers: [(&dyn IngestSource, &[u8]); 2] = [
        (&frequency, frequency_bytes.as_slice()),
        (&hsk_source, hsk_bytes.as_slice()),
    ];
    progress(FetchStage::Building);
    let result =
        build_dictionary_db_with_layers(&mut conn, &CedictSource::default(), &artifact, &layers);
    drop(conn);
    match result {
        Ok(_) => {
            std::fs::rename(&tmp, output).map_err(|err| {
                VocabError::new(format!(
                    "install {} -> {}: {err}",
                    tmp.display(),
                    output.display()
                ))
            })?;
            write_stamp(output, &current_year_month())?;
            Ok(())
        }
        Err(err) => {
            let _ = std::fs::remove_file(&tmp);
            Err(err)
        }
    }
}

fn download(url: &str, dest: &Path, force: bool) -> Result<()> {
    if dest.is_file() && !force {
        return Ok(());
    }
    let tmp = sibling_tmp(dest);
    let status = Command::new("curl")
        .args([
            "-Lsf",
            "--retry",
            "2",
            "--connect-timeout",
            "20",
            "--max-time",
            "600",
            "-o",
        ])
        .arg(&tmp)
        .arg(url)
        .status()
        .map_err(|err| VocabError::new(format!("curl: {err}")))?;
    if !status.success() {
        let _ = std::fs::remove_file(&tmp);
        return Err(VocabError::new(format!(
            "download failed ({status}): {url}"
        )));
    }
    std::fs::rename(&tmp, dest)
        .map_err(|err| VocabError::new(format!("save {}: {err}", dest.display())))
}

fn sibling_tmp(dest: &Path) -> PathBuf {
    let name = dest.file_name().map_or("download.tmp".into(), |n| {
        format!("{}.tmp", n.to_string_lossy())
    });
    dest.with_file_name(name)
}

fn decompress_gz(src: &Path, dest: &Path, force: bool) -> Result<()> {
    if dest.is_file() && !force {
        return Ok(());
    }
    let output = Command::new("gzip")
        .args(["-dc"])
        .arg(src)
        .output()
        .map_err(|err| VocabError::new(format!("gzip: {err}")))?;
    if !output.status.success() {
        return Err(VocabError::new(format!(
            "gzip -dc {} failed: {}",
            src.display(),
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    std::fs::write(dest, output.stdout)
        .map_err(|err| VocabError::new(format!("write {}: {err}", dest.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "vocab-ensure-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp");
        (dir.clone(), dir.join("dictionary.db"))
    }

    #[test]
    fn sibling_tmp_keeps_full_filename() {
        assert_eq!(
            sibling_tmp(Path::new("/tmp/cc-cedict.txt.gz")),
            PathBuf::from("/tmp/cc-cedict.txt.gz.tmp")
        );
    }

    #[test]
    fn year_month_unix_epoch_examples() {
        assert_eq!(year_month_from_unix(0), "1970-01");
        assert_eq!(year_month_from_unix(1_704_067_200), "2024-01");
    }

    #[test]
    fn refresh_when_stamp_is_a_previous_month() {
        let (dir, path) = temp_db();
        std::fs::write(&path, b"keep").expect("write");
        std::fs::write(stamp_path(&path), "2020-01").expect("stamp");
        assert!(due_for_monthly_refresh(&path, "2026-09"));
        assert!(!due_for_monthly_refresh(&path, "2020-01"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn damaged_files_and_stale_locks_are_noticed() {
        let (dir, path) = temp_db();
        std::fs::write(&path, b"not a database").expect("write");
        assert!(matches!(
            super::build_state(&path),
            super::BuildState::Unusable(_)
        ));
        let lock = path.with_extension("fetch.lock");
        std::fs::write(&lock, b"").expect("lock");
        assert!(!super::lock_is_stale(&lock), "fresh lock");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ensure_leaves_current_month_file_alone() {
        let (dir, path) = temp_db();
        let mut conn = rusqlite::Connection::open(&path).expect("open");
        crate::build_dictionary_db(
            &mut conn,
            &crate::CedictSource::default(),
            "你好 你好 [ni3 hao3] /hello/\n".as_bytes(),
        )
        .expect("build");
        drop(conn);
        let before = std::fs::read(&path).expect("read");
        std::fs::write(stamp_path(&path), current_year_month()).expect("stamp");
        let outcome =
            ensure_dictionary_db(&path, &mut |_| panic!("no work expected")).expect("noop");
        assert_eq!(outcome, super::Ensured::Current);
        assert_eq!(std::fs::read(&path).expect("read"), before);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
