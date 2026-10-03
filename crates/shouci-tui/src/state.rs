//! View state, the saved-word lists, and selection helpers.
//!
//! Pure state logic lives here so it can be unit-tested without a terminal.
//! Rendering lives in `ui`; the event loop and core calls live in `app`.

use ratatui::{text::Line, widgets::ListState};
use shouci_core::{
    CandidateView, DictionaryResults, ItemView, LibraryFilter, LibraryView, MatchBasis, QueryKind,
    Verification,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum View {
    Search,
    Saved,
}

/// Which saved words the saved view lists. Tab steps through them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
    /// Everything not archived or in the trash.
    All,
    NeedsReview,
    Archived,
    Trash,
}

impl Scope {
    pub(crate) fn next(self) -> Self {
        match self {
            Self::All => Self::NeedsReview,
            Self::NeedsReview => Self::Archived,
            Self::Archived => Self::Trash,
            Self::Trash => Self::All,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::NeedsReview => "needs review",
            Self::Archived => "archived",
            Self::Trash => "trash",
        }
    }

    pub(crate) fn filter(self) -> LibraryFilter {
        let (view, verification) = match self {
            Self::All => (LibraryView::Active, None),
            Self::NeedsReview => (LibraryView::Active, Some(Verification::NeedsReview)),
            Self::Archived => (LibraryView::Archived, None),
            Self::Trash => (LibraryView::Trash, None),
        };
        LibraryFilter {
            view,
            verification,
            ..LibraryFilter::default()
        }
    }
}

/// A line in the search results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Row {
    Candidate(Box<CandidateView>),
    /// No dictionary has the word: keep it as typed, to fill in later.
    AsTyped(String),
}

impl Row {
    /// Stable identity across re-searches.
    pub(crate) fn key(&self) -> String {
        match self {
            Self::Candidate(candidate) => format!(
                "{}\u{1f}{}\u{1f}{}",
                candidate.simplified, candidate.traditional, candidate.pinyin
            ),
            Self::AsTyped(text) => format!("typed\u{1f}{text}"),
        }
    }
}

/// The rows for a search: the dictionary's results, led by a row that keeps
/// the characters as typed when no dictionary has them.
pub(crate) fn rows(found: &DictionaryResults) -> Vec<Row> {
    let strong = found.candidates.iter().any(|candidate| !candidate.inferred);
    let mut rows = Vec::with_capacity(found.candidates.len() + 1);
    if !strong && found.kind == QueryKind::Chinese && has_han(&found.query) {
        rows.push(Row::AsTyped(found.query.clone()));
    }
    rows.extend(
        found
            .candidates
            .iter()
            .map(|candidate| Row::Candidate(Box::new(candidate.clone()))),
    );
    rows
}

/// Whether `text` has a Chinese character: only then can it be kept as a
/// word to fill in later. (Read as Hanzi, `mao` matches nothing, but it is
/// no word either.)
pub(crate) fn has_han(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(c,
            '\u{3400}'..='\u{4dbf}'
            | '\u{4e00}'..='\u{9fff}'
            | '\u{f900}'..='\u{faff}'
            | '\u{20000}'..='\u{3134f}')
    })
}

/// How a query kind is named on screen.
pub(crate) fn kind_title(kind: QueryKind) -> &'static str {
    match kind {
        QueryKind::Chinese => "Hanzi",
        QueryKind::Pinyin => "Pinyin",
        QueryKind::English => "English",
    }
}

/// Tab's order: worked out, then each kind.
pub(crate) fn next_kind(kind: Option<QueryKind>) -> Option<QueryKind> {
    match kind {
        None => Some(QueryKind::Chinese),
        Some(QueryKind::Chinese) => Some(QueryKind::Pinyin),
        Some(QueryKind::Pinyin) => Some(QueryKind::English),
        Some(QueryKind::English) => None,
    }
}

/// `1 item`, `3 items`: the count with its noun, singular when it is one.
#[must_use]
pub(crate) fn counted(count: impl Into<u64>, singular: &str, plural: &str) -> String {
    match count.into() {
        1 => format!("1 {singular}"),
        count => format!("{count} {plural}"),
    }
}

/// How a search result matched, in the user's words.
#[must_use]
pub(crate) fn basis_label(basis: MatchBasis) -> &'static str {
    match basis {
        MatchBasis::EnglishGloss => "matched the English definition",
        MatchBasis::Pinyin => "matched the pinyin",
        MatchBasis::Simplified => "matched the characters",
        MatchBasis::CharacterFallback => "contains these characters",
        MatchBasis::ContainedWord => "a word inside what you typed",
    }
}

