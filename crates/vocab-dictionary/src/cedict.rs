use vocab_core::{Result, SourceId, SourceVersion, VocabError};

use crate::{DataSourceDescriptor, IngestSource, LayerKind, RawEntry};

/// CC-CEDICT, the standard open CC BY-SA 4.0 lexicon.
///
/// Line grammar: `Traditional Simplified [pin1 yin1] /gloss1/gloss2/`
/// Header lines start with `#`.
#[derive(Debug, Clone)]
pub struct CedictSource {
    descriptor: DataSourceDescriptor,
}

impl CedictSource {
    pub fn new(id: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            descriptor: DataSourceDescriptor {
                id: SourceId(id.into()),
                layer: LayerKind::BaseLexicon,
                version: SourceVersion(version.into()),
                license: "CC BY-SA 4.0".to_owned(),
            },
        }
    }
}

impl Default for CedictSource {
    fn default() -> Self {
        Self::new("cc-cedict", "1.0.0")
    }
}

impl IngestSource for CedictSource {
    fn descriptor(&self) -> &DataSourceDescriptor {
        &self.descriptor
    }

    fn parse(&self, artifact: &[u8]) -> Result<Vec<RawEntry>> {
        let text = std::str::from_utf8(artifact)
            .map_err(|err| VocabError::new(format!("cedict artifact is not UTF-8: {err}")))?;
        let mut entries = Vec::new();
        for (index, raw_line) in text.lines().enumerate() {
            let line_number = index + 1;
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            match parse_line(line) {
                Some((traditional, simplified, pinyin, glosses)) => {
                    entries.push(RawEntry::new(simplified, traditional, pinyin, glosses));
                }
                None => {
                    return Err(VocabError::new(format!(
                        "cedict line {line_number} is malformed and would be dropped: {line}"
                    )));
                }
            }
        }
        Ok(entries)
    }
}

fn parse_line(line: &str) -> Option<(String, String, String, Vec<String>)> {
    let traditional_end = line.find(char::is_whitespace)?;
    let traditional = line[..traditional_end].to_owned();
    let rest = line[traditional_end..].trim_start();
    let simplified_end = rest.find(char::is_whitespace)?;
    let simplified = rest[..simplified_end].to_owned();
    let rest = rest[simplified_end..].trim_start();

    let pinyin_start = rest.find('[')?;
    let pinyin_end = rest[pinyin_start..].find(']')? + pinyin_start;
    let pinyin = rest[pinyin_start + 1..pinyin_end].to_owned();
    let glosses = rest[pinyin_end + 1..]
        .split('/')
        .filter(|part| !part.is_empty())
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if glosses.is_empty() {
        return None;
    }
    Some((traditional, simplified, pinyin, glosses))
}

#[cfg(test)]
mod tests {
    use super::parse_line;

    #[test]
    fn parses_a_standard_line() {
        let (traditional, simplified, pinyin, glosses) =
            parse_line("麪 面 [mian4] /flour/noodles/").unwrap();
        assert_eq!(traditional, "麪");
        assert_eq!(simplified, "面");
        assert_eq!(pinyin, "mian4");
        assert_eq!(glosses, vec!["flour", "noodles"]);
    }

    #[test]
    fn parses_multi_gloss_and_space_in_pinyin() {
        let (traditional, simplified, pinyin, glosses) =
            parse_line("你好 你好 [ni3 hao3] /hello!/hi/").unwrap();
        assert_eq!(traditional, "你好");
        assert_eq!(simplified, "你好");
        assert_eq!(pinyin, "ni3 hao3");
        assert_eq!(glosses, vec!["hello!", "hi"]);
    }

    #[test]
    fn rejects_missing_pinyin_or_glosses() {
        assert!(parse_line("你好 你好 [ni3 hao3] /").is_none());
        assert!(parse_line("你好 你好").is_none());
    }
}
