//! The product's use cases, end to end through `Shouci`, one test each.
//! Each test reads as Given / When / Then.

use std::path::Path;

use shouci_core::testing::{empty_sandbox, install_dictionary, sandbox, scratch_dir};
use shouci_core::{
    BulkAction, Config, DictionaryStatus, ErrorKind, ExportRequest, ExportScope, ImportAction,
    ImportPolicy, ItemPatch, LibraryFilter, LibraryView, Lifecycle, ManualWord, MatchBasis,
    QueryKind, QuickAdd, SaveOutcome, Shouci, SourceKind, Verification,
};

fn saved(found: QuickAdd) -> shouci_core::SaveResult {
    match found {
        QuickAdd::Saved(result) => *result,
        QuickAdd::Ambiguous { candidates } => panic!("ambiguous: {candidates:?}"),
    }
}

fn active() -> LibraryFilter {
    LibraryFilter::default()
}

fn view(view: LibraryView) -> LibraryFilter {
    LibraryFilter {
        view,
        ..LibraryFilter::default()
    }
}

fn heads(shouci: &Shouci, filter: &LibraryFilter) -> Vec<String> {
    shouci
        .list_items(filter)
        .unwrap()
        .into_iter()
        .map(|item| item.simplified)
        .collect()
}

// --- Capture --------------------------------------------------------------

#[test]
fn add_from_a_dictionary_result() {
    // Given a search result for "school"
    let shouci = sandbox();
    let found = shouci.search_dictionary("school", None, None).unwrap();
    let school = &found.candidates[0];
    assert_eq!(school.simplified, "学校");
    assert_eq!(school.pinyin_display, "xué xiào");
    assert!(school.saved.is_none());
    // When it is saved
    let result = shouci.save_candidate(school).unwrap();
    // Then the word is in the library, confirmed, from the dictionary
    assert_eq!(result.outcome, SaveOutcome::Inserted);
    assert_eq!(result.item.verification, Verification::Confirmed);
    assert_eq!(result.item.source.kind, SourceKind::Dictionary);
    assert_eq!(result.item.source.id.as_deref(), Some("cc-cedict"));
    // and searching again marks it saved
    let again = shouci.search_dictionary("school", None, None).unwrap();
    assert_eq!(
        again.candidates[0].saved.as_ref().unwrap().id,
        result.item.id
    );
}

#[test]
fn quick_add_saves_a_single_strong_match() {
    let shouci = sandbox();
    let result = saved(shouci.quick_add("旅行", None).unwrap());
    assert_eq!(result.item.simplified, "旅行");
    assert_eq!(result.item.pinyin, "lv3 xing2");
    assert_eq!(result.item.pinyin_display, "lǚ xíng");
}

#[test]
fn quick_add_never_guesses_between_strong_matches() {
    let shouci = sandbox();
    let QuickAdd::Ambiguous { candidates } = shouci.quick_add("lv3", None).unwrap() else {
        panic!("lv3 matches 旅途 and 旅行");
    };
    let heads: Vec<_> = candidates.iter().map(|c| c.simplified.as_str()).collect();
    assert!(
        heads.contains(&"旅途") && heads.contains(&"旅行"),
        "{heads:?}"
    );
    assert_eq!(shouci.list_items(&active()).unwrap(), []);
}

#[test]
fn quick_add_keeps_unknown_text_for_review() {
    let shouci = sandbox();
    let result = saved(shouci.quick_add("zzzqqq", None).unwrap());
    assert_eq!(result.item.verification, Verification::NeedsReview);
    assert_eq!(result.item.simplified, "zzzqqq");
    assert_eq!(result.item.definition, "", "no invented definition");
}

