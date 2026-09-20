//! First-run download + ingest of CC-CEDICT, frequency, and HSK.
//! Existing installs refresh when the calendar month changes.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;
use vocab_core::{Result, VocabError};

use crate::{
    CedictSource, FrequencySource, HskSource, IngestSource, build_dictionary_db_with_layers,
};

const CEDICT_URL: &str =
    "https://www.mdbg.net/chinese/export/cedict/cedict_1_0_ts_utf-8_mdbg.txt.gz";
const FREQ_URL: &str = "https://raw.githubusercontent.com/hermitdave/FrequencyWords/master/content/2018/zh_cn/zh_cn_50k.txt";
const HSK_URL: &str = "https://raw.githubusercontent.com/ivankra/hsk30/master/hsk30-expanded.csv";

/// Builds `output` if missing. If it exists and was fetched in a previous
/// calendar month, re-downloads and replaces it. A failed refresh keeps the
/// current file. Set `VOCAB_SKIP_DICTIONARY_REFRESH=1` to never refresh.
///
/// # Errors
///
/// First-run download/ingest failure. Refresh failures are logged, not returned.
pub fn ensure_dictionary_db(output: &Path) -> Result<()> {
    if output.is_file() {
        if skip_refresh() || !due_for_monthly_refresh(output, &current_year_month()) {
            return Ok(());
        }
        match fetch_and_ingest(output, "monthly dictionary update") {
            Ok(()) => Ok(()),
            Err(err) => {
                eprintln!("dictionary update failed; keeping existing: {err}");
                Ok(())
            }
        }
    } else {
        fetch_and_ingest(output, "first run")
    }
}

fn skip_refresh() -> bool {
    matches!(
        std::env::var("VOCAB_SKIP_DICTIONARY_REFRESH"),
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

fn fetch_and_ingest(output: &Path, reason: &str) -> Result<()> {
    let parent = output.parent().filter(|p| !p.as_os_str().is_empty());
    let parent = parent.unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|err| VocabError::new(format!("create {}: {err}", parent.display())))?;
    let sources = parent.join("sources");
    std::fs::create_dir_all(&sources)
        .map_err(|err| VocabError::new(format!("create {}: {err}", sources.display())))?;

    eprintln!("downloading CC-CEDICT, frequency, and HSK ({reason})…");
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
    eprintln!("ingesting {}…", output.display());
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
    let tmp = dest.with_extension("tmp");
    let status = Command::new("curl")
        .args(["-Lsf", "--retry", "2", "-o"])
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
    fn ensure_leaves_current_month_file_alone() {
        let (dir, path) = temp_db();
        std::fs::write(&path, b"keep").expect("write");
        std::fs::write(stamp_path(&path), current_year_month()).expect("stamp");
        ensure_dictionary_db(&path).expect("noop");
        assert_eq!(std::fs::read(&path).expect("read"), b"keep");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
