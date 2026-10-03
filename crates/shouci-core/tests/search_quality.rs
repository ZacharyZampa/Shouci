//! Search quality against the real CC-CEDICT build and the committed probe
//! list (`fixtures/search/top1000.tsv`): each probe word must surface in the
//! first results for its English, pinyin, and Hanzi queries.
//!
//! Needs `data/dictionaries/cc-cedict.db` (`./scripts/check.sh` fetches it).

use std::path::Path;

use shouci_core::testing::scratch_dir;
use shouci_core::{Config, QueryKind, Shouci};
use vocab_search::load_search_probes;

/// What the menu-bar popover shows.
const POPOVER: u32 = 10;

fn shouci() -> Shouci {
    let dictionaries = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/dictionaries");
    assert!(
        dictionaries.join("cc-cedict.db").is_file(),
        "search quality tests need data/dictionaries/cc-cedict.db; run ./scripts/check.sh"
    );
    let mut config = Config::in_dir(scratch_dir("quality"));
    config.dictionaries_dir = dictionaries;
    config.fetch_dictionaries = false;
    let shouci = Shouci::open(config).expect("open");
    shouci.load_dictionaries().expect("load");
    shouci
}

fn run(limit: usize, english: bool, kinds: bool) {
    let shouci = shouci();
    let probes = load_search_probes();
    assert!(
        probes.len() >= limit,
        "probe list too short: {}",
        probes.len()
    );
    let mut misses = Vec::new();
    let mut lookups = 0usize;
    for probe in probes.iter().take(limit) {
        let mut cases = vec![
            (
                "pinyin-numbered",
                QueryKind::Pinyin,
                probe.numbered.as_str(),
            ),
            ("hanzi", QueryKind::Chinese, probe.simplified.as_str()),
        ];
        if english {
            cases.insert(0, ("english", QueryKind::English, probe.english.as_str()));
        }
        if probe.pinyin.contains(' ') {
            cases.insert(
                1,
                ("pinyin-untoned", QueryKind::Pinyin, probe.untoned.as_str()),
            );
        }
        for (label, kind, query) in cases {
            lookups += 1;
            let kind = kinds.then_some(kind);
            match shouci.search_dictionary(query, kind, Some(POPOVER)) {
                Ok(found) => {
                    let heads: Vec<&str> = found
                        .candidates
                        .iter()
                        .map(|c| c.simplified.as_str())
                        .collect();
                    if !heads.contains(&probe.simplified.as_str()) {
                        misses.push(format!(
                            "{label} {query:?} expected {} in {heads:?}",
                            probe.simplified
                        ));
                    }
                }
                Err(err) => misses.push(format!("{label} {query:?} error: {err}")),
            }
        }
    }
    assert!(
        misses.is_empty(),
        "{} / {lookups} probe lookups missed:\n{}",
        misses.len(),
        misses.join("\n")
    );
}

#[test]
fn top50_surface_with_automatic_search() {
    run(50, true, false);
}

#[test]
fn top1000_pinyin_and_hanzi_surface_with_automatic_search() {
    run(1000, false, false);
}

#[test]
fn top50_surface_when_the_kind_is_given() {
    run(50, true, true);
}

/// Everyday English finds the everyday word first, not a rare one that
/// happens to say just that (暍 `hot`) or a verb sense nobody uses (屯驻
/// `to quarter`).
#[test]
fn common_words_lead_their_english() {
    let shouci = shouci();
    let words = [
        ("hot", "热"),
        ("quarter", "刻"),
        ("cold", "冷"),
        ("big", "大"),
        ("small", "小"),
        ("eat", "吃"),
        ("drink", "喝"),
        ("water", "水"),
        ("dog", "狗"),
        ("cat", "猫"),
        ("book", "书"),
        ("friend", "朋友"),
        ("red", "红"),
        ("slow", "慢"),
        ("tired", "累"),
        ("hungry", "饿"),
        ("snow", "雪"),
        ("buy", "买"),
        ("sell", "卖"),
        ("expensive", "贵"),
        ("cheap", "便宜"),
        ("bitter", "苦"),
        ("sour", "酸"),
        ("salty", "咸"),
    ];
    let mut misses = Vec::new();
    for (english, word) in words {
        let found = shouci
            .search_dictionary(english, None, Some(3))
            .expect("search");
        let heads: Vec<&str> = found
            .candidates
            .iter()
            .map(|c| c.simplified.as_str())
            .collect();
        if heads.first() != Some(&word) {
            misses.push(format!("{english:?} expected {word} first in {heads:?}"));
        }
    }
    assert!(misses.is_empty(), "{}", misses.join("\n"));
}

/// Verbs are searched the way the dictionary writes them: every probe word
/// with a `to <english>` sense surfaces for `to <english>`. (Retrieval used
/// to keep the first 2000 glosses with any word of the query, and `to`
/// alone filled them.)
#[test]
fn verbs_surface_when_searched_with_to() {
    let shouci = shouci();
    let mut checked = 0usize;
    let mut misses = Vec::new();
    for probe in load_search_probes() {
        let Ok(entry) =
            shouci.search_dictionary(&probe.simplified, Some(QueryKind::Chinese), Some(5))
        else {
            continue;
        };
        let verb = format!("to {}", probe.english.to_lowercase());
        let is_verb = entry.candidates.iter().any(|c| {
            c.simplified == probe.simplified && c.glosses.iter().any(|g| g.to_lowercase() == verb)
        });
        if !is_verb {
            continue;
        }
        checked += 1;
        let found = shouci
            .search_dictionary(&verb, None, Some(POPOVER))
            .expect("search");
        if !found
            .candidates
            .iter()
            .any(|c| c.simplified == probe.simplified)
        {
            let heads: Vec<&str> = found
                .candidates
                .iter()
                .map(|c| c.simplified.as_str())
                .collect();
            misses.push(format!(
                "{verb:?} expected {} in {heads:?}",
                probe.simplified
            ));
        }
    }
    assert!(checked > 100, "only {checked} verb probes");
    assert!(
        misses.is_empty(),
        "{} / {checked} missed:\n{}",
        misses.len(),
        misses.join("\n")
    );
}
