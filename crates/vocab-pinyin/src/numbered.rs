//! Tone-marked pinyin → numbered pinyin, the form Shouci stores.
//!
//! [`crate::normalize`] converts marks syllable by syllable, so it needs the
//! syllables already separated: `lǚ xíng` works, `lǚxíng` loses a tone.
//! [`numbered`] splits first, using the marks to choose between splits.

use crate::{marked_tone, normalize, segment};

/// Pinyin with tone marks, written the way CC-CEDICT writes it: tone numbers
/// and one space between syllables. `lǚxíng` → `lv3 xing2`,
/// `Běijīng` → `bei3 jing1`, `xī'ān` → `xi1 an1`, `māma` → `ma1 ma`.
///
/// Splitting follows pinyin spelling: a syllable carries at most one mark,
/// and an apostrophe is a boundary (`fāngàn` is fān gàn; 方案 is written
/// `fāng'àn`).
///
/// Input without tone marks (already numbered, untoned, or not pinyin) comes
/// back trimmed and otherwise unchanged. Marked input that is not pinyin
/// falls back to [`normalize`].
#[must_use]
pub fn numbered(input: &str) -> String {
    let trimmed = input.trim();
    let lowered = trimmed.to_lowercase();
    if !lowered.chars().any(|c| marked_tone(c).is_some()) {
        return trimmed.to_owned();
    }
    // The text with marks removed, and the tone (if any) of each letter.
    let mut plain = String::with_capacity(lowered.len());
    let mut tones: Vec<Option<u8>> = Vec::new();
    for c in lowered.chars() {
        if c.is_whitespace() {
            if !plain.ends_with(' ') && !plain.is_empty() {
                plain.push(' ');
            }
            continue;
        }
        let (base, tone) = match c {
            'ü' => ('v', None),
            c => marked_tone(c).map_or((c, None), |(base, tone)| (base, Some(tone))),
        };
        plain.push(base);
        if base != '\'' {
            tones.push(tone);
        }
    }
    // Fewest syllables first: `xue xiao`, not `xue xi ao`.
    let mut variants = segment(plain.trim_end());
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
                [] => syllables.push(syllable.to_owned()),
                [tone] => syllables.push(format!("{syllable}{tone}")),
                _ => continue 'variants,
            }
        }
        if at == tones.len() {
            return syllables.join(" ");
        }
    }
    normalize(trimmed).as_str().to_owned()
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
    fn spaced_and_apostrophes_and_neutral_tones() {
        assert_eq!(numbered("lǚ xíng"), "lv3 xing2");
        assert_eq!(numbered("xī'ān"), "xi1 an1");
        assert_eq!(numbered("māma"), "ma1 ma");
        assert_eq!(
            numbered("fāngàn"),
            "fan1 gan4",
            "spelling rules, not guessing"
        );
        assert_eq!(numbered("fāng'àn"), "fang1 an4");
    }

    #[test]
    fn unmarked_input_is_left_as_written() {
        assert_eq!(numbered(" xue2 xiao4 "), "xue2 xiao4");
        assert_eq!(numbered("nihao"), "nihao");
        assert_eq!(numbered("lu:3"), "lu:3");
        assert_eq!(numbered(""), "");
    }

    #[test]
    fn marked_text_that_is_not_pinyin_falls_back() {
        assert_eq!(numbered("café"), "cafe2");
    }
}