#[test]
fn text_that_is_not_a_word_shows_the_words_inside_it_but_is_kept_as_typed() {
    let shouci = sandbox();
    let found = shouci.search_dictionary("学校教师", None, None).unwrap();
    let parts: Vec<&str> = found
        .candidates
        .iter()
        .map(|c| c.simplified.as_str())
        .collect();
    assert_eq!(parts, ["学校", "教师"]);
    assert!(
        found
            .candidates
            .iter()
            .all(|c| c.inferred && c.basis == MatchBasis::ContainedWord)
    );
    let sample = shouci.search_dictionary("猫喝水", None, None).unwrap();
    let parts: Vec<&str> = sample
        .candidates
        .iter()
        .map(|c| c.simplified.as_str())
        .collect();
    assert_eq!(parts, ["猫", "喝水"], "the parts, in reading order");

    let picked = shouci.save_candidate(&found.candidates[0]).unwrap();
    assert_eq!(picked.item.simplified, "学校");
    assert_eq!(
        picked.item.verification,
        Verification::Confirmed,
        "a word picked from inside the query is that word"
    );
    shouci.bulk(&[picked.item.id], &BulkAction::Trash).unwrap();
    shouci.bulk(&[picked.item.id], &BulkAction::Purge).unwrap();

    let result = saved(shouci.quick_add("学校教师", None).unwrap());
    assert_eq!(
        result.item.simplified, "学校教师",
        "saved as typed, not as one of its parts"
    );
    assert_eq!(result.item.verification, Verification::NeedsReview);
}

#[test]
fn saving_twice_reports_already_saved_and_a_trashed_word_comes_back() {
    let shouci = sandbox();
    let first = saved(shouci.quick_add("旅行", None).unwrap());
    let again = saved(shouci.quick_add("旅行", None).unwrap());
    assert_eq!(again.outcome, SaveOutcome::AlreadySaved);
    shouci.bulk(&[first.item.id], &BulkAction::Trash).unwrap();
    let back = saved(shouci.quick_add("旅行", None).unwrap());
    assert_eq!(back.outcome, SaveOutcome::Restored);
    assert_eq!(back.item.id, first.item.id);
    assert_eq!(back.item.lifecycle, Lifecycle::Active);
}

#[test]
fn manual_words_get_their_traditional_form_from_the_dictionary() {
    let shouci = sandbox();
    let result = shouci
        .add_manual(&ManualWord {
            simplified: "学校".to_owned(),
            definition: Some("where I teach".to_owned()),
            tags: vec!["work".to_owned()],
            ..ManualWord::default()
        })
        .unwrap();
    assert_eq!(result.item.traditional, "學校");
    assert_eq!(result.item.pinyin, "xue2 xiao4");
    assert_eq!(result.item.verification, Verification::Confirmed);
    assert_eq!(result.item.source.kind, SourceKind::Manual);
    assert_eq!(result.item.tags, vec!["work"]);
    let bare = shouci
        .add_manual(&ManualWord {
            simplified: "生词".to_owned(),
            ..ManualWord::default()
        })
        .unwrap();
    assert_eq!(bare.item.verification, Verification::NeedsReview);
}

// --- Search ---------------------------------------------------------------

#[test]
fn the_dictionary_is_searched_without_choosing_a_mode() {
    let shouci = sandbox();
    let nihao = shouci.search_dictionary("nihao", None, None).unwrap();
    assert_eq!(nihao.guessed, QueryKind::English);
    assert_eq!(nihao.kind, QueryKind::Pinyin);
    assert_eq!(nihao.candidates[0].simplified, "你好");
    assert_eq!(
        shouci.search_dictionary("旅行", None, None).unwrap().kind,
        QueryKind::Chinese
    );
    let limited = shouci.search_dictionary("teach", None, Some(1)).unwrap();
    assert_eq!(limited.candidates.len(), 1);
    assert!(limited.total > 1);
}

#[test]
fn the_library_is_searched_by_hanzi_pinyin_or_english() {
    let shouci = sandbox();
    for word in ["学校", "旅行", "米饭"] {
        saved(shouci.quick_add(word, None).unwrap());
    }
    let search = |query: &str| -> Vec<String> {
        shouci
            .search_library(query, &active(), None, None)
            .unwrap()
            .items
            .into_iter()
            .map(|item| item.simplified)
            .collect()
    };
    assert_eq!(search("学"), vec!["学校"]);
    assert_eq!(search("xuexiao"), vec!["学校"]);
    assert_eq!(search("lǚxíng"), vec!["旅行"]);
    assert_eq!(search("rice"), vec!["米饭"]);
    assert_eq!(search("").len(), 3, "empty lists everything");
    assert_eq!(
        shouci
            .search_library("rice", &active(), None, None)
            .unwrap()
            .kind,
        Some(QueryKind::English)
    );
}

