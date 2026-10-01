//! Searching the dictionaries and the library, and saving words.

use rusqlite::Connection;
use vocab_core::{ItemSource, LibraryFilter, Result, SourceKind, Verification};
use vocab_db::{NewItem, Saved};
use vocab_dictionary::Candidate;
use vocab_search::{LibraryDoc, QueryKind, match_library};

use crate::dto::{
    CandidateView, DictionaryResults, LibraryResults, ManualWord, QuickAdd, SaveOutcome,
    SaveResult, SavedRef,
};
use crate::library::{item_view, item_views};
use crate::{Error, Shouci};

/// Results returned when the caller gives no limit.
pub const DEFAULT_LIMIT: usize = 50;

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
    };
    Ok(SaveResult {
        outcome,
        item: item_view(conn, item)?,
    })
}

impl Shouci {
    /// Searches the enabled dictionaries. Without `kind`, the query is read
    /// as whatever finds something (Hanzi, pinyin with or without tones,
    /// English), in a fixed, deterministic order.
    ///
    /// # Errors
    ///
    /// [`crate::ErrorKind::Unavailable`] until the dictionaries are loaded.
    pub fn search_dictionary(
        &self,
        query: &str,
        kind: Option<QueryKind>,
        limit: Option<usize>,
    ) -> Result<DictionaryResults> {
        let query = query.trim();
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
        let total = candidates.len();
        candidates.truncate(limit.unwrap_or(DEFAULT_LIMIT));
        Ok(DictionaryResults {
            query: query.to_owned(),
            kind,
            guessed,
            total,
            candidates: views(&*self.db()?, candidates)?,
        })
    }

    /// Searches saved words: Hanzi, pinyin (tones optional), or English in
    /// definitions and notes. An empty query lists every word the filter
    /// allows, newest first.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn search_library(
        &self,
        query: &str,
        filter: &LibraryFilter,
        kind: Option<QueryKind>,
    ) -> Result<LibraryResults> {
        let conn = self.db()?;
        let items = vocab_db::list_items(&conn, filter)?;
        let query = query.trim();
        if query.is_empty() {
            return Ok(LibraryResults {
                query: String::new(),
                kind: None,
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
        let mut slots: Vec<Option<vocab_core::VocabItem>> = items.into_iter().map(Some).collect();
        let matched = hits
            .iter()
            .filter_map(|hit| slots.get_mut(hit.index).and_then(Option::take))
            .collect();
        Ok(LibraryResults {
            query: query.to_owned(),
            kind,
            items: item_views(&conn, matched)?,
        })
    }

    /// Saves a dictionary result as it is. Saving a word that is already
    /// saved changes nothing; saving one from the trash brings it back.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn save_candidate(&self, candidate: &CandidateView) -> Result<SaveResult> {
        let conn = self.db()?;
        let saved = vocab_db::save_item(
            &conn,
            &NewItem {
                simplified: candidate.simplified.clone(),
                traditional: candidate.traditional.clone(),
                pinyin: candidate.pinyin.clone(),
                definition: candidate.definition(),
                notes: String::new(),
                verification: if candidate.inferred {
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
            },
        )?;
        save_result(&conn, saved)
    }

    /// Adds a word from one line of input, without a picker:
    ///
    /// - exactly one strong dictionary match → saved, confirmed;
    /// - no match at all → the text is saved as typed, needing review;
    /// - several strong matches → nothing is saved; the candidates come back
    ///   to choose from.
    ///
    /// # Errors
    ///
    /// [`crate::ErrorKind::Invalid`] for empty input;
    /// [`crate::ErrorKind::Unavailable`] until dictionaries are loaded.
    pub fn quick_add(&self, query: &str, kind: Option<QueryKind>) -> Result<QuickAdd> {
        let query = query.trim();
        if query.is_empty() {
            return Err(Error::invalid("type a word to add"));
        }
        let loaded = self.loaded()?;
        let ranked = match kind {
            Some(kind) => loaded.search.search(kind, query)?,
            None => loaded.search.search_auto(query)?.candidates,
        };
        let strong: Vec<&Candidate> = ranked
            .iter()
            .filter(|candidate| !candidate.diagnostic.is_inferred)
            .collect();
        if let [only] = strong.as_slice() {
            let view = CandidateView::new((*only).clone(), None);
            return Ok(QuickAdd::Saved(Box::new(self.save_candidate(&view)?)));
        }
        if ranked.is_empty() {
            let conn = self.db()?;
            let saved = vocab_db::save_item(
                &conn,
                &NewItem {
                    simplified: query.to_owned(),
                    traditional: String::new(),
                    pinyin: String::new(),
                    definition: String::new(),
                    notes: String::new(),
                    verification: Verification::NeedsReview,
                    source: ItemSource::manual(),
                },
            )?;
            return Ok(QuickAdd::Saved(Box::new(save_result(&conn, saved)?)));
        }
        let mut ranked = ranked;
        ranked.truncate(DEFAULT_LIMIT);
        Ok(QuickAdd::Ambiguous {
            candidates: views(&*self.db()?, ranked)?,
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
            if let Some(loaded) = self.loaded_opt() {
                let key = pinyin.as_deref().map(vocab_db::reading_key);
                let fits: Vec<_> = loaded
                    .provider()
                    .entries_by_headword(simplified)?
                    .into_iter()
                    .filter(|entry| entry.simplified == simplified)
                    .filter(|entry| {
                        key.as_ref()
                            .is_none_or(|key| vocab_db::reading_key(&entry.pinyin) == *key)
                    })
                    .collect();
                if let [entry] = fits.as_slice() {
                    traditional = Some(entry.traditional.clone());
                    pinyin.get_or_insert_with(|| entry.pinyin.clone());
                }
            }
        }
        let definition = text(&word.definition).unwrap_or_default();
        let pinyin = pinyin.unwrap_or_default();
        let verification = if pinyin.is_empty() || definition.is_empty() {
            Verification::NeedsReview
        } else {
            Verification::Confirmed
        };
        let mut conn = self.db()?;
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
            save_result(tx, saved)
        })
    }
}
