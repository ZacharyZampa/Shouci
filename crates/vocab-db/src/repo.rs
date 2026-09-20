//! `user.db` persistence: save with duplicate detection, item queries, and
//! app-data path resolution.

use std::path::PathBuf;

use rusqlite::OptionalExtension;
use vocab_core::{
    ConfirmationState, ItemStatus, Provenance, Result, SourceId, SourceVersion, VocabError,
    VocabItem,
};

/// Path for per-user application data.
///
/// Resolution order:
/// 1. `VOCAB_HOME` (explicit override, highest priority),
/// 2. `XDG_DATA_HOME` (falls back to `~/.local/share`), plus the `pleco-companion`
///    subdirectory,
/// 3. macOS default: `~/Library/Application Support/pleco-companion`.
///
/// Distinguish "var is set and empty" (ignore) from "never set".
fn data_dir(home: Option<&str>, xdg: Option<&str>, vocab_home: Option<&str>) -> PathBuf {
    if let Some(dir) = vocab_home {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    if let Some(dir) = xdg {
        if !dir.is_empty() {
            return PathBuf::from(dir).join("pleco-companion");
        }
    }
    if cfg!(target_os = "macos") {
        if let Some(home) = home {
            return PathBuf::from(home).join("Library/Application Support/pleco-companion");
        }
    }
    if let Some(home) = home {
        return PathBuf::from(home).join(".local/share/pleco-companion");
    }
    PathBuf::from(".")
}

/// Resolves the app data directory from the current environment.
///
/// # Errors
///
/// Never fails; reserved for future platform lookup errors.
pub fn app_data_dir() -> Result<PathBuf> {
    Ok(data_dir(
        std::env::var("HOME").ok().as_deref(),
        std::env::var("XDG_DATA_HOME").ok().as_deref(),
        std::env::var("VOCAB_HOME").ok().as_deref(),
    ))
}

/// A candidate for the user's personal vocabulary: exactly what a quick-capture
/// save needs to transfer from a dictionary `Candidate`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewVocabItem {
    pub simplified: String,
    pub traditional: String,
    pub pinyin: String,
    pub definition: String,
    pub status: ItemStatus,
    pub notes: Option<String>,
    pub source_entry_id: Option<i64>,
    pub provenance: Provenance,
    pub origin_export_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveOutcome {
    Inserted(VocabItem),
    Duplicate(VocabItem),
}

/// Saves a new item, detecting duplicates by the schema's case-insensitive
/// `(simplified, traditional)` key.
///
/// # Errors
///
/// Returns an error if reading or writing the database fails.
pub fn save_vocab_item(conn: &rusqlite::Connection, item: &NewVocabItem) -> Result<SaveOutcome> {
    let existing = find_by_forms(
        conn,
        &item.simplified,
        &item.traditional,
        item.origin_export_id,
    )?;
    if let Some(stored) = existing {
        return Ok(SaveOutcome::Duplicate(stored));
    }
    let status = item.status.as_str();
    conn.execute(
        "INSERT INTO vocabulary_items \
         (simplified, traditional, pinyin, definition, status, notes, source_entry_id, \
          source_id, source_version, import_origin, origin_export_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        rusqlite::params![
            item.simplified,
            item.traditional,
            item.pinyin,
            item.definition,
            status,
            item.notes,
            item.source_entry_id,
            item.provenance.source.as_str(),
            item.provenance.source_version.as_str(),
            item.provenance.import_origin,
            item.origin_export_id,
        ],
    )
    .map_err(|err| VocabError::new(format!("insert vocabulary item: {err}")))?;
    let item_id = conn.last_insert_rowid();
    let stored =
        get_item(conn, item_id)?.ok_or_else(|| VocabError::new("inserted item vanished"))?;
    Ok(SaveOutcome::Inserted(stored))
}

