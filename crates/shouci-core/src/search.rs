//! Searching the dictionaries and the library, and saving words.

use rusqlite::Connection;
use vocab_core::{
    DictionaryEntry, ItemSource, LibraryFilter, MatchBasis, Result, SourceKind, Verification,
    VocabItem,
};
use vocab_db::{NewItem, Saved};
use vocab_dictionary::{Candidate, DictionaryProvider};
use vocab_search::{LibraryDoc, QueryKind, match_library};

use crate::dto::{
    CandidateView, DictionaryResults, LibraryResults, ManualWord, QuickAdd, SaveOutcome,
    SaveResult, SavedRef,
};
use crate::library::{item_view, item_views};
use crate::{Error, Shouci, check_query, count};

/// Dictionary results returned when the caller gives no limit.
pub const DEFAULT_LIMIT: u32 = 50;

fn saved_ref(
    conn: &Connection,
    simplified: &str,
    traditional: &str,
    pinyin: &str,
) -> Result<Option<SavedRef>> {
    Ok(
        vocab_db::find_by_identity(conn, simplified, traditional, pinyin)?.map(|item| SavedRef {
            id: item.id,
            verification: item.verification,
            lifecycle: item.lifecycle(),
        }),
    )
}

fn views(conn: &Connection, candidates: Vec<Candidate>) -> Result<Vec<CandidateView>> {
    candidates
        .into_iter()
        .map(|candidate| {
            let saved = saved_ref(
                conn,
                &candidate.entry.simplified,
                &candidate.entry.traditional,
                &candidate.entry.pinyin,
            )?;
            Ok(CandidateView::new(candidate, saved))
        })
        .collect()
}

fn save_result(conn: &Connection, saved: Saved) -> Result<SaveResult> {
    let (outcome, item) = match saved {
        Saved::Inserted(item) => (SaveOutcome::Inserted, item),
        Saved::Existing(item) => (SaveOutcome::AlreadySaved, item),
        Saved::Restored(item) => (SaveOutcome::Restored, item),
        Saved::Completed(item) => (SaveOutcome::Completed, item),
    };
    Ok(SaveResult {
        outcome,
        item: item_view(conn, item)?,
    })
}

/// The candidate is the query itself, not a longer word containing it.
fn is_the_query(kind: QueryKind, query: &str, candidate: &Candidate) -> bool {
    let entry = &candidate.entry;
    match kind {
        QueryKind::Chinese => entry.simplified == query || entry.traditional == query,
        QueryKind::Pinyin => {
            let toneless = |pinyin: &str| -> String {
                vocab_db::reading_key(pinyin)
                    .chars()
                    .filter(|c| !c.is_ascii_digit())
                    .collect()
            };
            toneless(&entry.pinyin) == toneless(query)
        }
        QueryKind::English => {
            vocab_search::english_has_lemma(query, std::slice::from_ref(candidate))
        }
    }
}

/// For English, the best-ranked result when it is clearly the word: the
/// English is its main sense, frequency or HSK data backs its place, and no
/// other result with that main sense is as basic (HSK) or more common.
/// `school` → 学校 and `eat` → 吃, though 宗 and 食 have those senses too;
/// `teacher` (先生, 老师), `accept` (接下来, 接受), and `to` (了, 来, …) stay
/// a choice for the person.
fn clear_english_pick<'a>(
    provider: &dyn DictionaryProvider,
    query: &str,
    strong: &[&'a Candidate],
) -> Result<Option<&'a Candidate>> {
    let Some((&top, rest)) = strong.split_first() else {
        return Ok(None);
    };
    let top_entry = &top.entry;
    if top_entry.frequency_rank.is_none() && top_entry.hsk_rank.is_none() {
        return Ok(None);
    }
    if !vocab_search::english_is_main_sense(query, top) {
        return Ok(None);
    }
    let mut rivals = rest
        .iter()
        .filter(|other| vocab_search::english_is_main_sense_ignoring_notes(query, other));
    // Frequency counts characters, not readings: 了 liǎo (to finish) has the
    // count of 了 le. Such a word is chosen only when nothing else has the
    // sense.
    if has_other_reading(provider, top_entry)? {
        return Ok(rivals.next().is_none().then_some(top));
    }
    // Lower is better for both; missing data never contests.
    let beats = |other: Option<u64>, mine: Option<u64>, ties: bool| match (other, mine) {
        (Some(other), Some(mine)) => other < mine || (ties && other == mine),
        (Some(_), None) => true,
        (None, _) => false,
    };
    let contested = rivals.any(|other| {
        beats(other.entry.hsk_rank, top_entry.hsk_rank, true)
            || beats(other.entry.frequency_rank, top_entry.frequency_rank, false)
    });
    Ok((!contested).then_some(top))
}

