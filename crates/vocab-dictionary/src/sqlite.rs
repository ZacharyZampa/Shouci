use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::Mutex;

use rusqlite::OpenFlags;
use sha2::{Digest, Sha256};
use vocab_core::{
    ConfirmationState, DictionaryEntry, MatchBasis, Provenance, Result, SourceId, SourceVersion,
    VocabError,
};
use vocab_pinyin::{NormalizedPinyin, normalize, segment};

use crate::schema::{
    CREATE_DICTIONARY_SCHEMA, DICTIONARY_BUILD_VERSION, DICTIONARY_SCHEMA_VERSION,
    METADATA_BUILD_VERSION, METADATA_ENTRY_COUNT, METADATA_FREQUENCY_SOURCE_ID,
    METADATA_FREQUENCY_SOURCE_SHA256, METADATA_HSK_SOURCE_ID, METADATA_HSK_SOURCE_SHA256,
    METADATA_LICENSE, METADATA_SCHEMA_VERSION, METADATA_SOURCE_ID, METADATA_SOURCE_SHA256,
    METADATA_SOURCE_VERSION,
};
use crate::{Candidate, CandidateDiagnostic, DictionaryProvider};
use crate::{
    DataSourceDescriptor, IngestSource, LayerKind, RawEntry, VALUE_FREQUENCY_RANK, VALUE_HSK_RANK,
};

const MAX_RETRIEVAL: usize = 2000;
const BIND_CHUNK: usize = 400;

/// Keep every exact-token/phrase gloss id, then fill with prefix/trigram
/// hits up to [`MAX_RETRIEVAL`]. Truncating a mixed set by gloss id was
/// dropping late CEDICT lemmas (e.g. 走 for `go`) before ranking.
fn prefer_exact_gloss_ids(exact: BTreeSet<i64>, extra: BTreeSet<i64>) -> BTreeSet<i64> {
    let mut ids = exact;
    for id in extra {
        if ids.len() >= MAX_RETRIEVAL {
            break;
        }
        ids.insert(id);
    }
    ids
}

/// Deterministic ingestion of one base lexicon artifact into `dictionary.db`.
///
/// The build wipes existing rows, so the same source + version always yields the
/// same database content. Runs inside a single transaction.
///
/// # Errors
///
/// Returns an error if the schema cannot be applied, the source cannot be parsed,
/// or any insert fails; on error nothing is committed.
pub fn build_dictionary_db(
    conn: &mut rusqlite::Connection,
    source: &dyn IngestSource,
    artifact: &[u8],
) -> Result<BuildStats> {
    build_dictionary_db_with_layers(conn, source, artifact, &[])
}

/// Like [`build_dictionary_db`], then applies frequency / HSK enrichment layers
/// by simplified headword. Unmatched entries keep NULL ranks (ranker treats
/// those as last).
///
/// # Errors
///
/// Same as [`build_dictionary_db`], plus parse/apply failures on a layer.
pub fn build_dictionary_db_with_layers(
    conn: &mut rusqlite::Connection,
    source: &dyn IngestSource,
    artifact: &[u8],
    layers: &[(&dyn IngestSource, &[u8])],
) -> Result<BuildStats> {
    let tx = conn
        .transaction()
        .map_err(|err| VocabError::new(format!("failed to begin build: {err}")))?;

    tx.execute_batch(WIPE)
        .map_err(|err| VocabError::new(format!("failed to wipe previous build: {err}")))?;
    tx.execute_batch(CREATE_DICTIONARY_SCHEMA)
        .map_err(|err| VocabError::new(format!("failed to apply schema: {err}")))?;

    let entries = source.parse(artifact)?;
    let descriptor = source.descriptor();
    let (entry_count, gloss_count) = insert_lexicon(&tx, descriptor, &entries)?;

    let metadata = [
        (
            METADATA_SCHEMA_VERSION,
            DICTIONARY_SCHEMA_VERSION.to_string(),
        ),
        (METADATA_SOURCE_ID, descriptor.id.as_str().to_owned()),
        (
            METADATA_SOURCE_VERSION,
            descriptor.version.as_str().to_owned(),
        ),
        (METADATA_LICENSE, descriptor.license.clone()),
        (METADATA_BUILD_VERSION, DICTIONARY_BUILD_VERSION.to_owned()),
        (METADATA_ENTRY_COUNT, entry_count.to_string()),
        (METADATA_SOURCE_SHA256, sha256_hex(artifact)),
    ];
    for (key, value) in metadata {
        tx.execute(
            "INSERT INTO dictionary_metadata (key, value) VALUES (?1, ?2)",
            rusqlite::params![key, value],
        )
        .map_err(|err| VocabError::new(format!("write metadata {key}: {err}")))?;
    }

    let (frequency_updated, hsk_updated) = apply_enrichment_layers(&tx, layers)?;

    tx.commit()
        .map_err(|err| VocabError::new(format!("failed to commit build: {err}")))?;

    Ok(BuildStats {
        entries: entry_count,
        glosses: gloss_count,
        frequency_updated,
        hsk_updated,
    })
}

