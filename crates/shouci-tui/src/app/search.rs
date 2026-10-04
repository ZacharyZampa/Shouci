//! The search view (`F2`): live search, saving a result, and taking the
//! last save back.

use crossterm::event::KeyCode;
use ratatui::widgets::ListState;
use shouci_core::{BulkAction, ManualWord, Result, SaveOutcome, Verification};

use super::App;
use crate::state::{Row, View, find_row_index, kind_title, move_selection, next_kind, rows};
use crate::ui::plain_headword;

impl App<'_> {
    pub(super) fn on_search_key(&mut self, code: KeyCode) {
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
    pub(super) fn typed(&mut self) {
        self.error = None;
        self.last_save = None;
        let op = self.run_search();
        self.capture(op);
    }

    pub(super) fn run_search(&mut self) -> Result<()> {
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

    pub(super) fn save_selected(&mut self) -> Result<()> {
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

    /// Takes back the last save from search: a new word is deleted, a word
    /// brought back from the trash goes back there.
    pub(super) fn undo(&mut self) -> Result<()> {
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
}
