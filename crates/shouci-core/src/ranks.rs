//! How common a saved word is, and its HSK level.
//!
//! Read from the loaded dictionaries beside the word, never stored with it:
//! a newer dictionary's numbers show without changing the library, and a
//! word saved before any dictionary loaded gets them once one does. The
//! dictionaries give ranks to a headword's simplified form, so a word saved
//! without a reading (or with another traditional form) still gets them.

use std::collections::HashMap;

use vocab_core::Result;
use vocab_dictionary::HeadwordRanks;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Ranks {
    pub frequency: Option<u64>,
    pub hsk: Option<u64>,
}

/// Ranks by simplified form, built once per loaded set of dictionaries.
#[derive(Debug, Default)]
pub(crate) struct RankTable {
    by_simplified: HashMap<String, Ranks>,
}

impl RankTable {
    /// `headwords` highest-priority dictionary first: the first one to rank
    /// a headword wins.
    pub(crate) fn new(headwords: Vec<HeadwordRanks>) -> Self {
        let mut by_simplified = HashMap::with_capacity(headwords.len());
        for headword in headwords {
            by_simplified.entry(headword.simplified).or_insert(Ranks {
                frequency: headword.frequency_rank,
                hsk: headword.hsk_rank,
            });
        }
        Self { by_simplified }
    }

    /// Ranks are something to show, not data to keep: when the dictionaries
    /// cannot be read for them, words show none rather than failing to list.
    pub(crate) fn or_empty(headwords: Result<Vec<HeadwordRanks>>) -> Self {
        headwords.map(Self::new).unwrap_or_default()
    }

    pub(crate) fn of(&self, simplified: &str) -> Ranks {
        self.by_simplified
            .get(simplified)
            .copied()
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use vocab_dictionary::HeadwordRanks;

    use super::{RankTable, Ranks};

    fn headword(simplified: &str, frequency: Option<u64>, hsk: Option<u64>) -> HeadwordRanks {
        HeadwordRanks {
            simplified: simplified.to_owned(),
            frequency_rank: frequency,
            hsk_rank: hsk,
        }
    }

    #[test]
    fn the_first_dictionary_to_rank_a_word_wins() {
        let table = RankTable::new(vec![
            headword("猫", Some(3), Some(2)),
            headword("猫", Some(9), None),
            headword("水", None, Some(1)),
        ]);
        assert_eq!(
            table.of("猫"),
            Ranks {
                frequency: Some(3),
                hsk: Some(2)
            }
        );
        assert_eq!(table.of("水").hsk, Some(1));
        assert_eq!(table.of("蚌埠住了"), Ranks::default());
    }
}
