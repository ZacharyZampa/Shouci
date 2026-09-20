//! Shared quick-capture orchestration used by the `vocab` CLI and picker UIs.
//!
//! The capture decision table is written once here so CLI `add` stays consistent:
//!
//! * exactly one strong (non-inferred) dictionary match → save it `confirmed`;
//! * no dictionary match at all → save the raw query as `needs_review`;
//! * two or more strong matches → save **nothing** and surface the candidates.
//!
//! Ambiguity is never resolved silently: the caller decides how to present the
//! candidates, but no item is written on the caller's behalf.

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use vocab_core::{
    ConfirmationState, ItemStatus, Provenance, Result, SourceId, SourceVersion, VocabError,
    VocabItem,
};
use vocab_db::{NewVocabItem, SaveOutcome, app_data_dir, open_user_db, save_vocab_item};
use vocab_dictionary::{Candidate, SqliteDictionary};
use vocab_pinyin::{NormalizedPinyin, normalize, segment};
use vocab_search::{SearchService, english_has_lemma};

/// Query mode for a capture or search, chosen explicitly or guessed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum SearchMode {
    English,
    Pinyin,
    Chinese,
}

impl SearchMode {
    /// Lowercase name used in CLI/UI status lines (`english`, `pinyin`, `chinese`).
    #[must_use]
    pub fn as_label(self) -> &'static str {
        match self {
            Self::English => "english",
            Self::Pinyin => "pinyin",
            Self::Chinese => "chinese",
        }
    }
}

/// Infers the most likely query mode from the query text alone.
///
/// CJK with Latin letters mixed in is treated as pinyin; bare CJK as Chinese;
/// Latin with digits (pinyin tone numbers) as pinyin; everything else English.
/// Untoned Latin (`nihao`) is therefore English: [`resolve_auto`] retries
/// pinyin/chinese when that guess returns nothing, so `banana` stays English.
#[must_use]
pub fn guess_mode(word: &str) -> SearchMode {
    let mut has_cjk = false;
    let mut has_other_alpha = false;
    let mut has_digit = false;
    let mut has_latin = false;
    for c in word.chars() {
        let n = c as u32;
        if c.is_ascii_digit() {
            has_digit = true;
        } else if c.is_ascii_alphabetic() {
            has_latin = true;
        }
        if (0x3400..=0x4dbf).contains(&n) || (0x4e00..=0x9fff).contains(&n) {
            has_cjk = true;
        } else if c.is_alphabetic() {
            has_other_alpha = true;
        }
    }
    if has_cjk {
        if has_other_alpha {
            SearchMode::Pinyin
        } else {
            SearchMode::Chinese
        }
    } else if has_digit && has_latin {
        SearchMode::Pinyin
    } else {
        SearchMode::English
    }
}

/// Resolves the dictionary path.
///
/// Priority when no explicit path is given:
/// 1. `VOCAB_DICTIONARY` environment variable,
/// 2. repo `data/dictionary/dictionary.db` if that file exists,
/// 3. `dictionary.db` inside the app-data directory.
#[must_use]
pub fn dictionary_path(dictionary: Option<&Path>) -> PathBuf {
    if let Some(path) = dictionary {
        return path.to_path_buf();
    }
    if let Ok(db) = std::env::var("VOCAB_DICTIONARY") {
        if !db.is_empty() {
            return PathBuf::from(db);
        }
    }
    let repo = PathBuf::from("data/dictionary/dictionary.db");
    if repo.is_file() {
        return repo;
    }
    if let Ok(dir) = app_data_dir() {
        return dir.join("dictionary.db");
    }
    repo
}

/// Opens the dictionary database and wraps it in a ranked search service.
///
/// If the file is missing, downloads CC-CEDICT / frequency / HSK and ingests
/// it. Existing files refresh when the calendar month changes (best-effort;
/// a failed refresh keeps the current db).
///
/// # Errors
///
/// Returns an error when the database cannot be fetched or opened.
pub fn open_service(dictionary: Option<&Path>) -> Result<SearchService<SqliteDictionary>> {
    let path = dictionary_path(dictionary);
    vocab_dictionary::ensure_dictionary_db(&path).map_err(|err| {
        VocabError::new(format!(
            "dictionary missing at {} and fetch failed: {err}",
            path.display()
        ))
    })?;
    let provider = SqliteDictionary::open(&path).map_err(|err| {
        VocabError::new(format!(
            "{err}\n hint: rebuild dictionary.db or pass --dictionary <path>"
        ))
    })?;
    Ok(SearchService::new(
        provider,
        vocab_search::DeterministicRanker::default(),
    ))
}

