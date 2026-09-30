//! Anki plain-text note exchange (`anki-text/v1`).
//!
//! File grammar only. No `.apkg`, no `AnkiConnect`, no note-type creation.

mod text;

pub use text::{AnkiIssue, AnkiTextV1, IssueSeverity, NoteRow, ParsedNotes};
