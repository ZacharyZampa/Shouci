use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use vocab_capture::{
    CaptureOutcome, SearchMode, capture_to_path, clipboard_text, guess_mode, open_capture_db,
    open_service, resolve, resolve_auto,
};
use vocab_core::{ItemStatus, Result, VocabError};
use vocab_db::{
    ExportRun, ImportDecision, ImportPayload, ImportRecord, commit_export, import_sources,
    item_tags, list_items, persist_import, select_exportable,
};
use vocab_pleco::{CodecRegistry, ExportRow, IssueSeverity};

#[derive(Parser)]
#[command(name = "vocab")]
#[command(about = "Shouci: collect Chinese vocabulary alongside Pleco")]
struct Cli {
    /// Path to the built dictionary.db (defaults to data/dictionary/dictionary.db).
    #[arg(long, global = true)]
    dictionary: Option<PathBuf>,
    /// Path to user.db (defaults to the platform app-data directory).
    #[arg(long, global = true)]
    user_db: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Add a vocabulary item (quick capture).
    Add(AddArgs),
    /// Import a Pleco UTF-8 text export.
    Import(ImportArgs),
    /// Export vocabulary to a Pleco UTF-8 text file.
    Export(ExportArgs),
    /// List saved vocabulary items, newest first.
    List(ListArgs),
    /// Search the dictionary.
    Search(SearchArgs),
}

#[derive(Args)]
struct AddArgs {
    /// Take the headword from the clipboard instead of an argument.
    #[arg(short, long)]
    clipboard: bool,
    /// Force a query-mode guess override: `english` | `pinyin` | `chinese`.
    #[arg(long, value_enum)]
    mode: Option<SearchMode>,
    /// Headword to capture (Chinese, pinyin, or English).
    word: Option<String>,
}

#[derive(Args)]
struct ImportArgs {
    /// Pleco UTF-8 text export file.
    #[arg(value_name = "FILE")]
    path: PathBuf,
    /// Codec format/variant, e.g. pleco-utf8-text/v1.
    #[arg(long, default_value = "pleco-utf8-text/v1")]
    codec: String,
    /// Import even when the source has error-severity lines.
    #[arg(long)]
    force: bool,
}

