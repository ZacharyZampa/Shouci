//! Drawing: the query bar, the list, the detail pane, and the key line.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, Paragraph, Wrap};
use shouci_core::text::counted;
use shouci_core::{Lifecycle, MatchBasis, Verification};

use super::App;
use crate::state::{Row, Scope, View, basis_label, kind_title, scroll_into_view};
use crate::theme::Theme;
use crate::ui::{MIN_HEIGHT, MIN_WIDTH, plain_headword};

fn pane_title(name: &'static str, view: View, scope: Scope) -> Line<'static> {
    Line::from(vec![
        Span::styled(name, Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(
            format!(" · {}", crate::ui::pane_hint(view, scope)),
            Theme::dim(),
        ),
    ])
}

/// The date part of a stored timestamp (`2026-09-30T12:00:00.000Z`).
fn day(timestamp: &str) -> &str {
    timestamp.get(..10).unwrap_or(timestamp)
}

impl App<'_> {
    pub(super) fn search_status(&self) -> String {
        let Some(found) = &self.found else {
            return String::new();
        };
        let read_as = kind_title(found.kind);
        let shown = u32::try_from(self.rows.len()).unwrap_or(u32::MAX);
        match self.rows.first() {
            None => format!("no matches · read as {read_as}"),
            Some(Row::AsTyped(_)) => {
                String::from("no dictionary entry · Enter keeps it to fill in later")
            }
            Some(Row::Candidate(_)) if found.total > shown => format!(
                "showing {shown} of {} results · read as {read_as}",
                found.total
            ),
            Some(Row::Candidate(_)) => format!(
                "{} · read as {read_as}",
                counted(found.total, "result", "results")
            ),
        }
    }

    /// How the query is read, for the header: chosen, worked out, or not
    /// yet known.
    pub(super) fn mode_label(&self) -> String {
        match (self.kind, &self.found) {
            (Some(kind), _) => kind_title(kind).to_owned(),
            (None, Some(found)) => format!("{} · auto", kind_title(found.kind)),
            (None, None) => String::from("auto"),
        }
    }

    pub(super) fn selected_detail(&self) -> String {
        let Some(row) = self.selected_row() else {
            return String::from(
                "type characters, pinyin, or English · Tab chooses how your search is read · \
                 Enter saves the highlighted word",
            );
        };
        let candidate = match row {
            Row::AsTyped(text) => {
                return format!(
                    "{text}\nNo dictionary has this word. Enter keeps it as you typed it, \
                     marked needs review, so you can fill in its reading and meaning later."
                );
            }
            Row::Candidate(candidate) => candidate,
        };
        let mut basis = basis_label(candidate.basis).to_owned();
        if candidate.inferred && candidate.basis != MatchBasis::ContainedWord {
            basis.push_str(" (partial match)");
        }
        let mut facts = vec![basis];
        if let Some(freq) = candidate.frequency_rank {
            facts.push(format!("frequency rank {freq}"));
        }
        if let Some(hsk) = candidate.hsk_rank {
            facts.push(format!("HSK {hsk}"));
        }
        let mut lines = vec![
            plain_headword(
                &candidate.simplified,
                &candidate.traditional,
                &candidate.pinyin_display,
            ),
            candidate.definition_display.clone(),
            facts.join(" · "),
        ];
        if let Some(saved) = &candidate.saved {
            lines.push(String::from(match (saved.lifecycle, saved.verification) {
                (Lifecycle::Trashed, _) => "saved, now in the trash · Enter brings it back",
                (Lifecycle::Archived, _) => "saved and archived",
                (Lifecycle::Active, Verification::NeedsReview) => "saved · needs review",
                (Lifecycle::Active, Verification::Confirmed) => "saved",
            }));
        }
        lines.join("\n")
    }

    pub(super) fn saved_detail(&self) -> String {
        let Some(item) = self.selected_saved() else {
            return String::from(match self.scope {
                Scope::All => "no saved words yet — press F2 to search, then Enter to save a word",
                Scope::NeedsReview => "nothing needs review — Tab shows the next list",
                Scope::Archived => "nothing is archived — Tab shows the next list",
                Scope::Trash => "the trash is empty — Tab shows the next list",
            });
        };
        let mut lines = vec![plain_headword(
            &item.simplified,
            &item.traditional,
            &item.pinyin_display,
        )];
        if !item.definition_display.is_empty() {
            lines.push(item.definition_display.clone());
        }
        let mut facts = Vec::new();
        if item.verification == Verification::NeedsReview {
            facts.push(String::from("needs review"));
        }
        match item.lifecycle {
            Lifecycle::Active => {}
            Lifecycle::Archived => facts.push(String::from("archived")),
            Lifecycle::Trashed => facts.push(String::from("in the trash")),
        }
        facts.push(format!("saved {}", day(&item.created_at)));
        if item.modified_at != item.created_at {
            facts.push(format!("changed {}", day(&item.modified_at)));
        }
        facts.push(format!("item {}", item.id));
        lines.push(facts.join(" · "));
        if !item.tags.is_empty() {
            lines.push(format!("tags: {}", item.tags.join(", ")));
        }
        if !item.collections.is_empty() {
            lines.push(format!("collections: {}", item.collections.join(", ")));
        }
        if !item.destinations.is_empty() {
            let names: Vec<String> = item
                .destinations
                .iter()
                .map(|id| self.shouci.connector_name(id))
                .collect();
            lines.push(format!("in {}", names.join(", ")));
        }
        if !item.notes.is_empty() {
            lines.push(format!("notes: {}", item.notes));
        }
        if let Some(origin) = &item.source.import_origin {
            lines.push(format!("imported from {origin}"));
        }
        lines.join("\n")
    }

    pub(super) fn render_results(
        &mut self,
        frame: &mut ratatui::Frame<'_>,
        area: ratatui::layout::Rect,
    ) {
        let theme = self.theme;
        let width = frame.area().width;
        // No index numbers: selection is arrows-only (typing appends to the
        // query), so numbers would be decoration without a function.
        let items: Vec<ListItem> = self
            .rows
            .iter()
            .map(|row| match row {
                Row::AsTyped(text) => ListItem::new(crate::ui::row_line(
                    "",
                    (text, "", ""),
                    "not in the dictionary · Enter keeps it to fill in later",
                    width,
                )),
                Row::Candidate(candidate) => {
                    let marker = match &candidate.saved {
                        Some(saved) if saved.lifecycle != Lifecycle::Trashed => "saved",
                        _ if candidate.inferred => "partial",
                        _ => "",
                    };
                    ListItem::new(crate::ui::row_line(
                        marker,
                        (
                            &candidate.simplified,
                            &candidate.traditional,
                            &candidate.pinyin_display,
                        ),
                        &candidate.definition_display,
                        width,
                    ))
                }
            })
            .collect();
        scroll_into_view(&mut self.list_state, area.height as usize);
        frame.render_stateful_widget(
            List::new(items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(theme.focus_border())
                        .title(pane_title("results", View::Search, self.scope)),
                )
                .highlight_style(Theme::selected()),
            area,
            &mut self.list_state,
        );
    }

    pub(super) fn render_saved(
        &mut self,
        frame: &mut ratatui::Frame<'_>,
        area: ratatui::layout::Rect,
    ) {
        let theme = self.theme;
        let width = frame.area().width;
        // Only "needs review" is marked, and only in the full list: the list
        // name (header) and the detail pane carry the rest.
        let items: Vec<ListItem> = self
            .saved
            .iter()
            .map(|item| {
                let marker =
                    if self.scope == Scope::All && item.verification == Verification::NeedsReview {
                        "review"
                    } else {
                        ""
                    };
                ListItem::new(crate::ui::row_line(
                    marker,
                    (&item.simplified, &item.traditional, &item.pinyin_display),
                    &item.definition_display,
                    width,
                ))
            })
            .collect();
        scroll_into_view(&mut self.saved_state, area.height as usize);
        frame.render_stateful_widget(
            List::new(items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(theme.focus_border())
                        .title(pane_title("saved", View::Saved, self.scope)),
                )
                .highlight_style(Theme::selected()),
            area,
            &mut self.saved_state,
        );
    }

    pub fn draw(&mut self, frame: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect) {
        if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
            let notice =
                Paragraph::new("terminal too small — resize to at least 50x16 (Ctrl+Q quits)")
                    .block(titled("shouci"));
            frame.render_widget(notice, area);
            return;
        }
        let panes = crate::ui::panes(area);
        self.render_header(frame, panes.header);
        match self.view {
            View::Search => self.render_results(frame, panes.list),
            View::Saved => self.render_saved(frame, panes.list),
        }
        self.render_detail(frame, panes.detail);
        self.render_footer(frame, panes.footer);
    }

    fn render_header(&self, frame: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect) {
        let header = crate::ui::header_line(
            self.view,
            self.theme,
            &self.mode_label(),
            &self.query,
            self.saved.len(),
            self.scope,
        );
        let header_width = header.width();
        frame.render_widget(
            Paragraph::new(header).block(titled("query").border_style(Theme::plain_border())),
            area,
        );
        // Park the cursor at the end of the query so input-method
        // composition (pinyin → 汉字) appears where the text goes. Elsewhere
        // no cursor is shown: saved-view keys are commands, not text.
        if self.view == View::Search && self.prompt.is_none() {
            frame.set_cursor_position(crate::ui::query_cursor(area, header_width));
        }
    }

    fn render_detail(&mut self, frame: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect) {
        let detail = match self.view {
            View::Search => self.selected_detail(),
            View::Saved => self.saved_detail(),
        };
        // Clamp the scroll offset to wrapped content: without the pane width
        // the stored offset could scroll past the last row into blank space.
        let inner_width = area.width.saturating_sub(2);
        let visible_height = area.height.saturating_sub(2);
        let max = crate::ui::detail_scroll_max(&detail, inner_width, visible_height);
        self.detail_scroll = self.detail_scroll.min(max);
        // Advertise scrolling only when there is somewhere to go.
        let title = if max > 0 {
            format!("selected · PgUp/PgDn scroll {}/{}", self.detail_scroll, max)
        } else {
            String::from("selected")
        };
        frame.render_widget(
            Paragraph::new(detail)
                .wrap(Wrap { trim: true })
                .scroll((self.detail_scroll, 0))
                .block(titled(title).border_style(Theme::plain_border())),
            area,
        );
    }

    fn render_footer(&self, frame: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect) {
        let keys = match self.view {
            View::Search => crate::ui::SEARCH_KEYS,
            View::Saved => crate::ui::saved_keys(self.scope),
        };
        frame.render_widget(
            Paragraph::new(crate::ui::help_line(
                self.theme,
                keys,
                &self.status,
                self.error.as_deref(),
                area.width,
            ))
            .block(Block::default().borders(Borders::NONE)),
            area,
        );
    }
}

/// A rounded pane with a bold title.
fn titled<'a>(title: impl Into<std::borrow::Cow<'a, str>>) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(Line::styled(
            title,
            Style::default().add_modifier(Modifier::BOLD),
        ))
}
