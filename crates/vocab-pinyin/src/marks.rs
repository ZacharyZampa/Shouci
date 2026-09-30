//! Tone-number pinyin → tone-mark pinyin, for display only.
//!
//! Storage, search, and file exchange keep CC-CEDICT's numbered form
//! (`xue2 xiao4`); people read marks (`xué xiào`). This is the inverse of the
//! marking half of [`crate::normalize`], applied syllable by syllable.

/// Rewrites numbered pinyin with tone marks: `xue2 xiao4` → `xué xiào`,
/// `nu:3` → `nǚ`, `lu:e4` → `lüè`, `ma5` → `ma`, `r5` → `r`.
///
/// Placement follows the standard rule: `a` or `e` takes the mark; in `ou`
/// the `o` does; otherwise the last vowel. Case is preserved (`Bei3 jing1` →
/// `Běi jīng`). Anything that is not a letter run ending in a tone digit
/// passes through unchanged, as does a toned syllable with no vowel to mark
/// (`m2`, `ng4`), so no tone is silently lost.
#[must_use]
pub fn tone_marks(numbered: &str) -> String {
    let mut out = String::with_capacity(numbered.len() + 8);
    let mut syllable = String::new();
    for ch in numbered.chars() {
        if ch.is_ascii_alphabetic() || ch == ':' || ch == 'ü' || ch == 'Ü' {
            syllable.push(ch);
            continue;
        }
        if let Some(tone) = ch.to_digit(10).filter(|d| (1..=5).contains(d))
            && !syllable.is_empty()
        {
            if let Some(marked) = mark_syllable(&syllable, tone) {
                out.push_str(&marked);
            } else {
                out.push_str(&syllable);
                out.push(ch);
            }
            syllable.clear();
            continue;
        }
        out.push_str(&untoned(&syllable));
        syllable.clear();
        out.push(ch);
    }
    out.push_str(&untoned(&syllable));
    out
}

/// A letter run with no tone digit. Only the unambiguous `u:` becomes `ü`;
/// a bare `v` may be a Latin letter in the entry (`V ling3`, V-neck).
fn untoned(run: &str) -> String {
    run.replace("u:", "ü").replace("U:", "Ü")
}

/// CC-CEDICT spells ü as `u:`; some sources type it `v`.
fn umlaut(syllable: &str) -> String {
    syllable
        .replace("u:", "ü")
        .replace("U:", "Ü")
        .replace('v', "ü")
        .replace('V', "Ü")
}

fn mark_syllable(syllable: &str, tone: u32) -> Option<String> {
    let letters: Vec<char> = umlaut(syllable).chars().collect();
    let vowel = |c: char| "aeiouüAEIOUÜ".contains(c);
    if !letters.iter().copied().any(vowel) {
        // Erhua `r5` is the one vowel-less neutral syllable to write bare.
        return (tone == 5 && syllable.eq_ignore_ascii_case("r")).then(|| syllable.to_owned());
    }
    if tone == 5 {
        return Some(letters.into_iter().collect());
    }
    let lower: Vec<char> = letters
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect();
    let target = lower
        .iter()
        .position(|&c| c == 'a' || c == 'e')
        .or_else(|| lower.windows(2).position(|pair| pair == ['o', 'u']))
        .or_else(|| lower.iter().rposition(|&c| vowel(c)))?;
    let marked: String = letters
        .iter()
        .enumerate()
        .map(|(i, &c)| if i == target { with_tone(c, tone) } else { c })
        .collect();
    Some(marked)
}

fn with_tone(vowel: char, tone: u32) -> char {
    let row: [char; 4] = match vowel {
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
        other => return other,
    };
    usize::try_from(tone - 1)
        .ok()
        .and_then(|i| row.get(i).copied())
        .unwrap_or(vowel)
}

#[cfg(test)]
mod tests {
    use super::tone_marks;

    #[test]
    fn marks_each_syllable() {
        assert_eq!(tone_marks("xue2 xiao4"), "xué xiào");
        assert_eq!(tone_marks("ni3 hao3"), "nǐ hǎo");
        assert_eq!(
            tone_marks("Zhong1 hua2 ren2 min2 gong4 he2 guo2"),
            "Zhōng huá rén mín gòng hé guó"
        );
    }

    #[test]
    fn follows_placement_rules() {
        assert_eq!(tone_marks("hao3"), "hǎo", "a wins");
        assert_eq!(tone_marks("xie4"), "xiè", "e wins");
        assert_eq!(tone_marks("dou1"), "dōu", "o in ou");
        assert_eq!(tone_marks("gui4"), "guì", "last vowel");
        assert_eq!(tone_marks("liu2"), "liú", "last vowel");
        assert_eq!(tone_marks("zhuo1"), "zhuō", "last vowel");
    }

    #[test]
    fn handles_u_umlaut_spellings() {
        assert_eq!(tone_marks("nu:3"), "nǚ");
        assert_eq!(tone_marks("lu:e4"), "lüè");
        assert_eq!(tone_marks("lv4"), "lǜ");
        assert_eq!(tone_marks("nu:3 ren2"), "nǚ rén");
    }

    #[test]
    fn neutral_tone_and_erhua_drop_the_digit() {
        assert_eq!(tone_marks("ma5"), "ma");
        assert_eq!(tone_marks("wan2 r5"), "wán r");
        assert_eq!(tone_marks("de5"), "de");
    }

    #[test]
    fn keeps_what_it_cannot_mark() {
        assert_eq!(tone_marks("m2"), "m2", "no vowel: keep the tone number");
        assert_eq!(tone_marks("ng4"), "ng4");
        assert_eq!(tone_marks("xx5"), "xx5");
        assert_eq!(
            tone_marks("A A zhi4"),
            "A A zhì",
            "letters without tones pass"
        );
        assert_eq!(tone_marks("V ling3"), "V lǐng", "a Latin V stays a V");
        assert_eq!(tone_marks("Xi1 an1 , ha1"), "Xī ān , hā");
        assert_eq!(tone_marks(""), "");
    }

    #[test]
    fn preserves_case() {
        assert_eq!(tone_marks("Bei3 jing1"), "Běi jīng");
        assert_eq!(tone_marks("E2 guo2"), "É guó");
        assert_eq!(tone_marks("Ou1 zhou1"), "Ōu zhōu");
    }
}
