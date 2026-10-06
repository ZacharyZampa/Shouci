//! What an imported record becomes, given what the dictionaries say.
//!
//! - One dictionary entry with the record's characters (narrowed by its
//!   reading and traditional form, when the record has them) → confirmed,
//!   with the dictionary's forms and reading.
//! - No entry has the record's reading, but exactly one differs from it only
//!   where the record has a neutral tone → that entry (`deng3deng5` for 等等,
//!   which CC-CEDICT reads `deng3 deng3`). Only the record's neutral tones
//!   give way: `xie4 xie4` never becomes `xie4 xie5`.
//! - The record's traditional form is no entry's, and exactly one entry has
//!   its characters and reading → that entry; the record wrote it as a
//!   variant (`週圍` for 周圍). When several entries fit, or another entry is
//!   written that way, it may be a different word: needs review.
//! - No entry, but the record has both a reading and a definition →
//!   confirmed from the file.
//! - Anything else (several entries, nothing to go on) → needs review.
//!   Ambiguity is never resolved by guessing.
//!
//! When an entry is used although the record wrote the word differently, the
//! record's own forms are kept in [`AsWritten`]: the plan reports the
//! difference, and still finds a word saved the way the file writes it.
//!
//! A definition the record gives is always kept as written (Pleco's rule
//! too); the dictionary's fills in only a missing one.

use vocab_core::connector::ExchangeRecord;
use vocab_core::{DictionaryEntry, Verification};
use vocab_pinyin::{numbered, tone_marks};

use crate::reading_key;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Resolved {
    pub simplified: String,
    /// Empty when unknown: the file did not say and no single entry did.
    pub traditional: String,
    pub pinyin: String,
    pub definition: String,
    pub notes: String,
    pub verification: Verification,
    /// Set when a dictionary entry was used although the record writes the
    /// word differently.
    pub as_written: Option<AsWritten>,
}

/// The record's forms, where the dictionary's replaced them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AsWritten {
    pub traditional: String,
    pub pinyin: String,
    /// What changed, for people: `the file writes 週圍; the dictionary's
    /// 周圍 is used`.
    pub note: String,
}

/// `Err` carries why the record cannot be imported at all.
pub(crate) fn resolve(
    record: &ExchangeRecord,
    hits: &[DictionaryEntry],
) -> Result<Resolved, String> {
    let headword = record.headword.trim();
    if headword.is_empty() {
        return Err("record has no headword".to_owned());
    }
    let exact: Vec<&DictionaryEntry> = hits
        .iter()
        .filter(|hit| hit.simplified == headword || hit.traditional == headword)
        .collect();
    let by_reading: Vec<&DictionaryEntry> = match record.pinyin.text() {
        Some(reading) => {
            let key = reading_key(reading);
            let same: Vec<&DictionaryEntry> = exact
                .iter()
                .copied()
                .filter(|hit| reading_key(&hit.pinyin) == key)
                .collect();
            if same.is_empty() {
                let near: Vec<&DictionaryEntry> = exact
                    .iter()
                    .copied()
                    .filter(|hit| neutral_tones_fit(&key, &reading_key(&hit.pinyin)))
                    .collect();
                if near.len() == 1 { near } else { Vec::new() }
            } else {
                same
            }
        }
        None => exact.clone(),
    };
    let candidates: Vec<&DictionaryEntry> = match record.traditional.text() {
        Some(traditional) => {
            let same: Vec<&DictionaryEntry> = by_reading
                .iter()
                .copied()
                .filter(|hit| hit.traditional == traditional)
                .collect();
            let variant =
                by_reading.len() == 1 && !exact.iter().any(|hit| hit.traditional == traditional);
            if same.is_empty() && !by_reading.is_empty() && !variant {
                return Ok(from_file(record, headword, Verification::NeedsReview));
            }
            if same.is_empty() { by_reading } else { same }
        }
        None => by_reading,
    };
    if let [hit] = candidates.as_slice() {
        return Ok(from_hit(record, headword, hit));
    }
    let complete = record.pinyin.text().is_some() && record.definition.text().is_some();
    Ok(if candidates.is_empty() && complete {
        from_file(record, headword, Verification::Confirmed)
    } else {
        from_file(record, headword, Verification::NeedsReview)
    })
}

