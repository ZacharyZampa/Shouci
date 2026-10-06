use std::path::PathBuf;

use vocab_core::connector::{Connector, ExportRecord, Field, Severity, WriteOptions};
use vocab_pleco::{ExportRow, Pleco, Utf8TextV1};

fn fixture(rel: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/pleco")
        .join(rel);
    std::fs::read(&path).unwrap_or_else(|err| panic!("missing fixture {rel}: {err}"))
}

fn record(simplified: &str, traditional: &str, pinyin: &str, definition: &str) -> ExportRecord {
    ExportRecord::new(simplified, traditional, pinyin, definition)
}

fn text(present: &str) -> Field<String> {
    Field::Present(present.to_owned())
}

#[test]
fn parses_basic_flashcards() {
    let parsed = Pleco
        .parse(&fixture("v1/valid/basic-flashcards.txt"))
        .unwrap();
    assert_eq!(parsed.records.len(), 2);
    assert_eq!(parsed.records[0].headword, "你好");
    assert_eq!(parsed.records[0].pinyin, text("ni3 hao3"));
    assert_eq!(parsed.records[0].definition, text("hello"));
    assert_eq!(
        parsed.records[0].tags,
        Field::Omitted,
        "Pleco files have no tags"
    );
    assert!(!parsed.has_errors());
}

#[test]
fn double_slash_lines_start_categories() {
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
    assert!(!parsed.has_errors());
}

#[test]
fn characters_alone_and_traditional_in_brackets_are_valid() {
    let parsed = Pleco.parse(&fixture("v1/valid/headword-only.txt")).unwrap();
    assert!(!parsed.has_errors(), "{:?}", parsed.issues);
    let hello = &parsed.records[0];
    assert_eq!(hello.headword, "你好");
    assert_eq!(hello.pinyin, Field::Omitted);
    assert_eq!(hello.definition, Field::Omitted, "Pleco fills it in");
    let school = &parsed.records[1];
    assert_eq!(school.headword, "学校");
    assert_eq!(school.traditional, text("學校"));
    let rice = &parsed.records[2];
    assert_eq!(rice.pinyin, Field::Omitted);
    assert_eq!(rice.definition, text("cooked rice"));
}

#[test]
fn proof_of_concept_files_still_import() {
    let parsed = Pleco.parse(&fixture("v1/valid/poc-brackets.txt")).unwrap();
    assert!(!parsed.has_errors());
    assert_eq!(parsed.records.len(), 1);
    assert_eq!(
        parsed.records[0].collections,
        Field::Present(vec!["Greetings".to_owned()])
    );
}

#[test]
fn a_record_without_characters_is_an_error_with_its_line() {
    let parsed = Pleco
        .parse(&fixture("v1/malformed/empty-headword.txt"))
        .unwrap();
    let error = parsed
        .issues
        .iter()
        .find(|i| i.severity == Severity::Error)
        .expect("error");
    assert_eq!(error.line, 1);
    assert_eq!(parsed.records.len(), 1);
}

#[test]
fn byte_order_mark_and_crlf_are_ignored() {
    let parsed = Pleco
        .parse("\u{feff}//Food\r\n米饭\tmi3 fan4\tcooked rice\r\n".as_bytes())
        .unwrap();
    assert_eq!(parsed.records[0].headword, "米饭");
    assert_eq!(parsed.records[0].definition, text("cooked rice"));
    assert_eq!(
        parsed.records[0].collections,
        Field::Present(vec!["Food".to_owned()])
    );
}

/// A real iOS export: BOM, CRLF, `// Category` with a space, the bracket
/// written even when the forms match, unspaced pinyin, and `//` splitting
/// separable words.
#[test]
fn a_real_pleco_text_export_reads_as_written() {
    let parsed = Pleco.parse(&fixture("v1/samples/export-text.txt")).unwrap();
    assert!(parsed.issues.is_empty(), "{:?}", parsed.issues);
    assert_eq!(parsed.records.len(), 12);
    assert!(
        parsed
            .records
            .iter()
            .all(|record| record.collections == Field::Present(vec!["Class Words".to_owned()]))
    );
    let impression = &parsed.records[0];
    assert_eq!(impression.headword, "印象");
    assert_eq!(impression.traditional, text("印象"));
    assert_eq!(impression.pinyin, text("yin4xiang4"));
    assert_eq!(impression.definition, Field::Omitted, "Pleco exports none");
    let stay = &parsed.records[1];
    assert_eq!(stay.headword, "留下");
    assert_eq!(stay.pinyin, text("liu2 xia4"), "the split mark is dropped");
    let around = &parsed.records[6];
    assert_eq!(around.headword, "周围");
    assert_eq!(around.traditional, text("週圍"));
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
            simplified: "学校".to_owned(),
            traditional: "學校".to_owned(),
            pinyin: "xue2 xiao4".to_owned(),
            definition: "school".to_owned(),
            category: Some("Places".to_owned()),
        },
        ExportRow {
            simplified: "你好".to_owned(),
            traditional: "你好".to_owned(),
            pinyin: "ni3 hao3".to_owned(),
            definition: String::new(),
            category: None,
        },
    ];
    let bytes = Utf8TextV1.serialize(&rows);
    assert_eq!(
        String::from_utf8(bytes.clone()).unwrap(),
        "//Places\n学校[學校]\txue2 xiao4\tschool\n你好\tni3 hao3\n"
    );
    let back = Utf8TextV1.parse(&bytes).unwrap();
    assert_eq!(back.records[0].characters, "学校[學校]");
    assert_eq!(back.records[1].definition, None);
}

#[test]
fn dictionary_definitions_are_left_out_for_pleco() {
    let mut school = record("学校", "學校", "xue2 xiao4", "school");
    school.definition_is_dictionary_default = true;
    let custom = record("米饭", "米飯", "mi3 fan4", "rice, the way grandma makes it");
    let written = Pleco
        .write(&[school, custom], &WriteOptions::default())
        .unwrap();
    let text = String::from_utf8(written.bytes).unwrap();
    assert_eq!(
        text,
        "学校[學校]\txue2 xiao4\n米饭[米飯]\tmi3 fan4\trice, the way grandma makes it\n"
    );
    assert!(written.notes.iter().any(|note| note.contains("left out")));
}

#[test]
fn one_category_per_word_and_the_rest_is_reported() {
    let mut a = record("一", "一", "yi1", "one");
    a.collections = vec!["Numbers".to_owned(), "HSK 1".to_owned()];
    let mut b = record("猫", "貓", "mao1", "cat");
    b.collections = vec!["Animals".to_owned()];
    let plain = record("好", "好", "hao3", "good");
    let written = Pleco
        .write(&[a, b, plain], &WriteOptions::default())
        .unwrap();
    assert_eq!(
        String::from_utf8(written.bytes).unwrap(),
        "好\thao3\tgood\n//Animals\n猫[貓]\tmao1\tcat\n//HSK 1\n一\tyi1\tone\n"
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
    let mut a = record("一", "一", "yi1", "one");
    a.collections = vec!["Numbers".to_owned(), "HSK 1".to_owned()];
    let mut options = WriteOptions::default();
    options.collection = Some("Numbers".to_owned());
    let written = Pleco.write(&[a], &options).unwrap();
    assert_eq!(
        String::from_utf8(written.bytes).unwrap(),
        "//Numbers\n一\tyi1\tone\n"
    );
    assert!(written.notes.is_empty(), "{:?}", written.notes);
}
