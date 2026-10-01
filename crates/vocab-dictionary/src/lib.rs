//! Dictionaries: building them, finding them, and querying them.
//!
//! Each dictionary is its own read-only SQLite file (see [`catalog`]).
//! Runtime code programs against [`DictionaryProvider`]: one
//! [`SqliteDictionary`], or a [`DictionarySet`] of several in priority order.
//! Building happens through [`IngestSource`] (see [`CedictSource`]): a base
//! lexicon plus enrichment layers (frequency, HSK).

pub mod catalog;

mod cedict;
mod display;
mod fetch;
mod frequency;
mod hsk;
mod ingest;
mod provider;
mod schema;
mod set;
mod sqlite;

pub use catalog::{DictionaryInfo, DictionarySpec};
pub use cedict::CedictSource;
pub use display::display_definition;
pub use fetch::{BuildState, Ensured, FetchStage, build_state, ensure_dictionary_db};
pub use frequency::FrequencySource;
pub use hsk::HskSource;
pub use ingest::{DataSourceDescriptor, IngestSource, LayerKind, RawEntry};
pub(crate) use ingest::{VALUE_FREQUENCY_RANK, VALUE_HSK_RANK};
pub use provider::{Candidate, CandidateDiagnostic, DictionaryProvider};
pub use set::DictionarySet;
pub use sqlite::{
    BuildStats, SqliteDictionary, build_dictionary_db, build_dictionary_db_with_layers, sha256_hex,
};
