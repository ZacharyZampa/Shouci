//! Bringing words over from a Shouci proof-of-concept `user.db`.
//!
//! The old database is only read. Running the import again adds only what is
//! new: words match by identity, transfer runs by direction, destination,
//! path, and time.
//!
//! Mapping: `exported` and `confirmed` become confirmed (export history comes
//! along as ledger rows), `needs_review` stays, `archived` becomes a
//! confirmed word archived when it was last modified. Tags stay tags.

use std::collections::HashMap;
use std::path::Path;

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use vocab_core::{ItemSource, Result, SourceKind, Verification, VocabError};

use crate::items::{NewItem, find_by_identity, insert_item};
use crate::organize::add_tag;
use crate::{set_setting, with_tx};

/// Setting written after a successful import: the path imported from.
pub const IMPORTED_SETTING: &str = "legacy.poc_imported_from";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LegacyReport {
    pub items_added: usize,
    /// Words that were already in the new library.
    pub items_already_saved: usize,
    pub runs_added: usize,
    /// Rows that could not be brought over, with why.
    pub skipped: Vec<String>,
}

struct LegacyItem {
    id: i64,
    simplified: String,
    traditional: String,
    pinyin: String,
    definition: String,
    notes: Option<String>,
    status: String,
    source_id: Option<String>,
    source_version: Option<String>,
    import_origin: Option<String>,
    created_at: Option<String>,
    modified_at: Option<String>,
}

/// Imports the words, tags, and transfer history of a POC database.
///
/// # Errors
///
/// Returns an error if the old database cannot be read or is not a POC
/// database, or if writing fails (then nothing is written).
pub fn import_poc_database(conn: &mut Connection, legacy_path: &Path) -> Result<LegacyReport> {
    let legacy = open_legacy(legacy_path)?;
    if !has_table(&legacy, "vocabulary_items")? {
        return Err(VocabError::format(format!(
            "{} is not a Shouci proof-of-concept database",
            legacy_path.display()
        )));
    }
    let items = legacy_items(&legacy)?;
    let tags = if has_table(&legacy, "vocabulary_tags")? {
        legacy_tags(&legacy)?
    } else {
        Vec::new()
    };
    let runs = if has_table(&legacy, "transfer_runs")? && has_table(&legacy, "transfer_items")? {
        legacy_runs(&legacy)?
    } else {
        Vec::new()
    };
    with_tx(conn, |tx| {
        let mut report = LegacyReport::default();
        let mut ids: HashMap<i64, i64> = HashMap::new();
        for item in &items {
            if item.simplified.trim().is_empty() {
                report
                    .skipped
                    .push(format!("row {} has no characters", item.id));
                continue;
            }
            if let Some(existing) =
                find_by_identity(tx, &item.simplified, &item.traditional, &item.pinyin)?
            {
                report.items_already_saved += 1;
                ids.insert(item.id, existing.id);
                continue;
            }
            let new_id = insert_legacy_item(tx, item)?;
            report.items_added += 1;
            ids.insert(item.id, new_id);
        }
        for (legacy_id, name) in &tags {
            if let Some(&id) = ids.get(legacy_id) {
                if !name.trim().is_empty() {
                    add_tag(tx, id, name)?;
                }
            }
        }
        for run in &runs {
            if insert_legacy_run(tx, run, &ids)? {
                report.runs_added += 1;
            }
        }
        set_setting(tx, IMPORTED_SETTING, &legacy_path.to_string_lossy())?;
        Ok(report)
    })
}

