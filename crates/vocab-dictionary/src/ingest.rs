use std::collections::BTreeMap;

use vocab_core::{Result, SourceId, SourceVersion};

/// Scalar stored on a frequency-layer [`RawEntry`].
pub(crate) const VALUE_FREQUENCY_RANK: &str = "frequency_rank";
pub(crate) const VALUE_HSK_RANK: &str = "hsk_rank";

/// What a source contributes to a composed dictionary entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayerKind {
    BaseLexicon,
    Frequency,
    HskRanks,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataSourceDescriptor {
    pub id: SourceId,
    pub layer: LayerKind,
    pub version: SourceVersion,
    pub license: String,
}

/// One parsed record from a source artifact, normalized to the composition key.
///
/// Enrichment layers carry their scalar values (e.g. frequency rank, HSK rank) in
/// `values`; keys are serialized field names recorded in the ingestion manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEntry {
    pub simplified: String,
    pub traditional: String,
    pub pinyin: String,
    pub glosses: Vec<String>,
    pub values: BTreeMap<String, String>,
}

impl RawEntry {
    pub fn new(
        simplified: impl Into<String>,
        traditional: impl Into<String>,
        pinyin: impl Into<String>,
        glosses: Vec<String>,
    ) -> Self {
        Self {
            simplified: simplified.into(),
            traditional: traditional.into(),
            pinyin: pinyin.into(),
            glosses,
            values: BTreeMap::new(),
        }
    }
}

/// Build-time adapter for one dictionary data artifact.
///
/// `parse` must be deterministic for identical bytes and must never silently drop
/// records it cannot interpret — unsupported lines are reported, not skipped.
/// Every [`RawEntry`] it returns is attributed to its layer's provenance.
pub trait IngestSource: Send + Sync {
    fn descriptor(&self) -> &DataSourceDescriptor;

    /// Parses one artifact into normalized raw entries.
    ///
    /// # Errors
    ///
    /// Returns an error when the artifact cannot be decoded or when records would
    /// be dropped to make it parse — never silently.
    fn parse(&self, artifact: &[u8]) -> Result<Vec<RawEntry>>;
}
