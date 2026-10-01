//! The `pleco-utf8-text/v1` line grammar, separate from what the lines mean.
//!
//! As Pleco's manual defines it
//! (<https://android.pleco.com/manual/240/flash.html#textformat>):
//!
//! - a record is `characters <tab> pinyin <tab> definition`; pinyin and
//!   definition may be left out;
//! - characters are `simplified` or `simplified[traditional]`;
//! - a line starting with `//` begins a category.
//!
//! Also read, never written: the Shouci proof of concept's `[Category]` lines
//! and `%` comment lines.

use vocab_core::connector::{Issue, line_number};
use vocab_core::{Result, VocabError};

/// One record line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRecord {
    pub line: u32,
    pub raw: String,
    /// As written: `学校` or `学校[學校]`.
    pub characters: String,
    /// `None` when left out or blank.
    pub pinyin: Option<String>,
    /// `None` when left out or blank.
    pub definition: Option<String>,
    pub category: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedFile {
    pub records: Vec<ParsedRecord>,
    pub issues: Vec<Issue>,
}

/// One record line to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRow {
    pub simplified: String,
    /// Written as `simplified[traditional]` when it differs.
    pub traditional: String,
    pub pinyin: String,
    /// Empty: Pleco fills in its own.
    pub definition: String,
    pub category: Option<String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Utf8TextV1;

impl Utf8TextV1 {
    pub const KEY: &'static str = "pleco-utf8-text/v1";

    /// Reads every line. A record without characters is an error issue with
    /// its 1-based line number.
    ///
    /// # Errors
    ///
    /// Only when the bytes are not UTF-8.
    pub fn parse(self, bytes: &[u8]) -> Result<ParsedFile> {
        let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        let text = std::str::from_utf8(bytes).map_err(|err| {
            VocabError::format(format!(
                "file is not valid UTF-8 (expected {}): {err}",
                Self::KEY
            ))
        })?;
        let mut parsed = ParsedFile::default();
        let mut category: Option<String> = None;
        for (index, raw) in text.lines().enumerate() {
            let line = line_number(index);
            // Only the line ending and outer spaces go; tabs are separators.
            let raw = raw.trim_end_matches(['\r', '\n']);
            if raw.trim().is_empty() || raw.trim_start().starts_with('%') {
                continue;
            }
            if let Some(name) = category_line(raw) {
                category = Some(name);
                continue;
            }
            let mut fields = raw.split('\t');
            let characters = fields.next().unwrap_or_default().trim().to_owned();
            let pinyin = fields.next().map(str::trim).filter(|v| !v.is_empty());
            let rest: Vec<&str> = fields.collect();
            let definition = Some(rest.join("\t").trim().to_owned()).filter(|v| !v.is_empty());
            if characters.is_empty() {
                parsed
                    .issues
                    .push(Issue::error(line, "record has no characters"));
                continue;
            }
            parsed.records.push(ParsedRecord {
                line,
                raw: raw.to_owned(),
                characters,
                pinyin: pinyin.map(str::to_owned),
                definition,
                category: category.clone(),
            });
        }
        Ok(parsed)
    }

    /// Writes rows in order, starting a `//Category` line whenever the
    /// category changes.
    #[must_use]
    pub fn serialize(self, rows: &[ExportRow]) -> Vec<u8> {
        let mut out = String::new();
        let mut active: Option<&str> = None;
        for row in rows {
            if row.category.as_deref() != active {
                if let Some(category) = row.category.as_deref() {
                    out.push_str("//");
                    out.push_str(&one_line(category));
                    out.push('\n');
                }
                active = row.category.as_deref();
            }
            out.push_str(&one_line(&row.simplified));
            let traditional = one_line(&row.traditional);
            if !traditional.is_empty() && traditional != one_line(&row.simplified) {
                out.push('[');
                out.push_str(&traditional);
                out.push(']');
            }
            let pinyin = one_line(&row.pinyin);
            let definition = one_line(&row.definition);
            if !pinyin.is_empty() || !definition.is_empty() {
                out.push('\t');
                out.push_str(&pinyin);
            }
            if !definition.is_empty() {
                out.push('\t');
                out.push_str(&definition);
            }
            out.push('\n');
        }
        out.into_bytes()
    }
}

/// Tabs and line breaks would split a field; they become spaces.
fn one_line(value: &str) -> String {
    value.replace(['\r', '\n', '\t'], " ").trim().to_owned()
}

/// `//Name`, or the proof of concept's `[Name]` (on a line with no tabs).
fn category_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let name = if let Some(name) = trimmed.strip_prefix("//") {
        name
    } else if !trimmed.contains('\t') {
        trimmed.strip_prefix('[')?.strip_suffix(']')?
    } else {
        return None;
    };
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}
