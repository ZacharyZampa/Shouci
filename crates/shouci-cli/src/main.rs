//! `shouci`: your Shouci library from the command line.
//!
//! Every command is a call or two into `shouci-core`. This file reads the
//! arguments and prints what comes back: as lines for people, or with
//! `--json` as the core's own values, one JSON document per command.

mod text;
mod words;

use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use clap::{ArgGroup, Args, Parser, Subcommand, ValueEnum};
use serde::Serialize;
use shouci_core::{
    BulkAction, Config, DictionaryEntryView, DictionaryStatus, Error, ExportRequest, ExportScope,
    ImportPolicy, ItemPatch, ItemView, LibraryFilter, LibraryView, LoadingStage, ManualWord,
    QueryKind, QuickAdd, Result, SaveOutcome, SaveResult, Shouci, TransferSummary, Verification,
};

use crate::text::counted;
use crate::words::{Among, Words};

#[derive(Parser)]
#[command(
    name = "shouci",
    version,
    about = "Collect Chinese vocabulary: search, save, organize, import, export",
    after_help = AFTER_HELP
)]
struct Cli {
    /// Print the result as JSON instead of text.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

const AFTER_HELP: &str = "\
Name a saved word by its id (from `shouci list`) or its characters.

Your words are in ~/Library/Application Support/Shouci on macOS (the same
library as the Shouci app). Set SHOUCI_HOME to use another folder, and
SHOUCI_DICTIONARIES to keep dictionaries somewhere else.";

#[derive(Subcommand)]
enum Command {
    /// Search the dictionary.
    Search(SearchArgs),
    /// Save a word: the dictionary's match, or the word as you type it.
    Add(AddArgs),
    /// List your words, newest first, or search them.
    List(ListArgs),
    /// Show a saved word and what the dictionaries say about it.
    Show(WordArg),
    /// Change a saved word.
    Edit(EditArgs),
    /// Archive words: kept, but out of your lists.
    Archive(WordsArg),
    /// Bring words back from the archive.
    Unarchive(WordsArg),
    /// Move words to the trash.
    Delete(WordsArg),
    /// Bring words back from the trash.
    Restore(WordsArg),
    /// Delete words in the trash for good.
    Purge(PurgeArgs),
    /// List tags, or tag and untag words.
    Tags {
        #[command(subcommand)]
        action: Option<TagAction>,
    },
    /// List collections, or add words to them.
    Collections {
        #[command(subcommand)]
        action: Option<CollectionAction>,
    },
    /// List dictionaries, choose which to search, or update them.
    Dictionaries {
        #[command(subcommand)]
        action: Option<DictionaryAction>,
    },
    /// Import words from a Pleco or Anki text file.
    Import(ImportArgs),
    /// Export words to a Pleco or Anki text file.
    Export(ExportArgs),
    /// Bring words over from the proof of concept's library.
    MigratePoc(MigrateArgs),
}

/// How to read a query.
#[derive(Clone, Copy, ValueEnum)]
enum Kind {
    #[value(alias = "chinese")]
    Hanzi,
    Pinyin,
    English,
}

impl From<Kind> for QueryKind {
    fn from(kind: Kind) -> Self {
        match kind {
            Kind::Hanzi => Self::Chinese,
            Kind::Pinyin => Self::Pinyin,
            Kind::English => Self::English,
        }
    }
}

#[derive(Args)]
struct SearchArgs {
    /// Characters, pinyin (tones optional), or English.
    #[arg(required = true, value_name = "QUERY")]
    query: Vec<String>,
    /// Read the query as this instead of working it out.
    #[arg(long = "as", value_name = "KIND")]
    kind: Option<Kind>,
    /// Show at most this many results.
    #[arg(long, default_value_t = 20)]
    limit: u32,
}

#[derive(Args)]
struct AddArgs {
    /// Characters, pinyin, or English.
    #[arg(
        value_name = "WORD",
        required_unless_present = "clipboard",
        conflicts_with = "clipboard"
    )]
    word: Vec<String>,
    /// Take the word from the clipboard (macOS).
    #[arg(short, long)]
    clipboard: bool,
    /// Read the word as this instead of working it out.
    #[arg(long = "as", value_name = "KIND", conflicts_with_all = BY_HAND)]
    kind: Option<Kind>,
    /// When several words match, save result N of `shouci search WORD`.
    #[arg(
        long,
        value_name = "N",
        conflicts_with_all = BY_HAND,
        value_parser = clap::value_parser!(u32).range(1..)
    )]
    pick: Option<u32>,
    /// Save the characters as typed, with this traditional form.
    #[arg(long)]
    traditional: Option<String>,
    /// Save the characters as typed, with this reading (`xue2 xiao4`).
    #[arg(long)]
    pinyin: Option<String>,
    /// Save the characters as typed, with this definition.
    #[arg(long)]
    definition: Option<String>,
    /// Save the characters as typed, with these notes.
    #[arg(long)]
    notes: Option<String>,
    /// Tag the word (repeat for more).
    #[arg(long = "tag", value_name = "TAG")]
    tags: Vec<String>,
    /// Add the word to a collection (repeat for more).
    #[arg(long = "collection", value_name = "NAME")]
    collections: Vec<String>,
}

