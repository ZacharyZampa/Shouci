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

/// Every (word id, destination) pair the ledger says the destination has.
///
/// A ledger row is about a saved word by id or, for a word purged and saved
/// again, by identity. Each case is its own branch so each can use an index
/// (`idx_transfer_items_item`, then the items identity index); joining on
/// `item_id = i.id OR (identity)` instead makes SQLite scan the whole ledger
/// once per word.
macro_rules! pairs {
    () => {
        "SELECT ti.item_id AS item_id, tr.connector_id AS connector_id \
         FROM transfer_items ti JOIN transfer_runs tr ON tr.id = ti.run_id \
         JOIN items i ON i.id = ti.item_id \
         WHERE ti.item_id IS NOT NULL AND tr.status = 'committed' \
         AND ti.outcome IN ('inserted', 'updated', 'skipped', 'written') \
         UNION \
         SELECT i.id, tr.connector_id \
         FROM transfer_items ti JOIN transfer_runs tr ON tr.id = ti.run_id \
         JOIN items i ON i.simplified = ti.simplified COLLATE NOCASE \
         AND i.traditional = ti.traditional COLLATE NOCASE \
         AND i.reading_key = ti.reading_key \
         WHERE ti.item_id IS NULL AND tr.status = 'committed' \
         AND ti.outcome IN ('inserted', 'updated', 'skipped', 'written')"
    };
}