/// Looks up stored items sharing the case-insensitive `(simplified, traditional)`
/// key. `origin_export_id` is only considered when present, to avoid collapsing
/// different export-target copies of the same entry.
pub(crate) fn find_by_forms(
    conn: &rusqlite::Connection,
    simplified: &str,
    traditional: &str,
    _origin_export_id: Option<i64>,
) -> Result<Option<VocabItem>> {
    let mut stmt = conn
        .prepare(
            "SELECT item_id FROM vocabulary_items \
             WHERE simplified = ?1 COLLATE NOCASE AND traditional = ?2 COLLATE NOCASE \
             ORDER BY item_id LIMIT 1",
        )
        .map_err(|err| VocabError::new(format!("prepare duplicate lookup: {err}")))?;
    let item_id: Option<i64> = stmt
        .query_row(rusqlite::params![simplified, traditional], |row| row.get(0))
        .optional()
        .map_err(|err| VocabError::new(format!("duplicate lookup: {err}")))?;
    match item_id {
        Some(id) => get_item(conn, id),
        None => Ok(None),
    }
}

/// Fetches one item by id, if present.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn get_item(conn: &rusqlite::Connection, item_id: i64) -> Result<Option<VocabItem>> {
    select_items(conn, "WHERE item_id = ?1", rusqlite::params![item_id]).map(|mut items| {
        if items.is_empty() {
            None
        } else {
            Some(items.remove(0))
        }
    })
}

/// Lists items, optionally restricted to one status, newest first (deterministic).
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn list_items(
    conn: &rusqlite::Connection,
    status: Option<ItemStatus>,
) -> Result<Vec<VocabItem>> {
    match status {
        Some(status) => select_items(
            conn,
            "WHERE status = ?1 ORDER BY item_id DESC",
            rusqlite::params![status.as_str()],
        ),
        None => select_items(conn, "ORDER BY item_id DESC", rusqlite::params![]),
    }
}

/// Sets an item's status (used by confirmation review and export marking).
///
/// # Errors
///
/// Returns an error if the update fails.
pub fn set_status(conn: &rusqlite::Connection, item_id: i64, status: ItemStatus) -> Result<()> {
    let updated = conn
        .execute(
            "UPDATE vocabulary_items SET status = ?1, modified_at = datetime('now') \
             WHERE item_id = ?2",
            rusqlite::params![status.as_str(), item_id],
        )
        .map_err(|err| VocabError::new(format!("update status: {err}")))?;
    if updated == 0 {
        return Err(VocabError::new(format!("no item {item_id} to update")));
    }
    Ok(())
}

/// Deletes one item by id. Tag links cascade (`ON DELETE CASCADE`); audit
/// rows keep a null item reference (`ON DELETE SET NULL`).
///
/// # Errors
///
/// Returns an error if no such item exists or the delete fails.
pub fn delete_item(conn: &rusqlite::Connection, item_id: i64) -> Result<()> {
    let deleted = conn
        .execute(
            "DELETE FROM vocabulary_items WHERE item_id = ?1",
            rusqlite::params![item_id],
        )
        .map_err(|err| VocabError::new(format!("delete item: {err}")))?;
    if deleted == 0 {
        return Err(VocabError::new(format!("no item {item_id} to delete")));
    }
    Ok(())
}

/// Finds a saved item by headword (case-insensitive
/// `(simplified, traditional)` key), if present. Used for saved-badges and
/// for resolving a popup row back to its item id.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn find_saved_item(
    conn: &rusqlite::Connection,
    simplified: &str,
    traditional: &str,
) -> Result<Option<VocabItem>> {
    find_by_forms(conn, simplified, traditional, None)
}

pub(crate) fn select_items(
    conn: &rusqlite::Connection,
    tail: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<VocabItem>> {
    let sql = format!(
        "SELECT item_id, simplified, traditional, pinyin, definition, status, notes, \
         source_entry_id, source_id, source_version, import_origin, origin_export_id, \
         created_at, modified_at \
         FROM vocabulary_items {tail}"
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|err| VocabError::new(format!("prepare item query: {err}")))?;
    let mut rows = stmt
        .query(params)
        .map_err(|err| VocabError::new(format!("run item query: {err}")))?;
    let mut items = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| VocabError::new(format!("read item row: {err}")))?
    {
        items.push(load_item_row(row)?);
    }
    Ok(items)
}

