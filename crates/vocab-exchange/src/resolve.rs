//! What an imported record becomes, given what the dictionaries say.
//!
//! - One dictionary entry with the record's characters (and reading, when
//!   the record has one) → confirmed, with the dictionary's forms and
//!   reading. The record's own definition wins when it differs.
//! - The record's traditional form disagrees with that entry → needs review.
//! - No entry, but the record has both a reading and a definition →
//!   confirmed from the file.
//! - Anything else (several entries, nothing to go on) → needs review.
//!   Ambiguity is never resolved by guessing.

use vocab_core::connector::ExchangeRecord;
use vocab_core::{DictionaryEntry, Verification};

use crate::reading_key;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Resolved {
    pub simplified: String,
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
    let narrowed: Vec<&DictionaryEntry> = match record.pinyin.text() {
        Some(reading) => {
            let key = reading_key(reading);
            exact
                .filter(|hit| reading_key(&hit.pinyin) == key)
                .collect()
        }
        None => exact.collect(),
    };
    if let [hit] = narrowed.as_slice() {
        let disagrees = record
            .traditional
            .text()
            .is_some_and(|traditional| traditional != hit.traditional);
        return Ok(if disagrees {
            from_file(record, headword, Verification::NeedsReview)
        } else {
            from_hit(record, hit)
        });
    }
    let complete = record.pinyin.text().is_some() && record.definition.text().is_some();
    Ok(if narrowed.is_empty() && complete {
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
    let definition = match record.definition.text() {
        Some(inbound) if !definition_matches(inbound, &hit.glosses) => inbound.to_owned(),
        _ => hit.glosses.join("; "),
    };
    Resolved {
        simplified: hit.simplified.clone(),
        traditional: hit.traditional.clone(),
        pinyin: hit.pinyin.clone(),
        definition,
        notes: record.notes.text().unwrap_or_default().to_owned(),
        verification: Verification::Confirmed,
    }
}

fn from_file(record: &ExchangeRecord, headword: &str, verification: Verification) -> Resolved {
    Resolved {
        simplified: headword.to_owned(),
        traditional: record.traditional.text().unwrap_or(headword).to_owned(),
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

    fn hit(id: i64, pinyin: &str, gloss: &str) -> DictionaryEntry {
        DictionaryEntry {
            source: SourceId("cc-cedict".to_owned()),
            source_version: SourceVersion("1".to_owned()),
            entry_id: id,
            simplified: "行".to_owned(),
            traditional: "行".to_owned(),
            pinyin: pinyin.to_owned(),
            glosses: vec![gloss.to_owned()],
            frequency_rank: None,
            hsk_rank: None,
        }
    }

    fn record(pinyin: Field<String>, definition: Field<String>) -> ExchangeRecord {
        let mut record = ExchangeRecord::new(1, "行", "行");
        record.pinyin = pinyin;
        record.definition = definition;
        record
    }

    fn present(value: &str) -> Field<String> {
        Field::Present(value.to_owned())
    }

    #[test]
    fn unique_hit_fills_an_omitted_definition() {
        let resolved = resolve(
            &record(present("xing2"), Field::Omitted),
            &[hit(1, "xing2", "to walk")],
        )
        .unwrap();
        assert_eq!(resolved.verification, Verification::Confirmed);
        assert_eq!(resolved.definition, "to walk");
    }

    #[test]
    fn two_hits_are_not_picked() {
        let resolved = resolve(
            &record(Field::Omitted, Field::Omitted),
            &[hit(1, "xing2", "to walk"), hit(2, "hang2", "firm")],
        )
        .unwrap();
        assert_eq!(resolved.verification, Verification::NeedsReview);
        assert_eq!(resolved.definition, "", "no invented definition");
    }

    #[test]
    fn the_reading_picks_between_hits() {
        let resolved = resolve(
            &record(present("háng"), Field::Omitted),
            &[hit(1, "xing2", "to walk"), hit(2, "hang2", "firm")],
        )
        .unwrap();
        assert_eq!(resolved.verification, Verification::Confirmed);
        assert_eq!(resolved.definition, "firm");
    }

    #[test]
    fn complete_record_with_no_hit_is_confirmed() {
        let resolved = resolve(&record(present("xing2"), present("a custom gloss")), &[]).unwrap();
        assert_eq!(resolved.verification, Verification::Confirmed);
        assert_eq!(resolved.definition, "a custom gloss");
    }

    #[test]
    fn a_different_definition_is_kept() {
        let resolved = resolve(
            &record(present("xing2"), present("my gloss")),
            &[hit(1, "xing2", "to walk")],
        )
        .unwrap();
        assert_eq!(resolved.definition, "my gloss");
        assert!(definition_matches("to walk", &["to walk".to_owned()]));
    }

    #[test]
    fn disagreeing_traditional_needs_review() {
        let mut record = record(present("xing2"), Field::Omitted);
        record.traditional = present("衍");
        let resolved = resolve(&record, &[hit(1, "xing2", "to walk")]).unwrap();
        assert_eq!(resolved.verification, Verification::NeedsReview);
    }

    #[test]
    fn blank_headword_is_dropped() {
        let record = ExchangeRecord::new(1, "", " ");
        assert!(resolve(&record, &[]).is_err());
    }
}
