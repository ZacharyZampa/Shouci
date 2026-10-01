use std::path::Path;

use vocab_dictionary::{CedictSource, SqliteDictionary, build_dictionary_db};
use vocab_search::{DeterministicRanker, QueryKind, SearchService};

fn service() -> SearchService<SqliteDictionary> {
    let artifact = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/dictionary/cedict-sample.u8"),
    )
    .expect("fixture");
    let mut conn = rusqlite::Connection::open_in_memory().expect("memory");
    build_dictionary_db(&mut conn, &CedictSource::default(), &artifact).expect("build");
    let dictionary = SqliteDictionary::from_connection(conn).expect("provider");
    SearchService::new(dictionary, DeterministicRanker::default())
}

#[test]
fn falls_back_from_english_to_pinyin() {
    let found = service().search_auto("nihao").unwrap();
    assert_eq!(found.guessed, QueryKind::English);
    assert_eq!(found.kind, QueryKind::Pinyin);
    assert_eq!(found.candidates[0].entry.simplified, "你好");
}

#[test]
fn keeps_english_when_it_hits() {
    let found = service().search_auto("school").unwrap();
    assert_eq!(found.kind, QueryKind::English);
    assert_eq!(found.candidates[0].entry.simplified, "学校");
}

#[test]
fn chinese_is_looked_up_directly() {
    let found = service().search_auto("旅行").unwrap();
    assert_eq!(found.kind, QueryKind::Chinese);
    assert_eq!(found.candidates[0].entry.simplified, "旅行");
}

#[test]
fn nothing_found_reports_the_guess() {
    let found = service().search_auto("zzzqqq").unwrap();
    assert!(found.candidates.is_empty());
    assert_eq!(found.kind, found.guessed);
}

fn service_with(text: &str) -> SearchService<SqliteDictionary> {
    let mut conn = rusqlite::Connection::open_in_memory().expect("memory");
    build_dictionary_db(&mut conn, &CedictSource::default(), text.as_bytes()).expect("build");
    let dictionary = SqliteDictionary::from_connection(conn).expect("provider");
    SearchService::new(dictionary, DeterministicRanker::default())
}

#[test]
fn a_single_untoned_syllable_reaches_pinyin() {
    let service = service_with(
        "我 我 [wo3] /I; me; my/\n炒鍋 炒锅 [chao3 guo1] /wok/\n能 能 [neng2] /can; to be able to/\n",
    );
    let wo = service.search_auto("wo").unwrap();
    assert_eq!(wo.kind, QueryKind::Pinyin, "\"wok\" is not the word wo");
    assert_eq!(wo.candidates[0].entry.simplified, "我");
    let can = service.search_auto("can").unwrap();
    assert_eq!(can.kind, QueryKind::English, "can is an English word");
}
