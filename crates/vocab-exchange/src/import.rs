//! Import: file → plan → library.

use std::collections::HashMap;
use std::str::FromStr;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use vocab_core::connector::{Connector, ExchangeRecord, Field, Issue, Severity};
use vocab_core::{ItemPatch, ItemSource, Result, SourceKind, Verification, VocabError, VocabItem};
use vocab_db::{
    Direction, NewItem, Outcome, RunCounts, RunRecord, RunStatus, add_tag, add_to_collection,
    find_by_identity, insert_item, item_collections, item_tags, record_item, record_run,
    remove_from_collection, remove_tag, require_item, set_trashed, update_item, with_tx,
};
use vocab_dictionary::DictionaryProvider;

use crate::resolve::{Resolved, resolve};
use crate::{TransferSummary, content_hash, reading_key};

/// What to do when a file has a word that is already saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportPolicy {
    /// Leave the saved word alone.
    #[default]
    Skip,
    /// Replace every field the file has (an omitted field is not "had").
    Overwrite,
    /// Fill blank fields and add tags and collections. Where both sides have
    /// different values, keep the saved one and report the conflict.
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
    pub traditional: String,
    pub pinyin: String,
    pub definition: String,
    pub notes: String,
    pub verification: Verification,
    pub tags: Vec<String>,
    pub collections: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldChange {
    pub field: String,
    pub from: String,
    pub to: String,
}

/// Both sides have a value and they differ; the saved one is kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldConflict {
    pub field: String,
    pub kept: String,
    pub incoming: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct NameChange {
    pub add: Vec<String>,
    pub remove: Vec<String>,
}

