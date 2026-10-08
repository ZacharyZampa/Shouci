//! Tags and collections.
//!
//! Tags are free labels for finding and filtering words. Collections are
//! curated groups (a textbook chapter, a week of class) that export as a
//! Pleco category or an Anki deck. A word can have any number of each. Names
//! are case-insensitive: `HSK1` and `hsk1` are one tag. Renaming or merging
//! one renames it in smart collections' filters too (`smart.rs`).

use std::collections::{BTreeSet, HashMap};

use rusqlite::{Connection, OptionalExtension, params};
use vocab_core::{Result, VocabError};

use crate::items::{mark_changed, require_item};

/// A tag or collection and how many words outside the trash it has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameCount {
    pub name: String,
    pub count: u64,
}

#[derive(Clone, Copy)]
pub(crate) enum Group {
    Tag,
    Collection,
}

impl Group {
    /// The table of names.
    pub(crate) fn table(self) -> &'static str {
        match self {
            Self::Tag => "tags",
            Self::Collection => "collections",
        }
    }

    /// The table linking words to names.
    pub(crate) fn join_table(self) -> &'static str {
        match self {
            Self::Tag => "item_tags",
            Self::Collection => "collection_items",
        }
    }

    /// The join table's column naming the tag or collection.
    pub(crate) fn foreign_key(self) -> &'static str {
        match self {
            Self::Tag => "tag_id",
            Self::Collection => "collection_id",
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Self::Tag => "tag",
            Self::Collection => "collection",
        }
    }
}

fn clean_name(group: Group, name: &str) -> Result<String> {
    checked_name(group.noun(), name)
}

/// `name` trimmed, for a tag, a collection, or a smart collection (`noun`).
/// A blank name, or one with tabs or line breaks, is refused.
pub(crate) fn checked_name(noun: &str, name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(VocabError::invalid(format!("a {noun} needs a name")));
    }
    if name.contains(['\n', '\r', '\t']) {
        return Err(VocabError::invalid(format!(
            "a {noun} name cannot contain tabs or line breaks"
        )));
    }
    Ok(name.to_owned())
}

fn find(conn: &Connection, group: Group, name: &str) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            &format!("SELECT id FROM {} WHERE name = ?1", group.table()),
            [name],
            |row| row.get(0),
        )
        .optional()?)
}

fn require(conn: &Connection, group: Group, name: &str) -> Result<i64> {
    find(conn, group, name)?
        .ok_or_else(|| VocabError::not_found(format!("no {} named {name}", group.noun())))
}

fn ensure(conn: &Connection, group: Group, name: &str) -> Result<i64> {
    let name = clean_name(group, name)?;
    // Insert-or-ignore, then look up: safe when another process creates the
    // same name at the same moment.
    conn.execute(
        &format!("INSERT OR IGNORE INTO {} (name) VALUES (?1)", group.table()),
        [&name],
    )?;
    find(conn, group, &name)?
        .ok_or_else(|| VocabError::new(format!("{} {name} vanished", group.noun())))
}

