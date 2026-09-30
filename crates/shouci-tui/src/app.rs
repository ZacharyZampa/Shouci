use std::io;
use std::path::{Path, PathBuf};

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use vocab_capture::{SearchMode, open_service, resolve, save_candidate};
use vocab_core::{Result, VocabError, VocabItem};
use vocab_db::{SaveOutcome, delete_item, list_items};
use vocab_dictionary::{Candidate, SqliteDictionary, display_definition};
use vocab_exchange::{ExportOptions, ImportOptions, export_file, import_file};
use vocab_search::SearchService;

use crate::state::{
    STATUS_FILTERS, View, candidate_key, find_candidate_index, find_saved_index, move_selection,
    scroll_into_view,
};
use crate::theme::Theme;
use crate::ui::{DETAIL_SCROLL_STEP, MIN_HEIGHT, MIN_WIDTH};

const CODECS: [&str; 2] = ["pleco-utf8-text/v1", "anki-text/v1"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromptKind {
    Import,
    Export,
}

#[derive(Debug, Clone)]
struct TransferPrompt {
    kind: PromptKind,
    codec_idx: usize,
    only_new: bool,
    path: String,
    /// False while the prompt is a confirm step that hands off to the native
    /// panel. True only as a fallback, when no panel can open and the path has
    /// to be typed after all.
    typing: bool,
}

/// A request for the native file panel, answered by the event loop because only
/// it owns the terminal that has to be handed over to the panel and back.
#[derive(Debug, Clone)]
struct PickerRequest {
    kind: PromptKind,
    suggested: String,
}

fn pane_title(name: &'static str, view: View) -> Line<'static> {
    Line::from(vec![
        Span::styled(name, Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(format!(" · {}", crate::ui::pane_hint(view)), Theme::dim()),
    ])
}

/// Status after backing out of an import or export prompt.
fn cancelled_message(kind: Option<PromptKind>) -> String {
    String::from(match kind {
        Some(PromptKind::Import) => "import cancelled — nothing changed",
        Some(PromptKind::Export) => "export cancelled — no file written",
        None => "cancelled",
    })
}

fn default_transfer_path(codec: &str) -> String {
    let name = if codec == CODECS[1] {
        "shouci-anki.txt"
    } else {
        "shouci-pleco.txt"
    };
    let home = vocab_exchange::expand_path(Path::new("~")).unwrap_or_default();
    let desktop = home.join("Desktop");
    let dir = if desktop.is_dir() { desktop } else { home };
    dir.join(name).to_string_lossy().into_owned()
}

/// Two-pane terminal UI:
///
/// * Search (`F2`): type to search live, Enter saves the selected result.
///   PgUp/PgDn scroll the detail pane.
/// * Saved (`F1`): every item in the user database, newest first, with a
///   status filter (Tab) and full detail for the selection.
///
/// Operational errors (search/save/reload) are captured into a durable error
/// slot instead of killing the event loop; transient feedback goes to status.
pub struct App {
    service: SearchService<SqliteDictionary>,
    user_conn: rusqlite::Connection,
    theme: Theme,
    mode: SearchMode,
    query: String,
    results: Vec<Candidate>,
    list_state: ListState,
    view: View,
    saved: Vec<VocabItem>,
    saved_state: ListState,
    saved_filter_idx: usize,
    status: String,
    error: Option<String>,
    detail_scroll: u16,
    prompt: Option<TransferPrompt>,
    /// Set by the prompt when Enter should hand over to the native panel.
    picker: Option<PickerRequest>,
    /// Saved item awaiting a `y` to confirm deletion: `(item_id, headword)`.
    /// Deletion is irreversible, so `d` only arms it.
    pending_delete: Option<(i64, String)>,
    pub should_quit: bool,
}

impl App {
    #[must_use]
    pub fn new(service: SearchService<SqliteDictionary>, user_conn: rusqlite::Connection) -> Self {
        Self {
            service,
            user_conn,
            theme: Theme::detect(),
            mode: SearchMode::Pinyin,
            query: String::new(),
            results: Vec::new(),
            list_state: ListState::default(),
            view: View::Search,
            saved: Vec::new(),
            saved_state: ListState::default(),
            saved_filter_idx: 0,
            status: String::new(),
            error: None,
            detail_scroll: 0,
            prompt: None,
            picker: None,
            pending_delete: None,
            should_quit: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn io_search(&mut self, mode: SearchMode, query: &str) -> vocab_core::Result<()> {
        self.view = View::Search;
        self.mode = mode;
        self.query = query.to_owned();
        self.run_search()
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn io_headwords(&self, limit: usize) -> Vec<String> {
        self.results
            .iter()
            .take(limit)
            .map(|candidate| candidate.entry.simplified.clone())
            .collect()
    }

    /// Runs a fallible UI operation, capturing failures into the durable
    /// error slot instead of aborting the event loop.
    fn capture(&mut self, op: Result<()>) {
        if let Err(err) = op {
            self.error = Some(err.to_string());
        }
    }

    /// Records a successful operation: clears any stale error.
    fn succeeded(&mut self) {
        self.error = None;
    }

    /// Handles one key press.
    ///
    /// Hotkeys are confined to keys that are never typed: `F1`/`F2` switch the
    /// view, `Tab` cycles a selection, `Ctrl+*` commands, `Esc` clears (or
    /// steps back). Every printable key appends to the query in Search view —
    /// including `c`, `e`, `p`, `q`, and `s` — so pinyin and English input are
    /// never corrupted by a stray hotkey.
    ///
    /// # Errors
    ///
    /// Returns an error if a search, save, or list reload fails.
    pub fn on_key(&mut self, key: &KeyEvent) -> Result<()> {
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }
        if let Some((item_id, head)) = self.pending_delete.take() {
            let confirmed = matches!(key.code, KeyCode::Char('y' | 'Y'))
                && !key.modifiers.contains(KeyModifiers::CONTROL);
            if confirmed {
                let op = self.delete_saved(item_id, &head);
                self.capture(op);
                return Ok(());
            }
            self.status = format!("kept {head}");
            // Ctrl commands (quit, import, export) still run; any other key
            // only cancels, so a stray letter never lands somewhere else.
            if !key.modifiers.contains(KeyModifiers::CONTROL) {
                return Ok(());
            }
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            if self.prompt.is_some() {
                self.on_prompt_control(key.code);
            } else {
                self.on_control(key.code);
            }
            return Ok(());
        }
        if self.prompt.is_some() {
            let op = self.on_prompt_key(key.code);
            self.capture(op);
            return Ok(());
        }
        match key.code {
            KeyCode::F(1) => {
                let op = self.show_saved();
                self.capture(op);
            }
            KeyCode::F(2) => self.show_search(),
            // Shift+Tab toggles between the two views; plain Tab keeps its
            // per-view meaning (search mode / saved filter).
            KeyCode::BackTab => match self.view {
                View::Search => {
                    let op = self.show_saved();
                    self.capture(op);
                }
                View::Saved => self.show_search(),
            },
            KeyCode::PageUp => self.scroll_detail(true),
            KeyCode::PageDown => self.scroll_detail(false),
            _ => match self.view {
                View::Saved => self.on_saved_key(key.code),
                View::Search => self.on_search_key(key.code),
            },
        }
        Ok(())
    }

    fn on_control(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('q' | 'c') => self.should_quit = true,
            KeyCode::Char('o') => self.begin_prompt(PromptKind::Import),
            KeyCode::Char('e') => self.begin_prompt(PromptKind::Export),
            KeyCode::Char('r') if self.view == View::Saved => {
                let op = self.reload_saved();
                self.capture(op);
                if self.error.is_none() {
                    self.status = String::from("saved list reloaded");
                }
            }
            _ => {}
        }
    }

    fn show_saved(&mut self) -> Result<()> {
        self.view = View::Saved;
        self.reload_saved()
    }

    fn show_search(&mut self) {
        self.view = View::Search;
    }

    fn reload_saved(&mut self) -> Result<()> {
        let previous = self
            .saved_state
            .selected()
            .and_then(|index| self.saved.get(index))
            .map(|item| item.item_id);
        let filter = STATUS_FILTERS[self.saved_filter_idx];
        let items = list_items(&self.user_conn, filter)?;
        self.saved = items;
        // Restore the previous selection by stable id when it survives the
        // reload; otherwise fall back to the top of the list.
        let restored = previous.and_then(|id| find_saved_index(&self.saved, id));
        self.saved_state.select(if self.saved.is_empty() {
            None
        } else {
            Some(restored.unwrap_or(0))
        });
        self.detail_scroll = 0;
        self.succeeded();
        Ok(())
    }

    /// Arms deletion of the selected item; the next key confirms (`y`) or
    /// cancels (anything else). See [`App::on_key`].
    fn request_delete_selected(&mut self) {
        let Some(item) = self.saved_state.selected().and_then(|i| self.saved.get(i)) else {
            self.status = String::from("nothing selected");
            return;
        };
        let head = item.simplified.clone();
        self.status =
            format!("delete {head}? can't be undone · y deletes · any other key keeps it");
        self.pending_delete = Some((item.item_id, head));
    }

    fn delete_saved(&mut self, item_id: i64, head: &str) -> Result<()> {
        delete_item(&self.user_conn, item_id)?;
        self.reload_saved()?;
        self.status = format!("deleted {head}");
        Ok(())
    }

    fn begin_prompt(&mut self, kind: PromptKind) {
        let path = default_transfer_path(CODECS[0]);
        self.prompt = Some(TransferPrompt {
            kind,
            codec_idx: 0,
            only_new: false,
            path,
            typing: false,
        });
        self.refresh_prompt_status();
    }

    /// Takes the pending native-panel request, if the prompt asked for one.
    fn take_picker(&mut self) -> Option<PickerRequest> {
        self.picker.take()
    }

    /// Applies a path chosen in the native panel and runs the transfer.
    fn accept_picked_path(&mut self, path: String) {
        if let Some(prompt) = self.prompt.as_mut() {
            prompt.path = path;
        }
        let op = self.run_prompt();
        self.capture(op);
    }

    /// The panel was dismissed, so leave the prompt exactly as it was.
    fn cancel_picker(&mut self) {
        let kind = self.prompt.as_ref().map(|prompt| prompt.kind);
        self.prompt = None;
        self.status = cancelled_message(kind);
    }

    /// No panel could be opened, so fall back to typing the path.
    fn fall_back_to_typing(&mut self, reason: &str) {
        if let Some(prompt) = self.prompt.as_mut() {
            prompt.typing = true;
        }
        self.error = None;
        self.refresh_prompt_status();
        self.status = format!("no file picker ({reason}) — type the path, Enter runs, Esc cancels");
    }

    fn on_prompt_control(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('q' | 'c') => {
                self.should_quit = true;
                return;
            }
            KeyCode::Char('t') => {
                if let Some(prompt) = self.prompt.as_mut() {
                    prompt.codec_idx = (prompt.codec_idx + 1) % CODECS.len();
                    if !prompt.typing {
                        prompt.path = default_transfer_path(CODECS[prompt.codec_idx]);
                    }
                }
            }
            KeyCode::Char('n') => {
                if let Some(prompt) = self.prompt.as_mut() {
                    if prompt.kind == PromptKind::Export {
                        prompt.only_new = !prompt.only_new;
                    }
                }
            }
            _ => return,
        }
        self.refresh_prompt_status();
    }

    fn on_prompt_key(&mut self, code: KeyCode) -> Result<()> {
        let typing = self.prompt.as_ref().is_some_and(|p| p.typing);
        match code {
            KeyCode::Esc => {
                let kind = self.prompt.as_ref().map(|prompt| prompt.kind);
                self.prompt = None;
                self.status = cancelled_message(kind);
                Ok(())
            }
            KeyCode::Enter if typing => self.run_prompt(),
            KeyCode::Enter => {
                let request = self.prompt.as_ref().map(|prompt| PickerRequest {
                    kind: prompt.kind,
                    suggested: prompt.path.clone(),
                });
                self.picker = request;
                Ok(())
            }
            KeyCode::Backspace if typing => {
                if let Some(prompt) = self.prompt.as_mut() {
                    prompt.path.pop();
                }
                self.refresh_prompt_status();
                Ok(())
            }
            KeyCode::Char(ch) if typing && !ch.is_control() => {
                if let Some(prompt) = self.prompt.as_mut() {
                    prompt.path.push(ch);
                }
                self.refresh_prompt_status();
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn run_prompt(&mut self) -> Result<()> {
        let Some(prompt) = self.prompt.clone() else {
            return Ok(());
        };
        let path = prompt.path.trim();
        if path.is_empty() {
            self.status = String::from("type a file path first (Esc cancels)");
            return Ok(());
        }
        let codec = CODECS[prompt.codec_idx].to_owned();
        let dict = self.service.provider();
        let report = match prompt.kind {
            PromptKind::Import => import_file(
                &mut self.user_conn,
                Some(dict),
                Path::new(path),
                &ImportOptions {
                    codec,
                    force: false,
                    dry_run: false,
                },
            )?,
            PromptKind::Export => export_file(
                &mut self.user_conn,
                Some(dict),
                Path::new(path),
                &ExportOptions {
                    codec,
                    only_new: prompt.only_new,
                    allow_unresolved: false,
                    include_archived: false,
                    tags: Vec::new(),
                    category: None,
                    deck: "Shouci".to_owned(),
                    dry_run: false,
                },
            )?,
        };
        self.prompt = None;
        self.reload_saved()?;
        self.status = report.summary;
        self.succeeded();
        Ok(())
    }

    fn refresh_prompt_status(&mut self) {
        let Some(prompt) = self.prompt.clone() else {
            return;
        };
        let kind = match prompt.kind {
            PromptKind::Import => "import",
            PromptKind::Export => "export",
        };
        let scope = if prompt.kind == PromptKind::Export && prompt.only_new {
            " only-new"
        } else {
            ""
        };
        let codec = CODECS[prompt.codec_idx];
        let typed = prompt.path.trim();
        // Show the real destination, never the shorthand, so there is nothing
        // left to guess about where the file lands.
        let resolved = vocab_exchange::expand_path(Path::new(typed)).unwrap_or_default();
        let state = if typed.is_empty() {
            "no path".to_owned()
        } else if prompt.kind == PromptKind::Import {
            if resolved.exists() {
                "file found".to_owned()
            } else {
                "file missing".to_owned()
            }
        } else if resolved.exists() {
            "will replace".to_owned()
        } else {
            "new file".to_owned()
        };
        let action = if prompt.typing {
            "Enter run"
        } else if prompt.kind == PromptKind::Import {
            "Enter choose file"
        } else {
            "Enter choose location"
        };
        self.status = format!(
            "{kind} {codec}{scope} · {state} · {}  {action} · Ctrl+T codec · Esc cancel",
            resolved.display()
        );
    }

    fn on_saved_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => self.show_search(),
            KeyCode::Char('i') => self.begin_prompt(PromptKind::Import),
            KeyCode::Char('e') => self.begin_prompt(PromptKind::Export),
            KeyCode::Delete | KeyCode::Char('d') => self.request_delete_selected(),
            KeyCode::Tab => {
                self.saved_filter_idx = (self.saved_filter_idx + 1) % STATUS_FILTERS.len();
                let op = self.reload_saved();
                self.capture(op);
                if self.error.is_none() {
                    self.status = format!(
                        "showing {} · {}",
                        crate::state::filter_label(STATUS_FILTERS[self.saved_filter_idx]),
                        crate::state::counted(self.saved.len(), "word", "words")
                    );
                }
            }
            KeyCode::Down => {
                move_selection(&mut self.saved_state, self.saved.len(), true);
                self.detail_scroll = 0;
            }
            KeyCode::Up => {
                move_selection(&mut self.saved_state, self.saved.len(), false);
                self.detail_scroll = 0;
            }
            _ => {}
        }
    }

    fn on_search_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                if self.query.is_empty() {
                    self.should_quit = true;
                } else {
                    self.query.clear();
                    self.results.clear();
                    self.list_state.select(None);
                    self.detail_scroll = 0;
                    self.error = None;
                    self.status = String::from("query cleared");
                }
            }
            KeyCode::Tab => {
                self.mode = match self.mode {
                    SearchMode::English => SearchMode::Chinese,
                    SearchMode::Pinyin => SearchMode::English,
                    SearchMode::Chinese => SearchMode::Pinyin,
                };
                self.status = String::from(match self.mode {
                    SearchMode::English => "searching English definitions",
                    SearchMode::Pinyin => "searching pinyin",
                    SearchMode::Chinese => "searching Chinese characters",
                });
            }
            KeyCode::Char(c) if !c.is_control() => {
                self.query.push(c);
                self.error = None;
                let op = self.run_search();
                self.capture(op);
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.error = None;
                let op = self.run_search();
                self.capture(op);
            }
            KeyCode::Enter => {
                if self.list_state.selected().is_some() {
                    let op = self.save_selected();
                    self.capture(op);
                } else if !self.query.is_empty() {
                    self.status =
                        String::from("nothing to save: no matches (Tab tries another mode)");
                }
            }
            KeyCode::Down => {
                move_selection(&mut self.list_state, self.results.len(), true);
                self.detail_scroll = 0;
            }
            KeyCode::Up => {
                move_selection(&mut self.list_state, self.results.len(), false);
                self.detail_scroll = 0;
            }
            _ => {}
        }
    }

    /// Scrolls the detail pane. Clamping to content happens in `draw`, where
    /// the pane width is known, so this only moves the offset.
    fn scroll_detail(&mut self, up: bool) {
        if up {
            self.detail_scroll = self.detail_scroll.saturating_sub(DETAIL_SCROLL_STEP);
        } else {
            self.detail_scroll = self.detail_scroll.saturating_add(DETAIL_SCROLL_STEP);
        }
    }

    fn run_search(&mut self) -> Result<()> {
        let query = self.query.trim().to_owned();
        if query.is_empty() {
            // Live search: no query means no results, not stale ones.
            self.results.clear();
            self.list_state.select(None);
            self.detail_scroll = 0;
            return Ok(());
        }
        let previous = self
            .list_state
            .selected()
            .and_then(|index| self.results.get(index))
            .map(candidate_key);
        self.results = resolve(&self.service, self.mode, &query)?;
        // Keep the user's place when the same entry survives a re-search.
        let restored = previous
            .as_deref()
            .and_then(|key| find_candidate_index(&self.results, key));
        self.list_state = ListState::default();
        self.list_state.select(if self.results.is_empty() {
            None
        } else {
            Some(restored.unwrap_or(0))
        });
        self.detail_scroll = 0;
        self.succeeded();
        self.status = format!(
            "{} · {}",
            crate::state::counted(self.results.len(), "result", "results"),
            self.mode_label()
        );
        Ok(())
    }

    fn save_selected(&mut self) -> Result<()> {
        let Some(index) = self.list_state.selected() else {
            return Ok(());
        };
        let candidate = &self.results[index];
        let outcome = save_candidate(&self.user_conn, candidate)?;
        self.status = match outcome {
            SaveOutcome::Inserted(item) => format!(
                "saved {}",
                crate::ui::plain_headword(&item.simplified, &item.traditional, &item.pinyin)
            ),
            SaveOutcome::Duplicate(item) => format!(
                "already saved {} ({})",
                crate::ui::plain_headword(&item.simplified, &item.traditional, &item.pinyin),
                item.status.label()
            ),
        };
        self.succeeded();
        Ok(())
    }

    fn mode_label(&self) -> &'static str {
        match self.mode {
            SearchMode::English => "English",
            SearchMode::Pinyin => "Pinyin",
            SearchMode::Chinese => "Chinese",
        }
    }

    fn selected_detail(&self) -> String {
        let Some(index) = self.list_state.selected() else {
            return format!(
                "type to search {} · Tab changes what you're searching · Enter saves the highlighted word",
                self.mode_label()
            );
        };
        let Some(candidate) = self.results.get(index) else {
            return String::from("no selection");
        };
        let inferred = if candidate.diagnostic.is_inferred {
            " (inferred)"
        } else {
            ""
        };
        let mut facts = vec![format!(
            "{}{inferred}",
            crate::state::basis_label(candidate.diagnostic.basis)
        )];
        if let Some(freq) = candidate.entry.frequency_rank {
            facts.push(format!("frequency rank {freq}"));
        }
        if let Some(hsk) = candidate.entry.hsk_rank {
            facts.push(format!("HSK {hsk}"));
        }
        format!(
            "{}\n{}\n{}",
            crate::ui::plain_headword(
                &candidate.entry.simplified,
                &candidate.entry.traditional,
                &candidate.entry.pinyin,
            ),
            display_definition(&candidate.entry.glosses.join("; ")),
            facts.join(" · "),
        )
    }

    fn saved_detail(&self) -> String {
        let Some(index) = self.saved_state.selected() else {
            return String::from(if self.saved_filter_idx == 0 {
                "no saved words yet — press F2 to search, then Enter to save a word"
            } else {
                "no words with this status — Tab shows the next filter"
            });
        };
        let Some(item) = self.saved.get(index) else {
            return String::from("no selection");
        };
        let mut lines = vec![crate::ui::plain_headword(
            &item.simplified,
            &item.traditional,
            &item.pinyin,
        )];
        lines.push(display_definition(&item.definition));
        lines.push(format!(
            "{} · saved {} · changed {} · item {}",
            item.status.label(),
            item.created_at,
            item.modified_at,
            item.item_id
        ));
        if let Some(notes) = &item.notes {
            lines.push(format!("notes: {notes}"));
        }
        if let Some(origin) = &item.provenance.import_origin {
            lines.push(format!("import: {origin}"));
        }
        lines.join("\n")
    }

    fn render_results(&mut self, frame: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect) {
        let theme = self.theme;
        let width = frame.area().width;
        // No index numbers: selection is arrows-only (typing appends to the
        // query), so numbers would be decoration without a function.
        let items: Vec<ListItem> = self
            .results
            .iter()
            .map(|c| {
                let marker = if c.diagnostic.is_inferred {
                    "▸inferred"
                } else {
                    ""
                };
                ListItem::new(crate::ui::row_line(
                    marker,
                    (&c.entry.simplified, &c.entry.traditional, &c.entry.pinyin),
                    &display_definition(&c.entry.glosses.join("; ")),
                    width,
                ))
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
                        .title(pane_title("results", View::Search)),
                )
                .highlight_style(Theme::selected()),
            area,
            &mut self.list_state,
        );
    }

    fn render_saved(&mut self, frame: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect) {
        let theme = self.theme;
        let width = frame.area().width;
        // No ids or statuses on rows: every save is deliberately picked, so
        // the active filter (header) plus the `selected` detail pane carry
        // that context instead.
        let items: Vec<ListItem> = self
            .saved
            .iter()
            .map(|item| {
                ListItem::new(crate::ui::row_line(
                    "",
                    (&item.simplified, &item.traditional, &item.pinyin),
                    &display_definition(&item.definition),
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
                        .title(pane_title("saved", View::Saved)),
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
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_type(BorderType::Rounded)
                            .title(Line::styled(
                                "shouci",
                                Style::default().add_modifier(Modifier::BOLD),
                            )),
                    );
            frame.render_widget(notice, area);
            return;
        }

        let panes = crate::ui::panes(area);

        let theme = self.theme;
        let header = crate::ui::header_line(
            self.view,
            theme,
            self.mode_label(),
            &self.query,
            self.saved.len(),
            self.saved_filter_idx,
            &STATUS_FILTERS,
        );
        let header_width = header.width();
        frame.render_widget(
            Paragraph::new(header).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Theme::plain_border())
                    .title(Line::styled(
                        "query",
                        Style::default().add_modifier(Modifier::BOLD),
                    )),
            ),
            panes.header,
        );
        // Park the cursor at the end of the query so input-method
        // composition (pinyin → 汉字) appears where the text goes. Elsewhere
        // no cursor is shown: saved-view keys are commands, not text.
        if self.view == View::Search && self.prompt.is_none() {
            frame.set_cursor_position(crate::ui::query_cursor(panes.header, header_width));
        }

        match self.view {
            View::Search => self.render_results(frame, panes.list),
            View::Saved => self.render_saved(frame, panes.list),
        }

        let detail = match self.view {
            View::Search => self.selected_detail(),
            View::Saved => self.saved_detail(),
        };
        // Clamp the scroll offset to wrapped content: without the pane width
        // the stored offset could scroll past the last row into blank space.
        let inner_width = panes.detail.width.saturating_sub(2);
        let visible_height = panes.detail.height.saturating_sub(2);
        let max = crate::ui::detail_scroll_max(&detail, inner_width, visible_height);
        self.detail_scroll = self.detail_scroll.min(max);
        // Advertise scrolling only when there is somewhere to go.
        let detail_title = if max > 0 {
            format!("selected · PgUp/PgDn scroll {}/{}", self.detail_scroll, max)
        } else {
            String::from("selected")
        };
        frame.render_widget(
            Paragraph::new(detail)
                .wrap(Wrap { trim: true })
                .scroll((self.detail_scroll, 0))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Theme::plain_border())
                        .title(Line::styled(
                            detail_title,
                            Style::default().add_modifier(Modifier::BOLD),
                        )),
                ),
            panes.detail,
        );
        frame.render_widget(
            Paragraph::new(crate::ui::help_line(
                self.view,
                theme,
                &self.status,
                self.error.as_deref(),
                panes.footer.width,
            ))
            .block(Block::default().borders(Borders::NONE)),
            panes.footer,
        );
    }
}

