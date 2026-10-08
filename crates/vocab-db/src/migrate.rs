//! Schema versions, tracked in `PRAGMA user_version`.
//!
//! Each entry of [`MIGRATIONS`] moves the schema one version forward. To
//! change the schema, append a migration; never edit one that has shipped.
//! A migration may be SQL or Rust (for data that must be recomputed, such as
//! reading keys after a pinyin rule changes). A new condition for smart
//! collections' filters needs one too, even an empty one (`smart.rs`).
//!
//! How a migration runs, so table rebuilds are safe:
//! 1. foreign keys are switched off (impossible inside a transaction, which
//!    is why the runner does it, not the migration);
//! 2. an IMMEDIATE transaction takes the write lock before the version is
//!    read, so two processes opening the database at once never both
//!    migrate;
//! 3. after the migration, `PRAGMA foreign_key_check` must find nothing, or
//!    everything rolls back;
//! 4. foreign keys come back on.
//!
//! Enum-like columns have no CHECK constraints: widening one would need a
//! table rebuild. Values are validated in Rust instead.
//!
//! Before anything writes to a database, [`check`] makes sure it is one
//! Shouci can open safely: empty, or holding every table and column its
//! version should have (what the migrations build up to that version).
//! Anything else (another program's database, a newer Shouci's) is refused
//! and left as it is.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{Connection, Transaction, TransactionBehavior};
use vocab_core::{Result, VocabError};

const V1: &str = r"
CREATE TABLE items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    simplified TEXT NOT NULL,
    traditional TEXT NOT NULL,
    pinyin TEXT NOT NULL DEFAULT '',
    reading_key TEXT NOT NULL DEFAULT '',
    definition TEXT NOT NULL DEFAULT '',
    notes TEXT NOT NULL DEFAULT '',
    -- confirmed | needs_review
    verification TEXT NOT NULL DEFAULT 'confirmed',
    archived_at TEXT,
    deleted_at TEXT,
    -- dictionary | import | manual
    source_kind TEXT NOT NULL DEFAULT 'manual',
    source_id TEXT,
    source_version TEXT,
    import_origin TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    modified_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- Bumped by every change to the word, its tags, or its collections.
    rev INTEGER NOT NULL DEFAULT 1,
    UNIQUE (simplified COLLATE NOCASE, traditional COLLATE NOCASE, reading_key)
);

CREATE TABLE tags (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE COLLATE NOCASE
);

CREATE TABLE item_tags (
    item_id INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (item_id, tag_id)
);

CREATE TABLE collections (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE COLLATE NOCASE,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE collection_items (
    collection_id INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    item_id INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    added_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (collection_id, item_id)
);

-- One row per import or export. `connector_id` is the destination key
-- (`pleco`), so 'only new words' is answered per destination.
CREATE TABLE transfer_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- in | out
    direction TEXT NOT NULL,
    connector_id TEXT NOT NULL,
    format TEXT NOT NULL,
    path TEXT NOT NULL,
    content_sha256 TEXT,
    seen INTEGER NOT NULL DEFAULT 0,
    inserted INTEGER NOT NULL DEFAULT 0,
    updated INTEGER NOT NULL DEFAULT 0,
    skipped INTEGER NOT NULL DEFAULT 0,
    unresolved INTEGER NOT NULL DEFAULT 0,
    dropped INTEGER NOT NULL DEFAULT 0,
    errors INTEGER NOT NULL DEFAULT 0,
    warnings INTEGER NOT NULL DEFAULT 0,
    -- committed | rejected
    status TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- What happened to each word (or line) in a run. The word's identity is
-- copied in, so the history outlives a purged word and matches it if it is
-- saved again.
CREATE TABLE transfer_items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES transfer_runs(id) ON DELETE CASCADE,
    item_id INTEGER REFERENCES items(id) ON DELETE SET NULL,
    simplified TEXT,
    traditional TEXT,
    reading_key TEXT,
    line INTEGER,
    raw TEXT,
    -- inserted | updated | skipped | dropped | rejected | written
    outcome TEXT NOT NULL,
    detail TEXT NOT NULL DEFAULT ''
);

CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE INDEX idx_items_simplified ON items(simplified COLLATE NOCASE);
CREATE INDEX idx_item_tags_tag ON item_tags(tag_id);
CREATE INDEX idx_collection_items_item ON collection_items(item_id);
CREATE INDEX idx_transfer_items_item ON transfer_items(item_id);
CREATE INDEX idx_transfer_items_run ON transfer_items(run_id);
CREATE INDEX idx_transfer_items_word ON transfer_items(simplified COLLATE NOCASE, reading_key);
";

