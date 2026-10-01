//! Saved words.
//!
//! A word's identity is its characters plus its reading:
//! `(simplified, traditional, reading_key)`, case-insensitive. 行 `xing2`
//! and 行 `hang2` are two words; 学校 typed with tone marks or tone numbers
//! is one. Trashed words keep their identity, so saving one again brings it
//! back instead of making a copy.
//!
//! Two lenient matches keep partial information from making duplicates:
//! - a word saved without its traditional form matches the one saved word
//!   with the same simplified form and reading;
//! - saving a word with a reading completes a saved placeholder with the
//!   same characters and no reading (a capture made with no dictionary).

use rusqlite::{Connection, OptionalExtension, Row, params};
use vocab_core::{
    ItemPatch, ItemSource, LibraryFilter, LibraryView, Result, SourceKind, Verification,
    VocabError, VocabItem,
};

use crate::NOW;

const COLUMNS: &str = "id, simplified, traditional, pinyin, definition, notes, verification, \
     archived_at, deleted_at, source_kind, source_id, source_version, import_origin, \
     created_at, modified_at, rev";

/// SQL that marks a word changed.
const CHANGED: &str = "modified_at = strftime('%Y-%m-%dT%H:%M:%fZ','now'), rev = rev + 1";

/// A word about to be saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewItem {
    pub simplified: String,
    /// Empty means "same as simplified".
    pub traditional: String,
    pub pinyin: String,
    pub definition: String,
    pub notes: String,
    pub verification: Verification,
    pub source: ItemSource,
}

/// What [`save_item`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Saved {
    Inserted(VocabItem),
    /// Already saved; nothing changed.
    Existing(VocabItem),
    /// It was in the trash and is back.
    Restored(VocabItem),
    /// A placeholder saved without a reading was filled in.
    Completed(VocabItem),
}

impl Saved {
    #[must_use]
    pub fn item(&self) -> &VocabItem {
        match self {
            Self::Inserted(item)
            | Self::Existing(item)
            | Self::Restored(item)
            | Self::Completed(item) => item,
        }
    }
}

/// The reading part of a word's identity, so that every way of writing one
/// reading gives one key:
///
/// - tone marks or numbers, spaced or not: `xué xiào`, `xuéxiào`,
///   `xue2 xiao4`;
/// - the neutral tone written as `5` or not at all: `ma1 ma5`, `māma`;
/// - `ü` as `u:`, `v`, or (after `l` and `n`, before `e`) plain `u`:
///   `lu:e4`, `lve4`, `lue4`;
/// - apostrophes, capitals, and spacing ignored: `Xi1'an1`, `xi1 an1`.
///
/// The characters are part of the identity too, so this cannot merge two
/// different words. Empty when there is no reading.
#[must_use]
pub fn reading_key(pinyin: &str) -> String {
    let numbered = vocab_pinyin::numbered(pinyin);
    if numbered.is_empty() {
        return String::new();
    }
    vocab_pinyin::normalize(&numbered)
        .as_str()
        .split_whitespace()
        .map(|syllable| {
            let syllable: String = syllable
                .chars()
                .filter(|&c| c != '\'' && c != '5')
                .collect();
            match syllable.get(..3) {
                Some("lue" | "nue") => format!("{}ve{}", &syllable[..1], &syllable[3..]),
                _ => syllable,
            }
        })
        .collect()
}