const WIPE: &str = r"
DROP TABLE IF EXISTS english_fts_token;
DROP TABLE IF EXISTS english_fts_trigram;
DROP TABLE IF EXISTS dictionary_glosses;
DROP TABLE IF EXISTS dictionary_entries;
DROP TABLE IF EXISTS dictionary_metadata;
";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildStats {
    pub entries: u64,
    pub glosses: u64,
    pub frequency_updated: u64,
    pub hsk_updated: u64,
}

fn insert_lexicon(
    tx: &rusqlite::Transaction<'_>,
    descriptor: &DataSourceDescriptor,
    entries: &[RawEntry],
) -> Result<(u64, u64)> {
    let mut ins_entry = tx
        .prepare(
            "INSERT OR IGNORE INTO dictionary_entries \
             (simplified, traditional, pinyin, pinyin_normalized, source_id, source_version) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .map_err(|err| VocabError::new(format!("prepare entry insert: {err}")))?;
    let mut ins_gloss = tx
        .prepare("INSERT INTO dictionary_glosses (entry_id, position, gloss) VALUES (?1, ?2, ?3)")
        .map_err(|err| VocabError::new(format!("prepare gloss insert: {err}")))?;
    let mut ins_fts_token = tx
        .prepare("INSERT INTO english_fts_token (rowid, gloss) VALUES (?1, ?2)")
        .map_err(|err| VocabError::new(format!("prepare token fts insert: {err}")))?;
    let mut ins_fts_trigram = tx
        .prepare("INSERT INTO english_fts_trigram (rowid, gloss) VALUES (?1, ?2)")
        .map_err(|err| VocabError::new(format!("prepare trigram fts insert: {err}")))?;

    let mut entry_count = 0u64;
    let mut gloss_count = 0u64;
    for entry in entries {
        let pinyin = normalize(&entry.pinyin);
        let inserted = ins_entry
            .execute(rusqlite::params![
                entry.simplified,
                entry.traditional,
                entry.pinyin,
                pinyin.as_str(),
                descriptor.id.as_str(),
                descriptor.version.as_str(),
            ])
            .map_err(|err| VocabError::new(format!("insert entry: {err}")))?;
        if inserted == 0 {
            continue;
        }
        entry_count += 1;
        let entry_id = tx.last_insert_rowid();
        for (position, gloss) in entry.glosses.iter().enumerate() {
            let position = i64::try_from(position)
                .map_err(|_| VocabError::new("gloss position overflow"))?
                + 1;
            let gloss_id = ins_gloss
                .execute(rusqlite::params![entry_id, position, gloss])
                .map_err(|err| VocabError::new(format!("insert gloss: {err}")))?;
            if gloss_id == 0 {
                continue;
            }
            gloss_count += 1;
            let gloss_id = tx.last_insert_rowid();
            ins_fts_token
                .execute(rusqlite::params![gloss_id, gloss])
                .map_err(|err| VocabError::new(format!("index token: {err}")))?;
            ins_fts_trigram
                .execute(rusqlite::params![gloss_id, gloss])
                .map_err(|err| VocabError::new(format!("index trigram: {err}")))?;
        }
    }
    Ok((entry_count, gloss_count))
}

fn apply_enrichment_layers(
    tx: &rusqlite::Transaction<'_>,
    layers: &[(&dyn IngestSource, &[u8])],
) -> Result<(u64, u64)> {
    let mut frequency_updated = 0u64;
    let mut hsk_updated = 0u64;
    for (layer, layer_artifact) in layers {
        match layer.descriptor().layer {
            LayerKind::Frequency => {
                frequency_updated = apply_rank_layer(
                    tx,
                    *layer,
                    layer_artifact,
                    "frequency_rank",
                    VALUE_FREQUENCY_RANK,
                    METADATA_FREQUENCY_SOURCE_ID,
                    METADATA_FREQUENCY_SOURCE_SHA256,
                )?;
            }
            LayerKind::HskRanks => {
                hsk_updated = apply_rank_layer(
                    tx,
                    *layer,
                    layer_artifact,
                    "hsk_rank",
                    VALUE_HSK_RANK,
                    METADATA_HSK_SOURCE_ID,
                    METADATA_HSK_SOURCE_SHA256,
                )?;
            }
            LayerKind::BaseLexicon => {
                return Err(VocabError::new(format!(
                    "enrichment layer {} is not a frequency/HSK source",
                    layer.descriptor().id.as_str()
                )));
            }
        }
    }
    Ok((frequency_updated, hsk_updated))
}

fn apply_rank_layer(
    tx: &rusqlite::Transaction<'_>,
    source: &dyn IngestSource,
    artifact: &[u8],
    column: &str,
    value_key: &str,
    meta_id_key: &str,
    meta_sha_key: &str,
) -> Result<u64> {
    let entries = source.parse(artifact)?;
    tx.execute("DROP TABLE IF EXISTS enrichment_ranks", [])
        .map_err(|err| VocabError::new(format!("drop enrichment temp: {err}")))?;
    tx.execute(
        "CREATE TEMP TABLE enrichment_ranks (
            simplified TEXT PRIMARY KEY,
            rank INTEGER NOT NULL
        )",
        [],
    )
    .map_err(|err| VocabError::new(format!("create enrichment temp: {err}")))?;
    {
        let mut ins = tx
            .prepare(
                "INSERT INTO enrichment_ranks (simplified, rank) VALUES (?1, ?2)
                 ON CONFLICT(simplified) DO UPDATE SET rank = MIN(rank, excluded.rank)",
            )
            .map_err(|err| VocabError::new(format!("prepare enrichment insert: {err}")))?;
        for entry in &entries {
            let Some(raw) = entry.values.get(value_key) else {
                return Err(VocabError::new(format!(
                    "enrichment entry {} is missing {value_key}",
                    entry.simplified
                )));
            };
            let rank: i64 = raw.parse().map_err(|_| {
                VocabError::new(format!(
                    "enrichment entry {} has a non-integer {value_key}",
                    entry.simplified
                ))
            })?;
            ins.execute(rusqlite::params![entry.simplified, rank])
                .map_err(|err| VocabError::new(format!("insert enrichment rank: {err}")))?;
        }
    }
    let sql = format!(
        "UPDATE dictionary_entries SET {column} = (
            SELECT rank FROM enrichment_ranks
            WHERE enrichment_ranks.simplified = dictionary_entries.simplified
         ) WHERE simplified IN (SELECT simplified FROM enrichment_ranks)"
    );
    let updated = tx
        .execute(&sql, [])
        .map_err(|err| VocabError::new(format!("apply {column}: {err}")))?;
    let descriptor = source.descriptor();
    tx.execute(
        "INSERT INTO dictionary_metadata (key, value) VALUES (?1, ?2)",
        rusqlite::params![meta_id_key, descriptor.id.as_str()],
    )
    .map_err(|err| VocabError::new(format!("write metadata {meta_id_key}: {err}")))?;
    tx.execute(
        "INSERT INTO dictionary_metadata (key, value) VALUES (?1, ?2)",
        rusqlite::params![meta_sha_key, sha256_hex(artifact)],
    )
    .map_err(|err| VocabError::new(format!("write metadata {meta_sha_key}: {err}")))?;
    u64::try_from(updated).map_err(|_| VocabError::new("enrichment update count overflow"))
}

