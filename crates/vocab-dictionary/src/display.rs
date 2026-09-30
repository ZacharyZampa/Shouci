//! CC-CEDICT definition text → text for people, for display only.
//!
//! CC-CEDICT writes cross-references as `traditional|simplified[pin1 yin1]`
//! and measure words as `CL:個|个[ge4],張|张[zhang1]`. Stored definitions and
//! Pleco/Anki exports keep that notation; screens show
//! `see 坊子区 (Fāng zǐ Qū)` and `measure words: 个 (gè), 张 (zhāng)`.

use vocab_pinyin::tone_marks;

/// Rewrites CC-CEDICT notation in a definition (one gloss, or several
/// joined with `; `):
///
/// - `坊子區|坊子区[Fang1 zi3 Qu1]` → `坊子区 (Fāng zǐ Qū)`: simplified only,
///   reading with tone marks;
/// - `阪[ban3]` → `阪 (bǎn)`;
/// - `臉型|脸型` (no reading) → `脸型`;
/// - `CL:份[fen4],個|个[ge4]` → `measure words: 份 (fèn), 个 (gè)`.
///
/// Everything else passes through unchanged, including brackets that are
/// not a reading after Chinese text (`[sic]`, `[2]`).
#[must_use]
pub fn display_definition(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = find_classifiers(rest) {
        out.push_str(&references(&rest[..at]));
        let list = &rest[at + 3..];
        let end = list_end(list);
        let items: Vec<String> = split_items(&list[..end])
            .into_iter()
            .map(|item| references(item.trim()))
            .filter(|item| !item.is_empty())
            .collect();
        match items.len() {
            0 => out.push_str("CL:"),
            1 => out.push_str("measure word: "),
            _ => out.push_str("measure words: "),
        }
        out.push_str(&items.join(", "));
        rest = &list[end..];
    }
    out.push_str(&references(rest));
    out
}

/// Byte offset of a `CL:` that starts a measure-word list: at the start, or
/// after a space, `(`, or `;`.
fn find_classifiers(text: &str) -> Option<usize> {
    text.match_indices("CL:").map(|(at, _)| at).find(|&at| {
        text[..at]
            .chars()
            .next_back()
            .is_none_or(|c| c == ' ' || c == '(' || c == ';')
    })
}

/// Where a `CL:` list ends: the first `)`, `;`, or space outside a reading.
fn list_end(list: &str) -> usize {
    let mut in_reading = false;
    for (at, c) in list.char_indices() {
        match c {
            '[' => in_reading = true,
            ']' => in_reading = false,
            ')' | ';' => return at,
            c if c.is_whitespace() && !in_reading => return at,
            _ => {}
        }
    }
    list.len()
}

