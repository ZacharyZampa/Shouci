use vocab_anki::{AnkiTextV1, IssueSeverity, NoteRow};

#[test]
fn round_trips_notes() {
    let notes = [NoteRow {
        line: 1,
        headword: "你好".to_owned(),
        traditional: "你好".to_owned(),
        pinyin: "ni3 hao3".to_owned(),
        definition: "hello".to_owned(),
        notes: "said on arrival".to_owned(),
        tags: vec!["Greetings".to_owned()],
        raw: String::new(),
    }];
    let bytes = AnkiTextV1.write(&notes, "Shouci").unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(text.starts_with("#separator:tab\n"), "{text}");
    let parsed = AnkiTextV1.parse(&bytes).unwrap();
    assert!(!parsed.has_errors());
    assert_eq!(parsed.notes.len(), 1);
    assert_eq!(parsed.notes[0].headword, "你好");
    assert_eq!(parsed.notes[0].notes, "said on arrival");
    assert_eq!(parsed.notes[0].tags, vec!["Greetings".to_owned()]);
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
}
