//! Pinyin normalization and pronunciation authority chain.
//!
//! The normalization matrix is spec-first: fixtures under `fixtures/pinyin/`
//! document the rules, and the tests here pin them. Normalization never segments
//! unheard syllables or guesses tones — input that already carries a tone number
//! stays untouched, and ambiguous forms (`xi'an` vs `xian`) are preserved as
//! distinct strings.

use std::fmt;

mod marks;
mod numbered;
mod syllables;

pub use marks::tone_marks;
pub use numbered::numbered;

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
    let lowered = compose(input.trim()).to_lowercase();
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
/// way using the syllable table, and the results are joined. Tone digits
/// stay bound to their syllable; no tones are guessed; existing spacing and
/// apostrophes are preserved as boundaries.
///
/// Returns variants sorted lexicographically and deduplicated, at most
/// 64 of them: when there are more, the ones with the
/// fewest syllables are kept. The caller may prepend the raw (unspaced)
/// spelling if it wants it retained.
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
    hard_boundaries
        .into_iter()
        .map(part_forms)
        .reduce(cross_join)
        .unwrap_or_default()
}

/// Takes ownership to satisfy `Iterator::reduce`'s `Fn` bound while avoiding
/// needless clones of the accumulating segment lists. Capped like a single
/// part, so many tokens cannot multiply into a huge list.
#[allow(clippy::needless_pass_by_value)]
fn cross_join(left: Vec<String>, right: Vec<String>) -> Vec<String> {
    let mut joined = Vec::with_capacity(left.len() * right.len());
    for a in &left {
        for b in &right {
            joined.push(format!("{a} {b}"));
        }
    }
    joined.sort_by(|a, b| syllable_count_order(a, b));
    joined.truncate(MAX_SEGMENTATIONS);
    joined
}

/// Every way to split `part` into syllables, fewest syllables first, capped
/// at [`MAX_SEGMENTATIONS`].
///
/// Dynamic programming over positions: each suffix is split once and its
/// best splits reused, so the work grows with the length of the input, not
/// with the (exponential) number of possible splits. Keeping only the best
/// [`MAX_SEGMENTATIONS`] per suffix loses nothing: prefixing one syllable
/// preserves the order within a suffix's list.
fn part_forms(part: &str) -> Vec<String> {
    let mut memo: Vec<Option<Vec<String>>> = vec![None; part.len() + 1];
    suffix_forms(part, 0, &mut memo)
}

fn suffix_forms(part: &str, pos: usize, memo: &mut Vec<Option<Vec<String>>>) -> Vec<String> {
    if let Some(done) = &memo[pos] {
        return done.clone();
    }
    let bytes = part.as_bytes();
    let mut forms = Vec::new();
    if pos == bytes.len() {
        forms.push(String::new());
    } else {
        for syllable in syllables::SYLLABLES {
            // Single-letter syllables (a, e, o, n, r, ...) and the
            // interjections (m, n, ng, hm, hng) are excluded from
            // segmentation: they would otherwise produce spurious splits like
            // `xi a n`, `gu o`, or `xi ng2` for `xing2`. Standalone queries
            // for these still reach the raw (unsegmented) path.
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
            let head = &part[pos..end];
            for rest in suffix_forms(part, end, memo) {
                forms.push(if rest.is_empty() {
                    head.to_owned()
                } else {
                    format!("{head} {rest}")
                });
            }
        }
        forms.sort_by(|a, b| syllable_count_order(a, b));
        forms.dedup();
        forms.truncate(MAX_SEGMENTATIONS);
    }
    memo[pos] = Some(forms.clone());
    forms
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
        // Syllabic nasals: 嗯 ń / ň / ǹ, 呣 ḿ.
        'ń' => Some(('n', 2)),
        'ň' => Some(('n', 3)),
        'ǹ' => Some(('n', 4)),
        'ḿ' => Some(('m', 2)),
        _ => None,
    }
}

/// Folds a vowel followed by a combining tone mark (decomposed text, as some
/// keyboards and copy sources produce: `u` + U+0301) into the single
/// precomposed character (`ú`), so both spellings read the same.
pub(crate) fn compose(input: &str) -> std::borrow::Cow<'_, str> {
    if !input
        .chars()
        .any(|c| ('\u{0300}'..='\u{030C}').contains(&c))
    {
        return std::borrow::Cow::Borrowed(input);
    }
    let mut out: Vec<char> = Vec::with_capacity(input.len());
    for c in input.chars() {
        let combined = out.last().and_then(|&base| combine(base, c));
        match combined {
            Some(composed) => {
                out.pop();
                out.push(composed);
            }
            None => out.push(c),
        }
    }
    std::borrow::Cow::Owned(out.into_iter().collect())
}

fn combine(base: char, mark: char) -> Option<char> {
    if mark == '\u{0308}' {
        return match base {
            'u' => Some('ü'),
            'U' => Some('Ü'),
            _ => None,
        };
    }
    let tone = match mark {
        '\u{0304}' => 0,
        '\u{0301}' => 1,
        '\u{030C}' => 2,
        '\u{0300}' => 3,
        _ => return None,
    };
    let row: [char; 4] = match base {
        'a' => ['ā', 'á', 'ǎ', 'à'],
        'e' => ['ē', 'é', 'ě', 'è'],
        'i' => ['ī', 'í', 'ǐ', 'ì'],
        'o' => ['ō', 'ó', 'ǒ', 'ò'],
        'u' => ['ū', 'ú', 'ǔ', 'ù'],
        'ü' => ['ǖ', 'ǘ', 'ǚ', 'ǜ'],
        'A' => ['Ā', 'Á', 'Ǎ', 'À'],
        'E' => ['Ē', 'É', 'Ě', 'È'],
        'I' => ['Ī', 'Í', 'Ǐ', 'Ì'],
        'O' => ['Ō', 'Ó', 'Ǒ', 'Ò'],
        'U' => ['Ū', 'Ú', 'Ǔ', 'Ù'],
        'Ü' => ['Ǖ', 'Ǘ', 'Ǚ', 'Ǜ'],
        _ => return None,
    };
    row.get(tone).copied()
}

#[cfg(test)]
mod tests {
    use super::{normalize, segment};

    #[test]
    fn segmenting_long_input_is_fast_and_capped() {
        let long = "xian".repeat(30);
        let started = std::time::Instant::now();
        let forms = segment(&long);
        assert!(
            started.elapsed().as_millis() < 500,
            "{:?}",
            started.elapsed()
        );
        assert!(!forms.is_empty() && forms.len() <= super::MAX_SEGMENTATIONS);
        assert!(forms.contains(&vec!["xian"; 30].join(" ")));
    }

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
        assert_eq!(segment("   "), [] as [String; 0]);
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
