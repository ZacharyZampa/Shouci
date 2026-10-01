//! Tone-marked pinyin → numbered pinyin, the form Shouci stores.
//!
//! [`crate::normalize`] converts marks syllable by syllable, so it needs the
//! syllables already separated: `lǚ xíng` works, `lǚxíng` loses a tone.
//! [`numbered`] splits first, using the marks to choose between splits.

use crate::{compose, marked_tone, segment};

/// Pinyin with tone marks, written the way CC-CEDICT writes it: tone numbers,
/// one space between syllables, `5` for the neutral tone, and erhua as its
/// own `r5` syllable.
///
/// - `lǚxíng` → `lv3 xing2`, `Běijīng` → `bei3 jing1`, `xī'ān` → `xi1 an1`
/// - `māma` → `ma1 ma5` (a syllable without a mark is neutral)
/// - `nǎr` → `na3 r5`, `yīxiàr` → `yi1 xia4 r5`
///
/// Splitting follows pinyin spelling: a syllable carries at most one mark,
/// and an apostrophe is a boundary (`fāngàn` is fān gàn; 方案 is written
/// `fāng'àn`).
///
/// Each space-separated word is converted on its own. Input without tone
/// marks (already numbered, untoned, or not pinyin) comes back trimmed and
/// otherwise unchanged, and so does a marked word that is not pinyin
/// (`café`, `péngyou,`): nothing is ever converted lossily.
#[must_use]
pub fn numbered(input: &str) -> String {
    let composed = compose(input.trim());
    let has_marks = composed
        .chars()
        .any(|c| c.to_lowercase().any(|lower| marked_tone(lower).is_some()));
    if !has_marks {
        return composed.into_owned();
    }
    composed
        .split_whitespace()
        .map(|word| numbered_word(word).unwrap_or_else(|| word.to_owned()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// One word of marked (or, beside marked words, unmarked) pinyin, or `None`
/// when it does not spell pinyin.
fn numbered_word(word: &str) -> Option<String> {
    let lowered = word.to_lowercase();
    // The letters with marks removed, and the tone each letter carried.
    let mut plain = String::with_capacity(lowered.len());
    let mut tones: Vec<Option<u8>> = Vec::new();
    for c in lowered.chars() {
        let (base, tone) = match c {
            'ü' => ('v', None),
            c => marked_tone(c).map_or((c, None), |(base, tone)| (base, Some(tone))),
        };
        plain.push(base);
        if base != '\'' {
            tones.push(tone);
        }
    }
    if let Some(syllables) = split(&plain, &tones) {
        return Some(syllables.join(" "));
    }
    // Erhua: `nǎr`, `yīxiàr`. A final unmarked `r` is its own syllable.
    let stem = plain.strip_suffix('r')?;
    if tones.last() != Some(&None) {
        return None;
    }
    let mut syllables = split(stem, &tones[..tones.len() - 1])?;
    syllables.push("r5".to_owned());
    Some(syllables.join(" "))
}

/// Splits `plain` (marks removed) into syllables with tone numbers, using
/// `tones` (one per letter, apostrophes excluded) to pick the split: each
/// syllable may carry at most one mark. Fewest syllables first.
fn split(plain: &str, tones: &[Option<u8>]) -> Option<Vec<String>> {
    let mut variants = segment(plain);
    variants.sort_by_key(|variant| variant.split(' ').count());
    'variants: for variant in variants {
        let mut syllables = Vec::new();
        let mut at = 0;
        for syllable in variant.split(' ') {
            let letters = syllable.chars().count();
            let Some(span) = tones.get(at..at + letters) else {
                continue 'variants;
            };
            at += letters;
            let marks: Vec<u8> = span.iter().flatten().copied().collect();
            match marks.as_slice() {
                [] => syllables.push(format!("{syllable}5")),
                [tone] => syllables.push(format!("{syllable}{tone}")),
                _ => continue 'variants,
            }
        }
        if at == tones.len() {
            return Some(syllables);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::numbered;

    #[test]
    fn splits_unspaced_marked_pinyin() {
        assert_eq!(numbered("lǚxíng"), "lv3 xing2");
        assert_eq!(numbered("Běijīng"), "bei3 jing1");
        assert_eq!(numbered("xuéxiào"), "xue2 xiao4");
        assert_eq!(numbered("nǐhǎo"), "ni3 hao3");
    }

    #[test]
    fn spaced_and_apostrophes() {
        assert_eq!(numbered("lǚ xíng"), "lv3 xing2");
        assert_eq!(numbered("xī'ān"), "xi1 an1");
        assert_eq!(
            numbered("fāngàn"),
            "fan1 gan4",
            "spelling rules, not guessing"
        );
        assert_eq!(numbered("fāng'àn"), "fang1 an4");
    }

    #[test]
    fn neutral_tones_and_erhua_match_cedict() {
        assert_eq!(numbered("māma"), "ma1 ma5");
        assert_eq!(numbered("wǒmen"), "wo3 men5");
        assert_eq!(numbered("dōngxi"), "dong1 xi5");
        assert_eq!(numbered("nǎr"), "na3 r5");
        assert_eq!(numbered("yīxiàr"), "yi1 xia4 r5");
        assert_eq!(numbered("huār"), "hua1 r5");
    }

    #[test]
    fn decomposed_accents_read_like_composed() {
        assert_eq!(numbered("xue\u{301}xia\u{300}o"), "xue2 xiao4");
        assert_eq!(numbered("lu\u{308}\u{30c}"), "lv3");
    }

    #[test]
    fn unmarked_input_is_left_as_written() {
        assert_eq!(numbered(" xue2 xiao4 "), "xue2 xiao4");
        assert_eq!(numbered("nihao"), "nihao");
        assert_eq!(numbered("lu:3"), "lu:3");
        assert_eq!(numbered(""), "");
    }

    #[test]
    fn words_that_are_not_pinyin_are_kept_as_typed() {
        assert_eq!(numbered("café"), "café");
        assert_eq!(numbered("wǒmen péngyou,"), "wo3 men5 péngyou,");
    }

    #[test]
    fn long_input_stays_fast() {
        let long = "xiān".repeat(40);
        let started = std::time::Instant::now();
        let _ = numbered(&long);
        assert!(
            started.elapsed().as_millis() < 500,
            "{:?}",
            started.elapsed()
        );
    }
}
