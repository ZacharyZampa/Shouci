//! Input/output search quality for the menu-bar popover.
//!
//! Drives [`vocab_capture::search_auto`] the same way the popover does (query →
//! capped hit list). Requires a built `dictionary.db` (see README Build).

use vocab_capture::{open_service, search_auto};
use vocab_search::load_search_probes;

use crate::capture::RESULT_LIMIT;

fn record_miss(misses: &mut Vec<String>, kind: &str, query: &str, expected: &str, got: &[String]) {
    if !got.iter().any(|head| head == expected) {
        misses.push(format!("{kind} {query:?} expected {expected} in {got:?}"));
    }
}

fn run_probes(limit: usize, english: bool) {
    let service = open_service(None).unwrap_or_else(|err| {
        panic!(
            "search quality tests need dictionary.db — ingest it first (see README Build):\n{err}"
        );
    });
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
            ("pinyin-numbered", probe.numbered.as_str()),
            ("hanzi", probe.simplified.as_str()),
        ];
        if english {
            cases.insert(0, ("english", probe.english.as_str()));
        }
        if probe.pinyin.contains(' ') {
            cases.insert(1, ("pinyin-untoned", probe.untoned.as_str()));
        }
        for (kind, query) in cases {
            lookups += 1;
            match search_auto(&service, query, RESULT_LIMIT) {
                Ok((hits, _)) => {
                    let got: Vec<String> =
                        hits.into_iter().map(|hit| hit.entry.simplified).collect();
                    record_miss(&mut misses, kind, query, &probe.simplified, &got);
                }
                Err(err) => misses.push(format!("{kind} {query:?} error: {err}")),
            }
        }
    }
    assert!(
        misses.is_empty(),
        "{} / {} menu-bar probe lookups missed:\n{}",
        misses.len(),
        lookups,
        misses.join("\n")
    );
}

#[test]
fn top50_queries_surface_in_menubar_auto_search() {
    run_probes(50, true);
}

#[test]
fn top1000_pinyin_and_hanzi_surface_in_menubar_auto_search() {
    run_probes(1000, false);
}
