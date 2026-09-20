//! `user.db` schema and connection management.
//!
//! Writable, local-only. All multi-statement operations are transactional (Phase 1
//! adds the import/export transaction wrappers); this module owns the schema
//! contract and opens the database with the invariant defaults (WAL, FK checking).

mod repo;
mod schema;
mod transfer;

pub use repo::{
    NewVocabItem, SaveOutcome, app_data_dir, delete_item, find_saved_item, get_item, list_items,
    save_vocab_item, set_status,
};
pub(crate) use schema::CREATE_USER_SCHEMA;
pub use transfer::{
    ExportRun, ImportDecision, ImportPayload, ImportRecord, ImportSummary, add_tag, commit_export,
    import_sources, item_tags, persist_import, select_exportable,
};

use std::path::Path;

use vocab_core::{Result, VocabError};

/// Opens `user.db` at `path`, applying the schema and the invariant defaults
/// (WAL, foreign keys).
///
/// # Errors
///
/// Returns an error if the database cannot be opened, the pragmas cannot be set,
/// or the schema cannot be applied.
pub fn open_user_db(path: &Path) -> Result<rusqlite::Connection> {
    let conn = rusqlite::Connection::open(path).map_err(|err| {
        VocabError::new(format!("failed to open user db {}: {err}", path.display()))
    })?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|err| VocabError::new(format!("failed to enable WAL: {err}")))?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|err| VocabError::new(format!("failed to enable foreign keys: {err}")))?;
    apply_schema(&conn)?;
    Ok(conn)
}

/// Applies or upgrades the `user.db` schema in place.
///
/// # Errors
///
/// Returns an error if any schema statement fails.
pub fn apply_schema(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute_batch(CREATE_USER_SCHEMA)
        .map_err(|err| VocabError::new(format!("failed to apply user schema: {err}")))
}

/// Runs `work` inside a single transaction. On error the transaction rolls back
/// and existing data is untouched.
///
/// # Errors
///
/// Returns the closure's error (rolled back) or an error if the transaction
/// cannot be begun or committed.
pub fn with_tx<T>(
    conn: &mut rusqlite::Connection,
    work: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T>,
) -> Result<T> {
    let tx = conn
        .transaction()
        .map_err(|err| VocabError::new(format!("failed to begin transaction: {err}")))?;
    let result = work(&tx);
    match result {
        Ok(value) => {
            tx.commit()
                .map_err(|err| VocabError::new(format!("failed to commit: {err}")))?;
            Ok(value)
        }
        Err(err) => {
            let _ = tx.rollback();
            Err(err)
        }
    }
}
