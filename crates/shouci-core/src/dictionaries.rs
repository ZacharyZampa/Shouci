//! Which dictionaries are loaded, enabled, and in what order; looking a saved
//! word up in any of them.
//!
//! Loading keeps search available whenever it can be:
//! 1. dictionaries already on disk open first (Ready at once);
//! 2. missing built-ins are then downloaded and built, and stale ones
//!    refreshed, while the status reports what is happening (`Ready` with
//!    `updating` set, or `Loading` when nothing could open yet);
//! 3. the set is reopened and swapped in.
//!
//! One load runs at a time. Changing which dictionaries are enabled during a
//! load is applied when the load finishes.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use vocab_core::{
    DictionaryEntry, ItemPatch, ItemSource, Result, SourceKind, VocabError, VocabItem,
};
use vocab_dictionary::catalog::{self, dictionary_path};
use vocab_dictionary::{
    DictionaryInfo, DictionaryProvider, DictionarySet, Ensured, FetchStage, SqliteDictionary,
};
use vocab_pinyin::tone_marks;
use vocab_search::{DeterministicRanker, SearchService};

use crate::dto::{DictionaryEntryView, DictionaryStatus, DictionaryView, ItemView, LoadingStage};
use crate::library::item_view;
use crate::{Error, Shouci, count};

/// Setting: enabled dictionary ids, comma-separated, highest priority first.
const ENABLED_KEY: &str = "dictionaries.enabled";

pub(crate) enum DictState {
    NotLoaded,
    /// Nothing can be searched yet.
    Loading {
        stage: LoadingStage,
        dictionary: Option<String>,
    },
    Ready {
        loaded: Arc<Loaded>,
        /// A download or rebuild is running in the background.
        updating: Option<(LoadingStage, String)>,
        /// Worth telling the user once.
        notes: Vec<String>,
    },
    Failed(String),
}

pub(crate) struct Loaded {
    pub search: SearchService<DictionarySet>,
    pub enabled: Vec<String>,
}

impl Loaded {
    pub fn provider(&self) -> &dyn DictionaryProvider {
        self.search.provider()
    }
}

/// Loads in progress and preference changes that arrived during one.
#[derive(Default)]
pub(crate) struct LoadControl {
    running: std::sync::Mutex<()>,
    reload_after: AtomicBool,
}

impl Shouci {
    /// Makes dictionaries ready for search. Dictionaries on disk open first;
    /// then missing built-ins are downloaded and built (when the config
    /// allows) and stale ones refreshed, and the set is swapped in. Blocks
    /// for as long as that takes: call it off the UI thread and watch
    /// [`Shouci::dictionary_status`]. Search works as soon as the status is
    /// `Ready`, even while `updating`.
    ///
    /// # Errors
    ///
    /// [`crate::ErrorKind::Unavailable`] when no dictionary can be opened
    /// (the first download failed, say). The status then holds the message.
    pub fn load_dictionaries(&self) -> Result<()> {
        let _running = self
            .load
            .running
            .lock()
            .map_err(|_| Error::new("dictionary loading failed earlier"))?;
        self.load.reload_after.store(false, Ordering::SeqCst);
        let mut notes = Vec::new();
        if !self.is_ready() {
            self.set_loading(LoadingStage::Checking, None);
            // Whatever is already on disk can be searched while the rest
            // downloads.
            if let Ok((loaded, problems)) = self.open_installed() {
                self.set_ready(loaded, problems);
            }
        }
        let mut fetch_errors = Vec::new();
        if self.config.fetch_dictionaries {
            for spec in catalog::builtin() {
                let mut progress = |stage: FetchStage| {
                    let stage = match stage {
                        FetchStage::Waiting => LoadingStage::Waiting,
                        FetchStage::Downloading => LoadingStage::Downloading,
                        FetchStage::Building => LoadingStage::Building,
                    };
                    self.report(stage, spec.name);
                };
                match (spec.ensure)(
                    &dictionary_path(&self.config.dictionaries_dir, spec.id),
                    &mut progress,
                ) {
                    Ok(Ensured::RefreshFailed(err)) => notes.push(format!(
                        "{} could not be updated this month, so the previous copy is in use \
                         ({err}).",
                        spec.name
                    )),
                    Ok(Ensured::Current | Ensured::Built | Ensured::Refreshed) => {}
                    Err(err) => fetch_errors.push(format!("{}: {err}", spec.name)),
                }
            }
        }
        if !self.is_ready() {
            self.set_loading(LoadingStage::Opening, None);
        }
        let result = self.open_installed().map_err(|err| {
            if fetch_errors.is_empty() {
                err
            } else {
                Error::unavailable(format!(
                    "no dictionary is installed: {}. The first launch downloads one and needs \
                     an internet connection.",
                    fetch_errors.join("; ")
                ))
            }
        });
        let outcome = match result {
            Ok((loaded, problems)) => {
                notes.extend(problems);
                notes.extend(
                    fetch_errors
                        .iter()
                        .map(|err| format!("Could not download {err}")),
                );
                self.set_ready(loaded, notes);
                Ok(())
            }
            Err(err) if self.is_ready() => {
                // Keep searching the copy already open; say why it is old.
                self.add_note(format!("Dictionaries could not be reopened: {err}"));
                Ok(())
            }
            Err(err) => {
                self.set_state(DictState::Failed(err.message().to_owned()));
                Err(err)
            }
        };
        if self.load.reload_after.swap(false, Ordering::SeqCst) {
            self.reopen_keeping_notes();
        }
        outcome
    }

