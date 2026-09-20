use std::collections::BTreeMap;

use vocab_core::{Result, SourceId, SourceVersion, VocabError};

use crate::{DataSourceDescriptor, IngestSource, LayerKind, RawEntry, VALUE_FREQUENCY_RANK};

/// OpenSubtitles-style word frequency list (`word count` per line).
///
/// Rank 1 is the most frequent word. Duplicate words sum their counts; ties
/// break on the word string so the assigned ranks are deterministic.
#[derive(Debug, Clone)]
pub struct FrequencySource {
    descriptor: DataSourceDescriptor,
}

impl FrequencySource {
    pub fn new(
        id: impl Into<String>,
        version: impl Into<String>,
        license: impl Into<String>,
    ) -> Self {
        Self {
            descriptor: DataSourceDescriptor {
                id: SourceId(id.into()),
                layer: LayerKind::Frequency,
                version: SourceVersion(version.into()),
                license: license.into(),
            },
        }
    }
}

impl Default for FrequencySource {
    fn default() -> Self {
        Self::new("opensubtitles-zh", "2018", "CC BY-SA 4.0")
    }
}

impl IngestSource for FrequencySource {
    fn descriptor(&self) -> &DataSourceDescriptor {
        &self.descriptor
    }

    fn parse(&self, artifact: &[u8]) -> Result<Vec<RawEntry>> {
        let text = std::str::from_utf8(artifact)
            .map_err(|err| VocabError::new(format!("frequency artifact is not UTF-8: {err}")))?;
        let mut counts: BTreeMap<String, u64> = BTreeMap::new();
        for (index, raw_line) in text.lines().enumerate() {
            let line_number = index + 1;
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.split_whitespace();
            let Some(word) = parts.next() else {
                continue;
            };
            let Some(count_raw) = parts.next() else {
                return Err(VocabError::new(format!(
                    "frequency line {line_number} is missing a count: {line}"
                )));
            };
            if parts.next().is_some() {
                return Err(VocabError::new(format!(
                    "frequency line {line_number} has extra fields: {line}"
                )));
            }
            let count: u64 = count_raw.parse().map_err(|_| {
                VocabError::new(format!(
                    "frequency line {line_number} count is not an integer: {line}"
                ))
            })?;
            *counts.entry(word.to_owned()).or_insert(0) += count;
        }
        let mut ordered: Vec<(String, u64)> = counts.into_iter().collect();
        ordered.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let mut entries = Vec::with_capacity(ordered.len());
        for (index, (word, _)) in ordered.into_iter().enumerate() {
            let rank =
                u64::try_from(index + 1).map_err(|_| VocabError::new("frequency rank overflow"))?;
            let mut entry = RawEntry::new(word.clone(), word, "", Vec::new());
            entry
                .values
                .insert(VALUE_FREQUENCY_RANK.to_owned(), rank.to_string());
            entries.push(entry);
        }
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::FrequencySource;
    use crate::{IngestSource, VALUE_FREQUENCY_RANK};

    #[test]
    fn ranks_by_descending_count() {
        let source = FrequencySource::default();
        let parsed = source.parse(b"cat 10\ndog 50\nant 50\n").unwrap();
        let ranks: Vec<(&str, &str)> = parsed
            .iter()
            .map(|e| {
                (
                    e.simplified.as_str(),
                    e.values[VALUE_FREQUENCY_RANK].as_str(),
                )
            })
            .collect();
        assert_eq!(ranks, vec![("ant", "1"), ("dog", "2"), ("cat", "3")]);
    }

    #[test]
    fn sums_duplicate_words() {
        let source = FrequencySource::default();
        let parsed = source.parse(b"go 1\ngo 4\nstop 3\n").unwrap();
        assert_eq!(parsed[0].simplified, "go");
        assert_eq!(parsed[0].values[VALUE_FREQUENCY_RANK], "1");
        assert_eq!(parsed[1].simplified, "stop");
    }

    #[test]
    fn rejects_malformed_line() {
        let source = FrequencySource::default();
        let err = source.parse(b"onlyword\n").unwrap_err();
        assert!(err.to_string().contains("missing a count"));
    }
}
