//! Saved words: reading, editing, organizing, and bulk changes.

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;
use vocab_core::{ItemPatch, LibraryFilter, LibraryView, Result, VocabItem};

use crate::dictionaries::Loaded;
use crate::dto::{BulkAction, BulkResult, GroupView, ItemView};
use crate::ranks::RankTable;
use crate::{Error, Shouci};

/// One word with its tags, collections, and destinations, and its ranks
/// when a dictionary is loaded.
pub(crate) fn item_view(
    conn: &Connection,
    item: VocabItem,
    ranks: Option<&RankTable>,
) -> Result<ItemView> {
    let tags = vocab_db::item_tags(conn, item.id)?;
    let collections = vocab_db::item_collections(conn, item.id)?;
    let destinations = vocab_db::item_destinations(conn, item.id)?;
    let ranks = ranks
        .map(|table| table.of(&item.simplified))
        .unwrap_or_default();
    Ok(ItemView::new(item, tags, collections, destinations, ranks))
}

/// Many words, loading their groups in three queries instead of three each.
/// The groups are loaded for the whole library, however few `items` there
/// are: three indexed queries, cheaper than one round trip per word.
pub(crate) fn item_views(
    conn: &Connection,
    items: Vec<VocabItem>,
    ranks: Option<&RankTable>,
) -> Result<Vec<ItemView>> {
    let mut tags = vocab_db::tags_by_item(conn)?;
    let mut collections = vocab_db::collections_by_item(conn)?;
    let mut destinations: HashMap<i64, Vec<String>> = vocab_db::destinations_by_item(conn)?;
    Ok(items
        .into_iter()
        .map(|item| {
            let id = item.id;
            let word_ranks = ranks
                .map(|table| table.of(&item.simplified))
                .unwrap_or_default();
            ItemView::new(
                item,
                tags.remove(&id).unwrap_or_default(),
                collections.remove(&id).unwrap_or_default(),
                destinations.remove(&id).unwrap_or_default(),
                word_ranks,
            )
        })
        .collect())
}

/// Whether a word meets `filter`'s HSK and frequency conditions, which the
/// database can't check: the ranks come from the dictionaries. Before they
/// load, every word is at no HSK level and not in the frequency list.
pub(crate) fn admits(filter: &LibraryFilter, ranks: Option<&RankTable>, simplified: &str) -> bool {
    if !filter.asks_ranks() {
        return true;
    }
    let ranks = ranks.map(|table| table.of(simplified)).unwrap_or_default();
    filter.admits_ranks(ranks.hsk, ranks.frequency)
}

/// The words matching `filter`, HSK and frequency included, newest first.
pub(crate) fn list(
    conn: &Connection,
    filter: &LibraryFilter,
    ranks: Option<&RankTable>,
) -> Result<Vec<VocabItem>> {
    let mut items = vocab_db::list_items(conn, filter)?;
    items.retain(|item| admits(filter, ranks, &item.simplified));
    Ok(items)
}

