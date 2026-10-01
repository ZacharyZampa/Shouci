//! The vocabulary core every Shouci frontend projects.
//!
//! [`Shouci`] owns the user's library and the dictionaries and exposes every
//! use case as a method: search, capture, edit, organize, import, export.
//! Inputs and outputs are plain serializable values ([`dto`]); frontends
//! only render them. The CLI prints them as JSON, the Mac app receives them
//! over FFI, and a new UI is a new renderer over the same calls.
//!
//! Calls are synchronous and may block (a dictionary download, a large
//! import). [`Shouci`] is `Send + Sync`: call it from whatever thread suits
//! the frontend.

mod config;
mod connectors;
mod dictionaries;
pub mod dto;
mod library;
mod search;
mod transfer;

#[cfg(any(test, feature = "test-support"))]
pub mod testing;

use std::path::Path;
use std::sync::{Mutex, MutexGuard, RwLock};

use rusqlite::Connection;

pub use config::{Config, expand_tilde, expand_tilde_in};
pub use dto::{
    BulkAction, BulkResult, CandidateView, ConnectorView, DictionaryEntryView, DictionaryResults,
    DictionaryStatus, DictionaryView, GroupView, ItemView, LegacyImport, LibraryResults,
    LoadingStage, ManualWord, QuickAdd, SaveOutcome, SaveResult, SavedRef, SourceView,
};
pub use search::DEFAULT_LIMIT;
pub use vocab_core::connector::{Issue, Severity};
pub use vocab_core::{
    ErrorKind, ItemPatch, LibraryFilter, LibraryView, Lifecycle, MatchBasis, Result, SourceKind,
    Verification, VocabError as Error,
};
pub use vocab_exchange::{
    ExportPlan, ExportRequest, ExportScope, FieldChange, FieldConflict, ImportAction, ImportCounts,
    ImportPlan, ImportPolicy, Incoming, NameChange, PlannedLine, SkipReason, TransferSummary,
};
pub use vocab_search::QueryKind;

/// Text helpers for frontends that format raw fields themselves.
pub mod text {
    pub use vocab_dictionary::display_definition;
    pub use vocab_pinyin::tone_marks;
}

use connectors::Registry;
use dictionaries::{DictState, LoadControl};

/// Longest query accepted, in characters. Real queries are a few words; this
/// keeps pasted paragraphs from tying up search.
pub const MAX_QUERY_CHARS: usize = 100;

pub struct Shouci {
    config: Config,
    /// Every write goes through this connection.
    user: Mutex<Connection>,
    /// Reads (search, listings, previews) use their own connection, so a long
    /// import never blocks them, and so `data_version` sees this process's
    /// own writes.
    reader: Mutex<Connection>,
    dictionaries: RwLock<DictState>,
    load: LoadControl,
    connectors: Registry,
    startup_notes: Vec<String>,
}

impl Shouci {
    /// Opens (or creates) the library. Fast: dictionaries are loaded
    /// separately by [`Shouci::load_dictionaries`].
    ///
    /// When `user.db` is created and the config names a proof-of-concept
    /// directory, its words are brought over (see
    /// [`Shouci::startup_notes`]).
    ///
    /// # Errors
    ///
    /// When the data directory cannot be created or `user.db` cannot be
    /// opened (including one written by a newer Shouci).
    pub fn open(config: Config) -> Result<Self> {
        std::fs::create_dir_all(&config.data_dir).map_err(|err| {
            Error::io(format!(
                "cannot create {}: {err}",
                config.data_dir.display()
            ))
        })?;
        let path = config.user_db_path();
        let mut conn = vocab_db::open(&path)?;
        let reader = vocab_db::open(&path)?;
        let mut startup_notes = Vec::new();
        if let Some(legacy) = &config.legacy_dir {
            bring_over(&config, legacy, &mut conn, &mut startup_notes);
        }
        Ok(Self {
            config,
            user: Mutex::new(conn),
            reader: Mutex::new(reader),
            dictionaries: RwLock::new(DictState::NotLoaded),
            load: LoadControl::default(),
            connectors: Registry::builtin(),
            startup_notes,
        })
    }

