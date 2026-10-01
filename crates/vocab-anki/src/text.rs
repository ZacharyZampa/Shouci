use vocab_core::{Result, VocabError};

pub const KEY: &str = "anki-text/v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnkiIssue {
    pub line: usize,
    pub severity: IssueSeverity,
    pub message: String,
}

/// One note read from a file. A field is `None` when the file has no column
/// for it (a `#columns:` header without `Notes`), which is different from an
/// empty value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteRow {
    pub line: usize,
    pub headword: String,
    pub traditional: Option<String>,
    pub pinyin: Option<String>,
    pub definition: Option<String>,
    pub notes: Option<String>,
    pub tags: Option<Vec<String>>,
    pub raw: String,
}

/// One note to write.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AnkiNote {
    pub headword: String,
    pub traditional: String,
    pub pinyin: String,
    pub definition: String,
    pub notes: String,
    /// Written space-separated, so they must not contain spaces.
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedNotes {
    pub deck: Option<String>,
    pub html: bool,
    pub notes: Vec<NoteRow>,
    pub issues: Vec<AnkiIssue>,
}

impl ParsedNotes {
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.issues
            .iter()
            .any(|issue| issue.severity == IssueSeverity::Error)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AnkiTextV1;

impl AnkiTextV1 {
    /// Parses an Anki tab-separated note export.
    ///
    /// # Errors
    ///
    /// Fails only when the file is not UTF-8, or when a declared separator is not tab.
    pub fn parse(self, bytes: &[u8]) -> Result<ParsedNotes> {
        let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        let text = std::str::from_utf8(bytes).map_err(|err| {
            VocabError::format(format!("file is not valid UTF-8 (expected {KEY}): {err}"))
        })?;
        let mut deck = None;
        let mut html = false;
        let mut columns: Option<Vec<String>> = None;
        let mut tags_column: Option<usize> = None;
        let mut separator_seen = false;
        let mut issues = Vec::new();
        let mut notes = Vec::new();

        for (index, raw) in text.lines().enumerate() {
            let line_number = index + 1;
            // Trailing tabs are empty fields, not whitespace to trim.
            let line = raw.trim_end_matches(['\r', '\n']);
            if line.trim().is_empty() {
                continue;
            }
            if let Some(directive) = line.trim().strip_prefix('#') {
                apply_directive(
                    directive,
                    &mut DirectiveState {
                        deck: &mut deck,
                        html: &mut html,
                        columns: &mut columns,
                        tags_column: &mut tags_column,
                        separator_seen: &mut separator_seen,
                        issues: &mut issues,
                        line_number,
                    },
                )?;
                continue;
            }
            if !separator_seen && !line.contains('\t') {
                return Err(VocabError::format(
                    "anki-text/v1 is tab-only; re-export with a tab separator",
                ));
            }
            let fields: Vec<&str> = line.split('\t').collect();
            match map_fields(&fields, columns.as_deref(), tags_column) {
                Ok(mapped) => notes.push(NoteRow {
                    line: line_number,
                    headword: mapped.headword,
                    traditional: mapped.traditional,
                    pinyin: mapped.pinyin,
                    definition: mapped.definition,
                    notes: mapped.notes,
                    tags: mapped.tags,
                    raw: line.to_owned(),
                }),
                Err(message) => issues.push(AnkiIssue {
                    line: line_number,
                    severity: IssueSeverity::Error,
                    message,
                }),
            }
        }
        if html {
            issues.push(AnkiIssue {
                line: 1,
                severity: IssueSeverity::Warning,
                message: "HTML left literal; not rendered".to_owned(),
            });
        }
        Ok(ParsedNotes {
            deck,
            html,
            notes,
            issues,
        })
    }

