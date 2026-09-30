//! `Shouci` display helpers. Search and save live in `vocab-capture`.

use vocab_dictionary::Candidate;

/// Maximum hits rendered in the popover list.
pub const RESULT_LIMIT: usize = 10;

/// The characters: simplified, plus traditional when it differs.
#[must_use]
pub fn headword(candidate: &Candidate) -> String {
    let entry = &candidate.entry;
    if entry.simplified == entry.traditional {
        entry.simplified.clone()
    } else {
        format!("{} / {}", entry.simplified, entry.traditional)
    }
}

/// The reading with tone marks (`xué`), flagged when the match was
/// inferred rather than direct.
#[must_use]
pub fn reading(candidate: &Candidate) -> String {
    let marked = vocab_pinyin::tone_marks(&candidate.entry.pinyin);
    if candidate.diagnostic.is_inferred {
        format!("{marked} · inferred")
    } else {
        marked
    }
}

/// Headword and reading on one line, for tooltips and `VoiceOver`.
#[must_use]
pub fn headline(candidate: &Candidate) -> String {
    format!("{} {}", headword(candidate), reading(candidate))
}

#[must_use]
pub fn gloss_line(candidate: &Candidate) -> String {
    vocab_dictionary::display_definition(&candidate.entry.glosses.join("; "))
}

#[cfg(test)]
mod tests {
    use vocab_core::{
        ConfirmationState, DictionaryEntry, MatchBasis, Provenance, SourceId, SourceVersion,
    };
    use vocab_dictionary::CandidateDiagnostic;

    use super::*;

    fn candidate(inferred: bool) -> Candidate {
        Candidate {
            entry: DictionaryEntry {
                simplified: "学".into(),
                traditional: "學".into(),
                pinyin: "xue2".into(),
                glosses: vec!["learn".into(), "study".into()],
                frequency_rank: None,
                hsk_rank: None,
                stable_entry_id: Some(1),
                provenance: Provenance {
                    source: SourceId("cedict".into()),
                    source_version: SourceVersion("1".into()),
                    import_origin: None,
                    confirmation: ConfirmationState::DictionaryAuthority,
                },
            },
            diagnostic: CandidateDiagnostic {
                basis: MatchBasis::EnglishGloss,
                is_inferred: inferred,
            },
        }
    }

    #[test]
    fn headline_marks_tones_and_inferred() {
        assert_eq!(headline(&candidate(false)), "学 / 學 xué");
        assert_eq!(headline(&candidate(true)), "学 / 學 xué · inferred");
    }

    #[test]
    fn headword_hides_matching_traditional() {
        let mut same = candidate(false);
        same.entry.traditional = same.entry.simplified.clone();
        assert_eq!(headword(&same), "学");
        assert_eq!(headword(&candidate(false)), "学 / 學");
    }

    #[test]
    fn gloss_joins_definitions() {
        assert_eq!(gloss_line(&candidate(false)), "learn; study");
    }

    #[test]
    fn gloss_rewrites_cedict_notation() {
        let mut hit = candidate(false);
        hit.entry.glosses = vec!["school".into(), "CL:所[suo3]".into()];
        assert_eq!(gloss_line(&hit), "school; measure word: 所 (suǒ)");
    }
}