/// Runs the TUI event loop until the user quits.
///
/// # Errors
///
/// Returns an error if the terminal cannot be entered/exited or an event cannot
/// be read.
pub fn run(dictionary: &std::path::Path, user_db: &std::path::Path) -> Result<()> {
    let service = open_service(Some(dictionary))?;
    let user_conn = vocab_db::open_user_db(user_db)?;

    enable_raw_mode().map_err(|err| VocabError::new(format!("enable raw mode: {err}")))?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)
        .map_err(|err| VocabError::new(format!("enter alternate screen: {err}")))?;
    let mut terminal = match Terminal::new(CrosstermBackend::new(stdout)) {
        Ok(t) => t,
        Err(err) => {
            let _ = execute!(io::stdout(), LeaveAlternateScreen);
            disable_raw_mode().ok();
            return Err(VocabError::new(format!("terminal init: {err}")));
        }
    };

    // A panic would otherwise leave the shell in raw mode on the alternate
    // screen. Restore the terminal first, then report as usual.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, crossterm::cursor::Show);
        default_hook(info);
    }));

    let result = event_loop(&mut terminal, service, user_conn);

    disable_raw_mode().map_err(|err| VocabError::new(format!("disable raw mode: {err}")))?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .map_err(|err| VocabError::new(format!("leave alternate screen: {err}")))?;
    terminal
        .show_cursor()
        .map_err(|err| VocabError::new(format!("restore cursor: {err}")))?;

    result
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    service: SearchService<SqliteDictionary>,
    user_conn: rusqlite::Connection,
) -> Result<()> {
    let mut app = App::new(service, user_conn);
    while !app.should_quit {
        terminal
            .draw(|frame| app.draw(frame, frame.area()))
            .map_err(|err| VocabError::new(format!("draw: {err}")))?;
        if let Event::Key(key) =
            event::read().map_err(|err| VocabError::new(format!("read event: {err}")))?
        {
            app.on_key(&key)?;
        }
        if let Some(request) = app.take_picker() {
            match pick_path(terminal, &request) {
                Ok(Some(path)) => app.accept_picked_path(path),
                Ok(None) => app.cancel_picker(),
                Err(reason) => app.fall_back_to_typing(&reason),
            }
        }
    }
    Ok(())
}

