use std::fmt;
use std::str::FromStr;

use crate::VocabError;

/// Whether a saved word's data can be trusted as-is. Independent of
/// [`Lifecycle`]: an archived word can still need checking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum Verification {
    #[default]
    Confirmed,
    /// Ambiguous or incomplete: an inferred reading, an unresolved import, a
    /// capture with no dictionary match. Never resolved silently.
    NeedsReview,
}

impl Verification {
    /// Stored form.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::NeedsReview => "needs_review",
        }
    }

    /// Form for people.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::NeedsReview => "needs review",
        }
    }
}

impl FromStr for Verification {
    type Err = VocabError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "confirmed" => Ok(Self::Confirmed),
            "needs_review" | "needs-review" => Ok(Self::NeedsReview),
            other => Err(VocabError::invalid(format!(
                "unknown verification '{other}' (expected confirmed or needs_review)"
            ))),
        }
    }
}

impl fmt::Display for Verification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where a saved word is in its life. Derived from the item's `archived_at`
/// and `deleted_at`; trash wins over archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum Lifecycle {
    Active,
    Archived,
    Trashed,
}

impl Lifecycle {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Archived => "archived",
            Self::Trashed => "trashed",
        }
    }
}

/// How a saved word entered the library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum SourceKind {
    /// Saved from a dictionary result.
    Dictionary,
    /// Read from a Pleco, Anki, or other connector file.
    Import,
    /// Typed in by hand, or captured with no dictionary match.
    Manual,
}

impl SourceKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dictionary => "dictionary",
            Self::Import => "import",
            Self::Manual => "manual",
        }
    }
}

impl FromStr for SourceKind {
    type Err = VocabError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "dictionary" => Ok(Self::Dictionary),
            "import" => Ok(Self::Import),
            "manual" => Ok(Self::Manual),
            other => Err(VocabError::invalid(format!(
                "unknown source kind '{other}'"
            ))),
        }
    }
}

/// Provenance of a saved word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemSource {
    pub kind: SourceKind,
    /// Dictionary id (`cc-cedict`) or connector id (`pleco`).
    pub id: Option<String>,
    pub version: Option<String>,
    /// The file an import came from.
    pub import_origin: Option<String>,
}

impl ItemSource {
    #[must_use]
    pub fn manual() -> Self {
        Self {
            kind: SourceKind::Manual,
            id: None,
            version: None,
            import_origin: None,
        }
    }
}

/// A saved word. It owns its fields: a snapshot taken when it was saved, then
/// edited freely. Dictionaries are looked up beside it, never through it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VocabItem {
    pub id: i64,
    pub simplified: String,
    pub traditional: String,
    /// CC-CEDICT numbered pinyin: `xue2 xiao4`. Empty when unknown.
    pub pinyin: String,
    pub definition: String,
    pub notes: String,
    pub verification: Verification,
    /// UTC, `2026-09-30T12:00:00.000Z`.
    pub archived_at: Option<String>,
    pub deleted_at: Option<String>,
    pub source: ItemSource,
    pub created_at: String,
    pub modified_at: String,
    /// Increases with every change to the word, its tags, or its
    /// collections. Compare it to tell whether a word changed since it was
    /// read.
    pub rev: i64,
}

impl VocabItem {
    #[must_use]
    pub fn lifecycle(&self) -> Lifecycle {
        if self.deleted_at.is_some() {
            Lifecycle::Trashed
        } else if self.archived_at.is_some() {
            Lifecycle::Archived
        } else {
            Lifecycle::Active
        }
    }
}

/// A change to a saved word. `None` leaves a field as it is.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default)
)]
pub struct ItemPatch {
    pub simplified: Option<String>,
    pub traditional: Option<String>,
    pub pinyin: Option<String>,
    pub definition: Option<String>,
    pub notes: Option<String>,
    pub verification: Option<Verification>,
}

impl ItemPatch {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Which part of the library a query covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum LibraryView {
    /// Neither archived nor in the trash.
    #[default]
    Active,
    Archived,
    Trash,
    /// Active and archived; everything but the trash.
    All,
}

impl FromStr for LibraryView {
    type Err = VocabError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "active" => Ok(Self::Active),
            "archived" => Ok(Self::Archived),
            "trash" => Ok(Self::Trash),
            "all" => Ok(Self::All),
            other => Err(VocabError::invalid(format!(
                "unknown view '{other}' (expected active, archived, trash, or all)"
            ))),
        }
    }
}

/// Narrows a library listing, search, or export. Every condition must hold.
/// Every field is optional in JSON.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default)
)]
pub struct LibraryFilter {
    pub view: LibraryView,
    pub verification: Option<Verification>,
    /// Items carrying every one of these tags.
    pub tags: Vec<String>,
    pub collection: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{ItemSource, Lifecycle, Verification, VocabItem};

    fn item() -> VocabItem {
        VocabItem {
            id: 1,
            simplified: "学校".to_owned(),
            traditional: "學校".to_owned(),
            pinyin: "xue2 xiao4".to_owned(),
            definition: "school".to_owned(),
            notes: String::new(),
            verification: Verification::Confirmed,
            archived_at: None,
            deleted_at: None,
            source: ItemSource::manual(),
            created_at: String::new(),
            modified_at: String::new(),
            rev: 1,
        }
    }

    #[test]
    fn trash_wins_over_archive() {
        let mut item = item();
        assert_eq!(item.lifecycle(), Lifecycle::Active);
        item.archived_at = Some("t".to_owned());
        assert_eq!(item.lifecycle(), Lifecycle::Archived);
        item.deleted_at = Some("t".to_owned());
        assert_eq!(item.lifecycle(), Lifecycle::Trashed);
    }

    #[test]
    fn verification_round_trips() {
        for value in [Verification::Confirmed, Verification::NeedsReview] {
            assert_eq!(value.as_str().parse::<Verification>().unwrap(), value);
        }
        assert!("exported".parse::<Verification>().is_err());
    }
}
