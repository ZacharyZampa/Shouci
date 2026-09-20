//! Pinyin normalization and pronunciation authority chain.
//!
//! The normalization matrix is spec-first: fixtures under `fixtures/pinyin/`
//! document the rules, and the tests here pin them. Normalization never segments
//! unheard syllables or guesses tones — input that already carries a tone number
//! stays untouched, and ambiguous forms (`xi'an` vs `xian`) are preserved as
//! distinct strings.

use std::fmt;

mod syllables;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NormalizedPinyin(String);

impl NormalizedPinyin {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for NormalizedPinyin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for NormalizedPinyin {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::str::FromStr for NormalizedPinyin {
    type Err = std::convert::Infallible;

    /// Parses raw pinyin and normalizes it.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(normalize(s))
    }
}

/// Canonicalizes raw pinyin for lookup.
///
/// Rules, in order:
/// 1. trim surrounding whitespace,
/// 2. lowercase,
/// 3. `ü` and its tone-marked forms → `v` (carrying the tone), so `lǚ` == `lv3`;
///    CC-CEDICT's `u:` spelling is equivalent (`lu:3` == `lv3`),
/// 4. tone-marked vowels → plain vowel, with the tone digit appended at the end of
///    the syllable (`shuǐ` == `shui3`; `nǐ hǎo` == `ni3 hao3`),
/// 5. apostrophes preserved unchanged (`xi'an` stays distinct from `xian`),
/// 6. runs of spaces/tabs collapsed to a single space.
///
/// The output is deterministic and never infers segmentation or missing tones.
#[must_use]
pub fn normalize(input: &str) -> NormalizedPinyin {
    let mut out = String::with_capacity(input.len());
    let mut pending_tone: Option<u8> = None;
    let lowered = input.trim().to_lowercase();
    let mut chars = lowered.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' => {
                if !out.is_empty() && !out.ends_with(' ') {
                    if let Some(tone) = pending_tone.take() {
                        out.push(char::from(b'0' + tone));
                    }
                    out.push(' ');
                }
            }
            'ü' => out.push('v'),
            'u' if chars.peek() == Some(&':') => {
                chars.next();
                out.push('v');
            }
            '\'' => {
                if let Some(tone) = pending_tone.take() {
                    out.push(char::from(b'0' + tone));
                }
                out.push('\'');
            }
            c => match marked_tone(c) {
                Some((base, tone)) => {
                    out.push(base);
                    pending_tone = Some(tone);
                }
                None => out.push(c),
            },
        }
    }
    if let Some(tone) = pending_tone {
        out.push(char::from(b'0' + tone));
    }
    NormalizedPinyin(out.trim_end().to_string())
}

/// Every segmenter run is capped at this many returned spellings; pathological
/// repeated-syllable inputs are bounded while ordinary words never get close.
const MAX_SEGMENTATIONS: usize = 64;

/// Segments unspaced pinyin into space-separated syllable runs.
///
/// Input must already be normalized (lowercase, `ü`/`u:` as `v`, tone marks as
/// digits, as produced by [`normalize`]). Each whitespace token is split at
/// apostrophes into hard-boundary segments, each segment is split every valid
/// way using [`syllables::SYLLABLES`], and the results are joined. Tone digits
/// stay bound to their syllable; no tones are guessed; existing spacing and
/// apostrophes are preserved as boundaries.
///
/// Returns variants in deterministic order (fewer syllables first, then
/// lexicographic), deduplicated, capped at [`MAX_SEGMENTATIONS`]. The caller
/// may prepend the raw (unspaced) spelling if it wants it retained.
#[must_use]
pub fn segment(input: &str) -> Vec<String> {
    if input.trim().is_empty() {
        return Vec::new();
    }
    let mut results: Vec<String> = input
        .split(' ')
        .filter(|t| !t.is_empty())
        .map(token_forms)
        .reduce(cross_join)
        .unwrap_or_default();
    results.sort();
    results.dedup();
    results
}

fn token_forms(token: &str) -> Vec<String> {
    let hard_boundaries: Vec<&str> = token.split('\'').filter(|s| !s.is_empty()).collect();
    if hard_boundaries.is_empty() {
        return Vec::new();
    }
    let mut per_boundary: Vec<Vec<String>> = Vec::new();
    for part in hard_boundaries {
        let mut forms = Vec::new();
        let mut walked = Vec::new();
        walk_syllables(part, &mut walked, &mut forms);
        forms.sort_by(|a, b| syllable_count_order(a, b));
        forms.truncate(MAX_SEGMENTATIONS);
        per_boundary.push(forms);
    }
    per_boundary
        .into_iter()
        .reduce(cross_join)
        .unwrap_or_default()
}

/// Takes ownership to satisfy `Iterator::reduce`'s `Fn` bound while avoiding
/// needless clones of the accumulating segment lists.
#[allow(clippy::needless_pass_by_value)]
fn cross_join(left: Vec<String>, right: Vec<String>) -> Vec<String> {
    let mut joined = Vec::with_capacity(left.len() * right.len());
    for a in &left {
        for b in &right {
            joined.push(format!("{a} {b}"));
        }
    }
    joined
}

