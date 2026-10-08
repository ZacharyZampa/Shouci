use std::fmt::Write as _;
use std::path::PathBuf;

use vocab_core::MatchBasis;
use vocab_dictionary::{
    CedictSource, DictionaryProvider, FrequencySource, HskSource, SqliteDictionary,
    build_dictionary_db, build_dictionary_db_with_layers,
};
use vocab_pinyin::NormalizedPinyin;
use vocab_search::SearchService;

fn fixture() -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/dictionary/cedict-sample.u8");
    std::fs::read(&path).expect("missing cedict sample fixture")
}

fn service() -> SearchService<SqliteDictionary> {
    let mut conn = rusqlite::Connection::open_in_memory().expect("in-memory db");
    let artifact = fixture();
    let stats = build_dictionary_db(&mut conn, &CedictSource::default(), &artifact)
        .expect("build dictionary db");
    assert_eq!(stats.entries, 19);
    let dictionary = SqliteDictionary::from_connection(conn).expect("open provider");
    SearchService::new(dictionary, vocab_search::DeterministicRanker::default())
}

#[test]
fn build_records_metadata() {
    let mut conn = rusqlite::Connection::open_in_memory().expect("in-memory db");
    let artifact = fixture();
    build_dictionary_db(&mut conn, &CedictSource::default(), &artifact).expect("build");
    let dict = SqliteDictionary::from_connection(conn).expect("open");
    let metadata: std::collections::BTreeMap<String, String> =
        dict.metadata().unwrap().into_iter().collect();
    assert_eq!(
        metadata.get("source_id").map(String::as_str),
        Some("cc-cedict")
    );
    assert_eq!(
        metadata.get("schema_version").map(String::as_str),
        Some("1")
    );
    assert_eq!(dict.schema_version(), "1");
}

