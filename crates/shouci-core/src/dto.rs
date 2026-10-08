//! What frontends receive. These types are the projection contract: the
//! CLI prints them as JSON, and the FFI hands them to Swift. Change them
//! additively; a removed or renamed field breaks every frontend.
//!
//! Every word carries raw fields for editing and storage (`pinyin:
//! "xue2 xiao4"`) next to display fields for reading (`pinyin_display:
//! "xué xiào"`), so no frontend reimplements pinyin or definition
//! formatting.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use vocab_core::{
    FrequencyBand, LibraryFilter, Lifecycle, MatchBasis, SourceKind, Verification, VocabItem,
};
use vocab_dictionary::{Candidate, display_definition};
use vocab_exchange::ImportPlan;
use vocab_pinyin::tone_marks;
use vocab_search::QueryKind;

use crate::ranks::Ranks;

/// A saved word.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemView {
    pub id: i64,
    pub simplified: String,
    pub traditional: String,
    /// Numbered: `xue2 xiao4`.
    pub pinyin: String,
    /// Marked: `xué xiào`.
    pub pinyin_display: String,
    pub definition: String,
    /// CC-CEDICT notation cleaned up for reading.
    pub definition_display: String,
    pub notes: String,
    pub verification: Verification,
    pub lifecycle: Lifecycle,
    pub source: SourceView,
    pub tags: Vec<String>,
    pub collections: Vec<String>,
    /// Connectors that have this word: exported there or imported from it.
    pub destinations: Vec<String>,
    /// The word's place in the dictionaries' frequency list, 1 the most
    /// common. `None` when it is not in the list, or no dictionary is
    /// loaded yet.
    pub frequency_rank: Option<u64>,
    /// The band of the frequency list the rank falls in: what a list
    /// grouped by frequency heads the word with. Unlisted when there is no
    /// rank.
    pub frequency_band: FrequencyBand,
    /// Its HSK 3.0 level: 1–6, or 7 for the advanced band (7–9). `None`
    /// when it has none, or no dictionary is loaded yet.
    pub hsk_rank: Option<u64>,
    pub created_at: String,
    pub modified_at: String,
    pub archived_at: Option<String>,
    pub deleted_at: Option<String>,
    /// Increases with every change; tells a UI its copy is out of date.
    pub rev: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceView {
    pub kind: SourceKind,
    /// Dictionary or connector id.
    pub id: Option<String>,
    /// Dictionary version, or the connector's file format.
    pub version: Option<String>,
    pub import_origin: Option<String>,
}

impl ItemView {
    pub(crate) fn new(
        item: VocabItem,
        tags: Vec<String>,
        collections: Vec<String>,
        destinations: Vec<String>,
        ranks: Ranks,
    ) -> Self {
        Self {
            pinyin_display: tone_marks(&item.pinyin),
            definition_display: display_definition(&item.definition),
            lifecycle: item.lifecycle(),
            id: item.id,
            simplified: item.simplified,
            traditional: item.traditional,
            pinyin: item.pinyin,
            definition: item.definition,
            notes: item.notes,
            verification: item.verification,
            source: SourceView {
                kind: item.source.kind,
                id: item.source.id,
                version: item.source.version,
                import_origin: item.source.import_origin,
            },
            tags,
            collections,
            destinations,
            frequency_rank: ranks.frequency,
            frequency_band: FrequencyBand::of(ranks.frequency),
            hsk_rank: ranks.hsk,
            created_at: item.created_at,
            modified_at: item.modified_at,
            archived_at: item.archived_at,
            deleted_at: item.deleted_at,
            rev: item.rev,
        }
    }
}

/// A saved word, in brief, where a dictionary result points at it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedRef {
    pub id: i64,
    pub verification: Verification,
    pub lifecycle: Lifecycle,
}

/// A dictionary search result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateView {
    pub dictionary: String,
    pub dictionary_version: String,
    pub simplified: String,
    pub traditional: String,
    pub pinyin: String,
    pub pinyin_display: String,
    pub glosses: Vec<String>,
    /// Glosses joined with `; `, cleaned up for reading.
    pub definition_display: String,
    pub frequency_rank: Option<u64>,
    pub hsk_rank: Option<u64>,
    /// Matched only character by character; the word itself was not found.
    pub inferred: bool,
    pub basis: MatchBasis,
    /// Set when this word, with this reading, is already saved.
    pub saved: Option<SavedRef>,
}

impl CandidateView {
    pub(crate) fn new(candidate: Candidate, saved: Option<SavedRef>) -> Self {
        let entry = candidate.entry;
        Self {
            pinyin_display: tone_marks(&entry.pinyin),
            definition_display: display_definition(&entry.glosses.join("; ")),
            dictionary: entry.source.0,
            dictionary_version: entry.source_version.0,
            simplified: entry.simplified,
            traditional: entry.traditional,
            pinyin: entry.pinyin,
            glosses: entry.glosses,
            frequency_rank: entry.frequency_rank,
            hsk_rank: entry.hsk_rank,
            inferred: candidate.diagnostic.is_inferred,
            basis: candidate.diagnostic.basis,
            saved,
        }
    }

