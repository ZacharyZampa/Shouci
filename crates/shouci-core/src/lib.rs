//! The vocabulary core every Shouci frontend projects.
//!
//! [`Shouci`] owns the user's library and the dictionaries and exposes every
//! use case as a method: search, capture, edit, organize (smart collections
//! too), import, export.
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
mod ranks;
mod search;
mod smart;
pub mod text;
mod transfer;
mod undo;

#[cfg(any(test, feature = "test-support"))]
pub mod testing;

use std::sync::{Mutex, MutexGuard, RwLock};

use rusqlite::Connection;

pub use config::{Config, expand_tilde, expand_tilde_in};
pub use dto::{
    BulkAction, BulkResult, CandidateView, ConnectorView, DetectedImport, DictionaryEntryView,
    DictionaryResults, DictionaryStatus, DictionaryView, FilterConditionView, FrequencyBandView,
    GroupView, HskLevelView, ItemView, LibraryResults, LibrarySnapshot, LoadingStage, ManualWord,
    QuickAdd, SaveOutcome, SaveResult, SavedRef, SmartCollectionView, SourceView,
};
pub use search::DEFAULT_LIMIT;
pub use vocab_core::connector::{Issue, Severity};
pub use vocab_core::{
    ErrorKind, FrequencyBand, ItemPatch, LibraryFilter, LibraryView, Lifecycle, MatchBasis, Result,
    SourceKind, Verification, VocabError as Error,
};
pub use vocab_exchange::{
    ExportPlan, ExportRequest, ExportScope, FieldChange, FieldConflict, ImportAction, ImportCounts,
    ImportField, ImportPlan, ImportPolicy, Incoming, NameChange, PlannedLine, SkipReason,
    TransferSummary,
};
pub use vocab_search::QueryKind;

use connectors::Registry;
use dictionaries::{DictState, LoadControl};

/// Longest query accepted, in characters. Real queries are a few words; this
/// keeps pasted paragraphs from tying up search.
pub const MAX_QUERY_CHARS: usize = 100;

pub struct Shouci {
    config: Config,
    /// Every write goes through this connection.
    writer: Mutex<Connection>,
    /// Reads (search, listings, previews) use their own connection, so a long
    /// import never blocks them, and so `data_version` sees this process's
    /// own writes.
    reader: Mutex<Connection>,
    dictionaries: RwLock<DictState>,
    load: LoadControl,
    connectors: Registry,
}

impl Shouci {
    /// Opens (or creates) the library. Fast: dictionaries are loaded
    /// separately by [`Shouci::load_dictionaries`].
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
        let conn = vocab_db::open(&path)?;
        let reader = vocab_db::open(&path)?;
        Ok(Self {
            config,
            writer: Mutex::new(conn),
            reader: Mutex::new(reader),
            dictionaries: RwLock::new(DictState::NotLoaded),
            load: LoadControl::default(),
            connectors: Registry::builtin(),
        })
    }

    #[must_use]
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Changes whenever the library changes: from this `Shouci` (the popover
    /// saving a word) or another process (the CLI). Poll it to refresh an
    /// open window.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn data_version(&self) -> Result<i64> {
        vocab_db::data_version(&*self.reader()?)
    }

    /// The connection for writes.
    pub(crate) fn writer(&self) -> Result<MutexGuard<'_, Connection>> {
        self.writer
            .lock()
            .map_err(|_| Error::new("the library is unavailable after an earlier failure"))
    }

    /// The connection for reads.
    pub(crate) fn reader(&self) -> Result<MutexGuard<'_, Connection>> {
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

#[cfg(test)]
mod tests {
    /// Frontends, and the FFI layer, share one `Shouci` across threads.
    #[test]
    fn shouci_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<super::Shouci>();
    }
}
