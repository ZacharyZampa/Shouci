//! Searching the user's own words.
//!
//! A library is hundreds to a few thousand words, so matching runs in memory
//! over every item: no index to keep in sync. Order is deterministic: match
//! quality, then the order the caller passed items in (newest first, by
//! convention).

use vocab_pinyin::{normalize, numbered, segment};

use crate::query::{QueryKind, detect};

/// The searchable text of one saved word.
#[derive(Debug, Clone, Copy)]
pub struct LibraryDoc<'a> {
    pub simplified: &'a str,
    pub traditional: &'a str,
    /// Numbered pinyin: `xue2 xiao4`.
    pub pinyin: &'a str,
    pub definition: &'a str,
    pub notes: &'a str,
}

/// One matching item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LibraryHit {
    /// Position in the slice passed to [`match_library`].
    pub index: usize,
    /// How the query matched this item.
    pub kind: QueryKind,
    /// Lower is better: 0 exact, 1 prefix or whole word, 2 word prefix,
    /// 3 anywhere.
    pub quality: u8,
}

const EXACT: u8 = 0;
const PREFIX: u8 = 1;
const WORD_PREFIX: u8 = 2;
const ANYWHERE: u8 = 3;

/// Every item the query matches, best first.
///
/// With `kind` set, only that reading is tried. Otherwise the query's kind is
/// detected; plain letters are tried as both English and pinyin (`ma` is as
/// likely 妈 as "mammal"), and each item keeps its better match.
#[must_use]
pub fn match_library(
    query: &str,
    kind: Option<QueryKind>,
    docs: &[LibraryDoc<'_>],
) -> Vec<LibraryHit> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let kinds: &[QueryKind] = match kind {
        Some(QueryKind::English) => &[QueryKind::English],
        Some(QueryKind::Pinyin) => &[QueryKind::Pinyin],
        Some(QueryKind::Chinese) => &[QueryKind::Chinese],
        None => match detect(query) {
            QueryKind::Chinese => &[QueryKind::Chinese],
            QueryKind::Pinyin => &[QueryKind::Pinyin],
            QueryKind::English => &[QueryKind::English, QueryKind::Pinyin],
        },
    };
    let pinyin = kinds
        .contains(&QueryKind::Pinyin)
        .then(|| PinyinQuery::new(query))
        .flatten();
    let english = kinds
        .contains(&QueryKind::English)
        .then(|| EnglishQuery::new(query));

    let mut hits: Vec<LibraryHit> = docs
        .iter()
        .enumerate()
        .filter_map(|(index, doc)| {
            let mut best: Option<(u8, QueryKind)> = None;
            let mut consider = |quality: Option<u8>, kind: QueryKind| {
                if let Some(quality) = quality {
                    if best.is_none_or(|(current, _)| quality < current) {
                        best = Some((quality, kind));
                    }
                }
            };
            if kinds.contains(&QueryKind::Chinese) {
                consider(match_chinese(query, doc), QueryKind::Chinese);
            }
            if let Some(english) = &english {
                consider(english.matches(doc), QueryKind::English);
            }
            if let Some(pinyin) = &pinyin {
                consider(pinyin.matches(doc), QueryKind::Pinyin);
            }
            best.map(|(quality, kind)| LibraryHit {
                index,
                kind,
                quality,
            })
        })
        .collect();
    hits.sort_by_key(|hit| (hit.quality, hit.index));
    hits
}

fn match_chinese(query: &str, doc: &LibraryDoc<'_>) -> Option<u8> {
    let forms = [doc.simplified, doc.traditional];
    if forms.contains(&query) {
        Some(EXACT)
    } else if forms.iter().any(|form| form.starts_with(query)) {
        Some(PREFIX)
    } else if forms.iter().any(|form| form.contains(query)) {
        Some(ANYWHERE)
    } else {
        None
    }
}

struct EnglishQuery {
    text: String,
    tokens: Vec<String>,
}

impl EnglishQuery {
    fn new(query: &str) -> Self {
        let text = query.to_lowercase();
        let tokens = words(&text);
        Self { text, tokens }
    }

    fn matches(&self, doc: &LibraryDoc<'_>) -> Option<u8> {
        let definition = doc.definition.to_lowercase();
        let notes = doc.notes.to_lowercase();
        let exact = definition
            .split([';', '/', ','])
            .map(str::trim)
            .any(|gloss| gloss == self.text || gloss.strip_prefix("to ") == Some(&self.text));
        if exact {
            return Some(EXACT);
        }
        if self.tokens.is_empty() {
            return None;
        }
        let doc_words: Vec<String> = words(&definition)
            .into_iter()
            .chain(words(&notes))
            .collect();
        if self
            .tokens
            .iter()
            .all(|token| doc_words.iter().any(|word| word == token))
        {
            return Some(PREFIX);
        }
        if self.tokens.iter().all(|token| {
            doc_words
                .iter()
                .any(|word| word.starts_with(token.as_str()))
        }) {
            return Some(WORD_PREFIX);
        }
        (definition.contains(&self.text) || notes.contains(&self.text)).then_some(ANYWHERE)
    }
}

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Pinyin as syllable sequences: every way the query can be split.
struct PinyinQuery {
    variants: Vec<Vec<String>>,
}

