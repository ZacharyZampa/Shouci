//! Pure render builders: header, footer, and detail-scroll math.
//!
//! Everything here is a pure function of its inputs so it can be unit-tested
//! without a terminal. `app` owns the state and calls these during `draw`.

use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::text::{Line, Span};

use crate::state::{Scope, View};
use crate::theme::Theme;

/// Minimum usable terminal size. Below this the TUI renders a resize notice
/// (quit input keeps working) instead of squeezing panels unreadable.
pub(crate) const MIN_WIDTH: u16 = 50;
pub(crate) const MIN_HEIGHT: u16 = 16;

/// Detail-pane scroll step for PgUp/PgDn.
pub(crate) const DETAIL_SCROLL_STEP: u16 = 3;

/// Below this width rows drop the traditional form (it stays in the detail
/// pane) so the reading and gloss keep their room.
pub(crate) const COMPACT_BELOW: u16 = 80;
/// From this width the detail pane moves beside the list instead of under
/// it, so long definitions get the full height.
pub(crate) const SIDE_BY_SIDE_FROM: u16 = 120;

/// A key and what it does, for the footer.
pub(crate) type Key = (&'static str, &'static str);

/// List actions first, then navigation, so import/export never scroll off
/// the end of a narrow footer. Quit stays last.
pub(crate) const SEARCH_KEYS: &[Key] = &[
    ("Ctrl+O", "import"),
    ("Ctrl+E", "export"),
    ("Enter", "save"),
    ("Shift+Tab", "switch"),
    ("Tab", "mode"),
    ("Esc", "clear"),
    ("Ctrl+Q", "quit"),
];

/// The saved view's keys, which depend on the list shown.
pub(crate) fn saved_keys(scope: Scope) -> &'static [Key] {
    match scope {
        Scope::All | Scope::NeedsReview => &[
            ("i", "import"),
            ("e", "export"),
            ("d", "trash"),
            ("a", "archive"),
            ("n", "needs review"),
            ("Shift+Tab", "switch"),
            ("Tab", "list"),
            ("Ctrl+R", "reload"),
            ("Esc", "search"),
            ("Ctrl+Q", "quit"),
        ],
        Scope::Archived => &[
            ("i", "import"),
            ("e", "export"),
            ("d", "trash"),
            ("a", "unarchive"),
            ("n", "needs review"),
            ("Shift+Tab", "switch"),
            ("Tab", "list"),
            ("Ctrl+R", "reload"),
            ("Esc", "search"),
            ("Ctrl+Q", "quit"),
        ],
        Scope::Trash => &[
            ("r", "restore"),
            ("d", "delete for good"),
            ("Shift+Tab", "switch"),
            ("Tab", "list"),
            ("Ctrl+R", "reload"),
            ("Esc", "search"),
            ("Ctrl+Q", "quit"),
        ],
    }
}

/// Where each region of the screen goes for a given terminal size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Panes {
    pub header: Rect,
    pub list: Rect,
    pub detail: Rect,
    pub footer: Rect,
}

/// Splits `area` by size. Wide terminals put list and detail side by side;
/// narrower ones stack them, giving the detail pane about a quarter of the
/// height (6 to 10 rows) so tall terminals show more of a long definition.
#[must_use]
pub(crate) fn panes(area: Rect) -> Panes {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(area);
    let body = outer[1];
    let split = if area.width >= SIDE_BY_SIDE_FROM {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
            .split(body)
    } else {
        let detail = (area.height / 4).clamp(6, 10);
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(detail)])
            .split(body)
    };
    Panes {
        header: outer[0],
        list: split[0],
        detail: split[1],
        footer: outer[2],
    }
}

/// A list row: the headword leads in bold, the tone-marked reading follows,
/// the gloss trails dimmed. Traditional appears only when it differs from
/// simplified and the terminal is wide enough to spare it.
#[must_use]
pub(crate) fn row_line(
    marker: &str,
    (simplified, traditional, reading): (&str, &str, &str),
    gloss: &str,
    width: u16,
) -> Line<'static> {
    let headword = if traditional.is_empty() || simplified == traditional || width < COMPACT_BELOW {
        simplified.to_owned()
    } else {
        format!("{simplified} / {traditional}")
    };
    let mut spans = Vec::with_capacity(5);
    if !marker.is_empty() {
        spans.push(Span::styled(format!("{marker} "), Theme::dim()));
    }
    spans.push(Span::styled(headword, Theme::headword()));
    if reading.is_empty() {
        spans.push(Span::raw("  "));
    } else {
        spans.push(Span::raw(format!("  {reading}  ")));
    }
    spans.push(Span::styled(gloss.to_owned(), Theme::dim()));
    Line::from(spans)
}

