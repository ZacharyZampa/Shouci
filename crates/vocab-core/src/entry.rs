use crate::provenance::Provenance;
use crate::status::ItemStatus;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DictionaryEntry {
    pub simplified: String,
    pub traditional: String,
    pub pinyin: String,
    pub glosses: Vec<String>,
    pub frequency_rank: Option<u64>,
    pub hsk_rank: Option<u64>,
    pub stable_entry_id: Option<i64>,
    pub provenance: Provenance,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VocabItem {
    pub item_id: i64,
    pub simplified: String,
    pub traditional: String,
    pub pinyin: String,
    pub definition: String,
    pub status: ItemStatus,
    pub notes: Option<String>,
    pub source_entry_id: Option<i64>,
    pub provenance: Provenance,
    pub origin_export_id: Option<i64>,
    pub created_at: String,
    pub modified_at: String,
}
