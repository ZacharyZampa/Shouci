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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteRow {
    pub line: usize,
    pub headword: String,
    pub traditional: String,
    pub pinyin: String,
    pub definition: String,
    pub notes: String,
    pub tags: Vec<String>,
    pub raw: String,
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
        let text = std::str::from_utf8(bytes).map_err(|err| {
            VocabError::new(format!("file is not valid UTF-8 (expected {KEY}): {err}"))
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
            let line = raw.trim_end_matches('\r').trim();
            if line.is_empty() {
                continue;
            }
            if let Some(directive) = line.strip_prefix('#') {
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
                return Err(VocabError::new(
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
    pub fn write(self, notes: &[NoteRow], deck: &str) -> Result<Vec<u8>> {
        if deck.contains(['\n', '\r']) {
            return Err(VocabError::new("deck name cannot contain a newline"));
        }
        let mut out = String::new();
        out.push_str("#separator:tab\n");
        out.push_str("#html:false\n");
        out.push_str("#deck:");
        out.push_str(deck);
        out.push('\n');
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
    traditional: String,
    pinyin: String,
    definition: String,
    notes: String,
    tags: Vec<String>,
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
                return Err(VocabError::new(
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
        traditional: fields[1].trim().to_owned(),
        pinyin: fields[2].trim().to_owned(),
        definition: fields[3].trim().to_owned(),
        notes: fields[4].trim().to_owned(),
        tags: split_tags(fields[5]),
    })
}

fn from_named(fields: &[&str], columns: &[String], tags_column: Option<usize>) -> MappedFields {
    let mut mapped = MappedFields {
        headword: String::new(),
        traditional: String::new(),
        pinyin: String::new(),
        definition: String::new(),
        notes: String::new(),
        tags: Vec::new(),
    };
    for (index, name) in columns.iter().enumerate() {
        let value = fields[index].trim();
        if tags_column == Some(index + 1) || name == "tags" {
            mapped.tags = split_tags(value);
            continue;
        }
        match name.as_str() {
            "headword" | "simplified" => value.clone_into(&mut mapped.headword),
            "traditional" => value.clone_into(&mut mapped.traditional),
            "pinyin" => value.clone_into(&mut mapped.pinyin),
            "definition" | "meaning" | "english" => value.clone_into(&mut mapped.definition),
            "notes" => value.clone_into(&mut mapped.notes),
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
