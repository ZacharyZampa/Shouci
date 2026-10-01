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
use dictionaries::DictState;

pub struct Shouci {
    config: Config,
    user: Mutex<Connection>,
    dictionaries: RwLock<DictState>,
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
        let fresh = !path.exists();
        let mut conn = vocab_db::open(&path)?;
        let mut startup_notes = Vec::new();
        if fresh {
            if let Some(legacy) = &config.legacy_dir {
                bring_over(&config, legacy, &mut conn, &mut startup_notes);
            }
        }
        Ok(Self {
            config,
            user: Mutex::new(conn),
            dictionaries: RwLock::new(DictState::NotLoaded),
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

    /// Changes when another process (the CLI, the menu-bar app) commits to
    /// the library. Poll it to refresh an open window.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn data_version(&self) -> Result<i64> {
        vocab_db::data_version(&*self.db()?)
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
        Ok(vocab_db::import_poc_database(&mut conn, legacy_db)?.into())
    }

    pub(crate) fn db(&self) -> Result<MutexGuard<'_, Connection>> {
        self.user
            .lock()
            .map_err(|_| Error::new("the library is unavailable after an earlier failure"))
    }
}

/// First run after the proof of concept: copy its words, and its built
/// dictionary so the first launch needs no download. Failures are reported,
/// never fatal.
fn bring_over(config: &Config, legacy: &Path, conn: &mut Connection, notes: &mut Vec<String>) {
    let legacy_db = legacy.join("user.db");
    if legacy_db.is_file() {
        match vocab_db::import_poc_database(conn, &legacy_db) {
            Ok(report) if report.items_added > 0 => notes.push(format!(
                "Brought {} words over from {}.",
                report.items_added,
                legacy_db.display()
            )),
            Ok(_) => {}
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
        let copied = std::fs::create_dir_all(&config.dictionaries_dir)
            .and_then(|()| std::fs::copy(&legacy_dictionary, &target));
        if copied.is_ok() {
            let stamp = legacy_dictionary.with_extension("fetched");
            if stamp.is_file() {
                let _ = std::fs::copy(stamp, target.with_extension("fetched"));
            }
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
