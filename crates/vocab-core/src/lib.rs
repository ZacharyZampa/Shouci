//! Shared types, provenance invariants, and the canonical error type.

pub mod error;
pub mod provenance;
pub mod status;

mod entry;

pub use entry::{DictionaryEntry, VocabItem};
pub use error::{Result, VocabError};
pub use provenance::{ConfirmationState, Provenance, SourceId, SourceVersion};
pub use status::ItemStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MatchBasis {
    EnglishGloss,
    Pinyin,
    Simplified,
    CharacterFallback,
}
