use std::path::PathBuf;

use rusqlite::Connection;
use vocab_capture::{
    CaptureOutcome, SearchMode, capture, capture_to_path, dictionary_path, guess_mode,
    item_from_candidate, open_service, resolve, resolve_auto, save_candidate, search_auto,
};
use vocab_core::ItemStatus;
use vocab_dictionary::{CedictSource, SqliteDictionary, build_dictionary_db};
use vocab_search::SearchService;

fn fixture() -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/dictionary/cedict-sample.u8");
    std::fs::read(&path).expect("missing cedict sample fixture")
}

fn service() -> SearchService<SqliteDictionary> {
    let mut conn = Connection::open_in_memory().expect("in-memory db");
    let artifact = fixture();
    build_dictionary_db(&mut conn, &CedictSource::default(), &artifact).expect("build dictionary");
    let dictionary = SqliteDictionary::from_connection(conn).expect("open provider");
    SearchService::new(dictionary, vocab_search::DeterministicRanker::default())
}

fn user_conn() -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory user db");
    vocab_db::apply_schema(&conn).expect("apply schema");
    conn
}

fn item_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM vocabulary_items", [], |row| {
        row.get(0)
    })
    .expect("count items")
}

#[test]
fn guess_mode_classifies_common_queries() {
    assert_eq!(guess_mode("旅行"), SearchMode::Chinese);
    assert_eq!(guess_mode("lv3"), SearchMode::Pinyin);
    assert_eq!(guess_mode("lv3xing2"), SearchMode::Pinyin);
    assert_eq!(guess_mode("to travel"), SearchMode::English);
    assert_eq!(guess_mode("nihao"), SearchMode::English);
    assert_eq!(guess_mode("猫māo"), SearchMode::Pinyin);
}

#[test]
fn unique_strong_match_saves_confirmed() {
    let svc = service();
    let conn = user_conn();
    let outcome = capture(&svc, &conn, "旅行", None).expect("capture");
    match outcome {
        CaptureOutcome::Saved(item) => {
            assert_eq!(item.status, ItemStatus::Confirmed);
            assert_eq!(item.simplified, "旅行");
            assert_eq!(item.pinyin, "lv3 xing2");
            assert!(item.source_entry_id.is_some());
        }
        other => panic!("expected Saved, got {other:?}"),
    }
    assert_eq!(item_count(&conn), 1);
}

#[test]
fn ambiguous_matches_save_nothing() {
    let svc = service();
    let conn = user_conn();
    let outcome = capture(&svc, &conn, "lv3", None).expect("capture");
    match outcome {
        CaptureOutcome::Ambiguous { query, candidates } => {
            assert_eq!(query, "lv3");
            let simplified: Vec<&str> = candidates
                .iter()
                .map(|candidate| candidate.entry.simplified.as_str())
                .collect();
            assert!(simplified.contains(&"旅途"));
            assert!(simplified.contains(&"旅行"));
        }
        other => panic!("expected Ambiguous, got {other:?}"),
    }
    assert_eq!(item_count(&conn), 0, "ambiguity must not write an item");
}

#[test]
fn no_match_saves_needs_review() {
    let svc = service();
    let conn = user_conn();
    let outcome = capture(&svc, &conn, "zzzqqq", None).expect("capture");
    match outcome {
        CaptureOutcome::Saved(item) => {
            assert_eq!(item.status, ItemStatus::NeedsReview);
            assert_eq!(item.simplified, "zzzqqq");
            assert_eq!(item.definition, "unresolved query: zzzqqq");
            assert!(item.source_entry_id.is_none());
        }
        other => panic!("expected Saved(NeedsReview), got {other:?}"),
    }
    assert_eq!(item_count(&conn), 1);
}

#[test]
fn duplicate_capture_reports_already_saved() {
    let svc = service();
    let conn = user_conn();
    assert!(matches!(
        capture(&svc, &conn, "旅行", None).expect("first"),
        CaptureOutcome::Saved(_)
    ));
    match capture(&svc, &conn, "旅行", None).expect("second") {
        CaptureOutcome::AlreadySaved(item) => assert_eq!(item.simplified, "旅行"),
        other => panic!("expected AlreadySaved, got {other:?}"),
    }
    assert_eq!(item_count(&conn), 1);
}

