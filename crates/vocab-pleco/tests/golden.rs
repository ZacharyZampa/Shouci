use std::path::PathBuf;

use vocab_core::connector::{Connector, ExportRecord, Field, Severity, WriteOptions};
use vocab_pleco::{ExportRow, Pleco, Utf8TextV1};

fn fixture(rel: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/pleco")
        .join(rel);
    std::fs::read(&path).unwrap_or_else(|err| panic!("missing fixture {rel}: {err}"))
}

fn record(simplified: &str, pinyin: &str, definition: &str) -> ExportRecord {
    ExportRecord {
        simplified: simplified.to_owned(),
        traditional: simplified.to_owned(),
        pinyin: pinyin.to_owned(),
        definition: definition.to_owned(),
        notes: String::new(),
        definition_is_dictionary_default: false,
        tags: Vec::new(),
        collections: Vec::new(),
    }
}

#[test]
fn parses_basic_flashcards() {
    let parsed = Pleco
        .parse(&fixture("v1/valid/basic-flashcards.txt"))
        .unwrap();
    assert_eq!(parsed.records.len(), 2);
    assert_eq!(parsed.records[0].headword, "你好");
    assert_eq!(
        parsed.records[0].pinyin,
        Field::Present("ni3 hao3".to_owned())
    );
    assert_eq!(
        parsed.records[0].definition,
        Field::Present("hello".to_owned())
    );
    assert_eq!(
        parsed.records[0].tags,
        Field::Omitted,
        "Pleco files have no tags"
    );
    assert!(!parsed.has_errors());
}

#[test]
fn categories_become_collections() {
    let parsed = Pleco.parse(&fixture("v1/valid/categories.txt")).unwrap();
    let collections: Vec<_> = parsed
        .records
        .iter()
        .map(|record| record.collections.clone())
        .collect();
    assert_eq!(
        collections,
        vec![
            Field::Present(vec!["Greetings".to_owned()]),
            Field::Present(vec!["Greetings".to_owned()]),
            Field::Present(vec!["Food".to_owned()]),
        ]
    );
}

#[test]
fn comments_are_skipped() {
    let parsed = Pleco.parse(&fixture("v1/valid/comments.txt")).unwrap();
    assert_eq!(parsed.records.len(), 1);
    assert!(parsed.issues.iter().any(|i| i.severity == Severity::Info));
    assert!(!parsed.has_errors());
}

#[test]
fn malformed_lines_are_reported_not_silently_dropped() {
    let parsed = Pleco
        .parse(&fixture("v1/malformed/missing-fields.txt"))
        .unwrap();
    assert!(parsed.has_errors());
    let warning = parsed
        .issues
        .iter()
        .find(|i| i.severity == Severity::Warning)
        .expect("missing definition must be surfaced as a warning");
    assert_eq!(warning.line, 1);
    assert_eq!(
        parsed.records[0].definition,
        Field::Omitted,
        "no definition field means Pleco's own"
    );
    let error = parsed
        .issues
        .iter()
        .find(|i| i.severity == Severity::Error)
        .expect("a lone headword must be surfaced as an error");
    assert_eq!(error.line, 3);
    assert!(parsed.issues.iter().any(|i| i.line == 2));
}

#[test]
fn non_utf8_input_is_rejected_with_message() {
    let err = Pleco.parse(b"\xff\xfe garbage").unwrap_err();
    assert_eq!(err.kind(), vocab_core::ErrorKind::Format);
}

#[test]
fn grammar_round_trip_is_stable() {
    let rows = [
        ExportRow {
            headword: "你好".to_owned(),
            pinyin: "ni3 hao3".to_owned(),
            definition: "hello; hi".to_owned(),
            category: Some("Greetings".to_owned()),
        },
        ExportRow {
            headword: "米饭".to_owned(),
            pinyin: "mi3 fan4".to_owned(),
            definition: "cooked rice".to_owned(),
            category: Some("Food".to_owned()),
        },
    ];
    let reparsed = Utf8TextV1.parse(&Utf8TextV1.serialize(&rows)).unwrap();
    assert_eq!(reparsed.records.len(), 2);
    assert_eq!(reparsed.records[0].category.as_deref(), Some("Greetings"));
    assert_eq!(reparsed.records[1].category.as_deref(), Some("Food"));
    assert_eq!(reparsed.records[0].definition.as_deref(), Some("hello; hi"));
}

#[test]
fn dictionary_definitions_are_left_blank_for_pleco() {
    let mut school = record("学校", "xue2 xiao4", "school");
    school.definition_is_dictionary_default = true;
    let custom = record("米饭", "mi3 fan4", "rice, the way grandma makes it");
    let written = Pleco
        .write(&[school, custom], &WriteOptions::default())
        .unwrap();
    let text = String::from_utf8(written.bytes).unwrap();
    assert_eq!(
        text,
        "学校\txue2 xiao4\t\n米饭\tmi3 fan4\trice, the way grandma makes it\n"
    );
    assert!(written.notes.iter().any(|note| note.contains("left blank")));
}

#[test]
fn one_collection_per_word_and_the_rest_is_reported() {
    let mut a = record("一", "yi1", "one");
    a.collections = vec!["Numbers".to_owned(), "HSK 1".to_owned()];
    let mut b = record("猫", "mao1", "cat");
    b.collections = vec!["Animals".to_owned()];
    let plain = record("好", "hao3", "good");
    let written = Pleco
        .write(&[a, b, plain], &WriteOptions::default())
        .unwrap();
    let text = String::from_utf8(written.bytes).unwrap();
    assert_eq!(
        text,
        "好\thao3\tgood\n[Animals]\n猫\tmao1\tcat\n[HSK 1]\n一\tyi1\tone\n"
    );
    assert!(
        written
            .notes
            .iter()
            .any(|note| note.contains("一 (Numbers)")),
        "{:?}",
        written.notes
    );
}

#[test]
fn exporting_one_collection_uses_it_as_the_category() {
    let mut a = record("一", "yi1", "one");
    a.collections = vec!["Numbers".to_owned(), "HSK 1".to_owned()];
    let written = Pleco
        .write(
            &[a],
            &WriteOptions {
                collection: Some("Numbers".to_owned()),
                deck: None,
            },
        )
        .unwrap();
    assert_eq!(
        String::from_utf8(written.bytes).unwrap(),
        "[Numbers]\n一\tyi1\tone\n"
    );
    assert!(written.notes.is_empty(), "{:?}", written.notes);
}
