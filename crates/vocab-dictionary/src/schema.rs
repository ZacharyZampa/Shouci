pub const DICTIONARY_SCHEMA_VERSION: i32 = 1;

pub const DICTIONARY_BUILD_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Read-only dictionary database schema.
///
/// English glosses are indexed twice over independent FTS5 indexes:
/// - `english_fts_token` (unicode61): exact tokens, prefix (`tok*`), phrases,
/// - `english_fts_trigram`: arbitrary substring matches for partial phrase work.
///
/// Both are contentless; retrieval is deterministic in rowid order, with ranking
/// delegated to `vocab-search`.
pub const CREATE_DICTIONARY_SCHEMA: &str = r"
CREATE TABLE IF NOT EXISTS dictionary_metadata (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS dictionary_entries (
    entry_id INTEGER PRIMARY KEY,
    simplified TEXT NOT NULL,
    traditional TEXT NOT NULL,
    pinyin TEXT NOT NULL,
    pinyin_normalized TEXT NOT NULL,
    frequency_rank INTEGER,
    hsk_rank INTEGER,
    source_id TEXT NOT NULL,
    source_version TEXT NOT NULL,
    UNIQUE (simplified, traditional, pinyin, source_id, source_version)
);

CREATE TABLE IF NOT EXISTS dictionary_glosses (
    gloss_id INTEGER PRIMARY KEY,
    entry_id INTEGER NOT NULL REFERENCES dictionary_entries(entry_id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    gloss TEXT NOT NULL
);

CREATE VIRTUAL TABLE IF NOT EXISTS english_fts_token USING fts5(
    gloss,
    tokenize = 'unicode61',
    contentless_everything
);

CREATE VIRTUAL TABLE IF NOT EXISTS english_fts_trigram USING fts5(
    gloss,
    tokenize = 'trigram',
    contentless_everything
);

CREATE INDEX IF NOT EXISTS idx_entries_simplified ON dictionary_entries(simplified);
CREATE INDEX IF NOT EXISTS idx_entries_traditional ON dictionary_entries(traditional);
CREATE INDEX IF NOT EXISTS idx_entries_pinyin_norm ON dictionary_entries(pinyin_normalized);
CREATE INDEX IF NOT EXISTS idx_glosses_entry ON dictionary_glosses(entry_id, position);
";

pub const METADATA_SCHEMA_VERSION: &str = "schema_version";
pub const METADATA_SOURCE_ID: &str = "source_id";
pub const METADATA_SOURCE_VERSION: &str = "source_version";
pub const METADATA_LICENSE: &str = "license";
pub const METADATA_BUILD_VERSION: &str = "build_version";
pub const METADATA_ENTRY_COUNT: &str = "entry_count";
pub const METADATA_SOURCE_SHA256: &str = "source_sha256";
pub const METADATA_FREQUENCY_SOURCE_ID: &str = "frequency_source_id";
pub const METADATA_FREQUENCY_SOURCE_SHA256: &str = "frequency_source_sha256";
pub const METADATA_HSK_SOURCE_ID: &str = "hsk_source_id";
pub const METADATA_HSK_SOURCE_SHA256: &str = "hsk_source_sha256";