impl Shouci {
    /// Words matching `filter`, newest first.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn list_items(&self, filter: &LibraryFilter) -> Result<Vec<ItemView>> {
        let conn = self.reader()?;
        let loaded = self.loaded_opt();
        let ranks = loaded.as_deref().map(Loaded::ranks);
        let items = list(&conn, filter, ranks)?;
        item_views(&conn, items, ranks)
    }

    /// # Errors
    ///
    /// [`crate::ErrorKind::NotFound`] or a storage error.
    pub fn item(&self, id: i64) -> Result<ItemView> {
        let conn = self.reader()?;
        let item = vocab_db::require_item(&conn, id)?;
        let loaded = self.loaded_opt();
        item_view(&conn, item, loaded.as_deref().map(Loaded::ranks))
    }

    /// Edits a word's fields. Changing its characters or reading into
    /// another saved word is refused.
    ///
    /// # Errors
    ///
    /// [`crate::ErrorKind::NotFound`], [`crate::ErrorKind::Invalid`] (no
    /// characters), [`crate::ErrorKind::Conflict`], or a storage error.
    pub fn update_item(&self, id: i64, patch: &ItemPatch) -> Result<ItemView> {
        self.edit_item(id, patch, &[])
    }

    /// Edits a word's fields and applies `actions` to it (the tags and
    /// collections it gained and lost, say), all or nothing: a refused tag
    /// name leaves the word as it was.
    ///
    /// # Errors
    ///
    /// As [`Shouci::update_item`] and [`Shouci::bulk`]; nothing changes then.
    pub fn edit_item(
        &self,
        id: i64,
        patch: &ItemPatch,
        actions: &[BulkAction],
    ) -> Result<ItemView> {
        let loaded = self.loaded_opt();
        let mut conn = self.writer()?;
        // One transaction, so another process's edit in between is never
        // overwritten with fields read before it.
        vocab_db::with_tx(&mut conn, |tx| {
            vocab_db::update_item(tx, id, patch)?;
            for action in actions {
                apply(tx, id, action)?;
            }
            let item = vocab_db::require_item(tx, id)?;
            item_view(tx, item, loaded.as_deref().map(Loaded::ranks))
        })
    }

    /// Applies `action` to every word in `ids`, all or nothing. Repeated ids
    /// count once.
    ///
    /// # Errors
    ///
    /// The first failure (an unknown id, purging a word that isn't in the
    /// trash, a blank tag name); nothing changes then.
    pub fn bulk(&self, ids: &[i64], action: &BulkAction) -> Result<BulkResult> {
        let mut seen = HashSet::with_capacity(ids.len());
        let unique: Vec<i64> = ids.iter().copied().filter(|id| seen.insert(*id)).collect();
        if unique.is_empty() {
            return Ok(BulkResult { changed: 0 });
        }
        let mut conn = self.writer()?;
        // One transaction: the first failure rolls back the words before it.
        vocab_db::with_tx(&mut conn, |tx| {
            for &id in &unique {
                apply(tx, id, action)?;
            }
            Ok(BulkResult {
                changed: crate::count(unique.len()),
            })
        })
    }

    /// Deletes every word in the trash for good.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn empty_trash(&self) -> Result<BulkResult> {
        let ids: Vec<i64> = {
            let conn = self.reader()?;
            vocab_db::list_items(
                &conn,
                &LibraryFilter {
                    view: LibraryView::Trash,
                    ..LibraryFilter::default()
                },
            )?
            .into_iter()
            .map(|item| item.id)
            .collect()
        };
        self.bulk(&ids, &BulkAction::Purge)
    }

    /// # Errors
    ///
    /// Storage errors.
    pub fn tags(&self) -> Result<Vec<GroupView>> {
        Ok(vocab_db::tags(&*self.reader()?)?
            .into_iter()
            .map(GroupView::from)
            .collect())
    }

    /// # Errors
    ///
    /// [`crate::ErrorKind::NotFound`], [`crate::ErrorKind::Conflict`] when
    /// the new name is taken, or a storage error.
    pub fn rename_tag(&self, from: &str, to: &str) -> Result<()> {
        // With the smart collections that name it.
        let mut conn = self.writer()?;
        vocab_db::with_tx(&mut conn, |tx| vocab_db::rename_tag(tx, from, to))
    }

    /// Gives every word tagged `from` the tag `into` instead, then deletes
    /// `from`: the way to fold `HSK1` into `HSK 1`.
    ///
    /// # Errors
    ///
    /// [`crate::ErrorKind::NotFound`] if either tag is missing,
    /// [`crate::ErrorKind::Invalid`] if they are the same tag, or a storage
    /// error. Nothing changes then.
    pub fn merge_tags(&self, from: &str, into: &str) -> Result<()> {
        let mut conn = self.writer()?;
        vocab_db::with_tx(&mut conn, |tx| vocab_db::merge_tags(tx, from, into))
    }

    /// Deletes a tag; its words stay.
    ///
    /// # Errors
    ///
    /// [`crate::ErrorKind::NotFound`] or a storage error.
    pub fn delete_tag(&self, name: &str) -> Result<()> {
        let mut conn = self.writer()?;
        vocab_db::with_tx(&mut conn, |tx| vocab_db::delete_tag(tx, name))
    }

    /// # Errors
    ///
    /// Storage errors.
    pub fn collections(&self) -> Result<Vec<GroupView>> {
        Ok(vocab_db::collections(&*self.reader()?)?
            .into_iter()
            .map(GroupView::from)
            .collect())
    }

    /// # Errors
    ///
    /// [`crate::ErrorKind::Conflict`] if it exists,
    /// [`crate::ErrorKind::Invalid`] for a blank name, or a storage error.
    pub fn create_collection(&self, name: &str) -> Result<()> {
        vocab_db::create_collection(&*self.writer()?, name)
    }

    /// # Errors
    ///
    /// [`crate::ErrorKind::NotFound`], [`crate::ErrorKind::Conflict`] when
    /// the new name is taken, or a storage error.
    pub fn rename_collection(&self, from: &str, to: &str) -> Result<()> {
        // With the smart collections that name it.
        let mut conn = self.writer()?;
        vocab_db::with_tx(&mut conn, |tx| vocab_db::rename_collection(tx, from, to))
    }

    /// Puts every word of collection `from` into `into`, then deletes
    /// `from`.
    ///
    /// # Errors
    ///
    /// [`crate::ErrorKind::NotFound`] if either collection is missing,
    /// [`crate::ErrorKind::Invalid`] if they are the same collection, or a
    /// storage error. Nothing changes then.
    pub fn merge_collections(&self, from: &str, into: &str) -> Result<()> {
        let mut conn = self.writer()?;
        vocab_db::with_tx(&mut conn, |tx| vocab_db::merge_collections(tx, from, into))
    }

    /// Deletes a collection; its words stay.
    ///
    /// # Errors
    ///
    /// [`crate::ErrorKind::NotFound`] or a storage error.
    pub fn delete_collection(&self, name: &str) -> Result<()> {
        let mut conn = self.writer()?;
        vocab_db::with_tx(&mut conn, |tx| vocab_db::delete_collection(tx, name))
    }

    /// Turns a tag into a collection of the same name (the tag goes away).
    ///
    /// # Errors
    ///
    /// [`crate::ErrorKind::NotFound`] if there is no such tag, or a storage
    /// error.
    pub fn collection_from_tag(&self, tag: &str) -> Result<GroupView> {
        let mut conn = self.writer()?;
        vocab_db::with_tx(&mut conn, |tx| {
            Ok(vocab_db::collection_from_tag(tx, tag)?.into())
        })
    }
}

