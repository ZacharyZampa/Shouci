//! What an imported record becomes, given what the dictionaries say.
//!
//! - One dictionary entry with the record's characters (narrowed by its
//!   reading and traditional form, when the record has them) → confirmed,
//!   with the dictionary's forms and reading.
//! - The record's traditional form matches none of the entries → needs
//!   review.
//! - No entry, but the record has both a reading and a definition →
//!   confirmed from the file.
//! - Anything else (several entries, nothing to go on) → needs review.
//!   Ambiguity is never resolved by guessing.
//!
//! A definition the record gives is always kept as written (Pleco's rule
//! too); the dictionary's fills in only a missing one.

use vocab_core::connector::ExchangeRecord;
use vocab_core::{DictionaryEntry, Verification};

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
    let exact = hits
        .iter()
        .filter(|hit| hit.simplified == headword || hit.traditional == headword);
    let by_reading: Vec<&DictionaryEntry> = match record.pinyin.text() {
        Some(reading) => {
            let key = reading_key(reading);
            exact
                .filter(|hit| reading_key(&hit.pinyin) == key)
                .collect()
        }
        None => exact.collect(),
    };
    let candidates: Vec<&DictionaryEntry> = match record.traditional.text() {
        Some(traditional) => {
            let same: Vec<&DictionaryEntry> = by_reading
                .iter()
                .copied()
                .filter(|hit| hit.traditional == traditional)
                .collect();
            if same.is_empty() && !by_reading.is_empty() {
                return Ok(from_file(record, headword, Verification::NeedsReview));
            }
            same
        }
        None => by_reading,
    };
    if let [hit] = candidates.as_slice() {
        return Ok(from_hit(record, hit));
    }
    let complete = record.pinyin.text().is_some() && record.definition.text().is_some();
    Ok(if candidates.is_empty() && complete {
        from_file(record, headword, Verification::Confirmed)
    } else {
        from_file(record, headword, Verification::NeedsReview)
    })
}

/// A definition that says the same as the dictionary: all its glosses joined,
/// or exactly one of them.
pub(crate) fn definition_matches(definition: &str, glosses: &[String]) -> bool {
    let definition = definition.trim();
    !definition.is_empty()
        && (definition == glosses.join("; ").trim()
            || glosses.iter().any(|gloss| gloss.trim() == definition))
}

fn from_hit(record: &ExchangeRecord, hit: &DictionaryEntry) -> Resolved {
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
    }
}

fn from_file(record: &ExchangeRecord, headword: &str, verification: Verification) -> Resolved {
    Resolved {
        simplified: headword.to_owned(),
        traditional: record.traditional.text().unwrap_or_default().to_owned(),
        pinyin: record.pinyin.text().unwrap_or_default().to_owned(),
        definition: record.definition.text().unwrap_or_default().to_owned(),
        notes: record.notes.text().unwrap_or_default().to_owned(),
        verification,
    }
}

#[cfg(test)]
mod tests {
    use vocab_core::connector::{ExchangeRecord, Field};
    use vocab_core::{DictionaryEntry, SourceId, SourceVersion, Verification};

    use super::{definition_matches, resolve};

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
}
