//! The core's values as `UniFFI` sees them. Each declaration restates a type
//! from `shouci-core` (or a crate it re-exports) field for field; the
//! compiler rejects one that drifts from the original, so the Swift types
//! always match the core.

use std::collections::HashMap;

use shouci_core::{
    BulkAction, BulkResult, CandidateView, ConnectorView, DictionaryEntryView, DictionaryResults,
    DictionaryStatus, DictionaryView, ErrorKind, ExportRequest, ExportScope, FieldChange,
    FieldConflict, FilterConditionView, FrequencyBand, FrequencyBandView, GroupView, HskLevelView,
    ImportAction, ImportCounts, ImportField, ImportPolicy, Incoming, ItemPatch, ItemView,
    LibraryFilter, LibraryResults, LibrarySnapshot, LibraryView, Lifecycle, LoadingStage,
    ManualWord, MatchBasis, NameChange, PlannedLine, QueryKind, SaveOutcome, SaveResult, SavedRef,
    Severity, SkipReason, SmartCollectionView, SourceKind, SourceView, TransferSummary,
    Verification,
};

#[uniffi::remote(Enum)]
pub enum ErrorKind {
    NotFound,
    Conflict,
    Invalid,
    Unavailable,
    Format,
    Io,
    Storage,
    Internal,
}

#[uniffi::remote(Enum)]
pub enum Verification {
    Confirmed,
    NeedsReview,
}

#[uniffi::remote(Enum)]
pub enum Lifecycle {
    Active,
    Archived,
    Trashed,
}

#[uniffi::remote(Enum)]
pub enum SourceKind {
    Dictionary,
    Import,
    Manual,
}

#[uniffi::remote(Enum)]
pub enum MatchBasis {
    EnglishGloss,
    Pinyin,
    Simplified,
    CharacterFallback,
    ContainedWord,
}

#[uniffi::remote(Enum)]
pub enum QueryKind {
    English,
    Pinyin,
    Chinese,
}

#[uniffi::remote(Enum)]
pub enum LibraryView {
    Active,
    Archived,
    Trash,
    All,
}

#[uniffi::remote(Enum)]
pub enum FrequencyBand {
    Top1000,
    To5000,
    To10000,
    Beyond10000,
    Unlisted,
}

#[uniffi::remote(Record)]
pub struct LibraryFilter {
    pub view: LibraryView,
    #[uniffi(default)]
    pub verification: Option<Verification>,
    #[uniffi(default)]
    pub tags: Vec<String>,
    #[uniffi(default)]
    pub collection: Option<String>,
    #[uniffi(default)]
    pub no_collection: bool,
    #[uniffi(default)]
    pub any_tags: Vec<String>,
    #[uniffi(default)]
    pub without_tags: Vec<String>,
    #[uniffi(default)]
    pub any_collections: Vec<String>,
    #[uniffi(default)]
    pub without_collections: Vec<String>,
    #[uniffi(default)]
    pub added_within_days: Option<u32>,
    #[uniffi(default)]
    pub hsk_levels: Vec<u64>,
    #[uniffi(default)]
    pub frequency_bands: Vec<FrequencyBand>,
}

#[uniffi::remote(Record)]
pub struct ItemPatch {
    #[uniffi(default)]
    pub simplified: Option<String>,
    #[uniffi(default)]
    pub traditional: Option<String>,
    #[uniffi(default)]
    pub pinyin: Option<String>,
    #[uniffi(default)]
    pub definition: Option<String>,
    #[uniffi(default)]
    pub notes: Option<String>,
    #[uniffi(default)]
    pub verification: Option<Verification>,
}

#[uniffi::remote(Record)]
pub struct SourceView {
    pub kind: SourceKind,
    pub id: Option<String>,
    pub version: Option<String>,
    pub import_origin: Option<String>,
}