/// Read-only provider over a built `dictionary.db`.
///
/// Retrieval returns *all viable candidates* for a query; ordering is the
/// `vocab-search` ranker's job. Caps apply only to raw retrieval, never to
/// ranking decisions.
pub struct SqliteDictionary {
    conn: Mutex<rusqlite::Connection>,
    schema_version: String,
}

impl SqliteDictionary {
    /// Opens a built `dictionary.db` read-only.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be opened or its metadata cannot be read.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = rusqlite::Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|err| VocabError::new(format!("cannot open {}: {err}", path.display())))?;
        Self::from_connection(conn)
    }

    /// Wraps an existing connection as a provider.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection's metadata cannot be read.
    pub fn from_connection(conn: rusqlite::Connection) -> Result<Self> {
        let schema_version =
            metadata_value(&conn, METADATA_SCHEMA_VERSION)?.unwrap_or_else(|| "unknown".to_owned());
        Ok(Self {
            conn: Mutex::new(conn),
            schema_version,
        })
    }

    /// Returns every `(key, value)` metadata row, ordered by key.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails or the connection is poisoned.
    pub fn metadata(&self) -> Result<Vec<(String, String)>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| VocabError::new("dictionary connection poisoned"))?;
        let mut stmt = conn
            .prepare("SELECT key, value FROM dictionary_metadata ORDER BY key")
            .map_err(|err| VocabError::new(format!("read metadata: {err}")))?;
        let mut rows = stmt
            .query([])
            .map_err(|err| VocabError::new(format!("read metadata rows: {err}")))?;
        let mut out = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|err| VocabError::new(format!("read metadata: {err}")))?
        {
            out.push((row.get(0)?, row.get(1)?));
        }
        Ok(out)
    }

    fn select_by_gloss_ids(
        &self,
        gloss_ids: &BTreeSet<i64>,
        basis: MatchBasis,
    ) -> Result<Vec<Candidate>> {
        if gloss_ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self
            .conn
            .lock()
            .map_err(|_| VocabError::new("dictionary connection poisoned"))?;
        let ids: Vec<i64> = gloss_ids.iter().copied().collect();
        let join = " JOIN dictionary_glosses g ON e.entry_id = g.entry_id";
        let mut candidates = Vec::new();
        let mut seen = BTreeSet::new();
        for chunk in ids.chunks(BIND_CHUNK) {
            let placeholders = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
            let clause = format!("g.gloss_id IN ({placeholders})");
            let params: Vec<rusqlite::types::Value> = chunk
                .iter()
                .copied()
                .map(rusqlite::types::Value::Integer)
                .collect();
            let batch = select_entries(
                &conn,
                join,
                &[(clause, params)],
                ClauseJoin::Or,
                basis,
                false,
            )?;
            for candidate in batch {
                if let Some(id) = candidate.entry.stable_entry_id {
                    if seen.insert(id) {
                        candidates.push(candidate);
                    }
                }
            }
        }
        if candidates.len() > MAX_RETRIEVAL {
            candidates.truncate(MAX_RETRIEVAL);
        }
        attach_glosses(&conn, &mut candidates)?;
        Ok(candidates)
    }

    fn select_entries_where(
        &self,
        clauses: &[(String, Vec<rusqlite::types::Value>)],
        basis: MatchBasis,
        is_inferred: bool,
        join_with: ClauseJoin,
    ) -> Result<Vec<Candidate>> {
        if clauses.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self
            .conn
            .lock()
            .map_err(|_| VocabError::new("dictionary connection poisoned"))?;
        let mut candidates = select_entries(&conn, "", clauses, join_with, basis, is_inferred)?;
        attach_glosses(&conn, &mut candidates)?;
        Ok(candidates)
    }
}

