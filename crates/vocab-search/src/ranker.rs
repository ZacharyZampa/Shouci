//! Orders retrieved candidates. Ranking never removes one.
//!
//! Each query kind has its own key, whose fields compare in declaration
//! order: the first field that differs decides. `Reverse` marks a field
//! where true, or more, comes first. Every key ends in [`TailKey`]
//! (dictionary priority, frequency, HSK level, entry id), so equal matches
//! still get one fixed order. DEV.md ("Search path") has the rules in prose.

use std::cmp::Reverse;
use std::collections::BTreeSet;

use vocab_core::MatchBasis;
use vocab_core::SourceId;
use vocab_dictionary::{Candidate, english_terms, is_cross_reference};
use vocab_pinyin::{NormalizedPinyin, normalize, segment};

/// The frequency rank under which a word counts as everyday.
const COMMON_WORD_RANK: u64 = 10_000;

/// The last tiebreaks, shared by every query kind.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct TailKey {
    source_index: usize,
    frequency: u64,
    hsk: u64,
    entry_id: i64,
}

/// How an entry's glosses match an English query.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct EnglishKey {
    /// A sense is the query (`travel` in `to travel`), and the word is one
    /// people use.
    known_sense: Reverse<bool>,
    /// A sense starts with the query: `trav` in `to travel`.
    sense_prefix: Reverse<bool>,
    /// A sense starts with the query as a word: `cat` in `cat (CL:隻|只[zhi1])`.
    sense_word_prefix: Reverse<bool>,
    /// The glosses contain the query as typed.
    phrase: Reverse<bool>,
    /// Every word of the query is a word of the glosses.
    every_word: Reverse<bool>,
    /// Every word of the query starts a word of the glosses (`trav`).
    every_word_prefix: Reverse<bool>,
    /// How many of the query's words the glosses have, 0–100.
    percent_words: Reverse<usize>,
    /// Which sense the query is (0 for the first), plus [`rarity`].
    standing: usize,
    tail: TailKey,
}

/// How an entry's reading matches a pinyin query.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct PinyinKey {
    /// The reading is the query, in one of its segmentations.
    exact: Reverse<bool>,
    /// Too few syllables to spell the whole query (好 for `nihao`).
    undersized: bool,
    /// The longest run of the query's syllables, in order.
    aligned_run: Reverse<usize>,
    /// How many of the query's syllables the reading has, 0–100.
    percent_syllables: Reverse<usize>,
    /// Fewer syllables first.
    syllables: usize,
    tail: TailKey,
}

/// How an entry matches Chinese characters.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ChineseKey {
    /// The word starts with the query (or is it).
    prefix: Reverse<bool>,
    /// Inferred from the query's characters, not found as a word.
    inferred: bool,
    /// Has every character of the query: before the words inside it.
    every_character: Reverse<bool>,
    /// For a word inside the query (蚌埠 in 蚌埠住了): where it starts, then
    /// the longer word first. `None`, for any other match, comes first.
    inside_query: Option<(usize, Reverse<usize>)>,
    exact_simplified: Reverse<bool>,
    exact_traditional: Reverse<bool>,
    tail: TailKey,
}

#[must_use]
pub(crate) trait Ranker {
    fn rank_english(&self, query: &str, candidates: Vec<Candidate>) -> Vec<Candidate>;
    fn rank_pinyin(&self, query: &NormalizedPinyin, candidates: Vec<Candidate>) -> Vec<Candidate>;
    fn rank_chinese(&self, query: &str, candidates: Vec<Candidate>) -> Vec<Candidate>;
}

/// Implements the HLD ordering chain. Ranking never removes candidates; the tail
/// tiebreaks (source priority, frequency, HSK, entry id) guarantee determinism.
pub struct DeterministicRanker {
    source_priority: Vec<SourceId>,
}

impl Default for DeterministicRanker {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl DeterministicRanker {
    #[must_use]
    pub fn new(source_priority: Vec<SourceId>) -> Self {
        Self { source_priority }
    }

    fn tail(&self, cand: &Candidate) -> TailKey {
        TailKey {
            source_index: self.source_index(cand),
            frequency: cand.entry.frequency_rank.unwrap_or(u64::MAX),
            hsk: cand.entry.hsk_rank.unwrap_or(u64::MAX),
            entry_id: cand.entry.entry_id,
        }
    }

    fn source_index(&self, cand: &Candidate) -> usize {
        self.source_priority
            .iter()
            .position(|s| s == &cand.entry.source)
            .unwrap_or(usize::MAX)
    }

