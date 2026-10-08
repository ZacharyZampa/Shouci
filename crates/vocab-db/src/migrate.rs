//! Schema versions, tracked in `PRAGMA user_version`.
//!
//! Each entry of [`MIGRATIONS`] moves the schema one version forward. To
//! change the schema, append a migration; never edit one that has shipped.
//! A migration may be SQL or Rust (for data that must be recomputed, such as
//! reading keys after a pinyin rule changes).
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
        return Err(VocabError::storage(format!(
            "this user.db was written by a newer Shouci (schema {current}; this build \
             understands up to {latest}). Update Shouci to open it."
        )));
    }
    if current == 0 && is_poc_database(&tx)? {
        return Err(VocabError::invalid(
            "this is a Shouci proof-of-concept database. Open a new user.db and bring \
             these words in with `shouci migrate-poc`.",
        ));
    }
    let start = usize::try_from(current).unwrap_or(0);
    for (index, migration) in migrations.iter().enumerate().skip(start) {
        let step = index + 1;
        match migration {
            Migration::Sql(sql) => tx.execute_batch(sql).map_err(|err| {
                VocabError::storage(format!("schema migration {step} failed: {err}"))
            })?,
            Migration::Rust(apply) => apply(&tx)?,
        }
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

fn is_poc_database(conn: &Connection) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'vocabulary_items'",
        [],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::{MIGRATIONS, Migration, SCHEMA_VERSION, migrate, run};

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

    #[test]
    fn poc_database_is_refused_with_a_way_forward() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE vocabulary_items (item_id INTEGER PRIMARY KEY);")
            .unwrap();
        let err = migrate(&mut conn).unwrap_err();
        assert!(err.to_string().contains("migrate-poc"), "{err}");
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