    #[must_use]
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// What happened while opening, for the frontend to show once: words
    /// brought over from the proof of concept, or why they weren't.
    #[must_use]
    pub fn startup_notes(&self) -> &[String] {
        &self.startup_notes
    }

    /// Changes whenever the library changes: from this `Shouci` (the popover
    /// saving a word) or another process (the CLI). Poll it to refresh an
    /// open window.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn data_version(&self) -> Result<i64> {
        vocab_db::data_version(&*self.read()?)
    }

    /// Brings words, tags, and transfer history over from a
    /// proof-of-concept `user.db`. Safe to run again: only new words are
    /// added.
    ///
    /// # Errors
    ///
    /// When the file is missing or not a POC database, or writing fails.
    pub fn import_poc(&self, legacy_db: &Path) -> Result<LegacyImport> {
        let mut conn = self.db()?;
        Ok(vocab_db::import_poc_database(&mut conn, &expand_tilde(legacy_db)?)?.into())
    }

    /// The connection for writes.
    pub(crate) fn db(&self) -> Result<MutexGuard<'_, Connection>> {
        self.user
            .lock()
            .map_err(|_| Error::new("the library is unavailable after an earlier failure"))
    }

    /// The connection for reads.
    pub(crate) fn read(&self) -> Result<MutexGuard<'_, Connection>> {
        self.reader
            .lock()
            .map_err(|_| Error::new("the library is unavailable after an earlier failure"))
    }
}

/// A count for the API. Counts never come near `u32::MAX`.
pub(crate) fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// A trimmed query, or why it is refused.
pub(crate) fn check_query(query: &str) -> Result<&str> {
    let query = query.trim();
    if query.chars().count() > MAX_QUERY_CHARS {
        return Err(Error::invalid(format!(
            "search is limited to {MAX_QUERY_CHARS} characters"
        )));
    }
    Ok(query)
}

/// After the proof of concept: copy its words once, and its built
/// dictionary so the first launch needs no download. Runs on every open
/// until the words have come over (an interrupted first launch tries
/// again). Failures are reported, never fatal.
fn bring_over(config: &Config, legacy: &Path, conn: &mut Connection, notes: &mut Vec<String>) {
    let legacy_db = legacy.join("user.db");
    let done = vocab_db::get_setting(conn, vocab_db::LEGACY_IMPORTED_SETTING)
        .ok()
        .flatten()
        .is_some();
    if legacy_db.is_file() && !done {
        match vocab_db::import_poc_database(conn, &legacy_db) {
            Ok(report) => {
                if report.items_added > 0 {
                    notes.push(format!(
                        "Brought {} words over from {}.",
                        report.items_added,
                        legacy_db.display()
                    ));
                }
                if !report.skipped.is_empty() {
                    notes.push(format!(
                        "{} could not come over: {}",
                        report.skipped.len(),
                        report.skipped.join("; ")
                    ));
                }
            }
            Err(err) => notes.push(format!(
                "Could not bring words over from {}: {err}. Try `shouci migrate-poc`.",
                legacy_db.display()
            )),
        }
    }
    let legacy_dictionary = legacy.join("dictionary.db");
    let target = vocab_dictionary::catalog::dictionary_path(
        &config.dictionaries_dir,
        vocab_dictionary::catalog::CC_CEDICT.id,
    );
    if legacy_dictionary.is_file() && !target.exists() {
        // Copy beside, then rename: a copy cut short never looks finished.
        let partial = target.with_extension("building.db");
        let copied = std::fs::create_dir_all(&config.dictionaries_dir)
            .and_then(|()| std::fs::copy(&legacy_dictionary, &partial))
            .and_then(|_| std::fs::rename(&partial, &target));
        if copied.is_ok() {
            let stamp = legacy_dictionary.with_extension("fetched");
            if stamp.is_file() {
                let _ = std::fs::copy(stamp, target.with_extension("fetched"));
            }
        } else {
            let _ = std::fs::remove_file(&partial);
        }
    }
}

#[cfg(test)]
mod tests {
    /// Frontends, and the FFI layer, share one `Shouci` across threads.
    #[test]
    fn shouci_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<super::Shouci>();
    }
}