#[test]
fn searching_before_the_dictionary_loads_says_so() {
    let shouci = empty_sandbox();
    assert_eq!(shouci.dictionary_status(), DictionaryStatus::NotLoaded);
    let err = shouci.search_dictionary("school", None, None).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unavailable);
    let err = shouci.load_dictionaries().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unavailable);
    assert!(matches!(
        shouci.dictionary_status(),
        DictionaryStatus::Failed { .. }
    ));
    // The library itself works without a dictionary.
    assert_eq!(shouci.list_items(&active()).unwrap(), []);
}

// --- Manage ---------------------------------------------------------------

#[test]
fn edits_are_saved_to_the_word() {
    let shouci = sandbox();
    let id = saved(shouci.quick_add("学校", None).unwrap()).item.id;
    let edited = shouci
        .update_item(
            id,
            &ItemPatch {
                definition: Some("school; CL:所[suo3]".to_owned()),
                notes: Some("my old one".to_owned()),
                ..ItemPatch::default()
            },
        )
        .unwrap();
    assert_eq!(edited.definition_display, "school; measure word: 所 (suǒ)");
    assert_eq!(shouci.item(id).unwrap().notes, "my old one");
}

#[test]
fn editing_into_another_saved_word_is_refused() {
    let shouci = sandbox();
    saved(shouci.quick_add("旅行", None).unwrap());
    let id = saved(shouci.quick_add("旅途", None).unwrap()).item.id;
    let err = shouci
        .update_item(
            id,
            &ItemPatch {
                simplified: Some("旅行".to_owned()),
                pinyin: Some("lǚxíng".to_owned()),
                ..ItemPatch::default()
            },
        )
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conflict);
}

#[test]
fn delete_moves_to_the_trash_then_purge_removes_for_good() {
    let shouci = sandbox();
    let id = saved(shouci.quick_add("学校", None).unwrap()).item.id;
    shouci.bulk(&[id], &BulkAction::Trash).unwrap();
    assert_eq!(heads(&shouci, &active()), [] as [String; 0]);
    assert_eq!(heads(&shouci, &view(LibraryView::Trash)), vec!["学校"]);
    shouci.bulk(&[id], &BulkAction::Restore).unwrap();
    assert_eq!(heads(&shouci, &active()), vec!["学校"]);
    shouci.bulk(&[id], &BulkAction::Trash).unwrap();
    assert_eq!(shouci.empty_trash().unwrap().changed, 1);
    assert_eq!(shouci.item(id).unwrap_err().kind(), ErrorKind::NotFound);
}

#[test]
fn archive_keeps_a_word_out_of_the_way() {
    let shouci = sandbox();
    let id = saved(shouci.quick_add("学校", None).unwrap()).item.id;
    shouci.bulk(&[id], &BulkAction::Archive).unwrap();
    assert_eq!(heads(&shouci, &active()), [] as [String; 0]);
    assert_eq!(heads(&shouci, &view(LibraryView::Archived)), vec!["学校"]);
    assert_eq!(heads(&shouci, &view(LibraryView::All)), vec!["学校"]);
}

#[test]
fn tags_find_filter_and_count() {
    let shouci = sandbox();
    let a = saved(shouci.quick_add("学校", None).unwrap()).item.id;
    let b = saved(shouci.quick_add("米饭", None).unwrap()).item.id;
    shouci
        .bulk(&[a, b], &BulkAction::AddTags(vec!["HSK1".to_owned()]))
        .unwrap();
    shouci
        .bulk(&[b], &BulkAction::AddTags(vec!["food".to_owned()]))
        .unwrap();
    let filter = LibraryFilter {
        tags: vec!["hsk1".to_owned(), "food".to_owned()],
        ..LibraryFilter::default()
    };
    assert_eq!(heads(&shouci, &filter), vec!["米饭"]);
    let tags = shouci.tags().unwrap();
    assert_eq!(tags[1].name, "HSK1");
    assert_eq!(tags[1].count, 2);
    shouci.rename_tag("food", "Food").unwrap();
    assert_eq!(shouci.item(b).unwrap().tags, vec!["Food", "HSK1"]);
}