    /// The definition a saved copy gets.
    #[must_use]
    pub fn definition(&self) -> String {
        self.glosses.join("; ")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DictionaryResults {
    pub query: String,
    /// How the query was read.
    pub kind: QueryKind,
    /// The first guess; differs from `kind` when the search fell through.
    pub guessed: QueryKind,
    /// Matches before `limit` was applied.
    pub total: u32,
    pub candidates: Vec<CandidateView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryResults {
    pub query: String,
    /// How the best match read the query; `None` for an empty query, which
    /// lists everything the filter allows.
    pub kind: Option<QueryKind>,
    /// Matches before `limit` was applied.
    pub total: u32,
    pub items: Vec<ItemView>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SaveOutcome {
    Inserted,
    AlreadySaved,
    /// It was in the trash and is back.
    Restored,
    /// A word saved earlier without a reading (no dictionary match at the
    /// time) was filled in.
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveResult {
    pub outcome: SaveOutcome,
    pub item: ItemView,
}

/// What [`crate::Shouci::quick_add`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum QuickAdd {
    Saved(Box<SaveResult>),
    /// Several strong matches: nothing was saved. Pick one and save it.
    Ambiguous {
        candidates: Vec<CandidateView>,
    },
}

impl From<SaveResult> for QuickAdd {
    fn from(saved: SaveResult) -> Self {
        Self::Saved(Box::new(saved))
    }
}

/// A word typed in by hand.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ManualWord {
    pub simplified: String,
    /// Filled from the dictionary when it has exactly one match.
    pub traditional: Option<String>,
    pub pinyin: Option<String>,
    pub definition: Option<String>,
    pub notes: Option<String>,
    pub tags: Vec<String>,
    pub collections: Vec<String>,
}

/// Something to do to many words at once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", content = "value", rename_all = "snake_case")]
pub enum BulkAction {
    SetVerification(Verification),
    Archive,
    Unarchive,
    /// Move to the trash.
    Trash,
    /// Bring back from the trash.
    Restore,
    /// Delete trashed words for good.
    Purge,
    AddTags(Vec<String>),
    RemoveTags(Vec<String>),
    AddToCollection(String),
    RemoveFromCollection(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkResult {
    /// Distinct words the action was applied to.
    pub changed: u32,
}

/// Some words as they were at one moment, with every tag and collection
/// name: what an undo puts back ([`crate::Shouci::snapshot`]).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LibrarySnapshot {
    /// The words asked for that exist. A word a change saved is missing
    /// from the snapshot before it.
    pub words: Vec<ItemView>,
    /// The ids asked for that no longer exist. A word the snapshot after a
    /// change finds gone was deleted for good while the change ran, so an
    /// undo doesn't bring it back.
    pub missing: Vec<i64>,
    pub tags: Vec<String>,
    pub collections: Vec<String>,
    /// Every smart collection's filter, by name: changing smart collections
    /// changes them, and so does renaming or merging a tag or collection.
    pub smart_filters: HashMap<String, LibraryFilter>,
}

/// A tag or a collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupView {
    pub name: String,
    /// Words outside the trash.
    pub count: u64,
}

impl From<vocab_db::NameCount> for GroupView {
    fn from(value: vocab_db::NameCount) -> Self {
        Self {
            name: value.name,
            count: value.count,
        }
    }
}

/// A smart collection: a saved filter, and the words it matches now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmartCollectionView {
    pub name: String,
    pub filter: LibraryFilter,
    /// The words the filter matches now, newest first.
    pub item_ids: Vec<i64>,
}

/// One condition of a filter, worded for people
/// ([`crate::text::filter_conditions`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterConditionView {
    /// Tells the conditions of one filter apart: `hsk`, `tag pets`.
    pub id: String,
    /// What it asks for: `HSK 4 or 5`, `Not tagged drilled`.
    pub label: String,
    /// The same filter without it: what taking it off leaves.
    pub without: LibraryFilter,
}

/// A band of the frequency list, for a frontend that lists or names them
/// ([`crate::text::frequency_bands`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrequencyBandView {
    pub band: FrequencyBand,
    /// `Top 1,000`.
    pub label: String,
    /// `Top 1k`, where space is short.
    pub short_label: String,
}

/// An HSK level a filter can ask for ([`crate::text::hsk_levels`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HskLevelView {
    /// As a filter holds it: 1–6, 7 for 7–9, 0 for none.
    pub level: u64,
    /// `4`, `7–9`, or `None`: short, under an HSK heading.
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DictionaryStatus {
    NotLoaded,
    Loading {
        stage: LoadingStage,
        /// The dictionary being fetched or built, by name.
        dictionary: Option<String>,
    },
    Ready {
        enabled: u32,
        /// Worth telling the user once: a monthly refresh that failed, a
        /// file that would not open.
        notes: Vec<String>,
        /// A download or rebuild is running in the background; search works.
        updating: Option<LoadingStage>,
    },
    Failed {
        message: String,
    },
}

/// What [`crate::Shouci::load_dictionaries`] is doing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadingStage {
    /// Seeing what is installed and whether it is current.
    Checking,
    /// Another Shouci process is building the dictionary.
    Waiting,
    /// Downloading sources (first run, or the monthly refresh).
    Downloading,
    /// Building the database from the downloads.
    Building,
    Opening,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DictionaryView {
    pub id: String,
    pub name: String,
    pub version: String,
    pub license: String,
    pub entries: u64,
    pub enabled: bool,
    /// 0 is searched first. `None` when disabled.
    pub priority: Option<u32>,
}

/// One entry of one dictionary, shown beside a saved word.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DictionaryEntryView {
    pub dictionary: String,
    pub simplified: String,
    pub traditional: String,
    pub pinyin: String,
    pub pinyin_display: String,
    pub glosses: Vec<String>,
    pub definition_display: String,
    /// This entry is the saved word (same reading and traditional form).
    pub same_word: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorView {
    pub id: String,
    pub name: String,
    pub format: String,
    pub extensions: Vec<String>,
    pub can_import: bool,
    pub can_export: bool,
}

/// A file read as the format that fits it best.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectedImport {
    pub plan: ImportPlan,
    /// No other format reads the file as well.
    pub unambiguous: bool,
}
