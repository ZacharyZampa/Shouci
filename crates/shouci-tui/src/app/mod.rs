//! The TUI's state and event loop. [`App`] holds everything on screen; its
//! methods are split by job: key routing here, the search view
//! (`search.rs`), the saved lists (`saved.rs`), the import and export prompt
//! (`prompt.rs`), and drawing (`render.rs`). Pure helpers live in
//! `crate::state` and `crate::ui`.

use std::io;
use std::path::PathBuf;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::widgets::ListState;
use shouci_core::{
    Config, DictionaryResults, DictionaryStatus, Error, ItemView, QueryKind, Result, SaveResult,
    Shouci,
};

use crate::state::{Row, Scope, View};
use crate::theme::Theme;
use crate::ui::DETAIL_SCROLL_STEP;

mod prompt;
mod render;
mod saved;
mod search;
#[cfg(test)]
mod tests;

use prompt::{PickerRequest, PromptKind, TransferPrompt};

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

    /// Scrolls the detail pane. Clamping to content happens in `draw`, where
    /// the pane width is known, so this only moves the offset.
    fn scroll_detail(&mut self, up: bool) {
        if up {
            self.detail_scroll = self.detail_scroll.saturating_sub(DETAIL_SCROLL_STEP);
        } else {
            self.detail_scroll = self.detail_scroll.saturating_add(DETAIL_SCROLL_STEP);
        }
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
    let shouci = if shouci.dictionaries()?.is_empty() {
        eprintln!(
            "shouci-tui: downloading the dictionary (first run only; needs an internet \
             connection)…"
        );
        let mut config = shouci.config().clone();
        config.fetch_dictionaries = true;
        drop(shouci);
        Shouci::open(config)?
    } else {
        shouci
    };
    let problem = shouci.load_dictionaries().err().map(|err| err.to_string());
    let notes = match shouci.dictionary_status() {
        DictionaryStatus::Ready { notes, .. } => notes,
        _ => Vec::new(),
    };
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
