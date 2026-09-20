use std::fmt;

use vocab_core::Result;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CodecIdent {
    pub format: &'static str,
    pub variant: &'static str,
}

impl CodecIdent {
    #[must_use]
    pub fn key(&self) -> String {
        format!("{}/{}", self.format, self.variant)
    }
}

impl fmt::Display for CodecIdent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.key())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRow {
    pub simplified: String,
    pub pinyin: String,
    pub definition: String,
    pub category: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRecord {
    pub line: usize,
    pub simplified: String,
    pub pinyin: String,
    pub definition: String,
    pub category: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseIssue {
    pub line: usize,
    pub severity: IssueSeverity,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedFile {
    pub variant: CodecIdent,
    pub records: Vec<ParsedRecord>,
    pub issues: Vec<ParseIssue>,
}

impl ParsedFile {
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.issues
            .iter()
            .any(|i| i.severity == IssueSeverity::Error)
    }
}

/// A versioned, reversible codec for one Pleco text format variant.
///
/// Contract: `parse` never fails hard on a malformed *line* — it records a
/// [`ParseIssue`] with the exact line number so upstream import can reject loudly
/// instead of dropping data silently. `serialize` must produce valid UTF-8 that
/// Pleco accepts. Both directions are deterministic.
pub trait PlecoCodec: Send + Sync {
    fn ident(&self) -> CodecIdent;

    /// Parses one file into staged records with per-line issues.
    ///
    /// # Errors
    ///
    /// Fails only when the input cannot be decoded as UTF-8 at all; malformed
    /// *lines* are surfaced as [`ParseIssue`]s instead.
    fn parse(&self, bytes: &[u8]) -> Result<ParsedFile>;

    /// Serializes rows to valid UTF-8 bytes accepted by Pleco.
    ///
    /// # Errors
    ///
    /// Returns an error if any row cannot be represented in the variant grammar.
    fn serialize(&self, rows: &[ExportRow]) -> Result<Vec<u8>>;
}

/// `pleco-utf8-text/v1`: tab-delimited `headword\tpinyin\tdefinition` records,
/// `[category]` header lines, `%` comment lines.
#[derive(Debug, Clone, Copy, Default)]
pub struct Utf8TextV1;

impl Utf8TextV1 {
    pub const COMMENT: char = '%';
    pub const FIELD_SEPARATOR: char = '\t';
    pub const LINE_ENDING: &'static str = "\n";
}

impl PlecoCodec for Utf8TextV1 {
    fn ident(&self) -> CodecIdent {
        CodecIdent {
            format: "pleco-utf8-text",
            variant: "v1",
        }
    }

    fn parse(&self, bytes: &[u8]) -> Result<ParsedFile> {
        let text = match std::str::from_utf8(bytes) {
            Ok(text) => text,
            Err(err) => {
                return Err(vocab_core::VocabError::new(format!(
                    "file is not valid UTF-8 (expected pleco-utf8-text/v1): {err}"
                )));
            }
        };

        let mut records = Vec::new();
        let mut issues = Vec::new();
        let mut active_category: Option<String> = None;

        for (index, raw) in text.lines().enumerate() {
            let line_number = index + 1;
            let line = raw.trim_end();
            let line = line.trim();

            if line.is_empty() {
                continue;
            }
            if line.starts_with(Self::COMMENT) {
                issues.push(ParseIssue {
                    line: line_number,
                    severity: IssueSeverity::Info,
                    message: "comment line".to_owned(),
                });
                continue;
            }
            if let Some(name) = category_header(line) {
                active_category = Some(name);
                continue;
            }

            let fields: Vec<&str> = line.split(Self::FIELD_SEPARATOR).collect();
            match fields.len() {
                0 => unreachable!(),
                1 => issues.push(ParseIssue {
                    line: line_number,
                    severity: IssueSeverity::Error,
                    message: "record has no pinyin or definition fields".to_owned(),
                }),
                2 => {
                    issues.push(ParseIssue {
                        line: line_number,
                        severity: IssueSeverity::Warning,
                        message: "record has no definition field".to_owned(),
                    });
                    records.push(ParsedRecord {
                        line: line_number,
                        simplified: fields[0].trim().to_owned(),
                        pinyin: fields[1].trim().to_owned(),
                        definition: String::new(),
                        category: active_category.clone(),
                    });
                }
                _ => records.push(ParsedRecord {
                    line: line_number,
                    simplified: fields[0].trim().to_owned(),
                    pinyin: fields[1].trim().to_owned(),
                    definition: fields[2..]
                        .join(Self::FIELD_SEPARATOR_STR)
                        .trim()
                        .to_owned(),
                    category: active_category.clone(),
                }),
            }
        }

        Ok(ParsedFile {
            variant: self.ident(),
            records,
            issues,
        })
    }

    fn serialize(&self, rows: &[ExportRow]) -> Result<Vec<u8>> {
        let mut out = String::new();
        let mut active_category: Option<&str> = None;
        for row in rows {
            if row.category.as_deref() != active_category {
                if let Some(category) = row.category.as_deref() {
                    out.push('[');
                    out.push_str(category);
                    out.push_str("]\n");
                }
                active_category = row.category.as_deref();
            }
            out.push_str(&row.simplified);
            out.push(Self::FIELD_SEPARATOR);
            out.push_str(&row.pinyin.replace('\t', " "));
            out.push(Self::FIELD_SEPARATOR);
            out.push_str(&row.definition.replace(['\r', '\n', '\t'], " "));
            out.push_str(Self::LINE_ENDING);
        }
        Ok(out.into_bytes())
    }
}

impl Utf8TextV1 {
    const FIELD_SEPARATOR_STR: &'static str = "\t";
}

fn category_header(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        let name = trimmed[1..trimmed.len() - 1].trim();
        if !name.is_empty() {
            return Some(name.to_owned());
        }
    }
    None
}