fn walk_syllables<'a>(part: &'a str, walked: &mut Vec<&'a str>, forms: &mut Vec<String>) {
    let bytes = part.as_bytes();
    if walked.iter().map(|w| w.len()).sum::<usize>() == part.len() {
        forms.push(walked.join(" "));
        return;
    }
    let pos = walked.iter().map(|w| w.len()).sum::<usize>();
    for syllable in syllables::SYLLABLES {
        // Single-letter syllables (a, e, o, n, r, ...) and the interjections
        // (m, n, ng, hm, hng) are excluded from segmentation: they would
        // otherwise produce spurious splits like `xi a n`, `gu o`, or
        // `xi ng2` for `xing2`. Standalone queries for these still reach the
        // raw (unsegmented) path.
        if syllable.len() == 1 || matches!(*syllable, "m" | "n" | "ng" | "hm" | "hng") {
            continue;
        }
        let s = syllable.as_bytes();
        if !bytes[pos..].starts_with(s) {
            continue;
        }
        let mut end = pos + s.len();
        if end < bytes.len() && (b'1'..=b'5').contains(&bytes[end]) {
            end += 1;
        }
        walked.push(&part[pos..end]);
        walk_syllables(part, walked, forms);
        walked.pop();
    }
}

fn syllable_count_order(a: &str, b: &str) -> std::cmp::Ordering {
    a.split(' ')
        .count()
        .cmp(&b.split(' ').count())
        .then_with(|| a.cmp(b))
}

fn marked_tone(c: char) -> Option<(char, u8)> {
    match c {
        'ā' => Some(('a', 1)),
        'á' => Some(('a', 2)),
        'ǎ' => Some(('a', 3)),
        'à' => Some(('a', 4)),
        'ē' => Some(('e', 1)),
        'é' => Some(('e', 2)),
        'ě' => Some(('e', 3)),
        'è' => Some(('e', 4)),
        'ī' => Some(('i', 1)),
        'í' => Some(('i', 2)),
        'ǐ' => Some(('i', 3)),
        'ì' => Some(('i', 4)),
        'ō' => Some(('o', 1)),
        'ó' => Some(('o', 2)),
        'ǒ' => Some(('o', 3)),
        'ò' => Some(('o', 4)),
        'ū' => Some(('u', 1)),
        'ú' => Some(('u', 2)),
        'ǔ' => Some(('u', 3)),
        'ù' => Some(('u', 4)),
        'ǖ' => Some(('v', 1)),
        'ǘ' => Some(('v', 2)),
        'ǚ' => Some(('v', 3)),
        'ǜ' => Some(('v', 4)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{normalize, segment};

    #[test]
    fn segments_unspaced_pinyin() {
        let variants = segment("nihao");
        assert_eq!(variants, vec!["ni hao"]);
        assert_eq!(segment("zhongguo"), vec!["zhong guo"]);
    }

    #[test]
    fn segments_with_tone_numbers_bound_to_syllables() {
        assert_eq!(segment("ni3hao3"), vec!["ni3 hao3"]);
        assert_eq!(segment("wo3men"), vec!["wo3 men"]);
    }

    #[test]
    fn unambiguous_ambiguity_returns_all_spellings() {
        assert_eq!(segment("xian"), vec!["xi an", "xian"]);
        assert_eq!(segment("ni3hao"), vec!["ni3 hao"]);
    }

    #[test]
    fn spaced_input_is_left_alone() {
        assert_eq!(segment("ni hao"), vec!["ni hao"]);
        assert_eq!(segment("lv3 xing2"), vec!["lv3 xing2"]);
    }

    #[test]
    fn apostrophes_are_hard_boundaries() {
        let variants = segment("xi'an");
        assert!(variants.contains(&"xi an".to_owned()));
        let variants = segment("piao'er");
        assert!(variants.contains(&"piao er".to_owned()));
        assert!(
            variants.iter().all(|v| !v.contains("ng")),
            "interjections never split real syllables"
        );
    }

    #[test]
    fn empty_and_unsegmentable_input_returns_nothing() {
        assert!(segment("   ").is_empty());
        assert_eq!(segment("qqqq"), Vec::<String>::new(), "no valid syllables");
    }

    #[test]
    fn untoned_query_segments_to_toned_query() {
        assert_eq!(segment("shui"), vec!["shui"]);
        assert_eq!(segment("shuiguo"), vec!["shui guo"]);
    }

    #[test]
    fn lowercases_and_trims() {
        assert_eq!(normalize("  NI HAO ").as_str(), "ni hao");
    }

    #[test]
    fn tone_marks_become_tone_numbers() {
        assert_eq!(normalize("nǐ hǎo").as_str(), "ni3 hao3");
        assert_eq!(normalize("wǒ men").as_str(), "wo3 men");
        assert_eq!(normalize("shuǐ").as_str(), "shui3");
    }

    #[test]
    fn tone_numbers_pass_through() {
        assert_eq!(normalize("ni3 hao3").as_str(), "ni3 hao3");
    }

    #[test]
    fn marked_and_numbered_forms_are_equivalent() {
        assert_eq!(normalize("lǜ"), normalize("lv4"));
        assert_eq!(normalize("lǚ xíng"), normalize("lv3 xing2"));
    }

    #[test]
    fn cedict_u_colon_maps_to_v() {
        assert_eq!(normalize("lu:3 xing2").as_str(), "lv3 xing2");
        assert_eq!(normalize("lu:3 xing2"), normalize("lǚ xíng"));
        assert_eq!(normalize("nu:3").as_str(), "nv3");
    }

    #[test]
    fn u_umlaut_maps_to_v() {
        assert_eq!(normalize("lü").as_str(), "lv");
        assert_eq!(normalize("LÜ").as_str(), "lv");
    }

    #[test]
    fn apostrophes_preserve_syllable_boundaries() {
        assert_ne!(normalize("xi'an"), normalize("xian"));
        assert_eq!(normalize("xi'an").as_str(), "xi'an");
        assert_eq!(normalize("xǐ'an").as_str(), "xi3'an");
    }

    #[test]
    fn collapses_repeated_whitespace() {
        assert_eq!(normalize("ni   hao\t\tzhong").as_str(), "ni hao zhong");
    }

    #[test]
    fn empty_input_is_empty() {
        assert_eq!(normalize("   ").as_str(), "");
    }
}