/// Index of the row with the same identity key, if still present.
#[must_use]
pub(crate) fn find_row_index(rows: &[Row], key: &str) -> Option<usize> {
    rows.iter().position(|row| row.key() == key)
}

/// Index of the saved word with the given id, if still present.
#[must_use]
pub(crate) fn find_saved_index(saved: &[ItemView], item_id: i64) -> Option<usize> {
    saved.iter().position(|item| item.id == item_id)
}

pub(crate) fn move_selection(state: &mut ListState, len: usize, forward: bool) {
    if len == 0 {
        return;
    }
    let current = state.selected().unwrap_or(0);
    let next = if forward {
        (current + 1) % len
    } else {
        (current + len - 1) % len
    };
    state.select(Some(next));
}

pub(crate) fn scroll_into_view(state: &mut ListState, viewport: usize) {
    let Some(selected) = state.selected() else {
        return;
    };
    let viewport = viewport.max(1);
    let offset = state.offset();
    if selected + 1 > offset + viewport {
        *state.offset_mut() = selected + 1 - viewport;
    } else if selected < offset {
        *state.offset_mut() = selected;
    }
}

/// Estimated wrapped-row count of `text` at `inner_width` display cells.
/// Uses display-cell width (CJK counts double), so scroll clamping accounts
/// for wrapped glosses instead of only hard newlines.
#[must_use]
pub(crate) fn wrap_row_count(text: &str, inner_width: u16) -> usize {
    let width = usize::from(inner_width.max(1));
    text.split('\n')
        .map(|line| {
            let cells = Line::from(line).width();
            if cells == 0 { 1 } else { cells.div_ceil(width) }
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_selection_wraps_both_directions() {
        let mut state = ListState::default();
        state.select(Some(0));
        move_selection(&mut state, 3, true);
        assert_eq!(state.selected(), Some(1));
        move_selection(&mut state, 3, false);
        assert_eq!(state.selected(), Some(0));
        move_selection(&mut state, 3, false);
        assert_eq!(state.selected(), Some(2));
    }

    #[test]
    fn move_selection_ignores_empty_lists() {
        let mut state = ListState::default();
        move_selection(&mut state, 0, true);
        assert_eq!(state.selected(), None);
    }

    #[test]
    fn scroll_into_view_keeps_selection_visible() {
        let mut state = ListState::default();
        state.select(Some(9));
        scroll_into_view(&mut state, 4);
        assert_eq!(*state.offset_mut(), 6);
        state.select(Some(2));
        scroll_into_view(&mut state, 4);
        assert_eq!(*state.offset_mut(), 2);
    }

    #[test]
    fn scroll_into_view_ignores_missing_selection() {
        let mut state = ListState::default();
        scroll_into_view(&mut state, 4);
        assert_eq!(*state.offset_mut(), 0);
    }

    #[test]
    fn wrap_row_count_counts_hard_lines_and_wrapping() {
        assert_eq!(wrap_row_count("abc", 10), 1);
        assert_eq!(wrap_row_count("a\nb\nc", 10), 3);
        // 26 cells at width 10 wraps to 3 rows.
        assert_eq!(wrap_row_count("abcdefghijklmnopqrstuvwxyz", 10), 3);
        // CJK is double-width: 6 chars = 12 cells = 2 rows at width 10.
        assert_eq!(wrap_row_count("学校学校学校", 10), 2);
        assert_eq!(wrap_row_count("", 10), 1);
    }

    #[test]
    fn only_chinese_characters_are_kept_as_typed() {
        assert!(has_han("蚌埠住了"));
        assert!(has_han("卡拉OK"));
        assert!(has_han("𠮷"));
        assert!(!has_han("mao"));
        assert!(!has_han("ニャー"));
    }

    #[test]
    fn tab_visits_every_kind_and_comes_back() {
        let mut kind = None;
        let mut seen = Vec::new();
        for _ in 0..4 {
            kind = next_kind(kind);
            seen.push(kind);
        }
        assert_eq!(
            seen,
            [
                Some(QueryKind::Chinese),
                Some(QueryKind::Pinyin),
                Some(QueryKind::English),
                None
            ]
        );
    }

    #[test]
    fn scopes_cycle_back_to_all() {
        let mut scope = Scope::All;
        for _ in 0..4 {
            scope = scope.next();
        }
        assert_eq!(scope, Scope::All);
        assert_eq!(Scope::Trash.filter().view, LibraryView::Trash);
        assert_eq!(
            Scope::NeedsReview.filter().verification,
            Some(Verification::NeedsReview)
        );
    }
}
