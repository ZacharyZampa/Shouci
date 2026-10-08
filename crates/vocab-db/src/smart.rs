//! Smart collections: saved filters, listed beside the collections. A smart
//! collection holds no words of its own; its words are the ones its filter
//! matches whenever it is looked at.
//!
//! The filter is kept as JSON, so a newer Shouci can add conditions without
//! a migration (an older one ignores what it doesn't know). Renaming or
//! merging a tag or collection renames it in every filter too, so a smart
//! collection keeps finding the same words.

use rusqlite::{Connection, OptionalExtension, params};
use vocab_core::{LibraryFilter, Result, VocabError};

use crate::organize::{Group, checked_name};

const NOUN: &str = "smart collection";

/// A saved filter and its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmartCollection {
    pub name: String,
    pub filter: LibraryFilter,
}

/// Every smart collection, by name.
///
/// # Errors
///
/// Storage errors, or a filter that can't be read.
pub fn smart_collections(conn: &Connection) -> Result<Vec<SmartCollection>> {
    let mut stmt =
        conn.prepare("SELECT name, filter FROM smart_collections ORDER BY name COLLATE NOCASE")?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    rows.collect::<rusqlite::Result<Vec<(String, String)>>>()?
        .into_iter()
        .map(|(name, filter)| read(name, &filter))
        .collect()
}

