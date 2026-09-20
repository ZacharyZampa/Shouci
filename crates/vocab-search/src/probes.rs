use std::path::PathBuf;

/// One HSK probe row from `fixtures/search/top1000.tsv`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchProbe {
    pub simplified: String,
    pub english: String,
    pub pinyin: String,
    pub untoned: String,
    pub numbered: String,
}

/// Loads the committed search-quality fixture.
///
/// # Panics
///
/// Panics if the fixture file is missing or a row is malformed.
#[must_use]
pub fn load_search_probes() -> Vec<SearchProbe> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/search/top1000.tsv");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    text.lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let mut cols = line.split('\t');
            let simplified = cols
                .next()
                .unwrap_or_else(|| panic!("missing simplified: {line}"))
                .to_owned();
            let english = cols
                .next()
                .unwrap_or_else(|| panic!("missing english: {line}"))
                .to_owned();
            let pinyin = cols
                .next()
                .unwrap_or_else(|| panic!("missing pinyin: {line}"))
                .to_owned();
            let untoned: String = pinyin.chars().filter(char::is_ascii_alphabetic).collect();
            let numbered: String = pinyin.chars().filter(|c| !c.is_whitespace()).collect();
            SearchProbe {
                simplified,
                english,
                pinyin,
                untoned,
                numbered,
            }
        })
        .collect()
}