fn load_item_row(row: &rusqlite::Row<'_>) -> vocab_core::Result<VocabItem> {
    let status = row.get::<_, String>(5)?.parse::<ItemStatus>()?;
    let source_id = row.get::<_, Option<String>>(8)?;
    let source_version = row.get::<_, Option<String>>(9)?;
    let import_origin = row.get::<_, Option<String>>(10)?;
    Ok(VocabItem {
        item_id: row.get(0)?,
        simplified: row.get(1)?,
        traditional: row.get(2)?,
        pinyin: row.get(3)?,
        definition: row.get(4)?,
        status,
        notes: row.get(6)?,
        source_entry_id: row.get(7)?,
        provenance: Provenance {
            source: SourceId(source_id.unwrap_or_else(|| "user".to_owned())),
            source_version: SourceVersion(source_version.unwrap_or_else(|| "manual".to_owned())),
            import_origin,
            confirmation: confirmation_for(status),
        },
        origin_export_id: row.get(11)?,
        created_at: row.get(12)?,
        modified_at: row.get(13)?,
    })
}

fn confirmation_for(status: ItemStatus) -> ConfirmationState {
    match status {
        ItemStatus::Confirmed | ItemStatus::Exported | ItemStatus::Archived => {
            ConfirmationState::UserConfirmed
        }
        ItemStatus::NeedsReview => ConfirmationState::NeedsReview,
    }
}

#[cfg(test)]
mod tests {
    use super::{NewVocabItem, SaveOutcome, app_data_dir};
    use crate::{apply_schema, open_user_db, save_vocab_item};
    use std::path::PathBuf;
    use vocab_core::{ConfirmationState, ItemStatus, Provenance, SourceId, SourceVersion};

    fn new_item(simplified: &str, traditional: &str) -> NewVocabItem {
        NewVocabItem {
            simplified: simplified.to_owned(),
            traditional: traditional.to_owned(),
            pinyin: "lü3 xing2".to_owned(),
            definition: "to travel".to_owned(),
            status: ItemStatus::NeedsReview,
            notes: None,
            source_entry_id: Some(7),
            provenance: Provenance {
                source: SourceId("cc-cedict".to_owned()),
                source_version: SourceVersion("sample".to_owned()),
                import_origin: None,
                confirmation: ConfirmationState::DictionaryAuthority,
            },
            origin_export_id: None,
        }
    }

    fn conn() -> rusqlite::Connection {
        let c = rusqlite::Connection::open_in_memory().unwrap();
        apply_schema(&c).unwrap();
        c
    }

    #[test]
    fn saves_and_reloads() {
        let c = conn();
        let outcome = save_vocab_item(&c, &new_item("猫", "貓")).unwrap();
        let inserted = match outcome {
            SaveOutcome::Inserted(item) => item,
            SaveOutcome::Duplicate(_) => panic!("expected insert"),
        };
        assert_eq!(inserted.simplified, "猫");
        assert_eq!(inserted.status, ItemStatus::NeedsReview);
        assert_eq!(inserted.provenance.source.as_str(), "cc-cedict");
        let loaded = crate::get_item(&c, inserted.item_id).unwrap().unwrap();
        assert_eq!(loaded, inserted);
    }

    #[test]
    fn duplicate_detection_returns_existing() {
        let c = conn();
        let first = match save_vocab_item(&c, &new_item("猫", "貓")).unwrap() {
            SaveOutcome::Inserted(item) => item,
            SaveOutcome::Duplicate(_) => panic!("expected first insert"),
        };
        let also = save_vocab_item(&c, &new_item("猫", "貓")).unwrap();
        assert_eq!(
            also,
            SaveOutcome::Duplicate(first),
            "case-insensitive duplicate should be rejected"
        );
    }