    #[must_use]
    pub fn dictionary_status(&self) -> DictionaryStatus {
        match self.dictionaries.read().as_deref() {
            Ok(DictState::NotLoaded) => DictionaryStatus::NotLoaded,
            Ok(DictState::Loading { stage, dictionary }) => DictionaryStatus::Loading {
                stage: *stage,
                dictionary: dictionary.clone(),
            },
            Ok(DictState::Ready {
                loaded,
                updating,
                notes,
            }) => DictionaryStatus::Ready {
                enabled: count(loaded.enabled.len()),
                notes: notes.clone(),
                updating: updating.as_ref().map(|(stage, _)| *stage),
            },
            Ok(DictState::Failed(message)) => DictionaryStatus::Failed {
                message: message.clone(),
            },
            Err(_) => DictionaryStatus::Failed {
                message: "dictionary state is unavailable after an earlier failure".to_owned(),
            },
        }
    }

    /// Every installed dictionary, enabled ones first in priority order.
    /// Works before [`Shouci::load_dictionaries`].
    ///
    /// # Errors
    ///
    /// When the dictionaries directory cannot be read.
    pub fn dictionaries(&self) -> Result<Vec<DictionaryView>> {
        let installed = catalog::installed(&self.config.dictionaries_dir)?.dictionaries;
        let enabled = self.enabled_ids(&installed)?;
        let mut views: Vec<DictionaryView> = installed
            .into_iter()
            .map(|info| {
                let priority = enabled.iter().position(|id| *id == info.id).map(count);
                DictionaryView {
                    enabled: priority.is_some(),
                    priority,
                    id: info.id,
                    name: info.name,
                    version: info.version,
                    license: info.license,
                    entries: info.entry_count,
                }
            })
            .collect();
        views.sort_by_key(|view| (view.priority.unwrap_or(u32::MAX), view.id.clone()));
        Ok(views)
    }

    /// Chooses which dictionaries search uses, highest priority first, and
    /// reopens them (without downloading). Repeated ids count once. During a
    /// load, the change applies when the load finishes.
    ///
    /// # Errors
    ///
    /// [`crate::ErrorKind::Invalid`] for an empty list or an unknown id.
    pub fn set_enabled_dictionaries(&self, ids: &[String]) -> Result<Vec<DictionaryView>> {
        let mut unique: Vec<&str> = Vec::new();
        for id in ids {
            let id = id.trim();
            if !id.is_empty() && !unique.contains(&id) {
                unique.push(id);
            }
        }
        if unique.is_empty() {
            return Err(Error::invalid("enable at least one dictionary"));
        }
        let installed = catalog::installed(&self.config.dictionaries_dir)?.dictionaries;
        for id in &unique {
            if !installed.iter().any(|info| info.id == *id) {
                return Err(Error::invalid(format!("no dictionary '{id}' is installed")));
            }
        }
        vocab_db::set_setting(&*self.db()?, ENABLED_KEY, &unique.join(","))?;
        match self.load.running.try_lock() {
            Ok(_running) => {
                if self.is_ready() {
                    self.reopen_keeping_notes();
                }
            }
            Err(_) => self.load.reload_after.store(true, Ordering::SeqCst),
        }
        self.dictionaries()
    }

