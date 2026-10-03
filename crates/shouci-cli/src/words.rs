//! Finding saved words by what the user typed: an id or the characters.

use shouci_core::{Error, ItemView, LibraryFilter, LibraryView, Lifecycle, Result, Shouci};

/// Which saved words a command can act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Among {
    /// Any word; one outside the trash wins over one in it.
    Saved,
    /// Only words in the trash.
    Trash,
}

/// Every saved word, read once so a command naming several words lists the
/// library once.
pub struct Words {
    items: Vec<ItemView>,
}

impl Words {
    /// # Errors
    ///
    /// Storage errors.
    pub fn load(shouci: &Shouci) -> Result<Self> {
        let mut items = Vec::new();
        for view in [LibraryView::All, LibraryView::Trash] {
            items.extend(shouci.list_items(&LibraryFilter {
                view,
                ..LibraryFilter::default()
            })?);
        }
        Ok(Self { items })
    }

    /// The word `text` names: an id (`12`), or characters, simplified or
    /// traditional (`学校`, `學校`).
    ///
    /// # Errors
    ///
    /// [`shouci_core::ErrorKind::NotFound`] when nothing matches;
    /// [`shouci_core::ErrorKind::Conflict`] when the characters name
    /// several words (different readings), listing their ids.
    pub fn find(&self, text: &str, among: Among) -> Result<&ItemView> {
        let text = text.trim();
        if text.is_empty() {
            return Err(Error::invalid("name a word by its id or its characters"));
        }
        let fits = |item: &&ItemView| among == Among::Saved || item.lifecycle == Lifecycle::Trashed;
        if let Ok(id) = text.parse::<i64>() {
            return self
                .items
                .iter()
                .filter(fits)
                .find(|item| item.id == id)
                .ok_or_else(|| missing(text, among));
        }
        let named: Vec<&ItemView> = self
            .items
            .iter()
            .filter(fits)
            .filter(|item| item.simplified == text || item.traditional == text)
            .collect();
        let live: Vec<&ItemView> = named
            .iter()
            .copied()
            .filter(|item| item.lifecycle != Lifecycle::Trashed)
            .collect();
        let candidates = if live.is_empty() { named } else { live };
        match candidates.as_slice() {
            [] => Err(missing(text, among)),
            [only] => Ok(only),
            several => Err(Error::conflict(format!(
                "{} saved words are written {text}; name one by its id: {}",
                several.len(),
                several
                    .iter()
                    .map(|item| format!("{} [{}]", item.id, item.pinyin_display))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))),
        }
    }

    /// The ids of every word in `texts`, in order.
    ///
    /// # Errors
    ///
    /// The first word that cannot be found; see [`Words::find`].
    pub fn ids(&self, texts: &[String], among: Among) -> Result<Vec<i64>> {
        texts
            .iter()
            .map(|text| self.find(text, among).map(|item| item.id))
            .collect()
    }
}

fn missing(text: &str, among: Among) -> Error {
    Error::not_found(match among {
        Among::Saved => format!("no saved word {text}"),
        Among::Trash => format!("no word {text} in the trash"),
    })
}