/// Whether an entry read `entry` is the word a record read `record`, where
/// the record leaves some tones neutral: `deng3deng` (等等 written
/// `deng3deng5`) fits `deng3deng3`. Both are reading keys, which drop the
/// neutral tone's `5`, so each syllable's letters must match and the entry
/// may carry a tone wherever the record has none.
fn neutral_tones_fit(record: &str, entry: &str) -> bool {
    let mut record = record.chars().peekable();
    for c in entry.chars() {
        if record.peek() == Some(&c) {
            record.next();
        } else if !c.is_ascii_digit() {
            return false;
        }
    }
    record.next().is_none()
}

/// A definition that says the same as the dictionary: all its glosses joined,
/// or exactly one of them.
pub(crate) fn definition_matches(definition: &str, glosses: &[String]) -> bool {
    let definition = definition.trim();
    !definition.is_empty()
        && (definition == glosses.join("; ").trim()
            || glosses.iter().any(|gloss| gloss.trim() == definition))
}

fn from_hit(record: &ExchangeRecord, headword: &str, hit: &DictionaryEntry) -> Resolved {
    Resolved {
        simplified: hit.simplified.clone(),
        traditional: hit.traditional.clone(),
        pinyin: hit.pinyin.clone(),
        definition: record
            .definition
            .text()
            .map_or_else(|| hit.glosses.join("; "), str::to_owned),
        notes: record.notes.text().unwrap_or_default().to_owned(),
        verification: Verification::Confirmed,
        as_written: as_written(record, headword, hit),
    }
}

/// The record's forms and what differs, when the record writes `hit`'s word
/// another way than the entry: a variant traditional form, or a neutral tone
/// where the entry has a full one. Spacing and how tones are written are not
/// differences.
fn as_written(record: &ExchangeRecord, headword: &str, hit: &DictionaryEntry) -> Option<AsWritten> {
    let traditional = record
        .traditional
        .text()
        .filter(|traditional| *traditional != hit.traditional);
    let reading = record
        .pinyin
        .text()
        .filter(|reading| reading_key(reading) != reading_key(&hit.pinyin));
    if traditional.is_none() && reading.is_none() {
        return None;
    }
    let mut changes = Vec::new();
    if let Some(traditional) = traditional {
        changes.push(format!(
            "the file writes {traditional}; the dictionary's {} is used",
            hit.traditional
        ));
    }
    if let Some(reading) = reading {
        changes.push(format!(
            "the file reads {headword} {}; the dictionary's {} is used",
            tone_marks(&numbered(reading)),
            tone_marks(&hit.pinyin)
        ));
    }
    Some(AsWritten {
        traditional: traditional.unwrap_or(&hit.traditional).to_owned(),
        pinyin: record.pinyin.text().unwrap_or(&hit.pinyin).to_owned(),
        note: changes.join("; "),
    })
}

fn from_file(record: &ExchangeRecord, headword: &str, verification: Verification) -> Resolved {
    Resolved {
        simplified: headword.to_owned(),
        traditional: record.traditional.text().unwrap_or_default().to_owned(),
        pinyin: record.pinyin.text().unwrap_or_default().to_owned(),
        definition: record.definition.text().unwrap_or_default().to_owned(),
        notes: record.notes.text().unwrap_or_default().to_owned(),
        verification,
        as_written: None,
    }
}

#[cfg(test)]
mod tests {
    use vocab_core::connector::{ExchangeRecord, Field};
    use vocab_core::{DictionaryEntry, SourceId, SourceVersion, Verification};

