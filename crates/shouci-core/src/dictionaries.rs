//! Which dictionaries are loaded, enabled, and in what order; looking a saved
//! word up in any of them.

use std::sync::Arc;

use vocab_core::{ItemPatch, ItemSource, Result, SourceKind, VocabError};
use vocab_dictionary::catalog::{self, dictionary_path};
use vocab_dictionary::{
    DictionaryInfo, DictionaryProvider, DictionarySet, Ensured, FetchStage, SqliteDictionary,
};
use vocab_pinyin::tone_marks;
use vocab_search::{DeterministicRanker, SearchService};

use crate::dto::{DictionaryEntryView, DictionaryStatus, DictionaryView, ItemView, LoadingStage};
use crate::library::item_view;
use crate::{Error, Shouci};

/// Setting: enabled dictionary ids, comma-separated, highest priority first.
const ENABLED_KEY: &str = "dictionaries.enabled";

pub(crate) enum DictState {
    NotLoaded,
    Loading {
        stage: LoadingStage,
        dictionary: Option<String>,
    },
    Ready(Arc<Loaded>),
    Failed(String),
}

pub(crate) struct Loaded {
    pub search: SearchService<DictionarySet>,
    pub enabled: Vec<String>,
    pub notes: Vec<String>,
}

impl Loaded {
    pub fn provider(&self) -> &dyn DictionaryProvider {
        self.search.provider()
    }
}

impl Shouci {
    /// Makes dictionaries ready for search: downloads and builds missing
    /// built-in ones (when the config allows), refreshes stale ones, then
    /// opens the enabled set. Blocks; frontends call it off the UI thread
    /// and watch [`Shouci::dictionary_status`] meanwhile.
    ///
    /// # Errors
    ///
    /// [`crate::ErrorKind::Unavailable`] when no dictionary can be opened
    /// (the first download failed, say). The status then holds the message.
    pub fn load_dictionaries(&self) -> Result<()> {
        self.set_loading(LoadingStage::Checking, None);
        let result = self.build_state(true);
        match result {
            Ok(loaded) => {
                self.set_state(DictState::Ready(Arc::new(loaded)));
                Ok(())
            }
            Err(err) => {
                self.set_state(DictState::Failed(err.message().to_owned()));
                Err(err)
            }
        }
    }

