pub(crate) const CREATE_USER_SCHEMA: &str = r"
CREATE TABLE IF NOT EXISTS vocabulary_items (
    item_id INTEGER PRIMARY KEY,
    simplified TEXT NOT NULL,
    traditional TEXT NOT NULL,
    pinyin TEXT NOT NULL,
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
    UNIQUE (simplified COLLATE NOCASE, traditional COLLATE NOCASE)
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
";