#[test]
fn collections_organize_words() {
    let shouci = sandbox();
    let a = saved(shouci.quick_add("学校", None).unwrap()).item.id;
    let b = saved(shouci.quick_add("旅行", None).unwrap()).item.id;
    shouci.create_collection("Lesson 1").unwrap();
    assert_eq!(
        shouci.create_collection("lesson 1").unwrap_err().kind(),
        ErrorKind::Conflict
    );
    shouci
        .bulk(&[a, b], &BulkAction::AddToCollection("Lesson 1".to_owned()))
        .unwrap();
    let filter = LibraryFilter {
        collection: Some("lesson 1".to_owned()),
        ..LibraryFilter::default()
    };
    assert_eq!(heads(&shouci, &filter).len(), 2);
    assert_eq!(shouci.collections().unwrap()[0].count, 2);
    shouci
        .bulk(&[a], &BulkAction::AddTags(vec!["Week 2".to_owned()]))
        .unwrap();
    let made = shouci.collection_from_tag("week 2").unwrap();
    assert_eq!(made.count, 1);
    assert_eq!(shouci.tags().unwrap(), []);
}

#[test]
fn bulk_changes_are_all_or_nothing() {
    let shouci = sandbox();
    let a = saved(shouci.quick_add("学校", None).unwrap()).item.id;
    let err = shouci.bulk(&[a, 9999], &BulkAction::Archive).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert_eq!(shouci.item(a).unwrap().lifecycle, Lifecycle::Active);
    let review = shouci
        .bulk(
            &[a],
            &BulkAction::SetVerification(Verification::NeedsReview),
        )
        .unwrap();
    assert_eq!(review.changed, 1);
    assert_eq!(
        shouci.item(a).unwrap().verification,
        Verification::NeedsReview
    );
}

// --- Dictionaries ---------------------------------------------------------

const ALT: &str = "學校 学校 [xue2 xiao4] /educational institution/\n\
                   學生 学生 [xue2 sheng5] /pupil/\n";

#[test]
fn dictionaries_can_be_enabled_ordered_and_viewed_side_by_side() {
    // Given a second dictionary
    let shouci = sandbox();
    install_dictionary(&shouci.config().dictionaries_dir, "alt", ALT).unwrap();
    let id = saved(shouci.quick_add("学校", None).unwrap()).item.id;
    shouci.load_dictionaries().unwrap();
    let listed = shouci.dictionaries().unwrap();
    assert_eq!(listed[0].id, "cc-cedict", "built-ins first");
    assert_eq!(listed[0].priority, Some(0));
    assert_eq!(listed[1].id, "alt");

    // When the saved word is viewed in the other dictionary
    let alt = shouci.lookup_in(id, "alt").unwrap();
    assert_eq!(alt[0].glosses, vec!["educational institution"]);
    assert!(alt[0].same_word);
    // Then the saved word is unchanged
    assert_eq!(shouci.item(id).unwrap().definition, "school");

    // When only "alt" is enabled, search uses it
    shouci
        .set_enabled_dictionaries(&["alt".to_owned()])
        .unwrap();
    let found = shouci.search_dictionary("学生", None, None).unwrap();
    assert_eq!(found.candidates[0].dictionary, "alt");
    assert_eq!(
        shouci
            .search_dictionary("旅行", None, None)
            .unwrap()
            .candidates,
        []
    );
    // and a disabled dictionary can still be viewed
    assert_eq!(
        shouci.lookup_in(id, "cc-cedict").unwrap()[0].glosses,
        vec!["school"]
    );

    // When its definition is adopted
    let adopted = shouci.use_definition(id, "alt").unwrap();
    assert_eq!(adopted.definition, "educational institution");
    assert_eq!(adopted.source.id.as_deref(), Some("alt"));
}

#[test]
fn enabling_nothing_or_something_unknown_is_refused() {
    let shouci = sandbox();
    assert_eq!(
        shouci.set_enabled_dictionaries(&[]).unwrap_err().kind(),
        ErrorKind::Invalid
    );
    assert_eq!(
        shouci
            .set_enabled_dictionaries(&["nope".to_owned()])
            .unwrap_err()
            .kind(),
        ErrorKind::Invalid
    );
}