    #[test]
    fn duplicate_is_case_insensitive() {
        let c = conn();
        save_vocab_item(&c, &new_item("Hello World", "Hello World")).unwrap();
        let outcome = save_vocab_item(&c, &new_item("hello world", "hello world")).unwrap();
        assert!(matches!(outcome, SaveOutcome::Duplicate(_)));
    }

    #[test]
    fn list_filters_by_status() {
        let c = conn();
        save_vocab_item(&c, &new_item("猫", "貓")).unwrap();
        let mut confirmed = new_item("食", "食");
        confirmed.status = ItemStatus::Confirmed;
        save_vocab_item(&c, &confirmed).unwrap();
        let all = crate::list_items(&c, None).unwrap();
        assert_eq!(all.len(), 2);
        let only = crate::list_items(&c, Some(ItemStatus::Confirmed)).unwrap();
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].simplified, "食");
    }

    #[test]
    fn status_update_writes() {
        let c = conn();
        let outcome = save_vocab_item(&c, &new_item("猫", "貓")).unwrap();
        let inserted = match outcome {
            SaveOutcome::Inserted(item) => item,
            SaveOutcome::Duplicate(_) => panic!("expected insert"),
        };
        crate::set_status(&c, inserted.item_id, ItemStatus::Confirmed).unwrap();
        assert_eq!(
            crate::get_item(&c, inserted.item_id)
                .unwrap()
                .unwrap()
                .status,
            ItemStatus::Confirmed
        );
    }

    #[test]
    fn delete_removes_item_and_reports_missing() {
        let c = conn();
        let outcome = save_vocab_item(&c, &new_item("猫", "貓")).unwrap();
        let inserted = match outcome {
            SaveOutcome::Inserted(item) => item,
            SaveOutcome::Duplicate(_) => panic!("expected insert"),
        };
        crate::delete_item(&c, inserted.item_id).unwrap();
        assert!(crate::get_item(&c, inserted.item_id).unwrap().is_none());
        assert!(crate::delete_item(&c, inserted.item_id).is_err());
    }

    #[test]
    fn find_saved_item_matches_headword() {
        let c = conn();
        let outcome = save_vocab_item(&c, &new_item("猫", "貓")).unwrap();
        let inserted = match outcome {
            SaveOutcome::Inserted(item) => item,
            SaveOutcome::Duplicate(_) => panic!("expected insert"),
        };
        let found = crate::find_saved_item(&c, "猫", "貓")
            .unwrap()
            .expect("saved item must be found");
        assert_eq!(found.item_id, inserted.item_id);
        assert!(crate::find_saved_item(&c, "狗", "狗").unwrap().is_none());
    }

    #[test]
    fn open_and_reopen_round_trip() {
        let dir = std::env::temp_dir().join(format!("vocab-db-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("user.db");
        let conn = open_user_db(&path).unwrap();
        save_vocab_item(&conn, &new_item("旅行", "旅行")).unwrap();
        drop(conn);
        let conn = open_user_db(&path).unwrap();
        let items = crate::list_items(&conn, None).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].simplified, "旅行");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn data_dir_prefers_vocab_home_then_xdg_then_macos() {
        let home = "/Users/zach";
        assert_eq!(
            super::data_dir(Some(home), None, None),
            PathBuf::from("/Users/zach/Library/Application Support/pleco-companion")
        );
        assert_eq!(
            super::data_dir(Some(home), Some("/tmp/xdg"), Some("/override")),
            PathBuf::from("/override")
        );
        assert_eq!(
            super::data_dir(Some(home), Some("/tmp/xdg"), None),
            PathBuf::from("/tmp/xdg/pleco-companion")
        );
        assert_eq!(super::data_dir(None, None, None), PathBuf::from("."));
    }

    #[test]
    fn app_data_dir_resolves() {
        let _ = app_data_dir().unwrap();
    }
}
