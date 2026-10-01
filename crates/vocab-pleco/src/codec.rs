//! The `pleco-utf8-text/v1` line grammar, separate from what the lines mean.
//!
//! - records: `headword \t pinyin \t definition`;
//! - `[Name]` lines start a category for the records after them;
//! - `%` lines are comments; blank lines are ignored.

use vocab_core::connector::{Issue, Severity};
use vocab_core::{Result, VocabError};

/// One record line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRecord {
    pub line: usize,
    pub raw: String,
    pub headword: String,
    pub pinyin: String,
    /// `None` when the line has no definition field at all.
    pub definition: Option<String>,
    pub category: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedFile {
    pub records: Vec<ParsedRecord>,
    pub issues: Vec<Issue>,
}

impl ParsedFile {
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.issues
            .iter()
            .any(|issue| issue.severity == Severity::Error)
    }
}

/// One record line to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRow {
    pub headword: String,
    pub pinyin: String,
    pub definition: String,
    pub category: Option<String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Utf8TextV1;

impl Utf8TextV1 {
    pub const KEY: &'static str = "pleco-utf8-text/v1";
    const COMMENT: char = '%';
    const SEPARATOR: char = '\t';

    /// Reads every line. Lines that cannot be records become issues with
    /// their 1-based line number.
    ///
    /// # Errors
    ///
    /// Only when the bytes are not UTF-8.
    pub fn parse(self, bytes: &[u8]) -> Result<ParsedFile> {
        let text = std::str::from_utf8(bytes).map_err(|err| {
            VocabError::format(format!(
                "file is not valid UTF-8 (expected {}): {err}",
                Self::KEY
            ))
        })?;
        let mut parsed = ParsedFile::default();
        let mut category: Option<String> = None;
        for (index, raw) in text.lines().enumerate() {
            let line = index + 1;
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.starts_with(Self::COMMENT) {
                parsed
                    .issues
                    .push(issue(line, Severity::Info, "comment line"));
                continue;
            }
            if let Some(name) = category_header(trimmed) {
                category = Some(name);
                continue;
            }
            let fields: Vec<&str> = trimmed.split(Self::SEPARATOR).collect();
            if fields.len() < 2 {
                parsed.issues.push(issue(
                    line,
                    Severity::Error,
                    "record has no pinyin or definition fields",
                ));
                continue;
            }
            let definition = if fields.len() == 2 {
                parsed.issues.push(issue(
                    line,
                    Severity::Warning,
                    "record has no definition field",
                ));
                None
            } else {
                Some(fields[2..].join("\t").trim().to_owned())
            };
            parsed.records.push(ParsedRecord {
                line,
                raw: trimmed.to_owned(),
                headword: fields[0].trim().to_owned(),
                pinyin: fields[1].trim().to_owned(),
                definition,
                category: category.clone(),
            });
        }
        Ok(parsed)
    }

    /// Writes rows in order, starting a `[category]` line whenever the
    /// category changes.
    #[must_use]
    pub fn serialize(self, rows: &[ExportRow]) -> Vec<u8> {
        let mut out = String::new();
        let mut active: Option<&str> = None;
        for row in rows {
            if row.category.as_deref() != active {
                if let Some(category) = row.category.as_deref() {
                    out.push('[');
                    out.push_str(&one_line(category));
                    out.push_str("]\n");
                }
                active = row.category.as_deref();
            }
            out.push_str(&one_line(&row.headword));
            out.push(Self::SEPARATOR);
            out.push_str(&one_line(&row.pinyin));
            out.push(Self::SEPARATOR);
            out.push_str(&one_line(&row.definition));
            out.push('\n');
        }
        out.into_bytes()
    }
}

fn issue(line: usize, severity: Severity, message: &str) -> Issue {
    Issue {
        line,
        severity,
        message: message.to_owned(),
    }
}

fn one_line(value: &str) -> String {
    value.replace(['\r', '\n', '\t'], " ")
}

fn category_header(line: &str) -> Option<String> {
    let name = line.strip_prefix('[')?.strip_suffix(']')?.trim();
    (!name.is_empty()).then(|| name.to_owned())
}