const ITEMS_AT: &str = concat!(
    "SELECT item_id FROM (",
    pairs!(),
    ") WHERE connector_id = ?1"
);
const ITEM_DESTINATIONS: &str = concat!(
    "SELECT DISTINCT connector_id FROM (",
    pairs!(),
    ") WHERE item_id = ?1 ORDER BY connector_id"
);
const DESTINATIONS_BY_ITEM: &str = concat!(
    "SELECT item_id, connector_id FROM (",
    pairs!(),
    ") ORDER BY connector_id"
);

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
    line: Option<u32>,
    raw: Option<&str>,
    outcome: Outcome,
    detail: &str,
) -> Result<()> {
    // The word's identity is copied, so this history still matches it after
    // a purge and a fresh save.
    conn.execute(
        "INSERT INTO transfer_items (run_id, item_id, simplified, traditional, reading_key, \
         line, raw, outcome, detail) \
         SELECT ?1, ?2, i.simplified, i.traditional, i.reading_key, ?3, ?4, ?5, ?6 \
         FROM (SELECT 1) LEFT JOIN items i ON i.id = ?2",
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
    let mut stmt = conn.prepare(ITEMS_AT)?;
    let rows = stmt.query_map([connector_id], |row| row.get(0))?;
    Ok(rows.collect::<rusqlite::Result<HashSet<i64>>>()?)
}

/// The destinations that have one word, by id.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn item_destinations(conn: &Connection, item_id: i64) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(ITEM_DESTINATIONS)?;
    let rows = stmt.query_map([item_id], |row| row.get(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<String>>>()?)
}

/// For every word, the destinations that have it, by id.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn destinations_by_item(conn: &Connection) -> Result<HashMap<i64, Vec<String>>> {
    let mut stmt = conn.prepare(DESTINATIONS_BY_ITEM)?;
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

    use rusqlite::StatementStatus;

    use super::{
        DESTINATIONS_BY_ITEM, Direction, ITEM_DESTINATIONS, ITEMS_AT, Outcome, RunCounts,
        RunRecord, RunStatus, destinations_by_item, import_sources, item_destinations, items_at,
        record_item, record_run,
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
    fn history_outlives_a_purge() {
        let conn = open_in_memory().unwrap();
        let word = NewItem {
            simplified: "学校".to_owned(),
            traditional: String::new(),
            pinyin: "xue2 xiao4".to_owned(),
            definition: "school".to_owned(),
            notes: String::new(),
            verification: Verification::Confirmed,
            source: ItemSource::manual(),
        };
        let id = save_item(&conn, &word).unwrap().item().id;
        let out = record_run(
            &conn,
            &run(Direction::Out, "pleco", "/tmp/p.txt", RunStatus::Committed),
        )
        .unwrap();
        record_item(&conn, out, Some(id), None, None, Outcome::Written, "").unwrap();
        crate::set_trashed(&conn, id, true).unwrap();
        crate::purge_item(&conn, id).unwrap();
        let again = save_item(&conn, &word).unwrap().item().id;
        assert_ne!(again, id, "ids are never reused");
        assert!(items_at(&conn, "pleco").unwrap().contains(&again));
        assert_eq!(item_destinations(&conn, again).unwrap(), vec!["pleco"]);
        assert_eq!(
            destinations_by_item(&conn).unwrap().get(&again),
            Some(&vec!["pleco".to_owned()])
        );
    }

    #[test]
    fn a_word_matched_both_ways_is_listed_once() {
        let conn = open_in_memory().unwrap();
        let word = NewItem {
            simplified: "学校".to_owned(),
            traditional: String::new(),
            pinyin: "xue2 xiao4".to_owned(),
            definition: "school".to_owned(),
            notes: String::new(),
            verification: Verification::Confirmed,
            source: ItemSource::manual(),
        };
        let first = save_item(&conn, &word).unwrap().item().id;
        let out = record_run(
            &conn,
            &run(Direction::Out, "pleco", "/tmp/p.txt", RunStatus::Committed),
        )
        .unwrap();
        record_item(&conn, out, Some(first), None, None, Outcome::Written, "").unwrap();
        crate::set_trashed(&conn, first, true).unwrap();
        crate::purge_item(&conn, first).unwrap();
        let again = save_item(&conn, &word).unwrap().item().id;
        // Exported again: one row by id, one by identity.
        let out = record_run(
            &conn,
            &run(Direction::Out, "pleco", "/tmp/p.txt", RunStatus::Committed),
        )
        .unwrap();
        record_item(&conn, out, Some(again), None, None, Outcome::Written, "").unwrap();
        assert_eq!(item_destinations(&conn, again).unwrap(), vec!["pleco"]);
        assert_eq!(
            destinations_by_item(&conn).unwrap().get(&again),
            Some(&vec!["pleco".to_owned()])
        );
    }

    #[test]
    fn rejected_runs_and_dropped_lines_are_not_at_the_destination() {
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
        let rejected = record_run(
            &conn,
            &run(Direction::In, "pleco", "/tmp/in.txt", RunStatus::Rejected),
        )
        .unwrap();
        record_item(&conn, rejected, Some(id), None, None, Outcome::Skipped, "").unwrap();
        let committed = record_run(
            &conn,
            &run(Direction::In, "anki", "/tmp/in.txt", RunStatus::Committed),
        )
        .unwrap();
        record_item(&conn, committed, Some(id), None, None, Outcome::Dropped, "").unwrap();
        record_item(
            &conn,
            committed,
            None,
            Some(3),
            Some("x"),
            Outcome::Rejected,
            "",
        )
        .unwrap();
        assert!(destinations_by_item(&conn).unwrap().is_empty());
        assert_eq!(item_destinations(&conn, id).unwrap(), [] as [String; 0]);
    }

    /// Listing a library asks for every word's destinations, so the cost must
    /// grow with the ledger, not with words × ledger rows. Counted in SQLite
    /// VM steps, which unlike time do not depend on the machine.
    #[test]
    fn ledger_queries_stay_linear() {
        const WORDS: usize = 2_000;
        let conn = open_in_memory().unwrap();
        conn.execute_batch(&format!(
            "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n WHERE x < {WORDS}) \
             INSERT INTO items (simplified, traditional, reading_key) \
             SELECT 'w' || x, 'w' || x, 'k' || x FROM n; \
             INSERT INTO transfer_runs (direction, connector_id, format, path, status) \
             VALUES ('out', 'pleco', 'f', '/p', 'committed'), ('out', 'anki', 'f', '/a', 'committed'); \
             INSERT INTO transfer_items (run_id, item_id, simplified, traditional, reading_key, outcome) \
             SELECT 1, id, simplified, traditional, reading_key, 'written' FROM items; \
             INSERT INTO transfer_items (run_id, item_id, simplified, traditional, reading_key, outcome) \
             SELECT 2, NULL, simplified, traditional, reading_key, 'written' FROM items;"
        ))
        .unwrap();
        for (sql, param) in [
            (DESTINATIONS_BY_ITEM, None),
            (
                ITEMS_AT,
                Some(rusqlite::types::Value::from("pleco".to_owned())),
            ),
            (ITEM_DESTINATIONS, Some(rusqlite::types::Value::from(7_i64))),
        ] {
            let mut stmt = conn.prepare(sql).unwrap();
            let rows = match param {
                Some(value) => stmt.query_map([value], |_| Ok(())).unwrap().count(),
                None => stmt.query_map([], |_| Ok(())).unwrap().count(),
            };
            assert!(rows > 0, "{sql}");
            let steps = usize::try_from(stmt.get_status(StatementStatus::VmStep)).unwrap();
            // Linear is a few dozen steps per row; words × rows is millions.
            assert!(steps < 200 * WORDS, "{steps} VM steps for {sql}");
        }
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
