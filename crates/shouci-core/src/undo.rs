//! Taking a change back, and doing it again.
//!
//! A frontend reads the words a change touches before and after it
//! ([`Shouci::snapshot`]); undoing restores the first snapshot, and redoing
//! the second ([`Shouci::restore`]). One mechanism covers every library
//! change: fields, archive and trash, tags and collections, and the names a
//! rename, merge, or delete changes, since a word's tags and collections are
//! part of what is restored.
//!
//! A restore refuses, changing nothing, when a word or smart collection is
//! no longer as the change left it: edited by `shouci` or another window in
//! the meantime, say, or deleted for good. Words are compared by what they
//! hold, not by revision, so undoing several changes in turn works: each
//! restore leaves the words exactly as the change before it expects them.
//!
//! A word the change saved is one the first snapshot lacks: undoing deletes
//! it for good, and redoing puts it back with its id. Smart collections the
//! change created, deleted, renamed, or rewrote (a rename or merge renames
//! the tag or collection in every filter) are put back too.
//!
//! Names are restored only where the change made them differ: a name it
//! removed comes back, and a name it created goes once no word has it.
//! Names other changes made in between are left alone.

use std::collections::{BTreeSet, HashMap, HashSet};

use rusqlite::Connection;
use vocab_core::{ItemSource, LibraryFilter, Result, VocabItem};

use crate::dto::{ItemView, LibrarySnapshot};
use crate::library::item_views;
use crate::{Error, Shouci};

impl Shouci {
    /// The words `ids` as they are now, with every tag and collection name:
    /// what [`Shouci::restore`] puts back. Ids of words that no longer exist
    /// are left out.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn snapshot(&self, ids: &[i64]) -> Result<LibrarySnapshot> {
        let conn = self.reader()?;
        snapshot(&conn, ids)
    }

    /// Puts words, smart collections, and the tags and collections a
    /// change created or removed, back as `target` has them, all or
    /// nothing. `current` is how the change left them; restoring `current`
    /// with `target` as the current state does the change again.
    ///
    /// # Errors
    ///
    /// [`crate::ErrorKind::Conflict`] when a word or smart collection
    /// differs from `current` now, or a word was deleted for good, or its
    /// characters and reading belong to another word now; storage errors.
    /// Nothing changes then.
    pub fn restore(&self, target: &LibrarySnapshot, current: &LibrarySnapshot) -> Result<()> {
        let mut conn = self.writer()?;
        vocab_db::with_tx(&mut conn, |tx| {
            check_words(tx, target, current)?;
            // Before the names: renaming one back renames it in the filters
            // too, and those already have their old names.
            restore_smart(tx, &target.smart_filters, &current.smart_filters)?;
            let (in_target, in_current) = (by_id(target), by_id(current));
            for word in &target.words {
                if in_current.contains_key(&word.id) {
                    vocab_db::restore_item(tx, &vocab_item(word))?;
                } else {
                    vocab_db::reinsert_item(tx, &vocab_item(word))?;
                }
                vocab_db::set_tags(tx, word.id, &word.tags)?;
                vocab_db::set_collections(tx, word.id, &word.collections)?;
            }
            for word in &current.words {
                if !in_target.contains_key(&word.id) {
                    vocab_db::set_trashed(tx, word.id, true)?;
                    vocab_db::purge_item(tx, word.id)?;
                }
            }
            restore_names(tx, Names::Tags, &target.tags, &current.tags)?;
            restore_names(
                tx,
                Names::Collections,
                &target.collections,
                &current.collections,
            )?;
            Ok(())
        })
    }
}