#[test]
fn mode_override_beats_guess() {
    let svc = service();
    let conn = user_conn();
    let forced = capture(&svc, &conn, "旅行", Some(SearchMode::English)).expect("capture");
    match forced {
        CaptureOutcome::Saved(item) => assert_eq!(item.status, ItemStatus::NeedsReview),
        other => panic!("expected Saved(NeedsReview), got {other:?}"),
    }

    let fresh = user_conn();
    let pinyin = capture(&svc, &fresh, "lv3 xing2", Some(SearchMode::Pinyin)).expect("capture");
    match pinyin {
        CaptureOutcome::Saved(item) => {
            assert_eq!(item.status, ItemStatus::Confirmed);
            assert_eq!(item.simplified, "旅行");
        }
        other => panic!("expected Saved(Confirmed), got {other:?}"),
    }
}

#[test]
fn empty_query_is_rejected() {
    let svc = service();
    let conn = user_conn();
    assert!(capture(&svc, &conn, "", None).is_err());
    assert_eq!(item_count(&conn), 0);
}

#[test]
fn resolve_honours_mode() {
    let svc = service();
    let chinese = resolve(&svc, SearchMode::Chinese, "旅行").expect("chinese");
    assert_eq!(chinese[0].entry.simplified, "旅行");
    let english = resolve(&svc, SearchMode::English, "travel").expect("english");
    assert!(english.iter().any(|c| c.entry.simplified == "旅行"));
}

#[test]
fn resolve_auto_falls_back_from_english_to_pinyin() {
    let svc = service();
    assert_eq!(guess_mode("nihao"), SearchMode::English);
    let (ranked, mode) = resolve_auto(&svc, "nihao").expect("auto");
    assert_eq!(mode, SearchMode::Pinyin);
    assert_eq!(ranked[0].entry.simplified, "你好");
}

#[test]
fn resolve_auto_keeps_english_when_it_hits() {
    let svc = service();
    let (ranked, mode) = resolve_auto(&svc, "school").expect("auto");
    assert_eq!(mode, SearchMode::English);
    assert_eq!(ranked[0].entry.simplified, "学校");
}

#[test]
fn search_auto_caps_hits_and_labels_guess() {
    let svc = service();
    let (hits, label) = search_auto(&svc, "school", 10).expect("search");
    assert_eq!(label, "auto");
    assert!(hits.iter().any(|c| c.entry.simplified == "学校"));
    let (hits, _) = search_auto(&svc, "cat", 2).expect("search");
    assert!(hits.len() <= 2);
}

#[test]
fn save_candidate_writes_confirmed_item() {
    let svc = service();
    let conn = user_conn();
    let ranked = resolve(&svc, SearchMode::English, "school").expect("search");
    let school = ranked
        .iter()
        .find(|c| c.entry.simplified == "学校")
        .expect("学校");
    let item = item_from_candidate(school);
    assert_eq!(item.status, vocab_core::ItemStatus::Confirmed);
    assert!(item.definition.contains("school"));
    save_candidate(&conn, school).expect("save");
    assert_eq!(item_count(&conn), 1);
}

#[test]
fn capture_to_path_creates_user_db_directory() {
    let svc = service();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("vocab-capture-{}-{nanos}", std::process::id()));
    let db = dir.join("nested/user.db");

    let outcome = capture_to_path(&svc, Some(&db), "旅行", None).expect("capture to path");
    assert!(matches!(outcome, CaptureOutcome::Saved(_)));
    assert!(
        db.is_file(),
        "user.db should be created at {}",
        db.display()
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn open_service_reports_unwritable_dictionary_path() {
    let missing = PathBuf::from("/nonexistent/vocab-capture/dictionary.db");
    let err = open_service(Some(&missing))
        .map(|_| ())
        .expect_err("must fail");
    let text = err.to_string();
    assert!(
        text.contains("fetch failed") || text.contains("dictionary"),
        "{text}"
    );
}

#[test]
fn dictionary_path_prefers_explicit_argument() {
    let explicit = PathBuf::from("/explicit/dictionary.db");
    let resolved = dictionary_path(Some(&explicit));
    assert_eq!(resolved, explicit);
}
