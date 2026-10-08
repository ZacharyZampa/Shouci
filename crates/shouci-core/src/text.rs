//! Wording shared by the frontends, and helpers for frontends that format
//! raw fields themselves.

pub use vocab_dictionary::display_definition;
pub use vocab_pinyin::tone_marks;
pub use vocab_search::has_han;

use vocab_core::{FrequencyBand, LibraryFilter, LibraryView, Verification};
use vocab_exchange::TransferSummary;

use crate::dto::{FilterConditionView, FrequencyBandView, HskLevelView};

/// `1 word`, `3 words`: the count with its noun, singular when it is one.
#[must_use]
pub fn counted(count: impl Into<u64>, singular: &str, plural: &str) -> String {
    match count.into() {
        1 => format!("1 {singular}"),
        count => format!("{count} {plural}"),
    }
}

/// An HSK level as people know it: `HSK 3`, and `HSK 7–9` for the advanced
/// band, which the dictionaries store as 7.
#[must_use]
pub fn hsk_level(rank: u64) -> String {
    format!("HSK {}", hsk_band(rank))
}

/// `3`, or `7–9`.
fn hsk_band(rank: u64) -> String {
    if rank >= 7 {
        "7–9".to_owned()
    } else {
        rank.to_string()
    }
}

/// The bands of the frequency list, most common first, with their names:
/// a word carries its band ([`crate::ItemView::frequency_band`]), and these
/// name it.
#[must_use]
pub fn frequency_bands() -> Vec<FrequencyBandView> {
    FrequencyBand::ALL
        .into_iter()
        .map(|band| FrequencyBandView {
            band,
            label: band.label().to_owned(),
            short_label: band.short_label().to_owned(),
        })
        .collect()
}

/// The HSK levels a filter can ask for, in order, as
/// [`LibraryFilter::hsk_levels`] holds them: 1–6, 7 for the advanced band,
/// and 0 for words at no level.
#[must_use]
pub fn hsk_levels() -> Vec<HskLevelView> {
    (1..=7)
        .chain([0])
        .map(|level| HskLevelView {
            level,
            label: if level == 0 {
                "None".to_owned()
            } else {
                hsk_band(level)
            },
        })
        .collect()
}

/// A filter's conditions, each worded for people and with the filter that
/// taking it off leaves: the Mac window's chips, a line of `shouci smart`.
/// They come in the order the Mac filter panel lists them; which words the
/// filter covers (archived ones too) comes last, as it qualifies the rest.
#[must_use]
pub fn filter_conditions(filter: &LibraryFilter) -> Vec<FilterConditionView> {
    let mut conditions = Vec::new();
    let mut add = |id: &str, label: String, change: &dyn Fn(&mut LibraryFilter)| {
        let mut without = filter.clone();
        change(&mut without);
        conditions.push(FilterConditionView {
            id: id.to_owned(),
            label,
            without,
        });
    };
    if !filter.hsk_levels.is_empty() {
        let label = hsk_condition(&filter.hsk_levels);
        add("hsk", label, &|f| f.hsk_levels.clear());
    }
    if !filter.frequency_bands.is_empty() {
        let label = frequency_condition(&filter.frequency_bands);
        add("frequency", label, &|f| f.frequency_bands.clear());
    }
    if let Some(verification) = filter.verification {
        let label = match verification {
            Verification::NeedsReview => "Needs Review",
            Verification::Confirmed => "Confirmed",
        };
        add("verification", label.to_owned(), &|f| f.verification = None);
    }
    for tag in &filter.tags {
        let (id, label) = (format!("tag {tag}"), format!("Tagged {tag}"));
        add(&id, label, &|f| f.tags.retain(|other| other != tag));
    }
    if !filter.any_tags.is_empty() {
        let label = format!("Tagged {}", or_list(&filter.any_tags));
        add("any_tags", label, &|f| f.any_tags.clear());
    }
    if !filter.without_tags.is_empty() {
        let label = format!("Not tagged {}", or_list(&filter.without_tags));
        add("without_tags", label, &|f| f.without_tags.clear());
    }
    if let Some(collection) = &filter.collection {
        let label = format!("In {collection}");
        add("collection", label, &|f| f.collection = None);
    }
    if !filter.any_collections.is_empty() {
        let label = format!("In {}", or_list(&filter.any_collections));
        add("any_collections", label, &|f| f.any_collections.clear());
    }
    if !filter.without_collections.is_empty() {
        let label = format!("Not in {}", or_list(&filter.without_collections));
        add("without_collections", label, &|f| {
            f.without_collections.clear();
        });
    }
    if filter.no_collection {
        let label = "In no collection".to_owned();
        add("no_collection", label, &|f| f.no_collection = false);
    }
    if let Some(days) = filter.added_within_days {
        let label = added_condition(days);
        add("added", label, &|f| f.added_within_days = None);
    }
    let view = match filter.view {
        LibraryView::Active => None,
        LibraryView::Archived => Some("Archived only"),
        LibraryView::All => Some("Including archived"),
        LibraryView::Trash => Some("In the Trash"),
    };
    if let Some(label) = view {
        add("view", label.to_owned(), &|f| f.view = LibraryView::Active);
    }
    conditions
}