/// Pinyin as stored: tone marks become numbers (`lǚxíng` → `lv3 xing2`),
/// whitespace is collapsed, and anything else is kept as written.
fn stored_pinyin(pinyin: &str) -> String {
    vocab_pinyin::numbered(pinyin)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn clean_forms(simplified: &str, traditional: &str) -> Result<(String, String)> {
    let simplified = simplified.trim();
    if simplified.is_empty() {
        return Err(VocabError::invalid("a word needs its characters"));
    }
    let traditional = traditional.trim();
    let traditional = if traditional.is_empty() {
        simplified
    } else {
        traditional
    };
    Ok((simplified.to_owned(), traditional.to_owned()))
}

/// Saves a word unless it is already saved. A trashed copy is restored; a
/// placeholder without a reading is completed.
///
/// # Errors
///
/// Returns an error if the word has no characters or the write fails.
pub fn save_item(conn: &Connection, item: &NewItem) -> Result<Saved> {
    match find_by_identity(conn, &item.simplified, &item.traditional, &item.pinyin)? {
        Some(existing) if existing.deleted_at.is_some() => {
            set_trashed(conn, existing.id, false)?;
            Ok(Saved::Restored(require_item(conn, existing.id)?))
        }
        Some(existing) => Ok(Saved::Existing(existing)),
        None => {
            if let Some(placeholder) = find_placeholder(conn, item)? {
                complete(conn, placeholder.id, item)?;
                return Ok(Saved::Completed(require_item(conn, placeholder.id)?));
            }
            let id = insert_item(conn, item)?;
            Ok(Saved::Inserted(require_item(conn, id)?))
        }
    }
}

/// A saved word with the same characters and no reading, which `item` (with
/// a reading) can complete.
fn find_placeholder(conn: &Connection, item: &NewItem) -> Result<Option<VocabItem>> {
    if reading_key(&item.pinyin).is_empty() {
        return Ok(None);
    }
    let (simplified, traditional) = clean_forms(&item.simplified, &item.traditional)?;
    Ok(conn
        .query_row(
            &format!(
                "SELECT {COLUMNS} FROM items WHERE simplified = ?1 COLLATE NOCASE \
                 AND reading_key = '' \
                 AND (traditional = ?2 COLLATE NOCASE OR traditional = simplified) \
                 ORDER BY id LIMIT 1"
            ),
            params![simplified, traditional],
            item_from_row,
        )
        .optional()?)
}

fn complete(conn: &Connection, id: i64, item: &NewItem) -> Result<()> {
    let (simplified, traditional) = clean_forms(&item.simplified, &item.traditional)?;
    let pinyin = stored_pinyin(&item.pinyin);
    conn.execute(
        &format!(
            "UPDATE items SET traditional = ?1, pinyin = ?2, reading_key = ?3, \
             definition = CASE WHEN definition = '' THEN ?4 ELSE definition END, \
             notes = CASE WHEN notes = '' THEN ?5 ELSE notes END, verification = ?6, \
             source_kind = ?7, source_id = ?8, source_version = ?9, import_origin = ?10, \
             deleted_at = NULL, {CHANGED} WHERE id = ?11"
        ),
        params![
            traditional,
            pinyin,
            reading_key(&pinyin),
            item.definition.trim(),
            item.notes.trim(),
            item.verification.as_str(),
            item.source.kind.as_str(),
            item.source.id,
            item.source.version,
            item.source.import_origin,
            id,
        ],
    )
    .map_err(|err| identity_error(err, &simplified, &pinyin))?;
    Ok(())
}

/// Inserts a word that is known not to exist yet.
///
/// # Errors
///
/// [`vocab_core::ErrorKind::Conflict`] if it does exist; other errors if the
/// word has no characters or the write fails.
pub fn insert_item(conn: &Connection, item: &NewItem) -> Result<i64> {
    let (simplified, traditional) = clean_forms(&item.simplified, &item.traditional)?;
    let pinyin = stored_pinyin(&item.pinyin);
    let pinyin = pinyin.as_str();
    conn.execute(
        "INSERT INTO items (simplified, traditional, pinyin, reading_key, definition, notes, \
         verification, source_kind, source_id, source_version, import_origin) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            simplified,
            traditional,
            pinyin,
            reading_key(pinyin),
            item.definition.trim(),
            item.notes.trim(),
            item.verification.as_str(),
            item.source.kind.as_str(),
            item.source.id,
            item.source.version,
            item.source.import_origin,
        ],
    )
    .map_err(|err| identity_error(err, &simplified, pinyin))?;
    Ok(conn.last_insert_rowid())
}

fn identity_error(err: rusqlite::Error, simplified: &str, pinyin: &str) -> VocabError {
    match &err {
        rusqlite::Error::SqliteFailure(failure, _)
            if failure.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE =>
        {
            let reading = if pinyin.is_empty() {
                String::new()
            } else {
                format!(" [{pinyin}]")
            };
            VocabError::conflict(format!("{simplified}{reading} is already saved"))
        }
        _ => err.into(),
    }
}

/// # Errors
///
/// Returns an error if the query fails.
pub fn get_item(conn: &Connection, id: i64) -> Result<Option<VocabItem>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM items WHERE id = ?1"),
            [id],
            item_from_row,
        )
        .optional()?)
}