/// The characters have another reading with a meaning of its own (了 le and
/// liǎo; a `see 大夫` pointer does not count).
fn has_other_reading(provider: &dyn DictionaryProvider, entry: &DictionaryEntry) -> Result<bool> {
    let key = vocab_db::reading_key(&entry.pinyin);
    Ok(provider
        .entries_by_headword(&entry.simplified)?
        .iter()
        .any(|other| {
            other.simplified == entry.simplified
                && vocab_db::reading_key(&other.pinyin) != key
                && other
                    .glosses
                    .iter()
                    .any(|gloss| !vocab_dictionary::is_cross_reference(&gloss.to_lowercase()))
        }))
}

fn to_usize(n: u32) -> usize {
    usize::try_from(n).unwrap_or(usize::MAX)
}

impl Shouci {
    /// Searches the enabled dictionaries. Without `kind`, the query is read
    /// as whatever finds something (Hanzi, pinyin with or without tones,
    /// English), in a fixed, deterministic order.
    ///
    /// # Errors
    ///
    /// [`crate::ErrorKind::Unavailable`] until the dictionaries are loaded;
    /// [`crate::ErrorKind::Invalid`] for a query over
    /// [`crate::MAX_QUERY_CHARS`].
    pub fn search_dictionary(
        &self,
        query: &str,
        kind: Option<QueryKind>,
        limit: Option<u32>,
    ) -> Result<DictionaryResults> {
        let query = check_query(query)?;
        let guessed = vocab_search::detect(query);
        if query.is_empty() {
            return Ok(DictionaryResults {
                query: String::new(),
                kind: guessed,
                guessed,
                total: 0,
                candidates: Vec::new(),
            });
        }
        let loaded = self.loaded()?;
        let (mut candidates, kind, guessed) = if let Some(kind) = kind {
            (loaded.search.search(kind, query)?, kind, kind)
        } else {
            let found = loaded.search.search_auto(query)?;
            (found.candidates, found.kind, found.guessed)
        };
        let total = count(candidates.len());
        candidates.truncate(to_usize(limit.unwrap_or(DEFAULT_LIMIT)));
        Ok(DictionaryResults {
            query: query.to_owned(),
            kind,
            guessed,
            total,
            candidates: views(&*self.reader()?, candidates)?,
        })
    }

    /// Searches saved words: Hanzi, pinyin (tones optional), or English in
    /// definitions and notes. An empty query lists every word the filter
    /// allows, newest first. `limit` caps the words returned; `total` says
    /// how many matched.
    ///
    /// # Errors
    ///
    /// [`crate::ErrorKind::Invalid`] for a query over
    /// [`crate::MAX_QUERY_CHARS`]; storage errors.
    pub fn search_library(
        &self,
        query: &str,
        filter: &LibraryFilter,
        kind: Option<QueryKind>,
        limit: Option<u32>,
    ) -> Result<LibraryResults> {
        let query = check_query(query)?;
        let conn = self.reader()?;
        let items = vocab_db::list_items(&conn, filter)?;
        let limit = limit.map_or(usize::MAX, to_usize);
        if query.is_empty() {
            let total = count(items.len());
            let items: Vec<VocabItem> = items.into_iter().take(limit).collect();
            return Ok(LibraryResults {
                query: String::new(),
                kind: None,
                total,
                items: item_views(&conn, items)?,
            });
        }
        let docs: Vec<LibraryDoc<'_>> = items
            .iter()
            .map(|item| LibraryDoc {
                simplified: &item.simplified,
                traditional: &item.traditional,
                pinyin: &item.pinyin,
                definition: &item.definition,
                notes: &item.notes,
            })
            .collect();
        let hits = match_library(query, kind, &docs);
        let kind = hits
            .first()
            .map(|hit| hit.kind)
            .or(kind)
            .or_else(|| Some(vocab_search::detect(query)));
        let total = count(hits.len());
        let mut slots: Vec<Option<VocabItem>> = items.into_iter().map(Some).collect();
        let matched = hits
            .iter()
            .take(limit)
            .filter_map(|hit| slots.get_mut(hit.index).and_then(Option::take))
            .collect();
        Ok(LibraryResults {
            query: query.to_owned(),
            kind,
            total,
            items: item_views(&conn, matched)?,
        })
    }