// --- Import / export ------------------------------------------------------

#[test]
fn import_export_new_then_merge_back() {
    let shouci = sandbox();
    let dir = scratch_dir("transfer");
    let source = dir.join("from-pleco.txt");
    std::fs::write(&source, "[Food]\n米饭\tmi3 fan4\n你好\tni3 hao3\thello\n").unwrap();

    // Import from Pleco
    let plan = shouci
        .preview_import(&source, "pleco", ImportPolicy::Skip, false)
        .unwrap();
    assert_eq!(plan.counts().inserts, 2);
    let summary = shouci.apply_import(&plan).unwrap();
    assert_eq!(summary.inserted, 2);
    let rice = shouci
        .search_library("米饭", &active(), None, None)
        .unwrap()
        .items
        .remove(0);
    assert_eq!(rice.collections, vec!["Food"]);
    assert_eq!(rice.destinations, vec!["pleco"]);

    // Capture one more and export only new words to Pleco: just that one
    saved(shouci.quick_add("学校", None).unwrap());
    let out = dir.join("to-pleco.txt");
    let plan = shouci
        .preview_export(&out, "pleco", &ExportRequest::default())
        .unwrap();
    assert_eq!(plan.words, vec!["学校"]);
    assert_eq!(plan.left_out_already_there, 2);
    assert_eq!(shouci.apply_export(&plan).unwrap().written, 1);
    // A second "new" export is empty
    let again = shouci
        .preview_export(&out, "pleco", &ExportRequest::default())
        .unwrap();
    assert_eq!(again.item_ids, [] as [i64; 0]);

    // Merging the exported file back changes nothing
    let merge = shouci
        .preview_import(&out, "pleco", ImportPolicy::Merge, false)
        .unwrap();
    assert!(
        merge
            .lines
            .iter()
            .all(|line| matches!(line.action, ImportAction::Skip { .. })),
        "{:?}",
        merge.lines
    );

    // Exporting over an import source is refused
    let err = shouci
        .preview_export(&source, "pleco", &ExportRequest::default())
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Invalid);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn filtered_and_selected_exports() {
    let shouci = sandbox();
    let a = saved(shouci.quick_add("学校", None).unwrap()).item.id;
    let b = saved(shouci.quick_add("米饭", None).unwrap()).item.id;
    shouci
        .bulk(&[b], &BulkAction::AddToCollection("Food".to_owned()))
        .unwrap();
    let dir = scratch_dir("filtered");
    let out = dir.join("anki.txt");
    let collection = ExportRequest {
        scope: ExportScope::All,
        filter: LibraryFilter {
            collection: Some("Food".to_owned()),
            ..LibraryFilter::default()
        },
        ..ExportRequest::default()
    };
    let plan = shouci.preview_export(&out, "anki", &collection).unwrap();
    assert_eq!(plan.item_ids, vec![b]);
    shouci.apply_export(&plan).unwrap();
    assert!(
        std::fs::read_to_string(&out)
            .unwrap()
            .contains("#deck:Food\n")
    );
    let selected = ExportRequest {
        scope: ExportScope::Selected(vec![a]),
        ..ExportRequest::default()
    };
    assert_eq!(
        shouci
            .preview_export(&out, "anki", &selected)
            .unwrap()
            .item_ids,
        vec![a]
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_file_changed_after_the_preview_is_not_imported() {
    let shouci = sandbox();
    let dir = scratch_dir("changed");
    let source = dir.join("in.txt");
    std::fs::write(&source, "你好\tni3 hao3\thello\n").unwrap();
    let plan = shouci
        .preview_import(&source, "pleco", ImportPolicy::Skip, false)
        .unwrap();
    std::fs::write(&source, "学校\txue2 xiao4\tschool\n").unwrap();
    let err = shouci.apply_import(&plan).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert_eq!(shouci.list_items(&active()).unwrap(), []);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_import_file_is_read_as_the_format_that_fits() {
    // Given an Anki export and a Pleco file
    let shouci = sandbox();
    let id = saved(shouci.quick_add("学校", None).unwrap()).item.id;
    let dir = scratch_dir("detect");
    let anki = dir.join("anki.txt");
    let request = ExportRequest {
        scope: ExportScope::Selected(vec![id]),
        ..ExportRequest::default()
    };
    let plan = shouci.preview_export(&anki, "anki", &request).unwrap();
    shouci.apply_export(&plan).unwrap();
    let pleco = dir.join("pleco.txt");
    std::fs::write(&pleco, "你好\tni3 hao3\thello\n").unwrap();
    // Then each is read as its own format
    for (path, expected) in [(&anki, "anki"), (&pleco, "pleco")] {
        let plan = shouci
            .detect_import(path, ImportPolicy::Skip, false)
            .unwrap();
        assert_eq!(plan.connector_id, expected, "{}", path.display());
    }
    // and a missing file says so
    let err = shouci
        .detect_import(&dir.join("missing.txt"), ImportPolicy::Skip, false)
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn unknown_connectors_are_named() {
    let shouci = sandbox();
    let err = shouci
        .preview_import(
            Path::new("/tmp/x.txt"),
            "skritter",
            ImportPolicy::Skip,
            false,
        )
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert!(err.to_string().contains("pleco, anki"), "{err}");
}

// --- Connector contract ---------------------------------------------------

#[test]
fn every_connector_round_trips_a_word_and_rejects_binary() {
    let shouci = sandbox();
    let id = saved(shouci.quick_add("学校", None).unwrap()).item.id;
    let dir = scratch_dir("contract");
    for connector in shouci.connectors() {
        assert!(
            connector.can_import && connector.can_export,
            "{}",
            connector.id
        );
        let out = dir.join(format!("{}.txt", connector.id));
        let request = ExportRequest {
            scope: ExportScope::Selected(vec![id]),
            ..ExportRequest::default()
        };
        let plan = shouci
            .preview_export(&out, &connector.id, &request)
            .unwrap();
        shouci.apply_export(&plan).unwrap();
        let back = shouci
            .preview_import(&out, &connector.id, ImportPolicy::Skip, false)
            .unwrap();
        assert!(!back.refused, "{}: {:?}", connector.id, back.issues);
        assert!(
            matches!(back.lines[0].action, ImportAction::Skip { item_id: Some(found), .. } if found == id),
            "{} did not round-trip: {:?}",
            connector.id,
            back.lines
        );
        let binary = dir.join(format!("{}.bin", connector.id));
        std::fs::write(&binary, b"\xff\xfe\x00").unwrap();
        let err = shouci
            .preview_import(&binary, &connector.id, ImportPolicy::Skip, false)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Format, "{}", connector.id);
    }
    let _ = std::fs::remove_dir_all(dir);
}

// --- Data safety ----------------------------------------------------------

#[test]
fn proof_of_concept_words_come_over_on_first_open() {
    let legacy = scratch_dir("legacy");
    write_poc(&legacy.join("user.db"));
    let dir = scratch_dir("fresh");
    let mut config = Config::in_dir(&dir);
    config.fetch_dictionaries = false;
    config.legacy_dir = Some(legacy.clone());
    let shouci = Shouci::open(config.clone()).unwrap();
    assert!(
        shouci.startup_notes()[0].contains("Brought 1 words"),
        "{:?}",
        shouci.startup_notes()
    );
    assert_eq!(heads(&shouci, &active()), vec!["学校"]);
    drop(shouci);
    // Opening again does not import again.
    let reopened = Shouci::open(config).unwrap();
    assert_eq!(reopened.startup_notes(), [] as [String; 0]);
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(legacy);
}

/// A minimal proof-of-concept database with one exported word.
fn write_poc(path: &Path) {
    rusqlite::Connection::open(path)
        .unwrap()
        .execute_batch(
            "CREATE TABLE vocabulary_items (item_id INTEGER PRIMARY KEY, simplified TEXT, \
             traditional TEXT, pinyin TEXT, definition TEXT, status TEXT, notes TEXT, \
             source_id TEXT, source_version TEXT, import_origin TEXT, created_at TEXT, \
             modified_at TEXT); INSERT INTO vocabulary_items VALUES (1, '学校', '學校', \
             'xue2 xiao4', 'school', 'exported', NULL, 'cc-cedict', '1.0.0', NULL, \
             '2026-09-20 10:00:00', '2026-09-20 10:00:00');",
        )
        .unwrap();
}

#[test]
fn another_process_writing_is_noticed() {
    let shouci = sandbox();
    let mut config = shouci.config().clone();
    config.legacy_dir = None;
    let other = Shouci::open(config).unwrap();
    let before = shouci.data_version().unwrap();
    other
        .add_manual(&ManualWord {
            simplified: "生词".to_owned(),
            ..ManualWord::default()
        })
        .unwrap();
    assert_ne!(shouci.data_version().unwrap(), before);
    assert_eq!(heads(&shouci, &active()), vec!["生词"]);
}

// --- Contract shape -------------------------------------------------------

#[test]
fn json_shapes_are_stable() {
    let shouci = sandbox();
    let result = shouci.quick_add("学校", None).unwrap();
    let json = serde_json::to_value(&result).unwrap();
    assert_eq!(json["result"], "saved");
    assert_eq!(json["outcome"], "inserted");
    let item = &json["item"];
    assert_eq!(item["simplified"], "学校");
    assert_eq!(item["pinyin_display"], "xué xiào");
    assert_eq!(item["verification"], "confirmed");
    assert_eq!(item["lifecycle"], "active");
    assert_eq!(item["source"]["kind"], "dictionary");
    let bulk = serde_json::to_value(BulkAction::AddTags(vec!["a".to_owned()])).unwrap();
    assert_eq!(
        bulk,
        serde_json::json!({"action": "add_tags", "value": ["a"]})
    );
    let request: ExportRequest =
        serde_json::from_str(r#"{"scope": {"selected": [1, 2]}}"#).unwrap();
    assert_eq!(request.scope, ExportScope::Selected(vec![1, 2]));
    let request: ExportRequest = serde_json::from_str(r#"{"scope": "all"}"#).unwrap();
    assert_eq!(request.scope, ExportScope::All);
}

#[test]
fn loading_reports_problems_without_failing() {
    // Given a working dictionary next to a broken file
    let shouci = empty_sandbox();
    let dir = shouci.config().dictionaries_dir.clone();
    install_dictionary(&dir, "cc-cedict", shouci_core::testing::SAMPLE_DICTIONARY).unwrap();
    std::fs::write(dir.join("broken.db"), b"not a database").unwrap();
    // When dictionaries load
    shouci.load_dictionaries().unwrap();
    // Then search works and the problem is a note, not an error
    let DictionaryStatus::Ready { enabled, notes, .. } = shouci.dictionary_status() else {
        panic!("expected ready: {:?}", shouci.dictionary_status());
    };
    assert_eq!(enabled, 1);
    assert!(notes[0].contains("broken.db"), "{notes:?}");
    let json = serde_json::to_value(shouci.dictionary_status()).unwrap();
    assert_eq!(json["state"], "ready");
}

// --- Review regressions ---------------------------------------------------

#[test]
fn this_process_s_own_saves_change_the_data_version() {
    let shouci = sandbox();
    let before = shouci.data_version().unwrap();
    saved(shouci.quick_add("学校", None).unwrap());
    assert_ne!(shouci.data_version().unwrap(), before);
}

#[test]
fn proper_nouns_never_hide_common_words() {
    let shouci = empty_sandbox();
    install_dictionary(
        &shouci.config().dictionaries_dir,
        "cc-cedict",
        "白 白 [Bai2] /surname Bai/\n白 白 [bai2] /white/\n",
    )
    .unwrap();
    shouci.load_dictionaries().unwrap();
    let found = shouci.search_dictionary("白", None, None).unwrap();
    let readings: Vec<&str> = found.candidates.iter().map(|c| c.pinyin.as_str()).collect();
    assert!(
        readings.contains(&"Bai2") && readings.contains(&"bai2"),
        "{readings:?}"
    );
}

#[test]
fn a_single_untoned_syllable_finds_its_word() {
    let shouci = empty_sandbox();
    install_dictionary(
        &shouci.config().dictionaries_dir,
        "cc-cedict",
        "我 我 [wo3] /I; me; my/\n炒鍋 炒锅 [chao3 guo1] /wok/\n",
    )
    .unwrap();
    shouci.load_dictionaries().unwrap();
    let found = shouci.search_dictionary("wo", None, None).unwrap();
    assert_eq!(found.kind, QueryKind::Pinyin);
    assert_eq!(found.candidates[0].simplified, "我");
}

#[test]
fn queries_and_results_are_bounded() {
    let shouci = sandbox();
    let long = "x".repeat(shouci_core::MAX_QUERY_CHARS + 1);
    assert_eq!(
        shouci
            .search_dictionary(&long, None, None)
            .unwrap_err()
            .kind(),
        ErrorKind::Invalid
    );
    for word in ["学校", "旅行", "米饭"] {
        saved(shouci.quick_add(word, None).unwrap());
    }
    let page = shouci.search_library("", &active(), None, Some(2)).unwrap();
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.total, 3);
}

#[test]
fn repeated_ids_in_a_bulk_action_count_once() {
    let shouci = sandbox();
    let id = saved(shouci.quick_add("学校", None).unwrap()).item.id;
    let result = shouci.bulk(&[id, id], &BulkAction::Trash).unwrap();
    assert_eq!(result.changed, 1);
    assert_eq!(
        shouci.bulk(&[id, id], &BulkAction::Purge).unwrap().changed,
        1
    );
}

#[test]
fn the_dictionary_entry_for_a_word_is_matched_by_its_traditional_form() {
    let shouci = empty_sandbox();
    install_dictionary(
        &shouci.config().dictionaries_dir,
        "cc-cedict",
        "面 面 [mian4] /face; side/\n麵 面 [mian4] /noodles/\n",
    )
    .unwrap();
    shouci.load_dictionaries().unwrap();
    let noodles = shouci
        .search_dictionary("noodles", None, None)
        .unwrap()
        .candidates
        .remove(0);
    let id = shouci.save_candidate(&noodles).unwrap().item.id;
    let entries = shouci.lookup_in(id, "cc-cedict").unwrap();
    let matching: Vec<&str> = entries
        .iter()
        .filter(|entry| entry.same_word)
        .map(|entry| entry.traditional.as_str())
        .collect();
    assert_eq!(matching, vec!["麵"]);
    let adopted = shouci.use_definition(id, "cc-cedict").unwrap();
    assert_eq!(adopted.definition, "noodles");
}

#[test]
fn a_word_captured_offline_is_completed_later() {
    // Captured with no dictionary: saved as typed, needing review.
    let shouci = empty_sandbox();
    let placeholder = shouci
        .add_manual(&ManualWord {
            simplified: "学校".to_owned(),
            ..ManualWord::default()
        })
        .unwrap();
    assert_eq!(placeholder.item.verification, Verification::NeedsReview);
    // Later, with a dictionary, saving the word fills the same entry in.
    install_dictionary(
        &shouci.config().dictionaries_dir,
        "cc-cedict",
        shouci_core::testing::SAMPLE_DICTIONARY,
    )
    .unwrap();
    shouci.load_dictionaries().unwrap();
    let result = saved(shouci.quick_add("学校", None).unwrap());
    assert_eq!(result.outcome, SaveOutcome::Completed);
    assert_eq!(result.item.id, placeholder.item.id);
    assert_eq!(result.item.traditional, "學校");
    assert_eq!(result.item.verification, Verification::Confirmed);
    assert_eq!(shouci.list_items(&active()).unwrap().len(), 1);
}

#[test]
fn a_missing_import_file_is_not_found() {
    let shouci = sandbox();
    let err = shouci
        .preview_import(
            Path::new("/nonexistent/shouci/in.txt"),
            "pleco",
            ImportPolicy::Skip,
            false,
        )
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

#[test]
fn quick_add_saves_the_exact_word_even_when_longer_words_match() {
    let shouci = empty_sandbox();
    install_dictionary(
        &shouci.config().dictionaries_dir,
        "cc-cedict",
        "媽媽 妈妈 [ma1 ma5] /mama; mommy/\n媽媽的 妈妈的 [ma1 ma5 de5] /(slang) damn it/\n",
    )
    .unwrap();
    shouci.load_dictionaries().unwrap();
    let result = saved(shouci.quick_add("妈妈", None).unwrap());
    assert_eq!(result.item.simplified, "妈妈");
}
