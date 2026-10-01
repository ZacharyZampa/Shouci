use vocab_anki::{Anki, AnkiNote, AnkiTextV1, IssueSeverity};
use vocab_core::connector::{Connector, ExportRecord, Field, WriteOptions};

fn note() -> AnkiNote {
    AnkiNote {
        headword: "你好".to_owned(),
        traditional: "你好".to_owned(),
        pinyin: "ni3 hao3".to_owned(),
        definition: "hello".to_owned(),
        notes: "said on arrival".to_owned(),
        tags: vec!["Greetings".to_owned()],
    }
}

#[test]
fn round_trips_notes() {
    let bytes = AnkiTextV1.write(&[note()], Some("Shouci")).unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(text.starts_with("#separator:tab\n"), "{text}");
    let parsed = AnkiTextV1.parse(&bytes).unwrap();
    assert!(!parsed.has_errors());
    assert_eq!(parsed.notes.len(), 1);
    assert_eq!(parsed.notes[0].headword, "你好");
    assert_eq!(parsed.notes[0].notes.as_deref(), Some("said on arrival"));
    assert_eq!(parsed.notes[0].tags, Some(vec!["Greetings".to_owned()]));
    assert_eq!(parsed.deck.as_deref(), Some("Shouci"));
}

#[test]
fn wrong_width_is_a_line_error() {
    let parsed = AnkiTextV1
        .parse(b"#separator:tab\nonly-one-field\n")
        .unwrap();
    assert!(parsed.has_errors());
    assert_eq!(parsed.issues[0].line, 2);
    assert_eq!(parsed.issues[0].severity, IssueSeverity::Error);
}

#[test]
fn comma_separator_is_refused() {
    let err = AnkiTextV1.parse(b"#separator:comma\n").unwrap_err();
    assert!(err.to_string().contains("tab-only"), "{err}");
    assert_eq!(err.kind(), vocab_core::ErrorKind::Format);
}

#[test]
fn missing_columns_are_omitted_and_the_deck_is_a_collection() {
    let file = "#separator:tab\n#deck:Lesson 3\n#columns:Headword\tPinyin\tDefinition\n\
                学校\txue2 xiao4\tschool\n";
    let parsed = Anki.parse(file.as_bytes()).unwrap();
    let record = &parsed.records[0];
    assert_eq!(record.definition, Field::Present("school".to_owned()));
    assert_eq!(record.notes, Field::Omitted);
    assert_eq!(record.tags, Field::Omitted);
    assert_eq!(record.traditional, Field::Omitted);
    assert_eq!(
        record.collections,
        Field::Present(vec!["Lesson 3".to_owned()])
    );
}

#[test]
fn export_uses_the_collection_as_deck_and_fixes_tags() {
    let mut record = ExportRecord::new("学校", "學校", "xue2 xiao4", "school");
    record.definition_is_dictionary_default = true;
    record.tags = vec!["HSK 1".to_owned(), "places".to_owned()];
    let mut options = WriteOptions::default();
    options.collection = Some("Lesson 3".to_owned());
    let written = Anki.write(&[record.clone()], &options).unwrap();
    let text = String::from_utf8(written.bytes).unwrap();
    assert!(text.contains("#deck:Lesson 3\n"), "{text}");
    assert!(
        text.contains("学校\t學校\txue2 xiao4\tschool\t\tHSK_1 places\n"),
        "Anki always gets the definition: {text}"
    );
    assert!(written.notes[0].contains("HSK 1 → HSK_1"));
    let plain = Anki.write(&[record], &WriteOptions::default()).unwrap();
    assert!(
        !String::from_utf8(plain.bytes).unwrap().contains("#deck:"),
        "no invented deck"
    );
}

#[test]
fn empty_trailing_fields_survive_a_round_trip() {
    let mut bare = note();
    bare.notes.clear();
    bare.tags.clear();
    let bytes = AnkiTextV1.write(&[bare], Some("Shouci")).unwrap();
    let parsed = AnkiTextV1.parse(&bytes).unwrap();
    assert!(!parsed.has_errors(), "{:?}", parsed.issues);
    assert_eq!(parsed.notes[0].notes.as_deref(), Some(""));
    assert_eq!(parsed.notes[0].tags, Some(Vec::new()));
}
