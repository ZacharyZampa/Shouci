//! Dictionary access and ingestion adapters.
//!
//! Runtime code programs against [`DictionaryProvider`] (see [`SqliteDictionary`]
//! for the concrete read-only SQLite implementation); build-time ingestion happens
//! through [`IngestSource`] (see [`CedictSource`]). Sources are composable (a base
//! lexicon plus enrichment layers) and swappable — re-ingesting into a new
//! read-only `dictionary.db` never requires app-code changes.

mod cedict;
mod fetch;
mod frequency;
mod hsk;
mod ingest;
mod provider;
mod schema;
mod sqlite;

pub use cedict::CedictSource;
pub use fetch::ensure_dictionary_db;
pub use frequency::FrequencySource;
pub use hsk::HskSource;
pub use ingest::{DataSourceDescriptor, IngestSource, LayerKind, RawEntry};
pub(crate) use ingest::{VALUE_FREQUENCY_RANK, VALUE_HSK_RANK};
pub use provider::{Candidate, CandidateDiagnostic, DictionaryProvider};
pub use sqlite::{
    BuildStats, SqliteDictionary, build_dictionary_db, build_dictionary_db_with_layers, sha256_hex,
};
