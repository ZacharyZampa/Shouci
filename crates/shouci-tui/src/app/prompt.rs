//! The import and export prompt: a confirm step that hands over to the
//! native file panel, typing a path as the fallback, then the transfer.

use std::path::Path;

use crossterm::event::KeyCode;
use shouci_core::text::{counted, import_outcome};
use shouci_core::{
    ConnectorView, Error, ExportRequest, ExportScope, ImportPolicy, Result, Severity, expand_tilde,
};

use super::App;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PromptKind {
    Import,
    Export,
}

#[derive(Debug, Clone)]
pub(super) struct TransferPrompt {
    pub(super) kind: PromptKind,
    /// Export: the format, by connector id. Imports detect theirs.
    pub(super) format: String,
    /// Import: what to do with words already saved.
    pub(super) policy: ImportPolicy,
    /// Export: only the words that format doesn't have yet.
    pub(super) only_new: bool,
    pub(super) path: String,
    /// False while the prompt is a confirm step that hands off to the native
    /// panel. True only as a fallback, when no panel can open and the path has
    /// to be typed after all.
    pub(super) typing: bool,
}

/// A request for the native file panel, answered by the event loop because only
/// it owns the terminal that has to be handed over to the panel and back.
#[derive(Debug, Clone)]
pub(super) struct PickerRequest {
    pub(super) kind: PromptKind,
    pub(super) suggested: String,
}

/// Status after backing out of an import or export prompt.
pub(super) fn cancelled_message(kind: Option<PromptKind>) -> String {
    String::from(match kind {
        Some(PromptKind::Import) => "import cancelled — nothing changed",
        Some(PromptKind::Export) => "export cancelled — no file written",
        None => "cancelled",
    })
}

/// Where a transfer file goes unless another place is chosen: Downloads, as
/// in the Mac app, or home when there is no Downloads folder.
pub(super) fn default_transfer_path(connector: &str) -> String {
    let home = expand_tilde(Path::new("~")).unwrap_or_default();
    let downloads = home.join("Downloads");
    let dir = if downloads.is_dir() { downloads } else { home };
    dir.join(format!("shouci-{connector}.txt"))
        .to_string_lossy()
        .into_owned()
}

pub(super) fn policy_label(policy: ImportPolicy) -> &'static str {
    match policy {
        ImportPolicy::Skip => "skip",
        ImportPolicy::Merge => "merge",
        ImportPolicy::Overwrite => "overwrite",
    }
}

pub(super) fn next_policy(policy: ImportPolicy) -> ImportPolicy {
    match policy {
        ImportPolicy::Skip => ImportPolicy::Merge,
        ImportPolicy::Merge => ImportPolicy::Overwrite,
        ImportPolicy::Overwrite => ImportPolicy::Skip,
    }
}

impl App<'_> {
    /// The formats words can be exported to. Imports detect theirs.
    pub(super) fn export_formats(&self) -> Vec<ConnectorView> {
        self.shouci
            .connectors()
            .into_iter()
            .filter(|connector| connector.can_export)
            .collect()
    }

    pub(super) fn begin_prompt(&mut self, kind: PromptKind) {
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
    pub(super) fn take_picker(&mut self) -> Option<PickerRequest> {
        self.picker.take()
    }

    /// Applies a path chosen in the native panel and runs the transfer.
    pub(super) fn accept_picked_path(&mut self, path: String) {
        if let Some(prompt) = self.prompt.as_mut() {
            prompt.path = path;
        }
        let op = self.run_prompt();
        self.capture(op);
    }

    /// The panel was dismissed, so leave the prompt exactly as it was.
    pub(super) fn cancel_picker(&mut self) {
        let kind = self.prompt.as_ref().map(|prompt| prompt.kind);
        self.prompt = None;
        self.status = cancelled_message(kind);
    }

    /// No panel could be opened, so fall back to typing the path.
    pub(super) fn fall_back_to_typing(&mut self, reason: &str) {
        if let Some(prompt) = self.prompt.as_mut() {
            prompt.typing = true;
        }
        self.error = None;
        self.refresh_prompt_status();
        self.status = format!("no file picker ({reason}) — type the path, Enter runs, Esc cancels");
    }

    pub(super) fn on_prompt_control(&mut self, code: KeyCode) {
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

    pub(super) fn on_prompt_key(&mut self, code: KeyCode) -> Result<()> {
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

    pub(super) fn run_prompt(&mut self) -> Result<()> {
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
    pub(super) fn import(&self, path: &Path, policy: ImportPolicy) -> Result<String> {
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
        Ok(format!(
            "{} from {} file {}",
            import_outcome(&summary),
            self.shouci.connector_name(&plan.connector_id),
            path.display()
        ))
    }

    /// Exports to `format`, and says what was written and left out.
    pub(super) fn export(&self, path: &Path, format: &str, only_new: bool) -> Result<String> {
        let request = ExportRequest {
            scope: if only_new {
                ExportScope::New
            } else {
                ExportScope::All
            },
            ..ExportRequest::default()
        };
        let plan = self.shouci.preview_export(path, format, &request)?;
        let name = self.shouci.connector_name(format);
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

    pub(super) fn refresh_prompt_status(&mut self) {
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
                self.shouci.connector_name(&prompt.format),
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
}
