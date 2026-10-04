//! The core's values as lines for people. `--json` skips all of this and
//! prints the values themselves.

use std::fmt::Write;

pub use shouci_core::text::counted;
use shouci_core::text::{import_outcome, with_review};
use shouci_core::{
    CandidateView, ConnectorView, DictionaryEntryView, DictionaryView, ExportPlan, GroupView,
    ImportPlan, ItemView, Lifecycle, MatchBasis, Severity, SourceKind, TransferSummary,
    Verification,
};

/// Definitions in lists are cut to this many characters.
const DEFINITION_CHARS: usize = 60;

/// `学校 / 學校 [xué xiào]`: the traditional form only when it differs, the
/// reading only when there is one.
pub fn headword(simplified: &str, traditional: &str, reading: &str) -> String {
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

pub fn item_headword(item: &ItemView) -> String {
    headword(&item.simplified, &item.traditional, &item.pinyin_display)
}

pub fn clip(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut clipped: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    clipped.push('…');
    clipped
}

/// A line of `shouci list`: id, word, definition, and anything unusual.
pub fn item_line(item: &ItemView) -> String {
    format!("{:>5}  {}", item.id, word_line(item))
}

/// A saved word on one line: word, definition, and anything unusual.
pub fn word_line(item: &ItemView) -> String {
    let mut line = item_headword(item);
    if !item.definition_display.is_empty() {
        line.push_str("  ");
        line.push_str(&clip(&item.definition_display, DEFINITION_CHARS));
    }
    let marks = item_marks(item);
    if !marks.is_empty() {
        line.push_str("  · ");
        line.push_str(&marks.join(" · "));
    }
    line
}

fn item_marks(item: &ItemView) -> Vec<&'static str> {
    let mut marks = Vec::new();
    if item.verification == Verification::NeedsReview {
        marks.push("needs review");
    }
    match item.lifecycle {
        Lifecycle::Active => {}
        Lifecycle::Archived => marks.push("archived"),
        Lifecycle::Trashed => marks.push("in the trash"),
    }
    marks
}

/// Dictionary results numbered from 1, the numbers `shouci add --pick`
/// takes.
pub fn numbered(candidates: &[CandidateView]) -> Vec<String> {
    candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| format!("{:>3}  {}", index + 1, candidate_line(candidate)))
        .collect()
}

/// A dictionary result on one line.
pub fn candidate_line(candidate: &CandidateView) -> String {
    let mut line = format!(
        "{}  {}",
        headword(
            &candidate.simplified,
            &candidate.traditional,
            &candidate.pinyin_display
        ),
        candidate.definition_display
    );
    let mut marks = Vec::new();
    match &candidate.saved {
        Some(saved) if saved.lifecycle == Lifecycle::Trashed => marks.push("in the trash"),
        Some(_) => marks.push("saved"),
        None => {}
    }
    if candidate.basis == MatchBasis::ContainedWord {
        marks.push("inside what you typed");
    } else if candidate.inferred {
        marks.push("partial match");
    }
    if !marks.is_empty() {
        line.push_str("  · ");
        line.push_str(&marks.join(" · "));
    }
    line
}

/// `shouci show`: everything about one word.
pub fn item_detail(
    item: &ItemView,
    dictionaries: &[DictionaryView],
    connectors: &[ConnectorView],
) -> String {
    let dictionary_name = |id: &str| {
        dictionaries
            .iter()
            .find(|dictionary| dictionary.id == id)
            .map_or_else(|| id.to_owned(), |dictionary| dictionary.name.clone())
    };
    let connector_name = |id: &str| {
        connectors
            .iter()
            .find(|connector| connector.id == id)
            .map_or_else(|| id.to_owned(), |connector| connector.name.clone())
    };
    let mut lines = vec![item_headword(item)];
    if !item.definition_display.is_empty() {
        lines.push(item.definition_display.clone());
    }
    let marks = item_marks(item);
    if !marks.is_empty() {
        lines.push(marks.join(" · "));
    }
    if !item.notes.is_empty() {
        lines.push(format!("notes: {}", item.notes));
    }
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
            .map(|id| connector_name(id))
            .collect();
        lines.push(format!("in {}", names.join(", ")));
    }
    let source = match (item.source.kind, item.source.id.as_deref()) {
        (SourceKind::Dictionary, Some(id)) => format!(" from {}", dictionary_name(id)),
        (SourceKind::Import, Some(id)) => format!(" from a {} file", connector_name(id)),
        (SourceKind::Manual, _) => " by hand".to_owned(),
        _ => String::new(),
    };
    let mut dates = format!("saved {}{source}", day(&item.created_at));
    if item.modified_at != item.created_at {
        let _ = write!(dates, " · changed {}", day(&item.modified_at));
    }
    let _ = write!(dates, " · id {}", item.id);
    lines.push(dates);
    if let Some(origin) = &item.source.import_origin {
        lines.push(format!("imported from {origin}"));
    }
    lines.join("\n")
}

/// A dictionary's entries for a saved word, under its name.
pub fn lookup_section(name: &str, entries: &[DictionaryEntryView]) -> String {
    let mut lines = vec![format!("{name}:")];
    if entries.is_empty() {
        lines.push("  no entry".to_owned());
    }
    for entry in entries {
        lines.push(format!(
            "  {}  {}",
            headword(&entry.simplified, &entry.traditional, &entry.pinyin_display),
            entry.definition_display
        ));
    }
    lines.join("\n")
}