/// Opens the native save/open panel for a transfer.
///
/// The panel draws on the normal screen, so the alternate screen and raw mode
/// are handed back before it opens and taken again afterwards. Both restores
/// run even if the panel fails, otherwise the TUI is left unusable.
fn pick_path(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    request: &PickerRequest,
) -> std::result::Result<Option<String>, String> {
    let suggested = PathBuf::from(&request.suggested);
    let mut dialog = rfd::FileDialog::new();
    if let Some(name) = suggested.file_name() {
        dialog = dialog.set_file_name(name.to_string_lossy().into_owned());
    }
    if let Some(parent) = suggested.parent().filter(|p| !p.as_os_str().is_empty()) {
        dialog = dialog.set_directory(parent);
    }

    disable_raw_mode().map_err(|err| format!("cannot leave raw mode: {err}"))?;
    let left = execute!(io::stdout(), LeaveAlternateScreen).map_err(|err| err.to_string());

    let chosen = match request.kind {
        PromptKind::Import => dialog.pick_file(),
        PromptKind::Export => dialog.save_file(),
    };

    let restored = enable_raw_mode()
        .map_err(|err| format!("cannot restore raw mode: {err}"))
        .and_then(|()| execute!(io::stdout(), EnterAlternateScreen).map_err(|err| err.to_string()))
        .and_then(|()| terminal.clear().map_err(|err| err.to_string()));
    left?;
    restored?;

    match chosen {
        Some(path) => Ok(Some(path.to_string_lossy().into_owned())),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crossterm::event::KeyEventKind;
    use vocab_capture::SearchMode;
    use vocab_dictionary::{CedictSource, build_dictionary_db};
    use vocab_search::DeterministicRanker;

    use super::*;

    fn app() -> App {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/dictionary/cedict-sample.u8");
        let artifact = std::fs::read(&path).expect("missing cedict sample fixture");
        let mut conn = rusqlite::Connection::open_in_memory().expect("in-memory db");
        build_dictionary_db(&mut conn, &CedictSource::default(), &artifact).expect("build");
        let provider = SqliteDictionary::from_connection(conn).expect("open provider");
        let service = SearchService::new(provider, DeterministicRanker::default());
        let user_conn = rusqlite::Connection::open_in_memory().expect("in-memory user db");
        user_conn
            .pragma_update(None, "foreign_keys", "ON")
            .expect("foreign keys");
        vocab_db::apply_schema(&user_conn).expect("user schema");
        App::new(service, user_conn)
    }

    fn press(app: &mut App, code: KeyCode) -> vocab_core::Result<()> {
        app.on_key(&KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn ctrl(app: &mut App, code: KeyCode) -> vocab_core::Result<()> {
        app.on_key(&KeyEvent::new(code, KeyModifiers::CONTROL))
    }

    fn open_export_prompt(app: &mut App) {
        ctrl(app, KeyCode::Char('e')).expect("open export prompt");
    }

    #[test]
    fn export_prompt_starts_on_a_default_destination() {
        let mut app = app();
        open_export_prompt(&mut app);
        let prompt = app.prompt.as_ref().expect("prompt open");
        assert!(prompt.path.ends_with("shouci-pleco.txt"), "{}", prompt.path);
        assert!(Path::new(&prompt.path).is_absolute(), "{}", prompt.path);
        assert!(!prompt.typing, "should offer the panel, not typing");
        assert!(
            app.status.contains("Enter choose location"),
            "status: {:?}",
            app.status
        );
        assert!(app.status.contains("new file"), "status: {:?}", app.status);
    }

    #[test]
    fn codec_toggle_renames_the_default_file() {
        let mut app = app();
        open_export_prompt(&mut app);
        ctrl(&mut app, KeyCode::Char('t')).expect("cycle codec");
        let prompt = app.prompt.as_ref().expect("prompt open");
        assert!(prompt.path.ends_with("shouci-anki.txt"), "{}", prompt.path);
        assert!(
            app.status.contains("anki-text/v1"),
            "status: {:?}",
            app.status
        );
    }

    #[test]
    fn only_new_toggle_is_export_only() {
        let mut app = app();
        open_export_prompt(&mut app);
        ctrl(&mut app, KeyCode::Char('n')).expect("toggle only-new");
        assert!(app.prompt.as_ref().expect("prompt").only_new);
        assert!(app.status.contains("only-new"), "{:?}", app.status);
    }

    #[test]
    fn enter_requests_the_panel_instead_of_writing() {
        let mut app = app();
        search_and_save(&mut app, "school");
        open_export_prompt(&mut app);
        press(&mut app, KeyCode::Enter).expect("enter");
        let request = app.take_picker().expect("panel requested");
        assert_eq!(request.kind, PromptKind::Export);
        assert!(app.take_picker().is_none(), "request is taken once");
        // Nothing was written: the transfer only runs once a path comes back.
        assert!(!app.status.contains("exported"), "{:?}", app.status);
    }

    #[test]
    fn import_prompt_offers_the_open_panel() {
        let mut app = app();
        ctrl(&mut app, KeyCode::Char('o')).expect("open import prompt");
        press(&mut app, KeyCode::Enter).expect("enter");
        let request = app.take_picker().expect("panel requested");
        assert_eq!(request.kind, PromptKind::Import);
        assert!(
            app.prompt
                .as_ref()
                .expect("prompt")
                .path
                .ends_with("shouci-pleco.txt"),
            "import default"
        );
    }

    #[test]
    fn the_confirm_step_ignores_typed_characters() {
        let mut app = app();
        open_export_prompt(&mut app);
        let before = app.prompt.as_ref().expect("prompt").path.clone();
        for ch in "xyz".chars() {
            press(&mut app, KeyCode::Char(ch)).expect("ignored");
        }
        assert_eq!(app.prompt.as_ref().expect("prompt").path, before);
        assert!(app.take_picker().is_none(), "no panel without Enter");
    }

    #[test]
    fn backspace_is_ignored_until_typing_is_enabled() {
        let mut app = app();
        open_export_prompt(&mut app);
        let before = app.prompt.as_ref().expect("prompt").path.clone();
        press(&mut app, KeyCode::Backspace).expect("ignored");
        assert_eq!(app.prompt.as_ref().expect("prompt").path, before);
    }

    #[test]
    fn a_failed_panel_falls_back_to_typing() {
        let mut app = app();
        open_export_prompt(&mut app);
        app.fall_back_to_typing("no panel available");
        let prompt = app.prompt.as_ref().expect("prompt still open");
        assert!(prompt.typing, "typing enabled after failure");
        assert!(app.status.contains("type the path"), "{:?}", app.status);

        for ch in "/tmp".chars() {
            press(&mut app, KeyCode::Char(ch)).expect("typed");
        }
        assert!(
            app.prompt.as_ref().expect("prompt").path.ends_with("/tmp"),
            "typed into the path"
        );
    }

    #[test]
    fn a_dismissed_panel_leaves_the_prompt_clean() {
        let mut app = app();
        open_export_prompt(&mut app);
        press(&mut app, KeyCode::Enter).expect("enter");
        assert!(app.take_picker().is_some(), "panel requested");
        app.cancel_picker();
        assert!(app.prompt.is_none(), "prompt closed");
        assert_eq!(app.status, "export cancelled — no file written");
    }

    #[test]
    fn tilde_is_expanded_in_the_confirm_step() {
        let mut app = app();
        open_export_prompt(&mut app);
        let home = vocab_exchange::expand_path(Path::new("~")).expect("home");
        app.prompt.as_mut().expect("prompt").typing = true;
        app.prompt.as_mut().expect("prompt").path = format!("~{HOME_MARK}");
        app.refresh_prompt_status();
        assert!(
            app.status.contains(&home.to_string_lossy().into_owned()),
            "status should show the expanded path: {:?}",
            app.status
        );
    }

    const HOME_MARK: &str = "/definitely-not-here.txt";

    #[test]
    fn pane_hints_advertise_the_transfer_keys() {
        assert_eq!(crate::ui::pane_hint(View::Saved), "i import · e export");
        assert_eq!(
            crate::ui::pane_hint(View::Search),
            "Ctrl+O import · Ctrl+E export"
        );
    }

    fn search_and_save(app: &mut App, word: &str) {
        if !app.query.is_empty() {
            // Esc on a non-empty query clears it (never quits here).
            press(app, KeyCode::Esc).unwrap();
        }
        while app.mode != SearchMode::English {
            press(app, KeyCode::Tab).unwrap();
        }
        for ch in word.chars() {
            press(app, KeyCode::Char(ch)).unwrap();
        }
        // Live search already populated results; Enter saves the selection.
        press(app, KeyCode::Enter).unwrap();
        assert!(
            app.status.starts_with("saved ") || app.status.starts_with("already saved "),
            "unexpected status after Enter: {:?}",
            app.status
        );
    }

    #[test]
    fn typing_populates_results_live() {
        let mut app = app();
        press(&mut app, KeyCode::Tab).unwrap();
        for ch in "school".chars() {
            press(&mut app, KeyCode::Char(ch)).unwrap();
        }
        // No Enter needed: results arrive as you type.
        assert_eq!(app.results.len(), 1);
        assert_eq!(app.results[0].entry.simplified, "学校");
        assert_eq!(app.list_state.selected(), Some(0));
    }

    #[test]
    fn hotkey_letters_always_type_when_query_is_empty() {
        let mut app = app();
        for ch in "cepsq".chars() {
            press(&mut app, KeyCode::Char(ch)).unwrap();
        }
        assert_eq!(app.query, "cepsq");
        assert!(!app.should_quit);
    }

    #[test]
    fn enter_saves_the_selection() {
        let mut app = app();
        press(&mut app, KeyCode::Tab).unwrap();
        for ch in "school".chars() {
            press(&mut app, KeyCode::Char(ch)).unwrap();
        }
        press(&mut app, KeyCode::Enter).unwrap();
        assert!(
            app.status.starts_with("saved 学校 / 學校"),
            "unexpected status: {:?}",
            app.status
        );
        assert!(
            list_items(&app.user_conn, None)
                .unwrap()
                .iter()
                .any(|item| item.simplified == "学校")
        );
    }

    #[test]
    fn enter_with_no_results_reports_nothing_to_save() {
        let mut app = app();
        press(&mut app, KeyCode::Tab).unwrap();
        for ch in "zzzqqq".chars() {
            press(&mut app, KeyCode::Char(ch)).unwrap();
        }
        assert!(app.results.is_empty());
        press(&mut app, KeyCode::Enter).unwrap();
        assert!(app.status.starts_with("nothing to save"), "{}", app.status);
        assert!(!app.should_quit);
        assert!(
            list_items(&app.user_conn, None).unwrap().is_empty(),
            "nothing may be saved from empty results"
        );
    }

    #[test]
    fn enter_on_empty_query_is_silent() {
        let mut app = app();
        press(&mut app, KeyCode::Enter).unwrap();
        assert!(!app.should_quit);
        assert!(app.status.is_empty());
        assert!(
            list_items(&app.user_conn, None).unwrap().is_empty(),
            "nothing may be saved from an empty query"
        );
    }

    #[test]
    fn control_s_no_longer_saves() {
        let mut app = app();
        press(&mut app, KeyCode::Tab).unwrap();
        for ch in "school".chars() {
            press(&mut app, KeyCode::Char(ch)).unwrap();
        }
        assert!(!app.results.is_empty());
        app.on_key(&KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL))
            .unwrap();
        assert!(
            list_items(&app.user_conn, None).unwrap().is_empty(),
            "Ctrl+S must not save anything"
        );
        assert!(
            !app.status.starts_with("saved "),
            "Ctrl+S must not report a save: {:?}",
            app.status
        );
    }

    #[test]
    fn escape_clears_query_results_then_quits() {
        let mut app = app();
        for ch in "ni".chars() {
            press(&mut app, KeyCode::Char(ch)).unwrap();
        }
        assert!(!app.should_quit);
        press(&mut app, KeyCode::Esc).unwrap();
        assert!(app.query.is_empty(), "Esc clears a typed query");
        assert!(
            app.results.is_empty(),
            "live search leaves no stale results behind"
        );
        assert!(!app.should_quit);
        press(&mut app, KeyCode::Esc).unwrap();
        assert!(app.should_quit, "Esc with an empty query quits");
    }

    #[test]
    fn control_c_quits_regardless_of_query() {
        let mut app = app();
        for ch in "ni".chars() {
            press(&mut app, KeyCode::Char(ch)).unwrap();
        }
        app.on_key(&KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))
            .unwrap();
        assert!(app.should_quit);
        assert_eq!(app.query, "ni", "Ctrl+C leaves the typed query intact");
    }

    #[test]
    fn tab_cycles_search_modes() {
        let mut app = app();
        assert_eq!(app.mode, SearchMode::Pinyin);
        press(&mut app, KeyCode::Tab).unwrap();
        assert_eq!(app.mode, SearchMode::English);
        press(&mut app, KeyCode::Tab).unwrap();
        assert_eq!(app.mode, SearchMode::Chinese);
        press(&mut app, KeyCode::Tab).unwrap();
        assert_eq!(app.mode, SearchMode::Pinyin);
    }

    #[test]
    fn key_release_events_are_ignored() {
        let mut app = app();
        let mut release = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        app.on_key(&release).unwrap();
        assert!(!app.should_quit);
        assert_eq!(app.query, "");
    }

    #[test]
    fn up_down_wrap_through_results() {
        let mut app = app();
        press(&mut app, KeyCode::Tab).unwrap();
        for ch in "cat".chars() {
            press(&mut app, KeyCode::Char(ch)).unwrap();
        }
        assert!(app.results.len() >= 2);
        // Live search may restore a non-zero selection while typing, so
        // assert movement relative to wherever typing left the highlight.
        let len = app.results.len();
        let start = app.list_state.selected().unwrap();
        press(&mut app, KeyCode::Down).unwrap();
        assert_eq!(app.list_state.selected(), Some((start + 1) % len));
        press(&mut app, KeyCode::Down).unwrap();
        assert_eq!(app.list_state.selected(), Some((start + 2) % len));
        press(&mut app, KeyCode::Up).unwrap();
        assert_eq!(app.list_state.selected(), Some((start + 1) % len));
    }

    #[test]
    fn f1_shows_saved_items_newest_first() {
        let mut app = app();
        search_and_save(&mut app, "school");
        search_and_save(&mut app, "cat");

        press(&mut app, KeyCode::F(1)).unwrap();
        assert_eq!(app.view, View::Saved);
        assert_eq!(app.saved.len(), 2);
        assert!(
            app.saved[0].item_id > app.saved[1].item_id,
            "saved list is newest first"
        );
        assert_eq!(app.saved_state.selected(), Some(0));
        let detail = app.saved_detail();
        assert!(
            detail.contains(&display_definition(&app.saved[0].definition)),
            "detail shows the definition: {detail}"
        );
        assert!(
            !detail.contains("needs_review"),
            "no machine status names: {detail}"
        );
    }

    #[test]
    fn tab_cycles_saved_status_filter() {
        let mut app = app();
        search_and_save(&mut app, "school");
        press(&mut app, KeyCode::F(1)).unwrap();
        assert_eq!(app.saved_filter_idx, 0);
        assert_eq!(app.saved.len(), 1);

        press(&mut app, KeyCode::Tab).unwrap();
        assert_eq!(app.saved_filter_idx, 1);
        assert_eq!(app.saved.len(), 1, "the saved item is confirmed");

        press(&mut app, KeyCode::Tab).unwrap();
        assert_eq!(app.saved_filter_idx, 2);
        assert!(app.saved.is_empty(), "nothing needs review");

        press(&mut app, KeyCode::Tab).unwrap();
        assert_eq!(app.saved_filter_idx, 3);
        assert!(app.saved.is_empty(), "nothing exported");

        press(&mut app, KeyCode::Tab).unwrap();
        assert_eq!(app.saved_filter_idx, 4);
        assert!(app.saved.is_empty(), "nothing archived");

        press(&mut app, KeyCode::Tab).unwrap();
        assert_eq!(app.saved_filter_idx, 0);
        assert_eq!(app.saved.len(), 1, "wraps back to 'all'");
    }

    #[test]
    fn d_deletes_selected_saved_item() {
        let mut app = app();
        search_and_save(&mut app, "school");
        press(&mut app, KeyCode::F(1)).unwrap();
        assert_eq!(app.saved.len(), 1);
        press(&mut app, KeyCode::Char('d')).unwrap();
        assert_eq!(app.saved.len(), 1, "d alone only asks");
        assert!(app.status.contains("y deletes"), "{}", app.status);
        press(&mut app, KeyCode::Char('y')).unwrap();
        assert!(app.saved.is_empty());
        assert!(app.status.starts_with("deleted "), "{}", app.status);
        assert!(app.error.is_none(), "{:?}", app.error);
    }

    #[test]
    fn any_other_key_cancels_a_pending_delete() {
        let mut app = app();
        search_and_save(&mut app, "school");
        press(&mut app, KeyCode::F(1)).unwrap();
        press(&mut app, KeyCode::Delete).unwrap();
        press(&mut app, KeyCode::Down).unwrap();
        assert_eq!(app.saved.len(), 1, "cancelled delete keeps the item");
        assert!(app.status.starts_with("kept "), "{}", app.status);
        // The cancelling key is consumed; a later y does nothing.
        press(&mut app, KeyCode::Char('y')).unwrap();
        assert_eq!(app.saved.len(), 1);
    }

    #[test]
    fn escape_from_saved_returns_to_search() {
        let mut app = app();
        search_and_save(&mut app, "school");
        press(&mut app, KeyCode::F(1)).unwrap();
        assert_eq!(app.view, View::Saved);
        press(&mut app, KeyCode::Esc).unwrap();
        assert_eq!(app.view, View::Search);
    }

    #[test]
    fn f2_from_saved_returns_to_search() {
        let mut app = app();
        search_and_save(&mut app, "school");
        press(&mut app, KeyCode::F(1)).unwrap();
        press(&mut app, KeyCode::F(2)).unwrap();
        assert_eq!(app.view, View::Search);
    }

    #[test]
    fn research_restores_selection_by_identity() {
        let mut app = app();
        press(&mut app, KeyCode::Tab).unwrap();
        for ch in "cat".chars() {
            press(&mut app, KeyCode::Char(ch)).unwrap();
        }
        press(&mut app, KeyCode::Enter).unwrap();
        press(&mut app, KeyCode::Down).unwrap();
        press(&mut app, KeyCode::Down).unwrap();
        let selected = app.results[app.list_state.selected().unwrap()]
            .entry
            .simplified
            .clone();
        // Perturbing the query re-runs the search but keeps the same entry:
        // a trailing space still trims to the same query.
        press(&mut app, KeyCode::Char(' ')).unwrap();
        press(&mut app, KeyCode::Backspace).unwrap();
        let reselected = app.results[app.list_state.selected().unwrap()]
            .entry
            .simplified
            .clone();
        assert_eq!(selected, reselected);
    }

    #[test]
    fn saved_reload_preserves_selection_by_id() {
        let mut app = app();
        search_and_save(&mut app, "school");
        search_and_save(&mut app, "cat");

        press(&mut app, KeyCode::F(1)).unwrap();
        press(&mut app, KeyCode::Down).unwrap();
        let selected_id = app.saved[app.saved_state.selected().unwrap()].item_id;
        // Ctrl+R reloads without losing the selected item.
        app.on_key(&KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL))
            .unwrap();
        assert_eq!(
            app.saved[app.saved_state.selected().unwrap()].item_id,
            selected_id
        );
    }

    #[test]
    fn detail_scroll_moves_and_resets_on_selection_change() {
        let mut app = app();
        press(&mut app, KeyCode::Tab).unwrap();
        for ch in "cat".chars() {
            press(&mut app, KeyCode::Char(ch)).unwrap();
        }
        assert_eq!(app.detail_scroll, 0);
        press(&mut app, KeyCode::PageDown).unwrap();
        assert_eq!(app.detail_scroll, crate::ui::DETAIL_SCROLL_STEP);
        press(&mut app, KeyCode::PageUp).unwrap();
        assert_eq!(app.detail_scroll, 0);
        press(&mut app, KeyCode::PageDown).unwrap();
        press(&mut app, KeyCode::Down).unwrap();
        assert_eq!(
            app.detail_scroll, 0,
            "moving selection resets detail scroll"
        );
    }

    #[test]
    fn typing_clears_a_stale_error() {
        let mut app = app();
        app.error = Some(String::from("boom"));
        press(&mut app, KeyCode::Char('x')).unwrap();
        assert_eq!(app.error, None);
    }

    #[test]
    fn draw_renders_header_and_footer() {
        use ratatui::{Terminal, backend::TestBackend};

        let mut app = app();
        for ch in "ni".chars() {
            press(&mut app, KeyCode::Char(ch)).unwrap();
        }
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| app.draw(frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row = |y: u16| {
            (0..80)
                .map(|x| buffer[(x, y)].symbol().to_owned())
                .collect::<String>()
        };
        assert!(row(2).contains("shouci"), "header shows brand");
        assert!(row(2).contains("ni"), "header shows query");
        assert!(row(22).contains("Enter"), "footer shows save hint");
    }

    #[test]
    fn cursor_follows_the_query_for_input_methods() {
        use ratatui::{Terminal, backend::TestBackend, layout::Position};

        let mut app = app();
        for ch in "翻译".chars() {
            press(&mut app, KeyCode::Char(ch)).unwrap();
        }
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| app.draw(frame, frame.area()))
            .unwrap();
        // Header box at (1, 1); text "shouci [Pinyin]  翻译" is 21 cells wide
        // (Chinese is double width), so the cursor sits at column 2 + 21.
        assert_eq!(
            terminal.get_cursor_position().unwrap(),
            Position::new(23, 2)
        );
    }

    #[test]
    fn shift_tab_toggles_between_views() {
        let mut app = app();
        assert_eq!(app.view, View::Search);
        press(&mut app, KeyCode::BackTab).unwrap();
        assert_eq!(app.view, View::Saved);
        press(&mut app, KeyCode::BackTab).unwrap();
        assert_eq!(app.view, View::Search);
    }

    #[test]
    fn draw_narrow_terminal_shows_resize_notice() {
        use ratatui::{Terminal, backend::TestBackend};

        let mut app = app();
        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
        terminal
            .draw(|frame| app.draw(frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let mut text = String::new();
        for y in 0..10 {
            for x in 0..40 {
                text.push_str(buffer[(x, y)].symbol());
            }
        }
        assert!(text.contains("too small"), "narrow viewport shows notice");
        // Input still works while the notice is up.
        press(&mut app, KeyCode::Char('a')).unwrap();
        assert_eq!(app.query, "a");
    }
}
