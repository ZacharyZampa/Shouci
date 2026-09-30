//! Move saved vocabulary to and from Pleco and Anki files.
//!
//! CLI and TUI call [`import_file`] and [`export_file`]. Capture and search do not.

mod card;
mod resolve;

use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use vocab_anki::{AnkiTextV1, IssueSeverity as AnkiSeverity, NoteRow};
use vocab_core::{Result, VocabError, VocabItem};
use vocab_db::{
    InboundInsert, InboundOp, SessionOutcome, TransferDirection, TransferHeader, apply_inbound,
    commit_outbound, find_saved_reading, import_sources, item_tags, record_rejected,
    select_for_transfer,
};
use vocab_dictionary::DictionaryProvider;
use vocab_pleco::{
    CodecRegistry, ExportRow, IssueSeverity as PlecoSeverity, PlecoCodec, Utf8TextV1,
};

use card::{ExchangeCard, Field};
use resolve::{DictHit, Resolution, ResolvedCard, definition_matches, reading_key, resolve};

const PLECO_KEY: &str = "pleco-utf8-text/v1";
const ANKI_KEY: &str = "anki-text/v1";

/// Expands a leading `~` against `HOME`.
///
/// Split out from [`expand_path`] so tests can pass a home directory instead of
/// mutating the process environment.
///
/// # Errors
///
/// Returns an error when the path starts with `~` and no home directory is
/// available, instead of silently using a directory named `~`.
pub fn expand_path_in(path: &Path, home: Option<&Path>) -> Result<PathBuf> {
    let mut components = path.components();
    let starts_with_tilde = components.next() == Some(Component::Normal(OsStr::new("~")));
    if !starts_with_tilde {
        return Ok(path.to_path_buf());
    }
    let home = home.ok_or_else(|| VocabError::new("cannot expand ~: HOME is not set"))?;
    let rest: PathBuf = components.collect();
    if rest.as_os_str().is_empty() {
        Ok(home.to_path_buf())
    } else {
        Ok(home.join(rest))
    }
}