/// Smart collections (`smart.rs`): a name and a filter, kept as JSON.
const V2: &str = r"
CREATE TABLE smart_collections (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE COLLATE NOCASE,
    filter TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
";

/// One step forward.
pub(crate) enum Migration {
    Sql(&'static str),
    /// For data that SQL alone cannot recompute.
    #[allow(dead_code)] // the first Rust migration has not been needed yet
    Rust(fn(&Transaction<'_>) -> Result<()>),
}

const MIGRATIONS: &[Migration] = &[Migration::Sql(V1), Migration::Sql(V2)];

/// The schema version this build writes.
#[allow(clippy::cast_possible_wrap)] // a handful of migrations
pub const SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64;

pub(crate) fn migrate(conn: &mut Connection) -> Result<()> {
    run(conn, MIGRATIONS)
}

/// Tables and their columns.
type Schema = BTreeMap<String, BTreeSet<String>>;

/// Refuses a database this build can't open safely, before anything
/// writes to it: one from a newer Shouci, one holding tables at version 0
/// (another program's), or one missing a table or column its version
/// should have. Extra tables and columns are allowed.
pub(crate) fn check(conn: &mut Connection) -> Result<()> {
    check_against(conn, MIGRATIONS)
}

fn check_against(conn: &mut Connection, migrations: &[Migration]) -> Result<()> {
    #[allow(clippy::cast_possible_wrap)]
    let latest = migrations.len() as i64;
    // One read, so a migration committing in between can't mix versions.
    let tx = conn.transaction_with_behavior(TransactionBehavior::Deferred)?;
    let version: i64 = tx.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version > latest {
        return Err(newer(version, latest));
    }
    let found = schema_of(&tx)?;
    if version == 0 {
        return if found.is_empty() {
            Ok(())
        } else {
            Err(VocabError::storage(
                "this user.db isn't a Shouci library (it holds tables Shouci didn't make), \
                 so it was left as it is",
            ))
        };
    }
    let expected = schema_at(migrations, version)?;
    let missing = expected
        .iter()
        .find_map(|(table, columns)| match found.get(table) {
            None => Some(table.clone()),
            Some(have) => columns
                .iter()
                .find(|column| !have.contains(*column))
                .map(|column| format!("{table}.{column}")),
        });
    match missing {
        Some(missing) => Err(VocabError::storage(format!(
            "this user.db isn't a Shouci library this build can open: it says schema \
             {version} but has no {missing}, so it was left as it is"
        ))),
        None => Ok(()),
    }
}

/// What the first `version` migrations build, made in memory.
fn schema_at(migrations: &[Migration], version: i64) -> Result<Schema> {
    let mut conn = Connection::open_in_memory()?;
    let tx = conn.transaction()?;
    let steps = usize::try_from(version).unwrap_or(0);
    for (index, migration) in migrations.iter().enumerate().take(steps) {
        apply(&tx, index + 1, migration)?;
    }
    schema_of(&tx)
}

fn schema_of(conn: &Connection) -> Result<Schema> {
    let mut tables = conn.prepare(
        r"SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite\_%' ESCAPE '\'",
    )?;
    let names = tables
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut columns = conn.prepare("SELECT name FROM pragma_table_info(?1)")?;
    names
        .into_iter()
        .map(|table| {
            let names = columns
                .query_map([&table], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<BTreeSet<_>>>()?;
            Ok((table, names))
        })
        .collect()
}

fn newer(version: i64, latest: i64) -> VocabError {
    VocabError::storage(format!(
        "this user.db was written by a newer Shouci (schema {version}; this build \
         understands up to {latest}). Update Shouci to open it."
    ))
}

fn apply(tx: &Transaction<'_>, step: usize, migration: &Migration) -> Result<()> {
    match migration {
        Migration::Sql(sql) => tx
            .execute_batch(sql)
            .map_err(|err| VocabError::storage(format!("schema migration {step} failed: {err}"))),
        Migration::Rust(apply) => apply(tx),
    }
}

pub(crate) fn run(conn: &mut Connection, migrations: &[Migration]) -> Result<()> {
    #[allow(clippy::cast_possible_wrap)]
    let latest = migrations.len() as i64;
    // Cheap check first: nothing to do (and nothing to lock) when current.
    let current: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if current == latest {
        return Ok(());
    }
    conn.pragma_update(None, "foreign_keys", "OFF")?;
    let result = run_locked(conn, migrations, latest);
    conn.pragma_update(None, "foreign_keys", "ON")?;
    result
}

fn run_locked(conn: &mut Connection, migrations: &[Migration], latest: i64) -> Result<()> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // Read inside the write lock: another process may have just migrated.
    let current: i64 = tx.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if current > latest {
        return Err(newer(current, latest));
    }
    let start = usize::try_from(current).unwrap_or(0);
    for (index, migration) in migrations.iter().enumerate().skip(start) {
        let step = index + 1;
        apply(&tx, step, migration)?;
        tx.pragma_update(None, "user_version", crate::to_i64(step)?)?;
    }
    let broken: Option<String> = tx
        .query_row("PRAGMA foreign_key_check", [], |row| row.get(0))
        .ok();
    if let Some(table) = broken {
        return Err(VocabError::storage(format!(
            "schema migration left broken references in {table}; nothing was changed"
        )));
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::{MIGRATIONS, Migration, SCHEMA_VERSION, check, migrate, run};

    #[test]
    fn fresh_database_reaches_the_latest_version_once() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        migrate(&mut conn).unwrap();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn newer_database_is_refused() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        let err = migrate(&mut conn).unwrap_err();
        assert!(err.to_string().contains("newer Shouci"), "{err}");
    }

    /// A `user.db` on disk made by `sql`, as `crate::open` would find it.
    fn file_with(name: &str, sql: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("shouci-check-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("user.db");
        Connection::open(&path).unwrap().execute_batch(sql).unwrap();
        path
    }

    fn query(path: &std::path::Path, sql: &str) -> String {
        Connection::open(path)
            .unwrap()
            .query_row(sql, [], |row| row.get::<_, rusqlite::types::Value>(0))
            .map(|value| format!("{value:?}"))
            .unwrap()
    }

    #[test]
    fn another_programs_database_is_left_as_it_is() {
        let path = file_with(
            "foreign",
            "CREATE TABLE vocabulary_items (id INTEGER PRIMARY KEY);",
        );
        let err = crate::open(&path).unwrap_err();
        assert!(err.to_string().contains("isn't a Shouci library"), "{err}");
        assert_eq!(
            query(&path, "PRAGMA journal_mode"),
            r#"Text("delete")"#,
            "not WAL"
        );
        assert_eq!(
            query(&path, "SELECT count(*) FROM sqlite_master"),
            "Integer(1)",
            "nothing added"
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_database_missing_what_its_version_has_is_refused() {
        // Another program's database that happens to say schema 1
        let path = file_with(
            "claims",
            "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT); PRAGMA user_version = 1;",
        );
        let err = crate::open(&path).unwrap_err();
        assert!(
            err.to_string()
                .contains("schema 1 but has no collection_items"),
            "{err}"
        );
        assert_eq!(
            query(&path, "PRAGMA user_version"),
            "Integer(1)",
            "not migrated"
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());

        // A library missing a column
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        conn.execute_batch("ALTER TABLE items DROP COLUMN notes;")
            .unwrap();
        let err = check(&mut conn).unwrap_err();
        assert!(err.to_string().contains("has no items.notes"), "{err}");
    }

    #[test]
    fn a_library_holding_more_than_its_version_needs_opens() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        conn.execute_batch(
            "CREATE TABLE added_by_hand (id INTEGER); ALTER TABLE items ADD COLUMN extra TEXT;",
        )
        .unwrap();
        check(&mut conn).unwrap();
    }

    /// The shape of a future migration that rebuilds `items` (to change a
    /// column): with foreign keys on, dropping the table would cascade into
    /// tags and collections.
    const REBUILD_ITEMS: &str = r"
CREATE TABLE items_new AS SELECT * FROM items;
DROP TABLE items;
ALTER TABLE items_new RENAME TO items;
";

    #[test]
    fn a_table_rebuild_keeps_tags_and_collections() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        migrate(&mut conn).unwrap();
        conn.execute_batch(
            "INSERT INTO items (simplified, traditional) VALUES ('一', '一');
             INSERT INTO tags (name) VALUES ('HSK1');
             INSERT INTO item_tags (item_id, tag_id) VALUES (1, 1);",
        )
        .unwrap();
        // Every migration, then the rebuild: one step past the latest.
        assert_eq!(MIGRATIONS.len(), 2, "list every migration here");
        let steps = [
            Migration::Sql(super::V1),
            Migration::Sql(super::V2),
            Migration::Sql(REBUILD_ITEMS),
        ];
        run(&mut conn, &steps).unwrap();
        let tagged: i64 = conn
            .query_row("SELECT count(*) FROM item_tags", [], |row| row.get(0))
            .unwrap();
        assert_eq!(tagged, 1);
        let foreign_keys: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert_eq!(foreign_keys, 1, "back on afterwards");
    }

    #[test]
    fn processes_opening_a_new_database_together_do_not_collide() {
        let dir = std::env::temp_dir().join(format!("shouci-race-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for round in 0..20 {
            let path = dir.join(format!("user-{round}.db"));
            let _ = std::fs::remove_file(&path);
            let handles: Vec<_> = (0..4)
                .map(|_| {
                    let path = path.clone();
                    std::thread::spawn(move || crate::open(&path).map(|_| ()))
                })
                .collect();
            for handle in handles {
                handle.join().unwrap().unwrap();
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