/// The options that save a word as typed instead of looking it up.
const BY_HAND: [&str; 4] = ["traditional", "pinyin", "definition", "notes"];

impl AddArgs {
    fn by_hand(&self) -> bool {
        self.traditional.is_some()
            || self.pinyin.is_some()
            || self.definition.is_some()
            || self.notes.is_some()
    }
}

#[derive(Args)]
struct ListArgs {
    /// Only words matching this: characters, pinyin, or English in
    /// definitions and notes.
    #[arg(value_name = "QUERY")]
    query: Vec<String>,
    #[command(flatten)]
    filter: FilterArgs,
    /// Show at most this many words.
    #[arg(long)]
    limit: Option<u32>,
}

#[derive(Args)]
struct FilterArgs {
    /// Which words.
    #[arg(long, value_enum, default_value_t = View::Active)]
    view: View,
    /// Only words that need review.
    #[arg(long)]
    needs_review: bool,
    /// Only confirmed words.
    #[arg(long, conflicts_with = "needs_review")]
    confirmed: bool,
    /// Only words with this tag (repeat to require several).
    #[arg(long = "tag", value_name = "TAG")]
    tags: Vec<String>,
    /// Only words in this collection.
    #[arg(long, value_name = "NAME")]
    collection: Option<String>,
}

#[derive(Clone, Copy, ValueEnum)]
enum View {
    /// Neither archived nor in the trash.
    Active,
    Archived,
    Trash,
    /// Active and archived.
    All,
}

impl FilterArgs {
    fn filter(&self) -> LibraryFilter {
        LibraryFilter {
            view: match self.view {
                View::Active => LibraryView::Active,
                View::Archived => LibraryView::Archived,
                View::Trash => LibraryView::Trash,
                View::All => LibraryView::All,
            },
            verification: if self.needs_review {
                Some(Verification::NeedsReview)
            } else if self.confirmed {
                Some(Verification::Confirmed)
            } else {
                None
            },
            tags: self.tags.clone(),
            collection: self.collection.clone(),
        }
    }
}

#[derive(Args)]
struct WordArg {
    /// An id or the characters.
    word: String,
}

#[derive(Args)]
struct WordsArg {
    /// Ids or characters.
    #[arg(required = true, value_name = "WORD")]
    words: Vec<String>,
}

#[derive(Args)]
#[command(group(
    ArgGroup::new("change")
        .required(true)
        .multiple(true)
        .args(["simplified", "traditional", "pinyin", "definition", "notes", "needs_review", "confirmed"])
))]
struct EditArgs {
    /// An id or the characters.
    word: String,
    #[arg(long)]
    simplified: Option<String>,
    #[arg(long)]
    traditional: Option<String>,
    /// Numbered, like `xue2 xiao4`.
    #[arg(long)]
    pinyin: Option<String>,
    #[arg(long)]
    definition: Option<String>,
    #[arg(long)]
    notes: Option<String>,
    /// Mark the word as needing review.
    #[arg(long)]
    needs_review: bool,
    /// Mark the word as checked.
    #[arg(long, conflicts_with = "needs_review")]
    confirmed: bool,
}

#[derive(Args)]
struct PurgeArgs {
    /// Words in the trash: ids or characters.
    #[arg(value_name = "WORD", required_unless_present = "all")]
    words: Vec<String>,
    /// Empty the trash.
    #[arg(long, conflicts_with = "words")]
    all: bool,
}

#[derive(Subcommand)]
enum TagAction {
    /// List tags and how many words have each.
    List,
    /// Tag words.
    Add {
        tag: String,
        #[arg(required = true, value_name = "WORD")]
        words: Vec<String>,
    },
    /// Untag words.
    Remove {
        tag: String,
        #[arg(required = true, value_name = "WORD")]
        words: Vec<String>,
    },
    Rename {
        from: String,
        to: String,
    },
    /// Delete a tag. Its words stay.
    Delete {
        tag: String,
    },
}

