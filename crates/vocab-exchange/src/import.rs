//! Import: file → plan → library.
//!
//! Rules that keep imports from losing data:
//! - an omitted or blank field never changes a saved word;
//! - tags and collections are only ever added, never removed (files hold
//!   one category or deck per word, so they cannot say a word left one),
//!   whatever the policy: the policy decides a saved word's text only;
//! - a tag or collection is the library's own when only case, spaces, or
//!   underscores differ (`week 1`, `Week_1` → `Week 1`), so a file never
//!   splits one group in two;
//! - a word repeated in one file is folded into its first line: tags and
//!   collections combined, the first definition kept, differences reported.

use std::collections::HashMap;
use std::str::FromStr;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use vocab_core::connector::{Connector, ExchangeRecord, Field, Issue, Severity};
use vocab_core::{ItemPatch, ItemSource, Result, SourceKind, Verification, VocabError, VocabItem};
use vocab_db::{
    Direction, NewItem, Outcome, RunCounts, RunRecord, RunStatus, add_tag, add_to_collection,
    find_by_identity, insert_item, item_collections, item_tags, record_item, record_run,
    require_item, set_trashed, update_item, with_tx,
};
use vocab_dictionary::DictionaryProvider;

use crate::resolve::{Resolved, resolve};
use crate::{TransferSummary, content_hash, count, reading_key, strip_bom};

/// What to do with the text of a word that is already saved. Every policy
/// adds the tags and collections the file gives the word.
///
/// The default is the one every frontend starts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportPolicy {
    /// Keep the saved word's text. A word in the trash stays there,
    /// untouched.
    Skip,
    /// Replace each text field the file fills in.
    Overwrite,
    /// Fill blank text fields. Where both sides have different values, keep
    /// the saved one and report the conflict. A word in the trash comes back.
    #[default]
    Merge,
}

impl FromStr for ImportPolicy {
    type Err = VocabError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "skip" => Ok(Self::Skip),
            "overwrite" => Ok(Self::Overwrite),
            "merge" => Ok(Self::Merge),
            other => Err(VocabError::invalid(format!(
                "unknown import policy '{other}' (expected skip, overwrite, or merge)"
            ))),
        }
    }
}

/// A new word, as it will be saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Incoming {
    pub simplified: String,
    /// Empty when unknown; saved as the simplified form.
    pub traditional: String,
    pub pinyin: String,
    pub definition: String,
    pub notes: String,
    pub verification: Verification,
    pub tags: Vec<String>,
    pub collections: Vec<String>,
}

/// A text field an import can change on a saved word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportField {
    Definition,
    Notes,
}

impl ImportField {
    /// The field's name for people: `definition`, `notes`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Definition => "definition",
            Self::Notes => "notes",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldChange {
    pub field: ImportField,
    pub from: String,
    pub to: String,
}

/// Both sides have a value and they differ; the saved one is kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldConflict {
    pub field: ImportField,
    pub kept: String,
    pub incoming: String,
}