#[uniffi::remote(Record)]
pub struct ItemView {
    pub id: i64,
    pub simplified: String,
    pub traditional: String,
    pub pinyin: String,
    pub pinyin_display: String,
    pub definition: String,
    pub definition_display: String,
    pub notes: String,
    pub verification: Verification,
    pub lifecycle: Lifecycle,
    pub source: SourceView,
    pub tags: Vec<String>,
    pub collections: Vec<String>,
    pub destinations: Vec<String>,
    pub frequency_rank: Option<u64>,
    pub frequency_band: FrequencyBand,
    pub hsk_rank: Option<u64>,
    pub created_at: String,
    pub modified_at: String,
    pub archived_at: Option<String>,
    pub deleted_at: Option<String>,
    pub rev: i64,
}

#[uniffi::remote(Record)]
pub struct SavedRef {
    pub id: i64,
    pub verification: Verification,
    pub lifecycle: Lifecycle,
}

#[uniffi::remote(Record)]
pub struct CandidateView {
    pub dictionary: String,
    pub dictionary_version: String,
    pub simplified: String,
    pub traditional: String,
    pub pinyin: String,
    pub pinyin_display: String,
    pub glosses: Vec<String>,
    pub definition_display: String,
    pub frequency_rank: Option<u64>,
    pub hsk_rank: Option<u64>,
    pub inferred: bool,
    pub basis: MatchBasis,
    pub saved: Option<SavedRef>,
}

#[uniffi::remote(Record)]
pub struct DictionaryResults {
    pub query: String,
    pub kind: QueryKind,
    pub guessed: QueryKind,
    pub total: u32,
    pub candidates: Vec<CandidateView>,
}

#[uniffi::remote(Record)]
pub struct LibraryResults {
    pub query: String,
    pub kind: Option<QueryKind>,
    pub total: u32,
    pub items: Vec<ItemView>,
}

#[uniffi::remote(Enum)]
pub enum SaveOutcome {
    Inserted,
    AlreadySaved,
    Restored,
    Completed,
}

#[uniffi::remote(Record)]
pub struct SaveResult {
    pub outcome: SaveOutcome,
    pub item: ItemView,
}

#[uniffi::remote(Record)]
pub struct ManualWord {
    pub simplified: String,
    #[uniffi(default)]
    pub traditional: Option<String>,
    #[uniffi(default)]
    pub pinyin: Option<String>,
    #[uniffi(default)]
    pub definition: Option<String>,
    #[uniffi(default)]
    pub notes: Option<String>,
    #[uniffi(default)]
    pub tags: Vec<String>,
    #[uniffi(default)]
    pub collections: Vec<String>,
}

#[uniffi::remote(Enum)]
pub enum BulkAction {
    SetVerification(Verification),
    Archive,
    Unarchive,
    Trash,
    Restore,
    Purge,
    AddTags(Vec<String>),
    RemoveTags(Vec<String>),
    AddToCollection(String),
    RemoveFromCollection(String),
}

#[uniffi::remote(Record)]
pub struct BulkResult {
    pub changed: u32,
}

#[uniffi::remote(Record)]
pub struct GroupView {
    pub name: String,
    pub count: u64,
}

#[uniffi::remote(Record)]
pub struct SmartCollectionView {
    pub name: String,
    pub filter: LibraryFilter,
    pub item_ids: Vec<i64>,
}

#[uniffi::remote(Record)]
pub struct FilterConditionView {
    pub id: String,
    pub label: String,
    pub without: LibraryFilter,
}

#[uniffi::remote(Record)]
pub struct FrequencyBandView {
    pub band: FrequencyBand,
    pub label: String,
    pub short_label: String,
}

#[uniffi::remote(Record)]
pub struct HskLevelView {
    pub level: u64,
    pub label: String,
}

#[uniffi::remote(Record)]
pub struct LibrarySnapshot {
    pub words: Vec<ItemView>,
    pub missing: Vec<i64>,
    pub tags: Vec<String>,
    pub collections: Vec<String>,
    pub smart_filters: HashMap<String, LibraryFilter>,
}

#[uniffi::remote(Enum)]
pub enum LoadingStage {
    Checking,
    Waiting,
    Downloading,
    Building,
    Opening,
}