/// Runs a query in the given mode and returns the deterministically ranked
/// candidates.
///
/// # Errors
///
/// Forwards provider errors. Pinyin normalization is infallible.
pub fn resolve(
    svc: &SearchService<SqliteDictionary>,
    mode: SearchMode,
    query: &str,
) -> Result<Vec<Candidate>> {
    match mode {
        SearchMode::English => svc.search_english(query),
        SearchMode::Pinyin => {
            let normalized = query
                .parse::<NormalizedPinyin>()
                .map_err(|never| -> VocabError { match never {} })?;
            svc.search_pinyin(&normalized)
        }
        SearchMode::Chinese => svc.lookup_chinese(query),
    }
}

/// Resolves a query by guessing the mode, then falling through English →
/// pinyin → Chinese until one mode returns hits.
///
/// Returns the ranked hits and the mode that produced them. When every mode
/// is empty, the guessed mode is returned alongside an empty vec. Provider
/// errors are skipped so a later mode can still succeed; if every attempt
/// errors, the first error is returned.
///
/// # Errors
///
/// Returns the first provider error when no mode produces a successful
/// (possibly empty) result that can be used, or when every mode errors.
pub fn resolve_auto(
    svc: &SearchService<SqliteDictionary>,
    query: &str,
) -> Result<(Vec<Candidate>, SearchMode)> {
    let guessed = guess_mode(query);
    let mut order = vec![guessed];
    for mode in [SearchMode::English, SearchMode::Pinyin, SearchMode::Chinese] {
        if mode != guessed {
            order.push(mode);
        }
    }
    let mut first_error = None;
    let mut saw_success = false;
    let mut weak_english = None;
    for mode in order {
        match resolve(svc, mode, query) {
            Ok(ranked) if !ranked.is_empty() => {
                if mode == SearchMode::English
                    && looks_like_multisyllable_pinyin(query)
                    && !english_has_lemma(query, &ranked)
                {
                    if weak_english.is_none() {
                        weak_english = Some((ranked, mode));
                    }
                    saw_success = true;
                    continue;
                }
                return Ok((ranked, mode));
            }
            Ok(_) => saw_success = true,
            Err(err) => {
                if first_error.is_none() {
                    first_error = Some(err);
                }
            }
        }
    }
    if let Some(pair) = weak_english {
        return Ok(pair);
    }
    if !saw_success {
        if let Some(err) = first_error {
            return Err(err);
        }
    }
    Ok((Vec::new(), guessed))
}

/// Picker search: [`resolve_auto`] then cap. Label is `auto` when the guess
/// produced hits (or nothing did); otherwise the fallback mode name.
///
/// # Errors
///
/// Forwards [`resolve_auto`].
pub fn search_auto(
    svc: &SearchService<SqliteDictionary>,
    query: &str,
    limit: usize,
) -> Result<(Vec<Candidate>, &'static str)> {
    let guessed = guess_mode(query);
    let (ranked, mode) = resolve_auto(svc, query)?;
    let hits: Vec<Candidate> = ranked.into_iter().take(limit).collect();
    let label = if hits.is_empty() || mode == guessed {
        "auto"
    } else {
        mode.as_label()
    };
    Ok((hits, label))
}

/// Confirmed list item from picker fields.
#[must_use]
pub fn confirmed_item(
    simplified: String,
    traditional: String,
    pinyin: String,
    definition: String,
    source_entry_id: Option<i64>,
    provenance: Provenance,
) -> NewVocabItem {
    NewVocabItem {
        simplified,
        traditional,
        pinyin,
        definition,
        status: ItemStatus::Confirmed,
        notes: None,
        source_entry_id,
        provenance,
        origin_export_id: None,
    }
}

/// Confirmed list item from a dictionary candidate (picker save).
#[must_use]
pub fn item_from_candidate(candidate: &Candidate) -> NewVocabItem {
    confirmed_item(
        candidate.entry.simplified.clone(),
        candidate.entry.traditional.clone(),
        candidate.entry.pinyin.clone(),
        candidate.entry.glosses.join("; "),
        candidate.entry.stable_entry_id,
        candidate.entry.provenance.clone(),
    )
}

/// Persists a picker selection.
///
/// # Errors
///
/// Forwards `user.db` write errors.
pub fn save_candidate(conn: &Connection, candidate: &Candidate) -> Result<SaveOutcome> {
    save_vocab_item(conn, &item_from_candidate(candidate))
}

/// Untoned Latin that fully segments into two or more pinyin syllables
/// (`jingzi`, `nihao`). Single-syllable (`can`) and unsegmentable (`school`)
/// queries stay English so common words are not stolen by pinyin.
fn looks_like_multisyllable_pinyin(query: &str) -> bool {
    let q = query.trim();
    if q.is_empty()
        || !q
            .chars()
            .all(|c| c.is_ascii_alphabetic() || c == '\'' || c.is_ascii_whitespace())
    {
        return false;
    }
    segment(normalize(q).as_str())
        .iter()
        .any(|variant| variant.split_whitespace().count() >= 2)
}