impl DictionaryProvider for SqliteDictionary {
    fn search_english(&self, query: &str) -> Result<Vec<Candidate>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let lowercase = query.to_lowercase();
        let mut exact_ids = BTreeSet::new();
        let mut extra_ids = BTreeSet::new();
        {
            let conn = self
                .conn
                .lock()
                .map_err(|_| VocabError::new("dictionary connection poisoned"))?;
            let tokens = tokens(&lowercase);
            if !tokens.is_empty() {
                let any = tokens
                    .iter()
                    .map(|token| fts_quote(token))
                    .collect::<Vec<_>>()
                    .join(" OR ");
                let prefix = tokens
                    .iter()
                    .map(|t| fts_term_prefix(t))
                    .collect::<Vec<_>>()
                    .join(" OR ");
                fts_match_ids(&conn, "english_fts_token", &any, &mut exact_ids)?;
                fts_match_ids(&conn, "english_fts_token", &prefix, &mut extra_ids)?;
            }
            if lowercase.chars().count() >= 3 {
                fts_match_ids(
                    &conn,
                    "english_fts_token",
                    &fts_phrase(&lowercase),
                    &mut exact_ids,
                )?;
                fts_match_ids(
                    &conn,
                    "english_fts_trigram",
                    &fts_phrase(&lowercase),
                    &mut extra_ids,
                )?;
            }
        }
        self.select_by_gloss_ids(
            &prefer_exact_gloss_ids(exact_ids, extra_ids),
            MatchBasis::EnglishGloss,
        )
    }

    fn search_pinyin(&self, pinyin: &NormalizedPinyin) -> Result<Vec<Candidate>> {
        let q = pinyin.as_str();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let tokens: Vec<&str> = q.split_whitespace().collect();
        let mut clauses: Vec<(String, Vec<rusqlite::types::Value>)> = Vec::new();
        clauses.push((
            "pinyin_normalized = ?".to_owned(),
            vec![q.to_owned().into()],
        ));

        let mut crossable: Vec<Vec<String>> = Vec::new();
        let mut some_segmented = false;
        for token in tokens {
            let forms = segment(token);
            if forms.is_empty() {
                continue;
            }
            some_segmented = true;
            crossable.push(forms);
        }

        if some_segmented {
            for variant in cross_join_forms(crossable) {
                clauses.push((
                    "lower(pinyin_normalized) GLOB ?".to_owned(),
                    vec![syllable_pattern(&variant).into()],
                ));
            }
        } else {
            // Nothing segmented (single letters, numbers, unknown input). Match
            // only by prefix; the old `%last%` contains clause is what flooded
            // e.g. `nihao` with every entry containing "hao".
            let prefix = format!("{}%", escape_like(q));
            clauses.push((
                "pinyin_normalized LIKE ? ESCAPE '\\'".to_owned(),
                vec![prefix.into()],
            ));
        }
        self.select_entries_where(&clauses, MatchBasis::Pinyin, false, ClauseJoin::Or)
    }

    fn lookup_chinese(&self, text: &str) -> Result<Vec<Candidate>> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(Vec::new());
        }
        let escaped = escape_like(text);
        let mut clauses: Vec<(String, Vec<rusqlite::types::Value>)> = Vec::new();
        clauses.push(("simplified = ?".to_owned(), vec![text.to_owned().into()]));
        clauses.push(("traditional = ?".to_owned(), vec![text.to_owned().into()]));
        let prefix = format!("{escaped}%");
        clauses.push((
            "simplified LIKE ? ESCAPE '\\'".to_owned(),
            vec![prefix.clone().into()],
        ));
        clauses.push((
            "traditional LIKE ? ESCAPE '\\'".to_owned(),
            vec![prefix.into()],
        ));

        let exact_and_prefix =
            self.select_entries_where(&clauses, MatchBasis::Simplified, false, ClauseJoin::Or)?;
        let mut merged: Vec<Candidate> = exact_and_prefix;
        let mut known: BTreeSet<i64> = merged
            .iter()
            .filter_map(|c| c.entry.stable_entry_id)
            .collect();

        let mut char_clauses: Vec<(String, Vec<rusqlite::types::Value>)> = Vec::new();
        for ch in text.chars() {
            let contains = format!("%{}%", escape_like(&ch.to_string()));
            char_clauses.push((
                "simplified LIKE ? ESCAPE '\\'".to_owned(),
                vec![contains.into()],
            ));
        }
        for cand in self.select_entries_where(
            &char_clauses,
            MatchBasis::CharacterFallback,
            true,
            ClauseJoin::And,
        )? {
            if let Some(id) = cand.entry.stable_entry_id {
                if known.insert(id) {
                    merged.push(cand);
                }
            }
        }
        Ok(merged)
    }

    fn schema_version(&self) -> &str {
        &self.schema_version
    }
}

