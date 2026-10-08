use std::collections::BTreeSet;
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

/// Where a word stands in the dictionaries' frequency list, in the bands
/// the library sorts and filters by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum FrequencyBand {
    /// The 1,000 most common words.
    Top1000,
    /// Ranks 1,001–5,000.
    To5000,
    /// Ranks 5,001–10,000.
    To10000,
    /// Past rank 10,000.
    Beyond10000,
    /// Not in the list at all.
    Unlisted,
}

impl FrequencyBand {
    /// Most common first.
    pub const ALL: [Self; 5] = [
        Self::Top1000,
        Self::To5000,
        Self::To10000,
        Self::Beyond10000,
        Self::Unlisted,
    ];

    /// The band of a frequency rank (1 the most common); `None` is a word
    /// not in the list.
    #[must_use]
    pub fn of(rank: Option<u64>) -> Self {
        match rank {
            None => Self::Unlisted,
            Some(rank) => Self::ALL
                .into_iter()
                .find(|band| band.ceiling().is_some_and(|ceiling| rank <= ceiling))
                .unwrap_or(Self::Beyond10000),
        }
    }

    /// The last rank in the band; `None` for the open-ended bands.
    #[must_use]
    pub fn ceiling(self) -> Option<u64> {
        match self {
            Self::Top1000 => Some(1_000),
            Self::To5000 => Some(5_000),
            Self::To10000 => Some(10_000),
            Self::Beyond10000 | Self::Unlisted => None,
        }
    }

    /// Form for people: `Top 1,000`, `1,001–5,000`.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Top1000 => "Top 1,000",
            Self::To5000 => "1,001–5,000",
            Self::To10000 => "5,001–10,000",
            Self::Beyond10000 => "Beyond 10,000",
            Self::Unlisted => "Not in the frequency list",
        }
    }

    /// Form for people where space is short: `Top 1k`, `1k–5k`.
    #[must_use]
    pub fn short_label(self) -> &'static str {
        match self {
            Self::Top1000 => "Top 1k",
            Self::To5000 => "1k–5k",
            Self::To10000 => "5k–10k",
            Self::Beyond10000 => "10k+",
            Self::Unlisted => "Unlisted",
        }
    }
}

/// Narrows a library listing, search, or export. Every condition must hold;
/// within one that names several values, any of them will do. Every field
/// is optional in JSON.
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
    /// Only items in no collection: the ones not yet sorted into one.
    pub no_collection: bool,
    /// Items carrying at least one of these tags.
    pub any_tags: Vec<String>,
    /// Items carrying none of these tags.
    pub without_tags: Vec<String>,
    /// Items in at least one of these collections.
    pub any_collections: Vec<String>,
    /// Items in none of these collections.
    pub without_collections: Vec<String>,
    /// Items added in the last this many days.
    pub added_within_days: Option<u32>,
    /// Items at any of these HSK levels: 1–6, 7 for the advanced band
    /// (7–9), and 0 for words at none. Levels come from the dictionaries,
    /// so the core checks this, not the database ([`Self::admits_ranks`]).
    pub hsk_levels: Vec<u64>,
    /// Items in any of these bands of the frequency list. Checked by the
    /// core, like `hsk_levels`.
    pub frequency_bands: Vec<FrequencyBand>,
}

impl LibraryFilter {
    /// Whether anything narrows the words, besides which of them (active,
    /// archived, or both) the filter covers.
    #[must_use]
    pub fn has_conditions(&self) -> bool {
        *self
            != Self {
                view: self.view,
                ..Self::default()
            }
    }

    /// Whether two filters ask for the same words: values in any order, and
    /// names compared as the library compares them (trimmed, ignoring ASCII
    /// case).
    #[must_use]
    pub fn same_conditions(&self, other: &Self) -> bool {
        self.normalized() == other.normalized()
    }

