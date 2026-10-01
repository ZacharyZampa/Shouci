//! The contract a file format implements to move vocabulary in and out.
//!
//! A connector only turns bytes into [`ExchangeRecord`]s and
//! [`ExportRecord`]s into bytes. Dictionary resolution, duplicate handling,
//! and storage belong to the exchange engine, so a new connector is one crate
//! that depends on `vocab-core` alone, plus one registration line in
//! `shouci-core`.
//!
//! The engine hands [`Connector::parse`] text-ready bytes: a UTF-8 byte-order
//! mark is already removed. Line endings may still be `\r\n`; use
//! [`str::lines`].
//!
//! Every type here is `#[non_exhaustive]` so fields can be added without
//! breaking connectors: build them with their constructors, then set fields.
//!
//! The contract is file-based on purpose. A live connector (an API, a sync
//! service) would be a different trait.

use crate::Result;

/// A field a record may or may not carry. `Omitted` means "this file says
/// nothing about it". Blank text means the same thing: a file cannot clear a
/// saved definition by leaving a cell empty. Use [`Field::text`] to read text.
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

/// A 1-based line number from a 0-based index.
#[must_use]
pub fn line_number(index: usize) -> u32 {
    u32::try_from(index + 1).unwrap_or(u32::MAX)
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
#[non_exhaustive]
pub struct Issue {
    /// 1-based.
    pub line: u32,
    pub severity: Severity,
    pub message: String,
}

impl Issue {
    pub fn new(line: u32, severity: Severity, message: impl Into<String>) -> Self {
        Self {
            line,
            severity,
            message: message.into(),
        }
    }

    pub fn info(line: u32, message: impl Into<String>) -> Self {
        Self::new(line, Severity::Info, message)
    }

    pub fn warning(line: u32, message: impl Into<String>) -> Self {
        Self::new(line, Severity::Warning, message)
    }

    pub fn error(line: u32, message: impl Into<String>) -> Self {
        Self::new(line, Severity::Error, message)
    }
}

/// One word read from a file.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ExchangeRecord {
    /// 1-based line in the file.
    pub line: u32,
    /// The line as written, for audit and error messages.
    pub raw: String,
    /// The word as the file writes it; usually simplified.
    pub headword: String,
    pub traditional: Field<String>,
    pub pinyin: Field<String>,
    pub definition: Field<String>,
    pub notes: Field<String>,
    /// Tags the word has in the file. The engine only ever adds tags.
    pub tags: Field<Vec<String>>,
    /// Pleco categories, an Anki deck. The engine only ever adds
    /// collections: formats keep one per word, so a file never says a word
    /// left one.
    pub collections: Field<Vec<String>>,
}

impl ExchangeRecord {
    /// A record with every optional field omitted.
    #[must_use]
    pub fn new(line: u32, raw: impl Into<String>, headword: impl Into<String>) -> Self {
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
#[non_exhaustive]
pub struct ParsedRecords {
    pub records: Vec<ExchangeRecord>,
    pub issues: Vec<Issue>,
}

impl ParsedRecords {
    #[must_use]
    pub fn new(records: Vec<ExchangeRecord>, issues: Vec<Issue>) -> Self {
        Self { records, issues }
    }

    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.issues
            .iter()
            .any(|issue| issue.severity == Severity::Error)
    }
}

/// One saved word on its way out.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ExportRecord {
    pub simplified: String,
    pub traditional: String,
    /// CC-CEDICT numbered pinyin, when known.
    pub pinyin: String,
    pub definition: String,
    pub notes: String,
    /// The definition is exactly what the dictionary says for this reading.
    /// Pleco leaves such definitions blank so Pleco shows its own.
    pub definition_is_dictionary_default: bool,
    pub tags: Vec<String>,
    pub collections: Vec<String>,
}

impl ExportRecord {
    #[must_use]
    pub fn new(
        simplified: impl Into<String>,
        traditional: impl Into<String>,
        pinyin: impl Into<String>,
        definition: impl Into<String>,
    ) -> Self {
        Self {
            simplified: simplified.into(),
            traditional: traditional.into(),
            pinyin: pinyin.into(),
            definition: definition.into(),
            notes: String::new(),
            definition_is_dictionary_default: false,
            tags: Vec::new(),
            collections: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct WriteOptions {
    /// Set when the export is exactly one collection.
    pub collection: Option<String>,
    /// Anki deck name. Without it (and without a collection), no deck is
    /// written and Anki asks.
    pub deck: Option<String>,
}

/// The bytes to write, plus notes on what the format could not carry.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Written {
    pub bytes: Vec<u8>,
    pub notes: Vec<String>,
}

impl Written {
    #[must_use]
    pub fn new(bytes: Vec<u8>, notes: Vec<String>) -> Self {
        Self { bytes, notes }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
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

impl ConnectorInfo {
    /// A connector that imports and exports `.txt` files.
    #[must_use]
    pub fn new(id: &'static str, name: &'static str, format: &'static str) -> Self {
        Self {
            id,
            name,
            format,
            extensions: &["txt"],
            can_import: true,
            can_export: true,
        }
    }

    #[must_use]
    pub fn with_extensions(mut self, extensions: &'static [&'static str]) -> Self {
        self.extensions = extensions;
        self
    }
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

    /// Writes records. A format may group them (Pleco writes each category
    /// once).
    ///
    /// # Errors
    ///
    /// When a record or option cannot be represented in the format.
    fn write(&self, records: &[ExportRecord], options: &WriteOptions) -> Result<Written>;
}

#[cfg(test)]
mod tests {
    use super::{ExchangeRecord, Field, line_number};

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

    #[test]
    fn line_numbers_are_one_based() {
        assert_eq!(line_number(0), 1);
    }
}
