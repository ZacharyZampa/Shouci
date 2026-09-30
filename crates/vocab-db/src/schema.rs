pub(crate) const CREATE_USER_SCHEMA: &str = r"
CREATE TABLE IF NOT EXISTS vocabulary_items (
    item_id INTEGER PRIMARY KEY,
    simplified TEXT NOT NULL,
    traditional TEXT NOT NULL,
    pinyin TEXT NOT NULL,
    pinyin_key TEXT NOT NULL DEFAULT '',
    definition TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'confirmed'
        CHECK (status IN ('confirmed','needs_review','exported','archived')),
    notes TEXT,
    source_entry_id INTEGER,
    source_id TEXT,
    source_version TEXT,
    import_origin TEXT,
    origin_export_id INTEGER,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    modified_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE (simplified COLLATE NOCASE, traditional COLLATE NOCASE, pinyin_key)
);

CREATE TABLE IF NOT EXISTS tags (
    tag_id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE COLLATE NOCASE,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS vocabulary_tags (
    item_id INTEGER NOT NULL REFERENCES vocabulary_items(item_id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES tags(tag_id) ON DELETE CASCADE,
    PRIMARY KEY (item_id, tag_id)
);

CREATE TABLE IF NOT EXISTS categories (
    category_id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE COLLATE NOCASE,
    pleco_export_name TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS import_runs (
    run_id INTEGER PRIMARY KEY,
    source_path TEXT NOT NULL,
    codec_key TEXT NOT NULL,
    content_sha256 TEXT NOT NULL,
    records_seen INTEGER NOT NULL,
    records_imported INTEGER NOT NULL,
    records_skipped_duplicate INTEGER NOT NULL,
    issues_errors INTEGER NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('staged','committed','rejected')),
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS export_runs (
    run_id INTEGER PRIMARY KEY,
    target_path TEXT NOT NULL,
    codec_key TEXT NOT NULL,
    records_written INTEGER NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('staged','committed','rejected')),
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS transfer_runs (
    run_id INTEGER PRIMARY KEY,
    direction TEXT NOT NULL CHECK (direction IN ('in','out')),
    target TEXT NOT NULL,
    codec_key TEXT NOT NULL,
    profile TEXT NOT NULL,
    source_path TEXT NOT NULL,
    content_sha256 TEXT,
    records_seen INTEGER NOT NULL,
    records_inserted INTEGER NOT NULL,
    records_skipped_duplicate INTEGER NOT NULL,
    records_unresolved INTEGER NOT NULL,
    records_dropped INTEGER NOT NULL,
    issues_errors INTEGER NOT NULL,
    issues_warnings INTEGER NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('committed','rejected')),
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS transfer_items (
    transfer_item_id INTEGER PRIMARY KEY,
    run_id INTEGER NOT NULL REFERENCES transfer_runs(run_id) ON DELETE CASCADE,
    item_id INTEGER REFERENCES vocabulary_items(item_id) ON DELETE SET NULL,
    source_line INTEGER,
    raw_line TEXT,
    outcome TEXT NOT NULL,
    detail TEXT
);

CREATE TABLE IF NOT EXISTS user_pinyin_overrides (
    char TEXT PRIMARY KEY,
    pinyin TEXT NOT NULL,
    authority TEXT NOT NULL DEFAULT 'user_override',
    modified_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS audit_events (
    event_id INTEGER PRIMARY KEY,
    item_id INTEGER REFERENCES vocabulary_items(item_id) ON DELETE SET NULL,
    event_type TEXT NOT NULL,
    detail TEXT,
    at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_vocabulary_status ON vocabulary_items(status);
CREATE INDEX IF NOT EXISTS idx_vocabulary_simplified ON vocabulary_items(simplified COLLATE NOCASE);
CREATE INDEX IF NOT EXISTS idx_vocabulary_pinyin ON vocabulary_items(pinyin);
CREATE INDEX IF NOT EXISTS idx_import_runs_sha ON import_runs(content_sha256);
CREATE INDEX IF NOT EXISTS idx_audit_item ON audit_events(item_id);
CREATE INDEX IF NOT EXISTS idx_transfer_items_item ON transfer_items(item_id);
CREATE INDEX IF NOT EXISTS idx_transfer_items_run ON transfer_items(run_id);
";

/// Rebuilds `vocabulary_items` into the current shape. `{pinyin_key}` is the
/// source expression for the new column: `''` when upgrading from v1 (the
/// keys are backfilled afterwards), `pinyin_key` when repairing a table that
/// already has them.
const REBUILD_VOCABULARY: &str = r"
CREATE TABLE vocabulary_items_v2 (
    item_id INTEGER PRIMARY KEY,
    simplified TEXT NOT NULL,
    traditional TEXT NOT NULL,
    pinyin TEXT NOT NULL,
    pinyin_key TEXT NOT NULL DEFAULT '',
    definition TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'confirmed'
        CHECK (status IN ('confirmed','needs_review','exported','archived')),
    notes TEXT,
    source_entry_id INTEGER,
    source_id TEXT,
    source_version TEXT,
    import_origin TEXT,
    origin_export_id INTEGER,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    modified_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE (simplified COLLATE NOCASE, traditional COLLATE NOCASE, pinyin_key)
);
INSERT INTO vocabulary_items_v2 (
    item_id, simplified, traditional, pinyin, pinyin_key, definition, status, notes,
    source_entry_id, source_id, source_version, import_origin, origin_export_id,
    created_at, modified_at
)
SELECT
    item_id, simplified, traditional, pinyin, {pinyin_key}, definition, status, notes,
    source_entry_id, source_id, source_version, import_origin, origin_export_id,
    created_at, modified_at
FROM vocabulary_items;
DROP TABLE vocabulary_items;
ALTER TABLE vocabulary_items_v2 RENAME TO vocabulary_items;
CREATE INDEX IF NOT EXISTS idx_vocabulary_status ON vocabulary_items(status);
CREATE INDEX IF NOT EXISTS idx_vocabulary_simplified ON vocabulary_items(simplified COLLATE NOCASE);
CREATE INDEX IF NOT EXISTS idx_vocabulary_pinyin ON vocabulary_items(pinyin);
";

pub(crate) fn migrate_vocabulary_identity(conn: &rusqlite::Connection) -> vocab_core::Result<()> {
    let has_key = has_column(conn, "pinyin_key")?;
    // An earlier build of this migration dropped the timestamp defaults, so
    // every insert (which relies on them) failed with a NOT NULL error on
    // databases it had migrated. Those get rebuilt again, keeping their keys.
    if has_key && column_default(conn, "created_at")?.is_some() {
        return Ok(());
    }
    let foreign_keys: i64 = conn
        .pragma_query_value(None, "foreign_keys", |row| row.get(0))
        .unwrap_or(0);
    conn.pragma_update(None, "foreign_keys", "OFF")
        .map_err(|err| vocab_core::VocabError::new(format!("disable foreign keys: {err}")))?;
    let result = rebuild_vocabulary(conn, !has_key);
    if foreign_keys != 0 {
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|err| vocab_core::VocabError::new(format!("enable foreign keys: {err}")))?;
    }
    result
}

fn rebuild_vocabulary(conn: &rusqlite::Connection, backfill_keys: bool) -> vocab_core::Result<()> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|err| vocab_core::VocabError::new(format!("begin identity migration: {err}")))?;
    let key_source = if backfill_keys { "''" } else { "pinyin_key" };
    tx.execute_batch(&REBUILD_VOCABULARY.replace("{pinyin_key}", key_source))
        .map_err(|err| vocab_core::VocabError::new(format!("rebuild vocabulary_items: {err}")))?;
    let rows = if backfill_keys {
        pinyin_rows(&tx)?
    } else {
        Vec::new()
    };
    for (item_id, pinyin) in rows {
        let key = crate::repo::pinyin_key(&pinyin);
        tx.execute(
            "UPDATE vocabulary_items SET pinyin_key = ?1 WHERE item_id = ?2",
            rusqlite::params![key, item_id],
        )
        .map_err(|err| vocab_core::VocabError::new(format!("backfill pinyin_key: {err}")))?;
    }
    {
        let mut check = tx.prepare("PRAGMA foreign_key_check").map_err(|err| {
            vocab_core::VocabError::new(format!("prepare foreign key check: {err}"))
        })?;
        let mut violations = check
            .query([])
            .map_err(|err| vocab_core::VocabError::new(format!("run foreign key check: {err}")))?;
        if violations
            .next()
            .map_err(|err| vocab_core::VocabError::new(format!("read foreign key check: {err}")))?
            .is_some()
        {
            return Err(vocab_core::VocabError::new(
                "vocabulary identity migration failed foreign key check",
            ));
        }
    }
    tx.commit()
        .map_err(|err| vocab_core::VocabError::new(format!("commit identity migration: {err}")))?;
    Ok(())
}

fn pinyin_rows(conn: &rusqlite::Connection) -> vocab_core::Result<Vec<(i64, String)>> {
    let mut stmt = conn
        .prepare("SELECT item_id, pinyin FROM vocabulary_items")
        .map_err(|err| vocab_core::VocabError::new(format!("prepare pinyin backfill: {err}")))?;
    let mut rows = stmt
        .query([])
        .map_err(|err| vocab_core::VocabError::new(format!("run pinyin backfill: {err}")))?;
    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| vocab_core::VocabError::new(format!("read pinyin backfill: {err}")))?
    {
        out.push((row.get(0)?, row.get(1)?));
    }
    Ok(out)
}

/// The declared `DEFAULT` of a `vocabulary_items` column, if it has one.
fn column_default(conn: &rusqlite::Connection, column: &str) -> vocab_core::Result<Option<String>> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(vocabulary_items)")
        .map_err(|err| vocab_core::VocabError::new(format!("prepare table info: {err}")))?;
    let mut rows = stmt
        .query([])
        .map_err(|err| vocab_core::VocabError::new(format!("run table info: {err}")))?;
    while let Some(row) = rows
        .next()
        .map_err(|err| vocab_core::VocabError::new(format!("read table info: {err}")))?
    {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(row.get(4)?);
        }
    }
    Ok(None)
}

fn has_column(conn: &rusqlite::Connection, column: &str) -> vocab_core::Result<bool> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(vocabulary_items)")
        .map_err(|err| vocab_core::VocabError::new(format!("prepare table info: {err}")))?;
    let mut rows = stmt
        .query([])
        .map_err(|err| vocab_core::VocabError::new(format!("run table info: {err}")))?;
    while let Some(row) = rows
        .next()
        .map_err(|err| vocab_core::VocabError::new(format!("read table info: {err}")))?
    {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}