/// The date part of a stored timestamp (`2026-09-30T12:00:00.000Z`).
fn day(timestamp: &str) -> &str {
    timestamp.get(..10).unwrap_or(timestamp)
}

pub fn group_lines(groups: &[GroupView], none: &str) -> String {
    if groups.is_empty() {
        return none.to_owned();
    }
    groups
        .iter()
        .map(|group| format!("{}  ({})", group.name, group.count))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn dictionary_lines(dictionaries: &[DictionaryView]) -> String {
    if dictionaries.is_empty() {
        return "no dictionary is installed; `shouci dictionaries update` downloads one".to_owned();
    }
    dictionaries
        .iter()
        .map(|dictionary| {
            let place = dictionary.priority.map_or_else(
                || "  -".to_owned(),
                |priority| format!("{:>3}", priority + 1),
            );
            let unused = if dictionary.enabled {
                ""
            } else {
                " · not searched"
            };
            format!(
                "{place}  {} ({}) · version {} · {} · {}{unused}",
                dictionary.name,
                dictionary.id,
                dictionary.version,
                counted(dictionary.entries, "entry", "entries"),
                dictionary.license
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// What an import would do (`--dry-run`) or refused to do.
pub fn import_preview(plan: &ImportPlan, format: &str) -> String {
    let counts = plan.counts();
    let mut parts = vec![format!(
        "would add {}",
        with_review(counts.inserts, counts.unresolved)
    )];
    if counts.updates > 0 {
        parts.push(format!("update {}", counts.updates));
    }
    if counts.skips > 0 {
        parts.push(format!("skip {}", counts.skips));
    }
    if counts.drops > 0 {
        parts.push(format!("drop {}", counts.drops));
    }
    let mut lines = vec![format!("{format} file {}", plan.path), parts.join(", ")];
    if counts.conflicts > 0 {
        lines.push(format!(
            "{} where your words differ from the file; yours are kept",
            counted(counts.conflicts, "field", "fields")
        ));
    }
    lines.extend(issue_lines(plan));
    lines.extend(plan.notes.iter().cloned());
    lines.join("\n")
}

/// Error and warning lines from a file, by line number.
pub fn issue_lines(plan: &ImportPlan) -> Vec<String> {
    plan.issues
        .iter()
        .filter(|issue| issue.severity != Severity::Info)
        .map(|issue| {
            let severity = match issue.severity {
                Severity::Error => "error",
                Severity::Warning | Severity::Info => "warning",
            };
            format!("line {}: {severity}: {}", issue.line, issue.message)
        })
        .collect()
}

/// What an import did.
pub fn import_summary(summary: &TransferSummary, format: &str) -> String {
    let mut lines = vec![format!(
        "{} from {format} file {}",
        import_outcome(summary),
        summary.path
    )];
    lines.extend(summary.notes.iter().cloned());
    lines.join("\n")
}

/// What an export would write, and what it leaves out.
pub fn export_preview(plan: &ExportPlan, format: &str) -> String {
    let mut lines = vec![format!(
        "would write {} to {format} file {}",
        counted(count(plan.words.len()), "word", "words"),
        plan.path
    )];
    if !plan.words.is_empty() {
        lines.push(clip(&plan.words.join(" "), 200));
    }
    lines.extend(left_out(plan, format));
    if plan.replaces_existing {
        lines.push("the file exists and would be replaced".to_owned());
    }
    lines.extend(plan.notes.iter().cloned());
    lines.join("\n")
}

/// Why matching words are not in an export, with the option that adds them.
pub fn left_out(plan: &ExportPlan, format: &str) -> Vec<String> {
    let mut lines = Vec::new();
    if plan.left_out_needs_review > 0 {
        let (verb, them) = if plan.left_out_needs_review == 1 {
            ("needs", "it")
        } else {
            ("need", "them")
        };
        lines.push(format!(
            "left out {} that {verb} review (--include-needs-review adds {them})",
            counted(plan.left_out_needs_review, "word", "words"),
        ));
    }
    if plan.left_out_already_there > 0 {
        lines.push(format!(
            "left out {} already in {format} (--all adds {})",
            counted(plan.left_out_already_there, "word", "words"),
            if plan.left_out_already_there == 1 {
                "it"
            } else {
                "them"
            }
        ));
    }
    lines
}

pub fn export_summary(summary: &TransferSummary, format: &str) -> String {
    let mut lines = vec![format!(
        "wrote {} to {format} file {}",
        counted(summary.written, "word", "words"),
        summary.path
    )];
    lines.extend(summary.notes.iter().cloned());
    lines.join("\n")
}

fn count(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::{clip, day, headword};

    #[test]
    fn headwords_drop_what_adds_nothing() {
        assert_eq!(
            headword("学校", "學校", "xué xiào"),
            "学校 / 學校 [xué xiào]"
        );
        assert_eq!(headword("你好", "你好", "nǐ hǎo"), "你好 [nǐ hǎo]");
        assert_eq!(headword("蚌埠住了", "", ""), "蚌埠住了");
    }

    #[test]
    fn clipping_and_days() {
        assert_eq!(clip("abcdef", 4), "abc…");
        assert_eq!(clip("abc", 4), "abc");
        assert_eq!(day("2026-09-30T12:00:00.000Z"), "2026-09-30");
        assert_eq!(day("short"), "short");
    }
}