fn snapshot(conn: &Connection, ids: &[i64]) -> Result<LibrarySnapshot> {
    let mut seen = HashSet::with_capacity(ids.len());
    let mut items = Vec::with_capacity(ids.len());
    let mut missing = Vec::new();
    for &id in ids {
        if seen.insert(id) {
            match vocab_db::get_item(conn, id)? {
                Some(item) => items.push(item),
                None => missing.push(id),
            }
        }
    }
    Ok(LibrarySnapshot {
        words: item_views(conn, items, None)?,
        missing,
        tags: Names::Tags.all(conn)?,
        collections: Names::Collections.all(conn)?,
        smart_filters: vocab_db::smart_collections(conn)?
            .into_iter()
            .map(|smart| (smart.name, smart.filter))
            .collect(),
    })
}

fn by_id(snapshot: &LibrarySnapshot) -> HashMap<i64, &ItemView> {
    snapshot.words.iter().map(|word| (word.id, word)).collect()
}

/// Every word in either snapshot is as the change left it: as `current`
/// has it, or gone where `current` never read it (a word the change saved,
/// which the snapshot before it can't hold). A word `current` found gone
/// was deleted for good while the change ran.
fn check_words(
    conn: &Connection,
    target: &LibrarySnapshot,
    current: &LibrarySnapshot,
) -> Result<()> {
    let words: Vec<&ItemView> = target.words.iter().chain(&current.words).collect();
    let ids: Vec<i64> = words.iter().map(|word| word.id).collect();
    let now = snapshot(conn, &ids)?;
    let (now, left) = (by_id(&now), by_id(current));
    for word in words {
        let gone_during = current.missing.contains(&word.id);
        match (now.get(&word.id), left.get(&word.id)) {
            (Some(found), Some(left)) if same_state(found, left) => {}
            (None, None) if !gone_during => {}
            (None, _) => {
                return Err(Error::conflict(format!(
                    "{} was deleted for good in the meantime, so nothing was changed",
                    word.simplified
                )));
            }
            _ => {
                return Err(Error::conflict(format!(
                    "{} changed in the meantime, so nothing was changed",
                    word.simplified
                )));
            }
        }
    }
    Ok(())
}

/// Puts back the smart collections the change created, deleted, renamed,
/// or rewrote, as `target` has them: each must be as the change left it
/// (`current`). A rename shows as one name gone and another new.
fn restore_smart(
    conn: &Connection,
    target: &HashMap<String, LibraryFilter>,
    current: &HashMap<String, LibraryFilter>,
) -> Result<()> {
    let touched: BTreeSet<&String> = target
        .keys()
        .chain(current.keys())
        .filter(|name| target.get(*name) != current.get(*name))
        .collect();
    if touched.is_empty() {
        return Ok(());
    }
    let now: HashMap<String, LibraryFilter> = vocab_db::smart_collections(conn)?
        .into_iter()
        .map(|smart| (smart.name, smart.filter))
        .collect();
    if let Some(name) = touched
        .iter()
        .find(|name| now.get(**name) != current.get(**name))
    {
        return Err(Error::conflict(format!(
            "the smart collection {name} changed in the meantime, so nothing was changed"
        )));
    }
    // Those it made go first, so one renamed back can take its old name.
    for name in touched.iter().filter(|name| !target.contains_key(**name)) {
        vocab_db::delete_smart_collection(conn, name)?;
    }
    for name in touched {
        match (target.get(name), current.get(name)) {
            (Some(filter), Some(_)) => vocab_db::update_smart_collection(conn, name, filter)?,
            (Some(filter), None) => vocab_db::create_smart_collection(conn, name, filter)?,
            (None, _) => {}
        }
    }
    Ok(())
}

/// Whether two views of a word hold the same: everything a restore puts
/// back. Revisions, edit times, and where the word was exported differ
/// after a restore and are not compared.
fn same_state(a: &ItemView, b: &ItemView) -> bool {
    let sorted = |names: &[String]| {
        let mut names = names.to_vec();
        names.sort();
        names
    };
    a.simplified == b.simplified
        && a.traditional == b.traditional
        && a.pinyin == b.pinyin
        && a.definition == b.definition
        && a.notes == b.notes
        && a.verification == b.verification
        && a.archived_at == b.archived_at
        && a.deleted_at == b.deleted_at
        && a.source == b.source
        && sorted(&a.tags) == sorted(&b.tags)
        && sorted(&a.collections) == sorted(&b.collections)
}

