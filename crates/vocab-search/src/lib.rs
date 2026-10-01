//! Understanding queries, ranking dictionary results, and matching the
//! user's own library.
//!
//! Dictionary search is retrieval (the [`DictionaryProvider`]) then
//! deterministic ranking ([`DeterministicRanker`]): ranking never drops a
//! candidate, and the same input always gives the same order. Frequency and
//! HSK only break ties.

mod library;
mod probes;
mod query;
mod ranker;

pub use library::{LibraryDoc, LibraryHit, match_library};
pub use probes::{SearchProbe, load_search_probes};
pub use query::{QueryKind, detect, looks_like_pinyin};
use ranker::Ranker;
pub use ranker::{DeterministicRanker, english_has_lemma};

use vocab_core::Result;
use vocab_dictionary::{Candidate, DictionaryProvider};
use vocab_pinyin::{NormalizedPinyin, normalize, numbered};

/// Results of [`SearchService::search_auto`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoSearch {
    pub candidates: Vec<Candidate>,
    /// How the query was read in the end.
    pub kind: QueryKind,
    /// The first guess, from [`detect`]. Differs from `kind` when the search
    /// fell through (`nihao` guessed English, found as pinyin).
    pub guessed: QueryKind,
}

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

    /// Searches reading the query as `kind`.
    ///
    /// # Errors
    ///
    /// Forwards provider errors.
    pub fn search(&self, kind: QueryKind, query: &str) -> Result<Vec<Candidate>> {
        match kind {
            QueryKind::English => self.search_english(query),
            QueryKind::Pinyin => self.search_pinyin(&normalize(&numbered(query))),
            QueryKind::Chinese => self.lookup_chinese(query),
        }
    }

    /// Searches without a mode: tries the [`detect`]ed kind first, then the
    /// others, and returns the first that finds anything.
    ///
    /// One exception: when the letters also spell pinyin, English hits that
    /// don't contain the query as a whole gloss ("wok" for `wo`, "Jintian
    /// Uprising" for `jintian`) don't stop it being tried as pinyin; they are
    /// kept as a last resort. A real English word still wins: `can` finds
    /// "can".
    ///
    /// # Errors
    ///
    /// Only when every kind failed; a kind that errors is skipped while
    /// another can still answer.
    pub fn search_auto(&self, query: &str) -> Result<AutoSearch> {
        let guessed = detect(query);
        let mut order = vec![guessed];
        for kind in [QueryKind::English, QueryKind::Pinyin, QueryKind::Chinese] {
            if kind != guessed {
                order.push(kind);
            }
        }
        let mut first_error = None;
        let mut answered = false;
        let mut weak_english = None;
        for kind in order {
            match self.search(kind, query) {
                Ok(ranked) if !ranked.is_empty() => {
                    if kind == QueryKind::English
                        && looks_like_pinyin(query)
                        && !english_has_lemma(query, &ranked)
                    {
                        weak_english.get_or_insert(ranked);
                        answered = true;
                        continue;
                    }
                    return Ok(AutoSearch {
                        candidates: ranked,
                        kind,
                        guessed,
                    });
                }
                Ok(_) => answered = true,
                Err(err) => {
                    first_error.get_or_insert(err);
                }
            }
        }
        if let Some(candidates) = weak_english {
            return Ok(AutoSearch {
                candidates,
                kind: QueryKind::English,
                guessed,
            });
        }
        match first_error {
            Some(err) if !answered => Err(err),
            _ => Ok(AutoSearch {
                candidates: Vec::new(),
                kind: guessed,
                guessed,
            }),
        }
    }

    /// The dictionary behind this service. Transfer uses exact lookup, not ranking.
    #[must_use]
    pub fn provider(&self) -> &P {
        &self.provider
    }
}