/// Tags or collections a word gains. Imports never remove any.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct NameChange {
    pub add: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// Skip kept the saved word's text, and the file adds no tags or
    /// collections to it.
    AlreadySaved,
    /// Saved, but in the trash. Skip leaves it there.
    InTrash,
    /// The policy would change nothing.
    Unchanged,
    /// An earlier line of the same file has the same word; this line's tags
    /// and collections were folded into it.
    RepeatedInFile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ImportAction {
    Insert {
        word: Incoming,
    },
    Update {
        item_id: i64,
        simplified: String,
        /// The word's revision when planned; applying refuses if it moved.
        expected_rev: i64,
        changes: Vec<FieldChange>,
        conflicts: Vec<FieldConflict>,
        tags: NameChange,
        collections: NameChange,
        /// Brings the word out of the trash.
        restore: bool,
    },
    Skip {
        item_id: Option<i64>,
        simplified: String,
        expected_rev: Option<i64>,
        reason: SkipReason,
        conflicts: Vec<FieldConflict>,
    },
    Drop {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedLine {
    pub line: u32,
    pub raw: String,
    pub action: ImportAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportPlan {
    pub connector_id: String,
    pub format: String,
    pub path: String,
    pub content_sha256: String,
    pub policy: ImportPolicy,
    /// The file has error lines. Applying records the refusal and imports
    /// nothing, unless the plan was made with `force`.
    pub refused: bool,
    pub issues: Vec<Issue>,
    pub lines: Vec<PlannedLine>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ImportCounts {
    pub lines: u32,
    pub inserts: u32,
    pub updates: u32,
    pub skips: u32,
    pub drops: u32,
    /// New words that will be marked needs review.
    pub unresolved: u32,
    pub conflicts: u32,
    pub errors: u32,
    pub warnings: u32,
}

impl ImportPlan {
    #[must_use]
    pub fn counts(&self) -> ImportCounts {
        let mut counts = ImportCounts {
            lines: count(self.lines.len()),
            ..ImportCounts::default()
        };
        for line in &self.lines {
            match &line.action {
                ImportAction::Insert { word } => {
                    counts.inserts += 1;
                    if word.verification == Verification::NeedsReview {
                        counts.unresolved += 1;
                    }
                }
                ImportAction::Update { conflicts, .. } => {
                    counts.updates += 1;
                    counts.conflicts += count(conflicts.len());
                }
                ImportAction::Skip { conflicts, .. } => {
                    counts.skips += 1;
                    counts.conflicts += count(conflicts.len());
                }
                ImportAction::Drop { .. } => counts.drops += 1,
            }
        }
        for issue in &self.issues {
            match issue.severity {
                Severity::Error => counts.errors += 1,
                Severity::Warning => counts.warnings += 1,
                Severity::Info => {}
            }
        }
        counts
    }
}

/// Where one line of the file ended up while reading it.
enum LineOutcome {
    /// The first line of a word: an index into the words to plan.
    Word(usize),
    /// Decided already: a line that could not be read as a word, or a repeat
    /// folded into an earlier line.
    Decided(ImportAction),
}

struct InputLine {
    line: u32,
    raw: String,
    outcome: LineOutcome,
}

/// One word from the file, after resolution and folding repeated lines.
struct Word {
    line: u32,
    resolved: Resolved,
    /// What the file says, for comparing with a saved word.
    definition: Option<String>,
    notes: Option<String>,
    tags: Vec<String>,
    collections: Vec<String>,
}

/// Reads `bytes` with `connector` and decides what each line would do.
/// Writes nothing.
///
/// Without a dictionary, words are taken from the file as written and
/// anything incomplete is marked needs review.
///
/// # Errors
///
/// [`vocab_core::ErrorKind::Invalid`] if the connector cannot import;
/// [`vocab_core::ErrorKind::Format`] if the file cannot be read as its
/// format; storage or dictionary errors.
pub fn plan_import(
    conn: &Connection,
    dict: Option<&dyn DictionaryProvider>,
    connector: &dyn Connector,
    bytes: &[u8],
    path: &str,
    policy: ImportPolicy,
    force: bool,
) -> Result<ImportPlan> {
    let info = connector.info();
    if !info.can_import {
        return Err(VocabError::invalid(format!(
            "{} files cannot be imported",
            info.name
        )));
    }
    let parsed = connector.parse(strip_bom(bytes))?;
    let refused = parsed.has_errors() && !force;
    let mut issues = parsed.issues;
    let mut notes = Vec::new();
    if dict.is_none() {
        notes.push(
            "no dictionary loaded: words are taken as written, and incomplete ones need review"
                .to_owned(),
        );
    }

    let mut groups = LibraryNames {
        tags: Names::new(vocab_db::tags(conn)?),
        collections: Names::new(vocab_db::collections(conn)?),
    };
    let (words, input) = read_words(&parsed.records, dict, &mut groups, &mut issues)?;
    let actions = words
        .iter()
        .map(|word| plan_word(conn, word, policy))
        .collect::<Result<Vec<_>>>()?;
    let lines = input
        .into_iter()
        .map(|input| PlannedLine {
            line: input.line,
            raw: input.raw,
            action: match input.outcome {
                LineOutcome::Word(index) => actions[index].clone(),
                LineOutcome::Decided(action) => action,
            },
        })
        .collect();
    issues.sort_by_key(|issue| issue.line);
    Ok(ImportPlan {
        connector_id: info.id.to_owned(),
        format: info.format.to_owned(),
        path: path.to_owned(),
        content_sha256: content_hash(bytes),
        policy,
        refused,
        issues,
        lines,
        notes,
    })
}

/// Resolves every record, folding a repeated word into its first line.
/// Returns the words to plan, and every line in file order.
fn read_words(
    records: &[ExchangeRecord],
    dict: Option<&dyn DictionaryProvider>,
    groups: &mut LibraryNames,
    issues: &mut Vec<Issue>,
) -> Result<(Vec<Word>, Vec<InputLine>)> {
    let mut words: Vec<Word> = Vec::new();
    let mut first: HashMap<(String, String, String), usize> = HashMap::new();
    let mut input = Vec::with_capacity(records.len());
    for record in records {
        let hits = match dict {
            Some(dict) => dict.entries_by_headword(record.headword.trim())?,
            None => Vec::new(),
        };
        let outcome = match resolve(record, &hits) {
            Err(reason) => LineOutcome::Decided(ImportAction::Drop { reason }),
            Ok(resolved) => {
                if let Some(written) = &resolved.as_written {
                    issues.push(Issue::warning(record.line, written.note.clone()));
                }
                let tags = names(record, &record.tags, "tag", issues)
                    .into_iter()
                    .map(|name| groups.tags.canonical(name))
                    .collect();
                let collections = names(record, &record.collections, "collection", issues)
                    .into_iter()
                    .map(|name| groups.collections.canonical(name))
                    .collect();
                let key = (
                    resolved.simplified.to_lowercase(),
                    resolved.traditional.to_lowercase(),
                    reading_key(&resolved.pinyin),
                );
                if let Some(&index) = first.get(&key) {
                    fold(&mut words[index], record, tags, collections, issues);
                    LineOutcome::Decided(ImportAction::Skip {
                        item_id: None,
                        simplified: resolved.simplified,
                        expected_rev: None,
                        reason: SkipReason::RepeatedInFile,
                        conflicts: Vec::new(),
                    })
                } else {
                    first.insert(key, words.len());
                    words.push(Word {
                        line: record.line,
                        definition: record.definition.text().map(str::to_owned),
                        notes: record.notes.text().map(str::to_owned),
                        resolved,
                        tags,
                        collections,
                    });
                    LineOutcome::Word(words.len() - 1)
                }
            }
        };
        input.push(InputLine {
            line: record.line,
            raw: record.raw.clone(),
            outcome,
        });
    }
    Ok((words, input))
}

/// Valid, de-duplicated names from a record; invalid ones become warnings.
fn names(
    record: &ExchangeRecord,
    field: &Field<Vec<String>>,
    noun: &str,
    issues: &mut Vec<Issue>,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for name in field.present().into_iter().flatten() {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        if name.contains(['\t', '\n', '\r']) {
            issues.push(Issue::warning(
                record.line,
                format!("{noun} \"{name}\" skipped: names cannot contain tabs or line breaks"),
            ));
            continue;
        }
        if !contains(&out, name) {
            out.push(name.to_owned());
        }
    }
    out
}

/// Folds a repeated line into the first line with the same word.
fn fold(
    word: &mut Word,
    record: &ExchangeRecord,
    tags: Vec<String>,
    collections: Vec<String>,
    issues: &mut Vec<Issue>,
) {
    for tag in tags {
        if !contains(&word.tags, &tag) {
            word.tags.push(tag);
        }
    }
    for collection in collections {
        if !contains(&word.collections, &collection) {
            word.collections.push(collection);
        }
    }
    let first_line = word.line;
    let mut keep_first = |label: &str, kept: &mut Option<String>, incoming: Option<&str>| match (
        kept.as_deref(),
        incoming,
    ) {
        (None, Some(value)) => *kept = Some(value.to_owned()),
        (Some(existing), Some(value)) if !same_text(existing, value) => {
            issues.push(Issue::warning(
                    record.line,
                    format!("repeats line {first_line} with a different {label}; line {first_line}'s is used"),
                ));
        }
        _ => {}
    };
    keep_first("definition", &mut word.definition, record.definition.text());
    keep_first("notes", &mut word.notes, record.notes.text());
    if word.resolved.definition.is_empty() {
        if let Some(definition) = &word.definition {
            word.resolved.definition.clone_from(definition);
        }
    }
}

/// Case-insensitive, and `my tag` equals `my_tag` (Anki writes the latter).
fn contains(names: &[String], name: &str) -> bool {
    let wanted = name_key(name);
    names.iter().any(|n| name_key(n) == wanted)
}

fn name_key(name: &str) -> String {
    name.to_lowercase()
        .split(|c: char| c.is_whitespace() || c == '_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

struct LibraryNames {
    tags: Names,
    collections: Names,
}

/// The spelling each tag or collection name takes: the library's own when it
/// has the name, else the first spelling in the file. The library compares
/// names ignoring ASCII case only, so without this `Week_1` in a file would
/// go to a new collection beside `Week 1` for new words, while words already
/// in `Week 1` were counted as in it.
struct Names {
    /// By name as the library compares names (`COLLATE NOCASE`).
    exact: HashMap<String, String>,
    /// By [`name_key`].
    loose: HashMap<String, String>,
}

impl Names {
    /// Seeded with the library's names, in its order, so the first wins
    /// when two of them differ only in spacing (from before this rule).
    fn new(library: Vec<vocab_db::NameCount>) -> Self {
        let mut names = Self {
            exact: HashMap::new(),
            loose: HashMap::new(),
        };
        for group in library {
            names.learn(group.name);
        }
        names
    }

    fn learn(&mut self, name: String) {
        self.loose
            .entry(name_key(&name))
            .or_insert_with(|| name.clone());
        self.exact.entry(name.to_ascii_lowercase()).or_insert(name);
    }

    fn canonical(&mut self, name: String) -> String {
        if let Some(known) = self
            .exact
            .get(&name.to_ascii_lowercase())
            .or_else(|| self.loose.get(&name_key(&name)))
        {
            return known.clone();
        }
        self.learn(name.clone());
        name
    }
}

/// Equal apart from spacing and line breaks, which formats may flatten.
fn same_text(a: &str, b: &str) -> bool {
    a.split_whitespace().eq(b.split_whitespace())
}

fn plan_word(conn: &Connection, word: &Word, policy: ImportPolicy) -> Result<ImportAction> {
    let r = &word.resolved;
    let saved = match find_by_identity(conn, &r.simplified, &r.traditional, &r.pinyin)? {
        // A word saved the way the file writes it (an earlier import) is
        // the same word too.
        None => match &r.as_written {
            Some(written) => {
                find_by_identity(conn, &r.simplified, &written.traditional, &written.pinyin)?
            }
            None => None,
        },
        found => found,
    };
    match saved {
        None => Ok(ImportAction::Insert {
            word: Incoming {
                simplified: r.simplified.clone(),
                traditional: r.traditional.clone(),
                pinyin: r.pinyin.clone(),
                definition: r.definition.clone(),
                notes: word.notes.clone().unwrap_or_default(),
                verification: r.verification,
                tags: word.tags.clone(),
                collections: word.collections.clone(),
            },
        }),
        Some(existing) => plan_existing(conn, &existing, word, policy),
    }
}

/// The text fields an import can change on a saved word, with the file's
/// value when the file fills it in.
fn provided<'a>(
    existing: &'a VocabItem,
    word: &'a Word,
) -> [(ImportField, &'a str, Option<&'a str>); 2] {
    [
        (
            ImportField::Definition,
            existing.definition.as_str(),
            word.definition.as_deref(),
        ),
        (
            ImportField::Notes,
            existing.notes.as_str(),
            word.notes.as_deref(),
        ),
    ]
}

fn plan_existing(
    conn: &Connection,
    existing: &VocabItem,
    word: &Word,
    policy: ImportPolicy,
) -> Result<ImportAction> {
    let trashed = existing.deleted_at.is_some();
    let conflicts: Vec<FieldConflict> = provided(existing, word)
        .into_iter()
        .filter_map(|(field, kept, incoming)| {
            let incoming = incoming?;
            (!kept.is_empty() && !same_text(kept, incoming)).then(|| FieldConflict {
                field,
                kept: kept.to_owned(),
                incoming: incoming.to_owned(),
            })
        })
        .collect();
    let skip = |reason, conflicts| ImportAction::Skip {
        item_id: Some(existing.id),
        simplified: existing.simplified.clone(),
        expected_rev: Some(existing.rev),
        reason,
        conflicts,
    };
    if policy == ImportPolicy::Skip && trashed {
        return Ok(skip(SkipReason::InTrash, conflicts));
    }
    let changes: Vec<FieldChange> = provided(existing, word)
        .into_iter()
        .filter_map(|(field, from, to)| {
            let to = to?;
            let wanted = match policy {
                ImportPolicy::Overwrite => !same_text(from, to),
                ImportPolicy::Merge => from.is_empty(),
                ImportPolicy::Skip => false,
            };
            wanted.then(|| FieldChange {
                field,
                from: from.to_owned(),
                to: to.to_owned(),
            })
        })
        .collect();
    let conflicts = if policy == ImportPolicy::Overwrite {
        Vec::new()
    } else {
        conflicts
    };
    let tags_now = item_tags(conn, existing.id)?;
    let collections_now = item_collections(conn, existing.id)?;
    let tags = NameChange {
        add: word
            .tags
            .iter()
            .filter(|name| !contains(&tags_now, name))
            .cloned()
            .collect(),
    };
    let collections = NameChange {
        add: word
            .collections
            .iter()
            .filter(|name| !contains(&collections_now, name))
            .cloned()
            .collect(),
    };
    if changes.is_empty() && tags.add.is_empty() && collections.add.is_empty() && !trashed {
        let reason = if policy == ImportPolicy::Skip {
            SkipReason::AlreadySaved
        } else {
            SkipReason::Unchanged
        };
        return Ok(skip(reason, conflicts));
    }
    Ok(ImportAction::Update {
        item_id: existing.id,
        simplified: existing.simplified.clone(),
        expected_rev: existing.rev,
        changes,
        conflicts,
        tags,
        collections,
        restore: trashed,
    })
}

/// Carries out a plan in one transaction.
///
/// `current_hash` is the file's [`content_hash`] now; if it differs from the
/// plan's, nothing happens. A refused plan records the refusal only.
///
/// # Errors
///
/// [`vocab_core::ErrorKind::Conflict`] if the file or any affected word
/// changed since the plan was made (preview again); storage errors. On any
/// error nothing is written.
pub fn apply_import(
    conn: &mut Connection,
    plan: &ImportPlan,
    current_hash: &str,
) -> Result<TransferSummary> {
    if current_hash != plan.content_sha256 {
        return Err(VocabError::conflict(format!(
            "{} changed since the preview; preview it again",
            plan.path
        )));
    }
    let counts = plan.counts();
    let as_usize = |n: u32| usize::try_from(n).unwrap_or(usize::MAX);
    let mut run = RunRecord {
        direction: Direction::In,
        connector_id: plan.connector_id.clone(),
        format: plan.format.clone(),
        path: plan.path.clone(),
        content_sha256: Some(plan.content_sha256.clone()),
        counts: RunCounts {
            seen: plan.lines.len(),
            errors: as_usize(counts.errors),
            warnings: as_usize(counts.warnings),
            ..RunCounts::default()
        },
        status: RunStatus::Rejected,
    };
    let mut summary = TransferSummary {
        connector_id: plan.connector_id.clone(),
        path: plan.path.clone(),
        notes: plan.notes.clone(),
        ..TransferSummary::default()
    };
    let record_errors = |tx: &Connection, run_id: i64| -> Result<()> {
        for issue in plan
            .issues
            .iter()
            .filter(|issue| issue.severity == Severity::Error)
        {
            record_item(
                tx,
                run_id,
                None,
                Some(issue.line),
                None,
                Outcome::Rejected,
                &issue.message,
            )?;
        }
        Ok(())
    };
    if plan.refused {
        with_tx(conn, |tx| record_errors(tx, record_run(tx, &run)?))?;
        summary.refused = true;
        summary.notes.push(format!(
            "refused: {} lines have errors; fix them or import with --force",
            counts.errors
        ));
        return Ok(summary);
    }
    run.status = RunStatus::Committed;
    run.counts.inserted = as_usize(counts.inserts);
    run.counts.updated = as_usize(counts.updates);
    run.counts.skipped = as_usize(counts.skips);
    run.counts.dropped = as_usize(counts.drops);
    run.counts.unresolved = as_usize(counts.unresolved);
    let source = ItemSource {
        kind: SourceKind::Import,
        id: Some(plan.connector_id.clone()),
        version: Some(plan.format.clone()),
        import_origin: Some(plan.path.clone()),
    };
    with_tx(conn, |tx| {
        check_fresh(tx, plan)?;
        let run_id = record_run(tx, &run)?;
        // Forced past error lines: they are skipped, and recorded.
        record_errors(tx, run_id)?;
        for line in &plan.lines {
            apply_line(tx, run_id, line, &source)?;
        }
        Ok(())
    })?;
    summary.inserted = counts.inserts;
    summary.updated = counts.updates;
    summary.skipped = counts.skips;
    summary.dropped = counts.drops;
    summary.unresolved = counts.unresolved;
    match counts.conflicts {
        0 => {}
        1 => summary
            .notes
            .push("1 field differed from a saved word; the saved value was kept".to_owned()),
        n => summary.notes.push(format!(
            "{n} fields differed from saved words; the saved values were kept"
        )),
    }
    Ok(summary)
}

fn stale(word: &str) -> VocabError {
    VocabError::conflict(format!(
        "{word} changed since the preview; preview the import again"
    ))
}

fn check_fresh(conn: &Connection, plan: &ImportPlan) -> Result<()> {
    for line in &plan.lines {
        match &line.action {
            ImportAction::Insert { word } => {
                if find_by_identity(conn, &word.simplified, &word.traditional, &word.pinyin)?
                    .is_some()
                {
                    return Err(stale(&word.simplified));
                }
            }
            ImportAction::Update {
                item_id,
                simplified,
                expected_rev,
                ..
            }
            | ImportAction::Skip {
                item_id: Some(item_id),
                simplified,
                expected_rev: Some(expected_rev),
                ..
            } => {
                let current = require_item(conn, *item_id).map_err(|_| stale(simplified))?;
                if current.rev != *expected_rev {
                    return Err(stale(simplified));
                }
            }
            ImportAction::Skip { .. } | ImportAction::Drop { .. } => {}
        }
    }
    Ok(())
}

/// Applies one planned line and records it in the run's ledger.
fn apply_line(
    conn: &Connection,
    run_id: i64,
    line: &PlannedLine,
    source: &ItemSource,
) -> Result<()> {
    let at = Some(line.line);
    let raw = Some(line.raw.as_str());
    let (item_id, outcome, detail) = match &line.action {
        ImportAction::Insert { word } => (
            Some(apply_insert(conn, word, source)?),
            Outcome::Inserted,
            String::new(),
        ),
        ImportAction::Update {
            item_id,
            changes,
            tags,
            collections,
            restore,
            ..
        } => {
            apply_update(conn, *item_id, changes, tags, collections, *restore)?;
            (
                Some(*item_id),
                Outcome::Updated,
                update_detail(changes, tags, collections, *restore),
            )
        }
        ImportAction::Skip {
            item_id,
            reason,
            conflicts,
            ..
        } => (*item_id, Outcome::Skipped, skip_detail(*reason, conflicts)),
        ImportAction::Drop { reason } => (None, Outcome::Dropped, reason.clone()),
    };
    record_item(conn, run_id, item_id, at, raw, outcome, &detail)
}

fn apply_insert(conn: &Connection, word: &Incoming, source: &ItemSource) -> Result<i64> {
    let id = insert_item(
        conn,
        &NewItem {
            simplified: word.simplified.clone(),
            traditional: word.traditional.clone(),
            pinyin: word.pinyin.clone(),
            definition: word.definition.clone(),
            notes: word.notes.clone(),
            verification: word.verification,
            source: source.clone(),
        },
    )?;
    for tag in &word.tags {
        add_tag(conn, id, tag)?;
    }
    for collection in &word.collections {
        add_to_collection(conn, id, collection)?;
    }
    Ok(id)
}

fn apply_update(
    conn: &Connection,
    item_id: i64,
    changes: &[FieldChange],
    tags: &NameChange,
    collections: &NameChange,
    restore: bool,
) -> Result<()> {
    let mut patch = ItemPatch::default();
    for change in changes {
        let to = Some(change.to.clone());
        match change.field {
            ImportField::Definition => patch.definition = to,
            ImportField::Notes => patch.notes = to,
        }
    }
    update_item(conn, item_id, &patch)?;
    if restore {
        set_trashed(conn, item_id, false)?;
    }
    for tag in &tags.add {
        add_tag(conn, item_id, tag)?;
    }
    for collection in &collections.add {
        add_to_collection(conn, item_id, collection)?;
    }
    Ok(())
}

fn skip_detail(reason: SkipReason, conflicts: &[FieldConflict]) -> String {
    let mut detail = vec![
        match reason {
            SkipReason::AlreadySaved => "already saved",
            SkipReason::InTrash => "in the trash",
            SkipReason::Unchanged => "unchanged",
            SkipReason::RepeatedInFile => "repeats an earlier line",
        }
        .to_owned(),
    ];
    detail.extend(conflicts.iter().map(|c| format!("{} kept", c.field.name())));
    detail.join("; ")
}

fn update_detail(
    changes: &[FieldChange],
    tags: &NameChange,
    collections: &NameChange,
    restore: bool,
) -> String {
    let mut parts: Vec<String> = changes.iter().map(|c| c.field.name().to_owned()).collect();
    parts.extend(tags.add.iter().map(|t| format!("+tag {t}")));
    parts.extend(collections.add.iter().map(|c| format!("+collection {c}")));
    if restore {
        parts.push("restored from trash".to_owned());
    }
    parts.join(", ")
}