/// Where the terminal cursor goes while typing a query: just after the
/// header text, inside the header box. Input methods (Chinese pinyin input,
/// for one) draw their composition and candidate list at the cursor, so
/// without this they appear wherever drawing last stopped.
#[must_use]
pub(crate) fn query_cursor(header: Rect, header_text_width: usize) -> Position {
    let inner_x = header.x.saturating_add(1);
    let last_column = header.x.saturating_add(header.width.saturating_sub(2));
    let offset = u16::try_from(header_text_width).unwrap_or(u16::MAX);
    Position::new(
        inner_x.saturating_add(offset).min(last_column),
        header.y.saturating_add(1),
    )
}

/// Headword and reading as plain text, for status lines and the detail
/// pane where there is no styling to separate them: `学 / 學 [xué]`.
#[must_use]
pub(crate) fn plain_headword(simplified: &str, traditional: &str, reading: &str) -> String {
    let mut text = simplified.to_owned();
    if !traditional.is_empty() && traditional != simplified {
        text.push_str(" / ");
        text.push_str(traditional);
    }
    if !reading.is_empty() {
        text.push_str(" [");
        text.push_str(reading);
        text.push(']');
    }
    text
}

/// Key hints that fit in `room` cells, shown in their usual order. When not
/// all fit, whole hints are dropped (never cut mid-word), choosing in this
/// order: quit (the last hint) first, then the others as listed, with
/// import/export last because the pane border advertises them anyway.
fn key_spans<'a>(theme: Theme, pairs: &[(&'a str, &'a str)], room: usize) -> Vec<Span<'a>> {
    const SEPARATOR: usize = 3; // " · "
    let width = |(key, action): &(&str, &str)| Line::from(format!("{key} {action}")).width();
    let transfer = |(_, action): &(&str, &str)| matches!(*action, "import" | "export");
    let last = pairs.len().saturating_sub(1);
    let mut order: Vec<usize> = (0..pairs.len()).collect();
    order.sort_by_key(|&i| (i != last, transfer(&pairs[i]), i));
    let mut keep = vec![false; pairs.len()];
    let mut used = 0;
    for i in order {
        let cost = width(&pairs[i]) + if used == 0 { 0 } else { SEPARATOR };
        if used + cost <= room {
            keep[i] = true;
            used += cost;
        }
    }
    let mut spans = Vec::with_capacity(pairs.len() * 3);
    for (_, (key, action)) in pairs.iter().enumerate().filter(|(i, _)| keep[*i]) {
        if !spans.is_empty() {
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
    scope: Scope,
) -> Line<'static> {
    let mut spans = vec![Span::styled("shouci ", theme.accent())];
    match view {
        View::Search => {
            spans.push(Span::styled(format!("[{mode_label}]"), theme.accent()));
            spans.push(Span::raw("  "));
            spans.push(Span::raw(query.to_owned()));
        }
        View::Saved => {
            spans.push(Span::styled(
                format!("[saved · {}]", scope.label()),
                theme.accent(),
            ));
            spans.push(Span::raw(format!(
                "  {}",
                shouci_core::text::counted(
                    u64::try_from(saved_len).unwrap_or(u64::MAX),
                    "word",
                    "words"
                )
            )));
        }
    }
    Line::from(spans)
}

/// Footer help line: styled key hints plus transient status and durable error.
/// The error (when present) is literal text in the error slot — never color alone.
#[must_use]
pub(crate) fn help_line(
    theme: Theme,
    keys: &[Key],
    status: &str,
    error: Option<&str>,
    width: u16,
) -> Line<'static> {
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
    let used = Line::from(spans.clone()).width();
    let room = usize::from(width).saturating_sub(used);
    spans.extend(key_spans(theme, keys, room));
    Line::from(spans)
}

