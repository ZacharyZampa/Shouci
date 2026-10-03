//! The core, for Swift. [`Core`] wraps [`shouci_core::Shouci`] one call for
//! one call; `UniFFI` generates the Swift class and value types from it. Only
//! arguments and results are converted here: every rule stays in the core.
//!
//! Calls block (a dictionary download, a large import), so Swift makes them
//! off the main thread. `Core` is `Send + Sync`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use shouci_core::{
    BulkAction, BulkResult, CandidateView, Config, ConnectorView, DictionaryEntryView,
    DictionaryResults, DictionaryStatus, DictionaryView, ErrorKind, ExportRequest, GroupView,
    ImportPolicy, ItemPatch, ItemView, LegacyImport, LibraryFilter, LibraryResults, ManualWord,
    QueryKind, QuickAdd, SaveResult, Shouci, TransferSummary,
};

mod remote;
mod transfer;

pub use transfer::{ExportPlanView, ExportPreview, ImportPlanView, ImportPreview, IssueView};

uniffi::setup_scaffolding!();

/// A failure from the core. `message` is written for people.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum ShouciError {
    Failed { kind: ErrorKind, message: String },
}

impl std::fmt::Display for ShouciError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self::Failed { message, .. } = self;
        f.write_str(message)
    }
}

impl std::error::Error for ShouciError {}

impl From<shouci_core::Error> for ShouciError {
    fn from(err: shouci_core::Error) -> Self {
        Self::Failed {
            kind: err.kind(),
            message: err.message().to_owned(),
        }
    }
}

type Result<T> = std::result::Result<T, ShouciError>;

/// Where the library lives. See [`shouci_core::Config`].
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreConfig {
    pub data_dir: String,
    pub dictionaries_dir: String,
    pub fetch_dictionaries: bool,
    pub legacy_dir: Option<String>,
}

impl From<&Config> for CoreConfig {
    fn from(config: &Config) -> Self {
        Self {
            data_dir: text(&config.data_dir),
            dictionaries_dir: text(&config.dictionaries_dir),
            fetch_dictionaries: config.fetch_dictionaries,
            legacy_dir: config.legacy_dir.as_deref().map(text),
        }
    }
}