    /// Saves a dictionary result as it is. Saving a word that is already
    /// saved changes nothing; saving one from the trash brings it back; a
    /// placeholder saved without a reading is completed.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn save_candidate(&self, candidate: &CandidateView) -> Result<SaveResult> {
        self.save(&NewItem {
            simplified: candidate.simplified.clone(),
            traditional: candidate.traditional.clone(),
            pinyin: candidate.pinyin.clone(),
            definition: candidate.definition(),
            notes: String::new(),
            // A word picked from the ones inside the query is that
            // word, as the dictionary has it.
            verification: if candidate.inferred && candidate.basis != MatchBasis::ContainedWord {
                Verification::NeedsReview
            } else {
                Verification::Confirmed
            },
            source: ItemSource {
                kind: SourceKind::Dictionary,
                id: Some(candidate.dictionary.clone()),
                version: Some(candidate.dictionary_version.clone()),
                import_origin: None,
            },
        })
    }

    /// Saves `item` unless it is saved already. One transaction, so another
    /// process saving the same word in between is found, not reported as a
    /// conflict.
    fn save(&self, item: &NewItem) -> Result<SaveResult> {
        let mut conn = self.writer()?;
        vocab_db::with_tx(&mut conn, |tx| {
            let saved = vocab_db::save_item(tx, item)?;
            save_result(tx, saved)
        })
    }

    /// Adds a word from one line of input, without a picker:
    ///
    /// - exactly one result that is the query itself (the same characters,
    ///   the same reading ignoring tones, or a gloss that is exactly the
    ///   English) → saved, confirmed, however many longer words also match;
    /// - for English, first: the best-ranked result when the English is its
    ///   main sense and frequency or HSK data backs it (`school` → 学校,
    ///   though 宗 also means school) → saved, confirmed;
    /// - otherwise exactly one strong dictionary match → saved, confirmed;
    /// - no match at all → Chinese text is saved as typed, needing review (a
    ///   later save of the same characters with a reading completes it);
    ///   anything else (a typo of an English word) is refused;
    /// - several strong matches → nothing is saved; the candidates come back
    ///   to choose from.
    ///
    /// # Errors
    ///
    /// [`crate::ErrorKind::Invalid`] for empty or overlong input;
    /// [`crate::ErrorKind::NotFound`] for text with no match and no Chinese
    /// characters; [`crate::ErrorKind::Unavailable`] until dictionaries are
    /// loaded.
    pub fn quick_add(&self, query: &str, kind: Option<QueryKind>) -> Result<QuickAdd> {
        let query = check_query(query)?;
        if query.is_empty() {
            return Err(Error::invalid("type a word to add"));
        }
        let loaded = self.loaded()?;
        let (ranked, read_as) = if let Some(kind) = kind {
            (loaded.search.search(kind, query)?, kind)
        } else {
            let found = loaded.search.search_auto(query)?;
            (found.candidates, found.kind)
        };
        let strong: Vec<&Candidate> = ranked
            .iter()
            .filter(|candidate| !candidate.diagnostic.is_inferred)
            .collect();
        let exact: Vec<&Candidate> = strong
            .iter()
            .copied()
            .filter(|candidate| is_the_query(read_as, query, candidate))
            .collect();
        let clear = if read_as == QueryKind::English {
            clear_english_pick(loaded.provider(), query, &strong)?
        } else {
            None
        };
        let pick = clear.or(match (exact.as_slice(), strong.as_slice()) {
            ([only], _) | ([], [only]) => Some(*only),
            _ => None,
        });
        if let Some(only) = pick {
            let view = CandidateView::new(only.clone(), None);
            return Ok(self.save_candidate(&view)?.into());
        }
        // Words found inside the text are not the text: it is kept as typed.
        // Only Chinese text, though: `helllo` is a typo, not a word to fill
        // in later.
        if ranked
            .iter()
            .all(|candidate| candidate.diagnostic.basis == MatchBasis::ContainedWord)
        {
            if !vocab_search::has_han(query) {
                return Err(Error::not_found(format!(
                    "nothing in the dictionaries matches {query}; only Chinese characters \
                     are saved to fill in later"
                )));
            }
            let saved = self.save(&NewItem {
                simplified: query.to_owned(),
                traditional: String::new(),
                pinyin: String::new(),
                definition: String::new(),
                notes: String::new(),
                verification: Verification::NeedsReview,
                source: ItemSource::manual(),
            })?;
            return Ok(saved.into());
        }
        let mut ranked = ranked;
        ranked.truncate(to_usize(DEFAULT_LIMIT));
        Ok(QuickAdd::Ambiguous {
            candidates: views(&*self.reader()?, ranked)?,
        })
    }

    /// Saves a word typed in by hand. A missing traditional form (and
    /// reading) is filled from the dictionary when it has exactly one entry
    /// that fits. The word is confirmed when it has a reading and a
    /// definition, and needs review otherwise.
    ///
    /// # Errors
    ///
    /// [`crate::ErrorKind::Invalid`] without characters; storage errors.
    pub fn add_manual(&self, word: &ManualWord) -> Result<SaveResult> {
        let simplified = word.simplified.trim();
        if simplified.is_empty() {
            return Err(Error::invalid("a word needs its characters"));
        }
        let text = |value: &Option<String>| {
            value
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        };
        let mut traditional = text(&word.traditional);
        let mut pinyin = text(&word.pinyin);
        if traditional.is_none() {
            if let Some(entry) = self.only_entry_for(simplified, pinyin.as_deref())? {
                traditional = Some(entry.traditional);
                pinyin.get_or_insert(entry.pinyin);
            }
        }
        let definition = text(&word.definition).unwrap_or_default();
        let pinyin = pinyin.unwrap_or_default();
        let verification = if pinyin.is_empty() || definition.is_empty() {
            Verification::NeedsReview
        } else {
            Verification::Confirmed
        };
        let mut conn = self.writer()?;
        vocab_db::with_tx(&mut conn, |tx| {
            let saved = vocab_db::save_item(
                tx,
                &NewItem {
                    simplified: simplified.to_owned(),
                    traditional: traditional.clone().unwrap_or_default(),
                    pinyin: pinyin.clone(),
                    definition: definition.clone(),
                    notes: text(&word.notes).unwrap_or_default(),
                    verification,
                    source: ItemSource::manual(),
                },
            )?;
            let id = saved.item().id;
            for tag in &word.tags {
                vocab_db::add_tag(tx, id, tag)?;
            }
            for collection in &word.collections {
                vocab_db::add_to_collection(tx, id, collection)?;
            }
            // Tags change the word's revision; report it as stored.
            let item = vocab_db::require_item(tx, id)?;
            let outcome = save_result(tx, saved)?.outcome;
            Ok(SaveResult {
                outcome,
                item: item_view(tx, item)?,
            })
        })
    }

    /// The one dictionary entry for these characters (and reading, when
    /// given), or `None` when there are none, several, or no dictionaries.
    fn only_entry_for(
        &self,
        simplified: &str,
        pinyin: Option<&str>,
    ) -> Result<Option<DictionaryEntry>> {
        let Some(loaded) = self.loaded_opt() else {
            return Ok(None);
        };
        let key = pinyin.map(vocab_db::reading_key);
        let mut fits: Vec<DictionaryEntry> = loaded
            .provider()
            .entries_by_headword(simplified)?
            .into_iter()
            .filter(|entry| entry.simplified == simplified)
            .filter(|entry| {
                key.as_ref()
                    .is_none_or(|key| vocab_db::reading_key(&entry.pinyin) == *key)
            })
            .collect();
        Ok(if fits.len() == 1 { fits.pop() } else { None })
    }
}