    fn english_key(&self, query: &str, cand: &Candidate) -> EnglishKey {
        let gloss = cand.entry.glosses.join(" ").to_lowercase();
        let query = query.trim().to_lowercase();
        let has_query = !query.is_empty();
        let terms = english_terms(&query);
        let gloss_tokens = tokens(&gloss);
        let senses = senses(cand);

        let words_matched = terms
            .iter()
            .filter(|term| gloss_tokens.iter().any(|g| g == *term))
            .count();
        let words_prefixed = terms
            .iter()
            .filter(|term| gloss_tokens.iter().any(|g| g.starts_with(term.as_str())))
            .count();
        let is_sense = senses.iter().any(|sense| is_exact_sense(sense, &query));
        // Senses come most important first: among words that all have the
        // query as a sense, the one whose first sense it is leads. A rare
        // word gives up places, so it does not lead a common word for
        // having fewer senses: 暍 is only `hot`, but 热 is the word.
        let sense_position = senses
            .iter()
            .position(|sense| is_exact_sense(sense, &query))
            .unwrap_or(0);
        // A word nobody uses does not lead for having the query as a sense:
        // 屯驻 is `to quarter` (troops), and 刻 is the quarter hour.
        let known = cand.entry.frequency_rank.is_some() || cand.entry.hsk_rank.is_some();

        EnglishKey {
            known_sense: Reverse(known && is_sense),
            sense_prefix: Reverse(has_query && readings(&senses).any(|r| r.starts_with(&query))),
            sense_word_prefix: Reverse(
                has_query && readings(&senses).any(|r| starts_as_word(r, &query)),
            ),
            phrase: Reverse(has_query && gloss.contains(&query)),
            // Every word of the query, then every word at least as a prefix
            // (`trav` in `travel`); a gloss sharing only some words ranks by
            // how many.
            every_word: Reverse(!terms.is_empty() && words_matched == terms.len()),
            every_word_prefix: Reverse(!terms.is_empty() && words_prefixed == terms.len()),
            percent_words: Reverse(percent(words_matched, terms.len())),
            standing: sense_position + rarity(cand),
            tail: self.tail(cand),
        }
    }

    fn pinyin_key(
        &self,
        query_variants: &[Vec<String>],
        min_span: usize,
        cand: &Candidate,
    ) -> PinyinKey {
        let cand_norm = normalize(&cand.entry.pinyin);
        let cand_tokens = tokens(cand_norm.as_str());
        let exact = |variant: &Vec<String>| variant.join(" ") == cand_norm.as_str();
        let aligned_run = query_variants
            .iter()
            .map(|query| longest_aligned_run(query, &cand_tokens))
            .max()
            .unwrap_or(0);

        PinyinKey {
            exact: Reverse(query_variants.iter().any(exact)),
            undersized: cand_tokens.len() < min_span,
            aligned_run: Reverse(aligned_run),
            percent_syllables: Reverse(best_syllable_coverage(query_variants, &cand_tokens)),
            syllables: cand_tokens.len(),
            tail: self.tail(cand),
        }
    }

