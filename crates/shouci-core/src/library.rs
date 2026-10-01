//! Saved words: reading, editing, organizing, and bulk changes.

use std::collections::HashMap;

use rusqlite::Connection;
use vocab_core::{ItemPatch, LibraryFilter, LibraryView, Result, VocabItem};

use crate::dto::{BulkAction, BulkResult, GroupView, ItemView};
use crate::{Error, Shouci};

/// One word with its tags, collections, and destinations.
pub(crate) fn item_view(conn: &Connection, item: VocabItem) -> Result<ItemView> {
    let tags = vocab_db::item_tags(conn, item.id)?;
    let collections = vocab_db::item_collections(conn, item.id)?;
    let destinations = vocab_db::item_destinations(conn, item.id)?;
    Ok(ItemView::new(item, tags, collections, destinations))
}

/// Many words, loading their groups in three queries instead of three each.
pub(crate) fn item_views(conn: &Connection, items: Vec<VocabItem>) -> Result<Vec<ItemView>> {
    let mut tags = vocab_db::tags_by_item(conn)?;
    let mut collections = vocab_db::collections_by_item(conn)?;
    let mut destinations: HashMap<i64, Vec<String>> = vocab_db::destinations_by_item(conn)?;
    Ok(items
        .into_iter()
        .map(|item| {
            let id = item.id;
            ItemView::new(
                item,
                tags.remove(&id).unwrap_or_default(),
                collections.remove(&id).unwrap_or_default(),
                destinations.remove(&id).unwrap_or_default(),
            )
        })
        .collect())
}

impl Shouci {
    /// Words matching `filter`, newest first.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn list_items(&self, filter: &LibraryFilter) -> Result<Vec<ItemView>> {
        let conn = self.db()?;
        let items = vocab_db::list_items(&conn, filter)?;
        item_views(&conn, items)
    }

    /// # Errors
    ///
    /// [`crate::ErrorKind::NotFound`] or a storage error.
    pub fn item(&self, id: i64) -> Result<ItemView> {
        let conn = self.db()?;
        let item = vocab_db::require_item(&conn, id)?;
        item_view(&conn, item)
    }

    /// Edits a word's fields. Changing its characters or reading into
    /// another saved word is refused.
    ///
    /// # Errors
    ///
    /// [`crate::ErrorKind::NotFound`], [`crate::ErrorKind::Invalid`] (no
    /// characters), [`crate::ErrorKind::Conflict`], or a storage error.
    pub fn update_item(&self, id: i64, patch: &ItemPatch) -> Result<ItemView> {
        let conn = self.db()?;
        let item = vocab_db::update_item(&conn, id, patch)?;
        item_view(&conn, item)
    }

    /// Applies `action` to every word in `ids`, all or nothing.
    ///
    /// # Errors
    ///
    /// The first failure (an unknown id, purging a word that isn't in the
    /// trash, a blank tag name); nothing changes then.
    pub fn bulk(&self, ids: &[i64], action: &BulkAction) -> Result<BulkResult> {
        if ids.is_empty() {
            return Ok(BulkResult { changed: 0 });
        }
        let mut conn = self.db()?;
        vocab_db::with_tx(&mut conn, |tx| {
            for &id in ids {
                apply(tx, id, action)?;
            }
            Ok(BulkResult { changed: ids.len() })
        })
    }

    /// Deletes every word in the trash for good.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn empty_trash(&self) -> Result<BulkResult> {
        let ids: Vec<i64> = {
            let conn = self.db()?;
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
        Ok(vocab_db::tags(&*self.db()?)?
            .into_iter()
            .map(GroupView::from)
            .collect())
    }

    /// # Errors
    ///
    /// [`crate::ErrorKind::NotFound`], [`crate::ErrorKind::Conflict`] when
    /// the new name is taken, or a storage error.
    pub fn rename_tag(&self, from: &str, to: &str) -> Result<()> {
        vocab_db::rename_tag(&*self.db()?, from, to)
    }

    /// Deletes a tag; its words stay.
    ///
    /// # Errors
    ///
    /// [`crate::ErrorKind::NotFound`] or a storage error.
    pub fn delete_tag(&self, name: &str) -> Result<()> {
        vocab_db::delete_tag(&*self.db()?, name)
    }

    /// # Errors
    ///
    /// Storage errors.
    pub fn collections(&self) -> Result<Vec<GroupView>> {
        Ok(vocab_db::collections(&*self.db()?)?
            .into_iter()
            .map(GroupView::from)
            .collect())
    }

    /// # Errors
    ///
    /// [`crate::ErrorKind::Conflict`] if it exists,
    /// [`crate::ErrorKind::Invalid`] for a blank name, or a storage error.
    pub fn create_collection(&self, name: &str) -> Result<()> {
        vocab_db::create_collection(&*self.db()?, name)
    }

    /// # Errors
    ///
    /// [`crate::ErrorKind::NotFound`], [`crate::ErrorKind::Conflict`] when
    /// the new name is taken, or a storage error.
    pub fn rename_collection(&self, from: &str, to: &str) -> Result<()> {
        vocab_db::rename_collection(&*self.db()?, from, to)
    }

    /// Deletes a collection; its words stay.
    ///
    /// # Errors
    ///
    /// [`crate::ErrorKind::NotFound`] or a storage error.
    pub fn delete_collection(&self, name: &str) -> Result<()> {
        vocab_db::delete_collection(&*self.db()?, name)
    }

    /// Turns a tag into a collection of the same name (the tag goes away).
    ///
    /// # Errors
    ///
    /// [`crate::ErrorKind::NotFound`] if there is no such tag, or a storage
    /// error.
    pub fn collection_from_tag(&self, tag: &str) -> Result<GroupView> {
        let mut conn = self.db()?;
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