fn select_entries(
    conn: &rusqlite::Connection,
    join: &str,
    clauses: &[(String, Vec<rusqlite::types::Value>)],
    join_with: ClauseJoin,
    basis: MatchBasis,
    is_inferred: bool,
) -> Result<Vec<Candidate>> {
    let mut sql = String::from(
        "SELECT DISTINCT e.entry_id, e.simplified, e.traditional, e.pinyin, e.frequency_rank, \
         e.hsk_rank, e.source_id, e.source_version \
         FROM dictionary_entries e",
    );
    sql.push_str(join);
    sql.push_str(" WHERE ");
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    for (index, (clause, values)) in clauses.iter().enumerate() {
        if index > 0 {
            sql.push_str(join_with.sql());
        }
        sql.push('(');
        sql.push_str(clause);
        sql.push(')');
        for value in values {
            params.push(value.clone());
        }
    }
    sql.push_str(" ORDER BY e.entry_id");

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|err| VocabError::new(format!("prepare query: {err}: {sql}")))?;
    let mut rows = stmt
        .query(rusqlite::params_from_iter(params.iter()))
        .map_err(|err| VocabError::new(format!("run query: {err}")))?;

    let mut candidates = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| VocabError::new(format!("iterate query: {err}")))?
    {
        candidates.push(candidate_from_row(row, basis, is_inferred)?);
    }
    Ok(candidates)
}

