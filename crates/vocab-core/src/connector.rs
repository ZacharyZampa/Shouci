//! The contract a file format implements to move vocabulary in and out.
//!
//! A connector only turns bytes into [`ExchangeRecord`]s and
//! [`ExportRecord`]s into bytes. Dictionary resolution, duplicate handling,
//! and storage belong to the exchange engine, so a new connector is one crate
//! that depends on `vocab-core` alone, plus one registration line in
//! `shouci-core`.
//!
//! The contract is file-based on purpose. A live connector (an API, a sync
//! service) would be a different trait.

use crate::Result;

/// A field a record may or may not carry. `Omitted` means "this file says
/// nothing about it", which is different from an empty value: an omitted
/// field never overwrites saved data or counts as a conflict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Field<T> {
    Present(T),
    Omitted,
}

impl<T> Field<T> {
    #[must_use]
    pub fn present(&self) -> Option<&T> {
        match self {
            Self::Present(value) => Some(value),
            Self::Omitted => None,
        }
    }

    #[must_use]
    pub fn is_present(&self) -> bool {
        matches!(self, Self::Present(_))
    }
}

impl Field<String> {
    /// The value, trimmed, when present and not blank.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        self.present()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum Severity {
    Info,
    /// Imported, but something about the line is worth knowing.
    Warning,
    /// The line cannot be imported. The whole import is refused unless forced.
    Error,
}

impl Severity {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

/// Something noteworthy about one line of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Issue {
    /// 1-based.
    pub line: usize,
    pub severity: Severity,
    pub message: String,
}

/// One word read from a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExchangeRecord {
    /// 1-based line in the file.
    pub line: usize,
    /// The line as written, for audit and error messages.
    pub raw: String,
    /// The word as the file writes it; usually simplified.
    pub headword: String,
    pub traditional: Field<String>,
    pub pinyin: Field<String>,
    pub definition: Field<String>,
    pub notes: Field<String>,
    pub tags: Field<Vec<String>>,
    /// Pleco categories, an Anki deck.
    pub collections: Field<Vec<String>>,
}

impl ExchangeRecord {
    /// A record with every optional field omitted.
    #[must_use]
    pub fn new(line: usize, raw: impl Into<String>, headword: impl Into<String>) -> Self {
        Self {
            line,
            raw: raw.into(),
            headword: headword.into(),
            traditional: Field::Omitted,
            pinyin: Field::Omitted,
            definition: Field::Omitted,
            notes: Field::Omitted,
            tags: Field::Omitted,
            collections: Field::Omitted,
        }
    }
}

/// Everything read from one file. Lines that could not be read become
/// [`Issue`]s, never silent drops.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedRecords {
    pub records: Vec<ExchangeRecord>,
    pub issues: Vec<Issue>,
}

impl ParsedRecords {
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.issues
            .iter()
            .any(|issue| issue.severity == Severity::Error)
    }
}

/// One saved word on its way out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRecord {
    pub simplified: String,
    pub traditional: String,
    pub pinyin: String,
    pub definition: String,
    pub notes: String,
    /// The definition is exactly what the dictionary says for this reading.
    /// Pleco leaves such definitions blank so Pleco shows its own.
    pub definition_is_dictionary_default: bool,
    pub tags: Vec<String>,
    pub collections: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WriteOptions {
    /// Set when the export is exactly one collection.
    pub collection: Option<String>,
    /// Anki deck name; defaults to the collection, then "Shouci".
    pub deck: Option<String>,
}

/// The bytes to write, plus notes on what the format could not carry.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Written {
    pub bytes: Vec<u8>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorInfo {
    /// The destination key, stable across format versions: `pleco`.
    /// "Export only new words" is tracked per id.
    pub id: &'static str,
    pub name: &'static str,
    /// The exact grammar, recorded with every transfer: `pleco-utf8-text/v1`.
    pub format: &'static str,
    pub extensions: &'static [&'static str],
    pub can_import: bool,
    pub can_export: bool,
}

pub trait Connector: Send + Sync {
    fn info(&self) -> ConnectorInfo;

    /// Reads a file. Malformed lines become [`Issue`]s.
    ///
    /// # Errors
    ///
    /// Only when the file as a whole cannot be read as this format (not
    /// UTF-8, wrong separator).
    fn parse(&self, bytes: &[u8]) -> Result<ParsedRecords>;

    /// Writes records in the order given.
    ///
    /// # Errors
    ///
    /// When a record or option cannot be represented in the format.
    fn write(&self, records: &[ExportRecord], options: &WriteOptions) -> Result<Written>;
}

#[cfg(test)]
mod tests {
    use super::{ExchangeRecord, Field};

    #[test]
    fn new_records_omit_everything_optional() {
        let record = ExchangeRecord::new(1, "你好", "你好");
        assert_eq!(record.definition, Field::Omitted);
        assert_eq!(record.tags, Field::Omitted);
    }

    #[test]
    fn blank_text_is_not_text() {
        assert_eq!(Field::Present("  ".to_owned()).text(), None);
        assert_eq!(Field::Present(" hi ".to_owned()).text(), Some("hi"));
        assert_eq!(Field::<String>::Omitted.text(), None);
    }
}
