//! First-run download + ingest of CC-CEDICT, frequency, and HSK.
//! Existing installs refresh when the calendar month changes.
//!
//! Nothing here prints: progress goes to the caller's callback, and a failed
//! refresh comes back as [`Ensured::RefreshFailed`] for the caller to show.

use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::Connection;
use vocab_core::{Result, VocabError};

use crate::{
    BuildStats, CedictSource, FrequencySource, HskSource, IngestSource,
    build_dictionary_db_with_layers, sha256_hex,
};

/// One file to download, and what it must be.
struct Download {
    url: &'static str,
    /// The file's SHA-256, for sources pinned to one commit. `None` for a
    /// source that is republished in place, which [`check_build`] checks
    /// instead.
    sha256: Option<&'static str>,
    max_bytes: u64,
}

/// Republished daily at the same address, so it cannot be pinned.
const CEDICT: Download = Download {
    url: "https://www.mdbg.net/chinese/export/cedict/cedict_1_0_ts_utf-8_mdbg.txt.gz",
    sha256: None,
    max_bytes: 64 << 20,
};
const FREQUENCY: Download = Download {
    url: "https://raw.githubusercontent.com/hermitdave/FrequencyWords/f8a65e6ddc17e0baa2e366a909986798d8dbe55b/content/2018/zh_cn/zh_cn_50k.txt",
    sha256: Some("25599e00347b893e55c3058a748819b3fe19403291cd12f7c03af128e7f3fe45"),
    max_bytes: 16 << 20,
};
const HSK: Download = Download {
    url: "https://raw.githubusercontent.com/ivankra/hsk30/4ff9e3915ce87baaecd7ebe263085573a4ea3192/hsk30-expanded.csv",
    sha256: Some("62c428c1fc9c221b8ae90281b9e1c72484c7ffdf3d0476e58a4e90cfe797446f"),
    max_bytes: 16 << 20,
};

/// The most decompressed CC-CEDICT may take (it is about 40 MB).
const MAX_CEDICT_BYTES: u64 = 256 << 20;

/// Fewer entries than this means a truncated or wrong download, not a
/// dictionary (CC-CEDICT has over 120,000).
const MIN_ENTRIES: u64 = 100_000;

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

/// curl's time limit for one download attempt (`--max-time`).
const DOWNLOAD_TIME_LIMIT: Duration = Duration::from_secs(600);

/// curl's retries after a failed attempt (`--retry`).
const DOWNLOAD_RETRIES: u64 = 2;

/// How long a fetch lock may sit untouched before it is treated as left
/// behind by a process that died mid-fetch. The fetching process touches it
/// between steps (see [`touch`]), and the longest step is one download with
/// every retry, so a lock older than that has nobody behind it.
const STALE_LOCK: Duration = Duration::from_secs(40 * 60);
const _: () = assert!(
    STALE_LOCK.as_secs() > (DOWNLOAD_RETRIES + 1) * DOWNLOAD_TIME_LIMIT.as_secs(),
    "a download in progress would look abandoned"
);

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
        match take_lock(&lock_path) {
            Lock::Taken => {
                let result = complete_fetch(output, &lock_path, progress);
                let _ = std::fs::remove_file(&lock_path);
                return result;
            }
            // No lock can be made here; fetch anyway.
            Lock::Unsupported => return complete_fetch(output, &lock_path, progress),
            Lock::Abandoned => continue,
            Lock::Held => {}
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
}

/// What came of trying to take the fetch lock.
enum Lock {
    Taken,
    /// Another process is fetching.
    Held,
    /// A lock left behind by a process that died; it was removed.
    Abandoned,
    /// The lock file cannot be created (not because it exists).
    Unsupported,
}

fn take_lock(lock: &Path) -> Lock {
    match OpenOptions::new().write(true).create_new(true).open(lock) {
        Ok(_) => Lock::Taken,
        Err(err) if err.kind() == ErrorKind::AlreadyExists => {
            if lock_is_stale(lock) {
                let _ = std::fs::remove_file(lock);
                Lock::Abandoned
            } else {
                Lock::Held
            }
        }
        Err(_) => Lock::Unsupported,
    }
}

fn lock_is_stale(lock: &Path) -> bool {
    std::fs::metadata(lock)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age > STALE_LOCK)
}

/// Marks the lock as still in use, so no other process takes it over as
/// abandoned. Never creates it.
fn touch(lock: &Path) {
    let _ = OpenOptions::new()
        .append(true)
        .open(lock)
        .and_then(|file| file.set_modified(SystemTime::now()));
}

