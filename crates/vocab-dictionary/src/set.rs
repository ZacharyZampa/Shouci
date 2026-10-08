//! Several dictionaries searched as one, in priority order.

use std::collections::HashSet;

use vocab_core::{DictionaryEntry, Result, SourceId};
use vocab_pinyin::NormalizedPinyin;

use crate::{Candidate, DictionaryInfo, DictionaryProvider, HeadwordRanks, SqliteDictionary};

/// The enabled dictionaries, highest priority first.
///
/// Searches ask every member and keep all results. When two dictionaries have
/// the same word with the same reading, only the higher-priority entry is
/// kept, so results never show duplicates. Ranking stays with
/// `vocab-search`, which is given [`DictionarySet::priority`] to break ties.
pub struct DictionarySet {
    members: Vec<(DictionaryInfo, SqliteDictionary)>,
    schema_version: String,
}

impl DictionarySet {
    #[must_use]
    pub fn new(members: Vec<(DictionaryInfo, SqliteDictionary)>) -> Self {
        let schema_version = members
            .first()
            .map(|(_, dict)| dict.schema_version().to_owned())
            .unwrap_or_default();
        Self {
            members,
            schema_version,
        }
    }

    /// Opens each dictionary from its path, keeping the given order.
    ///
    /// # Errors
    ///
    /// Returns an error if a dictionary has no path or cannot be opened.
    pub fn open(infos: Vec<DictionaryInfo>) -> Result<Self> {
        let mut members = Vec::with_capacity(infos.len());
        for info in infos {
            let path = info.path.clone().ok_or_else(|| {
                vocab_core::VocabError::invalid(format!("dictionary {} has no file", info.id))
            })?;
            let dict = SqliteDictionary::open(&path)?;
            members.push((info, dict));
        }
        Ok(Self::new(members))
    }

    pub fn infos(&self) -> impl Iterator<Item = &DictionaryInfo> {
        self.members.iter().map(|(info, _)| info)
    }

    #[must_use]
    pub fn member(&self, id: &str) -> Option<&SqliteDictionary> {
        self.members
            .iter()
            .find(|(info, _)| info.id == id)
            .map(|(_, dict)| dict)
    }

    /// Dictionary ids, highest priority first, for the ranker's tiebreak.
    #[must_use]
    pub fn priority(&self) -> Vec<SourceId> {
        self.members
            .iter()
            .map(|(info, _)| SourceId(info.id.clone()))
            .collect()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// Every member's ranked headwords, highest priority first.
    ///
    /// # Errors
    ///
    /// Returns an error if a member cannot be read.
    pub fn headword_ranks(&self) -> Result<Vec<HeadwordRanks>> {
        let mut out = Vec::new();
        for (_, dict) in &self.members {
            out.extend(dict.headword_ranks()?);
        }
        Ok(out)
    }

    /// Every member's results, highest priority first. An entry is dropped
    /// only when a higher-priority dictionary has the same word with exactly
    /// the same pinyin. Entries within one dictionary are never merged:
    /// 白 `Bai2` (a surname) and 白 `bai2` (white) are different entries.
    fn gather<T>(
        &self,
        query: impl Fn(&SqliteDictionary) -> Result<Vec<T>>,
        entry: impl Fn(&T) -> &DictionaryEntry,
    ) -> Result<Vec<T>> {
        let key = |e: &DictionaryEntry| {
            (
                e.simplified.clone(),
                e.traditional.clone(),
                e.pinyin.clone(),
            )
        };
        let mut earlier: HashSet<(String, String, String)> = HashSet::new();
        let mut out = Vec::new();
        for (_, dict) in &self.members {
            let found = query(dict)?;
            let mut this_one = HashSet::new();
            for result in found {
                let k = key(entry(&result));
                if !earlier.contains(&k) {
                    this_one.insert(k);
                    out.push(result);
                }
            }
            earlier.extend(this_one);
        }
        Ok(out)
    }
}

impl DictionaryProvider for DictionarySet {
    fn search_english(&self, query: &str) -> Result<Vec<Candidate>> {
        self.gather(|dict| dict.search_english(query), |c| &c.entry)
    }

    fn search_pinyin(&self, pinyin: &NormalizedPinyin) -> Result<Vec<Candidate>> {
        self.gather(|dict| dict.search_pinyin(pinyin), |c| &c.entry)
    }

    fn lookup_chinese(&self, text: &str) -> Result<Vec<Candidate>> {
        self.gather(|dict| dict.lookup_chinese(text), |c| &c.entry)
    }

    fn entries_by_headword(&self, headword: &str) -> Result<Vec<DictionaryEntry>> {
        self.gather(|dict| dict.entries_by_headword(headword), |e| e)
    }

    fn schema_version(&self) -> &str {
        &self.schema_version
    }
}

#[cfg(test)]
mod tests {
    use super::DictionarySet;
    use crate::{CedictSource, DictionaryProvider, SqliteDictionary, build_dictionary_db};

    fn build(id: &str, text: &str) -> (crate::DictionaryInfo, SqliteDictionary) {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        build_dictionary_db(&mut conn, &CedictSource::new(id, "1"), text.as_bytes()).unwrap();
        let dict = SqliteDictionary::from_connection(conn).unwrap();
        (dict.info(None).unwrap(), dict)
    }

    #[test]
    fn entries_within_one_dictionary_are_never_merged() {
        let only = build("only", "白 白 [Bai2] /surname Bai/\n白 白 [bai2] /white/\n");
        let set = DictionarySet::new(vec![only]);
        let hits = set.entries_by_headword("白").unwrap();
        assert_eq!(hits.len(), 2, "{hits:?}");
    }

    #[test]
    fn duplicates_keep_the_higher_priority_entry_and_the_rest_stay() {
        let primary = build("primary", "學校 学校 [xue2 xiao4] /school/\n");
        let secondary = build(
            "secondary",
            "學校 学校 [xue2 xiao4] /école/\n學生 学生 [xue2 sheng5] /élève/\n",
        );
        let set = DictionarySet::new(vec![primary, secondary]);
        let hits = set.entries_by_headword("学校").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].source.as_str(), "primary");
        assert_eq!(hits[0].glosses, vec!["school".to_owned()]);
        let student = set.entries_by_headword("学生").unwrap();
        assert_eq!(student[0].source.as_str(), "secondary");
        assert_eq!(set.priority()[0].as_str(), "primary");
        assert!(set.member("secondary").is_some());
    }
}
