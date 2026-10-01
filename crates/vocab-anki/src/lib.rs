//! Anki as a [`Connector`]: tab-separated note text (`anki-text/v1`).
//!
//! File grammar only: no `.apkg`, no `AnkiConnect`, no note types.
//!
//! - The `#deck:` header becomes a collection.
//! - Columns a file lacks are omitted, never read as empty.
//! - On export, the deck is the chosen deck, else the one collection being
//!   exported, else none (Anki asks for one on import). Anki tags cannot
//!   contain spaces, so spaces become underscores.

mod text;

pub use text::{AnkiIssue, AnkiNote, AnkiTextV1, IssueSeverity, KEY, NoteRow, ParsedNotes};

use vocab_core::Result;
use vocab_core::connector::{
    Connector, ConnectorInfo, ExchangeRecord, ExportRecord, Field, Issue, ParsedRecords, Severity,
    WriteOptions, Written,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct Anki;

fn field<T>(value: Option<T>) -> Field<T> {
    value.map_or(Field::Omitted, Field::Present)
}

impl Connector for Anki {
    fn info(&self) -> ConnectorInfo {
        ConnectorInfo::new("anki", "Anki", KEY).with_extensions(&["txt", "tsv"])
    }

    fn parse(&self, bytes: &[u8]) -> Result<ParsedRecords> {
        let parsed = AnkiTextV1.parse(bytes)?;
        let collections = field(parsed.deck.map(|deck| vec![deck]));
        let issues = parsed
            .issues
            .into_iter()
            .map(|issue| {
                let severity = match issue.severity {
                    IssueSeverity::Info => Severity::Info,
                    IssueSeverity::Warning => Severity::Warning,
                    IssueSeverity::Error => Severity::Error,
                };
                Issue::new(line(issue.line), severity, issue.message)
            })
            .collect();
        let mut records = Vec::with_capacity(parsed.notes.len());
        let mut issues: Vec<Issue> = issues;
        for note in parsed.notes {
            if note.headword.trim().is_empty() {
                issues.push(Issue::error(line(note.line), "note has no headword"));
                continue;
            }
            let mut record = ExchangeRecord::new(line(note.line), note.raw, note.headword);
            record.traditional = field(note.traditional.filter(|value| !value.is_empty()));
            record.pinyin = field(note.pinyin);
            record.definition = field(note.definition);
            record.notes = field(note.notes);
            record.tags = field(note.tags);
            record.collections = collections.clone();
            records.push(record);
        }
        issues.sort_by_key(|issue| issue.line);
        Ok(ParsedRecords::new(records, issues))
    }

    fn write(&self, records: &[ExportRecord], options: &WriteOptions) -> Result<Written> {
        let deck = options.deck.clone().or_else(|| options.collection.clone());
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
        Ok(Written::new(
            AnkiTextV1.write(&notes, deck.as_deref())?,
            notes_out,
        ))
    }
}

/// Note lines are 1-based `usize`s in the grammar.
fn line(number: usize) -> u32 {
    u32::try_from(number).unwrap_or(u32::MAX)
}