#[test]
fn metadata_records_source_sha256() {
    let mut conn = rusqlite::Connection::open_in_memory().expect("in-memory db");
    let artifact = fixture();
    build_dictionary_db(&mut conn, &CedictSource::default(), &artifact).expect("build");
    let dict = SqliteDictionary::from_connection(conn).expect("open");
    let metadata: std::collections::BTreeMap<String, String> =
        dict.metadata().unwrap().into_iter().collect();
    let sha = metadata
        .get("source_sha256")
        .expect("source_sha256 recorded");
    assert_eq!(sha.len(), 64);
    assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn english_search_returns_ranked_candidates() {
    let svc = service();
    let ranked = svc.search_english("cat").expect("search");
    assert_ne!(ranked, []);
    assert!(ranked[0].entry.simplified == "猫" || ranked[0].entry.simplified == "猫咪");
    assert!(ranked.iter().any(|c| c.entry.simplified == "猫"));
    assert!(ranked.iter().any(|c| c.entry.simplified == "猫咪"));
}

#[test]
fn english_query_same_order_twice() {
    let svc = service();
    let a = svc.search_english("school").expect("search");
    let b = svc.search_english("school").expect("search");
    assert_eq!(a, b);
    assert_eq!(a[0].entry.simplified, "学校");
}

#[test]
fn pinyin_finds_dotted_form_via_normalization() {
    let svc = service();
    let ranked = svc
        .search_pinyin(&NormalizedPinyin::from_str("lǚ xíng").unwrap())
        .expect("search");
    assert_eq!(ranked[0].entry.simplified, "旅行");
}

#[test]
fn pinyin_unspaced_segments_into_syllables() {
    let svc = service();
    let ranked = svc
        .search_pinyin(&NormalizedPinyin::from_str("nihao").unwrap())
        .expect("search");
    assert!(
        ranked.iter().any(|c| c.entry.simplified == "你好"),
        "unspaced 'nihao' segments to ni hao and finds 你好"
    );
}

#[test]
fn pinyin_unspaced_with_tone_numbers_segments_too() {
    let svc = service();
    let ranked = svc
        .search_pinyin(&NormalizedPinyin::from_str("lv3xing2").unwrap())
        .expect("search");
    assert!(
        ranked.iter().any(|c| c.entry.simplified == "旅行"),
        "tone numbers stay attached: 'lv3xing2' -> lv3 xing2"
    );
}

#[test]
fn pinyin_ambiguity_shows_all_matches() {
    let svc = service();
    let ranked = svc
        .search_pinyin(&NormalizedPinyin::from_str("lv3").unwrap())
        .expect("search");
    let found: Vec<&str> = ranked.iter().map(|c| c.entry.simplified.as_str()).collect();
    assert!(found.contains(&"旅途"));
    assert!(found.contains(&"旅行"));
}

#[test]
fn chinese_exact_simplified_ranks_first() {
    let svc = service();
    let ranked = svc.lookup_chinese("猫").expect("search");
    assert_eq!(ranked[0].entry.simplified, "猫");
}

#[test]
fn chinese_traditional_lookup_finds_simplified_entry() {
    let svc = service();
    let ranked = svc.lookup_chinese("學校").expect("search");
    assert_eq!(ranked[0].entry.traditional, "學校");
    assert_eq!(ranked[0].entry.simplified, "学校");
}

#[test]
fn multi_reading_entries_remain_distinct() {
    let svc = service();
    let ranked = svc.lookup_chinese("教").expect("search");
    let distinct_pinyin: std::collections::BTreeSet<&str> =
        ranked.iter().map(|c| c.entry.pinyin.as_str()).collect();
    assert!(distinct_pinyin.len() >= 3);
}

#[test]
fn chinese_multi_char_fallback_requires_all_characters() {
    let svc = service();
    let ranked = svc.lookup_chinese("旅行").expect("search");
    for candidate in &ranked {
        assert!(
            candidate.entry.simplified.contains('旅') && candidate.entry.simplified.contains('行'),
            "result {} must contain every query character under AND fallback",
            candidate.entry.simplified
        );
    }
    assert!(
        !ranked.iter().any(|c| c.entry.simplified == "旅途"),
        "OR fallback would leak single-character matches like 旅途"
    );
}

#[test]
fn chinese_single_char_prefixes_and_fallback_stay_distinct() {
    let svc = service();
    let ranked = svc.lookup_chinese("猫").expect("search");
    assert_eq!(ranked[0].entry.simplified, "猫");
    assert!(!ranked[0].diagnostic.is_inferred);
    let fallback: Vec<&str> = ranked
        .iter()
        .filter(|c| c.diagnostic.basis == MatchBasis::CharacterFallback)
        .map(|c| c.entry.simplified.as_str())
        .collect();
    assert_eq!(
        fallback,
        vec!["熊猫"],
        "only non-prefix variants need the fallback"
    );
    for candidate in ranked
        .iter()
        .filter(|c| c.diagnostic.basis == MatchBasis::CharacterFallback)
    {
        assert!(candidate.diagnostic.is_inferred);
        assert!(candidate.entry.simplified.contains('猫'));
    }
    let prefix_clean = ranked
        .iter()
        .find(|c| c.entry.simplified == "猫咪" || c.entry.simplified == "猫熊")
        .expect("prefix matches present");
    assert_eq!(prefix_clean.diagnostic.basis, MatchBasis::Simplified);
    assert!(!prefix_clean.diagnostic.is_inferred);
}

#[test]
fn every_word_of_an_english_query_is_retrieved_before_any_word() {
    // More `to …` glosses than retrieval keeps, all listed before 旅行. A
    // gloss with every word of the query must not be cut off by the ones
    // that only share `to`.
    let mut text = String::new();
    for n in 0..2_500 {
        writeln!(text, "字{n} 字{n} [zi4] /to do thing {n}/").expect("write");
    }
    text.push_str("旅行 旅行 [lu:3 xing2] /to travel/journey/\n");
    let mut conn = rusqlite::Connection::open_in_memory().expect("in-memory db");
    build_dictionary_db(&mut conn, &CedictSource::default(), text.as_bytes()).expect("build");
    let dict = SqliteDictionary::from_connection(conn).expect("open");
    let found = dict.search_english("to travel").expect("search");
    assert!(
        found.iter().any(|c| c.entry.simplified == "旅行"),
        "旅行 was cut off by {} entries that only share `to`",
        found.len()
    );
}

#[test]
fn a_common_word_keeps_its_definitions_when_pointers_fill_the_cap() {
    // Thousands of `see …` pointers share the word `see`; the entry that
    // means it must survive the cap.
    let mut text = String::new();
    for n in 0..2_500 {
        writeln!(text, "字{n} 字{n} [zi4] /see 词{n}[ci2]/").expect("write");
    }
    text.push_str("看見 看见 [kan4 jian4] /to see/to catch sight of/\n");
    let mut conn = rusqlite::Connection::open_in_memory().expect("in-memory db");
    build_dictionary_db(&mut conn, &CedictSource::default(), text.as_bytes()).expect("build");
    let dict = SqliteDictionary::from_connection(conn).expect("open");
    let found = dict.search_english("see").expect("search");
    assert!(
        found.iter().any(|c| c.entry.simplified == "看见"),
        "看见 was cut off by {} pointer entries",
        found.len()
    );
}

#[test]
fn words_inside_a_chinese_query_that_is_not_a_word() {
    let svc = service();
    let found = svc.lookup_chinese("学校教师").expect("search");
    let parts: Vec<&str> = found
        .iter()
        .filter(|c| c.diagnostic.basis == MatchBasis::ContainedWord)
        .map(|c| c.entry.simplified.as_str())
        .collect();
    assert_eq!(parts, ["学校", "教师"], "longest first, in reading order");
    assert!(
        found.iter().all(|c| c.diagnostic.is_inferred),
        "none of them is the query"
    );

    let characters = svc.lookup_chinese("猫水").expect("search");
    let parts: Vec<&str> = characters
        .iter()
        .map(|c| c.entry.simplified.as_str())
        .collect();
    assert_eq!(
        parts,
        ["猫", "水"],
        "single characters when no longer word fits"
    );

    let mixed = svc.lookup_chinese("猫喝水").expect("search");
    let parts: Vec<&str> = mixed.iter().map(|c| c.entry.simplified.as_str()).collect();
    assert!(
        parts.contains(&"喝水") && parts.contains(&"猫"),
        "{parts:?}"
    );
    assert!(!parts.contains(&"水"), "水 is covered by 喝水: {parts:?}");

    let word = svc.lookup_chinese("学校").expect("search");
    assert!(
        word.iter()
            .all(|c| c.diagnostic.basis != MatchBasis::ContainedWord),
        "a word is not broken into parts"
    );
}

#[test]
fn english_trigram_finds_infix_substring() {
    let svc = service();
    let ranked = svc.search_english("colloquia").expect("search");
    assert!(
        ranked.iter().any(|c| c.entry.simplified == "猫咪"),
        "trigram phrase match should surface the gloss substring"
    );
}

#[test]
fn rebuild_is_stable() {
    let build = || {
        let mut conn = rusqlite::Connection::open_in_memory().expect("in-memory db");
        let artifact = fixture();
        build_dictionary_db(&mut conn, &CedictSource::default(), &artifact).expect("build");
        conn.query_row(
            "SELECT value FROM dictionary_metadata WHERE key = 'entry_count'",
            [],
            |row| row.get::<_, String>(0),
        )
        .expect("read count")
    };
    assert_eq!(build(), build());
}

#[test]
fn enrichment_writes_frequency_and_hsk_ranks() {
    let mut conn = rusqlite::Connection::open_in_memory().expect("in-memory db");
    let artifact = fixture();
    let frequency = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/dictionary/frequency-sample.txt"),
    )
    .expect("frequency fixture");
    let hsk = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/dictionary/hsk-sample.txt"),
    )
    .expect("hsk fixture");
    let freq_source = FrequencySource::default();
    let hsk_source = HskSource::default();
    let stats = build_dictionary_db_with_layers(
        &mut conn,
        &CedictSource::default(),
        &artifact,
        &[
            (&freq_source, frequency.as_slice()),
            (&hsk_source, hsk.as_slice()),
        ],
    )
    .expect("build with layers");
    assert!(stats.frequency_updated > 0);
    assert!(stats.hsk_updated > 0);
    let dictionary = SqliteDictionary::from_connection(conn).expect("open");
    let ranked = dictionary.headword_ranks().expect("headword ranks");
    let cat = ranked.iter().find(|e| e.simplified == "猫").expect("猫");
    assert_eq!((cat.frequency_rank, cat.hsk_rank), (Some(3), Some(2)));
    assert!(
        ranked
            .iter()
            .all(|e| e.frequency_rank.is_some() || e.hsk_rank.is_some()),
        "only headwords with a rank"
    );
    assert!(!ranked.iter().any(|e| e.simplified == "世界"), "unranked");
    let svc = SearchService::new(dictionary, vocab_search::DeterministicRanker::default());
    let ranked = svc.search_english("cat").expect("search");
    let cat = ranked
        .iter()
        .find(|c| c.entry.simplified == "猫")
        .expect("猫");
    assert_eq!(cat.entry.frequency_rank, Some(3));
    assert_eq!(cat.entry.hsk_rank, Some(2));
    let slang = ranked
        .iter()
        .find(|c| c.entry.simplified == "猫咪")
        .expect("猫咪");
    assert_eq!(slang.entry.frequency_rank, Some(6));
    assert_eq!(slang.entry.hsk_rank, None);
    assert_eq!(ranked[0].entry.simplified, "猫");
}

use std::str::FromStr;