fn list(conn: &Connection, group: Group) -> Result<Vec<NameCount>> {
    let sql = format!(
        "SELECT g.name, count(i.id) FROM {table} g \
         LEFT JOIN {links} l ON l.{key} = g.id \
         LEFT JOIN items i ON i.id = l.item_id AND i.deleted_at IS NULL \
         GROUP BY g.id ORDER BY g.name COLLATE NOCASE",
        table = group.table(),
        links = group.join_table(),
        key = group.foreign_key(),
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let count: i64 = row.get(1)?;
        Ok(NameCount {
            name: row.get(0)?,
            count: u64::try_from(count).unwrap_or(0),
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn names_of(conn: &Connection, group: Group, item_id: i64) -> Result<Vec<String>> {
    let sql = format!(
        "SELECT g.name FROM {table} g JOIN {links} l ON l.{key} = g.id \
         WHERE l.item_id = ?1 ORDER BY g.name COLLATE NOCASE",
        table = group.table(),
        links = group.join_table(),
        key = group.foreign_key(),
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([item_id], |row| row.get(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<String>>>()?)
}

fn names_by_item(conn: &Connection, group: Group) -> Result<HashMap<i64, Vec<String>>> {
    let sql = format!(
        "SELECT l.item_id, g.name FROM {table} g JOIN {links} l ON l.{key} = g.id \
         ORDER BY g.name COLLATE NOCASE",
        table = group.table(),
        links = group.join_table(),
        key = group.foreign_key(),
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query([])?;
    let mut out: HashMap<i64, Vec<String>> = HashMap::new();
    while let Some(row) = rows.next()? {
        out.entry(row.get(0)?).or_default().push(row.get(1)?);
    }
    Ok(out)
}

fn link(conn: &Connection, group: Group, item_id: i64, name: &str) -> Result<()> {
    require_item(conn, item_id)?;
    let group_id = ensure(conn, group, name)?;
    let added = conn.execute(
        &format!(
            "INSERT OR IGNORE INTO {} (item_id, {}) VALUES (?1, ?2)",
            group.join_table(),
            group.foreign_key()
        ),
        params![item_id, group_id],
    )?;
    if added > 0 {
        mark_changed(conn, item_id)?;
    }
    Ok(())
}

fn unlink(conn: &Connection, group: Group, item_id: i64, name: &str) -> Result<()> {
    let Some(group_id) = find(conn, group, name.trim())? else {
        return Ok(());
    };
    let removed = conn.execute(
        &format!(
            "DELETE FROM {} WHERE item_id = ?1 AND {} = ?2",
            group.join_table(),
            group.foreign_key()
        ),
        params![item_id, group_id],
    )?;
    if removed > 0 {
        mark_changed(conn, item_id)?;
    }
    Ok(())
}

fn replace(conn: &Connection, group: Group, item_id: i64, names: &[String]) -> Result<()> {
    require_item(conn, item_id)?;
    // Every name must be valid before the old ones go.
    let mut wanted = BTreeSet::new();
    for name in names {
        // Names compare as `COLLATE NOCASE` does: ASCII case only.
        wanted.insert(clean_name(group, name)?.to_ascii_lowercase());
    }
    let current: BTreeSet<String> = names_of(conn, group, item_id)?
        .iter()
        .map(|name| name.to_ascii_lowercase())
        .collect();
    if current == wanted {
        // Nothing to change, so the word's revision stays.
        return Ok(());
    }
    mark_changed(conn, item_id)?;
    conn.execute(
        &format!("DELETE FROM {} WHERE item_id = ?1", group.join_table()),
        [item_id],
    )?;
    for name in names {
        link(conn, group, item_id, name)?;
    }
    Ok(())
}

fn rename(conn: &Connection, group: Group, from: &str, to: &str) -> Result<()> {
    let id = require(conn, group, from.trim())?;
    let to = clean_name(group, to)?;
    if let Some(existing) = find(conn, group, &to)? {
        if existing != id {
            return Err(VocabError::conflict(format!(
                "a {} named {to} already exists",
                group.noun()
            )));
        }
    }
    let from = name_of(conn, group, id)?;
    conn.execute(
        &format!("UPDATE {} SET name = ?1 WHERE id = ?2", group.table()),
        params![to, id],
    )?;
    crate::smart::follow_rename(conn, group, &from, &to)?;
    mark_members_changed(conn, group, id)
}

/// The name as the library spells it.
fn name_of(conn: &Connection, group: Group, id: i64) -> Result<String> {
    Ok(conn.query_row(
        &format!("SELECT name FROM {} WHERE id = ?1", group.table()),
        [id],
        |row| row.get(0),
    )?)
}

/// Whether any word, in the trash or not, has the name.
fn in_use(conn: &Connection, group: Group, name: &str) -> Result<bool> {
    let sql = format!(
        "SELECT EXISTS (SELECT 1 FROM {links} l JOIN {table} g ON g.id = l.{key} \
         WHERE g.name = ?1)",
        links = group.join_table(),
        table = group.table(),
        key = group.foreign_key(),
    );
    Ok(conn.query_row(&sql, [name.trim()], |row| row.get(0))?)
}

/// Moves every word of `from` into `into`, then deletes `from`. A word in
/// both stays in `into` once.
fn merge(conn: &Connection, group: Group, from: &str, into: &str) -> Result<()> {
    let from_id = require(conn, group, from.trim())?;
    let into_id = require(conn, group, into.trim())?;
    if from_id == into_id {
        return Err(VocabError::invalid(format!(
            "a {} cannot be merged into itself",
            group.noun()
        )));
    }
    mark_members_changed(conn, group, from_id)?;
    conn.execute(
        &format!(
            "INSERT OR IGNORE INTO {links} (item_id, {key}) \
             SELECT item_id, ?1 FROM {links} WHERE {key} = ?2",
            links = group.join_table(),
            key = group.foreign_key(),
        ),
        params![into_id, from_id],
    )?;
    let (from, into) = (
        name_of(conn, group, from_id)?,
        name_of(conn, group, into_id)?,
    );
    conn.execute(
        &format!("DELETE FROM {} WHERE id = ?1", group.table()),
        [from_id],
    )?;
    crate::smart::follow_rename(conn, group, &from, &into)
}

/// Every word in a tag or collection changed with it.
fn mark_members_changed(conn: &Connection, group: Group, group_id: i64) -> Result<()> {
    conn.execute(
        &format!(
            "UPDATE items SET modified_at = strftime('%Y-%m-%dT%H:%M:%fZ','now'), \
             rev = rev + 1 WHERE id IN (SELECT item_id FROM {} WHERE {} = ?1)",
            group.join_table(),
            group.foreign_key()
        ),
        [group_id],
    )?;
    Ok(())
}

fn delete(conn: &Connection, group: Group, name: &str) -> Result<()> {
    let id = require(conn, group, name.trim())?;
    mark_members_changed(conn, group, id)?;
    conn.execute(
        &format!("DELETE FROM {} WHERE id = ?1", group.table()),
        [id],
    )?;
    Ok(())
}

/// Every tag, by name.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn tags(conn: &Connection) -> Result<Vec<NameCount>> {
    list(conn, Group::Tag)
}

/// # Errors
///
/// Returns an error if the query fails.
pub fn item_tags(conn: &Connection, item_id: i64) -> Result<Vec<String>> {
    names_of(conn, Group::Tag, item_id)
}

/// Every item's tags at once, for listings.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn tags_by_item(conn: &Connection) -> Result<HashMap<i64, Vec<String>>> {
    names_by_item(conn, Group::Tag)
}

/// Tags a word, creating the tag on first use.
///
/// # Errors
///
/// Returns an error for a blank name, a missing item, or a failed write.
pub fn add_tag(conn: &Connection, item_id: i64, name: &str) -> Result<()> {
    link(conn, Group::Tag, item_id, name)
}

/// # Errors
///
/// Returns an error if the write fails.
pub fn remove_tag(conn: &Connection, item_id: i64, name: &str) -> Result<()> {
    unlink(conn, Group::Tag, item_id, name)
}

/// Replaces a word's tags.
///
/// # Errors
///
/// Returns an error for a blank name, a missing item, or a failed write.
pub fn set_tags(conn: &Connection, item_id: i64, names: &[String]) -> Result<()> {
    replace(conn, Group::Tag, item_id, names)
}

/// # Errors
///
/// [`vocab_core::ErrorKind::NotFound`], [`vocab_core::ErrorKind::Conflict`]
/// if the new name is taken, or a storage error.
pub fn rename_tag(conn: &Connection, from: &str, to: &str) -> Result<()> {
    rename(conn, Group::Tag, from, to)
}

/// Gives every word tagged `from` the tag `into` instead, then deletes
/// `from`. The way to fold `HSK1` into `HSK 1`.
///
/// # Errors
///
/// [`vocab_core::ErrorKind::NotFound`] if either tag is missing,
/// [`vocab_core::ErrorKind::Invalid`] if they are the same tag, or a storage
/// error.
pub fn merge_tags(conn: &Connection, from: &str, into: &str) -> Result<()> {
    merge(conn, Group::Tag, from, into)
}

/// Creates a tag no word has yet: how an undo brings back a deleted one.
///
/// # Errors
///
/// [`vocab_core::ErrorKind::Conflict`] if it exists; an error for a blank
/// name or a failed write.
pub fn create_tag(conn: &Connection, name: &str) -> Result<()> {
    let name = clean_name(Group::Tag, name)?;
    if find(conn, Group::Tag, &name)?.is_some() {
        return Err(VocabError::conflict(format!(
            "a tag named {name} already exists"
        )));
    }
    ensure(conn, Group::Tag, &name)?;
    Ok(())
}

/// Whether any word, in the trash or not, has the tag.
///
/// # Errors
///
/// Storage errors.
pub fn tag_in_use(conn: &Connection, name: &str) -> Result<bool> {
    in_use(conn, Group::Tag, name)
}

/// Whether any word, in the trash or not, is in the collection.
///
/// # Errors
///
/// Storage errors.
pub fn collection_in_use(conn: &Connection, name: &str) -> Result<bool> {
    in_use(conn, Group::Collection, name)
}

/// Deletes a tag. Its words stay.
///
/// # Errors
///
/// [`vocab_core::ErrorKind::NotFound`] or a storage error.
pub fn delete_tag(conn: &Connection, name: &str) -> Result<()> {
    delete(conn, Group::Tag, name)
}

/// Every collection, by name.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn collections(conn: &Connection) -> Result<Vec<NameCount>> {
    list(conn, Group::Collection)
}

/// # Errors
///
/// Returns an error if the query fails.
pub fn item_collections(conn: &Connection, item_id: i64) -> Result<Vec<String>> {
    names_of(conn, Group::Collection, item_id)
}

/// Every item's collections at once, for listings.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn collections_by_item(conn: &Connection) -> Result<HashMap<i64, Vec<String>>> {
    names_by_item(conn, Group::Collection)
}

/// Creates an empty collection.
///
/// # Errors
///
/// [`vocab_core::ErrorKind::Conflict`] if it exists; an error for a blank
/// name or a failed write.
pub fn create_collection(conn: &Connection, name: &str) -> Result<()> {
    let name = clean_name(Group::Collection, name)?;
    if find(conn, Group::Collection, &name)?.is_some() {
        return Err(VocabError::conflict(format!(
            "a collection named {name} already exists"
        )));
    }
    ensure(conn, Group::Collection, &name)?;
    Ok(())
}

/// Adds a word to a collection, creating the collection on first use.
///
/// # Errors
///
/// Returns an error for a blank name, a missing item, or a failed write.
pub fn add_to_collection(conn: &Connection, item_id: i64, name: &str) -> Result<()> {
    link(conn, Group::Collection, item_id, name)
}

/// # Errors
///
/// Returns an error if the write fails.
pub fn remove_from_collection(conn: &Connection, item_id: i64, name: &str) -> Result<()> {
    unlink(conn, Group::Collection, item_id, name)
}

/// Replaces a word's collections.
///
/// # Errors
///
/// Returns an error for a blank name, a missing item, or a failed write.
pub fn set_collections(conn: &Connection, item_id: i64, names: &[String]) -> Result<()> {
    replace(conn, Group::Collection, item_id, names)
}

/// # Errors
///
/// [`vocab_core::ErrorKind::NotFound`], [`vocab_core::ErrorKind::Conflict`]
/// if the new name is taken, or a storage error.
pub fn rename_collection(conn: &Connection, from: &str, to: &str) -> Result<()> {
    rename(conn, Group::Collection, from, to)
}

/// Puts every word of collection `from` into `into`, then deletes `from`.
///
/// # Errors
///
/// [`vocab_core::ErrorKind::NotFound`] if either collection is missing,
/// [`vocab_core::ErrorKind::Invalid`] if they are the same collection, or a
/// storage error.
pub fn merge_collections(conn: &Connection, from: &str, into: &str) -> Result<()> {
    merge(conn, Group::Collection, from, into)
}

/// Deletes a collection. Its words stay.
///
/// # Errors
///
/// [`vocab_core::ErrorKind::NotFound`] or a storage error.
pub fn delete_collection(conn: &Connection, name: &str) -> Result<()> {
    delete(conn, Group::Collection, name)
}

/// Turns a tag into a collection of the same name: every word with the tag
/// joins the collection (created if needed), then the tag is deleted.
/// Returns how many words the collection holds now.
///
/// # Errors
///
/// [`vocab_core::ErrorKind::NotFound`] if there is no such tag, or a storage
/// error.
pub fn collection_from_tag(conn: &Connection, tag: &str) -> Result<NameCount> {
    let tag_id = require(conn, Group::Tag, tag.trim())?;
    let name: String = conn.query_row("SELECT name FROM tags WHERE id = ?1", [tag_id], |row| {
        row.get(0)
    })?;
    let collection_id = ensure(conn, Group::Collection, &name)?;
    mark_members_changed(conn, Group::Tag, tag_id)?;
    conn.execute(
        "INSERT OR IGNORE INTO collection_items (collection_id, item_id) \
         SELECT ?1, item_id FROM item_tags WHERE tag_id = ?2",
        params![collection_id, tag_id],
    )?;
    conn.execute("DELETE FROM tags WHERE id = ?1", [tag_id])?;
    crate::smart::follow_tag_to_collection(conn, &name)?;
    let (name, count): (String, i64) = conn.query_row(
        "SELECT c.name, count(i.id) FROM collections c \
         LEFT JOIN collection_items ci ON ci.collection_id = c.id \
         LEFT JOIN items i ON i.id = ci.item_id AND i.deleted_at IS NULL \
         WHERE c.id = ?1 GROUP BY c.id",
        [collection_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(NameCount {
        name,
        count: u64::try_from(count).unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use vocab_core::{ErrorKind, ItemSource, Verification};

    use super::{
        add_tag, add_to_collection, collection_from_tag, collections, delete_tag, item_collections,
        item_tags, merge_collections, merge_tags, rename_tag, set_collections, set_tags, tags,
    };
    use crate::items::require_item;
    use crate::items::{NewItem, save_item, set_trashed};
    use crate::open_in_memory;

    fn saved(conn: &rusqlite::Connection, simplified: &str, pinyin: &str) -> i64 {
        save_item(
            conn,
            &NewItem {
                simplified: simplified.to_owned(),
                traditional: String::new(),
                pinyin: pinyin.to_owned(),
                definition: String::new(),
                notes: String::new(),
                verification: Verification::Confirmed,
                source: ItemSource::manual(),
            },
        )
        .unwrap()
        .item()
        .id
    }

    #[test]
    fn tags_are_case_insensitive_and_counted_outside_the_trash() {
        let conn = open_in_memory().unwrap();
        let a = saved(&conn, "一", "yi1");
        let b = saved(&conn, "二", "er4");
        add_tag(&conn, a, "HSK1").unwrap();
        add_tag(&conn, b, "hsk1").unwrap();
        add_tag(&conn, b, "hsk1").unwrap();
        set_trashed(&conn, b, true).unwrap();
        let all = tags(&conn).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "HSK1");
        assert_eq!(all[0].count, 1);
    }

    #[test]
    fn set_tags_replaces_and_rename_refuses_a_taken_name() {
        let conn = open_in_memory().unwrap();
        let a = saved(&conn, "一", "yi1");
        add_tag(&conn, a, "old").unwrap();
        set_tags(&conn, a, &["b".to_owned(), "a".to_owned()]).unwrap();
        assert_eq!(item_tags(&conn, a).unwrap(), vec!["a", "b"]);
        assert_eq!(
            rename_tag(&conn, "a", "B").unwrap_err().kind(),
            ErrorKind::Conflict
        );
        rename_tag(&conn, "a", "A").unwrap();
        delete_tag(&conn, "b").unwrap();
        assert_eq!(item_tags(&conn, a).unwrap(), vec!["A"]);
        assert_eq!(
            delete_tag(&conn, "b").unwrap_err().kind(),
            ErrorKind::NotFound
        );
    }

    #[test]
    fn a_bad_name_leaves_the_old_tags_alone() {
        let conn = open_in_memory().unwrap();
        let a = saved(&conn, "一", "yi1");
        add_tag(&conn, a, "keep").unwrap();
        assert!(set_tags(&conn, a, &["new".to_owned(), "  ".to_owned()]).is_err());
        assert_eq!(item_tags(&conn, a).unwrap(), vec!["keep"]);
    }

    #[test]
    fn tag_edits_count_as_changes_to_the_word() {
        let conn = open_in_memory().unwrap();
        let a = saved(&conn, "一", "yi1");
        let before = crate::require_item(&conn, a).unwrap().rev;
        add_tag(&conn, a, "x").unwrap();
        rename_tag(&conn, "x", "y").unwrap();
        let after = crate::require_item(&conn, a).unwrap().rev;
        assert_eq!(after, before + 2);
    }

    #[test]
    fn setting_the_same_groups_is_no_change() {
        let conn = open_in_memory().unwrap();
        let a = saved(&conn, "一", "yi1");
        set_tags(&conn, a, &["x".to_owned(), "y".to_owned()]).unwrap();
        set_collections(&conn, a, &["Food".to_owned()]).unwrap();
        let before = crate::require_item(&conn, a).unwrap().rev;
        set_tags(&conn, a, &["Y".to_owned(), " x ".to_owned()]).unwrap();
        set_collections(&conn, a, &["food".to_owned()]).unwrap();
        assert_eq!(crate::require_item(&conn, a).unwrap().rev, before);
        set_tags(&conn, a, &["x".to_owned()]).unwrap();
        assert!(crate::require_item(&conn, a).unwrap().rev > before);
        assert_eq!(item_tags(&conn, a).unwrap(), vec!["x"]);
    }

    #[test]
    fn blank_names_are_refused() {
        let conn = open_in_memory().unwrap();
        let a = saved(&conn, "一", "yi1");
        assert_eq!(
            add_tag(&conn, a, " ").unwrap_err().kind(),
            ErrorKind::Invalid
        );
        assert_eq!(
            add_to_collection(&conn, a, "a\tb").unwrap_err().kind(),
            ErrorKind::Invalid
        );
    }

    #[test]
    fn a_tag_becomes_a_collection() {
        let conn = open_in_memory().unwrap();
        let a = saved(&conn, "一", "yi1");
        let b = saved(&conn, "二", "er4");
        add_tag(&conn, a, "Lesson 1").unwrap();
        add_tag(&conn, b, "Lesson 1").unwrap();
        add_to_collection(&conn, b, "lesson 1").unwrap();
        let made = collection_from_tag(&conn, "lesson 1").unwrap();
        assert_eq!(made.count, 2);
        assert_eq!(collections(&conn).unwrap().len(), 1);
        assert_eq!(tags(&conn).unwrap(), []);
    }

    #[test]
    fn merging_moves_every_word_once_and_removes_the_old_name() {
        let conn = open_in_memory().unwrap();
        let both = saved(&conn, "一", "yi1");
        let only_old = saved(&conn, "二", "er4");
        let untouched = saved(&conn, "三", "san1");
        for id in [both, only_old] {
            add_tag(&conn, id, "HSK1").unwrap();
            add_to_collection(&conn, id, "Week_1").unwrap();
        }
        add_tag(&conn, both, "HSK 1").unwrap();
        add_to_collection(&conn, both, "Week 1").unwrap();
        add_to_collection(&conn, untouched, "Week 1").unwrap();
        let rev = |id| require_item(&conn, id).unwrap().rev;
        let before = (rev(both), rev(only_old), rev(untouched));

        merge_tags(&conn, "hsk1", "HSK 1").unwrap();
        merge_collections(&conn, "Week_1", "week 1").unwrap();

        assert_eq!(item_tags(&conn, both).unwrap(), vec!["HSK 1"]);
        assert_eq!(item_tags(&conn, only_old).unwrap(), vec!["HSK 1"]);
        assert_eq!(item_collections(&conn, only_old).unwrap(), vec!["Week 1"]);
        let names: Vec<_> = collections(&conn)
            .unwrap()
            .into_iter()
            .map(|g| (g.name, g.count))
            .collect();
        assert_eq!(names, vec![("Week 1".to_owned(), 3)]);
        assert_eq!(tags(&conn).unwrap().len(), 1);
        assert!(
            rev(both) > before.0 && rev(only_old) > before.1,
            "their groups changed"
        );
        assert_eq!(rev(untouched), before.2, "already in Week 1 only");
        assert_eq!(
            merge_tags(&conn, "HSK 1", "hsk 1").unwrap_err().kind(),
            ErrorKind::Invalid
        );
        assert_eq!(
            merge_tags(&conn, "missing", "HSK 1").unwrap_err().kind(),
            ErrorKind::NotFound
        );
    }
}
