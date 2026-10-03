use std::fmt::Write;
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
use shouci_core::{
    BulkAction, Config, ConnectorView, DictionaryResults, DictionaryStatus, Error, ExportRequest,
    ExportScope, ImportPolicy, ItemView, Lifecycle, ManualWord, MatchBasis, QueryKind, Result,
    SaveOutcome, SaveResult, Severity, Shouci, Verification, expand_tilde,
};

use crate::state::{
    Row, Scope, View, basis_label, counted, find_row_index, find_saved_index, kind_title,
    move_selection, next_kind, rows, scroll_into_view,
};
use crate::theme::Theme;
use crate::ui::{DETAIL_SCROLL_STEP, MIN_HEIGHT, MIN_WIDTH, plain_headword};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromptKind {
    Import,
    Export,
}

#[derive(Debug, Clone)]
struct TransferPrompt {
    kind: PromptKind,
    /// Export: the format, by connector id. Imports detect theirs.
    format: String,
    /// Import: what to do with words already saved.
    policy: ImportPolicy,
    /// Export: only the words that format doesn't have yet.
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

fn pane_title(name: &'static str, view: View, scope: Scope) -> Line<'static> {
    Line::from(vec![
        Span::styled(name, Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(
            format!(" · {}", crate::ui::pane_hint(view, scope)),
            Theme::dim(),
        ),
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

/// Where a transfer file goes unless another place is chosen: Downloads, as
/// in the Mac app, or home when there is no Downloads folder.
fn default_transfer_path(connector: &str) -> String {
    let home = expand_tilde(Path::new("~")).unwrap_or_default();
    let downloads = home.join("Downloads");
    let dir = if downloads.is_dir() { downloads } else { home };
    dir.join(format!("shouci-{connector}.txt"))
        .to_string_lossy()
        .into_owned()
}

fn policy_label(policy: ImportPolicy) -> &'static str {
    match policy {
        ImportPolicy::Skip => "skip",
        ImportPolicy::Merge => "merge",
        ImportPolicy::Overwrite => "overwrite",
    }
}

fn next_policy(policy: ImportPolicy) -> ImportPolicy {
    match policy {
        ImportPolicy::Skip => ImportPolicy::Merge,
        ImportPolicy::Merge => ImportPolicy::Overwrite,
        ImportPolicy::Overwrite => ImportPolicy::Skip,
    }
}

/// The date part of a stored timestamp (`2026-09-30T12:00:00.000Z`).
fn day(timestamp: &str) -> &str {
    timestamp.get(..10).unwrap_or(timestamp)
}

/// Two-pane terminal UI over the Shouci core:
///
/// * Search (`F2`): type to search live, Enter saves the selected result,
///   Ctrl+Z takes the save back. PgUp/PgDn scroll the detail pane.
/// * Saved (`F1`): saved words, newest first, in four lists (Tab): all,
///   needs review, archived, and the trash, with full detail for the
///   selection.
///
/// Operational errors (search/save/reload) are captured into a durable error
/// slot instead of killing the event loop; transient feedback goes to status.
pub struct App<'a> {
    shouci: &'a Shouci,
    theme: Theme,
    /// How to read the query; `None` lets the core work it out.
    kind: Option<QueryKind>,
    query: String,
    /// The last search, for how the query was read.
    found: Option<DictionaryResults>,
    rows: Vec<Row>,
    list_state: ListState,
    view: View,
    saved: Vec<ItemView>,
    saved_state: ListState,
    scope: Scope,
    status: String,
    error: Option<String>,
    detail_scroll: u16,
    prompt: Option<TransferPrompt>,
    /// Set by the prompt when Enter should hand over to the native panel.
    picker: Option<PickerRequest>,
    /// A word in the trash awaiting a `y` to delete it for good:
    /// `(item_id, headword)`. That can't be undone, so `d` only arms it.
    pending_purge: Option<(i64, String)>,
    /// The last save from search, while Ctrl+Z can take it back.
    last_save: Option<SaveResult>,
    pub should_quit: bool,
}

impl<'a> App<'a> {
    #[must_use]
    pub fn new(shouci: &'a Shouci) -> Self {
        Self {
            shouci,
            theme: Theme::detect(),
            kind: None,
            query: String::new(),
            found: None,
            rows: Vec::new(),
            list_state: ListState::default(),
            view: View::Search,
            saved: Vec::new(),
            saved_state: ListState::default(),
            scope: Scope::All,
            status: String::new(),
            error: None,
            detail_scroll: 0,
            prompt: None,
            picker: None,
            pending_purge: None,
            last_save: None,
            should_quit: false,
        }
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

    fn selected_row(&self) -> Option<&Row> {
        self.list_state
            .selected()
            .and_then(|index| self.rows.get(index))
    }

    fn selected_saved(&self) -> Option<&ItemView> {
        self.saved_state
            .selected()
            .and_then(|index| self.saved.get(index))
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
    /// None today: failures land in the error slot. The signature leaves
    /// room for errors the loop cannot recover from.
    pub fn on_key(&mut self, key: &KeyEvent) -> Result<()> {
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }
        if let Some((item_id, head)) = self.pending_purge.take() {
            let confirmed = matches!(key.code, KeyCode::Char('y' | 'Y'))
                && !key.modifiers.contains(KeyModifiers::CONTROL);
            if confirmed {
                let op = self.purge(item_id, &head);
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
            // per-view meaning (how to read the query / which list).
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
            KeyCode::Char('z') => {
                let op = self.undo();
                self.capture(op);
            }
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
        let previous = self.saved_state.selected();
        let previous_id = self.selected_saved().map(|item| item.id);
        self.saved = self.shouci.list_items(&self.scope.filter())?;
        // Keep the selected word when it is still listed; when it left the
        // list (trashed, archived), stay at the same place in it.
        let restored = previous_id
            .and_then(|id| find_saved_index(&self.saved, id))
            .or(previous.map(|index| index.min(self.saved.len().saturating_sub(1))));
        self.saved_state.select(if self.saved.is_empty() {
            None
        } else {
            Some(restored.unwrap_or(0))
        });
        self.detail_scroll = 0;
        self.succeeded();
        Ok(())
    }

    /// Applies `action` to the selected saved word and says what happened.
    fn change_selected(
        &mut self,
        action: &BulkAction,
        done: impl FnOnce(&str) -> String,
    ) -> Result<()> {
        let Some(item) = self.selected_saved() else {
            self.status = String::from("nothing selected");
            return Ok(());
        };
        let (id, head) = (item.id, item.simplified.clone());
        self.shouci.bulk(&[id], action)?;
        self.reload_saved()?;
        self.status = done(&head);
        Ok(())
    }

    /// `d`: moves the selected word to the trash, or, in the trash, asks
    /// before deleting it for good (the next key confirms with `y`; see
    /// [`App::on_key`]).
    fn trash_selected(&mut self) -> Result<()> {
        if self.scope != Scope::Trash {
            return self.change_selected(&BulkAction::Trash, |head| {
                format!("moved {head} to the trash")
            });
        }
        let Some((id, head)) = self
            .selected_saved()
            .map(|item| (item.id, item.simplified.clone()))
        else {
            self.status = String::from("nothing selected");
            return Ok(());
        };
        self.status =
            format!("delete {head} for good? can't be undone · y deletes · any other key keeps it");
        self.pending_purge = Some((id, head));
        Ok(())
    }

    fn purge(&mut self, item_id: i64, head: &str) -> Result<()> {
        self.shouci.bulk(&[item_id], &BulkAction::Purge)?;
        self.reload_saved()?;
        self.status = format!("deleted {head} for good");
        Ok(())
    }

    /// `n`: needs review, or checked again.
    fn toggle_review(&mut self) -> Result<()> {
        let Some(item) = self.selected_saved() else {
            self.status = String::from("nothing selected");
            return Ok(());
        };
        let to = if item.verification == Verification::NeedsReview {
            Verification::Confirmed
        } else {
            Verification::NeedsReview
        };
        self.change_selected(&BulkAction::SetVerification(to), |head| match to {
            Verification::NeedsReview => format!("marked {head} as needing review"),
            Verification::Confirmed => format!("marked {head} as checked"),
        })
    }

    /// Takes back the last save from search: a new word is deleted, a word
    /// brought back from the trash goes back there.
    fn undo(&mut self) -> Result<()> {
        let Some(saved) = self.last_save.take() else {
            self.status = String::from("nothing to take back");
            return Ok(());
        };
        let ids = [saved.item.id];
        self.shouci.bulk(&ids, &BulkAction::Trash)?;
        if saved.outcome == SaveOutcome::Inserted {
            self.shouci.bulk(&ids, &BulkAction::Purge)?;
        }
        match self.view {
            View::Search => self.run_search()?,
            View::Saved => self.reload_saved()?,
        }
        self.status = format!("took back {}", saved.item.simplified);
        Ok(())
    }

    /// The formats words can be exported to. Imports detect theirs.
    fn export_formats(&self) -> Vec<ConnectorView> {
        self.shouci
            .connectors()
            .into_iter()
            .filter(|connector| connector.can_export)
            .collect()
    }

    fn format_name(&self, id: &str) -> String {
        self.shouci
            .connectors()
            .into_iter()
            .find(|connector| connector.id == id)
            .map_or_else(|| id.to_owned(), |connector| connector.name)
    }

    fn begin_prompt(&mut self, kind: PromptKind) {
        let format = self
            .export_formats()
            .first()
            .map(|connector| connector.id.clone())
            .unwrap_or_default();
        self.prompt = Some(TransferPrompt {
            kind,
            path: default_transfer_path(&format),
            format,
            policy: ImportPolicy::Skip,
            only_new: true,
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
        let formats = self.export_formats();
        let Some(prompt) = self.prompt.as_mut() else {
            return;
        };
        match code {
            KeyCode::Char('q' | 'c') => {
                self.should_quit = true;
                return;
            }
            KeyCode::Char('t') if prompt.kind == PromptKind::Export && !formats.is_empty() => {
                let at = formats
                    .iter()
                    .position(|format| format.id == prompt.format)
                    .map_or(0, |at| (at + 1) % formats.len());
                prompt.format.clone_from(&formats[at].id);
                if !prompt.typing {
                    prompt.path = default_transfer_path(&prompt.format);
                }
            }
            KeyCode::Char('n') if prompt.kind == PromptKind::Export => {
                prompt.only_new = !prompt.only_new;
            }
            KeyCode::Char('p') if prompt.kind == PromptKind::Import => {
                prompt.policy = next_policy(prompt.policy);
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
        let typed = prompt.path.trim();
        if typed.is_empty() {
            self.status = String::from("type a file path first (Esc cancels)");
            return Ok(());
        }
        let path = expand_tilde(Path::new(typed))?;
        let status = match prompt.kind {
            PromptKind::Import => self.import(&path, prompt.policy)?,
            PromptKind::Export => self.export(&path, &prompt.format, prompt.only_new)?,
        };
        self.prompt = None;
        self.reload_saved()?;
        self.status = status;
        Ok(())
    }

    /// Imports a file in whichever format reads it, and says what changed.
    fn import(&self, path: &Path, policy: ImportPolicy) -> Result<String> {
        let plan = self.shouci.detect_import(path, policy, false)?.plan;
        if plan.refused {
            let errors = plan.counts().errors;
            let first = plan
                .issues
                .iter()
                .find(|issue| issue.severity == Severity::Error)
                .map(|issue| format!(" (line {}: {})", issue.line, issue.message))
                .unwrap_or_default();
            return Err(Error::invalid(format!(
                "{} {} errors, so nothing was imported{first}",
                counted(errors, "line", "lines"),
                if errors == 1 { "has" } else { "have" }
            )));
        }
        let summary = self.shouci.apply_import(&plan)?;
        let mut added = format!("added {}", counted(summary.inserted, "word", "words"));
        if summary.unresolved > 0 {
            let _ = write!(added, " ({} to review)", summary.unresolved);
        }
        let mut parts = vec![added];
        for (count, done) in [
            (summary.updated, "updated"),
            (summary.skipped, "skipped"),
            (summary.dropped, "dropped"),
        ] {
            if count > 0 {
                parts.push(format!("{done} {count}"));
            }
        }
        Ok(format!(
            "{} from {} file {}",
            parts.join(", "),
            self.format_name(&plan.connector_id),
            path.display()
        ))
    }

    /// Exports to `format`, and says what was written and left out.
    fn export(&self, path: &Path, format: &str, only_new: bool) -> Result<String> {
        let request = ExportRequest {
            scope: if only_new {
                ExportScope::New
            } else {
                ExportScope::All
            },
            ..ExportRequest::default()
        };
        let plan = self.shouci.preview_export(path, format, &request)?;
        let name = self.format_name(format);
        let mut left = Vec::new();
        if plan.left_out_already_there > 0 {
            left.push(format!(
                "{} already in {name}",
                counted(plan.left_out_already_there, "word", "words")
            ));
        }
        if plan.left_out_needs_review > 0 {
            left.push(format!(
                "{} needing review",
                counted(plan.left_out_needs_review, "word", "words")
            ));
        }
        let left = if left.is_empty() {
            String::new()
        } else {
            format!(" · left out {}", left.join(", "))
        };
        if plan.item_ids.is_empty() {
            return Ok(format!("nothing to export, so no file was written{left}"));
        }
        let summary = self.shouci.apply_export(&plan)?;
        Ok(format!(
            "wrote {} to {}{left}",
            counted(summary.written, "word", "words"),
            summary.path
        ))
    }

    fn refresh_prompt_status(&mut self) {
        let Some(prompt) = self.prompt.clone() else {
            return;
        };
        let typed = prompt.path.trim();
        // Show the real destination, never the shorthand, so there is nothing
        // left to guess about where the file lands.
        let resolved = expand_tilde(Path::new(typed)).unwrap_or_default();
        let state = if typed.is_empty() {
            "no path"
        } else if prompt.kind == PromptKind::Import {
            if resolved.exists() {
                "file found"
            } else {
                "file missing"
            }
        } else if resolved.exists() {
            "will replace"
        } else {
            "new file"
        };
        let action = match (prompt.typing, prompt.kind) {
            (true, _) => "Enter run",
            (false, PromptKind::Import) => "Enter choose file",
            (false, PromptKind::Export) => "Enter choose location",
        };
        let path = resolved.display();
        self.status = match prompt.kind {
            PromptKind::Import => format!(
                "import Pleco or Anki · words you have: {} · {state} · {path}  {action} · \
                 Ctrl+P words you have · Esc cancel",
                policy_label(prompt.policy)
            ),
            PromptKind::Export => format!(
                "export to {} · {} · {state} · {path}  {action} · Ctrl+T format · Ctrl+N {} · \
                 Esc cancel",
                self.format_name(&prompt.format),
                if prompt.only_new {
                    "new words only"
                } else {
                    "every word"
                },
                if prompt.only_new {
                    "every word"
                } else {
                    "new only"
                }
            ),
        };
    }

    fn on_saved_key(&mut self, code: KeyCode) {
        let trash = self.scope == Scope::Trash;
        let op = match code {
            KeyCode::Esc => {
                self.show_search();
                Ok(())
            }
            KeyCode::Char('i') if !trash => {
                self.begin_prompt(PromptKind::Import);
                Ok(())
            }
            KeyCode::Char('e') if !trash => {
                self.begin_prompt(PromptKind::Export);
                Ok(())
            }
            KeyCode::Delete | KeyCode::Char('d') => self.trash_selected(),
            KeyCode::Char('r') if trash => self.change_selected(&BulkAction::Restore, |head| {
                format!("brought {head} back from the trash")
            }),
            KeyCode::Char('a') => match self.scope {
                Scope::All | Scope::NeedsReview => {
                    self.change_selected(&BulkAction::Archive, |head| format!("archived {head}"))
                }
                Scope::Archived => self
                    .change_selected(&BulkAction::Unarchive, |head| format!("unarchived {head}")),
                Scope::Trash => Ok(()),
            },
            KeyCode::Char('n') if !trash => self.toggle_review(),
            KeyCode::Tab => {
                self.scope = self.scope.next();
                self.reload_saved().map(|()| {
                    self.status = format!(
                        "showing {} · {}",
                        self.scope.label(),
                        counted(
                            u64::try_from(self.saved.len()).unwrap_or(u64::MAX),
                            "word",
                            "words"
                        )
                    );
                })
            }
            KeyCode::Down => {
                move_selection(&mut self.saved_state, self.saved.len(), true);
                self.detail_scroll = 0;
                Ok(())
            }
            KeyCode::Up => {
                move_selection(&mut self.saved_state, self.saved.len(), false);
                self.detail_scroll = 0;
                Ok(())
            }
            _ => Ok(()),
        };
        self.capture(op);
    }

    fn on_search_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                if self.query.is_empty() {
                    self.should_quit = true;
                } else {
                    self.query.clear();
                    self.found = None;
                    self.rows.clear();
                    self.list_state.select(None);
                    self.detail_scroll = 0;
                    self.error = None;
                    self.status = String::from("query cleared");
                }
            }
            KeyCode::Tab => {
                self.kind = next_kind(self.kind);
                self.status = match self.kind {
                    Some(kind) => format!("reading your search as {}", kind_title(kind)),
                    None => String::from("working out what you type"),
                };
                if !self.query.trim().is_empty() {
                    let op = self.run_search();
                    self.capture(op);
                }
            }
            KeyCode::Char(c) if !c.is_control() => {
                self.query.push(c);
                self.typed();
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.typed();
            }
            KeyCode::Enter => {
                if self.list_state.selected().is_some() {
                    let op = self.save_selected();
                    self.capture(op);
                } else if !self.query.is_empty() {
                    self.status = String::from(
                        "nothing to save: no matches (Tab changes how your search is read)",
                    );
                }
            }
            KeyCode::Down => {
                move_selection(&mut self.list_state, self.rows.len(), true);
                self.detail_scroll = 0;
            }
            KeyCode::Up => {
                move_selection(&mut self.list_state, self.rows.len(), false);
                self.detail_scroll = 0;
            }
            _ => {}
        }
    }

    /// The query changed: search again. A save can no longer be taken back
    /// once the search has moved on.
    fn typed(&mut self) {
        self.error = None;
        self.last_save = None;
        let op = self.run_search();
        self.capture(op);
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
            self.found = None;
            self.rows.clear();
            self.list_state.select(None);
            self.detail_scroll = 0;
            return Ok(());
        }
        // Keep the user's place when the same query is searched again (after
        // a save, say); a changed query starts from its best match.
        let previous = self
            .found
            .as_ref()
            .filter(|found| found.query == query)
            .and(self.selected_row())
            .map(Row::key);
        let found = match self.shouci.search_dictionary(&query, self.kind, None) {
            Ok(found) => found,
            Err(err) => {
                self.found = None;
                self.rows.clear();
                self.list_state.select(None);
                return Err(err);
            }
        };
        self.rows = rows(&found);
        self.found = Some(found);
        let restored = previous
            .as_deref()
            .and_then(|key| find_row_index(&self.rows, key));
        self.list_state = ListState::default();
        self.list_state.select(if self.rows.is_empty() {
            None
        } else {
            Some(restored.unwrap_or(0))
        });
        self.detail_scroll = 0;
        self.succeeded();
        self.status = self.search_status();
        Ok(())
    }

    fn search_status(&self) -> String {
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

    fn save_selected(&mut self) -> Result<()> {
        let Some(row) = self.selected_row().cloned() else {
            return Ok(());
        };
        let result = match &row {
            Row::Candidate(candidate) => self.shouci.save_candidate(candidate)?,
            Row::AsTyped(text) => self.shouci.add_manual(&ManualWord {
                simplified: text.clone(),
                ..ManualWord::default()
            })?,
        };
        // Search again so the result shows as saved.
        self.run_search()?;
        let item = &result.item;
        let word = plain_headword(&item.simplified, &item.traditional, &item.pinyin_display);
        self.status = match result.outcome {
            SaveOutcome::Inserted if item.pinyin.is_empty() => {
                format!("saved {word} to fill in later · Ctrl+Z takes it back")
            }
            SaveOutcome::Inserted => format!("saved {word} · Ctrl+Z takes it back"),
            SaveOutcome::Restored => {
                format!("brought {word} back from the trash · Ctrl+Z takes it back")
            }
            SaveOutcome::AlreadySaved if item.verification == Verification::NeedsReview => {
                format!("already saved {word} (needs review)")
            }
            SaveOutcome::AlreadySaved => format!("already saved {word}"),
            SaveOutcome::Completed => format!("filled in {word}"),
        };
        self.last_save = matches!(
            result.outcome,
            SaveOutcome::Inserted | SaveOutcome::Restored
        )
        .then_some(result);
        Ok(())
    }

    /// How the query is read, for the header: chosen, worked out, or not
    /// yet known.
    fn mode_label(&self) -> String {
        match (self.kind, &self.found) {
            (Some(kind), _) => kind_title(kind).to_owned(),
            (None, Some(found)) => format!("{} · auto", kind_title(found.kind)),
            (None, None) => String::from("auto"),
        }
    }

    fn selected_detail(&self) -> String {
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

    fn saved_detail(&self) -> String {
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
                .map(|id| self.format_name(id))
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

    fn render_results(&mut self, frame: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect) {
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

    fn render_saved(&mut self, frame: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect) {
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
            &self.mode_label(),
            &self.query,
            self.saved.len(),
            self.scope,
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
        let keys = match self.view {
            View::Search => crate::ui::SEARCH_KEYS,
            View::Saved => crate::ui::saved_keys(self.scope),
        };
        frame.render_widget(
            Paragraph::new(crate::ui::help_line(
                theme,
                keys,
                &self.status,
                self.error.as_deref(),
                panes.footer.width,
            ))
            .block(Block::default().borders(Borders::NONE)),
            panes.footer,
        );
    }
}

/// The library, ready to search, and what to tell the user once the TUI is
/// up.
struct Opened {
    shouci: Shouci,
    /// Words brought over, dictionary notes.
    notes: Vec<String>,
    /// Why search is unavailable.
    problem: Option<String>,
}

/// Opens the library and loads the dictionaries before the TUI takes the
/// screen, so a first-run download can say so in the terminal. Only a first
/// run downloads: the monthly refresh is left to `shouci dictionaries
/// update` and the Mac app.
fn open() -> Result<Opened> {
    let mut config = Config::from_env();
    config.fetch_dictionaries = false;
    let shouci = Shouci::open(config)?;
    let mut notes = shouci.startup_notes().to_vec();
    let shouci = if shouci.dictionaries()?.is_empty() {
        eprintln!(
            "shouci-tui: downloading the dictionary (first run only; needs an internet \
             connection)…"
        );
        let mut config = shouci.config().clone();
        config.fetch_dictionaries = true;
        drop(shouci);
        let fetched = Shouci::open(config)?;
        notes.extend(fetched.startup_notes().iter().cloned());
        fetched
    } else {
        shouci
    };
    let problem = shouci.load_dictionaries().err().map(|err| err.to_string());
    if let DictionaryStatus::Ready { notes: more, .. } = shouci.dictionary_status() {
        notes.extend(more);
    }
    Ok(Opened {
        shouci,
        notes,
        problem,
    })
}

/// Opens the library (see `SHOUCI_HOME`) and runs the TUI until the user
/// quits.
///
/// # Errors
///
/// Returns an error if the library cannot be opened, the terminal cannot be
/// entered/exited, or an event cannot be read.
pub fn run() -> Result<()> {
    let opened = open()?;

    enable_raw_mode().map_err(|err| Error::new(format!("enable raw mode: {err}")))?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)
        .map_err(|err| Error::new(format!("enter alternate screen: {err}")))?;
    let mut terminal = match Terminal::new(CrosstermBackend::new(stdout)) {
        Ok(t) => t,
        Err(err) => {
            let _ = execute!(io::stdout(), LeaveAlternateScreen);
            disable_raw_mode().ok();
            return Err(Error::new(format!("terminal init: {err}")));
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

    let result = event_loop(&mut terminal, &opened);

    disable_raw_mode().map_err(|err| Error::new(format!("disable raw mode: {err}")))?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .map_err(|err| Error::new(format!("leave alternate screen: {err}")))?;
    terminal
        .show_cursor()
        .map_err(|err| Error::new(format!("restore cursor: {err}")))?;

    result
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    opened: &Opened,
) -> Result<()> {
    let mut app = App::new(&opened.shouci);
    app.status = opened.notes.join(" · ");
    app.error.clone_from(&opened.problem);
    while !app.should_quit {
        terminal
            .draw(|frame| app.draw(frame, frame.area()))
            .map_err(|err| Error::new(format!("draw: {err}")))?;
        if let Event::Key(key) =
            event::read().map_err(|err| Error::new(format!("read event: {err}")))?
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
    use crossterm::event::KeyEventKind;
    use shouci_core::LibraryFilter;
    use shouci_core::testing::{Sandbox, sandbox, scratch_dir};

    use super::*;

    fn press(app: &mut App<'_>, code: KeyCode) {
        app.on_key(&KeyEvent::new(code, KeyModifiers::NONE))
            .expect("key handled");
    }

    fn ctrl(app: &mut App<'_>, code: KeyCode) {
        app.on_key(&KeyEvent::new(code, KeyModifiers::CONTROL))
            .expect("key handled");
    }

    fn type_text(app: &mut App<'_>, text: &str) {
        for ch in text.chars() {
            press(app, KeyCode::Char(ch));
        }
    }

    /// Saved words outside the trash, newest first.
    fn saved_heads(library: &Sandbox) -> Vec<String> {
        library
            .list_items(&LibraryFilter::default())
            .unwrap()
            .into_iter()
            .map(|item| item.simplified)
            .collect()
    }

    fn search_and_save(app: &mut App<'_>, word: &str) {
        if !app.query.is_empty() {
            // Esc on a non-empty query clears it (never quits here).
            press(app, KeyCode::Esc);
        }
        type_text(app, word);
        // Live search already populated results; Enter saves the selection.
        press(app, KeyCode::Enter);
        assert!(
            app.status.starts_with("saved ") || app.status.starts_with("already saved "),
            "unexpected status after Enter: {:?}",
            app.status
        );
    }

    fn headword(row: &Row) -> &str {
        match row {
            Row::Candidate(candidate) => &candidate.simplified,
            Row::AsTyped(text) => text,
        }
    }

    // --- Search ----------------------------------------------------------

    #[test]
    fn typing_populates_results_live() {
        let library = sandbox();
        let mut app = App::new(&library);
        type_text(&mut app, "school");
        // No Enter needed: results arrive as you type.
        assert_eq!(app.rows.len(), 1);
        assert_eq!(headword(&app.rows[0]), "学校");
        assert_eq!(app.list_state.selected(), Some(0));
        assert_eq!(app.mode_label(), "English · auto");
        assert_eq!(app.status, "1 result · read as English");
    }

    #[test]
    fn tab_chooses_how_the_query_is_read() {
        let library = sandbox();
        let mut app = App::new(&library);
        type_text(&mut app, "mao");
        assert_eq!(headword(&app.rows[0]), "猫", "worked out as pinyin");
        // Tab: Hanzi, Pinyin, English, then back to working it out
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.kind, Some(QueryKind::Chinese));
        assert_eq!(app.rows, [], "mao is not characters, so not kept as typed");
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.mode_label(), "Pinyin");
        assert_eq!(headword(&app.rows[0]), "猫");
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.kind, None);
        assert_eq!(app.mode_label(), "Pinyin · auto");
    }

    #[test]
    fn hotkey_letters_always_type_when_query_is_empty() {
        let library = sandbox();
        let mut app = App::new(&library);
        type_text(&mut app, "cepsq");
        assert_eq!(app.query, "cepsq");
        assert!(!app.should_quit);
    }

    #[test]
    fn enter_saves_the_selection_and_marks_it() {
        let library = sandbox();
        let mut app = App::new(&library);
        type_text(&mut app, "school");
        press(&mut app, KeyCode::Enter);
        assert!(
            app.status.starts_with("saved 学校 / 學校 [xué xiào]"),
            "unexpected status: {:?}",
            app.status
        );
        assert_eq!(saved_heads(&library), ["学校"]);
        let Row::Candidate(candidate) = &app.rows[0] else {
            panic!("a dictionary row");
        };
        assert!(candidate.saved.is_some(), "results show it as saved");
        press(&mut app, KeyCode::Enter);
        assert!(
            app.status.starts_with("already saved 学校"),
            "{}",
            app.status
        );
    }

    #[test]
    fn control_z_takes_a_save_back() {
        let library = sandbox();
        let mut app = App::new(&library);
        type_text(&mut app, "school");
        press(&mut app, KeyCode::Enter);
        ctrl(&mut app, KeyCode::Char('z'));
        assert_eq!(app.status, "took back 学校");
        assert_eq!(saved_heads(&library), [] as [String; 0]);
        ctrl(&mut app, KeyCode::Char('z'));
        assert_eq!(app.status, "nothing to take back");
        // Typing moves on: the next save is not taken back by an old Ctrl+Z
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Backspace);
        ctrl(&mut app, KeyCode::Char('z'));
        assert_eq!(saved_heads(&library), ["学校"]);
    }

    #[test]
    fn characters_no_dictionary_has_are_kept_to_fill_in_later() {
        let library = sandbox();
        let mut app = App::new(&library);
        type_text(&mut app, "旅行社");
        assert_eq!(app.rows[0], Row::AsTyped("旅行社".to_owned()));
        assert!(
            app.selected_detail()
                .contains("No dictionary has this word")
        );
        press(&mut app, KeyCode::Enter);
        assert!(
            app.status.starts_with("saved 旅行社 to fill in later"),
            "{}",
            app.status
        );
        let saved = library
            .list_items(&LibraryFilter::default())
            .unwrap()
            .remove(0);
        assert_eq!(saved.simplified, "旅行社");
        assert_eq!(saved.verification, Verification::NeedsReview);
    }

    #[test]
    fn enter_with_no_results_reports_nothing_to_save() {
        let library = sandbox();
        let mut app = App::new(&library);
        type_text(&mut app, "zzzqqq");
        assert_eq!(app.rows, []);
        press(&mut app, KeyCode::Enter);
        assert!(app.status.starts_with("nothing to save"), "{}", app.status);
        assert!(!app.should_quit);
        assert_eq!(saved_heads(&library), [] as [String; 0]);
    }

    #[test]
    fn enter_on_empty_query_is_silent() {
        let library = sandbox();
        let mut app = App::new(&library);
        press(&mut app, KeyCode::Enter);
        assert!(!app.should_quit);
        assert_eq!(app.status, "");
        assert_eq!(saved_heads(&library), [] as [String; 0]);
    }

    #[test]
    fn control_s_does_not_save() {
        let library = sandbox();
        let mut app = App::new(&library);
        type_text(&mut app, "school");
        assert_ne!(app.rows, []);
        ctrl(&mut app, KeyCode::Char('s'));
        assert_eq!(saved_heads(&library), [] as [String; 0]);
        assert!(!app.status.starts_with("saved "), "{:?}", app.status);
    }

    #[test]
    fn escape_clears_query_results_then_quits() {
        let library = sandbox();
        let mut app = App::new(&library);
        type_text(&mut app, "ni");
        assert!(!app.should_quit);
        press(&mut app, KeyCode::Esc);
        assert!(app.query.is_empty(), "Esc clears a typed query");
        assert!(app.rows.is_empty(), "no stale results left behind");
        assert!(!app.should_quit);
        press(&mut app, KeyCode::Esc);
        assert!(app.should_quit, "Esc with an empty query quits");
    }

    #[test]
    fn control_c_quits_regardless_of_query() {
        let library = sandbox();
        let mut app = App::new(&library);
        type_text(&mut app, "ni");
        ctrl(&mut app, KeyCode::Char('c'));
        assert!(app.should_quit);
        assert_eq!(app.query, "ni", "Ctrl+C leaves the typed query intact");
    }

    #[test]
    fn key_release_events_are_ignored() {
        let library = sandbox();
        let mut app = App::new(&library);
        let mut release = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        app.on_key(&release).unwrap();
        assert!(!app.should_quit);
        assert_eq!(app.query, "");
    }

    #[test]
    fn up_down_wrap_through_results() {
        let library = sandbox();
        let mut app = App::new(&library);
        type_text(&mut app, "cat");
        assert!(app.rows.len() >= 2);
        // Live search may restore a non-zero selection while typing, so
        // assert movement relative to wherever typing left the highlight.
        let len = app.rows.len();
        let start = app.list_state.selected().unwrap();
        press(&mut app, KeyCode::Down);
        assert_eq!(app.list_state.selected(), Some((start + 1) % len));
        press(&mut app, KeyCode::Down);
        assert_eq!(app.list_state.selected(), Some((start + 2) % len));
        press(&mut app, KeyCode::Up);
        assert_eq!(app.list_state.selected(), Some((start + 1) % len));
    }

    #[test]
    fn research_restores_selection_by_identity() {
        let library = sandbox();
        let mut app = App::new(&library);
        type_text(&mut app, "cat");
        press(&mut app, KeyCode::Down);
        let selected = headword(app.selected_row().unwrap()).to_owned();
        // A trailing space trims to the same query and searches again.
        press(&mut app, KeyCode::Char(' '));
        press(&mut app, KeyCode::Backspace);
        assert_eq!(headword(app.selected_row().unwrap()), selected);
    }

    #[test]
    fn detail_scroll_moves_and_resets_on_selection_change() {
        let library = sandbox();
        let mut app = App::new(&library);
        type_text(&mut app, "cat");
        assert_eq!(app.detail_scroll, 0);
        press(&mut app, KeyCode::PageDown);
        assert_eq!(app.detail_scroll, crate::ui::DETAIL_SCROLL_STEP);
        press(&mut app, KeyCode::PageUp);
        assert_eq!(app.detail_scroll, 0);
        press(&mut app, KeyCode::PageDown);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.detail_scroll, 0, "moving selection resets the scroll");
    }

    #[test]
    fn typing_clears_a_stale_error() {
        let library = sandbox();
        let mut app = App::new(&library);
        app.error = Some(String::from("boom"));
        press(&mut app, KeyCode::Char('x'));
        assert_eq!(app.error, None);
    }

    #[test]
    fn search_without_a_dictionary_says_why() {
        let library = shouci_core::testing::empty_sandbox();
        let mut app = App::new(&library);
        press(&mut app, KeyCode::Char('x'));
        let error = app.error.clone().expect("an error");
        assert!(error.contains("not loaded"), "{error}");
    }

    // --- Saved -----------------------------------------------------------

    #[test]
    fn f1_shows_saved_words_newest_first() {
        let library = sandbox();
        let mut app = App::new(&library);
        search_and_save(&mut app, "school");
        search_and_save(&mut app, "cat");

        press(&mut app, KeyCode::F(1));
        assert_eq!(app.view, View::Saved);
        assert_eq!(app.saved.len(), 2);
        assert!(app.saved[0].id > app.saved[1].id, "newest first");
        assert_eq!(app.saved_state.selected(), Some(0));
        let detail = app.saved_detail();
        assert!(
            detail.contains(&app.saved[0].definition_display),
            "detail shows the definition: {detail}"
        );
        assert!(
            !detail.contains("needs_review"),
            "no machine names: {detail}"
        );
    }

    #[test]
    fn tab_cycles_the_saved_lists() {
        let library = sandbox();
        let mut app = App::new(&library);
        search_and_save(&mut app, "school");
        press(&mut app, KeyCode::F(1));
        assert_eq!(app.scope, Scope::All);
        assert_eq!(app.saved.len(), 1);
        for (scope, len) in [
            (Scope::NeedsReview, 0),
            (Scope::Archived, 0),
            (Scope::Trash, 0),
            (Scope::All, 1),
        ] {
            press(&mut app, KeyCode::Tab);
            assert_eq!(app.scope, scope);
            assert_eq!(app.saved.len(), len, "{scope:?}");
        }
    }

    #[test]
    fn d_moves_to_the_trash_and_r_brings_back() {
        let library = sandbox();
        let mut app = App::new(&library);
        search_and_save(&mut app, "school");
        press(&mut app, KeyCode::F(1));
        press(&mut app, KeyCode::Char('d'));
        assert_eq!(app.saved, []);
        assert_eq!(app.status, "moved 学校 to the trash");
        // Over in the trash, r restores
        for _ in 0..3 {
            press(&mut app, KeyCode::Tab);
        }
        assert_eq!(app.scope, Scope::Trash);
        assert_eq!(app.saved.len(), 1);
        press(&mut app, KeyCode::Char('r'));
        assert_eq!(app.saved, []);
        assert_eq!(saved_heads(&library), ["学校"]);
    }

    #[test]
    fn deleting_from_the_trash_asks_first() {
        let library = sandbox();
        let mut app = App::new(&library);
        search_and_save(&mut app, "school");
        press(&mut app, KeyCode::F(1));
        press(&mut app, KeyCode::Char('d'));
        for _ in 0..3 {
            press(&mut app, KeyCode::Tab);
        }
        press(&mut app, KeyCode::Char('d'));
        assert_eq!(app.saved.len(), 1, "d alone only asks");
        assert!(app.status.contains("y deletes"), "{}", app.status);
        // Any other key keeps it
        press(&mut app, KeyCode::Down);
        assert!(app.status.starts_with("kept "), "{}", app.status);
        press(&mut app, KeyCode::Char('y'));
        assert_eq!(app.saved.len(), 1, "a later y does nothing");
        // y deletes
        press(&mut app, KeyCode::Char('d'));
        press(&mut app, KeyCode::Char('y'));
        assert_eq!(app.saved, []);
        assert_eq!(app.status, "deleted 学校 for good");
        assert!(app.error.is_none(), "{:?}", app.error);
    }

    #[test]
    fn a_archives_and_n_marks_for_review() {
        let library = sandbox();
        let mut app = App::new(&library);
        search_and_save(&mut app, "school");
        search_and_save(&mut app, "cat");
        press(&mut app, KeyCode::F(1));
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.saved[0].verification, Verification::NeedsReview);
        assert!(app.status.starts_with("marked "), "{}", app.status);
        press(&mut app, KeyCode::Char('a'));
        assert_eq!(app.saved.len(), 1, "archived words leave the list");
        assert_eq!(
            app.saved_state.selected(),
            Some(0),
            "the selection stays in place"
        );
        press(&mut app, KeyCode::Tab);
        assert_eq!(
            app.saved.len(),
            0,
            "the archived word no longer needs review here"
        );
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.saved.len(), 1);
        press(&mut app, KeyCode::Char('a'));
        assert!(app.status.starts_with("unarchived "), "{}", app.status);
    }

    #[test]
    fn escape_and_f2_return_to_search() {
        let library = sandbox();
        let mut app = App::new(&library);
        press(&mut app, KeyCode::F(1));
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.view, View::Search);
        press(&mut app, KeyCode::F(1));
        press(&mut app, KeyCode::F(2));
        assert_eq!(app.view, View::Search);
    }

    #[test]
    fn saved_reload_preserves_selection_by_id() {
        let library = sandbox();
        let mut app = App::new(&library);
        search_and_save(&mut app, "school");
        search_and_save(&mut app, "cat");
        press(&mut app, KeyCode::F(1));
        press(&mut app, KeyCode::Down);
        let selected_id = app.selected_saved().unwrap().id;
        ctrl(&mut app, KeyCode::Char('r'));
        assert_eq!(app.selected_saved().unwrap().id, selected_id);
    }

    #[test]
    fn shift_tab_toggles_between_views() {
        let library = sandbox();
        let mut app = App::new(&library);
        assert_eq!(app.view, View::Search);
        press(&mut app, KeyCode::BackTab);
        assert_eq!(app.view, View::Saved);
        press(&mut app, KeyCode::BackTab);
        assert_eq!(app.view, View::Search);
    }

    // --- Import and export -------------------------------------------------

    fn open_export_prompt(app: &mut App<'_>) {
        ctrl(app, KeyCode::Char('e'));
    }

    #[test]
    fn export_prompt_starts_on_a_default_destination() {
        let library = sandbox();
        let mut app = App::new(&library);
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
        assert!(app.status.contains("new words only"), "{:?}", app.status);
    }

    #[test]
    fn the_format_toggle_renames_the_default_file() {
        let library = sandbox();
        let mut app = App::new(&library);
        open_export_prompt(&mut app);
        ctrl(&mut app, KeyCode::Char('t'));
        let prompt = app.prompt.as_ref().expect("prompt open");
        assert!(prompt.path.ends_with("shouci-anki.txt"), "{}", prompt.path);
        assert!(app.status.contains("export to Anki"), "{:?}", app.status);
        ctrl(&mut app, KeyCode::Char('t'));
        assert!(app.status.contains("export to Pleco"), "{:?}", app.status);
    }

    #[test]
    fn only_new_and_policy_toggles_belong_to_their_direction() {
        let library = sandbox();
        let mut app = App::new(&library);
        open_export_prompt(&mut app);
        ctrl(&mut app, KeyCode::Char('n'));
        assert!(!app.prompt.as_ref().expect("prompt").only_new);
        assert!(app.status.contains("every word"), "{:?}", app.status);
        ctrl(&mut app, KeyCode::Char('p'));
        assert!(!app.status.contains("words you have"), "{:?}", app.status);

        press(&mut app, KeyCode::Esc);
        ctrl(&mut app, KeyCode::Char('o'));
        assert!(
            app.status.contains("words you have: skip"),
            "{:?}",
            app.status
        );
        ctrl(&mut app, KeyCode::Char('p'));
        assert!(
            app.status.contains("words you have: merge"),
            "{:?}",
            app.status
        );
    }

    #[test]
    fn enter_requests_the_panel_instead_of_writing() {
        let library = sandbox();
        let mut app = App::new(&library);
        search_and_save(&mut app, "school");
        open_export_prompt(&mut app);
        press(&mut app, KeyCode::Enter);
        let request = app.take_picker().expect("panel requested");
        assert_eq!(request.kind, PromptKind::Export);
        assert!(app.take_picker().is_none(), "request is taken once");
        // Nothing was written: the transfer only runs once a path comes back.
        assert!(!app.status.contains("wrote"), "{:?}", app.status);
    }

    #[test]
    fn import_prompt_offers_the_open_panel() {
        let library = sandbox();
        let mut app = App::new(&library);
        ctrl(&mut app, KeyCode::Char('o'));
        press(&mut app, KeyCode::Enter);
        let request = app.take_picker().expect("panel requested");
        assert_eq!(request.kind, PromptKind::Import);
    }

    #[test]
    fn the_confirm_step_ignores_typed_characters() {
        let library = sandbox();
        let mut app = App::new(&library);
        open_export_prompt(&mut app);
        let before = app.prompt.as_ref().expect("prompt").path.clone();
        type_text(&mut app, "xyz");
        press(&mut app, KeyCode::Backspace);
        assert_eq!(app.prompt.as_ref().expect("prompt").path, before);
        assert!(app.take_picker().is_none(), "no panel without Enter");
    }

    #[test]
    fn a_failed_panel_falls_back_to_typing() {
        let library = sandbox();
        let mut app = App::new(&library);
        open_export_prompt(&mut app);
        app.fall_back_to_typing("no panel available");
        let prompt = app.prompt.as_ref().expect("prompt still open");
        assert!(prompt.typing, "typing enabled after failure");
        assert!(app.status.contains("type the path"), "{:?}", app.status);
        type_text(&mut app, "/tmp");
        assert!(
            app.prompt.as_ref().expect("prompt").path.ends_with("/tmp"),
            "typed into the path"
        );
    }

    #[test]
    fn a_dismissed_panel_leaves_the_prompt_clean() {
        let library = sandbox();
        let mut app = App::new(&library);
        open_export_prompt(&mut app);
        press(&mut app, KeyCode::Enter);
        assert!(app.take_picker().is_some(), "panel requested");
        app.cancel_picker();
        assert!(app.prompt.is_none(), "prompt closed");
        assert_eq!(app.status, "export cancelled — no file written");
    }

    #[test]
    fn tilde_is_expanded_in_the_confirm_step() {
        let library = sandbox();
        let mut app = App::new(&library);
        open_export_prompt(&mut app);
        let home = expand_tilde(Path::new("~")).expect("home");
        app.prompt.as_mut().expect("prompt").typing = true;
        app.prompt.as_mut().expect("prompt").path = String::from("~/definitely-not-here.txt");
        app.refresh_prompt_status();
        assert!(
            app.status.contains(&home.to_string_lossy().into_owned()),
            "status should show the expanded path: {:?}",
            app.status
        );
    }

    #[test]
    fn a_picked_file_is_imported_and_new_words_exported() {
        let library = sandbox();
        let mut app = App::new(&library);
        let dir = scratch_dir("tui-transfer");
        let source = dir.join("from-pleco.txt");
        std::fs::write(&source, "你好\tni3 hao3\thello\n").unwrap();

        ctrl(&mut app, KeyCode::Char('o'));
        app.accept_picked_path(source.to_string_lossy().into_owned());
        assert!(app.error.is_none(), "{:?}", app.error);
        assert!(
            app.status.starts_with("added 1 word from Pleco file"),
            "{}",
            app.status
        );
        assert!(app.prompt.is_none(), "the prompt closes");

        search_and_save(&mut app, "school");
        let out = dir.join("to-pleco.txt");
        open_export_prompt(&mut app);
        app.accept_picked_path(out.to_string_lossy().into_owned());
        assert!(app.status.starts_with("wrote 1 word to "), "{}", app.status);
        assert!(
            app.status.contains("left out 1 word already in Pleco"),
            "{}",
            app.status
        );
        assert!(std::fs::read_to_string(&out).unwrap().contains("学校"));

        open_export_prompt(&mut app);
        app.accept_picked_path(out.to_string_lossy().into_owned());
        assert!(
            app.status.starts_with("nothing to export"),
            "{}",
            app.status
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn pane_hints_advertise_the_main_keys() {
        assert_eq!(
            crate::ui::pane_hint(View::Search, Scope::All),
            "Ctrl+O import · Ctrl+E export"
        );
        assert_eq!(
            crate::ui::pane_hint(View::Saved, Scope::All),
            "i import · e export"
        );
        assert_eq!(
            crate::ui::pane_hint(View::Saved, Scope::Trash),
            "r restore · d delete for good"
        );
    }

    // --- Drawing -----------------------------------------------------------

    fn screen(
        app: &mut App<'_>,
        width: u16,
        height: u16,
    ) -> (Vec<String>, ratatui::layout::Position) {
        use ratatui::backend::TestBackend;

        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| app.draw(frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let rows = (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol().to_owned())
                    .collect()
            })
            .collect();
        (rows, terminal.get_cursor_position().unwrap())
    }

    #[test]
    fn draw_renders_header_and_footer() {
        let library = sandbox();
        let mut app = App::new(&library);
        type_text(&mut app, "ni");
        let (rows, _) = screen(&mut app, 80, 24);
        assert!(rows[2].contains("shouci"), "header shows brand");
        assert!(rows[2].contains("ni"), "header shows query");
        assert!(rows[22].contains("Enter"), "footer shows save hint");
    }

    #[test]
    fn cursor_follows_the_query_for_input_methods() {
        let library = sandbox();
        let mut app = App::new(&library);
        type_text(&mut app, "翻译");
        assert_eq!(app.mode_label(), "Hanzi · auto");
        let (_, cursor) = screen(&mut app, 80, 24);
        // Header box at (1, 1); "shouci [Hanzi · auto]  翻译" is 27 cells
        // wide (Chinese is double width), so the cursor sits at column 2 + 27.
        assert_eq!(cursor, ratatui::layout::Position::new(29, 2));
    }

    #[test]
    fn draw_narrow_terminal_shows_resize_notice() {
        let library = sandbox();
        let mut app = App::new(&library);
        let (rows, _) = screen(&mut app, 40, 10);
        assert!(
            rows.concat().contains("too small"),
            "narrow viewport shows notice"
        );
        // Input still works while the notice is up.
        press(&mut app, KeyCode::Char('a'));
        assert_eq!(app.query, "a");
    }
}
