//! Pleco as a [`Connector`]: UTF-8 flashcard text files
//! (`pleco-utf8-text/v1`), never `.pqb`.
//!
//! - A `[Category]` line becomes a collection.
//! - A headword written `简体[繁體]` carries its traditional form.
//! - A blank definition means "use Pleco's dictionary": it is omitted on
//!   import, and on export a definition identical to the dictionary's is
//!   left blank so Pleco shows its own.

mod codec;

pub use codec::{ExportRow, ParsedFile, ParsedRecord, Utf8TextV1};

use vocab_core::Result;
use vocab_core::connector::{
    Connector, ConnectorInfo, ExchangeRecord, ExportRecord, Field, Issue, ParsedRecords, Severity,
    WriteOptions, Written,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct Pleco;

impl Connector for Pleco {
    fn info(&self) -> ConnectorInfo {
        ConnectorInfo {
            id: "pleco",
            name: "Pleco",
            format: Utf8TextV1::KEY,
            extensions: &["txt"],
            can_import: true,
            can_export: true,
        }
    }

    fn parse(&self, bytes: &[u8]) -> Result<ParsedRecords> {
        let parsed = Utf8TextV1.parse(bytes)?;
        let mut out = ParsedRecords {
            records: Vec::with_capacity(parsed.records.len()),
            issues: parsed.issues,
        };
        for record in parsed.records {
            let (simplified, traditional) = split_headword(&record.headword);
            if simplified.is_empty() {
                out.issues.push(Issue {
                    line: record.line,
                    severity: Severity::Error,
                    message: "record has no headword".to_owned(),
                });
                continue;
            }
            let mut exchange = ExchangeRecord::new(record.line, record.raw, simplified);
            if let Some(traditional) = traditional {
                exchange.traditional = Field::Present(traditional);
            }
            exchange.pinyin = Field::Present(record.pinyin);
            exchange.definition = match record.definition {
                Some(definition) if !definition.is_empty() => Field::Present(definition),
                _ => Field::Omitted,
            };
            if let Some(category) = record.category {
                exchange.collections = Field::Present(vec![category]);
            }
            out.records.push(exchange);
        }
        out.issues.sort_by_key(|issue| issue.line);
        Ok(out)
    }

    fn write(&self, records: &[ExportRecord], options: &WriteOptions) -> Result<Written> {
        let mut rows: Vec<(usize, ExportRow)> = Vec::with_capacity(records.len());
        let mut blank_definitions = 0usize;
        let mut notes_dropped = 0usize;
        let mut traditional_dropped = 0usize;
        let mut extra_collections: Vec<String> = Vec::new();
        for (order, record) in records.iter().enumerate() {
            let category = if let Some(collection) = &options.collection {
                Some(collection.clone())
            } else {
                let mut names = record.collections.clone();
                names.sort_by_key(|name| name.to_lowercase());
                if names.len() > 1 {
                    extra_collections.push(format!(
                        "{} ({})",
                        record.simplified,
                        names[1..].join(", ")
                    ));
                }
                names.into_iter().next()
            };
            let definition = if record.definition_is_dictionary_default {
                blank_definitions += 1;
                String::new()
            } else {
                record.definition.clone()
            };
            if !record.notes.trim().is_empty() {
                notes_dropped += 1;
            }
            if record.traditional != record.simplified {
                traditional_dropped += 1;
            }
            rows.push((
                order,
                ExportRow {
                    headword: record.simplified.clone(),
                    pinyin: record.pinyin.clone(),
                    definition,
                    category,
                },
            ));
        }
        // Uncategorized words first, then each category once, in name order.
        rows.sort_by(|(a_order, a), (b_order, b)| {
            let key = |row: &ExportRow| row.category.as_ref().map(|c| c.to_lowercase());
            key(a).cmp(&key(b)).then(a_order.cmp(b_order))
        });
        let rows: Vec<ExportRow> = rows.into_iter().map(|(_, row)| row).collect();
        let mut notes = Vec::new();
        if blank_definitions > 0 {
            notes.push(format!(
                "{blank_definitions} definitions left blank so Pleco shows its own"
            ));
        }
        if traditional_dropped > 0 {
            notes.push(format!(
                "{traditional_dropped} traditional forms not written (Pleco shows its own)"
            ));
        }
        if notes_dropped > 0 {
            notes.push(format!(
                "{notes_dropped} notes not written (Pleco files have no notes)"
            ));
        }
        if !extra_collections.is_empty() {
            notes.push(format!(
                "only the first collection is written for: {}",
                extra_collections.join("; ")
            ));
        }
        Ok(Written {
            bytes: Utf8TextV1.serialize(&rows),
            notes,
        })
    }
}

/// `学校[學校]` → (`学校`, Some(`學校`)); anything else is simplified only.
fn split_headword(headword: &str) -> (String, Option<String>) {
    let headword = headword.trim();
    if let Some(inner) = headword.strip_suffix(']') {
        if let Some((simplified, traditional)) = inner.split_once('[') {
            let (simplified, traditional) = (simplified.trim(), traditional.trim());
            if !simplified.is_empty() && !traditional.is_empty() {
                return (simplified.to_owned(), Some(traditional.to_owned()));
            }
        }
    }
    (headword.to_owned(), None)
}

#[cfg(test)]
mod tests {
    use super::split_headword;

    #[test]
    fn bracketed_traditional_is_split_off() {
        assert_eq!(
            split_headword("学校[學校]"),
            ("学校".to_owned(), Some("學校".to_owned()))
        );
        assert_eq!(split_headword("你好"), ("你好".to_owned(), None));
        assert_eq!(split_headword("[x]"), ("[x]".to_owned(), None));
    }
}