#[derive(Args)]
struct ExportArgs {
    /// Output file; writes a new file atomically, never overwriting input.
    #[arg(value_name = "FILE")]
    path: PathBuf,
    /// Codec format/variant, e.g. pleco-utf8-text/v1.
    #[arg(long, default_value = "pleco-utf8-text/v1")]
    codec: String,
    /// Restrict to a single Pleco category.
    #[arg(long)]
    category: Option<String>,
    /// Restrict to items carrying this tag (repeatable).
    #[arg(long)]
    tag: Vec<String>,
    /// Allow exporting `needs_review` items (default: refuse).
    #[arg(long)]
    allow_unresolved: bool,
    /// Preview what would be exported without writing the file.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Args)]
struct SearchArgs {
    /// Search mode: english | pinyin | chinese | auto (guess from the query).
    #[arg(value_enum)]
    mode: CliSearchMode,
    #[arg(value_name = "QUERY")]
    query: String,
    /// Output format: human-readable text (default) or machine-readable JSON.
    #[arg(long, value_enum, default_value = "text")]
    format: OutputFormat,
    /// Maximum number of results to print (text and JSON).
    #[arg(long)]
    limit: Option<usize>,
}

/// CLI search modes: the dictionary modes plus `auto`, which delegates to
/// [`guess_mode`] so scripts and menu-bar sidecars need no mode switcher.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
enum CliSearchMode {
    English,
    Pinyin,
    Chinese,
    Auto,
}

impl CliSearchMode {
    fn resolve(self, query: &str) -> SearchMode {
        match self {
            Self::English => SearchMode::English,
            Self::Pinyin => SearchMode::Pinyin,
            Self::Chinese => SearchMode::Chinese,
            Self::Auto => guess_mode(query),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
enum OutputFormat {
    Text,
    Json,
}

/// One machine-readable search hit. Field names are the menu-bar contract;
/// adding fields is fine, renaming or removing is a breaking change.
#[derive(serde::Serialize)]
struct SearchHit {
    simplified: String,
    traditional: String,
    pinyin: String,
    glosses: Vec<String>,
    inferred: bool,
}

fn search_hits(ranked: &[vocab_dictionary::Candidate], limit: Option<usize>) -> Vec<SearchHit> {
    let take = limit.unwrap_or(ranked.len());
    ranked
        .iter()
        .take(take)
        .map(|candidate| SearchHit {
            simplified: candidate.entry.simplified.clone(),
            traditional: candidate.entry.traditional.clone(),
            pinyin: candidate.entry.pinyin.clone(),
            glosses: candidate.entry.glosses.clone(),
            inferred: candidate.diagnostic.is_inferred,
        })
        .collect()
}

/// Serializes ranked candidates to a single-line JSON array (`[]` when empty).
///
/// # Errors
///
/// Returns an error if serialization fails (cannot happen for these types,
/// but the signature keeps the failure explicit for callers).
fn format_search_json(
    ranked: &[vocab_dictionary::Candidate],
    limit: Option<usize>,
) -> Result<String> {
    let hits = search_hits(ranked, limit);
    serde_json::to_string(&hits).map_err(|err| VocabError::new(format!("encode JSON: {err}")))
}

#[derive(Args)]
struct ListArgs {
    /// Restrict to one status: confirmed | `needs_review` | exported | archived.
    #[arg(long, value_parser = parse_status)]
    status: Option<ItemStatus>,
}

fn parse_status(s: &str) -> std::result::Result<ItemStatus, String> {
    s.parse()
}

fn main() -> Result<()> {
    let Cli {
        dictionary,
        user_db,
        command,
    } = Cli::parse();
    match command {
        Command::Add(args) => run_add(dictionary.as_deref(), user_db.as_deref(), &args),
        Command::Import(args) => run_import(user_db.as_deref(), &args),
        Command::Export(args) => run_export(user_db.as_deref(), &args),
        Command::List(args) => run_list(user_db.as_deref(), &args),
        Command::Search(args) => run_search(dictionary.as_deref(), &args),
    }
}

fn run_search(cli_dictionary: Option<&std::path::Path>, args: &SearchArgs) -> Result<()> {
    let svc = open_service(cli_dictionary)?;
    let ranked = if args.mode == CliSearchMode::Auto {
        resolve_auto(&svc, &args.query)?.0
    } else {
        resolve(&svc, args.mode.resolve(&args.query), &args.query)?
    };
    match args.format {
        OutputFormat::Json => println!("{}", format_search_json(&ranked, args.limit)?),
        OutputFormat::Text => {
            let shown: Vec<_> = match args.limit {
                Some(n) => ranked.iter().take(n).collect(),
                None => ranked.iter().collect(),
            };
            if shown.is_empty() {
                println!("no results");
                return Ok(());
            }
            for candidate in shown {
                let inferred = if candidate.diagnostic.is_inferred {
                    "  [inferred]"
                } else {
                    ""
                };
                println!(
                    "{} / {}  [{}]  {}{}",
                    candidate.entry.simplified,
                    candidate.entry.traditional,
                    candidate.entry.pinyin,
                    candidate.entry.glosses.join("; "),
                    inferred
                );
            }
        }
    }
    Ok(())
}

fn run_add(
    cli_dictionary: Option<&std::path::Path>,
    cli_user_db: Option<&std::path::Path>,
    args: &AddArgs,
) -> Result<()> {
    let word = if args.clipboard {
        clipboard_text()?
    } else {
        args.word
            .clone()
            .ok_or_else(|| VocabError::new("no word given; pass one or use --clipboard"))?
    };
    if word.is_empty() {
        return Err(VocabError::new("empty query, nothing to add"));
    }

    let svc = open_service(cli_dictionary)?;
    match capture_to_path(&svc, cli_user_db, &word, args.mode)? {
        CaptureOutcome::Saved(item) => {
            let notice = if item.status == ItemStatus::NeedsReview {
                "no dictionary match; saved as needs_review"
            } else {
                "saved"
            };
            println!(
                "{notice}: {} / {}  [{}]",
                item.simplified, item.traditional, item.pinyin
            );
        }
        CaptureOutcome::AlreadySaved(item) => {
            println!(
                "already saved: {} / {}  [{}] (item {})",
                item.simplified, item.traditional, item.pinyin, item.item_id
            );
        }
        CaptureOutcome::Ambiguous { candidates, .. } => {
            println!("multiple matches, nothing saved — re-run with the exact headword:");
            for candidate in &candidates {
                println!(
                    "  {} / {}  [{}]  {}",
                    candidate.entry.simplified,
                    candidate.entry.traditional,
                    candidate.entry.pinyin,
                    candidate.entry.glosses.join("; ")
                );
            }
        }
    }
    Ok(())
}

fn run_list(cli_user_db: Option<&std::path::Path>, args: &ListArgs) -> Result<()> {
    let conn = open_capture_db(cli_user_db)?;
    let items = list_items(&conn, args.status)?;
    if items.is_empty() {
        println!("no saved items");
        return Ok(());
    }
    for item in &items {
        println!(
            "{}  {:>13}  {} / {}  [{}]  {}",
            item.item_id,
            item.status,
            item.simplified,
            item.traditional,
            item.pinyin,
            clip(&item.definition, 60)
        );
    }
    Ok(())
}

fn run_import(cli_user_db: Option<&std::path::Path>, args: &ImportArgs) -> Result<()> {
    let bytes = std::fs::read(&args.path)
        .map_err(|err| VocabError::new(format!("cannot read {}: {err}", args.path.display())))?;
    let registry = CodecRegistry::builtin();
    let codec = registry
        .get_by_key(&args.codec)
        .ok_or_else(|| VocabError::new(format!("unknown codec '{}'", args.codec)))?;
    let parsed = codec.parse(&bytes)?;

    for issue in &parsed.issues {
        let severity = match issue.severity {
            IssueSeverity::Info => "info",
            IssueSeverity::Warning => "warning",
            IssueSeverity::Error => "error",
        };
        println!("  line {} [{}]: {}", issue.line, severity, issue.message);
    }

    let mut records: Vec<ImportRecord> = Vec::new();
    let mut dropped_headwords = 0usize;
    for record in parsed.records {
        if record.simplified.trim().is_empty() {
            println!(
                "  line {} [error]: record has no headword; dropped",
                record.line
            );
            dropped_headwords += 1;
            continue;
        }
        records.push(ImportRecord {
            simplified: record.simplified,
            pinyin: record.pinyin,
            definition: record.definition,
            category: record.category,
        });
    }
    let issues_errors = parsed
        .issues
        .iter()
        .filter(|issue| issue.severity == IssueSeverity::Error)
        .count();
    let issues_warnings = parsed
        .issues
        .iter()
        .filter(|issue| issue.severity == IssueSeverity::Warning)
        .count();

    let mut conn = open_capture_db(cli_user_db)?;
    let payload = ImportPayload {
        source_path: args.path.to_string_lossy().into_owned(),
        codec_key: args.codec.clone(),
        content_sha256: vocab_dictionary::sha256_hex(&bytes),
        records,
        issues_errors: issues_errors + dropped_headwords,
        issues_warnings,
    };
    match persist_import(&mut conn, &payload, args.force)? {
        ImportDecision::Imported(summary) => {
            println!(
                "imported {} ({} duplicates skipped, {} seen, {} errors, {} warnings)",
                summary.records_imported,
                summary.records_skipped_duplicate,
                summary.records_seen,
                summary.issues_errors,
                summary.issues_warnings
            );
        }
        ImportDecision::Refused => {
            println!(
                "refused: source has {} error-severity lines; fix them or pass --force",
                payload.issues_errors
            );
        }
    }
    Ok(())
}

fn run_export(cli_user_db: Option<&std::path::Path>, args: &ExportArgs) -> Result<()> {
    let mut conn = open_capture_db(cli_user_db)?;
    let sources = import_sources(&conn)?;
    if sources
        .iter()
        .any(|source| source == &args.path.to_string_lossy())
    {
        return Err(VocabError::new(format!(
            "refusing to overwrite import source {}",
            args.path.display()
        )));
    }

    let mut required_tags = args.tag.clone();
    if let Some(category) = args.category.as_deref() {
        required_tags.push(category.to_owned());
    }
    let items = select_exportable(&conn, &required_tags, args.allow_unresolved)?;
    if items.is_empty() {
        println!("nothing to export");
        return Ok(());
    }

    let rows: Vec<ExportRow> = items
        .iter()
        .map(|item| ExportRow {
            simplified: item.simplified.clone(),
            pinyin: item.pinyin.clone(),
            definition: item.definition.clone(),
            category: item_tags(&conn, item.item_id)
                .ok()
                .and_then(|tags| tags.into_iter().next()),
        })
        .collect();

    if args.dry_run {
        println!(
            "dry run: would export {} items to {}",
            rows.len(),
            args.path.display()
        );
        for row in rows.iter().take(5) {
            println!(
                "  {} / {}  [{}]  {}",
                row.simplified,
                row.pinyin,
                row.category.as_deref().unwrap_or(""),
                row.definition
            );
        }
        return Ok(());
    }

    let registry = CodecRegistry::builtin();
    let codec = registry
        .get_by_key(&args.codec)
        .ok_or_else(|| VocabError::new(format!("unknown codec '{}'", args.codec)))?;
    let bytes = codec.serialize(&rows)?;
    atomic_write(&args.path, &bytes)?;

    commit_export(
        &mut conn,
        &ExportRun {
            target_path: args.path.to_string_lossy().into_owned(),
            codec_key: args.codec.clone(),
            records_written: rows.len(),
        },
        &items.iter().map(|item| item.item_id).collect::<Vec<_>>(),
    )?;
    println!("exported {} items to {}", rows.len(), args.path.display());
    Ok(())
}

fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
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

fn clip(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut clipped: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    clipped.push('…');
    clipped
}

#[cfg(test)]
mod tests {
    use vocab_core::{
        ConfirmationState, DictionaryEntry, MatchBasis, Provenance, SourceId, SourceVersion,
    };
    use vocab_dictionary::{Candidate, CandidateDiagnostic};

    use super::*;

    fn candidate(
        simplified: &str,
        traditional: &str,
        pinyin: &str,
        glosses: &[&str],
        inferred: bool,
    ) -> Candidate {
        Candidate {
            entry: DictionaryEntry {
                simplified: simplified.to_owned(),
                traditional: traditional.to_owned(),
                pinyin: pinyin.to_owned(),
                glosses: glosses.iter().map(ToString::to_string).collect(),
                frequency_rank: None,
                hsk_rank: None,
                stable_entry_id: Some(1),
                provenance: Provenance {
                    source: SourceId(String::from("test")),
                    source_version: SourceVersion(String::from("1.0")),
                    import_origin: None,
                    confirmation: ConfirmationState::DictionaryAuthority,
                },
            },
            diagnostic: CandidateDiagnostic {
                basis: MatchBasis::EnglishGloss,
                is_inferred: inferred,
            },
        }
    }

    #[test]
    fn auto_mode_delegates_to_guess() {
        assert_eq!(CliSearchMode::Auto.resolve("旅行"), SearchMode::Chinese);
        assert_eq!(CliSearchMode::Auto.resolve("lv3"), SearchMode::Pinyin);
        assert_eq!(CliSearchMode::Auto.resolve("school"), SearchMode::English);
        assert_eq!(
            CliSearchMode::English.resolve("旅行"),
            SearchMode::English,
            "explicit modes never guess"
        );
    }

    #[test]
    fn json_output_keeps_field_contract() {
        let ranked = vec![candidate("学校", "學校", "xue2 xiao4", &["school"], false)];
        let json = format_search_json(&ranked, None).unwrap();
        assert!(
            json.contains(r#""simplified":"学校""#),
            "unexpected JSON: {json}"
        );
        assert!(
            json.contains(r#""traditional":"學校""#),
            "unexpected JSON: {json}"
        );
        assert!(
            json.contains(r#""pinyin":"xue2 xiao4""#),
            "unexpected JSON: {json}"
        );
        assert!(
            json.contains(r#""glosses":["school"]"#),
            "unexpected JSON: {json}"
        );
        assert!(
            json.contains(r#""inferred":false"#),
            "unexpected JSON: {json}"
        );
        assert!(json.starts_with('[') && json.ends_with(']'));
        assert!(!json.contains('\n'), "single-line JSON for process piping");
    }

    #[test]
    fn json_output_respects_limit_and_empty() {
        let ranked = vec![
            candidate("一", "一", "yi1", &["one"], false),
            candidate("二", "二", "er4", &["two"], true),
        ];
        let limited = format_search_json(&ranked, Some(1)).unwrap();
        assert!(limited.contains('一'), "unexpected JSON: {limited}");
        assert!(!limited.contains('二'), "unexpected JSON: {limited}");
        assert!(limited.contains(r#""inferred":false"#));
        assert_eq!(format_search_json(&[], None).unwrap(), "[]");
    }
}