fn candidate_from_row(
    row: &rusqlite::Row<'_>,
    basis: MatchBasis,
    is_inferred: bool,
) -> Result<Candidate> {
    let entry_id: i64 = row.get(0)?;
    let simplified: String = row.get(1)?;
    let traditional: String = row.get(2)?;
    let pinyin: String = row.get(3)?;
    let frequency_rank: Option<i64> = row.get(4)?;
    let hsk_rank: Option<i64> = row.get(5)?;
    let source_id: String = row.get(6)?;
    let source_version: String = row.get(7)?;
    Ok(Candidate {
        entry: DictionaryEntry {
            simplified,
            traditional,
            pinyin,
            glosses: Vec::new(),
            frequency_rank: frequency_rank.and_then(|v| u64::try_from(v).ok()),
            hsk_rank: hsk_rank.and_then(|v| u64::try_from(v).ok()),
            stable_entry_id: Some(entry_id),
            provenance: Provenance {
                source: SourceId(source_id),
                source_version: SourceVersion(source_version),
                import_origin: None,
                confirmation: ConfirmationState::DictionaryAuthority,
            },
        },
        diagnostic: CandidateDiagnostic { basis, is_inferred },
    })
}

fn attach_glosses(conn: &rusqlite::Connection, candidates: &mut [Candidate]) -> Result<()> {
    if candidates.is_empty() {
        return Ok(());
    }
    let ids: Vec<i64> = candidates
        .iter()
        .filter_map(|c| c.entry.stable_entry_id)
        .collect();
    let index: HashMap<i64, usize> = candidates
        .iter()
        .enumerate()
        .filter_map(|(position, c)| c.entry.stable_entry_id.map(|id| (id, position)))
        .collect();
    for chunk in ids.chunks(BIND_CHUNK) {
        let placeholders = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
        let sql = format!(
            "SELECT entry_id, gloss FROM dictionary_glosses \
             WHERE entry_id IN ({placeholders}) ORDER BY entry_id, position"
        );
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|err| VocabError::new(format!("prepare gloss fetch: {err}")))?;
        let mut rows = stmt
            .query(rusqlite::params_from_iter(chunk.iter()))
            .map_err(|err| VocabError::new(format!("run gloss fetch: {err}")))?;
        while let Some(row) = rows
            .next()
            .map_err(|err| VocabError::new(format!("iterate gloss fetch: {err}")))?
        {
            let entry_id: i64 = row.get(0)?;
            let gloss: String = row.get(1)?;
            if let Some(&position) = index.get(&entry_id) {
                candidates[position].entry.glosses.push(gloss);
            }
        }
    }
    Ok(())
}

