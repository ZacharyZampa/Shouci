//! The saved lists (`F1`): reloading, and changing or deleting the selection.

use crossterm::event::KeyCode;
use shouci_core::text::counted;
use shouci_core::{BulkAction, Result, Verification};

use super::App;
use super::prompt::PromptKind;
use crate::state::{Scope, find_saved_index, move_selection};

impl App<'_> {
    pub(super) fn reload_saved(&mut self) -> Result<()> {
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
    pub(super) fn change_selected(
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
    pub(super) fn trash_selected(&mut self) -> Result<()> {
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

    pub(super) fn purge(&mut self, item_id: i64, head: &str) -> Result<()> {
        self.shouci.bulk(&[item_id], &BulkAction::Purge)?;
        self.reload_saved()?;
        self.status = format!("deleted {head} for good");
        Ok(())
    }

    /// `n`: needs review, or checked again.
    pub(super) fn toggle_review(&mut self) -> Result<()> {
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

    pub(super) fn on_saved_key(&mut self, code: KeyCode) {
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
}
