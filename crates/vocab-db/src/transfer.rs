//! The transfer ledger: every import and export, and what happened to each
//! word in it. "Only words not yet sent to Pleco" is a question for this
//! ledger, asked per destination.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, params};
use vocab_core::Result;

use crate::to_i64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    In,
    Out,
}

impl Direction {
    fn as_str(self) -> &'static str {
        match self {
            Self::In => "in",
            Self::Out => "out",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Committed,
    /// Refused (error lines, not forced). Nothing else was written.
    Rejected,
}

impl RunStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Committed => "committed",
            Self::Rejected => "rejected",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RunCounts {
    pub seen: usize,
    pub inserted: usize,
    pub updated: usize,
    pub skipped: usize,
    /// Imported, but marked needs review.
    pub unresolved: usize,
    pub dropped: usize,
    pub errors: usize,
    pub warnings: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRecord {
    pub direction: Direction,
    /// Destination key: `pleco`.
    pub connector_id: String,
    /// Exact grammar: `pleco-utf8-text/v1`.
    pub format: String,
    pub path: String,
    pub content_sha256: Option<String>,
    pub counts: RunCounts,
    pub status: RunStatus,
}

/// What happened to one word or line in a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Inserted,
    Updated,
    /// Already saved; left alone. The word is known to be at the source.
    Skipped,
    Dropped,
    Rejected,
    Written,
}

impl Outcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Inserted => "inserted",
            Self::Updated => "updated",
            Self::Skipped => "skipped",
            Self::Dropped => "dropped",
            Self::Rejected => "rejected",
            Self::Written => "written",
        }
    }
}

/// Outcomes that mean the destination has the word.
const AT_DESTINATION: &str = "('inserted', 'updated', 'skipped', 'written')";

/// Records a run and returns its id.
///
/// # Errors
///
/// Returns an error if the write fails.
pub fn record_run(conn: &Connection, run: &RunRecord) -> Result<i64> {
    let c = &run.counts;
    conn.execute(
        "INSERT INTO transfer_runs (direction, connector_id, format, path, content_sha256, \
         seen, inserted, updated, skipped, unresolved, dropped, errors, warnings, status) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            run.direction.as_str(),
            run.connector_id,
            run.format,
            run.path,
            run.content_sha256,
            to_i64(c.seen)?,
            to_i64(c.inserted)?,
            to_i64(c.updated)?,
            to_i64(c.skipped)?,
            to_i64(c.unresolved)?,
            to_i64(c.dropped)?,
            to_i64(c.errors)?,
            to_i64(c.warnings)?,
            run.status.as_str(),
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Records what happened to one word or line.
///
/// # Errors
///
/// Returns an error if the write fails.
pub fn record_item(
    conn: &Connection,
    run_id: i64,
    item_id: Option<i64>,
    line: Option<usize>,
    raw: Option<&str>,
    outcome: Outcome,
    detail: &str,
) -> Result<()> {
    let line = line.map(to_i64).transpose()?;
    conn.execute(
        "INSERT INTO transfer_items (run_id, item_id, line, raw, outcome, detail) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![run_id, item_id, line, raw, outcome.as_str(), detail],
    )?;
    Ok(())
}

/// Files that were imported from. Exports never overwrite them.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn import_sources(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT path FROM transfer_runs \
         WHERE direction = 'in' AND status = 'committed' ORDER BY path",
    )?;
    let rows = stmt.query_map([], |row| row.get(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<String>>>()?)
}

/// Words the destination already has: exported there, or imported (or
/// matched) from it.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn items_at(conn: &Connection, connector_id: &str) -> Result<HashSet<i64>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT DISTINCT ti.item_id FROM transfer_items ti \
         JOIN transfer_runs tr ON tr.id = ti.run_id \
         WHERE tr.connector_id = ?1 AND tr.status = 'committed' \
         AND ti.item_id IS NOT NULL AND ti.outcome IN {AT_DESTINATION}"
    ))?;
    let rows = stmt.query_map([connector_id], |row| row.get(0))?;
    Ok(rows.collect::<rusqlite::Result<HashSet<i64>>>()?)
}

/// The destinations that have one word, by id.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn item_destinations(conn: &Connection, item_id: i64) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT DISTINCT tr.connector_id FROM transfer_items ti \
         JOIN transfer_runs tr ON tr.id = ti.run_id \
         WHERE ti.item_id = ?1 AND tr.status = 'committed' \
         AND ti.outcome IN {AT_DESTINATION} ORDER BY tr.connector_id"
    ))?;
    let rows = stmt.query_map([item_id], |row| row.get(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<String>>>()?)
}

/// For every word, the destinations that have it, by id.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn destinations_by_item(conn: &Connection) -> Result<HashMap<i64, Vec<String>>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT DISTINCT ti.item_id, tr.connector_id FROM transfer_items ti \
         JOIN transfer_runs tr ON tr.id = ti.run_id \
         WHERE tr.status = 'committed' AND ti.item_id IS NOT NULL \
         AND ti.outcome IN {AT_DESTINATION} ORDER BY tr.connector_id"
    ))?;
    let mut rows = stmt.query([])?;
    let mut out: HashMap<i64, Vec<String>> = HashMap::new();
    while let Some(row) = rows.next()? {
        out.entry(row.get(0)?).or_default().push(row.get(1)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use vocab_core::{ItemSource, Verification};

    use super::{
        Direction, Outcome, RunCounts, RunRecord, RunStatus, import_sources, items_at, record_item,
        record_run,
    };
    use crate::items::{NewItem, save_item};
    use crate::open_in_memory;

    fn run(direction: Direction, connector: &str, path: &str, status: RunStatus) -> RunRecord {
        RunRecord {
            direction,
            connector_id: connector.to_owned(),
            format: format!("{connector}/v1"),
            path: path.to_owned(),
            content_sha256: None,
            counts: RunCounts::default(),
            status,
        }
    }

    #[test]
    fn destinations_are_tracked_separately() {
        let conn = open_in_memory().unwrap();
        let id = save_item(
            &conn,
            &NewItem {
                simplified: "学校".to_owned(),
                traditional: String::new(),
                pinyin: "xue2 xiao4".to_owned(),
                definition: "school".to_owned(),
                notes: String::new(),
                verification: Verification::Confirmed,
                source: ItemSource::manual(),
            },
        )
        .unwrap()
        .item()
        .id;
        let out = record_run(
            &conn,
            &run(Direction::Out, "pleco", "/tmp/p.txt", RunStatus::Committed),
        )
        .unwrap();
        record_item(&conn, out, Some(id), None, None, Outcome::Written, "").unwrap();
        assert!(items_at(&conn, "pleco").unwrap().contains(&id));
        assert_eq!(super::item_destinations(&conn, id).unwrap(), vec!["pleco"]);
        assert!(items_at(&conn, "anki").unwrap().is_empty());
    }

    #[test]
    fn only_committed_imports_are_sources() {
        let conn = open_in_memory().unwrap();
        record_run(
            &conn,
            &run(Direction::In, "pleco", "/tmp/in.txt", RunStatus::Committed),
        )
        .unwrap();
        record_run(
            &conn,
            &run(Direction::In, "pleco", "/tmp/bad.txt", RunStatus::Rejected),
        )
        .unwrap();
        record_run(
            &conn,
            &run(
                Direction::Out,
                "pleco",
                "/tmp/out.txt",
                RunStatus::Committed,
            ),
        )
        .unwrap();
        assert_eq!(
            import_sources(&conn).unwrap(),
            vec!["/tmp/in.txt".to_owned()]
        );
    }
}