fn complete_fetch(
    output: &Path,
    lock: &Path,
    progress: &mut dyn FnMut(FetchStage),
) -> Result<Ensured> {
    if output.is_file() {
        Ok(match fetch_and_ingest(output, lock, progress) {
            Ok(()) => Ensured::Refreshed,
            Err(err) => {
                let _ = std::fs::write(failed_stamp_path(output), today());
                Ensured::RefreshFailed(err.to_string())
            }
        })
    } else {
        fetch_and_ingest(output, lock, progress).map(|()| Ensured::Built)
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

/// Refuses a build that parsed but cannot be the real thing: a truncated or
/// substituted download that would otherwise replace a good dictionary.
fn check_build(stats: &BuildStats) -> Result<()> {
    if stats.entries < MIN_ENTRIES {
        return Err(VocabError::new(format!(
            "the downloaded dictionary has only {} entries; keeping the current one",
            stats.entries
        )));
    }
    if stats.frequency_updated == 0 || stats.hsk_updated == 0 {
        return Err(VocabError::new(
            "the downloaded frequency or HSK list matched no entries; keeping the current \
             dictionary",
        ));
    }
    Ok(())
}

fn fetch_and_ingest(
    output: &Path,
    lock: &Path,
    progress: &mut dyn FnMut(FetchStage),
) -> Result<()> {
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
    download(&CEDICT, &cedict_gz)?;
    touch(lock);
    decompress_gz(&cedict_gz, &cedict, MAX_CEDICT_BYTES)?;
    download(&FREQUENCY, &freq)?;
    touch(lock);
    download(&HSK, &hsk)?;
    touch(lock);

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
        build_dictionary_db_with_layers(&mut conn, &CedictSource::default(), &artifact, &layers)
            .and_then(|stats| check_build(&stats));
    drop(conn);
    match result {
        Ok(()) => {
            std::fs::rename(&tmp, output).map_err(|err| {
                VocabError::new(format!(
                    "install {} -> {}: {err}",
                    tmp.display(),
                    output.display()
                ))
            })?;
            // Installed either way. Without the stamp, the file's own date
            // (this month) stands in for it.
            let _ = std::fs::write(stamp_path(output), current_year_month());
            Ok(())
        }
        Err(err) => {
            let _ = std::fs::remove_file(&tmp);
            Err(err)
        }
    }
}

/// Downloads over HTTPS only (redirects included), within a size limit, and
/// checks the hash of a pinned source.
fn download(source: &Download, dest: &Path) -> Result<()> {
    let tmp = sibling_tmp(dest);
    let status = Command::new("curl")
        .args([
            "-Lsf",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--tlsv1.2",
            "--connect-timeout",
            "20",
        ])
        .args(["--retry", &DOWNLOAD_RETRIES.to_string()])
        .args(["--max-time", &DOWNLOAD_TIME_LIMIT.as_secs().to_string()])
        .args(["--max-filesize", &source.max_bytes.to_string()])
        .arg("-o")
        .arg(&tmp)
        .arg(source.url)
        .status()
        .map_err(|err| VocabError::new(format!("curl: {err}")))?;
    let checked = if status.success() {
        verify(source, &tmp)
    } else {
        Err(VocabError::new(format!(
            "download failed ({status}): {}",
            source.url
        )))
    };
    if let Err(err) = checked {
        let _ = std::fs::remove_file(&tmp);
        return Err(err);
    }
    std::fs::rename(&tmp, dest)
        .map_err(|err| VocabError::new(format!("save {}: {err}", dest.display())))
}

/// The size limit (a server may not announce the size up front, which curl
/// needs to enforce it) and, for a pinned source, the hash.
fn verify(source: &Download, file: &Path) -> Result<()> {
    let size = std::fs::metadata(file)
        .map_err(|err| VocabError::new(format!("read {}: {err}", file.display())))?
        .len();
    if size > source.max_bytes {
        return Err(VocabError::new(format!(
            "{} is larger than expected ({size} bytes)",
            source.url
        )));
    }
    if let Some(expected) = source.sha256 {
        let bytes = std::fs::read(file)
            .map_err(|err| VocabError::new(format!("read {}: {err}", file.display())))?;
        if sha256_hex(&bytes) != expected {
            return Err(VocabError::new(format!(
                "{} is not the expected file (SHA-256 mismatch)",
                source.url
            )));
        }
    }
    Ok(())
}

fn sibling_tmp(dest: &Path) -> PathBuf {
    let name = dest.file_name().map_or("download.tmp".into(), |n| {
        format!("{}.tmp", n.to_string_lossy())
    });
    dest.with_file_name(name)
}

/// Streams `gzip -dc src` into `dest`, giving up past `max_bytes` (a
/// decompression bomb would otherwise fill the disk).
fn decompress_gz(src: &Path, dest: &Path, max_bytes: u64) -> Result<()> {
    let tmp = sibling_tmp(dest);
    let result = decompress_into(src, &tmp, max_bytes).and_then(|()| {
        std::fs::rename(&tmp, dest)
            .map_err(|err| VocabError::new(format!("save {}: {err}", dest.display())))
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn decompress_into(src: &Path, dest: &Path, max_bytes: u64) -> Result<()> {
    let mut child = Command::new("gzip")
        .arg("-dc")
        .arg(src)
        .stdout(Stdio::piped())
        // Never read: a full stderr pipe would block gzip while this waits on
        // stdout. The exit status says enough.
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| VocabError::new(format!("gzip: {err}")))?;
    let copied = match (child.stdout.take(), File::create(dest)) {
        (Some(stdout), Ok(mut file)) => std::io::copy(&mut stdout.take(max_bytes + 1), &mut file)
            .map_err(|err| VocabError::new(format!("write {}: {err}", dest.display()))),
        (None, _) => Err(VocabError::new("gzip: no output")),
        (_, Err(err)) => Err(VocabError::new(format!("create {}: {err}", dest.display()))),
    };
    if !matches!(copied, Ok(n) if n <= max_bytes) {
        let _ = child.kill();
    }
    let status = child
        .wait()
        .map_err(|err| VocabError::new(format!("gzip: {err}")))?;
    if copied? > max_bytes {
        return Err(VocabError::new(format!(
            "{} decompresses to more than {max_bytes} bytes",
            src.display()
        )));
    }
    if !status.success() {
        return Err(VocabError::new(format!(
            "{} is not a readable gzip file ({status})",
            src.display()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> (PathBuf, PathBuf) {
        // The clock alone can repeat across tests running in parallel.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "vocab-ensure-{}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
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
    fn implausible_builds_are_refused() {
        let good = BuildStats {
            entries: 125_000,
            glosses: 200_000,
            frequency_updated: 40_000,
            hsk_updated: 11_000,
        };
        assert!(check_build(&good).is_ok());
        assert!(
            check_build(&BuildStats {
                entries: 12,
                ..good
            })
            .is_err()
        );
        assert!(
            check_build(&BuildStats {
                hsk_updated: 0,
                ..good
            })
            .is_err()
        );
    }

    #[test]
    fn pinned_sources_are_checked() {
        let (dir, _) = temp_db();
        let file = dir.join("hsk.csv");
        std::fs::write(&file, b"not the real list").expect("write");
        assert!(verify(&HSK, &file).is_err());
        let tiny = Download {
            url: "https://example.invalid/x",
            sha256: Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
            max_bytes: 4,
        };
        std::fs::write(&file, b"").expect("write");
        assert!(verify(&tiny, &file).is_ok(), "SHA-256 of nothing");
        std::fs::write(&file, b"too long").expect("write");
        assert!(verify(&tiny, &file).is_err(), "over the size limit");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn decompression_is_capped() {
        let (dir, _) = temp_db();
        let plain = dir.join("words.txt");
        std::fs::write(&plain, "你好 ".repeat(1000)).expect("write");
        let status = Command::new("gzip")
            .arg("-k")
            .arg(&plain)
            .status()
            .expect("gzip");
        assert!(status.success());
        let gz = dir.join("words.txt.gz");
        let out = dir.join("out.txt");
        decompress_gz(&gz, &out, 1 << 20).expect("fits");
        assert_eq!(
            std::fs::read(&out).expect("read"),
            std::fs::read(&plain).expect("read")
        );
        std::fs::remove_file(&out).expect("remove");
        assert!(decompress_gz(&gz, &out, 100).is_err());
        assert!(!out.exists(), "nothing half-written is kept");
        assert!(!sibling_tmp(&out).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn touching_a_lock_never_creates_one() {
        let (dir, path) = temp_db();
        let lock = path.with_extension("fetch.lock");
        touch(&lock);
        assert!(!lock.exists());
        let _ = std::fs::remove_dir_all(&dir);
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
