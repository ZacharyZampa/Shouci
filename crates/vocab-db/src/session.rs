//! Target-agnostic transfer session. Executes a plan; does not resolve cards.

use vocab_core::{ItemStatus, Result, VocabError, VocabItem};

use crate::repo::{find_by_reading, pinyin_key};
use crate::transfer::{count_to_i64, ensure_tag, link_tag, record_audit};
use crate::{set_status, with_tx};

const STALE: &str = "stale transfer plan";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferDirection {
    In,
    Out,
}

impl TransferDirection {
    fn as_str(self) -> &'static str {
        match self {
            Self::In => "in",
            Self::Out => "out",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferHeader {
    pub direction: TransferDirection,
    pub target: String,
    pub codec_key: String,
    pub profile: String,
    pub source_path: String,
    pub content_sha256: Option<String>,
    pub records_seen: usize,
    pub issues_errors: usize,
    pub issues_warnings: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundInsert {
    pub simplified: String,
    pub traditional: String,
    pub pinyin: String,
    pub definition: String,
    pub notes: Option<String>,
    pub status: ItemStatus,
    pub source_entry_id: Option<i64>,
    pub source_id: String,
    pub source_version: String,
    pub import_origin: String,
    pub tags: Vec<String>,
    pub line: Option<usize>,
    pub raw_line: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InboundOp {
    Insert(Box<InboundInsert>),
    Skip {
        item_id: i64,
        line: Option<usize>,
        raw_line: Option<String>,
        detail: String,
    },
    Drop {
        line: Option<usize>,
        raw_line: Option<String>,
        detail: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TransferCounts {
    pub inserted: usize,
    pub skipped: usize,
    pub unresolved: usize,
    pub dropped: usize,
    pub written: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionOutcome {
    Applied(TransferCounts),
    Stale,
}

/// Applies an inbound plan in one transaction.
///
/// A skip does not change the stored item. Insert and skip both record a
/// per-target transfer mark. `Stale` means the reading key changed under the
/// plan; nothing is committed.
///
/// # Errors
///
/// Returns an error if the transaction fails for a reason other than staleness.
pub fn apply_inbound(
    conn: &mut rusqlite::Connection,
    header: &TransferHeader,
    ops: &[InboundOp],
) -> Result<SessionOutcome> {
    match with_tx(conn, |tx| apply_inbound_tx(tx, header, ops)) {
        Ok(counts) => Ok(SessionOutcome::Applied(counts)),
        Err(err) if err.to_string() == STALE => Ok(SessionOutcome::Stale),
        Err(err) => Err(err),
    }
}

/// Records a refused inbound file. No vocabulary rows and no `import_runs` row.
///
/// # Errors
///
/// Returns an error if the audit write fails.
pub fn record_rejected(
    conn: &mut rusqlite::Connection,
    header: &TransferHeader,
    lines: &[(usize, String)],
) -> Result<()> {
    with_tx(conn, |tx| {
        let run_id = insert_run(tx, header, TransferCounts::default(), "rejected")?;
        for (line, raw) in lines {
            insert_transfer_item(tx, run_id, None, Some(*line), Some(raw), "rejected", "")?;
        }
        Ok(())
    })
}

/// Marks exported items and records the outbound run.
///
/// Also sets `ItemStatus::Exported` so existing status filters keep meaning
/// "included in at least one export". The per-target mark is the transfer row.
///
/// # Errors
///
/// Returns an error if the transaction fails.
pub fn commit_outbound(
    conn: &mut rusqlite::Connection,
    header: &TransferHeader,
    item_ids: &[i64],
) -> Result<()> {
    with_tx(conn, |tx| {
        for &item_id in item_ids {
            set_status(tx, item_id, ItemStatus::Exported)?;
        }
        let counts = TransferCounts {
            written: item_ids.len(),
            ..TransferCounts::default()
        };
        let run_id = insert_run(tx, header, counts, "committed")?;
        for &item_id in item_ids {
            insert_transfer_item(tx, run_id, Some(item_id), None, None, "written", "")?;
        }
        tx.execute(
            "INSERT INTO export_runs (target_path, codec_key, records_written, status) \
             VALUES (?1, ?2, ?3, 'committed')",
            rusqlite::params![
                header.source_path,
                header.codec_key,
                count_to_i64(item_ids.len())?,
            ],
        )
        .map_err(|err| VocabError::new(format!("record export run: {err}")))?;
        record_audit(
            tx,
            "export",
            &format!(
                "{} | written {} -> {}",
                header.codec_key,
                item_ids.len(),
                header.source_path
            ),
        )?;
        Ok(())
    })
}

/// Items eligible to leave Shouci.
///
/// `needs_review` and `archived` are excluded unless requested. `only_new`
/// drops items that already have a committed transfer mark for `target`.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn select_for_transfer(
    conn: &rusqlite::Connection,
    target: &str,
    required_tags: &[String],
    include_unresolved: bool,
    include_archived: bool,
    only_new: bool,
) -> Result<Vec<VocabItem>> {
    let mut conditions = Vec::new();
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    if !include_unresolved {
        conditions.push("status <> 'needs_review'".to_owned());
    }
    if !include_archived {
        conditions.push("status <> 'archived'".to_owned());
    }
    if only_new {
        conditions.push(
            "NOT EXISTS (SELECT 1 FROM transfer_items ti \
             JOIN transfer_runs tr ON tr.run_id = ti.run_id \
             WHERE ti.item_id = vocabulary_items.item_id \
             AND tr.target = ? AND tr.status = 'committed' \
             AND ti.outcome IN ('inserted', 'skipped_duplicate', 'written'))"
                .to_owned(),
        );
        params.push(target.to_owned().into());
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
    crate::repo::select_items(conn, &tail, rusqlite::params_from_iter(params.iter()))
}

fn apply_inbound_tx(
    tx: &rusqlite::Transaction<'_>,
    header: &TransferHeader,
    ops: &[InboundOp],
) -> Result<TransferCounts> {
    if plan_is_stale(tx, ops)? {
        return Err(VocabError::new(STALE));
    }
    let mut counts = TransferCounts::default();
    let run_id = insert_run(tx, header, counts, "committed")?;
    for op in ops {
        match op {
            InboundOp::Insert(row) => apply_insert(tx, run_id, row, &mut counts)?,
            InboundOp::Skip {
                item_id,
                line,
                raw_line,
                detail,
            } => {
                counts.skipped += 1;
                insert_transfer_item(
                    tx,
                    run_id,
                    Some(*item_id),
                    *line,
                    raw_line.as_deref(),
                    "skipped_duplicate",
                    detail,
                )?;
            }
            InboundOp::Drop {
                line,
                raw_line,
                detail,
            } => {
                counts.dropped += 1;
                insert_transfer_item(
                    tx,
                    run_id,
                    None,
                    *line,
                    raw_line.as_deref(),
                    "dropped",
                    detail,
                )?;
            }
        }
    }
    tx.execute(
        "UPDATE transfer_runs SET records_inserted = ?1, records_skipped_duplicate = ?2, \
         records_unresolved = ?3, records_dropped = ?4 WHERE run_id = ?5",
        rusqlite::params![
            count_to_i64(counts.inserted)?,
            count_to_i64(counts.skipped)?,
            count_to_i64(counts.unresolved)?,
            count_to_i64(counts.dropped)?,
            run_id,
        ],
    )
    .map_err(|err| VocabError::new(format!("update transfer counts: {err}")))?;
    tx.execute(
        "INSERT INTO import_runs \
         (source_path, codec_key, content_sha256, records_seen, records_imported, \
          records_skipped_duplicate, issues_errors, status) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'committed')",
        rusqlite::params![
            header.source_path,
            header.codec_key,
            header.content_sha256.as_deref().unwrap_or(""),
            count_to_i64(header.records_seen)?,
            count_to_i64(counts.inserted)?,
            count_to_i64(counts.skipped)?,
            count_to_i64(header.issues_errors)?,
        ],
    )
    .map_err(|err| VocabError::new(format!("record import run: {err}")))?;
    record_audit(
        tx,
        "import",
        &format!(
            "{} | seen {} inserted {} skipped {} unresolved {} dropped {}",
            header.codec_key,
            header.records_seen,
            counts.inserted,
            counts.skipped,
            counts.unresolved,
            counts.dropped
        ),
    )?;
    Ok(counts)
}

fn plan_is_stale(tx: &rusqlite::Transaction<'_>, ops: &[InboundOp]) -> Result<bool> {
    for op in ops {
        match op {
            InboundOp::Insert(row) => {
                let key = pinyin_key(&row.pinyin);
                if find_by_reading(tx, &row.simplified, &row.traditional, &key)?.is_some() {
                    return Ok(true);
                }
            }
            InboundOp::Skip { item_id, .. } => {
                if crate::get_item(tx, *item_id)?.is_none() {
                    return Ok(true);
                }
            }
            InboundOp::Drop { .. } => {}
        }
    }
    Ok(false)
}

fn apply_insert(
    tx: &rusqlite::Transaction<'_>,
    run_id: i64,
    row: &InboundInsert,
    counts: &mut TransferCounts,
) -> Result<()> {
    let item_id = insert_inbound(tx, row)?;
    if row.status == ItemStatus::NeedsReview {
        counts.unresolved += 1;
    }
    counts.inserted += 1;
    for tag in &row.tags {
        let tag_id = ensure_tag(tx, tag)?;
        link_tag(tx, item_id, tag_id)?;
    }
    insert_transfer_item(
        tx,
        run_id,
        Some(item_id),
        row.line,
        row.raw_line.as_deref(),
        "inserted",
        "",
    )
}

fn insert_inbound(tx: &rusqlite::Transaction<'_>, row: &InboundInsert) -> Result<i64> {
    let key = pinyin_key(&row.pinyin);
    tx.execute(
        "INSERT INTO vocabulary_items \
         (simplified, traditional, pinyin, pinyin_key, definition, status, notes, \
          source_entry_id, source_id, source_version, import_origin) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        rusqlite::params![
            row.simplified,
            row.traditional,
            row.pinyin,
            key,
            row.definition,
            row.status.as_str(),
            row.notes,
            row.source_entry_id,
            row.source_id,
            row.source_version,
            row.import_origin,
        ],
    )
    .map_err(|err| {
        let message = err.to_string();
        if message.contains("UNIQUE constraint failed") {
            VocabError::new(STALE)
        } else {
            VocabError::new(format!("insert inbound item: {err}"))
        }
    })?;
    Ok(tx.last_insert_rowid())
}

fn insert_run(
    tx: &rusqlite::Transaction<'_>,
    header: &TransferHeader,
    counts: TransferCounts,
    status: &str,
) -> Result<i64> {
    tx.execute(
        "INSERT INTO transfer_runs \
         (direction, target, codec_key, profile, source_path, content_sha256, \
          records_seen, records_inserted, records_skipped_duplicate, records_unresolved, \
          records_dropped, issues_errors, issues_warnings, status) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        rusqlite::params![
            header.direction.as_str(),
            header.target,
            header.codec_key,
            header.profile,
            header.source_path,
            header.content_sha256,
            count_to_i64(header.records_seen)?,
            count_to_i64(counts.inserted)?,
            count_to_i64(counts.skipped)?,
            count_to_i64(counts.unresolved)?,
            count_to_i64(counts.dropped)?,
            count_to_i64(header.issues_errors)?,
            count_to_i64(header.issues_warnings)?,
            status,
        ],
    )
    .map_err(|err| VocabError::new(format!("insert transfer run: {err}")))?;
    Ok(tx.last_insert_rowid())
}

fn insert_transfer_item(
    tx: &rusqlite::Transaction<'_>,
    run_id: i64,
    item_id: Option<i64>,
    line: Option<usize>,
    raw_line: Option<&str>,
    outcome: &str,
    detail: &str,
) -> Result<()> {
    let line = match line {
        Some(value) => Some(count_to_i64(value)?),
        None => None,
    };
    tx.execute(
        "INSERT INTO transfer_items \
         (run_id, item_id, source_line, raw_line, outcome, detail) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![run_id, item_id, line, raw_line, outcome, detail],
    )
    .map_err(|err| VocabError::new(format!("insert transfer item: {err}")))?;
    Ok(())
}