    /// A saved word as `dictionary` has it: every entry with its characters,
    /// the ones matching the saved word first. Any installed dictionary
    /// works, enabled or not; the saved word itself never changes.
    ///
    /// # Errors
    ///
    /// [`crate::ErrorKind::NotFound`] for an unknown word or dictionary.
    pub fn lookup_in(&self, item_id: i64, dictionary: &str) -> Result<Vec<DictionaryEntryView>> {
        let item = vocab_db::require_item(&*self.read()?, item_id)?;
        let entries = self.with_dictionary(dictionary, |dict| {
            dict.entries_by_headword(&item.simplified)
        })?;
        let same = same_word(&item, &entries);
        let mut views: Vec<DictionaryEntryView> = entries
            .into_iter()
            .enumerate()
            .map(|(index, entry)| DictionaryEntryView {
                same_word: same.contains(&index),
                pinyin_display: tone_marks(&entry.pinyin),
                definition_display: vocab_dictionary::display_definition(&entry.glosses.join("; ")),
                dictionary: entry.source.0,
                simplified: entry.simplified,
                traditional: entry.traditional,
                pinyin: entry.pinyin,
                glosses: entry.glosses,
            })
            .collect();
        views.sort_by_key(|view| !view.same_word);
        Ok(views)
    }

    /// Replaces a saved word's definition with `dictionary`'s entry for the
    /// same word (characters and reading).
    ///
    /// # Errors
    ///
    /// [`crate::ErrorKind::NotFound`] when the dictionary has no such entry;
    /// [`crate::ErrorKind::Conflict`] when it has several.
    pub fn use_definition(&self, item_id: i64, dictionary: &str) -> Result<ItemView> {
        let item = vocab_db::require_item(&*self.read()?, item_id)?;
        let entries = self.with_dictionary(dictionary, |dict| {
            dict.entries_by_headword(&item.simplified)
        })?;
        let same = same_word(&item, &entries);
        let entry = match same.as_slice() {
            [index] => &entries[*index],
            [] => {
                return Err(Error::not_found(format!(
                    "{dictionary} has no entry for {} [{}]",
                    item.simplified, item.pinyin
                )));
            }
            _ => {
                return Err(Error::conflict(format!(
                    "{dictionary} has {} entries for {} [{}]; edit the definition instead",
                    same.len(),
                    item.simplified,
                    item.pinyin
                )));
            }
        };
        let mut conn = self.db()?;
        vocab_db::with_tx(&mut conn, |tx| {
            vocab_db::update_item(
                tx,
                item_id,
                &ItemPatch {
                    definition: Some(entry.glosses.join("; ")),
                    ..ItemPatch::default()
                },
            )?;
            vocab_db::set_source(
                tx,
                item_id,
                &ItemSource {
                    kind: SourceKind::Dictionary,
                    id: Some(entry.source.0.clone()),
                    version: Some(entry.source_version.0.clone()),
                    import_origin: None,
                },
            )?;
            item_view(tx, vocab_db::require_item(tx, item_id)?)
        })
    }

    /// The loaded dictionaries, or why there are none.
    pub(crate) fn loaded(&self) -> Result<Arc<Loaded>> {
        match self.dictionaries.read().as_deref() {
            Ok(DictState::Ready { loaded, .. }) => Ok(Arc::clone(loaded)),
            Ok(DictState::Loading { .. }) => Err(Error::unavailable(
                "the dictionary is still loading; try again in a moment",
            )),
            Ok(DictState::NotLoaded) => Err(Error::unavailable("the dictionary is not loaded")),
            Ok(DictState::Failed(message)) => Err(Error::unavailable(format!(
                "the dictionary failed to load: {message}"
            ))),
            Err(_) => Err(Error::unavailable(
                "dictionary state is unavailable after an earlier failure",
            )),
        }
    }

    pub(crate) fn loaded_opt(&self) -> Option<Arc<Loaded>> {
        self.loaded().ok()
    }

    fn is_ready(&self) -> bool {
        matches!(
            self.dictionaries.read().as_deref(),
            Ok(DictState::Ready { .. })
        )
    }

    fn set_state(&self, state: DictState) {
        if let Ok(mut current) = self.dictionaries.write() {
            *current = state;
        }
    }

    fn set_loading(&self, stage: LoadingStage, dictionary: Option<&str>) {
        self.set_state(DictState::Loading {
            stage,
            dictionary: dictionary.map(str::to_owned),
        });
    }

    fn set_ready(&self, loaded: Loaded, notes: Vec<String>) {
        self.set_state(DictState::Ready {
            loaded: Arc::new(loaded),
            updating: None,
            notes,
        });
    }

    /// Progress from a download: in the background when search already
    /// works, in the foreground otherwise.
    fn report(&self, stage: LoadingStage, dictionary: &str) {
        if let Ok(mut state) = self.dictionaries.write() {
            match &mut *state {
                DictState::Ready { updating, .. } => {
                    *updating = Some((stage, dictionary.to_owned()));
                }
                other => {
                    *other = DictState::Loading {
                        stage,
                        dictionary: Some(dictionary.to_owned()),
                    };
                }
            }
        }
    }

