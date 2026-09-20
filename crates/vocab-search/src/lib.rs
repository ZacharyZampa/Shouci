//! Retrieval plus deterministic ranking.
//!
//! The [`DeterministicRanker`] validates the HLD ordering chain:
//! retrieval happens in the `DictionaryProvider`, ranking never drops candidates,
//! and identical input always produces identical order. Frequency/HSK influence
//! order only — they never remove candidates.

mod probes;
mod ranker;

pub use probes::{SearchProbe, load_search_probes};
use ranker::Ranker;
pub use ranker::{DeterministicRanker, english_has_lemma};

use vocab_core::Result;
use vocab_dictionary::{Candidate, DictionaryProvider};
use vocab_pinyin::NormalizedPinyin;

pub struct SearchService<P: DictionaryProvider> {
    provider: P,
    ranker: DeterministicRanker,
}

impl<P: DictionaryProvider> SearchService<P> {
    pub fn new(provider: P, ranker: DeterministicRanker) -> Self {
        Self { provider, ranker }
    }

    /// English search, retrieves candidates then ranks deterministically.
    ///
    /// # Errors
    ///
    /// Forwards provider errors; ranking itself is infallible.
    pub fn search_english(&self, query: &str) -> Result<Vec<Candidate>> {
        let candidates = self.provider.search_english(query)?;
        Ok(self.ranker.rank_english(query, candidates))
    }

    /// Pinyin search, retrieves candidates then ranks deterministically.
    ///
    /// # Errors
    ///
    /// Forwards provider errors; ranking itself is infallible.
    pub fn search_pinyin(&self, query: &NormalizedPinyin) -> Result<Vec<Candidate>> {
        let candidates = self.provider.search_pinyin(query)?;
        Ok(self.ranker.rank_pinyin(query, candidates))
    }

    /// Chinese lookup, retrieves candidates then ranks deterministically.
    ///
    /// # Errors
    ///
    /// Forwards provider errors; ranking itself is infallible.
    pub fn lookup_chinese(&self, text: &str) -> Result<Vec<Candidate>> {
        let candidates = self.provider.lookup_chinese(text)?;
        Ok(self.ranker.rank_chinese(text, candidates))
    }
}