fn apply(conn: &Connection, id: i64, action: &BulkAction) -> Result<()> {
    match action {
        BulkAction::SetVerification(verification) => {
            vocab_db::set_verification(conn, id, *verification)
        }
        BulkAction::Archive => vocab_db::set_archived(conn, id, true),
        BulkAction::Unarchive => vocab_db::set_archived(conn, id, false),
        BulkAction::Trash => vocab_db::set_trashed(conn, id, true),
        BulkAction::Restore => vocab_db::set_trashed(conn, id, false),
        BulkAction::Purge => vocab_db::purge_item(conn, id),
        BulkAction::AddTags(names) => names
            .iter()
            .try_for_each(|name| vocab_db::add_tag(conn, id, name)),
        BulkAction::RemoveTags(names) => {
            vocab_db::require_item(conn, id)?;
            names
                .iter()
                .try_for_each(|name| vocab_db::remove_tag(conn, id, name))
        }
        BulkAction::AddToCollection(name) => vocab_db::add_to_collection(conn, id, name),
        BulkAction::RemoveFromCollection(name) => {
            vocab_db::require_item(conn, id)?;
            vocab_db::remove_from_collection(conn, id, name)
        }
    }
    .map_err(|err| {
        if err.kind() == crate::ErrorKind::NotFound {
            Error::not_found(format!("no saved word {id}; nothing was changed"))
        } else {
            err
        }
    })
}