    #[must_use]
    pub fn dictionary_status(&self) -> DictionaryStatus {
        match self.dictionaries.read().as_deref() {
            Ok(DictState::NotLoaded) => DictionaryStatus::NotLoaded,
            Ok(DictState::Loading { stage, dictionary }) => DictionaryStatus::Loading {
                stage: *stage,
                dictionary: dictionary.clone(),
            },
            Ok(DictState::Ready(loaded)) => DictionaryStatus::Ready {
                enabled: loaded.enabled.len(),
                notes: loaded.notes.clone(),
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
                let priority = enabled.iter().position(|id| *id == info.id);
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
        views.sort_by_key(|view| (view.priority.unwrap_or(usize::MAX), view.id.clone()));
        Ok(views)
    }

    /// Chooses which dictionaries search uses, highest priority first, and
    /// reloads them (without downloading).
    ///
    /// # Errors
    ///
    /// [`crate::ErrorKind::Invalid`] for an empty list or an unknown id.
    pub fn set_enabled_dictionaries(&self, ids: &[String]) -> Result<Vec<DictionaryView>> {
        if ids.is_empty() {
            return Err(Error::invalid("enable at least one dictionary"));
        }
        let installed = catalog::installed(&self.config.dictionaries_dir)?.dictionaries;
        for id in ids {
            if !installed.iter().any(|info| &info.id == id) {
                return Err(Error::invalid(format!("no dictionary '{id}' is installed")));
            }
        }
        vocab_db::set_setting(&*self.db()?, ENABLED_KEY, &ids.join(","))?;
        if matches!(
            self.dictionaries.read().as_deref(),
            Ok(DictState::Ready(_) | DictState::Failed(_))
        ) {
            match self.build_state(false) {
                Ok(loaded) => self.set_state(DictState::Ready(Arc::new(loaded))),
                Err(err) => self.set_state(DictState::Failed(err.message().to_owned())),
            }
        }
        self.dictionaries()
    }

    /// A saved word as `dictionary` has it: every entry with its characters,
    /// same-reading entries first. Any installed dictionary works, enabled
    /// or not; the saved word itself never changes.
    ///
    /// # Errors
    ///
    /// [`crate::ErrorKind::NotFound`] for an unknown word or dictionary.
    pub fn lookup_in(&self, item_id: i64, dictionary: &str) -> Result<Vec<DictionaryEntryView>> {
        let item = vocab_db::require_item(&*self.db()?, item_id)?;
        let key = vocab_db::reading_key(&item.pinyin);
        let mut entries: Vec<DictionaryEntryView> = self
            .with_dictionary(dictionary, |dict| {
                dict.entries_by_headword(&item.simplified)
            })?
            .into_iter()
            .map(|entry| DictionaryEntryView {
                same_reading: vocab_db::reading_key(&entry.pinyin) == key,
                pinyin_display: tone_marks(&entry.pinyin),
                definition_display: vocab_dictionary::display_definition(&entry.glosses.join("; ")),
                dictionary: entry.source.0,
                simplified: entry.simplified,
                traditional: entry.traditional,
                pinyin: entry.pinyin,
                glosses: entry.glosses,
            })
            .collect();
        entries.sort_by_key(|entry| !entry.same_reading);
        Ok(entries)
    }

    /// Replaces a saved word's definition with `dictionary`'s for the same
    /// characters and reading.
    ///
    /// # Errors
    ///
    /// [`crate::ErrorKind::NotFound`] when the dictionary has no entry with
    /// that reading; [`crate::ErrorKind::Conflict`] when it has several.
    pub fn use_definition(&self, item_id: i64, dictionary: &str) -> Result<ItemView> {
        let item = vocab_db::require_item(&*self.db()?, item_id)?;
        let key = vocab_db::reading_key(&item.pinyin);
        let matches: Vec<_> = self
            .with_dictionary(dictionary, |dict| {
                dict.entries_by_headword(&item.simplified)
            })?
            .into_iter()
            .filter(|entry| vocab_db::reading_key(&entry.pinyin) == key)
            .collect();
        let entry = match matches.as_slice() {
            [entry] => entry,
            [] => {
                return Err(Error::not_found(format!(
                    "{dictionary} has no entry for {} [{}]",
                    item.simplified, item.pinyin
                )));
            }
            _ => {
                return Err(Error::conflict(format!(
                    "{dictionary} has {} entries for {} [{}]; edit the definition instead",
                    matches.len(),
                    item.simplified,
                    item.pinyin
                )));
            }
        };
        let conn = self.db()?;
        vocab_db::update_item(
            &conn,
            item_id,
            &ItemPatch {
                definition: Some(entry.glosses.join("; ")),
                ..ItemPatch::default()
            },
        )?;
        vocab_db::set_source(
            &conn,
            item_id,
            &ItemSource {
                kind: SourceKind::Dictionary,
                id: Some(entry.source.0.clone()),
                version: Some(entry.source_version.0.clone()),
                import_origin: None,
            },
        )?;
        item_view(&conn, vocab_db::require_item(&conn, item_id)?)
    }

    /// The loaded dictionaries, or why there are none.
    pub(crate) fn loaded(&self) -> Result<Arc<Loaded>> {
        match self.dictionaries.read().as_deref() {
            Ok(DictState::Ready(loaded)) => Ok(Arc::clone(loaded)),
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

    fn enabled_ids(&self, installed: &[DictionaryInfo]) -> Result<Vec<String>> {
        let saved = vocab_db::get_setting(&*self.db()?, ENABLED_KEY)?;
        let mut ids: Vec<String> = match saved {
            Some(list) => list
                .split(',')
                .map(str::trim)
                .filter(|id| installed.iter().any(|info| info.id == *id))
                .map(str::to_owned)
                .collect(),
            None => Vec::new(),
        };
        if ids.is_empty() {
            // Default: built-ins first, then the rest by id.
            let builtin = |id: &str| catalog::builtin().iter().position(|spec| spec.id == id);
            let mut all: Vec<&DictionaryInfo> = installed.iter().collect();
            all.sort_by_key(|info| (builtin(&info.id).unwrap_or(usize::MAX), info.id.clone()));
            ids = all.into_iter().map(|info| info.id.clone()).collect();
        }
        Ok(ids)
    }

    fn build_state(&self, fetch: bool) -> Result<Loaded> {
        let dir = &self.config.dictionaries_dir;
        let mut fetch_errors = Vec::new();
        let mut notes = Vec::new();
        if fetch && self.config.fetch_dictionaries {
            for spec in catalog::builtin() {
                let mut progress = |stage: FetchStage| {
                    let stage = match stage {
                        FetchStage::Waiting => LoadingStage::Waiting,
                        FetchStage::Downloading => LoadingStage::Downloading,
                        FetchStage::Building => LoadingStage::Building,
                    };
                    self.set_loading(stage, Some(spec.name));
                };
                match (spec.ensure)(&dictionary_path(dir, spec.id), &mut progress) {
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
        self.set_loading(LoadingStage::Opening, None);
        let found = catalog::installed(dir)?;
        notes.extend(
            found
                .problems
                .iter()
                .map(|problem| format!("A dictionary file could not be opened: {problem}")),
        );
        if found.dictionaries.is_empty() {
            let mut why = fetch_errors;
            why.extend(found.problems.clone());
            let detail = if why.is_empty() {
                format!("none in {}", dir.display())
            } else {
                why.join("; ")
            };
            return Err(Error::unavailable(format!(
                "no dictionary is installed ({detail}). The first launch downloads one and \
                 needs an internet connection."
            )));
        }
        // Another dictionary works, but say what didn't.
        notes.extend(
            fetch_errors
                .into_iter()
                .map(|err| format!("Could not download {err}")),
        );
        let enabled = self.enabled_ids(&found.dictionaries)?;
        let members = enabled
            .iter()
            .filter_map(|id| found.dictionaries.iter().find(|info| &info.id == id))
            .cloned()
            .collect();
        let set = DictionarySet::open(members)?;
        let ranker = DeterministicRanker::new(set.priority());
        Ok(Loaded {
            search: SearchService::new(set, ranker),
            enabled,
            notes,
        })
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