    /// Every value once, in order, and names lowercased.
    fn normalized(&self) -> Self {
        fn names(names: &[String]) -> Vec<String> {
            let names: BTreeSet<String> = names
                .iter()
                .map(|name| name.trim().to_ascii_lowercase())
                .collect();
            names.into_iter().collect()
        }
        fn sorted<T: Ord + Copy>(values: &[T]) -> Vec<T> {
            let values: BTreeSet<T> = values.iter().copied().collect();
            values.into_iter().collect()
        }
        Self {
            tags: names(&self.tags),
            collection: self
                .collection
                .as_deref()
                .map(|name| name.trim().to_ascii_lowercase()),
            any_tags: names(&self.any_tags),
            without_tags: names(&self.without_tags),
            any_collections: names(&self.any_collections),
            without_collections: names(&self.without_collections),
            hsk_levels: sorted(&self.hsk_levels),
            frequency_bands: sorted(&self.frequency_bands),
            ..self.clone()
        }
    }

    /// Whether the filter asks for HSK levels or frequency bands, which only
    /// the dictionaries can answer.
    #[must_use]
    pub fn asks_ranks(&self) -> bool {
        !self.hsk_levels.is_empty() || !self.frequency_bands.is_empty()
    }

    /// Whether a word with these ranks meets the HSK and frequency
    /// conditions. `hsk` is the dictionaries' level (7 and up is the
    /// advanced band); `frequency` its place in the frequency list.
    #[must_use]
    pub fn admits_ranks(&self, hsk: Option<u64>, frequency: Option<u64>) -> bool {
        let level = hsk.map_or(0, |level| level.min(7));
        (self.hsk_levels.is_empty() || self.hsk_levels.contains(&level))
            && (self.frequency_bands.is_empty()
                || self.frequency_bands.contains(&FrequencyBand::of(frequency)))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FrequencyBand, ItemSource, LibraryFilter, LibraryView, Lifecycle, Verification, VocabItem,
    };

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

    #[test]
    fn frequency_bands_end_at_their_ceilings() {
        assert_eq!(FrequencyBand::of(Some(1)), FrequencyBand::Top1000);
        assert_eq!(FrequencyBand::of(Some(1_000)), FrequencyBand::Top1000);
        assert_eq!(FrequencyBand::of(Some(1_001)), FrequencyBand::To5000);
        assert_eq!(FrequencyBand::of(Some(10_000)), FrequencyBand::To10000);
        assert_eq!(FrequencyBand::of(Some(10_001)), FrequencyBand::Beyond10000);
        assert_eq!(FrequencyBand::of(None), FrequencyBand::Unlisted);
    }

    #[test]
    fn rank_conditions_take_any_of_their_values() {
        let filter = LibraryFilter {
            hsk_levels: vec![4, 0],
            frequency_bands: vec![FrequencyBand::Top1000, FrequencyBand::Unlisted],
            ..LibraryFilter::default()
        };
        assert!(filter.asks_ranks());
        assert!(filter.admits_ranks(Some(4), Some(900)));
        assert!(filter.admits_ranks(None, None), "no level, not listed");
        assert!(!filter.admits_ranks(Some(3), Some(900)));
        assert!(!filter.admits_ranks(Some(4), Some(2_000)));
        let advanced = LibraryFilter {
            hsk_levels: vec![7],
            ..LibraryFilter::default()
        };
        assert!(advanced.admits_ranks(Some(9), None), "7–9 is one band");
        assert!(!LibraryFilter::default().asks_ranks());
        assert!(LibraryFilter::default().admits_ranks(None, None));
    }

    #[test]
    fn conditions_compare_whatever_their_order() {
        let archived = LibraryFilter {
            view: LibraryView::Archived,
            ..LibraryFilter::default()
        };
        assert!(!archived.has_conditions(), "which words, not a condition");
        let filter = LibraryFilter {
            any_tags: vec!["Pets".to_owned(), "food".to_owned()],
            hsk_levels: vec![4, 1],
            ..LibraryFilter::default()
        };
        assert!(filter.has_conditions());
        let reordered = LibraryFilter {
            any_tags: vec!["food".to_owned(), " pets".to_owned(), "FOOD".to_owned()],
            hsk_levels: vec![1, 4],
            ..LibraryFilter::default()
        };
        assert!(filter.same_conditions(&reordered));
        assert!(!filter.same_conditions(&LibraryFilter {
            view: LibraryView::All,
            ..reordered
        }));
    }
}
