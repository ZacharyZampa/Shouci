//! Input/output search quality against the HSK probe list.
//!
//! Drives the TUI the same way a user does (mode + query → result list).
//! Requires a built `dictionary.db` (see README Build).

use vocab_capture::{SearchMode, open_service};
use vocab_search::load_search_probes;

use crate::App;

const TOP: usize = 10;
const SMOKE: usize = 50;

fn app_from_installed_dict() -> App {
    let service = open_service(None).unwrap_or_else(|err| {
        panic!(
            "search quality tests need dictionary.db — ingest it first (see README Build):\n{err}"
        );
    });
    let user_conn = rusqlite::Connection::open_in_memory().expect("in-memory user db");
    vocab_db::apply_schema(&user_conn).expect("user schema");
    App::new(service, user_conn)
}

fn record_miss(misses: &mut Vec<String>, kind: &str, query: &str, expected: &str, got: &[String]) {
    if !got.iter().any(|head| head == expected) {
        misses.push(format!("{kind} {query:?} expected {expected} in {got:?}"));
    }
}

fn run_probes(limit: usize, english: bool) {
    let mut app = app_from_installed_dict();
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
                SearchMode::Pinyin,
                probe.numbered.as_str(),
            ),
            ("hanzi", SearchMode::Chinese, probe.simplified.as_str()),
        ];
        if english {
            cases.insert(0, ("english", SearchMode::English, probe.english.as_str()));
        }
        if probe.pinyin.contains(' ') {
            cases.insert(
                1,
                ("pinyin-untoned", SearchMode::Pinyin, probe.untoned.as_str()),
            );
        }
        for (kind, mode, query) in cases {
            lookups += 1;
            if let Err(err) = app.io_search(mode, query) {
                misses.push(format!("{kind} {query:?} error: {err}"));
                continue;
            }
            record_miss(
                &mut misses,
                kind,
                query,
                &probe.simplified,
                &app.io_headwords(TOP),
            );
        }
    }
    assert!(
        misses.is_empty(),
        "{} / {} TUI probe lookups missed:\n{}",
        misses.len(),
        lookups,
        misses.join("\n")
    );
}

#[test]
fn top50_english_pinyin_hanzi_surface_in_tui() {
    run_probes(SMOKE, true);
}

#[test]
fn top1000_pinyin_and_hanzi_surface_in_tui() {
    run_probes(1000, false);
}