/// The hint shown on the list pane's border, so the main actions are visible
/// on the box rather than only on a footer that can clip.
pub(crate) fn pane_hint(view: View, scope: Scope) -> &'static str {
    match (view, scope) {
        (View::Search, _) => "Ctrl+O import · Ctrl+E export",
        (View::Saved, Scope::Trash) => "r restore · d delete for good",
        (View::Saved, _) => "i import · e export",
    }
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

    fn text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn header_search_shows_mode_and_query() {
        let theme = Theme::monochrome();
        let line = header_line(View::Search, theme, "Pinyin", "nihao", 0, Scope::All);
        let text = text(&line);
        assert!(text.contains("shouci"), "{text}");
        assert!(text.contains("[Pinyin]"), "{text}");
        assert!(text.contains("nihao"), "{text}");
    }

    #[test]
    fn header_saved_shows_list_and_count() {
        let theme = Theme::monochrome();
        let line = header_line(View::Saved, theme, "", "", 7, Scope::All);
        let shown = text(&line);
        assert!(shown.contains("[saved · all]"), "{shown}");
        assert!(shown.contains("7 words"), "{shown}");
        let trash = header_line(View::Saved, theme, "", "", 1, Scope::Trash);
        assert!(text(&trash).contains("[saved · trash]  1 word"));
    }

    #[test]
    fn help_line_contains_view_keys_and_status() {
        let theme = Theme::monochrome();
        let line = help_line(theme, SEARCH_KEYS, "3 results", None, 200);
        let text = text(&line);
        assert!(text.contains("Enter"), "{text}");
        assert!(text.contains("save"), "{text}");
        assert!(!text.contains("Ctrl+S"), "{text}");
        assert!(text.contains("3 results"), "{text}");
        assert!(text.contains("Shift+Tab"), "{text}");
        // Transient status leads so it survives narrow-terminal clipping.
        assert!(text.starts_with("3 results"), "{text}");
    }

    #[test]
    fn help_line_surfaces_errors_literally() {
        let theme = Theme::monochrome();
        let line = help_line(theme, saved_keys(Scope::All), "", Some("disk full"), 200);
        let text = text(&line);
        assert!(text.contains("error: "), "{text}");
        assert!(text.contains("disk full"), "{text}");
    }

    #[test]
    fn the_trash_offers_restore_and_delete_for_good() {
        let theme = Theme::monochrome();
        let line = help_line(theme, saved_keys(Scope::Trash), "", None, 200);
        let text = text(&line);
        assert!(text.contains("r restore"), "{text}");
        assert!(text.contains("d delete for good"), "{text}");
        assert!(!text.contains("export"), "{text}");
    }

    #[test]
    fn narrow_footers_drop_whole_hints_and_keep_quit() {
        let theme = Theme::monochrome();
        let line = help_line(theme, SEARCH_KEYS, "", None, 40);
        let text = text(&line);
        assert!(line.width() <= 40, "{text}");
        assert!(text.contains("Ctrl+Q quit"), "{text}");
        assert!(!text.ends_with(' '), "no half-cut hint: {text}");
        assert!(text.contains("Enter save"), "save outranks import: {text}");
        for (key, action) in [("Ctrl+O", "import"), ("Enter", "save")] {
            // Each hint is all there or not there at all.
            assert_eq!(
                text.contains(key),
                text.contains(&format!("{key} {action}")),
                "{text}"
            );
        }
    }

    #[test]
    fn wide_terminals_put_detail_beside_the_list() {
        let wide = panes(Rect::new(0, 0, 140, 30));
        assert_eq!(wide.list.y, wide.detail.y, "side by side");
        assert!(wide.detail.x > wide.list.x);
        let regular = panes(Rect::new(0, 0, 100, 30));
        assert_eq!(regular.list.x, regular.detail.x, "stacked");
        assert!(regular.detail.y > regular.list.y);
    }

    #[test]
    fn detail_pane_grows_with_height_within_bounds() {
        assert_eq!(panes(Rect::new(0, 0, 100, 16)).detail.height, 6);
        assert_eq!(panes(Rect::new(0, 0, 100, 36)).detail.height, 9);
        assert_eq!(panes(Rect::new(0, 0, 100, 80)).detail.height, 10);
    }

    #[test]
    fn rows_lead_with_the_headword() {
        let wide = row_line("", ("学", "學", "xué"), "learn", 100);
        assert_eq!(text(&wide), "学 / 學  xué  learn");
        assert!(
            wide.spans[0]
                .style
                .add_modifier
                .contains(ratatui::style::Modifier::BOLD),
            "headword is bold"
        );
        assert_eq!(
            text(&row_line("", ("学", "學", "xué"), "learn", 60)),
            "学  xué  learn"
        );
        assert_eq!(
            text(&row_line("", ("你好", "你好", "nǐ hǎo"), "hello", 100)),
            "你好  nǐ hǎo  hello"
        );
        assert_eq!(
            text(&row_line("saved", ("你", "你", "nǐ"), "you", 100)),
            "saved 你  nǐ  you"
        );
        assert_eq!(
            text(&row_line("new", ("蚌埠住了", "", ""), "keep it", 100)),
            "new 蚌埠住了  keep it"
        );
    }

    #[test]
    fn query_cursor_sits_after_the_text_and_stays_in_the_box() {
        let header = Rect::new(1, 1, 40, 3);
        // "shouci [Pinyin]  " is 17 cells; 翻译 is 4 more (double width).
        assert_eq!(query_cursor(header, 21), Position::new(23, 2));
        assert_eq!(
            query_cursor(header, 500),
            Position::new(39, 2),
            "clamped inside the border"
        );
    }

    #[test]
    fn plain_headwords_keep_brackets_and_drop_what_is_missing() {
        assert_eq!(
            plain_headword("学校", "學校", "xué xiào"),
            "学校 / 學校 [xué xiào]"
        );
        assert_eq!(plain_headword("你好", "你好", "nǐ hǎo"), "你好 [nǐ hǎo]");
        assert_eq!(plain_headword("蚌埠住了", "", ""), "蚌埠住了");
    }

    #[test]
    fn detail_scroll_max_clamps_to_content() {
        assert_eq!(detail_scroll_max("a\nb\nc", 10, 2), 1);
        assert_eq!(detail_scroll_max("abc", 10, 5), 0);
        assert_eq!(detail_scroll_max("", 10, 1), 0);
    }
}