#[uniffi::remote(Enum)]
pub enum DictionaryStatus {
    NotLoaded,
    Loading {
        stage: LoadingStage,
        dictionary: Option<String>,
    },
    Ready {
        enabled: u32,
        notes: Vec<String>,
        updating: Option<LoadingStage>,
    },
    Failed {
        message: String,
    },
}

#[uniffi::remote(Record)]
pub struct DictionaryView {
    pub id: String,
    pub name: String,
    pub version: String,
    pub license: String,
    pub entries: u64,
    pub enabled: bool,
    pub priority: Option<u32>,
}

#[uniffi::remote(Record)]
pub struct DictionaryEntryView {
    pub dictionary: String,
    pub simplified: String,
    pub traditional: String,
    pub pinyin: String,
    pub pinyin_display: String,
    pub glosses: Vec<String>,
    pub definition_display: String,
    pub same_word: bool,
}

#[uniffi::remote(Record)]
pub struct ConnectorView {
    pub id: String,
    pub name: String,
    pub format: String,
    pub extensions: Vec<String>,
    pub can_import: bool,
    pub can_export: bool,
}

#[uniffi::remote(Enum)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[uniffi::remote(Enum)]
pub enum ImportPolicy {
    Skip,
    Overwrite,
    Merge,
}

#[uniffi::remote(Record)]
pub struct Incoming {
    pub simplified: String,
    pub traditional: String,
    pub pinyin: String,
    pub definition: String,
    pub notes: String,
    pub verification: Verification,
    pub tags: Vec<String>,
    pub collections: Vec<String>,
}

#[uniffi::remote(Enum)]
pub enum ImportField {
    Definition,
    Notes,
}

#[uniffi::remote(Record)]
pub struct FieldChange {
    pub field: ImportField,
    pub from: String,
    pub to: String,
}

#[uniffi::remote(Record)]
pub struct FieldConflict {
    pub field: ImportField,
    pub kept: String,
    pub incoming: String,
}

#[uniffi::remote(Record)]
pub struct NameChange {
    pub add: Vec<String>,
}

#[uniffi::remote(Enum)]
pub enum SkipReason {
    AlreadySaved,
    InTrash,
    Unchanged,
    RepeatedInFile,
}

#[uniffi::remote(Enum)]
pub enum ImportAction {
    Insert {
        word: Incoming,
    },
    Update {
        item_id: i64,
        simplified: String,
        expected_rev: i64,
        changes: Vec<FieldChange>,
        conflicts: Vec<FieldConflict>,
        tags: NameChange,
        collections: NameChange,
        restore: bool,
    },
    Skip {
        item_id: Option<i64>,
        simplified: String,
        expected_rev: Option<i64>,
        reason: SkipReason,
        conflicts: Vec<FieldConflict>,
    },
    Drop {
        reason: String,
    },
}

#[uniffi::remote(Record)]
pub struct PlannedLine {
    pub line: u32,
    pub raw: String,
    pub action: ImportAction,
}

#[uniffi::remote(Record)]
pub struct ImportCounts {
    pub lines: u32,
    pub inserts: u32,
    pub updates: u32,
    pub skips: u32,
    pub drops: u32,
    pub unresolved: u32,
    pub conflicts: u32,
    pub errors: u32,
    pub warnings: u32,
}

#[uniffi::remote(Enum)]
pub enum ExportScope {
    New,
    All,
    Selected(Vec<i64>),
}

#[uniffi::remote(Record)]
pub struct ExportRequest {
    pub scope: ExportScope,
    pub filter: LibraryFilter,
    #[uniffi(default)]
    pub include_needs_review: bool,
    #[uniffi(default)]
    pub deck: Option<String>,
}

#[uniffi::remote(Record)]
pub struct TransferSummary {
    pub connector_id: String,
    pub path: String,
    pub inserted: u32,
    pub updated: u32,
    pub skipped: u32,
    pub dropped: u32,
    pub unresolved: u32,
    pub written: u32,
    pub refused: bool,
    pub notes: Vec<String>,
}