impl PinyinQuery {
    /// `None` when the query is not pinyin at all.
    fn new(query: &str) -> Option<Self> {
        let variants: Vec<Vec<String>> = segment(normalize(&numbered(query)).as_str())
            .into_iter()
            .map(|variant| variant.split(' ').map(str::to_owned).collect())
            .collect();
        (!variants.is_empty()).then_some(Self { variants })
    }

    fn matches(&self, doc: &LibraryDoc<'_>) -> Option<u8> {
        if doc.pinyin.trim().is_empty() {
            return None;
        }
        let normalized = normalize(doc.pinyin);
        let syllables: Vec<&str> = normalized.as_str().split(' ').collect();
        self.variants
            .iter()
            .filter_map(|variant| align(variant, &syllables))
            .min()
    }
}

fn align(query: &[String], doc: &[&str]) -> Option<u8> {
    if query.is_empty() || query.len() > doc.len() {
        return None;
    }
    let fits = |start: usize| {
        query
            .iter()
            .zip(&doc[start..])
            .all(|(q, d)| syllable_matches(q, d))
    };
    if fits(0) {
        return Some(if query.len() == doc.len() {
            EXACT
        } else {
            PREFIX
        });
    }
    (1..=doc.len() - query.len()).any(fits).then_some(ANYWHERE)
}

/// A toned query syllable must match the tone; an untoned one matches any.
fn syllable_matches(query: &str, doc: &str) -> bool {
    if query.ends_with(|c: char| c.is_ascii_digit()) {
        query == doc
    } else {
        doc.trim_end_matches(|c: char| c.is_ascii_digit()) == query
    }
}

#[cfg(test)]
mod tests {
    use super::{LibraryDoc, match_library};
    use crate::QueryKind;

    fn doc<'a>(simplified: &'a str, pinyin: &'a str, definition: &'a str) -> LibraryDoc<'a> {
        LibraryDoc {
            simplified,
            traditional: simplified,
            pinyin,
            definition,
            notes: "",
        }
    }

    fn library() -> Vec<LibraryDoc<'static>> {
        vec![
            doc("学校", "xue2 xiao4", "school"),
            doc("学生", "xue2 sheng5", "student; schoolchild"),
            doc("妈妈", "ma1 ma5", "mom"),
            doc("马", "ma3", "horse"),
            doc("旅行", "lv3 xing2", "to travel; travel"),
        ]
    }

    fn heads(query: &str, kind: Option<QueryKind>) -> Vec<&'static str> {
        let docs = library();
        match_library(query, kind, &docs)
            .into_iter()
            .map(|hit| docs[hit.index].simplified)
            .collect()
    }

    #[test]
    fn chinese_exact_then_prefix() {
        assert_eq!(heads("学", None), vec!["学校", "学生"]);
        assert_eq!(heads("学生", None), vec!["学生"]);
    }

    #[test]
    fn untoned_pinyin_finds_toned_readings() {
        assert_eq!(heads("xuexiao", None), vec!["学校"]);
        assert_eq!(heads("xue", None), vec!["学校", "学生"]);
        assert_eq!(heads("xué xiào", None), vec!["学校"]);
    }

    #[test]
    fn toned_pinyin_must_match_the_tone() {
        assert_eq!(heads("ma3", None), vec!["马"]);
        assert_eq!(heads("ma1", None), vec!["妈妈"]);
    }

    #[test]
    fn plain_letters_try_english_and_pinyin() {
        assert_eq!(heads("ma", None), vec!["马", "妈妈"]);
        assert_eq!(heads("school", None), vec!["学校", "学生"]);
        assert_eq!(heads("travel", None), vec!["旅行"]);
    }

    #[test]
    fn lv_reads_as_umlaut() {
        assert_eq!(heads("lüxing", None), vec!["旅行"]);
        assert_eq!(heads("lvxing", None), vec!["旅行"]);
    }

    #[test]
    fn forced_kind_is_the_only_one_tried() {
        assert_eq!(heads("ma", Some(QueryKind::English)), [] as [&str; 0]);
        assert_eq!(
            heads("school", Some(QueryKind::English)),
            vec!["学校", "学生"]
        );
    }

    #[test]
    fn blank_query_matches_nothing() {
        assert_eq!(heads("  ", None), [] as [&str; 0]);
    }
}