/// Expands a leading `~` to the user's home directory.
///
/// This is deliberately not a shell: only `~` and `~/...` are rewritten.
/// `$VAR`, quoted strings, backslash escapes, and `~user` forms are left alone,
/// so a path that is meant literally still reaches the filesystem unchanged.
///
/// # Errors
///
/// Returns an error when the path starts with `~` and `HOME` is not set.
pub fn expand_path(path: &Path) -> Result<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    expand_path_in(path, home.as_deref())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportOptions {
    pub codec: String,
    pub force: bool,
    pub dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct ExportOptions {
    pub codec: String,
    pub only_new: bool,
    pub allow_unresolved: bool,
    pub include_archived: bool,
    pub tags: Vec<String>,
    pub category: Option<String>,
    pub deck: String,
    pub dry_run: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            codec: PLECO_KEY.to_owned(),
            only_new: false,
            allow_unresolved: false,
            include_archived: false,
            tags: Vec::new(),
            category: None,
            deck: "Shouci".to_owned(),
            dry_run: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferReport {
    pub summary: String,
    pub details: Vec<String>,
    pub refused: bool,
}

/// Imports a Pleco or Anki file into `user.db`.
///
/// An existing reading is left unchanged and marked as already at that target.
/// A missing dictionary does not fail the import.
///
/// # Errors
///
/// Returns an error if the file cannot be read, the codec is unknown, or the
/// database write fails.
pub fn import_file(
    conn: &mut rusqlite::Connection,
    dict: Option<&dyn DictionaryProvider>,
    path: &Path,
    options: &ImportOptions,
) -> Result<TransferReport> {
    let source = expand_path(path)?;
    let bytes = std::fs::read(&source)
        .map_err(|err| VocabError::new(format!("cannot read {}: {err}", source.display())))?;
    let parsed = parse_cards(&options.codec, &bytes)?;
    let (target, _) = target_of(&options.codec)?;
    let path_text = source.to_string_lossy().into_owned();
    let header = TransferHeader {
        direction: TransferDirection::In,
        target: target.to_owned(),
        codec_key: options.codec.clone(),
        profile: target.to_owned(),
        source_path: path_text.clone(),
        content_sha256: Some(vocab_dictionary::sha256_hex(&bytes)),
        records_seen: parsed.cards.len(),
        issues_errors: parsed.errors.len(),
        issues_warnings: parsed.warnings.len(),
    };
    let mut details = parsed.details;
    if dict.is_none() {
        details.push("dictionary unavailable; definitions not filled".to_owned());
    }
    if !parsed.errors.is_empty() && !options.force {
        if !options.dry_run {
            record_rejected(conn, &header, &parsed.errors)?;
        }
        return Ok(TransferReport {
            summary: format!(
                "refused: source has {} error-severity lines; fix them or pass --force",
                parsed.errors.len()
            ),
            details,
            refused: true,
        });
    }
    let ops = plan_cards(conn, dict, &parsed.cards, target, &path_text)?;
    if options.dry_run {
        return Ok(report_import(&header, &ops, &details, true));
    }
    let outcome = apply_with_retry(conn, dict, &header, &parsed.cards, target, &path_text)?;
    match outcome {
        SessionOutcome::Applied(counts) => {
            let mut report = report_import(&header, &ops, &details, false);
            report.summary = format!(
                "imported {} ({} duplicates skipped, {} seen, {} errors, {} warnings)",
                counts.inserted,
                counts.skipped,
                header.records_seen,
                header.issues_errors,
                header.issues_warnings
            );
            let _ = counts;
            Ok(report)
        }
        SessionOutcome::Stale => Err(VocabError::new(
            "import plan went stale; nothing was written",
        )),
    }
}

/// Writes a Pleco or Anki file from saved vocabulary.
///
/// Pleco leaves the definition blank when it matches the dictionary. Anki always
/// writes the definition. `only_new` is per target.
///
/// # Errors
///
/// Returns an error if the codec is unknown, the path is an import source, or
/// the write fails.
pub fn export_file(
    conn: &mut rusqlite::Connection,
    dict: Option<&dyn DictionaryProvider>,
    path: &Path,
    options: &ExportOptions,
) -> Result<TransferReport> {
    let target_path = expand_path(path)?;
    let path_text = target_path.to_string_lossy().into_owned();
    let sources = import_sources(conn)?;
    if sources.iter().any(|source| source == &path_text) {
        return Err(VocabError::new(format!(
            "refusing to overwrite import source {}",
            target_path.display()
        )));
    }
    let (target, kind) = target_of(&options.codec)?;
    let mut required = options.tags.clone();
    if let Some(category) = &options.category {
        required.push(category.clone());
    }
    let items = select_for_transfer(
        conn,
        target,
        &required,
        options.allow_unresolved,
        options.include_archived,
        options.only_new,
    )?;
    if items.is_empty() {
        return Ok(TransferReport {
            summary: "nothing to export".to_owned(),
            details: Vec::new(),
            refused: false,
        });
    }
    let projected = project(conn, dict, &items, kind, options)?;
    if options.dry_run {
        return Ok(TransferReport {
            summary: format!(
                "dry run: would export {} items to {}",
                items.len(),
                target_path.display()
            ),
            details: projected.details,
            refused: false,
        });
    }
    atomic_write(&target_path, &projected.bytes)?;
    let header = TransferHeader {
        direction: TransferDirection::Out,
        target: target.to_owned(),
        codec_key: options.codec.clone(),
        profile: target.to_owned(),
        source_path: path_text,
        content_sha256: None,
        records_seen: items.len(),
        issues_errors: 0,
        issues_warnings: projected.details.len(),
    };
    commit_outbound(conn, &header, &projected.item_ids)?;
    Ok(TransferReport {
        summary: format!(
            "exported {} items to {}",
            items.len(),
            target_path.display()
        ),
        details: projected.details,
        refused: false,
    })
}

struct ParsedFile {
    cards: Vec<ExchangeCard>,
    errors: Vec<(usize, String)>,
    warnings: Vec<String>,
    details: Vec<String>,
}

struct ProjectedFile {
    bytes: Vec<u8>,
    item_ids: Vec<i64>,
    details: Vec<String>,
}

#[derive(Clone, Copy)]
enum FormatKind {
    Pleco,
    Anki,
}

fn parse_cards(codec: &str, bytes: &[u8]) -> Result<ParsedFile> {
    match codec {
        PLECO_KEY => parse_pleco(bytes),
        ANKI_KEY => parse_anki(bytes),
        other => Err(VocabError::new(format!("unknown codec '{other}'"))),
    }
}

fn parse_pleco(bytes: &[u8]) -> Result<ParsedFile> {
    let parsed = CodecRegistry::builtin()
        .get_by_key(PLECO_KEY)
        .ok_or_else(|| VocabError::new(format!("unknown codec '{PLECO_KEY}'")))?
        .parse(bytes)?;
    let mut cards = Vec::new();
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut details = Vec::new();
    for issue in &parsed.issues {
        let line = format!(
            "line {} [{}]: {}",
            issue.line,
            severity_pleco(issue.severity),
            issue.message
        );
        details.push(line);
        match issue.severity {
            PlecoSeverity::Error => errors.push((issue.line, issue.message.clone())),
            PlecoSeverity::Warning => warnings.push(issue.message.clone()),
            PlecoSeverity::Info => {}
        }
    }
    for record in parsed.records {
        if record.simplified.trim().is_empty() {
            errors.push((record.line, "record has no headword".to_owned()));
            details.push(format!(
                "line {} [error]: record has no headword",
                record.line
            ));
            continue;
        }
        let definition = if record.definition.is_empty()
            && parsed
                .issues
                .iter()
                .any(|issue| issue.line == record.line && issue.severity == PlecoSeverity::Warning)
        {
            Field::Omitted
        } else {
            Field::Present(record.definition.clone())
        };
        let tags = record
            .category
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(|name| vec![name.to_owned()])
            .unwrap_or_default();
        cards.push(ExchangeCard {
            line: record.line,
            raw: format!(
                "{}\t{}\t{}",
                record.simplified, record.pinyin, record.definition
            ),
            headword: record.simplified,
            traditional: None,
            pinyin: Field::Present(record.pinyin),
            definition,
            notes: Field::Omitted,
            tags,
        });
    }
    Ok(ParsedFile {
        cards,
        errors,
        warnings,
        details,
    })
}

fn parse_anki(bytes: &[u8]) -> Result<ParsedFile> {
    let parsed = AnkiTextV1.parse(bytes)?;
    let mut details = Vec::new();
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    for issue in &parsed.issues {
        details.push(format!(
            "line {} [{}]: {}",
            issue.line,
            severity_anki(issue.severity),
            issue.message
        ));
        match issue.severity {
            AnkiSeverity::Error => errors.push((issue.line, issue.message.clone())),
            AnkiSeverity::Warning => warnings.push(issue.message.clone()),
            AnkiSeverity::Info => {}
        }
    }
    let cards = parsed
        .notes
        .into_iter()
        .map(|note| ExchangeCard {
            line: note.line,
            raw: note.raw.clone(),
            headword: note.headword,
            traditional: Some(note.traditional).filter(|value| !value.is_empty()),
            pinyin: Field::Present(note.pinyin),
            definition: Field::Present(note.definition),
            notes: Field::Present(note.notes),
            tags: note.tags,
        })
        .collect();
    Ok(ParsedFile {
        cards,
        errors,
        warnings,
        details,
    })
}

fn plan_cards(
    conn: &rusqlite::Connection,
    dict: Option<&dyn DictionaryProvider>,
    cards: &[ExchangeCard],
    app: &str,
    path: &str,
) -> Result<Vec<InboundOp>> {
    let mut ops = Vec::new();
    for card in cards {
        ops.push(plan_one(conn, dict, card, app, path)?);
    }
    Ok(ops)
}

fn plan_one(
    conn: &rusqlite::Connection,
    dict: Option<&dyn DictionaryProvider>,
    card: &ExchangeCard,
    app: &str,
    path: &str,
) -> Result<InboundOp> {
    let hits = lookup(dict, &card.headword)?;
    match resolve(card, &hits, app) {
        Resolution::Drop { line, raw, message } => Ok(InboundOp::Drop {
            line: Some(line),
            raw_line: Some(raw),
            detail: message,
        }),
        Resolution::Card(resolved) => {
            let existing = find_saved_reading(
                conn,
                &resolved.simplified,
                &resolved.traditional,
                &resolved.pinyin,
            )?;
            Ok(match existing {
                Some(item) => {
                    let detail = conflict_detail(&item, &resolved);
                    InboundOp::Skip {
                        item_id: item.item_id,
                        line: Some(resolved.line),
                        raw_line: Some(resolved.raw),
                        detail,
                    }
                }
                None => InboundOp::Insert(Box::new(to_insert(resolved, path))),
            })
        }
    }
}

fn to_insert(card: ResolvedCard, path: &str) -> InboundInsert {
    InboundInsert {
        simplified: card.simplified,
        traditional: card.traditional,
        pinyin: card.pinyin,
        definition: card.definition,
        notes: card.notes,
        status: card.status,
        source_entry_id: card.source_entry_id,
        source_id: card.source_id,
        source_version: card.source_version,
        import_origin: path.to_owned(),
        tags: card.tags,
        line: Some(card.line),
        raw_line: Some(card.raw),
    }
}

fn conflict_detail(existing: &VocabItem, incoming: &ResolvedCard) -> String {
    if existing.definition.trim() != incoming.definition.trim() && !incoming.definition.is_empty() {
        "definition conflict; stored row kept".to_owned()
    } else {
        String::new()
    }
}

fn lookup(dict: Option<&dyn DictionaryProvider>, headword: &str) -> Result<Vec<DictHit>> {
    let Some(dict) = dict else {
        return Ok(Vec::new());
    };
    Ok(dict
        .entries_by_headword(headword)?
        .iter()
        .map(DictHit::from_entry)
        .collect())
}

fn apply_with_retry(
    conn: &mut rusqlite::Connection,
    dict: Option<&dyn DictionaryProvider>,
    header: &TransferHeader,
    cards: &[ExchangeCard],
    app: &str,
    path: &str,
) -> Result<SessionOutcome> {
    let ops = plan_cards(conn, dict, cards, app, path)?;
    match apply_inbound(conn, header, &ops)? {
        SessionOutcome::Stale => {
            let ops = plan_cards(conn, dict, cards, app, path)?;
            apply_inbound(conn, header, &ops)
        }
        applied @ SessionOutcome::Applied(_) => Ok(applied),
    }
}

fn report_import(
    header: &TransferHeader,
    ops: &[InboundOp],
    details: &[String],
    dry_run: bool,
) -> TransferReport {
    let inserted = ops
        .iter()
        .filter(|op| matches!(op, InboundOp::Insert(_)))
        .count();
    let skipped = ops
        .iter()
        .filter(|op| matches!(op, InboundOp::Skip { .. }))
        .count();
    let prefix = if dry_run {
        "dry run: would import"
    } else {
        "imported"
    };
    let mut details = details.to_vec();
    let conflicts = ops
        .iter()
        .filter(|op| matches!(op, InboundOp::Skip { detail, .. } if !detail.is_empty()))
        .count();
    if conflicts > 0 {
        details.push(format!(
            "{conflicts} definition conflicts, stored rows kept"
        ));
    }
    TransferReport {
        summary: format!(
            "{prefix} {inserted} ({skipped} duplicates skipped, {} seen, {} errors, {} warnings)",
            header.records_seen, header.issues_errors, header.issues_warnings
        ),
        details,
        refused: false,
    }
}

fn project(
    conn: &rusqlite::Connection,
    dict: Option<&dyn DictionaryProvider>,
    items: &[VocabItem],
    kind: FormatKind,
    options: &ExportOptions,
) -> Result<ProjectedFile> {
    match kind {
        FormatKind::Pleco => project_pleco(conn, dict, items, options),
        FormatKind::Anki => project_anki(conn, items, options),
    }
}

fn project_pleco(
    conn: &rusqlite::Connection,
    dict: Option<&dyn DictionaryProvider>,
    items: &[VocabItem],
    options: &ExportOptions,
) -> Result<ProjectedFile> {
    let mut rows = Vec::new();
    let mut details = Vec::new();
    let mut omitted = 0usize;
    let mut notes = 0usize;
    let mut traditional = 0usize;
    let mut unmapped: Vec<String> = Vec::new();
    if dict.is_none() {
        details.push("dictionary unavailable; definitions written".to_owned());
    }
    for item in items {
        let tags = item_tags(conn, item.item_id)?;
        let (category, left) = pleco_category(&tags, options.category.as_deref());
        if !left.is_empty() {
            unmapped.push(format!("{}: {}", item.simplified, left.join(", ")));
        }
        if item
            .notes
            .as_deref()
            .is_some_and(|note| !note.trim().is_empty())
        {
            notes += 1;
        }
        if item.traditional != item.simplified {
            traditional += 1;
        }
        let definition = pleco_definition(dict, item)?;
        if definition.is_empty() {
            omitted += 1;
        }
        rows.push((
            category.clone(),
            item.item_id,
            ExportRow {
                simplified: item.simplified.clone(),
                pinyin: item.pinyin.clone(),
                definition,
                category,
            },
        ));
    }
    rows.sort_by(|left, right| match (&left.0, &right.0) {
        (None, None) => left.1.cmp(&right.1),
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (Some(_), None) => std::cmp::Ordering::Less,
        (Some(left_name), Some(right_name)) => left_name.cmp(right_name).then(left.1.cmp(&right.1)),
    });
    if omitted > 0 {
        details.push(format!("definitions omitted {omitted}"));
    }
    if notes > 0 {
        details.push(format!("notes not written {notes}"));
    }
    if traditional > 0 {
        details.push(format!("traditional not written {traditional}"));
    }
    if !unmapped.is_empty() {
        details.push(format!("tags not mapped {}", unmapped.join("; ")));
    }
    let export_rows: Vec<ExportRow> = rows.into_iter().map(|(_, _, row)| row).collect();
    let item_ids: Vec<i64> = items.iter().map(|item| item.item_id).collect();
    let bytes = Utf8TextV1.serialize(&export_rows)?;
    Ok(ProjectedFile {
        bytes,
        item_ids,
        details,
    })
}

fn pleco_definition(dict: Option<&dyn DictionaryProvider>, item: &VocabItem) -> Result<String> {
    let Some(dict) = dict else {
        return Ok(item.definition.clone());
    };
    let hits = lookup(Some(dict), &item.simplified)?;
    let key = reading_key(&item.pinyin);
    let matched: Vec<_> = hits
        .iter()
        .filter(|hit| reading_key(&hit.pinyin) == key)
        .filter(|hit| item.source_entry_id.is_none() || hit.entry_id == item.source_entry_id)
        .collect();
    if matched.len() == 1 && definition_matches(&item.definition, &matched[0].glosses) {
        return Ok(String::new());
    }
    let any: Vec<_> = hits
        .iter()
        .filter(|hit| reading_key(&hit.pinyin) == key)
        .collect();
    if item.source_entry_id.is_some()
        && any.len() == 1
        && definition_matches(&item.definition, &any[0].glosses)
    {
        return Ok(String::new());
    }
    Ok(item.definition.clone())
}

fn pleco_category(tags: &[String], category: Option<&str>) -> (Option<String>, Vec<String>) {
    if let Some(name) = category {
        return (Some(name.to_owned()), Vec::new());
    }
    match tags {
        [one] => (Some(one.clone()), Vec::new()),
        [] => (None, Vec::new()),
        many => (None, many.to_vec()),
    }
}

fn project_anki(
    conn: &rusqlite::Connection,
    items: &[VocabItem],
    options: &ExportOptions,
) -> Result<ProjectedFile> {
    let mut notes = Vec::new();
    let mut renamed = Vec::new();
    let mut item_ids = Vec::new();
    for item in items {
        let tags = item_tags(conn, item.item_id)?;
        let mut written = Vec::new();
        for tag in tags {
            if tag.contains(' ') {
                let next = tag.replace(' ', "_");
                renamed.push(format!("{tag} -> {next}"));
                written.push(next);
            } else {
                written.push(tag);
            }
        }
        written.sort();
        item_ids.push(item.item_id);
        notes.push(NoteRow {
            line: 0,
            headword: item.simplified.clone(),
            traditional: item.traditional.clone(),
            pinyin: item.pinyin.clone(),
            definition: item.definition.clone(),
            notes: item.notes.clone().unwrap_or_default(),
            tags: written,
            raw: String::new(),
        });
    }
    let mut details = Vec::new();
    if !renamed.is_empty() {
        details.push(format!("tags renamed for Anki: {}", renamed.join(", ")));
    }
    let bytes = AnkiTextV1.write(&notes, &options.deck)?;
    Ok(ProjectedFile {
        bytes,
        item_ids,
        details,
    })
}

fn target_of(codec: &str) -> Result<(&'static str, FormatKind)> {
    match codec {
        PLECO_KEY => Ok(("pleco", FormatKind::Pleco)),
        ANKI_KEY => Ok(("anki", FormatKind::Anki)),
        other => Err(VocabError::new(format!("unknown codec '{other}'"))),
    }
}

fn severity_pleco(severity: PlecoSeverity) -> &'static str {
    match severity {
        PlecoSeverity::Info => "info",
        PlecoSeverity::Warning => "warning",
        PlecoSeverity::Error => "error",
    }
}

fn severity_anki(severity: AnkiSeverity) -> &'static str {
    match severity {
        AnkiSeverity::Info => "info",
        AnkiSeverity::Warning => "warning",
        AnkiSeverity::Error => "error",
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|err| VocabError::new(format!("cannot create output dir: {err}")))?;
        }
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)
        .map_err(|err| VocabError::new(format!("cannot write {}: {err}", tmp.display())))?;
    std::fs::rename(&tmp, path)
        .map_err(|err| VocabError::new(format!("cannot replace {}: {err}", path.display())))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{Result, VocabError, expand_path_in};

    fn home() -> &'static Path {
        Path::new("/Users/tester")
    }

    fn expand(input: &str) -> Result<std::path::PathBuf> {
        expand_path_in(Path::new(input), Some(home()))
    }

    #[test]
    fn bare_tilde_becomes_home() {
        assert_eq!(expand("~").expect("tilde"), home());
    }

    #[test]
    fn tilde_slash_prefix_joins_remainder() {
        assert_eq!(
            expand("~/Desktop/shouci-pleco.txt").expect("tilde path"),
            Path::new("/Users/tester/Desktop/shouci-pleco.txt")
        );
    }

    #[test]
    fn nested_relative_parts_survive() {
        assert_eq!(
            expand("~/notes/2026/out.txt").expect("nested"),
            Path::new("/Users/tester/notes/2026/out.txt")
        );
    }

    #[test]
    fn tilde_user_form_is_left_alone() {
        let input = "~someone/out.txt";
        assert_eq!(expand(input).expect("literal"), Path::new(input));
    }

    #[test]
    fn absolute_and_relative_paths_are_untouched() {
        for input in ["/tmp/out.txt", "out.txt", "./out.txt", "../out.txt"] {
            assert_eq!(expand(input).expect("unchanged"), Path::new(input));
        }
    }

    #[test]
    fn dollar_vars_are_not_expanded() {
        let input = "$HOME/out.txt";
        assert_eq!(expand(input).expect("literal"), Path::new(input));
    }

    #[test]
    fn missing_home_is_an_error_not_a_literal_tilde_dir() {
        let err = expand_path_in(Path::new("~/out.txt"), None).expect_err("needs HOME");
        assert!(err.to_string().contains("HOME is not set"), "{err}");
    }

    #[test]
    fn missing_home_only_matters_for_tilde() {
        assert_eq!(
            expand_path_in(Path::new("out.txt"), None).expect("relative is fine"),
            Path::new("out.txt")
        );
    }

    #[test]
    fn empty_path_is_left_alone() {
        assert_eq!(expand("").expect("empty"), Path::new(""));
    }

    #[test]
    fn error_type_is_the_shared_vocab_error() {
        let err: VocabError = expand_path_in(Path::new("~"), None).expect_err("needs HOME");
        assert!(!err.to_string().is_empty());
    }
}
