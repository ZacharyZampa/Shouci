//! Pleco import/export transaction boundaries.
//!
//! These functions are the only writers of `import_runs` / `export_runs` and the
//! only path that changes item statuses on an export commit. Everything runs
//! inside `with_tx`, so a failed run leaves `user.db` untouched.

use rusqlite::OptionalExtension;
use vocab_core::{
    ConfirmationState, ItemStatus, Provenance, Result, SourceId, SourceVersion, VocabError,
    VocabItem,
};

use crate::repo::{find_by_forms, select_items};
use crate::{set_status, with_tx};

/// One staged import record (already validated by the codec layer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportRecord {
    pub simplified: String,
    pub pinyin: String,
    pub definition: String,
    pub category: Option<String>,
}

/// Everything the persistence layer needs to stage and commit one import run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportPayload {
    /// Path of the source file, recorded verbatim for provenance and the
    /// export-overwrite guard.
    pub source_path: String,
    /// Codec key, e.g. `pleco-utf8-text/v1`.
    pub codec_key: String,
    /// SHA-256 (hex) of the raw source bytes, pinned in `import_runs`.
    pub content_sha256: String,
    pub records: Vec<ImportRecord>,
    /// Number of error-severity parse issues in the source.
    pub issues_errors: usize,
    /// Number of warning-severity parse issues in the source.
    pub issues_warnings: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportSummary {
    pub records_seen: usize,
    pub records_imported: usize,
    pub records_skipped_duplicate: usize,
    pub issues_errors: usize,
    pub issues_warnings: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportDecision {
    /// The whole run was persisted in one transaction.
    Imported(ImportSummary),
    /// The run was refused because the source had error-severity issues and
    /// `--force` was not given. Nothing was written.
    Refused,
}

/// Stages and commits one import run, or refuses it.
///
/// When the source contains error-severity issues and `force` is false the run
/// is refused without touching the database. Duplicates (matching the schema's
/// case-insensitive `(simplified, traditional)` key) are skipped and counted,
/// never overwritten. Records missing a definition or pinyin import as
/// `needs_review` so the gap stays visible.
///
/// # Errors
///
/// Returns an error if the transaction cannot be begun, committed, or any
/// statement fails; on error nothing is committed.
pub fn persist_import(
    conn: &mut rusqlite::Connection,
    payload: &ImportPayload,
    force: bool,
) -> Result<ImportDecision> {
    if payload.issues_errors > 0 && !force {
        return Ok(ImportDecision::Refused);
    }

    with_tx(conn, |tx| {
        let mut imported = 0usize;
        let mut skipped = 0usize;
        for record in &payload.records {
            let simplified = record.simplified.trim();
            if simplified.is_empty() {
                continue;
            }
            if find_by_forms(tx, simplified, simplified, None)?.is_some() {
                skipped += 1;
                continue;
            }
            let status = if record.pinyin.trim().is_empty() || record.definition.trim().is_empty() {
                ItemStatus::NeedsReview
            } else {
                ItemStatus::Confirmed
            };
            let item_id = insert_item(tx, record, status)?;
            if let Some(category) = record
                .category
                .as_deref()
                .map(str::trim)
                .filter(|c| !c.is_empty())
            {
                let tag_id = ensure_tag(tx, category)?;
                link_tag(tx, item_id, tag_id)?;
            }
            imported += 1;
        }
        record_import_run(tx, payload, imported, skipped)?;
        record_audit(
            tx,
            "import",
            &format!(
                "{} | seen {} imported {} skipped {} errors {} warnings {}",
                payload.codec_key,
                payload.records.len(),
                imported,
                skipped,
                payload.issues_errors,
                payload.issues_warnings
            ),
        )?;
        Ok(ImportSummary {
            records_seen: payload.records.len(),
            records_imported: imported,
            records_skipped_duplicate: skipped,
            issues_errors: payload.issues_errors,
            issues_warnings: payload.issues_warnings,
        })
    })
    .map(ImportDecision::Imported)
}

/// Selects items eligible for export: never `needs_review` unless explicitly
/// allowed, optionally restricted to items carrying *all* the named tags.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn select_exportable(
    conn: &rusqlite::Connection,
    required_tags: &[String],
    include_unresolved: bool,
) -> Result<Vec<VocabItem>> {
    let mut conditions: Vec<String> = Vec::new();
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    if !include_unresolved {
        conditions.push("status <> 'needs_review'".to_owned());
    }
    for tag in required_tags {
        conditions.push(
            "EXISTS (SELECT 1 FROM vocabulary_tags vt JOIN tags t ON vt.tag_id = t.tag_id \
             WHERE vt.item_id = vocabulary_items.item_id \
             AND t.name = ? COLLATE NOCASE)"
                .to_owned(),
        );
        params.push(tag.clone().into());
    }
    if let Some(first) = conditions.first_mut() {
        first.insert_str(0, "WHERE ");
    }
    let tail = format!("{} ORDER BY item_id", conditions.join(" AND "));
    select_items(conn, &tail, rusqlite::params_from_iter(params.iter()))
}

/// Metadata recorded for a completed export run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRun {
    pub target_path: String,
    pub codec_key: String,
    pub records_written: usize,
}

