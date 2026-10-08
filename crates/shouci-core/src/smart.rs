//! Smart collections: saved filters. A smart collection keeps no words of
//! its own; its words are whatever its filter matches when it is looked at,
//! so they are never out of date.

use rusqlite::Connection;
use vocab_core::{LibraryFilter, LibraryView, Result};

use crate::dictionaries::Loaded;
use crate::dto::SmartCollectionView;
use crate::library::admits;
use crate::ranks::RankTable;
use crate::{Error, Shouci};

impl Shouci {
    /// The words matching `filter`, HSK and frequency included, newest
    /// first: what a frontend that holds the library itself needs to show a
    /// filter. Before the dictionaries load, no word has an HSK level or a
    /// frequency rank.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn matching_ids(&self, filter: &LibraryFilter) -> Result<Vec<i64>> {
        let conn = self.reader()?;
        let loaded = self.loaded_opt();
        matching(&conn, filter, loaded.as_deref().map(Loaded::ranks))
    }

    /// Every smart collection, by name, with the words each matches now.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn smart_collections(&self) -> Result<Vec<SmartCollectionView>> {
        let conn = self.reader()?;
        let loaded = self.loaded_opt();
        let ranks = loaded.as_deref().map(Loaded::ranks);
        vocab_db::smart_collections(&conn)?
            .into_iter()
            .map(|smart| view(&conn, smart, ranks))
            .collect()
    }

    /// The smart collection called `name` (case aside).
    ///
    /// # Errors
    ///
    /// [`crate::ErrorKind::NotFound`] or a storage error.
    pub fn smart_collection(&self, name: &str) -> Result<SmartCollectionView> {
        let conn = self.reader()?;
        let smart = vocab_db::smart_collection(&conn, name)?.ok_or_else(|| {
            Error::not_found(format!("no smart collection named {}", name.trim()))
        })?;
        let loaded = self.loaded_opt();
        view(&conn, smart, loaded.as_deref().map(Loaded::ranks))
    }

    /// Saves `filter` as a smart collection.
    ///
    /// # Errors
    ///
    /// [`crate::ErrorKind::Conflict`] if a smart collection has the name,
    /// [`crate::ErrorKind::Invalid`] for a blank name or a filter a smart
    /// collection can't have (one asking for the Trash, an HSK level past
    /// 7), or a storage error.
    pub fn create_smart_collection(
        &self,
        name: &str,
        filter: &LibraryFilter,
    ) -> Result<SmartCollectionView> {
        check(filter)?;
        vocab_db::create_smart_collection(&*self.writer()?, name, filter)?;
        self.smart_collection(name)
    }

    /// Gives a smart collection another filter.
    ///
    /// # Errors
    ///
    /// [`crate::ErrorKind::NotFound`], [`crate::ErrorKind::Invalid`] as for
    /// [`Shouci::create_smart_collection`], or a storage error.
    pub fn update_smart_collection(
        &self,
        name: &str,
        filter: &LibraryFilter,
    ) -> Result<SmartCollectionView> {
        check(filter)?;
        vocab_db::update_smart_collection(&*self.writer()?, name, filter)?;
        self.smart_collection(name)
    }

    /// # Errors
    ///
    /// [`crate::ErrorKind::NotFound`], [`crate::ErrorKind::Conflict`] when
    /// another smart collection has the new name,
    /// [`crate::ErrorKind::Invalid`] for a blank one, or a storage error.
    pub fn rename_smart_collection(&self, from: &str, to: &str) -> Result<()> {
        vocab_db::rename_smart_collection(&*self.writer()?, from, to)
    }

    /// Deletes a smart collection. Its words stay as they are.
    ///
    /// # Errors
    ///
    /// [`crate::ErrorKind::NotFound`] or a storage error.
    pub fn delete_smart_collection(&self, name: &str) -> Result<()> {
        vocab_db::delete_smart_collection(&*self.writer()?, name)
    }
}

fn matching(
    conn: &Connection,
    filter: &LibraryFilter,
    ranks: Option<&RankTable>,
) -> Result<Vec<i64>> {
    Ok(vocab_db::list_headwords(conn, filter)?
        .into_iter()
        .filter(|(_, simplified)| admits(filter, ranks, simplified))
        .map(|(id, _)| id)
        .collect())
}

fn view(
    conn: &Connection,
    smart: vocab_db::SmartCollection,
    ranks: Option<&RankTable>,
) -> Result<SmartCollectionView> {
    let item_ids = matching(conn, &smart.filter, ranks)?;
    Ok(SmartCollectionView {
        name: smart.name,
        filter: smart.filter,
        item_ids,
    })
}

/// A filter a smart collection can keep.
fn check(filter: &LibraryFilter) -> Result<()> {
    if filter.view == LibraryView::Trash {
        return Err(Error::invalid("a smart collection can't show the Trash"));
    }
    if let Some(level) = filter.hsk_levels.iter().find(|&&level| level > 7) {
        return Err(Error::invalid(format!(
            "there is no HSK level {level}: levels run from 1 to 6, then 7 for 7–9, or 0 for none"
        )));
    }
    Ok(())
}