/// # Errors
///
/// [`vocab_core::ErrorKind::NotFound`] if there is no such item.
pub fn require_item(conn: &Connection, id: i64) -> Result<VocabItem> {
    get_item(conn, id)?.ok_or_else(|| VocabError::not_found(format!("no saved word {id}")))
}

/// The saved word with these characters and this reading, trashed or not.
///
/// A blank `traditional` means "unknown": the word saved with the same
/// simplified form (as its traditional form too) matches first, then the one
/// saved word with this simplified form and reading, whatever its
/// traditional form. Several such words match none.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn find_by_identity(
    conn: &Connection,
    simplified: &str,
    traditional: &str,
    pinyin: &str,
) -> Result<Option<VocabItem>> {
    let Ok((simplified, full_traditional)) = clean_forms(simplified, traditional) else {
        return Ok(None);
    };
    let key = reading_key(pinyin);
    let exact = conn
        .query_row(
            &format!(
                "SELECT {COLUMNS} FROM items WHERE simplified = ?1 COLLATE NOCASE \
                 AND traditional = ?2 COLLATE NOCASE AND reading_key = ?3"
            ),
            params![simplified, full_traditional, key],
            item_from_row,
        )
        .optional()?;
    if exact.is_some() || !traditional.trim().is_empty() {
        return Ok(exact);
    }
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM items WHERE simplified = ?1 COLLATE NOCASE \
         AND reading_key = ?2 LIMIT 2"
    ))?;
    let mut matches = stmt
        .query_map(params![simplified, key], item_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(if matches.len() == 1 {
        matches.pop()
    } else {
        None
    })
}

