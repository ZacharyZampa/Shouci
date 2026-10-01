//! Schema versions, tracked in `PRAGMA user_version`.
//!
//! Each entry of [`MIGRATIONS`] moves the schema one version forward and runs
//! in its own transaction. To change the schema, append a migration; never
//! edit one that has shipped.

use rusqlite::Connection;
use vocab_core::{Result, VocabError};

const V1: &str = r"
CREATE TABLE items (
    id INTEGER PRIMARY KEY,
    simplified TEXT NOT NULL,
    traditional TEXT NOT NULL,
    pinyin TEXT NOT NULL DEFAULT '',
    reading_key TEXT NOT NULL DEFAULT '',
    definition TEXT NOT NULL DEFAULT '',
    notes TEXT NOT NULL DEFAULT '',
    verification TEXT NOT NULL DEFAULT 'confirmed'
        CHECK (verification IN ('confirmed', 'needs_review')),
    archived_at TEXT,
    deleted_at TEXT,
    source_kind TEXT NOT NULL DEFAULT 'manual'
        CHECK (source_kind IN ('dictionary', 'import', 'manual')),
    source_id TEXT,
    source_version TEXT,
    import_origin TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    modified_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
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
    id INTEGER PRIMARY KEY,
    direction TEXT NOT NULL CHECK (direction IN ('in', 'out')),
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
    status TEXT NOT NULL CHECK (status IN ('committed', 'rejected')),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- What happened to each word (or line) in a run.
CREATE TABLE transfer_items (
    id INTEGER PRIMARY KEY,
    run_id INTEGER NOT NULL REFERENCES transfer_runs(id) ON DELETE CASCADE,
    item_id INTEGER REFERENCES items(id) ON DELETE SET NULL,
    line INTEGER,
    raw TEXT,
    outcome TEXT NOT NULL
        CHECK (outcome IN ('inserted', 'updated', 'skipped', 'dropped', 'rejected', 'written')),
    detail TEXT NOT NULL DEFAULT ''
);

CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE INDEX idx_items_simplified ON items(simplified);
CREATE INDEX idx_item_tags_tag ON item_tags(tag_id);
CREATE INDEX idx_collection_items_item ON collection_items(item_id);
CREATE INDEX idx_transfer_items_item ON transfer_items(item_id);
CREATE INDEX idx_transfer_items_run ON transfer_items(run_id);
";

const MIGRATIONS: &[&str] = &[V1];

/// The schema version this build writes.
#[allow(clippy::cast_possible_wrap)] // a handful of migrations
pub const SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64;

pub(crate) fn migrate(conn: &mut Connection) -> Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if current > SCHEMA_VERSION {
        return Err(VocabError::storage(format!(
            "this user.db was written by a newer Shouci (schema {current}; this build \
             understands up to {SCHEMA_VERSION}). Update Shouci to open it."
        )));
    }
    if current == 0 && is_poc_database(conn)? {
        return Err(VocabError::invalid(
            "this is a Shouci proof-of-concept database. Open a new user.db and bring \
             these words in with `shouci migrate-poc`.",
        ));
    }
    let start = usize::try_from(current).unwrap_or(0);
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(start) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql).map_err(|err| {
            VocabError::storage(format!("schema migration {} failed: {err}", index + 1))
        })?;
        tx.pragma_update(None, "user_version", crate::to_i64(index + 1)?)?;
        tx.commit()?;
    }
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

    use super::{SCHEMA_VERSION, migrate};

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
}
