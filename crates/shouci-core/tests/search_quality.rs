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
const POPOVER: usize = 10;

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