fn vocab_item(word: &ItemView) -> VocabItem {
    VocabItem {
        id: word.id,
        simplified: word.simplified.clone(),
        traditional: word.traditional.clone(),
        pinyin: word.pinyin.clone(),
        definition: word.definition.clone(),
        notes: word.notes.clone(),
        verification: word.verification,
        archived_at: word.archived_at.clone(),
        deleted_at: word.deleted_at.clone(),
        source: ItemSource {
            kind: word.source.kind,
            id: word.source.id.clone(),
            version: word.source.version.clone(),
            import_origin: word.source.import_origin.clone(),
        },
        created_at: word.created_at.clone(),
        modified_at: word.modified_at.clone(),
        rev: word.rev,
    }
}

#[derive(Clone, Copy)]
enum Names {
    Tags,
    Collections,
}

impl Names {
    fn all(self, conn: &Connection) -> Result<Vec<String>> {
        let groups = match self {
            Self::Tags => vocab_db::tags(conn)?,
            Self::Collections => vocab_db::collections(conn)?,
        };
        Ok(groups.into_iter().map(|group| group.name).collect())
    }

    /// The library's spelling of `name`, which it compares ignoring ASCII
    /// case.
    fn find(self, conn: &Connection, name: &str) -> Result<Option<String>> {
        Ok(self
            .all(conn)?
            .into_iter()
            .find(|known| known.eq_ignore_ascii_case(name)))
    }

    fn create(self, conn: &Connection, name: &str) -> Result<()> {
        match self {
            Self::Tags => vocab_db::create_tag(conn, name),
            Self::Collections => vocab_db::create_collection(conn, name),
        }
    }

    fn rename(self, conn: &Connection, from: &str, to: &str) -> Result<()> {
        match self {
            Self::Tags => vocab_db::rename_tag(conn, from, to),
            Self::Collections => vocab_db::rename_collection(conn, from, to),
        }
    }

    fn delete_if_unused(self, conn: &Connection, name: &str) -> Result<()> {
        let used = match self {
            Self::Tags => vocab_db::tag_in_use(conn, name)?,
            Self::Collections => vocab_db::collection_in_use(conn, name)?,
        };
        if used {
            return Ok(());
        }
        match self {
            Self::Tags => vocab_db::delete_tag(conn, name),
            Self::Collections => vocab_db::delete_collection(conn, name),
        }
    }
}

/// Undoes what a change did to the names: `target` and `current` are the
/// names before and after it. A name it removed comes back (renamed back,
/// when only its case changed); a name it created goes once no word has it.
fn restore_names(
    conn: &Connection,
    names: Names,
    target: &[String],
    current: &[String],
) -> Result<()> {
    let removed: Vec<&String> = target
        .iter()
        .filter(|name| !current.contains(name))
        .collect();
    let created: Vec<&String> = current
        .iter()
        .filter(|name| !target.contains(name))
        .collect();
    let recased = |name: &str| {
        created
            .iter()
            .find(|other| other.eq_ignore_ascii_case(name))
    };
    for name in &removed {
        match names.find(conn, name)? {
            Some(known) if known == **name => {}
            Some(known) if recased(name).is_some() => names.rename(conn, &known, name)?,
            Some(_) => {}
            None => names.create(conn, name)?,
        }
    }
    for name in &created {
        let renamed_back = removed.iter().any(|other| other.eq_ignore_ascii_case(name));
        if !renamed_back && names.find(conn, name)?.as_deref() == Some(name.as_str()) {
            names.delete_if_unused(conn, name)?;
        }
    }
    Ok(())
}