/// Marks the exported items as `exported` and records the run, atomically.
///
/// # Errors
///
/// Returns an error if the transaction fails; on error nothing is committed.
pub fn commit_export(
    conn: &mut rusqlite::Connection,
    run: &ExportRun,
    exported_item_ids: &[i64],
) -> Result<()> {
    with_tx(conn, |tx| {
        for &item_id in exported_item_ids {
            set_status(tx, item_id, ItemStatus::Exported)?;
        }
        tx.execute(
            "INSERT INTO export_runs \
             (target_path, codec_key, records_written, status) \
             VALUES (?1, ?2, ?3, 'committed')",
            rusqlite::params![
                run.target_path,
                run.codec_key,
                count_to_i64(run.records_written)?,
            ],
        )
        .map_err(|err| VocabError::new(format!("record export run: {err}")))?;
        record_audit(
            tx,
            "export",
            &format!(
                "{} | written {} -> {}",
                run.codec_key, run.records_written, run.target_path
            ),
        )?;
        Ok(())
    })
}

/// Every path ever imported, used to refuse overwriting an import source.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn import_sources(conn: &rusqlite::Connection) -> Result<Vec<String>> {
    let mut stmt = conn
        .prepare("SELECT DISTINCT source_path FROM import_runs ORDER BY source_path")
        .map_err(|err| VocabError::new(format!("prepare import source query: {err}")))?;
    let mut rows = stmt
        .query([])
        .map_err(|err| VocabError::new(format!("run import source query: {err}")))?;
    let mut sources = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| VocabError::new(format!("read import source row: {err}")))?
    {
        sources.push(row.get(0)?);
    }
    Ok(sources)
}

/// Attaches a named tag to an item, creating the tag row on first use.
///
/// # Errors
///
/// Returns an error if the database write fails.
pub fn add_tag(conn: &rusqlite::Connection, item_id: i64, name: &str) -> Result<()> {
    let tag_id = ensure_tag(conn, name)?;
    link_tag(conn, item_id, tag_id)
}

/// Tags attached to one item, in stable name order.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn item_tags(conn: &rusqlite::Connection, item_id: i64) -> Result<Vec<String>> {
    let mut stmt = conn
        .prepare(
            "SELECT t.name FROM vocabulary_tags vt JOIN tags t ON vt.tag_id = t.tag_id \
             WHERE vt.item_id = ?1 ORDER BY t.name",
        )
        .map_err(|err| VocabError::new(format!("prepare item tags query: {err}")))?;
    let mut rows = stmt
        .query(rusqlite::params![item_id])
        .map_err(|err| VocabError::new(format!("run item tags query: {err}")))?;
    let mut names = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| VocabError::new(format!("read item tags row: {err}")))?
    {
        names.push(row.get(0)?);
    }
    Ok(names)
}

fn insert_item(
    tx: &rusqlite::Transaction<'_>,
    record: &ImportRecord,
    status: ItemStatus,
) -> Result<i64> {
    let provenance = Provenance {
        source: SourceId("pleco".to_owned()),
        source_version: SourceVersion("import".to_owned()),
        import_origin: None,
        confirmation: status_confirmation(status),
    };
    tx.execute(
        "INSERT INTO vocabulary_items \
         (simplified, traditional, pinyin, definition, status, source_id, source_version) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            record.simplified.trim(),
            record.simplified.trim(),
            record.pinyin.trim(),
            record.definition.trim(),
            status.as_str(),
            provenance.source.as_str(),
            provenance.source_version.as_str(),
        ],
    )
    .map_err(|err| VocabError::new(format!("insert imported item: {err}")))?;
    Ok(tx.last_insert_rowid())
}

fn status_confirmation(status: ItemStatus) -> ConfirmationState {
    match status {
        ItemStatus::NeedsReview => ConfirmationState::NeedsReview,
        _ => ConfirmationState::DictionaryAuthority,
    }
}

fn record_import_run(
    tx: &rusqlite::Transaction<'_>,
    payload: &ImportPayload,
    imported: usize,
    skipped: usize,
) -> Result<()> {
    tx.execute(
        "INSERT INTO import_runs \
         (source_path, codec_key, content_sha256, records_seen, records_imported, \
          records_skipped_duplicate, issues_errors, status) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'committed')",
        rusqlite::params![
            payload.source_path,
            payload.codec_key,
            payload.content_sha256,
            count_to_i64(payload.records.len())?,
            count_to_i64(imported)?,
            count_to_i64(skipped)?,
            count_to_i64(payload.issues_errors)?,
        ],
    )
    .map_err(|err| VocabError::new(format!("record import run: {err}")))?;
    Ok(())
}

fn count_to_i64(count: usize) -> Result<i64> {
    i64::try_from(count).map_err(|_| VocabError::new("count out of range for i64"))
}

fn ensure_tag(conn: &rusqlite::Connection, name: &str) -> Result<i64> {
    let existing = conn
        .query_row(
            "SELECT tag_id FROM tags WHERE name = ?1 COLLATE NOCASE",
            rusqlite::params![name],
            |row| row.get(0),
        )
        .optional()
        .map_err(|err| VocabError::new(format!("lookup tag: {err}")))?;
    if let Some(tag_id) = existing {
        return Ok(tag_id);
    }
    conn.execute(
        "INSERT INTO tags (name) VALUES (?1)",
        rusqlite::params![name],
    )
    .map_err(|err| VocabError::new(format!("insert tag: {err}")))?;
    Ok(conn.last_insert_rowid())
}

fn link_tag(conn: &rusqlite::Connection, item_id: i64, tag_id: i64) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO vocabulary_tags (item_id, tag_id) VALUES (?1, ?2)",
        rusqlite::params![item_id, tag_id],
    )
    .map_err(|err| VocabError::new(format!("link tag: {err}")))?;
    Ok(())
}

fn record_audit(conn: &rusqlite::Connection, event_type: &str, detail: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO audit_events (event_type, detail) VALUES (?1, ?2)",
        rusqlite::params![event_type, detail],
    )
    .map_err(|err| VocabError::new(format!("record audit event: {err}")))?;
    Ok(())
}
