//! What search needs from a dictionary: retrieval by query kind, and lookup
//! by headword. [`crate::SqliteDictionary`] is one; [`crate::DictionarySet`]
//! searches several in priority order.

use vocab_core::{DictionaryEntry, MatchBasis, Result};
use vocab_pinyin::NormalizedPinyin;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateDiagnostic {
    pub basis: MatchBasis,
    pub is_inferred: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub entry: DictionaryEntry,
    pub diagnostic: CandidateDiagnostic,
}

/// Runtime query API over a read-only dictionary database.
///
/// Implementations return *all* viable candidates for a query. Ranking is not the
/// provider's job — `vocab-search` orders results deterministically. No candidate
/// may be silently dropped here.
pub trait DictionaryProvider: Send + Sync {
    /// Search English glosses (FTS5-backed in the SQLite implementation).
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying dictionary database cannot be queried.
    fn search_english(&self, query: &str) -> Result<Vec<Candidate>>;

    /// Search normalized pinyin.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying dictionary database cannot be queried.
    fn search_pinyin(&self, pinyin: &NormalizedPinyin) -> Result<Vec<Candidate>>;

    /// Look up Chinese text (simplified and traditional).
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying dictionary database cannot be queried.
    fn lookup_chinese(&self, text: &str) -> Result<Vec<Candidate>>;

    /// Exact simplified or traditional match. No prefix, no character fallback.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying dictionary database cannot be queried.
    fn entries_by_headword(&self, headword: &str) -> Result<Vec<DictionaryEntry>>;

    fn schema_version(&self) -> &str;
}
