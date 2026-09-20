use std::path::PathBuf;

use vocab_core::{ItemStatus, Provenance, SourceId, SourceVersion};
use vocab_db::{
    ExportRun, ImportDecision, ImportPayload, ImportRecord, NewVocabItem, commit_export,
    import_sources, item_tags, list_items, persist_import, save_vocab_item, select_exportable,
};
use vocab_pleco::{ExportRow, PlecoCodec, Utf8TextV1};

fn conn() -> rusqlite::Connection {
    let c = rusqlite::Connection::open_in_memory().unwrap();
    vocab_db::apply_schema(&c).unwrap();
    c
}

fn record(simplified: &str, pinyin: &str, definition: &str) -> ImportRecord {
    ImportRecord {
        simplified: simplified.to_owned(),
        pinyin: pinyin.to_owned(),
        definition: definition.to_owned(),
        category: None,
    }
}

fn payload(
    source_path: &str,
    records: Vec<ImportRecord>,
    errors: usize,
    warnings: usize,
) -> ImportPayload {
    ImportPayload {
        source_path: source_path.to_owned(),
        codec_key: "pleco-utf8-text/v1".to_owned(),
        content_sha256: "abcd".repeat(16),
        records,
        issues_errors: errors,
        issues_warnings: warnings,
    }
}

fn save_direct(conn: &rusqlite::Connection, simplified: &str, status: ItemStatus) {
    let provenance = Provenance {
        source: SourceId("user".to_owned()),
        source_version: SourceVersion("manual".to_owned()),
        import_origin: None,
        confirmation: vocab_core::ConfirmationState::UserConfirmed,
    };
    save_vocab_item(
        conn,
        &NewVocabItem {
            simplified: simplified.to_owned(),
            traditional: simplified.to_owned(),
            pinyin: "ping1".to_owned(),
            definition: "def".to_owned(),
            status,
            notes: None,
            source_entry_id: None,
            provenance,
            origin_export_id: None,
        },
    )
    .unwrap();
}

#[test]
fn import_persists_records_tags_and_run() {
    let mut c = conn();
    let payload = payload(
        "categories.txt",
        vec![
            ImportRecord {
                simplified: "你好".to_owned(),
                pinyin: "ni3 hao3".to_owned(),
                definition: "hello".to_owned(),
                category: Some("Greetings".to_owned()),
            },
            record("世界", "shi4 jie4", "world"),
        ],
        0,
        0,
    );
    let decision = persist_import(&mut c, &payload, false).unwrap();
    let ImportDecision::Imported(summary) = decision else {
        panic!("expected import");
    };
    assert_eq!(summary.records_imported, 2);
    assert_eq!(summary.records_skipped_duplicate, 0);

    let items = list_items(&c, None).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].status, ItemStatus::Confirmed);

    let tags = item_tags(&c, items[1].item_id).unwrap();
    assert_eq!(tags, vec!["Greetings"]);

    let imported: i64 = c
        .query_row(
            "SELECT records_imported FROM import_runs WHERE source_path = 'categories.txt'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(imported, 2);
    assert_eq!(
        import_sources(&c).unwrap(),
        vec!["categories.txt".to_owned()]
    );
}

#[test]
fn import_missing_definition_lands_in_needs_review() {
    let mut c = conn();
    let payload = payload(
        "missing-def.txt",
        vec![record("再见", "zai4 jian4", "")],
        0,
        1,
    );
    let ImportDecision::Imported(_) = persist_import(&mut c, &payload, false).unwrap() else {
        panic!("expected import");
    };
    assert_eq!(
        list_items(&c, None).unwrap()[0].status,
        ItemStatus::NeedsReview
    );
}

#[test]
fn import_skips_duplicates_on_second_run() {
    let mut c = conn();
    let payload = payload(
        "twice.txt",
        vec![record("旅行", "lv3 xing2", "travel")],
        0,
        0,
    );
    let ImportDecision::Imported(first) = persist_import(&mut c, &payload, false).unwrap() else {
        panic!("expected import");
    };
    assert_eq!(first.records_imported, 1);
    let ImportDecision::Imported(second) = persist_import(&mut c, &payload, false).unwrap() else {
        panic!("expected import");
    };
    assert_eq!(second.records_imported, 0);
    assert_eq!(second.records_skipped_duplicate, 1);
    assert_eq!(list_items(&c, None).unwrap().len(), 1);
}

