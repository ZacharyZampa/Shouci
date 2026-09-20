//! View/mode state, status filters, and selection helpers.
//!
//! Pure state logic lives here so it can be unit-tested without a terminal.
//! Rendering lives in `ui`; the event loop and DB wiring live in `app`.

use ratatui::{text::Line, widgets::ListState};
use vocab_core::{ItemStatus, VocabItem};
use vocab_dictionary::Candidate;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum View {
    Search,
    Saved,
}

pub(crate) const STATUS_FILTERS: [Option<ItemStatus>; 5] = [
    None,
    Some(ItemStatus::Confirmed),
    Some(ItemStatus::NeedsReview),
    Some(ItemStatus::Exported),
    Some(ItemStatus::Archived),
];

#[must_use]
pub(crate) fn filter_label(filter: Option<ItemStatus>) -> &'static str {
    match filter {
        None => "all",
        Some(status) => status.as_str(),
    }
}

/// Stable identity for a search candidate across re-searches.
#[must_use]
pub(crate) fn candidate_key(candidate: &Candidate) -> String {
    format!(
        "{}\u{1f}{}\u{1f}{}",
        candidate.entry.simplified, candidate.entry.traditional, candidate.entry.pinyin
    )
}

/// Index of the candidate with the same identity key, if still present.
#[must_use]
pub(crate) fn find_candidate_index(results: &[Candidate], key: &str) -> Option<usize> {
    results.iter().position(|c| candidate_key(c) == key)
}

/// Index of the saved item with the given id, if still present.
#[must_use]
pub(crate) fn find_saved_index(saved: &[VocabItem], item_id: i64) -> Option<usize> {
    saved.iter().position(|item| item.item_id == item_id)
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
}