    use super::{definition_matches, neutral_tones_fit, resolve};

    fn hit(id: i64, traditional: &str, pinyin: &str, gloss: &str) -> DictionaryEntry {
        DictionaryEntry {
            source: SourceId("cc-cedict".to_owned()),
            source_version: SourceVersion("1".to_owned()),
            entry_id: id,
            simplified: "后".to_owned(),
            traditional: traditional.to_owned(),
            pinyin: pinyin.to_owned(),
            glosses: vec![gloss.to_owned()],
            frequency_rank: None,
            hsk_rank: None,
        }
    }

    fn record(pinyin: Field<String>, definition: Field<String>) -> ExchangeRecord {
        let mut record = ExchangeRecord::new(1, "后", "后");
        record.pinyin = pinyin;
        record.definition = definition;
        record
    }

    fn present(value: &str) -> Field<String> {
        Field::Present(value.to_owned())
    }

    fn hou() -> Vec<DictionaryEntry> {
        vec![
            hit(1, "后", "Hou4", "surname Hou"),
            hit(2, "後", "hou4", "back; behind"),
        ]
    }

    #[test]
    fn unique_hit_fills_an_omitted_definition() {
        let resolved = resolve(
            &record(present("Hou4"), Field::Omitted),
            &[hit(1, "后", "Hou4", "surname Hou")],
        )
        .unwrap();
        assert_eq!(resolved.verification, Verification::Confirmed);
        assert_eq!(resolved.definition, "surname Hou");
    }

    #[test]
    fn several_hits_are_not_picked() {
        let resolved = resolve(&record(present("hou4"), Field::Omitted), &hou()).unwrap();
        assert_eq!(resolved.verification, Verification::NeedsReview);
        assert_eq!(resolved.traditional, "", "unknown, not guessed");
        assert_eq!(resolved.definition, "", "no invented definition");
    }

    #[test]
    fn the_traditional_form_picks_between_hits() {
        let mut with_traditional = record(present("hou4"), Field::Omitted);
        with_traditional.traditional = present("後");
        let resolved = resolve(&with_traditional, &hou()).unwrap();
        assert_eq!(resolved.verification, Verification::Confirmed);
        assert_eq!(resolved.definition, "back; behind");
    }

    #[test]
    fn a_traditional_form_no_entry_has_needs_review() {
        let mut odd = record(present("hou4"), Field::Omitted);
        odd.traditional = present("厚");
        let resolved = resolve(&odd, &hou()).unwrap();
        assert_eq!(resolved.verification, Verification::NeedsReview);
    }

    #[test]
    fn complete_record_with_no_hit_is_confirmed() {
        let resolved = resolve(&record(present("hou4"), present("a custom gloss")), &[]).unwrap();
        assert_eq!(resolved.verification, Verification::Confirmed);
        assert_eq!(resolved.definition, "a custom gloss");
    }

    #[test]
    fn a_given_definition_is_kept_as_written() {
        let mut one = record(present("Hou4"), present("surname Hou"));
        one.traditional = present("后");
        let resolved = resolve(&one, &[hit(1, "后", "Hou4", "surname Hou")]).unwrap();
        assert_eq!(resolved.definition, "surname Hou");
        assert!(definition_matches("to walk", &["to walk".to_owned()]));
    }

    #[test]
    fn blank_headword_is_dropped() {
        let record = ExchangeRecord::new(1, "", " ");
        assert!(resolve(&record, &[]).is_err());
    }

    fn entry(simplified: &str, traditional: &str, pinyin: &str, gloss: &str) -> DictionaryEntry {
        DictionaryEntry {
            simplified: simplified.to_owned(),
            ..hit(1, traditional, pinyin, gloss)
        }
    }

    fn pleco_card(simplified: &str, traditional: &str, pinyin: &str) -> ExchangeRecord {
        let mut record = ExchangeRecord::new(1, simplified, simplified);
        record.traditional = present(traditional);
        record.pinyin = present(pinyin);
        record
    }