/// The smart collection called `name`, compared ignoring ASCII case.
///
/// # Errors
///
/// Storage errors, or a filter that can't be read.
pub fn smart_collection(conn: &Connection, name: &str) -> Result<Option<SmartCollection>> {
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT name, filter FROM smart_collections WHERE name = ?1",
            [name.trim()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    row.map(|(name, filter)| read(name, &filter)).transpose()
}

/// # Errors
///
/// [`vocab_core::ErrorKind::Conflict`] if a smart collection has the name,
/// [`vocab_core::ErrorKind::Invalid`] for a blank one, or a storage error.
pub fn create_smart_collection(
    conn: &Connection,
    name: &str,
    filter: &LibraryFilter,
) -> Result<()> {
    let name = checked_name(NOUN, name)?;
    if smart_collection(conn, &name)?.is_some() {
        return Err(VocabError::conflict(format!(
            "a smart collection named {name} already exists"
        )));
    }
    conn.execute(
        "INSERT INTO smart_collections (name, filter) VALUES (?1, ?2)",
        params![name, write(filter)?],
    )?;
    Ok(())
}

/// Gives a smart collection another filter.
///
/// # Errors
///
/// [`vocab_core::ErrorKind::NotFound`] or a storage error.
pub fn update_smart_collection(
    conn: &Connection,
    name: &str,
    filter: &LibraryFilter,
) -> Result<()> {
    let changed = conn.execute(
        "UPDATE smart_collections SET filter = ?1 WHERE name = ?2",
        params![write(filter)?, name.trim()],
    )?;
    if changed == 0 {
        return Err(not_found(name));
    }
    Ok(())
}

/// # Errors
///
/// [`vocab_core::ErrorKind::NotFound`], [`vocab_core::ErrorKind::Conflict`]
/// when another smart collection has the new name,
/// [`vocab_core::ErrorKind::Invalid`] for a blank one, or a storage error.
pub fn rename_smart_collection(conn: &Connection, from: &str, to: &str) -> Result<()> {
    let to = checked_name(NOUN, to)?;
    if !to.eq_ignore_ascii_case(from.trim()) && smart_collection(conn, &to)?.is_some() {
        return Err(VocabError::conflict(format!(
            "a smart collection named {to} already exists"
        )));
    }
    let changed = conn.execute(
        "UPDATE smart_collections SET name = ?1 WHERE name = ?2",
        params![to, from.trim()],
    )?;
    if changed == 0 {
        return Err(not_found(from));
    }
    Ok(())
}

/// Deletes a smart collection. Its words are untouched.
///
/// # Errors
///
/// [`vocab_core::ErrorKind::NotFound`] or a storage error.
pub fn delete_smart_collection(conn: &Connection, name: &str) -> Result<()> {
    let changed = conn.execute(
        "DELETE FROM smart_collections WHERE name = ?1",
        [name.trim()],
    )?;
    if changed == 0 {
        return Err(not_found(name));
    }
    Ok(())
}

/// After a tag or collection called `from` became `to` (renamed, or merged
/// into it): every filter that names `from` names `to` instead.
pub(crate) fn follow_rename(conn: &Connection, group: Group, from: &str, to: &str) -> Result<()> {
    rewrite(conn, |filter| {
        let lists = match group {
            Group::Tag => vec![
                &mut filter.tags,
                &mut filter.any_tags,
                &mut filter.without_tags,
            ],
            Group::Collection => {
                if let Some(collection) = &mut filter.collection {
                    if collection.trim().eq_ignore_ascii_case(from) {
                        to.clone_into(collection);
                    }
                }
                vec![&mut filter.any_collections, &mut filter.without_collections]
            }
        };
        for names in lists {
            for name in names.iter_mut() {
                if name.trim().eq_ignore_ascii_case(from) {
                    to.clone_into(name);
                }
            }
            // Merging into a name the filter already had leaves it twice.
            let mut seen = Vec::with_capacity(names.len());
            names.retain(|name| {
                let key = name.to_ascii_lowercase();
                let new = !seen.contains(&key);
                seen.push(key);
                new
            });
        }
    })
}

/// After the tag `name` became a collection of that name: filters asking
/// for the tag ask for the collection instead, where they can say the same
/// thing that way.
pub(crate) fn follow_tag_to_collection(conn: &Connection, name: &str) -> Result<()> {
    rewrite(conn, |filter| {
        let is = |other: &String| other.trim().eq_ignore_ascii_case(name);
        // Having none of some tags and none of some collections is having
        // none of either, so the tag simply moves over.
        if filter.without_tags.iter().any(is) {
            filter.without_tags.retain(|other| !is(other));
            if !filter.without_collections.iter().any(is) {
                filter.without_collections.push(name.to_owned());
            }
        }
        // A tag every word must have becomes a collection every word must be
        // in, where the filter has room for one; so do tags any of which
        // will do, when this is the only one.
        let required = filter.tags.iter().any(is);
        let sole = filter.any_tags.len() == 1 && filter.any_tags.iter().any(is);
        // Not where the filter keeps words out of the collection (tagged
        // `pets`, in no collection): asking for it would match nothing ever
        // again. Such a filter keeps asking for the tag, as no word has it
        // now, and matches the words tagged that way later.
        let kept_out = filter.no_collection || filter.without_collections.iter().any(is);
        if !(required || sole) || kept_out {
            return;
        }
        match &filter.collection {
            Some(collection) if is(collection) => {}
            None => filter.collection = Some(name.to_owned()),
            Some(_) if filter.any_collections.is_empty() => {
                filter.any_collections.push(name.to_owned());
            }
            // No room for a second collection every word must be in: the
            // filter keeps asking for the tag, so it matches none of these
            // words until it is changed.
            Some(_) => return,
        }
        filter.tags.retain(|other| !is(other));
        // Any of several tags, one of them required: the required one
        // already meets it.
        if filter.any_tags.iter().any(is) {
            filter.any_tags.clear();
        }
    })
}

/// Applies `change` to every filter, saving those it changed.
fn rewrite(conn: &Connection, mut change: impl FnMut(&mut LibraryFilter)) -> Result<()> {
    for smart in smart_collections(conn)? {
        let mut filter = smart.filter.clone();
        change(&mut filter);
        if filter != smart.filter {
            update_smart_collection(conn, &smart.name, &filter)?;
        }
    }
    Ok(())
}

fn read(name: String, filter: &str) -> Result<SmartCollection> {
    let filter = serde_json::from_str(filter).map_err(|err| {
        VocabError::storage(format!("the smart collection {name} can't be read: {err}"))
    })?;
    Ok(SmartCollection { name, filter })
}

fn write(filter: &LibraryFilter) -> Result<String> {
    serde_json::to_string(filter)
        .map_err(|err| VocabError::new(format!("cannot save the filter: {err}")))
}

fn not_found(name: &str) -> VocabError {
    VocabError::not_found(format!("no smart collection named {}", name.trim()))
}

#[cfg(test)]
mod tests {
    use vocab_core::{ErrorKind, LibraryFilter, LibraryView};

    use super::{
        create_smart_collection, delete_smart_collection, rename_smart_collection,
        smart_collection, smart_collections, update_smart_collection,
    };
    use crate::{
        add_tag, collection_from_tag, merge_collections, merge_tags, open_in_memory,
        rename_collection, rename_tag, save_item,
    };

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|&name| name.to_owned()).collect()
    }

    #[test]
    fn smart_collections_are_saved_renamed_and_deleted() {
        let conn = open_in_memory().unwrap();
        let filter = LibraryFilter {
            view: LibraryView::All,
            hsk_levels: vec![4],
            ..LibraryFilter::default()
        };
        create_smart_collection(&conn, " HSK 4 ", &filter).unwrap();
        let err = create_smart_collection(&conn, "hsk 4", &filter).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Conflict);
        let err = create_smart_collection(&conn, "  ", &filter).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Invalid);
        let saved = smart_collection(&conn, "hsk 4").unwrap().unwrap();
        assert_eq!(saved.name, "HSK 4");
        assert_eq!(saved.filter, filter);

        let narrower = LibraryFilter {
            hsk_levels: vec![4, 5],
            ..filter.clone()
        };
        update_smart_collection(&conn, "HSK 4", &narrower).unwrap();
        rename_smart_collection(&conn, "HSK 4", "Upper").unwrap();
        create_smart_collection(&conn, "Another", &LibraryFilter::default()).unwrap();
        let err = rename_smart_collection(&conn, "Upper", "another").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Conflict);
        rename_smart_collection(&conn, "Upper", "upper").unwrap();
        let all = smart_collections(&conn).unwrap();
        assert_eq!(
            all.iter()
                .map(|smart| smart.name.as_str())
                .collect::<Vec<_>>(),
            ["Another", "upper"]
        );
        assert_eq!(all[1].filter, narrower);

        delete_smart_collection(&conn, "UPPER").unwrap();
        let err = delete_smart_collection(&conn, "upper").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
        let err = update_smart_collection(&conn, "upper", &filter).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn filters_follow_renamed_and_merged_names() {
        let conn = open_in_memory().unwrap();
        let id = save_item(
            &conn,
            &crate::NewItem {
                simplified: "猫".to_owned(),
                traditional: String::new(),
                pinyin: String::new(),
                definition: String::new(),
                notes: String::new(),
                verification: vocab_core::Verification::Confirmed,
                source: vocab_core::ItemSource::manual(),
            },
        )
        .unwrap()
        .item()
        .id;
        for tag in ["hsk1", "HSK 1", "drilled"] {
            add_tag(&conn, id, tag).unwrap();
        }
        crate::add_to_collection(&conn, id, "Week 1").unwrap();
        crate::add_to_collection(&conn, id, "Week One").unwrap();
        create_smart_collection(
            &conn,
            "Mine",
            &LibraryFilter {
                any_tags: names(&["HSK1", "HSK 1"]),
                without_tags: names(&["drilled"]),
                collection: Some("week 1".to_owned()),
                ..LibraryFilter::default()
            },
        )
        .unwrap();
        rename_tag(&conn, "drilled", "Drilled").unwrap();
        merge_tags(&conn, "hsk1", "HSK 1").unwrap();
        merge_collections(&conn, "Week 1", "Week One").unwrap();
        rename_collection(&conn, "Week One", "First Week").unwrap();
        let filter = smart_collection(&conn, "Mine").unwrap().unwrap().filter;
        assert_eq!(filter.any_tags, names(&["HSK 1"]), "merged, and only once");
        assert_eq!(filter.without_tags, names(&["Drilled"]));
        assert_eq!(filter.collection.as_deref(), Some("First Week"));
    }

    #[test]
    fn filters_follow_a_tag_that_becomes_a_collection() {
        let conn = open_in_memory().unwrap();
        let id = save_item(
            &conn,
            &crate::NewItem {
                simplified: "猫".to_owned(),
                traditional: String::new(),
                pinyin: String::new(),
                definition: String::new(),
                notes: String::new(),
                verification: vocab_core::Verification::Confirmed,
                source: vocab_core::ItemSource::manual(),
            },
        )
        .unwrap()
        .item()
        .id;
        for tag in ["pets", "loud"] {
            add_tag(&conn, id, tag).unwrap();
        }
        let mine = LibraryFilter {
            any_tags: names(&["pets"]),
            without_tags: names(&["loud"]),
            ..LibraryFilter::default()
        };
        create_smart_collection(&conn, "Mine", &mine).unwrap();
        collection_from_tag(&conn, "pets").unwrap();
        collection_from_tag(&conn, "loud").unwrap();
        let filter = smart_collection(&conn, "Mine").unwrap().unwrap().filter;
        assert_eq!(
            filter,
            LibraryFilter {
                collection: Some("pets".to_owned()),
                without_collections: names(&["loud"]),
                ..LibraryFilter::default()
            }
        );
    }

    #[test]
    fn a_tag_becoming_a_collection_never_leaves_a_filter_asking_the_impossible() {
        let conn = open_in_memory().unwrap();
        crate::create_tag(&conn, "pets").unwrap();
        crate::create_collection(&conn, "pets").unwrap();
        let unfiled = LibraryFilter {
            tags: names(&["pets"]),
            no_collection: true,
            ..LibraryFilter::default()
        };
        let not_either = LibraryFilter {
            without_tags: names(&["pets"]),
            without_collections: names(&["Pets"]),
            ..LibraryFilter::default()
        };
        let both = LibraryFilter {
            tags: names(&["pets"]),
            collection: Some("Pets".to_owned()),
            ..LibraryFilter::default()
        };
        for (name, filter) in [
            ("Unfiled", &unfiled),
            ("Neither", &not_either),
            ("Both", &both),
        ] {
            create_smart_collection(&conn, name, filter).unwrap();
        }
        collection_from_tag(&conn, "pets").unwrap();
        let filter = |name| smart_collection(&conn, name).unwrap().unwrap().filter;
        assert_eq!(filter("Unfiled"), unfiled, "in pets and in none can't be");
        assert_eq!(
            filter("Neither"),
            LibraryFilter {
                without_collections: names(&["Pets"]),
                ..LibraryFilter::default()
            },
            "once"
        );
        assert_eq!(
            filter("Both"),
            LibraryFilter {
                collection: Some("Pets".to_owned()),
                ..LibraryFilter::default()
            }
        );
    }
}
