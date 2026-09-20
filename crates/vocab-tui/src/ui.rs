//! Pure render builders: header, footer, and detail-scroll math.
//!
//! Everything here is a pure function of its inputs so it can be unit-tested
//! without a terminal. `app` owns the state and calls these during `draw`.

use ratatui::text::{Line, Span};

use crate::state::{View, filter_label};
use crate::theme::Theme;

/// Minimum usable terminal size. Below this the TUI renders a resize notice
/// (quit input keeps working) instead of squeezing panels unreadable.
pub(crate) const MIN_WIDTH: u16 = 50;
pub(crate) const MIN_HEIGHT: u16 = 16;

/// Detail-pane scroll step for PgUp/PgDn.
pub(crate) const DETAIL_SCROLL_STEP: u16 = 3;

fn key_spans<'a>(theme: Theme, pairs: &[(&'a str, &'a str)]) -> Vec<Span<'a>> {
    let mut spans = Vec::with_capacity(pairs.len() * 3);
    for (index, (key, action)) in pairs.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" · ", Theme::dim()));
        }
        spans.push(Span::styled(*key, theme.key()));
        spans.push(Span::styled(format!(" {action}"), Theme::dim()));
    }
    spans
}

/// Header line for the current view. Owns its text so callers can render
/// directly without lifetime plumbing.
#[must_use]
pub(crate) fn header_line(
    view: View,
    theme: Theme,
    mode_label: &str,
    query: &str,
    saved_len: usize,
    filter_idx: usize,
    filters: &[Option<vocab_core::ItemStatus>],
) -> Line<'static> {
    let mut spans = vec![Span::styled("vocab ", theme.accent())];
    match view {
        View::Search => {
            spans.push(Span::styled(format!("[{mode_label}]"), theme.accent()));
            spans.push(Span::raw("  "));
            spans.push(Span::raw(query.to_owned()));
        }
        View::Saved => {
            let label = filters.get(filter_idx).copied().unwrap_or(None);
            spans.push(Span::styled(
                format!("[saved · {}]", filter_label(label)),
                theme.accent(),
            ));
            spans.push(Span::raw(format!("  {saved_len} item(s)")));
        }
    }
    Line::from(spans)
}

/// Footer help line: styled key hints plus transient status and durable error.
/// The error (when present) is literal text in the error slot — never color alone.
#[must_use]
pub(crate) fn help_line(
    view: View,
    theme: Theme,
    status: &str,
    error: Option<&str>,
) -> Line<'static> {
    const SEARCH_KEYS: &[(&str, &str)] = &[
        ("Shift+Tab", "switch"),
        ("Tab", "mode"),
        ("Enter", "save"),
        ("Esc", "clear"),
        ("Ctrl+Q", "quit"),
    ];
    const SAVED_KEYS: &[(&str, &str)] = &[
        ("Shift+Tab", "switch"),
        ("Tab", "filter"),
        ("Ctrl+R", "reload"),
        ("Esc", "search"),
        ("Ctrl+Q", "quit"),
    ];
    // Transient message first: it just changed, so it must not be the part
    // that clips on narrow terminals. Idle footers still lead with the keys.
    let mut spans = Vec::new();
    if let Some(error) = error.filter(|e| !e.is_empty()) {
        spans.push(Span::styled("error: ", theme.error()));
        spans.push(Span::styled(error.to_owned(), theme.error()));
        spans.push(Span::styled(" · ", Theme::dim()));
    } else if !status.is_empty() {
        spans.push(Span::raw(status.to_owned()));
        spans.push(Span::styled(" · ", Theme::dim()));
    }
    spans.extend(match view {
        View::Search => key_spans(theme, SEARCH_KEYS),
        View::Saved => key_spans(theme, SAVED_KEYS),
    });
    Line::from(spans)
}

/// Maximum detail scroll offset so the last content row can reach the top of
/// the visible area without scrolling into blank space.
#[must_use]
pub(crate) fn detail_scroll_max(detail: &str, inner_width: u16, visible_height: u16) -> u16 {
    let total = crate::state::wrap_row_count(detail, inner_width);
    let max = total.saturating_sub(usize::from(visible_height.max(1)));
    u16::try_from(max).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::STATUS_FILTERS;

    #[test]
    fn header_search_shows_mode_and_query() {
        let theme = Theme::monochrome();
        let line = header_line(
            View::Search,
            theme,
            "Pinyin",
            "nihao",
            0,
            0,
            &STATUS_FILTERS,
        );
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("vocab"), "{text}");
        assert!(text.contains("[Pinyin]"), "{text}");
        assert!(text.contains("nihao"), "{text}");
    }

    #[test]
    fn header_saved_shows_filter_and_count() {
        let theme = Theme::monochrome();
        let line = header_line(View::Saved, theme, "", "", 7, 0, &STATUS_FILTERS);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("[saved · all]"), "{text}");
        assert!(text.contains("7 item(s)"), "{text}");
    }

    #[test]
    fn help_line_contains_view_keys_and_status() {
        let theme = Theme::monochrome();
        let line = help_line(View::Search, theme, "3 result(s)", None);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("Enter"), "{text}");
        assert!(text.contains("save"), "{text}");
        assert!(!text.contains("Ctrl+S"), "{text}");
        assert!(text.contains("3 result(s)"), "{text}");
        assert!(text.contains("Shift+Tab"), "{text}");
        // Transient status leads so it survives narrow-terminal clipping.
        assert!(text.starts_with("3 result(s)"), "{text}");
    }

    #[test]
    fn help_line_surfaces_errors_literally() {
        let theme = Theme::monochrome();
        let line = help_line(View::Saved, theme, "", Some("disk full"));
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("error: "), "{text}");
        assert!(text.contains("disk full"), "{text}");
    }

    #[test]
    fn detail_scroll_max_clamps_to_content() {
        assert_eq!(detail_scroll_max("a\nb\nc", 10, 2), 1);
        assert_eq!(detail_scroll_max("abc", 10, 5), 0);
        assert_eq!(detail_scroll_max("", 10, 1), 0);
    }
}
