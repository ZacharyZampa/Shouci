//! Wording shared by the text frontends (CLI and TUI), and helpers for
//! frontends that format raw fields themselves.

pub use vocab_dictionary::display_definition;
pub use vocab_pinyin::tone_marks;
pub use vocab_search::has_han;

use vocab_exchange::TransferSummary;

/// `1 word`, `3 words`: the count with its noun, singular when it is one.
#[must_use]
pub fn counted(count: impl Into<u64>, singular: &str, plural: &str) -> String {
    match count.into() {
        1 => format!("1 {singular}"),
        count => format!("{count} {plural}"),
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
