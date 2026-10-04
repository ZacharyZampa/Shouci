//! `user.db`: saved words, their tags and collections, the transfer ledger,
//! and settings.
//!
//! Plain SQLite and plain functions over a [`rusqlite::Connection`]; a
//! [`rusqlite::Transaction`] works anywhere a connection does. The schema is
//! versioned with `PRAGMA user_version` (see `migrate.rs`). Nothing outside
//! `shouci-core` calls this crate.

mod items;
mod legacy;
mod migrate;
mod organize;
mod settings;
mod transfer;

pub use items::{
    NewItem, Saved, find_by_identity, get_item, insert_item, list_items, purge_item, reading_key,
    require_item, save_item, set_archived, set_source, set_trashed, set_verification, update_item,
};
pub use legacy::{IMPORTED_SETTING as LEGACY_IMPORTED_SETTING, LegacyReport, import_poc_database};
pub use migrate::SCHEMA_VERSION;
pub use organize::{
    NameCount, add_tag, add_to_collection, collection_from_tag, collections, collections_by_item,
    create_collection, delete_collection, delete_tag, item_collections, item_tags,
    remove_from_collection, remove_tag, rename_collection, rename_tag, set_collections, set_tags,
    tags, tags_by_item,
};
pub use settings::{get_setting, set_setting};
pub use transfer::{
    Direction, Outcome, RunCounts, RunRecord, RunStatus, destinations_by_item, import_sources,
    item_destinations, items_at, record_item, record_run,
};

use std::path::Path;
use std::time::Duration;

use rusqlite::Connection;
use vocab_core::{Result, VocabError};

/// SQL for "now" in the format every timestamp column uses:
/// `2026-09-30T12:00:00.000Z` (UTC, millisecond precision, sortable).
pub(crate) const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%fZ','now')";

/// How long a write waits for another process (the menu-bar app, the CLI)
/// to finish its own before giving up.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Opens `user.db`, creating it when missing, and brings the schema up to
/// date. WAL mode lets the app and the CLI read while the other writes.
///
/// # Errors
///
/// Returns an error if the file cannot be opened, is a database this build
/// cannot use, or a migration fails.
pub fn open(path: &Path) -> Result<Connection> {
    // Setting up a database (switching it to WAL, migrating) can report
    // "busy" at once, without waiting, when another process is setting it up
    // at the same moment. Those races last milliseconds, so retry briefly.
    let mut attempt = 0;
    loop {
        let conn = Connection::open(path)
            .map_err(|err| VocabError::storage(format!("cannot open {}: {err}", path.display())))?;
        match configure(conn) {
            Err(err) if err.kind() == vocab_core::ErrorKind::Unavailable && attempt < 50 => {
                attempt += 1;
                std::thread::sleep(Duration::from_millis(20));
            }
            result => return result,
        }
    }
}

/// A private in-memory `user.db`, for tests and previews.
///
/// # Errors
///
/// Returns an error if the schema cannot be applied.
pub fn open_in_memory() -> Result<Connection> {
    configure(Connection::open_in_memory()?)
}

fn configure(mut conn: Connection) -> Result<Connection> {
    conn.busy_timeout(BUSY_TIMEOUT)?;
    // Returns the resulting mode as a row, so it is queried, not updated.
    let _: String = conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    migrate::migrate(&mut conn)?;
    Ok(conn)
}

/// Runs `work` in one transaction: all of it lands, or none of it does.
///
/// The transaction takes the write lock up front (IMMEDIATE). A deferred
/// transaction that reads, then writes after another process committed,
/// fails at once without waiting; this one waits up to the busy timeout.
///
/// # Errors
///
/// Returns `work`'s error (after rolling back), or an error if the
/// transaction cannot begin or commit; a busy database is
/// [`vocab_core::ErrorKind::Unavailable`].
pub fn with_tx<T>(
    conn: &mut Connection,
    work: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T>,
) -> Result<T> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let value = work(&tx)?;
    tx.commit()?;
    Ok(value)
}

/// Changes whenever another connection commits to this database: another
/// process, or another connection in this one. A connection never sees its
/// own commits here, so poll on a different connection than the writer.
///
/// # Errors
///
/// Returns an error if the pragma cannot be read.
pub fn data_version(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("PRAGMA data_version", [], |row| row.get(0))?)
}

/// Converts a count for storage.
pub(crate) fn to_i64(count: usize) -> Result<i64> {
    i64::try_from(count).map_err(|_| VocabError::new("count too large to store"))
}

#[cfg(test)]
mod tests {
    use super::{data_version, open};

    #[test]
    fn another_connection_bumps_the_data_version() {
        let dir = std::env::temp_dir().join(format!("shouci-dv-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("user.db");
        let _ = std::fs::remove_file(&path);
        let watcher = open(&path).unwrap();
        let writer = open(&path).unwrap();
        let before = data_version(&watcher).unwrap();
        writer
            .execute("INSERT INTO settings (key, value) VALUES ('k', 'v')", [])
            .unwrap();
        assert_ne!(data_version(&watcher).unwrap(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
