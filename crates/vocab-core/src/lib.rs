//! Shared vocabulary types: saved items, dictionary entries, errors, and the
//! [`connector::Connector`] contract that file formats implement.
//!
//! This crate has no required dependencies. Connector crates depend on it and
//! nothing else, so a new format never sees storage.

pub mod connector;

mod dictionary;
mod error;
mod item;

pub use dictionary::{DictionaryEntry, MatchBasis, SourceId, SourceVersion};
pub use error::{ErrorKind, Result, VocabError};
pub use item::{
    ItemPatch, ItemSource, LibraryFilter, LibraryView, Lifecycle, SourceKind, Verification,
    VocabItem,
};