    fn chinese_key(&self, query: &str, cand: &Candidate) -> ChineseKey {
        let query = query.trim();
        let has_query = !query.is_empty();
        let entry = &cand.entry;
        // Below every real match: entries with all of the query's characters,
        // then the words found inside it in reading order.
        let inside_query = (cand.diagnostic.basis == MatchBasis::ContainedWord).then(|| {
            let starts_at = query
                .find(entry.simplified.as_str())
                .or_else(|| query.find(entry.traditional.as_str()))
                .map_or(usize::MAX, |byte| query[..byte].chars().count());
            (starts_at, Reverse(entry.simplified.chars().count()))
        });
        ChineseKey {
            prefix: Reverse(has_query && entry.simplified.starts_with(query)),
            inferred: cand.diagnostic.is_inferred,
            every_character: Reverse(cand.diagnostic.basis == MatchBasis::CharacterFallback),
            inside_query,
            exact_simplified: Reverse(has_query && entry.simplified == query),
            exact_traditional: Reverse(has_query && entry.traditional == query),
            tail: self.tail(cand),
        }
    }
}

/// An entry's senses, lowercased. One gloss can hold several: `to visit;
/// to call on`. A pointer to another entry (`see 大夫[dai4 fu5]`) is not one.
fn senses(cand: &Candidate) -> Vec<String> {
    cand.entry
        .glosses
        .iter()
        .flat_map(|g| g.split(';'))
        .map(|g| sense_core(&g.to_lowercase()))
        .filter(|sense| !sense.is_empty() && !is_cross_reference(sense))
        .collect()
}

/// Each sense as written, and a verb's sense without its `to `: `to travel`
/// starts with `travel` too.
fn readings(senses: &[String]) -> impl Iterator<Item = &str> {
    senses
        .iter()
        .flat_map(|sense| [Some(sense.as_str()), sense.strip_prefix("to ")])
        .flatten()
}

/// True when `reading` is `query` or starts with it as a whole word.
fn starts_as_word(reading: &str, query: &str) -> bool {
    reading
        .strip_prefix(query)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', ';', '(']))
}

/// Places a word gives up for being uncommon: none for an everyday word, a
/// few for one only learners are taught, most for one in no list at all
/// (rare in modern Chinese).
fn rarity(cand: &Candidate) -> usize {
    match (cand.entry.frequency_rank, cand.entry.hsk_rank) {
        (Some(rank), _) if rank <= COMMON_WORD_RANK => 0,
        (_, Some(_)) => 1,
        (Some(_), None) => 3,
        (None, None) => 5,
    }
}

/// `part` as a percentage of `whole`; 0 when there is nothing to count.
fn percent(part: usize, whole: usize) -> usize {
    (part * 100).checked_div(whole).unwrap_or(0)
}

/// Sorts by `key`, computing it once per candidate. The sort is stable, so
/// equal keys keep retrieval order.
fn sort_by_key<K: Ord>(
    candidates: Vec<Candidate>,
    key: impl Fn(&Candidate) -> K,
) -> Vec<Candidate> {
    let mut keyed: Vec<(K, Candidate)> = candidates
        .into_iter()
        .map(|candidate| (key(&candidate), candidate))
        .collect();
    keyed.sort_by(|(a, _), (b, _)| a.cmp(b));
    keyed.into_iter().map(|(_, candidate)| candidate).collect()
}

impl Ranker for DeterministicRanker {
    fn rank_english(&self, query: &str, candidates: Vec<Candidate>) -> Vec<Candidate> {
        sort_by_key(candidates, |c| self.english_key(query, c))
    }

    fn rank_pinyin(&self, query: &NormalizedPinyin, candidates: Vec<Candidate>) -> Vec<Candidate> {
        let query_variants = ranking_variants(query);
        let min_span = segment_min_span(query);
        sort_by_key(candidates, |c| {
            self.pinyin_key(&query_variants, min_span, c)
        })
    }

    fn rank_chinese(&self, query: &str, candidates: Vec<Candidate>) -> Vec<Candidate> {
        sort_by_key(candidates, |c| self.chinese_key(query, c))
    }
}

fn tokens(s: &str) -> Vec<String> {
    s.split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// True when any candidate gloss is an English lemma for `query`
/// (`cat`, `cat (CL:…)`, `to go`), not merely a containing phrase.
#[must_use]
pub fn english_has_lemma(query: &str, candidates: &[Candidate]) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return false;
    }
    candidates.iter().any(|candidate| {
        candidate
            .entry
            .glosses
            .iter()
            .any(|gloss| is_exact_definition(&gloss.to_lowercase(), &query))
    })
}

/// True when `query` is the candidate's main sense: the first synonym of its
/// first gloss (`school` for 学校, `to eat` in `to eat; to consume` for 吃),
/// not a later one.
#[must_use]
pub fn english_is_main_sense(query: &str, candidate: &Candidate) -> bool {
    main_sense(candidate)
        .is_some_and(|sense| is_exact_definition(&sense, &query.trim().to_lowercase()))
}

/// [`english_is_main_sense`], ignoring every note in parentheses:
/// `to accept (a suggestion, punishment, bribe etc)` is `to accept`.
#[must_use]
pub fn english_is_main_sense_ignoring_notes(query: &str, candidate: &Candidate) -> bool {
    main_sense(candidate).is_some_and(|sense| {
        let mut plain = String::with_capacity(sense.len());
        let mut depth = 0usize;
        for c in sense.chars() {
            match c {
                '(' => depth += 1,
                ')' => depth = depth.saturating_sub(1),
                _ if depth == 0 => plain.push(c),
                _ => {}
            }
        }
        is_exact_definition(&plain, &query.trim().to_lowercase())
    })
}

/// The first synonym of the first gloss, lowercased.
fn main_sense(candidate: &Candidate) -> Option<String> {
    let gloss = candidate.entry.glosses.first()?.to_lowercase();
    Some(gloss.split("; ").next().unwrap_or_default().to_owned())
}

/// True when a lowercased gloss is the dictionary definition of `query`.
///
/// CC-CEDICT writes the lemma as `cat (CL:…)` or the verb as `to go`. Those
/// are exact senses, not merely phrases that happen to start with the query
/// (`go to hell`, `cat (Internet slang)`). Classifier notes are structural
/// and stripped; other parentheticals are left in place so slang/literary
/// marked usages stay below the unmarked lemma.
fn is_exact_definition(gloss_lower: &str, query_lower: &str) -> bool {
    is_exact_sense(&sense_core(gloss_lower), query_lower)
}

fn is_exact_sense(sense: &str, query_lower: &str) -> bool {
    !query_lower.is_empty()
        && (sense == query_lower
            || sense
                .strip_prefix("to ")
                .is_some_and(|rest| rest == query_lower))
}

/// A lowercased gloss as a sense: classifier notes, a clarifying note
/// (`hot (of weather)`), surrounding space, and closing punctuation
/// (`what?`) are not part of the meaning. Register notes (`(slang)`,
/// `(dialect)`) stay, so those senses rank below the plain word.
fn sense_core(gloss_lower: &str) -> String {
    let mut sense = strip_classifier_notes(gloss_lower);
    for clarifier in [" (of ", " (e.g. ", " (esp. "] {
        if let Some(start) = sense.find(clarifier) {
            if let Some(end) = sense[start..].find(')') {
                sense.replace_range(start..=start + end, "");
            }
        }
    }
    sense.trim_end_matches(['?', '!', '.']).trim().to_owned()
}

fn strip_classifier_notes(gloss: &str) -> String {
    let mut s = gloss.to_owned();
    while let Some(start) = s.find("(cl:") {
        match s[start..].find(')') {
            Some(rel) => s.replace_range(start..=start + rel, " "),
            None => break,
        }
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Segmented spelling variants used for ranking: the raw normalized query plus
/// every distinct syllable segmentation. Coverage is computed per variant and
/// the best result wins, so both `xian` (one syllable) and `xi an` (two) are
/// rewarded on their own terms.
fn ranking_variants(query: &NormalizedPinyin) -> Vec<Vec<String>> {
    let q = normalize(query.as_str());
    let mut variants = vec![tokens(q.as_str())];
    for segmented in segment(q.as_str()) {
        let variant = tokens(&segmented);
        if !variants.contains(&variant) {
            variants.push(variant);
        }
    }
    variants
}

fn best_syllable_coverage(query_variants: &[Vec<String>], cand_tokens: &[String]) -> usize {
    let mut best = 0;
    for variant in query_variants {
        if variant.is_empty() {
            continue;
        }
        let unique: BTreeSet<&str> = variant.iter().map(String::as_str).collect();
        let matched = unique
            .iter()
            .filter(|t| cand_tokens.iter().any(|c| syllable_equates(t, c)))
            .count();
        best = best.max(percent(matched, unique.len()));
    }
    best
}

/// Smallest number of syllables any segmentation produces. Candidates with
/// fewer syllables than this can never realize a full reading of a
/// multi-syllable query (e.g. single `好` for `nihao`), so they rank at the
/// bottom instead of floating between genuine matches. Zero means the query
/// did not segment (single letters, unknown input) and no gate applies.
fn segment_min_span(query: &NormalizedPinyin) -> usize {
    let q = normalize(query.as_str());
    segment(q.as_str())
        .iter()
        .map(|variant| tokens(variant).len())
        .min()
        .unwrap_or(0)
}

/// Longest contiguous run of candidate syllables aligning to a contiguous
/// window of the query. `nihao` (`ni hao`) scores 2 against 你好 and 1 against
/// 尼米兹号 (`Ni2 mi3 zi1 Hao4`), where the syllables are scattered.
fn longest_aligned_run(query: &[String], cand_tokens: &[String]) -> usize {
    let mut dp = vec![vec![0usize; cand_tokens.len() + 1]; query.len() + 1];
    let mut best = 0;
    for i in 0..query.len() {
        for j in 0..cand_tokens.len() {
            let run = if syllable_equates(&query[i], &cand_tokens[j]) {
                dp[i][j] + 1
            } else {
                0
            };
            dp[i + 1][j + 1] = run;
            best = best.max(run);
        }
    }
    best
}

/// A query syllable matches a dictionary syllable when they are identical, or
/// when the dictionary syllable is the untoned query syllable plus exactly one
/// tone digit (`hao` matches `hao3`, never `chao`). Tones are therefore never
/// guessed by ranking either.
fn syllable_equates(query: &str, cand: &str) -> bool {
    if query == cand {
        return true;
    }
    cand.strip_prefix(query).is_some_and(suffix_is_tone_digit)
}

fn suffix_is_tone_digit(suffix: &str) -> bool {
    let mut chars = suffix.chars();
    matches!(chars.next(), Some('1'..='5')) && chars.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use vocab_core::{DictionaryEntry, MatchBasis, SourceId, SourceVersion};
    use vocab_dictionary::CandidateDiagnostic;

    fn cand(
        id: i64,
        simplified: &str,
        traditional: &str,
        pinyin: &str,
        glosses: &[&str],
        freq: Option<u64>,
        source: &str,
    ) -> Candidate {
        Candidate {
            entry: DictionaryEntry {
                simplified: simplified.to_owned(),
                traditional: traditional.to_owned(),
                pinyin: pinyin.to_owned(),
                glosses: glosses
                    .iter()
                    .map(std::string::ToString::to_string)
                    .collect(),
                frequency_rank: freq,
                hsk_rank: None,
                entry_id: id,
                source: SourceId(source.to_owned()),
                source_version: SourceVersion("1.0".to_owned()),
            },
            diagnostic: CandidateDiagnostic {
                basis: MatchBasis::EnglishGloss,
                is_inferred: false,
            },
        }
    }

    fn ranker() -> DeterministicRanker {
        DeterministicRanker::default()
    }

    #[test]
    fn the_main_sense_is_the_first_synonym_of_the_first_gloss() {
        let eat = cand(
            1,
            "吃",
            "吃",
            "chi1",
            &["to eat; to consume", "to absorb"],
            None,
            "t",
        );
        assert!(english_is_main_sense("eat", &eat));
        assert!(english_is_main_sense("to eat", &eat));
        assert!(!english_is_main_sense("consume", &eat), "a later synonym");
        assert!(!english_is_main_sense("absorb", &eat), "a later gloss");
        let accept = cand(
            2,
            "接受",
            "接受",
            "jie1 shou4",
            &["to accept (a suggestion, punishment, bribe etc); to acquiesce"],
            None,
            "t",
        );
        assert!(!english_is_main_sense("accept", &accept));
        assert!(english_is_main_sense_ignoring_notes("accept", &accept));
        let nothing = cand(3, "x", "x", "x1", &[], None, "t");
        assert!(!english_is_main_sense_ignoring_notes("x", &nothing));
    }

    #[test]
    fn english_filler_words_do_not_outrank_the_meaning() {
        // 也 has `to` (and `too`, which starts with `to`) and is far more
        // common; only 旅行社 is about travelling.
        let also = cand(
            1,
            "也",
            "也",
            "ye3",
            &["also", "too", "(used after a verb) to emphasize"],
            Some(25),
            "src",
        );
        let agency = cand(
            2,
            "旅行社",
            "旅行社",
            "lu:3 xing2 she4",
            &["travel agency"],
            Some(22_700),
            "src",
        );
        let ranked = ranker().rank_english("to travel", vec![also, agency]);
        assert_eq!(ranked[0].entry.simplified, "旅行社");
    }

    #[test]
    fn english_verb_sense_starts_with_the_bare_word() {
        // 旅行's sense is written `to travel`; it is as much a `travel`
        // sense as 旅游's bare `travel`, and 旅行 is the more common word.
        let tourism = cand(
            1,
            "旅游",
            "旅遊",
            "lu:3 you2",
            &["trip", "travel", "to travel"],
            Some(5893),
            "src",
        );
        let travel = cand(
            2,
            "旅行",
            "旅行",
            "lu:3 xing2",
            &["to travel", "journey"],
            Some(1483),
            "src",
        );
        let ranked = ranker().rank_english("travel", vec![tourism, travel]);
        assert_eq!(ranked[0].entry.simplified, "旅行");
    }

    #[test]
    fn english_first_sense_beats_a_later_one() {
        // Senses are listed most important first.
        let tour = cand(
            1,
            "参观",
            "參觀",
            "can1 guan1",
            &["to look around", "to tour", "to visit"],
            Some(100),
            "src",
        );
        let visit = cand(
            2,
            "访问",
            "訪問",
            "fang3 wen4",
            &["to visit", "to call on"],
            Some(900),
            "src",
        );
        let ranked = ranker().rank_english("visit", vec![tour, visit]);
        assert_eq!(ranked[0].entry.simplified, "访问");
    }

    #[test]
    fn english_question_mark_is_not_part_of_the_sense() {
        let what = cand(
            1,
            "什么",
            "什麼",
            "shen2 me5",
            &["what?", "something"],
            Some(40),
            "src",
        );
        let whatever = cand(
            2,
            "无论",
            "無論",
            "wu2 lun4",
            &["no matter what"],
            Some(900),
            "src",
        );
        let ranked = ranker().rank_english("what", vec![whatever, what.clone()]);
        assert_eq!(ranked[0].entry.simplified, "什么");
        assert!(english_has_lemma("what", &[what]));
    }

    #[test]
    fn english_one_gloss_can_hold_several_senses() {
        // CC-CEDICT writes 访问's first gloss as one string.
        let visit = cand(
            1,
            "访问",
            "訪問",
            "fang3 wen4",
            &["to visit; to call on (a person or place)"],
            Some(4125),
            "src",
        );
        let stroll = cand(
            2,
            "逛",
            "逛",
            "guang4",
            &["to stroll", "to visit"],
            Some(3000),
            "src",
        );
        let ranked = ranker().rank_english("visit", vec![stroll, visit]);
        assert_eq!(ranked[0].entry.simplified, "访问");
    }

    #[test]
    fn english_a_pointer_to_another_entry_is_not_a_sense() {
        // `see 大夫` starts with the word `see`, but means nothing by itself.
        let pointer = cand(
            1,
            "大",
            "大",
            "dai4",
            &["see 大夫[dai4 fu5]"],
            Some(10),
            "src",
        );
        let seeing = cand(
            2,
            "眼见为实",
            "眼見為實",
            "yan3 jian4 wei2 shi2",
            &["seeing is believing"],
            Some(40_000),
            "src",
        );
        let ranked = ranker().rank_english("see", vec![pointer, seeing]);
        assert_eq!(ranked[0].entry.simplified, "眼见为实");
    }

    #[test]
    fn english_clarifying_note_is_part_of_the_sense() {
        // `hot (of weather)` is the plain meaning, so the common word is not
        // beaten by a rare one that happens to say just `hot`.
        let rare = cand(1, "暍", "暍", "he4", &["hot"], Some(40_000), "src");
        let hot = cand(
            2,
            "热",
            "熱",
            "re4",
            &["to warm up", "hot (of weather)"],
            Some(500),
            "src",
        );
        let ranked = ranker().rank_english("hot", vec![rare, hot]);
        assert_eq!(ranked[0].entry.simplified, "热");
    }

    #[test]
    fn english_a_rare_word_does_not_lead_for_having_fewer_senses() {
        // As CC-CEDICT writes them: `hot` is 热's third sense, and 暍's only.
        let rare = cand(1, "暍", "暍", "he4", &["hot"], Some(33_347), "src");
        let mut hot = cand(
            2,
            "热",
            "熱",
            "re4",
            &[
                "to warm up",
                "to heat up",
                "hot (of weather)",
                "heat",
                "fervent",
            ],
            Some(1580),
            "src",
        );
        hot.entry.hsk_rank = Some(1);
        let ranked = ranker().rank_english("hot", vec![rare, hot]);
        assert_eq!(ranked[0].entry.simplified, "热");
    }

    #[test]
    fn english_an_unused_word_does_not_lead_on_its_sense() {
        // 屯驻 means `to quarter` troops, and is in no frequency list or
        // HSK level; 刻 is the quarter hour.
        let station = cand(
            1,
            "屯驻",
            "屯駐",
            "tun2 zhu4",
            &["to station; to quarter; to garrison"],
            None,
            "src",
        );
        let mut quarter = cand(
            2,
            "刻",
            "刻",
            "ke4",
            &["quarter (hour)", "moment", "to carve"],
            Some(11_122),
            "src",
        );
        quarter.entry.hsk_rank = Some(2);
        let ranked = ranker().rank_english("quarter", vec![station, quarter]);
        assert_eq!(ranked[0].entry.simplified, "刻");
    }

    #[test]
    fn english_a_word_with_no_frequency_does_not_lead() {
        // 叕 has the sense first, but no frequency at all; 缺少 is common.
        let rare = cand(1, "叕", "叕", "zhuo2", &["to lack"], None, "src");
        let common = cand(
            2,
            "缺少",
            "缺少",
            "que1 shao3",
            &["lack", "shortage of", "to lack"],
            Some(2500),
            "src",
        );
        let ranked = ranker().rank_english("to lack", vec![rare, common]);
        assert_eq!(ranked[0].entry.simplified, "缺少");
    }

    #[test]
    fn english_exact_phrase_ranks_first() {
        let a = cand(2, "学校", "學校", "xue2 xiao4", &["school"], None, "src");
        let b = cand(
            1,
            "学校见",
            "學校見",
            "xue2 xiao4 jian4",
            &["see you at school"],
            None,
            "src",
        );
        let ranked = ranker().rank_english("at school", vec![a.clone(), b.clone()]);
        assert_eq!(ranked[0].entry.simplified, "学校见");
    }

    #[test]
    fn english_exact_single_gloss_beats_containment() {
        let direct = cand(
            1,
            "学校",
            "學校",
            "xue2 xiao4",
            &["school", "CL:所[suo3]"],
            None,
            "src",
        );
        let indirect = cand(
            2,
            "兵家",
            "兵家",
            "Bing1 jia1",
            &["the School of the Military"],
            None,
            "src",
        );
        let ranked = ranker().rank_english("school", vec![indirect.clone(), direct.clone()]);
        assert_eq!(
            ranked[0].entry.simplified, "学校",
            "an entry whose gloss list contains 'school' must beat a phrase containing it"
        );
    }

    #[test]
    fn english_gloss_prefix_beats_longer_phrase() {
        let cat = cand(
            1,
            "猫",
            "貓",
            "mao1",
            &["cat (CL:隻|只[zhi1])", "(dialect) to hide oneself"],
            None,
            "src",
        );
        let proverb = cand(
            2,
            "不管白猫黑猫",
            "不管白貓黑貓",
            "...",
            &["it doesn't matter whether a cat is white or black"],
            None,
            "src",
        );
        let ranked = ranker().rank_english("cat", vec![proverb.clone(), cat.clone()]);
        assert_eq!(
            ranked[0].entry.simplified, "猫",
            "gloss starting with 'cat' must beat a phrase that merely contains it"
        );
    }

    #[test]
    fn english_classifier_note_counts_as_exact_lemma() {
        // One frequency for all, so only the lemma decides.
        let lemma = cand(
            3,
            "猫",
            "貓",
            "mao1",
            &["cat (CL:隻|只[zhi1])", "(dialect) to hide oneself"],
            Some(500),
            "src",
        );
        let slang = cand(
            1,
            "喵星人",
            "喵星人",
            "miao1 xing1 ren2",
            &["cat (Internet slang)"],
            Some(500),
            "src",
        );
        let literary = cand(
            2,
            "狸奴",
            "狸奴",
            "li2 nu2",
            &["cat (literary or jocular)"],
            Some(500),
            "src",
        );
        let ranked =
            ranker().rank_english("cat", vec![slang.clone(), literary.clone(), lemma.clone()]);
        assert_eq!(
            ranked[0].entry.simplified, "猫",
            "CEDICT 'cat (CL:…)' is the unmarked lemma and must beat slang/literary 'cat (…)'"
        );
    }

    #[test]
    fn english_to_verb_counts_as_exact_lemma() {
        // One frequency for both, so only the lemma decides.
        let go = cand(
            2,
            "去",
            "去",
            "qu4",
            &["to go", "to leave", "to remove"],
            Some(500),
            "src",
        );
        let curse = cand(
            1,
            "去死",
            "去死",
            "qu4 si3",
            &["go to hell!"],
            Some(500),
            "src",
        );
        let ranked = ranker().rank_english("go", vec![curse.clone(), go.clone()]);
        assert_eq!(
            ranked[0].entry.simplified, "去",
            "CEDICT 'to go' is the lemma for query 'go' and must beat a phrase starting with 'go'"
        );
    }

    #[test]
    fn romanized_name_in_gloss_is_not_an_english_lemma() {
        let name = cand(
            1,
            "吴敬梓",
            "吳敬梓",
            "Wu2 Jing4 zi3",
            &["Wu Jingzi (1701–1754), Qing novelist"],
            None,
            "src",
        );
        let uprising = cand(
            3,
            "金田起义",
            "金田起義",
            "Jin1 tian2 qi3 yi4",
            &["Jintian Uprising"],
            None,
            "src",
        );
        let mirror = cand(2, "镜子", "鏡子", "jing4 zi5", &["mirror"], None, "src");
        assert!(!english_has_lemma("jingzi", std::slice::from_ref(&name)));
        assert!(!english_has_lemma(
            "jintian",
            std::slice::from_ref(&uprising)
        ));
        assert!(english_has_lemma("mirror", std::slice::from_ref(&mirror)));
    }

    #[test]
    fn english_shorter_gloss_preferred() {
        let short = cand(1, "学校", "學校", "xue2 xiao4", &["school"], None, "src");
        let long = cand(
            2,
            "宗",
            "宗",
            "zong1",
            &["school", "sect", "purpose", "model", "ancestor", "clan"],
            None,
            "src",
        );
        let ranked = ranker().rank_english("school", vec![long.clone(), short.clone()]);
        assert_eq!(
            ranked[0].entry.simplified, "学校",
            "single-gloss entry beats multi-gloss entry with identical first gloss"
        );
    }

    #[test]
    fn english_frequency_breaks_match_ties() {
        let low = cand(1, "世界", "世界", "shi4 jie4", &["world"], Some(50), "src");
        let high = cand(
            2,
            "世界盃",
            "世界盃",
            "shi4 jie4 bei1",
            &["world cup"],
            Some(900),
            "src",
        );
        let ranked = ranker().rank_english("world", vec![high.clone(), low.clone()]);
        assert_eq!(ranked[0].entry.simplified, "世界");
    }

    #[test]
    fn identical_query_and_data_gives_identical_order() {
        let a = cand(3, "教", "教", "jiao1", &["teaching"], None, "src");
        let b = cand(1, "教育", "教育", "jiao4 yu4", &["education"], None, "src");
        let c = cand(2, "教学", "教學", "jiao4 xue2", &["teaching"], None, "src");
        let named = vec![a.clone(), b.clone(), c.clone()];
        let first = ranker().rank_english("teaching", named.clone());
        let second = ranker().rank_english("teaching", named);
        assert_eq!(first, second);
    }

    #[test]
    fn entry_id_breaks_full_ties() {
        let a = cand(5, "猫", "貓", "mao1", &["cat"], None, "src");
        let b = cand(2, "猫", "貓", "mao1", &["cat"], None, "src");
        let ranked = ranker().rank_chinese("猫", vec![a.clone(), b.clone()]);
        assert_eq!(ranked[0].entry.entry_id, 2);
    }

    #[test]
    fn pinyin_exact_match_ranks_first() {
        let a = cand(1, "你好", "你好", "ni3 hao3", &["hello"], None, "src");
        let b = cand(2, "您", "您", "nin2", &["you (formal)"], None, "src");
        let ranked = ranker().rank_pinyin(&normalize("ni3 hao3"), vec![b.clone(), a.clone()]);
        assert_eq!(ranked[0].entry.simplified, "你好");
        assert_eq!(ranked[1].entry.entry_id, 2);
    }

    #[test]
    fn untoned_query_matches_toned_syllables() {
        let target = cand(1, "你好", "你好", "ni3 hao3", &["hello"], None, "src");
        let noise = cand(
            2,
            "车号",
            "車號",
            "che1 hao4",
            &["vehicle number"],
            Some(10),
            "src",
        );
        let louder = cand(
            3,
            "起超",
            "起超",
            "qi3 chao1",
            &["rise above"],
            Some(1),
            "src",
        );
        let ranked = ranker().rank_pinyin(
            &normalize("ni hao"),
            vec![louder.clone(), noise.clone(), target.clone()],
        );
        assert_eq!(
            ranked[0].entry.simplified, "你好",
            "untoned ni+hao must beat noise che1+hao4 and qi3+chao1"
        );
    }

    #[test]
    fn query_syllable_is_never_prefix_match_across_syllables() {
        let target = cand(1, "泥", "泥", "ni2", &["mud"], None, "src");
        let neighbour = cand(2, "鸟", "鳥", "niao3", &["bird"], None, "src");
        let ranked =
            ranker().rank_pinyin(&normalize("ni"), vec![neighbour.clone(), target.clone()]);
        assert_eq!(
            ranked[0].entry.simplified, "泥",
            "ni must match ni2 exactly, not slurp niao3"
        );
        assert_eq!(
            ranker().rank_pinyin(&normalize("ni"), vec![neighbour.clone(), target.clone()]),
            ranked
        );
    }

    #[test]
    fn pinyin_syllable_coverage_beats_partial_match() {
        let full = cand(1, "旅行", "旅行", "lv3 xing2", &["travel"], None, "src");
        let partial = cand(2, "旅游", "旅游", "lv3 you2", &["tourism"], None, "src");
        let ranked =
            ranker().rank_pinyin(&normalize("lv3 xing2"), vec![partial.clone(), full.clone()]);
        assert_eq!(
            ranked[0].entry.simplified, "旅行",
            "full syllable coverage compounds"
        );
    }

    #[test]
    fn shorter_span_beats_compound_at_same_coverage() {
        let school = cand(1, "学校", "學校", "xue2 xiao4", &["school"], None, "src");
        let compound = cand(
            2,
            "公立学校",
            "公立學校",
            "gong1 li4 xue2 xiao4",
            &["public school"],
            None,
            "src",
        );
        let ranked = ranker().rank_pinyin(
            &normalize("xuexiao"),
            vec![compound.clone(), school.clone()],
        );
        assert_eq!(
            ranked[0].entry.simplified, "学校",
            "2-syllable reading must beat a 4-syllable word that contains it"
        );
    }

    #[test]
    fn contiguous_sequence_beats_scattered_syllables() {
        let exact = cand(1, "你好", "你好", "ni3 hao3", &["hello"], None, "src");
        let plus_suffix = cand(
            2,
            "你好吗",
            "你好嗎",
            "ni3 hao3 ma5",
            &["how are you"],
            None,
            "src",
        );
        let scattered = cand(
            3,
            "尼米兹号",
            "尼米茲號",
            "Ni2 mi3 zi1 Hao4",
            &["Nimitz class"],
            Some(10),
            "src",
        );
        let ranked = ranker().rank_pinyin(
            &normalize("nihao"),
            vec![scattered.clone(), plus_suffix.clone(), exact.clone()],
        );
        assert_eq!(ranked[0].entry.simplified, "你好");
        assert_eq!(
            ranked[1].entry.simplified, "你好吗",
            "contiguous ni+hao separates the real reading from Ni-mi-zi-hao"
        );
        assert_eq!(ranked[2].entry.simplified, "尼米兹号");
    }

    #[test]
    fn undersized_single_syllables_sink_for_multisyllable_query() {
        let exact = cand(1, "你好", "你好", "ni3 hao3", &["hello"], None, "src");
        let single = cand(2, "好", "好", "hao3", &["good"], Some(900), "src");
        let single_too = cand(3, "告", "告", "gao4", &["to sue"], Some(800), "src");
        let ranked = ranker().rank_pinyin(
            &normalize("nihao"),
            vec![single_too.clone(), single.clone(), exact.clone()],
        );
        assert_eq!(
            ranked[0].entry.simplified, "你好",
            "full 2-syllable reading stays on top"
        );
        assert_eq!(
            ranked[1].entry.simplified, "好",
            "frequent single 好 still loses to the real reading"
        );
        assert_eq!(
            ranked[2].entry.simplified, "告",
            "single syllables sort deterministically by tail below real matches"
        );
        assert_eq!(
            ranked[1].entry.simplified, "好",
            "the higher-frequency single still loses to the real reading"
        );
    }

    #[test]
    fn cedict_u_colon_form_matches_dotted_query() {
        let cedict = cand(1, "旅行", "旅行", "lu:3 xing2", &["travel"], None, "src");
        let dotted = cand(2, "旅途", "旅途", "lv3 tu2", &["journey"], None, "src");
        let ranked = ranker().rank_pinyin(&normalize("lv3 xing2"), vec![dotted, cedict.clone()]);
        assert_eq!(ranked[0].entry.simplified, "旅行");
        assert_eq!(
            ranks_ordinal(&ranker(), "lü xíng", &cedict),
            ranks_ordinal(&ranker(), "lv3 xing2", &cedict),
            "dotted and digit forms must rank identically"
        );
    }

    fn ranks_ordinal(ranker: &DeterministicRanker, query: &str, target: &Candidate) -> usize {
        let normalized = crate::ranker::normalize(query);
        let output = ranker.rank_pinyin(&normalized, vec![target.clone()]);
        output
            .iter()
            .position(|c| c == target)
            .unwrap_or(usize::MAX)
    }

    #[test]
    fn chinese_exact_simplified_beats_prefix() {
        let prefix = cand(1, "世界", "世界", "shi4 jie4", &["world"], None, "src");
        let exact = cand(
            2,
            "世界盃",
            "世界盃",
            "shi4 jie4 bei1",
            &["world cup"],
            None,
            "src",
        );
        let ranked = ranker().rank_chinese("世", vec![exact.clone(), prefix.clone()]);
        assert_eq!(ranked[0].entry.simplified, "世界");
    }
}