#[derive(Subcommand)]
enum CollectionAction {
    /// List collections and how many words are in each.
    List,
    /// Create an empty collection.
    Create {
        name: String,
    },
    /// Add words to a collection, creating it if needed.
    Add {
        name: String,
        #[arg(required = true, value_name = "WORD")]
        words: Vec<String>,
    },
    /// Take words out of a collection.
    Remove {
        name: String,
        #[arg(required = true, value_name = "WORD")]
        words: Vec<String>,
    },
    Rename {
        from: String,
        to: String,
    },
    /// Delete a collection. Its words stay.
    Delete {
        name: String,
    },
}

#[derive(Subcommand)]
enum DictionaryAction {
    /// List installed dictionaries in the order search uses them.
    List,
    /// Search only these dictionaries, in this order.
    Use {
        #[arg(required = true, value_name = "ID")]
        ids: Vec<String>,
    },
    /// Download missing dictionaries and refresh any over a month old.
    Update,
}

#[derive(Args)]
struct ImportArgs {
    file: PathBuf,
    /// The file's format, `pleco` or `anki` (default: whichever reads it).
    #[arg(long, value_name = "FORMAT")]
    from: Option<String>,
    /// What to do with words you already have.
    #[arg(long, value_enum, default_value_t = Existing::Skip)]
    existing: Existing,
    /// Import even when some lines have errors; those lines are skipped.
    #[arg(long)]
    force: bool,
    /// Show what would happen without changing anything.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum Existing {
    /// Leave them as they are.
    Skip,
    /// Fill in what they are missing; keep what differs.
    Merge,
    /// Replace each field the file fills in.
    Overwrite,
}

impl From<Existing> for ImportPolicy {
    fn from(existing: Existing) -> Self {
        match existing {
            Existing::Skip => Self::Skip,
            Existing::Merge => Self::Merge,
            Existing::Overwrite => Self::Overwrite,
        }
    }
}

#[derive(Args)]
// Flags are bools in clap.
#[allow(clippy::struct_excessive_bools)]
struct ExportArgs {
    file: PathBuf,
    /// `pleco` or `anki`.
    #[arg(long, value_name = "FORMAT", default_value = "pleco")]
    to: String,
    /// Every matching word, not only those new since the last export there.
    #[arg(long)]
    all: bool,
    /// Only this word (repeat for more): an id or the characters.
    #[arg(long = "word", value_name = "WORD", conflicts_with = "all")]
    words: Vec<String>,
    #[command(flatten)]
    filter: FilterArgs,
    /// Include words that need review.
    #[arg(long)]
    include_needs_review: bool,
    /// The Anki deck (default: the collection being exported).
    #[arg(long, value_name = "NAME")]
    deck: Option<String>,
    /// Write over the file if it exists.
    #[arg(long)]
    replace: bool,
    /// Show what would be written without writing it.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Args)]
struct MigrateArgs {
    /// The old library (default:
    /// ~/Library/Application Support/pleco-companion/user.db).
    file: Option<PathBuf>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let out = Out { json: cli.json };
    match run(cli.command, out) {
        Ok(code) => code,
        Err(err) => {
            out.error(&err);
            ExitCode::FAILURE
        }
    }
}

fn run(command: Command, out: Out) -> Result<ExitCode> {
    match command {
        Command::Search(args) => search(&args, out),
        Command::Add(args) => add(args, out),
        Command::List(args) => list(&args, out),
        Command::Show(args) => show(&args, out),
        Command::Edit(args) => edit(args, out),
        Command::Archive(args) => change(
            &args.words,
            Among::Saved,
            &BulkAction::Archive,
            out,
            |names| format!("archived {names}"),
        ),
        Command::Unarchive(args) => change(
            &args.words,
            Among::Saved,
            &BulkAction::Unarchive,
            out,
            |names| format!("unarchived {names}"),
        ),
        Command::Delete(args) => change(
            &args.words,
            Among::Saved,
            &BulkAction::Trash,
            out,
            |names| format!("moved {names} to the trash (`shouci restore` brings them back)"),
        ),
        Command::Restore(args) => change(
            &args.words,
            Among::Trash,
            &BulkAction::Restore,
            out,
            |names| format!("brought {names} back from the trash"),
        ),
        Command::Purge(args) => purge(&args, out),
        Command::Tags { action } => tags(action.unwrap_or(TagAction::List), out),
        Command::Collections { action } => {
            collections(action.unwrap_or(CollectionAction::List), out)
        }
        Command::Dictionaries { action } => {
            dictionaries(action.unwrap_or(DictionaryAction::List), out)
        }
        Command::Import(args) => import(&args, out),
        Command::Export(args) => export(args, out),
        Command::MigratePoc(args) => migrate(args, out),
    }
}

// --- Output ---------------------------------------------------------------

#[derive(Clone, Copy)]
struct Out {
    json: bool,
}

impl Out {
    /// Prints a result: the value itself with `--json`, otherwise `text`.
    fn show<T: Serialize>(self, value: &T, text: impl FnOnce() -> String) -> Result<ExitCode> {
        if self.json {
            let json = serde_json::to_string(value)
                .map_err(|err| Error::new(format!("cannot write JSON: {err}")))?;
            print(&json);
        } else {
            let text = text();
            if !text.is_empty() {
                print(&text);
            }
        }
        Ok(ExitCode::SUCCESS)
    }

