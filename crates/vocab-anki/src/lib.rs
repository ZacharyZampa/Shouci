//! Anki as a [`Connector`]: tab-separated note text (`anki-text/v1`).
//!
//! File grammar only: no `.apkg`, no `AnkiConnect`, no note types.
//!
//! - The `#deck:` header becomes a collection.
//! - Columns a file lacks are omitted, never read as empty.
//! - On export, the deck is the chosen deck, else the one collection being
//!   exported, else "Shouci". Anki tags cannot contain spaces, so spaces
//!   become underscores.

mod text;

pub use text::{AnkiIssue, AnkiNote, AnkiTextV1, IssueSeverity, KEY, NoteRow, ParsedNotes};

use vocab_core::Result;
use vocab_core::connector::{
    Connector, ConnectorInfo, ExchangeRecord, ExportRecord, Field, Issue, ParsedRecords, Severity,
    WriteOptions, Written,
};

/// Deck used when neither a deck nor a single collection is given.
pub const DEFAULT_DECK: &str = "Shouci";

#[derive(Debug, Clone, Copy, Default)]
pub struct Anki;

fn field<T>(value: Option<T>) -> Field<T> {
    value.map_or(Field::Omitted, Field::Present)
}

impl Connector for Anki {
    fn info(&self) -> ConnectorInfo {
        ConnectorInfo {
            id: "anki",
            name: "Anki",
            format: KEY,
            extensions: &["txt", "tsv"],
            can_import: true,
            can_export: true,
        }
    }

    fn parse(&self, bytes: &[u8]) -> Result<ParsedRecords> {
        let parsed = AnkiTextV1.parse(bytes)?;
        let collections = field(parsed.deck.map(|deck| vec![deck]));
        let issues = parsed
            .issues
            .into_iter()
            .map(|issue| Issue {
                line: issue.line,
                severity: match issue.severity {
                    IssueSeverity::Info => Severity::Info,
                    IssueSeverity::Warning => Severity::Warning,
                    IssueSeverity::Error => Severity::Error,
                },
                message: issue.message,
            })
            .collect();
        let mut records = Vec::with_capacity(parsed.notes.len());
        let mut issues: Vec<Issue> = issues;
        for note in parsed.notes {
            if note.headword.trim().is_empty() {
                issues.push(Issue {
                    line: note.line,
                    severity: Severity::Error,
                    message: "note has no headword".to_owned(),
                });
                continue;
            }
            let mut record = ExchangeRecord::new(note.line, note.raw, note.headword);
            record.traditional = field(note.traditional.filter(|value| !value.is_empty()));
            record.pinyin = field(note.pinyin);
            record.definition = field(note.definition);
            record.notes = field(note.notes);
            record.tags = field(note.tags);
            record.collections = collections.clone();
            records.push(record);
        }
        issues.sort_by_key(|issue| issue.line);
        Ok(ParsedRecords { records, issues })
    }

    fn write(&self, records: &[ExportRecord], options: &WriteOptions) -> Result<Written> {
        let deck = options
            .deck
            .clone()
            .or_else(|| options.collection.clone())
            .unwrap_or_else(|| DEFAULT_DECK.to_owned());
        let mut renamed: Vec<String> = Vec::new();
        let notes: Vec<AnkiNote> = records
            .iter()
            .map(|record| {
                let mut tags: Vec<String> = record
                    .tags
                    .iter()
                    .map(|tag| {
                        if tag.contains(char::is_whitespace) {
                            let safe = tag.split_whitespace().collect::<Vec<_>>().join("_");
                            let change = format!("{tag} → {safe}");
                            if !renamed.contains(&change) {
                                renamed.push(change);
                            }
                            safe
                        } else {
                            tag.clone()
                        }
                    })
                    .collect();
                tags.sort();
                tags.dedup();
                AnkiNote {
                    headword: record.simplified.clone(),
                    traditional: record.traditional.clone(),
                    pinyin: record.pinyin.clone(),
                    definition: record.definition.clone(),
                    notes: record.notes.clone(),
                    tags,
                }
            })
            .collect();
        let mut notes_out = Vec::new();
        if !renamed.is_empty() {
            notes_out.push(format!(
                "tags renamed for Anki (no spaces allowed): {}",
                renamed.join(", ")
            ));
        }
        Ok(Written {
            bytes: AnkiTextV1.write(&notes, &deck)?,
            notes: notes_out,
        })
    }
}