fn fts_match_ids(
    conn: &rusqlite::Connection,
    table: &str,
    fts_query: &str,
    ids: &mut BTreeSet<i64>,
) -> Result<()> {
    let sql = format!("SELECT rowid FROM {table} WHERE {table} MATCH ?1");
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|err| VocabError::new(format!("prepare fts query on {table}: {err}")))?;
    let mut rows = stmt
        .query(rusqlite::params![fts_query])
        .map_err(|err| VocabError::new(format!("run fts query on {table}: {err}")))?;
    while let Some(row) = rows.next().map_err(|err| {
        VocabError::new(format!(
            "iterate fts query on {table} ({fts_query:?}): {err}"
        ))
    })? {
        ids.insert(row.get(0)?);
    }
    Ok(())
}

fn metadata_value(conn: &rusqlite::Connection, key: &str) -> Result<Option<String>> {
    let mut stmt = conn
        .prepare("SELECT value FROM dictionary_metadata WHERE key = ?1")
        .map_err(|err| VocabError::new(format!("read metadata {key}: {err}")))?;
    let mut rows = stmt
        .query(rusqlite::params![key])
        .map_err(|err| VocabError::new(format!("read metadata {key}: {err}")))?;
    if let Some(row) = rows
        .next()
        .map_err(|err| VocabError::new(format!("read metadata {key}: {err}")))?
    {
        Ok(Some(row.get(0)?))
    } else {
        Ok(None)
    }
}

fn tokens(s: &str) -> Vec<String> {
    s.split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn fts_quote(token: &str) -> String {
    format!("\"{}\"", token.replace('"', "\"\""))
}

fn fts_term_prefix(token: &str) -> String {
    format!("{}*", fts_quote(token))
}

fn fts_phrase(query: &str) -> String {
    fts_quote(query)
}

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Cartesian product of per-token segmentation forms, capped to bound the
/// clause count for heavily ambiguous input (e.g. `xian` -> `xi an` + `xian`).
const MAX_PINYIN_PATTERNS: usize = 128;

fn cross_join_forms(groups: Vec<Vec<String>>) -> Vec<String> {
    let mut acc: Vec<String> = vec![String::new()];
    for forms in groups {
        let mut next = Vec::with_capacity(acc.len() * forms.len());
        for prefix in acc {
            for form in &forms {
                if prefix.is_empty() {
                    next.push(form.clone());
                } else {
                    next.push(format!("{prefix} {form}"));
                }
            }
        }
        acc = next;
        if acc.len() > MAX_PINYIN_PATTERNS {
            acc.truncate(MAX_PINYIN_PATTERNS);
            break;
        }
    }
    acc
}

/// GLOB pattern that requires adjacent syllables in query order while letting
/// each syllable carry its tone digit (`ni hao` -> `ni[0-5] hao[0-5]*`). All
/// dictionary pinyin syllables end in a 1-5 tone digit, so a bare letter token
/// means a phonetic letter (A, B, ...) rather than a real syllable.
fn syllable_pattern(variant: &str) -> String {
    let mut pattern = String::new();
    for (index, syllable) in variant.split(' ').enumerate() {
        if index > 0 {
            pattern.push(' ');
        }
        let ends_in_tone = syllable
            .as_bytes()
            .last()
            .is_some_and(|b| matches!(b, b'1'..=b'5'));
        pattern.push_str(syllable);
        if !ends_in_tone {
            pattern.push_str("[0-5]");
        }
    }
    pattern.push('*');
    pattern
}

#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(out, "{byte:02x}").expect("writing to a String is infallible");
    }
    out
}

/// How multiple evidence clauses are combined into one retrieval query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClauseJoin {
    And,
    Or,
}

impl ClauseJoin {
    fn sql(self) -> &'static str {
        match self {
            Self::And => " AND ",
            Self::Or => " OR ",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::escape_like;

    #[test]
    fn escapes_like_wildcards() {
        assert_eq!(escape_like("100%_done"), "100\\%\\_done");
    }
}
