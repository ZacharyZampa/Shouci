use std::fmt;

/// Which dictionary (or other source) something came from: `cc-cedict`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceId(pub String);

impl SourceId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceVersion(pub String);

impl SourceVersion {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SourceVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One entry of one dictionary build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DictionaryEntry {
    /// The dictionary this entry belongs to.
    pub source: SourceId,
    pub source_version: SourceVersion,
    /// Row id inside this build. Stable only until the dictionary is rebuilt,
    /// so it breaks ranking ties but is never stored with a saved item.
    pub entry_id: i64,
    pub simplified: String,
    pub traditional: String,
    /// CC-CEDICT numbered pinyin: `xue2 xiao4`.
    pub pinyin: String,
    pub glosses: Vec<String>,
    pub frequency_rank: Option<u64>,
    pub hsk_rank: Option<u64>,
}

/// Which part of an entry a search matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum MatchBasis {
    EnglishGloss,
    Pinyin,
    Simplified,
    CharacterFallback,
}