    #[test]
    fn a_variant_traditional_form_is_the_one_entry_it_could_be() {
        let around = [entry("周围", "周圍", "zhou1 wei2", "surroundings")];
        let resolved = resolve(&pleco_card("周围", "週圍", "zhou1wei2"), &around).unwrap();
        assert_eq!(resolved.verification, Verification::Confirmed);
        assert_eq!(resolved.traditional, "周圍");
        assert_eq!(resolved.definition, "surroundings");
        let written = resolved.as_written.expect("the difference is kept");
        assert_eq!(written.traditional, "週圍");
        assert_eq!(
            written.note,
            "the file writes 週圍; the dictionary's 周圍 is used"
        );
    }

    #[test]
    fn a_traditional_form_another_entry_has_is_a_different_word() {
        // 乾 (dry) and 幹 (to do) are both 干. A card 干[幹] read gan1 fits
        // 乾 by its reading but 幹 by its form: not a variant, but a mistake
        // to look at.
        let dry_or_do = [
            entry("干", "乾", "gan1", "dry"),
            entry("干", "幹", "gan4", "to do"),
        ];
        let resolved = resolve(&pleco_card("干", "幹", "gan1"), &dry_or_do).unwrap();
        assert_eq!(resolved.verification, Verification::NeedsReview);
        assert_eq!(resolved.traditional, "幹", "kept as written");
    }

    #[test]
    fn a_neutral_tone_gives_way_to_the_one_entry_it_could_be() {
        let etc = [entry("等等", "等等", "deng3 deng3", "etc.")];
        let resolved = resolve(&pleco_card("等等", "等等", "deng3deng5"), &etc).unwrap();
        assert_eq!(resolved.verification, Verification::Confirmed);
        assert_eq!(resolved.pinyin, "deng3 deng3");
        let written = resolved.as_written.expect("the difference is kept");
        assert_eq!(written.pinyin, "deng3deng5");
        assert_eq!(
            written.note,
            "the file reads 等等 děngdeng; the dictionary's děng děng is used"
        );
    }

    #[test]
    fn only_the_record_s_neutral_tones_give_way() {
        let thanks = [entry("谢谢", "謝謝", "xie4 xie5", "thanks")];
        let resolved = resolve(&pleco_card("谢谢", "謝謝", "xie4 xie4"), &thanks).unwrap();
        assert_eq!(resolved.verification, Verification::NeedsReview);
        assert_eq!(resolved.pinyin, "xie4 xie4");
        let things = [
            entry("东西", "東西", "dong1 xi1", "east and west"),
            entry("东西", "東西", "dong1 xi5", "thing"),
        ];
        let resolved = resolve(&pleco_card("东西", "東西", "dong1xi"), &things).unwrap();
        assert_eq!(resolved.pinyin, "dong1 xi5", "an exact reading wins");
        let resolved = resolve(&pleco_card("东西", "東西", "dongxi"), &things).unwrap();
        assert_eq!(resolved.verification, Verification::NeedsReview, "two fit");
    }

    #[test]
    fn neutral_tones_fit_only_where_the_record_has_none() {
        assert!(neutral_tones_fit("deng3deng", "deng3deng3"));
        assert!(neutral_tones_fit("xian", "xi1an1"));
        assert!(!neutral_tones_fit("xie4xie4", "xie4xie"));
        assert!(!neutral_tones_fit("ba2", "ba1"));
        assert!(!neutral_tones_fit("ba", "bai1"));
    }

    #[test]
    fn spacing_and_tone_style_are_not_differences() {
        let impression = [entry("印象", "印象", "yin4 xiang4", "impression")];
        let resolved = resolve(&pleco_card("印象", "印象", "yìnxiàng"), &impression).unwrap();
        assert_eq!(resolved.as_written, None);
    }
}
