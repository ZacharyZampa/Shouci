//! `Shouci` display helpers. Search and save live in `vocab-capture`.

use vocab_dictionary::Candidate;

/// Maximum hits rendered in the popover list.
pub const RESULT_LIMIT: usize = 10;

#[must_use]
pub fn headline(candidate: &Candidate) -> String {
    let marker = if candidate.diagnostic.is_inferred {
        " ▸inferred"
    } else {
        ""
    };
    format!(
        "{} / {}  [{}]{marker}",
        candidate.entry.simplified, candidate.entry.traditional, candidate.entry.pinyin
    )
}

#[must_use]
pub fn gloss_line(candidate: &Candidate) -> String {
    candidate.entry.glosses.join("; ")
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
    fn headline_marks_inferred() {
        assert_eq!(headline(&candidate(false)), "学 / 學  [xue2]");
        assert_eq!(headline(&candidate(true)), "学 / 學  [xue2] ▸inferred");
    }

    #[test]
    fn gloss_joins_definitions() {
        assert_eq!(gloss_line(&candidate(false)), "learn; study");
    }
}