    fn add_note(&self, note: String) {
        if let Ok(mut state) = self.dictionaries.write() {
            if let DictState::Ready {
                updating, notes, ..
            } = &mut *state
            {
                notes.push(note);
                *updating = None;
            }
        }
    }

    fn current_notes(&self) -> Vec<String> {
        match self.dictionaries.read().as_deref() {
            Ok(DictState::Ready { notes, .. }) => notes.clone(),
            _ => Vec::new(),
        }
    }

    /// Reopens the set (preferences changed), keeping the current notes.
    fn reopen_keeping_notes(&self) {
        let notes = self.current_notes();
        match self.open_installed() {
            Ok((loaded, _)) => self.set_ready(loaded, notes),
            Err(err) => self.set_state(DictState::Failed(err.message().to_owned())),
        }
    }

    fn enabled_ids(&self, installed: &[DictionaryInfo]) -> Result<Vec<String>> {
        let saved = vocab_db::get_setting(&*self.read()?, ENABLED_KEY)?;
        let mut ids: Vec<String> = Vec::new();
        for id in saved
            .as_deref()
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
        {
            if installed.iter().any(|info| info.id == id) && !ids.iter().any(|seen| seen == id) {
                ids.push(id.to_owned());
            }
        }
        if ids.is_empty() {
            // Default: built-ins first, then the rest by id.
            let builtin = |id: &str| catalog::builtin().iter().position(|spec| spec.id == id);
            let mut all: Vec<&DictionaryInfo> = installed.iter().collect();
            all.sort_by_key(|info| (builtin(&info.id).unwrap_or(usize::MAX), info.id.clone()));
            ids = all.into_iter().map(|info| info.id.clone()).collect();
        }
        Ok(ids)
    }

    /// Opens the enabled, installed dictionaries, with notes about files
    /// that could not be opened.
    fn open_installed(&self) -> Result<(Loaded, Vec<String>)> {
        let dir = &self.config.dictionaries_dir;
        let found = catalog::installed(dir)?;
        let notes: Vec<String> = found
            .problems
            .iter()
            .map(|problem| format!("A dictionary file could not be opened: {problem}"))
            .collect();
        if found.dictionaries.is_empty() {
            let detail = if found.problems.is_empty() {
                format!("none in {}", dir.display())
            } else {
                found.problems.join("; ")
            };
            return Err(Error::unavailable(format!(
                "no dictionary is installed ({detail}). The first launch downloads one and \
                 needs an internet connection."
            )));
        }
        let enabled = self.enabled_ids(&found.dictionaries)?;
        let members = enabled
            .iter()
            .filter_map(|id| found.dictionaries.iter().find(|info| &info.id == id))
            .cloned()
            .collect();
        let set = DictionarySet::open(members)?;
        let ranker = DeterministicRanker::new(set.priority());
        Ok((
            Loaded {
                search: SearchService::new(set, ranker),
                enabled,
            },
            notes,
        ))
    }

    fn with_dictionary<T>(
        &self,
        id: &str,
        query: impl FnOnce(&SqliteDictionary) -> Result<T>,
    ) -> Result<T> {
        if let Some(loaded) = self.loaded_opt() {
            if let Some(dict) = loaded.search.provider().member(id) {
                return query(dict);
            }
        }
        let installed = catalog::installed(&self.config.dictionaries_dir)?.dictionaries;
        let info = installed
            .iter()
            .find(|info| info.id == id)
            .ok_or_else(|| VocabError::not_found(format!("no dictionary '{id}' is installed")))?;
        let path = info
            .path
            .as_ref()
            .ok_or_else(|| VocabError::not_found(format!("dictionary '{id}' has no file")))?;
        query(&SqliteDictionary::open(path)?)
    }
}

/// Indexes of the entries that are the saved word: same reading and same
/// traditional form; or, when no entry has its traditional form (a word
/// saved without one), the same reading alone.
fn same_word(item: &VocabItem, entries: &[DictionaryEntry]) -> Vec<usize> {
    let key = vocab_db::reading_key(&item.pinyin);
    let reading: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| vocab_db::reading_key(&entry.pinyin) == key)
        .map(|(index, _)| index)
        .collect();
    let exact: Vec<usize> = reading
        .iter()
        .copied()
        .filter(|&index| entries[index].traditional == item.traditional)
        .collect();
    if exact.is_empty() { reading } else { exact }
}