#[test]
fn import_with_errors_is_refused_without_force() {
    let mut c = conn();
    let payload = payload(
        "malformed.txt",
        vec![record("世界", "shi4 jie4", "world")],
        2,
        0,
    );
    let decision = persist_import(&mut c, &payload, false).unwrap();
    assert_eq!(decision, ImportDecision::Refused);
    assert!(list_items(&c, None).unwrap().is_empty());
    assert!(import_sources(&c).unwrap().is_empty());

    let ImportDecision::Imported(summary) = persist_import(&mut c, &payload, true).unwrap() else {
        panic!("expected forced import");
    };
    assert_eq!(summary.records_imported, 1);
    assert_eq!(summary.issues_errors, 2);
}

#[test]
fn export_defaults_to_confirmed_only() {
    let c = conn();
    save_direct(&c, "旅行", ItemStatus::Confirmed);
    save_direct(&c, "未解决", ItemStatus::NeedsReview);
    let confirmed = select_exportable(&c, &[], false).unwrap();
    assert_eq!(confirmed.len(), 1);
    assert_eq!(confirmed[0].simplified, "旅行");
    let all = select_exportable(&c, &[], true).unwrap();
    assert_eq!(all.len(), 2);
}

#[test]
fn export_tag_filter_requires_every_tag() {
    let c = conn();
    save_direct(&c, "旅行", ItemStatus::Confirmed);
    save_direct(&c, "米饭", ItemStatus::Confirmed);
    let ids = list_items(&c, None).unwrap();
    let id = |needle: &str| {
        ids.iter()
            .find(|item| item.simplified == needle)
            .unwrap()
            .item_id
    };
    vocab_db::add_tag(&c, id("旅行"), "Food").unwrap();
    vocab_db::add_tag(&c, id("米饭"), "Food").unwrap();
    vocab_db::add_tag(&c, id("旅行"), "Travel").unwrap();
    let both = select_exportable(&c, &["Food".to_owned(), "Travel".to_owned()], false).unwrap();
    assert_eq!(both.len(), 1);
    assert_eq!(both[0].simplified, "旅行");
}

#[test]
fn commit_export_marks_items_as_exported() {
    let mut c = conn();
    save_direct(&c, "旅行", ItemStatus::Confirmed);
    let item = list_items(&c, None).unwrap().remove(0);
    commit_export(
        &mut c,
        &ExportRun {
            target_path: "/tmp/export.txt".to_owned(),
            codec_key: "pleco-utf8-text/v1".to_owned(),
            records_written: 1,
        },
        &[item.item_id],
    )
    .unwrap();
    assert_eq!(
        list_items(&c, None).unwrap()[0].status,
        ItemStatus::Exported
    );
    let run: String = c
        .query_row(
            "SELECT status FROM export_runs WHERE target_path = '/tmp/export.txt'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(run, "committed");
    let events: i64 = c
        .query_row("SELECT count(*) FROM audit_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(events, 1);
}

#[test]
fn round_trip_through_the_real_codec() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/pleco/v1/valid/categories.txt");
    let bytes = std::fs::read(&path).unwrap();
    let parsed = Utf8TextV1.parse(&bytes).unwrap();
    assert_eq!(parsed.records.len(), 3);

    let records: Vec<ImportRecord> = parsed
        .records
        .iter()
        .map(|r| ImportRecord {
            simplified: r.simplified.clone(),
            pinyin: r.pinyin.clone(),
            definition: r.definition.clone(),
            category: r.category.clone(),
        })
        .collect();
    let mut c = conn();
    let ImportDecision::Imported(summary) =
        persist_import(&mut c, &payload("categories.txt", records, 0, 0), false).unwrap()
    else {
        panic!("expected import");
    };
    assert_eq!(summary.records_imported, 3);

    let items = select_exportable(&c, &[], true).unwrap();
    let rows: Vec<ExportRow> = items
        .iter()
        .map(|item| ExportRow {
            simplified: item.simplified.clone(),
            pinyin: item.pinyin.clone(),
            definition: item.definition.clone(),
            category: item_tags(&c, item.item_id).unwrap().into_iter().next(),
        })
        .collect();
    let round = Utf8TextV1.serialize(&rows).unwrap();
    let reparsed = Utf8TextV1.parse(&round).unwrap();
    assert_eq!(reparsed.records.len(), parsed.records.len());
    for (original, roundtripped) in parsed.records.iter().zip(&reparsed.records) {
        assert_eq!(roundtripped.simplified, original.simplified);
        assert_eq!(roundtripped.pinyin, original.pinyin);
        assert_eq!(roundtripped.definition, original.definition);
        assert_eq!(roundtripped.category, original.category);
    }
}
