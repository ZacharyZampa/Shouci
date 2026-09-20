use std::path::PathBuf;

use vocab_pleco::{ExportRow, IssueSeverity, PlecoCodec, Utf8TextV1};

fn fixture(rel: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/pleco")
        .join(rel);
    std::fs::read(&path).unwrap_or_else(|err| panic!("missing fixture {rel}: {err}"))
}

#[test]
fn parses_basic_flashcards() {
    let parsed = Utf8TextV1
        .parse(&fixture("v1/valid/basic-flashcards.txt"))
        .unwrap();
    assert_eq!(parsed.variant.key(), "pleco-utf8-text/v1");
    assert_eq!(parsed.records.len(), 2);
    assert_eq!(parsed.records[0].simplified, "你好");
    assert_eq!(parsed.records[0].pinyin, "ni3 hao3");
    assert_eq!(parsed.records[0].definition, "hello");
    assert!(!parsed.has_errors());
}

#[test]
fn category_headers_attach_to_following_records() {
    let parsed = Utf8TextV1
        .parse(&fixture("v1/valid/categories.txt"))
        .unwrap();
    assert_eq!(parsed.records.len(), 3);
    assert_eq!(parsed.records[0].category.as_deref(), Some("Greetings"));
    assert_eq!(parsed.records[1].category.as_deref(), Some("Greetings"));
    assert_eq!(parsed.records[2].category.as_deref(), Some("Food"));
}

#[test]
fn comments_are_skipped() {
    let parsed = Utf8TextV1.parse(&fixture("v1/valid/comments.txt")).unwrap();
    assert_eq!(parsed.records.len(), 1);
    assert!(
        parsed
            .issues
            .iter()
            .any(|i| i.severity == IssueSeverity::Info)
    );
    assert!(!parsed.has_errors());
}

#[test]
fn malformed_lines_are_reported_not_silently_dropped() {
    let parsed = Utf8TextV1
        .parse(&fixture("v1/malformed/missing-fields.txt"))
        .unwrap();
    assert!(parsed.has_errors());
    let warning = parsed
        .issues
        .iter()
        .find(|i| i.severity == IssueSeverity::Warning)
        .expect("missing definition must be surfaced as a warning");
    assert_eq!(warning.line, 1);
    let error = parsed
        .issues
        .iter()
        .find(|i| i.severity == IssueSeverity::Error)
        .expect("empty headword must be surfaced as an error");
    assert_eq!(error.line, 3);
    assert!(parsed.issues.iter().any(|i| i.line == 2));
}

#[test]
fn non_utf8_input_is_rejected_with_message() {
    assert!(Utf8TextV1.parse(b"\xff\xfe garbage").is_err());
}

#[test]
fn round_trip_serialize_to_parse_is_stable() {
    let rows = [
        ExportRow {
            simplified: "你好".to_owned(),
            pinyin: "ni3 hao3".to_owned(),
            definition: "hello; hi".to_owned(),
            category: Some("Greetings".to_owned()),
        },
        ExportRow {
            simplified: "米饭".to_owned(),
            pinyin: "mi3 fan4".to_owned(),
            definition: "cooked rice".to_owned(),
            category: Some("Food".to_owned()),
        },
    ];
    let bytes = Utf8TextV1.serialize(&rows).unwrap();
    let reparsed = Utf8TextV1.parse(&bytes).unwrap();
    assert_eq!(reparsed.records.len(), 2);
    assert_eq!(reparsed.records[0].category.as_deref(), Some("Greetings"));
    assert_eq!(reparsed.records[1].category.as_deref(), Some("Food"));
    assert_eq!(reparsed.records[0].definition, "hello; hi");
}