impl From<CoreConfig> for Config {
    fn from(config: CoreConfig) -> Self {
        Self {
            data_dir: PathBuf::from(config.data_dir),
            dictionaries_dir: PathBuf::from(config.dictionaries_dir),
            fetch_dictionaries: config.fetch_dictionaries,
            legacy_dir: config.legacy_dir.map(PathBuf::from),
        }
    }
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// The standard locations, as [`shouci_core::Config::from_env`] finds them.
#[uniffi::export]
#[must_use]
pub fn default_config() -> CoreConfig {
    CoreConfig::from(&Config::from_env())
}

/// Longest query search accepts, in characters.
#[uniffi::export]
#[must_use]
pub fn max_query_chars() -> u32 {
    u32::try_from(shouci_core::MAX_QUERY_CHARS).unwrap_or(u32::MAX)
}

/// Numbered pinyin (`xue2 xiao4`) with tone marks (`xué xiào`).
#[uniffi::export]
#[must_use]
pub fn tone_marks(pinyin: &str) -> String {
    shouci_core::text::tone_marks(pinyin)
}

/// A stored definition cleaned up for reading: CC-CEDICT notation such as
/// `CL:個|个[ge4]` becomes `measure word: 个 (gè)`.
#[uniffi::export]
#[must_use]
pub fn display_definition(definition: &str) -> String {
    shouci_core::text::display_definition(definition)
}

/// What [`Core::quick_add`] did. (The core's `QuickAdd`, unboxed.)
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
#[allow(clippy::large_enum_variant)] // `UniFFI` cannot pass a `Box`; it copies the value anyway
pub enum QuickAddResult {
    Saved {
        result: SaveResult,
    },
    /// Several strong matches: nothing was saved. Pick one and save it.
    Ambiguous {
        candidates: Vec<CandidateView>,
    },
}

impl From<QuickAdd> for QuickAddResult {
    fn from(value: QuickAdd) -> Self {
        match value {
            QuickAdd::Saved(result) => Self::Saved { result: *result },
            QuickAdd::Ambiguous { candidates } => Self::Ambiguous { candidates },
        }
    }
}

/// One user's library and dictionaries. Each method is the
/// [`shouci_core::Shouci`] method of the same name.
#[derive(uniffi::Object)]
pub struct Core {
    shouci: Shouci,
}

#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // `UniFFI` hands every argument over owned
#[allow(clippy::missing_errors_doc)] // documented on the `Shouci` methods
impl Core {
    #[uniffi::constructor]
    pub fn open(config: CoreConfig) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            shouci: Shouci::open(config.into())?,
        }))
    }

    #[must_use]
    pub fn config(&self) -> CoreConfig {
        CoreConfig::from(self.shouci.config())
    }

    #[must_use]
    pub fn startup_notes(&self) -> Vec<String> {
        self.shouci.startup_notes().to_vec()
    }

    pub fn data_version(&self) -> Result<i64> {
        Ok(self.shouci.data_version()?)
    }

    pub fn import_poc(&self, legacy_db: String) -> Result<LegacyImport> {
        Ok(self.shouci.import_poc(Path::new(&legacy_db))?)
    }

    pub fn load_dictionaries(&self) -> Result<()> {
        Ok(self.shouci.load_dictionaries()?)
    }

    #[must_use]
    pub fn dictionary_status(&self) -> DictionaryStatus {
        self.shouci.dictionary_status()
    }

    pub fn dictionaries(&self) -> Result<Vec<DictionaryView>> {
        Ok(self.shouci.dictionaries()?)
    }

    pub fn set_enabled_dictionaries(&self, ids: Vec<String>) -> Result<Vec<DictionaryView>> {
        Ok(self.shouci.set_enabled_dictionaries(&ids)?)
    }

    pub fn lookup_in(&self, item_id: i64, dictionary: String) -> Result<Vec<DictionaryEntryView>> {
        Ok(self.shouci.lookup_in(item_id, &dictionary)?)
    }

    pub fn use_definition(&self, item_id: i64, dictionary: String) -> Result<ItemView> {
        Ok(self.shouci.use_definition(item_id, &dictionary)?)
    }

    pub fn search_dictionary(
        &self,
        query: String,
        kind: Option<QueryKind>,
        limit: Option<u32>,
    ) -> Result<DictionaryResults> {
        Ok(self.shouci.search_dictionary(&query, kind, limit)?)
    }

    pub fn search_library(
        &self,
        query: String,
        filter: LibraryFilter,
        kind: Option<QueryKind>,
        limit: Option<u32>,
    ) -> Result<LibraryResults> {
        Ok(self.shouci.search_library(&query, &filter, kind, limit)?)
    }

    pub fn save_candidate(&self, candidate: CandidateView) -> Result<SaveResult> {
        Ok(self.shouci.save_candidate(&candidate)?)
    }

    pub fn quick_add(&self, query: String, kind: Option<QueryKind>) -> Result<QuickAddResult> {
        Ok(self.shouci.quick_add(&query, kind)?.into())
    }

    pub fn add_manual(&self, word: ManualWord) -> Result<SaveResult> {
        Ok(self.shouci.add_manual(&word)?)
    }

    pub fn list_items(&self, filter: LibraryFilter) -> Result<Vec<ItemView>> {
        Ok(self.shouci.list_items(&filter)?)
    }

    pub fn item(&self, id: i64) -> Result<ItemView> {
        Ok(self.shouci.item(id)?)
    }

    pub fn update_item(&self, id: i64, patch: ItemPatch) -> Result<ItemView> {
        Ok(self.shouci.update_item(id, &patch)?)
    }

    pub fn bulk(&self, ids: Vec<i64>, action: BulkAction) -> Result<BulkResult> {
        Ok(self.shouci.bulk(&ids, &action)?)
    }

    pub fn empty_trash(&self) -> Result<BulkResult> {
        Ok(self.shouci.empty_trash()?)
    }

    pub fn tags(&self) -> Result<Vec<GroupView>> {
        Ok(self.shouci.tags()?)
    }

    pub fn rename_tag(&self, from: String, to: String) -> Result<()> {
        Ok(self.shouci.rename_tag(&from, &to)?)
    }

    pub fn delete_tag(&self, name: String) -> Result<()> {
        Ok(self.shouci.delete_tag(&name)?)
    }

    pub fn collections(&self) -> Result<Vec<GroupView>> {
        Ok(self.shouci.collections()?)
    }

    pub fn create_collection(&self, name: String) -> Result<()> {
        Ok(self.shouci.create_collection(&name)?)
    }

    pub fn rename_collection(&self, from: String, to: String) -> Result<()> {
        Ok(self.shouci.rename_collection(&from, &to)?)
    }

    pub fn delete_collection(&self, name: String) -> Result<()> {
        Ok(self.shouci.delete_collection(&name)?)
    }

    pub fn collection_from_tag(&self, tag: String) -> Result<GroupView> {
        Ok(self.shouci.collection_from_tag(&tag)?)
    }

    #[must_use]
    pub fn connectors(&self) -> Vec<ConnectorView> {
        self.shouci.connectors()
    }

    pub fn preview_import(
        &self,
        path: String,
        connector: String,
        policy: ImportPolicy,
        force: bool,
    ) -> Result<Arc<ImportPreview>> {
        let plan = self
            .shouci
            .preview_import(Path::new(&path), &connector, policy, force)?;
        Ok(Arc::new(ImportPreview::new(plan)))
    }

    pub fn apply_import(&self, preview: Arc<ImportPreview>) -> Result<TransferSummary> {
        Ok(self.shouci.apply_import(preview.plan())?)
    }

    pub fn preview_export(
        &self,
        path: String,
        connector: String,
        request: ExportRequest,
    ) -> Result<Arc<ExportPreview>> {
        let plan = self
            .shouci
            .preview_export(Path::new(&path), &connector, &request)?;
        Ok(Arc::new(ExportPreview::new(plan)))
    }

    pub fn apply_export(&self, preview: Arc<ExportPreview>) -> Result<TransferSummary> {
        Ok(self.shouci.apply_export(preview.plan())?)
    }
}
