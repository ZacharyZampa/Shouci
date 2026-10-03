use std::cmp::Ordering;
use std::collections::BTreeSet;

use vocab_core::MatchBasis;
use vocab_core::SourceId;
use vocab_dictionary::{Candidate, english_terms, is_cross_reference};
use vocab_pinyin::{NormalizedPinyin, normalize, segment};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TailKey {
    pub source_index: usize,
    pub frequency: u64,
    pub hsk: u64,
    pub entry_id: i64,
}

impl TailKey {
    #[must_use]
    pub fn compare(&self, other: &Self) -> Ordering {
        self.source_index
            .cmp(&other.source_index)
            .then_with(|| self.frequency.cmp(&other.frequency))
            .then_with(|| self.hsk.cmp(&other.hsk))
            .then_with(|| self.entry_id.cmp(&other.entry_id))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct RankKey {
    pub exact_single_gloss: bool,
    pub gloss_prefix: bool,
    pub gloss_word_prefix: bool,
    pub exact_phrase: bool,
    pub exact_token: bool,
    pub prefix: bool,
    pub undersized: bool,
    pub pinyin_sequence: usize,
    pub token_coverage: usize,
    pub span: usize,
    pub exact_simplified: bool,
    pub exact_traditional: bool,
    pub tail: TailKey,
}

impl RankKey {
    #[must_use]
    pub fn compare(&self, other: &Self) -> Ordering {
        self.exact_single_gloss
            .cmp(&other.exact_single_gloss)
            .reverse()
            .then_with(|| self.gloss_prefix.cmp(&other.gloss_prefix).reverse())
            .then_with(|| {
                self.gloss_word_prefix
                    .cmp(&other.gloss_word_prefix)
                    .reverse()
            })
            .then_with(|| self.exact_phrase.cmp(&other.exact_phrase).reverse())
            .then_with(|| self.exact_token.cmp(&other.exact_token).reverse())
            .then_with(|| self.prefix.cmp(&other.prefix).reverse())
            .then_with(|| self.undersized.cmp(&other.undersized))
            .then_with(|| other.pinyin_sequence.cmp(&self.pinyin_sequence))
            .then_with(|| other.token_coverage.cmp(&self.token_coverage))
            .then_with(|| self.span.cmp(&other.span))
            .then_with(|| self.exact_simplified.cmp(&other.exact_simplified).reverse())
            .then_with(|| {
                self.exact_traditional
                    .cmp(&other.exact_traditional)
                    .reverse()
            })
            .then_with(|| self.tail.compare(&other.tail))
    }
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

    fn english_key(&self, query: &str, cand: &Candidate) -> RankKey {
        let gloss = cand.entry.glosses.join(" ").to_lowercase();
        let query = query.trim().to_lowercase();
        let terms = english_terms(&query);
        let gloss_tokens = tokens(&gloss);
        // One gloss can hold several senses: `to visit; to call on`. A
        // pointer to another entry (`see 大夫[dai4 fu5]`) is not one.
        let senses: Vec<String> = cand
            .entry
            .glosses
            .iter()
            .flat_map(|g| g.split(';'))
            .map(|g| sense_core(&g.to_lowercase()))
            .filter(|sense| !sense.is_empty() && !is_cross_reference(sense))
            .collect();
        // A verb's sense is written `to travel`; it starts with `travel` too.
        let readings = || {
            senses
                .iter()
                .flat_map(|sense| [Some(sense.as_str()), sense.strip_prefix("to ")])
                .flatten()
        };

        let matched = terms
            .iter()
            .filter(|term| gloss_tokens.iter().any(|g| g == *term))
            .count();
        let prefixed = terms
            .iter()
            .filter(|term| gloss_tokens.iter().any(|g| g.starts_with(term.as_str())))
            .count();
        let coverage = if terms.is_empty() {
            0
        } else {
            matched * 100 / terms.len()
        };
        // Senses come most important first: among words that all have the
        // query as a sense, the one whose first sense it is leads. A rare
        // word gives up a place or two, so it does not lead a common word
        // for having fewer senses.
        let sense_position = senses
            .iter()
            .position(|sense| is_exact_sense(sense, &query))
            .unwrap_or(0);
        let rarity = match cand.entry.frequency_rank {
            Some(rank) if rank <= 10_000 => 0,
            Some(rank) if rank <= 50_000 => 1,
            Some(_) => 2,
            // No frequency at all: rare in modern Chinese.
            None => 5,
        };

        RankKey {
            exact_single_gloss: !query.is_empty()
                && senses.iter().any(|sense| is_exact_sense(sense, &query)),
            gloss_prefix: !query.is_empty() && readings().any(|r| r.starts_with(&query)),
            gloss_word_prefix: !query.is_empty()
                && readings().any(|r| {
                    r == query
                        || r.starts_with(&format!("{query} "))
                        || r.starts_with(&format!("{query};"))
                        || r.starts_with(&format!("{query}("))
                }),
            exact_phrase: !query.is_empty() && gloss.contains(&query),
            // Every word of the query, then every word at least as a prefix
            // (`trav` in `travel`); a gloss sharing only some words ranks by
            // how many.
            exact_token: !terms.is_empty() && matched == terms.len(),
            prefix: !terms.is_empty() && prefixed == terms.len(),
            undersized: false,
            pinyin_sequence: 0,
            token_coverage: coverage,
            span: sense_position + rarity,
            exact_simplified: false,
            exact_traditional: false,
            tail: self.tail(cand),
        }
    }

    fn pinyin_key(
        &self,
        query_variants: &[Vec<String>],
        min_span: usize,
        cand: &Candidate,
    ) -> RankKey {
        let cand_norm = normalize(&cand.entry.pinyin);
        let cand_tokens = tokens(cand_norm.as_str());
        let exact = |variant: &Vec<String>| variant.join(" ") == cand_norm.as_str();
        let sequence = query_variants
            .iter()
            .map(|query| longest_aligned_run(query, &cand_tokens))
            .max()
            .unwrap_or(0);

        RankKey {
            exact_single_gloss: false,
            gloss_prefix: false,
            gloss_word_prefix: false,
            exact_phrase: false,
            exact_token: query_variants.iter().any(exact),
            prefix: false,
            undersized: cand_tokens.len() < min_span,
            pinyin_sequence: sequence,
            token_coverage: best_syllable_coverage(query_variants, &cand_tokens),
            span: cand_tokens.len(),
            exact_simplified: false,
            exact_traditional: false,
            tail: self.tail(cand),
        }
    }

    fn chinese_key(&self, query: &str, cand: &Candidate) -> RankKey {
        let query = query.trim();
        // Below every real match: entries with all of the query's characters,
        // then the words found inside it in reading order, the longer word
        // first where two start together.
        let (coverage, span) = match cand.diagnostic.basis {
            MatchBasis::CharacterFallback => (1, 0),
            MatchBasis::ContainedWord => {
                let entry = &cand.entry;
                let at = query
                    .find(entry.simplified.as_str())
                    .or_else(|| query.find(entry.traditional.as_str()))
                    .map_or(usize::MAX / 64, |byte| query[..byte].chars().count());
                (
                    0,
                    at * 32 + 32_usize.saturating_sub(entry.simplified.chars().count()),
                )
            }
            _ => (0, 0),
        };
        RankKey {
            exact_single_gloss: false,
            gloss_prefix: false,
            gloss_word_prefix: false,
            exact_phrase: false,
            exact_token: false,
            prefix: !query.is_empty() && cand.entry.simplified.starts_with(query),
            undersized: cand.diagnostic.is_inferred,
            pinyin_sequence: 0,
            token_coverage: coverage,
            span,
            exact_simplified: !query.is_empty() && cand.entry.simplified == query,
            exact_traditional: !query.is_empty() && cand.entry.traditional == query,
            tail: self.tail(cand),
        }
    }
}

/// Sorts by `key`, computing it once per candidate. The sort is stable, so
/// equal keys keep retrieval order.
fn sort_by_key(candidates: Vec<Candidate>, key: impl Fn(&Candidate) -> RankKey) -> Vec<Candidate> {
    let mut keyed: Vec<(RankKey, Candidate)> = candidates
        .into_iter()
        .map(|candidate| (key(&candidate), candidate))
        .collect();
    keyed.sort_by(|(a, _), (b, _)| a.compare(b));
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

/// True when a lowercased gloss is the dictionary definition of `query`.
///
/// CC-CEDICT writes the lemma as `cat (CL:…)` or the verb as `to go`. Those
/// are exact senses, not merely phrases that happen to start with the query
/// (`go to hell`, `cat (Internet slang)`). Classifier notes are structural
/// and stripped; other parentheticals are left in place so slang/literary
/// marked usages stay below the unmarked lemma.
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
        best = best.max(matched * 100 / unique.len());
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
        let lemma = cand(
            3,
            "猫",
            "貓",
            "mao1",
            &["cat (CL:隻|只[zhi1])", "(dialect) to hide oneself"],
            None,
            "src",
        );
        let slang = cand(
            1,
            "喵星人",
            "喵星人",
            "miao1 xing1 ren2",
            &["cat (Internet slang)"],
            None,
            "src",
        );
        let literary = cand(
            2,
            "狸奴",
            "狸奴",
            "li2 nu2",
            &["cat (literary or jocular)"],
            None,
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
        let go = cand(
            2,
            "去",
            "去",
            "qu4",
            &["to go", "to leave", "to remove"],
            None,
            "src",
        );
        let curse = cand(1, "去死", "去死", "qu4 si3", &["go to hell!"], None, "src");
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