    /// Writes notes Anki can import as tab-separated text.
    ///
    /// # Errors
    ///
    /// Returns an error if the deck name contains a newline.
    pub fn write(self, notes: &[AnkiNote], deck: Option<&str>) -> Result<Vec<u8>> {
        if deck.is_some_and(|deck| deck.contains(['\n', '\r'])) {
            return Err(VocabError::invalid("deck name cannot contain a newline"));
        }
        let mut out = String::new();
        out.push_str("#separator:tab\n");
        out.push_str("#html:false\n");
        if let Some(deck) = deck {
            out.push_str("#deck:");
            out.push_str(deck);
            out.push('\n');
        }
        out.push_str("#columns:Headword\tTraditional\tPinyin\tDefinition\tNotes\tTags\n");
        out.push_str("#tags column:6\n");
        for note in notes {
            out.push_str(&sanitize(&note.headword));
            out.push('\t');
            out.push_str(&sanitize(&note.traditional));
            out.push('\t');
            out.push_str(&sanitize(&note.pinyin));
            out.push('\t');
            out.push_str(&sanitize(&note.definition));
            out.push('\t');
            out.push_str(&sanitize(&note.notes));
            out.push('\t');
            out.push_str(&note.tags.join(" "));
            out.push('\n');
        }
        Ok(out.into_bytes())
    }
}

struct MappedFields {
    headword: String,
    traditional: Option<String>,
    pinyin: Option<String>,
    definition: Option<String>,
    notes: Option<String>,
    tags: Option<Vec<String>>,
}

struct DirectiveState<'a> {
    deck: &'a mut Option<String>,
    html: &'a mut bool,
    columns: &'a mut Option<Vec<String>>,
    tags_column: &'a mut Option<usize>,
    separator_seen: &'a mut bool,
    issues: &'a mut Vec<AnkiIssue>,
    line_number: usize,
}

fn apply_directive(directive: &str, state: &mut DirectiveState<'_>) -> Result<()> {
    let (name, value) = directive
        .split_once(':')
        .map_or((directive.trim(), ""), |(name, value)| {
            (name.trim(), value.trim())
        });
    match name.to_ascii_lowercase().as_str() {
        "separator" => {
            *state.separator_seen = true;
            if !value.eq_ignore_ascii_case("tab") {
                return Err(VocabError::format(
                    "anki-text/v1 is tab-only; re-export with a tab separator",
                ));
            }
        }
        "html" => *state.html = value.eq_ignore_ascii_case("true"),
        "deck" => {
            if !value.is_empty() {
                *state.deck = Some(value.to_owned());
            }
        }
        "columns" => {
            *state.columns = Some(
                value
                    .split('\t')
                    .map(|part| part.trim().to_ascii_lowercase())
                    .filter(|part| !part.is_empty())
                    .collect(),
            );
        }
        "tags column" => {
            *state.tags_column = value.parse::<usize>().ok().filter(|n| *n > 0);
            if state.tags_column.is_none() {
                state.issues.push(AnkiIssue {
                    line: state.line_number,
                    severity: IssueSeverity::Warning,
                    message: "ignored tags column directive".to_owned(),
                });
            }
        }
        "notetype" => {}
        other => {
            state.issues.push(AnkiIssue {
                line: state.line_number,
                severity: IssueSeverity::Warning,
                message: format!("ignored Anki directive {other}"),
            });
        }
    }
    Ok(())
}

fn map_fields(
    fields: &[&str],
    columns: Option<&[String]>,
    tags_column: Option<usize>,
) -> std::result::Result<MappedFields, String> {
    if let Some(columns) = columns {
        if fields.len() != columns.len() {
            return Err(format!(
                "expected {} fields, found {}",
                columns.len(),
                fields.len()
            ));
        }
        return Ok(from_named(fields, columns, tags_column));
    }
    if fields.len() != 6 {
        return Err(format!("expected 6 fields, found {}", fields.len()));
    }
    Ok(MappedFields {
        headword: fields[0].trim().to_owned(),
        traditional: Some(fields[1].trim().to_owned()),
        pinyin: Some(fields[2].trim().to_owned()),
        definition: Some(fields[3].trim().to_owned()),
        notes: Some(fields[4].trim().to_owned()),
        tags: Some(split_tags(fields[5])),
    })
}

fn from_named(fields: &[&str], columns: &[String], tags_column: Option<usize>) -> MappedFields {
    let mut mapped = MappedFields {
        headword: String::new(),
        traditional: None,
        pinyin: None,
        definition: None,
        notes: None,
        tags: None,
    };
    for (index, name) in columns.iter().enumerate() {
        let value = fields[index].trim();
        if tags_column == Some(index + 1) || name == "tags" {
            mapped.tags = Some(split_tags(value));
            continue;
        }
        if matches!(name.as_str(), "headword" | "simplified") {
            value.clone_into(&mut mapped.headword);
            continue;
        }
        let value = Some(value.to_owned());
        match name.as_str() {
            "traditional" => mapped.traditional = value,
            "pinyin" => mapped.pinyin = value,
            "definition" | "meaning" | "english" => mapped.definition = value,
            "notes" => mapped.notes = value,
            _ => {}
        }
    }
    mapped
}

fn split_tags(value: &str) -> Vec<String> {
    value
        .split_whitespace()
        .filter(|tag| !tag.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn sanitize(value: &str) -> String {
    value
        .replace(['\t', '\r', '\n'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