impl NameChange {
    fn is_empty(&self) -> bool {
        self.add.is_empty() && self.remove.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    AlreadySaved,
    /// Saved, but in the trash. Skip leaves it there.
    InTrash,
    /// The policy would change nothing.
    Unchanged,
    /// An earlier line of the same file has the same word.
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
        expected_modified_at: String,
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
        expected_modified_at: Option<String>,
        reason: SkipReason,
        conflicts: Vec<FieldConflict>,
    },
    Drop {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedLine {
    pub line: usize,
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
    pub lines: usize,
    pub inserts: usize,
    pub updates: usize,
    pub skips: usize,
    pub drops: usize,
    /// New words that will be marked needs review.
    pub unresolved: usize,
    pub conflicts: usize,
    pub errors: usize,
    pub warnings: usize,
}

impl ImportPlan {
    #[must_use]
    pub fn counts(&self) -> ImportCounts {
        let mut counts = ImportCounts {
            lines: self.lines.len(),
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
                    counts.conflicts += conflicts.len();
                }
                ImportAction::Skip { conflicts, .. } => {
                    counts.skips += 1;
                    counts.conflicts += conflicts.len();
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
    let parsed = connector.parse(bytes)?;
    let mut notes = Vec::new();
    if dict.is_none() {
        notes.push(
            "no dictionary loaded: words are taken as written, and incomplete ones need review"
                .to_owned(),
        );
    }
    let mut seen: HashMap<(String, String, String), usize> = HashMap::new();
    let mut lines = Vec::with_capacity(parsed.records.len());
    for record in &parsed.records {
        let action = plan_record(conn, dict, record, policy, &mut seen)?;
        lines.push(PlannedLine {
            line: record.line,
            raw: record.raw.clone(),
            action,
        });
    }
    Ok(ImportPlan {
        connector_id: info.id.to_owned(),
        format: info.format.to_owned(),
        path: path.to_owned(),
        content_sha256: content_hash(bytes),
        policy,
        refused: parsed.has_errors() && !force,
        issues: parsed.issues,
        lines,
        notes,
    })
}

fn plan_record(
    conn: &Connection,
    dict: Option<&dyn DictionaryProvider>,
    record: &ExchangeRecord,
    policy: ImportPolicy,
    seen: &mut HashMap<(String, String, String), usize>,
) -> Result<ImportAction> {
    let hits = match dict {
        Some(dict) => dict.entries_by_headword(record.headword.trim())?,
        None => Vec::new(),
    };
    let resolved = match resolve(record, &hits) {
        Ok(resolved) => resolved,
        Err(reason) => return Ok(ImportAction::Drop { reason }),
    };
    let identity = (
        resolved.simplified.to_lowercase(),
        resolved.traditional.to_lowercase(),
        reading_key(&resolved.pinyin),
    );
    if seen.contains_key(&identity) {
        return Ok(ImportAction::Skip {
            item_id: None,
            simplified: resolved.simplified,
            expected_modified_at: None,
            reason: SkipReason::RepeatedInFile,
            conflicts: Vec::new(),
        });
    }
    seen.insert(identity, record.line);
    match find_by_identity(
        conn,
        &resolved.simplified,
        &resolved.traditional,
        &resolved.pinyin,
    )? {
        None => Ok(ImportAction::Insert {
            word: incoming(resolved, record),
        }),
        Some(existing) => plan_existing(conn, &existing, record, policy),
    }
}

fn incoming(resolved: Resolved, record: &ExchangeRecord) -> Incoming {
    Incoming {
        simplified: resolved.simplified,
        traditional: resolved.traditional,
        pinyin: resolved.pinyin,
        definition: resolved.definition,
        notes: resolved.notes,
        verification: resolved.verification,
        tags: names(&record.tags),
        collections: names(&record.collections),
    }
}

fn names(field: &Field<Vec<String>>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for name in field.present().into_iter().flatten() {
        let name = name.trim();
        if !name.is_empty() && !contains(&out, name) {
            out.push(name.to_owned());
        }
    }
    out
}

fn contains(names: &[String], name: &str) -> bool {
    let name = name.to_lowercase();
    names.iter().any(|n| n.to_lowercase() == name)
}

/// The text fields an import can change on a saved word, with the file's
/// value when the file has one.
fn provided<'a>(
    existing: &'a VocabItem,
    record: &'a ExchangeRecord,
) -> [(&'static str, &'a str, Option<&'a str>); 2] {
    let value = |field: &'a Field<String>| field.present().map(|v| v.trim());
    [
        (
            "definition",
            existing.definition.as_str(),
            value(&record.definition),
        ),
        ("notes", existing.notes.as_str(), value(&record.notes)),
    ]
}

fn conflicts(existing: &VocabItem, record: &ExchangeRecord) -> Vec<FieldConflict> {
    provided(existing, record)
        .into_iter()
        .filter_map(|(field, kept, incoming)| {
            let incoming = incoming?;
            (!kept.is_empty() && !incoming.is_empty() && kept != incoming).then(|| FieldConflict {
                field: field.to_owned(),
                kept: kept.to_owned(),
                incoming: incoming.to_owned(),
            })
        })
        .collect()
}

fn plan_existing(
    conn: &Connection,
    existing: &VocabItem,
    record: &ExchangeRecord,
    policy: ImportPolicy,
) -> Result<ImportAction> {
    let trashed = existing.deleted_at.is_some();
    let skip = |reason, conflicts| ImportAction::Skip {
        item_id: Some(existing.id),
        simplified: existing.simplified.clone(),
        expected_modified_at: Some(existing.modified_at.clone()),
        reason,
        conflicts,
    };
    let found_conflicts = conflicts(existing, record);
    if policy == ImportPolicy::Skip {
        let reason = if trashed {
            SkipReason::InTrash
        } else {
            SkipReason::AlreadySaved
        };
        return Ok(skip(reason, found_conflicts));
    }
    let tags_now = item_tags(conn, existing.id)?;
    let collections_now = item_collections(conn, existing.id)?;
    let (changes, conflicts, tags, collections) = match policy {
        ImportPolicy::Overwrite => {
            let changes: Vec<FieldChange> = provided(existing, record)
                .into_iter()
                .filter_map(|(field, from, to)| {
                    let to = to?;
                    (from != to).then(|| FieldChange {
                        field: field.to_owned(),
                        from: from.to_owned(),
                        to: to.to_owned(),
                    })
                })
                .collect();
            (
                changes,
                Vec::new(),
                replace_names(&tags_now, &record.tags),
                replace_names(&collections_now, &record.collections),
            )
        }
        ImportPolicy::Merge | ImportPolicy::Skip => {
            let changes: Vec<FieldChange> = provided(existing, record)
                .into_iter()
                .filter_map(|(field, from, to)| {
                    let to = to.filter(|to| !to.is_empty())?;
                    from.is_empty().then(|| FieldChange {
                        field: field.to_owned(),
                        from: String::new(),
                        to: to.to_owned(),
                    })
                })
                .collect();
            (
                changes,
                found_conflicts,
                add_names(&tags_now, &record.tags),
                add_names(&collections_now, &record.collections),
            )
        }
    };
    if changes.is_empty() && tags.is_empty() && collections.is_empty() && !trashed {
        return Ok(skip(SkipReason::Unchanged, conflicts));
    }
    Ok(ImportAction::Update {
        item_id: existing.id,
        simplified: existing.simplified.clone(),
        expected_modified_at: existing.modified_at.clone(),
        changes,
        conflicts,
        tags,
        collections,
        restore: trashed,
    })
}

fn add_names(now: &[String], incoming: &Field<Vec<String>>) -> NameChange {
    NameChange {
        add: names(incoming)
            .into_iter()
            .filter(|name| !contains(now, name))
            .collect(),
        remove: Vec::new(),
    }
}

fn replace_names(now: &[String], incoming: &Field<Vec<String>>) -> NameChange {
    if !incoming.is_present() {
        return NameChange::default();
    }
    let wanted = names(incoming);
    NameChange {
        add: wanted
            .iter()
            .filter(|name| !contains(now, name))
            .cloned()
            .collect(),
        remove: now
            .iter()
            .filter(|name| !contains(&wanted, name))
            .cloned()
            .collect(),
    }
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
    let mut run = RunRecord {
        direction: Direction::In,
        connector_id: plan.connector_id.clone(),
        format: plan.format.clone(),
        path: plan.path.clone(),
        content_sha256: Some(plan.content_sha256.clone()),
        counts: RunCounts {
            seen: counts.lines,
            errors: counts.errors,
            warnings: counts.warnings,
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
    if plan.refused {
        with_tx(conn, |tx| {
            let run_id = record_run(tx, &run)?;
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
        })?;
        summary.refused = true;
        summary.notes.push(format!(
            "refused: {} lines have errors; fix them or import with --force",
            counts.errors
        ));
        return Ok(summary);
    }
    run.status = RunStatus::Committed;
    run.counts.inserted = counts.inserts;
    run.counts.updated = counts.updates;
    run.counts.skipped = counts.skips;
    run.counts.dropped = counts.drops;
    run.counts.unresolved = counts.unresolved;
    let source = ItemSource {
        kind: SourceKind::Import,
        id: Some(plan.connector_id.clone()),
        version: Some(plan.format.clone()),
        import_origin: Some(plan.path.clone()),
    };
    with_tx(conn, |tx| {
        check_fresh(tx, plan)?;
        let run_id = record_run(tx, &run)?;
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
    if counts.conflicts > 0 {
        summary.notes.push(format!(
            "{} fields differed from saved words; the saved values were kept",
            counts.conflicts
        ));
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
                expected_modified_at,
                ..
            }
            | ImportAction::Skip {
                item_id: Some(item_id),
                simplified,
                expected_modified_at: Some(expected_modified_at),
                ..
            } => {
                let current = require_item(conn, *item_id).map_err(|_| stale(simplified))?;
                if &current.modified_at != expected_modified_at {
                    return Err(stale(simplified));
                }
            }
            ImportAction::Skip { .. } | ImportAction::Drop { .. } => {}
        }
    }
    Ok(())
}

fn apply_line(
    conn: &Connection,
    run_id: i64,
    line: &PlannedLine,
    source: &ItemSource,
) -> Result<()> {
    let at = Some(line.line);
    let raw = Some(line.raw.as_str());
    match &line.action {
        ImportAction::Insert { word } => {
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
            record_item(conn, run_id, Some(id), at, raw, Outcome::Inserted, "")
        }
        ImportAction::Update {
            item_id,
            changes,
            tags,
            collections,
            restore,
            ..
        } => {
            let mut patch = ItemPatch::default();
            for change in changes {
                match change.field.as_str() {
                    "definition" => patch.definition = Some(change.to.clone()),
                    "notes" => patch.notes = Some(change.to.clone()),
                    _ => {}
                }
            }
            update_item(conn, *item_id, &patch)?;
            if *restore {
                set_trashed(conn, *item_id, false)?;
            }
            for tag in &tags.add {
                add_tag(conn, *item_id, tag)?;
            }
            for tag in &tags.remove {
                remove_tag(conn, *item_id, tag)?;
            }
            for collection in &collections.add {
                add_to_collection(conn, *item_id, collection)?;
            }
            for collection in &collections.remove {
                remove_from_collection(conn, *item_id, collection)?;
            }
            let detail = update_detail(changes, tags, collections, *restore);
            record_item(
                conn,
                run_id,
                Some(*item_id),
                at,
                raw,
                Outcome::Updated,
                &detail,
            )
        }
        ImportAction::Skip {
            item_id,
            reason,
            conflicts,
            ..
        } => {
            let mut detail = vec![
                match reason {
                    SkipReason::AlreadySaved => "already saved",
                    SkipReason::InTrash => "in the trash",
                    SkipReason::Unchanged => "unchanged",
                    SkipReason::RepeatedInFile => "repeats an earlier line",
                }
                .to_owned(),
            ];
            detail.extend(conflicts.iter().map(|c| format!("{} kept", c.field)));
            let detail = detail.join("; ");
            record_item(conn, run_id, *item_id, at, raw, Outcome::Skipped, &detail)
        }
        ImportAction::Drop { reason } => {
            record_item(conn, run_id, None, at, raw, Outcome::Dropped, reason)
        }
    }
}

fn update_detail(
    changes: &[FieldChange],
    tags: &NameChange,
    collections: &NameChange,
    restore: bool,
) -> String {
    let mut parts: Vec<String> = changes.iter().map(|c| c.field.clone()).collect();
    parts.extend(tags.add.iter().map(|t| format!("+tag {t}")));
    parts.extend(tags.remove.iter().map(|t| format!("-tag {t}")));
    parts.extend(collections.add.iter().map(|c| format!("+collection {c}")));
    parts.extend(
        collections
            .remove
            .iter()
            .map(|c| format!("-collection {c}")),
    );
    if restore {
        parts.push("restored from trash".to_owned());
    }
    parts.join(", ")
}
