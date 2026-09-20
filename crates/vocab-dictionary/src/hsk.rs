use std::collections::BTreeMap;

use vocab_core::{Result, SourceId, SourceVersion, VocabError};

use crate::{DataSourceDescriptor, IngestSource, LayerKind, RawEntry, VALUE_HSK_RANK};

/// HSK word list: TSV `word<TAB>level` or a CSV with `Simplified` and `Level`.
///
/// Level is 1–9. The HSK 3.0 advanced band `7-9` is stored as 7 (lowest of
/// the band) so listed words still beat unlisted ones. Duplicate words keep
/// the easiest (lowest) level.
#[derive(Debug, Clone)]
pub struct HskSource {
    descriptor: DataSourceDescriptor,
}

impl HskSource {
    pub fn new(
        id: impl Into<String>,
        version: impl Into<String>,
        license: impl Into<String>,
    ) -> Self {
        Self {
            descriptor: DataSourceDescriptor {
                id: SourceId(id.into()),
                layer: LayerKind::HskRanks,
                version: SourceVersion(version.into()),
                license: license.into(),
            },
        }
    }
}

impl Default for HskSource {
    fn default() -> Self {
        Self::new("hsk", "3.0", "MIT")
    }
}

impl IngestSource for HskSource {
    fn descriptor(&self) -> &DataSourceDescriptor {
        &self.descriptor
    }

    fn parse(&self, artifact: &[u8]) -> Result<Vec<RawEntry>> {
        let text = std::str::from_utf8(artifact)
            .map_err(|err| VocabError::new(format!("hsk artifact is not UTF-8: {err}")))?;
        let mut levels: BTreeMap<String, u64> = BTreeMap::new();
        let mut csv_columns: Option<(usize, usize)> = None;
        for (index, raw_line) in text.lines().enumerate() {
            let line_number = index + 1;
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if csv_columns.is_none() && line.contains("Simplified") && line.contains("Level") {
                let header = parse_csv_row(line);
                let simplified =
                    header
                        .iter()
                        .position(|h| h == "Simplified")
                        .ok_or_else(|| {
                            VocabError::new(format!(
                                "hsk line {line_number} CSV header has no Simplified"
                            ))
                        })?;
                let level = header.iter().position(|h| h == "Level").ok_or_else(|| {
                    VocabError::new(format!("hsk line {line_number} CSV header has no Level"))
                })?;
                csv_columns = Some((simplified, level));
                continue;
            }
            let (word, level_raw) = if let Some((word_idx, level_idx)) = csv_columns {
                let cols = parse_csv_row(line);
                let word = cols.get(word_idx).map_or("", String::as_str);
                let level = cols.get(level_idx).map_or("", String::as_str);
                if word.is_empty() || level.is_empty() {
                    return Err(VocabError::new(format!(
                        "hsk line {line_number} is missing Simplified or Level: {line}"
                    )));
                }
                (word.to_owned(), level.to_owned())
            } else {
                let mut parts = line.split_whitespace();
                let Some(word) = parts.next() else {
                    continue;
                };
                let Some(level_raw) = parts.next() else {
                    return Err(VocabError::new(format!(
                        "hsk line {line_number} is missing a level: {line}"
                    )));
                };
                if parts.next().is_some() {
                    return Err(VocabError::new(format!(
                        "hsk line {line_number} has extra fields: {line}"
                    )));
                }
                (word.to_owned(), level_raw.to_owned())
            };
            let Some(level) = parse_hsk_level(&level_raw) else {
                return Err(VocabError::new(format!(
                    "hsk line {line_number} has an invalid level {level_raw:?}: {line}"
                )));
            };
            levels
                .entry(word)
                .and_modify(|current| *current = (*current).min(level))
                .or_insert(level);
        }
        Ok(levels
            .into_iter()
            .map(|(word, level)| {
                let mut entry = RawEntry::new(word.clone(), word, "", Vec::new());
                entry
                    .values
                    .insert(VALUE_HSK_RANK.to_owned(), level.to_string());
                entry
            })
            .collect())
    }
}

fn parse_hsk_level(raw: &str) -> Option<u64> {
    if raw == "7-9" {
        return Some(7);
    }
    let level: u64 = raw.parse().ok()?;
    (1..=9).contains(&level).then_some(level)
}

fn parse_csv_row(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if in_quotes => {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    cur.push('"');
                } else {
                    in_quotes = false;
                }
            }
            '"' => in_quotes = true,
            ',' if !in_quotes => {
                out.push(std::mem::take(&mut cur));
            }
            c => cur.push(c),
        }
    }
    out.push(cur);
    out
}

#[cfg(test)]
mod tests {
    use super::HskSource;
    use crate::{IngestSource, VALUE_HSK_RANK};

    #[test]
    fn parses_tsv_and_keeps_easiest_level() {
        let source = HskSource::default();
        let parsed = source.parse("去\t3\n去\t1\n于\t6\n".as_bytes()).unwrap();
        let map: std::collections::BTreeMap<_, _> = parsed
            .iter()
            .map(|e| (e.simplified.as_str(), e.values[VALUE_HSK_RANK].as_str()))
            .collect();
        assert_eq!(map["去"], "1");
        assert_eq!(map["于"], "6");
    }

    #[test]
    fn parses_csv_header_and_band() {
        let source = HskSource::default();
        let artifact = "ID,Simplified,Level\nL1,猫,2\nL7,禅,7-9\n".as_bytes();
        let parsed = source.parse(artifact).unwrap();
        let map: std::collections::BTreeMap<_, _> = parsed
            .iter()
            .map(|e| (e.simplified.as_str(), e.values[VALUE_HSK_RANK].as_str()))
            .collect();
        assert_eq!(map["猫"], "2");
        assert_eq!(map["禅"], "7");
    }

    #[test]
    fn rejects_bad_level() {
        let source = HskSource::default();
        let err = source.parse("去 banana\n".as_bytes()).unwrap_err();
        assert!(err.to_string().contains("invalid level"));
    }
}
