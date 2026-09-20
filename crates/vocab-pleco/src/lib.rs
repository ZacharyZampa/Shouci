//! Pleco text exchange adapters.
//!
//! The format is a pluggable, versioned codec — when Pleco changes its UTF-8 text
//! grammar, a new variant slots into the [`CodecRegistry`] without touching the
//! staging/validation/diff layers upstream. Each codec is pinned to golden
//! fixtures under `fixtures/pleco`.

mod codec;
mod registry;

pub use codec::{
    CodecIdent, ExportRow, IssueSeverity, ParseIssue, ParsedFile, ParsedRecord, PlecoCodec,
    Utf8TextV1,
};
pub use registry::CodecRegistry;