/// Words matching `filter`, newest first.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn list_items(conn: &Connection, filter: &LibraryFilter) -> Result<Vec<VocabItem>> {
    let mut conditions: Vec<&str> = vec![match filter.view {
        LibraryView::Active => "deleted_at IS NULL AND archived_at IS NULL",
        LibraryView::Archived => "deleted_at IS NULL AND archived_at IS NOT NULL",
        LibraryView::Trash => "deleted_at IS NOT NULL",
        LibraryView::All => "deleted_at IS NULL",
    }];
    let mut values: Vec<String> = Vec::new();
    if let Some(verification) = filter.verification {
        conditions.push("verification = ?");
        values.push(verification.as_str().to_owned());
    }
    for tag in &filter.tags {
        conditions.push(
            "EXISTS (SELECT 1 FROM item_tags it JOIN tags t ON t.id = it.tag_id \
             WHERE it.item_id = items.id AND t.name = ? COLLATE NOCASE)",
        );
        values.push(tag.trim().to_owned());
    }
    if let Some(collection) = &filter.collection {
        conditions.push(
            "EXISTS (SELECT 1 FROM collection_items ci \
             JOIN collections c ON c.id = ci.collection_id \
             WHERE ci.item_id = items.id AND c.name = ? COLLATE NOCASE)",
        );
        values.push(collection.trim().to_owned());
    }
    let sql = format!(
        "SELECT {COLUMNS} FROM items WHERE {} ORDER BY id DESC",
        conditions.join(" AND ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(values.iter()), item_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Applies `patch`. Changing the characters or reading into another saved
/// word's identity is refused.
///
/// When the traditional form equals the simplified one and only the
/// simplified form is edited, the traditional form follows it.
///
/// # Errors
///
/// [`vocab_core::ErrorKind::NotFound`], [`vocab_core::ErrorKind::Invalid`]
/// (no characters), [`vocab_core::ErrorKind::Conflict`] (identity taken), or
/// a storage error.
pub fn update_item(conn: &Connection, id: i64, patch: &ItemPatch) -> Result<VocabItem> {
    let current = require_item(conn, id)?;
    if patch.is_empty() {
        return Ok(current);
    }
    let simplified = patch.simplified.as_deref().unwrap_or(&current.simplified);
    let traditional = match &patch.traditional {
        Some(traditional) => traditional.as_str(),
        None if current.traditional == current.simplified => simplified,
        None => &current.traditional,
    };
    let (simplified, traditional) = clean_forms(simplified, traditional)?;
    let pinyin = stored_pinyin(patch.pinyin.as_deref().unwrap_or(&current.pinyin));
    if let Some(other) = find_by_identity(conn, &simplified, &traditional, &pinyin)? {
        if other.id != id {
            let place = if other.deleted_at.is_some() {
                "in the trash; restore that one instead"
            } else {
                "already saved as another word"
            };
            return Err(VocabError::conflict(format!(
                "{simplified} [{pinyin}] is {place}"
            )));
        }
    }
    conn.execute(
        &format!(
            "UPDATE items SET simplified = ?1, traditional = ?2, pinyin = ?3, reading_key = ?4, \
             definition = ?5, notes = ?6, verification = ?7, {CHANGED} WHERE id = ?8"
        ),
        params![
            simplified,
            traditional,
            pinyin,
            reading_key(&pinyin),
            patch
                .definition
                .as_deref()
                .unwrap_or(&current.definition)
                .trim(),
            patch.notes.as_deref().unwrap_or(&current.notes).trim(),
            patch.verification.unwrap_or(current.verification).as_str(),
            id,
        ],
    )
    .map_err(|err| identity_error(err, &simplified, &pinyin))?;
    require_item(conn, id)
}

fn touch(conn: &Connection, id: i64, assignment: &str, value: &dyn rusqlite::ToSql) -> Result<()> {
    let changed = conn.execute(
        &format!("UPDATE items SET {assignment}, {CHANGED} WHERE id = ?2"),
        params![value, id],
    )?;
    if changed == 0 {
        return Err(VocabError::not_found(format!("no saved word {id}")));
    }
    Ok(())
}

/// # Errors
///
/// [`vocab_core::ErrorKind::NotFound`] or a storage error.
pub fn set_verification(conn: &Connection, id: i64, verification: Verification) -> Result<()> {
    touch(conn, id, "verification = ?1", &verification.as_str())
}

/// Archives (keeps the time it was first archived) or unarchives.
///
/// # Errors
///
/// [`vocab_core::ErrorKind::NotFound`] or a storage error.
pub fn set_archived(conn: &Connection, id: i64, archived: bool) -> Result<()> {
    touch(
        conn,
        id,
        &format!("archived_at = CASE WHEN ?1 THEN coalesce(archived_at, {NOW}) ELSE NULL END"),
        &archived,
    )
}

/// Moves to or out of the trash.
///
/// # Errors
///
/// [`vocab_core::ErrorKind::NotFound`] or a storage error.
pub fn set_trashed(conn: &Connection, id: i64, trashed: bool) -> Result<()> {
    touch(
        conn,
        id,
        &format!("deleted_at = CASE WHEN ?1 THEN coalesce(deleted_at, {NOW}) ELSE NULL END"),
        &trashed,
    )
}

/// Records where a word's current data came from.
///
/// # Errors
///
/// [`vocab_core::ErrorKind::NotFound`] or a storage error.
pub fn set_source(conn: &Connection, id: i64, source: &ItemSource) -> Result<()> {
    let changed = conn.execute(
        &format!(
            "UPDATE items SET source_kind = ?1, source_id = ?2, source_version = ?3, \
             import_origin = ?4, {CHANGED} WHERE id = ?5"
        ),
        params![
            source.kind.as_str(),
            source.id,
            source.version,
            source.import_origin,
            id
        ],
    )?;
    if changed == 0 {
        return Err(VocabError::not_found(format!("no saved word {id}")));
    }
    Ok(())
}

/// Deletes a trashed word for good, with its tags and collection places.
/// Transfer history keeps its lines, without the link.
///
/// # Errors
///
/// [`vocab_core::ErrorKind::Invalid`] if the word is not in the trash;
/// [`vocab_core::ErrorKind::NotFound`] or a storage error.
pub fn purge_item(conn: &Connection, id: i64) -> Result<()> {
    let item = require_item(conn, id)?;
    if item.deleted_at.is_none() {
        return Err(VocabError::invalid(format!(
            "move {} to the trash before deleting it for good",
            item.simplified
        )));
    }
    conn.execute("DELETE FROM items WHERE id = ?1", [id])?;
    Ok(())
}

fn item_from_row(row: &Row<'_>) -> rusqlite::Result<VocabItem> {
    let text_err = |index: usize, err: VocabError| {
        rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(err))
    };
    let verification: String = row.get(6)?;
    let kind: String = row.get(9)?;
    Ok(VocabItem {
        id: row.get(0)?,
        simplified: row.get(1)?,
        traditional: row.get(2)?,
        pinyin: row.get(3)?,
        definition: row.get(4)?,
        notes: row.get(5)?,
        verification: verification.parse().map_err(|err| text_err(6, err))?,
        archived_at: row.get(7)?,
        deleted_at: row.get(8)?,
        source: ItemSource {
            kind: kind.parse::<SourceKind>().map_err(|err| text_err(9, err))?,
            id: row.get(10)?,
            version: row.get(11)?,
            import_origin: row.get(12)?,
        },
        created_at: row.get(13)?,
        modified_at: row.get(14)?,
        rev: row.get(15)?,
    })
}

/// Marks a word changed: its tags or collections were edited.
pub(crate) fn mark_changed(conn: &Connection, id: i64) -> Result<()> {
    conn.execute(&format!("UPDATE items SET {CHANGED} WHERE id = ?1"), [id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use vocab_core::{
        ErrorKind, ItemPatch, ItemSource, LibraryFilter, LibraryView, Lifecycle, Verification,
    };

    use super::{
        NewItem, Saved, find_by_identity, list_items, purge_item, save_item, set_archived,
        set_trashed, update_item,
    };
    use crate::open_in_memory;

    fn word(simplified: &str, traditional: &str, pinyin: &str) -> NewItem {
        NewItem {
            simplified: simplified.to_owned(),
            traditional: traditional.to_owned(),
            pinyin: pinyin.to_owned(),
            definition: "gloss".to_owned(),
            notes: String::new(),
            verification: Verification::Confirmed,
            source: ItemSource::manual(),
        }
    }

    #[test]
    fn same_word_with_marks_or_numbers_is_one_word() {
        let conn = open_in_memory().unwrap();
        let first = save_item(&conn, &word("学校", "學校", "xue2 xiao4")).unwrap();
        assert!(matches!(first, Saved::Inserted(_)));
        let again = save_item(&conn, &word("学校", "學校", "xué xiào")).unwrap();
        assert!(matches!(again, Saved::Existing(_)));
    }

    #[test]
    fn different_readings_are_different_words() {
        let conn = open_in_memory().unwrap();
        save_item(&conn, &word("行", "", "xing2")).unwrap();
        let other = save_item(&conn, &word("行", "", "hang2")).unwrap();
        assert!(matches!(other, Saved::Inserted(_)));
        assert_eq!(
            other.item().traditional,
            "行",
            "blank traditional = simplified"
        );
    }

    #[test]
    fn saving_a_trashed_word_restores_it() {
        let conn = open_in_memory().unwrap();
        let id = save_item(&conn, &word("猫", "貓", "mao1"))
            .unwrap()
            .item()
            .id;
        set_trashed(&conn, id, true).unwrap();
        let back = save_item(&conn, &word("猫", "貓", "mao1")).unwrap();
        assert!(matches!(back, Saved::Restored(_)));
        assert_eq!(back.item().id, id);
        assert_eq!(back.item().lifecycle(), Lifecycle::Active);
    }

    #[test]
    fn views_split_active_archived_and_trash() {
        let conn = open_in_memory().unwrap();
        let a = save_item(&conn, &word("一", "", "yi1")).unwrap().item().id;
        let b = save_item(&conn, &word("二", "", "er4")).unwrap().item().id;
        let c = save_item(&conn, &word("三", "", "san1")).unwrap().item().id;
        set_archived(&conn, b, true).unwrap();
        set_trashed(&conn, c, true).unwrap();
        let ids = |view| {
            list_items(
                &conn,
                &LibraryFilter {
                    view,
                    ..LibraryFilter::default()
                },
            )
            .unwrap()
            .into_iter()
            .map(|item| item.id)
            .collect::<Vec<_>>()
        };
        assert_eq!(ids(LibraryView::Active), vec![a]);
        assert_eq!(ids(LibraryView::Archived), vec![b]);
        assert_eq!(ids(LibraryView::Trash), vec![c]);
        assert_eq!(ids(LibraryView::All), vec![b, a], "newest first");
    }

    #[test]
    fn editing_into_another_word_is_a_conflict() {
        let conn = open_in_memory().unwrap();
        save_item(&conn, &word("行", "", "xing2")).unwrap();
        let hang = save_item(&conn, &word("行", "", "hang2"))
            .unwrap()
            .item()
            .id;
        let err = update_item(
            &conn,
            hang,
            &ItemPatch {
                pinyin: Some("xíng".to_owned()),
                ..ItemPatch::default()
            },
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Conflict);
    }

    #[test]
    fn edits_recompute_the_reading_and_follow_simplified() {
        let conn = open_in_memory().unwrap();
        let id = save_item(&conn, &word("你好", "", "ni3 hao3"))
            .unwrap()
            .item()
            .id;
        let edited = update_item(
            &conn,
            id,
            &ItemPatch {
                simplified: Some("您好".to_owned()),
                pinyin: Some("nín hǎo".to_owned()),
                notes: Some("  polite  ".to_owned()),
                ..ItemPatch::default()
            },
        )
        .unwrap();
        assert_eq!(edited.traditional, "您好");
        assert_eq!(edited.notes, "polite");
        assert!(
            find_by_identity(&conn, "您好", "", "nin2 hao3")
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn purge_needs_the_trash_first() {
        let conn = open_in_memory().unwrap();
        let id = save_item(&conn, &word("水", "", "shui3"))
            .unwrap()
            .item()
            .id;
        assert_eq!(
            purge_item(&conn, id).unwrap_err().kind(),
            ErrorKind::Invalid
        );
        set_trashed(&conn, id, true).unwrap();
        purge_item(&conn, id).unwrap();
        assert!(
            find_by_identity(&conn, "水", "", "shui3")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn readings_written_any_way_are_one_word() {
        let conn = open_in_memory().unwrap();
        save_item(&conn, &word("妈妈", "媽媽", "ma1 ma5")).unwrap();
        assert!(matches!(
            save_item(&conn, &word("妈妈", "媽媽", "māma")).unwrap(),
            Saved::Existing(_)
        ));
        save_item(&conn, &word("哪儿", "哪兒", "na3 r5")).unwrap();
        assert!(matches!(
            save_item(&conn, &word("哪儿", "哪兒", "nǎr")).unwrap(),
            Saved::Existing(_)
        ));
        save_item(&conn, &word("略", "", "lve4")).unwrap();
        assert!(matches!(
            save_item(&conn, &word("略", "", "lue4")).unwrap(),
            Saved::Existing(_)
        ));
        save_item(&conn, &word("西安", "", "Xi1 an1")).unwrap();
        assert!(matches!(
            save_item(&conn, &word("西安", "", "xi1'an1")).unwrap(),
            Saved::Existing(_)
        ));
    }

    #[test]
    fn unknown_traditional_matches_the_one_saved_word() {
        let conn = open_in_memory().unwrap();
        save_item(&conn, &word("学校", "學校", "xue2 xiao4")).unwrap();
        assert!(matches!(
            save_item(&conn, &word("学校", "", "xué xiào")).unwrap(),
            Saved::Existing(_)
        ));
    }

    #[test]
    fn a_placeholder_without_a_reading_is_completed() {
        let conn = open_in_memory().unwrap();
        let mut placeholder = word("学校", "", "");
        placeholder.definition = String::new();
        placeholder.verification = Verification::NeedsReview;
        let id = save_item(&conn, &placeholder).unwrap().item().id;
        let completed = save_item(&conn, &word("学校", "學校", "xue2 xiao4")).unwrap();
        let Saved::Completed(item) = completed else {
            panic!("expected completion: {completed:?}");
        };
        assert_eq!(item.id, id);
        assert_eq!(item.traditional, "學校");
        assert_eq!(item.definition, "gloss");
        assert_eq!(item.verification, Verification::Confirmed);
    }

    #[test]
    fn editing_into_a_trashed_word_says_so() {
        let conn = open_in_memory().unwrap();
        let gone = save_item(&conn, &word("行", "", "xing2"))
            .unwrap()
            .item()
            .id;
        set_trashed(&conn, gone, true).unwrap();
        let hang = save_item(&conn, &word("行", "", "hang2"))
            .unwrap()
            .item()
            .id;
        let err = update_item(
            &conn,
            hang,
            &ItemPatch {
                pinyin: Some("xing2".to_owned()),
                ..ItemPatch::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("trash"), "{err}");
    }

    #[test]
    fn a_word_needs_characters() {
        let conn = open_in_memory().unwrap();
        let err = save_item(&conn, &word("  ", "", "")).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Invalid);
    }
}