fn open_legacy(path: &Path) -> Result<Connection> {
    if !path.is_file() {
        return Err(VocabError::not_found(format!(
            "no database at {}",
            path.display()
        )));
    }
    let mut header = [0u8; 16];
    let is_sqlite = std::fs::File::open(path)
        .and_then(|mut file| std::io::Read::read_exact(&mut file, &mut header))
        .is_ok_and(|()| &header == b"SQLite format 3\0");
    if !is_sqlite {
        return Err(VocabError::format(format!(
            "{} is not a SQLite database",
            path.display()
        )));
    }
    // A WAL database without its shared-memory file cannot be opened
    // read-only; opening it normally only ever reads here.
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .and_then(|conn| {
            conn.query_row("SELECT count(*) FROM sqlite_master", [], |row| {
                row.get::<_, i64>(0)
            })?;
            Ok(conn)
        })
        .or_else(|_| {
            Connection::open_with_flags(
                path,
                OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
        })
        .map_err(|err| VocabError::storage(format!("cannot read {}: {err}", path.display())))
}

fn has_table(conn: &Connection, name: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn columns(conn: &Connection, table: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = stmt.query_map([], |row| row.get(1))?;
    Ok(rows.collect::<rusqlite::Result<Vec<String>>>()?)
}

fn legacy_items(conn: &Connection) -> Result<Vec<LegacyItem>> {
    let present = columns(conn, "vocabulary_items")?;
    let column = |name: &str| {
        if present.iter().any(|c| c == name) {
            name.to_owned()
        } else {
            "NULL".to_owned()
        }
    };
    let sql = format!(
        "SELECT item_id, simplified, traditional, pinyin, definition, status, {}, {}, {}, {}, \
         {}, {} FROM vocabulary_items ORDER BY item_id",
        column("notes"),
        column("source_id"),
        column("source_version"),
        column("import_origin"),
        column("created_at"),
        column("modified_at"),
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        Ok(LegacyItem {
            id: row.get(0)?,
            simplified: row.get(1)?,
            traditional: row.get(2)?,
            pinyin: row.get(3)?,
            definition: row.get(4)?,
            status: row.get(5)?,
            notes: row.get(6)?,
            source_id: row.get(7)?,
            source_version: row.get(8)?,
            import_origin: row.get(9)?,
            created_at: row.get(10)?,
            modified_at: row.get(11)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn legacy_tags(conn: &Connection) -> Result<Vec<(i64, String)>> {
    let mut stmt = conn.prepare(
        "SELECT vt.item_id, t.name FROM vocabulary_tags vt JOIN tags t ON t.tag_id = vt.tag_id",
    )?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// `(item_id, line, raw, outcome, detail)` from the old `transfer_items`.
type LegacyRunItem = (
    Option<i64>,
    Option<i64>,
    Option<String>,
    String,
    Option<String>,
);

struct LegacyRun {
    direction: String,
    target: String,
    codec_key: String,
    path: String,
    content_sha256: Option<String>,
    counts: [i64; 7],
    status: String,
    created_at: String,
    items: Vec<LegacyRunItem>,
}

fn legacy_runs(conn: &Connection) -> Result<Vec<LegacyRun>> {
    let mut stmt = conn.prepare(
        "SELECT run_id, direction, target, codec_key, source_path, content_sha256, records_seen, \
         records_inserted, records_skipped_duplicate, records_unresolved, records_dropped, \
         issues_errors, issues_warnings, status, created_at FROM transfer_runs ORDER BY run_id",
    )?;
    let heads = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                LegacyRun {
                    direction: row.get(1)?,
                    target: row.get(2)?,
                    codec_key: row.get(3)?,
                    path: row.get(4)?,
                    content_sha256: row.get(5)?,
                    counts: [
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                        row.get(9)?,
                        row.get(10)?,
                        row.get(11)?,
                        row.get(12)?,
                    ],
                    status: row.get(13)?,
                    created_at: row.get(14)?,
                    items: Vec::new(),
                },
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut item_stmt = conn.prepare(
        "SELECT item_id, source_line, raw_line, outcome, detail FROM transfer_items \
         WHERE run_id = ?1 ORDER BY transfer_item_id",
    )?;
    let mut runs = Vec::with_capacity(heads.len());
    for (run_id, mut run) in heads {
        run.items = item_stmt
            .query_map([run_id], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        runs.push(run);
    }
    Ok(runs)
}

/// `2026-09-20 13:38:00` (SQLite `datetime('now')`, UTC) →
/// `2026-09-20T13:38:00.000Z`.
fn iso(timestamp: &str) -> String {
    let t = timestamp.trim();
    if t.contains('T') {
        return t.to_owned();
    }
    format!("{}.000Z", t.replacen(' ', "T", 1))
}

fn insert_legacy_item(conn: &Connection, item: &LegacyItem) -> Result<i64> {
    let (verification, archived) = match item.status.as_str() {
        "needs_review" => (Verification::NeedsReview, false),
        "archived" => (Verification::Confirmed, true),
        _ => (Verification::Confirmed, false),
    };
    let source_id = item.source_id.clone().filter(|id| !id.is_empty());
    let kind = match source_id.as_deref() {
        _ if item.import_origin.is_some() => SourceKind::Import,
        Some("pleco" | "anki") => SourceKind::Import,
        None | Some("user") => SourceKind::Manual,
        Some(_) => SourceKind::Dictionary,
    };
    let id = insert_item(
        conn,
        &NewItem {
            simplified: item.simplified.clone(),
            traditional: item.traditional.clone(),
            pinyin: item.pinyin.clone(),
            definition: item.definition.clone(),
            notes: item.notes.clone().unwrap_or_default(),
            verification,
            source: ItemSource {
                kind,
                id: source_id.filter(|id| id != "user"),
                version: item
                    .source_version
                    .clone()
                    .filter(|v| !v.is_empty() && v != "manual"),
                import_origin: item.import_origin.clone(),
            },
        },
    )?;
    let created = item.created_at.as_deref().map(iso);
    let modified = item.modified_at.as_deref().map(iso).or(created.clone());
    conn.execute(
        "UPDATE items SET created_at = coalesce(?1, created_at), \
         modified_at = coalesce(?2, modified_at), \
         archived_at = CASE WHEN ?3 THEN coalesce(?2, modified_at) ELSE NULL END \
         WHERE id = ?4",
        params![created, modified, archived, id],
    )?;
    Ok(id)
}

fn insert_legacy_run(conn: &Connection, run: &LegacyRun, ids: &HashMap<i64, i64>) -> Result<bool> {
    let created_at = iso(&run.created_at);
    let exists = conn
        .query_row(
            "SELECT 1 FROM transfer_runs WHERE direction = ?1 AND connector_id = ?2 \
             AND path = ?3 AND created_at = ?4",
            params![run.direction, run.target, run.path, created_at],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if exists {
        return Ok(false);
    }
    let [
        seen,
        inserted,
        skipped,
        unresolved,
        dropped,
        errors,
        warnings,
    ] = run.counts;
    conn.execute(
        "INSERT INTO transfer_runs (direction, connector_id, format, path, content_sha256, seen, \
         inserted, skipped, unresolved, dropped, errors, warnings, status, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            run.direction,
            run.target,
            run.codec_key,
            run.path,
            run.content_sha256,
            seen,
            inserted,
            skipped,
            unresolved,
            dropped,
            errors,
            warnings,
            run.status,
            created_at,
        ],
    )?;
    let run_id = conn.last_insert_rowid();
    for (item_id, line, raw, outcome, detail) in &run.items {
        let outcome = match outcome.as_str() {
            "inserted" => "inserted",
            "skipped_duplicate" => "skipped",
            "dropped" => "dropped",
            "rejected" => "rejected",
            "written" => "written",
            _ => continue,
        };
        conn.execute(
            "INSERT INTO transfer_items (run_id, item_id, simplified, traditional, reading_key, \
             line, raw, outcome, detail) \
             SELECT ?1, ?2, i.simplified, i.traditional, i.reading_key, ?3, ?4, ?5, ?6 \
             FROM (SELECT 1) LEFT JOIN items i ON i.id = ?2",
            params![
                run_id,
                item_id.and_then(|id| ids.get(&id).copied()),
                line,
                raw,
                outcome,
                detail.clone().unwrap_or_default(),
            ],
        )?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use vocab_core::{LibraryFilter, LibraryView, Lifecycle, Verification};

    use super::{import_poc_database, iso};
    use crate::{item_tags, items_at, list_items, open_in_memory};

    /// The POC schema, as its last build left it.
    const POC: &str = r"
CREATE TABLE vocabulary_items (
    item_id INTEGER PRIMARY KEY, simplified TEXT NOT NULL, traditional TEXT NOT NULL,
    pinyin TEXT NOT NULL, pinyin_key TEXT NOT NULL DEFAULT '', definition TEXT NOT NULL,
    status TEXT NOT NULL, notes TEXT, source_entry_id INTEGER, source_id TEXT,
    source_version TEXT, import_origin TEXT, origin_export_id INTEGER,
    created_at TEXT NOT NULL, modified_at TEXT NOT NULL);
CREATE TABLE tags (tag_id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, created_at TEXT);
CREATE TABLE vocabulary_tags (item_id INTEGER NOT NULL, tag_id INTEGER NOT NULL);
CREATE TABLE transfer_runs (run_id INTEGER PRIMARY KEY, direction TEXT NOT NULL,
    target TEXT NOT NULL, codec_key TEXT NOT NULL, profile TEXT NOT NULL,
    source_path TEXT NOT NULL, content_sha256 TEXT, records_seen INTEGER NOT NULL,
    records_inserted INTEGER NOT NULL, records_skipped_duplicate INTEGER NOT NULL,
    records_unresolved INTEGER NOT NULL, records_dropped INTEGER NOT NULL,
    issues_errors INTEGER NOT NULL, issues_warnings INTEGER NOT NULL, status TEXT NOT NULL,
    created_at TEXT NOT NULL);
CREATE TABLE transfer_items (transfer_item_id INTEGER PRIMARY KEY, run_id INTEGER NOT NULL,
    item_id INTEGER, source_line INTEGER, raw_line TEXT, outcome TEXT NOT NULL, detail TEXT);
INSERT INTO vocabulary_items VALUES
  (1, '学校', '學校', 'xue2 xiao4', 'xue2 xiao4', 'school', 'exported', NULL, 7,
   'cc-cedict', '1.0.0', NULL, NULL, '2026-09-20 10:00:00', '2026-09-21 10:00:00'),
  (2, 'zzz', 'zzz', '', '', 'unresolved query: zzz', 'needs_review', NULL, NULL,
   'user', 'manual', NULL, NULL, '2026-09-20 11:00:00', '2026-09-20 11:00:00'),
  (3, '猫', '貓', 'mao1', 'mao1', 'cat', 'archived', 'pet', NULL, 'pleco', 'import',
   '/tmp/in.txt', NULL, '2026-09-20 12:00:00', '2026-09-22 12:00:00');
INSERT INTO tags VALUES (1, 'Animals', NULL);
INSERT INTO vocabulary_tags VALUES (3, 1);
INSERT INTO transfer_runs VALUES (1, 'out', 'pleco', 'pleco-utf8-text/v1', 'pleco',
  '/tmp/out.txt', NULL, 1, 0, 0, 0, 0, 0, 0, 'committed', '2026-09-21 10:00:00');
INSERT INTO transfer_items VALUES (1, 1, 1, NULL, NULL, 'written', '');
";

    fn legacy_file(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("shouci-legacy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.db"));
        let _ = std::fs::remove_file(&path);
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch(POC)
            .unwrap();
        path
    }

    #[test]
    fn words_tags_and_history_come_over_once() {
        let path = legacy_file("once");
        let mut conn = open_in_memory().unwrap();
        let first = import_poc_database(&mut conn, &path).unwrap();
        assert_eq!(first.items_added, 3);
        assert_eq!(first.runs_added, 1);

        let all = list_items(
            &conn,
            &LibraryFilter {
                view: LibraryView::All,
                ..LibraryFilter::default()
            },
        )
        .unwrap();
        let find = |s: &str| all.iter().find(|item| item.simplified == s).unwrap();
        let school = find("学校");
        assert_eq!(school.verification, Verification::Confirmed);
        assert_eq!(school.created_at, "2026-09-20T10:00:00.000Z");
        assert!(items_at(&conn, "pleco").unwrap().contains(&school.id));
        assert_eq!(find("zzz").verification, Verification::NeedsReview);
        let cat = find("猫");
        assert_eq!(cat.lifecycle(), Lifecycle::Archived);
        assert_eq!(cat.notes, "pet");
        assert_eq!(cat.source.import_origin.as_deref(), Some("/tmp/in.txt"));
        assert_eq!(item_tags(&conn, cat.id).unwrap(), vec!["Animals"]);

        let again = import_poc_database(&mut conn, &path).unwrap();
        assert_eq!(again.items_added, 0);
        assert_eq!(again.items_already_saved, 3);
        assert_eq!(again.runs_added, 0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_bad_row_is_skipped_not_fatal_and_junk_is_a_format_error() {
        let path = legacy_file("bad-row");
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute(
                "INSERT INTO vocabulary_items VALUES (9, ' ', ' ', '', '', '', 'confirmed', \
                 NULL, NULL, NULL, NULL, NULL, NULL, '2026-09-20 10:00:00', \
                 '2026-09-20 10:00:00')",
                [],
            )
            .unwrap();
        let mut conn = open_in_memory().unwrap();
        let report = import_poc_database(&mut conn, &path).unwrap();
        assert_eq!(report.items_added, 3);
        assert_eq!(report.skipped.len(), 1);
        let junk = path.with_file_name("junk.db");
        std::fs::write(&junk, b"not a database at all").unwrap();
        let err = import_poc_database(&mut conn, &junk).unwrap_err();
        assert_eq!(err.kind(), vocab_core::ErrorKind::Format);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&junk);
    }

    #[test]
    fn timestamps_become_iso() {
        assert_eq!(iso("2026-09-20 13:38:00"), "2026-09-20T13:38:00.000Z");
        assert_eq!(iso("2026-09-20T13:38:00.000Z"), "2026-09-20T13:38:00.000Z");
    }
}
