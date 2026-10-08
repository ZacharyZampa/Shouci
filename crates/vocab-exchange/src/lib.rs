//! Moving words between the library and connector files.
//!
//! Both directions are plan, then apply:
//!
//! - [`plan_import`] reads a file and decides, line by line, what would
//!   happen. Nothing is written. [`apply_import`] carries the plan out in one
//!   transaction, after checking that neither the file nor the affected words
//!   changed since.
//! - [`plan_export`] picks the words and renders the file in memory.
//!   [`apply_export`] writes it atomically and records the run.
//!
//! Plans are plain values, so a UI can show one as a preview, and the CLI's
//! `--dry-run` is just a plan that is never applied.
//!
//! The engine only talks to [`vocab_core::connector::Connector`]; it knows
//! nothing about Pleco or Anki.

mod export;
mod import;
mod resolve;

pub use export::{
    ExportPlan, ExportRequest, ExportScope, apply_export, plan_export, plan_export_where,
};
pub use import::{
    FieldChange, FieldConflict, ImportAction, ImportCounts, ImportField, ImportPlan, ImportPolicy,
    Incoming, NameChange, PlannedLine, SkipReason, apply_import, plan_import,
};

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use vocab_core::{Result, VocabError};

/// What a transfer did, for people and for `shouci --json`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TransferSummary {
    pub connector_id: String,
    pub path: String,
    pub inserted: u32,
    pub updated: u32,
    pub skipped: u32,
    pub dropped: u32,
    /// Imported but marked needs review.
    pub unresolved: u32,
    pub written: u32,
    /// The import was refused because of error lines; nothing was imported.
    pub refused: bool,
    pub notes: Vec<String>,
}

/// SHA-256 of a file's bytes, hex. Plans carry it so an apply can tell that
/// the file changed after the preview.
#[must_use]
pub fn content_hash(bytes: &[u8]) -> String {
    vocab_dictionary::sha256_hex(bytes)
}

pub(crate) fn reading_key(pinyin: &str) -> String {
    vocab_db::reading_key(pinyin)
}

/// A count for the API (counts never come near `u32::MAX`).
pub(crate) fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Files from Windows editors often start with a UTF-8 byte-order mark;
/// connectors never see it.
pub(crate) fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes)
}

/// Writes next to the target, then renames over it: a reader never sees half
/// a file, and a failed write leaves the old file alone.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    // Unique per process and call, so two writers never share one.
    static NEXT: AtomicU64 = AtomicU64::new(0);
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|err| VocabError::io(format!("cannot create {}: {err}", parent.display())))?;
    }
    let name = path
        .file_name()
        .ok_or_else(|| VocabError::invalid(format!("{} is not a file path", path.display())))?;
    sweep_left_behind(path, &name.to_string_lossy());
    let tmp = path.with_file_name(format!(
        ".{}.{}-{}.shouci-tmp",
        name.to_string_lossy(),
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&tmp, bytes)
        .map_err(|err| VocabError::io(format!("cannot write {}: {err}", tmp.display())))?;
    std::fs::rename(&tmp, path).map_err(|err| {
        let _ = std::fs::remove_file(&tmp);
        VocabError::io(format!("cannot replace {}: {err}", path.display()))
    })
}

/// Temporary files a crashed write left next to `path`. Only old ones go: a
/// recent one may belong to a write still in progress.
const LEFT_BEHIND_AFTER: Duration = Duration::from_secs(60 * 60);

fn sweep_left_behind(path: &Path, name: &str) {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let prefix = format!(".{name}.");
    for entry in entries.flatten() {
        let file = entry.file_name();
        let file = file.to_string_lossy();
        if !(file.starts_with(&prefix) && file.ends_with(".shouci-tmp")) {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > LEFT_BEHIND_AFTER);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::atomic_write;

    #[test]
    fn old_temporary_files_from_a_crashed_write_are_swept() {
        let dir = std::env::temp_dir().join(format!("shouci-sweep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let make = |name: &str, age: Duration| {
            let path = dir.join(name);
            std::fs::write(&path, "x").unwrap();
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(SystemTime::now() - age)
                .unwrap();
            path
        };
        let crashed = make(".out.txt.123-0.shouci-tmp", Duration::from_secs(2 * 3600));
        let older_style = make(".out.txt.shouci-tmp", Duration::from_secs(2 * 3600));
        let in_progress = make(".out.txt.456-0.shouci-tmp", Duration::ZERO);
        let other_file = make(".notes.txt.1-0.shouci-tmp", Duration::from_secs(2 * 3600));

        atomic_write(&dir.join("out.txt"), b"words").unwrap();

        assert_eq!(std::fs::read(dir.join("out.txt")).unwrap(), b"words");
        assert!(!crashed.exists());
        assert!(!older_style.exists());
        assert!(in_progress.exists(), "may still be written");
        assert!(other_file.exists(), "belongs to another file");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
