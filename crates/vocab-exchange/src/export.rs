//! Export: library → plan → file.

use std::path::Path;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use vocab_core::connector::{Connector, ExportRecord, WriteOptions};
use vocab_core::{LibraryFilter, LibraryView, Result, Verification, VocabError, VocabItem};
use vocab_db::{
    Direction, Outcome, RunCounts, RunRecord, RunStatus, collections_by_item, import_sources,
    items_at, list_items, record_item, record_run, tags_by_item, with_tx,
};
use vocab_dictionary::DictionaryProvider;

use crate::resolve::definition_matches;
use crate::{TransferSummary, atomic_write, reading_key};

/// Which words to export, on top of the filter.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportScope {
    /// Only words this destination doesn't have yet: never exported there,
    /// never imported from it.
    #[default]
    New,
    /// Every matching word.
    All,
    /// These words (still subject to the filter).
    Selected(Vec<i64>),
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ExportRequest {
    #[serde(default)]
    pub scope: ExportScope,
    /// Defaults to active words. The trash is never exported.
    #[serde(default)]
    pub filter: LibraryFilter,
    /// Words marked needs review are left out unless this is set.
    #[serde(default)]
    pub include_needs_review: bool,
    /// Anki deck name.
    #[serde(default)]
    pub deck: Option<String>,
}

/// A rendered export, ready to write. It can be shown (serialized) but not
/// read back: the file's bytes stay in memory, so an apply always writes
/// exactly what was previewed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportPlan {
    pub connector_id: String,
    pub format: String,
    pub path: String,
    pub item_ids: Vec<i64>,
    /// Headwords of the exported words, oldest first, for a preview.
    pub words: Vec<String>,
    /// Matching words left out because they need review.
    pub left_out_needs_review: u32,
    /// Matching words left out because the destination has them (scope `new`).
    pub left_out_already_there: u32,
    /// A file already exists at `path` and will be replaced. Confirm first.
    pub replaces_existing: bool,
    /// What the format could not carry.
    pub notes: Vec<String>,
    #[serde(skip)]
    pub bytes: Vec<u8>,
}

/// Picks the words and renders the file in memory. Writes nothing.
///
/// # Errors
///
/// [`vocab_core::ErrorKind::Invalid`] if the connector cannot export, the
/// filter asks for the trash, or `path` was imported from (exports never
/// overwrite an import source); storage, dictionary, or format errors.
pub fn plan_export(
    conn: &Connection,
    dict: Option<&dyn DictionaryProvider>,
    connector: &dyn Connector,
    path: &str,
    request: &ExportRequest,
) -> Result<ExportPlan> {
    let info = connector.info();
    if !info.can_export {
        return Err(VocabError::invalid(format!(
            "{} files cannot be exported",
            info.name
        )));
    }
    if request.filter.view == LibraryView::Trash {
        return Err(VocabError::invalid("the trash is never exported"));
    }
    if import_sources(conn)?
        .iter()
        .any(|source| Path::new(source) == Path::new(path))
    {
        return Err(VocabError::invalid(format!(
            "{path} was imported from; export to a different file so it is never overwritten"
        )));
    }
    let mut items = list_items(conn, &request.filter)?;
    items.reverse(); // oldest first: the order words were collected
    if let ExportScope::Selected(ids) = &request.scope {
        items.retain(|item| ids.contains(&item.id));
    }
    let before = items.len();
    if !request.include_needs_review {
        items.retain(|item| item.verification != Verification::NeedsReview);
    }
    let left_out_needs_review = crate::count(before - items.len());
    let before = items.len();
    if request.scope == ExportScope::New {
        let already = items_at(conn, info.id)?;
        items.retain(|item| !already.contains(&item.id));
    }
    let left_out_already_there = crate::count(before - items.len());

    let mut tags = tags_by_item(conn)?;
    let mut collections = collections_by_item(conn)?;
    let mut records = Vec::with_capacity(items.len());
    for item in &items {
        let mut record = ExportRecord::new(
            &item.simplified,
            &item.traditional,
            &item.pinyin,
            &item.definition,
        );
        record.notes.clone_from(&item.notes);
        record.definition_is_dictionary_default = is_dictionary_default(dict, item)?;
        record.tags = tags.remove(&item.id).unwrap_or_default();
        record.collections = collections.remove(&item.id).unwrap_or_default();
        records.push(record);
    }
    let mut options = WriteOptions::default();
    options.collection.clone_from(&request.filter.collection);
    options.deck.clone_from(&request.deck);
    let written = connector.write(&records, &options)?;
    Ok(ExportPlan {
        connector_id: info.id.to_owned(),
        format: info.format.to_owned(),
        path: path.to_owned(),
        item_ids: items.iter().map(|item| item.id).collect(),
        words: items.iter().map(|item| item.simplified.clone()).collect(),
        left_out_needs_review,
        left_out_already_there,
        replaces_existing: Path::new(path).exists(),
        notes: written.notes,
        bytes: written.bytes,
    })
}

/// The word's definition is exactly what a dictionary says for its reading.
fn is_dictionary_default(dict: Option<&dyn DictionaryProvider>, item: &VocabItem) -> Result<bool> {
    let Some(dict) = dict else {
        return Ok(false);
    };
    let key = reading_key(&item.pinyin);
    Ok(dict
        .entries_by_headword(&item.simplified)?
        .iter()
        .any(|entry| {
            reading_key(&entry.pinyin) == key
                && definition_matches(&item.definition, &entry.glosses)
        }))
}

/// Writes the planned file and records the run. A plan with no words writes
/// nothing.
///
/// # Errors
///
/// I/O errors writing the file (then nothing is recorded), or storage errors.
pub fn apply_export(conn: &mut Connection, plan: &ExportPlan) -> Result<TransferSummary> {
    let mut summary = TransferSummary {
        connector_id: plan.connector_id.clone(),
        path: plan.path.clone(),
        notes: plan.notes.clone(),
        ..TransferSummary::default()
    };
    if plan.item_ids.is_empty() {
        summary.notes.push("nothing to export".to_owned());
        return Ok(summary);
    }
    if plan.bytes.is_empty() {
        return Err(VocabError::invalid(
            "this export plan has no file contents; preview the export again",
        ));
    }
    atomic_write(Path::new(&plan.path), &plan.bytes)?;
    let run = RunRecord {
        direction: Direction::Out,
        connector_id: plan.connector_id.clone(),
        format: plan.format.clone(),
        path: plan.path.clone(),
        content_sha256: Some(crate::content_hash(&plan.bytes)),
        counts: RunCounts {
            seen: plan.item_ids.len(),
            ..RunCounts::default()
        },
        status: RunStatus::Committed,
    };
    with_tx(conn, |tx| {
        let run_id = record_run(tx, &run)?;
        for &id in &plan.item_ids {
            record_item(tx, run_id, Some(id), None, None, Outcome::Written, "")?;
        }
        Ok(())
    })?;
    summary.written = crate::count(plan.item_ids.len());
    Ok(summary)
}
