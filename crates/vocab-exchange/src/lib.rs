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

pub use export::{ExportPlan, ExportRequest, ExportScope, apply_export, plan_export};
pub use import::{
    FieldChange, FieldConflict, ImportAction, ImportCounts, ImportPlan, ImportPolicy, Incoming,
    NameChange, PlannedLine, SkipReason, apply_import, plan_import,
};

use std::path::Path;

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
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|err| VocabError::io(format!("cannot create {}: {err}", parent.display())))?;
    }
    let name = path
        .file_name()
        .ok_or_else(|| VocabError::invalid(format!("{} is not a file path", path.display())))?;
    let tmp = path.with_file_name(format!(".{}.shouci-tmp", name.to_string_lossy()));
    std::fs::write(&tmp, bytes)
        .map_err(|err| VocabError::io(format!("cannot write {}: {err}", tmp.display())))?;
    std::fs::rename(&tmp, path).map_err(|err| {
        let _ = std::fs::remove_file(&tmp);
        VocabError::io(format!("cannot replace {}: {err}", path.display()))
    })
}