/// Resolves the `user.db` path, defaulting to the platform app-data directory.
#[must_use]
pub fn user_db_path(user_db: Option<&Path>) -> PathBuf {
    if let Some(path) = user_db {
        return path.to_path_buf();
    }
    match app_data_dir() {
        Ok(dir) => dir.join("user.db"),
        Err(_) => PathBuf::from("user.db"),
    }
}

/// Reads the clipboard as trimmed UTF-8 text.
///
/// # Errors
///
/// Returns an error when `pbpaste` is unavailable or its output is not UTF-8;
/// on non-macOS platforms clipboard capture is reported as unsupported.
pub fn clipboard_text() -> Result<String> {
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("pbpaste")
            .output()
            .map_err(|err| VocabError::new(format!("pbpaste failed: {err}")))?;
        Ok(String::from_utf8(output.stdout)
            .map_err(|err| VocabError::new(format!("clipboard is not UTF-8: {err}")))?
            .trim()
            .to_owned())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(VocabError::new(
            "clipboard capture is only supported on macOS",
        ))
    }
}

/// Opens `user.db`, creating its parent directory when needed.
///
/// # Errors
///
/// Returns an error when the directory cannot be created or the database cannot
/// be opened.
pub fn open_capture_db(user_db: Option<&Path>) -> Result<Connection> {
    let path = user_db_path(user_db);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| VocabError::new(format!("cannot create user data dir: {err}")))?;
    }
    open_user_db(&path)
}

/// The result of a quick-capture attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureOutcome {
    /// A new item was written. Inspect `item.status` to tell a clean
    /// `confirmed` save from a `needs_review` fallback.
    Saved(VocabItem),
    /// The `(simplified, traditional)` pair already existed; nothing changed.
    AlreadySaved(VocabItem),
    /// Several strong matches; nothing was written and the caller must choose.
    Ambiguous {
        query: String,
        candidates: Vec<Candidate>,
    },
}

/// Applies the capture decision table against an already-open `user.db`.
///
/// # Errors
///
/// Returns an error for an empty query, a dictionary lookup failure, or a
/// persistence failure.
pub fn capture(
    svc: &SearchService<SqliteDictionary>,
    conn: &Connection,
    word: &str,
    mode_override: Option<SearchMode>,
) -> Result<CaptureOutcome> {
    if word.is_empty() {
        return Err(VocabError::new("empty query, nothing to add"));
    }

    let ranked = match mode_override {
        Some(mode) => resolve(svc, mode, word)?,
        None => resolve_auto(svc, word)?.0,
    };
    let strong: Vec<&Candidate> = ranked
        .iter()
        .filter(|candidate| !candidate.diagnostic.is_inferred)
        .collect();

    if strong.len() == 1 {
        return save_item(conn, &item_from_candidate(strong[0]));
    }

    if ranked.is_empty() {
        let item = NewVocabItem {
            simplified: word.to_owned(),
            traditional: word.to_owned(),
            pinyin: String::new(),
            definition: format!("unresolved query: {word}"),
            status: ItemStatus::NeedsReview,
            notes: None,
            source_entry_id: None,
            provenance: Provenance {
                source: SourceId("user".to_owned()),
                source_version: SourceVersion("manual".to_owned()),
                import_origin: None,
                confirmation: ConfirmationState::NeedsReview,
            },
            origin_export_id: None,
        };
        return save_item(conn, &item);
    }

    Ok(CaptureOutcome::Ambiguous {
        query: word.to_owned(),
        candidates: ranked,
    })
}

/// Convenience wrapper around [`capture`] that resolves and opens `user.db`.
///
/// # Errors
///
/// Returns an error when `user.db` cannot be opened/created, or when
/// [`capture`] fails.
pub fn capture_to_path(
    svc: &SearchService<SqliteDictionary>,
    user_db: Option<&Path>,
    word: &str,
    mode_override: Option<SearchMode>,
) -> Result<CaptureOutcome> {
    let conn = open_capture_db(user_db)?;
    capture(svc, &conn, word, mode_override)
}

fn save_item(conn: &Connection, item: &NewVocabItem) -> Result<CaptureOutcome> {
    match save_vocab_item(conn, item)? {
        SaveOutcome::Inserted(item) => Ok(CaptureOutcome::Saved(item)),
        SaveOutcome::Duplicate(item) => Ok(CaptureOutcome::AlreadySaved(item)),
    }
}

#[cfg(test)]
mod tests {
    use super::looks_like_multisyllable_pinyin;

    #[test]
    fn untoned_multisyllable_pinyin_is_detected() {
        assert!(looks_like_multisyllable_pinyin("jingzi"));
        assert!(looks_like_multisyllable_pinyin("nihao"));
        assert!(looks_like_multisyllable_pinyin("xuexiao"));
        assert!(!looks_like_multisyllable_pinyin("school"));
        assert!(!looks_like_multisyllable_pinyin("hello"));
        assert!(!looks_like_multisyllable_pinyin("can"));
        assert!(!looks_like_multisyllable_pinyin("cat"));
    }
}