/// `HSK 4 or 5`, `HSK 4, 7–9, or none`, `No HSK level`.
fn hsk_condition(levels: &[u64]) -> String {
    let mut levels: Vec<u64> = levels.iter().map(|&level| level.min(7)).collect();
    // Words at no level last.
    levels.sort_unstable_by_key(|&level| if level == 0 { u64::MAX } else { level });
    levels.dedup();
    if levels == [0] {
        return "No HSK level".to_owned();
    }
    let names: Vec<String> = levels
        .into_iter()
        .map(|level| match level {
            0 => "none".to_owned(),
            level => hsk_band(level),
        })
        .collect();
    format!("HSK {}", or_list(&names))
}

/// `Top 1,000`, or for several bands their short names: `Top 1k or 1k–5k`.
fn frequency_condition(bands: &[FrequencyBand]) -> String {
    let bands: Vec<FrequencyBand> = FrequencyBand::ALL
        .into_iter()
        .filter(|band| bands.contains(band))
        .collect();
    if let [band] = bands.as_slice() {
        return band.label().to_owned();
    }
    let names: Vec<String> = bands
        .iter()
        .map(|band| band.short_label().to_owned())
        .collect();
    or_list(&names)
}

/// `Added in the past week`, `Added in the last 12 days`.
fn added_condition(days: u32) -> String {
    let period = match days {
        1 => "the past day",
        7 => "the past week",
        30 => "the past month",
        90 => "the past 3 months",
        365 => "the past year",
        days => return format!("Added in the last {days} days"),
    };
    format!("Added in {period}")
}

/// `a`, `a or b`, `a, b, or c`.
fn or_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [first, second] => format!("{first} or {second}"),
        [rest @ .., last] => format!("{}, or {last}", rest.join(", ")),
    }
}

/// `3 words (1 needs review)`.
#[must_use]
pub fn with_review(words: u32, review: u32) -> String {
    let words = counted(words, "word", "words");
    match review {
        0 => words,
        1 => format!("{words} (1 needs review)"),
        review => format!("{words} ({review} need review)"),
    }
}

/// What an import changed: `added 3 words (1 needs review), updated 2,
/// skipped 4`. Counts of zero are left out, except for the words added.
#[must_use]
pub fn import_outcome(summary: &TransferSummary) -> String {
    let mut parts = vec![format!(
        "added {}",
        with_review(summary.inserted, summary.unresolved)
    )];
    for (count, done) in [
        (summary.updated, "updated"),
        (summary.skipped, "skipped"),
        (summary.dropped, "dropped"),
    ] {
        if count > 0 {
            parts.push(format!("{done} {count}"));
        }
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsk_levels_are_offered_as_filters_hold_them() {
        let levels: Vec<(u64, String)> = hsk_levels()
            .into_iter()
            .map(|level| (level.level, level.label))
            .collect();
        assert_eq!(levels.len(), 8);
        assert_eq!(levels[3], (4, "4".to_owned()));
        assert_eq!(levels[6], (7, "7–9".to_owned()));
        assert_eq!(levels[7], (0, "None".to_owned()));
    }

    #[test]
    fn a_filter_reads_as_what_it_asks_for() {
        assert_eq!(filter_conditions(&LibraryFilter::default()), Vec::new());
        let filter = LibraryFilter {
            view: LibraryView::All,
            hsk_levels: vec![0, 7, 4, 8],
            frequency_bands: vec![FrequencyBand::To5000, FrequencyBand::Top1000],
            tags: vec!["pets".to_owned()],
            without_collections: vec!["Week 1".to_owned(), "Week 2".to_owned()],
            added_within_days: Some(30),
            ..LibraryFilter::default()
        };
        let conditions = filter_conditions(&filter);
        let labels: Vec<&str> = conditions.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "HSK 4, 7–9, or none",
                "Top 1k or 1k–5k",
                "Tagged pets",
                "Not in Week 1 or Week 2",
                "Added in the past month",
                "Including archived",
            ]
        );
        assert_eq!(
            conditions[2].without,
            LibraryFilter {
                tags: Vec::new(),
                ..filter.clone()
            },
            "taking one off leaves the rest"
        );
        let none = LibraryFilter {
            hsk_levels: vec![0],
            frequency_bands: vec![FrequencyBand::Unlisted],
            ..LibraryFilter::default()
        };
        let labels: Vec<String> = filter_conditions(&none)
            .into_iter()
            .map(|c| c.label)
            .collect();
        assert_eq!(labels, ["No HSK level", "Not in the frequency list"]);
    }

    #[test]
    fn the_advanced_hsk_band_is_one_level() {
        assert_eq!(hsk_level(1), "HSK 1");
        assert_eq!(hsk_level(7), "HSK 7–9");
    }

    #[test]
    fn counts_take_the_singular_only_for_one() {
        assert_eq!(counted(1_u32, "word", "words"), "1 word");
        assert_eq!(counted(0_u32, "word", "words"), "0 words");
    }

    #[test]
    fn an_import_outcome_leaves_out_what_did_not_happen() {
        let mut summary = TransferSummary {
            inserted: 3,
            ..TransferSummary::default()
        };
        assert_eq!(import_outcome(&summary), "added 3 words");
        summary.unresolved = 1;
        summary.skipped = 4;
        assert_eq!(
            import_outcome(&summary),
            "added 3 words (1 needs review), skipped 4"
        );
        summary.unresolved = 2;
        summary.updated = 1;
        summary.dropped = 1;
        assert_eq!(
            import_outcome(&summary),
            "added 3 words (2 need review), updated 1, skipped 4, dropped 1"
        );
    }
}