/// Splits a `CL:` list on the commas between items, not inside readings.
fn split_items(list: &str) -> Vec<&str> {
    let mut items = Vec::new();
    let mut start = 0;
    let mut in_reading = false;
    for (at, c) in list.char_indices() {
        match c {
            '[' => in_reading = true,
            ']' => in_reading = false,
            ',' if !in_reading => {
                items.push(&list[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    items.push(&list[start..]);
    items
}

fn is_cjk(c: char) -> bool {
    c >= '\u{2E80}'
}

/// Word boundaries around references. Anything else, including `|`, can be
/// part of a headword.
fn is_delimiter(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            ',' | ';' | ':' | '(' | ')' | '"' | '“' | '”' | '，' | '；' | '（' | '）' | '、'
        )
}

/// A bracketed CC-CEDICT reading: letters (Latin, or the odd `γ`), spaces,
/// and at least one tone digit (`Fang1 zi3 Qu1`, `nu:3`, `γ she4 xian4`).
fn is_reading(inner: &str) -> bool {
    !inner.is_empty()
        && inner.chars().any(|c| matches!(c, '1'..='5'))
        && inner.chars().all(|c| {
            (c.is_alphanumeric() && !is_cjk(c)) || matches!(c, ' ' | ':' | '·' | ',' | '-')
        })
}

/// The simplified half of `traditional|simplified`; the word itself when it
/// has no `|`.
fn simplified(word: &str) -> &str {
    word.rsplit('|').next().unwrap_or(word)
}

/// A finished word with no reading: `臉型|脸型` becomes `脸型` when both
/// halves are Chinese; anything else stays as written.
fn plain_word(word: &str) -> &str {
    match word.split_once('|') {
        Some((left, right)) if left.chars().any(is_cjk) && right.chars().any(is_cjk) => {
            simplified(word)
        }
        _ => word,
    }
}

fn references(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() + 16);
    let mut word = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '['
            && let Some(len) = chars[i + 1..].iter().position(|&c| c == ']')
        {
            let inner: String = chars[i + 1..i + 1 + len].iter().collect();
            if word.chars().any(is_cjk) && is_reading(&inner) {
                out.push_str(simplified(&word));
                out.push_str(" (");
                out.push_str(&tone_marks(&inner));
                out.push(')');
                word.clear();
                i += len + 2;
                continue;
            }
            // A reading on its own (`Taiwan pr. [ling4]`) keeps its brackets
            // and gains tone marks. Cantonese Jyutping is not pinyin: leave it.
            if word.is_empty() && is_reading(&inner) && !out.trim_end().ends_with("Jyutping") {
                out.push('[');
                out.push_str(&tone_marks(&inner));
                out.push(']');
                i += len + 2;
                continue;
            }
        }
        if c == '[' || c == ']' || is_delimiter(c) {
            out.push_str(plain_word(&word));
            word.clear();
            out.push(c);
        } else {
            word.push(c);
        }
        i += 1;
    }
    out.push_str(plain_word(&word));
    out
}

#[cfg(test)]
mod tests {
    use super::display_definition;

    #[test]
    fn references_show_simplified_with_tone_marks() {
        assert_eq!(
            display_definition("see 坊子區|坊子区[Fang1 zi3 Qu1]"),
            "see 坊子区 (Fāng zǐ Qū)"
        );
        assert_eq!(
            display_definition("variant of 阪[ban3]"),
            "variant of 阪 (bǎn)"
        );
        assert_eq!(
            display_definition("amniocentesis (abbr. for 羊膜穿刺[yang2 mo2 chuan1 ci4])"),
            "amniocentesis (abbr. for 羊膜穿刺 (yáng mó chuān cì))"
        );
        assert_eq!(
            display_definition(
                "Longhai, a district of Zhangzhou City 漳州市[Zhang1 zhou1 Shi4], Fujian"
            ),
            "Longhai, a district of Zhangzhou City 漳州市 (Zhāng zhōu Shì), Fujian"
        );
    }

    #[test]
    fn pairs_without_a_reading_keep_the_simplified_form() {
        assert_eq!(
            display_definition("variant of 臉型|脸型, shape of face"),
            "variant of 脸型, shape of face"
        );
        assert_eq!(
            display_definition("General Lü Meng 呂蒙|吕蒙 of the southern state of Wu"),
            "General Lü Meng 吕蒙 of the southern state of Wu"
        );
    }

    #[test]
    fn measure_words_are_named_and_listed() {
        assert_eq!(display_definition("CL:把[ba3]"), "measure word: 把 (bǎ)");
        assert_eq!(
            display_definition("CL:份[fen4],個|个[ge4]"),
            "measure words: 份 (fèn), 个 (gè)"
        );
        assert_eq!(
            display_definition("a saw (CL:把[ba3])"),
            "a saw (measure word: 把 (bǎ))"
        );
        assert_eq!(
            display_definition(
                "cigarette (CL:支[zhi1],條|条[tiao2],根[gen1]) (variant of 香煙|香烟[xiang1 yan1])"
            ),
            "cigarette (measure words: 支 (zhī), 条 (tiáo), 根 (gēn)) (variant of 香烟 (xiāng yān))"
        );
    }

    #[test]
    fn works_on_joined_definitions() {
        assert_eq!(
            display_definition("house; building; room; CL:棟|栋[dong4],間|间[jian1]"),
            "house; building; room; measure words: 栋 (dòng), 间 (jiān)"
        );
    }

    #[test]
    fn standalone_readings_gain_marks_but_jyutping_does_not() {
        assert_eq!(
            display_definition("used in 令狐[Ling2 hu2] (Taiwan pr. [ling4])"),
            "used in 令狐 (Líng hú) (Taiwan pr. [lìng])"
        );
        assert_eq!(
            display_definition("variant of 岡|冈 [gang1]"),
            "variant of 冈 [gāng]"
        );
        assert_eq!(
            display_definition("borrowed from English \"ball\", Jyutping [bo1])"),
            "borrowed from English \"ball\", Jyutping [bo1])"
        );
        assert_eq!(
            display_definition("PM2.5, γ射線|γ射线[γ she4 xian4])"),
            "PM2.5, γ射线 (γ shè xiàn))"
        );
    }

    #[test]
    fn hyphenated_readings_are_marked() {
        assert_eq!(
            display_definition("also pr. [yi1mo2-yi1yang4]"),
            "also pr. [yīmó-yīyàng]"
        );
    }

    #[test]
    fn leaves_other_text_alone() {
        for text in [
            "prescription (medicine)",
            "a word [sic] in brackets",
            "footnote [2] here",
            "OK|KO but Latin",
            "ratio 3:1",
            "",
        ] {
            assert_eq!(display_definition(text), text);
        }
        assert_eq!(
            display_definition("UNCLE[ge4]"),
            "UNCLE[ge4]",
            "attached readings need Chinese"
        );
    }
}