    fn error(self, err: &Error) {
        if self.json {
            let error = serde_json::json!({
                "error": { "kind": err.kind(), "message": err.message() }
            });
            eprintln!("{error}");
        } else {
            eprintln!("shouci: {err}");
        }
    }
}

/// Writes to stdout. A reader that went away (`shouci list | head`) ends the
/// program quietly.
fn print(text: &str) {
    if let Err(err) = writeln!(std::io::stdout().lock(), "{text}") {
        if err.kind() == std::io::ErrorKind::BrokenPipe {
            std::process::exit(0);
        }
        eprintln!("shouci: cannot write the output: {err}");
        std::process::exit(1);
    }
}

/// Something worth knowing that isn't the result: on stderr, so pipes
/// carry only results.
fn note(text: &str) {
    eprintln!("shouci: {text}");
}

// --- Opening the library --------------------------------------------------

/// The library, with dictionaries not yet loaded. Only `dictionaries
/// update` downloads them on purpose (and a first search, when there are
/// none): the monthly refresh never holds up a command.
fn open(fetch: bool) -> Result<Shouci> {
    let mut config = Config::from_env();
    config.fetch_dictionaries = fetch;
    let shouci = Shouci::open(config)?;
    for line in shouci.startup_notes() {
        note(line);
    }
    Ok(shouci)
}

/// The library, ready to search. With no dictionary installed yet, it is
/// downloaded first.
fn open_searchable() -> Result<Shouci> {
    let shouci = open(false)?;
    if shouci.dictionaries()?.is_empty() {
        drop(shouci);
        let shouci = open(true)?;
        load(&shouci)?;
        return Ok(shouci);
    }
    load(&shouci)?;
    Ok(shouci)
}

/// The library, with dictionaries loaded when they can be. For commands
/// that work without them.
fn open_maybe_searchable() -> Result<Shouci> {
    let shouci = open(false)?;
    if !shouci.dictionaries()?.is_empty() {
        if let Err(err) = load(&shouci) {
            note(&format!("{err}; going on without the dictionary"));
        }
    }
    Ok(shouci)
}

/// Loads the dictionaries, saying what a long load is doing.
fn load(shouci: &Shouci) -> Result<()> {
    let done = AtomicBool::new(false);
    std::thread::scope(|scope| {
        if std::io::stderr().is_terminal() {
            scope.spawn(|| report_progress(shouci, &done));
        }
        let loaded = shouci.load_dictionaries();
        done.store(true, Ordering::SeqCst);
        loaded
    })?;
    if let DictionaryStatus::Ready { notes, .. } = shouci.dictionary_status() {
        for line in &notes {
            note(line);
        }
    }
    Ok(())
}

fn report_progress(shouci: &Shouci, done: &AtomicBool) {
    let mut last = None;
    while !done.load(Ordering::SeqCst) {
        let now = match shouci.dictionary_status() {
            DictionaryStatus::Loading { stage, dictionary } => {
                progress(stage, dictionary.as_deref())
            }
            DictionaryStatus::Ready {
                updating: Some(stage),
                ..
            } => progress(stage, None),
            _ => None,
        };
        if now.is_some() && now != last {
            note(now.as_deref().unwrap_or_default());
            last = now;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn progress(stage: LoadingStage, dictionary: Option<&str>) -> Option<String> {
    let name = dictionary.unwrap_or("the dictionary");
    match stage {
        LoadingStage::Checking | LoadingStage::Opening => None,
        LoadingStage::Waiting => Some(format!(
            "waiting for another Shouci to finish building {name}…"
        )),
        LoadingStage::Downloading => Some(format!(
            "downloading {name} (needs an internet connection)…"
        )),
        LoadingStage::Building => Some(format!("building {name}…")),
    }
}

fn format_name(shouci: &Shouci, connector: &str) -> String {
    shouci
        .connectors()
        .into_iter()
        .find(|view| view.id == connector)
        .map_or_else(|| connector.to_owned(), |view| view.name)
}

/// `学校`, `学校 and 猫`, `学校, 猫, and 米饭`.
fn names(items: &[&ItemView]) -> String {
    let names: Vec<&str> = items.iter().map(|item| item.simplified.as_str()).collect();
    match names.as_slice() {
        [] => "nothing".to_owned(),
        [one] => (*one).to_owned(),
        [first, second] => format!("{first} and {second}"),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    }
}

// --- Words ----------------------------------------------------------------

fn search(args: &SearchArgs, out: Out) -> Result<ExitCode> {
    let shouci = open_searchable()?;
    let query = args.query.join(" ");
    let found = shouci.search_dictionary(&query, args.kind.map(Into::into), Some(args.limit))?;
    out.show(&found, || {
        let mut lines = text::numbered(&found.candidates);
        let strong = found.candidates.iter().any(|candidate| !candidate.inferred);
        if !strong && found.kind == QueryKind::Chinese && text::has_han(&found.query) {
            lines.insert(
                0,
                format!(
                    "no dictionary entry for {0}; `shouci add {0}` saves it to fill in later",
                    found.query
                ),
            );
        } else if lines.is_empty() {
            lines.push("no matches".to_owned());
        }
        let shown = u32::try_from(found.candidates.len()).unwrap_or(u32::MAX);
        if found.total > shown {
            lines.push(format!(
                "… {} more (--limit shows more)",
                found.total - shown
            ));
        }
        lines.join("\n")
    })
}

fn add(args: AddArgs, out: Out) -> Result<ExitCode> {
    let word = if args.clipboard {
        clipboard()?
    } else {
        args.word.join(" ")
    };
    let result = if args.by_hand() {
        let shouci = open_maybe_searchable()?;
        QuickAdd::Saved(Box::new(shouci.add_manual(&ManualWord {
            simplified: word.clone(),
            traditional: args.traditional,
            pinyin: args.pinyin,
            definition: args.definition,
            notes: args.notes,
            tags: args.tags,
            collections: args.collections,
        })?))
    } else {
        let shouci = open_searchable()?;
        let kind = args.kind.map(Into::into);
        let found = match args.pick {
            Some(n) => QuickAdd::Saved(Box::new(pick(&shouci, &word, kind, n)?)),
            None => shouci.quick_add(&word, kind)?,
        };
        match found {
            QuickAdd::Saved(mut saved) => {
                organize(&shouci, &mut saved, &args.tags, &args.collections)?;
                QuickAdd::Saved(saved)
            }
            ambiguous @ QuickAdd::Ambiguous { .. } => ambiguous,
        }
    };
    out.show(&result, || add_text(&result, &word))?;
    Ok(match result {
        QuickAdd::Saved(_) => ExitCode::SUCCESS,
        QuickAdd::Ambiguous { .. } => ExitCode::FAILURE,
    })
}

/// Saves result `n` (from 1) of searching the dictionary for `word`.
fn pick(shouci: &Shouci, word: &str, kind: Option<QueryKind>, n: u32) -> Result<SaveResult> {
    let found = shouci.search_dictionary(word, kind, Some(n))?;
    let index = usize::try_from(n - 1).unwrap_or(usize::MAX);
    let candidate = found.candidates.get(index).ok_or_else(|| {
        Error::not_found(format!(
            "{word} has {}; `shouci search {word}` lists them",
            counted(found.total, "result", "results")
        ))
    })?;
    shouci.save_candidate(candidate)
}

/// Tags a word saved by `add` and puts it in collections.
fn organize(
    shouci: &Shouci,
    saved: &mut SaveResult,
    tags: &[String],
    collections: &[String],
) -> Result<()> {
    if tags.is_empty() && collections.is_empty() {
        return Ok(());
    }
    let ids = [saved.item.id];
    if !tags.is_empty() {
        shouci.bulk(&ids, &BulkAction::AddTags(tags.to_vec()))?;
    }
    for collection in collections {
        shouci.bulk(&ids, &BulkAction::AddToCollection(collection.clone()))?;
    }
    saved.item = shouci.item(saved.item.id)?;
    Ok(())
}

/// Words listed when `add` cannot choose.
const CHOICES_SHOWN: usize = 10;

fn add_text(result: &QuickAdd, word: &str) -> String {
    match result {
        QuickAdd::Saved(saved) => {
            let item = &saved.item;
            let word = text::word_line(item);
            match saved.outcome {
                SaveOutcome::Inserted if item.pinyin.is_empty() => format!(
                    "saved {} to fill in later (no dictionary entry, so it needs review)",
                    item.simplified
                ),
                SaveOutcome::Inserted => format!("saved {word}"),
                SaveOutcome::AlreadySaved => format!("already saved: {word}"),
                SaveOutcome::Restored => format!("brought back from the trash: {word}"),
                SaveOutcome::Completed => format!("filled in {word}"),
            }
        }
        QuickAdd::Ambiguous { candidates } => {
            let mut lines = vec![format!(
                "several words match, so nothing was saved. Save one with \
                 `shouci add {word} --pick N`:"
            )];
            let shown = candidates.len().min(CHOICES_SHOWN);
            lines.extend(text::numbered(&candidates[..shown]));
            if candidates.len() > shown {
                lines.push(format!(
                    "… more with `shouci search {word} --limit {}`",
                    candidates.len()
                ));
            }
            lines.join("\n")
        }
    }
}

fn clipboard() -> Result<String> {
    if !cfg!(target_os = "macos") {
        return Err(Error::unavailable("--clipboard works on macOS only"));
    }
    let output = std::process::Command::new("pbpaste")
        .output()
        .map_err(|err| Error::io(format!("cannot read the clipboard: {err}")))?;
    let text = String::from_utf8(output.stdout)
        .map_err(|_| Error::invalid("the clipboard does not hold text"))?;
    let text = text.trim();
    if text.is_empty() {
        return Err(Error::invalid("the clipboard is empty"));
    }
    Ok(text.to_owned())
}

fn list(args: &ListArgs, out: Out) -> Result<ExitCode> {
    let shouci = open(false)?;
    let query = args.query.join(" ");
    let found = shouci.search_library(&query, &args.filter.filter(), None, args.limit)?;
    out.show(&found, || {
        if found.items.is_empty() {
            return if query.trim().is_empty() {
                "no saved words".to_owned()
            } else {
                format!("no saved word matches {}", query.trim())
            };
        }
        let mut lines: Vec<String> = found.items.iter().map(text::item_line).collect();
        let shown = u32::try_from(found.items.len()).unwrap_or(u32::MAX);
        if found.total > shown {
            lines.push(format!("… {} more", found.total - shown));
        }
        lines.join("\n")
    })
}

/// `shouci show --json`.
#[derive(Serialize)]
struct Shown {
    item: ItemView,
    /// What each searched dictionary has for the word.
    dictionaries: Vec<Lookup>,
}

#[derive(Serialize)]
struct Lookup {
    id: String,
    name: String,
    entries: Vec<DictionaryEntryView>,
}

fn show(args: &WordArg, out: Out) -> Result<ExitCode> {
    let shouci = open(false)?;
    let item = Words::load(&shouci)?
        .find(&args.word, Among::Saved)?
        .clone();
    let installed = shouci.dictionaries()?;
    let mut lookups = Vec::new();
    for dictionary in installed.iter().filter(|dictionary| dictionary.enabled) {
        lookups.push(Lookup {
            id: dictionary.id.clone(),
            name: dictionary.name.clone(),
            entries: shouci.lookup_in(item.id, &dictionary.id)?,
        });
    }
    let shown = Shown {
        item,
        dictionaries: lookups,
    };
    out.show(&shown, || {
        let mut sections = vec![text::item_detail(
            &shown.item,
            &installed,
            &shouci.connectors(),
        )];
        sections.extend(
            shown
                .dictionaries
                .iter()
                .map(|lookup| text::lookup_section(&lookup.name, &lookup.entries)),
        );
        sections.join("\n\n")
    })
}

fn edit(args: EditArgs, out: Out) -> Result<ExitCode> {
    let shouci = open(false)?;
    let id = Words::load(&shouci)?.find(&args.word, Among::Saved)?.id;
    let verification = if args.needs_review {
        Some(Verification::NeedsReview)
    } else if args.confirmed {
        Some(Verification::Confirmed)
    } else {
        None
    };
    let item = shouci.update_item(
        id,
        &ItemPatch {
            simplified: args.simplified,
            traditional: args.traditional,
            pinyin: args.pinyin,
            definition: args.definition,
            notes: args.notes,
            verification,
        },
    )?;
    out.show(&item, || format!("changed {}", text::word_line(&item)))
}

/// One action on several words.
fn change(
    words: &[String],
    among: Among,
    action: &BulkAction,
    out: Out,
    done: impl FnOnce(&str) -> String,
) -> Result<ExitCode> {
    let shouci = open(false)?;
    let library = Words::load(&shouci)?;
    let items = words
        .iter()
        .map(|word| library.find(word, among))
        .collect::<Result<Vec<_>>>()?;
    let ids: Vec<i64> = items.iter().map(|item| item.id).collect();
    let result = shouci.bulk(&ids, action)?;
    out.show(&result, || done(&names(&items)))
}

fn purge(args: &PurgeArgs, out: Out) -> Result<ExitCode> {
    if args.all {
        let shouci = open(false)?;
        let result = shouci.empty_trash()?;
        return out.show(&result, || {
            format!(
                "emptied the trash: {} deleted for good",
                counted(result.changed, "word", "words")
            )
        });
    }
    change(
        &args.words,
        Among::Trash,
        &BulkAction::Purge,
        out,
        |names| format!("deleted {names} for good"),
    )
}

// --- Tags, collections, dictionaries --------------------------------------

fn tags(action: TagAction, out: Out) -> Result<ExitCode> {
    let shouci = open(false)?;
    match action {
        TagAction::List => {
            let tags = shouci.tags()?;
            out.show(&tags, || text::group_lines(&tags, "no tags"))
        }
        TagAction::Add { tag, words } => change(
            &words,
            Among::Saved,
            &BulkAction::AddTags(vec![tag.clone()]),
            out,
            |names| format!("tagged {names} {tag}"),
        ),
        TagAction::Remove { tag, words } => change(
            &words,
            Among::Saved,
            &BulkAction::RemoveTags(vec![tag.clone()]),
            out,
            |names| format!("untagged {names} {tag}"),
        ),
        TagAction::Rename { from, to } => {
            shouci.rename_tag(&from, &to)?;
            out.show(&shouci.tags()?, || format!("renamed tag {from} to {to}"))
        }
        TagAction::Delete { tag } => {
            shouci.delete_tag(&tag)?;
            out.show(&shouci.tags()?, || {
                format!("deleted tag {tag}; its words are still saved")
            })
        }
    }
}

fn collections(action: CollectionAction, out: Out) -> Result<ExitCode> {
    let shouci = open(false)?;
    match action {
        CollectionAction::List => {
            let collections = shouci.collections()?;
            out.show(&collections, || {
                text::group_lines(&collections, "no collections")
            })
        }
        CollectionAction::Create { name } => {
            shouci.create_collection(&name)?;
            out.show(&shouci.collections()?, || {
                format!("created collection {name}")
            })
        }
        CollectionAction::Add { name, words } => change(
            &words,
            Among::Saved,
            &BulkAction::AddToCollection(name.clone()),
            out,
            |names| format!("added {names} to {name}"),
        ),
        CollectionAction::Remove { name, words } => change(
            &words,
            Among::Saved,
            &BulkAction::RemoveFromCollection(name.clone()),
            out,
            |names| format!("took {names} out of {name}"),
        ),
        CollectionAction::Rename { from, to } => {
            shouci.rename_collection(&from, &to)?;
            out.show(&shouci.collections()?, || {
                format!("renamed collection {from} to {to}")
            })
        }
        CollectionAction::Delete { name } => {
            shouci.delete_collection(&name)?;
            out.show(&shouci.collections()?, || {
                format!("deleted collection {name}; its words are still saved")
            })
        }
    }
}

fn dictionaries(action: DictionaryAction, out: Out) -> Result<ExitCode> {
    let shouci = match action {
        DictionaryAction::Update => {
            let shouci = open(true)?;
            load(&shouci)?;
            shouci
        }
        DictionaryAction::List | DictionaryAction::Use { .. } => open(false)?,
    };
    let dictionaries = match action {
        DictionaryAction::Use { ids } => shouci.set_enabled_dictionaries(&ids)?,
        DictionaryAction::List | DictionaryAction::Update => shouci.dictionaries()?,
    };
    out.show(&dictionaries, || text::dictionary_lines(&dictionaries))
}

// --- Import and export ----------------------------------------------------

fn import(args: &ImportArgs, out: Out) -> Result<ExitCode> {
    let shouci = open_searchable()?;
    let policy = args.existing.into();
    let plan = match &args.from {
        Some(format) => shouci.preview_import(&args.file, format, policy, args.force)?,
        None => shouci.detect_import(&args.file, policy, args.force)?,
    };
    let format = format_name(&shouci, &plan.connector_id);
    if args.dry_run {
        return out.show(&plan, || text::import_preview(&plan, &format));
    }
    if plan.refused {
        out.show(&plan, || text::issue_lines(&plan).join("\n"))?;
        let errors = plan.counts().errors;
        return Err(Error::invalid(format!(
            "{} of {} {} errors, so nothing was imported. Fix {}, or use --force to \
             import the rest.",
            counted(errors, "line", "lines"),
            plan.path,
            if errors == 1 { "has" } else { "have" },
            if errors == 1 { "it" } else { "them" },
        )));
    }
    let summary = shouci.apply_import(&plan)?;
    out.show(&summary, || {
        let mut lines = vec![text::import_summary(&summary, &format)];
        lines.extend(text::issue_lines(&plan));
        lines.join("\n")
    })
}

fn export(args: ExportArgs, out: Out) -> Result<ExitCode> {
    let shouci = open_maybe_searchable()?;
    let scope = if !args.words.is_empty() {
        let library = Words::load(&shouci)?;
        ExportScope::Selected(library.ids(&args.words, Among::Saved)?)
    } else if args.all {
        ExportScope::All
    } else {
        ExportScope::New
    };
    let filter = args.filter.filter();
    let request = ExportRequest {
        scope,
        include_needs_review: args.include_needs_review
            || filter.verification == Some(Verification::NeedsReview),
        filter,
        deck: args.deck,
    };
    let plan = shouci.preview_export(&args.file, &args.to, &request)?;
    let format = format_name(&shouci, &plan.connector_id);
    if args.dry_run {
        return out.show(&plan, || text::export_preview(&plan, &format));
    }
    if plan.item_ids.is_empty() {
        let mut notes = text::left_out(&plan, &format);
        notes.extend(plan.notes.iter().cloned());
        let summary = TransferSummary {
            connector_id: plan.connector_id.clone(),
            path: plan.path.clone(),
            notes,
            ..TransferSummary::default()
        };
        return out.show(&summary, || {
            let mut lines = vec!["nothing to export, so no file was written".to_owned()];
            lines.extend(summary.notes.iter().cloned());
            lines.join("\n")
        });
    }
    if plan.replaces_existing && !args.replace {
        return Err(Error::conflict(format!(
            "{} exists; --replace writes over it",
            plan.path
        )));
    }
    let summary = shouci.apply_export(&plan)?;
    out.show(&summary, || {
        let mut lines = vec![text::export_summary(&summary, &format)];
        lines.extend(text::left_out(&plan, &format));
        lines.join("\n")
    })
}

fn migrate(args: MigrateArgs, out: Out) -> Result<ExitCode> {
    let shouci = open(false)?;
    let path = match args.file {
        Some(path) => path,
        None => shouci
            .config()
            .legacy_dir
            .as_ref()
            .map(|dir| dir.join("user.db"))
            .ok_or_else(|| {
                Error::invalid("name the old user.db (SHOUCI_HOME is set, so there is no default)")
            })?,
    };
    let report = shouci.import_poc(&path)?;
    out.show(&report, || {
        let mut lines = vec![format!(
            "brought {} over from {} ({} already here)",
            counted(report.items_added, "word", "words"),
            path.display(),
            report.items_already_saved
        )];
        lines.extend(
            report
                .skipped
                .iter()
                .map(|why| format!("not brought over: {why}")),
        );
        lines.join("\n")
    })
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::{Cli, ItemView, names};

    #[test]
    fn the_arguments_are_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn names_read_as_a_list() {
        let item = |simplified: &str| ItemView {
            simplified: simplified.to_owned(),
            ..serde_json::from_value(serde_json::json!({
                "id": 1, "simplified": "", "traditional": "", "pinyin": "",
                "pinyin_display": "", "definition": "", "definition_display": "",
                "notes": "", "verification": "confirmed", "lifecycle": "active",
                "source": { "kind": "manual", "id": null, "version": null, "import_origin": null },
                "tags": [], "collections": [], "destinations": [],
                "created_at": "", "modified_at": "", "archived_at": null, "deleted_at": null,
                "rev": 1
            }))
            .unwrap()
        };
        let (a, b, c) = (item("学校"), item("猫"), item("米饭"));
        assert_eq!(names(&[&a]), "学校");
        assert_eq!(names(&[&a, &b]), "学校 and 猫");
        assert_eq!(names(&[&a, &b, &c]), "学校, 猫, and 米饭");
    }
}
